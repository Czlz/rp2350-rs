//! PIO clock-divider timing tests / PIO 时钟分频计时测试
//!
//! These tests pin the `clk_sys` → PIO clock relationship: one instruction per
//! `CLKDIV_INT + CLKDIV_FRAC/256` system-clock cycles, fractional remainders
//! carried across batches, a free-running divider, and a word rate derived from
//! the program length. Every expectation is computed from the datasheet
//! encoding; no firmware-specific constant is involved.
//! 这些测试锁定 `clk_sys` → PIO 时钟关系：每 `CLKDIV_INT + CLKDIV_FRAC/256` 个
//! 系统时钟周期执行一条指令、小数余数跨批次保留、分频器自由运行，以及由程序
//! 长度推导的字速率。所有期望值都由数据手册编码算出，不含固件专用常量。

// 测试断言直接使用数据手册寄存器常量与块内私有状态，通配导入保持命名直观。
// Tests assert against datasheet register constants and block-private state, so
// glob imports keep the naming faithful to the datasheet.
#![allow(clippy::wildcard_imports)]

use rp2350_core::bus::AddressBus;
use rp2350_core::memory::{RamRegion, SimpleBus};
use rp2350_core::mmio::MmioDevice;

use super::regs::*;
use super::{Pio, SharedPio};
use crate::BootPeripherals;
use crate::dma::{CTRL_DATA_SIZE_SHIFT, CTRL_EN, CTRL_TREQ_SEL_SHIFT, DmaChannelState};
use crate::dreq;
use crate::map;

/// `jmp always, 0`: a one-instruction spin / 单指令自旋
const SPIN: u16 = 0x0000;

/// `pull block` / 阻塞式 `PULL`
const PULL_BLOCK: u16 = 0x80A0;

/// `out pins, 1` with no side-set or delay / 无 side-set/延迟的 `out pins, 1`
const OUT_PIN: u16 = 0x6001;

/// `jmp always, 0` / `jmp always, 0`
const JMP_SELF: u16 = 0x0000;

/// 8-instruction, 32-bit serializer used by the word-rate tests
/// 字速率测试使用的 8 指令 32 位移位程序
///
/// It emits 32 data bits per wrap: two `out` instructions per bit through the
/// two `jmp x--` loops, giving 64 PIO instructions per 32-bit word. The
/// table is a synthetic program, not a copy of any firmware's configuration.
/// 每次回绕输出 32 个数据位：两个 `jmp x--` 循环每比特执行两条 `out`，因此每个
/// 32 位字恰好 64 条 PIO 指令。该表是合成程序，不包含任何固件的配置。
const SERIALIZER: [u16; 8] = [
    0x7001, // 0: out pins, 1   side 2
    0x1840, // 1: jmp x--, 0    side 3
    0x6001, // 2: out pins, 1   side 0
    0xE82E, // 3: set x, 14     side 1
    0x6001, // 4: out pins, 1   side 0
    0x0844, // 5: jmp x--, 4    side 1
    0x7001, // 6: out pins, 1   side 2
    0xF82E, // 7: set x, 14     side 3
];

/// Instructions executed per 32-bit word by [`SERIALIZER`]
/// [`SERIALIZER`] 每 32 位字执行的指令数
const INSTRUCTIONS_PER_WORD: u64 = 64;

/// A PIO block with `SPIN` programmed and SM0 enabled at `clkdiv`
/// 一个已编程 `SPIN` 并在 `clkdiv` 下使能 SM0 的 PIO 块
fn spin_rig(clkdiv: u32) -> Pio {
    let mut pio = Pio::new(0);
    assert!(pio.write32(INSTR_MEM0, u32::from(SPIN)));
    assert!(pio.write32(SM0_CLKDIV, clkdiv));
    assert!(pio.write32(CTRL, 1));
    pio
}

