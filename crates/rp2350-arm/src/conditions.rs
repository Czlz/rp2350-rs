//! Condition flags and IT block state for Cortex-M33
//! Cortex-M33 条件标志和 IT 块状态
//!
//! Implements APSR condition flags (N, Z, C, V, Q), condition code
//! evaluation (EQ, NE, CS, CC, MI, PL, VS, VC, HI, LS, GE, LT, GT, LE),
//! and If-Then block state machine.
//!
//! 实现 APSR 条件标志（N, Z, C, V, Q）、条件码求值
//!（EQ, NE, CS, CC, MI, PL, VS, VC, HI, LS, GE, LT, GT, LE）
//! 和 If-Then 块状态机。

// ARM 条件码使用 ARM 体系参考手册中定义的单字母助记符，
// 文档标记和变量命名反映了这些标准约定。
// ARM condition codes use single-letter mnemonics defined in the ARM Architecture
// Reference Manual; doc markup and naming reflect these standard conventions.
#![allow(
    clippy::doc_markdown,
    clippy::empty_line_after_doc_comments,
    clippy::similar_names,
    clippy::module_name_repetitions,
    clippy::fn_params_excessive_bools
)]

use super::registers::CortexM33Registers;

/// ARM condition codes (4-bit, used in `B<cond>` and IT blocks)
/// ARM 条件码（4 位，用于 `B<cond>` 和 IT 块）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Condition {
    /// Equal — Z == 1
    Eq = 0x0,
    /// Not equal — Z == 0
    Ne = 0x1,
    /// Carry set / Unsigned higher or same — C == 1
    Cs = 0x2,
    /// Carry clear / Unsigned lower — C == 0
    Cc = 0x3,
    /// Minus / Negative — N == 1
    Mi = 0x4,
    /// Plus / Positive or zero — N == 0
    Pl = 0x5,
    /// Overflow set — V == 1
    Vs = 0x6,
    /// Overflow clear — V == 0
    Vc = 0x7,
    /// Unsigned higher — C == 1 && Z == 0
    Hi = 0x8,
    /// Unsigned lower or same — C == 0 || Z == 1
    Ls = 0x9,
    /// Signed greater than or equal — N == V
    Ge = 0xA,
    /// Signed less than — N != V
    Lt = 0xB,
    /// Signed greater than — Z == 0 && N == V
    Gt = 0xC,
    /// Signed less than or equal — Z == 1 || N != V
    Le = 0xD,
    /// Always (unconditional)
    Al = 0xE,
}

impl Condition {
    /// Parse condition code from 4-bit value
    /// 从 4 位值解析条件码
    #[must_use]
    pub fn from_bits(bits: u8) -> Self {
        match bits & 0xF {
            0x0 => Self::Eq,
            0x1 => Self::Ne,
            0x2 => Self::Cs,
            0x3 => Self::Cc,
            0x4 => Self::Mi,
            0x5 => Self::Pl,
            0x6 => Self::Vs,
            0x7 => Self::Vc,
            0x8 => Self::Hi,
            0x9 => Self::Ls,
            0xA => Self::Ge,
            0xB => Self::Lt,
            0xC => Self::Gt,
            0xD => Self::Le,
            _ => Self::Al,
        }
    }

    /// Evaluate the condition against current APSR flags
    /// 根据当前 APSR 标志求值条件
    #[must_use]
    pub fn evaluate(&self, regs: &CortexM33Registers) -> bool {
        let n = regs.apsr_n();
        let z = regs.apsr_z();
        let c = regs.apsr_c();
        let v = regs.apsr_v();
        match self {
            Self::Eq => z,
            Self::Ne => !z,
            Self::Cs => c,
            Self::Cc => !c,
            Self::Mi => n,
            Self::Pl => !n,
            Self::Vs => v,
            Self::Vc => !v,
            Self::Hi => c && !z,
            Self::Ls => !c || z,
            Self::Ge => n == v,
            Self::Lt => n != v,
            Self::Gt => !z && (n == v),
            Self::Le => z || (n != v),
            Self::Al => true,
        }
    }
}

