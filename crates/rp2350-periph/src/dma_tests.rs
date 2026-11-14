//! DMA controller tests / DMA 控制器测试
//!
//! Every test mounts the DMA register block next to a RAM region on a
//! [`SimpleBus`], programs channels through the real MMIO registers and drives
//! transfers with [`Dma::run`].
//! 每个测试都把 DMA 寄存器块与 RAM 区域一起挂到 [`SimpleBus`] 上，通过真实
//! MMIO 寄存器配置通道，并用 [`Dma::run`] 驱动传输。

use rp2350_core::bus::AddressBus;
use rp2350_core::memory::{RamRegion, SimpleBus};
use rp2350_core::mmio::{MmioDevice, mount_device};

use crate::dma::{
    CTRL_BUSY, CTRL_CHAIN_TO_SHIFT, CTRL_DATA_SIZE_SHIFT, CTRL_EN, CTRL_INCR_READ, CTRL_INCR_WRITE,
    CTRL_IRQ_QUIET, CTRL_RING_SEL, CTRL_RING_SIZE_SHIFT, CTRL_TREQ_SEL_SHIFT, Dma, DmaChannelState,
    TREQ_PERMANENT,
};
use crate::irq::IrqAssert;
use crate::map;

/// Base address of the test RAM region / 测试 RAM 区域基地址
const RAM_BASE: u32 = 0x2000_0000;

/// Size of the test RAM region / 测试 RAM 区域大小
const RAM_SIZE: usize = 0x2000;

/// Source address used by most tests / 多数测试使用的源地址
const SRC: u32 = RAM_BASE;

/// Destination address used by most tests / 多数测试使用的目标地址
const DST: u32 = RAM_BASE + 0x1000;

/// `INTR` offset / `INTR` 偏移
const INTR: u32 = 0x400;

/// `INTE0` offset / `INTE0` 偏移
const INTE0: u32 = 0x404;

/// `INTF0` offset / `INTF0` 偏移
const INTF0: u32 = 0x408;

/// `INTS0` offset / `INTS0` 偏移
const INTS0: u32 = 0x40C;

/// `INTE1` offset / `INTE1` 偏移
const INTE1: u32 = 0x414;

/// `INTS1` offset / `INTS1` 偏移
const INTS1: u32 = 0x41C;

/// `MULTI_CHAN_TRIGGER` offset / `MULTI_CHAN_TRIGGER` 偏移
const MULTI_CHAN_TRIGGER: u32 = 0x450;

/// `CHAN_ABORT` offset / `CHAN_ABORT` 偏移
const CHAN_ABORT: u32 = 0x464;

/// Per-channel register stride / 每通道寄存器步长
const STRIDE: u32 = 0x40;

/// Test fixture: bus, DMA handle and shared interrupt set
/// 测试夹具：总线、DMA 句柄与共享中断集合
struct Fixture {
    bus: SimpleBus,
    dma: Dma,
    irq: IrqAssert,
}

impl Fixture {
    /// Write one word to the bus / 向总线写一个字
    fn write_word(&mut self, address: u32, value: u32) {
        let mut cycles = 0u64;
        self.bus
            .write_u32(address, value, &mut cycles)
            .expect("word write");
    }

    /// Read one word from the bus / 从总线读一个字
    fn read_word(&mut self, address: u32) -> u32 {
        let mut cycles = 0u64;
        self.bus.read_u32(address, &mut cycles).expect("word read")
    }

    /// Write one halfword to the bus / 向总线写一个半字
    fn write_half(&mut self, address: u32, value: u16) {
        let mut cycles = 0u64;
        self.bus
            .write_u16(address, value, &mut cycles)
            .expect("half write");
    }

    /// Read one halfword from the bus / 从总线读一个半字
    fn read_half(&mut self, address: u32) -> u16 {
        let mut cycles = 0u64;
        self.bus.read_u16(address, &mut cycles).expect("half read")
    }

    /// Write a DMA register by window offset / 按窗口偏移写 DMA 寄存器
    fn reg_write(&mut self, offset: u32, value: u32) {
        self.write_word(map::DMA_BASE + offset, value);
    }

