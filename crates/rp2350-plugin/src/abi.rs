//! Plugin ABI constants shared by the host loader and loadable plugins
//! 主机加载器与可加载插件共享的 ABI 常量

/// ABI version implemented by the host and expected from plugins
/// 主机实现、并要求插件匹配的 ABI 版本
pub const ABI_VERSION: u32 = 1;

/// Exported symbol returning the plugin ABI version (`fn() -> u32`)
/// 返回插件 ABI 版本的导出符号（`fn() -> u32`）
pub const SYMBOL_VERSION: &str = "rp2350_plugin_version";

/// Exported symbol reporting whether the plugin is a device (`fn() -> u32`)
/// 报告插件是否为设备插件的导出符号（`fn() -> u32`）
pub const SYMBOL_IS_DEVICE: &str = "rp2350_plugin_is_device";

/// Exported symbol creating a device plugin (`fn() -> *mut dyn DevicePlugin`)
/// 创建设备插件的导出符号（`fn() -> *mut dyn DevicePlugin`）
pub const SYMBOL_DEVICE_CREATE: &str = "rp2350_device_plugin_create";