/// If-Then block state machine
/// If-Then 块状态机
///
/// Tracks the current state of IT block execution: the base condition,
/// the condition-swap mask, and how many instructions remain in the block.
/// 跟踪 IT 块执行的当前状态：基础条件、条件交换掩码和块中剩余多少指令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ITBlockState {
    /// Architectural ITSTATE byte (0 = no IT block active)
    /// 架构 ITSTATE 字节（0 = 无活跃 IT 块）
    pub state: u8,
    /// Condition for the next instruction in the block
    /// 块中下一条指令的条件
    pub cond: Condition,
}

impl ITBlockState {
    /// Create a new inactive IT block state
    /// 创建新的非活跃 IT 块状态
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: 0,
            cond: Condition::Al,
        }
    }

    /// Execute an IT instruction and set up the block
    /// 执行 IT 指令并设置块
    ///
    /// `firstcond` is the base condition (bits 7:4 of IT instruction)
    /// `mask` is the condition-swap mask (bits 3:0 of IT instruction)
    ///
    /// Returns the number of instructions in the block.
    /// 返回块中的指令数。
    pub fn execute_it(&mut self, firstcond: u8, mask: u8) -> u8 {
        if mask == 0 {
            // mask == 0 is the hint space, never a real IT block.
            // mask == 0 属于提示空间，不是真正的 IT 块。
            self.state = 0;
            self.cond = Condition::Al;
            return 0;
        }
        // ITSTATE[7:4] = firstcond, ITSTATE[3:0] = mask.
        // ITSTATE[7:4] = firstcond，ITSTATE[3:0] = mask。
        self.state = (firstcond << 4) | (mask & 0x0F);
        self.cond = Condition::from_bits(firstcond);
        4 - mask.trailing_zeros() as u8
    }

    /// Advance to the next instruction in the IT block
    /// 推进到 IT 块中的下一条指令
    ///
    /// Returns the condition for the *current* instruction and applies
    /// `ITAdvance` for the following one, matching ARMv8-M `ITSTATE`
    /// behaviour.
    /// 返回*当前*指令的条件，并对下一条指令执行 `ITAdvance`，符合 ARMv8-M
    /// 的 `ITSTATE` 行为。
    #[must_use]
    pub fn advance(&mut self) -> Condition {
        if self.state == 0 {
            self.cond = Condition::Al;
            return Condition::Al;
        }
        let current = Condition::from_bits((self.state >> 4) & 0x0F);
        if self.state.trailing_zeros() >= 3 {
            self.state = 0;
            self.cond = Condition::Al;
        } else {
            // ITAdvance: keep the base-condition high bits, shift the
            // then/else mask left by one.
            // ITAdvance：保留基础条件的高位，把 then/else 掩码左移一位。
            self.state = (self.state & 0xE0) | ((self.state << 1) & 0x1F);
            self.cond = Condition::from_bits((self.state >> 4) & 0x0F);
        }
        current
    }

    /// Check if currently inside an IT block
    /// 检查当前是否在 IT 块内
    #[must_use]
    pub fn in_it_block(&self) -> bool {
        self.state != 0
    }

    /// Architectural ITSTATE byte for exception stacking
    /// 用于异常压栈的架构 ITSTATE 字节
    #[must_use]
    pub fn arch_state(&self) -> u8 {
        self.state
    }

    /// Rebuild from the ITSTATE byte decoded out of a stacked xPSR
    /// 从异常栈 xPSR 中解码出的 ITSTATE 字节重建
    #[must_use]
    pub fn from_arch_state(state: u8) -> Self {
        Self {
            state,
            cond: Condition::from_bits((state >> 4) & 0x0F),
        }
    }
}

impl Default for ITBlockState {
    fn default() -> Self {
        Self::new()
    }
}

/// NZCV flag utilities for CortexM33Registers
/// CortexM33Registers 的 NZCV 标志工具

