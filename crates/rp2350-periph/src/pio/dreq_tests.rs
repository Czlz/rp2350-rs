//! PIO ↔ DMA DREQ gating tests / PIO 与 DMA DREQ 门控测试
//!
//! Each test mounts the real [`BootPeripherals`] set (PIO blocks + DMA block) on
//! a [`SimpleBus`] with a RAM region, programs the DMA channel through MMIO,
//! drives the FIFOs through the shared PIO handles and checks the published
//! DREQ levels. No FIFO or register state is stubbed.
//! 每个测试在带 RAM 区域的 [`SimpleBus`] 上挂载真实的 [`BootPeripherals`]（PIO 块
//! 与 DMA 块），通过 MMIO 编程 DMA 通道，用共享 PIO 句柄驱动 FIFO，并检查发布的
//! DREQ 电平；不伪造任何 FIFO 或寄存器状态。

use rp2350_core::bus::AddressBus;
use rp2350_core::memory::{RamRegion, SimpleBus};

use crate::boot::BootPeripherals;
use crate::dma::{
    CTRL_CHAIN_TO_SHIFT, CTRL_DATA_SIZE_SHIFT, CTRL_EN, CTRL_INCR_READ, CTRL_INCR_WRITE,
    CTRL_TREQ_SEL_SHIFT, DmaChannelState,
};
use crate::dreq;
use crate::map;
use crate::pio::{PIO_SM_COUNT, Pio};

/// Test RAM base address / 测试 RAM 基地址
const RAM_BASE: u32 = 0x2000_0000;

/// Test RAM size / 测试 RAM 大小
const RAM_SIZE: usize = 0x2000;

/// `CTRL_TRIG.DATA_SIZE` value for word transfers / 字传输的 `DATA_SIZE` 值
const DATA_SIZE_WORD: u32 = 2;

/// PIO `FSTAT` offset / PIO `FSTAT` 偏移
const PIO_FSTAT: u32 = 0x004;

/// PIO `TXF0` offset / PIO `TXF0` 偏移
const PIO_TXF0: u32 = 0x010;

/// PIO `RXF0` offset / PIO `RXF0` 偏移
const PIO_RX0: u32 = 0x020;

/// `SM0_SHIFTCTRL` offset / `SM0_SHIFTCTRL` 偏移
const PIO_SM0_SHIFTCTRL: u32 = 0x0D0;

/// `SHIFTCTRL.FJOIN_TX` / `SHIFTCTRL.FJOIN_TX`
const FJOIN_TX: u32 = 1 << 30;

/// `SHIFTCTRL.FJOIN_RX` / `SHIFTCTRL.FJOIN_RX`
const FJOIN_RX: u32 = 1 << 31;

/// Fixture: bus plus the mounted boot peripheral set
/// 夹具：总线与已挂载的启动外设集合
struct Fixture {
    bus: SimpleBus,
    peripherals: BootPeripherals,
}

impl Fixture {
    /// Write one word through the bus / 通过总线写一个字
    fn write_word(&mut self, address: u32, value: u32) {
        let mut cycles = 0u64;
        self.bus
            .write_u32(address, value, &mut cycles)
            .expect("word write");
    }

    /// Read one word through the bus / 通过总线读一个字
    fn read_word(&mut self, address: u32) -> u32 {
        let mut cycles = 0u64;
        self.bus.read_u32(address, &mut cycles).expect("word read")
    }

    /// Write a channel's address, count and control registers
    /// 写入通道的地址、计数与控制寄存器
    fn write_channel(&mut self, channel: u32, read: u32, write: u32, count: u32, ctrl: u32) {
        let base = map::DMA_BASE + channel * 0x40;
        self.write_word(base, read);
        self.write_word(base + 0x04, write);
        self.write_word(base + 0x08, count);
        self.write_word(base + 0x0C, ctrl);
    }

    /// Program a word-mode channel feeding a PIO TX FIFO
    /// 编程一个向 PIO TX FIFO 写入的字宽通道
    fn program_tx_channel(&mut self, channel: u32, src: u32, fifo: u32, count: u32, treq: u8) {
        let ctrl = ctrl_word(channel, treq, true, false);
        self.write_channel(channel, src, fifo, count, ctrl);
    }

    /// Program a word-mode channel draining a PIO RX FIFO
    /// 编程一个从 PIO RX FIFO 读出的字宽通道
    fn program_rx_channel(&mut self, channel: u32, fifo: u32, dst: u32, count: u32, treq: u8) {
        let ctrl = ctrl_word(channel, treq, false, true);
        self.write_channel(channel, fifo, dst, count, ctrl);
    }