    /// Read a DMA register by window offset / 按窗口偏移读 DMA 寄存器
    fn reg_read(&mut self, offset: u32) -> u32 {
        self.read_word(map::DMA_BASE + offset)
    }

    /// Run the controller with a transfer budget / 按传输预算执行控制器
    fn run(&mut self, max_transfers: u64) -> u64 {
        self.dma.run(&mut self.bus, max_transfers)
    }

    /// Snapshot one channel / 获取一个通道的快照
    fn channel(&self, index: usize) -> DmaChannelState {
        self.dma.channel_state(index).expect("channel exists")
    }
}

/// Create a fixture with RAM and the DMA block mounted / 创建挂载 RAM 与 DMA 的夹具
fn fixture() -> Fixture {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("ram", RAM_BASE, RAM_SIZE)))
        .expect("ram region");
    let irq = IrqAssert::new();
    let dma = Dma::new(irq.clone());
    mount_device(&mut bus, Box::new(dma.clone())).expect("dma window");
    Fixture { bus, dma, irq }
}

/// Write a byte slice into RAM / 向 RAM 写入字节切片
fn write_bytes(fixture: &mut Fixture, address: u32, bytes: &[u8]) {
    let mut cycles = 0u64;
    for (offset, byte) in bytes.iter().enumerate() {
        fixture
            .bus
            .write_u8(address + offset as u32, *byte, &mut cycles)
            .expect("byte write");
    }
}

/// Read a byte slice from RAM / 从 RAM 读取字节切片
fn read_bytes(fixture: &mut Fixture, address: u32, len: usize) -> Vec<u8> {
    let mut cycles = 0u64;
    let mut bytes = Vec::with_capacity(len);
    for offset in 0..len {
        bytes.push(
            fixture
                .bus
                .read_u8(address + offset as u32, &mut cycles)
                .expect("byte read"),
        );
    }
    bytes
}

/// Build a `CTRL_TRIG` word for a channel / 为通道构造 `CTRL_TRIG` 字
fn ctrl_word(channel: usize, data_size: u32, treq: u8) -> u32 {
    CTRL_EN
        | CTRL_INCR_READ
        | CTRL_INCR_WRITE
        | (data_size << CTRL_DATA_SIZE_SHIFT)
        | (u32::from(treq) << CTRL_TREQ_SEL_SHIFT)
        | ((channel as u32) << CTRL_CHAIN_TO_SHIFT)
}

/// Program addresses, count and control, then trigger the channel
/// 编程地址、计数与控制寄存器，然后触发通道
fn configure(fixture: &mut Fixture, channel: usize, read: u32, write: u32, count: u32, ctrl: u32) {
    let base = channel as u32 * STRIDE;
    fixture.reg_write(base, read);
    fixture.reg_write(base + 0x04, write);
    fixture.reg_write(base + 0x08, count);
    fixture.reg_write(base + 0x0C, ctrl);
}

#[test]
fn byte_copy_increments_both_addresses() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[1, 2, 3, 4, 5]);
    let ctrl = ctrl_word(0, 0, TREQ_PERMANENT);
    configure(&mut f, 0, SRC, DST, 5, ctrl);
    assert_eq!(f.reg_read(0x0C) & CTRL_BUSY, CTRL_BUSY);
    assert_eq!(f.run(5), 5);
    assert_eq!(read_bytes(&mut f, DST, 5), vec![1u8, 2, 3, 4, 5]);
    let state = f.channel(0);
    assert_eq!(state.read_addr, SRC + 5);
    assert_eq!(state.write_addr, DST + 5);
    assert_eq!(state.trans_count, 0);
    assert!(!state.busy);
    assert!(state.enabled);
    assert_eq!(f.dma.busy_mask(), 0);
}

#[test]
fn halfword_copy_moves_two_bytes_per_step() {
    let mut f = fixture();
    f.write_half(SRC, 0xBEEF);
    f.write_half(SRC + 2, 0x1234);
    let ctrl = ctrl_word(0, 1, TREQ_PERMANENT);
    configure(&mut f, 0, SRC, DST, 2, ctrl);
    assert_eq!(f.run(2), 2);
    assert_eq!(f.read_half(DST), 0xBEEF);
    assert_eq!(f.read_half(DST + 2), 0x1234);
    assert_eq!(f.channel(0).read_addr, SRC + 4);
    assert_eq!(f.channel(0).write_addr, DST + 4);
}