/// Extension methods for APSR flag access on registers
/// 寄存器上 APSR 标志访问的扩展方法
impl CortexM33Registers {
    /// Extract Negative flag from APSR
    /// 从 APSR 提取负号标志
    #[must_use]
    pub fn apsr_n(&self) -> bool {
        self.xpsr & 0x8000_0000 != 0
    }

    /// Extract Zero flag from APSR
    /// 从 APSR 提取零标志
    #[must_use]
    pub fn apsr_z(&self) -> bool {
        self.xpsr & 0x4000_0000 != 0
    }

    /// Extract Carry flag from APSR
    /// 从 APSR 提取进位标志
    #[must_use]
    pub fn apsr_c(&self) -> bool {
        self.xpsr & 0x2000_0000 != 0
    }

    /// Extract Overflow flag from APSR
    /// 从 APSR 提取溢出标志
    #[must_use]
    pub fn apsr_v(&self) -> bool {
        self.xpsr & 0x1000_0000 != 0
    }

    /// Update NZCV flags from APSR condition flag values
    /// 从 APSR 条件标志值更新 NZCV 标志
    pub fn set_apsr_nzcv(&mut self, n: bool, z: bool, c: bool, v: bool) {
        // Clear existing NZCV bits in xpsr
        // 清除 xpsr 中现有的 NZCV 位
        self.xpsr &= !0xF000_0000;
        // Set new values
        // 设置新值
        if n {
            self.xpsr |= 0x8000_0000;
        }
        if z {
            self.xpsr |= 0x4000_0000;
        }
        if c {
            self.xpsr |= 0x2000_0000;
        }
        if v {
            self.xpsr |= 0x1000_0000;
        }
    }

    /// Update N and Z flags based on a 32-bit arithmetic result
    /// 根据 32 位算术结果更新 N 和 Z 标志
    pub fn set_nz_flags(&mut self, result: u32) {
        self.set_apsr_nzcv(
            (result & 0x8000_0000) != 0,
            result == 0,
            self.apsr_c(),
            self.apsr_v(),
        );
    }

    /// Set C flag / 设置 C 标志
    pub fn set_c_flag(&mut self, c: bool) {
        if c {
            self.xpsr |= 0x2000_0000;
        } else {
            self.xpsr &= !0x2000_0000;
        }
    }

    /// Set V flag / 设置 V 标志
    pub fn set_v_flag(&mut self, v: bool) {
        if v {
            self.xpsr |= 0x1000_0000;
        } else {
            self.xpsr &= !0x1000_0000;
        }
    }

