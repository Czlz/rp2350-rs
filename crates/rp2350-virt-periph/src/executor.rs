//! Threaded plugin executors — deterministic, fixed-latency device mailbox
//! 线程化插件执行器 — 确定性、固定延迟的设备邮箱
//!
//! # Model / 模型
//!
//! In threaded mode every device plugin is **owned by its own worker thread**
//! ([`crate::DeviceExecutor`]). The simulation thread never calls into a plugin;
//! it sends [`crate::Command`]s and waits for exactly one [`crate::Response`]
//! with a bounded timeout. One executor per device (instead of one executor for
//! the whole manager) is deliberate: a hung or panicking device can only stall
//! itself, the other devices keep advancing at their own epochs, and their
//! outputs never interleave.
//! 线程化模式下每个设备插件由**各自的**工作线程独占（[`crate::DeviceExecutor`]）。
//! 模拟线程从不直接调用插件，只发送 [`crate::Command`] 并在有界超时内等待恰好一个
//! [`crate::Response`]。为每个设备使用独立执行器（而不是整个管理器共用一个）是
//! 刻意的：卡死或 panic 的设备只会拖住自己，其它设备按各自 epoch 继续推进，输出
//! 互不串扰。
//!
//! # Delay semantics / 延迟语义
//!
//! The device sees the pin levels sampled at `epoch` and its outputs become
//! effective at `epoch + latency_ticks` (default 1 peripheral tick). The delay is
//! kept in [`PublishSchedule`] on the simulation thread and is measured in
//! simulation epochs, never in host time, so the same input sequence always
//! produces the same output sequence.
//! 设备看到的是 `epoch` 采样的引脚电平，其输出在 `epoch + latency_ticks`（默认
//! 1 个外设 tick）生效。延迟由模拟线程上的 [`PublishSchedule`] 维护，以仿真 epoch
//! 计量而非宿主时间，因此同一输入序列必然产生同一输出序列。
//!
//! # Failure handling / 失败处理
//!
//! A timeout or a panic latches the device into the **high-Z** state: it stops
//! contributing output levels and keeps reporting the same diagnostic line. The
//! simulation continues. Rust cannot kill a thread, so a worker that never
//! answers is detached; its library is intentionally leaked so the plugin vtable
//! is never unmapped while the stuck thread still references it.
//! 超时或 panic 会把设备锁定为**高阻**：它不再贡献输出电平，并且始终报告同一
//! 条诊断。仿真继续运行。Rust 无法终止线程，因此永不响应的线程会被分离；其库会
//! 被有意泄漏，避免卡住的线程仍引用插件时 vtable 被解除映射。
//!
//! # Panic limitation / panic 限制
//!
//! `catch_unwind` only covers Rust-level unwinding. A `panic = "abort"` plugin,
//! or an unwind that crosses a foreign (`extern "C"`) frame, aborts the process
//! and cannot be isolated. 插件的 `panic = "abort"`，或展开穿过外部
//! （`extern "C"`）帧时，会终止进程且无法隔离。
//!
//! # Modules / 模块划分
//!
//! `executor` holds the protocol types and the publish schedule, `executor_host`
//! the worker handle, `executor_device` the per-device inline/threaded host and
//! `executor_manager` the manager-facing step API.
//! `executor` 存放协议类型与发布队列，`executor_host` 存放工作线程句柄，
//! `executor_device` 存放单设备的内联/线程化宿主，`executor_manager` 存放面向
//! 管理器的步进 API。

// 类型名沿用项目命名约定；文档引用使用内部标识符。
// Type names follow project conventions; doc references use internal identifiers.
#![allow(clippy::doc_markdown, clippy::module_name_repetitions)]

use std::collections::BTreeMap;
use std::time::Duration;

use rp2350_core::gpio::GpioPin;

use super::device::PortDirection;
use super::manager::DeviceId;

/// Number of GPIO pins a [`PinSnapshot`] can represent
/// [`PinSnapshot`] 可表示的 GPIO 引脚数量
///
/// RP2350 exposes GPIO0..47, so a 64-bit mask covers the whole chip with room to
/// spare. Wiring entries above pin 63 are treated as unconnected.
/// RP2350 提供 GPIO0..47，64 位掩码足以覆盖整颗芯片。接线中高于 63 的引脚视为未连接。
pub const PIN_SNAPSHOT_PINS: usize = 64;

/// Default propagation delay of the threaded path, in peripheral ticks
/// 线程化路径的默认传播延迟（以“外设 tick”为单位）
pub const DEFAULT_LATENCY_TICKS: u64 = 1;

