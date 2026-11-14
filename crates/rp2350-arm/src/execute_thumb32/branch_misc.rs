//! Thumb-32 branch, immediate, bitfield, and miscellaneous instructions
//! Thumb-32 分支、立即数、位域与杂项指令
//!
//! Covers conditional and unconditional branches, BL/BLX, MRS/MSR, barriers,
//! hints, plain-binary immediates (ADDW/SUBW/MOVW/MOVT), and the bitfield
//! instructions (SBFX/UBFX/BFI/BFC).
//! 覆盖条件/无条件分支、BL/BLX、MRS/MSR、屏障、提示指令、纯二进制立即数
//!（ADDW/SUBW/MOVW/MOVT）以及位域指令（SBFX/UBFX/BFI/BFC）。

use crate::conditions::Condition;
use crate::execute_thumb16::StepResult;
use crate::registers::CortexM33Registers;

use super::sign_extend;

/// Execute branch and miscellaneous control, `op1 == 0b10`, hw2 bit 15 set
/// 执行分支与杂项控制，`op1 == 0b10`，第二半字位 15 置位
///
/// Second-halfword bits [15:14] select the long branch encodings: `11` is
/// `BL` (bit 12 set) or `BLX` (bit 12 clear), and `10` with bit 12 set is
/// `B.W` T4. When bits [15:14] are `10` with bit 12 clear, instruction bits
/// [25:22] select `MRS` (0xF), system instructions (0xE), or a conditional
/// branch (`B<cond>.W` T3).
/// 第二半字位 [15:14] 选择长分支编码：`11` 为 `BL`（位 12 置位）或
/// `BLX`（位 12 清零），`10` 且位 12 置位为 `B.W` T4。当 [15:14] 为 `10`
/// 且位 12 清零时，指令位 [25:22] 选择 `MRS`（0xF）、系统指令（0xE）或
/// 条件分支（`B<cond>.W` T3）。
pub(super) fn execute_branch_or_misc(instr: u32, regs: &mut CortexM33Registers) -> StepResult {
    let hw2 = instr & 0xFFFF;

    if hw2 & 0x8000 != 0 {
        // bits[15:14] == 11 selects BL (bit 12 set) or BLX (bit 12 clear);
        // bits[15:14] == 10 with bit 12 set is B.W T4. The remaining
        // bits[15:14] == 10 with bit 12 clear cases are B<cond>.W or system
        // instructions and fall through to the decoder below.
        // bits[15:14] == 11 时，位 12 置位为 BL、清零为 BLX；
        // bits[15:14] == 10 且位 12 置位为 B.W T4。其余 bits[15:14] == 10
        // 且位 12 清零的情况是 B<cond>.W 或系统指令，交由下方解码。
        let kind = hw2 & 0xC000;
        if kind == 0xC000 {
            let target = regs.pc.wrapping_add(long_branch_offset(instr));
            if hw2 & 0x1000 != 0 {
                // BL keeps the halfword alignment guaranteed by imm25[0] = 0.
                // BL：偏移最低位编码为 0，天然保证半字对齐。
                if target & 1 != 0 {
                    return StepResult::Undefined;
                }
            } else {
                // BLX switches to ARM state, so the target must be word aligned.
                // BLX：切换到 ARM 状态，目标必须字对齐。
                if target & 0x3 != 0 {
                    return StepResult::Undefined;
                }
            }
            regs.lr = regs.pc | 1;
            regs.pc = target;
            return StepResult::BranchTaken;
        }
        if kind == 0x8000 && hw2 & 0x1000 != 0 {
            regs.pc = regs.pc.wrapping_add(long_branch_offset(instr));
            return StepResult::BranchTaken;
        }
    }

    let cond_bits = ((instr >> 22) & 0xF) as u8;
    match cond_bits {
        0xF => execute_mrs(instr, regs),
        0xE => execute_msr_or_misc(instr, regs),
        _ => {
            let cond = Condition::from_bits(cond_bits);
            if !cond.evaluate(regs) {
                return StepResult::Ok;
            }
            let s = (instr >> 26) & 1;
            let j2 = (instr >> 11) & 1;
            let j1 = (instr >> 13) & 1;
            let imm6 = (instr >> 16) & 0x3F;
            let imm11 = instr & 0x7FF;
            let imm21 = (s << 20) | (j2 << 19) | (j1 << 18) | (imm6 << 12) | (imm11 << 1);
            let offset = sign_extend(imm21, 21);
            regs.pc = regs.pc.wrapping_add(offset);
            StepResult::BranchTaken
        }
    }
}

