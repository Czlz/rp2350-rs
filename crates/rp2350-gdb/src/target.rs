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
/// `org.gnu.gdb.arm.m-profile` feature set.
/// GDB 的 `g`/`p`/`P` 包按编号访问寄存器。这里使用的 ARM 目标描述暴露
/// R0-R15 加 XPSR，与 `org.gnu.gdb.arm.m-profile` 特性集一致。
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

/// Kind of watchpoint requested by the debugger / 调试器请求的监视点类型
///
/// `Write` maps to RSP `Z2` (`watch`), `Read` to `Z3` (`rwatch`), and `Access`
/// to `Z4` (`awatch`) which fires on both reads and writes.
/// `Write` 对应 RSP `Z2`（`watch`），`Read` 对应 `Z3`（`rwatch`），`Access` 对应
/// `Z4`（`awatch`，读写均触发）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WatchKind {
    /// Write watchpoint (`Z2`, GDB `watch`) / 写监视点（`Z2`，GDB `watch`）
    Write,
    /// Read watchpoint (`Z3`, GDB `rwatch`) / 读监视点（`Z3`，GDB `rwatch`）
    Read,
    /// Access watchpoint (`Z4`, GDB `awatch`) / 访问监视点（`Z4`，GDB `awatch`）
    Access,
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
    /// A write watchpoint observed a value change / 写监视点观察到值变化
    Watchpoint,
    /// A read watchpoint observed an access / 读监视点观察到访问
    ReadWatch,
    /// An access watchpoint observed a read or write / 访问监视点观察到读或写
    AccessWatch,
}

impl StopReason {
    /// GDB signal number to report in a stop reply packet
    /// 停止回复包中应上报的 GDB 信号编号
    ///
    /// Uses SIGTRAP for breakpoints, single steps, and watchpoints, SIGSEGV for
    /// faults, and SIGINT for other halts, following GDB's remote protocol
    /// conventions.
    /// 断点、单步与监视点使用 SIGTRAP，故障使用 SIGSEGV，其他暂停使用 SIGINT，
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
            | Self::InstructionLimit
            | Self::Watchpoint
            | Self::ReadWatch
            | Self::AccessWatch => 5,
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

