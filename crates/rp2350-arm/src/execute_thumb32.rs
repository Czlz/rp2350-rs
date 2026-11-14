//! Thumb-32 instruction decoder and executor
//! Thumb-32 指令解码器和执行器
//!
//! Decodes and executes 32-bit Thumb instructions following the ARMv7-M /
//! ARMv8-M top-level encoding hierarchy: the first halfword occupies
//! instruction bits 31:16, so `op1` is bits 28:27 and the primary
//! opcode field `op` is bits 26:20.
//!
//! Covered groups: data-processing (modified immediate, shifted register,
//! register), single data transfers (register offset, 12-bit immediate,
//! 8-bit pre/post-index and PC-relative literal), LDM/STM, LDRD/STRD,
//! LDREX/STREX, TBB/TBH, multiply/divide (including long multiply),
//! plain-binary immediate, bitfield, branches (B.W, B&lt;cond&gt;.W, BL, BLX),
//! and miscellaneous system instructions. DSP/SIMD/FPU encodings remain
//! undefined for now.
//!
//! 按 ARMv7-M / ARMv8-M 顶层编码层次解码并执行 32 位 Thumb 指令：
//! 第一个半字位于指令位段 31:16，因此 `op1` 为位段 28:27，主操作码
//! `op` 为位段 26:20。
//!
//! 已覆盖：数据处理（改良立即数、移位寄存器、寄存器）、单字数据传输
//! （寄存器偏移、12 位立即数、8 位前/后索引与 PC 相对字面量）、LDM/STM、
//! LDRD/STRD、LDREX/STREX、TBB/TBH、乘除法（含长乘法）、纯二进制立即数、
//! 位域、分支（B.W、B&lt;cond&gt;.W、BL、BLX）和杂项系统指令。
//! DSP/SIMD/FPU 编码暂时视为未定义。

// 指令解码器涉及大量位操作和类型转换，这是 ARM 指令集仿真的固有特性。
// Instruction decoders involve extensive bit manipulation and casts,
// inherent to ARM ISA emulation.
#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
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

