//! `DevicePlugin` trait — extends `RpPlugin` with GPIO ports, config, and optional UI
//! `DevicePlugin` trait — 扩展 `RpPlugin`，增加 GPIO 端口、配置和可选 UI

use rp2350_plugin::RpPlugin;

use super::config::ConfigSchema;
use super::error::ConfigError;

/// A single I/O port on a virtual device
/// 虚拟设备上的单个 I/O 端口
#[derive(Debug, Clone)]
pub struct DevicePort {
    /// Port name (e.g. "MOSI", "SCK", "LED") / 端口名称
    pub name: String,
    /// Port direction / 端口方向
    pub direction: PortDirection,
    /// Current signal level / 当前信号电平
    pub level: bool,
    /// Port description for UI tooltip / 端口描述（UI 提示用）
    pub description: String,
}

impl DevicePort {
    /// Create a new input port / 创建输入端口
    #[must_use]
    pub fn input(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            direction: PortDirection::Input,
            level: false,
            description: description.into(),
        }
    }

    /// Create a new output port / 创建输出端口
    #[must_use]
    pub fn output(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            direction: PortDirection::Output,
            level: false,
            description: description.into(),
        }
    }

    /// Create a bidirectional port / 创建双向端口
    #[must_use]
    pub fn bidirectional(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            direction: PortDirection::Bidirectional,
            level: false,
            description: description.into(),
        }
    }
}

/// Port signal direction / 端口信号方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortDirection {
    /// Signal flows into the device from GPIO pin
    /// 信号从 GPIO 引脚流入设备
    Input,
    /// Signal flows out of the device to GPIO pin
    /// 信号从设备流出到 GPIO 引脚
    Output,
    /// Signal can flow in either direction
    /// 信号可双向流动
    Bidirectional,
}

/// `DevicePlugin` extends `RpPlugin` with GPIO port management,
/// optional configuration, and optional GUI rendering.
/// `DevicePlugin` 扩展 `RpPlugin`，增加 GPIO 端口管理、
/// 可选配置和可选 GUI 渲染。
///
/// # Headless Mode / 无头模式
///
/// All device logic must work **without** `render_ui()` ever being called.
/// The `state_snapshot()` method is the sole authoritative interface used
/// by MCP and headless mode. GUI is purely optional visualization.
/// 所有设备逻辑必须能在 **不调用** `render_ui()` 的情况下正常工作。
/// `state_snapshot()` 是 MCP 和无头模式使用的唯一权威接口。GUI 纯粹是可选的视觉层。
pub trait DevicePlugin: RpPlugin {
    /// Return the list of I/O ports this device exposes
    /// 返回此设备暴露的 I/O 端口列表
    fn ports(&self) -> &[DevicePort];

    /// Return a mutable reference to the port list
    /// 返回端口列表的可变引用
    fn ports_mut(&mut self) -> &mut [DevicePort];

    /// Called when a GPIO pin wired to this port changes level
    /// 当连接到该端口的 GPIO 引脚电平变化时调用
    fn on_gpio_change(&mut self, port_index: usize, level: bool);

    /// Return the configuration schema (JSON Schema), if configurable
    /// 返回配置模式（JSON Schema），如果可配置的话
    fn config_schema(&self) -> Option<ConfigSchema> {
        None
    }

    /// Apply a new configuration from JSON
    /// 应用来自 JSON 的新配置
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if the configuration is invalid.
    fn apply_config(&mut self, _config: &serde_json::Value) -> Result<(), ConfigError> {
        Ok(())
    }

    /// Does this device have a GUI? / 此设备是否有 GUI？
    ///
    /// If `false`, `render_ui()` will never be called.
    /// 如果为 `false`，`render_ui()` 永远不会被调用。
    fn has_ui(&self) -> bool {
        false
    }

    /// Render the device's GUI panel (only called if `has_ui()` returns `true`)
    /// 渲染设备的 GUI 面板（仅当 `has_ui()` 返回 `true` 时调用）
    ///
    /// Available only with the `gui` feature enabled.
    #[cfg(feature = "gui")]
    fn render_ui(&mut self, _ui: &mut egui::Ui) {}
}
