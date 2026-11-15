//! DMA controller tests / DMA 控制器测试
//!
//! Every test mounts the DMA register block next to a RAM region on a
//! [`SimpleBus`], programs channels through the real MMIO registers and drives
//! transfers with [`Dma::run`].
//! 每个测试都把 DMA 寄存器块与 RAM 区域一起挂到 [`SimpleBus`] 上，通过真实
//! MMIO 寄存器配置通道，并用 [`Dma::run`] 驱动传输。

use rp2350_core::bus::AddressBus;
use rp2350_core::memory::{RamRegion, SimpleBus};
use rp2350_core::mmio::{MmioDevice, MmioWindow, mount_device};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::dma::{
    CTRL_BSWAP, CTRL_BUSY, CTRL_CHAIN_TO_SHIFT, CTRL_DATA_SIZE_SHIFT, CTRL_EN, CTRL_INCR_READ,
    CTRL_INCR_WRITE, CTRL_IRQ_QUIET, CTRL_RING_SEL, CTRL_RING_SIZE_SHIFT, CTRL_SNIFF_EN,
    CTRL_TREQ_SEL_SHIFT, Dma, DmaChannelState, SNIFF_BSWAP, SNIFF_CALC_CRC16R, SNIFF_CALC_CRC32R,
    SNIFF_CALC_SHIFT, SNIFF_CALC_SUM, SNIFF_CALC_XOR, SNIFF_DMACH_SHIFT, SNIFF_EN, TREQ_PERMANENT,
};
use crate::dreq;
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

/// `AL1_CTRL` offset — non-triggering control write / `AL1_CTRL` 偏移：非触发控制写入
const CH_AL1_CTRL_OFFSET: u32 = 0x10;

/// `AL2_CTRL` offset — non-triggering control write / `AL2_CTRL` 偏移：非触发控制写入
const CH_AL2_CTRL_OFFSET: u32 = 0x20;

/// `AL3_CTRL` offset — non-triggering control write / `AL3_CTRL` 偏移：非触发控制写入
const CH_AL3_CTRL_OFFSET: u32 = 0x30;

/// `AL3_TRANS_COUNT` offset — rewrite value only / `AL3_TRANS_COUNT` 偏移：仅重载值
const CH_AL3_TRANS_COUNT_OFFSET: u32 = 0x38;

/// `AL3_READ_ADDR_TRIG` offset — write starts the channel / `AL3_READ_ADDR_TRIG` 偏移：写入启动通道
const CH_AL3_READ_ADDR_TRIG_OFFSET: u32 = 0x3C;

/// `CTRL_TRIG` offset / `CTRL_TRIG` 偏移
const CH_CTRL_TRIG_OFFSET: u32 = 0x0C;

/// `TRANS_COUNT` offset / `TRANS_COUNT` 偏移
const CH_TRANS_COUNT_OFFSET: u32 = 0x08;

/// Read address offset / 读地址偏移
const CH_READ_ADDR_OFFSET: u32 = 0x00;

/// Write address offset / 写地址偏移
const CH_WRITE_ADDR_OFFSET: u32 = 0x04;

/// `CTRL_TRIG.DATA_SIZE` = word / `CTRL_TRIG.DATA_SIZE` = 字
const DATA_SIZE_WORD: u32 = 2;

/// Address window of the stand-in HSTX FIFO / 替代 HSTX FIFO 的地址窗口
const SINK_BASE: u32 = 0x4008_0000;

/// 32-bit sink standing in for the HSTX FIFO data register
/// 替代 HSTX FIFO 数据寄存器的 32 位接收器
///
/// The scanout DMA writes one word per data unit to a fixed `WRITE_ADDR`;
/// recording every write is enough to prove that the ring keeps delivering.
/// 扫描 DMA 每个数据单元向固定 `WRITE_ADDR` 写一个字；记录每次写入即可证明环
/// 一直在投递数据。
struct WordSink {
    window: MmioWindow,
    words: Arc<Mutex<Vec<u32>>>,
}

impl MmioDevice for WordSink {
    fn name(&self) -> &str {
        "test-word-sink"
    }

