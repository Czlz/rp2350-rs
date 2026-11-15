//! Interrupt-delivery gate tests / 中断投递门控测试
//!
//! The core skips [`CortexM33Core::check_pending_exceptions`]'s scan whenever
//! `Nvic::has_runnable` reports "nothing enabled is pending". These tests pin
//! the observable behaviour that gate must preserve: enable state, PRIMASK,
//! ICPR clears and SysTick-driven pending bits all still decide delivery.
//! 当 `Nvic::has_runnable` 报告「没有已使能且挂起的中断」时，核心跳过
//! [`CortexM33Core::check_pending_exceptions`] 的扫描。本模块锁定该门控必须保持
//! 的可观察行为：使能状态、PRIMASK、ICPR 清除与 SysTick 置挂起仍然决定投递。

use super::*;

/// NVIC ISER0 absolute address / NVIC ISER0 绝对地址
const ISER0: u32 = 0xE000_E100;

/// NVIC ICPR0 absolute address / NVIC ICPR0 绝对地址
const ICPR0: u32 = 0xE000_E180;

/// SysTick control register absolute address / SysTick 控制寄存器绝对地址
const SYST_CSR: u32 = 0xE000_E010;

/// SysTick reload value register absolute address / SysTick 重载值寄存器绝对地址
const SYST_RVR: u32 = 0xE000_E014;

/// SysTick current value register absolute address / SysTick 当前值寄存器绝对地址
const SYST_CVR: u32 = 0xE000_E018;

/// Build a bus with vectors, NOP code and a stack, and program one handler entry
/// 构建带向量表、NOP 代码与栈的总线，并写入一个异常处理入口
fn gate_bus(handler_vector: u32, handler: u32) -> rp2350_core::memory::SimpleBus {
    let mut bus = rp2350_core::memory::SimpleBus::new();
    bus.register_region(Box::new(rp2350_core::memory::RamRegion::new(
        "code",
        0x0000_0000,
        0x400,
    )))
    .unwrap();
    bus.register_region(Box::new(rp2350_core::memory::RamRegion::new(
        "stack",
        0x2000_0000,
        0x1000,
    )))
    .unwrap();
    let mut cycles = 0;
    bus.write_u32(0x0000_0000, 0x2000_1000, &mut cycles)
        .unwrap();
    bus.write_u32(0x0000_0004, 0x0000_0101, &mut cycles)
        .unwrap();
    bus.write_u32(handler_vector, handler, &mut cycles).unwrap();
    // 0x100..0x10C 全部是 NOP（0xBF00）。
    for offset in (0x100..0x10C).step_by(2) {
        bus.write_u16(offset, 0xBF00, &mut cycles).unwrap();
    }
    bus
}

/// Boot a core on `bus` / 在 `bus` 上启动核心
fn boot(core: &mut CortexM33Core, bus: &mut rp2350_core::memory::SimpleBus) {
    core.boot(bus).unwrap();
    assert_eq!(core.state, CpuState::Running);
}

#[test]
fn pending_irq_waits_for_enable_then_is_delivered() {
    // IRQ5 → exception 16 + 5 = 21 → vector at 21 * 4 = 0x54.
    // IRQ5 → 异常号 16 + 5 = 21 → 向量位于 21 * 4 = 0x54。
    let mut bus = gate_bus(0x54, 0x0000_0301);
    let mut core = CortexM33Core::new(CpuId::Core0);
    boot(&mut core, &mut bus);

    // 挂起但未使能：门控为假，核心继续取下一条 NOP。
    // Pending but not enabled: the gate stays false and the core runs on.
    core.signal_interrupt(5);
    assert!(!core.nvic.has_runnable(), "not enabled yet / 尚未使能");
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x0000_0102);
    assert_eq!(core.read_register(14), 0xFFFF_FFFF);

    // 使能后门控为真，下一步必须投递异常。
    // Enabling makes the gate true, so the next step must deliver.
    core.system_write(ISER0, 1 << 5);
    assert!(core.nvic.has_runnable());
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x0000_0300);
    assert_eq!(core.read_register(14), 0xFFFF_FFF9);
    assert!(core.active_exceptions.contains(&21));
}

