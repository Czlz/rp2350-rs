//! `SCR` sleep bits, `SLEEPONEXIT`/`SLEEPDEEP`, `SEVONPEND` and `CONTROL.FPCA`
//! `SCR` 睡眠位、`SLEEPONEXIT`/`SLEEPDEEP`、`SEVONPEND` 与 `CONTROL.FPCA`
//!
//! These tests pin ARM ARM B3.2.7 (`SCR`), B1.4.2 (sleep/deep-sleep events) and
//! B3.4.4 (`CONTROL.FPCA`): a transition of an exception pending bit sends a
//! `SEVONPEND` event, `SLEEPONEXIT` suspends only on a return to Thread mode,
//! `SLEEPDEEP` is reported in the sleep reason, and an exception return
//! restores `FPCA` from the frame type.
//! 这些测试锁定 ARM ARM B3.2.7（`SCR`）、B1.4.2（sleep/deep sleep 事件）与
//! B3.4.4（`CONTROL.FPCA`）：异常挂起位跳变发送 `SEVONPEND` 事件；`SLEEPONEXIT`
//! 只在返回到 Thread 模式时挂起；`SLEEPDEEP` 记录在睡眠原因中；异常返回按帧类型
//! 恢复 `FPCA`。

use super::*;

use crate::scr;
use rp2350_core::memory::{RamRegion, SimpleBus};

/// Thread code base / 线程代码基址
const CODE: u32 = 0x0000_0100;
/// First interrupt handler address (Thumb bit set) / 第一个中断处理程序地址（含 Thumb 位）
const HANDLER5: u32 = 0x0000_0301;
/// Second interrupt handler address (Thumb bit set) / 第二个中断处理程序地址（含 Thumb 位）
const HANDLER6: u32 = 0x0000_0381;
/// Vector table slot for IRQ5 (exception 21) / IRQ5（异常 21）的向量表槽
const IRQ5_VECTOR: u32 = 0x54;
/// Vector table slot for IRQ6 (exception 22) / IRQ6（异常 22）的向量表槽
const IRQ6_VECTOR: u32 = 0x58;
/// NVIC ISER0 / NVIC ISER0
const ISER0: u32 = 0xE000_E100;
/// NVIC ISPR0 / NVIC ISPR0
const ISPR0: u32 = 0xE000_E200;
/// NVIC ICPR0 / NVIC ICPR0
const ICPR0: u32 = 0xE000_E280;
/// Software Trigger Interrupt Register / 软件触发中断寄存器
const STIR: u32 = 0xE000_EF00;
/// SysTick control and status register / SysTick 控制与状态寄存器
const SYST_CSR: u32 = 0xE000_E010;
/// SysTick reload value register / SysTick 重载值寄存器
const SYST_RVR: u32 = 0xE000_E014;
/// SysTick current value register / SysTick 当前值寄存器
const SYST_CVR: u32 = 0xE000_E018;

/// `NOP` (`0xBF00`) / `NOP`
const NOP: u16 = 0xBF00;
/// `B .` (`0xE7FE`) / `B .`
const SPIN: u16 = 0xE7FE;
/// `WFI` (`0xBF30`) / `WFI`
const WFI: u16 = 0xBF30;
/// `WFE` (`0xBF20`) / `WFE`
const WFE: u16 = 0xBF20;
/// `CPSID i` (`0xB672`) / `CPSID i`
const CPSID_I: u16 = 0xB672;
/// `BX LR` (`0x4770`) / `BX LR`
const BX_LR: u16 = 0x4770;
/// `MOVS r0, #4` (`0x2004`) / `MOVS r0, #4`
const MOVS_R0_4: u16 = 0x2004;
/// `MOVS r0, #7` (`0x2007`) / `MOVS r0, #7`
const MOVS_R0_7: u16 = 0x2007;
/// `MOVS r0, #0` (`0x2000`) / `MOVS r0, #0`
const MOVS_R0_0: u16 = 0x2000;
/// `MSR CONTROL, r0` (`0xF380_8814`) / `MSR CONTROL, r0`
const MSR_CONTROL_R0: [u16; 2] = [0xF380, 0x8814];
/// `MRS r1, CONTROL` (`0xF3EF_8114`) / `MRS r1, CONTROL`
const MRS_R1_CONTROL: [u16; 2] = [0xF3EF, 0x8114];