/// Program SM0 as the [`SERIALIZER`] at `clkdiv` / 把 SM0 配置成 `clkdiv` 下的 `SERIALIZER`
fn serializer_rig(clkdiv: u32) -> Pio {
    let mut pio = Pio::new(2);
    for (slot, instr) in SERIALIZER.iter().enumerate() {
        assert!(pio.write32(INSTR_MEM0 + slot as u32 * 4, u32::from(*instr)));
    }
    // WRAP_TOP = 7, WRAP_BOTTOM = 0；SIDESET_COUNT = 2；OUT_COUNT = 1。
    // WRAP_TOP = 7, WRAP_BOTTOM = 0; SIDESET_COUNT = 2; OUT_COUNT = 1.
    assert!(pio.write32(SM0_CLKDIV + SM_EXECCTRL, 7 << EXECCTRL_WRAP_TOP_SHIFT));
    assert!(pio.write32(
        SM0_CLKDIV + SM_PINCTRL,
        (1 << PINCTRL_OUT_COUNT_SHIFT) | (2 << PINCTRL_SIDESET_COUNT_SHIFT)
    ));
    // 自动拉取、32 位阈值、TX FIFO 合并为 8 深度。
    // Autopull, 32-bit threshold, TX FIFO joined to 8 words.
    assert!(pio.write32(
        SM0_CLKDIV + SM_SHIFTCTRL,
        SHIFTCTRL_FJOIN_TX | SHIFTCTRL_AUTOPULL
    ));
    assert!(pio.write32(SM0_CLKDIV, clkdiv));
    // 从 entry_point（pc 7）进入，X = 14，与固件的 `pio_sm_exec` 等价。
    // Enter at entry_point (pc 7) with X = 14, matching `pio_sm_exec`.
    pio.sm[0].x = 14;
    assert!(pio.write32(SM0_CLKDIV + SM_INSTR, u32::from(0x0007u16)));
    assert_eq!(pio.read32(SM0_CLKDIV + SM_ADDR), Some(7));
    assert!(pio.write32(CTRL, 1));
    pio
}

/// Keep the 4-word TX FIFO topped up and return the number of consumed words
/// 保持 4 深度 TX FIFO 有数据，返回被消费的字数
///
/// One word is offered per chunk, far above the consumption rate, so every
/// accepted word stays in the FIFO until the PIO shifts it out. The consumed
/// count is `initial + accepted - final_level`, which is exact.
/// 每个块尝试压入一个字，远高于消费速率，因此被接受的字都留在 FIFO 中直到 PIO
/// 移出。消费数等于 `初始 + 接受 - 最终占用`，是精确值。
fn drain_words(pio: &mut Pio, clk_sys_cycles: u64, chunk: u64) -> u64 {
    const INITIAL: u64 = 4;
    for word in 0..INITIAL {
        assert!(pio.push_tx(0, word as u32));
    }
    let mut accepted = 0u64;
    let mut remaining = clk_sys_cycles;
    while remaining > 0 {
        let step = chunk.min(remaining);
        pio.tick(step);
        remaining -= step;
        if pio.push_tx(0, 0xA5A5_0000) {
            accepted += 1;
        }
    }
    let level = u64::from(pio.read32(FLEVEL).expect("flevel") & 0x0F);
    INITIAL + accepted - level
}

#[test]
fn integer_divider_issues_one_instruction_per_divisor_cycles() {
    // CLKDIV = 4.0：4000 个 clk_sys 恰好 1000 条指令。
    // CLKDIV = 4.0: 4000 clk_sys cycles retire exactly 1000 instructions.
    let mut pio = spin_rig(4 << 16);
    pio.tick(4000);
    assert_eq!(pio.clk_sys_cycles(), 4000);
    assert_eq!(pio.instructions_retired(0), Some(1000));
    assert_eq!(pio.machine_cycles(0), Some(1000));
    // 前三个边沿被分频器拦住，第四个才退休。
    // The first three edges are gated; the fourth retires.
    let mut fresh = spin_rig(4 << 16);
    fresh.tick(3);
    assert_eq!(fresh.instructions_retired(0), Some(0));
    fresh.tick(1);
    assert_eq!(fresh.instructions_retired(0), Some(1));
}

