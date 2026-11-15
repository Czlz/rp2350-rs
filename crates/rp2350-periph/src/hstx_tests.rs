//! HSTX register, FIFO and command-model tests / HSTX 寄存器、FIFO 与命令模型测试

use rp2350_core::bus::AddressBus;
use rp2350_core::memory::SimpleBus;

use crate::boot::BootPeripherals;
use crate::hstx::{
    CMD_NOP, CMD_OPCODE_SHIFT, CMD_RAW, CMD_RAW_REPEAT, CMD_TMDS, CMD_TMDS_REPEAT, CMD_WAIT,
    CSR_EN, CSR_EXPAND_EN, FIFO_DEPTH, FIFO_STAT_EMPTY, FIFO_STAT_FULL, FIFO_STAT_LEVEL_MASK,
    FIFO_STAT_WOF, Hstx, IRQ_UNDERRUN, IRQ_UNKNOWN_COMMAND,
};
use crate::irq::IrqAssert;
use crate::map;

fn new_hstx() -> Hstx {
    Hstx::new(IrqAssert::new())
}

fn write_ctrl(hstx: &Hstx, offset: u32, value: u32) {
    let mut device = hstx.ctrl_device();
    assert!(
        device.write32(offset, value),
        "control write at {offset:#X} must be modelled"
    );
}

fn read_ctrl(hstx: &Hstx, offset: u32) -> u32 {
    let mut device = hstx.ctrl_device();
    device
        .read32(offset)
        .expect("control register must be modelled")
}

fn write_fifo_reg(hstx: &Hstx, offset: u32, value: u32) {
    let mut device = hstx.fifo_device();
    assert!(
        device.write32(offset, value),
        "FIFO write at {offset:#X} must be modelled"
    );
}

fn read_fifo_reg(hstx: &Hstx, offset: u32) -> u32 {
    let mut device = hstx.fifo_device();
    device
        .read32(offset)
        .expect("FIFO register must be modelled")
}

fn enable(hstx: &Hstx, expander: bool) {
    let csr = if expander {
        CSR_EN | CSR_EXPAND_EN
    } else {
        CSR_EN
    };
    write_ctrl(hstx, 0x00, csr);
}

#[test]
fn control_and_fifo_reset_values_match_the_datasheet() {
    let hstx = new_hstx();
    assert_eq!(read_ctrl(&hstx, 0x00), 0x1005_0600);
    assert_eq!(read_ctrl(&hstx, 0x04), 0, "BIT0 resets to zero");
    assert_eq!(read_ctrl(&hstx, 0x24), 0x0100_0100);
    assert_eq!(read_ctrl(&hstx, 0x28), 0);
    assert_eq!(read_fifo_reg(&hstx, 0x00), FIFO_STAT_EMPTY);
    assert!(!hstx.enabled());
    assert_eq!(hstx.fifo_level(), 0);
    assert_eq!(hstx.output_level(), 0);
    assert_eq!(hstx.output_underrun(), 0);
}

#[test]
fn enable_and_expander_bits_are_read_write() {
    let hstx = new_hstx();
    write_ctrl(&hstx, 0x00, CSR_EN | CSR_EXPAND_EN);
    assert_eq!(
        read_ctrl(&hstx, 0x00),
        CSR_EN | CSR_EXPAND_EN,
        "CSR is a fully writable RW register"
    );
    assert!(hstx.enabled());
    write_ctrl(&hstx, 0x00, 0);
    assert_eq!(read_ctrl(&hstx, 0x00) & (CSR_EN | CSR_EXPAND_EN), 0);
    assert!(!hstx.enabled());
}

#[test]
fn fifo_reports_depth_full_and_empty_and_sets_wof() {
    let hstx = new_hstx();
    for word in 0..FIFO_DEPTH as u32 {
        write_fifo_reg(&hstx, 0x04, word);
        assert_eq!(u32::from(hstx.fifo_level()), word + 1);
    }
    let stat = read_fifo_reg(&hstx, 0x00);
    assert_eq!(stat & FIFO_STAT_LEVEL_MASK, FIFO_DEPTH as u32);
    assert_ne!(stat & FIFO_STAT_FULL, 0);
    assert_eq!(stat & FIFO_STAT_EMPTY, 0);

    // 第 9 个字被丢弃，FIFO 保持满并置位 WOF。
    // The ninth word is dropped, the FIFO stays full and WOF is set.
    write_fifo_reg(&hstx, 0x04, 0xBAD);
    assert_eq!(u32::from(hstx.fifo_level()), FIFO_DEPTH as u32);
    assert_ne!(read_fifo_reg(&hstx, 0x00) & FIFO_STAT_WOF, 0);

    write_fifo_reg(&hstx, 0x00, FIFO_STAT_WOF);
    assert_eq!(read_fifo_reg(&hstx, 0x00) & FIFO_STAT_WOF, 0);
}