/// Default upper bound of one mailbox round trip / 单次邮箱往返的默认上限
///
/// Five seconds is deliberately far above any legitimate plugin step: the
/// bound exists to detect a **stuck** worker, not to police host scheduling.
/// A shorter value turns a busy `cargo test` / concurrent build into a false
/// high-Z latch, so tests that verify degradation pass an explicit short
/// timeout through [`ExecutorConfig::new`].
/// 5 秒远高于任何合法插件步进：该上限用于发现**卡死**的工作线程，而不是衡量宿主
/// 调度。更短的值会让繁忙的 `cargo test`/并行构建误判为高阻；需要验证降级的测试
/// 应通过 [`ExecutorConfig::new`] 显式传入短超时。
pub const DEFAULT_STEP_TIMEOUT: Duration = Duration::from_secs(5);

/// GPIO pin levels at one simulation epoch, encoded as a 64-bit mask
/// 某个仿真 epoch 的 GPIO 引脚电平（64 位掩码编码）
///
/// Bit `n` is the level of GPIO `n` (`true` = high). Pins `>= 64` are not
/// representable and are reported as low; RP2350 does not have them.
/// 第 `n` 位是 GPIO `n` 的电平（`true` = 高）。`>= 64` 的引脚无法表示并读作低电平；
/// RP2350 没有这些引脚。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PinSnapshot {
    /// One bit per GPIO pin / 每个 GPIO 引脚一位
    bits: u64,
}

impl PinSnapshot {
    /// Empty snapshot (every represented pin low) / 空快照（所有可表示引脚为低）
    #[must_use]
    pub fn empty() -> Self {
        Self { bits: 0 }
    }

    /// Build a snapshot from a raw mask / 由原始掩码构造快照
    #[must_use]
    pub fn from_bits(bits: u64) -> Self {
        Self { bits }
    }

    /// Sample the current levels of `pins` / 采样 `pins` 的当前电平
    #[must_use]
    pub fn from_gpio_pins(pins: &[GpioPin]) -> Self {
        let mut snapshot = Self::empty();
        for (index, pin) in pins.iter().take(PIN_SNAPSHOT_PINS).enumerate() {
            // `index < 64` 由上方的 `take` 保证，转换不会截断。
            // `index < 64` is guaranteed by `take` above, so the cast cannot truncate.
            snapshot.set(u8::try_from(index).unwrap_or(u8::MAX), pin.level());
        }
        snapshot
    }

    /// Level of GPIO `pin`; `false` outside the representable range
    /// GPIO `pin` 的电平；超出可表示范围时为 `false`
    #[must_use]
    pub fn get(self, pin: u8) -> bool {
        u32::from(pin) < u32::try_from(PIN_SNAPSHOT_PINS).unwrap_or(u32::MAX)
            && (self.bits >> u32::from(pin)) & 1 == 1
    }

    /// Set the level of GPIO `pin`; ignored outside the representable range
    /// 设置 GPIO `pin` 的电平；超出可表示范围时忽略
    pub fn set(&mut self, pin: u8, level: bool) {
        if u32::from(pin) >= u32::try_from(PIN_SNAPSHOT_PINS).unwrap_or(u32::MAX) {
            return;
        }
        if level {
            self.bits |= 1 << u32::from(pin);
        } else {
            self.bits &= !(1 << u32::from(pin));
        }
    }

    /// Raw bit mask / 原始位掩码
    #[must_use]
    pub fn bits(self) -> u64 {
        self.bits
    }

    /// Are all represented pins low? / 所有可表示引脚是否为低？
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.bits == 0
    }

    /// Wired-OR combination of two snapshots / 两个快照的线或组合
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        Self {
            bits: self.bits | other.bits,
        }
    }

    /// Write every masked pin into the GPIO model as an external input
    /// 将掩码覆盖的引脚作为外部输入写入 GPIO 模型
    ///
    /// Only pins selected by `mask` are written, so unconnected pins keep the
    /// level driven by the host. 仅写入 `mask` 选中的引脚，未连接引脚保持宿主驱动
    /// 的电平。
    pub fn apply_masked(self, mask: Self, pins: &mut [GpioPin]) {
        for (index, pin) in pins.iter_mut().take(PIN_SNAPSHOT_PINS).enumerate() {
            // `index < 64` 由 `take` 保证，转换安全。
            // `index < 64` is guaranteed by `take`, so the cast is safe.
            let Ok(number) = u8::try_from(index) else {
                break;
            };
            if mask.get(number) {
                pin.set_input(self.get(number));
            }
        }
    }
}

/// One device port bound to an optional GPIO pin
/// 一个设备端口及其可选的 GPIO 引脚绑定
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortBinding {
    /// Port index on the device / 设备上的端口索引
    pub port_index: usize,
    /// Signal direction of the port / 端口信号方向
    pub direction: PortDirection,
    /// Wired GPIO pin, `None` when the port is unwired / 已接线引脚；未接线为 `None`
    pub gpio_pin: Option<u8>,
}

