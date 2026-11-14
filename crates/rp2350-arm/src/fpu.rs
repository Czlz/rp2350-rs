//! Cortex-M33 floating-point unit state (FPv5-SP, D16)
//! Cortex-M33 浮点单元状态（FPv5-SP，D16）
//!
//! RP2350 的 Cortex-M33 集成 FPv5-SP 单元：32 个单精度寄存器 S0-S31，
//! 可按 D0-D15 双字视图访问，但本实现不做双精度算术。该模块保存状态
//! （S/D 视图、FPSCR、FPEXC、FPCCR/FPCAR/FPDSCR）与 CPACR 访问控制字段；
//! 指令语义在 `execute_thumb32` 的 FPU 解码器中实现，惰性压栈的栈帧操作在
//! [`crate::exceptions`] 中实现。
//!
//! RP2350's Cortex-M33 integrates an FPv5-SP unit: 32 single-precision
//! registers S0-S31 addressable as the D0-D15 doubleword view, with no
//! double-precision arithmetic implemented here. This module holds state
//! (S/D views, FPSCR, FPEXC, FPCCR/FPCAR/FPDSCR) and the CPACR access-control
//! fields; instruction semantics live in the FPU decoder under
//! `execute_thumb32`, and lazy-stacking frame operations live in
//! [`crate::exceptions`].

/// FPSCR bit definitions / FPSCR 位定义
///
/// Exception status bits follow the ARMv8-M FPSCR layout: IOC/DZC/OFC/UFC/IXC
/// in bits 0-4 and IDC in bit 7. Rounding mode occupies bits 23:22.
/// 异常状态位遵循 ARMv8-M FPSCR 布局：IOC/DZC/OFC/UFC/IXC 位于位 0-4，
/// IDC 位于位 7。舍入模式位于位段 23:22。
pub mod fpscr {
    /// Negative condition flag / 负号条件标志
    pub const N: u32 = 1 << 31;
    /// Zero condition flag / 零条件标志
    pub const Z: u32 = 1 << 30;
    /// Carry condition flag / 进位条件标志
    pub const C: u32 = 1 << 29;
    /// Overflow condition flag / 溢出条件标志
    pub const V: u32 = 1 << 28;
    /// Cumulative saturation flag / 累积饱和标志
    pub const QC: u32 = 1 << 27;
    /// Alternative half-precision control / 备用半精度控制
    pub const AHP: u32 = 1 << 26;
    /// Default NaN control / 默认 NaN 控制
    pub const DN: u32 = 1 << 25;
    /// Flush-to-zero control / 清零模式控制
    pub const FZ: u32 = 1 << 24;
    /// Rounding mode field shift / 舍入模式字段位移
    pub const RMODE_SHIFT: u32 = 22;
    /// Rounding mode field mask, in place / 原地掩码的舍入模式字段
    pub const RMODE_MASK: u32 = 0x3 << RMODE_SHIFT;
    /// Input denormal cumulative exception / 输入非正规数累积异常
    pub const IDC: u32 = 1 << 7;
    /// Inexact cumulative exception / 不精确累积异常
    pub const IXC: u32 = 1 << 4;
    /// Underflow cumulative exception / 下溢累积异常
    pub const UFC: u32 = 1 << 3;
    /// Overflow cumulative exception / 上溢累积异常
    pub const OFC: u32 = 1 << 2;
    /// Divide-by-zero cumulative exception / 除零累积异常
    pub const DZC: u32 = 1 << 1;
    /// Invalid-operation cumulative exception / 无效操作累积异常
    pub const IOC: u32 = 1 << 0;
    /// All cumulative exception bits / 全部累积异常位
    pub const EXCEPTION_FLAGS: u32 = IDC | IXC | UFC | OFC | DZC | IOC;
    /// Condition flag mask / 条件标志掩码
    pub const CONDITION_FLAGS: u32 = N | Z | C | V;
}

/// FPEXC bit definitions / FPEXC 位定义
pub mod fpexc {
    /// Exception flag (bits 7:0 hold the exception type) / 异常标志（位 7:0 为异常类型）
    pub const EX: u32 = 1 << 31;
    /// FPU enable flag / FPU 使能标志
    pub const EN: u32 = 1 << 30;
}

