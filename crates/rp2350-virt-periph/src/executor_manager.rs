//! Manager-facing step API — dispatch, collect, fixed latency and shutdown
//! 面向管理器的步进 API — 派发、收集、固定延迟与关闭

// 类型名沿用项目命名约定；文档引用使用内部标识符。
// Type names follow project conventions; doc references use internal identifiers.
#![allow(clippy::doc_markdown, clippy::module_name_repetitions)]

use rp2350_core::gpio::GpioPin;
use std::time::Duration;

use super::device::PortDirection;
use super::executor::{
    ExecutorConfig, ExecutorError, PinSnapshot, PublishSchedule, StepError, StepReport,
};
use super::executor_device::SubmitOutcome;
use super::manager::{DeviceId, DeviceManager};

impl DeviceManager {
    /// Create a manager whose devices run on their own worker threads
    /// 创建“设备运行在各自工作线程上”的管理器
    ///
    /// This is the opt-in threading switch; [`DeviceManager::new`] keeps the
    /// synchronous default. 这是可选的多线程开关；[`DeviceManager::new`] 保持
    /// 同步默认路径。
    #[must_use]
    pub fn with_executor(config: ExecutorConfig) -> Self {
        let mut manager = Self::new();
        manager.executor = Some(config);
        manager.schedule = PublishSchedule::new(config.latency_ticks);
        manager
    }

    /// Create a synchronous manager with a custom fixed latency
    /// 创建具有自定义固定延迟的同步管理器
    ///
    /// Useful to compare the two paths at the same latency; the default manager
    /// uses one peripheral tick. 用于在同一延迟下对照两条路径；默认管理器为
    /// 1 个外设 tick。
    #[must_use]
    pub fn with_latency(latency_ticks: u64) -> Self {
        let mut manager = Self::new();
        manager.schedule = PublishSchedule::new(latency_ticks);
        manager
    }

    /// Executor configuration, `None` in synchronous mode
    /// 执行器配置；同步模式下为 `None`
    #[must_use]
    pub fn executor_config(&self) -> Option<ExecutorConfig> {
        self.executor
    }

    /// Propagation delay in epochs / 传播延迟（epoch）
    #[must_use]
    pub fn latency_ticks(&self) -> u64 {
        self.schedule.latency()
    }

    /// Number of running plugin worker threads owned by this manager
    /// 本管理器持有的运行中插件工作线程数量
    #[must_use]
    pub fn thread_count(&self) -> usize {
        self.devices
            .values()
            .filter(|managed| managed.host.has_live_worker())
            .count()
    }

