//! Cortex-M33 floating-point unit state (FPv5-SP, D16)
//! Cortex-M33 浮点单元状态（FPv5-SP，D16）
//!
//! RP2350 的 Cortex-M33 集成 FPv5-SP 单元：32 个单精度寄存器 S0-S31，
//! 可按 D0-D15 双字视图访问，但本实现不做双精度算术。该模块只保存状态
//! （S/D 视图、FPSCR、FPEXC）与 CPACR 访问控制字段；指令语义在
//! `execute_thumb32` 的 FPU 解码器中实现。
//!
//! RP2350's Cortex-M33 integrates an FPv5-SP unit: 32 single-precision
//! registers S0-S31 addressable as the D0-D15 doubleword view, with no
//! double-precision arithmetic implemented here. This module holds only state
//! (S/D views, FPSCR, FPEXC) and the CPACR access-control fields; instruction
//! semantics live in the FPU decoder under `execute_thumb32`.

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
}

impl FpuRegisters {
    /// Create a zeroed FPU register file in reset state
    /// 创建复位状态下清零的 FPU 寄存器文件
    #[must_use]
    pub fn new() -> Self {
        Self {
            s: [0; 32],
            fpscr: 0,
            fpexc: 0,
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