/// FPCCR bit definitions / FPCCR 位定义
///
/// Layout follows the ARMv8-M `FPCCR` (no Security Extension): `ASPEN` in bit
/// 31, `LSPEN` in bit 30 and the hardware-managed `LSPACT` in bit 0. Bits that
/// this model does not implement (`LSPENS`, `CLRONRET`, `CLRONRETS`, `TS`,
/// `USER`, `THREAD` and the readiness flags) read as zero. The reset value is
/// `0xC000_0000`, i.e. automatic and lazy state preservation both enabled, as
/// documented for the Cortex-M33.
/// 布局遵循 ARMv8-M 的 `FPCCR`（无安全扩展）：`ASPEN` 位于 bit 31，`LSPEN`
/// 位于 bit 30，硬件管理的 `LSPACT` 位于 bit 0。本模型未实现的位
/// （`LSPENS`、`CLRONRET`、`CLRONRETS`、`TS`、`USER`、`THREAD` 与就绪标志）
/// 读为 0。复位值为 `0xC000_0000`，即自动压栈与惰性压栈都使能，与
/// Cortex-M33 手册一致。
pub mod fpccr {
    /// Automatic state preservation enable / 自动状态保存使能
    pub const ASPEN: u32 = 1 << 31;
    /// Lazy state preservation enable / 惰性状态保存使能
    pub const LSPEN: u32 = 1 << 30;
    /// Lazy state preservation active, hardware managed, read-only
    /// 惰性状态保存进行中，硬件管理，只读
    pub const LSPACT: u32 = 1 << 0;
    /// Bits writable by software through `system_write`
    /// 可经 `system_write` 由软件写入的位
    pub const WRITABLE_MASK: u32 = ASPEN | LSPEN;
    /// Reset value: `ASPEN | LSPEN` / 复位值：`ASPEN | LSPEN`
    pub const RESET: u32 = ASPEN | LSPEN;
}

/// FPCAR bit definitions / FPCAR 位定义
///
/// `FPCAR` holds the address of the floating-point context area reserved on the
/// exception stack frame. The three low bits are reserved and read as zero, so
/// the area is 8-byte aligned.
/// `FPCAR` 保存异常栈帧上预留的浮点上下文区地址。低 3 位保留并读为 0，
/// 因此该区域按 8 字节对齐。
pub mod fpcar {
    /// Reserved low bits / 保留低位
    pub const RESERVED_MASK: u32 = 0x7;
    /// Address field mask / 地址字段掩码
    pub const ADDRESS_MASK: u32 = !RESERVED_MASK;
}

/// FPDSCR bit definitions / FPDSCR 位定义
///
/// `FPDSCR` provides the default `AHP`, `DN`, `FZ` and `RMode` values loaded on
/// exception entry. Only the bit positions that also exist in `FPSCR` are
/// modeled, and no arithmetic path reads them (rounding always stays
/// round-to-nearest-even), so they are plain storage.
/// `FPDSCR` 提供异常进入时装载的默认 `AHP`、`DN`、`FZ` 与 `RMode` 值。本模型
/// 只保存与 `FPSCR` 同位置的位段，且没有任何算术路径读取它们（舍入恒为
/// 就近偶数），因此它们是纯存储。
pub mod fpdscr {
    /// `AHP`/`DN`/`FZ`/`RMode` field mask / `AHP`/`DN`/`FZ`/`RMode` 字段掩码
    pub const MODELED_MASK: u32 = 0x07C0_0000;
}

/// Absolute addresses of the FPU system registers / 浮点系统寄存器的绝对地址
pub mod sys_reg {
    /// Floating-point context control register / 浮点上下文控制寄存器
    pub const FPCCR: u32 = 0xE000_EF34;
    /// Floating-point context address register / 浮点上下文地址寄存器
    pub const FPCAR: u32 = 0xE000_EF38;
    /// Floating-point default status control register / 浮点默认状态控制寄存器
    pub const FPDSCR: u32 = 0xE000_EF3C;
}

