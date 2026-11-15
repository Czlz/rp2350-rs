//! PendSV / SysTick system-exception delivery tests
//! PendSV / SysTick 系统异常投递测试
//!
//! ARMv8-M numbers PendSV as exception 14 and SysTick as 15; only external
//! interrupts use `16 + n`. These tests pin the split between the NVIC pending
//! bits (external IRQs) and the `SCB_ICSR` pending latches, the `VTOR + 0x38` /
//! `VTOR + 0x3C` vectors, the `SHPR3` priority and `PRIMASK`/`FAULTMASK`/
//! `BASEPRI` gates, and the `EXC_RETURN` path.
//! ARMv8-M 的 PendSV 为异常 14、SysTick 为 15；只有外部中断用 `16 + n`。本模块
//! 锁定 NVIC 挂起位（外部中断）与 `SCB_ICSR` 挂起锁存的分工、`VTOR + 0x38` /
//! `VTOR + 0x3C` 向量、`SHPR3` 优先级与 `PRIMASK`/`FAULTMASK`/`BASEPRI` 门控，
//! 以及 `EXC_RETURN` 返回路径。

use super::*;

use crate::scb::icsr;
use rp2350_core::memory::{RamRegion, SimpleBus};

/// Vector table base programmed into `VTOR` (128-byte aligned)
/// 写入 `VTOR` 的向量表基址（128 字节对齐）
const VECTORS: u32 = 0x0000_0400;
/// Thread code base / 线程代码基址
const THREAD: u32 = 0x0000_0100;
/// SysTick handler (Thumb bit set) / SysTick 处理程序（含 Thumb 位）
const SYSTICK_HANDLER: u32 = 0x0000_0181;
/// PendSV handler (Thumb bit set) / PendSV 处理程序（含 Thumb 位）
const PENDSV_HANDLER: u32 = 0x0000_0201;
/// IRQ5 handler (Thumb bit set) / IRQ5 处理程序（含 Thumb 位）
const IRQ5_HANDLER: u32 = 0x0000_0281;
/// IRQ15 handler (Thumb bit set) / IRQ15 处理程序（含 Thumb 位）
const IRQ15_HANDLER: u32 = 0x0000_0301;
/// IRQ0 handler (Thumb bit set) / IRQ0 处理程序（含 Thumb 位）
const IRQ0_HANDLER: u32 = 0x0000_0381;
/// Initial MSP / 初始 MSP
const STACK_TOP: u32 = 0x2000_1000;

/// `SCB_VTOR` / `SCB_VTOR`
const SCB_VTOR: u32 = 0xE000_ED08;
/// `SCB_ICSR` / `SCB_ICSR`
const SCB_ICSR: u32 = 0xE000_ED04;
/// `SCB_SHPR2` / `SCB_SHPR2`
const SCB_SHPR2: u32 = 0xE000_ED1C;
/// `SCB_SHPR3` / `SCB_SHPR3`
const SCB_SHPR3: u32 = 0xE000_ED20;
/// `SYST_CSR` / `SYST_CSR`
const SYST_CSR: u32 = 0xE000_E010;
/// `SYST_RVR` / `SYST_RVR`
const SYST_RVR: u32 = 0xE000_E014;
/// `SYST_CVR` / `SYST_CVR`
const SYST_CVR: u32 = 0xE000_E018;
/// NVIC `ISER0` / NVIC `ISER0`
const NVIC_ISER0: u32 = 0xE000_E100;
/// NVIC `ISPR0` / NVIC `ISPR0`
const NVIC_ISPR0: u32 = 0xE000_E200;
/// NVIC `ICPR0` / NVIC `ICPR0`
const NVIC_ICPR0: u32 = 0xE000_E280;
/// NVIC `IPR` base (`IPR<irq>` byte address is this plus the IRQ number)
/// NVIC `IPR` 基址（`IPR<irq>` 字节地址为此值加 IRQ 号）
const NVIC_IPR: u32 = 0xE000_E400;

