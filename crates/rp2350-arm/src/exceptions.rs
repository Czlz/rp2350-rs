//! Exception entry, exit, and stack frame management (ARMv8-M)
//! 异常进入、退出和栈帧管理（ARMv8-M）
//!
//! Implements exception stacking/unstacking, EXC_RETURN value decoding,
//! and the vector table fetch logic.
//! 实现异常压栈/出栈、EXC_RETURN 值解码和向量表获取逻辑。

// 异常处理涉及大量寄存器操作和栈帧管理，类型转换和文档标记是 ARM 体系仿真的固有特征。
// Exception handling involves extensive register ops and stack frame management;
// casts and doc markup patterns are inherent to ARM architecture emulation.
#![allow(
    clippy::doc_markdown,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::missing_errors_doc,
    clippy::fn_params_excessive_bools,
    clippy::module_name_repetitions,
    clippy::struct_excessive_bools
)]

use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;
use rp2350_core::types::Address;

use super::nvic::Nvic;
use super::registers::CortexM33Registers;

/// Standard ARMv8-M stack frame on exception entry (8 words / 32 bytes)
/// ARMv8-M 异常进入时的标准栈帧（8 字 / 32 字节）
pub const EXCEPTION_STACK_FRAME_SIZE: u32 = 32;

/// Exception frame with floating point context (26 words / 104 bytes)
/// 带浮点上下文的异常帧（26 字 / 104 字节）
pub const EXCEPTION_STACK_FRAME_EXTENDED_SIZE: u32 = 104;

/// Exception number for SVCall / SVCall 的异常号
pub const EXC_SVCALL: u16 = 11;
/// Exception number for PendSV / PendSV 的异常号
pub const EXC_PENDSV: u16 = 14;
/// Exception number for SysTick / SysTick 的异常号
pub const EXC_SYSTICK: u16 = 15;

/// Decode an EXC_RETURN value to determine return behavior
/// 解码 EXC_RETURN 值以确定返回行为
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExcReturnInfo {
    /// Return to Handler mode (true) or Thread mode (false)
    /// 返回到 Handler 模式（true）或 Thread 模式（false）
    pub to_handler: bool,
    /// Use PSP for stack (true) or MSP (false) after return
    /// 返回后使用 PSP（true）或 MSP（false）
    pub use_psp: bool,
    /// Return to non-secure state (true for secure-world return)
    /// 返回到非安全状态（安全世界返回时为 true）
    pub to_non_secure: bool,
    /// Stack frame type: standard (false) or extended with FP (true)
    /// 栈帧类型：标准（false）或带 FP 扩展（true）
    pub has_fp: bool,
}

/// Parse an EXC_RETURN value
/// 解析 EXC_RETURN 值
#[must_use]
pub fn decode_exc_return(lr: u32) -> Option<ExcReturnInfo> {
    // EXC_RETURN values have prefix 0xFFFX_XXXX
    // EXC_RETURN 值具有前缀 0xFFFX_XXXX
    if (lr & 0xF000_0000) != 0xF000_0000 {
        return None;
    }

    Some(ExcReturnInfo {
        to_handler: (lr & 0x8) == 0,
        use_psp: (lr & 0x4) != 0,
        to_non_secure: (lr & 0x1) != 0,
        has_fp: (lr & 0x10) != 0,
    })
}

