//! Minimal debug-target abstraction for the GDB RSP server
//! GDB RSP 服务器的最小调试目标抽象
//!
//! The RSP transport speaks GDB register numbers, byte-addressed memory,
//! resume/step, instruction breakpoints, and stop replies. [`GdbTarget`]
//! captures exactly that surface so the server can drive any CPU backend
//! (Cortex-M33 today, other cores later) without knowing its concrete type.
//! RSP 传输层使用 GDB 寄存器编号、按字节寻址的内存、继续/单步、指令断点
//! 和停止回复。[`GdbTarget`] 精确建模这组能力，使服务器无需了解具体类型
//! 即可驱动任意 CPU 后端（当前为 Cortex-M33，后续可扩展其他核心）。

use rp2350_core::error::CoreError;
use rp2350_cpu::CpuState;

/// GDB register numbers for the ARMv8-M register file
/// ARMv8-M 寄存器文件的 GDB 寄存器编号
///
/// GDB's `g`/`p`/`P` packets address registers numerically. The ARM target
/// description used here exposes R0-R15 plus XPSR, which matches the
/// `org.gnu.gdb.arm.m-system` feature set.
/// GDB 的 `g`/`p`/`P` 包按编号访问寄存器。这里使用的 ARM 目标描述暴露
/// R0-R15 加 XPSR，与 `org.gnu.gdb.arm.m-system` 特性集一致。
pub mod regs {
    /// Register R0 / 寄存器 R0
    pub const R0: u32 = 0;
    /// Register R1 / 寄存器 R1
    pub const R1: u32 = 1;
    /// Register R2 / 寄存器 R2
    pub const R2: u32 = 2;
    /// Register R3 / 寄存器 R3
    pub const R3: u32 = 3;
    /// Register R4 / 寄存器 R4
    pub const R4: u32 = 4;
    /// Register R5 / 寄存器 R5
    pub const R5: u32 = 5;
    /// Register R6 / 寄存器 R6
    pub const R6: u32 = 6;
    /// Register R7 / 寄存器 R7
    pub const R7: u32 = 7;
    /// Register R8 / 寄存器 R8
    pub const R8: u32 = 8;
    /// Register R9 / 寄存器 R9
    pub const R9: u32 = 9;
    /// Register R10 / 寄存器 R10
    pub const R10: u32 = 10;
    /// Register R11 / 寄存器 R11
    pub const R11: u32 = 11;
    /// Register R12 / 寄存器 R12
    pub const R12: u32 = 12;
    /// Stack pointer (R13) / 栈指针（R13）
    pub const SP: u32 = 13;
    /// Link register (R14) / 链接寄存器（R14）
    pub const LR: u32 = 14;
    /// Program counter (R15) / 程序计数器（R15）
    pub const PC: u32 = 15;
    /// Program status register (XPSR) / 程序状态寄存器（XPSR）
    pub const XPSR: u32 = 16;
    /// Number of registers exposed by this target description
    /// 此目标描述暴露的寄存器数量
    pub const COUNT: u32 = 17;
}

/// How the target should resume execution / 目标应如何恢复执行
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeMode {
    /// Run until a breakpoint, fault, or other stop event
    /// 运行直到断点、故障或其他停止事件
    Continue,
    /// Execute exactly one instruction / 精确执行一条指令
    Step,
}

/// Why the target stopped executing / 目标停止执行的原因
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// An instruction breakpoint was reached / 命中了指令断点
    Breakpoint,
    /// A single-step completed / 单步完成
    SingleStep,
    /// The core faulted while executing / 执行期间核心发生故障
    Fault,
    /// The core entered WFI/WFE sleep / 核心进入 WFI/WFE 休眠
    Sleeping,
    /// The core is halted for another reason / 核心因其他原因暂停
    Halted,
    /// The core is still in reset state / 核心仍处于复位状态
    Reset,
    /// The resume instruction budget was exhausted / 恢复执行的指令预算耗尽
    InstructionLimit,
}