#[test]
fn raw_command_emits_the_requested_number_of_words() {
    let mut hstx = new_hstx();
    enable(&hstx, true);
    assert!(hstx.push_command(CMD_RAW | 3));
    for word in [0x11, 0x22, 0x33] {
        assert!(hstx.push_data(word));
    }
    // Each RAW word occupies one pixel period (2 clk_sys at CLKDIV=1).
    // CLKDIV=1 时每个 RAW 字占一个像素周期（2 个 clk_sys）。
    hstx.advance(8);
    assert_eq!(hstx.drain_output(), [0x11, 0x22, 0x33]);
    assert_eq!(hstx.fifo_level(), 0);
    assert_eq!(hstx.output_level(), 0);
}

#[test]
fn raw_repeat_emits_one_word_repeated() {
    let mut hstx = new_hstx();
    enable(&hstx, true);
    assert!(hstx.push_command(CMD_RAW_REPEAT | 4));
    assert!(hstx.push_data(0xABC));
    hstx.advance(2);
    assert_eq!(hstx.take_scanline(), [0xABC; 4]);
}

#[test]
fn wait_command_produces_no_pixels_and_pauses_the_fifo() {
    let mut hstx = new_hstx();
    enable(&hstx, true);
    assert!(hstx.push_command(CMD_WAIT | 5));
    assert!(hstx.push_command(CMD_NOP));
    hstx.advance(2);
    assert_eq!(hstx.output_level(), 0);
    assert_eq!(hstx.fifo_level(), 1, "NOP waits behind the delay");
    hstx.advance(10);
    assert_eq!(hstx.fifo_level(), 0);
    assert_eq!(hstx.output_level(), 0);
    assert_eq!(hstx.output_underrun(), 0);
}

#[test]
fn fifo_underrun_counts_and_asserts_the_shared_irq() {
    let irq = IrqAssert::new();
    let mut hstx = Hstx::new(irq.clone());
    enable(&hstx, true);
    write_ctrl(&hstx, 0x34, IRQ_UNDERRUN);

    assert!(hstx.push_command(CMD_RAW | 4));
    assert!(hstx.push_data(1));
    assert!(hstx.push_data(2));
    // Word 1 fills t=0..2, word 2 fills t=2..4; the starve is visible once the
    // budget reaches t=4. / 第 1 字占 0..2、第 2 字占 2..4，预算到 4 才可见断流。
    hstx.advance(4);

    assert_eq!(hstx.output_underrun(), 1);
    assert_eq!(hstx.output_level(), 2);
    assert_eq!(read_ctrl(&hstx, 0x30) & IRQ_UNDERRUN, IRQ_UNDERRUN);
    assert_ne!(irq.mask() & (1 << map::HSTX_IRQ), 0);

    // W1C 清状态后共享中断线释放。Clearing W1C releases the shared line.
    write_ctrl(&hstx, 0x30, IRQ_UNDERRUN);
    assert_eq!(irq.mask() & (1 << map::HSTX_IRQ), 0);

    assert!(hstx.push_data(3));
    assert!(hstx.push_data(4));
    hstx.advance(4);
    assert_eq!(hstx.drain_output(), [1, 2, 3, 4]);
    assert_eq!(hstx.output_underrun(), 1);
}

#[test]
fn unknown_commands_are_diagnosed_without_panicking() {
    let mut hstx = new_hstx();
    enable(&hstx, true);
    let bogus = 0x5 << CMD_OPCODE_SHIFT | 1;
    assert!(hstx.push_command(bogus));
    hstx.advance(1);

    assert_eq!(hstx.unknown_commands(), 1);
    assert_eq!(hstx.last_unknown_command(), Some(bogus));
    assert_eq!(
        read_ctrl(&hstx, 0x30) & IRQ_UNKNOWN_COMMAND,
        IRQ_UNKNOWN_COMMAND
    );
    assert_eq!(hstx.output_level(), 0);

    // 后续合法命令仍正常执行。A later valid command still runs normally.
    assert!(hstx.push_command(CMD_RAW | 1));
    assert!(hstx.push_data(0x55));
    hstx.advance(2);
    assert_eq!(hstx.drain_output(), [0x55]);
}