/// `NOP` (`0xBF00`) / `NOP`
const NOP: u16 = 0xBF00;
/// `MOVS r0, #7` (`0x2007`) / `MOVS r0, #7`
const MOVS_R0_7: u16 = 0x2007;
/// `BX LR` (`0x4770`) / `BX LR`
const BX_LR: u16 = 0x4770;
/// `B .` (`0xE7FE`) / `B .`
const SPIN: u16 = 0xE7FE;

/// Write a halfword program / 写入半字程序
fn write_code(bus: &mut SimpleBus, base: u32, code: &[u16]) {
    let mut cycles = 0u64;
    for (index, halfword) in code.iter().enumerate() {
        bus.write_u16(base + 2 * index as u32, *halfword, &mut cycles)
            .unwrap();
    }
}

/// Build a core whose `VTOR` points at a vector table with every handler
/// 构建 `VTOR` 指向含全部处理程序向量表的核心
fn fixture() -> (CortexM33Core, SimpleBus) {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("code", 0x0000_0000, 0x800)))
        .unwrap();
    bus.register_region(Box::new(RamRegion::new("stack", 0x2000_0000, 0x1000)))
        .unwrap();
    let mut cycles = 0u64;
    // 启动向量位于地址 0；异常向量位于 VTOR = 0x400。
    // Boot vectors live at address 0; exception vectors at VTOR = 0x400.
    bus.write_u32(0x0000_0000, STACK_TOP, &mut cycles).unwrap();
    bus.write_u32(0x0000_0004, THREAD + 1, &mut cycles).unwrap();
    bus.write_u32(VECTORS + 14 * 4, PENDSV_HANDLER, &mut cycles)
        .unwrap();
    bus.write_u32(VECTORS + 15 * 4, SYSTICK_HANDLER, &mut cycles)
        .unwrap();
    bus.write_u32(VECTORS + 16 * 4, IRQ0_HANDLER, &mut cycles)
        .unwrap();
    bus.write_u32(VECTORS + 21 * 4, IRQ5_HANDLER, &mut cycles)
        .unwrap();
    bus.write_u32(VECTORS + 31 * 4, IRQ15_HANDLER, &mut cycles)
        .unwrap();

    write_code(&mut bus, THREAD, &[NOP, NOP, MOVS_R0_7, SPIN]);
    for handler in [
        SYSTICK_HANDLER,
        PENDSV_HANDLER,
        IRQ5_HANDLER,
        IRQ15_HANDLER,
        IRQ0_HANDLER,
    ] {
        write_code(&mut bus, handler & !1, &[BX_LR]);
    }

    let mut core = CortexM33Core::new(CpuId::Core0);
    core.boot(&mut bus).unwrap();
    core.system_write(SCB_VTOR, VECTORS);
    (core, bus)
}

/// Enable and software-pend external `irq` / 使能并软件挂起外部 `irq`
fn pend_external(core: &mut CortexM33Core, irq: u16) {
    core.system_write(NVIC_ISER0, 1 << irq);
    core.system_write(NVIC_ISPR0, 1 << irq);
}

/// Read `ICSR` through the core's read view / 通过核心读视图读取 `ICSR`
fn icsr_value(core: &CortexM33Core) -> u32 {
    core.read_icsr()
}

/// Current exception number from `IPSR` / 从 `IPSR` 取当前异常号
fn ipsr(core: &CortexM33Core) -> u32 {
    core.xpsr() & 0x1FF
}

