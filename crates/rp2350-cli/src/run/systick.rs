//! SysTick pending forwarding from the peripheral counter to the core
//! 外设侧 SysTick 挂起向核心的转发
//!
//! The peripheral [`rp2350_periph::systick::SysTickCounter`] is the single
//! source of truth for `SYST_CSR`/`SYST_RVR`/`SYST_CVR`, the countdown, the
//! live clock-tree source and the one-shot expiry latch. A machine never
//! replays the SysTick registers into the core, so the core's own counter stays
//! disabled and cannot tick a second time. This module only moves the latch:
//! one `take_systick_pending()` per pump is forwarded to
//! `CortexM33Core::pend_systick()`, which pends system exception 15
//! (`SCB_ICSR.PENDSTSET`, vector `VTOR + 0x3C`).
//!
//! 外设 [`rp2350_periph::systick::SysTickCounter`] 是 `SYST_CSR`/`SYST_RVR`/
//! `SYST_CVR`、递减计数、实时时钟源与一次性到期锁存的唯一真相源。机器从不把
//! SysTick 寄存器回放到核，因此核内计数器保持关闭、不会二次计时。本模块只搬运
//! 锁存：每次泵送最多取走一次 `take_systick_pending()` 并转发给
//! `CortexM33Core::pend_systick()`，由它挂起系统异常 15
//! （`SCB_ICSR.PENDSTSET`，向量 `VTOR + 0x3C`）。

// SysTick 是 ARM 参考手册中的驼峰寄存器名，文档标记保持硬件写法以便检索。
// SysTick is the CamelCase name used by the ARM reference manuals; doc markup
// keeps the hardware spelling so it stays searchable.
#![allow(clippy::doc_markdown)]

use rp2350_cpu::CpuCore;
use rp2350_periph::BootPeripherals;

use super::{CoreHandle, IRQ_LOG_LIMIT, IrqLog};

/// Forward one peripheral SysTick expiry to the ARM core
/// 把一次外设侧 SysTick 到期转发给 ARM 核心
///
/// Called from [`super::deliver_irqs`] on every pump, so at most one forward
/// happens per step. The peripheral latch is one-shot, which is what makes a
/// single underflow enter exactly one exception; a second expiry only pends
/// again after the next counter wrap.
/// 由 [`super::deliver_irqs`] 在每次泵送时调用，因此每步最多转发一次。外设锁存
/// 是一次性的，这保证一次下溢只进入一次异常；下一次计数器回绕才会再次挂起。
///
/// Hazard3 has no ARMv8-M SysTick, so the shared PPB counter is never consumed
/// there (the latch would have no consumer).
/// Hazard3 没有 ARMv8-M SysTick，因此那里不消费共享 PPB 计数器（其锁存没有消费者）。
pub(super) fn forward_systick_pending(
    core: &mut CoreHandle,
    peripherals: &BootPeripherals,
    irq_log: &mut IrqLog,
) {
    let CoreHandle::Arm(core) = core else {
        return;
    };
    if !peripherals.take_systick_pending() {
        return;
    }
    let pc = core.pc();
    core.pend_systick();
    log_systick(irq_log, pc);
}

/// Record one forwarded SysTick expiry in the bounded interrupt log
/// 在有界中断日志中记录一次 SysTick 转发
///
/// The line uses the same `[irq]` prefix as external deliveries so a frontend
/// can parse one stream; it stays distinct from `signal <n>` because exception
/// 15 is a system exception, not external IRQ 15 (exception 31).
/// 该行沿用与外设投递相同的 `[irq]` 前缀，前端可解析同一条流；它与
/// `signal <n>` 区分开，因为异常 15 是系统异常，不是外部 IRQ 15（异常 31）。
pub(super) fn log_systick(irq_log: &mut IrqLog, pc: u32) {
    if irq_log.lines.len() == IRQ_LOG_LIMIT {
        irq_log.lines.remove(0);
    }
    irq_log.total = irq_log.total.saturating_add(1);
    irq_log
        .lines
        .push(format!("[irq] core0 systick at PC={pc:#010X}"));
}