    /// The target cannot provide this debugger feature
    /// 目标无法提供该调试器特性
    ///
    /// Reported for capabilities a backend cannot emulate, such as write
    /// watchpoints on a target without safe memory sampling.
    /// 用于后端无法模拟的能力，例如无法安全采样内存的目标上的写监视点。
    #[error("unsupported operation: {0} / 不支持的操作: {0}")]
    Unsupported(&'static str),
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

    /// Static XML target description served by `qXfer:features:read`
    /// 由 `qXfer:features:read` 提供的静态 XML 目标描述
    ///
    /// The default returns [`crate::target_xml::TARGET_XML`], the ARMv8-M
    /// description that matches [`regs`]. Targets with a different register
    /// file (for example a RISC-V core) may return `None`; the RSP layer then
    /// answers the transfer with `E00` instead of advertising a wrong layout.
    /// 默认返回与 [`regs`] 匹配的 ARMv8-M 描述
    /// [`crate::target_xml::TARGET_XML`]。寄存器文件不同的目标（例如 RISC-V
    /// 核心）可返回 `None`，RSP 层随后以 `E00` 应答该传输，而不会声明错误的
    /// 寄存器布局。
    #[must_use]
    fn target_description(&self) -> Option<&'static str> {
        Some(crate::target_xml::TARGET_XML)
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

    /// Whether the target can detect write watchpoints
    /// 目标是否能够检测写监视点
    ///
    /// The RSP layer only advertises `watchpoint+` in `qSupported` when this
    /// returns `true`; the default keeps watchpoint-free targets unchanged.
    /// 仅当本方法返回 `true` 时 RSP 层才会在 `qSupported` 中声明
    /// `watchpoint+`；默认值使未实现监视点的目标保持原行为。
    #[must_use]
    fn supports_watchpoints(&self) -> bool {
        false
    }

    /// Add a write watchpoint covering `length` bytes at `address`
    /// 在 `address` 处添加覆盖 `length` 字节的写监视点
    ///
    /// # Errors
    /// The default implementation reports [`GdbError::Unsupported`] so existing
    /// targets do not need to change.
    /// 默认实现返回 [`GdbError::Unsupported`]，现有目标无需修改。
    fn set_watchpoint(&mut self, _address: u32, _length: usize) -> Result<(), GdbError> {
        Err(GdbError::Unsupported("write watchpoints"))
    }

    /// Remove the write watchpoint at `address` / 移除 `address` 处的写监视点
    ///
    /// # Errors
    /// Defaults to [`GdbError::Unsupported`], mirroring
    /// [`Self::set_watchpoint`]. / 与 [`Self::set_watchpoint`] 一致，默认返回
    /// [`GdbError::Unsupported`]。
    fn clear_watchpoint(&mut self, _address: u32, _length: usize) -> Result<(), GdbError> {
        Err(GdbError::Unsupported("write watchpoints"))
    }

    /// Add a read watchpoint covering `length` bytes at `address`
    /// 在 `address` 处添加覆盖 `length` 字节的读监视点
    ///
    /// # Errors
    /// The default implementation reports [`GdbError::Unsupported`] so existing
    /// targets (such as `CortexM33Target`) need no changes; the RSP layer then
    /// answers `Z3` with `E01` and GDB reports the watchpoint as unsupported.
    /// 默认实现返回 [`GdbError::Unsupported`]，现有目标（如 `CortexM33Target`）
    /// 无需修改；RSP 层随后对 `Z3` 回复 `E01`，GDB 会报告该监视点不受支持。
    fn set_read_watchpoint(&mut self, _address: u32, _length: usize) -> Result<(), GdbError> {
        Err(GdbError::Unsupported("read watchpoints"))
    }

    /// Remove the read watchpoint at `address` / 移除 `address` 处的读监视点
    ///
    /// # Errors
    /// Defaults to [`GdbError::Unsupported`], mirroring
    /// [`Self::set_read_watchpoint`]. / 与 [`Self::set_read_watchpoint`] 一致，
    /// 默认返回 [`GdbError::Unsupported`]。
    fn clear_read_watchpoint(&mut self, _address: u32, _length: usize) -> Result<(), GdbError> {
        Err(GdbError::Unsupported("read watchpoints"))
    }

    /// Add an access watchpoint covering `length` bytes at `address`
    /// 在 `address` 处添加覆盖 `length` 字节的访问监视点
    ///
    /// An access watchpoint fires on reads *and* writes. It is a separate hook
    /// rather than a read plus a write watch so a target can arm one combined
    /// comparator and report [`StopReason::AccessWatch`].
    /// 访问监视点在读与写上都会触发。它独立于读/写监视点，使目标可以安装一个
    /// 组合比较器并上报 [`StopReason::AccessWatch`]。
    ///
    /// # Errors
    /// Defaults to [`GdbError::Unsupported`]; see [`Self::set_read_watchpoint`].
    /// 默认返回 [`GdbError::Unsupported`]；参见 [`Self::set_read_watchpoint`]。
    fn set_access_watchpoint(&mut self, _address: u32, _length: usize) -> Result<(), GdbError> {
        Err(GdbError::Unsupported("access watchpoints"))
    }

    /// Remove the access watchpoint at `address` / 移除 `address` 处的访问监视点
    ///
    /// # Errors
    /// Defaults to [`GdbError::Unsupported`], mirroring
    /// [`Self::set_access_watchpoint`]. / 与 [`Self::set_access_watchpoint`]
    /// 一致，默认返回 [`GdbError::Unsupported`]。
    fn clear_access_watchpoint(&mut self, _address: u32, _length: usize) -> Result<(), GdbError> {
        Err(GdbError::Unsupported("access watchpoints"))
    }

    /// Address of the watchpoint that caused the latest stop
    /// 最近一次停止所命中的监视点地址
    ///
    /// Used for all three kinds: the RSP layer appends `watch:<address>;` for
    /// [`StopReason::Watchpoint`], `rwatch:<address>;` for
    /// [`StopReason::ReadWatch`], and `awatch:<address>;` for
    /// [`StopReason::AccessWatch`]. / 三种类型共用：RSP 层对
    /// [`StopReason::Watchpoint`] 追加 `watch:<地址>;`，对
    /// [`StopReason::ReadWatch`] 追加 `rwatch:<地址>;`，对
    /// [`StopReason::AccessWatch`] 追加 `awatch:<地址>;`。
    #[must_use]
    fn watchpoint_hit(&self) -> Option<u32> {
        None
    }

    /// Execute one `monitor` command and return human-readable output
    /// 执行一条 `monitor` 命令并返回可读输出
    ///
    /// The RSP layer decodes `qRcmd,<hex>` into a UTF-8 command string, calls
    /// this hook, and hex-encodes the returned text. The default returns an
    /// empty string, which the RSP layer maps to `OK` (success, no output), so
    /// targets without monitor support accept the command silently.
    /// RSP 层把 `qRcmd,<hex>` 解码为 UTF-8 命令字符串后调用本钩子，并把返回文本
    /// 编码为十六进制。默认返回空串，RSP 层将其映射为 `OK`（成功且无输出），
    /// 因此不支持 monitor 的目标会静默接受命令。
    fn monitor_command(&mut self, _command: &str) -> String {
        String::new()
    }

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
    fn watchpoint_stops_report_sigtrap() {
        assert_eq!(StopReason::Watchpoint.signal(), 5);
        assert_eq!(StopReason::ReadWatch.signal(), 5);
        assert_eq!(StopReason::AccessWatch.signal(), 5);
        assert_eq!(
            StopReply {
                reason: StopReason::Watchpoint,
                pc: 0x2000_0000,
            }
            .signal(),
            5
        );
    }

    #[test]
    fn watch_kinds_are_ordered_by_rsp_number() {
        assert!(WatchKind::Write < WatchKind::Read);
        assert!(WatchKind::Read < WatchKind::Access);
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