#[test]
fn word_copy_moves_four_bytes_per_step() {
    let mut f = fixture();
    f.write_word(SRC, 0x1122_3344);
    f.write_word(SRC + 4, 0x5566_7788);
    f.write_word(SRC + 8, 0x99AA_BBCC);
    let ctrl = ctrl_word(0, 2, TREQ_PERMANENT);
    configure(&mut f, 0, SRC, DST, 3, ctrl);
    assert_eq!(f.run(3), 3);
    assert_eq!(f.read_word(DST), 0x1122_3344);
    assert_eq!(f.read_word(DST + 4), 0x5566_7788);
    assert_eq!(f.read_word(DST + 8), 0x99AA_BBCC);
    assert_eq!(f.channel(0).read_addr, SRC + 12);
}

#[test]
fn trans_count_decrements_and_busy_clears() {
    let mut f = fixture();
    configure(&mut f, 0, SRC, DST, 5, ctrl_word(0, 0, TREQ_PERMANENT));
    assert_eq!(f.run(2), 2);
    let partial = f.channel(0);
    assert_eq!(partial.trans_count, 3);
    assert!(partial.busy);
    assert_eq!(f.run(3), 3);
    let done = f.channel(0);
    assert_eq!(done.trans_count, 0);
    assert!(!done.busy);
    assert_eq!(f.reg_read(0x0C) & CTRL_BUSY, 0);
}

#[test]
fn clearing_en_stops_the_channel() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[9, 9, 9]);
    configure(&mut f, 0, SRC, DST, 3, ctrl_word(0, 0, TREQ_PERMANENT));
    f.reg_write(0x0C, 0);
    let state = f.channel(0);
    assert!(!state.enabled);
    assert!(!state.busy);
    assert_eq!(f.run(3), 0);
    assert_eq!(read_bytes(&mut f, DST, 3), vec![0u8, 0, 0]);
}

#[test]
fn chain_to_starts_the_next_channel_and_raises_both_irqs() {
    let mut f = fixture();
    write_bytes(
        &mut f,
        SRC,
        &[0xA0, 0xA1, 0xA2, 0xA3, 0xB0, 0xB1, 0xB2, 0xB3],
    );
    let chained = ctrl_word(0, 0, TREQ_PERMANENT) | (1 << CTRL_CHAIN_TO_SHIFT);
    configure(&mut f, 0, SRC, DST, 4, chained);
    configure(
        &mut f,
        1,
        SRC + 4,
        DST + 4,
        4,
        ctrl_word(1, 0, TREQ_PERMANENT),
    );
    assert_eq!(f.run(8), 8);
    assert_eq!(f.channel(0).trans_count, 0);
    assert_eq!(f.channel(1).trans_count, 0);
    assert_eq!(f.reg_read(INTR) & 0x3, 0x3);
    assert_eq!(
        read_bytes(&mut f, DST, 8),
        vec![0xA0u8, 0xA1, 0xA2, 0xA3, 0xB0, 0xB1, 0xB2, 0xB3]
    );
}

#[test]
fn ints_inte_and_intf_follow_datasheet_semantics() {
    let mut f = fixture();
    configure(&mut f, 0, SRC, DST, 1, ctrl_word(0, 0, TREQ_PERMANENT));
    f.reg_write(INTE0, 1);
    assert_eq!(f.run(1), 1);
    assert_eq!(f.reg_read(INTR), 1);
    assert_eq!(f.reg_read(INTS0), 1);
    let line = 1u64 << map::DMA_IRQ_0;
    assert_eq!(f.irq.mask() & line, line);

    // INTF forces the status bit into INTS without touching INTR.
    // INTF 将状态位强制注入 INTS，但不改变 INTR。
    f.reg_write(INTF0, 1 << 1);
    assert_eq!(f.reg_read(INTS0), 0x3);

    // Writing INTR clears the raw bit; the forced bit remains.
    // 写 INTR 清除原始位；强制位保留。
    f.reg_write(INTR, 1);
    assert_eq!(f.reg_read(INTR), 0);
    assert_eq!(f.reg_read(INTS0), 0x2);
    assert_eq!(f.irq.mask() & line, line);

    f.reg_write(INTF0, 0);
    assert_eq!(f.reg_read(INTS0), 0);
    assert_eq!(f.irq.mask() & line, 0);
}

