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

/// Return true for Thumb-16 instructions whose flag updates are unconditional
/// even inside an IT block: the compare/test family (CMP/TST/CMN).
/// 返回即使在 IT 块内也“无条件更新标志”的 16 位指令——比较/测试类
/// （CMP/TST/CMN）。
///
/// ARM defines `setflags = !InITBlock()` for the S-implicit 16-bit
/// arithmetic/data-processing encodings, but CMP/TST/CMN have no non-flag
/// variant, so they always update APSR.
/// ARM 规定 S 位隐含的 16 位算术/数据处理编码 `setflags = !InITBlock()`，
/// 但 CMP/TST/CMN 没有不带标志的变体，因此它们始终更新 APSR。
fn thumb16_always_writes_flags(opcode: u16) -> bool {
    match (opcode >> 11) & 0x1F {
        // CMP (immediate) / CMP 立即数
        0b00101 => true,
        0b01000 => {
            if (opcode >> 10) & 0x1 != 0 {
                // High-register group: only CMP (op == 0b01) writes flags.
                // 高寄存器组中只有 CMP（op == 0b01）写标志。
                (opcode >> 8) & 0x3 == 0b01
            } else {
                // Data-processing (register): TST, CMP, CMN.
                // 数据处理（寄存器）：TST、CMP、CMN。
                matches!((opcode >> 6) & 0xF, 0x8 | 0xA | 0xB)
            }
        }
        _ => false,
    }
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
    /// Exception return (any PC write carrying an EXC_RETURN value)
    /// 异常返回（任意写 PC 的指令携带 EXC_RETURN 值）
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
    //
    // 必须在 advance() 之前记录“本条是否位于 IT 块内”：advance() 会消费最后一个
    // 槽位，使其后 in_it_block() 立即变为 false。
    // Capture whether this instruction started inside an IT block before
    // advance(), because advance() consumes the final slot and clears
    // in_it_block() right afterwards.
    let in_it = it.in_it_block();
    if in_it {
        let cond = it.advance();
        if !cond.evaluate(regs) {
            // Instruction in IT block is not executed
            // IT 块中的指令不执行
            *cycles += 1;
            return Ok(StepResult::Ok);
        }
    }

    // 16 位算术/数据处理编码的 S 位是隐含的，ARM 规定 setflags =
    // !InITBlock()：在 IT 块内执行时不更新标志位。真实固件依赖这一点，例如
    // strlen 的 `itt eq { ldreq; addeq r0,#4 }` 之后紧跟 `beq`；若 `addeq`
    // 清掉 Z，循环会提前退出。比较/测试类（CMP/TST/CMN）始终更新标志，故
    // 不参与快照恢复。
    // Thumb-16 arithmetic/data-processing encodings carry an implicit S bit and
    // ARM specifies setflags = !InITBlock(): inside an IT block they leave the
    // flags unchanged. Real firmware depends on this (strlen's
    // `itt eq { ldreq; addeq r0,#4 }` is followed by `beq`; clearing Z in
    // `addeq` would exit the loop early). Compare/test instructions
    // (CMP/TST/CMN) always update the flags and are excluded from the restore.
    let saved_flags = regs.xpsr & 0xF000_0000;
    let suppress_flags = in_it && !thumb16_always_writes_flags(opcode);

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
            let value = regs.r[rn];
            let result = if subtract {
                value.wrapping_sub(operand)
            } else {
                value.wrapping_add(operand)
            };
            // ADDS/SUBS must update all four NZCV flags. The carry chain in
            // 64-bit arithmetic (for example CircuitPython's delay loops) relies
            // on the low-word ADDS producing the carry consumed by ADC.
            // ADDS/SUBS 必须更新全部 NZCV 标志；64 位算术的进位链（例如
            // CircuitPython 的延时循环）依赖低位 ADDS 产出的进位被 ADC 使用。
            let carry = if subtract {
                value >= operand
            } else {
                u64::from(value) + u64::from(operand) > u64::from(u32::MAX)
            };
            let overflow = if subtract {
                ((value ^ operand) & (value ^ result) & 0x8000_0000) != 0
            } else {
                ((value ^ operand) & !(value ^ result) & 0x8000_0000) != 0
            };
            regs.r[rd] = result;
            regs.set_apsr_nzcv((result & 0x8000_0000) != 0, result == 0, carry, overflow);
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
                    } else if crate::exceptions::write_pc(regs, target) {
                        // BX 携带 EXC_RETURN：交由核执行异常返回序列。
                        // BX carries EXC_RETURN: let the core perform the return sequence.
                        return Ok(StepResult::ExceptionReturn);
                    }
                    if target & 0x1 == 0 && crate::exceptions::decode_exc_return(target).is_none() {
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
                    // RSBS / 反减（0 - Rm）：Rdn 只是结果寄存器，取反的是源操作数。
                    // RSBS negates the source register Rm; Rdn only receives the
                    // result. Negating Rdn is wrong (e.g. NEGS r5, r3 at 0x425D).
                    let (result, overflow) = 0u32.overflowing_sub(rm_val);
                    let carry = rm_val == 0; // C = NOT borrow / C 表示无借位
                    regs.r[rdn] = result;
                    regs.set_apsr_nzcv((result & 0x8000_0000) != 0, result == 0, carry, overflow);
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
                    let value = if op == 0 {
                        regs.sp.wrapping_add(imm7)
                    } else {
                        regs.sp.wrapping_sub(imm7)
                    };
                    // 走 write(13) 以便同步 MSP/PSP 影子寄存器。
                    // Route through write(13) to keep the MSP/PSP bank in sync.
                    regs.write(13, value);
                }
                // SXTH / SXTB / UXTH / UXTB (bits[7:6] select the operation)
                // 符号/零扩展半字与字节（bit[7:6] 选择操作）
                0xB2 => {
                    let op = (opcode >> 6) & 0x3;
                    let rm = ((opcode >> 3) & 0x7) as usize;
                    let rd = (opcode & 0x7) as usize;
                    let value = regs.r[rm];
                    let result = match op {
                        0 => sign_extend(value & 0xFFFF, 16), // SXTH
                        1 => sign_extend(value & 0xFF, 8),    // SXTB
                        2 => value & 0xFFFF,                  // UXTH
                        _ => value & 0xFF,                    // UXTB
                    };
                    // ARMv8-M: the 16-bit SXTB/SXTH/UXTB/UXTH encodings do
                    // not affect APSR at all; only the 32-bit register forms
                    // update N and Z. CircuitPython's SET_CONFIGURATION loop
                    // relies on `uxtb` preserving the Z flag from `cmp`.
                    // ARMv8-M：16 位 SXTB/SXTH/UXTB/UXTH 编码完全不改变
                    // APSR，只有 32 位寄存器形式才更新 N 与 Z。CircuitPython
                    // 的 SET_CONFIGURATION 循环依赖 `uxtb` 保留 `cmp` 产生的
                    // Z 标志。
                    regs.r[rd] = result;
                }
                // CBZ Rn, label (bit9 supplies the high bit of the 7-bit immediate)
                // CBZ Rn, label（bit9 是 7 位立即数的高位）
                0xB1 | 0xB3 => {
                    let i = ((opcode >> 9) & 0x1) as u32;
                    let imm5 = ((opcode >> 3) & 0x1F) as u32;
                    let rn = (opcode & 0x7) as usize;
                    // CBZ/CBNZ 的偏移是 0..=126 的前向无符号值（iiiiii0），
                    // 不能按 7 位有符号数扩展，否则偏移 >= 64 字节时会反向跳。
                    // The CBZ/CBNZ offset is a forward unsigned 0..=126 value
                    // (iiiiii0); sign-extending it makes every offset >= 64
                    // bytes branch backwards.
                    let offset = (i << 6) | (imm5 << 1);
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
                    regs.write(13, regs.sp.wrapping_sub(count * 4));
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
                    // 与 CBZ 相同：7 位偏移是前向无符号值，不做符号扩展。
                    // Same as CBZ: the 7-bit offset is forward unsigned.
                    let offset = (i << 6) | (imm5 << 1);
                    if regs.r[rn] != 0 {
                        regs.pc = regs.pc.wrapping_add(2).wrapping_add(offset);
                        return Ok(StepResult::BranchTaken);
                    }
                }
                // REV / REV16 / REVSH (bits[7:6] select the operation)
                // 字节反转（bit[7:6] 选择操作）
                0xBA => {
                    let op = (opcode >> 6) & 0x3;
                    let rm = ((opcode >> 3) & 0x7) as usize;
                    let rd = (opcode & 0x7) as usize;
                    let value = regs.r[rm];
                    let result = match op {
                        0 => value.swap_bytes(),                                             // REV
                        1 => ((value & 0x00FF_00FF) << 8) | ((value & 0xFF00_FF00) >> 8), // REV16
                        3 => sign_extend(((value & 0xFF) << 8) | ((value >> 8) & 0xFF), 16), // REVSH
                        _ => return Ok(StepResult::Undefined),
                    };
                    regs.r[rd] = result;
                    regs.set_nz_flags(result);
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
                    let mut exception_return = false;
                    if m {
                        let val = bus.read_u32(addr, cycles)?;
                        *cycles += 1;
                        // POP {..., PC} uses LoadWritePC: an EXC_RETURN value
                        // must trigger the exception return sequence.
                        // POP {..., PC} 使用 LoadWritePC：EXC_RETURN 值必须
                        // 触发异常返回序列，而不是被当作普通跳转地址。
                        exception_return = crate::exceptions::write_pc(regs, val);
                    }
                    regs.write(
                        13,
                        regs.sp
                            .wrapping_add((reg_list.count_ones() + u32::from(m)) * 4),
                    );
                    if exception_return {
                        return Ok(StepResult::ExceptionReturn);
                    }
                    if m {
                        return Ok(StepResult::BranchTaken);
                    }
                }
                // BKPT #imm8
                // BKPT #imm8
                0xBE => return Ok(StepResult::Breakpoint((opcode & 0xFF) as u8)),
                // HINT / IT
                // HINT / IT 指令
                0xBF => {
                    // 编码按 ARMv8-M 手册：bits[3:0] 为 0 时进入 hint 空间，
                    // bits[7:4] 为提示编号；只有 bits[3:0] 非 0 才是 IT 块。
                    // 依据：`IT EQ` 汇编为 0xBF08、`IT NE` 为 0xBF18，
                    // 而 NOP/YIELD/WFE/WFI/SEV/SEVL 分别是 0xBF00/10/20/30/40/50。
                    // Per ARMv8-M, bits[3:0] == 0 selects the hint space with
                    // the hint number in bits[7:4]; only a non-zero mask is IT.
                    let firstcond = ((opcode >> 4) & 0xF) as u8;
                    let mask = (opcode & 0xF) as u8;
                    if mask == 0 {
                        // 提示语义：WFE 只在事件寄存器清零时睡眠；WFI 总是睡眠；
                        // SEV 置本核事件并锁存跨核信号；SEVL 只置本核事件。
                        // NOP/YIELD 与手册保留的 hint 编码按 NOP 执行。
                        // Hint semantics: WFE sleeps only while the event register is
                        // clear; WFI always sleeps; SEV sets the local event and
                        // latches the cross-core signal; SEVL only sets the local
                        // event. NOP/YIELD and the reserved hint encodings are NOPs.
                        return Ok(match firstcond {
                            0x2 => {
                                if regs.take_event() {
                                    regs.set_wait_on_event(false);
                                    StepResult::Ok
                                } else {
                                    regs.set_wait_on_event(true);
                                    StepResult::Sleep
                                }
                            }
                            // WFI 与事件寄存器无关，ARM 规定总是进入等待。
                            // WFI ignores the event register and always waits.
                            0x3 => {
                                regs.set_wait_on_event(false);
                                StepResult::Sleep
                            }
                            0x4 => {
                                regs.set_event();
                                regs.set_sev_signal();
                                StepResult::Ok
                            }
                            0x5 => {
                                regs.set_event();
                                StepResult::Ok
                            }
                            _ => StepResult::Ok,
                        });
                    }
                    // mask 非 0 才是 IT；AL（0xE）与 NV（0xF）条件非法。
                    // A non-zero mask selects IT; AL (0xE) and NV (0xF) are illegal.
                    if firstcond == 0xF || firstcond == 0xE {
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
            // LDMIA Rn, {reg_list} / 多加载后递增
            // 若基址寄存器本身在加载列表中，架构规定基址取加载值、不执行写回；
            // 编译器（例如 MicroPython 的 `ldmia r2,{r0,r1,r2}`）依赖该语义。
            // If the base register is also loaded, the loaded value wins and the
            // write-back is not performed; compilers (e.g. MicroPython's
            // `ldmia r2,{r0,r1,r2}`) rely on this behaviour.
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
            if (reg_list >> rb) & 1 == 0 {
                regs.r[rb] = addr;
            }
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

    // 恢复 IT 块内不应被更新的标志位（见函数开头的说明）。
    // Restore the flags that must not be updated inside an IT block (see the
    // note at the top of this function).
    if suppress_flags {
        regs.xpsr = (regs.xpsr & !0xF000_0000) | saved_flags;
    }

    *cycles += 1;
    Ok(StepResult::Ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ldmia_base_in_list_keeps_the_loaded_value() {
        // `ldmia r2,{r0,r1,r2}` is the 3-word copy idiom GCC emits (seen in
        // MicroPython's `mp_lexer_new_from_str_len` reader copy). Armv8-M keeps
        // the loaded value in the base register instead of writing back the
        // post-increment address, which is what the caller expects.
        // `ldmia r2,{r0,r1,r2}` 是 GCC 生成的三字拷贝惯用法（见于 MicroPython
        // `mp_lexer_new_from_str_len` 的 reader 拷贝）。Armv8-M 规定基址取得
        // 加载值，而不是写回后递增地址，调用者依赖这一语义。
        let mut regs = CortexM33Registers::new();
        regs.r[2] = 0x100;
        let mut bus = rp2350_core::memory::SimpleBus::new();
        bus.register_region(Box::new(rp2350_core::memory::RamRegion::new(
            "ram", 0x100, 0x100,
        )))
        .unwrap();
        let mut cycles = 0;
        bus.write_u32(0x100, 0x1111_1111, &mut cycles).unwrap();
        bus.write_u32(0x104, 0x2222_2222, &mut cycles).unwrap();
        bus.write_u32(0x108, 0x3333_3333, &mut cycles).unwrap();
        let mut nvic = Nvic::new();
        let mut systick = SysTick::new();
        let mut it = ITBlockState::new();
        let result = execute_thumb16(
            0xCA07,
            &mut regs,
            &mut bus,
            &mut nvic,
            &mut systick,
            &mut it,
            &mut cycles,
        )
        .unwrap();
        assert_eq!(result, StepResult::Ok);
        assert_eq!(regs.r[0], 0x1111_1111);
        assert_eq!(regs.r[1], 0x2222_2222);
        assert_eq!(regs.r[2], 0x3333_3333, "base must hold the loaded value");
    }

    #[test]
    fn ldmia_without_base_in_list_writes_back() {
        // `ldmia r2,{r0,r1}` still updates the base to the post-increment
        // address because r2 is not part of the loaded register set.
        // `ldmia r2,{r0,r1}` 的基址不在加载列表中，仍写回后递增地址。
        let mut regs = CortexM33Registers::new();
        regs.r[2] = 0x100;
        let mut bus = rp2350_core::memory::SimpleBus::new();
        bus.register_region(Box::new(rp2350_core::memory::RamRegion::new(
            "ram", 0x100, 0x100,
        )))
        .unwrap();
        let mut cycles = 0;
        bus.write_u32(0x100, 0xAAAA_AAAA, &mut cycles).unwrap();
        bus.write_u32(0x104, 0xBBBB_BBBB, &mut cycles).unwrap();
        let mut nvic = Nvic::new();
        let mut systick = SysTick::new();
        let mut it = ITBlockState::new();
        let result = execute_thumb16(
            0xCA03,
            &mut regs,
            &mut bus,
            &mut nvic,
            &mut systick,
            &mut it,
            &mut cycles,
        )
        .unwrap();
        assert_eq!(result, StepResult::Ok);
        assert_eq!(regs.r[0], 0xAAAA_AAAA);
        assert_eq!(regs.r[1], 0xBBBB_BBBB);
        assert_eq!(regs.r[2], 0x108);
    }

    #[test]
    fn adds_register_updates_carry_for_wide_arithmetic() {
        // `adds r0,r0,r1` with 0xFFFF_FFFF + 1 must set C and Z so the
        // following `adcs.w` sees the carry in a 64-bit addition.
        // `adds r0,r0,r1` 在 0xFFFF_FFFF + 1 时必须置位 C 与 Z，使后续
        // `adcs.w` 在 64 位加法中取得进位。
        let mut regs = CortexM33Registers::new();
        regs.r[0] = 0xFFFF_FFFF;
        regs.r[1] = 1;
        let mut bus = rp2350_core::memory::SimpleBus::new();
        let mut nvic = Nvic::new();
        let mut systick = SysTick::new();
        let mut it = ITBlockState::new();
        let mut cycles = 0;
        execute_thumb16(
            0x1840,
            &mut regs,
            &mut bus,
            &mut nvic,
            &mut systick,
            &mut it,
            &mut cycles,
        )
        .unwrap();
        assert_eq!(regs.r[0], 0);
        assert!(regs.apsr_c(), "carry-out must be visible");
        assert!(regs.apsr_z(), "zero flag must be visible");
    }

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

    #[test]
    fn cbz_large_offset_is_forward_unsigned() {
        // 0xB3C8 = CBZ r0, +0x72：偏移 >= 64 字节时仍是前向跳转，
        // 不能被当成 7 位有符号数反向跳到 0x1000_014E。
        // 0xB3C8 = CBZ r0, +0x72: offsets >= 64 bytes are still forward, so
        // this must not be sign-extended into a backwards branch.
        let mut regs = CortexM33Registers::new();
        regs.pc = 0x1000_0162;
        regs.r[0] = 0;
        let mut bus = rp2350_core::memory::SimpleBus::new();
        let mut nvic = Nvic::new();
        let mut systick = SysTick::new();
        let mut it = ITBlockState::new();
        let mut cycles = 0;
        let result = execute_thumb16(
            0xB3C8,
            &mut regs,
            &mut bus,
            &mut nvic,
            &mut systick,
            &mut it,
            &mut cycles,
        )
        .unwrap();
        assert_eq!(result, StepResult::BranchTaken);
        assert_eq!(regs.pc, 0x1000_01D6);
    }

    #[test]
    fn cbnz_large_offset_is_forward_unsigned() {
        // 0xBBC8 = CBNZ r0, +0x72，与 CBZ 共用同一偏移编码规则。
        // 0xBBC8 = CBNZ r0, +0x72, sharing CBZ's offset encoding.
        let mut regs = CortexM33Registers::new();
        regs.pc = 0x1000_0162;
        regs.r[0] = 1;
        let mut bus = rp2350_core::memory::SimpleBus::new();
        let mut nvic = Nvic::new();
        let mut systick = SysTick::new();
        let mut it = ITBlockState::new();
        let mut cycles = 0;
        let result = execute_thumb16(
            0xBBC8,
            &mut regs,
            &mut bus,
            &mut nvic,
            &mut systick,
            &mut it,
            &mut cycles,
        )
        .unwrap();
        assert_eq!(result, StepResult::BranchTaken);
        assert_eq!(regs.pc, 0x1000_01D6);
    }

    /// Execute one Thumb-16 opcode with `value` in `rm` and return the registers.
    /// 在 `rm` 中放入 `value` 执行一条 Thumb-16 指令并返回寄存器。
    fn run16(opcode: u16, rm: usize, value: u32) -> CortexM33Registers {
        let mut regs = CortexM33Registers::new();
        regs.r[rm] = value;
        let mut bus = rp2350_core::memory::SimpleBus::new();
        let mut nvic = Nvic::new();
        let mut systick = SysTick::new();
        let mut it = ITBlockState::new();
        let mut cycles = 0;
        execute_thumb16(
            opcode,
            &mut regs,
            &mut bus,
            &mut nvic,
            &mut systick,
            &mut it,
            &mut cycles,
        )
        .unwrap();
        regs
    }

    #[test]
    fn extend_instructions_sign_and_zero_extend() {
        // uxtb r0, r4 / sxtb r0, r4 / uxth r0, r4 / sxth r0, r4
        assert_eq!(run16(0xB2E0, 4, 0x1234_56FF).r[0], 0x0000_00FF);
        assert_eq!(run16(0xB260, 4, 0x0000_00FF).r[0], 0xFFFF_FFFF);
        assert_eq!(run16(0xB2A0, 4, 0x1234_5678).r[0], 0x0000_5678);
        assert_eq!(run16(0xB220, 4, 0x0000_8000).r[0], 0xFFFF_8000);
    }

    #[test]
    fn thumb16_extend_instructions_leave_apsr_unchanged() {
        // ARMv8-M: the 16-bit SXTB/SXTH/UXTB/UXTH encodings (0xB2 group) do
        // not affect APSR, unlike their 32-bit register forms which update
        // N/Z. Pre-set NZCV in both polarities and check that each extend
        // instruction leaves XPSR bit-for-bit identical.
        // ARMv8-M：16 位 SXTB/SXTH/UXTB/UXTH（0xB2 组）不影响 APSR，与更新
        // N/Z 的 32 位寄存器形式不同。预先以两种极性置位 NZCV，验证每条
        // 扩展指令执行后 XPSR 逐位不变。
        for opcode in [0xB260u16, 0xB220, 0xB2E0, 0xB2A0] {
            for (value, (n, z, c, v)) in [
                (0x0000_0000u32, (true, false, true, false)),
                (0x0000_0080u32, (false, true, false, true)),
            ] {
                let mut regs = CortexM33Registers::new();
                regs.r[4] = value;
                regs.set_apsr_nzcv(n, z, c, v);
                let expected = regs.xpsr;
                let mut bus = rp2350_core::memory::SimpleBus::new();
                let mut it = ITBlockState::new();
                assert_eq!(step16(opcode, &mut regs, &mut bus, &mut it), StepResult::Ok);
                assert_eq!(
                    regs.xpsr, expected,
                    "opcode {opcode:#06X} must not change APSR for input {value:#010X}"
                );
            }
        }
    }

    #[test]
    fn reverse_instructions_reorder_bytes() {
        // rev r0, r4 / rev16 r0, r4 / revsh r0, r4
        assert_eq!(run16(0xBA20, 4, 0x1234_5678).r[0], 0x7856_3412);
        assert_eq!(run16(0xBA60, 4, 0x1234_5678).r[0], 0x3412_7856);
        assert_eq!(run16(0xBAE0, 4, 0x1234_5678).r[0], 0x0000_7856);
        assert_eq!(run16(0xBAE0, 4, 0x1234_80FF).r[0], 0xFFFF_FF80);
    }

    /// Execute one Thumb-16 opcode against shared CPU state.
    /// 用共享的 CPU 状态执行一条 Thumb-16 指令。
    fn step16(
        opcode: u16,
        regs: &mut CortexM33Registers,
        bus: &mut rp2350_core::memory::SimpleBus,
        it: &mut ITBlockState,
    ) -> StepResult {
        let mut nvic = Nvic::new();
        let mut systick = SysTick::new();
        let mut cycles = 0;
        execute_thumb16(opcode, regs, bus, &mut nvic, &mut systick, it, &mut cycles).unwrap()
    }

    #[test]
    fn sev_hint_does_not_swallow_following_pop() {
        // 0xBF40 SEV 是 hint，不是 IT MI；后随 pop {r4, pc} 必须执行。
        // 0xBF40 SEV is a hint, not IT MI; the following pop must execute.
        let mut regs = CortexM33Registers::new();
        regs.sp = 0x2000_1000;
        let mut bus = rp2350_core::memory::SimpleBus::new();
        bus.register_region(Box::new(rp2350_core::memory::RamRegion::new(
            "stack",
            0x2000_1000,
            0x10,
        )))
        .unwrap();
        let mut cycles = 0;
        bus.write_u32(0x2000_1000, 0xAAAA_AAAA, &mut cycles)
            .unwrap();
        bus.write_u32(0x2000_1004, 0x1000_0001, &mut cycles)
            .unwrap();
        let mut it = ITBlockState::new();
        assert_eq!(step16(0xBF40, &mut regs, &mut bus, &mut it), StepResult::Ok);
        assert!(!it.in_it_block());
        assert_eq!(
            step16(0xBD10, &mut regs, &mut bus, &mut it),
            StepResult::BranchTaken
        );
        assert_eq!(regs.r[4], 0xAAAA_AAAA);
        assert_eq!(regs.pc, 0x1000_0000);
        assert_eq!(regs.sp, 0x2000_1008);
    }

    #[test]
    fn nop_hint_does_not_skip_next_instruction() {
        // 0xBF00 NOP 不得被当成 IT EQ：Z=0 时下一条仍要执行。
        // NOP must not become IT EQ: with Z clear the next instruction still runs.
        let mut regs = CortexM33Registers::new();
        regs.set_nz_flags(1); // Z = 0
        let mut bus = rp2350_core::memory::SimpleBus::new();
        let mut it = ITBlockState::new();
        assert_eq!(step16(0xBF00, &mut regs, &mut bus, &mut it), StepResult::Ok);
        // movs r0, #1
        assert_eq!(step16(0x2001, &mut regs, &mut bus, &mut it), StepResult::Ok);
        assert_eq!(regs.r[0], 1);
    }

    #[test]
    fn wfe_and_wfi_hints_return_sleep() {
        // 0xBF20 WFE / 0xBF30 WFI 必须表达为 Sleep，且不武装 IT 块。
        // WFE / WFI must surface as Sleep and must not arm an IT block.
        let mut regs = CortexM33Registers::new();
        let mut bus = rp2350_core::memory::SimpleBus::new();
        let mut it = ITBlockState::new();
        assert_eq!(
            step16(0xBF20, &mut regs, &mut bus, &mut it),
            StepResult::Sleep
        );
        assert_eq!(
            step16(0xBF30, &mut regs, &mut bus, &mut it),
            StepResult::Sleep
        );
        assert!(!it.in_it_block());
    }

    #[test]
    fn ite_ne_runs_the_eq_slot_when_z_is_set() {
        // 复现固件 0x100309EA 处的用法：TST 结果为 0（Z=1）时 ITE NE 的第一槽
        // 跳过、第二槽（EQ）执行。
        // Mirrors the firmware at 0x100309EA: with Z set (TST result zero),
        // ITE NE skips slot 1 and executes slot 2 (EQ).
        let mut regs = CortexM33Registers::new();
        regs.r[0] = 0x2000_5024;
        regs.set_nz_flags(0); // Z = 1, NE false
        let mut bus = rp2350_core::memory::SimpleBus::new();
        let mut it = ITBlockState::new();
        assert_eq!(step16(0xBF14, &mut regs, &mut bus, &mut it), StepResult::Ok);
        // movne r0, #1 — skipped
        assert_eq!(step16(0x2001, &mut regs, &mut bus, &mut it), StepResult::Ok);
        assert_eq!(regs.r[0], 0x2000_5024);
        // moveq r0, #0 — executed
        assert_eq!(step16(0x2000, &mut regs, &mut bus, &mut it), StepResult::Ok);
        assert_eq!(regs.r[0], 0);
        assert!(!it.in_it_block());
    }

    #[test]
    fn negs_negates_the_source_register_not_the_destination() {
        // 0x425D = NEGS r5, r3：Rdn=5 只是结果寄存器，取反的是 Rm=3。
        // 固件在 0x100314F4 用它把 RM 基址转成数组下标；若错误地取反 Rdn，
        // 地址会算出 0x1F40xxxx 这类越界值。
        // 0x425D is NEGS r5, r3: Rdn=5 only receives the result, Rm=3 is
        // negated. The firmware relies on this to map a base address to an
        // array index; negating Rdn instead yields an out-of-range address.
        let mut regs = CortexM33Registers::new();
        regs.r[3] = 0xFFFF_8000;
        regs.r[5] = 0x400B_8000;
        let mut bus = rp2350_core::memory::SimpleBus::new();
        let mut it = ITBlockState::new();
        assert_eq!(step16(0x425D, &mut regs, &mut bus, &mut it), StepResult::Ok);
        assert_eq!(regs.r[5], 0x0000_8000);
        assert!(!regs.apsr_z());
        assert!(!regs.apsr_c());
    }

    #[test]
    fn negs_of_zero_sets_carry_and_zero() {
        // 0 - 0 = 0：C = 1（无借位），Z = 1。
        // 0 - 0 = 0 sets C (no borrow) and Z.
        let mut regs = CortexM33Registers::new();
        regs.r[3] = 0;
        regs.r[5] = 0x1234_5678;
        let mut bus = rp2350_core::memory::SimpleBus::new();
        let mut it = ITBlockState::new();
        assert_eq!(step16(0x425D, &mut regs, &mut bus, &mut it), StepResult::Ok);
        assert_eq!(regs.r[5], 0);
        assert!(regs.apsr_z());
        assert!(regs.apsr_c());
    }

    #[test]
    fn real_it_instruction_still_arms_it_block() {
        // 0xBF0C = ITE EQ：Z=0 时第一条跳过、第二条（NE）执行。
        // 0xBF0C ITE EQ with Z clear: skip the EQ slot, execute the NE slot.
        let mut regs = CortexM33Registers::new();
        regs.set_nz_flags(1); // Z = 0
        let mut bus = rp2350_core::memory::SimpleBus::new();
        let mut it = ITBlockState::new();
        assert_eq!(step16(0xBF0C, &mut regs, &mut bus, &mut it), StepResult::Ok);
        assert!(it.in_it_block());
        // movs r0, #7 — EQ false, skipped
        assert_eq!(step16(0x2007, &mut regs, &mut bus, &mut it), StepResult::Ok);
        assert_eq!(regs.r[0], 0);
        // movs r1, #9 — NE true, executed
        assert_eq!(step16(0x2109, &mut regs, &mut bus, &mut it), StepResult::Ok);
        assert_eq!(regs.r[1], 9);
        assert!(!it.in_it_block());
    }

    #[test]
    fn sixteen_bit_add_inside_it_block_keeps_flags() {
        // 复现 strlen（0x100397D0）的 `itt eq { ldreq.w; addeq r0,#4 }` 后紧跟
        // `beq`：IT 块内的 16 位 ADDS 不得清除 Z，否则循环提前退出。
        // Reproduce strlen's `itt eq { ldreq.w; addeq r0,#4 }` followed by
        // `beq`: the 16-bit ADDS in an IT slot must not clear Z, or the loop
        // exits early.
        let mut regs = CortexM33Registers::new();
        regs.set_apsr_nzcv(false, true, false, false); // Z = 1
        regs.r[0] = 0xFFFF_FFFE;
        let mut bus = rp2350_core::memory::SimpleBus::new();
        let mut it = ITBlockState::new();
        // itt eq
        assert_eq!(step16(0xBF04, &mut regs, &mut bus, &mut it), StepResult::Ok);
        // addeq r0, #4 → r0 = 2, but Z must stay set for the following beq.
        assert_eq!(step16(0x3004, &mut regs, &mut bus, &mut it), StepResult::Ok);
        assert_eq!(regs.r[0], 2);
        assert!(regs.apsr_z(), "IT slot must not update Z");
    }

    #[test]
    fn sixteen_bit_add_outside_it_block_sets_flags() {
        // 同样的 ADDS 在 IT 块外必须照常更新标志（Z 清零）。
        // The same ADDS outside an IT block must update flags (Z cleared).
        let mut regs = CortexM33Registers::new();
        regs.set_apsr_nzcv(false, true, false, false); // Z = 1
        regs.r[0] = 0xFFFF_FFFE;
        let mut bus = rp2350_core::memory::SimpleBus::new();
        let mut it = ITBlockState::new();
        assert_eq!(step16(0x3004, &mut regs, &mut bus, &mut it), StepResult::Ok);
        assert_eq!(regs.r[0], 2);
        assert!(!regs.apsr_z(), "outside IT the ADDS must clear Z");
    }

    #[test]
    fn compare_inside_it_block_still_sets_flags() {
        // CMP 属于无条件写标志的指令，IT 块内仍更新 Z。
        // CMP always writes flags, so it must update Z even inside an IT block.
        let mut regs = CortexM33Registers::new();
        regs.set_apsr_nzcv(false, true, false, false); // Z = 1 so the EQ slot runs
        regs.r[0] = 5;
        regs.r[1] = 6;
        let mut bus = rp2350_core::memory::SimpleBus::new();
        let mut it = ITBlockState::new();
        // itt eq
        assert_eq!(step16(0xBF04, &mut regs, &mut bus, &mut it), StepResult::Ok);
        // cmp r0, r1 (0x4288): 5 != 6 → it must clear Z even inside the block.
        assert_eq!(step16(0x4288, &mut regs, &mut bus, &mut it), StepResult::Ok);
        assert!(!regs.apsr_z(), "CMP must update Z even inside IT");
    }
}