    /// Set the cumulative saturation flag (APSR.Q) / 设置累积饱和标志（APSR.Q）
    pub fn set_q_flag(&mut self) {
        self.xpsr |= 1 << 27;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registers::CortexM33Registers;

    #[test]
    fn condition_eq() {
        let mut regs = CortexM33Registers::new();
        regs.set_apsr_nzcv(false, true, false, false);
        assert!(Condition::Eq.evaluate(&regs));
        assert!(!Condition::Ne.evaluate(&regs));
    }

    #[test]
    fn condition_cs_cc() {
        let mut regs = CortexM33Registers::new();
        regs.set_apsr_nzcv(false, false, true, false);
        assert!(Condition::Cs.evaluate(&regs));
        assert!(!Condition::Cc.evaluate(&regs));
    }

    #[test]
    fn condition_ge_lt() {
        let mut regs = CortexM33Registers::new();
        // N=V=0 → GE is true
        regs.set_apsr_nzcv(false, false, false, false);
        assert!(Condition::Ge.evaluate(&regs));
        assert!(!Condition::Lt.evaluate(&regs));

        // N=1, V=0 → LT is true
        regs.set_apsr_nzcv(true, false, false, false);
        assert!(!Condition::Ge.evaluate(&regs));
        assert!(Condition::Lt.evaluate(&regs));
    }

    #[test]
    fn condition_gt_le() {
        let mut regs = CortexM33Registers::new();
        // Z=0, N=V=0 → GT is true
        regs.set_apsr_nzcv(false, false, false, false);
        assert!(Condition::Gt.evaluate(&regs));
        assert!(!Condition::Le.evaluate(&regs));

        // Z=1 → LE is true
        regs.set_apsr_nzcv(false, true, false, false);
        assert!(!Condition::Gt.evaluate(&regs));
        assert!(Condition::Le.evaluate(&regs));
    }

    #[test]
    fn condition_al_always_true() {
        let regs = CortexM33Registers::new();
        assert!(Condition::Al.evaluate(&regs));
    }

    #[test]
    fn it_block_4_instructions() {
        let mut it = ITBlockState::new();
        let count = it.execute_it(0x0, 0xB); // ITETE eq
        assert_eq!(count, 4);
        assert_eq!(it.cond, Condition::Eq);
        assert!(it.in_it_block());

        // 1st: eq, 2nd: ne, 3rd: eq, 4th: ne
        assert_eq!(it.advance(), Condition::Eq); // 1st
        assert_eq!(it.advance(), Condition::Ne); // 2nd
        assert_eq!(it.advance(), Condition::Eq); // 3rd
        assert_eq!(it.advance(), Condition::Ne); // 4th
        assert!(!it.in_it_block());
    }

    #[test]
    fn it_block_3_instructions_repeats_the_base_condition() {
        let mut it = ITBlockState::new();
        let count = it.execute_it(0x0, 0x2); // ITTT eq
        assert_eq!(count, 3);

        // 0b0010: 两位 T 分别对应第 2、3 条指令，最低位的 1 只是终止标记。
        // 0b0010: both T bits cover instructions 2 and 3; the lowest 1 only
        // terminates the block.
        assert_eq!(it.advance(), Condition::Eq);
        assert_eq!(it.advance(), Condition::Eq);
        assert_eq!(it.advance(), Condition::Eq);
        assert!(!it.in_it_block());
    }

    #[test]
    fn set_q_flag_keeps_the_other_condition_flags() {
        let mut regs = CortexM33Registers::new();
        regs.set_apsr_nzcv(true, false, true, false);
        regs.set_q_flag();
        assert_eq!(regs.xpsr & (1 << 27), 1 << 27);
        assert!(regs.apsr_n());
        assert!(!regs.apsr_z());
        assert!(regs.apsr_c());
        assert!(!regs.apsr_v());
    }

    #[test]
    fn it_state_arch_round_trip() {
        for state in [0x00, 0x08, 0x0B, 0x16, 0x0C, 0x18, 0x1C, 0xFF] {
            let restored = ITBlockState::from_arch_state(state);
            assert_eq!(restored.arch_state(), state);
        }
    }

    #[test]
    fn it_block_inverts_relative_to_an_odd_base_condition() {
        // ITE NE（0xBF14）第二槽应为 EQ，而不是继续 NE。
        // ITE NE (0xBF14) must run its second slot as EQ, not NE.
        let mut it = ITBlockState::new();
        assert_eq!(it.execute_it(0x1, 0x4), 2);
        assert_eq!(it.advance(), Condition::Ne);
        assert_eq!(it.advance(), Condition::Eq);
        assert!(!it.in_it_block());

        // ITT NE（0xBF1C）两槽都是 NE。
        // ITT NE (0xBF1C) keeps NE in both slots.
        assert_eq!(it.execute_it(0x1, 0xC), 2);
        assert_eq!(it.advance(), Condition::Ne);
        assert_eq!(it.advance(), Condition::Ne);
        assert!(!it.in_it_block());
    }

    #[test]
    fn zero_mask_does_not_arm_it_block() {
        // mask == 0 属于 hint 空间，不能武装 IT 块。
        // A zero mask is hint space and must not arm an IT block.
        let mut it = ITBlockState::new();
        let count = it.execute_it(0x8, 0x0);
        assert_eq!(count, 0);
        assert!(!it.in_it_block());
        assert_eq!(it.advance(), Condition::Al);
    }
}
