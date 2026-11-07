//! Error types for rp2350-core
//! rp2350-core 的错误类型

use super::types::Address;

/// Bus error classification
/// 总线错误分类
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusErrorKind {
    /// No device mapped at this address / 该地址未映射到任何设备
    NoDevice,
    /// Address is not aligned for the access width / 地址未按访问宽度对齐
    Unaligned,
    /// Access denied (e.g. write to read-only region) / 访问被拒绝（如写入只读区域）
    AccessDenied,
    /// Device returned an error / 设备返回错误
    DeviceError,
}

impl std::fmt::Display for BusErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDevice => write!(f, "NoDevice"),
            Self::Unaligned => write!(f, "Unaligned"),
            Self::AccessDenied => write!(f, "AccessDenied"),
            Self::DeviceError => write!(f, "DeviceError"),
        }
    }
}

/// Core-level errors
/// 核心级错误
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// Bus access error / 总线访问错误
    #[error("Bus error at address {addr:#010X}: {kind} / 总线错误 {addr:#010X}: {kind}")]
    BusError {
        /// Failing address / 出错地址
        addr: Address,
        /// Error kind / 错误类型
        kind: BusErrorKind,
    },

    /// Invalid GPIO configuration / 无效的 GPIO 配置
    #[error("Invalid GPIO configuration: {0} / 无效的 GPIO 配置: {0}")]
    InvalidGpio(String),

    /// Invalid IRQ number / 无效的中断号
    #[error("IRQ number out of range: {0} / 中断号超出范围: {0}")]
    InvalidIrq(u16),

    /// Clock configuration error / 时钟配置错误
    #[error("Clock configuration error: {0} / 时钟配置错误: {0}")]
    ClockError(String),
}