impl StopReason {
    /// GDB signal number to report in a stop reply packet
    /// 停止回复包中应上报的 GDB 信号编号
    ///
    /// Uses SIGTRAP for breakpoints and single steps, SIGSEGV for faults,
    /// and SIGINT for other halts, following GDB's remote protocol conventions.
    /// 断点与单步使用 SIGTRAP，故障使用 SIGSEGV，其他暂停使用 SIGINT，
    /// 符合 GDB 远程协议约定。
    #[must_use]
    pub fn signal(self) -> u8 {
        match self {
            Self::Fault => 11,
            Self::Halted => 2,
            Self::Breakpoint
            | Self::SingleStep
            | Self::Sleeping
            | Self::Reset
            | Self::InstructionLimit => 5,
        }
    }
}

/// Result of a resume request / 恢复执行请求的结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopReply {
    /// Why execution stopped / 执行停止的原因
    pub reason: StopReason,
    /// Program counter at the stop point / 停止时的程序计数器
    pub pc: u32,
}

impl StopReply {
    /// GDB signal number for this stop / 此停止对应的 GDB 信号编号
    #[must_use]
    pub fn signal(self) -> u8 {
        self.reason.signal()
    }
}

/// Observable state of a debug target / 调试目标的可观测状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetStatus {
    /// Core execution state / 核心执行状态
    pub state: CpuState,
    /// Reason for the last stop, if the target has stopped
    /// 最近一次停止的原因（如果目标已停止）
    pub stop_reason: Option<StopReason>,
    /// Current program counter / 当前程序计数器
    pub pc: u32,
    /// Cumulative executed cycles / 累计执行周期数
    pub total_cycles: u64,
}

/// Errors surfaced by a debug target to the RSP layer
/// 调试目标上报给 RSP 层的错误
#[derive(Debug, thiserror::Error)]
pub enum GdbError {
    /// Register number is outside the target's register file
    /// 寄存器编号超出目标的寄存器文件范围
    #[error("invalid GDB register {0} / 无效的 GDB 寄存器 {0}")]
    InvalidRegister(u32),

    /// Bus access failed / 总线访问失败
    #[error("bus access failed: {0} / 总线访问失败: {0}")]
    Bus(#[from] CoreError),

    /// Address arithmetic overflowed the 32-bit address space
    /// 地址运算超出 32 位地址空间
    #[error("address out of range: {0:#010X} / 地址超出范围 {0:#010X}")]
    AddressOutOfRange(u32),
}

/// Debug target contract used by the GDB RSP server
/// GDB RSP 服务器使用的调试目标契约
///
/// Implementors own the CPU and its address bus, so the server only needs a
/// `Box<dyn GdbTarget>` (or a generic target) plus the register description
/// from [`regs`]. Memory is byte-addressed because GDB `m`/`M` packets operate
/// on byte ranges regardless of alignment.
/// 实现者持有 CPU 及其地址总线，因此服务器只需一个 `Box<dyn GdbTarget>`
/// （或泛型目标）以及 [`regs`] 中的寄存器描述。内存按字节寻址，因为 GDB 的
/// `m`/`M` 包不要求对齐。
pub trait GdbTarget: Send {
    /// Number of registers addressable via [`Self::read_register`]
    /// 可通过 [`Self::read_register`] 访问的寄存器数量
    #[must_use]
    fn register_count(&self) -> u32 {
        regs::COUNT
    }

    /// Read a register by GDB register number / 按 GDB 寄存器编号读取寄存器
    ///
    /// # Errors
    /// Returns [`GdbError::InvalidRegister`] for unknown register numbers.
    /// 对未知寄存器编号返回 [`GdbError::InvalidRegister`]。
    fn read_register(&mut self, register: u32) -> Result<u32, GdbError>;

