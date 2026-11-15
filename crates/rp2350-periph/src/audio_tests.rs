//! I2S audio capture tests / I2S 采样捕获测试
//!
//! The sink-level tests pin the sample packing, ordering, capacity and reset
//! semantics. The PIO tests drive the real register file and the real bus
//! adapters: CPU writes go through `Pio::write32`, DMA writes go through
//! `Dma::run` → `SimpleBus` → `PioWindow::write32`, and both must reach the same
//! tap.
//! 缓冲层测试固定样本打包、顺序、容量与复位语义。PIO 测试驱动真实寄存器文件与
//! 真实总线适配器：CPU 写入经 `Pio::write32`，DMA 写入经 `Dma::run` →
//! `SimpleBus` → `PioWindow::write32`，两条路径都必须到达同一个 tap。

use rp2350_core::bus::AddressBus;
use rp2350_core::memory::{RamRegion, SimpleBus};
use rp2350_core::mmio::MmioDevice;

use crate::audio::{AudioSink, AudioTap};
use crate::boot::BootPeripherals;
use crate::dma::{
    CTRL_CHAIN_TO_SHIFT, CTRL_DATA_SIZE_SHIFT, CTRL_EN, CTRL_INCR_READ, CTRL_TREQ_SEL_SHIFT,
};
use crate::dreq;
use crate::map;
use crate::pio::Pio;

/// Test RAM base address / 测试 RAM 基地址
const RAM_BASE: u32 = 0x2000_0000;

/// Test RAM size / 测试 RAM 大小
const RAM_SIZE: usize = 0x2000;

/// PIO `TXF0` register offset / PIO `TXF0` 寄存器偏移
const PIO_TXF0: u32 = 0x010;

/// `CTRL_TRIG.DATA_SIZE` value for word transfers / 字传输的 `DATA_SIZE` 值
const DATA_SIZE_WORD: u32 = 2;

/// Rate used by the test sinks (the emerald firmware's I2S rate)
/// 测试缓冲使用的采样率（emerald 固件的 I2S 速率）
const RATE: u32 = 13_440;

/// Create a stereo tap with room for `capacity` samples / 创建可容纳 `capacity` 个样本的立体声 tap
fn tap(capacity: usize) -> AudioTap {
    AudioTap::new(AudioSink::new(RATE, 2, capacity))
}

#[test]
fn push_word_splits_low_then_high_halfword() {
    // 低半字先播放（左声道），高半字随后（右声道）。
    // The low halfword plays first (left), the high halfword follows (right).
    let mut sink = AudioSink::new(RATE, 2, 8);
    sink.push_word(0x1234_5678);
    assert_eq!(sink.take_samples(8), vec![0x5678, 0x1234]);
    assert_eq!(sink.len(), 0);
}

#[test]
fn push_word_sign_extends_both_halves() {
    // 两半都按二进制补码 i16 解释。Both halves are two's-complement i16.
    let mut sink = AudioSink::new(RATE, 2, 8);
    sink.push_word(0x8000_7FFF);
    assert_eq!(sink.take_samples(8), vec![32_767, -32_768]);
    sink.push_word(0x0000_8000);
    assert_eq!(sink.take_samples(8), vec![-32_768, 0]);
}

#[test]
fn take_samples_preserves_order_and_honours_max() {
    let mut sink = AudioSink::new(RATE, 2, 16);
    sink.push_word(0x0002_0001);
    sink.push_word(0x0004_0003);
    assert_eq!(sink.len(), 4);
    assert_eq!(sink.take_samples(3), vec![1, 2, 3]);
    assert_eq!(sink.len(), 1);
    assert_eq!(sink.take_samples(usize::MAX), vec![4]);
    assert!(sink.is_empty());
}