impl PortBinding {
    /// Create a binding / 创建绑定
    #[must_use]
    pub fn new(port_index: usize, direction: PortDirection, gpio_pin: Option<u8>) -> Self {
        Self {
            port_index,
            direction,
            gpio_pin,
        }
    }

    /// Pin usable inside a [`PinSnapshot`], `None` when unwired or out of range
    /// 可在 [`PinSnapshot`] 中使用的引脚；未接线或超出范围时为 `None`
    #[must_use]
    pub fn effective_pin(self) -> Option<u8> {
        self.gpio_pin
            .filter(|pin| usize::from(*pin) < PIN_SNAPSHOT_PINS)
    }
}

/// Configuration of one plugin executor / 单个插件执行器的配置
#[derive(Debug, Clone, Copy)]
pub struct ExecutorConfig {
    /// Propagation delay in peripheral ticks (clamped to `>= 1`)
    /// 传播延迟（外设 tick，最小为 1）
    pub latency_ticks: u64,
    /// Upper bound of one mailbox round trip / 单次邮箱往返的上限
    ///
    /// Only a worker that exceeds this bound is latched high-Z. Use
    /// [`DEFAULT_STEP_TIMEOUT`] outside tests; degradation tests pass a short
    /// value explicitly. 只有超过该上限的工作线程才会被锁定为高阻。非测试场景请用
    /// [`DEFAULT_STEP_TIMEOUT`]；降级测试需显式传入短超时。
    pub step_timeout: Duration,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            latency_ticks: DEFAULT_LATENCY_TICKS,
            step_timeout: DEFAULT_STEP_TIMEOUT,
        }
    }
}

impl ExecutorConfig {
    /// Create a configuration; `latency_ticks` is clamped to at least 1
    /// 创建配置；`latency_ticks` 至少为 1
    #[must_use]
    pub fn new(latency_ticks: u64, step_timeout: Duration) -> Self {
        Self {
            latency_ticks: latency_ticks.max(1),
            step_timeout,
        }
    }
}

/// Fixed-latency publish queue measured in simulation epochs
/// 以仿真 epoch 计量的固定延迟发布队列
///
/// Outputs produced while stepping `epoch` are stored under `epoch + latency`
/// and released by [`PublishSchedule::take_effective`]. Entries whose epoch was
/// skipped (the host stepped further ahead) are released together with
/// wired-OR semantics, so no input is ever merged and no output is lost.
/// 在步进 `epoch` 时产生的输出存放在 `epoch + latency` 下，由
/// [`PublishSchedule::take_effective`] 释放。若宿主跳过了某些 epoch，被跳过的条目
/// 会在下一次调用时按线或语义一并释放，因此既不合并输入也不丢失输出。
#[derive(Debug, Clone)]
pub struct PublishSchedule {
    /// Configured latency in epochs / 配置的延迟（epoch）
    latency: u64,
    /// Due epoch → wired-OR output mask / 生效 epoch → 线或输出掩码
    pending: BTreeMap<u64, u64>,
}

impl PublishSchedule {
    /// Create a schedule; `latency_ticks` is clamped to at least 1
    /// 创建调度器；`latency_ticks` 至少为 1
    #[must_use]
    pub fn new(latency_ticks: u64) -> Self {
        Self {
            latency: latency_ticks.max(1),
            pending: BTreeMap::new(),
        }
    }

    /// Configured latency in epochs / 配置的延迟（epoch）
    #[must_use]
    pub fn latency(&self) -> u64 {
        self.latency
    }

    /// Schedule `outputs` for `epoch + latency` / 将 `outputs` 排到 `epoch + latency`
    pub fn publish(&mut self, epoch: u64, outputs: PinSnapshot) {
        let due = epoch.saturating_add(self.latency);
        let slot = self.pending.entry(due).or_insert(0);
        *slot |= outputs.bits();
    }

    /// Release every output due at or before `epoch`; `None` when nothing was due
    /// 释放所有在 `epoch` 或之前生效的输出；无到期条目时返回 `None`
    #[must_use]
    pub fn take_effective(&mut self, epoch: u64) -> Option<PinSnapshot> {
        let due: Vec<u64> = self.pending.range(..=epoch).map(|(due, _)| *due).collect();
        if due.is_empty() {
            return None;
        }
        let mut combined = 0u64;
        for key in due {
            if let Some(bits) = self.pending.remove(&key) {
                combined |= bits;
            }
        }
        Some(PinSnapshot::from_bits(combined))
    }

    /// Number of epochs still waiting for release / 仍在等待释放的 epoch 数量
    #[must_use]
    pub fn pending(&self) -> usize {
        self.pending.len()
    }
}

