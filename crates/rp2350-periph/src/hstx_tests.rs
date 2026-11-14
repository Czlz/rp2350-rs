//! HSTX register, FIFO and command-model tests / HSTX 寄存器、FIFO 与命令模型测试

use rp2350_core::bus::AddressBus;
use rp2350_core::memory::SimpleBus;

use crate::boot::BootPeripherals;
use crate::hstx::{
    CMD_NOP, CMD_OPCODE_SHIFT, CMD_RAW, CMD_RAW_REPEAT, CMD_WAIT, CSR_EN, CSR_EXPAND_EN,
    FIFO_DEPTH, FIFO_STAT_EMPTY, FIFO_STAT_FULL, FIFO_STAT_LEVEL_MASK, FIFO_STAT_WOF, Hstx,
    IRQ_UNDERRUN, IRQ_UNKNOWN_COMMAND,
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
    hstx.advance(4);
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
    hstx.advance(3);

    assert_eq!(hstx.output_underrun(), 1);
    assert_eq!(hstx.output_level(), 2);
    assert_eq!(read_ctrl(&hstx, 0x30) & IRQ_UNDERRUN, IRQ_UNDERRUN);
    assert_ne!(irq.mask() & (1 << map::HSTX_IRQ), 0);

    // W1C 清状态后共享中断线释放。Clearing W1C releases the shared line.
    write_ctrl(&hstx, 0x30, IRQ_UNDERRUN);
    assert_eq!(irq.mask() & (1 << map::HSTX_IRQ), 0);

    assert!(hstx.push_data(3));
    assert!(hstx.push_data(4));
    hstx.advance(2);
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
    hstx.advance(1);
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
    hstx.advance(2);
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
    assert_eq!(peripherals.hstx_mut().drain_output(), [0x1234_5678]);
}
