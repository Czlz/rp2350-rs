//! RISC-V sleep, event and blocked-access tests / RISC-V 睡眠、事件与阻塞访问测试
//!
//! Hazard3 has no `WFE`, but the pico-sdk maps `__wfe()` to `wfi` for RISC-V, so
//! the inter-core mailbox handshake needs both interrupt wake-up and a
//! host-signalled event. These tests pin that contract together with the retry
//! rule for a load that hits a blocking device.
//! Hazard3 没有 `WFE`，但 pico-sdk 在 RISC-V 上把 `__wfe()` 映射为 `wfi`，因此核间
//! 信箱握手既需要中断唤醒也需要宿主事件。本模块锁定该契约与命中阻塞设备的载入的重试规则。

#![allow(clippy::similar_names)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use super::*;
use rp2350_core::memory::{RamRegion, SimpleBus};
use rp2350_cpu::CpuCore;

/// Code base used by these tests / 测试使用的代码基址
const CODE_BASE: u32 = 0x2000_0000;

/// Stack top used by these tests / 测试使用的栈顶
const STACK_TOP: u32 = 0x2000_0F00;

/// `wfi` / `wfi`
const WFI: u32 = 0x1050_0073;

/// `addi a0, a0, 1` / `addi a0, a0, 1`
const ADDI_A0_1: u32 = 0x0015_0513;

/// `jal x0, 0` (jump to self) / `jal x0, 0`（跳转到自身）
const JUMP_SELF: u32 = 0x0000_006F;

/// `lw a0, 0(a1)` / `lw a0, 0(a1)`
const LW_A0_0_A1: u32 = 0x0005_A503;

/// Base of the fake mailbox region / 伪信箱区域基址
const MAILBOX_BASE: u32 = 0xD000_0000;

/// Offset of the blocking read register / 阻塞读寄存器偏移
const MAILBOX_RD: u32 = 0x058;

/// Trap handler address installed in `mtvec` / 写入 `mtvec` 的陷阱处理地址
const HANDLER: u32 = 0x2000_0800;

/// Interrupt number used for the wake tests / 唤醒测试使用的中断号
const IRQ: u16 = 3;

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

/// Build a core executing `code` with the mailbox mounted
/// 构建执行 `code` 且挂载信箱的核心
fn core_with_mailbox(code: &[u32]) -> (Hazard3Core, SimpleBus, Mailbox) {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("sram", CODE_BASE, 0x1000)))
        .unwrap();
    let mailbox = Mailbox::new();
    bus.register_region(Box::new(mailbox.clone())).unwrap();
    let mut cycles = 0u64;
    for (index, word) in code.iter().enumerate() {
        let address = CODE_BASE + u32::try_from(index).unwrap() * 4;
        bus.write_u32(address, *word, &mut cycles).unwrap();
    }
    let mut core = Hazard3Core::new(CpuId::Core0);
    core.set_boot_vectors(STACK_TOP, CODE_BASE);
    core.init(&mut bus).unwrap();
    core.csr.mtvec = HANDLER;
    (core, bus, mailbox)
}

/// Enable `mstatus.MIE`, `mie.MEIE` and one Xh3irq source
/// 使能 `mstatus.MIE`、`mie.MEIE` 与一个 Xh3irq 源
fn enable_external_interrupt(core: &mut Hazard3Core) {
    core.csr.mstatus |= csr::MSTATUS_MIE;
    core.csr.mie |= csr::MIE_MEIE;
    let window = usize::from(IRQ) / 16;
    core.csr.xh3irq.enable[window] |= 1u16 << (IRQ % 16);
    core.csr.refresh_xh3irq();
}

// ---- WFI -----------------------------------------------------------------

#[test]
fn wfi_sleeps_without_a_pending_interrupt() {
    let (mut core, mut bus, _mailbox) = core_with_mailbox(&[WFI, ADDI_A0_1, JUMP_SELF]);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Sleeping);
    assert_eq!(core.sleep_reason(), Some(SleepReason::WaitForInterrupt));
    assert_eq!(core.pc(), CODE_BASE + 4, "wfi retires before the sleep");
    let before = core.total_cycles();
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Sleeping);
    assert_eq!(core.pc(), CODE_BASE + 4);
    assert!(core.total_cycles() > before, "time keeps advancing");
}