    fn window(&self) -> MmioWindow {
        self.window
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        (offset == 0).then_some(0)
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        if offset != 0 {
            return false;
        }
        self.words.lock().push(value);
        true
    }
}

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
    fixture.reg_write(base + CH_TRANS_COUNT_OFFSET, count);
    fixture.reg_write(base + CH_CTRL_TRIG_OFFSET, ctrl);
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
fn enabled_selects_dreq_ignores_the_busy_flag() {
    // A freshly triggered channel is enabled before `Dma::run` marks it busy;
    // the host must still resynchronise it per unit, otherwise a batch runs with
    // a stale DREQ level and overruns a small FIFO (the HSTX FIFO is 8 words).
    // 刚被触发的通道先置 EN、之后才由 `Dma::run` 置 busy；宿主仍必须逐单元
    // 重新同步，否则整批会用陈旧的 DREQ 电平执行并冲垮小 FIFO（HSTX 仅 8 字）。
    let mut f = fixture();
    f.dma.set_dreq(52, true);
    configure(&mut f, 0, SRC, DST, 3, ctrl_word(0, 0, 52));
    assert!(f.channel(0).busy, "configure() triggers the channel");
    assert!(f.dma.enabled_selects_dreq(52, 52));
    assert_eq!(f.run(3), 3);

    // Idle but enabled: still reported, unlike `busy_selects_dreq`.
    // 空闲但仍使能：仍会上报，与 `busy_selects_dreq` 不同。
    let mut idle = fixture();
    idle.reg_write(0x10, ctrl_word(0, 0, 52));
    assert!(idle.dma.enabled_selects_dreq(52, 52));
    assert!(!idle.dma.busy_selects_dreq(52, 52));

    // Disabled and permanent selectors are never reported.
    // 未使能与永久请求选择器都不上报。
    let mut disabled = fixture();
    disabled.reg_write(0x10, 0);
    assert!(!disabled.dma.enabled_selects_dreq(52, 52));
    configure(
        &mut disabled,
        0,
        SRC,
        DST,
        3,
        ctrl_word(0, 0, TREQ_PERMANENT),
    );
    assert!(
        !disabled
            .dma
            .enabled_selects_dreq(TREQ_PERMANENT, TREQ_PERMANENT)
    );
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
fn al1_ctrl_alias_configures_without_triggering() {
    // Pico SDK 的 dma_channel_configure(..., false) 写 al1_ctrl 且期望不启动。
    // The Pico SDK writes al1_ctrl from dma_channel_configure(..., false) and
    // expects the channel *not* to start.
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[1, 2]);
    f.reg_write(0x00, SRC);
    f.reg_write(0x04, DST);
    f.reg_write(0x08, 2);
    f.reg_write(CH_AL1_CTRL_OFFSET, ctrl_word(0, 0, TREQ_PERMANENT));
    let state = f.channel(0);
    assert!(state.enabled);
    assert!(!state.busy);
    assert_eq!(f.run(2), 0, "al1_ctrl must not start the channel");

    // 之后 MULTI_CHAN_TRIGGER 才真正启动（dma_channel_start）。
    // Only MULTI_CHAN_TRIGGER starts it afterwards (dma_channel_start).
    f.reg_write(MULTI_CHAN_TRIGGER, 1);
    assert_eq!(f.channel(0).trans_count, 2);
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

// ---------------------------------------------------------------------------
// SDK control-block ring / SDK 控制块环
// ---------------------------------------------------------------------------

/// Entries in each control-block ring / 每个控制块环的条目数
const CB_RING_WORDS: usize = 256;
/// Byte size of one control-block ring / 每个控制块环的字节数
const CB_RING_BYTES: u32 = 1024;
/// `RING_SIZE` log2 used by the SDK ring channels / SDK 环通道的 `RING_SIZE` log2
const RING_LOG2: u32 = 10;
/// Ring laps streamed by the test / 测试流过的环圈数
const RING_LAPS: usize = 3;
/// `cb_count` ring base / `cb_count` 环基地址
const COUNT_RING: u32 = RAM_BASE;
/// `cb_addr` ring base / `cb_addr` 环基地址
const ADDR_RING: u32 = RAM_BASE + 0x400;
/// First 1024 B line block / 第一个 1024 B 行块
const LINE_A: u32 = RAM_BASE + 0x800;
/// Second 1024 B line block / 第二个 1024 B 行块
const LINE_B: u32 = RAM_BASE + 0xC00;
/// Words in one line block / 一个行块的字数
const BLOCK_WORDS: usize = 256;
/// Border-line control-block word count / 边界行控制块字数
const SHORT_COUNT: u32 = 21;
/// Count that wraps the 1024 B read ring / 会环绕 1024 B 读环的计数
const LONG_COUNT: u32 = 300;

/// Word count of control-block `index` / 控制块 `index` 的字数
fn ring_count(index: usize) -> u32 {
    // 每第 8 个控制块跨越 1024 B 读环边界，用来验证环绕。
    // Every eighth control block crosses the 1024 B read-ring boundary.
    if index % 8 == 7 {
        LONG_COUNT
    } else {
        SHORT_COUNT
    }
}

/// Line block used by control-block `index` / 控制块 `index` 使用的行块
fn ring_block(index: usize) -> u32 {
    if index % 2 == 0 { LINE_A } else { LINE_B }
}

/// Payload word `word` of `block` / `block` 的第 `word` 个载荷字
fn block_word(block: u32, word: usize) -> u32 {
    let tag = if block == LINE_A {
        0xA0A0_0000
    } else {
        0xB0B0_0000
    };
    tag + word as u32
}

/// One full lap of the expected FIFO stream / 一整圈期望的 FIFO 数据流
fn expected_lap() -> Vec<u32> {
    let mut stream = Vec::new();
    for index in 0..CB_RING_WORDS {
        let block = ring_block(index);
        for word in 0..ring_count(index) as usize {
            // 读环在 256 字（1024 B）处环绕，长计数会重放块首。
            // The read ring wraps at 256 words, so a long count replays the head.
            stream.push(block_word(block, word % BLOCK_WORDS));
        }
    }
    stream
}

/// Build a `CTRL` word the way the Pico SDK config helpers do
/// 按 Pico SDK 配置辅助函数的语义构造 `CTRL` 字
///
/// `dma_channel_get_default_config` yields word transfers, read increment and
/// write hold; `channel_config_set_ring(.., false, n)` selects the read side.
/// `dma_channel_get_default_config` 给出字宽传输、读递增、写保持；
/// `channel_config_set_ring(.., false, n)` 选择读侧环绕。
fn sdk_ctrl(treq: u8, ring_log2: u32, chain: usize) -> u32 {
    CTRL_EN
        | (DATA_SIZE_WORD << CTRL_DATA_SIZE_SHIFT)
        | CTRL_INCR_READ
        | (ring_log2 << CTRL_RING_SIZE_SHIFT)
        | ((chain as u32) << CTRL_CHAIN_TO_SHIFT)
        | (u32::from(treq) << CTRL_TREQ_SEL_SHIFT)
}

/// Program a channel exactly like `dma_channel_configure(..., false)`
/// 完全按 `dma_channel_configure(..., false)` 编程通道
fn sdk_configure(f: &mut Fixture, channel: usize, read: u32, write: u32, count: u32, ctrl: u32) {
    let base = channel as u32 * STRIDE;
    f.reg_write(base + CH_READ_ADDR_OFFSET, read);
    f.reg_write(base + CH_WRITE_ADDR_OFFSET, write);
    f.reg_write(base + CH_TRANS_COUNT_OFFSET, count);
    // SDK 用 al1_ctrl 写配置：不触发传输。
    // The SDK writes the config through al1_ctrl, which does not trigger.
    f.reg_write(base + CH_AL1_CTRL_OFFSET, ctrl);
}

/// Program the three-channel `hstx_start_dma()` ring on a fresh fixture
/// 在全新夹具上编程 `hstx_start_dma()` 的三通道路由
fn program_ring(f: &mut Fixture) -> Arc<Mutex<Vec<u32>>> {
    let words = Arc::new(Mutex::new(Vec::new()));
    let window = MmioWindow::new(SINK_BASE, 4);
    let sink = WordSink {
        window,
        words: Arc::clone(&words),
    };
    mount_device(&mut f.bus, Box::new(sink)).expect("word sink");
    for index in 0..CB_RING_WORDS {
        f.write_word(COUNT_RING + index as u32 * 4, ring_count(index));
        f.write_word(ADDR_RING + index as u32 * 4, ring_block(index));
    }
    for word in 0..BLOCK_WORDS {
        f.write_word(LINE_A + word as u32 * 4, block_word(LINE_A, word));
        f.write_word(LINE_B + word as u32 * 4, block_word(LINE_B, word));
    }
    // DATA：1024 B 读环 → HSTX FIFO，DREQ_HSTX 节拍，链到 CB_COUNT。
    let data = sdk_ctrl(dreq::HSTX_DREQ, RING_LOG2, 1);
    sdk_configure(f, 0, LINE_A, SINK_BASE, SHORT_COUNT, data);
    // CB_COUNT：环字 → DATA.al3_transfer_count（不触发），链到 CB_ADDR。
    let cb_count = sdk_ctrl(TREQ_PERMANENT, RING_LOG2, 2);
    sdk_configure(
        f,
        1,
        COUNT_RING,
        map::DMA_BASE + CH_AL3_TRANS_COUNT_OFFSET,
        1,
        cb_count,
    );
    // CB_ADDR：环字 → DATA.al3_read_addr_trig（触发），自链。
    let cb_addr = sdk_ctrl(TREQ_PERMANENT, RING_LOG2, 2);
    sdk_configure(
        f,
        2,
        ADDR_RING,
        map::DMA_BASE + CH_AL3_READ_ADDR_TRIG_OFFSET,
        1,
        cb_addr,
    );
    words
}

#[test]
fn hstx_control_block_ring_streams_three_laps() {
    let mut f = fixture();
    let sink = program_ring(&mut f);
    f.dma.set_dreq(dreq::HSTX_DREQ, true);
    // 配置阶段不允许启动任何通道（al1_ctrl 非触发）。
    // Configuration must not start anything (al1_ctrl does not trigger).
    assert_eq!(f.dma.busy_mask(), 0, "configuration must not trigger");
    f.reg_write(MULTI_CHAN_TRIGGER, 1 << 1); // dma_channel_start(CB_COUNT)
    assert_eq!(f.dma.busy_mask(), 0b10, "only CB_COUNT starts");

    let lap = expected_lap();
    let target = lap.len() * RING_LAPS;
    let mut budget = 0u64;
    let mut saw_partial = false;
    while sink.lock().len() < target {
        let moved = f.run(97);
        assert!(moved > 0, "ring stalled after {} words", sink.lock().len());
        budget += moved;
        assert!(budget < 4_000_000, "ring made no progress");
        // 环指针必须留在自己的 1024 B 窗口内，且 EN 不会被链式清掉。
        // Ring pointers stay inside their 1024 B windows and keep EN.
        let (data, count, addr) = (f.channel(0), f.channel(1), f.channel(2));
        assert!((COUNT_RING..COUNT_RING + CB_RING_BYTES).contains(&count.read_addr));
        assert!((ADDR_RING..ADDR_RING + CB_RING_BYTES).contains(&addr.read_addr));
        assert!((LINE_A..LINE_B + CB_RING_BYTES).contains(&data.read_addr));
        let enabled = (data.ctrl | count.ctrl | addr.ctrl) & CTRL_EN;
        assert_eq!(enabled, CTRL_EN, "a ring channel lost EN");
        if data.busy {
            // 工作计数只能来自 CB 写入的重载值。
            // The working count can only come from the CB rewrite value.
            assert!(data.trans_count > 0 && data.trans_count <= LONG_COUNT);
            saw_partial |= data.trans_count < LONG_COUNT;
        } else {
            assert_eq!(data.trans_count, 0, "idle DATA keeps a drained count");
        }
    }
    assert!(saw_partial, "never observed DATA mid-entry");

    // 三圈数据流必须逐字一致：链式、重载与环绕都没有丢数据。
    // Three laps must match word for word: chain, reload and wrap lose nothing.
    let words = sink.lock().clone();
    for lap_index in 0..RING_LAPS {
        let start = lap_index * lap.len();
        assert_eq!(
            &words[start..start + lap.len()],
            &lap[..],
            "lap {lap_index}"
        );
    }
    // 环没有提前停止：继续泵送仍会推进。
    // The ring did not stop early: further pumping still progresses.
    assert!(f.run(200) > 0, "ring stopped after three laps");
    assert_ne!(f.dma.busy_mask(), 0);
}

#[test]
fn read_ring_streams_a_line_block_twice() {
    let mut f = fixture();
    for word in 0..BLOCK_WORDS {
        f.write_word(LINE_A + word as u32 * 4, block_word(LINE_A, word));
    }
    // 读环 1024 B + 计数 300 → 读完 256 字后重放块首 44 字。
    // Read ring 1024 B with count 300 replays the first 44 words.
    let ctrl = ctrl_word(0, DATA_SIZE_WORD, TREQ_PERMANENT) | (RING_LOG2 << CTRL_RING_SIZE_SHIFT);
    configure(&mut f, 0, LINE_A, DST, LONG_COUNT, ctrl);
    assert_eq!(f.run(u64::from(LONG_COUNT)), u64::from(LONG_COUNT));
    assert_eq!(f.channel(0).read_addr, LINE_A + 44 * 4);
    assert_eq!(f.read_word(DST + 255 * 4), block_word(LINE_A, 255));
    assert_eq!(f.read_word(DST + 256 * 4), block_word(LINE_A, 0));
    assert_eq!(f.read_word(DST + 299 * 4), block_word(LINE_A, 43));
}

#[test]
fn al2_and_al3_ctrl_aliases_do_not_trigger() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[3, 4]);
    for (channel, offset) in [(0usize, CH_AL2_CTRL_OFFSET), (1, CH_AL3_CTRL_OFFSET)] {
        let ctrl = sdk_ctrl(TREQ_PERMANENT, 0, channel);
        sdk_configure(&mut f, channel, SRC, DST + channel as u32 * 0x10, 2, ctrl);
        f.reg_write(channel as u32 * STRIDE + offset, ctrl);
        assert!(
            !f.channel(channel).busy,
            "alias {offset:#04x} must not trigger"
        );
    }
    assert_eq!(f.run(4), 0);
    f.reg_write(MULTI_CHAN_TRIGGER, 0b11);
    assert_eq!(f.run(4), 4);
}