/// CPACR CP10/CP11 field layout / CPACR CP10/CP11 字段布局
pub mod cpacr {
    /// Shift of the CP10 field / CP10 字段位移
    pub const CP10_SHIFT: u32 = 20;
    /// Shift of the CP11 field / CP11 字段位移
    pub const CP11_SHIFT: u32 = 22;
    /// Mask covering both CP10 and CP11 / 覆盖 CP10 与 CP11 的掩码
    pub const CP10_CP11_MASK: u32 = 0x00F0_0000;
    /// Mask covering CP0-CP7 / 覆盖 CP0-CP7 的掩码
    ///
    /// CP0-CP7 occupy bits 15:0 (two bits each). In particular CP4 lives in
    /// bits 9:8, so a `0x0000_00FF` mask would cover only CP0-CP3 and drop
    /// the DCP key written by pico-sdk.
    /// CP0-CP7 占用 bits 15:0（每个两位）。特别是 CP4 位于 bits 9:8，
    /// 因此 `0x0000_00FF` 掩码只覆盖 CP0-CP3，会丢掉 pico-sdk 写入的
    /// DCP 键。
    pub const CP0_CP7_MASK: u32 = 0x0000_FFFF;
    /// Mask covering every CPACR field modeled by the core
    /// 核心建模的全部 CPACR 字段掩码
    ///
    /// CP0-CP7 are stored because the GPIO coprocessor (CP0) and the
    /// double-precision coprocessor (CP4) use them; CP10-CP11 gate the FPU.
    /// CP0-CP7 需要保存，因为 GPIO 协处理器（CP0）与双精度协处理器（CP4）
    /// 使用这些字段；CP10-CP11 控制 FPU。
    pub const MODELED_MASK: u32 = CP0_CP7_MASK | CP10_CP11_MASK;
    /// Access denied — using the coprocessor raises NOCP
    /// 禁止访问 —— 使用协处理器会产生 NOCP
    pub const DENIED: u32 = 0b00;
    /// Privileged access only / 仅特权模式可访问
    pub const PRIVILEGED: u32 = 0b01;
    /// Full access / 完全访问
    pub const FULL: u32 = 0b11;
}

/// Floating-point register file / 浮点寄存器文件
///
/// The S registers are the architectural storage; D0-D15 are aliases where
/// `Dn` is formed from `S(2n)` in the low word and `S(2n+1)` in the high word.
/// S 寄存器是体系结构存储；D0-D15 为别名，`Dn` 由低字 `S(2n)` 与高字
/// `S(2n+1)` 组成。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FpuRegisters {
    /// Single-precision registers S0-S31 / 单精度寄存器 S0-S31
    pub s: [u32; 32],
    /// Floating-point status and control register / 浮点状态与控制寄存器
    pub fpscr: u32,
    /// Floating-point exception control register / 浮点异常控制寄存器
    pub fpexc: u32,
    /// Floating-point context control register / 浮点上下文控制寄存器
    ///
    /// Holds `FPCCR.ASPEN`, `FPCCR.LSPEN` and the hardware-managed
    /// `FPCCR.LSPACT` bit. / 保存 `FPCCR.ASPEN`、`FPCCR.LSPEN` 与硬件管理的
    /// `FPCCR.LSPACT` 位。
    pub fpccr: u32,
    /// Floating-point context address register / 浮点上下文地址寄存器
    ///
    /// Address of the reserved floating-point context area on the exception
    /// stack frame; the low three bits are always zero.
    /// 异常栈帧上预留的浮点上下文区地址；低 3 位恒为 0。
    pub fpcar: u32,
    /// Floating-point default status control register / 浮点默认状态控制寄存器
    pub fpdscr: u32,
}

impl FpuRegisters {
    /// Create a zeroed FPU register file in reset state
    /// 创建复位状态下清零的 FPU 寄存器文件
    ///
    /// `FPCCR` starts at its ARM reset value (`ASPEN | LSPEN`); `FPCAR` and
    /// `FPDSCR` reset to zero. / `FPCCR` 以 ARM 复位值（`ASPEN | LSPEN`）启动；
    /// `FPCAR` 与 `FPDSCR` 复位为 0。
    #[must_use]
    pub fn new() -> Self {
        Self {
            s: [0; 32],
            fpscr: 0,
            fpexc: 0,
            fpccr: fpccr::RESET,
            fpcar: 0,
            fpdscr: 0,
        }
    }

    /// Report whether lazy FP state preservation is currently active
    /// 报告惰性浮点状态保存当前是否活跃
    #[must_use]
    pub fn lspact(&self) -> bool {
        self.fpccr & fpccr::LSPACT != 0
    }

    /// Set or clear `FPCCR.LSPACT`
    /// 设置或清除 `FPCCR.LSPACT`
    pub fn set_lspact(&mut self, active: bool) {
        if active {
            self.fpccr |= fpccr::LSPACT;
        } else {
            self.fpccr &= !fpccr::LSPACT;
        }
    }

