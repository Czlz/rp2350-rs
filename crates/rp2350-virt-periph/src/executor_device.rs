//! Per-device host — inline plugin or worker thread, with ports and bindings
//! 单设备宿主 — 内联插件或工作线程，含端口与绑定

// 类型名沿用项目命名约定；文档引用使用内部标识符。
// Type names follow project conventions; doc references use internal identifiers.
#![allow(clippy::doc_markdown, clippy::module_name_repetitions)]

use rp2350_plugin::PluginMetadata;

use super::device::{DevicePlugin, DevicePort};
use super::executor::{ExecutorConfig, ExecutorError, PinSnapshot, PortBinding};
use super::executor_host::DeviceExecutor;
use super::executor_worker::{WorkerContext, step_plugin};
use super::manager::{ConfigApplyError, DeviceId};
use super::wiring::WiringTable;

/// Outcome of dispatching one device step / 派发单设备步进的结果
pub(crate) enum SubmitOutcome {
    /// The step already finished (inline device or an immediate failure)
    /// 步进已完成（内联设备或立即失败）
    Ready(Result<PinSnapshot, ExecutorError>),
    /// The command was sent; the result must be collected with `collect_step`
    /// 命令已发送；结果需用 `collect_step` 收集
    Waiting,
}

/// Outcome of stopping one device host / 停止单个设备宿主的结果
pub(crate) struct StopOutcome {
    /// Plugin handed back to the host, `None` when already gone
    /// 交还宿主的插件；已不存在时为 `None`
    pub(crate) plugin: Option<Box<dyn DevicePlugin>>,
    /// Worker was detached and its library must stay mapped
    /// 工作线程已分离，其库必须保持映射
    pub(crate) detached: bool,
}

/// Where the plugin instance actually lives / 插件实例实际所在位置
enum HostInner {
    /// Owned by the simulation thread (default path) / 由模拟线程持有（默认路径）
    Inline(Option<Box<dyn DevicePlugin>>),
    /// Owned by a worker thread / 由工作线程持有
    Threaded(DeviceExecutor),
}

/// One managed device: metadata cache plus inline or threaded plugin
/// 单个受管设备：元数据缓存 + 内联或线程化插件
pub(crate) struct DeviceHost {
    /// Metadata captured at registration / 注册时捕获的元数据
    metadata: PluginMetadata,
    /// `has_ui()` captured at registration / 注册时捕获的 has_ui
    has_ui: bool,
    /// Port → pin bindings used by this device / 本设备使用的端口→引脚绑定
    bindings: Vec<PortBinding>,
    /// Plugin storage / 插件存储
    inner: HostInner,
}

impl DeviceHost {
    /// Host a plugin on the simulation thread (same semantics as before MT-5)
    /// 在模拟线程上托管插件（与 MT-5 之前语义一致）
    pub(crate) fn inline(plugin: Box<dyn DevicePlugin>) -> Self {
        let metadata = plugin.metadata().clone();
        let has_ui = plugin.has_ui();
        Self {
            metadata,
            has_ui,
            bindings: Vec::new(),
            inner: HostInner::Inline(Some(plugin)),
        }
    }

    /// Host a plugin on its own worker thread / 在独立工作线程上托管插件
    pub(crate) fn threaded(
        id: DeviceId,
        plugin: Box<dyn DevicePlugin>,
        bindings: Vec<PortBinding>,
        config: ExecutorConfig,
    ) -> Self {
        let metadata = plugin.metadata().clone();
        let has_ui = plugin.has_ui();
        let ports = plugin.ports().to_vec();
        let executor = DeviceExecutor::spawn(id, plugin, ports, bindings.clone(), config);
        Self {
            metadata,
            has_ui,
            bindings,
            inner: HostInner::Threaded(executor),
        }
    }

    /// Plugin metadata / 插件元数据
    pub(crate) fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    /// Whether the plugin renders a GUI panel / 插件是否渲染 GUI 面板
    pub(crate) fn has_ui(&self) -> bool {
        self.has_ui
    }