#[test]
fn al3_read_addr_trig_starts_an_enabled_idle_channel() {
    let mut f = fixture();
    write_bytes(&mut f, SRC + 0x40, &[7]);
    sdk_configure(&mut f, 0, SRC, DST, 1, sdk_ctrl(TREQ_PERMANENT, 0, 0));
    assert!(!f.channel(0).busy);
    // 零值写入不是触发（只有 IRQ_QUIET 时才产生空触发中断）。
    // A zero write is not a trigger (only a null-trigger IRQ with IRQ_QUIET).
    f.reg_write(CH_AL3_READ_ADDR_TRIG_OFFSET, 0);
    assert!(!f.channel(0).busy);
    f.reg_write(CH_AL3_READ_ADDR_TRIG_OFFSET, SRC + 0x40);
    assert!(f.channel(0).busy);
    assert_eq!(f.channel(0).trans_count, 1);
    assert_eq!(f.run(1), 1);
    assert_eq!(f.read_word(DST), 7);
}

#[test]
fn trans_count_writes_latch_a_reload_value() {
    let mut f = fixture();
    sdk_configure(&mut f, 0, SRC, DST, 1, sdk_ctrl(TREQ_PERMANENT, 0, 0));
    f.reg_write(CH_AL3_TRANS_COUNT_OFFSET, 4);
    assert!(!f.channel(0).busy, "0x38 must not trigger");
    f.reg_write(MULTI_CHAN_TRIGGER, 1);
    assert_eq!(f.channel(0).trans_count, 4);
    assert_eq!(f.run(2), 2);
    // 忙碌时写计数只改重载值，工作计数继续递减。
    // A count write while busy only changes the reload value.
    f.reg_write(CH_TRANS_COUNT_OFFSET, 9);
    assert_eq!(f.channel(0).trans_count, 2);
    assert_eq!(f.reg_read(CH_TRANS_COUNT_OFFSET), 2);
    assert_eq!(f.run(2), 2);
    f.reg_write(MULTI_CHAN_TRIGGER, 1);
    assert_eq!(f.channel(0).trans_count, 9, "reload applies on restart");
}