    /// Pop one word from a TX FIFO through the shared handle
    /// 通过共享句柄从 TX FIFO 取出一个字
    fn pop_tx(&mut self, block: usize, sm: usize) -> Option<u32> {
        self.peripherals
            .with_pio(block, |pio| pio.pop_tx(sm))
            .flatten()
    }

    /// Push one word into an RX FIFO through the shared handle
    /// 通过共享句柄向 RX FIFO 压入一个字
    fn push_rx(&mut self, block: usize, sm: usize, value: u32) -> bool {
        self.peripherals
            .with_pio(block, |pio| pio.push_rx(sm, value))
            .unwrap_or(false)
    }

    /// Published DREQ level of one selector / 某个选择器已发布的 DREQ 电平
    fn dreq(&self, selector: u8) -> Option<bool> {
        self.peripherals.dma().dreq(selector)
    }

    /// Republish all PIO FIFO levels / 重新发布全部 PIO FIFO 电平
    fn sync(&self) {
        self.peripherals.sync_pio_dreqs();
    }

    /// Channel snapshot / 通道快照
    fn channel(&self, index: usize) -> DmaChannelState {
        self.peripherals
            .dma()
            .channel_state(index)
            .expect("channel")
    }
}

/// Create a fixture with RAM, PIO and DMA mounted / 创建挂载 RAM、PIO 与 DMA 的夹具
fn fixture() -> Fixture {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("ram", RAM_BASE, RAM_SIZE)))
        .expect("ram region");
    let peripherals = BootPeripherals::mount(&mut bus).expect("mount");
    Fixture { bus, peripherals }
}

/// Build a word-mode `CTRL_TRIG` word with the channel's self-chain target
/// 构造字宽 `CTRL_TRIG` 字，链式目标指向通道自身
fn ctrl_word(channel: u32, treq: u8, incr_read: bool, incr_write: bool) -> u32 {
    let mut ctrl = CTRL_EN
        | (DATA_SIZE_WORD << CTRL_DATA_SIZE_SHIFT)
        | (u32::from(treq) << CTRL_TREQ_SEL_SHIFT)
        | (channel << CTRL_CHAIN_TO_SHIFT);
    if incr_read {
        ctrl |= CTRL_INCR_READ;
    }
    if incr_write {
        ctrl |= CTRL_INCR_WRITE;
    }
    ctrl
}

#[test]
fn empty_tx_fifo_asserts_tx_dreq_and_dma_fills_it() {
    // TX FIFO 空 → TX DREQ 高 → 由 PIO0_TX0 节拍的 DMA 把源缓冲写入 TX FIFO。
    // Empty TX FIFO -> TX DREQ high -> DMA paced by PIO0_TX0 writes into the FIFO.
    let mut f = fixture();
    let words = [0x0102_0304u32, 0x0506_0708, 0x090A_0B0C];
    for (index, word) in words.iter().enumerate() {
        f.write_word(RAM_BASE + index as u32 * 4, *word);
    }
    f.program_tx_channel(0, RAM_BASE, map::PIO0_BASE + PIO_TXF0, 3, dreq::PIO0_TX0);
    assert_eq!(
        f.peripherals.pio(0).expect("pio0").tx_fifo_empty(0),
        Some(true)
    );
    f.sync();
    assert_eq!(f.dreq(dreq::PIO0_TX0), Some(true));
    assert_eq!(f.peripherals.dma_mut().run(&mut f.bus, 3), 3);

    for word in words {
        assert_eq!(f.pop_tx(0, 0), Some(word), "FIFO order must be preserved");
    }
    assert_eq!(f.pop_tx(0, 0), None);
    assert_eq!(f.channel(0).trans_count, 0);
    assert!(!f.channel(0).busy);
}

#[test]
fn full_tx_fifo_deasserts_tx_dreq_and_stalls_dma() {
    // TX FIFO 满 → DREQ 低 → DMA 不推进；腾出一个字后恢复。
    // Full TX FIFO -> DREQ low -> DMA stalls; freeing one word resumes it.
    let mut f = fixture();
    for word in [1u32, 2, 3, 4] {
        f.write_word(map::PIO0_BASE + PIO_TXF0, word);
    }
    assert_eq!(
        f.peripherals.pio(0).expect("pio0").tx_fifo_full(0),
        Some(true)
    );
    f.program_tx_channel(0, RAM_BASE, map::PIO0_BASE + PIO_TXF0, 2, dreq::PIO0_TX0);
    f.sync();
    assert_eq!(f.dreq(dreq::PIO0_TX0), Some(false));
    assert_eq!(f.peripherals.dma_mut().run(&mut f.bus, 2), 0);
    assert_eq!(f.channel(0).trans_count, 2);
    assert!(f.channel(0).busy);

    assert_eq!(f.pop_tx(0, 0), Some(1));
    f.sync();
    assert_eq!(f.dreq(dreq::PIO0_TX0), Some(true));
    assert_eq!(f.peripherals.dma_mut().run(&mut f.bus, 1), 1);
    assert_eq!(f.channel(0).trans_count, 1);
}