/// Result of one simulation step / 单次仿真步进的结果
#[derive(Debug, Clone, Default)]
pub struct StepReport {
    /// Epoch that was stepped / 本次步进的 epoch
    pub epoch: u64,
    /// Outputs that became effective at this epoch (`epoch - latency` inputs)
    /// 在本 epoch 生效的输出（来自 `epoch - latency` 的输入）
    pub effective: Option<PinSnapshot>,
    /// Per-device outputs produced now, effective at `epoch + latency`
    /// 本次产生、将在 `epoch + latency` 生效的各设备输出
    ///
    /// Holds **one entry per stepped device** in ID order. A device that timed
    /// out, panicked or was already stopped during this step is high-Z: it
    /// appears here with [`PinSnapshot::empty`] (it drives no pin) and is also
    /// listed in [`StepReport::degraded`]. The entry is never silently dropped.
    /// 按 ID 顺序为**每台参与步进的设备**保留一个条目。本步超时、panic 或已停止的
    /// 设备处于高阻：它在此处以 [`PinSnapshot::empty`] 出现（不驱动任何引脚），并同时
    /// 列入 [`StepReport::degraded`]，绝不会静默消失。
    pub produced: Vec<(DeviceId, PinSnapshot)>,
    /// Devices that failed this step and are now high-Z, sorted by ID
    /// 本次步进失败并已处于高阻的设备（按 ID 排序）
    pub degraded: Vec<DeviceId>,
}

/// Protocol errors of [`super::DeviceManager::step`]
/// [`super::DeviceManager::step`] 的协议错误
#[derive(Debug, thiserror::Error)]
pub enum StepError {
    /// The requested epoch is not strictly increasing
    /// 请求的 epoch 未严格递增
    #[error(
        "epoch {requested} is not after the last stepped epoch {last} / \
         epoch {requested} 未大于上一个步进 epoch {last}"
    )]
    NonMonotonicEpoch {
        /// Last epoch stepped successfully / 上一次成功步进的 epoch
        last: u64,
        /// Requested epoch / 请求的 epoch
        requested: u64,
    },
}

/// Errors returned by one plugin executor / 单个插件执行器返回的错误
#[derive(Debug, thiserror::Error)]
pub enum ExecutorError {
    /// The device is latched high-Z after a timeout or panic
    /// 设备因超时或 panic 被锁定为高阻
    #[error("plugin '{id}' is high-Z: {reason} / 插件 '{id}' 处于高阻: {reason}")]
    Degraded {
        /// Device ID / 设备 ID
        id: DeviceId,
        /// Stable degradation reason / 稳定的降级原因
        reason: String,
    },
    /// The worker did not answer within the configured timeout
    /// 工作线程未在配置超时内响应
    #[error(
        "plugin '{id}' timed out after {timeout_ms} ms during {op} / \
         插件 '{id}' 在 {op} 阶段超时（{timeout_ms} ms）"
    )]
    Timeout {
        /// Device ID / 设备 ID
        id: DeviceId,
        /// Operation name / 操作名
        op: &'static str,
        /// Timeout in milliseconds / 超时毫秒数
        timeout_ms: u128,
    },
    /// The executor has no running worker / 执行器没有运行中的工作线程
    #[error("plugin '{id}' worker is not running / 插件 '{id}' 的工作线程未运行")]
    Stopped {
        /// Device ID / 设备 ID
        id: DeviceId,
    },
    /// The mailbox channel is closed / 邮箱通道已关闭
    #[error("plugin '{id}' mailbox is closed / 插件 '{id}' 的邮箱已关闭")]
    Disconnected {
        /// Device ID / 设备 ID
        id: DeviceId,
    },
    /// The plugin panicked while handling a command
    /// 插件处理命令时 panic
    #[error("plugin '{id}' panicked: {message} / 插件 '{id}' 发生 panic: {message}")]
    Panic {
        /// Device ID / 设备 ID
        id: DeviceId,
        /// Panic message / panic 消息
        message: String,
    },
    /// The plugin rejected a command / 插件拒绝了命令
    #[error("plugin '{id}' rejected {op}: {message} / 插件 '{id}' 拒绝 {op}: {message}")]
    Plugin {
        /// Device ID / 设备 ID
        id: DeviceId,
        /// Operation name / 操作名
        op: &'static str,
        /// Error message / 错误消息
        message: String,
    },
    /// The worker answered with an unexpected response
    /// 工作线程返回了非预期的响应
    #[error("plugin '{id}' answered {op} with an unexpected response / 插件响应与请求不匹配")]
    Protocol {
        /// Device ID / 设备 ID
        id: DeviceId,
        /// Operation name / 操作名
        op: &'static str,
    },
}