/// Push a standard exception stack frame onto the stack
/// 将标准异常栈帧压入栈中
///
/// # Arguments
/// * `bus` - Address bus for memory access
/// * `sp` - Stack pointer (will be decremented by frame size)
/// * `regs` - Current CPU registers
///
/// # Returns
/// Updated stack pointer value
pub fn exception_entry_stacking(
    bus: &mut dyn AddressBus,
    sp: u32,
    regs: &CortexM33Registers,
    cycles: &mut u64,
) -> Result<u32, CoreError> {
    let frame_sp = sp - EXCEPTION_STACK_FRAME_SIZE;

    // Stack frame layout (ARMv8-M, from high to low address):
    // 栈帧布局（ARMv8-M，从高地址到低地址）：
    // [sp-4]  = xPSR
    // [sp-8]  = ReturnAddress (PC)
    // [sp-12] = LR (R14)
    // [sp-16] = R12
    // [sp-20] = R3
    // [sp-24] = R2
    // [sp-28] = R1
    // [sp-32] = R0

    bus.write_u32(frame_sp, regs.r[0], cycles)?; // R0
    bus.write_u32(frame_sp + 4, regs.r[1], cycles)?; // R1
    bus.write_u32(frame_sp + 8, regs.r[2], cycles)?; // R2
    bus.write_u32(frame_sp + 12, regs.r[3], cycles)?; // R3
    bus.write_u32(frame_sp + 16, regs.r[12], cycles)?; // R12
    bus.write_u32(frame_sp + 20, regs.lr, cycles)?; // LR (R14)
    bus.write_u32(frame_sp + 24, regs.pc, cycles)?; // ReturnAddress (PC)
    bus.write_u32(frame_sp + 28, regs.xpsr, cycles)?; // xPSR

    *cycles += 12; // Exception entry takes 12 cycles
    Ok(frame_sp)
}

/// Pop a standard exception stack frame from the stack to restore registers
/// 从栈中弹出标准异常栈帧以恢复寄存器
///
/// # Arguments
/// * `bus` - Address bus for memory access
/// * `sp` - Stack pointer (points to bottom of frame)
/// * `regs` - CPU registers to restore into
pub fn exception_return_unstacking(
    bus: &mut dyn AddressBus,
    sp: u32,
    regs: &mut CortexM33Registers,
    cycles: &mut u64,
) -> Result<(u32, u32), CoreError> {
    let r0 = bus.read_u32(sp, cycles)?;
    let r1 = bus.read_u32(sp + 4, cycles)?;
    let r2 = bus.read_u32(sp + 8, cycles)?;
    let r3 = bus.read_u32(sp + 12, cycles)?;
    let r12 = bus.read_u32(sp + 16, cycles)?;
    let lr = bus.read_u32(sp + 20, cycles)?;
    let pc = bus.read_u32(sp + 24, cycles)?;
    let xpsr = bus.read_u32(sp + 28, cycles)?;

    regs.r[0] = r0;
    regs.r[1] = r1;
    regs.r[2] = r2;
    regs.r[3] = r3;
    regs.r[12] = r12;
    regs.lr = lr;
    regs.pc = pc & !1u32; // Clear Thumb bit on PC restore
    regs.xpsr = xpsr & 0xFF0F_FFFF; // Mask reserved bits

    *cycles += 12; // Exception return takes 12 cycles
    Ok((sp + EXCEPTION_STACK_FRAME_SIZE, pc))
}

/// Fetch the exception vector from the vector table
/// 从向量表获取异常向量
pub fn fetch_exception_vector(
    bus: &mut dyn AddressBus,
    vtor: u32,
    exception_number: u32,
    cycles: &mut u64,
) -> Result<Address, CoreError> {
    let vector_addr = vtor + exception_number * 4;
    let vector = bus.read_u32(vector_addr, cycles)?;
    *cycles += 1;
    Ok(vector)
}