#[test]
fn rx_fifo_data_asserts_rx_dreq_and_dma_drains_it() {
    // RX FIFO 有数据 → RX DREQ 高 → DMA 读走；排空后 DREQ 变低并停止。
    // RX FIFO data -> RX DREQ high -> DMA drains it; once empty the DREQ drops
    // and the channel stops.
    let mut f = fixture();
    let words = [0x1111_2222u32, 0x3333_4444, 0x5555_6666];
    for word in words {
        assert!(f.push_rx(0, 0, word));
    }
    let dst = RAM_BASE + 0x1000;
    f.program_rx_channel(0, map::PIO0_BASE + PIO_RX0, dst, 3, dreq::PIO0_RX0);
    f.sync();
    assert_eq!(f.dreq(dreq::PIO0_RX0), Some(true));
    assert_eq!(f.peripherals.dma_mut().run(&mut f.bus, 3), 3);
    for (index, word) in words.iter().enumerate() {
        assert_eq!(f.read_word(dst + index as u32 * 4), *word);
    }
    assert_eq!(
        f.peripherals.pio(0).expect("pio0").rx_fifo_empty(0),
        Some(true)
    );
    f.sync();
    assert_eq!(f.dreq(dreq::PIO0_RX0), Some(false));
    assert_eq!(f.peripherals.dma_mut().run(&mut f.bus, 1), 0);
}

#[test]
fn joined_tx_fifo_uses_its_eight_word_depth() {
    // FJOIN_TX：TX FIFO 变 8 深度，第 8 个字才把 DREQ 拉低；RX 方向被禁用。
    // FJOIN_TX: the TX FIFO becomes 8 deep, only the eighth word drops the DREQ,
    // and the joined-away RX direction stays deasserted.
    let mut f = fixture();
    f.write_word(map::PIO0_BASE + PIO_SM0_SHIFTCTRL, FJOIN_TX);
    for word in 0..7u32 {
        f.write_word(map::PIO0_BASE + PIO_TXF0, word);
    }
    assert_eq!(
        f.peripherals.pio(0).expect("pio0").tx_fifo_full(0),
        Some(false)
    );
    f.sync();
    assert_eq!(f.dreq(dreq::PIO0_TX0), Some(true));
    assert_eq!(f.dreq(dreq::PIO0_RX0), Some(false), "RX is joined away");

    f.write_word(map::PIO0_BASE + PIO_TXF0, 7);
    assert_eq!(
        f.peripherals.pio(0).expect("pio0").tx_fifo_full(0),
        Some(true)
    );
    // FSTAT 与查询接口必须一致：位 16 是 SM0 的 TXFULL。
    // FSTAT and the query API must agree: bit 16 is SM0's TXFULL.
    assert_eq!(f.read_word(map::PIO0_BASE + PIO_FSTAT) & (1 << 16), 1 << 16);
    f.sync();
    assert_eq!(f.dreq(dreq::PIO0_TX0), Some(false));
}

#[test]
fn joined_rx_fifo_uses_its_eight_word_depth() {
    // FJOIN_RX：RX FIFO 变 8 深度且非空即断言 DREQ；TX 方向被禁用。
    // FJOIN_RX: the RX FIFO becomes 8 deep and asserts its DREQ while non-empty;
    // the joined-away TX direction stays deasserted.
    let mut f = fixture();
    f.write_word(map::PIO0_BASE + PIO_SM0_SHIFTCTRL, FJOIN_RX);
    for word in 0..8u32 {
        assert!(f.push_rx(0, 0, 0x8000_0000 | word));
    }
    assert_eq!(
        f.peripherals.pio(0).expect("pio0").rx_fifo_full(0),
        Some(true)
    );
    assert!(!f.push_rx(0, 0, 0xDEAD_BEEF), "ninth word must not fit");
    f.sync();
    assert_eq!(f.dreq(dreq::PIO0_RX0), Some(true));
    assert_eq!(f.dreq(dreq::PIO0_TX0), Some(false), "TX is joined away");

    for word in 0..8u32 {
        assert_eq!(f.read_word(map::PIO0_BASE + PIO_RX0), 0x8000_0000 | word);
    }
    f.sync();
    assert_eq!(f.dreq(dreq::PIO0_RX0), Some(false));
}