/// Compute the signed offset shared by `B.W` T4, `BL`, and `BLX`
/// 计算 `B.W` T4、`BL` 与 `BLX` 共享的有符号偏移
///
/// The offset is `S:I1:I2:imm10:imm11:0` with `I1 = NOT(J1 XOR S)` and
/// `I2 = NOT(J2 XOR S)`.
/// 偏移为 `S:I1:I2:imm10:imm11:0`，其中 `I1 = NOT(J1 XOR S)`、
/// `I2 = NOT(J2 XOR S)`。
fn long_branch_offset(instr: u32) -> u32 {
    let s = (instr >> 26) & 1;
    let j1 = (instr >> 13) & 1;
    let j2 = (instr >> 11) & 1;
    let i1 = u32::from((j1 ^ s) == 0);
    let i2 = u32::from((j2 ^ s) == 0);
    let imm10 = (instr >> 16) & 0x3FF;
    let imm11 = instr & 0x7FF;
    let imm25 = (s << 24) | (i1 << 23) | (i2 << 22) | (imm10 << 12) | (imm11 << 1);
    sign_extend(imm25, 25)
}

/// Execute `MRS <Rd>, <special register>`
/// 执行 `MRS <Rd>, <特殊寄存器>`
fn execute_mrs(instr: u32, regs: &mut CortexM33Registers) -> StepResult {
    let rd = ((instr >> 8) & 0xF) as u8;
    let sysm = (instr & 0xFF) as u8;
    let value = match sysm {
        // APSR: NZCVQ plus GE[3:0] (ARMv8-M APSR bits[19:16]).
        // APSR：NZCVQ 加上 GE[3:0]（ARMv8-M APSR 的 bits[19:16]）。
        0x00 => regs.xpsr & 0xF80F_0000,
        0x03 => regs.xpsr & 0xFF0F_01FF, // xPSR / PSR
        0x05 => regs.xpsr & 0x0000_01FF, // IPSR
        0x10 => regs.primask & 1,        // PRIMASK
        0x11 | 0x12 => regs.basepri,     // BASEPRI / BASEPRI_MAX (reads current BASEPRI)
        0x13 => regs.faultmask & 1,      // FAULTMASK
        0x14 => regs.control,            // CONTROL
        0x0A => regs.msplim,             // MSPLIM
        0x0B => regs.psplim,             // PSPLIM
        _ => return StepResult::Undefined,
    };
    if rd == 15 {
        return StepResult::Undefined;
    }
    regs.write(rd, value);
    StepResult::Ok
}

