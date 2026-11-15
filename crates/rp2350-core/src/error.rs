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
    /// Region overlaps an already mapped region / 区域与已映射区域重叠
    Overlap,
    /// Device returned an error / 设备返回错误
    DeviceError,
    /// Retryable access: the device cannot complete it yet / 可重试访问：设备暂无法完成
    ///
    /// Unlike the other kinds this is **not** a fault. A device raises it when the
    /// access must be replayed later (for example a blocking FIFO read while the
    /// receive queue is empty). The access has no side effects, so the CPU may
    /// retry the same instruction once an event makes progress possible; a
    /// frontend must never report it as a bus fault to the guest.
    /// 与其它类型不同，这**不是**故障。设备在访问必须稍后重放时抛出它（例如接收队列
    /// 为空时的阻塞 FIFO 读）。该访问没有副作用，因此 CPU 可以在事件使进展成为可能
    /// 后重试同一条指令；前端绝不能把它当作总线故障报告给客户机。
    Stall,
}

impl std::fmt::Display for BusErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDevice => write!(f, "NoDevice"),
            Self::Unaligned => write!(f, "Unaligned"),
            Self::AccessDenied => write!(f, "AccessDenied"),
            Self::Overlap => write!(f, "Overlap"),
            Self::DeviceError => write!(f, "DeviceError"),
            Self::Stall => write!(f, "Stall"),
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

    /// Firmware image error / 固件映像错误
    #[error("Firmware error: {0} / 固件错误: {0}")]
    FirmwareError(String),

    /// Chip family or capability is not implemented / 芯片家族或能力尚未实现
    #[error("Unsupported chip capability: {0} / 不支持的芯片能力: {0}")]
    UnsupportedChip(String),

    /// Undefined instruction encountered in strict decode mode
    /// 严格解码模式下遇到未定义指令
    #[error(
        "Undefined instruction at PC {pc:#010X}: opcode {opcode:#010X} ({width} bytes) / 未定义指令 PC {pc:#010X}: opcode {opcode:#010X}（{width} 字节）"
    )]
    UndefinedInstruction {
        /// Program counter of the instruction / 指令所在的程序计数器
        pc: Address,
        /// Raw opcode bits / 原始操作码位
        opcode: u32,
        /// Instruction width in bytes / 指令宽度（字节）
        width: u8,
    },
}

impl CoreError {
    /// Whether this error is a retryable bus stall rather than a fault
    /// 该错误是可重试的总线 stall（而非故障）
    ///
    /// This is the single query API for [`BusErrorKind::Stall`]. A scratch flag
    /// on the bus (`take_stall()`-style) would be worse: it could leak a stale
    /// stall from an earlier access into an unrelated instruction. Carrying the
    /// kind inside the error keeps the retry decision local to one access.
    /// 这是 [`BusErrorKind::Stall`] 的唯一查询接口。在总线里放一个「上次是否 stall」
    /// 的粘滞标志（`take_stall()` 之类）更糟：早先访问遗留的标志可能被无关指令读到。
    /// 把类型放在错误里，重试决策只属于这一次访问。
    #[must_use]
    pub fn is_stall(&self) -> bool {
        matches!(
            self,
            Self::BusError {
                kind: BusErrorKind::Stall,
                ..
            }
        )
    }
}
