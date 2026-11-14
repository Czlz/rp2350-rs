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

use super::conditions::ITBlockState;
use super::nvic::Nvic;
use super::registers::CortexM33Registers;

/// ITSTATE bits inside xPSR (`[26:25,15:10]`) / xPSR 中的 ITSTATE 位
pub const XPSR_IT_MASK: u32 = 0x0600_FC00;

/// Encode an `ITSTATE` byte into the xPSR layout / 将 `ITSTATE` 字节编码进 xPSR
#[must_use]
pub fn it_state_to_xpsr_bits(state: u8) -> u32 {
    ((u32::from(state) & 0xC0) << 19) | ((u32::from(state) & 0x3F) << 10)
}

/// Decode an `ITSTATE` byte out of a stacked xPSR / 从异常栈 xPSR 解码 `ITSTATE`
#[must_use]
pub fn xpsr_bits_to_it_state(xpsr: u32) -> u8 {
    ((((xpsr >> 19) & 0xC0) | ((xpsr >> 10) & 0x3F)) & 0xFF) as u8
}

/// Standard ARMv8-M stack frame on exception entry (8 words / 32 bytes)
/// ARMv8-M 异常进入时的标准栈帧（8 字 / 32 字节）
pub const EXCEPTION_STACK_FRAME_SIZE: u32 = 32;

/// Exception frame with floating point context (26 words / 104 bytes)
/// 带浮点上下文的异常帧（26 字 / 104 字节）
pub const EXCEPTION_STACK_FRAME_EXTENDED_SIZE: u32 = 104;

/// CFSR.UFSR.NOCP: coprocessor access denied (bit 19 of CFSR)
/// CFSR.UFSR.NOCP：协处理器访问被拒绝（CFSR 位 19）
pub const CFSR_UFSR_NOCP: u32 = 1 << 19;

/// Exception number for UsageFault / 用法故障的异常号
pub const EXC_USAGE_FAULT: u16 = 6;
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