#[test]
fn systick_expiry_enters_exception_15_at_vtor_plus_0x3c() {
    let (mut core, mut bus) = fixture();
    // ENABLE | TICKINT | CLKSOURCE，计数器在下一个周期归零。
    // ENABLE | TICKINT | CLKSOURCE; the counter wraps on the next cycle.
    core.system_write(SYST_RVR, 1);
    core.system_write(SYST_CVR, 1);
    core.system_write(SYST_CSR, 0x7);

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    // 异常 15 的向量位于 VTOR + 15*4 = 0x43C → 处理程序 0x180。
    // Exception 15 vectors at VTOR + 15*4 = 0x43C → handler 0x180.
    assert_eq!(core.pc(), SYSTICK_HANDLER & !1);
    assert_eq!(ipsr(&core), 15);
    assert_eq!(core.read_register(14), 0xFFFF_FFF9);
    assert_eq!(core.active_exceptions, vec![15]);
    // 被中断指令之后的地址压在帧内（xPSR 上方第 8 字之前的 PC 槽）。
    // The address after the interrupted instruction is stacked.
    let mut cycles = 0u64;
    assert_eq!(
        bus.read_u32(STACK_TOP - 32 + 24, &mut cycles).unwrap(),
        THREAD + 2
    );
    // 取走异常后 PENDSTSET 必须清除。
    // Taking the exception clears PENDSTSET.
    assert_eq!(icsr_value(&core) & icsr::PENDSTSET, 0);
    assert_eq!(
        core.system_exception_state().summary(),
        "systick=active pendsv=idle"
    );
}

#[test]
fn pendsv_enters_exception_14_at_vtor_plus_0x38() {
    let (mut core, mut bus) = fixture();
    core.system_write(SCB_ICSR, icsr::PENDSVSET);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);

    assert_eq!(core.pc(), PENDSV_HANDLER & !1);
    assert_eq!(ipsr(&core), 14);
    assert_eq!(core.read_register(14), 0xFFFF_FFF9);
    assert_eq!(core.active_exceptions, vec![14]);
    let mut cycles = 0u64;
    assert_eq!(
        bus.read_u32(STACK_TOP - 32 + 24, &mut cycles).unwrap(),
        THREAD + 2
    );
    assert_eq!(icsr_value(&core) & icsr::PENDSVSET, 0);
    assert_eq!(
        core.system_exception_state().summary(),
        "systick=idle pendsv=active"
    );
}

#[test]
fn icsr_pendstset_and_pendstclr_round_trip() {
    let (mut core, _bus) = fixture();
    assert_eq!(icsr_value(&core) & icsr::PENDSTSET, 0);

    core.system_write(SCB_ICSR, icsr::PENDSTSET);
    assert_ne!(icsr_value(&core) & icsr::PENDSTSET, 0);
    assert!(core.system_exception_state().systick_pending);
    // `system_read` 必须把同一次访问路由到 ICSR 视图。
    // `system_read` must route the same access to the ICSR view.
    assert_eq!(core.system_read(SCB_ICSR), Some(icsr_value(&core)));
    // PENDSTSET 同时锁存 SYST_CSR.COUNTFLAG（ARMv8-M B3.2.6）。
    // PENDSTSET also latches SYST_CSR.COUNTFLAG (ARMv8-M B3.2.6).
    assert_ne!(
        core.system_read(SYST_CSR).unwrap() & (1 << 16),
        0,
        "COUNTFLAG"
    );

    core.system_write(SCB_ICSR, icsr::PENDSTCLR);
    assert_eq!(icsr_value(&core) & icsr::PENDSTSET, 0);
    assert!(!core.system_exception_state().systick_pending);
    assert_ne!(
        core.system_read(SYST_CSR).unwrap() & (1 << 16),
        0,
        "clearing pending keeps COUNTFLAG / 清挂起不清 COUNTFLAG"
    );
}

#[test]
fn icsr_pendsvset_and_pendsvclr_round_trip() {
    let (mut core, _bus) = fixture();
    assert_eq!(icsr_value(&core) & icsr::PENDSVSET, 0);

    core.system_write(SCB_ICSR, icsr::PENDSVSET);
    assert_ne!(icsr_value(&core) & icsr::PENDSVSET, 0);
    assert!(core.system_exception_state().pendsv_pending);

    core.system_write(SCB_ICSR, icsr::PENDSVCLR);
    assert_eq!(icsr_value(&core) & icsr::PENDSVSET, 0);
    assert!(!core.system_exception_state().pendsv_pending);
    // PendSV 有独立挂起位，绝不触碰 SysTick 的位。
    // PendSV has its own latch and never touches SysTick's.
    assert_eq!(icsr_value(&core) & icsr::PENDSTSET, 0);
}