#[test]
fn draining_output_leaves_the_fifo_empty() {
    let mut hstx = new_hstx();
    enable(&hstx, true);
    assert!(hstx.push_command(CMD_RAW | 2));
    assert!(hstx.push_data(0xA));
    assert!(hstx.push_data(0xB));
    hstx.advance(3);

    assert_eq!(hstx.fifo_level(), 0);
    assert_eq!(
        read_fifo_reg(&hstx, 0x00) & FIFO_STAT_EMPTY,
        FIFO_STAT_EMPTY
    );
    assert_eq!(hstx.drain_output(), [0xA, 0xB]);
    assert_eq!(hstx.output_level(), 0);
    assert_eq!(hstx.drain_output(), Vec::<u32>::new());
}

#[test]
fn reserved_and_read_only_bits_ignore_writes() {
    let hstx = new_hstx();
    write_ctrl(&hstx, 0x00, 0xFFFF_FFFF);
    assert_eq!(read_ctrl(&hstx, 0x00), 0xFF1F_1F73);
    write_ctrl(&hstx, 0x04, 0xFFFF_FFFF);
    assert_eq!(read_ctrl(&hstx, 0x04), 0x0003_1F1F);

    // FIFO STAT 的 LEVEL/FULL/EMPTY 只读，写 1 只清 WOF。
    // FIFO STAT LEVEL/FULL/EMPTY are read-only; writing 1 only clears WOF.
    write_fifo_reg(&hstx, 0x00, 0xFFFF_FFFF);
    let stat = read_fifo_reg(&hstx, 0x00);
    assert_eq!(stat & FIFO_STAT_LEVEL_MASK, 0);
    assert_ne!(stat & FIFO_STAT_EMPTY, 0);
    assert_eq!(stat & FIFO_STAT_WOF, 0, "W1C clears an already-clear flag");

    // FIFO 数据寄存器只写，读回 0。
    // The FIFO data register is write-only and reads as zero.
    assert_eq!(read_fifo_reg(&hstx, 0x04), 0);
}

#[test]
fn expander_disabled_passes_raw_words_to_the_host() {
    let mut hstx = new_hstx();
    enable(&hstx, false);
    assert!(hstx.push_data(0xDEAD_BEEF));
    // Raw words are paced by the shift register: N_SHIFTS = 0 → 1, CLKDIV = 0 →
    // 1, so one word lasts 1 * 2 * 1 = 2 clk_sys and reaches the host when its
    // shift period completes.
    // 原始字按移位寄存器计时：N_SHIFTS = 0 → 1、CLKDIV = 0 → 1，因此每字占
    // 1 * 2 * 1 = 2 个 clk_sys，移位周期结束后才交给宿主。
    hstx.advance(2);
    assert_eq!(hstx.drain_output(), [0xDEAD_BEEF]);
}

#[test]
fn reset_clears_fifos_irq_and_counters() {
    let irq = IrqAssert::new();
    let mut hstx = Hstx::new(irq.clone());
    enable(&hstx, true);
    write_ctrl(&hstx, 0x34, IRQ_UNDERRUN);
    assert!(hstx.push_command(CMD_RAW | 4));
    assert!(hstx.push_data(1));
    hstx.advance(3);
    assert_eq!(hstx.output_underrun(), 1);
    assert_ne!(irq.mask() & (1 << map::HSTX_IRQ), 0);

    hstx.reset();
    assert_eq!(read_ctrl(&hstx, 0x00), 0x1005_0600);
    assert_eq!(hstx.fifo_level(), 0);
    assert_eq!(hstx.output_level(), 0);
    assert_eq!(hstx.output_underrun(), 0);
    assert_eq!(irq.mask() & (1 << map::HSTX_IRQ), 0);
}

#[test]
fn tmds_data_words_follow_the_pixel_clock() {
    let mut hstx = new_hstx();
    // CLKDIV = 5 → pixel clock = clk_hstx/5 = clk_sys/10; ENC_SHIFT = 16 bits
    // → 2 pixels per FIFO word → 20 clk_sys per word.
    // CLKDIV = 5：像素时钟 = clk_hstx/5 = clk_sys/10；ENC_SHIFT = 16 位
    // → 每字 2 像素 → 每字 20 个 clk_sys。
    write_ctrl(&hstx, 0x00, CSR_EN | CSR_EXPAND_EN | (5 << 28));
    write_ctrl(&hstx, 0x24, (2 << 24) | (16 << 16));
    assert!(hstx.push_command(CMD_TMDS | 4));
    assert!(hstx.push_data(0xA1));
    assert!(hstx.push_data(0xB2));

    hstx.advance(19);
    assert_eq!(hstx.output_level(), 1, "first word covers t=0..20");
    hstx.advance(2);
    assert_eq!(hstx.output_level(), 2, "second word covers t=20..40");
    assert_eq!(hstx.drain_output(), [0xA1, 0xB2]);
    assert_eq!(hstx.output_underrun(), 0);
}