/// Write a halfword program / 写入半字程序
fn write_code(bus: &mut SimpleBus, base: u32, code: &[u16]) {
    let mut cycles = 0u64;
    for (index, halfword) in code.iter().enumerate() {
        bus.write_u16(base + 2 * index as u32, *halfword, &mut cycles)
            .unwrap();
    }
}

/// Build a core with thread code at `0x100` and both IRQ vectors installed
/// 构建线程代码位于 `0x100`、两个 IRQ 向量均已安装的核心
fn scr_bus(program: &[u16]) -> (CortexM33Core, SimpleBus) {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("code", 0x0000_0000, 0x800)))
        .unwrap();
    bus.register_region(Box::new(RamRegion::new("stack", 0x2000_0000, 0x1000)))
        .unwrap();
    let mut cycles = 0u64;
    bus.write_u32(0x0000_0000, 0x2000_1000, &mut cycles)
        .unwrap();
    bus.write_u32(0x0000_0004, 0x0000_0101, &mut cycles)
        .unwrap();
    bus.write_u32(IRQ5_VECTOR, HANDLER5, &mut cycles).unwrap();
    bus.write_u32(IRQ6_VECTOR, HANDLER6, &mut cycles).unwrap();
    write_code(&mut bus, CODE, program);
    let mut core = CortexM33Core::new(CpuId::Core0);
    core.boot(&mut bus).unwrap();
    (core, bus)
}

/// Enable and assert one IRQ / 使能并断言一个中断
fn enable_and_raise(core: &mut CortexM33Core, irq: u16) {
    core.system_write(ISER0, 1 << irq);
    core.signal_interrupt(irq);
}

// ---- SCR register --------------------------------------------------------

#[test]
fn scr_stores_only_the_three_sleep_bits_and_resets() {
    let mut core = CortexM33Core::new(CpuId::Core0);
    assert_eq!(core.system_read(scr::SCR), Some(0));
    core.system_write(scr::SCR, 0xFFFF_FFFF);
    assert_eq!(
        core.system_read(scr::SCR),
        Some(scr::MODELED_MASK),
        "reserved bits read zero / 保留位读为 0"
    );
    assert_eq!(scr::MODELED_MASK, 0x16);
    core.reset();
    assert_eq!(core.system_read(scr::SCR), Some(0));
}

// ---- SEVONPEND -----------------------------------------------------------

#[test]
fn sev_on_pend_sets_an_event_for_a_disabled_interrupt() {
    let (mut core, _bus) = scr_bus(&[SPIN]);
    core.system_write(scr::SCR, scr::SEVONPEND);
    assert!(!core.event_register());
    core.signal_interrupt(5);
    assert!(core.nvic.is_pending(5));
    assert!(
        !core.nvic.is_enabled(5),
        "the event must not depend on enable / 事件不得依赖使能"
    );
    assert!(
        core.event_register(),
        "a pending transition raises the event / 挂起跳变置位事件"
    );
}

#[test]
fn sev_on_pend_is_inert_when_the_bit_is_clear() {
    let (mut core, _bus) = scr_bus(&[SPIN]);
    core.signal_interrupt(5);
    assert!(core.nvic.is_pending(5));
    assert!(!core.event_register());
}

#[test]
fn sev_on_pend_event_makes_wfe_continue_once() {
    let (mut core, mut bus) = scr_bus(&[WFE, WFE, SPIN]);
    core.system_write(scr::SCR, scr::SEVONPEND);
    core.signal_interrupt(5);
    assert!(core.event_register());

    // The disabled interrupt's event wakes WFE; the IRQ itself is not taken.
    // 被禁用中断产生的事件唤醒 WFE；该 IRQ 本身不会被响应。
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x102);
    assert!(!core.event_register(), "the wake consumes the event");
    assert_eq!(core.active_exceptions.len(), 0);

    // A second assertion of the already-pending line must not send a new event.
    // 对仍挂起的中断线再次断言不得产生新事件。
    core.signal_interrupt(5);
    assert!(!core.event_register());
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Sleeping);
    assert_eq!(core.sleep_reason(), Some(SleepReason::WaitForEvent));
}