#[test]
fn fractional_divider_keeps_its_remainder_across_batches() {
    // CLKDIV = 2.5：1000 个周期恰好 400 条指令；逐周期推进与整批推进一致。
    // CLKDIV = 2.5: 1000 cycles retire exactly 400 instructions, and stepping
    // one cycle at a time matches one big batch.
    let mut batched = spin_rig((2 << 16) | 0x8000);
    batched.tick(1000);
    assert_eq!(batched.instructions_retired(0), Some(400));

    let mut single = spin_rig((2 << 16) | 0x8000);
    for _ in 0..1000 {
        single.tick(1);
    }
    assert_eq!(single.instructions_retired(0), Some(400));
    assert_eq!(single.machine_cycles(0), Some(400));
}

#[test]
fn fractional_divider_matches_the_rate_within_one_tick() {
    // CLKDIV = 1.5：1e6 个周期 = 666 666.6… 条指令，误差 ≤ 1。
    // CLKDIV = 1.5: 1e6 cycles give 666_666.6… instructions, within one tick.
    let mut pio = spin_rig((1 << 16) | 0x8000);
    pio.tick(1_000_000);
    let retired = pio.instructions_retired(0).expect("sm0");
    assert!(
        retired.abs_diff(666_667) <= 1,
        "expected ~666667 instructions, got {retired}"
    );
}

#[test]
fn clkdiv_write_changes_the_period_and_keeps_the_remainder() {
    // 2.0 分频走了一拍（相位 256/512）后改成 1.5（384）：相位保留，下一拍即退休。
    // After one edge at 2.0 (phase 256/512), writing 1.5 (384) keeps the phase,
    // so the very next cycle retires an instruction.
    let mut pio = spin_rig(2 << 16);
    pio.tick(1);
    assert_eq!(pio.instructions_retired(0), Some(0));
    assert!(pio.write32(SM0_CLKDIV, (1 << 16) | 0x8000));
    assert_eq!(pio.read32(SM0_CLKDIV), Some((1 << 16) | 0x8000));
    pio.tick(1);
    assert_eq!(pio.instructions_retired(0), Some(1));
    // 余数 128 保留在累加器里：再走一拍（384）又退休。
    // The 128 remainder stays in the accumulator: one more cycle (384) retires.
    pio.tick(1);
    assert_eq!(pio.instructions_retired(0), Some(2));
}

#[test]
fn clkdiv_restart_discards_the_current_period() {
    // 4.0 分频走了三拍后 CTRL.CLKDIV_RESTART：需要再等满四拍。
    // Three edges into a 4.0 period, CTRL.CLKDIV_RESTART requires a full new
    // period before the next instruction.
    let mut pio = spin_rig(4 << 16);
    pio.tick(3);
    assert_eq!(pio.instructions_retired(0), Some(0));
    assert!(pio.write32(CTRL, 1 | CLKDIV_RESTART));
    assert_eq!(pio.read32(CTRL), Some(1), "enable survives the strobe");
    pio.tick(3);
    assert_eq!(pio.instructions_retired(0), Some(0));
    pio.tick(1);
    assert_eq!(pio.instructions_retired(0), Some(1));
}

#[test]
fn a_disabled_machine_freezes_while_its_divider_free_runs() {
    // SM1 禁用期间不执行指令，但分频器照常推进；重新使能当拍即可退休。
    // SM1 executes nothing while disabled, but its divider keeps advancing; the
    // first cycle after re-enabling retires.
    let mut pio = Pio::new(0);
    assert!(pio.write32(INSTR_MEM0, u32::from(SPIN)));
    assert!(pio.write32(SM0_CLKDIV, 4 << 16));
    assert!(pio.write32(SM0_CLKDIV + SM_STRIDE, 4 << 16));
    assert!(pio.write32(CTRL, 0b0011));
    pio.tick(4);
    assert_eq!(pio.instructions_retired(0), Some(1));
    assert_eq!(pio.instructions_retired(1), Some(1));

    assert!(pio.write32(CTRL, 0b0001));
    pio.tick(3);
    assert_eq!(pio.instructions_retired(1), Some(1), "disabled SM frozen");
    assert_eq!(pio.machine_cycles(1), Some(1), "no clock edges delivered");

    assert!(pio.write32(CTRL, 0b0011));
    pio.tick(1);
    assert_eq!(pio.instructions_retired(1), Some(2), "divider free-ran");
    assert_eq!(
        pio.instructions_retired(0),
        Some(2),
        "SM0 only ran one edge"
    );
}