/// The firmware's 256-word active scanline block / 固件的 256 字有效扫描行块
fn scanline_block() -> Vec<u32> {
    let mut block = vec![0u32; 256];
    block[0] = CMD_RAW_REPEAT | 0x10;
    block[1] = 0x1111_1111;
    block[2] = CMD_NOP;
    block[3] = CMD_RAW_REPEAT | 0x60;
    block[4] = 0x2222_2222;
    block[5] = CMD_NOP;
    block[6] = CMD_RAW_REPEAT | 0x30;
    block[7] = 0x3333_3333;
    block[8] = CMD_TMDS_REPEAT | 0x50;
    block[9] = 0;
    block[10] = CMD_TMDS | 0x1E0;
    for (index, word) in block.iter_mut().enumerate().take(251).skip(11) {
        *word = index as u32 | 0x0100_0000;
    }
    block[251] = CMD_TMDS_REPEAT | 0x4C;
    block[252] = 0;
    block[253] = CMD_TMDS | 4;
    block[254] = 0;
    block[255] = 0;
    block
}

#[test]
fn a_scanline_block_streams_for_800_pixel_periods() {
    let mut hstx = new_hstx();
    write_ctrl(&hstx, 0x00, CSR_EN | CSR_EXPAND_EN | (5 << 28));
    write_ctrl(&hstx, 0x24, (2 << 24) | (16 << 16) | (1 << 8));
    let block = scanline_block();

    for lap in 0..3 {
        let start = hstx.output_words();
        let mut fed = 0usize;
        let mut used = 0u64;
        while used < 8000 {
            while fed < block.len() && hstx.fifo_level() < FIFO_DEPTH as u8 {
                assert!(hstx.push_command(block[fed]), "lap {lap} push {fed}");
                fed += 1;
            }
            hstx.advance(1);
            used += 1;
        }
        assert_eq!(fed, block.len(), "lap {lap} feeds the whole block");
        assert_eq!(
            hstx.output_words() - start,
            block.len() as u64,
            "lap {lap} pops the whole block"
        );
        let _ = hstx.drain_output();
    }

    assert_eq!(
        hstx.unknown_commands(),
        0,
        "command/data phase stays aligned"
    );
    assert_eq!(hstx.output_underrun(), 0, "the FIFO never runs dry");
}

/// Clock tree configured by the firmware: 252 MHz `CLK_SYS`, 126 MHz `CLK_HSTX`
/// 固件配置的时钟树：`CLK_SYS` 252 MHz、`CLK_HSTX` 126 MHz
fn tree_252_126() -> crate::clock_tree::ClockTree {
    use crate::clock_tree::{CLK_HSTX, ClockTree, PllId};
    let tree = ClockTree::new();
    tree.write_pll(PllId::Sys, 0x08, 126);
    tree.write_pll(PllId::Sys, 0x0C, (6 << 16) | (1 << 12));
    tree.write_channel_div(CLK_HSTX, 2 << 16);
    assert_eq!(tree.sysclk_hz(), 252_000_000);
    assert_eq!(tree.hstxclk_hz(), 126_000_000);
    tree
}

#[test]
fn the_scanline_block_keeps_800_pixel_periods_at_252_126_mhz() {
    // 时钟树配置为 252/126 MHz、CLKDIV=5：像素周期 = 5 * 252/126 = 10 个
    // clk_sys，256 字行块仍是 800 像素 = 8 000 个 clk_sys 周期。
    // With the tree at 252/126 MHz and CLKDIV=5 the pixel period is
    // 5 * 252/126 = 10 clk_sys cycles, so a 256-word block still spans 800
    // pixels = 8000 clk_sys cycles.
    let mut hstx = Hstx::with_clock(IrqAssert::new(), tree_252_126());
    write_ctrl(&hstx, 0x00, CSR_EN | CSR_EXPAND_EN | (5 << 28));
    write_ctrl(&hstx, 0x24, (2 << 24) | (16 << 16) | (1 << 8));
    let block = scanline_block();
    let mut fed = 0usize;
    let mut used = 0u64;
    while used < 8_000 {
        while fed < block.len() && hstx.fifo_level() < FIFO_DEPTH as u8 {
            assert!(hstx.push_command(block[fed]), "push {fed}");
            fed += 1;
        }
        hstx.advance(1);
        used += 1;
    }
    assert_eq!(fed, block.len(), "the whole block is fed");
    assert_eq!(hstx.output_words(), block.len() as u64);
    assert_eq!(hstx.output_underrun(), 0);
}

