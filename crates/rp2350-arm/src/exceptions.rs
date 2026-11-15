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
use super::fpu::{FpuRegisters, fpccr};
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

/// Architectural ARMv8-M extended frame with FP context (26 words / 104 bytes)
/// ARMv8-M 带浮点上下文的架构扩展帧（26 字 / 104 字节）
///
/// The 8-word state frame plus the 17-word FP context (S0-S15 and FPSCR) plus
/// one padding word above the context, which keeps the total a multiple of 8
/// bytes so that `FPCAR` (pointing at S0) stays 8-byte aligned. This equals
/// [`EXCEPTION_STACK_FRAME_WITH_FP_SIZE`].
/// 即 8 字状态帧 + 17 字浮点上下文（S0-S15 与 FPSCR）+ 上下文上方 1 字填充；
/// 填充使总长为 8 的倍数，从而让指向 S0 的 `FPCAR` 保持 8 字节对齐。该值与
/// [`EXCEPTION_STACK_FRAME_WITH_FP_SIZE`] 相同。
pub const EXCEPTION_STACK_FRAME_EXTENDED_SIZE: u32 = 104;

/// Floating-point context written by the lazy/eager stacking path (17 words)
/// 惰性/立即压栈路径写入的浮点上下文（17 字）
///
/// S0-S15 occupy the first 16 words and FPSCR the last one.
/// 前 16 字为 S0-S15，最后一字为 FPSCR。
pub const FP_CONTEXT_SIZE: u32 = 17 * 4;

/// Architectural padding word above the FP context / 浮点上下文上方的架构填充字
///
/// The padding keeps the extended frame a multiple of 8 bytes so `FPCAR`, which
/// points at S0, stays 8-byte aligned.
/// 该填充字使扩展帧长度为 8 的倍数，从而让指向 S0 的 `FPCAR` 保持 8 字节对齐。
pub const FP_CONTEXT_PADDING_SIZE: u32 = 4;

/// Space reserved below the interrupted SP for the floating-point context
/// 在被打断的 SP 下方为浮点上下文预留的空间
pub const FP_CONTEXT_RESERVED_SIZE: u32 = FP_CONTEXT_SIZE + FP_CONTEXT_PADDING_SIZE;

/// Offset of `FPSCR` inside the FP context area / 浮点上下文区内 `FPSCR` 的偏移
pub const FP_CONTEXT_FPSCR_OFFSET: u32 = 16 * 4;

