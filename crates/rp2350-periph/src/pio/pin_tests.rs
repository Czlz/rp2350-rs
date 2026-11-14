//! PIO pin mapping, side-set, delay, CLKDIV and FIFO-join tests
//! PIO 引脚映射、side-set、延迟、CLKDIV 与 FIFO 合并测试

// 测试断言直接使用数据手册寄存器常量与块内私有状态，通配导入保持命名直观。
// Tests assert against datasheet register constants and block-private state, so
// glob imports keep the naming faithful to the datasheet.
#![allow(clippy::wildcard_imports)]

use rp2350_core::mmio::MmioDevice;

use super::regs::*;
use super::*;

// Instruction encoders / 指令编码器

const DEST_PINS: u8 = 0;
const DEST_PINDIRS: u8 = 4;
const SRC_PINS: u8 = 0;

const COND_PIN: u8 = 6;

const WAIT_GPIO: u8 = 0;
const WAIT_PIN: u8 = 1;

/// `SET destination, data` / `SET destination, data`
fn set(dest: u8, data: u8) -> u16 {
    0xE000 | (u16::from(dest) << 5) | (u16::from(data) & 0x1F)
}

/// `OUT destination, count` / `OUT destination, count`
fn out(dest: u8, count: u8) -> u16 {
    0x6000 | (u16::from(dest) << 5) | (u16::from(count) & 0x1F)
}

/// `IN source, count` / `IN source, count`
fn instr_in(source: u8, count: u8) -> u16 {
    0x4000 | (u16::from(source) << 5) | (u16::from(count) & 0x1F)
}

/// `JMP condition, address` / `JMP condition, address`
fn jmp(condition: u8, address: u8) -> u16 {
    (u16::from(condition) << 5) | (u16::from(address) & 0x1F)
}

/// `WAIT polarity source, index` / `WAIT polarity source, index`
fn wait(polarity: bool, source: u8, index: u8) -> u16 {
    0x2000 | if polarity { 0x80 } else { 0 } | (u16::from(source) << 5) | u16::from(index & 0x1F)
}

/// `MOV Y, Y` used as a no-op / 用作空操作的 `MOV Y, Y`
fn nop() -> u16 {
    0xA042
}

/// `PUSH` without blocking / 非阻塞 `PUSH`
fn push() -> u16 {
    0x8000
}

/// `PULL with block` / 阻塞式 `PULL`
fn pull_blocking() -> u16 {
    0x80A0
}

/// Overlay the shared delay/side-set field (bits 12:8) / 覆盖共享延迟/side-set 字段
fn with_field(instr: u16, field: u8) -> u16 {
    instr | ((u16::from(field) & 0x1F) << 8)
}

/// Write one instruction-memory slot / 写入一个指令存储槽
fn write_imem(pio: &mut Pio, slot: u32, instr: u16) {
    assert!(pio.write32(INSTR_MEM0 + slot * 4, u32::from(instr)));
}

/// Write one state-machine window register / 写入一个状态机窗口寄存器
fn write_sm(pio: &mut Pio, index: u32, register: u32, value: u32) {
    assert!(pio.write32(SM0_CLKDIV + index * SM_STRIDE + register, value));
}

#[test]
fn host_pin_api_tracks_the_level_vectors() {
    let mut pio = Pio::new(0);
    assert_eq!(pio.input_levels(), 0);
    assert_eq!(pio.output_levels(), 0);
    assert_eq!(pio.output_enables(), 0);

    pio.set_input_levels(0xFFFF_FFFF);
    assert_eq!(pio.input_levels(), u32::MAX);
    pio.set_input_level(3, false);
    assert_eq!(pio.input_levels(), !(1 << 3));
    pio.set_input_level(3, true);
    assert_eq!(pio.input_levels(), u32::MAX);
    // 越界引脚被忽略而不是 panic。Out-of-range pins are ignored, not panicked on.
    pio.set_input_level(40, true);
    assert_eq!(pio.input_levels(), u32::MAX);
}

#[test]
fn out_pins_drives_out_count_pins_from_out_base() {
    let mut pio = Pio::new(0);
    write_sm(&mut pio, 0, SM_PINCTRL, 8 | (4 << PINCTRL_OUT_COUNT_SHIFT));
    write_imem(&mut pio, 0, pull_blocking());
    write_imem(&mut pio, 1, out(DEST_PINS, 4));
    pio.write32(TXF0, 0xABCD);

    assert!(pio.step_sm(0));
    assert!(pio.step_sm(0));
    assert_eq!(pio.output_levels() & 0x0000_0F00, 0x0000_0D00);
    assert_eq!(
        pio.output_levels() & !0x0000_0F00,
        0,
        "only OUT pins change"
    );
    assert_eq!(pio.read32(DBG_PADOUT), Some(pio.output_levels()));
}