#[test]
fn a_different_hstx_divider_changes_the_pixel_period() {
    // 252 MHz / CLK_HSTX=/4 = 63 MHz、CLKDIV=5：像素周期 = 20 个 clk_sys，
    // 同一个 256 字块需要 16 000 个周期，证明时序来自 CLK_HSTX 通道。
    // 252 MHz with CLK_HSTX /4 = 63 MHz and CLKDIV=5 gives 20 clk_sys per
    // pixel, so the same 256-word block needs 16 000 cycles — proving the
    // timing comes from the CLK_HSTX channel.
    use crate::clock_tree::CLK_HSTX;
    let tree = tree_252_126();
    tree.write_channel_div(CLK_HSTX, 4 << 16);
    assert_eq!(tree.hstxclk_hz(), 63_000_000);
    let mut hstx = Hstx::with_clock(IrqAssert::new(), tree);
    write_ctrl(&hstx, 0x00, CSR_EN | CSR_EXPAND_EN | (5 << 28));
    write_ctrl(&hstx, 0x24, (2 << 24) | (16 << 16) | (1 << 8));
    let block = scanline_block();
    let mut fed = 0usize;
    let mut used = 0u64;
    while used < 16_000 {
        while fed < block.len() && hstx.fifo_level() < FIFO_DEPTH as u8 {
            assert!(hstx.push_command(block[fed]), "push {fed}");
            fed += 1;
        }
        hstx.advance(1);
        used += 1;
    }
    assert_eq!(fed, block.len(), "the whole block is fed");
    assert_eq!(hstx.output_words(), block.len() as u64);
    assert_eq!(hstx.output_underrun(), 0);
}

#[test]
fn a_dma_burst_cannot_overrun_the_hstx_fifo() {
    // The HSTX FIFO is 8 words deep, so a 64-unit pump must stop at 8 and never
    // set WOF: the budget is capped at the FIFO's free space, and the words that
    // would not fit stay pending in the channel.
    // HSTX FIFO 只有 8 字深，因此 64 单元的泵送必须停在 8 且不置 WOF：预算被限制
    // 为 FIFO 空闲空间，放不下的字留在通道里。
    use crate::dma::{CTRL_DATA_SIZE_SHIFT, CTRL_EN, CTRL_INCR_READ, CTRL_TREQ_SEL_SHIFT};
    use rp2350_core::memory::RamRegion;

    const RAM_BASE: u32 = 0x2000_0000;
    const SOURCE_WORDS: u32 = 64;

    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("sram", RAM_BASE, 0x1000)))
        .expect("ram");
    let mut peripherals = BootPeripherals::mount(&mut bus).expect("mount");
    let mut cycles = 0u64;
    for index in 0..SOURCE_WORDS {
        bus.write_u32(RAM_BASE + index * 4, 0xC0DE_0000 | index, &mut cycles)
            .expect("source word");
    }

    let ctrl = CTRL_EN
        | CTRL_INCR_READ
        | (2 << CTRL_DATA_SIZE_SHIFT)
        | (u32::from(crate::dreq::HSTX_DREQ) << CTRL_TREQ_SEL_SHIFT);
    let base = map::DMA_BASE;
    bus.write_u32(base, RAM_BASE, &mut cycles)
        .expect("read addr");
    bus.write_u32(base + 0x04, map::HSTX_FIFO_BASE + 0x04, &mut cycles)
        .expect("write addr");
    bus.write_u32(base + 0x08, SOURCE_WORDS, &mut cycles)
        .expect("count");
    bus.write_u32(base + 0x0C, ctrl, &mut cycles).expect("ctrl");

    let moved = peripherals.pump_dma(&mut bus, 64, 64);
    assert_eq!(
        moved, FIFO_DEPTH as u64,
        "only the free slots may be filled"
    );
    assert_eq!(
        u32::from(peripherals.hstx().fifo_level()),
        FIFO_DEPTH as u32
    );
    assert!(!peripherals.hstx().fifo_overrun(), "WOF must stay clear");
    let channel = peripherals.dma().channel_state(0).expect("ch0");
    assert!(channel.busy, "the channel keeps its remaining count");
    assert_eq!(channel.trans_count, SOURCE_WORDS - FIFO_DEPTH as u32);
}