#[test]
fn chain_reloads_a_completed_one_shot_channel() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[1, 2, 3, 4, 5, 6]);
    // ch0（计数 2）↔ ch1（计数 1）构成三段式乒乓环。
    // ch0 (count 2) <-> ch1 (count 1) form a 3-unit ping-pong ring.
    sdk_configure(&mut f, 0, SRC, DST, 2, sdk_ctrl(TREQ_PERMANENT, 0, 1));
    sdk_configure(
        &mut f,
        1,
        DST,
        DST + 0x100,
        1,
        sdk_ctrl(TREQ_PERMANENT, 0, 0),
    );
    f.reg_write(MULTI_CHAN_TRIGGER, 1);
    // 没有重载语义时 ch1 只搬运一次，环在 3 个单元后停住。
    // Without reload semantics ch1 moves one unit only and the ring dies at 3.
    assert_eq!(f.run(9), 9);
    assert_eq!(f.channel(1).trans_count, 0);
    // 第九个单元是 ch1 完成，它把 ch0 重新装载为 2 并置为忙碌。
    // Unit nine is ch1 completing, which reloads ch0 to 2 and makes it busy.
    assert!(f.channel(0).busy);
    assert_eq!(f.channel(0).trans_count, 2);
    assert_eq!(f.dma.busy_mask(), 0b1);
}