#[test]
fn out_pindirs_drives_the_output_enables() {
    let mut pio = Pio::new(0);
    write_sm(&mut pio, 0, SM_PINCTRL, 8 | (4 << PINCTRL_OUT_COUNT_SHIFT));
    write_imem(&mut pio, 0, pull_blocking());
    write_imem(&mut pio, 1, out(DEST_PINDIRS, 4));
    pio.write32(TXF0, 0xABCD);

    assert!(pio.step_sm(0));
    assert!(pio.step_sm(0));
    assert_eq!(pio.output_enables() & 0x0000_0F00, 0x0000_0D00);
    assert_eq!(pio.output_levels(), 0, "PINDIRS leaves the levels alone");
    assert_eq!(pio.read32(DBG_PADOE), Some(pio.output_enables()));
}

#[test]
fn set_pins_drives_set_count_pins_from_set_base() {
    let mut pio = Pio::new(0);
    write_sm(
        &mut pio,
        0,
        SM_PINCTRL,
        (12 << PINCTRL_SET_BASE_SHIFT) | (3 << PINCTRL_SET_COUNT_SHIFT),
    );
    write_imem(&mut pio, 0, set(DEST_PINS, 5));

    assert!(pio.step_sm(0));
    // 5 = 0b101 写到 SET_BASE(12) 起的 3 个引脚。
    // 5 = 0b101 occupies pins 12..14.
    assert_eq!(pio.output_levels() & 0x0000_7000, 0x0000_5000);
    assert_eq!(pio.output_levels() & !0x0000_7000, 0);
}

#[test]
fn set_pindirs_drives_the_output_enables() {
    let mut pio = Pio::new(0);
    write_sm(
        &mut pio,
        0,
        SM_PINCTRL,
        (1 << PINCTRL_SET_BASE_SHIFT) | (1 << PINCTRL_SET_COUNT_SHIFT),
    );
    write_imem(&mut pio, 0, set(DEST_PINDIRS, 1));

    assert!(pio.step_sm(0));
    assert_eq!(pio.output_enables(), 0b10);
    assert_eq!(pio.output_levels(), 0);
}

#[test]
fn in_pins_samples_in_base_and_shifts_right() {
    let mut pio = Pio::new(0);
    write_sm(&mut pio, 0, SM_PINCTRL, 2 << PINCTRL_IN_BASE_SHIFT);
    write_imem(&mut pio, 0, instr_in(SRC_PINS, 4));
    write_imem(&mut pio, 1, push());
    // 引脚 2、3 为高，IN_BASE = 2 时采样得到 0b0011。
    // Pins 2 and 3 are high, which IN_BASE = 2 samples as 0b0011.
    pio.set_input_levels(0x0000_000C);

    assert!(pio.step_sm(0));
    assert!(pio.step_sm(0));
    assert_eq!(
        pio.read32(RXF0),
        Some(0x3000_0000),
        "right shift into ISR[31:28]"
    );
}

#[test]
fn in_pins_shifts_left_when_configured() {
    let mut pio = Pio::new(0);
    // 清空 SHIFTDIR 位后 IN/OUT 都左移。Clearing the SHIFTDIR bits selects left shift.
    write_sm(&mut pio, 0, SM_SHIFTCTRL, 0);
    write_imem(&mut pio, 0, instr_in(SRC_PINS, 4));
    write_imem(&mut pio, 1, push());
    pio.set_input_levels(0x3);

    assert!(pio.step_sm(0));
    assert!(pio.step_sm(0));
    assert_eq!(
        pio.read32(RXF0),
        Some(0x3),
        "left shift keeps data in ISR[3:0]"
    );
}

#[test]
fn in_count_masks_the_sampled_pins() {
    let mut pio = Pio::new(0);
    // RP2350 的 IN_COUNT = 4 只保留 4 个 LSB 引脚。
    // RP2350's IN_COUNT = 4 keeps only the four LSB pins.
    write_sm(&mut pio, 0, SM_SHIFTCTRL, SHIFTCTRL_RESET | 4);
    write_imem(&mut pio, 0, instr_in(SRC_PINS, 8));
    write_imem(&mut pio, 1, push());
    pio.set_input_levels(0x0000_00AB);

    assert!(pio.step_sm(0));
    assert!(pio.step_sm(0));
    assert_eq!(pio.read32(RXF0), Some(0x0B00_0000));
}

