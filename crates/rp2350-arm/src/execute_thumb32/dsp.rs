//! Thumb-32 DSP packed SIMD addition and subtraction
//! Thumb-32 DSP 打包 SIMD 加/减指令
//!
//! Covers the parallel byte/halfword add and subtract instructions
//! (`SADD8`/`SSUB8`/`SADD16`/`SSUB16` and their unsigned `U` counterparts).
//! Every lane is computed independently, results wrap modulo 2^8 or 2^16 and
//! APSR.GE receives one flag per byte lane or a duplicated pair per halfword
//! lane. N, Z, C, V and Q are never modified.
//! 覆盖并行字节/半字加/减指令（`SADD8`/`SSUB8`/`SADD16`/`SSUB16` 及无符号
//! `U` 变体）。每个通道独立运算，结果按 2^8 或 2^16 取模回绕，APSR.GE 为每个
//! 字节通道写入一个标志、为每个半字通道写入一对重复标志。N、Z、C、V、Q 不变。

use crate::execute_thumb16::StepResult;
use crate::registers::CortexM33Registers;

/// Try to execute a packed parallel add/subtract instruction
/// 尝试执行打包并行加/减指令
///
/// The Thumb-32 encodings are `1111 1010 10xx Rn 1111 Rd 0u00 Rm`:
/// bits[23:20] select `ADD8` (0x8), `ADD16` (0x9), `SUB8` (0xC) or `SUB16`
/// (0xD), and second-halfword bit 6 selects the unsigned variant. Only the
/// plain saturating-free forms with bits[7:4] = 0b0000 (signed) or 0b0100
/// (unsigned) are claimed; the Q/SH/UQ/UH variants remain undefined.
/// Thumb-32 编码为 `1111 1010 10xx Rn 1111 Rd 0u00 Rm`：bits[23:20] 选择
/// `ADD8`（0x8）、`ADD16`（0x9）、`SUB8`（0xC）或 `SUB16`（0xD），第二半字
/// 位 6 选择无符号变体。仅认领 bits[7:4] = 0b0000（有符号）或 0b0100
/// （无符号）的普通形式；Q/SH/UQ/UH 变体仍视为未定义。
pub(super) fn try_execute(instr: u32, regs: &mut CortexM33Registers) -> Option<StepResult> {
    let op = (instr >> 20) & 0xF;
    let lane_bits = match op {
        0x8 | 0xC => 8,
        0x9 | 0xD => 16,
        _ => return None,
    };
    let op2 = (instr >> 4) & 0xF;
    if (instr >> 12) & 0xF != 0xF || !matches!(op2, 0x0 | 0x4) {
        return None;
    }

    let rd = ((instr >> 8) & 0xF) as u8;
    if rd == 15 {
        // Writing PC is UNPREDICTABLE for these encodings.
        // 这些编码写 PC 属于不可预测行为。
        return Some(StepResult::Undefined);
    }
    let rn = ((instr >> 16) & 0xF) as u8;
    let rm = (instr & 0xF) as u8;
    let subtract = matches!(op, 0xC | 0xD);
    let unsigned = op2 == 0x4;

    let (result, ge) = if lane_bits == 8 {
        add_sub_bytes(regs.read(rn), regs.read(rm), subtract, unsigned)
    } else {
        add_sub_halfwords(regs.read(rn), regs.read(rm), subtract, unsigned)
    };

    regs.write(rd, result);
    regs.set_apsr_ge(ge);
    Some(StepResult::Ok)
}

/// Add or subtract four independent byte lanes
/// 对四个独立字节通道做加/减
///
/// Signed lanes set GE when the full-width sum/difference is greater than or
/// equal to zero. Unsigned lanes use the carry-out/no-borrow rule, exactly
/// like the C flag of `ADDS`/`SUBS`.
/// 有符号通道在完整宽度和/差大于等于零时置 GE；无符号通道采用进位/无借位
/// 规则，与 `ADDS`/`SUBS` 的 C 标志一致。
fn add_sub_bytes(a: u32, b: u32, subtract: bool, unsigned: bool) -> (u32, u32) {
    let mut result = 0u32;
    let mut ge = 0u32;
    for i in 0..4u32 {
        let a_lane = ((a >> (i * 8)) & 0xFF) as i32;
        let b_lane = ((b >> (i * 8)) & 0xFF) as i32;
        let (lane, lane_ge) = if unsigned {
            if subtract {
                let diff = a_lane - b_lane;
                (diff, diff >= 0)
            } else {
                let sum = a_lane + b_lane;
                (sum, sum > 0xFF)
            }
        } else {
            // Toggle the sign bit to reinterpret the byte as i8 without casts.
            // 翻转符号位，以无强制转换的方式把字节解释为 i8。
            let a_signed = (a_lane ^ 0x80) - 0x80;
            let b_signed = (b_lane ^ 0x80) - 0x80;
            if subtract {
                let diff = a_signed - b_signed;
                (diff, diff >= 0)
            } else {
                let sum = a_signed + b_signed;
                (sum, sum >= 0)
            }
        };
        result |= ((lane as u32) & 0xFF) << (i * 8);
        if lane_ge {
            ge |= 1 << i;
        }
    }
    (result, ge)
}

/// Add or subtract two independent halfword lanes
/// 对两个独立半字通道做加/减
///
/// A halfword lane drives a whole GE pair: `GE[1:0]` for `bits[15:0]` and
/// `GE[3:2]` for `bits[31:16]`, with both bits of the pair set or cleared
/// together.
/// 一个半字通道驱动整对 GE：`bits[15:0]` 对应 `GE[1:0]`，`bits[31:16]`
/// 对应 `GE[3:2]`，每对中的两位一起置位或一起清零。
fn add_sub_halfwords(a: u32, b: u32, subtract: bool, unsigned: bool) -> (u32, u32) {
    let mut result = 0u32;
    let mut ge = 0u32;
    for i in 0..2u32 {
        let a_lane = ((a >> (i * 16)) & 0xFFFF) as i32;
        let b_lane = ((b >> (i * 16)) & 0xFFFF) as i32;
        let (lane, lane_ge) = if unsigned {
            if subtract {
                let diff = a_lane - b_lane;
                (diff, diff >= 0)
            } else {
                let sum = a_lane + b_lane;
                (sum, sum > 0xFFFF)
            }
        } else {
            // Toggle the sign bit to reinterpret the halfword as i16.
            // 翻转符号位，把半字解释为 i16。
            let a_signed = (a_lane ^ 0x8000) - 0x8000;
            let b_signed = (b_lane ^ 0x8000) - 0x8000;
            if subtract {
                let diff = a_signed - b_signed;
                (diff, diff >= 0)
            } else {
                let sum = a_signed + b_signed;
                (sum, sum >= 0)
            }
        };
        result |= ((lane as u32) & 0xFFFF) << (i * 16);
        if lane_ge {
            ge |= 0b11 << (i * 2);
        }
    }
    (result, ge)
}