#[test]
fn multi_chan_trigger_skips_disabled_channels() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[1, 2]);
    sdk_configure(&mut f, 0, SRC, DST, 2, sdk_ctrl(TREQ_PERMANENT, 0, 0));
    let disabled = sdk_ctrl(TREQ_PERMANENT, 0, 1) & !CTRL_EN;
    sdk_configure(&mut f, 1, SRC, DST + 0x10, 2, disabled);
    f.reg_write(MULTI_CHAN_TRIGGER, 0b11);
    assert_eq!(f.dma.busy_mask(), 0b1, "only the enabled channel starts");
}

#[test]
fn trigger_while_busy_keeps_the_working_count() {
    let mut f = fixture();
    sdk_configure(&mut f, 0, SRC, DST, 5, sdk_ctrl(TREQ_PERMANENT, 0, 0));
    f.reg_write(MULTI_CHAN_TRIGGER, 1);
    assert_eq!(f.run(2), 2);
    // 忙碌通道上的触发被忽略，不会重置计数。
    // A trigger on a busy channel is ignored and does not reload the count.
    f.reg_write(MULTI_CHAN_TRIGGER, 1);
    assert!(f.channel(0).busy);
    assert_eq!(f.channel(0).trans_count, 3);
    assert_eq!(f.run(3), 3);
}