/// Execute `MSR`, barrier, and hint instructions (instruction bits [25:22] = 1110)
/// 执行 `MSR`、屏障与提示指令（指令位 [25:22] = 1110）
fn execute_msr_or_misc(instr: u32, regs: &mut CortexM33Registers) -> StepResult {
    let op4 = ((instr >> 20) & 0xF) as u8;
    let rn = ((instr >> 16) & 0xF) as u8;
    let sysm = (instr & 0xFF) as u8;

    match op4 {
        // MSR (register)
        0x8 => {
            let value = regs.read(rn);
            match sysm {
                0x00 => {
                    // APSR writes are qualified by the mask field in
                    // bits[11:8]: 0b1000 = nzcvq, 0b0100 = g, 0b1100 = both.
                    // APSR 写入由 bits[11:8] 的掩码字段限定：
                    // 0b1000 = nzcvq，0b0100 = g，0b1100 = 两者。
                    let mask = (instr >> 8) & 0xF;
                    if mask & 0x8 != 0 {
                        regs.set_apsr_nzcv(
                            value & 0x8000_0000 != 0,
                            value & 0x4000_0000 != 0,
                            value & 0x2000_0000 != 0,
                            value & 0x1000_0000 != 0,
                        );
                        regs.set_apsr_q(value & (1 << 27) != 0);
                    }
                    if mask & 0x4 != 0 {
                        regs.set_apsr_ge((value >> 16) & 0xF);
                    }
                    if mask & 0x3 != 0 || mask == 0 {
                        return StepResult::Undefined;
                    }
                }
                0x10 => regs.primask = value & 1,
                0x11 => regs.basepri = value,
                // BASEPRI_MAX only raises the priority mask: a zero value or a
                // numerically larger (lower-priority) value leaves BASEPRI
                // unchanged.
                // BASEPRI_MAX 只提高优先级掩码：零值或数值更大（优先级更低）的
                // 值不会改变 BASEPRI。
                0x12 => {
                    if value != 0 && (regs.basepri == 0 || value < regs.basepri) {
                        regs.basepri = value;
                    }
                }
                0x13 => regs.faultmask = value & 1,
                0x14 => regs.control = value & 0x3,
                // Stack Pointer limits: bits[2:0] are reserved and read as zero.
                // 栈指针下限：bits[2:0] 保留，读为 0。
                0x0A => regs.msplim = value & !0x7,
                0x0B => regs.psplim = value & !0x7,
                _ => return StepResult::Undefined,
            }
            StepResult::Ok
        }
        // DSB (4), DMB (5), ISB (6): ordering only, no architectural state.
        // The selector is bits[7:4]; bits[3:0] carry the option field.
        // DSB（4）、DMB（5）、ISB（6）：仅影响顺序，无体系结构状态。
        // 选择子位于 bits[7:4]；bits[3:0] 携带选项字段。
        0xB => match (instr >> 4) & 0xF {
            0x4..=0x6 => StepResult::Ok,
            _ => StepResult::Undefined,
        },
        // Hints: NOP (0), YIELD (1), WFE (2), WFI (3), SEV (4).
        // 提示指令：NOP（0）、YIELD（1）、WFE（2）、WFI（3）、SEV（4）。
        0xA => match instr & 0xF {
            0x0 | 0x1 | 0x4 => StepResult::Ok,
            0x2 | 0x3 => StepResult::Sleep,
            _ => StepResult::Undefined,
        },
        _ => StepResult::Undefined,
    }
}