    /// Write `FPCCR`; only software-writable bits are stored
    /// 写入 `FPCCR`；只保存软件可写位
    ///
    /// `LSPACT` is hardware managed and keeps its current value, matching the
    /// read-only nature of the bit. / `LSPACT` 由硬件管理并保持当前值，与该位
    /// 只读的性质一致。
    pub fn write_fpccr(&mut self, value: u32) {
        self.fpccr = (self.fpccr & fpccr::LSPACT) | (value & fpccr::WRITABLE_MASK);
    }

    /// Write `FPCAR`; the three low bits are reserved and forced to zero
    /// 写入 `FPCAR`；低 3 位保留并强制为 0
    pub fn write_fpcar(&mut self, value: u32) {
        self.fpcar = value & fpcar::ADDRESS_MASK;
    }

    /// Write `FPDSCR`; only the modeled `AHP`/`DN`/`FZ`/`RMode` bits are stored
    /// 写入 `FPDSCR`；只保存建模的 `AHP`/`DN`/`FZ`/`RMode` 位
    pub fn write_fpdscr(&mut self, value: u32) {
        self.fpdscr = value & fpdscr::MODELED_MASK;
    }

    /// Handle an FPU system-register access from the private peripheral bus
    /// 处理来自私有外设总线的浮点系统寄存器访问
    ///
    /// Returns `Some(value)` when `addr` is `FPCCR`, `FPCAR` or `FPDSCR`, with
    /// the read-back value after a write. `LSPACT` is hardware managed, so
    /// software writes to `FPCCR` cannot change it, and `FPCAR`/`FPDSCR`
    /// filter their reserved or unmodeled bits.
    /// 当 `addr` 为 `FPCCR`、`FPCAR` 或 `FPDSCR` 时返回 `Some(value)`，写入后
    /// 返回回读值。`LSPACT` 由硬件管理，软件写 `FPCCR` 不会改变它；
    /// `FPCAR`/`FPDSCR` 会过滤保留位与未建模位。
    pub fn system_access(&mut self, addr: u32, write: bool, value: u32) -> Option<u32> {
        match addr {
            sys_reg::FPCCR => {
                if write {
                    self.write_fpccr(value);
                }
                Some(self.fpccr)
            }
            sys_reg::FPCAR => {
                if write {
                    self.write_fpcar(value);
                }
                Some(self.fpcar)
            }
            sys_reg::FPDSCR => {
                if write {
                    self.write_fpdscr(value);
                }
                Some(self.fpdscr)
            }
            _ => None,
        }
    }

    /// Read S0-S31; the index wraps modulo 32
    /// 读取 S0-S31；索引按 32 取模回绕
    #[must_use]
    pub fn read_s(&self, index: u8) -> u32 {
        self.s[usize::from(index & 0x1F)]
    }

    /// Write S0-S31; the index wraps modulo 32
    /// 写入 S0-S31；索引按 32 取模回绕
    pub fn write_s(&mut self, index: u8, value: u32) {
        self.s[usize::from(index & 0x1F)] = value;
    }

    /// Read D0-D15 as the S-pair alias; the index wraps modulo 16
    /// 按 S 对别名读取 D0-D15；索引按 16 取模回绕
    #[must_use]
    pub fn read_d(&self, index: u8) -> u64 {
        let low = usize::from((index & 0x0F) * 2);
        u64::from(self.s[low]) | (u64::from(self.s[low + 1]) << 32)
    }

    /// Write D0-D15 as the S-pair alias; the index wraps modulo 16
    /// 按 S 对别名写入 D0-D15；索引按 16 取模回绕
    pub fn write_d(&mut self, index: u8, value: u64) {
        let low = usize::from((index & 0x0F) * 2);
        self.s[low] = value as u32;
        self.s[low + 1] = (value >> 32) as u32;
    }
}

impl Default for FpuRegisters {
    fn default() -> Self {
        Self::new()
    }
}