#[test]
fn block_and_state_machine_selectors_do_not_alias() {
    // PIO1 SM2 满只影响选择器 10；PIO2 SM1 满只影响 17；PIO0 保持就绪。
    // A full PIO1 SM2 only affects selector 10, a full PIO2 SM1 only affects 17,
    // and PIO0 stays ready.
    let mut f = fixture();
    for word in 0..4u32 {
        f.write_word(map::PIO1_BASE + PIO_TXF0 + 2 * 4, word);
        f.write_word(map::PIO2_BASE + PIO_TXF0 + 4, word);
    }
    f.sync();
    assert_eq!(dreq::pio_tx_dreq(1, 2), Some(10));
    assert_eq!(dreq::pio_tx_dreq(2, 1), Some(17));
    assert_eq!(f.dreq(10), Some(false), "PIO1 SM2 TX is full");
    assert_eq!(f.dreq(17), Some(false), "PIO2 SM1 TX is full");
    assert_eq!(f.dreq(9), Some(true), "PIO1 SM1 TX is empty");
    assert_eq!(f.dreq(16), Some(true), "PIO2 SM0 TX is empty");
    assert_eq!(f.dreq(dreq::PIO0_TX3), Some(true));
    assert_eq!(f.dreq(dreq::PIO1_RX2), Some(false));
    assert_eq!(f.dreq(dreq::PIO2_RX1), Some(false));
}

#[test]
fn sync_without_dma_activity_is_idempotent_and_scoped_to_pio() {
    // 空闲系统上同步两次：PIO 选择器反映真实 FIFO 状态，非 PIO 选择器、
    // 通道状态与中断线都不受影响。
    // Two syncs on an idle system: PIO selectors reflect the real FIFO state,
    // while non-PIO selectors, channel state and interrupt lines stay untouched.
    let f = fixture();
    f.peripherals.dma().set_dreq(40, false);
    let irq_before = f.peripherals.irq_mask();
    let busy_before = f.peripherals.dma().busy_mask();
    let channel_before = f.channel(0);

    f.sync();
    f.sync();

    for selector in dreq::PIO_DREQ_FIRST..=dreq::PIO_DREQ_LAST {
        // 每个块的 TX0..TX3 就绪、RX0..RX3 为空；FIFO 复位为空。
        // TX0..TX3 of every block are ready and RX0..RX3 are empty at reset.
        let expected = selector % dreq::PIO_DREQ_BLOCK_STRIDE < dreq::PIO_TX_PER_BLOCK;
        assert_eq!(f.dreq(selector), Some(expected), "selector {selector}");
    }
    assert_eq!(f.dreq(40), Some(false), "non-PIO selector untouched");
    assert_eq!(f.peripherals.irq_mask(), irq_before);
    assert_eq!(f.peripherals.dma().busy_mask(), busy_before);
    assert_eq!(f.channel(0), channel_before);
}

#[test]
fn pump_dma_resamples_pio_dreq_between_units() {
    // 一批 16 单元的计划里，PIO 节拍通道只能填满 4 深度 FIFO；腾出空间后继续。
    // In a 16-unit batch a PIO-paced channel can only fill the 4-deep FIFO; it
    // continues once space is freed.
    let mut f = fixture();
    for index in 0..8u32 {
        f.write_word(RAM_BASE + index * 4, 0xC000_0000 | index);
    }
    f.program_tx_channel(0, RAM_BASE, map::PIO0_BASE + PIO_TXF0, 8, dreq::PIO0_TX0);
    let moved = f.peripherals.pump_dma(&mut f.bus, 16);
    assert_eq!(moved, 4, "only the free FIFO slots may be written");
    assert_eq!(f.channel(0).trans_count, 4);
    assert!(f.channel(0).busy);

    assert_eq!(f.pop_tx(0, 0), Some(0xC000_0000));
    assert_eq!(f.pop_tx(0, 0), Some(0xC000_0001));
    let moved = f.peripherals.pump_dma(&mut f.bus, 16);
    assert_eq!(moved, 2, "two freed slots, then the FIFO is full again");
    assert_eq!(f.channel(0).trans_count, 2);
}

#[test]
fn fifo_queries_reject_out_of_range_state_machines() {
    // 查询接口与其余句柄一致：越界状态机返回 None，不 panic。
    // The queries follow the other handles: an out-of-range machine returns
    // None instead of panicking.
    let pio = Pio::new(0);
    assert_eq!(pio.tx_fifo_empty(PIO_SM_COUNT), None);
    assert_eq!(pio.tx_fifo_full(usize::MAX), None);
    assert_eq!(pio.rx_fifo_empty(PIO_SM_COUNT), None);
    assert_eq!(pio.rx_fifo_full(PIO_SM_COUNT), None);
    assert_eq!(pio.tx_fifo_empty(0), Some(true));
    assert_eq!(pio.rx_fifo_empty(0), Some(true));
}