#[test]
fn sev_on_pend_event_crosses_primask_to_wake_wfe_only() {
    let (mut core, mut bus) = scr_bus(&[CPSID_I, WFE, NOP, SPIN]);
    core.system_write(scr::SCR, scr::SEVONPEND);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.registers.primask & 1, 1);

    enable_and_raise(&mut core, 5);
    assert!(core.nvic.has_runnable());
    assert!(core.event_register());

    // WFE consumes the event and continues; the masked IRQ stays pending and
    // is not entered.
    // WFE 消费事件后继续；被屏蔽的 IRQ 保持挂起且不会进入。
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x104);
    assert!(!core.event_register());
    assert_eq!(core.active_exceptions.len(), 0);
}

#[test]
fn sev_on_pend_never_wakes_wfi_by_itself() {
    let (mut core, mut bus) = scr_bus(&[WFI, NOP, SPIN]);
    core.system_write(scr::SCR, scr::SEVONPEND);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Sleeping);
    assert_eq!(core.sleep_reason(), Some(SleepReason::WaitForInterrupt));

    // A disabled interrupt raises the event but WFI ignores events.
    // 被禁用中断置位事件，但 WFI 忽略事件。
    core.signal_interrupt(5);
    assert!(core.event_register());
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Sleeping);

    // Enabling the still-pending IRQ makes it runnable and wakes WFI, which
    // enters the handler before any thread instruction.
    // 使能仍挂起的 IRQ 后它成为可运行中断并唤醒 WFI，核心在任意线程指令前进入处理程序。
    core.system_write(ISER0, 1 << 5);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x300);
    assert!(core.active_exceptions.contains(&21));
}

#[test]
fn sev_on_pend_covers_isp_writes_and_stir() {
    let (mut core, _bus) = scr_bus(&[SPIN]);
    core.system_write(scr::SCR, scr::SEVONPEND);
    core.system_write(ISPR0, 1 << 7);
    assert!(core.nvic.is_pending(7));
    assert!(
        core.event_register(),
        "a software ISPR write is a pending transition / 软件 ISPR 写入也是挂起跳变"
    );

    let (mut core, _bus) = scr_bus(&[SPIN]);
    core.system_write(scr::SCR, scr::SEVONPEND);
    core.system_write(STIR, 9);
    assert!(core.nvic.is_pending(9));
    assert!(
        core.event_register(),
        "STIR pends through the same transition path / STIR 走同一跳变路径"
    );
}

#[test]
fn sev_on_pend_covers_systick_pending() {
    let (mut core, mut bus) = scr_bus(&[NOP, NOP, NOP, SPIN]);
    core.system_write(SYST_RVR, 2);
    core.system_write(SYST_CVR, 2);
    core.system_write(SYST_CSR, 0x3); // ENABLE | TICKINT
    core.system_write(scr::SCR, scr::SEVONPEND);

    core.step(&mut bus).unwrap();
    assert!(!core.nvic.is_pending(15));
    core.step(&mut bus).unwrap();
    assert!(
        core.nvic.is_pending(15),
        "SysTick underflow pends the exception / SysTick 下溢置位异常"
    );
    assert!(
        core.event_register(),
        "the SysTick 0-to-1 transition raises SEVONPEND / SysTick 0→1 跳变产生事件"
    );
}

// ---- SLEEPDEEP -----------------------------------------------------------

#[test]
fn sleepdeep_wfi_is_reported_as_deep_and_wakes_on_an_irq() {
    let (mut core, mut bus) = scr_bus(&[WFI, NOP, SPIN]);
    core.system_write(scr::SCR, scr::SLEEPDEEP);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Sleeping);
    let reason = core.sleep_reason().unwrap();
    assert_eq!(
        reason,
        SleepReason::SleepDeep {
            wait_for_event: false
        }
    );
    assert!(reason.is_deep_sleep());

    enable_and_raise(&mut core, 5);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x300);
    assert_eq!(core.sleep_reason(), None);
}