#[test]
fn boot_peripherals_mounts_both_hstx_windows() {
    let mut bus = SimpleBus::new();
    let mut peripherals = BootPeripherals::mount(&mut bus).expect("mount");
    let mut cycles = 0u64;

    assert_eq!(
        bus.read_u32(map::HSTX_CTRL_BASE, &mut cycles)
            .expect("control read"),
        0x1005_0600
    );
    assert_eq!(
        bus.read_u32(map::HSTX_FIFO_BASE, &mut cycles)
            .expect("FIFO read"),
        FIFO_STAT_EMPTY
    );

    bus.write_u32(map::HSTX_CTRL_BASE, CSR_EN, &mut cycles)
        .expect("enable");
    bus.write_u32(map::HSTX_FIFO_BASE + 0x04, 0x1234_5678, &mut cycles)
        .expect("FIFO push");
    assert_eq!(peripherals.hstx().fifo_level(), 1);

    peripherals.advance(1);
    // HSTX is host-pumped at the DMA batch cadence; advance it explicitly. The
    // raw path needs N_SHIFTS(0 → 1) * 2 * CLKDIV(0 → 1) = 2 clk_sys per word.
    // HSTX 由宿主按 DMA 批次节拍推进；测试中显式推进。原始路径每字需要
    // N_SHIFTS(0 → 1) * 2 * CLKDIV(0 → 1) = 2 个 clk_sys。
    peripherals.hstx_mut().advance(2);
    assert_eq!(peripherals.hstx_mut().drain_output(), [0x1234_5678]);
}

/// The GBA firmware's HSTX/DVI setup / GBA 固件的 HSTX/DVI 配置
///
/// `csr = EXPAND_EN | CLKDIV=5 | N_SHIFTS=5 | SHIFT=2 | EN`,
/// `expand_shift = ENC_N_SHIFTS=2 | ENC_SHIFT=16`,
/// `expand_tmds = L2 red 5b @ rot 8 | L1 green 6b @ rot 3 | L0 blue 5b @ rot 29`
/// (field positions from the RP2350 `hstx_ctrl.h` header).
fn load_gba_display_config(hstx: &Hstx) {
    write_ctrl(
        hstx,
        0x00,
        CSR_EN | CSR_EXPAND_EN | (5 << 28) | (5 << 16) | (2 << 8),
    );
    write_ctrl(hstx, 0x24, (2 << 24) | (16 << 16));
    write_ctrl(
        hstx,
        0x28,
        (4 << 21) | (8 << 16) | (5 << 13) | (3 << 8) | (4 << 5) | 0x001d,
    );
}

#[test]
fn raw_passthrough_words_take_n_shifts_times_two_clkdiv_cycles() {
    let mut hstx = new_hstx();
    // N_SHIFTS = 5, CLKDIV = 5 → 5 * 2 * 5 = 50 clk_sys per raw word.
    // N_SHIFTS = 5、CLKDIV = 5 → 每个原始字 5 * 2 * 5 = 50 个 clk_sys。
    write_ctrl(&hstx, 0x00, CSR_EN | (5 << 28) | (5 << 16));
    assert!(hstx.push_data(0xAAAA_0001));
    assert!(hstx.push_data(0xAAAA_0002));

    hstx.advance(49);
    assert_eq!(hstx.fifo_level(), 2, "49 of 50 cycles shift nothing out");
    assert_eq!(hstx.output_level(), 0);

    hstx.advance(1);
    assert_eq!(hstx.fifo_level(), 1, "the first word completes at t = 50");
    assert_eq!(hstx.output_level(), 1);

    hstx.advance(50);
    assert_eq!(hstx.fifo_level(), 0);
    assert_eq!(hstx.drain_output(), [0xAAAA_0001, 0xAAAA_0002]);
    assert_eq!(hstx.output_words(), 2, "two words were shifted out");
}

