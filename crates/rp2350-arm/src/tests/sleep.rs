//! Sleep, event and blocked-access tests / 睡眠、事件与阻塞访问测试
//!
//! They pin the ARMv8-M `WFI`/`WFE`/`SEV` semantics the machine scheduler relies
//! on, plus the retry contract for a load that hits a blocking device.
//! 它们锁定机器调度器依赖的 ARMv8-M `WFI`/`WFE`/`SEV` 语义，以及命中阻塞设备的
//! 载入的重试契约。

// 测试里 `code`（代码）与 `core`（核心）成对出现，名字相似告警在此不适用。
// The tests use `code` (code buffer) next to `core` (processor); the
// similar-names lint does not apply.
#![allow(clippy::similar_names)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use super::*;

/// NVIC ISER0 absolute address / NVIC ISER0 绝对地址
const ISER0: u32 = 0xE000_E100;

/// Vector handler used by the interrupt tests / 中断测试使用的处理程序地址
const HANDLER: u32 = 0x0000_0301;

/// `WFI` (`0xBF30`) / `WFI`
const WFI: u16 = 0xBF30;

/// `WFE` (`0xBF20`) / `WFE`
const WFE: u16 = 0xBF20;

/// `SEV` (`0xBF40`) / `SEV`
const SEV: u16 = 0xBF40;

/// `NOP` (`0xBF00`) / `NOP`
const NOP: u16 = 0xBF00;

/// `B .` (`0xE7FE`) / `B .`
const SPIN: u16 = 0xE7FE;

/// `LDR r0, [r1]` (`0x6808`) / `LDR r0, [r1]`
const LDR_R0_R1: u16 = 0x6808;

/// `ADDS r0, #1` (`0x3001`) / `ADDS r0, #1`
const ADDS_R0_1: u16 = 0x3001;

/// Base of the fake mailbox region / 伪信箱区域基址
const MAILBOX_BASE: u32 = 0xD000_0000;

/// Offset of the blocking read register / 阻塞读寄存器偏移
const MAILBOX_RD: u32 = 0x058;

/// A mailbox that blocks its read register while empty
/// 读取空时阻塞读寄存器的信箱
#[derive(Debug, Clone)]
struct Mailbox {
    frames: Arc<Mutex<VecDeque<u32>>>,
    reads: Arc<Mutex<u32>>,
}

impl Mailbox {
    fn new() -> Self {
        Self {
            frames: Arc::new(Mutex::new(VecDeque::new())),
            reads: Arc::new(Mutex::new(0)),
        }
    }

    fn push(&self, value: u32) {
        self.frames.lock().unwrap().push_back(value);
    }

    fn is_empty(&self) -> bool {
        self.frames.lock().unwrap().is_empty()
    }

    fn reads(&self) -> u32 {
        *self.reads.lock().unwrap()
    }
}

impl rp2350_core::bus::MemoryRegion for Mailbox {
    fn name(&self) -> &'static str {
        "mailbox"
    }

    fn base(&self) -> u32 {
        MAILBOX_BASE
    }

    fn size(&self) -> usize {
        0x1000
    }

    fn read_would_stall(&self, addr: u32) -> bool {
        addr - MAILBOX_BASE == MAILBOX_RD && self.is_empty()
    }

    fn read_u8(&mut self, _addr: u32, cycles: &mut u64) -> rp2350_core::bus::BusResult<u8> {
        *cycles += 1;
        *self.reads.lock().unwrap() += 1;
        Ok(0)
    }

    fn read_u32(&mut self, _addr: u32, cycles: &mut u64) -> rp2350_core::bus::BusResult<u32> {
        *cycles += 1;
        *self.reads.lock().unwrap() += 1;
        Ok(self.frames.lock().unwrap().pop_front().unwrap_or(0))
    }

    fn write_u8(
        &mut self,
        _addr: u32,
        _value: u8,
        cycles: &mut u64,
    ) -> rp2350_core::bus::BusResult<()> {
        *cycles += 1;
        Ok(())
    }
}