#[test]
fn sleepdeep_wfe_still_waits_for_an_event() {
    let (mut core, mut bus) = scr_bus(&[WFE, NOP, SPIN]);
    core.system_write(scr::SCR, scr::SLEEPDEEP);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Sleeping);
    let reason = core.sleep_reason().unwrap();
    assert_eq!(
        reason,
        SleepReason::SleepDeep {
            wait_for_event: true
        }
    );
    assert!(reason.is_deep_sleep());

    core.signal_event();
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x104);
    assert!(!core.event_register());
}

// ---- SLEEPONEXIT ---------------------------------------------------------

#[test]
fn sleeponexit_sleeps_after_return_to_thread() {
    let (mut core, mut bus) = scr_bus(&[NOP, NOP, SPIN]);
    write_code(&mut bus, 0x300, &[BX_LR]);
    enable_and_raise(&mut core, 5);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x300, "handler entered / 已进入处理程序");

    core.system_write(scr::SCR, scr::SLEEPONEXIT);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Sleeping);
    assert_eq!(
        core.sleep_reason(),
        Some(SleepReason::SleepOnExit { deep: false })
    );
    assert_eq!(
        core.pc(),
        0x102,
        "PC is the restored thread address and did not advance / PC 为恢复的线程地址且未前进"
    );

    // Still asleep until a runnable interrupt arrives.
    // 在有可运行中断之前保持睡眠。
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Sleeping);
    assert_eq!(core.pc(), 0x102);

    // The wake takes the interrupt before the thread instruction.
    // 唤醒先进入中断，不执行线程指令。
    enable_and_raise(&mut core, 5);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x300);
    assert!(core.active_exceptions.contains(&21));
}

#[test]
fn sleeponexit_clear_returns_and_runs_the_next_instruction() {
    let (mut core, mut bus) = scr_bus(&[NOP, NOP, SPIN]);
    write_code(&mut bus, 0x300, &[BX_LR]);
    enable_and_raise(&mut core, 5);
    core.step(&mut bus).unwrap();
    assert_eq!(core.pc(), 0x300);

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.sleep_reason(), None);
    assert_eq!(core.pc(), 0x102);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x104);
}

#[test]
fn sleeponexit_combines_with_sleepdeep_in_the_reason() {
    let (mut core, mut bus) = scr_bus(&[NOP, NOP, SPIN]);
    write_code(&mut bus, 0x300, &[BX_LR]);
    enable_and_raise(&mut core, 5);
    core.step(&mut bus).unwrap();

    core.system_write(scr::SCR, scr::SLEEPONEXIT | scr::SLEEPDEEP);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Sleeping);
    let reason = core.sleep_reason().unwrap();
    assert_eq!(reason, SleepReason::SleepOnExit { deep: true });
    assert!(reason.is_deep_sleep());
}

#[test]
fn handler_to_handler_return_ignores_sleeponexit() {
    let (mut core, mut bus) = scr_bus(&[NOP, NOP, SPIN]);
    // Handler 5: NOP ; NOP ; BX LR.  Handler 6: BX LR.
    // 处理程序 5：NOP ; NOP ; BX LR。处理程序 6：BX LR。
    write_code(&mut bus, 0x300, &[NOP, NOP, BX_LR]);
    write_code(&mut bus, 0x380, &[BX_LR]);

    enable_and_raise(&mut core, 5);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x300);

    // Nest IRQ6 while IRQ5 is active: the first handler instruction retires,
    // then the higher-priority candidate is taken from Handler mode.
    // 在 IRQ5 活跃时嵌套 IRQ6：处理程序第一条指令退役后，从 Handler 模式进入候选中断。
    enable_and_raise(&mut core, 6);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x380);
    assert_eq!(
        core.registers.lr, 0xFFFF_FFF1,
        "a nested entry must return to Handler mode / 嵌套进入必须返回 Handler 模式"
    );

    core.system_write(scr::SCR, scr::SLEEPONEXIT);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.sleep_reason(), None);
    assert_eq!(
        core.pc(),
        0x302,
        "the outer handler resumes / 外层处理程序恢复"
    );
    assert!(core.active_exceptions.contains(&21));

    // The outer Handler-to-Thread return does sleep.
    // 外层 Handler→Thread 返回才会睡眠。
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x304);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Sleeping);
    assert_eq!(
        core.sleep_reason(),
        Some(SleepReason::SleepOnExit { deep: false })
    );
    assert_eq!(core.pc(), 0x102);
}