    /// Write a register by GDB register number / 按 GDB 寄存器编号写入寄存器
    ///
    /// # Errors
    /// Returns [`GdbError::InvalidRegister`] for unknown register numbers.
    /// 对未知寄存器编号返回 [`GdbError::InvalidRegister`]。
    fn write_register(&mut self, register: u32, value: u32) -> Result<(), GdbError>;

    /// Read `length` bytes starting at `address` / 从 `address` 起读取 `length` 字节
    ///
    /// # Errors
    /// Returns [`GdbError::Bus`] if any byte is unmapped or unreadable, or
    /// [`GdbError::AddressOutOfRange`] on address overflow.
    /// 任一字节未映射或不可读时返回 [`GdbError::Bus`]；地址溢出时返回
    /// [`GdbError::AddressOutOfRange`]。
    fn read_memory(&mut self, address: u32, length: usize) -> Result<Vec<u8>, GdbError>;

    /// Write `bytes` starting at `address` / 从 `address` 起写入 `bytes`
    ///
    /// # Errors
    /// Returns [`GdbError::Bus`] if any byte is unmapped or unwritable, or
    /// [`GdbError::AddressOutOfRange`] on address overflow.
    /// 任一字节未映射或不可写时返回 [`GdbError::Bus`]；地址溢出时返回
    /// [`GdbError::AddressOutOfRange`]。
    fn write_memory(&mut self, address: u32, bytes: &[u8]) -> Result<(), GdbError>;

    /// Set an instruction breakpoint at `address` / 在 `address` 设置指令断点
    ///
    /// # Errors
    /// Implementations may reject invalid addresses; the default Cortex-M33
    /// adapter accepts any address and detects the breakpoint by PC comparison.
    /// 实现可拒绝无效地址；默认 Cortex-M33 适配器接受任意地址并通过 PC
    /// 比较检测断点。
    fn set_breakpoint(&mut self, address: u32) -> Result<(), GdbError>;

    /// Remove the instruction breakpoint at `address` / 移除 `address` 的指令断点
    ///
    /// # Errors
    /// Implementations may reject invalid addresses; clearing an absent
    /// breakpoint is not an error.
    /// 实现可拒绝无效地址；清除不存在的断点不算错误。
    fn clear_breakpoint(&mut self, address: u32) -> Result<(), GdbError>;

    /// Check whether `address` currently has a breakpoint
    /// 检查 `address` 当前是否设有断点
    #[must_use]
    fn has_breakpoint(&self, address: u32) -> bool;

    /// Resume execution according to `mode` / 按 `mode` 恢复执行
    ///
    /// # Errors
    /// Returns [`GdbError`] if the target cannot start executing (for example
    /// a bus error during the first fetch that is not reported as a stop).
    /// 若目标无法开始执行（例如首次取指的总线错误未以停止回复表达），
    /// 返回 [`GdbError`]。
    fn resume(&mut self, mode: ResumeMode) -> Result<StopReply, GdbError>;

    /// Snapshot the target state and last stop reason
    /// 获取目标状态与最近停止原因的快照
    #[must_use]
    fn status(&self) -> TargetStatus;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_numbers_match_arm_target_description() {
        assert_eq!(regs::R0, 0);
        assert_eq!(regs::R12, 12);
        assert_eq!(regs::SP, 13);
        assert_eq!(regs::LR, 14);
        assert_eq!(regs::PC, 15);
        assert_eq!(regs::XPSR, 16);
        assert_eq!(regs::COUNT, 17);
    }

    #[test]
    fn stop_signals_follow_rsp_conventions() {
        assert_eq!(StopReason::Breakpoint.signal(), 5);
        assert_eq!(StopReason::SingleStep.signal(), 5);
        assert_eq!(StopReason::Fault.signal(), 11);
        assert_eq!(StopReason::Halted.signal(), 2);
        assert_eq!(
            StopReply {
                reason: StopReason::Breakpoint,
                pc: 0x2000_0002,
            }
            .signal(),
            5
        );
    }
}