mod branch_misc;
mod coproc;
mod data_processing;
mod dsp;
mod fpu;
mod fpu_arith;
mod load_store;
mod multiple;
mod multiply;

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

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
/// The 32-bit instruction word follows the ARM ARM layout with the first
/// halfword in bits 31:16. Decoding is layered exactly as in the reference
/// manual: `op1 = bits 28:27`, `op = bits 26:20`.
/// 32 位指令字遵循 ARM ARM 布局，第一个半字位于位段 31:16。解码层次与
/// 参考手册一致：`op1 = bits 28:27`，`op = bits 26:20`。
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

    // FPU (VFPv5-SP) data movement: extension-register transfers occupy
    // encodings that the integer decoder leaves undefined, so they are
    // decoded first. Non-FPU encodings return None and fall through.
    // FPU（VFPv5-SP）数据搬移：扩展寄存器传输占用整数解码器视为未定义的
    // 编码空间，因此先解码。非 FPU 编码返回 None 并继续常规解码。
    if let Some(result) = fpu::try_execute(instr, regs, bus, cycles)? {
        *cycles += 1;
        return Ok(result);
    }

    // GPIO coprocessor (CP0) and double-precision coprocessor (CP4) transfers
    // also occupy coprocessor encoding space left undefined by the integer
    // decoder. They run after the FPU so the shared `0xEC` space is resolved
    // in favour of the VFP `VMOV` encoding first.
    // GPIO 协处理器（CP0）与双精度协处理器（CP4）传输同样位于整数解码器
    // 视为未定义的协处理器编码空间。它们在 FPU 之后解码，使共享的 `0xEC`
    // 空间优先解析为 VFP `VMOV` 编码。
    if let Some(result) = coproc::try_execute(instr, regs, bus, cycles)? {
        *cycles += 1;
        return Ok(result);
    }

    let op1 = (instr >> 27) & 0x3;
    let op = (instr >> 20) & 0x7F;
    let second_halfword = instr & 0xFFFF;

    let result = match op1 {
        // 0b01: load/store multiple, dual, exclusive, table branch when
        // op2 == 0b00; data-processing (shifted register) when op2 == 0b01.
        // 0b01：op2 == 0b00 时为多寄存器加载/存储、双字、独占访问、表分支；
        // op2 == 0b01 时为数据处理（移位寄存器）。
        0b01 => {
            let op2 = (instr >> 25) & 0x3;
            if op2 == 0 {
                load_store::execute_group(instr, regs, bus, cycles)?
            } else if op2 == 0b01 {
                data_processing::execute_shifted_register(instr, regs)
            } else {
                StepResult::Undefined
            }
        }
        // 0b10: branch/misc when second halfword bit 15 is set; otherwise
        // bit 25 selects data-processing (modified immediate) when clear
        // and plain-binary immediate or bitfield when set.
        // 0b10：第二半字位 15 置位时为分支/杂项；否则位 25 清零表示
        // 数据处理（改良立即数），置位表示纯二进制立即数或位域。
        0b10 => {
            if second_halfword & 0x8000 != 0 {
                branch_misc::execute_branch_or_misc(instr, regs)
            } else if (instr >> 25) & 1 == 0 {
                data_processing::execute_modified_immediate(instr, regs)
            } else if matches!(op, 0x20..=0x2F | 0x60..=0x6F) {
                branch_misc::execute_plain_immediate(instr, regs)
            } else if matches!(op, 0x30 | 0x32 | 0x38 | 0x3A) {
                // SSAT/SSAT16/USAT/USAT16 share the plain-binary-immediate
                // group; the DSP unit owns their saturation semantics.
                // SSAT/SSAT16/USAT/USAT16 属于纯二进制立即数组，其饱和语义由
                // DSP 单元负责。
                dsp::execute_saturate(instr, regs)
            } else if matches!(op, 0x34 | 0x36 | 0x3C) {
                branch_misc::execute_bitfield(instr, regs)
            } else {
                StepResult::Undefined
            }
        }
        // 0b11: load/store single, data-processing (register), multiply/divide
        // 0b11：单字加载/存储、数据处理（寄存器）、乘法/除法
        0b11 => {
            if op < 0x20 {
                load_store::execute_single(instr, regs, bus, cycles)?
            } else if op < 0x30 {
                data_processing::execute_register_shifted_or_misc(instr, regs)
            } else if op <= 0x3F {
                multiply::execute(instr, regs)
            } else {
                StepResult::Undefined
            }
        }
        _ => StepResult::Undefined,
    };

    *cycles += 1;
    Ok(result)
}

/// Shift types used by the Thumb-32 data-processing encodings
/// Thumb-32 数据处理编码使用的移位类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShiftType {
    /// Logical shift left / 逻辑左移
    Lsl,
    /// Logical shift right / 逻辑右移
    Lsr,
    /// Arithmetic shift right / 算术右移
    Asr,
    /// Rotate right / 循环右移
    Ror,
}

impl ShiftType {
    /// Decode the two-bit shift type field
    /// 解码两位移位类型字段
    fn from_bits(bits: u32) -> Self {
        match bits & 0x3 {
            0 => Self::Lsl,
            1 => Self::Lsr,
            2 => Self::Asr,
            _ => Self::Ror,
        }
    }
}

/// Sign-extend a value from `bits` bits to 32 bits
/// 将 `bits` 位宽的值符号扩展到 32 位
fn sign_extend(value: u32, bits: u32) -> u32 {
    if bits == 0 || bits >= 32 {
        return value;
    }
    let mask = (1u32 << bits) - 1;
    let sign = 1u32 << (bits - 1);
    let truncated = value & mask;
    if truncated & sign != 0 {
        truncated | !mask
    } else {
        truncated
    }
}

