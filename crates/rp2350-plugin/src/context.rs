//! `PluginContext` trait — lifecycle callbacks and environment access
//! `PluginContext` trait — 生命周期回调和环境访问

use std::any::Any;

/// Trait for memory-mapped bus devices (from plugin perspective)
/// 内存映射总线设备的 trait（从插件视角）
///
/// This is a minimal interface that plugins can use to interact with the
/// RP2350 address bus. Full `MemoryRegion` capability is available in
/// `rp2350-core` for crate-internal use.
/// 这是插件可用于与 RP2350 地址总线交互的最小接口。完整的
/// `MemoryRegion` 功能在 `rp2350-core` 中供 crate 内部使用。
pub trait BusDevice: Send {
    /// The device's base address on the bus / 设备在总线上的基地址
    fn base_address(&self) -> u32;
    /// The size of the device's address space / 设备地址空间的大小
    fn size(&self) -> usize;
}

/// The context provided to a plugin at initialization and during tick.
/// 插件初始化和 tick 时提供的上下文。
///
/// Provides access to the simulation environment without exposing internals.
/// 提供对仿真环境的访问，但不暴露内部细节。
pub trait PluginContext: Send {
    /// Log a message at `info` level through the host's logging system
    /// 通过主机的日志系统记录 info 级别的消息
    fn log_info(&self, message: &str);

    /// Log a message at `warn` level through the host's logging system
    /// 通过主机的日志系统记录 warn 级别的消息
    fn log_warn(&self, message: &str);

    /// Log a message at `error` level through the host's logging system
    /// 通过主机的日志系统记录 error 级别的消息
    fn log_error(&self, message: &str);

    /// Get the current simulation time in cycles
    /// 获取当前仿真时间（周期数）
    fn current_cycles(&self) -> u64;

    /// Request that the host schedule a callback after `cycles` ticks
    /// 请求主机在 `cycles` 个 tick 后调度一个回调
    fn schedule_callback(&mut self, cycles: u64, tag: &str);

    /// Access a typed service from the host (e.g. an external bus reference)
    /// 从主机访问一个类型化服务（如外部总线引用）
    ///
    /// Returns `None` if the service is not available.
    /// 如果服务不可用则返回 `None`。
    #[doc(hidden)]
    fn get_service_erased(&self) -> Option<&dyn Any>;
}

/// Extension trait for typed service access / 类型化服务访问的扩展 trait
pub trait PluginContextExt: PluginContext {
    /// Access a typed service from the host / 从主机访问类型化服务
    fn get_service<T: Any>(&self) -> Option<&T> {
        self.get_service_erased()
            .and_then(|s| s.downcast_ref::<T>())
    }
}

impl<T: PluginContext + ?Sized> PluginContextExt for T {}