#[test]
fn raw_passthrough_treats_zero_n_shifts_as_one_shift() {
    // CLKDIV = 1 → 2 clk_sys per shift; N_SHIFTS = 0 is modelled as 1 shift, so
    // a word lasts 2 clk_sys — identical to an explicit N_SHIFTS = 1.
    // CLKDIV = 1 → 每次移位 2 个 clk_sys；N_SHIFTS = 0 按 1 次移位处理，因此
    // 每字 2 个 clk_sys，与显式 N_SHIFTS = 1 完全一致。
    let mut implicit = new_hstx();
    write_ctrl(&implicit, 0x00, CSR_EN | (1 << 28));
    assert!(implicit.push_data(0x0000_0042));
    implicit.advance(1);
    assert_eq!(implicit.output_level(), 0);
    implicit.advance(1);
    assert_eq!(implicit.drain_output(), [0x42]);

    let mut explicit = new_hstx();
    write_ctrl(&explicit, 0x00, CSR_EN | (1 << 28) | (1 << 16));
    assert!(explicit.push_data(0x0000_0042));
    explicit.advance(1);
    assert_eq!(explicit.output_level(), 0);
    explicit.advance(1);
    assert_eq!(explicit.drain_output(), [0x42]);
}

#[test]
fn raw_passthrough_produces_no_tmds_pixels() {
    let mut hstx = new_hstx();
    write_ctrl(&hstx, 0x00, CSR_EN | (5 << 28) | (5 << 16));
    assert!(hstx.push_data(0xF800_07E0));
    hstx.advance(50);

    assert_eq!(hstx.output_level(), 1, "the raw word is still delivered");
    assert!(
        hstx.take_pixels().is_empty(),
        "without the expander no word is TMDS data"
    );
    assert_eq!(hstx.pixels_emitted(), 0);
    assert_eq!(
        hstx.drain_output(),
        [0xF800_07E0],
        "the word stays available as raw data"
    );
}

#[test]
fn tmds_word_decodes_into_two_rgb565_pixels() {
    // 0xF800_07E0 holds two RGB565 pixels: the low half 0x07E0 is pure green
    // (G = 0b111111 = 63) and the high half 0xF800 is pure red (R = 31).
    // ENC_SHIFT = 16 → 2 pixels per word, pixel 0 = bits [15:0].
    // Lane decode rule: symbol = bits [7:8-width] of word.rotate_right(rot +
    // 16 * pixel), with L0 blue 5b @ rot 29, L1 green 6b @ rot 3, L2 red 5b @
    // rot 8. Per pixel:
    //   p0 blue  = bits [7:3] of rotate_right(29)    = word[4:0]   = 0
    //   p0 green = bits [7:2] of rotate_right(3)     = word[10:5]  = 63
    //   p0 red   = bits [7:3] of rotate_right(8)     = word[15:11] = 0
    //   p1 blue  = bits [7:3] of rotate_right(29+16) = word[20:16] = 0
    //   p1 green = bits [7:2] of rotate_right(3+16)  = word[26:21] = 0
    //   p1 red   = bits [7:3] of rotate_right(8+16)  = word[31:27] = 31
    let mut hstx = new_hstx();
    load_gba_display_config(&hstx);
    assert!(hstx.push_command(CMD_TMDS | 2), "2 pixels, ENC_SHIFT = 16");
    assert!(hstx.push_data(0xF800_07E0));
    // One data word covering 2 pixels × 10 clk_sys per pixel = 20 clk_sys.
    hstx.advance(20);

    let pixels = hstx.take_pixels();
    assert_eq!(pixels.len(), 2, "ENC_SHIFT = 16 → two pixels per word");
    assert_eq!(pixels[0].lanes, [0, 63, 0], "0x07E0 is pure green");
    assert_eq!(pixels[1].lanes, [0, 0, 31], "0xF800 is pure red");
    assert_eq!(hstx.pixels_emitted(), 2);
    assert_eq!(hstx.output_level(), 0, "the TMDS word was consumed");
}

#[test]
fn take_pixels_leaves_raw_words_for_drain_output() {
    // 0x0000_001F decodes to blue = 31 in pixel 0 and black in pixel 1.
    // 0x0000_001F 解码为像素 0 的蓝 = 31、像素 1 全黑。
    let mut hstx = new_hstx();
    load_gba_display_config(&hstx);
    assert!(hstx.push_command(CMD_RAW | 1));
    assert!(hstx.push_data(0x354));
    assert!(hstx.push_command(CMD_TMDS | 2));
    assert!(hstx.push_data(0x0000_001F));
    // RAW word 1 pixel period, TMDS word 2 pixel periods → 30 clk_sys.
    // RAW 字 1 个像素周期、TMDS 字 2 个像素周期 → 30 个 clk_sys。
    hstx.advance(30);

    let pixels = hstx.take_pixels();
    assert_eq!(pixels.len(), 2);
    assert_eq!(pixels[0].lanes, [31, 0, 0]);
    assert_eq!(pixels[1].lanes, [0, 0, 0]);
    assert_eq!(
        hstx.drain_output(),
        [0x354],
        "the RAW sync word is not a pixel"
    );
}

