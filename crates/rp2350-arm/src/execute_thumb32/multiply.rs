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