#[test]
fn zero_length_trigger_completes_chains_and_honours_irq_quiet() {
    let mut f = fixture();
    sdk_configure(&mut f, 0, SRC, DST, 0, sdk_ctrl(TREQ_PERMANENT, 0, 1));
    sdk_configure(&mut f, 1, SRC, DST, 1, sdk_ctrl(TREQ_PERMANENT, 0, 1));
    f.reg_write(MULTI_CHAN_TRIGGER, 1);
    assert!(!f.channel(0).busy);
    assert!(f.channel(1).busy, "the zero-length channel chains on");
    assert_eq!(f.reg_read(INTR) & 1, 1);
    assert_eq!(f.run(1), 1);
    // 空触发：向 *_TRIG 写零，在 IRQ_QUIET 时仍产生完成中断。
    // Null trigger: a zero write to a *_TRIG alias with IRQ_QUIET still
    // raises the completion flag.
    let quiet = sdk_ctrl(TREQ_PERMANENT, 0, 2) | CTRL_IRQ_QUIET;
    sdk_configure(&mut f, 2, SRC, DST, 4, quiet);
    f.reg_write(2 * STRIDE + CH_AL3_READ_ADDR_TRIG_OFFSET, 0);
    assert!(!f.channel(2).busy);
    assert_eq!(f.reg_read(INTR) & (1 << 2), 1 << 2);
}

// ---------------------------------------------------------------------------
// Sniffer CALC vectors / 嗅探 `CALC` 已知向量
// ---------------------------------------------------------------------------

/// `SNIFF_CTRL` offset / `SNIFF_CTRL` 偏移
const SNIFF_CTRL_OFFSET: u32 = 0x454;

/// `SNIFF_DATA` offset / `SNIFF_DATA` 偏移
const SNIFF_DATA_OFFSET: u32 = 0x458;

/// Build a `SNIFF_CTRL` value for one channel and `CALC` mode
/// 为某个通道与 `CALC` 模式构造 `SNIFF_CTRL` 值
fn sniff_ctrl_for(calc: u32, channel: u32) -> u32 {
    SNIFF_EN | (channel << SNIFF_DMACH_SHIFT) | (calc << SNIFF_CALC_SHIFT)
}

/// Channel control word that feeds the sniffer / 会把数据送入嗅探器的通控制字
fn sniffing_ctrl(channel: usize, data_size: u32) -> u32 {
    ctrl_word(channel, data_size, TREQ_PERMANENT) | CTRL_SNIFF_EN
}

#[test]
fn sniff_crc32r_matches_the_bit_reversed_bzip2_vector() {
    // CALC=0x1（输入字节位反转）等价于 CRC-32/BZIP2 的内部寄存器位反转形式：
    // 多项式 0x04C1_1DB7、init 0xFFFF_FFFF、refin/refout=false、xorout 0xFFFF_FFFF
    // 对 "123456789" 的内部寄存器为 0x0376_E6E7，bitreverse32 后即 0xE767_6EC0。
    // CALC=0x1 (bit-reversed input bytes) is the bit-reversed-register form of
    // CRC-32/BZIP2 (poly 0x04C1_1DB7, init 0xFFFF_FFFF, no refin/refout): its
    // internal register for "123456789" is 0x0376_E6E7, so reverse_bits() gives
    // 0xE767_6EC0 for seed 0xFFFF_FFFF.
    let mut f = fixture();
    write_bytes(&mut f, SRC, b"123456789");
    f.reg_write(SNIFF_CTRL_OFFSET, sniff_ctrl_for(SNIFF_CALC_CRC32R, 0));
    f.reg_write(SNIFF_DATA_OFFSET, 0xFFFF_FFFF);
    configure(&mut f, 0, SRC, DST, 9, sniffing_ctrl(0, 0));
    assert_eq!(f.run(9), 9);
    assert_eq!(f.reg_read(SNIFF_DATA_OFFSET), 0xE767_6EC0);
}

