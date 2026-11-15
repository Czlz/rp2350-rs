//! SysTick forwarding tests / SysTick 转发测试
//!
//! These tests drive a synthetic Arm ELF whose guest program writes
//! `SYST_RVR`/`SYST_CVR`/`SYST_CSR` through the PPB and whose exception-15
//! handler increments an SRAM word. They are the instruction-level evidence for
//! the split of responsibilities: the peripheral `SysTickCounter` owns counting,
//! and `Machine` forwards its one-shot expiry latch to
//! `CortexM33Core::pend_systick()` on every pump.
//! 这些测试驱动一个合成 Arm ELF：客户机经 PPB 写 `SYST_RVR`/`SYST_CVR`/
//! `SYST_CSR`，异常 15 处理程序自增一个 SRAM 字。它们是职责划分的指令级证据：
//! 外设 `SysTickCounter` 负责计数，`Machine` 在每次泵送时把一次性到期锁存转发给
//! `CortexM33Core::pend_systick()`。

// SysTick 是 ARM 参考手册中的驼峰寄存器名，测试文档保持硬件写法。
// SysTick is the CamelCase name used by the ARM reference manuals; the test
// documentation keeps the hardware spelling.
#![allow(clippy::doc_markdown)]

use super::*;
use crate::test_support::{SYSTICK_COUNTER, SYSTICK_HANDLER, infinite_loop_elf, systick_elf};

/// `SYST_CSR` absolute address / `SYST_CSR` 绝对地址
const SYST_CSR: u32 = 0xE000_E010;

/// `ENABLE | TICKINT | CLKSOURCE` / `ENABLE | TICKINT | CLKSOURCE`
const CSR_PROCESSOR_TICK: u32 = 0x7;

/// `ENABLE | CLKSOURCE` (no TICKINT) / `ENABLE | CLKSOURCE`（无 TICKINT）
const CSR_PROCESSOR_ONLY: u32 = 0x5;

/// `ENABLE` only, reference clock (no TICKINT) / 仅 `ENABLE`、参考时钟（无 TICKINT）
const CSR_REFERENCE_ONLY: u32 = 0x1;

/// A reload value far above any expiry window / 远大于任何到期窗口的重载值
const NO_EXPIRY_RELOAD: u32 = 0x00FF_FFF0;

/// Build a machine around the SysTick fixture / 围绕 SysTick 夹具构建机器
fn systick_machine(reload: u32, csr: u32, masked_delay: Option<u32>) -> Machine {
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles: 2_000_000,
        strict_decode: false,
        psram: None,
    };
    Machine::new(&systick_elf(reload, csr, masked_delay), &options).expect("machine")
}

/// Build a machine with a plain infinite loop / 构建使用普通死循环的机器
fn idle_machine() -> Machine {
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles: 20_000,
        strict_decode: false,
        psram: None,
    };
    Machine::new(&infinite_loop_elf(), &options).expect("machine")
}

/// Read one word through the machine bus / 通过机器总线读取一个字
fn read_word(machine: &mut Machine, addr: u32) -> u32 {
    let mut cycles = 0u64;
    machine.bus.read_u32(addr, &mut cycles).expect("bus read")
}

/// Handler entries recorded by the fixture / 夹具记录的处理程序进入次数
fn handler_count(machine: &mut Machine) -> u32 {
    read_word(machine, SYSTICK_COUNTER)
}

/// Step until the SysTick handler is entered, returning its step and IPSR
/// 单步直到进入 SysTick 处理程序，返回步号与 IPSR
fn run_until_handler(machine: &mut Machine, limit: u64) -> Option<(u64, u32)> {
    for _ in 0..limit {
        machine.step().expect("step");
        if let Some(reason) = machine.halt_reason() {
            panic!("halt at PC={:#010X}: {reason}", machine.pc());
        }
        if machine.pc() == SYSTICK_HANDLER {
            return Some((machine.steps(), machine.xpsr() & 0x1FF));
        }
    }
    None
}