#[test]
fn take_pixels_returns_nothing_once_the_queue_is_drained() {
    let mut hstx = new_hstx();
    load_gba_display_config(&hstx);
    assert!(hstx.push_command(CMD_TMDS | 4));
    assert!(hstx.push_data(0xF800_0000));
    assert!(hstx.push_data(0x0000_07E0));
    hstx.advance(40);

    assert_eq!(hstx.take_pixels().len(), 4, "two words × two pixels");
    assert_eq!(hstx.pixels_emitted(), 4);
    assert!(hstx.take_pixels().is_empty(), "the queue is drained");
    assert_eq!(hstx.pixels_emitted(), 4, "the counter never decreases");

    assert!(hstx.push_command(CMD_TMDS | 2));
    assert!(hstx.push_data(0));
    hstx.advance(20);
    assert_eq!(hstx.take_pixels().len(), 2);
    assert_eq!(hstx.pixels_emitted(), 6, "the counter keeps growing");
}

#[test]
fn tmds_decode_falls_back_to_the_low_byte_without_lane_config() {
    let mut hstx = new_hstx();
    // Expander on, ENC_SHIFT = 16, but EXPAND_TMDS keeps its all-zero reset
    // value, so no lane can be decoded reliably.
    // 扩展器开启、ENC_SHIFT = 16，但 EXPAND_TMDS 保持全 0 复位值，无法可靠解码。
    write_ctrl(&hstx, 0x00, CSR_EN | CSR_EXPAND_EN | (5 << 28));
    write_ctrl(&hstx, 0x24, (2 << 24) | (16 << 16));
    assert_eq!(read_ctrl(&hstx, 0x28), 0);
    assert!(hstx.push_command(CMD_TMDS | 2));
    assert!(hstx.push_data(0xF800_07E0));
    hstx.advance(20);

    let pixels = hstx.take_pixels();
    assert_eq!(pixels.len(), 2);
    assert_eq!(pixels[0].lanes, [0xE0, 0, 0], "slice 0x07E0 → low byte");
    assert_eq!(pixels[1].lanes, [0x00, 0, 0], "slice 0xF800 → low byte");
}

#[test]
fn zero_enc_shift_decodes_one_fallback_pixel_per_word() {
    let mut hstx = new_hstx();
    // ENC_SHIFT = 0 cannot split a word: one pixel per word, one pixel period.
    // ENC_SHIFT = 0 无法切分字：每字一个像素、一个像素周期。
    write_ctrl(&hstx, 0x00, CSR_EN | CSR_EXPAND_EN | (5 << 28));
    write_ctrl(&hstx, 0x24, 0);
    assert!(hstx.push_command(CMD_TMDS | 1));
    assert!(hstx.push_data(0x1234_5678));
    hstx.advance(10);

    let pixels = hstx.take_pixels();
    assert_eq!(pixels.len(), 1);
    assert_eq!(pixels[0].lanes, [0x78, 0, 0], "low byte of the whole word");
}

#[test]
fn eight_bit_lanes_decode_rgb888_from_the_sdk_layout() {
    let mut hstx = new_hstx();
    // NBITS = 7 → 8-bit lanes; ROT 0/8/16 places blue/green/red in bytes 0/1/2
    // and ENC_SHIFT = 24 keeps one pixel per word.
    // NBITS = 7 → 8 位通道；ROT 0/8/16 把蓝/绿/红放在字节 0/1/2，
    // ENC_SHIFT = 24 使每字仍只有 1 个像素。
    write_ctrl(&hstx, 0x00, CSR_EN | CSR_EXPAND_EN | (5 << 28));
    write_ctrl(&hstx, 0x24, (2 << 24) | (24 << 16));
    write_ctrl(
        &hstx,
        0x28,
        (7 << 21) | (16 << 16) | (7 << 13) | (8 << 8) | (7 << 5),
    );
    assert!(hstx.push_command(CMD_TMDS | 1));
    assert!(hstx.push_data(0x0012_3456));
    hstx.advance(10);

    let pixels = hstx.take_pixels();
    assert_eq!(pixels.len(), 1, "32 / 24 rounds down to one pixel");
    assert_eq!(
        pixels[0].lanes,
        [0x56, 0x34, 0x12],
        "blue, green, red bytes"
    );
}