#[test]
fn capacity_drops_oldest_samples_and_counts() {
    // 容量 4：最后 4 个样本保留，前 2 个被丢弃并计数。
    // Capacity 4: the newest four samples survive, the two oldest are counted
    // as dropped.
    let mut sink = AudioSink::new(RATE, 2, 4);
    sink.push_word(0x0002_0001);
    sink.push_word(0x0004_0003);
    sink.push_word(0x0006_0005);
    assert_eq!(sink.len(), 4);
    assert_eq!(sink.dropped(), 2);
    assert_eq!(sink.take_samples(8), vec![3, 4, 5, 6]);
}

#[test]
fn zero_capacity_sink_drops_every_sample() {
    let mut sink = AudioSink::new(RATE, 2, 0);
    sink.push_word(0x0002_0001);
    assert_eq!(sink.len(), 0);
    assert_eq!(sink.dropped(), 2);
    assert_eq!(sink.take_samples(4), Vec::<i16>::new());
    assert_eq!(sink.capacity(), 0);
}

#[test]
fn clear_discards_samples_but_keeps_drop_counter() {
    let mut sink = AudioSink::new(RATE, 2, 2);
    sink.push_word(0x0002_0001);
    sink.push_word(0x0004_0003);
    let dropped = sink.dropped();
    assert_eq!(dropped, 2);
    sink.clear();
    assert!(sink.is_empty());
    assert_eq!(sink.dropped(), dropped, "clear keeps the dropout count");
    sink.push_word(0x0008_0007);
    assert_eq!(sink.take_samples(8), vec![7, 8]);
}

#[test]
fn empty_take_returns_no_samples() {
    let mut sink = AudioSink::new(RATE, 2, 8);
    assert!(sink.is_empty());
    assert_eq!(sink.take_samples(4), Vec::<i16>::new());
    assert_eq!(sink.dropped(), 0);
    assert_eq!(sink.rate_hz(), RATE);
    assert_eq!(sink.channels(), 2);
}

#[test]
fn tap_clones_share_one_sink() {
    let tap = tap(8);
    let clone = tap.clone();
    clone.push_word(0x0002_0001);
    assert_eq!(tap.len(), 2);
    assert_eq!(clone.take_samples(1), vec![1]);
    assert_eq!(tap.take_samples(8), vec![2]);
    assert_eq!(tap.rate_hz(), RATE);
    assert_eq!(tap.channels(), 2);
}

#[test]
fn pio_cpu_write_into_selected_txf_feeds_tap() {
    let tap = tap(8);
    let mut pio = Pio::new(2);
    pio.set_audio_tap(tap.clone(), 0);
    assert!(pio.write32(PIO_TXF0, 0x0002_0001));
    assert_eq!(tap.take_samples(8), vec![1, 2]);
    // 字仍按正常路径进入 FIFO。The word also entered the FIFO normally.
    assert_eq!(pio.pop_tx(0), Some(0x0002_0001));
}

#[test]
fn pio_without_tap_captures_nothing() {
    // 未安装 tap：压入路径只做 Option 判断，FIFO 行为完全不变。
    // No tap: the push path only runs an Option check and the FIFO is unchanged.
    let mut pio = Pio::new(2);
    assert!(pio.audio_tap().is_none());
    assert!(pio.write32(PIO_TXF0, 0xAAAA_BBBB));
    assert_eq!(pio.pop_tx(0), Some(0xAAAA_BBBB));
    assert!(pio.read32(PIO_TXF0).is_some());
}

#[test]
fn pio_tap_ignores_unselected_state_machine() {
    let tap = tap(8);
    let mut pio = Pio::new(1);
    pio.set_audio_tap(tap.clone(), 1);
    assert!(pio.write32(PIO_TXF0, 0x0002_0001));
    assert!(tap.is_empty(), "SM0 is not the tapped state machine");
    assert!(pio.write32(PIO_TXF0 + 4, 0x0004_0003));
    assert_eq!(tap.take_samples(8), vec![3, 4]);
}

