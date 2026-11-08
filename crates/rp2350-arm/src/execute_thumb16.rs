//! Thumb-16 instruction decoder and executor
//! Thumb-16 指令解码器和执行器
//!
//! Decodes and executes all Thumb-16 (16-bit) instructions defined by ARMv8-M.
//! Each instruction is decoded from a 16-bit opcode and executed with register
//! and bus access side-effects.
//! 解码并执行 ARMv8-M 定义的所有 Thumb-16（16 位）指令。
//! 每条指令从 16 位操作码解码，并执行寄存器和总线访问的副作用。

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
    clippy::collapsible_if,
    clippy::absurd_extreme_comparisons,
    clippy::cast_possible_wrap,
    clippy::manual_rotate,
    clippy::no_effect_underscore_binding,
    clippy::match_same_arms,
    clippy::single_match_else,
    clippy::similar_names
)]

use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;

use super::conditions::{Condition, ITBlockState};
use super::nvic::Nvic;
use super::registers::CortexM33Registers;
use super::systick::SysTick;

// === Helper functions ===

/// Sign-extend a value from `bits` bits to 32 bits
/// 将值从 `bits` 位符号扩展到 32 位
fn sign_extend(value: u32, bits: u32) -> u32 {
    let shift = 32 - bits;
    ((value << shift) as i32 >> shift) as u32
}

/// Perform a logical shift left: value << shift, with APSR carry
/// 执行逻辑左移：value << shift，带 APSR 进位
fn lsl_c(value: u32, shift: u32, regs: &mut CortexM33Registers) -> u32 {
    if shift == 0 {
        regs.set_c_flag(false);
        return value;
    }
    if shift >= 32 {
        regs.set_c_flag(if shift == 32 { value & 1 != 0 } else { false });
        return 0;
    }
    regs.set_c_flag((value << (shift - 1)) & 0x8000_0000 != 0);
    value << shift
}

/// Perform a logical shift right: value >> shift, with APSR carry
/// 执行逻辑右移：value >> shift，带 APSR 进位
fn lsr_c(value: u32, shift: u32, regs: &mut CortexM33Registers) -> u32 {
    if shift == 0 {
        regs.set_c_flag(false);
        return value;
    }
    if shift >= 32 {
        regs.set_c_flag(if shift == 32 {
            value & 0x8000_0000 != 0
        } else {
            false
        });
        return 0;
    }
    regs.set_c_flag((value >> (shift - 1)) & 1 != 0);
    value >> shift
}

/// Perform an arithmetic shift right: value >> shift (sign-extending), with APSR carry
/// 执行算术右移：value >> shift（符号扩展），带 APSR 进位
fn asr_c(value: u32, shift: u32, regs: &mut CortexM33Registers) -> u32 {
    if shift == 0 || shift >= 32 {
        if shift >= 32 {
            regs.set_c_flag(value & 0x8000_0000 != 0);
            return if value & 0x8000_0000 != 0 {
                0xFFFF_FFFF
            } else {
                0
            };
        }
        regs.set_c_flag(false);
        return value;
    }
    let result = ((value as i32) >> shift) as u32;
    regs.set_c_flag((value >> (shift - 1)) & 1 != 0);
    result
}

/// Rotate right: value ROR shift, with APSR carry
/// 循环右移：value ROR shift，带 APSR 进位
fn ror_c(value: u32, shift: u32, regs: &mut CortexM33Registers) -> u32 {
    if shift == 0 {
        regs.set_c_flag(value & 0x8000_0000 != 0);
        return value;
    }
    let shift = shift & 0x1F; // Shift mod 32
    if shift == 0 {
        regs.set_c_flag(value & 0x8000_0000 != 0);
        return value;
    }
    let result = (value >> shift) | (value << (32 - shift));
    regs.set_c_flag(result & 0x8000_0000 != 0);
    result
}

/// Add with carry: Rd = Rn + Rm + C
/// 带进位加法：Rd = Rn + Rm + C
fn adc(result: u64, regs: &mut CortexM33Registers, setflags: bool) -> u32 {
    let result32 = result as u32;
    if setflags {
        let _n = (result32 & 0x8000_0000) != 0;
        let _z = result32 == 0;
        let c = result > 0xFFFF_FFFF;
        // Overflow: operands same sign, result different sign
        // 溢出：操作数同号，结果异号
        regs.set_c_flag(c);
        regs.set_nz_flags(result32);
    }
    result32
}

