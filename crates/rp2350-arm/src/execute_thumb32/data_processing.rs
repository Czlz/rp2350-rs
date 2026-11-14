//! Thumb-32 data-processing instructions
//! Thumb-32 数据处理指令
//!
//! Covers data-processing (modified immediate), data-processing (shifted
//! register) and data-processing (register), including logical operations,
//! add/subtract, shifts, MOV/MVN, compare/test, and the extend/reverse/
//! bit-count instructions.
//! 覆盖数据处理（改良立即数）、数据处理（移位寄存器）和数据处理（寄存器），
//! 包括逻辑运算、加减法、移位、MOV/MVN、比较/测试以及扩展/字节反转/位计数
//! 指令。

use crate::execute_thumb16::StepResult;
use crate::registers::CortexM33Registers;

use super::{
    ShiftType, add_with_carry, dsp, set_arith_flags, set_logic_flags, shift_immediate,
    shift_register, sign_extend, sub_with_carry,
};

/// Execute data-processing (modified immediate), `op1 == 0b10`, bit 25 clear
/// 执行数据处理（改良立即数），`op1 == 0b10`，位 25 清零
///
/// The opcode nibble is instruction `bits[24:21]` and the S bit is bit 20.
/// The 12-bit immediate is `i:imm3:imm8` (i = bit 26, imm3 = `bits[14:12]`,
/// imm8 = `bits[7:0]`) and is expanded by [`thumb_expand_imm`]. This group
/// also encodes `MOV.W` (ORR with Rn = 1111), `MVN.W` (ORN with Rn = 1111),
/// the flag-only forms `TST/TEQ/CMN/CMP` (Rd = 1111 with S = 1), and the
/// `ADD/SUB (SP) immediate` forms where Rn = Rd = 1101.
/// 操作码半字节为指令 `bits[24:21]`，S 位为位 20。12 位立即数为
/// `i:imm3:imm8`（i = 位 26，imm3 = `bits[14:12]`，imm8 = `bits[7:0]`），由
/// [`thumb_expand_imm`] 展开。该组还编码 `MOV.W`（Rn = 1111 的 ORR）、
/// `MVN.W`（Rn = 1111 的 ORN）、仅改标志的 `TST/TEQ/CMN/CMP`
///（Rd = 1111 且 S = 1）以及 Rn = Rd = 1101 的 `ADD/SUB (SP) immediate`。
pub(super) fn execute_modified_immediate(instr: u32, regs: &mut CortexM33Registers) -> StepResult {
    let opcode = ((instr >> 21) & 0xF) as u8;
    let setflags = (instr >> 20) & 1 != 0;
    let rn = ((instr >> 16) & 0xF) as u8;
    let rd = ((instr >> 8) & 0xF) as u8;
    let imm12 = (((instr >> 26) & 1) << 11) | (((instr >> 12) & 0x7) << 8) | (instr & 0xFF);
    let Some(imm) = thumb_expand_imm(imm12) else {
        return StepResult::Undefined;
    };
    let rn_value = regs.read(rn);

    if opcode <= 0x4 {
        // Logical operations. ORR with Rn = 15 is MOV; ORN with Rn = 15 is MVN.
        // Rd = 15 with S = 1 turns AND/EOR into TST/TEQ; other PC writes are
        // not permitted on Cortex-M.
        // 逻辑运算。Rn = 15 的 ORR 即 MOV；Rn = 15 的 ORN 即 MVN。
        // Rd = 15 且 S = 1 时 AND/EOR 变为 TST/TEQ；其余写 PC 形式在
        // Cortex-M 上不被允许。
        let result = match opcode {
            0x0 => rn_value & imm,
            0x1 => rn_value & !imm,
            0x2 => {
                if rn == 15 {
                    imm
                } else {
                    rn_value | imm
                }
            }
            0x3 => {
                if rn == 15 {
                    !imm
                } else {
                    rn_value | !imm
                }
            }
            _ => rn_value ^ imm,
        };
        let carry = regs.apsr_c();
        if rd == 15 {
            if !setflags || !matches!(opcode, 0x0 | 0x4) {
                return StepResult::Undefined;
            }
            set_logic_flags(regs, result, carry);
        } else {
            regs.write(rd, result);
            if setflags {
                set_logic_flags(regs, result, carry);
            }
        }
        return StepResult::Ok;
    }

    // Arithmetic operations. Rd = 15 with S = 1 gives CMP/CMN.
    // 算术运算。Rd = 15 且 S = 1 时为 CMP/CMN。
    let (result, carry, overflow) = match opcode {
        0x8 => add_with_carry(rn_value, imm, false),
        0xA => add_with_carry(rn_value, imm, regs.apsr_c()),
        0xB => sub_with_carry(rn_value, imm, regs.apsr_c()),
        0xD => sub_with_carry(rn_value, imm, true),
        0xE => sub_with_carry(imm, rn_value, true),
        _ => return StepResult::Undefined,
    };

    if rd == 15 {
        if !setflags || !matches!(opcode, 0x8 | 0xD) {
            return StepResult::Undefined;
        }
        set_arith_flags(regs, result, carry, overflow);
    } else {
        regs.write(rd, result);
        if setflags {
            set_arith_flags(regs, result, carry, overflow);
        }
    }
    StepResult::Ok
}

