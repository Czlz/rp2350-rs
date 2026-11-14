//! Thumb-32 multiply and divide instructions
//! Thumb-32 乘法与除法指令
//!
//! Covers MUL/MLA/MLS, the long multiplies SMULL/UMULL/SMLAL/UMLAL, and the
//! signed/unsigned divide instructions.
//! 覆盖 MUL/MLA/MLS、长乘法 SMULL/UMULL/SMLAL/UMLAL，以及有符号/无符号除法。

use crate::execute_thumb16::StepResult;
use crate::registers::CortexM33Registers;

/// Execute a multiply/divide instruction, `op1 == 0b11`, `op` in `0x30..=0x3F`
/// 执行乘除指令，`op1 == 0b11`，`op` 在 `0x30..=0x3F`
///
/// Instruction bits [23:20] select the operation; the long multiplies and
/// divides use bits [15:12] and [11:8] as the two destination registers.
/// 指令位 [23:20] 选择操作；长乘法和除法使用位 [15:12] 与 [11:8]
/// 作为两个目标寄存器。
pub(super) fn execute(instr: u32, regs: &mut CortexM33Registers) -> StepResult {
    let sub = ((instr >> 20) & 0xF) as u8;
    let rn = ((instr >> 16) & 0xF) as u8;
    let ra_or_lo = ((instr >> 12) & 0xF) as u8;
    let rd = ((instr >> 8) & 0xF) as u8;
    let bits7_4 = ((instr >> 4) & 0xF) as u8;
    let rm = (instr & 0xF) as u8;

    match sub {
        // MUL/MLA when bits[7:4] = 0, MLS when bits[7:4] = 1.
        // bits[7:4] = 0 时为 MUL/MLA，bits[7:4] = 1 时为 MLS。
        0x0 => {
            let product = regs.read(rn).wrapping_mul(regs.read(rm));
            let result = match bits7_4 {
                0x0 => {
                    if ra_or_lo == 0xF {
                        product
                    } else {
                        regs.read(ra_or_lo).wrapping_add(product)
                    }
                }
                0x1 => regs.read(ra_or_lo).wrapping_sub(product),
                _ => return StepResult::Undefined,
            };
            regs.write(rd, result);
            StepResult::Ok
        }
        // SMULxy/SMLAxy (DSP extension): signed 16x16 multiply of selected
        // halfwords. `bits7_4` bit 1 selects the high half of Rn, bit 0 the
        // high half of Rm; Ra = 15 selects the non-accumulating SMULxy form.
        // SMULxy/SMLAxy（DSP 扩展）：选取半字做有符号 16×16 乘法。
        // `bits7_4` 位 1 选择 Rn 的高半字，位 0 选择 Rm 的高半字；
        // Ra = 15 表示非累加的 SMULxy 形式。
        0x1 => {
            let n_high = bits7_4 & 0b10 != 0;
            let m_high = bits7_4 & 0b01 != 0;
            let n_half = if n_high {
                i32::from((regs.read(rn) >> 16) as i16)
            } else {
                i32::from(regs.read(rn) as i16)
            };
            let m_half = if m_high {
                i32::from((regs.read(rm) >> 16) as i16)
            } else {
                i32::from(regs.read(rm) as i16)
            };
            let product = n_half.wrapping_mul(m_half);
            if ra_or_lo == 0xF {
                regs.write(rd, product as u32);
            } else {
                let exact = i64::from(regs.read(ra_or_lo) as i32) + i64::from(product);
                let accumulated = exact as i32;
                if exact != i64::from(accumulated) {
                    regs.set_q_flag();
                }
                regs.write(rd, accumulated as u32);
            }
            StepResult::Ok
        }
        // SMMLA/SMMUL (DSP extension): high 32 bits of a signed 32x32 product,
        // optionally rounded and accumulated. Ra = 15 selects SMMUL.
        // SMMLA/SMMUL（DSP 扩展）：有符号 32×32 乘积的高 32 位，可选舍入与
        // 累加。Ra = 15 表示 SMMUL。
        0x5 => {
            let product = i64::from(regs.read(rn) as i32) * i64::from(regs.read(rm) as i32);
            let low = product as u32;
            let mut high = (product >> 32) as i32;
            if bits7_4 & 1 != 0 && low >= 0x8000_0000 {
                high = high.wrapping_add(1);
            }
            let accumulated = if ra_or_lo == 0xF {
                high
            } else {
                high.wrapping_add(regs.read(ra_or_lo) as i32)
            };
            regs.write(rd, accumulated as u32);
            StepResult::Ok
        }
        // USAD8/USADA8 (DSP extension): sum of the absolute differences of the
        // four byte lanes. Ra = 15 selects the non-accumulating USAD8 form.
        // Flags are not modified.
        // USAD8/USADA8（DSP 扩展）：四个字节通道绝对差之和。Ra = 15 表示
        // 非累加的 USAD8 形式。不更新标志。
        0x7 => {
            if bits7_4 != 0x0 {
                return StepResult::Undefined;
            }
            if rd == 15 {
                return StepResult::Undefined;
            }
            let a = regs.read(rn);
            let b = regs.read(rm);
            let mut sum = 0u32;
            for i in 0..4u32 {
                let a_lane = (a >> (i * 8)) & 0xFF;
                let b_lane = (b >> (i * 8)) & 0xFF;
                sum += a_lane.abs_diff(b_lane);
            }
            let result = if ra_or_lo == 0xF {
                sum
            } else {
                regs.read(ra_or_lo).wrapping_add(sum)
            };
            regs.write(rd, result);
            StepResult::Ok
        }
        // SMULL: signed 64-bit product, low word in bits[15:12], high in [11:8].
        // SMULL：有符号 64 位乘积，低位字写入 bits[15:12]，高位字写入 [11:8]。
        0x8 => {
            let product = i64::from(regs.read(rn) as i32) * i64::from(regs.read(rm) as i32);
            regs.write(ra_or_lo, product as u32);
            regs.write(rd, (product >> 32) as u32);
            StepResult::Ok
        }
        // UMULL: unsigned 64-bit product.
        // UMULL：无符号 64 位乘积。
        0xA => {
            let product = u64::from(regs.read(rn)) * u64::from(regs.read(rm));
            regs.write(ra_or_lo, product as u32);
            regs.write(rd, (product >> 32) as u32);
            StepResult::Ok
        }
        // SMLAL: signed 64-bit accumulate.
        // SMLAL：有符号 64 位累加。
        0xC => {
            let accumulator = (u64::from(regs.read(rd)) << 32) | u64::from(regs.read(ra_or_lo));
            let product = i64::from(regs.read(rn) as i32) * i64::from(regs.read(rm) as i32);
            let result = (accumulator as i64).wrapping_add(product) as u64;
            regs.write(ra_or_lo, result as u32);
            regs.write(rd, (result >> 32) as u32);
            StepResult::Ok
        }
        // UMLAL: unsigned 64-bit accumulate.
        // UMLAL：无符号 64 位累加。
        0xE => {
            let accumulator = (u64::from(regs.read(rd)) << 32) | u64::from(regs.read(ra_or_lo));
            let product = u64::from(regs.read(rn)) * u64::from(regs.read(rm));
            let result = accumulator.wrapping_add(product);
            regs.write(ra_or_lo, result as u32);
            regs.write(rd, (result >> 32) as u32);
            StepResult::Ok
        }
        0x9 => {
            if bits7_4 != 0xF {
                return StepResult::Undefined;
            }
            let divisor = regs.read(rm) as i32;
            let result = (regs.read(rn) as i32)
                .checked_div(divisor)
                .unwrap_or(if divisor == 0 { 0 } else { i32::MIN });
            regs.write(rd, result as u32);
            StepResult::Ok
        }
        0xB => {
            if bits7_4 != 0xF {
                return StepResult::Undefined;
            }
            let divisor = regs.read(rm);
            let result = regs.read(rn).checked_div(divisor).unwrap_or(0);
            regs.write(rd, result);
            StepResult::Ok
        }
        _ => StepResult::Undefined,
    }
}