    /// Devices currently latched high-Z, sorted by ID
    /// 当前处于高阻的设备（按 ID 排序）
    #[must_use]
    pub fn degraded_devices(&self) -> Vec<DeviceId> {
        self.devices
            .iter()
            .filter(|(_, managed)| managed.host.is_degraded())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// One stable diagnostic line per degraded device, sorted by ID
    /// 每个高阻设备一条稳定诊断（按 ID 排序）
    #[must_use]
    pub fn degraded_reasons(&self) -> Vec<(DeviceId, String)> {
        self.devices
            .iter()
            .filter_map(|(id, managed)| {
                managed
                    .host
                    .degraded_reason()
                    .map(|reason| (id.clone(), reason))
            })
            .collect()
    }

    /// Override one device's mailbox round-trip bound / 覆盖单台设备的邮箱往返上限
    ///
    /// Only worker-hosted devices have a mailbox, so inline devices and unknown
    /// IDs return `false`. On `true`, the device's next command uses `timeout`;
    /// a device already latched high-Z stays high-Z until it is reloaded.
    /// Useful to give a known-slow device a tighter stall bound while the rest
    /// of the manager keeps the production default.
    /// 只有工作线程设备有邮箱，因此内联设备与未知 ID 返回 `false`。返回 `true` 时，
    /// 该设备的下一条命令使用 `timeout`；已锁定高阻的设备在重载前保持高阻。可用它
    /// 给已知较慢的设备设置更紧的卡死上限，而管理器其余设备保持生产默认值。
    #[must_use]
    pub fn set_step_timeout(&mut self, id: &str, timeout: Duration) -> bool {
        self.devices
            .get_mut(id)
            .is_some_and(|managed| managed.host.set_step_timeout(timeout))
    }

    /// Pins currently driven by at least one device output
    /// 当前至少被一个设备输出驱动的引脚
    #[must_use]
    pub fn driven_pins(&self) -> PinSnapshot {
        self.driven_pins
    }

    /// Advance every device by one epoch with a fixed propagation delay
    /// 以固定传播延迟推进所有设备一个 epoch
    ///
    /// Devices see `inputs` at `epoch`; the outputs they produce become effective
    /// at `epoch + latency_ticks` and are returned by a later call in
    /// [`StepReport::effective`]. Device outputs are combined with wired-OR and
    /// per-device results are reported in [`StepReport::produced`], so the
    /// outcome never depends on host scheduling. `produced` has one entry per
    /// registered device: a high-Z device carries an empty snapshot (it drives
    /// nothing) and is also listed in [`StepReport::degraded`].
    /// 设备在 `epoch` 看到 `inputs`；产生的输出在 `epoch + latency_ticks` 生效，并由
    /// 之后的调用通过 [`StepReport::effective`] 返回。设备输出按线或组合，各设备结果
    /// 记录在 [`StepReport::produced`] 中，因此结果不依赖宿主调度。`produced` 为每台
    /// 已注册设备保留一个条目：高阻设备携带空快照（不驱动任何引脚），并同时列入
    /// [`StepReport::degraded`]。
    ///
    /// # Errors
    ///
    /// Returns [`StepError::NonMonotonicEpoch`] when `epoch` does not strictly
    /// increase. Device-level failures (timeout, panic) are not errors: the
    /// device is latched high-Z and listed in [`StepReport::degraded`].
    /// `epoch` 未严格递增时返回 [`StepError::NonMonotonicEpoch`]。设备级失败
    /// （超时、panic）不算错误：设备锁定为高阻并列入 [`StepReport::degraded`]。
    pub fn step(
        &mut self,
        epoch: u64,
        inputs: PinSnapshot,
        cycles: u64,
    ) -> Result<StepReport, StepError> {
        if let Some(last) = self.last_step_epoch {
            if epoch <= last {
                return Err(StepError::NonMonotonicEpoch {
                    last,
                    requested: epoch,
                });
            }
        }
        let effective = self.schedule.take_effective(epoch);
        // 阶段 1：先向所有设备派发，工作线程并行执行。
        // Phase 1: dispatch to every device so the workers run in parallel.
        let mut pending: Vec<(DeviceId, Option<Result<PinSnapshot, ExecutorError>>)> =
            Vec::with_capacity(self.devices.len());
        for (id, managed) in &mut self.devices {
            let ready = match managed.host.submit_step(epoch, inputs, cycles) {
                SubmitOutcome::Ready(result) => Some(result),
                SubmitOutcome::Waiting => None,
            };
            pending.push((id.clone(), ready));
        }

        // 阶段 2：按 ID 顺序收集响应；顺序只影响失败上报顺序，不影响输出。
        // 高阻设备仍写入 `produced`（空快照 = 不驱动），避免从输出里静默消失。
        // Phase 2: collect in ID order; the order only affects failure reporting.
        // High-Z devices still get a `produced` entry (empty snapshot = no drive)
        // so they never disappear from the report.
        let mut produced = Vec::with_capacity(self.devices.len());
        let mut degraded = Vec::new();
        let mut combined = 0u64;
        for (id, ready) in pending {
            let result = ready.unwrap_or_else(|| match self.devices.get_mut(&id) {
                Some(managed) => managed.host.collect_step(epoch),
                None => Ok(PinSnapshot::empty()),
            });
            if let Ok(outputs) = result {
                combined |= outputs.bits();
                produced.push((id, outputs));
            } else {
                produced.push((id.clone(), PinSnapshot::empty()));
                degraded.push(id);
            }
        }
        self.schedule
            .publish(epoch, PinSnapshot::from_bits(combined));
        self.last_step_epoch = Some(epoch);
        Ok(StepReport {
            epoch,
            effective,
            produced,
            degraded,
        })
    }

    /// Step one epoch while sampling and driving a GPIO pin array
    /// 采样并驱动 GPIO 引脚数组的同时推进一个 epoch
    ///
    /// Inputs are sampled with [`PinSnapshot::from_gpio_pins`]; effective outputs
    /// are written back only for pins in [`DeviceManager::driven_pins`].
    /// 输入通过 [`PinSnapshot::from_gpio_pins`] 采样；生效的输出只写回
    /// [`DeviceManager::driven_pins`] 覆盖的引脚。
    ///
    /// # Errors
    ///
    /// Same as [`DeviceManager::step`] / 与 [`DeviceManager::step`] 相同
    pub fn step_gpio(
        &mut self,
        epoch: u64,
        pins: &mut [GpioPin],
        cycles: u64,
    ) -> Result<StepReport, StepError> {
        let inputs = PinSnapshot::from_gpio_pins(pins);
        let report = self.step(epoch, inputs, cycles)?;
        if let Some(outputs) = report.effective {
            outputs.apply_masked(self.driven_pins, pins);
        }
        Ok(report)
    }

    /// Rebuild one device's binding table after a wiring change
    /// 接线变化后重建单个设备的绑定表
    pub(crate) fn refresh_bindings(&mut self, id: &str) {
        let bindings = match self.devices.get(id) {
            Some(managed) => managed.host.bindings_for(id, &self.wiring),
            None => return,
        };
        if let Some(managed) = self.devices.get_mut(id) {
            managed.host.set_bindings(bindings);
        }
        self.recompute_driven_pins();
    }

    /// Recompute the set of externally driven pins / 重新计算外部驱动的引脚集合
    pub(crate) fn recompute_driven_pins(&mut self) {
        let mut mask = PinSnapshot::empty();
        for managed in self.devices.values() {
            for binding in managed.host.bindings() {
                if matches!(
                    binding.direction,
                    PortDirection::Output | PortDirection::Bidirectional
                ) {
                    if let Some(pin) = binding.effective_pin() {
                        mask.set(pin, true);
                    }
                }
            }
        }
        self.driven_pins = mask;
    }

    /// Stop every plugin worker and join it (threaded mode only)
    /// 停止并回收所有插件工作线程（仅线程化模式）
    ///
    /// Returns the number of workers that were running. Devices stay registered:
    /// their metadata is still listed, but they are no longer stepped. 返回被停止的
    /// 工作线程数量。设备仍保持注册：元数据仍可列出，但不再推进。
    pub fn shutdown_threads(&mut self) -> usize {
        let mut stopped = 0;
        for managed in self.devices.values_mut() {
            let Some(outcome) = managed.host.stop_thread() else {
                continue;
            };
            stopped += 1;
            drop(outcome.plugin);
            if outcome.detached {
                // 卡死的线程仍可能引用插件 vtable：有意泄漏库句柄保持映射。
                // A stuck thread may still reference the vtable: intentionally
                // leak the library handle so the code stays mapped.
                if let Some(library) = managed.library.take() {
                    log::warn!(
                        "plugin thread did not stop; keeping its library mapped / \
                         插件线程未停止，保留其库映射"
                    );
                    std::mem::forget(library);
                }
            }
        }
        stopped
    }
}

impl Drop for DeviceManager {
    fn drop(&mut self) {
        // 管理器析构时必须有界地回收所有工作线程。
        // Manager drop must reclaim every worker thread with a bounded wait.
        let _ = self.shutdown_threads();
    }
}