#[test]
fn icsr_vectpending_reports_the_exception_number() {
    let (mut core, _bus) = fixture();
    assert_eq!(
        icsr_value(&core) & (0x1FF << icsr::VECTPENDING_SHIFT),
        0,
        "nothing pending"
    );

    pend_external(&mut core, 5);
    assert_eq!(
        (icsr_value(&core) >> icsr::VECTPENDING_SHIFT) & 0x1FF,
        21,
        "IRQ5 is exception 21"
    );
    assert_ne!(icsr_value(&core) & icsr::ISRPENDING, 0);

    // 同优先级时 PendSV（14）优先于外部异常 21。
    // At equal priority PendSV (14) wins over external exception 21.
    core.pend_pendsv();
    assert_eq!((icsr_value(&core) >> icsr::VECTPENDING_SHIFT) & 0x1FF, 14);
    core.system_write(SCB_ICSR, icsr::PENDSVCLR);
    core.pend_systick();
    assert_eq!(
        (icsr_value(&core) >> icsr::VECTPENDING_SHIFT) & 0x1FF,
        15,
        "SysTick is exception 15"
    );

    core.system_write(NVIC_ICPR0, 1 << 5);
    core.system_write(SCB_ICSR, icsr::PENDSTCLR);
    assert_eq!(icsr_value(&core) & (0x1FF << icsr::VECTPENDING_SHIFT), 0);
    // RETTOBASE 在无活跃异常时置位。 / RETTOBASE is set with no active exception.
    assert_ne!(icsr_value(&core) & icsr::RETTOBASE, 0);
}

#[test]
fn shpr3_systick_priority_beats_a_lower_priority_external_irq() {
    let (mut core, mut bus) = fixture();
    pend_external(&mut core, 5);
    // IRQ5 优先级 2，SysTick 优先级 1 → SysTick 先取。
    // IRQ5 priority 2, SysTick priority 1 → SysTick is taken first.
    core.system_write(NVIC_IPR + 5, 0x40);
    core.system_write(SCB_SHPR3, 0x2000_0000);
    core.pend_systick();

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), SYSTICK_HANDLER & !1);
    assert_eq!(ipsr(&core), 15);
    assert_eq!(core.system_exception_state().systick_priority, 1);
}

#[test]
fn shpr3_systick_priority_loses_to_a_higher_priority_external_irq() {
    let (mut core, mut bus) = fixture();
    pend_external(&mut core, 5);
    // IRQ5 优先级 0，SysTick 优先级 3 → 外部中断先取。
    // IRQ5 priority 0, SysTick priority 3 → the external IRQ wins.
    core.system_write(SCB_SHPR3, 0x6000_0000);
    core.pend_systick();

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), IRQ5_HANDLER & !1);
    assert_eq!(ipsr(&core), 21);
    // SysTick 仍挂起，等待返回后投递。 / SysTick stays pending.
    assert!(core.system_exception_state().systick_pending);
}

#[test]
fn pendsv_and_systick_tie_break_on_the_exception_number() {
    let (mut core, mut bus) = fixture();
    core.pend_systick();
    core.pend_pendsv();

    // 同优先级 0：PendSV（14）先于 SysTick（15）。
    // Both priority 0: PendSV (14) before SysTick (15).
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), PENDSV_HANDLER & !1);
    assert_eq!(ipsr(&core), 14);

    // 处理程序执行 BX LR：返回同一拍内投递等待中的 SysTick。
    // The handler executes BX LR; the pending SysTick is tail-chained in the
    // same step.
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), SYSTICK_HANDLER & !1);
    assert_eq!(ipsr(&core), 15);
    assert_eq!(core.active_exceptions, vec![15]);
}