/// Apply `BXWritePC` / `LoadWritePC` semantics to a value written to PC
/// 对写入 PC 的值应用 `BXWritePC` / `LoadWritePC` 语义
///
/// ARMv8-M requires every PC write through `BX`, `BLX`, `POP`, `LDM` and `LDR`
/// to check for an EXC_RETURN value; only `ALUWritePC` (MOV/ADD to PC) is a
/// plain branch. A `POP {..., pc}` that restores `0xFFFF_FFF9` is therefore an
/// exception return, not a branch to `0xFFFF_FFF8` — masking bit 0 first is
/// exactly how such firmware used to run off into a bus fault.
/// ARMv8-M 要求通过 `BX`、`BLX`、`POP`、`LDM`、`LDR` 写 PC 时都检查
/// EXC_RETURN；只有 `ALUWritePC`（MOV/ADD 写 PC）是普通分支。因此
/// `POP {..., pc}` 恢复 `0xFFFF_FFF9` 是异常返回，而非跳转到 `0xFFFF_FFF8`；
/// 先清 bit 0 正是这类固件跑飞并触发总线错误的原因。
///
/// The EXC_RETURN value is carried in `regs.lr` so the core's private
/// `return_from_exception` step can decode it; exception
/// unstacking restores LR from the stack frame afterwards.
/// EXC_RETURN 值通过 `regs.lr` 传递，供异常返回流程解码；随后出栈会用栈帧
/// 中的 LR 覆盖它。
///
/// Returns `true` when the caller must raise `StepResult::ExceptionReturn`.
/// 返回 `true` 表示调用方须返回 `StepResult::ExceptionReturn`。
#[must_use]
pub fn write_pc(regs: &mut CortexM33Registers, value: u32) -> bool {
    if decode_exc_return(value).is_some() {
        regs.lr = value;
        true
    } else {
        regs.pc = value & !1u32;
        false
    }
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
    it: &ITBlockState,
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

    // EPSR.IT lives outside the APSR flags; stack it in the architectural
    // xPSR bit positions so a thread interrupted inside an IT block resumes
    // the remaining conditional instructions after the exception return.
    // EPSR.IT 不在 APSR 标志中；按架构 xPSR 位段压栈，使在 IT 块中间被打断的
    // 线程能在异常返回后继续执行剩余的条件指令。
    let stacked_xpsr = (regs.xpsr & !XPSR_IT_MASK) | it_state_to_xpsr_bits(it.arch_state());

    bus.write_u32(frame_sp, regs.r[0], cycles)?; // R0
    bus.write_u32(frame_sp + 4, regs.r[1], cycles)?; // R1
    bus.write_u32(frame_sp + 8, regs.r[2], cycles)?; // R2
    bus.write_u32(frame_sp + 12, regs.r[3], cycles)?; // R3
    bus.write_u32(frame_sp + 16, regs.r[12], cycles)?; // R12
    bus.write_u32(frame_sp + 20, regs.lr, cycles)?; // LR (R14)
    bus.write_u32(frame_sp + 24, regs.pc, cycles)?; // ReturnAddress (PC)
    bus.write_u32(frame_sp + 28, stacked_xpsr, cycles)?; // xPSR

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
///
/// Returns `(new_sp, return_pc, it_state)`. / 返回 `(新 SP, 返回 PC, ITSTATE)`。
pub fn exception_return_unstacking(
    bus: &mut dyn AddressBus,
    sp: u32,
    regs: &mut CortexM33Registers,
    cycles: &mut u64,
) -> Result<(u32, u32, u8), CoreError> {
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
    // Restore the architectural xPSR: force the T bit, drop the alignment
    // padding bit (9) and keep ITSTATE in the separate IT block state.
    // 恢复架构 xPSR：强制 T 位、去掉对齐填充位（9），ITSTATE 保留在独立的
    // IT 块状态中。
    regs.xpsr = (xpsr | 0x0100_0000) & !(1 << 9) & !XPSR_IT_MASK;
    let it_state = xpsr_bits_to_it_state(xpsr);

    *cycles += 12; // Exception return takes 12 cycles
    Ok((sp + EXCEPTION_STACK_FRAME_SIZE, pc, it_state))
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
    it: &mut ITBlockState,
    cycles: &mut u64,
) -> Result<(), CoreError> {
    // Save current SP
    // 保存当前 SP
    let thread_used_psp = regs.control & 0x2 != 0;
    let sp = if exception_number >= 16 {
        // External interrupt: always use MSP during handler
        // 外部中断：处理程序期间始终使用 MSP
        regs.msp
    } else if thread_used_psp {
        regs.psp
    } else {
        regs.msp
    };

    // Stack the current context
    // 压栈当前上下文
    let new_sp = exception_entry_stacking(bus, sp, regs, it, cycles)?;
    // The handler starts with no active IT block; the interrupted thread's
    // IT state was saved in the stacked xPSR.
    // 处理程序开始时没有活跃的 IT 块；被打断线程的 IT 状态已保存在栈中 xPSR。
    *it = ITBlockState::new();
    regs.xpsr = (regs.xpsr | 0x0100_0000) & !XPSR_IT_MASK;

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
        // Return to Thread mode, use MSP or PSP depending on the interrupted context
        // 返回 Thread 模式，按被打断的上下文选择 MSP 或 PSP
        if thread_used_psp {
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
        // NVIC 寄存器的位号是 IRQ 号（异常号减去 16）。
        // NVIC register bit indices are IRQ numbers (exception number minus 16).
        let irq = exception_number as u16 - 16;
        nvic.set_active(irq);
        nvic.clear_pending(irq);
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
    it: &mut ITBlockState,
    cycles: &mut u64,
) -> Result<(), CoreError> {
    // Clear active status
    // 清除活跃状态
    if active_exception >= 16 {
        nvic.clear_active(active_exception as u16 - 16);
    }

    // Determine which SP to use for unstacking
    // 确定使用哪个 SP 进行出栈
    let sp = if exc_return_info.use_psp {
        regs.psp
    } else {
        regs.msp
    };

    let (new_sp, _pc, it_state) = exception_return_unstacking(bus, sp, regs, cycles)?;
    *it = ITBlockState::from_arch_state(it_state);

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

    #[test]
    fn write_pc_treats_exc_return_as_an_exception_return() {
        let mut regs = CortexM33Registers {
            pc: 0x1000_0100,
            ..Default::default()
        };
        // POP {..., pc} 恢复 0xFFFF_FFF9：不能把 bit 0 清成 0xFFFF_FFF8
        // 后当成普通跳转目标，否则 CPU 会跳出 Flash 触发总线错误。
        // A POP restoring 0xFFFF_FFF9 must not mask bit 0 into a branch to
        // 0xFFFF_FFF8; that would leave flash and raise a bus error.
        assert!(write_pc(&mut regs, 0xFFFF_FFF9));
        assert_eq!(
            regs.pc, 0x1000_0100,
            "PC must stay until unstacking / 出栈前 PC 不变"
        );
        assert_eq!(regs.lr, 0xFFFF_FFF9);
    }

    #[test]
    fn it_state_round_trips_through_stacked_xpsr() {
        // ITSTATE[7:6] lives in xPSR[26:25] and ITSTATE[5:0] in xPSR[15:10].
        // ITSTATE[7:6] 位于 xPSR[26:25]，ITSTATE[5:0] 位于 xPSR[15:10]。
        for state in [0x00, 0x08, 0x0B, 0x16, 0x0C, 0x18, 0x1C, 0xFF] {
            let encoded = it_state_to_xpsr_bits(state);
            assert_eq!(xpsr_bits_to_it_state(encoded), state);
            assert_eq!(encoded & !XPSR_IT_MASK, 0, "only IT bits may be set");
        }
    }

    #[test]
    fn exception_stacking_preserves_it_state_in_xpsr() {
        use rp2350_core::bus::AddressBus;
        use rp2350_core::memory::{RamRegion, SimpleBus};

        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(RamRegion::new("stack", 0x2000_0000, 0x100)))
            .expect("region");
        let regs = CortexM33Registers {
            pc: 0x1000_0100,
            xpsr: 0x0100_0000,
            ..Default::default()
        };
        let it = ITBlockState::from_arch_state(0x0B);
        let mut cycles = 0u64;
        let frame_sp = exception_entry_stacking(&mut bus, 0x2000_0080, &regs, &it, &mut cycles)
            .expect("stacking");
        let stacked = bus.read_u32(frame_sp + 28, &mut cycles).expect("xpsr");
        assert_eq!(xpsr_bits_to_it_state(stacked), 0x0B);
        assert_eq!(stacked & (1 << 24), 1 << 24, "T bit remains set");
    }

    #[test]
    fn write_pc_branches_for_ordinary_addresses() {
        let mut regs = CortexM33Registers {
            lr: 0x1234_5678,
            ..Default::default()
        };
        assert!(!write_pc(&mut regs, 0x1000_0123));
        assert_eq!(
            regs.pc, 0x1000_0122,
            "Thumb bit is cleared / Thumb 位应清除"
        );
        assert_eq!(regs.lr, 0x1234_5678, "LR is untouched / LR 不应被修改");
    }
}
