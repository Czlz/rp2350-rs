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
            // Shift by immediate: LSLS, LSRS, ASRS
            // 立即数移位：LSLS、LSRS、ASRS
            let op = (opcode >> 11) & 0x3;
            let imm5 = ((opcode >> 6) & 0x1F) as u32;
            let rm = ((opcode >> 3) & 0x7) as usize;
            let rd = (opcode & 0x7) as usize;
            match op {
                0b00 => {
                    // LSLS Rd, Rm, #imm5 / 逻辑左移
                    let result = lsl_c(regs.r[rm], imm5, regs);
                    regs.r[rd] = result;
                    regs.set_nz_flags(result);
                }
                _ => {
                    // LSRS or ASRS / 逻辑或算术右移
                    let shift = if imm5 == 0 { 32 } else { imm5 };
                    let result = if op == 0b01 {
                        lsr_c(regs.r[rm], shift, regs)
                    } else {
                        asr_c(regs.r[rm], shift, regs)
                    };
                    regs.r[rd] = result;
                    regs.set_nz_flags(result);
                }
            }
        }
        0b00001..=0b00011 => {
            // ADD/SUB/CMP/MOV immediate (3-bit register, 8-bit immediate)
            // ADD/SUB/CMP/MOV 立即数（3 位寄存器，8 位立即数）
            let op = (opcode >> 11) & 0x3;
            let rd_or_rn = ((opcode >> 8) & 0x7) as usize;
            let imm8 = (opcode & 0xFF) as u32;
            match op {
                0b01 => {
                    // MOVS Rd, #imm8 / 移动到 Rd
                    regs.r[rd_or_rn] = imm8;
                    regs.set_nz_flags(imm8);
                }
                0b10 => {
                    // CMP Rn, #imm8 / 比较
                    let rn = regs.r[rd_or_rn];
                    let (result, _overflow) = rn.overflowing_sub(imm8);
                    let c = rn >= imm8;
                    let v = ((rn ^ imm8) & (rn ^ result) & 0x8000_0000) != 0;
                    regs.set_apsr_nzcv((result & 0x8000_0000) != 0, result == 0, c, v);
                }
                0b11 => {
                    // ADDS Rd, #imm8 / 加法
                    let rn = regs.r[rd_or_rn];
                    let (result, overflow) = rn.overflowing_add(imm8);
                    let c = overflow;
                    let v = ((rn ^ imm8) & !(rn ^ result) & 0x8000_0000) == 0
                        && ((rn ^ imm8) & 0x8000_0000) != 0;
                    regs.r[rd_or_rn] = result;
                    regs.set_apsr_nzcv((result & 0x8000_0000) != 0, result == 0, c, v);
                }
                0b00 => {
                    // SUBS Rd, #imm8 / 减法
                    let rn = regs.r[rd_or_rn];
                    let (result, _overflow) = rn.overflowing_sub(imm8);
                    let c = rn >= imm8;
                    let v = ((rn ^ imm8) & (rn ^ result) & 0x8000_0000) != 0;
                    regs.r[rd_or_rn] = result;
                    regs.set_apsr_nzcv((result & 0x8000_0000) != 0, result == 0, c, v);
                }
                _ => unreachable!(),
            }
        }
        0b00100 => {
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
        0b00101 => {
            // ADD/CMP/MOV high registers: ADD, CMP, MOV, BX, BLX
            // ADD/CMP/MOV 高寄存器：ADD、CMP、MOV、BX、BLX
            let op = (opcode >> 8) & 0x3;
            let h1 = ((opcode >> 7) & 0x1) != 0;
            let h2 = ((opcode >> 6) & 0x1) != 0;
            let rs_or_rm = ((opcode >> 3) & 0xF) as usize | if h2 { 8 } else { 0 };
            let rd = ((opcode & 0x7) as usize) | if h1 { 8 } else { 0 };
            match op {
                0b00 => {
                    // ADD Rd, Rm (high regs) / 加法（高寄存器）
                    let result = regs.r[rd].wrapping_add(regs.r[rs_or_rm]);
                    if h1 && rd == 15 {
                        // ADD PC, Rm → branch to aligned address
                        // ADD PC, Rm → 跳转到对齐地址
                        regs.pc = result & !1u32;
                        return Ok(StepResult::BranchTaken);
                    }
                    regs.r[rd] = result;
                }
                0b01 => {
                    // CMP Rn, Rm (high regs) / 比较（高寄存器）
                    let rn = regs.r[rd];
                    let rm = regs.r[rs_or_rm];
                    let (result, _overflow) = rn.overflowing_sub(rm);
                    let c = rn >= rm;
                    let v = ((rn ^ rm) & (rn ^ result) & 0x8000_0000) != 0;
                    regs.set_apsr_nzcv((result & 0x8000_0000) != 0, result == 0, c, v);
                }
                0b10 => {
                    // MOV Rd, Rm (high regs) / 移动（高寄存器）
                    regs.r[rd] = regs.r[rs_or_rm];
                    // MOV to PC = branch
                    if rd == 15 {
                        regs.pc &= !1u32;
                        return Ok(StepResult::BranchTaken);
                    }
                }
                0b11 => {
                    // BX Rm or BLX Rm / 分支并交换或带链接分支并交换
                    let target = regs.r[rs_or_rm];
                    if h2 {
                        // BLX Rm: set LR = next PC, switch to Thumb if needed
                        // BLX Rm：设置 LR = 下一条 PC，根据需要切换到 Thumb
                        regs.lr = (regs.pc - 2) | 1;
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
        0b00110 => {
            // MOV Rd, #imm8
            let rd = ((opcode >> 8) & 0x7) as usize;
            let imm8 = (opcode & 0xFF) as u32;
            regs.r[rd] = imm8;
            regs.set_nz_flags(imm8);
        }
        0b00111 => {
            // ADD/SUB immediate (3-bit register)
            // ADD/SUB 立即数（3 位寄存器）
            let op = (opcode >> 9) & 0x1;
            let rn = ((opcode >> 6) & 0x7) as usize;
            let rd = ((opcode >> 3) & 0x7) as usize;
            let imm3 = (opcode & 0x7) as u32;
            if op == 0 {
                // ADDS Rd, Rn, #imm3 / 加法
                let result = regs.r[rn].wrapping_add(imm3);
                regs.r[rd] = result;
                regs.set_nz_flags(result);
            } else {
                // SUBS Rd, Rn, #imm3 / 减法
                let result = regs.r[rn].wrapping_sub(imm3);
                regs.r[rd] = result;
                regs.set_nz_flags(result);
            }
        }
        0b01000 => {
            // Load/store register offset: STR, STRH, STRB, LDRSB, LDR, LDRH, LDRB, LDRSH
            // 加载/存储寄存器偏移
            let op_a = (opcode >> 10) & 0x3;
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
        0b01001 => {
            // Load/store register offset (continued): LDR, LDRH, LDRB, LDRSH
            // 加载/存储寄存器偏移（续）
            let op_b = (opcode >> 10) & 0x3;
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
        0b01010 => {
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
        0b01011 => {
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
        0b01100 => {
            // LDR Rd, [PC, #imm8*4] — literal load
            // LDR Rd, [PC, #imm8*4] — 文字池加载
            let rd = ((opcode >> 8) & 0x7) as usize;
            let imm8 = (opcode & 0xFF) as u32;
            let addr = (regs.pc & !0x3u32).wrapping_add(imm8 << 2);
            regs.r[rd] = bus.read_u32(addr, cycles)?;
            *cycles += 1;
        }
        0b01101 => {
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
        0b01110 => {
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
        0b01111 => {
            // Load/store multiple: STMIA, LDMIA
            // 多加载/存储
            let op = (opcode >> 11) & 0x1;
            let rb = ((opcode >> 8) & 0x7) as usize;
            let reg_list = (opcode & 0xFF) as u16;
            let mut addr = regs.r[rb];
            if reg_list == 0 {
                return Ok(StepResult::Undefined);
            }
            for i in 0..8u16 {
                if (reg_list >> i) & 1 != 0 {
                    if op == 0 {
                        // STMIA Rb!, {reg_list}: store and increment after
                        // STMIA Rb!, {reg_list}：存储后递增
                        bus.write_u32(addr, regs.r[i as usize], cycles)?;
                        *cycles += 1;
                    } else {
                        // LDMIA Rb!, {reg_list}: load and increment after
                        // LDMIA Rb!, {reg_list}：加载后递增
                        let val = bus.read_u32(addr, cycles)?;
                        regs.r[i as usize] = val;
                        *cycles += 1;
                    }
                    addr = addr.wrapping_add(4);
                }
            }
            // Update base register if not in reg_list (write-back)
            // 如果不在 reg_list 中，更新基址寄存器（写回）
            if (reg_list >> rb) & 1 == 0 || op == 1 {
                regs.r[rb] = addr;
            }
        }
        0b10000..=0b10011 => {
            // Unconditional branch: B, conditional branch: B<cond>
            // 无条件分支：B，条件分支：B<cond>
            let cond_bits = (opcode >> 8) & 0xF;
            if cond_bits == 0xE {
                // B (unconditional) / 无条件
                let offset = sign_extend(((opcode & 0x7FF) as u32) << 1, 12);
                regs.pc = regs.pc.wrapping_add(offset);
                return Ok(StepResult::BranchTaken);
            } else if cond_bits != 0xF {
                // B<cond> (conditional) / 有条件
                let cond = Condition::from_bits(cond_bits as u8);
                if cond.evaluate(regs) {
                    let offset = sign_extend(((opcode & 0xFF) as u32) << 1, 9);
                    regs.pc = regs.pc.wrapping_add(offset);
                    return Ok(StepResult::BranchTaken);
                }
            }
            // 0xF: SVC instruction / SVC 指令
            if cond_bits == 0xF {
                let imm8 = (opcode & 0xFF) as u8;
                return Ok(StepResult::SvCall(imm8));
            }
        }
        0b10100 => {
            // ADR Rd, label / 地址生成
            let rd = ((opcode >> 8) & 0x7) as usize;
            let imm8 = (opcode & 0xFF) as u32;
            regs.r[rd] = (regs.pc & !0x3u32).wrapping_add(imm8 << 2);
        }
        0b10101 => {
            // ADD Rd, SP, #imm8*4
            let rd = ((opcode >> 8) & 0x7) as usize;
            let imm8 = (opcode & 0xFF) as u32;
            regs.r[rd] = regs.sp.wrapping_add(imm8 << 2);
        }
        0b10110 => {
            // Miscellaneous: ADD/SUB SP, PUSH, POP, SETEND, CPS, BKPT, IT, etc.
            // 杂项：ADD/SUB SP、PUSH、POP、SETEND、CPS、BKPT、IT 等
            let op_a = (opcode >> 7) & 0x1F;
            let op_b = (opcode >> 5) & 0x7;
            match (op_a, op_b) {
                // ADD SP, #imm7*4 or SUB SP, #imm7*4
                (_, _) if (opcode >> 7) == 0b10110_000 => {
                    let op = (opcode >> 7) & 0x1;
                    let imm7 = ((opcode & 0x7F) as u32) << 2;
                    if op == 0 {
                        regs.sp = regs.sp.wrapping_add(imm7);
                    } else {
                        regs.sp = regs.sp.wrapping_sub(imm7);
                    }
                }
                // SETEND (deprecated in ARMv8-M, treated as NOP)
                (0b10110_010, _) => {
                    // SETEND not supported → NOP
                }
                // CPSID, CPSIE
                (0b10110_011, _) => {
                    let enable = (opcode >> 4) & 0x1 == 0;
                    let affect_fault = (opcode & 0x1) != 0;
                    if affect_fault {
                        if enable {
                            regs.faultmask &= !1;
                        } else {
                            regs.faultmask |= 1;
                        }
                    } else {
                        if enable {
                            regs.primask &= !1;
                        } else {
                            regs.primask |= 1;
                        }
                    }
                }
                // BKPT
                (0b10111_110, _) => {
                    let imm8 = (opcode & 0xFF) as u8;
                    return Ok(StepResult::Breakpoint(imm8));
                }
                // IT
                (0b10111_111, _) => {
                    let firstcond = ((opcode >> 4) & 0xF) as u8;
                    let mask = (opcode & 0xF) as u8;
                    if firstcond == 0xF || (firstcond == 0xE && mask != 0) {
                        return Ok(StepResult::Undefined);
                    }
                    it.execute_it(firstcond, mask);
                }
                // CBZ / CBNZ
                _ if (opcode >> 11) == 0b10111 => {
                    let op = (opcode >> 11) & 0x1;
                    let i = ((opcode >> 9) & 0x1) as u32;
                    let imm5 = ((opcode >> 3) & 0x1F) as u32;
                    let rn = (opcode & 0x7) as usize;
                    let imm = (i << 6) | (imm5 << 1);
                    let offset = sign_extend(imm, 7);
                    if (op == 0 && regs.r[rn] == 0) || (op == 1 && regs.r[rn] != 0) {
                        regs.pc = regs.pc.wrapping_add(offset);
                        return Ok(StepResult::BranchTaken);
                    }
                }
                // PUSH / POP
                _ => {
                    let op = (opcode >> 11) & 0x1;
                    let m = ((opcode >> 8) & 0x1) != 0; // LR for PUSH, PC for POP
                    let reg_list = (opcode & 0xFF) as u16;
                    if op == 0 {
                        // PUSH {reg_list, LR?}
                        let count = reg_list.count_ones() + if m { 1 } else { 0 };
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
                    } else {
                        // POP {reg_list, PC?}
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
                            .wrapping_add((reg_list.count_ones() + if m { 1 } else { 0 }) * 4);
                        if m {
                            return Ok(StepResult::BranchTaken);
                        }
                    }
                }
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
            // B<cond> (conditional branch, short)
            // B<cond>（条件分支，短）
            let cond = Condition::from_bits(((opcode >> 8) & 0xF) as u8);
            if cond != Condition::Al {
                if cond.evaluate(regs) {
                    let offset = sign_extend(((opcode & 0xFF) as u32) << 1, 9);
                    regs.pc = regs.pc.wrapping_add(offset);
                    return Ok(StepResult::BranchTaken);
                }
            }
        }
        0b11100 => {
            // B (unconditional branch) / 无条件分支
            let offset = sign_extend(((opcode & 0x7FF) as u32) << 1, 12);
            regs.pc = regs.pc.wrapping_add(offset);
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
}