#[test]
fn second_irq_line_is_independently_enabled() {
    let mut f = fixture();
    configure(&mut f, 2, SRC, DST, 1, ctrl_word(2, 0, TREQ_PERMANENT));
    f.reg_write(INTE1, 1 << 2);
    assert_eq!(f.run(1), 1);
    assert_eq!(f.reg_read(INTS1), 1 << 2);
    let line = 1u64 << map::DMA_IRQ_1;
    assert_eq!(f.irq.mask() & line, line);
    f.reg_write(INTR, 1 << 2);
    assert_eq!(f.irq.mask() & line, 0);
}

#[test]
fn permanent_request_ignores_host_dreq_level() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[4, 5, 6]);
    f.dma.set_dreq(TREQ_PERMANENT, false);
    assert_eq!(f.dma.dreq(TREQ_PERMANENT), Some(false));
    configure(&mut f, 0, SRC, DST, 3, ctrl_word(0, 0, TREQ_PERMANENT));
    assert_eq!(f.run(3), 3);
    assert_eq!(read_bytes(&mut f, DST, 3), vec![4u8, 5, 6]);
}

#[test]
fn gated_dreq_pauses_and_resumes_transfers() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[7, 8, 9]);
    f.dma.set_dreq(5, false);
    configure(&mut f, 0, SRC, DST, 3, ctrl_word(0, 0, 5));
    assert_eq!(f.run(3), 0);
    assert!(f.channel(0).busy);
    assert_eq!(f.dma.busy_mask(), 1);

    f.dma.set_dreq(5, true);
    assert_eq!(f.run(3), 3);
    assert_eq!(read_bytes(&mut f, DST, 3), vec![7u8, 8, 9]);
}

#[test]
fn chan_abort_clears_busy_and_keeps_the_remaining_count() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[1; 8]);
    configure(&mut f, 0, SRC, DST, 8, ctrl_word(0, 0, TREQ_PERMANENT));
    assert_eq!(f.run(2), 2);
    f.reg_write(CHAN_ABORT, 1);
    let state = f.channel(0);
    assert!(!state.busy);
    assert_eq!(state.trans_count, 6);
    assert!(state.enabled);
    assert_eq!(f.run(8), 0);
}

#[test]
fn invalid_indices_and_reserved_offsets_do_not_panic() {
    let mut f = fixture();
    assert!(f.dma.channel_state(16).is_none());
    assert!(f.dma.channel_state(usize::MAX).is_none());
    f.dma.set_dreq(200, false);
    assert_eq!(f.dma.dreq(64), None);
    f.dma.abort(0xFFFF);
    // 0x470 是未建模的全局偏移：读零、写入被忽略。0x470 is unmodelled.
    f.reg_write(0x470, 0xDEAD_BEEF);
    assert_eq!(f.reg_read(0x470), 0);
    assert_eq!(f.reg_read(0x468), 16);
}

#[test]
fn channels_are_independent() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[1, 2]);
    write_bytes(&mut f, SRC + 0x100, &[3, 4]);
    f.dma.set_dreq(5, false);
    configure(&mut f, 0, SRC, DST, 2, ctrl_word(0, 0, 5));
    configure(
        &mut f,
        1,
        SRC + 0x100,
        DST + 0x100,
        2,
        ctrl_word(1, 0, TREQ_PERMANENT),
    );
    // 通道 0 被 DREQ 门控，通道 1 应独立完成。Channel 0 is gated; channel 1 runs.
    assert_eq!(f.run(4), 2);
    assert_eq!(f.dma.busy_mask(), 1);
    assert_eq!(f.channel(1).trans_count, 0);
    assert_eq!(read_bytes(&mut f, DST + 0x100, 2), vec![3u8, 4]);

    f.dma.set_dreq(5, true);
    assert_eq!(f.run(2), 2);
    assert_eq!(f.dma.busy_mask(), 0);
}