#[test]
fn machines_keep_independent_dividers() {
    // SM0 = 1.0、SM1 = 3.0：同样 300 个周期分别退休 300 / 100 条指令。
    // SM0 = 1.0 and SM1 = 3.0: 300 cycles retire 300 / 100 instructions.
    let mut pio = Pio::new(0);
    assert!(pio.write32(INSTR_MEM0, u32::from(JMP_SELF)));
    assert!(pio.write32(SM0_CLKDIV, 1 << 16));
    assert!(pio.write32(SM0_CLKDIV + SM_STRIDE, 3 << 16));
    assert!(pio.write32(CTRL, 0b0011));
    pio.tick(300);
    assert_eq!(pio.instructions_retired(0), Some(300));
    assert_eq!(pio.instructions_retired(1), Some(100));
}

#[test]
fn instruction_delay_costs_divisor_cycles_per_delay_slot() {
    // CLKDIV = 2.0，`jmp 0 [3]`：每条指令占 (1 + 3) 个机器周期 = 8 个 clk_sys。
    // CLKDIV = 2.0 with `jmp 0 [3]`: each instruction takes (1 + 3) machine
    // cycles = 8 clk_sys cycles.
    let mut pio = Pio::new(0);
    assert!(pio.write32(INSTR_MEM0, u32::from(JMP_SELF | (3 << 8))));
    assert!(pio.write32(SM0_CLKDIV, 2 << 16));
    assert!(pio.write32(CTRL, 1));
    pio.tick(8000);
    assert_eq!(pio.instructions_retired(0), Some(1000));
    assert_eq!(
        pio.machine_cycles(0),
        Some(4000),
        "delay burns machine cycles"
    );
}

#[test]
fn a_stalled_instruction_retries_every_machine_cycle() {
    // TX 为空时 `pull block` 每个机器周期重试；压入数据后当拍退休。
    // An empty TX FIFO makes `pull block` retry every machine cycle; a push
    // retires it on the next edge.
    let mut pio = Pio::new(0);
    assert!(pio.write32(INSTR_MEM0, u32::from(PULL_BLOCK)));
    assert!(pio.write32(SM0_CLKDIV, 4 << 16));
    assert!(pio.write32(CTRL, 1));
    pio.tick(400);
    assert_eq!(pio.machine_cycles(0), Some(100), "edges keep arriving");
    assert_eq!(pio.instructions_retired(0), Some(0));
    assert!(pio.push_tx(0, 0x1234_5678));
    pio.tick(4);
    assert_eq!(pio.instructions_retired(0), Some(1));
}

#[test]
fn serializer_word_rate_is_13440_at_252_mhz() {
    // 252 MHz、CLKDIV = 252e6 / (13440 * 64) = 292.96875：1/128 秒（1 968 750 个
    // clk_sys）应产出 105 个字，即 13 440 字/秒。
    // 252 MHz with CLKDIV = 252e6 / (13440 * 64) = 292.96875: 1/128 s
    // (1_968_750 clk_sys cycles) must produce 105 words = 13_440 words/s.
    let mut pio = serializer_rig((292 << 16) | (248 << 8));
    let words = drain_words(&mut pio, 1_968_750, 1024);
    assert!(
        words.abs_diff(105) <= 1,
        "expected 105 words (13440/s), got {words}"
    );
    let retired = pio.instructions_retired(0).expect("sm0");
    assert_eq!(
        retired / INSTRUCTIONS_PER_WORD,
        words,
        "one word per 64 retired instructions"
    );
}