/// Expand the 12-bit modified immediate into a 32-bit constant
/// 将 12 位改良立即数展开为 32 位常量
///
/// When `imm12[11:10]` is zero, `imm12[9:8]` selects one of four byte
/// replication patterns. Otherwise the immediate is `1:imm12[6:0]` rotated
/// right by `imm12[11:7]`. Returns `None` for the UNPREDICTABLE encodings
/// where a replication pattern is selected but the byte is zero.
/// 当 `imm12[11:10]` 为零时，`imm12[9:8]` 选择四种字节复制模式之一；否则
/// 立即数为 `1:imm12[6:0]` 按 `imm12[11:7]` 循环右移。若选择了复制模式
/// 但字节为零（不可预测编码）则返回 `None`。
fn thumb_expand_imm(imm12: u32) -> Option<u32> {
    if imm12 & 0xC00 == 0 {
        let select = (imm12 >> 8) & 0x3;
        let byte = imm12 & 0xFF;
        match select {
            0 => Some(byte),
            1 if byte != 0 => Some((byte << 16) | byte),
            2 if byte != 0 => Some((byte << 24) | (byte << 8)),
            3 if byte != 0 => Some((byte << 24) | (byte << 16) | (byte << 8) | byte),
            _ => None,
        }
    } else {
        let unrotated = 0x80 | (imm12 & 0x7F);
        let rotation = (imm12 >> 7) & 0x1F;
        Some(unrotated.rotate_right(rotation))
    }
}

/// Execute data-processing (shifted register), `op1 == 0b01`, `op >= 0x20`
/// 执行数据处理（移位寄存器），`op1 == 0b01`，`op >= 0x20`
///
/// The opcode nibble is instruction bits [24:21] and the S bit is bit 20.
/// 操作码半字节为指令位 [24:21]，S 位为位 20。
pub(super) fn execute_shifted_register(instr: u32, regs: &mut CortexM33Registers) -> StepResult {
    let op_field = (instr >> 20) & 0x7F;
    let opcode = (op_field >> 1) & 0xF;
    let setflags = op_field & 1 != 0;
    let rn = ((instr >> 16) & 0xF) as u8;
    let rd = ((instr >> 8) & 0xF) as u8;
    let rm = (instr & 0xF) as u8;
    let amount = (((instr >> 12) & 0x7) << 2) | ((instr >> 6) & 0x3);
    let ty = ShiftType::from_bits((instr >> 4) & 0x3);

    let (operand2, shift_carry) = shift_immediate(regs.read(rm), amount, ty, regs.apsr_c());
    let rn_value = regs.read(rn);

    if opcode <= 0x4 {
        // Logical operations. ORR with Rn = 15 is MOV; ORN with Rn = 15 is MVN.
        // 逻辑运算。Rn = 15 的 ORR 即 MOV；Rn = 15 的 ORN 即 MVN。
        let result = match opcode {
            0x0 => rn_value & operand2,
            0x1 => rn_value & !operand2,
            0x2 => {
                if rn == 15 {
                    operand2
                } else {
                    rn_value | operand2
                }
            }
            0x3 => {
                if rn == 15 {
                    !operand2
                } else {
                    rn_value | !operand2
                }
            }
            _ => rn_value ^ operand2,
        };

        if rd == 15 {
            // TST/TEQ: flags only, no register write.
            // TST/TEQ：仅更新标志，不写寄存器。
            if setflags {
                set_logic_flags(regs, result, shift_carry);
                StepResult::Ok
            } else {
                StepResult::Undefined
            }
        } else {
            regs.write(rd, result);
            if setflags {
                set_logic_flags(regs, result, shift_carry);
            }
            StepResult::Ok
        }
    } else {
        // Arithmetic operations. Rd = 15 with S = 1 gives CMP/CMN.
        // 算术运算。Rd = 15 且 S = 1 时为 CMP/CMN。
        let (result, carry, overflow) = match opcode {
            0x8 => add_with_carry(rn_value, operand2, false),
            0xA => add_with_carry(rn_value, operand2, regs.apsr_c()),
            0xB => sub_with_carry(rn_value, operand2, regs.apsr_c()),
            0xD => sub_with_carry(rn_value, operand2, true),
            0xE => sub_with_carry(operand2, rn_value, true),
            _ => return StepResult::Undefined,
        };

        if rd == 15 {
            if !setflags {
                return StepResult::Undefined;
            }
            set_arith_flags(regs, result, carry, overflow);
        } else {
            regs.write(rd, result);
            if setflags {
                set_arith_flags(regs, result, carry, overflow);
            }
        }
        StepResult::Ok
    }
}