/// Build a bus with a vector table, `code` at `0x100` and a stack
/// 构建带向量表、`0x100` 处代码与栈的总线
fn sleep_bus(code: &[u16]) -> (CortexM33Core, rp2350_core::memory::SimpleBus, Mailbox) {
    use rp2350_core::memory::{RamRegion, SimpleBus};
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("code", 0x0000_0000, 0x800)))
        .unwrap();
    bus.register_region(Box::new(RamRegion::new("stack", 0x2000_0000, 0x1000)))
        .unwrap();
    let mailbox = Mailbox::new();
    bus.register_region(Box::new(mailbox.clone())).unwrap();
    let mut cycles = 0u64;
    bus.write_u32(0x0000_0000, 0x2000_1000, &mut cycles)
        .unwrap();
    bus.write_u32(0x0000_0004, 0x0000_0101, &mut cycles)
        .unwrap();
    // IRQ5 → exception 21 → vector at 21 * 4 = 0x54.
    // IRQ5 → 异常号 21 → 向量位于 21 * 4 = 0x54。
    bus.write_u32(0x54, HANDLER, &mut cycles).unwrap();
    for (index, halfword) in code.iter().enumerate() {
        bus.write_u16(0x100 + 2 * index as u32, *halfword, &mut cycles)
            .unwrap();
    }
    let mut core = CortexM33Core::new(CpuId::Core0);
    core.boot(&mut bus).unwrap();
    (core, bus, mailbox)
}

/// Enable IRQ5 and make it pending / 使能 IRQ5 并置为挂起
fn raise_irq5(core: &mut CortexM33Core) {
    core.system_write(ISER0, 1 << 5);
    core.signal_interrupt(5);
    assert!(core.nvic.has_runnable());
}

// ---- WFI -----------------------------------------------------------------

#[test]
fn wfi_sleeps_without_a_pending_interrupt() {
    let (mut core, mut bus, _mailbox) = sleep_bus(&[WFI, NOP, SPIN]);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Sleeping);
    assert_eq!(core.sleep_reason(), Some(SleepReason::WaitForInterrupt));
    assert_eq!(core.pc(), 0x102, "WFI retires before the sleep begins");
    // 睡眠步不退役指令，但时间继续。
    // A sleep step retires nothing while time keeps moving.
    let before = core.total_cycles();
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Sleeping);
    assert_eq!(core.pc(), 0x102);
    assert!(core.total_cycles() > before);
}

#[test]
fn wfi_does_not_sleep_with_a_runnable_interrupt() {
    let (mut core, mut bus, _mailbox) = sleep_bus(&[WFI, NOP, SPIN]);
    raise_irq5(&mut core);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.sleep_reason(), None, "WFI must not suspend here");
    // 同一步内已进入异常处理器，WFI 之后的 NOP 尚未执行。
    // The exception was entered in the same step; the NOP after WFI has not run.
    assert_eq!(core.pc(), 0x300);
    assert!(core.active_exceptions.contains(&21));
}

#[test]
fn wfi_wakes_on_a_later_interrupt_and_enters_the_handler() {
    let (mut core, mut bus, _mailbox) = sleep_bus(&[WFI, NOP, SPIN]);
    core.step(&mut bus).unwrap();
    assert_eq!(core.state, CpuState::Sleeping);
    raise_irq5(&mut core);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x300);
    assert_eq!(core.sleep_reason(), None);
    // 唤醒时先进入中断，而不是先执行一条线程指令。
    // The wake enters the handler before any thread instruction runs.
    assert!(core.active_exceptions.contains(&21));
}

#[test]
fn primask_masked_interrupt_keeps_wfi_awake_without_entering() {
    // `CPSID i` (0xB672) then WFI: the pending IRQ is enabled but masked.
    // `CPSID i`（0xB672）后接 WFI：挂起中断已使能但被屏蔽。
    let (mut core, mut bus, _mailbox) = sleep_bus(&[0xB672, WFI, NOP, SPIN]);
    raise_irq5(&mut core);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x102, "CPSID retires");
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.sleep_reason(), None, "a pending IRQ prevents sleeping");
    assert_eq!(core.pc(), 0x104, "but the masked IRQ is not taken");
    assert_eq!(core.active_exceptions.len(), 0);
}

#[test]
fn wfi_ignores_events() {
    let (mut core, mut bus, _mailbox) = sleep_bus(&[WFI, NOP, SPIN]);
    core.step(&mut bus).unwrap();
    assert_eq!(core.state, CpuState::Sleeping);
    core.signal_event();
    assert_eq!(
        core.step(&mut bus).unwrap(),
        CpuState::Sleeping,
        "SEV must never wake WFI / SEV 绝不唤醒 WFI"
    );
    assert!(
        core.event_register(),
        "the event stays pending for a later WFE"
    );
}

// ---- WFE / SEV -----------------------------------------------------------

