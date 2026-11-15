//! Shared Thumb-16 executors used by both the decoder and the decode cache
//! 译码器与译码缓存共用的 Thumb-16 执行器
//!
//! The bodies below are the hot arms of [`super::execute_thumb16`]. They live
//! here so the cached path and the full decoder run **the same code**: the
//! cache never holds a second, potentially divergent implementation.
//! 下面的函数体是 [`super::execute_thumb16`] 的热点分支。把它们放在这里，是为了让
//! 缓存路径与完整译码器执行**同一份代码**：缓存绝不持有第二份可能分叉的实现。
//!
//! Every function here is written to be equivalent to its original arm, including
//! the cycle bookings and the point at which the flags are updated.
//! 这里的每个函数都与其原始分支等价，包括周期记账与标志位更新的时机。

use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;

use super::{SleepKind, StepResult, adc, asr_c, lsl_c, lsr_c, ror_c, sbc, sign_extend};
use crate::conditions::Condition;
use crate::registers::CortexM33Registers;

/// `LDR Rd, [PC, #imm8*4]` — literal load / 文字池加载
#[inline]
pub(crate) fn load_literal(
    opcode: u16,
    regs: &mut CortexM33Registers,
    bus: &mut dyn AddressBus,
    cycles: &mut u64,
) -> Result<StepResult, CoreError> {
    let rd = ((opcode >> 8) & 0x7) as usize;
    let imm8 = (opcode & 0xFF) as u32;
    let addr = (regs.pc.wrapping_add(2) & !0x3u32).wrapping_add(imm8 << 2);
    regs.r[rd] = bus.read_u32(addr, cycles)?;
    *cycles += 1;
    Ok(StepResult::Ok)
}

/// `STR`/`LDR (immediate)` word forms / `STR`/`LDR（立即数）` 字形式
///
/// `store` selects the direction; the encoder bit 11 decides it in the decoder.
/// `store` 选择方向；译码器中由编码位 11 决定。
#[inline]
pub(crate) fn load_store_word_imm5(
    opcode: u16,
    store: bool,
    regs: &mut CortexM33Registers,
    bus: &mut dyn AddressBus,
    cycles: &mut u64,
) -> Result<StepResult, CoreError> {
    let imm5 = ((opcode >> 6) & 0x1F) as u32;
    let rb = ((opcode >> 3) & 0x7) as usize;
    let rd = (opcode & 0x7) as usize;
    let addr = regs.r[rb].wrapping_add(imm5 << 2);
    if store {
        // STR Rd, [Rb, #imm5*4]
        bus.write_u32(addr, regs.r[rd], cycles)?;
    } else {
        // LDR Rd, [Rb, #imm5*4]
        regs.r[rd] = bus.read_u32(addr, cycles)?;
    }
    *cycles += 1;
    Ok(StepResult::Ok)
}

/// Data-processing (register): ANDS, EORS, LSLS, LSRS, ASRS, ADCS, SBCS, RORS,
/// TST, RSBS, CMP, CMN, ORRS, MULS, BICS, MVNS
/// 数据处理（寄存器）：ANDS、EORS、LSLS、LSRS、ASRS、ADCS、SBCS、RORS、
/// TST、RSBS、CMP、CMN、ORRS、MULS、BICS、MVNS
///
/// The caller keeps the shared `suppress_flags` tail, so this returns nothing.
/// 调用方保留共享的 `suppress_flags` 尾部处理，因此本函数不返回值。
#[inline]
pub(crate) fn dp_register(opcode: u16, regs: &mut CortexM33Registers) {
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
            // RSBS / 反减（0 - Rm）
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
        _ => {
            // MVNS / 按位取反
            let result = !rm_val;
            regs.r[rdn] = result;
            regs.set_nz_flags(result);
        }
    }
}

/// `B<cond>` or `SVC` / `B<cond>` 或 `SVC`
#[inline]
pub(crate) fn branch_conditional(opcode: u16, regs: &mut CortexM33Registers) -> StepResult {
    let cond_bits = ((opcode >> 8) & 0xF) as u8;
    if cond_bits == 0xF {
        return StepResult::SvCall((opcode & 0xFF) as u8);
    }
    let cond = Condition::from_bits(cond_bits);
    if cond != Condition::Al && cond.evaluate(regs) {
        let offset = sign_extend(((opcode & 0xFF) as u32) << 1, 9);
        regs.pc = regs.pc.wrapping_add(2).wrapping_add(offset);
        return StepResult::BranchTaken;
    }
    StepResult::Ok
}

/// `B` (unconditional branch) / `B`（无条件分支）
#[inline]
pub(crate) fn branch_unconditional(opcode: u16, regs: &mut CortexM33Registers) -> StepResult {
    let offset = sign_extend(((opcode & 0x7FF) as u32) << 1, 12);
    regs.pc = regs.pc.wrapping_add(2).wrapping_add(offset);
    StepResult::BranchTaken
}

/// Hint space (`NOP`/`YIELD`/`WFE`/`WFI`/`SEV`/`SEVL`) or an `IT` block start
/// 提示空间（`NOP`/`YIELD`/`WFE`/`WFI`/`SEV`/`SEVL`）或 `IT` 块起始
///
/// Returns `Some` for the hint space; `None` means an `IT` block was installed
/// and the caller must fall through to the shared tail.
/// 提示空间返回 `Some`；`None` 表示安装了 `IT` 块，调用方必须继续走到共享尾部。
#[inline]
pub(crate) fn hint_space(
    opcode: u16,
    it: &mut crate::conditions::ITBlockState,
) -> Option<StepResult> {
    // 编码按 ARMv8-M 手册：bits[3:0] 为 0 时进入 hint 空间，
    // bits[7:4] 为提示编号；只有 bits[3:0] 非 0 才是 IT 块。
    // Per ARMv8-M, bits[3:0] == 0 selects the hint space with the hint number
    // in bits[7:4]; only a non-zero mask is IT.
    let firstcond = ((opcode >> 4) & 0xF) as u8;
    let mask = (opcode & 0xF) as u8;
    if mask == 0 {
        return Some(match firstcond {
            // WFE / WFI：显式请求等待，由核心按实时事件/NVIC 状态决定是否挂起。
            // WFE / WFI: an explicit wait request; the core decides whether to
            // suspend from live event/NVIC state.
            0x2 => StepResult::Sleep(SleepKind::WaitForEvent),
            0x3 => StepResult::Sleep(SleepKind::WaitForInterrupt),
            // SEV 置位所有核心的事件寄存器；SEVL 只置位本核。
            // SEV sets every core's event register; SEVL only this core's.
            0x4 => StepResult::SendEvent { local: false },
            0x5 => StepResult::SendEvent { local: true },
            // NOP / YIELD 以及手册保留的 hint 编码（0x6..=0xF）均按 NOP 执行。
            // NOP / YIELD and the reserved hint encodings (0x6..=0xF) all
            // execute as NOP.
            _ => StepResult::Ok,
        });
    }
    // mask 非 0 才是 IT；AL（0xE）与 NV（0xF）条件非法。
    // A non-zero mask selects IT; AL (0xE) and NV (0xF) are illegal.
    if firstcond == 0xF || firstcond == 0xE {
        return Some(StepResult::Undefined);
    }
    it.execute_it(firstcond, mask);
    None
}

/// `CPS` for PRIMASK/FAULTMASK / 用于 PRIMASK/FAULTMASK 的 `CPS`
#[inline]
pub(crate) fn cps(opcode: u16, regs: &mut CortexM33Registers) {
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