#[test]
fn word_rate_scales_inversely_with_the_divisor() {
    // 同一位流程序在 4 倍分频下应产出 1/4 的字数（允许一个字的量化误差）。
    // The same serializer at four times the divisor must produce a quarter of
    // the words (within one word of quantization).
    let window = 3_000_000u64;
    let mut fast = serializer_rig(64 << 16);
    let fast_words = drain_words(&mut fast, window, 4096);
    let mut slow = serializer_rig(256 << 16);
    let slow_words = drain_words(&mut slow, window, 4096);
    assert!(
        fast_words.abs_diff(4 * slow_words) <= 4,
        "4x divisor must give a quarter of the words: {fast_words} vs {slow_words}"
    );
}

#[test]
fn shared_handle_counters_follow_tick_batches() {
    // SharedPio 禁用时不推进也不计数；使能后每个 clk_sys 周期计入一次。
    // A disabled SharedPio neither advances nor counts; once enabled every
    // clk_sys cycle is counted.
    let mut bus = SimpleBus::new();
    let peripherals = BootPeripherals::mount(&mut bus).expect("mount");
    let mut cycles = 0u64;
    bus.write_u32(map::PIO0_BASE + INSTR_MEM0, u32::from(SPIN), &mut cycles)
        .expect("imem");
    let pio: &SharedPio = peripherals.pio(0).expect("PIO0");
    pio.tick(64);
    assert_eq!(pio.clk_sys_cycles(), 0);
    assert_eq!(pio.instructions_retired(0), Some(0));

    bus.write_u32(map::PIO0_BASE + CTRL, 1, &mut cycles)
        .expect("ctrl");
    pio.tick(64);
    assert_eq!(pio.clk_sys_cycles(), 64);
    assert_eq!(pio.machine_cycles(0), Some(64));
    assert_eq!(pio.instructions_retired(0), Some(64));
    assert_eq!(pio.machine_cycles(usize::MAX), None);
    assert_eq!(pio.instructions_retired(usize::MAX), None);
}

#[test]
fn zero_length_tick_is_a_no_op() {
    // tick(0) 不推进、不计数；复位后计数器与分频值回到复位值。
    // tick(0) advances and counts nothing; reset restores counters and CLKDIV.
    let mut pio = spin_rig(8 << 16);
    pio.tick(0);
    assert_eq!(pio.clk_sys_cycles(), 0);
    assert_eq!(pio.instructions_retired(0), Some(0));
    pio.tick(80);
    assert_eq!(pio.instructions_retired(0), Some(10));
    MmioDevice::reset(&mut pio);
    assert_eq!(pio.clk_sys_cycles(), 0);
    assert_eq!(pio.instructions_retired(0), Some(0));
    assert_eq!(pio.read32(SM0_CLKDIV), Some(CLKDIV_RESET));
}

/// Fixture: bus with RAM plus the mounted boot peripherals
/// 夹具：带 RAM 的总线与已挂载的启动外设
struct DmaFixture {
    bus: SimpleBus,
    peripherals: BootPeripherals,
}

impl DmaFixture {
    /// Create the fixture / 创建夹具
    fn new() -> Self {
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(RamRegion::new("ram", 0x2000_0000, 0x100)))
            .expect("ram");
        let peripherals = BootPeripherals::mount(&mut bus).expect("mount");
        Self { bus, peripherals }
    }

    /// Write one word through the bus / 通过总线写一个字
    fn write(&mut self, address: u32, value: u32) {
        let mut cycles = 0u64;
        self.bus
            .write_u32(address, value, &mut cycles)
            .expect("bus write");
    }

    /// Read one word through the bus / 通过总线读一个字
    fn read(&mut self, address: u32) -> u32 {
        let mut cycles = 0u64;
        self.bus.read_u32(address, &mut cycles).expect("bus read")
    }

    /// Program a word-mode DMA channel paced by `treq` into PIO0 TX0
    /// 编程一个由 `treq` 节拍、写入 PIO0 TX0 的字宽 DMA 通道
    fn program_tx_dma(&mut self, count: u32) {
        let base = map::DMA_BASE;
        self.write(base, 0x2000_0000);
        self.write(base + 0x04, map::PIO0_BASE + TXF0);
        self.write(base + 0x08, count);
        let ctrl = CTRL_EN
            | (2 << CTRL_DATA_SIZE_SHIFT)
            | (u32::from(dreq::PIO0_TX0) << CTRL_TREQ_SEL_SHIFT);
        self.write(base + 0x0C, ctrl);
    }

    /// Current SM0 TX occupancy / 当前 SM0 TX 占用
    fn level(&mut self) -> u64 {
        u64::from(self.read(map::PIO0_BASE + FLEVEL) & 0x0F)
    }

    /// Channel 0 snapshot / 通道 0 快照
    fn channel(&self) -> DmaChannelState {
        self.peripherals.dma().channel_state(0).expect("channel")
    }
}

