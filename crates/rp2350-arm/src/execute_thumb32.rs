//! Thumb-32 instruction decoder and executor
//! Thumb-32 指令解码器和执行器
//!
//! Decodes and executes Thumb-32 (32-bit) instructions. Currently covers
//! the most common instructions: BL, MOVW, MOVT, ADDW, SUBW, LDR/STR
//! with 12-bit offset, and some data processing modified immediate.
//! Full DSP/FPU support is deferred to a later phase.
//! 解码并执行 Thumb-32（32 位）指令。当前覆盖最常见的指令：
//! BL、MOVW、MOVT、ADDW、SUBW、LDR/STR（12 位偏移）和一些修改立即数
//! 数据处理。完整的 DSP/FPU 支持延后到后续阶段。

// 指令解码器涉及大量位操作和类型转换，这是 ARM 指令集仿真的固有特性。
// Instruction decoders involve extensive bit manipulation and casts,
// inherent to ARM ISA emulation.
#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::missing_errors_doc,
    clippy::doc_markdown,
    clippy::module_name_repetitions,
    clippy::unusual_byte_groupings,
    clippy::unreadable_literal,
    clippy::bool_to_int_with_if,
    clippy::unnecessary_cast,
    clippy::collapsible_if
)]

use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;

use super::conditions::ITBlockState;
use super::execute_thumb16::StepResult;
use super::nvic::Nvic;
use super::registers::CortexM33Registers;
use super::systick::SysTick;