/// Enter an exception handler — full sequence
/// 进入异常处理程序 —— 完整序列
///
/// Returns `Ok(())` on success, with registers and SP updated.
/// 成功返回 `Ok(())`，寄存器和 SP 已更新。
pub fn enter_exception(
    bus: &mut dyn AddressBus,
    regs: &mut CortexM33Registers,
    nvic: &mut Nvic,
    exception_number: u32,
    vtor: u32,
    cycles: &mut u64,
) -> Result<(), CoreError> {
    // Save current SP
    let sp = if exception_number >= 16 {
        // External interrupt: always use MSP during handler
        // 外部中断：处理程序期间始终使用 MSP
        regs.msp
    } else if regs.control & 0x2 != 0 {
        regs.psp
    } else {
        regs.msp
    };

    // Stack the current context
    // 压栈当前上下文
    let new_sp = exception_entry_stacking(bus, sp, regs, cycles)?;

    // Update SP based on mode
    // 根据模式更新 SP
    if exception_number >= 16 || (regs.control & 0x2 == 0) {
        regs.msp = new_sp;
        regs.sp = new_sp;
    } else {
        regs.psp = new_sp;
        regs.sp = new_sp;
    }

    // Set CONTROL.SPSEL=0 (use MSP in handler) if external interrupt
    // 如果是外部中断，设置 CONTROL.SPSEL=0（处理程序中使用 MSP）
    if exception_number >= 16 {
        regs.control &= !0x2;
    }

    // Set up return value in LR (EXC_RETURN)
    // 在 LR 中设置返回值（EXC_RETURN）
    if exception_number >= 16 {
        // Return to Thread mode, use MSP or PSP depending on original CONTROL
        // 返回 Thread 模式，使用 MSP 或 PSP 取决于原始 CONTROL
        if regs.control & 0x2 != 0 {
            regs.lr = 0xFFFF_FFED; // Return to Thread, PSP
        } else {
            regs.lr = 0xFFFF_FFF9; // Return to Thread, MSP
        }
    } else {
        // System exception / 系统异常
        regs.lr = 0xFFFF_FFF1; // Return to Handler mode
    }

    // Fetch exception vector and set PC
    // 获取异常向量并设置 PC
    let vector = fetch_exception_vector(bus, vtor, exception_number, cycles)?;
    regs.pc = vector & !1u32; // Clear Thumb bit

    // Mark interrupt as active
    // 标记中断为活跃
    if exception_number >= 16 {
        nvic.set_active(exception_number as u16);
        nvic.clear_pending(exception_number as u16);
    }

    Ok(())
}

/// Exit an exception handler — full sequence
/// 退出异常处理程序 —— 完整序列
///
/// Returns `Ok(new_sp)` — the SP after unstacking.
/// 返回 `Ok(new_sp)` —— 出栈后的 SP。
pub fn exit_exception(
    bus: &mut dyn AddressBus,
    regs: &mut CortexM33Registers,
    nvic: &mut Nvic,
    exc_return_info: &ExcReturnInfo,
    active_exception: u32,
    cycles: &mut u64,
) -> Result<(), CoreError> {
    // Clear active status
    // 清除活跃状态
    if active_exception >= 16 {
        nvic.clear_active(active_exception as u16);
    }

    // Determine which SP to use for unstacking
    // 确定使用哪个 SP 进行出栈
    let sp = if exc_return_info.use_psp {
        regs.psp
    } else {
        regs.msp
    };

    let (new_sp, _pc) = exception_return_unstacking(bus, sp, regs, cycles)?;

    // Update the correct SP
    // 更新正确的 SP
    if exc_return_info.use_psp {
        regs.psp = new_sp;
    } else {
        regs.msp = new_sp;
    }
    regs.sp = new_sp;

    // Restore CONTROL bits if returning to Thread mode
    // 如果返回 Thread 模式，恢复 CONTROL 位
    if !exc_return_info.to_handler {
        regs.control = if exc_return_info.use_psp {
            regs.control | 0x2
        } else {
            regs.control & !0x2
        };
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_exc_return_to_thread_msp() {
        // EXC_RETURN = 0xFFFF_FFF9 → Return to Thread mode, MSP, non-secure
        let info = decode_exc_return(0xFFFF_FFF9).unwrap();
        assert!(!info.to_handler);
        assert!(!info.use_psp);
        assert!(info.to_non_secure);
    }

    #[test]
    fn decode_exc_return_to_thread_psp() {
        // EXC_RETURN = 0xFFFF_FFFD → Return to Thread mode, PSP
        let info = decode_exc_return(0xFFFF_FFFD).unwrap();
        assert!(!info.to_handler);
        assert!(info.use_psp);
    }

    #[test]
    fn decode_exc_return_to_handler() {
        // EXC_RETURN = 0xFFFF_FFF1 → Return to Handler mode
        let info = decode_exc_return(0xFFFF_FFF1).unwrap();
        assert!(info.to_handler);
        assert!(!info.use_psp);
    }

    #[test]
    fn decode_invalid_exc_return() {
        assert!(decode_exc_return(0x2000_1000).is_none());
        assert!(decode_exc_return(0x0000_0000).is_none());
    }

    #[test]
    fn exception_stack_frame_size_is_correct() {
        assert_eq!(EXCEPTION_STACK_FRAME_SIZE, 32);
    }
}