    /// Port table (live for inline hosts, cached for threaded ones)
    /// 端口表（内联宿主实时读取，线程化宿主使用缓存）
    pub(crate) fn ports(&self) -> &[DevicePort] {
        match &self.inner {
            HostInner::Inline(Some(plugin)) => plugin.ports(),
            HostInner::Inline(None) => &[],
            HostInner::Threaded(executor) => executor.ports(),
        }
    }

    /// Bindings currently in use / 当前使用的绑定
    pub(crate) fn bindings(&self) -> &[PortBinding] {
        &self.bindings
    }

    /// Build the binding table for `id` from the wiring table
    /// 依据接线表构造 `id` 的绑定表
    pub(crate) fn bindings_for(&self, id: &str, wiring: &WiringTable) -> Vec<PortBinding> {
        self.ports()
            .iter()
            .enumerate()
            .map(|(index, port)| {
                PortBinding::new(index, port.direction, wiring.find_by_device_port(id, index))
            })
            .collect()
    }

    /// Store a new binding table and forward it to a worker if present
    /// 保存新绑定表，并在使用工作线程时转发
    pub(crate) fn set_bindings(&mut self, bindings: Vec<PortBinding>) {
        self.bindings.clone_from(&bindings);
        if let HostInner::Threaded(executor) = &mut self.inner {
            if let Err(error) = executor.set_bindings(bindings) {
                log::warn!("plugin '{}' binding update failed: {error}", executor.id());
            }
        }
    }

    /// Override the mailbox bound of a worker-hosted device
    /// 覆盖工作线程设备的邮箱往返上限
    ///
    /// Returns `false` for inline hosts, which have no mailbox and therefore no
    /// timeout. 内联宿主没有邮箱、也没有超时，返回 `false`。
    pub(crate) fn set_step_timeout(&mut self, timeout: std::time::Duration) -> bool {
        match &mut self.inner {
            HostInner::Inline(_) => false,
            HostInner::Threaded(executor) => {
                executor.set_step_timeout(timeout);
                true
            }
        }
    }

    /// Does this device still own a running worker? / 该设备是否仍持有运行中的工作线程？
    pub(crate) fn has_live_worker(&self) -> bool {
        match &self.inner {
            HostInner::Inline(_) => false,
            HostInner::Threaded(executor) => executor.is_alive(),
        }
    }

    /// Is the device latched high-Z? / 设备是否锁定为高阻？
    pub(crate) fn is_degraded(&self) -> bool {
        match &self.inner {
            HostInner::Inline(_) => false,
            HostInner::Threaded(executor) => executor.is_degraded(),
        }
    }

    /// Stable degradation reason / 稳定的降级原因
    pub(crate) fn degraded_reason(&self) -> Option<String> {
        match &self.inner {
            HostInner::Inline(_) => None,
            HostInner::Threaded(executor) => executor.degraded_reason().map(str::to_string),
        }
    }

    /// Deliver one GPIO level to an inline plugin (legacy `sync_gpio` path)
    /// 向内联插件投递一个 GPIO 电平（旧版 `sync_gpio` 路径）
    pub(crate) fn notify_gpio(&mut self, port_index: usize, level: bool) {
        if let HostInner::Inline(Some(plugin)) = &mut self.inner {
            plugin.on_gpio_change(port_index, level);
        }
    }

    /// Current level of an inline output port / 内联输出端口的当前电平
    pub(crate) fn output_level(&self, port_index: usize) -> Option<bool> {
        match &self.inner {
            HostInner::Inline(Some(plugin)) => {
                plugin.ports().get(port_index).map(|port| port.level)
            }
            HostInner::Inline(None) | HostInner::Threaded(_) => None,
        }
    }

    /// Advance an inline plugin (legacy `tick_all` path) / 推进内联插件
    pub(crate) fn tick_inline(&mut self, ctx: &mut dyn rp2350_plugin::PluginContext, cycles: u64) {
        if let HostInner::Inline(Some(plugin)) = &mut self.inner {
            plugin.tick(ctx, cycles);
        }
    }