#[test]
fn a_systick_underflow_enters_exception_15_at_vtor_plus_0x3c() {
    // 小重载值在共享时基第一次推进时到期；处理程序地址只能由异常 15
    // （VTOR + 0x3C）得到，外部 IRQ 15 会进入异常 31。
    // A small reload expires on the first timebase advance; only exception 15
    // (VTOR + 0x3C) can reach the handler, external IRQ 15 would enter
    // exception 31.
    let mut machine = systick_machine(999, CSR_PROCESSOR_TICK, None);
    let (step, ipsr) = run_until_handler(&mut machine, 12_000).expect("SysTick handler entry");
    assert_eq!(ipsr, 15, "IPSR must report the SysTick system exception");
    assert!(
        step >= ADVANCE_INTERVAL,
        "the expiry must come from the shared timebase, step={step}"
    );
    machine.run_steps(16).expect("run the handler");
    assert!(
        handler_count(&mut machine) > 0,
        "handler must increment SRAM"
    );
}

#[test]
fn the_clock_source_selects_the_live_clock_tree_rate() {
    // 同样的 10 000 个 clk_sys 周期：CLKSOURCE=1 按 150 MHz 计 10 000 个源周期，
    // CLKSOURCE=0 按 clk_ref/2 = 6 MHz 计 400 个源周期。
    // The same 10_000 clk_sys cycles: CLKSOURCE=1 counts 10_000 source cycles at
    // 150 MHz, CLKSOURCE=0 counts 400 at clk_ref/2 = 6 MHz.
    fn cvr_after_one_window(csr: u32) -> u32 {
        let mut machine = systick_machine(NO_EXPIRY_RELOAD, csr, None);
        machine
            .run_steps(ADVANCE_INTERVAL)
            .expect("one timebase window");
        assert_eq!(machine.steps(), ADVANCE_INTERVAL);
        machine.peripherals().systick().lock().cvr()
    }

    let processor = cvr_after_one_window(CSR_PROCESSOR_ONLY);
    let reference = cvr_after_one_window(CSR_REFERENCE_ONLY);
    assert_eq!(
        NO_EXPIRY_RELOAD - processor,
        10_000,
        "CLKSOURCE=1 counts clk_sys cycles"
    );
    assert_eq!(
        NO_EXPIRY_RELOAD - reference,
        400,
        "CLKSOURCE=0 counts clk_ref/2 cycles (6 MHz at 150 MHz sysclk)"
    );
}

#[test]
fn primask_holds_the_forwarded_expiry_until_it_is_cleared() {
    let mut machine = systick_machine(999, CSR_PROCESSOR_TICK, Some(25_000));
    machine.run_steps(45_000).expect("masked window");
    assert_eq!(
        handler_count(&mut machine),
        0,
        "PRIMASK must keep the forwarded expiry out of the handler"
    );
    assert!(
        machine.core_description().contains("systick=pending"),
        "the core keeps the exception pending: {}",
        machine.core_description()
    );

    // cpsie 在第 50 011 步左右执行；解除屏蔽后补投递一次（第 50 012 步进入）。
    // 第 50 048 步才会转发 50 000 步窗口的新到期，因此 50 020 步时只剩这次补投递。
    // `cpsie` runs around step 50_011; exactly one deferred delivery follows on
    // step 50_012. The 50_000-step window's new expiry is forwarded only at
    // step 50_048, so step 50_020 sees just the deferred delivery.
    machine.run_steps(5_020).expect("unmasked window");
    assert_eq!(handler_count(&mut machine), 1, "one deferred delivery");
    assert!(
        machine.core_description().contains("systick=idle"),
        "the pending latch is consumed: {}",
        machine.core_description()
    );
}

#[test]
fn one_expiry_latch_enters_exactly_one_exception() {
    let mut machine = systick_machine(999, CSR_PROCESSOR_TICK, None);
    machine.run_steps(19_000).expect("first window");
    assert_eq!(handler_count(&mut machine), 1, "one underflow, one entry");
    machine.run_steps(2_000).expect("second window");
    assert_eq!(
        handler_count(&mut machine),
        2,
        "the next counter wrap pends exactly once more"
    );
}