#[test]
fn jmp_pin_branches_on_the_jmp_pin_field() {
    let mut pio = Pio::new(0);
    write_sm(
        &mut pio,
        0,
        SM_EXECCTRL,
        EXECCTRL_RESET | (5 << EXECCTRL_JMP_PIN_SHIFT),
    );
    write_imem(&mut pio, 0, jmp(COND_PIN, 2));
    write_imem(&mut pio, 1, nop());
    write_imem(&mut pio, 2, nop());

    assert!(pio.step_sm(0));
    assert_eq!(pio.sm[0].pc, 1, "low JMP_PIN falls through");

    assert!(pio.write32(CTRL, SM_RESTART));
    pio.set_input_level(5, true);
    assert!(pio.step_sm(0));
    assert_eq!(pio.sm[0].pc, 2, "high JMP_PIN branches");
}

#[test]
fn wait_pin_uses_in_base_and_both_polarities() {
    let mut pio = Pio::new(0);
    write_sm(&mut pio, 0, SM_PINCTRL, 3 << PINCTRL_IN_BASE_SHIFT);
    write_imem(&mut pio, 0, wait(true, WAIT_PIN, 2));
    write_imem(&mut pio, 1, wait(false, WAIT_PIN, 2));
    write_imem(&mut pio, 2, nop());
    pio.set_input_level(5, true);

    assert!(pio.step_sm(0), "WAIT 1 PIN 2 passes on pin 5 = 1");
    assert_eq!(pio.sm[0].pc, 1);
    assert!(!pio.step_sm(0), "WAIT 0 PIN 2 blocks while pin 5 = 1");
    assert_eq!(pio.sm[0].pc, 1);
    assert_eq!(pio.read32(FDEBUG), Some(0), "pin waits are not FIFO stalls");
    pio.set_input_level(5, false);
    assert!(pio.step_sm(0));
    assert_eq!(pio.sm[0].pc, 2);
}

#[test]
fn wait_gpio_uses_the_absolute_pin_index() {
    let mut pio = Pio::new(0);
    write_sm(&mut pio, 0, SM_PINCTRL, 3 << PINCTRL_IN_BASE_SHIFT);
    write_imem(&mut pio, 0, wait(true, WAIT_GPIO, 2));
    write_imem(&mut pio, 1, nop());

    assert!(!pio.step_sm(0), "GPIO 2 is low");
    pio.set_input_level(5, true);
    assert!(!pio.step_sm(0), "WAIT GPIO ignores IN_BASE");
    pio.set_input_level(2, true);
    assert!(pio.step_sm(0));
    assert_eq!(pio.sm[0].pc, 1);
}

#[test]
fn sideset_drives_sideset_pins() {
    let mut pio = Pio::new(0);
    write_sm(
        &mut pio,
        0,
        SM_PINCTRL,
        (10 << PINCTRL_SIDESET_BASE_SHIFT) | (2 << PINCTRL_SIDESET_COUNT_SHIFT),
    );
    // SIDESET_COUNT = 2 时字段高 2 位是 side-set，低 3 位是延迟。
    // With SIDESET_COUNT = 2 the top two field bits are side-set and three are delay.
    write_imem(&mut pio, 0, with_field(nop(), 0b10000));
    write_imem(&mut pio, 1, with_field(nop(), 0b01000));

    assert!(pio.step_sm(0));
    assert_eq!(pio.output_levels() & 0x0000_0C00, 0x0000_0800);
    assert!(pio.step_sm(0));
    assert_eq!(pio.output_levels() & 0x0000_0C00, 0x0000_0400);
}

#[test]
fn sideset_optional_enable_gates_the_write() {
    let mut pio = Pio::new(0);
    write_sm(
        &mut pio,
        0,
        SM_PINCTRL,
        (20 << PINCTRL_SIDESET_BASE_SHIFT) | (2 << PINCTRL_SIDESET_COUNT_SHIFT),
    );
    write_sm(&mut pio, 0, SM_EXECCTRL, EXECCTRL_RESET | EXECCTRL_SIDE_EN);
    // 字段最高位是 side-set 使能，剩余 1 位是数据。
    // The field MSB is the side-set enable; the remaining bit is data.
    write_imem(&mut pio, 0, with_field(nop(), 0b11000));
    write_imem(&mut pio, 1, with_field(nop(), 0b01000));
    write_imem(&mut pio, 2, with_field(nop(), 0b10000));

    assert!(pio.step_sm(0));
    assert_eq!(pio.output_levels() & (1 << 20), 1 << 20);
    assert!(pio.step_sm(0));
    assert_eq!(
        pio.output_levels() & (1 << 20),
        1 << 20,
        "enable bit clear leaves the pin untouched"
    );
    assert!(pio.step_sm(0));
    assert_eq!(pio.output_levels() & (1 << 20), 0);
}

