//! Error types for the plugin system
//! 插件系统的错误类型

/// Plugin lifecycle errors
/// 插件生命周期错误
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    /// Plugin initialization failed / 插件初始化失败
    #[error("Plugin initialization failed: {0} / 插件初始化失败: {0}")]
    InitFailed(String),

    /// Plugin already shut down / 插件已关闭
    #[error("Plugin is already shut down / 插件已经关闭")]
    AlreadyShutdown,

    /// Invalid plugin configuration / 无效的插件配置
    #[error("Invalid plugin configuration: {0} / 无效的插件配置: {0}")]
    InvalidConfig(String),

    /// A plugin operation timed out / 插件操作超时
    #[error("Plugin operation timed out: {0} / 插件操作超时: {0}")]
    Timeout(String),

    /// Internal plugin error / 插件内部错误
    #[error("Internal plugin error: {0} / 插件内部错误: {0}")]
    Internal(String),
}