#[test]
fn sniff_crc16r_matches_the_reflected_ccitt_vector() {
    // CALC=0x3 是 CRC-16/CCITT-FALSE（0x1021, init 0xFFFF）的反射寄存器形式：
    // 标准校验值 0x29B1 的 bitreverse16 即 0x8D94。
    // CALC=0x3 is the reflected-register form of CRC-16/CCITT-FALSE (0x1021,
    // init 0xFFFF): bitreverse16 of the standard check value 0x29B1 is 0x8D94.
    let mut f = fixture();
    write_bytes(&mut f, SRC, b"123456789");
    f.reg_write(SNIFF_CTRL_OFFSET, sniff_ctrl_for(SNIFF_CALC_CRC16R, 0));
    f.reg_write(SNIFF_DATA_OFFSET, 0x0000_FFFF);
    configure(&mut f, 0, SRC, DST, 9, sniffing_ctrl(0, 0));
    assert_eq!(f.run(9), 9);
    assert_eq!(f.reg_read(SNIFF_DATA_OFFSET), 0x8D94);
    // 结果与正向 CRC16 的 0x29B1 互为位反转，可交叉验证。
    // The result is the bit reverse of the forward CRC16 value 0x29B1.
    assert_eq!(0x8D94u16.reverse_bits(), 0x29B1);
}

#[test]
fn sniff_sum_wraps_and_masks_to_the_unit_width() {
    let mut f = fixture();
    // 半字单元按 16 位掩码累加：0xBEEF + 0x1234 = 0xD123。
    // Halfword units accumulate under a 16-bit mask: 0xBEEF + 0x1234 = 0xD123.
    f.write_half(SRC, 0xBEEF);
    f.write_half(SRC + 2, 0x1234);
    f.reg_write(SNIFF_CTRL_OFFSET, sniff_ctrl_for(SNIFF_CALC_SUM, 0));
    configure(&mut f, 0, SRC, DST, 2, sniffing_ctrl(0, 1));
    assert_eq!(f.run(2), 2);
    assert_eq!(f.reg_read(SNIFF_DATA_OFFSET), 0xD123);
    // 32 位加法校验和在 u32 上环绕：0xFFFF_FFFF + 2 = 1。
    // The 32-bit addition checksum wraps: 0xFFFF_FFFF + 2 = 1.
    let mut wrap = fixture();
    wrap.write_word(SRC, 0xFFFF_FFFF);
    wrap.write_word(SRC + 4, 0x0000_0002);
    wrap.reg_write(SNIFF_CTRL_OFFSET, sniff_ctrl_for(SNIFF_CALC_SUM, 0));
    configure(&mut wrap, 0, SRC, DST, 2, sniffing_ctrl(0, DATA_SIZE_WORD));
    assert_eq!(wrap.run(2), 2);
    assert_eq!(wrap.reg_read(SNIFF_DATA_OFFSET), 1);
}

#[test]
fn sniff_xor_folds_every_unit_across_a_transfer() {
    let mut f = fixture();
    // 奇偶按单元折叠：0x00FF（8 个 1，偶）+ 0x0003（偶）+ 0x0001（奇）= 1。
    // Parity folds per unit: 0x00FF (8 ones, even) + 0x0003 (even) + 0x0001 = 1.
    f.write_half(SRC, 0x00FF);
    f.write_half(SRC + 2, 0x0003);
    f.write_half(SRC + 4, 0x0001);
    f.reg_write(SNIFF_CTRL_OFFSET, sniff_ctrl_for(SNIFF_CALC_XOR, 0));
    configure(&mut f, 0, SRC, DST, 3, sniffing_ctrl(0, 1));
    assert_eq!(f.run(3), 3);
    assert_eq!(f.reg_read(SNIFF_DATA_OFFSET), 1);
    // 再观察一个奇校验单元后折叠回 0。One odd unit folds it back to 0.
    f.reg_write(SNIFF_CTRL_OFFSET, sniff_ctrl_for(SNIFF_CALC_XOR, 0));
    configure(&mut f, 0, SRC + 4, DST, 1, sniffing_ctrl(0, 1));
    assert_eq!(f.run(1), 1);
    assert_eq!(f.reg_read(SNIFF_DATA_OFFSET), 0);
}

#[test]
fn sniff_reserved_calc_values_leave_the_accumulator_unchanged() {
    // hardware/regs/dma.h 只定义 CALC 0x0..=0x3、0xE、0xF；0x4..=0xD 为保留值，
    // 累加器必须保持不变（不伪造算法）。
    // hardware/regs/dma.h defines only CALC 0x0..=0x3, 0xE and 0xF; 0x4..=0xD are
    // reserved, so the accumulator must stay untouched (no invented algorithms).
    let mut f = fixture();
    write_bytes(&mut f, SRC, b"123456789");
    for calc in 0x4u32..=0xD {
        f.reg_write(SNIFF_CTRL_OFFSET, sniff_ctrl_for(calc, 0));
        f.reg_write(SNIFF_DATA_OFFSET, 0x1234_5678);
        configure(&mut f, 0, SRC, DST, 9, sniffing_ctrl(0, 0));
        assert_eq!(f.run(9), 9, "calc {calc:#X} still moves data");
        assert_eq!(
            f.reg_read(SNIFF_DATA_OFFSET),
            0x1234_5678,
            "reserved calc {calc:#X} must not touch the accumulator"
        );
        assert_eq!(read_bytes(&mut f, DST, 9), b"123456789".to_vec());
    }
}