#[test]
fn sideset_pindir_drives_output_enables() {
    let mut pio = Pio::new(0);
    write_sm(
        &mut pio,
        0,
        SM_PINCTRL,
        (4 << PINCTRL_SIDESET_BASE_SHIFT) | (1 << PINCTRL_SIDESET_COUNT_SHIFT),
    );
    write_sm(
        &mut pio,
        0,
        SM_EXECCTRL,
        EXECCTRL_RESET | EXECCTRL_SIDE_PINDIR,
    );
    // SIDESET_COUNT = 1 时数据位是字段最高位。
    // With SIDESET_COUNT = 1 the data bit is the field MSB.
    write_imem(&mut pio, 0, with_field(nop(), 0b10000));

    assert!(pio.step_sm(0));
    assert_eq!(pio.output_enables(), 1 << 4);
    assert_eq!(pio.output_levels(), 0);
}

#[test]
fn delay_cycles_stall_the_following_steps() {
    let mut pio = Pio::new(0);
    write_imem(&mut pio, 0, with_field(nop(), 3));
    write_imem(&mut pio, 1, nop());

    assert!(pio.step_sm(0), "the instruction itself retires");
    assert_eq!(pio.sm[0].pc, 1);
    assert_eq!(pio.sm[0].delay_cycles, 3);
    for remaining in (0..3).rev() {
        assert!(!pio.step_sm(0), "delay cycles report no retirement");
        assert_eq!(pio.sm[0].pc, 1);
        assert_eq!(pio.sm[0].delay_cycles, remaining);
    }
    assert!(pio.step_sm(0), "the next instruction runs after the delay");
    assert_eq!(pio.sm[0].pc, 2);
}

#[test]
fn tick_consumes_delay_before_the_next_instruction() {
    let mut pio = Pio::new(0);
    write_imem(&mut pio, 0, with_field(nop(), 3));
    write_imem(&mut pio, 1, nop());
    assert!(pio.write32(CTRL, 1));

    pio.tick(4);
    assert_eq!(
        pio.read32(SM0_CLKDIV + SM_ADDR),
        Some(1),
        "1 cycle + 3 delay"
    );
    pio.tick(1);
    assert_eq!(pio.read32(SM0_CLKDIV + SM_ADDR), Some(2));
}

#[test]
fn clock_divider_scales_the_instruction_rate() {
    let mut pio = Pio::new(0);
    for slot in 0..8 {
        write_imem(&mut pio, slot, nop());
    }
    assert!(pio.write32(CTRL, 1));
    write_sm(&mut pio, 0, SM_CLKDIV, 0x0002_0000);

    pio.tick(1);
    assert_eq!(
        pio.read32(SM0_CLKDIV + SM_ADDR),
        Some(0),
        "divider 2.0 gates"
    );
    pio.tick(5);
    assert_eq!(pio.read32(SM0_CLKDIV + SM_ADDR), Some(3), "6 cycles / 2");
}

#[test]
fn fractional_clock_divider_and_default_keep_their_rates() {
    let mut pio = Pio::new(0);
    for slot in 0..8 {
        write_imem(&mut pio, slot, nop());
    }
    assert!(pio.write32(CTRL, 1));
    // 0x0001_8000 = 1 + 0x80/256 = 1.5。0x0001_8000 is the 1.5 divisor.
    write_sm(&mut pio, 0, SM_CLKDIV, 0x0001_8000);
    pio.tick(6);
    assert_eq!(pio.read32(SM0_CLKDIV + SM_ADDR), Some(4), "6 cycles / 1.5");

    let mut default = Pio::new(0);
    for slot in 0..8 {
        write_imem(&mut default, slot, nop());
    }
    assert!(default.write32(CTRL, 1));
    default.tick(6);
    assert_eq!(
        default.read32(SM0_CLKDIV + SM_ADDR),
        Some(6),
        "reset CLKDIV stays 1 instruction per cycle"
    );
}

