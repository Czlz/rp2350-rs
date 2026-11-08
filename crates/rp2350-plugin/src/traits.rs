//! `RpPlugin` trait — the base contract all plugins must implement
//! `RpPlugin` trait — 所有插件必须实现的基础契约

use super::context::PluginContext;
use super::error::PluginError;
use super::metadata::PluginMetadata;

/// The base plugin contract. Every plugin (pure logic or device) implements this.
/// 基础插件契约。每个插件（纯逻辑或设备）都实现此 trait。
///
/// # Lifecycle / 生命周期
///
/// ```text
/// 1. metadata()     — host queries plugin identity
/// 2. init()         — host initializes the plugin
/// 3. tick() × N     — host advances simulation each cycle
/// 4. shutdown()     — host shuts down the plugin
/// ```
///
/// # Headless Guarantee / 无头保证
///
/// This trait has **zero** dependency on egui or any rendering library.
/// All methods must work correctly in headless mode (MCP, CLI, CI).
/// 本 trait **零**依赖 egui 或任何渲染库。
/// 所有方法必须能在无头模式（MCP、CLI、CI）中正常工作。
pub trait RpPlugin: Send + Sync {
    /// Return plugin metadata for discovery and display
    /// 返回插件元数据用于发现和显示
    fn metadata(&self) -> &PluginMetadata;

    /// Initialize the plugin with the given host context
    /// 使用给定主机上下文初始化插件
    ///
    /// Called once before any `tick()` calls.
    /// 在任何 `tick()` 调用之前调用一次。
    ///
    /// # Errors
    ///
    /// Returns `PluginError` if initialization fails.
    fn init(&mut self, ctx: &mut dyn PluginContext) -> Result<(), PluginError>;

    /// Advance the plugin's internal state by `cycles` simulation cycles
    /// 将插件的内部状态推进 `cycles` 个仿真周期
    fn tick(&mut self, ctx: &mut dyn PluginContext, cycles: u64);

    /// Shut down the plugin and release resources
    /// 关闭插件并释放资源
    ///
    /// Called once at the end of the plugin's lifecycle.
    /// 在插件生命周期结束时调用一次。
    fn shutdown(&mut self);

    /// If this plugin acts as a bus device, return a mutable reference
    /// 如果此插件充当总线设备，返回可变引用
    ///
    /// Returns `None` by default — only memory-mapped peripherals override this.
    /// 默认返回 `None` — 仅内存映射外设覆写此方法。
    fn as_bus_device(&mut self) -> Option<&mut dyn crate::context::BusDevice> {
        None
    }

    /// Return a complete plugin state as JSON (for MCP / headless query)
    /// 返回完整插件状态（JSON 格式，用于 MCP / 无头查询）
    ///
    /// Default: empty object. Override for meaningful data.
    /// 默认：空对象。覆写以提供有意义的数据。
    fn state_snapshot(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}