#[test]
fn wfe_sleeps_and_an_event_wakes_it() {
    let (mut core, mut bus, _mailbox) = sleep_bus(&[WFE, NOP, SPIN]);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Sleeping);
    assert_eq!(core.sleep_reason(), Some(SleepReason::WaitForEvent));
    assert_eq!(core.pc(), 0x102);
    core.signal_event();
    // 唤醒发生在下一步之内：该步立刻执行 WFE 之后的那条指令（0 步唤醒延迟）。
    // The wake happens inside the next step, which immediately executes the
    // instruction after WFE (zero-step wake latency).
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x104, "the post-WFE NOP ran once");
    assert!(!core.event_register(), "the wake consumes the event");
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x104, "the NOP is not replayed");
}

#[test]
fn wfe_continues_when_the_event_register_is_set() {
    let (mut core, mut bus, _mailbox) = sleep_bus(&[SEV, WFE, NOP, SPIN]);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert!(core.event_register());
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(
        core.sleep_reason(),
        None,
        "WFE clears the event and runs on"
    );
    assert_eq!(core.pc(), 0x104);
    assert!(!core.event_register());
}

#[test]
fn sev_reports_a_broadcast_event_and_sevl_stays_local() {
    let (mut core, mut bus, _mailbox) = sleep_bus(&[SEV, 0xBF50, SPIN]);
    core.step(&mut bus).unwrap();
    assert_eq!(
        core.take_events_sent(),
        1,
        "SEV must reach the peer core / SEV 必须送达对端核心"
    );
    core.step(&mut bus).unwrap();
    assert_eq!(
        core.take_events_sent(),
        0,
        "SEVL is local only / SEVL 只作用于本核"
    );
}

#[test]
fn exception_entry_sets_the_event_register() {
    let (mut core, mut bus, _mailbox) = sleep_bus(&[NOP, NOP, SPIN]);
    raise_irq5(&mut core);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert!(core.active_exceptions.contains(&21));
    assert!(
        core.event_register(),
        "exception entry is an event source / 异常进入是事件源"
    );
}

// ---- blocked load --------------------------------------------------------

#[test]
fn blocked_load_keeps_its_registers_and_pc() {
    let (mut core, mut bus, mailbox) = sleep_bus(&[LDR_R0_R1, ADDS_R0_1, SPIN]);
    core.write_register(1, MAILBOX_BASE + MAILBOX_RD);
    core.write_register(0, 0x1111_1111);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Sleeping);
    assert_eq!(core.sleep_reason(), Some(SleepReason::BlockedRead));
    assert_eq!(core.pc(), 0x100, "the load must be retried, not skipped");
    assert_eq!(core.read_register(0), 0x1111_1111, "destination untouched");
    assert_eq!(core.read_register(1), MAILBOX_BASE + MAILBOX_RD);
    assert_eq!(
        mailbox.reads(),
        0,
        "the stalled read never reached the device"
    );
    assert!(mailbox.is_empty());
}

#[test]
fn blocked_load_completes_once_data_arrives() {
    let (mut core, mut bus, mailbox) = sleep_bus(&[LDR_R0_R1, ADDS_R0_1, SPIN]);
    core.write_register(1, MAILBOX_BASE + MAILBOX_RD);
    core.step(&mut bus).unwrap();
    assert_eq!(core.state, CpuState::Sleeping);
    mailbox.push(0xCAFE_F00D);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.read_register(0), 0xCAFE_F00D);
    assert_eq!(core.pc(), 0x102, "the retry retired exactly once");
    assert!(mailbox.is_empty(), "exactly one frame consumed");
    assert_eq!(core.sleep_reason(), None);
}

#[test]
fn blocked_load_never_double_consumes_after_a_wake() {
    let (mut core, mut bus, mailbox) = sleep_bus(&[LDR_R0_R1, ADDS_R0_1, SPIN]);
    core.write_register(1, MAILBOX_BASE + MAILBOX_RD);
    core.step(&mut bus).unwrap();
    mailbox.push(1);
    mailbox.push(2);
    core.step(&mut bus).unwrap();
    assert_eq!(core.read_register(0), 1);
    // 后续执行 ADDS，不重复读取 FIFO。 / The next step runs ADDS, no second pop.
    core.step(&mut bus).unwrap();
    assert_eq!(core.read_register(0), 2);
    assert_eq!(core.pc(), 0x104);
}

#[test]
fn blocked_load_wake_takes_a_pending_interrupt_first() {
    let (mut core, mut bus, mailbox) = sleep_bus(&[LDR_R0_R1, ADDS_R0_1, SPIN]);
    core.write_register(1, MAILBOX_BASE + MAILBOX_RD);
    core.step(&mut bus).unwrap();
    assert_eq!(core.state, CpuState::Sleeping);
    raise_irq5(&mut core);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), 0x300, "the ISR runs before the retried load");
    assert_eq!(core.read_register(0), 0);
    assert!(mailbox.is_empty());
}