/// Full size of the modeled extended frame (state frame + FP reservation)
/// 本模型扩展帧的总大小（状态帧 + 浮点预留区）
pub const EXCEPTION_STACK_FRAME_WITH_FP_SIZE: u32 =
    EXCEPTION_STACK_FRAME_SIZE + FP_CONTEXT_RESERVED_SIZE;

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
    ///
    /// Derived from `EXC_RETURN` bit 4, which the architecture defines as 1 for
    /// the standard frame and 0 for the extended (floating-point) frame.
    /// 由 `EXC_RETURN` bit 4 推出：架构规定该位为 1 表示标准帧，为 0 表示扩展
    /// （带浮点）帧。
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
        // Bit 4 is 1 for the standard frame and 0 for the extended FP frame.
        // bit 4 为 1 表示标准帧，为 0 表示带浮点的扩展帧。
        has_fp: (lr & 0x10) == 0,
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
/// * `stack_aligner` - A 4-byte aligner word sits above the frame; recorded in
///   stacked xPSR bit 9 so the return path restores the unaligned SP
/// * `stack_aligner` - 帧上方存在 4 字节对齐字；记录在压栈 xPSR 的 bit 9，
///   使返回路径能恢复未对齐的 SP
///
/// # Returns
/// Updated stack pointer value
pub fn exception_entry_stacking(
    bus: &mut dyn AddressBus,
    sp: u32,
    regs: &CortexM33Registers,
    it: &ITBlockState,
    cycles: &mut u64,
    stack_aligner: bool,
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
    //
    // xPSR bit 9 records the 4-byte stack aligner inserted by exception entry.
    // xPSR bit 9 记录异常进入时插入的 4 字节栈对齐字。
    let mut stacked_xpsr = (regs.xpsr & !XPSR_IT_MASK) | it_state_to_xpsr_bits(it.arch_state());
    if stack_aligner {
        stacked_xpsr |= 1 << 9;
    } else {
        stacked_xpsr &= !(1 << 9);
    }

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

/// Write S0-S15 and FPSCR into a reserved FP context area
/// 将 S0-S15 与 FPSCR 写入预留的浮点上下文区
///
/// `fpcar` is the address of the S0 slot; S15 follows at `fpcar + 0x3C` and
/// FPSCR at `fpcar + 0x40`. The padding word at `fpcar + 0x44` is left
/// untouched.
/// `fpcar` 为 S0 槽地址；S15 位于 `fpcar + 0x3C`，FPSCR 位于 `fpcar + 0x40`。
/// `fpcar + 0x44` 处的填充字不被写入。
///
/// # Errors
/// Returns a bus error if the stack region is not writable.
/// 若栈区域不可写则返回总线错误。
pub fn store_fp_context(
    bus: &mut dyn AddressBus,
    fpcar: u32,
    fpu: &FpuRegisters,
    cycles: &mut u64,
) -> Result<(), CoreError> {
    for index in 0..16u8 {
        bus.write_u32(fpcar + u32::from(index) * 4, fpu.read_s(index), cycles)?;
    }
    bus.write_u32(fpcar + FP_CONTEXT_FPSCR_OFFSET, fpu.fpscr, cycles)?;
    *cycles += 17; // Saving 17 words / 保存 17 字
    Ok(())
}

/// Restore S0-S15 and FPSCR from a reserved FP context area
/// 从预留的浮点上下文区恢复 S0-S15 与 FPSCR
///
/// Inverse of [`store_fp_context`]. / [`store_fp_context`] 的逆操作。
///
/// # Errors
/// Returns a bus error if the stack region is not readable.
/// 若栈区域不可读则返回总线错误。
pub fn load_fp_context(
    bus: &mut dyn AddressBus,
    fpcar: u32,
    fpu: &mut FpuRegisters,
    cycles: &mut u64,
) -> Result<(), CoreError> {
    for index in 0..16u8 {
        let value = bus.read_u32(fpcar + u32::from(index) * 4, cycles)?;
        fpu.write_s(index, value);
    }
    fpu.fpscr = bus.read_u32(fpcar + FP_CONTEXT_FPSCR_OFFSET, cycles)?;
    *cycles += 17; // Restoring 17 words / 恢复 17 字
    Ok(())
}

/// Commit a deferred (lazy) FP context to `FPCAR` and clear `LSPACT`
/// 将延迟（惰性）的浮点上下文提交到 `FPCAR` 并清除 `LSPACT`
///
/// Called before a VFP instruction executes while `FPCCR.LSPACT` is set: the
/// interrupted context's S0-S15 and FPSCR are written to the area reserved at
/// exception entry, then lazy state preservation is deactivated so the save
/// happens exactly once per frame.
/// 当 `FPCCR.LSPACT` 置位时，在执行 VFP 指令前调用：把被中断上下文的
/// S0-S15 与 FPSCR 写入异常进入时预留的区域，然后关闭惰性状态保存，保证每个
/// 栈帧只提交一次。
///
/// Returns `true` when a context was committed.
/// 提交了上下文时返回 `true`。
///
/// # Errors
/// Returns a bus error if the reserved stack area is not writable.
/// 若预留的栈区域不可写则返回总线错误。
pub fn commit_lazy_fp_context(
    bus: &mut dyn AddressBus,
    regs: &mut CortexM33Registers,
    cycles: &mut u64,
) -> Result<bool, CoreError> {
    if !regs.fpu.lspact() {
        return Ok(false);
    }
    let fpcar = regs.fpu.fpcar;
    store_fp_context(bus, fpcar, &regs.fpu, cycles)?;
    regs.fpu.set_lspact(false);
    Ok(true)
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
    let aligner = if xpsr & (1 << 9) != 0 { 4 } else { 0 };
    regs.xpsr = (xpsr | 0x0100_0000) & !(1 << 9) & !XPSR_IT_MASK;
    let it_state = xpsr_bits_to_it_state(xpsr);

    *cycles += 12; // Exception return takes 12 cycles
    Ok((sp + EXCEPTION_STACK_FRAME_SIZE + aligner, pc, it_state))
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

    // ---- Floating-point context reservation ----
    // 浮点上下文预留：
    // 若浮点单元已被使用（模型以 FPEXC.EN 代替 CONTROL.FPCA），异常帧为扩展帧
    // （EXC_RETURN bit4 = 0）。FPCCR.LSPEN=1 时只预留 17 字上下文区并置
    // LSPACT，S0-S15/FPSCR 推迟到处理程序的第一条 VFP 指令；LSPEN=0 时立即
    // 写入。FPU 未使用时保持标准 8 字帧且不预留空间。
    // If the FPU has been used (the model uses FPEXC.EN instead of
    // CONTROL.FPCA) the frame is extended (EXC_RETURN bit 4 = 0). With
    // FPCCR.LSPEN set, only the 17-word context area is reserved and LSPACT is
    // raised; S0-S15/FPSCR are deferred to the handler's first VFP instruction.
    // With LSPEN clear the context is stored immediately. An unused FPU keeps
    // the standard 8-word frame with no reservation.
    //
    // A pending lazy frame (nested entry) is committed first so the new frame
    // cannot overwrite its FPCAR. / 若已有待保存的惰性帧（嵌套进入），先提交
    // 该帧，避免新帧覆盖它的 FPCAR。
    if regs.fp_context_active() && regs.fpu.lspact() {
        commit_lazy_fp_context(bus, regs, cycles)?;
    }

    let extended_frame = regs.fp_context_active();
    // 扩展帧要求 FPCAR 8 字节对齐：SP 未对齐时先向下对齐，并在压栈 xPSR
    // 的 bit 9 记录 4 字节对齐字，返回时再补回。
    // An extended frame requires an 8-byte aligned FPCAR: an unaligned SP is
    // rounded down and the 4-byte aligner is recorded in stacked xPSR bit 9 so
    // the return path can restore the exact SP.
    let (frame_top, stack_aligner) = if extended_frame {
        let aligned = sp & !0x7;
        (aligned, aligned != sp)
    } else {
        (sp, false)
    };
    let frame_base = if extended_frame {
        // FPCAR points at S0 and must stay 8-byte aligned; the padding word
        // sits right above the 17-word context.
        // FPCAR 指向 S0 且必须保持 8 字节对齐；填充字紧邻 17 字上下文上方。
        regs.fpu.write_fpcar(frame_top - FP_CONTEXT_RESERVED_SIZE);
        let fpcar = regs.fpu.fpcar;
        if regs.fpu.fpccr & fpccr::LSPEN != 0 {
            regs.fpu.set_lspact(true);
        } else {
            store_fp_context(bus, fpcar, &regs.fpu, cycles)?;
            regs.fpu.set_lspact(false);
        }
        // `exception_entry_stacking` subtracts the 8-word state frame from the
        // value passed in, so it receives the top of the state frame, which is
        // exactly where the FP context starts.
        // `exception_entry_stacking` 会从传入值减去 8 字状态帧，因此这里传入
        // 状态帧顶端——也就是浮点上下文的起始地址。
        fpcar
    } else {
        frame_top
    };

    // Stack the current context
    // 压栈当前上下文
    let new_sp = exception_entry_stacking(bus, frame_base, regs, it, cycles, stack_aligner)?;
    // The handler starts with no active IT block; the interrupted thread's
    // IT state was saved in the stacked xPSR.
    // 处理程序开始时没有活跃的 IT 块；被打断线程的 IT 状态已保存在栈中 xPSR。
    *it = ITBlockState::new();
    // IPSR 记录当前异常号（外部中断为 16 + n），固件通过 MRS IPSR 判断中断源。
    // IPSR holds the active exception number (16 + n for external interrupts);
    // firmware reads it with MRS IPSR to identify the interrupt source.
    regs.xpsr = (regs.xpsr & !0x1FF) | (exception_number & 0x1FF);
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
            // 0xFFFF_FFFD = standard frame; bit 4 is cleared below when the
            // entry allocated an extended frame (0xFFFF_FFED).
            // 0xFFFF_FFFD 为标准帧；若本次进入分配了扩展帧，下方会清除 bit 4
            // （变为 0xFFFF_FFED）。
            regs.lr = 0xFFFF_FFFD; // Return to Thread, PSP
        } else {
            regs.lr = 0xFFFF_FFF9; // Return to Thread, MSP
        }
    } else {
        // System exception / 系统异常
        regs.lr = 0xFFFF_FFF1; // Return to Handler mode
    }

    // EXC_RETURN bit 4 records the frame type: 1 = standard, 0 = extended.
    // EXC_RETURN bit 4 记录帧类型：1 = 标准帧，0 = 扩展帧。
    if extended_frame {
        regs.lr &= !0x10;
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

    // ---- Floating-point context recovery ----
    // 浮点上下文恢复：
    // EXC_RETURN bit4 = 0 表示扩展帧。LSPACT 仍为 1 说明处理程序从未执行 VFP
    // 指令，被中断的浮点寄存器仍是活跃值，只需回收预留区；否则从该帧的浮点
    // 上下文区（S0 位于帧底 + 0x20）恢复 S0-S15 与 FPSCR。
    // EXC_RETURN bit 4 = 0 selects the extended frame. LSPACT still set means
    // the handler never executed a VFP instruction, so the interrupted FP
    // registers are still live and only the reservation is reclaimed; otherwise
    // S0-S15 and FPSCR are restored from the FP context area of this frame
    // (S0 sits at frame base + 0x20).
    let extended_frame = exc_return_info.has_fp;
    if extended_frame {
        if regs.fpu.lspact() {
            // Nothing was written to the reserved area; discard it.
            // 预留区未写入任何内容，直接丢弃。
            regs.fpu.set_lspact(false);
        } else {
            let fpcar = sp + EXCEPTION_STACK_FRAME_SIZE;
            load_fp_context(bus, fpcar, &mut regs.fpu, cycles)?;
        }
    }

    let (state_sp, _pc, it_state) = exception_return_unstacking(bus, sp, regs, cycles)?;
    // An extended frame also reclaims the 17-word FP context and its padding
    // word above the state frame (`state_sp` already includes any 4-byte
    // aligner recorded in stacked xPSR bit 9).
    // 扩展帧还需回收状态帧上方的 17 字浮点上下文与其填充字
    // （`state_sp` 已包含压栈 xPSR bit 9 记录的 4 字节对齐字）。
    let new_sp = if extended_frame {
        state_sp + FP_CONTEXT_RESERVED_SIZE
    } else {
        state_sp
    };
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
        let frame_sp =
            exception_entry_stacking(&mut bus, 0x2000_0080, &regs, &it, &mut cycles, false)
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