/// Program PIO0 SM0 so every `out pins, 1` pulls one word (threshold 1 bit)
/// 配置 PIO0 SM0，使每条 `out pins, 1` 拉取一个字（阈值 1 位）
fn program_bit_consumer(fixture: &mut DmaFixture, clkdiv: u32) {
    fixture.write(map::PIO0_BASE + INSTR_MEM0, u32::from(OUT_PIN));
    fixture.write(map::PIO0_BASE + INSTR_MEM0 + 4, u32::from(JMP_SELF));
    fixture.write(
        map::PIO0_BASE + SM0_CLKDIV + SM_PINCTRL,
        1 << PINCTRL_OUT_COUNT_SHIFT,
    );
    fixture.write(
        map::PIO0_BASE + SM0_CLKDIV + SM_SHIFTCTRL,
        SHIFTCTRL_FJOIN_TX | SHIFTCTRL_AUTOPULL | (1 << SHIFTCTRL_PULL_THRESH_SHIFT),
    );
    fixture.write(map::PIO0_BASE + SM0_CLKDIV, clkdiv);
    fixture.write(map::PIO0_BASE + CTRL, 1);
}

/// Words drained by DMA over `chunks` batches of 64 clk_sys cycles
/// 在 `chunks` 个 64 周期批次内被 DMA 搬入后由 PIO 消费的字数
fn dma_drained_words(fixture: &mut DmaFixture, clkdiv: u32, chunks: u64) -> u64 {
    program_bit_consumer(fixture, clkdiv);
    fixture.program_tx_dma(1_000_000);
    let mut moved = 0u64;
    for _ in 0..chunks {
        fixture.peripherals.sync_pio_dreqs();
        moved += fixture.peripherals.pump_dma(&mut fixture.bus, 64, 64);
        fixture.peripherals.pio(0).expect("pio0").tick(64);
    }
    let level = fixture.level();
    assert_eq!(fixture.channel().trans_count, 1_000_000 - moved as u32);
    moved - level
}

#[test]
fn dma_dreq_cadence_follows_the_pio_rate() {
    // `out pins,1`（1 位阈值）每两个指令消费一个字：CLKDIV 16 → 每 32 个
    // clk_sys 一个字，CLKDIV 32 → 每 64 个周期一个字，DMA 搬运量随之减半。
    // `out pins,1` with a 1-bit threshold consumes one word per two
    // instructions: CLKDIV 16 gives one word per 32 clk_sys cycles, CLKDIV 32
    // one per 64 cycles, so the DMA transfer count halves with the rate.
    let mut fast = DmaFixture::new();
    let fast_words = dma_drained_words(&mut fast, 16 << 16, 500);
    let mut slow = DmaFixture::new();
    let slow_words = dma_drained_words(&mut slow, 32 << 16, 500);
    assert!(
        fast_words.abs_diff(2 * slow_words) <= 4,
        "DMA must follow the PIO rate: {fast_words} vs {slow_words}"
    );
    assert!(
        fast_words.abs_diff(1000) <= 4,
        "500 batches x 64 cycles at 32 cycles/word = 1000 words, got {fast_words}"
    );
}