/// Execute plain-binary immediate instructions (ADDW/SUBW/MOVW/MOVT)
/// 执行纯二进制立即数指令（ADDW/SUBW/MOVW/MOVT）
///
/// `op1 == 0b10`, hw2 bit 15 clear, `op` in `0x20..=0x2F`.
/// `op1 == 0b10`，第二半字位 15 清零，`op` 在 `0x20..=0x2F`。
pub(super) fn execute_plain_immediate(instr: u32, regs: &mut CortexM33Registers) -> StepResult {
    let op = ((instr >> 20) & 0x7F) as u8;
    let rn = ((instr >> 16) & 0xF) as u8;
    let rd = ((instr >> 8) & 0xF) as u8;
    let i = (instr >> 26) & 1;
    let imm3 = (instr >> 12) & 0x7;
    let imm8 = instr & 0xFF;

    match op {
        // ADDW (0x20/0x60) and SUBW (0x2A/0x6A): imm12 = i:imm3:imm8.
        // The high opcode variants carry i = 1.
        // ADDW（0x20/0x60）与 SUBW（0x2A/0x6A）：imm12 = i:imm3:imm8，
        // 高位操作码变体携带 i = 1。
        0x20 | 0x60 | 0x2A | 0x6A => {
            let imm12 = (i << 11) | (imm3 << 8) | imm8;
            let result = if matches!(op, 0x20 | 0x60) {
                regs.read(rn).wrapping_add(imm12)
            } else {
                regs.read(rn).wrapping_sub(imm12)
            };
            regs.write(rd, result);
            StepResult::Ok
        }
        // MOVW (0x24/0x64) and MOVT (0x2C/0x6C): imm16 = imm4:i:imm3:imm8.
        // MOVW（0x24/0x64）与 MOVT（0x2C/0x6C）：imm16 = imm4:i:imm3:imm8。
        0x24 | 0x64 | 0x2C | 0x6C => {
            let imm16 = (rn as u32) << 12 | (i << 11) | (imm3 << 8) | imm8;
            let value = if matches!(op, 0x24 | 0x64) {
                imm16
            } else {
                (regs.read(rd) & 0xFFFF) | (imm16 << 16)
            };
            regs.write(rd, value);
            StepResult::Ok
        }
        _ => StepResult::Undefined,
    }
}

/// Execute bitfield instructions (SBFX/UBFX/BFI/BFC)
/// 执行位域指令（SBFX/UBFX/BFI/BFC）
///
/// `op == 0x34` is SBFX, `0x3C` is UBFX, and `0x36` is BFI/BFC depending on
/// whether Rn is 1111. The low 5 bits of the instruction are `widthm1` for
/// the extract instructions and `msb` for BFI/BFC.
/// `op == 0x34` 为 SBFX，`0x3C` 为 UBFX，`0x36` 根据 Rn 是否为 1111
/// 决定 BFI 还是 BFC。指令低 5 位在提取指令中为 `widthm1`，在 BFI/BFC
/// 中为 `msb`。
pub(super) fn execute_bitfield(instr: u32, regs: &mut CortexM33Registers) -> StepResult {
    let op = ((instr >> 20) & 0x7F) as u8;
    let rn = ((instr >> 16) & 0xF) as u8;
    let rd = ((instr >> 8) & 0xF) as u8;
    let lsb = (((instr >> 12) & 0x7) << 2) | ((instr >> 6) & 0x3);
    let low5 = instr & 0x1F;

    match op {
        // SBFX and UBFX: extract `widthm1 + 1` bits starting at `lsb`.
        // SBFX 与 UBFX：从 `lsb` 开始提取 `widthm1 + 1` 位。
        0x34 | 0x3C => {
            let width = low5 + 1;
            if lsb >= 32 || lsb + width > 32 {
                return StepResult::Undefined;
            }
            let mask = if width == 32 {
                u32::MAX
            } else {
                (1u32 << width) - 1
            };
            let extracted = (regs.read(rn) >> lsb) & mask;
            let result = if op == 0x34 {
                sign_extend(extracted, width)
            } else {
                extracted
            };
            regs.write(rd, result);
            StepResult::Ok
        }
        // BFI/BFC: clear bits [msb:lsb] then insert Rn (BFI only).
        // BFI/BFC：清除位 [msb:lsb]，然后插入 Rn（仅 BFI）。
        0x36 => {
            let msb = low5;
            if msb < lsb || msb > 31 {
                return StepResult::Undefined;
            }
            let width = msb - lsb + 1;
            let mask = if width == 32 {
                u32::MAX
            } else {
                ((1u32 << width) - 1) << lsb
            };
            let base = regs.read(rd);
            let result = if rn == 0xF {
                base & !mask
            } else {
                (base & !mask) | ((regs.read(rn) << lsb) & mask)
            };
            regs.write(rd, result);
            StepResult::Ok
        }
        _ => StepResult::Undefined,
    }
}