#[test]
fn fifo_join_tx_expands_tx_and_disables_rx() {
    let mut pio = Pio::new(0);
    write_sm(
        &mut pio,
        0,
        SM_SHIFTCTRL,
        SHIFTCTRL_RESET | SHIFTCTRL_FJOIN_TX,
    );
    for word in 1..=8 {
        assert!(pio.write32(TXF0, word));
    }
    assert_eq!(pio.read32(FLEVEL), Some(8), "joined TX is 8 deep");
    let fstat = pio.read32(FSTAT).expect("fstat");
    assert_eq!(fstat & (1 << 16), 1 << 16, "TXFULL");
    assert_eq!(fstat & (1 << 0), 1 << 0, "disabled RX reads full");
    assert_eq!(fstat & (1 << 8), 1 << 8, "disabled RX reads empty");

    assert!(pio.write32(TXF0, 9), "overflow write is accepted");
    assert_eq!(pio.read32(FLEVEL), Some(8));
    assert_eq!(
        pio.read32(FDEBUG).expect("fdebug") & FDEBUG_TXOVER,
        FDEBUG_TXOVER
    );
    for word in 1..=8 {
        assert_eq!(pio.pop_tx(0), Some(word));
    }
    assert!(!pio.push_rx(0, 0xAA), "RX is disabled by FJOIN_TX");
    assert_ne!(pio.read32(FDEBUG).expect("fdebug") & FDEBUG_RXSTALL, 0);
}

#[test]
fn fifo_join_rx_expands_rx_and_disables_tx() {
    let mut pio = Pio::new(0);
    write_sm(
        &mut pio,
        0,
        SM_SHIFTCTRL,
        SHIFTCTRL_RESET | SHIFTCTRL_FJOIN_RX,
    );
    for word in 0x10..0x18 {
        assert!(pio.push_rx(0, word));
    }
    assert_eq!(pio.read32(FLEVEL), Some(8 << 4), "joined RX is 8 deep");
    assert!(!pio.push_rx(0, 0x18), "ninth word is dropped");
    let fstat = pio.read32(FSTAT).expect("fstat");
    assert_eq!(fstat & (1 << 16), 1 << 16, "disabled TX reads full");
    assert_eq!(fstat & (1 << 24), 1 << 24, "disabled TX reads empty");
    for word in 0x10..0x18 {
        assert_eq!(pio.read32(RXF0), Some(word));
    }
    assert_eq!(pio.pop_tx(0), None, "TX is disabled by FJOIN_RX");
}

#[test]
fn fifo_join_change_flushes_both_fifos() {
    let mut pio = Pio::new(0);
    assert!(pio.write32(TXF0, 1));
    assert!(pio.write32(TXF0, 2));
    assert!(pio.push_rx(0, 3));
    assert_eq!(pio.read32(FLEVEL), Some(0x12));

    write_sm(
        &mut pio,
        0,
        SM_SHIFTCTRL,
        SHIFTCTRL_RESET | SHIFTCTRL_FJOIN_TX,
    );
    assert_eq!(
        pio.read32(FLEVEL),
        Some(0),
        "join change flushes both FIFOs"
    );

    write_sm(&mut pio, 0, SM_SHIFTCTRL, SHIFTCTRL_RESET);
    assert_eq!(pio.read32(FLEVEL), Some(0));
    for word in 1..=4 {
        assert!(pio.write32(TXF0, word));
    }
    assert_eq!(pio.read32(FLEVEL), Some(4), "unjoined TX is 4 deep again");
    assert!(pio.write32(TXF0, 5));
    assert_eq!(pio.read32(FLEVEL), Some(4));
}

#[test]
fn reserved_register_fields_do_not_panic() {
    let mut pio = Pio::new(0);
    write_sm(&mut pio, 0, SM_PINCTRL, 0xFFFF_FFFF);
    write_sm(&mut pio, 0, SM_EXECCTRL, 0xFFFF_FFFF);
    write_sm(&mut pio, 0, SM_SHIFTCTRL, 0xFFFF_FFFF);
    write_imem(&mut pio, 0, with_field(nop(), 0x1F));

    assert!(pio.step_sm(0), "reserved encodings still retire");
    // SIDESET_COUNT 被钳到 5、SIDE_PINDIR 生效，引脚按 31 回绕。
    // SIDESET_COUNT is clamped to 5, SIDE_PINDIR applies and pins wrap at 31.
    assert_eq!(pio.output_enables() & 0b111, 0b111);
    assert_ne!(pio.output_enables() & (1 << 31), 0);
}