/// Report whether a Thumb-32 encoding belongs to the FPU (CP10/CP11)
/// 报告一条 Thumb-32 编码是否属于浮点单元（CP10/CP11）
///
/// The coprocessor encodings live in the `0xEC`, `0xED`, `0xEE` and `0xFE`
/// spaces and carry the coprocessor number in `bits[11:8]`. Coprocessors 10 and
/// 11 are the FPv5-SP unit; every VFP data-processing, transfer and
/// load/store encoding uses one of those two keys. The core uses this
/// predicate to decide when a pending lazy FP context must be committed
/// before executing the instruction.
/// 协处理器编码位于 `0xEC`、`0xED`、`0xEE` 与 `0xFE` 空间，协处理器号在
/// `bits[11:8]`。协处理器 10 与 11 即 FPv5-SP 单元；所有 VFP 数据处理、
/// 传输与加载/存储编码都使用这两个键之一。核心用它判断执行某条指令前
/// 是否需要提交待处理的惰性浮点上下文。
#[must_use]
pub fn is_fpu_encoding(instr: u32) -> bool {
    let top = instr >> 24;
    matches!(top, 0xEC..=0xEE | 0xFE) && matches!((instr >> 8) & 0xF, 0xA | 0xB)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_state_is_zeroed() {
        let fpu = FpuRegisters::new();
        assert_eq!(fpu.s, [0; 32]);
        assert_eq!(fpu.fpscr, 0);
        assert_eq!(fpu.fpexc, 0);
    }

    #[test]
    fn reset_fpccr_enables_automatic_and_lazy_preservation() {
        let fpu = FpuRegisters::new();
        // Cortex-M33 复位值：ASPEN=1、LSPEN=1、LSPACT=0。
        // Cortex-M33 reset value: ASPEN=1, LSPEN=1, LSPACT=0.
        assert_eq!(fpu.fpccr, 0xC000_0000);
        assert!(!fpu.lspact());
        assert_eq!(fpu.fpcar, 0);
        assert_eq!(fpu.fpdscr, 0);
    }

    #[test]
    fn fpccr_write_ignores_lspact_and_reserved_bits() {
        let mut fpu = FpuRegisters::new();
        fpu.set_lspact(true);
        fpu.write_fpccr(0xFFFF_FFFF);
        assert_eq!(fpu.fpccr, 0xC000_0001);
        fpu.write_fpccr(0);
        assert_eq!(fpu.fpccr, 0x0000_0001);
        fpu.set_lspact(false);
        assert_eq!(fpu.fpccr, 0);
    }

    #[test]
    fn fpcar_write_aligns_to_eight_bytes() {
        let mut fpu = FpuRegisters::new();
        fpu.write_fpcar(0x2000_0FFF);
        assert_eq!(fpu.fpcar, 0x2000_0FF8);
        assert_eq!(fpu.fpcar & fpcar::RESERVED_MASK, 0);
    }

    #[test]
    fn fpdscr_keeps_only_modeled_rounding_fields() {
        let mut fpu = FpuRegisters::new();
        fpu.write_fpdscr(0xFFFF_FFFF);
        assert_eq!(fpu.fpdscr, 0x07C0_0000);
        fpu.write_fpdscr(0x0040_0000);
        assert_eq!(fpu.fpdscr, 0x0040_0000);
    }

    #[test]
    fn fpu_encoding_predicate_selects_cp10_cp11_only() {
        // VLDR S0, [R0, #4] / VMOV S0, R0 / VMRS R0, FPSCR are FPU encodings.
        assert!(is_fpu_encoding(0xED90_0A01));
        assert!(is_fpu_encoding(0xEE00_0A10));
        assert!(is_fpu_encoding(0xEEF1_0A10));
        // CP0 GPIO MCR and CP4 DCP MRC use the same space but not CP10/CP11.
        assert!(!is_fpu_encoding(0xEE00_0000));
        assert!(!is_fpu_encoding(0xEE10_0430));
        // Ordinary Thumb-32 encodings never match.
        assert!(!is_fpu_encoding(0xF000_F800));
    }

    #[test]
    fn d_view_aliases_s_pair() {
        let mut fpu = FpuRegisters::new();
        fpu.write_s(0, 0x1234_5678);
        fpu.write_s(1, 0x9ABC_DEF0);
        assert_eq!(fpu.read_d(0), 0x9ABC_DEF0_1234_5678);
    }

    #[test]
    fn write_d_splits_into_s_pair() {
        let mut fpu = FpuRegisters::new();
        fpu.write_d(7, 0x0123_4567_89AB_CDEF);
        assert_eq!(fpu.read_s(14), 0x89AB_CDEF);
        assert_eq!(fpu.read_s(15), 0x0123_4567);
        assert_eq!(fpu.read_d(7), 0x0123_4567_89AB_CDEF);
    }

    #[test]
    fn index_wraps_within_register_bank() {
        let mut fpu = FpuRegisters::new();
        fpu.write_s(32, 0xAAAA_BBBB);
        assert_eq!(fpu.read_s(0), 0xAAAA_BBBB);
        fpu.write_d(16, 0x1111_2222_3333_4444);
        assert_eq!(fpu.read_d(0), 0x1111_2222_3333_4444);
    }
}