    /// Dispatch one step without waiting for worker-hosted devices
    /// 派发一次步进，不等待工作线程设备
    pub(crate) fn submit_step(
        &mut self,
        epoch: u64,
        inputs: PinSnapshot,
        cycles: u64,
    ) -> SubmitOutcome {
        match &mut self.inner {
            HostInner::Inline(Some(plugin)) => {
                let mut ctx = WorkerContext::new();
                SubmitOutcome::Ready(Ok(step_plugin(
                    plugin.as_mut(),
                    &self.bindings,
                    epoch,
                    inputs,
                    cycles,
                    &mut ctx,
                )))
            }
            HostInner::Inline(None) => SubmitOutcome::Ready(Ok(PinSnapshot::empty())),
            HostInner::Threaded(executor) => match executor.submit_step(epoch, inputs, cycles) {
                Ok(()) => SubmitOutcome::Waiting,
                Err(error) => SubmitOutcome::Ready(Err(error)),
            },
        }
    }

    /// Collect the result of a step dispatched with [`DeviceHost::submit_step`]
    /// 收集 [`DeviceHost::submit_step`] 派发的步进结果
    pub(crate) fn collect_step(&mut self, epoch: u64) -> Result<PinSnapshot, ExecutorError> {
        match &mut self.inner {
            HostInner::Inline(_) => Ok(PinSnapshot::empty()),
            HostInner::Threaded(executor) => executor.collect_step(epoch),
        }
    }

    /// Configure the plugin through the mailbox (threaded) or directly (inline)
    /// 通过邮箱（线程化）或直接（内联）配置插件
    pub(crate) fn configure(
        &mut self,
        id: &str,
        config: &serde_json::Value,
    ) -> Result<(), ConfigApplyError> {
        match &mut self.inner {
            HostInner::Inline(Some(plugin)) => plugin
                .apply_config(config)
                .map_err(ConfigApplyError::Invalid),
            HostInner::Inline(None) => Err(ConfigApplyError::Executor(ExecutorError::Stopped {
                id: id.to_string(),
            })),
            HostInner::Threaded(executor) => executor
                .configure(config)
                .map_err(ConfigApplyError::Executor),
        }
    }

    /// Query the plugin state / 查询插件状态
    pub(crate) fn state_snapshot(&mut self) -> serde_json::Value {
        match &mut self.inner {
            HostInner::Inline(Some(plugin)) => plugin.state_snapshot(),
            HostInner::Inline(None) => serde_json::json!({
                "error": "device stopped / 设备已停止",
            }),
            HostInner::Threaded(executor) => match executor.query_state() {
                Ok(value) => value,
                Err(error) => serde_json::json!({
                    "error": error.to_string(),
                    "degraded": executor.is_degraded(),
                }),
            },
        }
    }

    /// Shut the plugin down without giving up ownership (inline keeps the box)
    /// 关闭插件但不交出所有权（内联模式保留 box）
    pub(crate) fn shutdown_in_place(&mut self) {
        match &mut self.inner {
            HostInner::Inline(Some(plugin)) => plugin.shutdown(),
            HostInner::Inline(None) => {}
            HostInner::Threaded(executor) => {
                let _ = executor.shutdown();
            }
        }
    }

    /// Stop this device and hand the plugin back for ordered destruction
    /// 停止设备并交还插件以便按序销毁
    pub(crate) fn stop(&mut self) -> StopOutcome {
        match &mut self.inner {
            HostInner::Inline(slot) => {
                let mut plugin = slot.take();
                if let Some(plugin_ref) = plugin.as_mut() {
                    plugin_ref.shutdown();
                }
                StopOutcome {
                    plugin,
                    detached: false,
                }
            }
            HostInner::Threaded(executor) => {
                let plugin = executor.shutdown();
                StopOutcome {
                    plugin,
                    detached: executor.is_detached(),
                }
            }
        }
    }

    /// Stop only a threaded host; inline hosts are left untouched
    /// 仅停止线程化宿主；内联宿主不受影响
    pub(crate) fn stop_thread(&mut self) -> Option<StopOutcome> {
        match &mut self.inner {
            HostInner::Inline(_) => None,
            HostInner::Threaded(executor) => {
                // 已经停止的执行器不再计入回收数量（保证幂等）。
                // An already stopped executor is not counted again (idempotent).
                if !executor.is_alive() {
                    return None;
                }
                let plugin = executor.shutdown();
                Some(StopOutcome {
                    plugin,
                    detached: executor.is_detached(),
                })
            }
        }
    }
}