/// Shift a value by an immediate-encoded amount, returning the carry-out
/// 按立即数编码的移位量移位，并返回移出位
///
/// Immediate encodings use `amount == 0` to mean 32 for LSR/ASR and RRX for
/// ROR. LSL with amount 0 leaves the value and carry unchanged.
/// 立即数编码中 LSR/ASR 的移位量为 0 表示 32，ROR 的 0 表示 RRX；
/// LSL 的 0 表示不移位且进位不变。
fn shift_immediate(value: u32, amount: u32, ty: ShiftType, carry_in: bool) -> (u32, bool) {
    match ty {
        ShiftType::Lsl => {
            if amount == 0 {
                (value, carry_in)
            } else if amount < 32 {
                (value << amount, (value >> (32 - amount)) & 1 != 0)
            } else if amount == 32 {
                (0, value & 1 != 0)
            } else {
                (0, false)
            }
        }
        ShiftType::Lsr => {
            let amount = if amount == 0 { 32 } else { amount };
            match amount {
                1..=31 => (value >> amount, (value >> (amount - 1)) & 1 != 0),
                32 => (0, value >> 31 != 0),
                _ => (0, false),
            }
        }
        ShiftType::Asr => {
            let amount = if amount == 0 { 32 } else { amount };
            if amount < 32 {
                (
                    ((value as i32) >> amount) as u32,
                    (value >> (amount - 1)) & 1 != 0,
                )
            } else {
                let filled = if value >> 31 != 0 { 0xFFFF_FFFF } else { 0 };
                (filled, value >> 31 != 0)
            }
        }
        ShiftType::Ror => {
            if amount == 0 {
                // RRX: 33-bit rotate right through carry
                // RRX：经进位标志的 33 位循环右移
                let result = (u32::from(carry_in) << 31) | (value >> 1);
                (result, value & 1 != 0)
            } else {
                let rotation = amount % 32;
                if rotation == 0 {
                    (value, value >> 31 != 0)
                } else {
                    let result = value.rotate_right(rotation);
                    (result, result >> 31 != 0)
                }
            }
        }
    }
}

/// Shift a value by a register-encoded amount, returning the carry-out
/// 按寄存器编码的移位量移位，并返回移出位
///
/// Register-encoded shifts treat an amount of 0 as "no shift" for every shift
/// type (unlike immediate encodings, `ROR #0` here is not RRX).
/// 寄存器编码移位对任意类型都以 0 表示不移位（与立即数不同，此处 ROR 0 不是 RRX）。
fn shift_register(value: u32, amount: u32, ty: ShiftType, carry_in: bool) -> (u32, bool) {
    if amount == 0 {
        (value, carry_in)
    } else {
        shift_immediate(value, amount, ty, carry_in)
    }
}

/// Add two operands with carry-in, returning result, carry and overflow
/// 带进位的加法，返回结果、进位与溢出
fn add_with_carry(a: u32, b: u32, carry_in: bool) -> (u32, bool, bool) {
    let wide = u64::from(a) + u64::from(b) + u64::from(carry_in);
    let result = wide as u32;
    let carry = wide > u64::from(u32::MAX);
    let overflow = ((a ^ result) & (b ^ result) & 0x8000_0000) != 0;
    (result, carry, overflow)
}

/// Subtract two operands with borrow, returning result, carry and overflow
/// 带借位的减法，返回结果、进位与溢出
///
/// `carry_in` follows the ARM convention: C == 1 means no borrow.
/// `carry_in` 遵循 ARM 约定：C 为 1 表示无借位。
fn sub_with_carry(a: u32, b: u32, carry_in: bool) -> (u32, bool, bool) {
    let borrow = if carry_in { 0i64 } else { 1i64 };
    let wide = i64::from(a) - i64::from(b) - borrow;
    let result = wide as u32;
    let carry = wide >= 0;
    let overflow = ((a ^ b) & (a ^ result) & 0x8000_0000) != 0;
    (result, carry, overflow)
}

/// Write NZCV flags for a result that also produces C and V
/// 为同时产生 C 和 V 的结果写入 NZCV 标志
fn set_arith_flags(regs: &mut CortexM33Registers, result: u32, carry: bool, overflow: bool) {
    regs.set_apsr_nzcv(result >> 31 != 0, result == 0, carry, overflow);
}

/// Write N, Z and C flags for a logical result, preserving V
/// 为逻辑运算结果写入 N、Z、C 标志，保持 V 不变
fn set_logic_flags(regs: &mut CortexM33Registers, result: u32, carry: bool) {
    regs.set_apsr_nzcv(result >> 31 != 0, result == 0, carry, regs.apsr_v());
}