#[test]
fn the_irq_log_records_the_forwarded_systick_expiry() {
    let mut machine = systick_machine(999, CSR_PROCESSOR_TICK, None);
    machine.run_steps(11_000).expect("one expiry");
    let lines: Vec<&str> = machine
        .irqs()
        .iter()
        .map(String::as_str)
        .filter(|line| line.contains("systick"))
        .collect();
    assert_eq!(lines.len(), 1, "one forwarded expiry: {:?}", machine.irqs());
    assert!(
        lines[0].starts_with("[irq] core0 systick at PC=0x"),
        "stable format: {}",
        lines[0]
    );
}

#[test]
fn batched_steps_and_single_steps_agree_on_systick_state() {
    let mut batched = systick_machine(999, CSR_PROCESSOR_TICK, None);
    let mut single = systick_machine(999, CSR_PROCESSOR_TICK, None);
    batched.run_steps(30_000).expect("batched steps");
    for _ in 0..30_000 {
        single.step().expect("single step");
    }

    assert_eq!(batched.steps(), single.steps());
    assert_eq!(batched.pc(), single.pc(), "same PC after the last step");
    assert_eq!(
        handler_count(&mut batched),
        handler_count(&mut single),
        "same number of handler entries"
    );
    assert_eq!(
        batched.peripherals().systick().lock().cvr(),
        single.peripherals().systick().lock().cvr(),
        "same counter phase"
    );
    assert_eq!(batched.irqs(), single.irqs(), "same delivery log");
}

#[test]
fn an_unused_systick_stays_idle() {
    // 不使用 SysTick 的固件保持 `systick=idle`，也不产生转发日志；FV 门禁因此
    // 与接线前逐字节一致。
    // Firmware that never touches SysTick keeps `systick=idle` and logs no
    // forward, so the FV gates stay byte-identical to the pre-wiring state.
    let mut machine = idle_machine();
    machine.run_steps(12_000).expect("steps");
    assert!(
        machine.core_description().contains("systick=idle"),
        "{}",
        machine.core_description()
    );
    assert!(
        machine.irqs().iter().all(|line| !line.contains("systick")),
        "{:?}",
        machine.irqs()
    );
    let systick = machine.peripherals().systick();
    let counter = systick.lock();
    assert_eq!(counter.csr(), 0);
    assert!(!counter.interrupt_pending());
}

#[test]
fn a_disabled_counter_keeps_its_value_and_never_forwards() {
    let mut machine = systick_machine(999, 0, None);
    machine.run_steps(20_000).expect("steps");
    assert_eq!(handler_count(&mut machine), 0);
    let systick = machine.peripherals().systick();
    let counter = systick.lock();
    assert_eq!(counter.cvr(), 999, "a disabled counter does not move");
    assert_eq!(counter.csr(), 0);
    assert!(!counter.interrupt_pending());
    assert!(
        machine.irqs().iter().all(|line| !line.contains("systick")),
        "{:?}",
        machine.irqs()
    );
}

#[test]
fn the_peripheral_counter_is_the_only_counting_source() {
    // 机器从不把 `SYST_CSR` 回放到核：总线看到 ENABLE/TICKINT/CLKSOURCE，
    // 核内计数器保持关闭（只可能有 `pend_systick()` 锁存的 COUNTFLAG），
    // 因此不存在双计时。
    // The machine never replays `SYST_CSR` into the core: the bus sees
    // ENABLE/TICKINT/CLKSOURCE while the core counter stays disabled (only the
    // COUNTFLAG latched by `pend_systick()` may show), so nothing ticks twice.
    let mut machine = systick_machine(999, CSR_PROCESSOR_TICK, None);
    machine.run_steps(11_000).expect("one expiry");

    assert_eq!(
        read_word(&mut machine, SYST_CSR) & 0x7,
        CSR_PROCESSOR_TICK,
        "the peripheral owns SYST_CSR"
    );
    let core_csr = machine.system_register(SYST_CSR).expect("core SYST_CSR");
    assert_eq!(
        core_csr & 0x7,
        0,
        "the core-side counter must stay disabled (no double counting)"
    );
    assert_eq!(handler_count(&mut machine), 1);
}
