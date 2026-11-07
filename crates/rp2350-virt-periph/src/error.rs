//! Error types for the virtual device manager
//! 虚拟设备管理器的错误类型

/// Device configuration errors / 设备配置错误
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A required field is missing / 缺少必填字段
    #[error("Missing required config field: {0} / 缺少必填配置字段: {0}")]
    MissingField(String),

    /// A field has an invalid type / 字段类型无效
    #[error("Invalid type for field {field}: expected {expected}, got {actual} / 字段 {field} 类型无效: 期望 {expected}，实际 {actual}")]
    InvalidType {
        /// Field name / 字段名
        field: String,
        /// Expected type / 期望类型
        expected: String,
        /// Actual type / 实际类型
        actual: String,
    },

    /// A field value is out of range / 字段值超出范围
    #[error("Value out of range for {field}: {message} / {field} 的值超出范围: {message}")]
    OutOfRange {
        /// Field name / 字段名
        field: String,
        /// Error message / 错误消息
        message: String,
    },
}

/// Device management errors / 设备管理错误
#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    /// Device not found / 设备未找到
    #[error("Device not found: {0} / 设备未找到: {0}")]
    NotFound(String),

    /// Device already exists / 设备已存在
    #[error("Device already exists: {0} / 设备已存在: {0}")]
    AlreadyExists(String),

    /// Device initialization failed / 设备初始化失败
    #[error("Device init failed: {0} / 设备初始化失败: {0}")]
    InitFailed(String),

    /// Invalid wiring configuration / 无效的接线配置
    #[error("Invalid wiring: {0} / 无效接线: {0}")]
    InvalidWiring(String),

    /// Plugin loading failed / 插件加载失败
    #[error("Plugin load failed: {0} / 插件加载失败: {0}")]
    PluginLoadFailed(String),

    /// Port index out of bounds / 端口索引越界
    #[error("Port index out of bounds: {0} / 端口索引越界: {0}")]
    PortIndexOutOfBounds(usize),
}