// ---- CONTROL.FPCA through MRS/MSR ---------------------------------------

#[test]
fn msr_control_writes_fpca_and_mrs_reads_three_bits() {
    let (mut core, mut bus) = scr_bus(&[
        MOVS_R0_7,
        MSR_CONTROL_R0[0],
        MSR_CONTROL_R0[1],
        MRS_R1_CONTROL[0],
        MRS_R1_CONTROL[1],
        SPIN,
    ]);
    core.step(&mut bus).unwrap();
    core.step(&mut bus).unwrap();
    assert_eq!(
        core.registers.control, 0x7,
        "MSR CONTROL stores nPRIV/SPSEL/FPCA / MSR CONTROL 保存三位"
    );

    core.step(&mut bus).unwrap();
    assert_eq!(core.read_register(1), 0x7, "MRS CONTROL returns three bits");
}

#[test]
fn basic_frame_return_clears_fpca_written_by_msr() {
    let (mut core, mut bus) = scr_bus(&[NOP, NOP, SPIN]);
    write_code(
        &mut bus,
        0x300,
        &[MOVS_R0_4, MSR_CONTROL_R0[0], MSR_CONTROL_R0[1], BX_LR],
    );
    enable_and_raise(&mut core, 5);
    core.step(&mut bus).unwrap();
    assert_eq!(core.registers.lr, 0xFFFF_FFF9, "basic frame / 基础帧");

    core.step(&mut bus).unwrap();
    core.step(&mut bus).unwrap();
    assert_eq!(core.registers.control, 0x4);
    assert!(core.registers.fp_context_active());

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.registers.control & crate::registers::CONTROL_FPCA, 0);
    assert_eq!(core.pc(), 0x102);
}

#[test]
fn extended_frame_return_sets_fpca_after_the_handler_clear() {
    let (mut core, mut bus) =
        scr_bus(&[MOVS_R0_4, MSR_CONTROL_R0[0], MSR_CONTROL_R0[1], NOP, SPIN]);
    write_code(
        &mut bus,
        0x300,
        &[MOVS_R0_0, MSR_CONTROL_R0[0], MSR_CONTROL_R0[1], BX_LR],
    );
    core.step(&mut bus).unwrap();
    core.step(&mut bus).unwrap();
    assert_eq!(core.registers.control, 0x4);

    enable_and_raise(&mut core, 5);
    core.step(&mut bus).unwrap();
    assert_eq!(core.registers.lr, 0xFFFF_FFE9, "extended frame / 扩展帧");
    assert_eq!(core.pc(), 0x300);

    core.step(&mut bus).unwrap();
    core.step(&mut bus).unwrap();
    assert_eq!(core.registers.control, 0);

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(
        core.registers.control & crate::registers::CONTROL_FPCA,
        crate::registers::CONTROL_FPCA
    );
    assert_eq!(core.pc(), 0x108);
}

#[test]
fn clearing_a_pending_bit_allows_the_next_transition_to_signal_again() {
    let (mut core, _bus) = scr_bus(&[SPIN]);
    core.system_write(scr::SCR, scr::SEVONPEND);
    core.signal_interrupt(5);
    assert!(core.event_register());

    // Consume the event with WFE, then clear and re-pend the IRQ.
    // 用 WFE 消费事件，然后清除并重新挂起该 IRQ。
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("code", 0x0000_0000, 0x800)))
        .unwrap();
    let mut cycles = 0u64;
    bus.write_u16(0x100, WFE, &mut cycles).unwrap();
    bus.write_u16(0x102, SPIN, &mut cycles).unwrap();
    core.registers.pc = 0x100;
    core.state = CpuState::Running;
    core.step(&mut bus).unwrap();
    assert!(!core.event_register());

    core.system_write(ICPR0, 1 << 5);
    assert!(!core.nvic.is_pending(5));
    core.signal_interrupt(5);
    assert!(
        core.event_register(),
        "a new 0-to-1 transition sends an event"
    );
}