#[test]
fn run_respects_the_max_transfers_budget() {
    let mut f = fixture();
    configure(&mut f, 0, SRC, DST, 10, ctrl_word(0, 0, TREQ_PERMANENT));
    assert_eq!(f.run(3), 3);
    assert_eq!(f.channel(0).trans_count, 7);
    assert!(f.channel(0).busy);
    assert_eq!(f.run(100), 7);
    assert_eq!(f.channel(0).trans_count, 0);
    assert_eq!(f.run(100), 0);
}

#[test]
fn ring_write_wraps_within_the_ring_window() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[10, 11, 12, 13]);
    let ctrl = ctrl_word(0, 0, TREQ_PERMANENT) | (2 << CTRL_RING_SIZE_SHIFT) | CTRL_RING_SEL;
    configure(&mut f, 0, SRC, DST + 2, 4, ctrl);
    assert_eq!(f.run(4), 4);
    assert_eq!(read_bytes(&mut f, DST, 4), vec![12u8, 13, 10, 11]);
}

#[test]
fn al1_ctrl_alias_triggers_the_channel() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[1, 2]);
    f.reg_write(0x00, SRC);
    f.reg_write(0x04, DST);
    f.reg_write(0x08, 2);
    f.reg_write(0x10, ctrl_word(0, 0, TREQ_PERMANENT));
    assert!(f.channel(0).busy);
    assert_eq!(f.run(2), 2);
    assert_eq!(read_bytes(&mut f, DST, 2), vec![1u8, 2]);
}

#[test]
fn irq_quiet_suppresses_the_completion_flag() {
    let mut f = fixture();
    let ctrl = ctrl_word(0, 0, TREQ_PERMANENT) | CTRL_IRQ_QUIET;
    configure(&mut f, 0, SRC, DST, 1, ctrl);
    assert_eq!(f.run(1), 1);
    assert_eq!(f.reg_read(INTR), 0);
}

#[test]
fn multi_channel_trigger_starts_all_selected_channels() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[1]);
    write_bytes(&mut f, SRC + 0x100, &[2]);
    let base0 = 0;
    let base1 = STRIDE;
    for (base, read, write) in [(base0, SRC, DST), (base1, SRC + 0x100, DST + 0x100)] {
        f.reg_write(base, read);
        f.reg_write(base + 0x04, write);
        f.reg_write(base + 0x08, 1);
        f.reg_write(
            base + 0x0C,
            ctrl_word((base / STRIDE) as usize, 0, TREQ_PERMANENT),
        );
    }
    f.reg_write(MULTI_CHAN_TRIGGER, 0x3);
    assert_eq!(f.dma.busy_mask(), 0x3);
    assert_eq!(f.run(2), 2);
    assert_eq!(read_bytes(&mut f, DST, 1), vec![1u8]);
    assert_eq!(read_bytes(&mut f, DST + 0x100, 1), vec![2u8]);
}

#[test]
fn dma_irq_numbers_match_the_interrupt_map() {
    assert_eq!(map::DMA_BASE, 0x5000_0000);
    assert_eq!(map::DMA_SIZE, 0x4000);
    assert_eq!(map::DMA_IRQ_0, 10);
    assert_eq!(map::DMA_IRQ_1, map::DMA_IRQ_0 + 1);
    assert_eq!(map::DMA_IRQ_2, map::DMA_IRQ_0 + 2);
    assert_eq!(map::DMA_IRQ_3, map::DMA_IRQ_0 + 3);
}

#[test]
fn set_and_clear_aliases_update_inte1() {
    let mut fixture = fixture();
    fixture.reg_write(INTE1 + 0x2000, 1 << 3);
    assert_eq!(fixture.reg_read(INTE1), 1 << 3);
    fixture.reg_write(INTE1 + 0x3000, 1 << 3);
    assert_eq!(fixture.reg_read(INTE1), 0);
}

#[test]
fn xor_alias_updates_a_channel_register() {
    let mut fixture = fixture();
    fixture.reg_write(0x0000, 0x1000);
    fixture.reg_write(0x1000, 0x000F);
    assert_eq!(fixture.reg_read(0x0000), 0x100F);
}

#[test]
fn alias_window_is_sixteen_kilobytes() {
    let fixture = fixture();
    assert_eq!(fixture.dma.window().size, map::DMA_SIZE);
    assert_eq!(map::DMA_SIZE, 0x4000);
}