#[test]
fn basepri_masks_systick_and_pendsv() {
    let (mut core, mut bus) = fixture();
    // 两个系统异常优先级 1。 / Both system exceptions are at priority 1.
    core.system_write(SCB_SHPR3, 0x2020_0000);
    core.pend_systick();
    core.pend_pendsv();

    // BASEPRI=1 屏蔽优先级 >= 1：本步只执行线程 NOP。
    // BASEPRI=1 masks priorities >= 1: only the thread NOP runs.
    core.registers.basepri = 0x20;
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), THREAD + 2);
    assert_eq!(core.active_exceptions, Vec::<u16>::new());
    assert_eq!(
        core.system_exception_state().summary(),
        "systick=pending pendsv=pending"
    );

    // BASEPRI=2 允许优先级 1：PendSV 先取，SysTick 保持挂起。
    // BASEPRI=2 allows priority 1: PendSV is taken, SysTick stays pending.
    core.registers.basepri = 0x40;
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), PENDSV_HANDLER & !1);
    assert_eq!(ipsr(&core), 14);
    assert!(core.system_exception_state().systick_pending);
}

#[test]
fn primask_masks_systick_and_pendsv() {
    let (mut core, mut bus) = fixture();
    core.pend_systick();
    core.pend_pendsv();

    core.registers.primask = 1;
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), THREAD + 2);
    assert_eq!(core.active_exceptions, Vec::<u16>::new());
    assert!(core.system_exception_state().pendsv_pending);

    core.registers.primask = 0;
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), PENDSV_HANDLER & !1);
    assert_eq!(ipsr(&core), 14);
}

#[test]
fn faultmask_masks_systick_and_pendsv() {
    let (mut core, mut bus) = fixture();
    core.pend_systick();
    core.registers.faultmask = 1;
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), THREAD + 2);
    assert_eq!(core.active_exceptions, Vec::<u16>::new());

    core.registers.faultmask = 0;
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), SYSTICK_HANDLER & !1);
    assert_eq!(ipsr(&core), 15);
}

#[test]
fn systick_and_external_irq_tie_picks_exception_15_first() {
    let (mut core, mut bus) = fixture();
    pend_external(&mut core, 0);
    core.pend_systick();

    // 同优先级 0：异常 15 小于异常 16，SysTick 先取。
    // Both priority 0: exception 15 is below 16, so SysTick wins.
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), SYSTICK_HANDLER & !1);
    assert_eq!(ipsr(&core), 15);

    // 返回后外部 IRQ0 以同优先级尾链进入。 / IRQ0 tail-chains at equal priority.
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), IRQ0_HANDLER & !1);
    assert_eq!(ipsr(&core), 16);
    assert_eq!(core.active_exceptions, vec![16]);
}

#[test]
fn external_irq_15_is_still_exception_31_and_not_systick() {
    let (mut core, mut bus) = fixture();
    // 外部 IRQ15 走 NVIC：异常 16+15 = 31，向量 VTOR + 0x7C。
    // External IRQ15 goes through the NVIC: exception 31 at VTOR + 0x7C.
    pend_external(&mut core, 15);
    assert_eq!(
        icsr_value(&core) & icsr::PENDSTSET,
        0,
        "ISPR must not set PENDSTSET / ISPR 不得置位 PENDSTSET"
    );
    assert_eq!((icsr_value(&core) >> icsr::VECTPENDING_SHIFT) & 0x1FF, 31);

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), IRQ15_HANDLER & !1);
    assert_eq!(ipsr(&core), 31);
    assert_eq!(core.active_exceptions, vec![31]);
    assert_eq!(
        core.system_exception_state().summary(),
        "systick=idle pendsv=idle"
    );
}

#[test]
fn exception_return_resumes_the_thread_and_executes_the_next_instruction() {
    let (mut core, mut bus) = fixture();
    core.pend_systick();
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), SYSTICK_HANDLER & !1);
    assert_eq!(core.registers.msp, STACK_TOP - 32);

    // BX LR 携带 0xFFFF_FFF9：出栈恢复线程 SP/PC/LR。
    // BX LR carries 0xFFFF_FFF9: unstacking restores the thread SP/PC/LR.
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), THREAD + 2);
    assert_eq!(core.registers.msp, STACK_TOP);
    assert_eq!(core.read_register(14), 0xFFFF_FFFF);
    assert_eq!(core.active_exceptions, Vec::<u16>::new());
    assert_eq!(
        core.system_exception_state().summary(),
        "systick=idle pendsv=idle"
    );

    // 后续指令继续执行，未被异常返回破坏。
    // Later instructions still execute after the return.
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), THREAD + 4);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.read_register(0), 7);
}