#[test]
fn wfi_does_not_sleep_with_a_runnable_interrupt() {
    let (mut core, mut bus, _mailbox) = core_with_mailbox(&[WFI, ADDI_A0_1, JUMP_SELF]);
    enable_external_interrupt(&mut core);
    core.signal_interrupt(IRQ);
    assert!(core.pending_interrupt().is_some());
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.sleep_reason(), None, "wfi must not suspend here");
    // 同一步内已进入中断处理器。 / The handler was entered in the same step.
    assert_eq!(core.pc(), HANDLER);
    assert_eq!(core.csr.mcause & 0x8000_0000, 0x8000_0000);
}

#[test]
fn wfi_wakes_on_an_external_interrupt() {
    let (mut core, mut bus, _mailbox) = core_with_mailbox(&[WFI, ADDI_A0_1, JUMP_SELF]);
    enable_external_interrupt(&mut core);
    core.step(&mut bus).unwrap();
    assert_eq!(core.state, CpuState::Sleeping);
    core.signal_interrupt(IRQ);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), HANDLER);
    assert_eq!(core.sleep_reason(), None);
}

#[test]
fn wfi_wakes_on_a_host_event() {
    // FIFO 流量以宿主事件唤醒睡眠核，使 `__wfe()`→`wfi` 的信箱握手不死锁。
    // FIFO traffic wakes a sleeping core through the host event, so the
    // `__wfe()`→`wfi` mailbox handshake cannot deadlock.
    let (mut core, mut bus, _mailbox) = core_with_mailbox(&[WFI, ADDI_A0_1, JUMP_SELF]);
    core.step(&mut bus).unwrap();
    assert_eq!(core.state, CpuState::Sleeping);
    core.signal_event();
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(
        core.pc(),
        CODE_BASE + 8,
        "the post-wfi instruction ran once"
    );
    assert_eq!(core.read_register(10), 1);
    assert!(!core.event_pending(), "the wake consumes the event");
}

// ---- blocked load --------------------------------------------------------

#[test]
fn blocked_load_keeps_pc_and_destination() {
    let (mut core, mut bus, mailbox) = core_with_mailbox(&[LW_A0_0_A1, ADDI_A0_1, JUMP_SELF]);
    core.write_register(11, MAILBOX_BASE + MAILBOX_RD);
    core.write_register(10, 0x1111_1111);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Sleeping);
    assert_eq!(core.sleep_reason(), Some(SleepReason::BlockedRead));
    assert_eq!(core.pc(), CODE_BASE, "the load must be retried");
    assert_eq!(core.read_register(10), 0x1111_1111, "a0 untouched");
    assert_eq!(
        mailbox.reads(),
        0,
        "the stalled read never reached the device"
    );
    assert!(
        core.csr.mcause == 0,
        "a stall must never raise a guest exception / stall 绝不产生客户机异常"
    );
}

#[test]
fn blocked_load_retries_and_consumes_once() {
    let (mut core, mut bus, mailbox) = core_with_mailbox(&[LW_A0_0_A1, ADDI_A0_1, JUMP_SELF]);
    core.write_register(11, MAILBOX_BASE + MAILBOX_RD);
    core.step(&mut bus).unwrap();
    assert_eq!(core.state, CpuState::Sleeping);
    mailbox.push(0xC0DE_1234);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.read_register(10), 0xC0DE_1234);
    assert_eq!(core.pc(), CODE_BASE + 4, "the retry retired exactly once");
    assert!(mailbox.is_empty(), "exactly one frame consumed");
    assert_eq!(core.sleep_reason(), None);
}

#[test]
fn blocked_load_keeps_the_guest_visible_state_clean() {
    // 连续两次睡眠重试也不得改写寄存器或产生异常。
    // Two consecutive sleep retries must not modify registers or raise a fault.
    let (mut core, mut bus, mailbox) = core_with_mailbox(&[LW_A0_0_A1, ADDI_A0_1, JUMP_SELF]);
    core.write_register(11, MAILBOX_BASE + MAILBOX_RD);
    for _ in 0..3 {
        assert_eq!(core.step(&mut bus).unwrap(), CpuState::Sleeping);
    }
    assert_eq!(core.pc(), CODE_BASE);
    assert_eq!(core.read_register(10), 0);
    assert_eq!(core.csr.mcause, 0);
    assert!(mailbox.is_empty());
}