/// Decode and execute one Thumb-32 instruction (32-bit)
/// 解码并执行一条 Thumb-32 指令（32 位）
///
/// Thumb-32 instructions are identified by opcode\[15:11\] = 11101, 11110, or 11111.
/// The full 32-bit instruction is formed by joining two 16-bit halfwords.
/// Thumb-32 指令通过 opcode\[15:11\] = 11101、11110 或 11111 识别。
/// 完整的 32 位指令由两个 16 位半字连接而成。
pub fn execute_thumb32(
    instr: u32,
    regs: &mut CortexM33Registers,
    bus: &mut dyn AddressBus,
    _nvic: &mut Nvic,
    _systick: &mut SysTick,
    it: &mut ITBlockState,
    cycles: &mut u64,
) -> Result<StepResult, CoreError> {
    // IT block condition check for 32-bit instructions
    // 32 位指令的 IT 块条件检查
    if it.in_it_block() {
        let cond = it.advance();
        if !cond.evaluate(regs) {
            *cycles += 1;
            return Ok(StepResult::Ok);
        }
    }

    let op1 = (instr >> 27) & 0x3; // bits [28:27]
    let op = (instr >> 20) & 0x7F; // bits [26:20]
    let s = (instr >> 20) & 0x1; // S flag (set flags)

    match (op1, op) {
        // BL (Branch with Link)
        (0b10 | 0b11, 0b0111101 | 0b0111111) => {
            let s_bit = (instr >> 26) & 0x1;
            let imm10 = (instr >> 16) & 0x3FF;
            let j1 = (instr >> 13) & 0x1;
            let j2 = (instr >> 11) & 0x1;
            let imm11 = instr & 0x7FF;

            if s_bit == 0 {
                // BL (not BLX)
                let s = (instr >> 26) & 0x1;
                let i1 = !(j1 ^ s) & 1;
                let i2 = !(j2 ^ s) & 1;
                let offset = (s << 24) | (i1 << 23) | (i2 << 22) | (imm10 << 12) | (imm11 << 1);
                let mut offset_32 = offset;
                // Sign-extend to 32 bits
                if offset_32 & 0x0100_0000 != 0 {
                    offset_32 |= 0xFE00_0000;
                }
                regs.lr = (regs.pc - 2) | 1; // Return address
                regs.pc = regs.pc.wrapping_add(offset_32);
                return Ok(StepResult::BranchTaken);
            }
        }

        // Data processing (modified immediate) — MOVW, MOVT, ADDW, SUBW
        (0b10, 0b0000000..=0b0001111) => {
            let rn = ((instr >> 16) & 0xF) as usize;
            let rd = ((instr >> 8) & 0xF) as usize;
            let i = (instr >> 26) & 0x1;
            let imm3 = ((instr >> 12) & 0x7) as u32;
            let imm8 = (instr & 0xFF) as u32;
            let imm = (i << 11) | (imm3 << 8) | imm8;

            match (instr >> 21) & 0xF {
                0b0000 if rn == 0xF => {
                    // MOVW Rd, #imm16 / 移动到 Rd 低 16 位（零扩展）
                    regs.r[rd] = imm & 0xFFFF;
                }
                0b0100 if rn == 0xF => {
                    // MOVT Rd, #imm16 / 移动到 Rd 高 16 位
                    regs.r[rd] = (regs.r[rd] & 0xFFFF) | ((imm & 0xFFFF) << 16);
                }
                _ if (instr >> 23) & 0x1 == 0 => {
                    // ADDW Rd, Rn, #imm12 / 加法宽立即数
                    let result = regs.r[rn].wrapping_add(imm);
                    regs.r[rd] = result;
                }
                _ => {
                    // SUBW Rd, Rn, #imm12 / 减法宽立即数
                    let result = regs.r[rn].wrapping_sub(imm);
                    regs.r[rd] = result;
                }
            }
        }

        // Data processing (register) with 4 registers
        (0b10, 0b0101100 | 0b0101101) => {
            let rn = ((instr >> 16) & 0xF) as usize;
            let rd = ((instr >> 8) & 0xF) as usize;
            let rm = ((instr >> 0) & 0xF) as usize;
            let ra = ((instr >> 12) & 0xF) as usize;
            let op2 = (instr >> 4) & 0xF; // [7:4]

            match op2 {
                // MLA: Rd = Ra + Rn * Rm
                0b0000 => {
                    let result = regs.r[ra].wrapping_add(regs.r[rn].wrapping_mul(regs.r[rm]));
                    regs.r[rd] = result;
                    if s != 0 {
                        regs.set_nz_flags(result);
                    }
                }
                // MLS: Rd = Ra - Rn * Rm
                0b0001 => {
                    let result = regs.r[ra].wrapping_sub(regs.r[rn].wrapping_mul(regs.r[rm]));
                    regs.r[rd] = result;
                    if s != 0 {
                        regs.set_nz_flags(result);
                    }
                }
                _ => return Ok(StepResult::Undefined),
            }
        }

        // Load/store word/byte (immediate, 12-bit offset)
        (0b10, 0b0011000..=0b0011011) => {
            let rn = ((instr >> 16) & 0xF) as usize;
            let rt = ((instr >> 12) & 0xF) as usize;
            let imm12 = instr & 0xFFF;
            let op_type = (instr >> 20) & 0xF;

            // Determine pre-index vs post-index
            let p = (instr >> 24) & 0x1 == 1; // Pre-indexed (use offset before access)
            let u = (instr >> 23) & 0x1 == 1; // Add offset (+ vs -)
            let w = (instr >> 21) & 0x1 == 1; // Write-back

            let offset = if u { imm12 } else { 0u32.wrapping_sub(imm12) };
            let addr = if p {
                regs.r[rn].wrapping_add(offset)
            } else {
                regs.r[rn]
            };

            match op_type {
                // STRB, LDRB
                0b1000 => {
                    // STRB (immediate)
                    bus.write_u8(addr, regs.r[rt] as u8, cycles)?;
                    if w {
                        regs.r[rn] = regs.r[rn].wrapping_add(offset);
                    }
                }
                0b1001 => {
                    // LDRB (immediate)
                    regs.r[rt] = bus.read_u8(addr, cycles)? as u32;
                    if w {
                        regs.r[rn] = regs.r[rn].wrapping_add(offset);
                    }
                }
                // STRH, LDRH
                0b1010 => {
                    // STRH (immediate)
                    bus.write_u16(addr, regs.r[rt] as u16, cycles)?;
                    if w {
                        regs.r[rn] = regs.r[rn].wrapping_add(offset);
                    }
                }
                0b1011 => {
                    // LDRH (immediate)
                    regs.r[rt] = bus.read_u16(addr, cycles)? as u32;
                    if w {
                        regs.r[rn] = regs.r[rn].wrapping_add(offset);
                    }
                }
                // STR, LDR
                0b1100 => {
                    // STR (immediate)
                    bus.write_u32(addr, regs.r[rt], cycles)?;
                    if w {
                        regs.r[rn] = regs.r[rn].wrapping_add(offset);
                    }
                }
                0b1101 => {
                    // LDR (immediate)
                    let val = bus.read_u32(addr, cycles)?;
                    regs.r[rt] = val;
                    if w {
                        regs.r[rn] = regs.r[rn].wrapping_add(offset);
                    }
                    if rt == 15 {
                        regs.pc &= !1u32;
                        return Ok(StepResult::BranchTaken);
                    }
                }
                _ => return Ok(StepResult::Undefined),
            }
            *cycles += 1;
        }

        // Undefined/unhandled 32-bit instruction
        _ => {
            return Ok(StepResult::Undefined);
        }
    }

    *cycles += 1;
    Ok(StepResult::Ok)
}