#[test]
fn sniff_data_read_returns_the_accumulator_without_clearing_it() {
    // 数据手册：SNIFF_DATA 是种子兼累加器，写入设置，读出不清零。
    // Datasheet: SNIFF_DATA is seed + accumulator; writes set it, reads do not
    // clear it.
    let mut f = fixture();
    f.reg_write(SNIFF_CTRL_OFFSET, sniff_ctrl_for(SNIFF_CALC_SUM, 0));
    f.reg_write(SNIFF_DATA_OFFSET, 0x0000_1000);
    assert_eq!(f.reg_read(SNIFF_DATA_OFFSET), 0x0000_1000);
    assert_eq!(f.reg_read(SNIFF_DATA_OFFSET), 0x0000_1000);
    f.write_word(SRC, 0x0000_0023);
    configure(&mut f, 0, SRC, DST, 1, sniffing_ctrl(0, DATA_SIZE_WORD));
    assert_eq!(f.run(1), 1);
    assert_eq!(f.reg_read(SNIFF_DATA_OFFSET), 0x0000_1023);
    assert_eq!(f.reg_read(SNIFF_DATA_OFFSET), 0x0000_1023);
}

#[test]
fn sniff_observes_the_post_bswap_unit() {
    // 嗅探位于通道字节交换之后，因此 CTRL.BSWAP 会改变被观察数据；
    // 两个 BSWAP 位同时置位时相互抵消（数据手册明确说明）。
    // The sniffer sits downstream of the channel byte swap, so CTRL.BSWAP
    // changes what it sees; with both BSWAP bits set the effects cancel.
    let mut f = fixture();
    f.write_word(SRC, 0x1122_3344);
    let swapped = ctrl_word(0, DATA_SIZE_WORD, TREQ_PERMANENT) | CTRL_SNIFF_EN | CTRL_BSWAP;
    f.reg_write(SNIFF_CTRL_OFFSET, sniff_ctrl_for(SNIFF_CALC_SUM, 0));
    configure(&mut f, 0, SRC, DST, 1, swapped);
    assert_eq!(f.run(1), 1);
    assert_eq!(f.reg_read(SNIFF_DATA_OFFSET), 0x4433_2211);
    assert_eq!(f.read_word(DST), 0x4433_2211);
    // 两个 BSWAP 抵消：嗅探器又看到原始字节序。
    // Both swaps cancel: the sniffer sees the original byte order again.
    f.reg_write(
        SNIFF_CTRL_OFFSET,
        sniff_ctrl_for(SNIFF_CALC_SUM, 0) | SNIFF_BSWAP,
    );
    f.reg_write(SNIFF_DATA_OFFSET, 0);
    configure(&mut f, 0, SRC, DST, 1, swapped);
    assert_eq!(f.run(1), 1);
    assert_eq!(f.reg_read(SNIFF_DATA_OFFSET), 0x1122_3344);
}

#[test]
fn sniff_accumulator_continues_across_channel_switches() {
    let mut f = fixture();
    f.write_word(SRC, 0x0000_0001);
    f.write_word(SRC + 4, 0x0000_0002);
    f.write_word(SRC + 0x100, 0x0000_0004);
    f.reg_write(SNIFF_CTRL_OFFSET, sniff_ctrl_for(SNIFF_CALC_SUM, 0));
    configure(&mut f, 0, SRC, DST, 2, sniffing_ctrl(0, DATA_SIZE_WORD));
    assert_eq!(f.run(2), 2);
    assert_eq!(f.reg_read(SNIFF_DATA_OFFSET), 3);
    // DMACH 改到通道 1：累加器继续累加而不是重新开始。
    // Retarget DMACH to channel 1: the accumulator keeps accumulating.
    f.reg_write(SNIFF_CTRL_OFFSET, sniff_ctrl_for(SNIFF_CALC_SUM, 1));
    configure(
        &mut f,
        1,
        SRC + 0x100,
        DST + 0x100,
        1,
        sniffing_ctrl(1, DATA_SIZE_WORD),
    );
    assert_eq!(f.run(1), 1);
    assert_eq!(f.reg_read(SNIFF_DATA_OFFSET), 7);
}

#[path = "dma_feature_tests.rs"]
mod feature;