/// Execute data-processing (register), `op1 == 0b11`, `op` in `0x20..=0x2F`
/// 执行数据处理（寄存器），`op1 == 0b11`，`op` 在 `0x20..=0x2F`
///
/// This group contains register-controlled shifts, the extend instructions
/// (SXTB/SXTH/UXTB/UXTH), the byte-reverse instructions (REV/REV16/REVSH),
/// RBIT, and CLZ. They share the same top-level opcode range and are
/// distinguished by the fixed fields `Rn = 1111` and second-halfword bits.
/// 该组包含寄存器控制移位、扩展指令（SXTB/SXTH/UXTB/UXTH）、
/// 字节反转指令（REV/REV16/REVSH）、RBIT 和 CLZ。它们共享同一顶层操作码
/// 范围，通过固定字段 `Rn = 1111` 与第二半字位区分。
pub(super) fn execute_register_shifted_or_misc(
    instr: u32,
    regs: &mut CortexM33Registers,
) -> StepResult {
    let bits23_20 = ((instr >> 20) & 0xF) as u8;
    let rn_field = ((instr >> 16) & 0xF) as u8;
    let rd = ((instr >> 8) & 0xF) as u8;
    let hw2_bits7_4 = ((instr >> 4) & 0xF) as u8;
    let rm = (instr & 0xF) as u8;

    // Register-controlled shifts require bits[15:12] = 1111.
    // 寄存器控制移位要求 bits[15:12] = 1111。
    let fixed_hi = ((instr >> 12) & 0xF) == 0xF;

    // Packed SIMD parallel add/subtract shares this top-level opcode range but
    // occupies a disjoint encoding space.
    // 打包 SIMD 并行加/减共享该顶层操作码范围，但编码空间互不重叠。
    if let Some(result) = dsp::try_execute(instr, regs) {
        return result;
    }

    // Parallel byte extend: SXTB16/UXTB16 sign- or zero-extend bytes[7:0] and
    // [23:16] into two halfwords, honouring the same rotate field as SXTB.
    // Flags are not modified by these two encodings.
    // 并行字节扩展：SXTB16/UXTB16 把 bytes[7:0] 与 [23:16] 符号/零扩展到两个
    // 半字，rotate 字段与 SXTB 一致。这两条编码不更新标志。
    if fixed_hi && matches!(bits23_20, 0x2 | 0x3) && (0x8..=0xB).contains(&hw2_bits7_4) {
        // SXTAB16/UXTAB16 (Rn != 1111) are not modeled.
        // 不建模 SXTAB16/UXTAB16（Rn != 1111）。
        if rn_field != 0xF || rd == 15 {
            return StepResult::Undefined;
        }
        let rotate = ((instr >> 4) & 0x3) * 8;
        let value = regs.read(rm).rotate_right(rotate);
        let result = if bits23_20 == 0x2 {
            let low = sign_extend(value & 0xFF, 8) & 0xFFFF;
            let high = (sign_extend((value >> 16) & 0xFF, 8) & 0xFFFF) << 16;
            high | low
        } else {
            // UXTB16 keeps bytes 0 and 2 in place; bytes 1 and 3 are dropped.
            // UXTB16 保留字节 0 与 2 的位置，丢弃字节 1 与 3。
            (value & 0xFF) | (value & 0x00FF_0000)
        };
        regs.write(rd, result);
        return StepResult::Ok;
    }

    // Extend instructions: Rn determines the plain or accumulating form and
    // second-halfword bits[7:4] = 0b10xx carry the rotate amount in bits[5:4].
    // 扩展指令：Rn 决定普通或累加形式，第二半字 bits[7:4] = 0b10xx 的
    // bits[5:4] 携带循环移位量。
    if fixed_hi && (0x8..=0xB).contains(&hw2_bits7_4) && matches!(bits23_20, 0x0 | 0x1 | 0x4 | 0x5)
    {
        let rotate = ((instr >> 4) & 0x3) * 8;
        let value = regs.read(rm).rotate_right(rotate);
        let extended = match bits23_20 {
            0x0 => sign_extend(value, 16), // SXTH / SXTAH
            0x1 => value & 0xFFFF,         // UXTH / UXTAH
            0x4 => sign_extend(value, 8),  // SXTB / SXTAB
            _ => value & 0xFF,             // UXTB / UXTAB
        };
        let result = if rn_field == 0xF {
            // Plain SXTB/SXTH/UXTB/UXTH forms update N and Z (C and V are
            // preserved); the accumulating forms below leave flags unchanged.
            // ARMv8-M ARM: SXTB (register) T2, SXTH (register) T2,
            // UXTB (register) T2 and UXTH (register) T2.
            // 普通 SXTB/SXTH/UXTB/UXTH 形式更新 N 和 Z（C 与 V 保持不变）；
            // 下面的累加形式不改变标志。
            // ARMv8-M 架构手册：SXTB（寄存器）T2、SXTH（寄存器）T2、
            // UXTB（寄存器）T2 与 UXTH（寄存器）T2。
            regs.set_nz_flags(extended);
            extended
        } else {
            regs.read(rn_field).wrapping_add(extended)
        };
        regs.write(rd, result);
        return StepResult::Ok;
    }

    match bits23_20 {
        // Register-controlled shifts: LSL/LSR/ASR/ROR, S bit in bit 20.
        // 寄存器控制移位：LSL/LSR/ASR/ROR，S 位在位 20。
        0x0..=0x7 => {
            if !fixed_hi || hw2_bits7_4 != 0x0 {
                return StepResult::Undefined;
            }
            let ty = ShiftType::from_bits((instr >> 21) & 0x3);
            let setflags = (instr >> 20) & 1 != 0;
            let (result, carry) =
                shift_register(regs.read(rn_field), regs.read(rm), ty, regs.apsr_c());
            regs.write(rd, result);
            if setflags {
                set_logic_flags(regs, result, carry);
            }
            StepResult::Ok
        }
        // Byte-reverse group: REV/REV16/RBIT/REVSH, selected by bits[7:4].
        // 字节反转组：REV/REV16/RBIT/REVSH，由 bits[7:4] 选择。
        0x9 => {
            if !fixed_hi || rn_field != rm {
                return StepResult::Undefined;
            }
            let value = regs.read(rm);
            let result = match hw2_bits7_4 {
                0x8 => value.swap_bytes(),
                0x9 => {
                    let lo = (value & 0x00FF_00FF) << 8;
                    let hi = (value >> 8) & 0x00FF_00FF;
                    lo | hi
                }
                0xA => value.reverse_bits(),
                0xB => sign_extend(((value & 0xFF) << 8) | ((value >> 8) & 0xFF), 16),
                _ => return StepResult::Undefined,
            };
            regs.write(rd, result);
            StepResult::Ok
        }
        // CLZ: count leading zeros of Rm.
        // CLZ：计算 Rm 的前导零数量。
        0xB => {
            if !fixed_hi || hw2_bits7_4 != 0x8 || rn_field != rm {
                return StepResult::Undefined;
            }
            let result = regs.read(rm).leading_zeros();
            regs.write(rd, result);
            StepResult::Ok
        }
        _ => StepResult::Undefined,
    }
}