#[test]
fn pio_reset_clears_captured_samples_and_keeps_tap() {
    let tap = tap(8);
    let mut pio = Pio::new(2);
    pio.set_audio_tap(tap.clone(), 0);
    assert!(pio.write32(PIO_TXF0, 0x0002_0001));
    assert_eq!(tap.len(), 2);
    pio.reset();
    assert!(tap.is_empty(), "reset clears the captured stream");
    assert!(pio.audio_tap().is_some(), "the tap wiring survives reset");
    assert!(pio.write32(PIO_TXF0, 0x0006_0005));
    assert_eq!(tap.take_samples(8), vec![5, 6]);
}

/// Bus plus the mounted boot peripheral set / 总线与已挂载的启动外设集合
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

    /// Program a word-mode DMA channel feeding a PIO TX FIFO
    /// 编程一个向 PIO TX FIFO 写入的字宽 DMA 通道
    fn program_tx_channel(&mut self, channel: u32, src: u32, fifo: u32, count: u32, treq: u8) {
        let base = map::DMA_BASE + channel * 0x40;
        let ctrl = CTRL_EN
            | (DATA_SIZE_WORD << CTRL_DATA_SIZE_SHIFT)
            | (u32::from(treq) << CTRL_TREQ_SEL_SHIFT)
            | (channel << CTRL_CHAIN_TO_SHIFT)
            | CTRL_INCR_READ;
        self.write_word(base, src);
        self.write_word(base + 0x04, fifo);
        self.write_word(base + 0x08, count);
        self.write_word(base + 0x0C, ctrl);
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

#[test]
fn dma_writes_into_pio2_txf_are_captured() {
    let mut f = fixture();
    let tap = tap(16);
    assert!(f.peripherals.set_audio_tap(2, 0, tap.clone()));
    let words = [0x0002_0001u32, 0x0004_0003, 0x0006_0005];
    for (index, word) in words.iter().enumerate() {
        f.write_word(RAM_BASE + index as u32 * 4, *word);
    }
    f.program_tx_channel(
        0,
        RAM_BASE,
        map::PIO2_BASE + PIO_TXF0,
        words.len() as u32,
        dreq::PIO2_TX0,
    );
    f.peripherals.sync_pio_dreqs();
    assert_eq!(f.peripherals.dma_mut().run(&mut f.bus, 3), 3);
    assert_eq!(tap.take_samples(16), vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn boot_tap_captures_cpu_and_dma_paths_end_to_end() {
    let mut f = fixture();
    let tap = tap(32);
    assert!(f.peripherals.set_audio_tap(2, 0, tap.clone()));
    assert!(f.peripherals.audio_tap().is_some());
    // CPU 路径：总线写 PIO2 TXF0。CPU path: bus write to PIO2 TXF0.
    f.write_word(map::PIO2_BASE + PIO_TXF0, 0x00FF_FFFE);
    // DMA 路径：两个字经 pump_dma 搬入同一 FIFO。
    // DMA path: two words move into the same FIFO through pump_dma.
    f.write_word(RAM_BASE, 0x0002_0001);
    f.write_word(RAM_BASE + 4, 0x0004_0003);
    f.program_tx_channel(0, RAM_BASE, map::PIO2_BASE + PIO_TXF0, 2, dreq::PIO2_TX0);
    assert_eq!(f.peripherals.pump_dma(&mut f.bus, 8, 64), 2);
    assert_eq!(
        tap.take_samples(32),
        vec![-2, 255, 1, 2, 3, 4],
        "capture keeps CPU then DMA order"
    );
}

#[test]
fn boot_rejects_unknown_pio_block_and_reports_no_tap() {
    let mut f = fixture();
    let tap = tap(4);
    assert!(f.peripherals.audio_tap().is_none(), "mount installs no tap");
    assert!(!f.peripherals.set_audio_tap(3, 0, tap));
    assert!(f.peripherals.audio_tap().is_none());
}