#[test]
fn primask_still_masks_a_runnable_interrupt() {
    let mut bus = gate_bus(0x54, 0x0000_0301);
    let mut core = CortexM33Core::new(CpuId::Core0);
    boot(&mut core, &mut bus);
    core.system_write(ISER0, 1 << 5);
    core.signal_interrupt(5);
    assert!(core.nvic.has_runnable());

    // PRIMASK=1 时即使门控为真也不投递。
    // With PRIMASK=1 the gate is true but nothing is delivered.
    core.registers.primask = 1;
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x0000_0102);

    // 解除屏蔽后才投递。
    // Unmasking delivers on the next step.
    core.registers.primask = 0;
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x0000_0300);
}

#[test]
fn icpr_clear_before_the_step_closes_the_gate_again() {
    let mut bus = gate_bus(0x54, 0x0000_0301);
    let mut core = CortexM33Core::new(CpuId::Core0);
    boot(&mut core, &mut bus);
    core.system_write(ISER0, 1 << 5);
    core.signal_interrupt(5);
    assert!(core.nvic.has_runnable());

    // ICPR 清除挂起后门控必须回落，且不投递。
    // Clearing pending through ICPR must close the gate and deliver nothing.
    core.system_write(ICPR0, 1 << 5);
    assert!(!core.nvic.has_runnable());
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x0000_0102);
    assert_eq!(core.active_exceptions.len(), 0);
}

#[test]
fn systick_overflow_opens_the_gate_for_its_vector() {
    // SysTick 在本模型中按 IRQ15 挂起：异常号 31，向量位于 31 * 4 = 0x7C。
    // SysTick pendings as IRQ15 in this model: exception 31, vector at 0x7C.
    let mut bus = gate_bus(0x7C, 0x0000_0301);
    let mut core = CortexM33Core::new(CpuId::Core0);
    boot(&mut core, &mut bus);
    core.system_write(ISER0, 1 << 15);
    assert!(!core.nvic.has_runnable());

    // 使能 SysTick（ENABLE|TICKINT|CLKSOURCE），重载值 1，当前值 1。
    // Enable SysTick (ENABLE|TICKINT|CLKSOURCE) with reload 1 and current 1.
    core.system_write(SYST_RVR, 1);
    core.system_write(SYST_CVR, 1);
    core.system_write(SYST_CSR, 0x7);
    assert!(!core.nvic.has_runnable());

    // 一个周期后计数器归零并置挂起，同一步内即完成投递（PRIMASK=0）。
    // One tick wraps the counter, pends the IRQ, and delivery happens in the
    // same step because PRIMASK is clear.
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x0000_0300);
    assert!(core.active_exceptions.contains(&31));
    assert!(!core.nvic.has_runnable());
}

#[test]
fn gate_and_selector_agree_on_every_step_of_a_mixed_sequence() {
    let mut bus = gate_bus(0x54, 0x0000_0301);
    let mut core = CortexM33Core::new(CpuId::Core0);
    boot(&mut core, &mut bus);
    core.registers.primask = 1;

    // 伪随机地改写使能/挂起集合，门控为假时必须与选择器为 None 一致。
    // Randomly rewrite the enable/pending sets; a closed gate must agree with
    // the selector returning `None`.
    let mut state = 0x1234_5678u64;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as u32
    };
    for _ in 0..256 {
        let irq = next() % 32;
        match next() % 4 {
            0 => core.signal_interrupt(irq as u16),
            1 => core.nvic.write_icer(0, 1 << irq),
            2 => core.nvic.write_icpr(0, 1 << irq),
            _ => core.nvic.write_ipr_word((irq / 4) as usize, next()),
        }
        core.system_write(ISER0, next());
        if !core.nvic.has_runnable() {
            assert_eq!(
                core.nvic.find_highest_priority(),
                None,
                "a closed gate must never hide a runnable interrupt / 门控为假时不得漏掉可投递中断"
            );
        }
        core.step(&mut bus).unwrap();
    }
}