#[test]
fn systick_from_a_psp_thread_stacks_on_psp_and_returns_with_bit2_set() {
    let (mut core, mut bus) = fixture();
    core.registers.control = 0x2;
    core.write_register(13, 0x2000_0800);
    core.pend_systick();
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);

    assert_eq!(core.pc(), SYSTICK_HANDLER & !1);
    assert_eq!(core.read_register(14), 0xFFFF_FFFD);
    assert_eq!(core.registers.psp, 0x2000_0800 - 32);
    let mut cycles = 0u64;
    assert_eq!(
        bus.read_u32(0x2000_0800 - 32 + 24, &mut cycles).unwrap(),
        THREAD + 2
    );

    // 异常返回恢复 PSP 线程与非安全 Thread 模式。 / The return restores the PSP thread.
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.registers.psp, 0x2000_0800);
    assert_eq!(core.registers.control & 0x2, 0x2);
    assert_eq!(core.pc(), THREAD + 2);
}

#[test]
fn shpr3_priorities_read_back_and_gate_equal_priority_nesting() {
    let (mut core, mut bus) = fixture();
    // PendSV 字节 0x20（优先级 1）、SysTick 字节 0x40（优先级 2）。
    // PendSV byte 0x20 (priority 1), SysTick byte 0x40 (priority 2).
    core.system_write(SCB_SHPR3, 0x4020_0000);
    assert_eq!(core.system_read(SCB_SHPR3), Some(0x4020_0000));
    assert_eq!(core.system_exception_state().pendsv_priority, 1);
    assert_eq!(core.system_exception_state().systick_priority, 2);

    // SHPR2 只保存 SVCall 字节。 / SHPR2 stores the SVCall byte only.
    core.system_write(SCB_SHPR2, 0xFFFF_FFFF);
    assert_eq!(core.system_read(SCB_SHPR2), Some(0xE000_0000));

    // SysTick 处理程序改为自旋，以便观察嵌套。 / Spin in the SysTick handler.
    write_code(&mut bus, SYSTICK_HANDLER & !1, &[SPIN]);
    core.pend_systick();
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), SYSTICK_HANDLER & !1);
    assert_eq!(ipsr(&core), 15);

    // 同优先级 2 的外部 IRQ5 不能抢占，只能等待返回。
    // External IRQ5 at the same priority 2 cannot preempt; it must wait.
    pend_external(&mut core, 5);
    core.system_write(NVIC_IPR + 5, 0x40);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), SYSTICK_HANDLER & !1);
    assert_eq!(core.active_exceptions, vec![15]);
    assert_eq!(ipsr(&core), 15);

    // 提高 IRQ5 到优先级 0 后即可嵌套，EXC_RETURN 变为 Handler→Handler。
    // Raising IRQ5 to priority 0 allows nesting with a Handler-to-Handler return.
    core.system_write(NVIC_IPR + 5, 0x00);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), IRQ5_HANDLER & !1);
    assert_eq!(ipsr(&core), 21);
    assert_eq!(core.active_exceptions, vec![15, 21]);
    assert_eq!(core.read_register(14), 0xFFFF_FFF1);
}

#[test]
fn core_description_exposes_the_stable_system_exception_suffix() {
    let (mut core, _bus) = fixture();
    assert!(
        core.description().ends_with("[systick=idle pendsv=idle]"),
        "{}",
        core.description()
    );
    core.pend_systick();
    assert!(core.description().contains("[systick=pending pendsv=idle]"));
    core.pend_pendsv();
    assert!(
        core.description()
            .contains("[systick=pending pendsv=pending]")
    );
}

#[test]
fn wfi_wakes_on_a_pending_systick_exception() {
    let (mut core, mut bus) = fixture();
    // 把线程指令替换为 WFI。 / Replace the thread program with WFI.
    write_code(&mut bus, THREAD, &[0xBF30, NOP, SPIN]);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Sleeping);

    core.pend_systick();
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), SYSTICK_HANDLER & !1);
    assert_eq!(ipsr(&core), 15);
}