/// Subtract with carry: Rd = Rn - Rm - !C
/// 带借位减法：Rd = Rn - Rm - !C
fn sbc(result: u64, regs: &mut CortexM33Registers, setflags: bool) -> u32 {
    let result32 = result as u32;
    if setflags {
        let _n = (result32 & 0x8000_0000) != 0;
        let _z = result32 == 0;
        // Borrow = !(result > 0xFFFF_FFFF) → C = result <= 0xFFFF_FFFF
        // 借位 = !(result > 0xFFFF_FFFF) → C = result <= 0xFFFF_FFFF
        regs.set_c_flag(result <= 0xFFFF_FFFF);
        regs.set_nz_flags(result32);
    }
    result32
}

// === Decoder/Executor ===

/// Result of executing one Thumb-16 instruction
/// 执行一条 Thumb-16 指令的结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepResult {
    /// Normal execution, consumed 1 cycle
    /// 正常执行，消耗 1 个周期
    Ok,
    /// SVC instruction — needs supervisor call handling
    /// SVC 指令 — 需要超级调用处理
    SvCall(u8),
    /// BKPT instruction — breakpoint hit
    /// BKPT 指令 — 触发断点
    Breakpoint(u8),
    /// Branch taken (new PC set)
    /// 分支已执行（设置了新 PC）
    BranchTaken,
    /// WFI/WFE — enter sleep state
    /// WFI/WFE — 进入睡眠状态
    Sleep,
    /// Exception return (BX LR with EXC_RETURN value)
    /// 异常返回（带 EXC_RETURN 值的 BX LR）
    ExceptionReturn,
    /// Undefined instruction
    /// 未定义指令
    Undefined,
}

/// Decode and execute one Thumb-16 instruction
/// 解码并执行一条 Thumb-16 指令
///
/// # Arguments
/// * `opcode` - 16-bit instruction word
/// * `regs` - CPU registers
/// * `bus` - Address bus for memory access
/// * `nvic` - NVIC for interrupt control
/// * `systick` - SysTick timer (for tick-based instructions)
/// * `it` - IT block state (for conditional execution)
/// * `cycles` - Cycle counter
pub fn execute_thumb16(
    opcode: u16,
    regs: &mut CortexM33Registers,
    bus: &mut dyn AddressBus,
    _nvic: &mut Nvic,
    _systick: &mut SysTick,
    it: &mut ITBlockState,
    cycles: &mut u64,
) -> Result<StepResult, CoreError> {
    // IT block condition check
    // IT 块条件检查
    if it.in_it_block() {
        let cond = it.advance();
        if !cond.evaluate(regs) {
            // Instruction in IT block is not executed
            // IT 块中的指令不执行
            *cycles += 1;
            return Ok(StepResult::Ok);
        }
    }

    let instr_type = (opcode >> 11) & 0x1F; // bits [15:11]

    match instr_type {
        0b00000 => {
            // LSLS Rd, Rm, #imm5 / 逻辑左移立即数
            let imm5 = ((opcode >> 6) & 0x1F) as u32;
            let rm = ((opcode >> 3) & 0x7) as usize;
            let rd = (opcode & 0x7) as usize;
            let result = lsl_c(regs.r[rm], imm5, regs);
            regs.r[rd] = result;
            regs.set_nz_flags(result);
        }
        0b00001 => {
            // LSRS Rd, Rm, #imm5 (imm5 == 0 encodes 32) / 逻辑右移立即数
            let imm5 = ((opcode >> 6) & 0x1F) as u32;
            let rm = ((opcode >> 3) & 0x7) as usize;
            let rd = (opcode & 0x7) as usize;
            let shift = if imm5 == 0 { 32 } else { imm5 };
            let result = lsr_c(regs.r[rm], shift, regs);
            regs.r[rd] = result;
            regs.set_nz_flags(result);
        }
        0b00010 => {
            // ASRS Rd, Rm, #imm5 (imm5 == 0 encodes 32) / 算术右移立即数
            let imm5 = ((opcode >> 6) & 0x1F) as u32;
            let rm = ((opcode >> 3) & 0x7) as usize;
            let rd = (opcode & 0x7) as usize;
            let shift = if imm5 == 0 { 32 } else { imm5 };
            let result = asr_c(regs.r[rm], shift, regs);
            regs.r[rd] = result;
            regs.set_nz_flags(result);
        }
        0b00011 => {
            // ADDS/SUBS (register) and ADDS/SUBS (3-bit immediate)
            // 寄存器或 3 位立即数加减
            let immediate = (opcode >> 10) & 0x1 != 0;
            let subtract = (opcode >> 9) & 0x1 != 0;
            let rn = ((opcode >> 3) & 0x7) as usize;
            let rd = (opcode & 0x7) as usize;
            let operand = if immediate {
                ((opcode >> 6) & 0x7) as u32
            } else {
                regs.r[((opcode >> 6) & 0x7) as usize]
            };
            let result = if subtract {
                regs.r[rn].wrapping_sub(operand)
            } else {
                regs.r[rn].wrapping_add(operand)
            };
            regs.r[rd] = result;
            regs.set_nz_flags(result);
        }
        0b00100 => {
            // MOVS Rd, #imm8 / 立即数移动到 Rd
            let rd = ((opcode >> 8) & 0x7) as usize;
            let imm8 = (opcode & 0xFF) as u32;
            regs.r[rd] = imm8;
            regs.set_nz_flags(imm8);
        }
        0b00101 => {
            // CMP Rn, #imm8 / 立即数比较
            let rn = ((opcode >> 8) & 0x7) as usize;
            let imm8 = (opcode & 0xFF) as u32;
            let value = regs.r[rn];
            let (result, _overflow) = value.overflowing_sub(imm8);
            let c = value >= imm8;
            let v = ((value ^ imm8) & (value ^ result) & 0x8000_0000) != 0;
            regs.set_apsr_nzcv((result & 0x8000_0000) != 0, result == 0, c, v);
        }
        0b00110 => {
            // ADDS Rd, #imm8 / 立即数加法
            let rd = ((opcode >> 8) & 0x7) as usize;
            let imm8 = (opcode & 0xFF) as u32;
            let value = regs.r[rd];
            let (result, carry) = value.overflowing_add(imm8);
            let v = ((value ^ imm8) & !(value ^ result) & 0x8000_0000) != 0;
            regs.r[rd] = result;
            regs.set_apsr_nzcv((result & 0x8000_0000) != 0, result == 0, carry, v);
        }
        0b00111 => {
            // SUBS Rd, #imm8 / 立即数减法
            let rd = ((opcode >> 8) & 0x7) as usize;
            let imm8 = (opcode & 0xFF) as u32;
            let value = regs.r[rd];
            let (result, _overflow) = value.overflowing_sub(imm8);
            let c = value >= imm8;
            let v = ((value ^ imm8) & (value ^ result) & 0x8000_0000) != 0;
            regs.r[rd] = result;
            regs.set_apsr_nzcv((result & 0x8000_0000) != 0, result == 0, c, v);
        }
        0b01000 if (opcode >> 10) & 0x1 != 0 => {
            // ADD/CMP/MOV high registers, BX and BLX
            // 高寄存器 ADD/CMP/MOV，以及 BX 与 BLX
            let op = (opcode >> 8) & 0x3;
            let h1 = ((opcode >> 7) & 0x1) != 0;
            let rs_or_rm = ((opcode >> 3) & 0xF) as usize;
            let rd = ((opcode & 0x7) as usize) | if h1 { 8 } else { 0 };
            match op {
                0b00 => {
                    // ADD Rd, Rm (high regs) / 加法（高寄存器）
                    let result = regs.read(rd as u8).wrapping_add(regs.read(rs_or_rm as u8));
                    if h1 && rd == 15 {
                        regs.pc = result & !1u32;
                        return Ok(StepResult::BranchTaken);
                    }
                    regs.write(rd as u8, result);
                }
                0b01 => {
                    // CMP Rn, Rm (high regs) / 比较（高寄存器）
                    let rn = regs.read(rd as u8);
                    let rm = regs.read(rs_or_rm as u8);
                    let (result, _overflow) = rn.overflowing_sub(rm);
                    let c = rn >= rm;
                    let v = ((rn ^ rm) & (rn ^ result) & 0x8000_0000) != 0;
                    regs.set_apsr_nzcv((result & 0x8000_0000) != 0, result == 0, c, v);
                }
                0b10 => {
                    // MOV Rd, Rm (high regs) / 移动（高寄存器）
                    let value = regs.read(rs_or_rm as u8);
                    // MOV to PC = branch / 写入 PC 即分支
                    if rd == 15 {
                        regs.pc = value & !1u32;
                        return Ok(StepResult::BranchTaken);
                    }
                    regs.write(rd as u8, value);
                }
                0b11 => {
                    // BX Rm or BLX Rm / 分支并交换或带链接分支并交换
                    let target = regs.read(rs_or_rm as u8);
                    if h1 {
                        // BLX Rm: LR = 下一条指令的地址（PC 已由取指推进）。
                        // BLX Rm: LR = address of the next instruction (PC already advanced).
                        regs.lr = regs.pc | 1;
                    }
                    if target & 0x1 == 0 {
                        // Not Thumb target → ARM mode (not supported by M33)
                        // 非 Thumb 目标 → ARM 模式（M33 不支持）
                        regs.xpsr |= 0x200_0000; // UsageFault: INVSTATE
                        return Err(CoreError::BusError {
                            addr: target,
                            kind: rp2350_core::error::BusErrorKind::NoDevice,
                        });
                    }
                    regs.pc = target & !1u32;
                    return Ok(StepResult::BranchTaken);
                }
                _ => unreachable!(),
            }
        }
        0b01000 => {
            // Data processing register (two-operand): ANDS, EORS, LSLS, LSRS,
            // ASRS, ADCS, SBCS, RORS, TST, RSBS, CMP, CMN, ORRS, MULS, BICS, MVNS
            // 数据处理寄存器（双操作数）
            let op = (opcode >> 6) & 0xF;
            let rm = ((opcode >> 3) & 0x7) as usize;
            let rdn = (opcode & 0x7) as usize;
            let rn_val = regs.r[rdn];
            let rm_val = regs.r[rm];
            match op {
                0x0 => {
                    // ANDS / 按位与
                    let result = rn_val & rm_val;
                    regs.r[rdn] = result;
                    regs.set_nz_flags(result);
                }
                0x1 => {
                    // EORS / 异或
                    let result = rn_val ^ rm_val;
                    regs.r[rdn] = result;
                    regs.set_nz_flags(result);
                }
                0x2 => {
                    // LSLS / 逻辑左移
                    let shift = rm_val & 0xFF;
                    let result = lsl_c(rn_val, shift, regs);
                    regs.r[rdn] = result;
                    regs.set_nz_flags(result);
                }
                0x3 => {
                    // LSRS / 逻辑右移
                    let shift = rm_val & 0xFF;
                    let result = lsr_c(rn_val, shift, regs);
                    regs.r[rdn] = result;
                    regs.set_nz_flags(result);
                }
                0x4 => {
                    // ASRS / 算术右移
                    let shift = rm_val & 0xFF;
                    let result = asr_c(rn_val, shift, regs);
                    regs.r[rdn] = result;
                    regs.set_nz_flags(result);
                }
                0x5 => {
                    // ADCS / 带进位加法
                    let carry = if regs.apsr_c() { 1u64 } else { 0u64 };
                    let result64 = rn_val as u64 + rm_val as u64 + carry;
                    let result32 = adc(result64, regs, true);
                    regs.r[rdn] = result32;
                }
                0x6 => {
                    // SBCS / 带借位减法
                    let carry = if regs.apsr_c() { 0u64 } else { 1u64 };
                    let result64 = (rn_val as u64)
                        .wrapping_sub(rm_val as u64)
                        .wrapping_sub(carry);
                    let result32 = sbc(result64, regs, true);
                    regs.r[rdn] = result32;
                }
                0x7 => {
                    // RORS / 循环右移
                    let shift = rm_val & 0xFF;
                    let result = ror_c(rn_val, shift, regs);
                    regs.r[rdn] = result;
                    regs.set_nz_flags(result);
                }
                0x8 => {
                    // TST / 测试（AND 不存结果）
                    let result = rn_val & rm_val;
                    regs.set_nz_flags(result);
                    // C flag unchanged for TST
                }
                0x9 => {
                    // RSBS / 反减（0 - Rn）
                    let (result, _overflow) = 0u32.overflowing_sub(rn_val);
                    let c = 0 >= rn_val;
                    regs.r[rdn] = result;
                    regs.set_apsr_nzcv(
                        (result & 0x8000_0000) != 0,
                        result == 0,
                        c,
                        (rn_val & 0x8000_0000) != 0,
                    );
                }
                0xA => {
                    // CMP / 比较
                    let (result, _overflow) = rn_val.overflowing_sub(rm_val);
                    let c = rn_val >= rm_val;
                    let v = ((rn_val ^ rm_val) & (rn_val ^ result) & 0x8000_0000) != 0;
                    regs.set_apsr_nzcv((result & 0x8000_0000) != 0, result == 0, c, v);
                }
                0xB => {
                    // CMN / 比较负数
                    let (result, overflow) = rn_val.overflowing_add(rm_val);
                    regs.set_apsr_nzcv(
                        (result & 0x8000_0000) != 0,
                        result == 0,
                        overflow,
                        ((rn_val ^ rm_val) & !(rn_val ^ result) & 0x8000_0000) == 0
                            && ((rn_val ^ rm_val) & 0x8000_0000) != 0,
                    );
                }
                0xC => {
                    // ORRS / 按位或
                    let result = rn_val | rm_val;
                    regs.r[rdn] = result;
                    regs.set_nz_flags(result);
                }
                0xD => {
                    // MULS / 乘法
                    let result = rn_val.wrapping_mul(rm_val);
                    regs.r[rdn] = result;
                    regs.set_nz_flags(result);
                }
                0xE => {
                    // BICS / 位清零
                    let result = rn_val & !rm_val;
                    regs.r[rdn] = result;
                    regs.set_nz_flags(result);
                }
                0xF => {
                    // MVNS / 按位取反
                    let result = !rm_val;
                    regs.r[rdn] = result;
                    regs.set_nz_flags(result);
                }
                _ => unreachable!(),
            }
        }
        0b01010 => {
            // Load/store register offset: STR, STRH, STRB, LDRSB
            // 加载/存储寄存器偏移：STR、STRH、STRB、LDRSB
            let op_a = (opcode >> 9) & 0x3;
            let ro = ((opcode >> 6) & 0x7) as usize;
            let rb = ((opcode >> 3) & 0x7) as usize;
            let rd = (opcode & 0x7) as usize;
            let addr: u32 = regs.r[rb].wrapping_add(regs.r[ro]);
            match op_a {
                0b00 => {
                    // STR Rd, [Rb, Ro] / 存储字
                    bus.write_u32(addr, regs.r[rd], cycles)?;
                    *cycles += 1;
                }
                0b01 => {
                    // STRH Rd, [Rb, Ro] / 存储半字
                    bus.write_u16(addr, regs.r[rd] as u16, cycles)?;
                    *cycles += 1;
                }
                0b10 => {
                    // STRB Rd, [Rb, Ro] / 存储字节
                    bus.write_u8(addr, regs.r[rd] as u8, cycles)?;
                    *cycles += 1;
                }
                0b11 => {
                    // LDRSB Rd, [Rb, Ro] / 加载有符号字节
                    let val = bus.read_u8(addr, cycles)?;
                    regs.r[rd] = sign_extend(val as u32, 8);
                    *cycles += 1;
                }
                _ => unreachable!(),
            }
        }
        0b01011 => {
            // Load/store register offset (continued): LDR, LDRH, LDRB, LDRSH
            // 加载/存储寄存器偏移（续）
            let op_b = (opcode >> 9) & 0x3;
            let ro = ((opcode >> 6) & 0x7) as usize;
            let rb = ((opcode >> 3) & 0x7) as usize;
            let rd = (opcode & 0x7) as usize;
            let addr: u32 = regs.r[rb].wrapping_add(regs.r[ro]);
            match op_b {
                0b00 => {
                    // LDR Rd, [Rb, Ro] / 加载字
                    regs.r[rd] = bus.read_u32(addr, cycles)?;
                    *cycles += 1;
                }
                0b01 => {
                    // LDRH Rd, [Rb, Ro] / 加载半字
                    regs.r[rd] = bus.read_u16(addr, cycles)? as u32;
                    *cycles += 1;
                }
                0b10 => {
                    // LDRB Rd, [Rb, Ro] / 加载字节
                    regs.r[rd] = bus.read_u8(addr, cycles)? as u32;
                    *cycles += 1;
                }
                0b11 => {
                    // LDRSH Rd, [Rb, Ro] / 加载有符号半字
                    let val = bus.read_u16(addr, cycles)?;
                    regs.r[rd] = sign_extend(val as u32, 16);
                    *cycles += 1;
                }
                _ => unreachable!(),
            }
        }
        0b10000..=0b10001 => {
            // Load/store immediate offset: STRH, LDRH
            // 加载/存储立即数偏移：STRH、LDRH
            let op = (opcode >> 11) & 0x1;
            let imm5 = ((opcode >> 6) & 0x1F) as u32;
            let rb = ((opcode >> 3) & 0x7) as usize;
            let rd = (opcode & 0x7) as usize;
            let addr = regs.r[rb].wrapping_add(imm5 << 1);
            if op == 0 {
                // STRH Rd, [Rb, #imm5*2] / 存储半字
                bus.write_u16(addr, regs.r[rd] as u16, cycles)?;
            } else {
                // LDRH Rd, [Rb, #imm5*2] / 加载半字
                regs.r[rd] = bus.read_u16(addr, cycles)? as u32;
            }
            *cycles += 1;
        }
        0b10010..=0b10011 => {
            // Load/store SP-relative: STR, LDR
            // SP 相对加载/存储
            let op = (opcode >> 11) & 0x1;
            let rd = ((opcode >> 8) & 0x7) as usize;
            let imm8 = (opcode & 0xFF) as u32;
            let addr = regs.sp.wrapping_add(imm8 << 2);
            if op == 0 {
                // STR Rd, [SP, #imm8*4] / 存储字
                bus.write_u32(addr, regs.r[rd], cycles)?;
            } else {
                // LDR Rd, [SP, #imm8*4] / 加载字
                regs.r[rd] = bus.read_u32(addr, cycles)?;
            }
            *cycles += 1;
        }
        0b01001 => {
            // LDR Rd, [PC, #imm8*4] — literal load
            // LDR Rd, [PC, #imm8*4] — 文字池加载
            let rd = ((opcode >> 8) & 0x7) as usize;
            let imm8 = (opcode & 0xFF) as u32;
            let addr = (regs.pc.wrapping_add(2) & !0x3u32).wrapping_add(imm8 << 2);
            regs.r[rd] = bus.read_u32(addr, cycles)?;
            *cycles += 1;
        }
        0b01100..=0b01101 => {
            // Load/store immediate offset: STR, LDR
            // 加载/存储立即数偏移：STR、LDR
            let op = (opcode >> 11) & 0x1;
            let imm5 = ((opcode >> 6) & 0x1F) as u32;
            let rb = ((opcode >> 3) & 0x7) as usize;
            let rd = (opcode & 0x7) as usize;
            let addr = regs.r[rb].wrapping_add(imm5 << 2);
            if op == 0 {
                // STR Rd, [Rb, #imm5*4]
                bus.write_u32(addr, regs.r[rd], cycles)?;
            } else {
                // LDR Rd, [Rb, #imm5*4]
                regs.r[rd] = bus.read_u32(addr, cycles)?;
            }
            *cycles += 1;
        }
        0b01110..=0b01111 => {
            // Load/store immediate offset: STRB, LDRB
            // 加载/存储立即数偏移：STRB、LDRB
            let op = (opcode >> 11) & 0x1;
            let imm5 = ((opcode >> 6) & 0x1F) as u32;
            let rb = ((opcode >> 3) & 0x7) as usize;
            let rd = (opcode & 0x7) as usize;
            let addr = regs.r[rb].wrapping_add(imm5);
            if op == 0 {
                // STRB Rd, [Rb, #imm5]
                bus.write_u8(addr, regs.r[rd] as u8, cycles)?;
            } else {
                // LDRB Rd, [Rb, #imm5]
                regs.r[rd] = bus.read_u8(addr, cycles)? as u32;
            }
            *cycles += 1;
        }
        0b10100 => {
            // ADR Rd, label / 地址生成
            let rd = ((opcode >> 8) & 0x7) as usize;
            let imm8 = (opcode & 0xFF) as u32;
            regs.r[rd] = (regs.pc.wrapping_add(2) & !0x3u32).wrapping_add(imm8 << 2);
        }
        0b10101 => {
            // ADD Rd, SP, #imm8*4
            let rd = ((opcode >> 8) & 0x7) as usize;
            let imm8 = (opcode & 0xFF) as u32;
            regs.r[rd] = regs.sp.wrapping_add(imm8 << 2);
        }
        0b10110 => {
            // Miscellaneous 16-bit instructions in the 0xB0xx-0xB7xx range:
            // ADD/SUB SP, CBZ, PUSH, SETEND, CPS.
            // 0xB0xx-0xB7xx 范围内的杂项 16 位指令：ADD/SUB SP、CBZ、PUSH、SETEND、CPS。
            match opcode >> 8 {
                // ADD SP, #imm7*4 or SUB SP, #imm7*4
                // 加/减 SP 立即数
                0xB0 => {
                    let op = (opcode >> 7) & 0x1;
                    let imm7 = ((opcode & 0x7F) as u32) << 2;
                    if op == 0 {
                        regs.sp = regs.sp.wrapping_add(imm7);
                    } else {
                        regs.sp = regs.sp.wrapping_sub(imm7);
                    }
                }
                // CBZ Rn, label (bit9 supplies the high bit of the 7-bit immediate)
                // CBZ Rn, label（bit9 是 7 位立即数的高位）
                0xB1 | 0xB3 => {
                    let i = ((opcode >> 9) & 0x1) as u32;
                    let imm5 = ((opcode >> 3) & 0x1F) as u32;
                    let rn = (opcode & 0x7) as usize;
                    let offset = sign_extend((i << 6) | (imm5 << 1), 7);
                    if regs.r[rn] == 0 {
                        regs.pc = regs.pc.wrapping_add(2).wrapping_add(offset);
                        return Ok(StepResult::BranchTaken);
                    }
                }
                // PUSH {reg_list, LR?}
                // PUSH {reg_list, LR?}
                0xB4 | 0xB5 => {
                    let m = ((opcode >> 8) & 0x1) != 0;
                    let reg_list = (opcode & 0xFF) as u16;
                    let count = reg_list.count_ones() + u32::from(m);
                    regs.sp = regs.sp.wrapping_sub(count * 4);
                    let mut addr = regs.sp;
                    for i in 0..8u16 {
                        if (reg_list >> i) & 1 != 0 {
                            bus.write_u32(addr, regs.r[i as usize], cycles)?;
                            *cycles += 1;
                            addr = addr.wrapping_add(4);
                        }
                    }
                    if m {
                        bus.write_u32(addr, regs.lr, cycles)?;
                        *cycles += 1;
                    }
                }
                // SETEND (deprecated, NOP) and CPS for PRIMASK/FAULTMASK
                // SETEND（已废弃，视为 NOP）以及用于 PRIMASK/FAULTMASK 的 CPS
                0xB6 => match opcode {
                    0xB650 => {}
                    o if (o & 0xFFF0) == 0xB660 || (o & 0xFFF0) == 0xB670 => {
                        let enable = (opcode >> 4) & 0x1 == 0;
                        if (opcode & 0x1) != 0 {
                            if enable {
                                regs.faultmask &= !1;
                            } else {
                                regs.faultmask |= 1;
                            }
                        } else if enable {
                            regs.primask &= !1;
                        } else {
                            regs.primask |= 1;
                        }
                    }
                    _ => return Ok(StepResult::Undefined),
                },
                _ => return Ok(StepResult::Undefined),
            }
        }
        0b10111 => {
            // Miscellaneous 16-bit instructions in the 0xB8xx-0xBFFF range:
            // CBNZ, POP, BKPT, IT.
            // 0xB8xx-0xBFFF 范围内的杂项 16 位指令：CBNZ、POP、BKPT、IT。
            match opcode >> 8 {
                // CBNZ Rn, label
                // CBNZ Rn, label
                0xB9 | 0xBB => {
                    let i = ((opcode >> 9) & 0x1) as u32;
                    let imm5 = ((opcode >> 3) & 0x1F) as u32;
                    let rn = (opcode & 0x7) as usize;
                    let offset = sign_extend((i << 6) | (imm5 << 1), 7);
                    if regs.r[rn] != 0 {
                        regs.pc = regs.pc.wrapping_add(2).wrapping_add(offset);
                        return Ok(StepResult::BranchTaken);
                    }
                }
                // POP {reg_list, PC?}
                // POP {reg_list, PC?}
                0xBC | 0xBD => {
                    let m = ((opcode >> 8) & 0x1) != 0;
                    let reg_list = (opcode & 0xFF) as u16;
                    let mut addr = regs.sp;
                    for i in 0..8u16 {
                        if (reg_list >> i) & 1 != 0 {
                            regs.r[i as usize] = bus.read_u32(addr, cycles)?;
                            *cycles += 1;
                            addr = addr.wrapping_add(4);
                        }
                    }
                    if m {
                        let val = bus.read_u32(addr, cycles)?;
                        *cycles += 1;
                        regs.pc = val & !1u32;
                    }
                    regs.sp = regs
                        .sp
                        .wrapping_add((reg_list.count_ones() + u32::from(m)) * 4);
                    if m {
                        return Ok(StepResult::BranchTaken);
                    }
                }
                // BKPT #imm8
                // BKPT #imm8
                0xBE => return Ok(StepResult::Breakpoint((opcode & 0xFF) as u8)),
                // IT
                // IT 指令
                0xBF => {
                    let firstcond = ((opcode >> 4) & 0xF) as u8;
                    let mask = (opcode & 0xF) as u8;
                    if firstcond == 0xF || (firstcond == 0xE && mask != 0) {
                        return Ok(StepResult::Undefined);
                    }
                    it.execute_it(firstcond, mask);
                }
                _ => return Ok(StepResult::Undefined),
            }
        }
        0b11000 => {
            // STMIA Rn!, {reg_list} / 多存储后递增
            let rb = ((opcode >> 8) & 0x7) as usize;
            let reg_list = (opcode & 0xFF) as u16;
            let mut addr = regs.r[rb];
            for i in 0..8u16 {
                if (reg_list >> i) & 1 != 0 {
                    bus.write_u32(addr, regs.r[i as usize], cycles)?;
                    *cycles += 1;
                    addr = addr.wrapping_add(4);
                }
            }
            regs.r[rb] = addr;
        }
        0b11001 => {
            // LDMIA Rn!, {reg_list} / 多加载后递增
            let rb = ((opcode >> 8) & 0x7) as usize;
            let reg_list = (opcode & 0xFF) as u16;
            let mut addr = regs.r[rb];
            for i in 0..8u16 {
                if (reg_list >> i) & 1 != 0 {
                    regs.r[i as usize] = bus.read_u32(addr, cycles)?;
                    *cycles += 1;
                    addr = addr.wrapping_add(4);
                }
            }
            regs.r[rb] = addr;
        }
        0b11010..=0b11011 => {
            // B<cond> (conditional branch) or SVC (cond == 0xF)
            // 条件分支 B<cond>，或 cond == 0xF 时的 SVC
            let cond_bits = ((opcode >> 8) & 0xF) as u8;
            if cond_bits == 0xF {
                return Ok(StepResult::SvCall((opcode & 0xFF) as u8));
            }
            let cond = Condition::from_bits(cond_bits);
            if cond != Condition::Al && cond.evaluate(regs) {
                let offset = sign_extend(((opcode & 0xFF) as u32) << 1, 9);
                regs.pc = regs.pc.wrapping_add(2).wrapping_add(offset);
                return Ok(StepResult::BranchTaken);
            }
        }
        0b11100 => {
            // B (unconditional branch) / 无条件分支
            let offset = sign_extend(((opcode & 0x7FF) as u32) << 1, 12);
            regs.pc = regs.pc.wrapping_add(2).wrapping_add(offset);
            return Ok(StepResult::BranchTaken);
        }
        0b11101..=0b11111 => {
            // BL (unconditional branch with link) or miscellaneous
            // BL（带链接的无条件分支）或杂项
            // Thumb-32 instruction handled by caller
            // Thumb-32 指令由调用者处理
            return Ok(StepResult::Undefined);
        }
        _ => {
            return Ok(StepResult::Undefined);
        }
    }

    *cycles += 1;
    Ok(StepResult::Ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn condition_code_from_bits() {
        // Verify condition code mapping
        assert_eq!(Condition::from_bits(0x0), Condition::Eq);
        assert_eq!(Condition::from_bits(0xE), Condition::Al);
        assert_eq!(Condition::from_bits(0xFF), Condition::Al); // Out of range → Al
    }

    #[test]
    fn step_result_equality() {
        assert_eq!(StepResult::Ok, StepResult::Ok);
        assert_ne!(StepResult::Ok, StepResult::Undefined);
    }

    #[test]
    fn short_branch_uses_instruction_address_plus_four() {
        // B -13 at 0x1000_0164 must land on 0x1000_014E.
        // 0x1000_0164 处的 B -13 必须跳到 0x1000_014E。
        let mut regs = CortexM33Registers::new();
        regs.pc = 0x1000_0166; // 取指后已推进 2 字节 / fetcher already advanced by 2
        let mut bus = rp2350_core::memory::SimpleBus::new();
        let mut nvic = Nvic::new();
        let mut systick = SysTick::new();
        let mut it = ITBlockState::new();
        let mut cycles = 0;
        let result = execute_thumb16(
            0xE7F3,
            &mut regs,
            &mut bus,
            &mut nvic,
            &mut systick,
            &mut it,
            &mut cycles,
        )
        .unwrap();
        assert_eq!(result, StepResult::BranchTaken);
        assert_eq!(regs.pc, 0x1000_014E);
    }

    #[test]
    fn cbz_uses_instruction_address_plus_four() {
        // CBZ r0, +2 at 0x1000_0160 must land on 0x1000_0166.
        // 0x1000_0160 处的 CBZ r0, +2 必须跳到 0x1000_0166。
        let mut regs = CortexM33Registers::new();
        regs.pc = 0x1000_0162;
        regs.r[0] = 0;
        let mut bus = rp2350_core::memory::SimpleBus::new();
        let mut nvic = Nvic::new();
        let mut systick = SysTick::new();
        let mut it = ITBlockState::new();
        let mut cycles = 0;
        let result = execute_thumb16(
            0xB108,
            &mut regs,
            &mut bus,
            &mut nvic,
            &mut systick,
            &mut it,
            &mut cycles,
        )
        .unwrap();
        assert_eq!(result, StepResult::BranchTaken);
        assert_eq!(regs.pc, 0x1000_0166);
    }
}
