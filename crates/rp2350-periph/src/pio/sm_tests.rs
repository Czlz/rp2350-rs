//! PIO state-machine execution tests / PIO 状态机执行测试

// 测试断言直接使用数据手册寄存器常量与块内私有状态，通配导入保持命名直观。
// Tests assert against datasheet register constants and block-private state, so
// glob imports keep the naming faithful to the datasheet.
#![allow(clippy::wildcard_imports)]

use rp2350_core::mmio::MmioDevice;

use super::regs::*;
use super::*;

// Instruction encoders / 指令编码器

const COND_ALWAYS: u8 = 0;
const COND_XDEC: u8 = 2;

const DEST_X: u8 = 1;
const DEST_Y: u8 = 2;
const DEST_ISR: u8 = 6;
const SRC_X: u8 = 1;
const SRC_Y: u8 = 2;
const SRC_STATUS: u8 = 5;
const SRC_OSR: u8 = 7;

/// `JMP condition, address` / `JMP condition, address`
fn jmp(condition: u8, address: u8) -> u16 {
    (u16::from(condition) << 5) | (u16::from(address) & 0x1F)
}

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

/// `MOV destination, source` / `MOV destination, source`
fn mov(dest: u8, source: u8) -> u16 {
    0xA000 | (u16::from(dest) << 5) | (u16::from(source) & 0x07)
}

/// `PUSH block?` / `PUSH block?`
fn push(block: bool) -> u16 {
    0x8000 | if block { 0x20 } else { 0 }
}

/// `PULL block?` / `PULL block?`
fn pull(block: bool) -> u16 {
    0x8080 | if block { 0x20 } else { 0 }
}

/// `IRQ clear/wait, field` / `IRQ clear/wait, field`
fn irq_instr(clear: bool, wait: bool, field: u8) -> u16 {
    let flags = if clear { 0x40 } else { 0 } | if wait { 0x20 } else { 0 };
    0xC000 | flags | (u16::from(field) & 0x1F)
}

/// `WAIT polarity GPIO, index` / `WAIT polarity GPIO, index`
fn wait_gpio(polarity: bool, index: u8) -> u16 {
    0x2000 | if polarity { 0x80 } else { 0 } | (u16::from(index) & 0x1F)
}

/// `MOV Y, Y` used as a no-op / 用作空操作的 `MOV Y, Y`
fn nop() -> u16 {
    mov(DEST_Y, SRC_Y)
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
fn jmp_x_decrement_loops_until_x_is_zero() {
    let mut pio = Pio::new(0);
    write_imem(&mut pio, 0, jmp(COND_XDEC, 0));
    write_imem(&mut pio, 1, nop());
    pio.sm[0].x = 3;
    for _ in 0..3 {
        assert!(pio.step_sm(0));
        assert_eq!(pio.sm[0].pc, 0, "X-- branches while X is non-zero");
    }
    assert_eq!(pio.sm[0].x, 0);
    assert!(pio.step_sm(0));
    assert_eq!(pio.sm[0].pc, 1, "X == 0 falls through");
    assert_eq!(pio.sm[0].x, u32::MAX, "X-- still decrements");
}

#[test]
fn wrap_top_and_bottom_control_the_pc() {
    let mut pio = Pio::new(0);
    write_imem(&mut pio, 1, nop());
    write_imem(&mut pio, 2, nop());
    write_sm(
        &mut pio,
        0,
        SM_EXECCTRL,
        (2 << EXECCTRL_WRAP_TOP_SHIFT) | (1 << EXECCTRL_WRAP_BOTTOM_SHIFT),
    );
    write_sm(&mut pio, 0, SM_INSTR, u32::from(jmp(COND_ALWAYS, 1)));
    assert_eq!(pio.sm[0].pc, 1, "injected JMP redirects the PC");
    assert!(pio.step_sm(0));
    assert_eq!(pio.sm[0].pc, 2);
    assert!(pio.step_sm(0));
    assert_eq!(pio.sm[0].pc, 1, "PC wraps from WRAP_TOP to WRAP_BOTTOM");
}

#[test]
fn in_and_out_shift_right_by_default() {
    let mut pio = Pio::new(0);
    write_imem(&mut pio, 0, pull(true));
    write_imem(&mut pio, 1, out(DEST_X, 8));
    write_imem(&mut pio, 2, instr_in(SRC_X, 8));
    write_imem(&mut pio, 3, push(false));
    pio.write32(TXF0, 0x1234_5678);
    for _ in 0..4 {
        assert!(pio.step_sm(0));
    }
    assert_eq!(pio.sm[0].x, 0x78);
    assert_eq!(pio.sm[0].osr, 0x0012_3456);
    assert_eq!(pio.read32(FLEVEL), Some(0x10), "RX level 1, TX level 0");
    assert_eq!(pio.read32(RXF0), Some(0x7800_0000));
}

#[test]
fn in_and_out_shift_left_when_configured() {
    let mut pio = Pio::new(0);
    write_imem(&mut pio, 0, pull(true));
    write_imem(&mut pio, 1, out(DEST_X, 8));
    write_imem(&mut pio, 2, instr_in(SRC_X, 8));
    write_imem(&mut pio, 3, push(false));
    // 清空两个 SHIFTDIR 位，改为左移。Clear both SHIFTDIR bits to shift left.
    write_sm(&mut pio, 0, SM_SHIFTCTRL, 0);
    pio.write32(TXF0, 0x1234_5678);
    for _ in 0..4 {
        assert!(pio.step_sm(0));
    }
    assert_eq!(pio.sm[0].x, 0x12);
    assert_eq!(pio.sm[0].osr, 0x3456_7800);
    assert_eq!(pio.read32(RXF0), Some(0x12));
}

#[test]
fn autopush_threshold_fills_the_rx_fifo() {
    let mut pio = Pio::new(0);
    write_imem(&mut pio, 0, set(DEST_X, 5));
    write_imem(&mut pio, 1, instr_in(SRC_X, 8));
    write_sm(
        &mut pio,
        0,
        SM_SHIFTCTRL,
        SHIFTCTRL_RESET | SHIFTCTRL_AUTOPUSH | (8 << SHIFTCTRL_PUSH_THRESH_SHIFT),
    );
    assert!(pio.step_sm(0));
    assert!(pio.step_sm(0));
    assert_eq!(
        pio.read32(FLEVEL),
        Some(0x10),
        "autopush moved 1 word to RX"
    );
    assert_eq!(
        pio.read32(FSTAT).expect("fstat") & (1 << 8),
        0,
        "RX FIFO no longer empty"
    );
    assert_eq!(pio.read32(RXF0), Some(0x0500_0000));
    assert_eq!(pio.sm[0].isr_bits, 0, "ISR counter cleared by autopush");
}

#[test]
fn autopull_refills_the_osr_and_stalls_when_empty() {
    let mut pio = Pio::new(0);
    write_imem(&mut pio, 0, out(DEST_X, 8));
    write_imem(&mut pio, 1, out(DEST_Y, 8));
    write_sm(
        &mut pio,
        0,
        SM_SHIFTCTRL,
        SHIFTCTRL_RESET | SHIFTCTRL_AUTOPULL | (8 << SHIFTCTRL_PULL_THRESH_SHIFT),
    );
    pio.write32(TXF0, 0x1122_3344);
    assert!(pio.step_sm(0));
    assert_eq!(pio.sm[0].x, 0x44);
    assert_eq!(pio.sm[0].osr_bits, 24);
    // OSR 已低于阈值且 TX 为空，第二条 OUT 阻塞。
    // The OSR is below the threshold and TX is empty, so the second OUT stalls.
    assert!(!pio.step_sm(0));
    assert_eq!(pio.sm[0].pc, 1);
    assert_eq!(
        pio.read32(FDEBUG).expect("fdebug") & FDEBUG_TXSTALL,
        FDEBUG_TXSTALL
    );
}

#[test]
fn pull_block_stalls_until_the_tx_fifo_has_data() {
    let mut pio = Pio::new(0);
    write_imem(&mut pio, 0, pull(true));
    write_imem(&mut pio, 1, mov(DEST_ISR, SRC_OSR));
    write_imem(&mut pio, 2, push(true));
    assert!(!pio.step_sm(0));
    assert_eq!(pio.read32(SM0_CLKDIV + SM_ADDR), Some(0));
    assert_ne!(pio.sm[0].execctrl & EXECCTRL_EXEC_STALLED, 0);
    assert_eq!(
        pio.read32(FDEBUG).expect("fdebug") & FDEBUG_TXSTALL,
        FDEBUG_TXSTALL
    );
    pio.write32(TXF0, 0xCAFE_F00D);
    assert!(pio.step_sm(0));
    assert_eq!(pio.read32(SM0_CLKDIV + SM_ADDR), Some(1));
    assert_eq!(pio.sm[0].osr, 0xCAFE_F00D);
    assert_eq!(pio.sm[0].execctrl & EXECCTRL_EXEC_STALLED, 0);
    assert!(pio.step_sm(0));
    assert!(pio.step_sm(0));
    assert_eq!(pio.read32(RXF0), Some(0xCAFE_F00D));
}

#[test]
fn push_block_stalls_until_the_rx_fifo_has_space() {
    let mut pio = Pio::new(0);
    for word in 1..=4 {
        assert!(pio.push_rx(0, word));
    }
    write_imem(&mut pio, 0, set(DEST_X, 9));
    write_imem(&mut pio, 1, push(true));
    write_imem(&mut pio, 2, nop());
    assert!(pio.step_sm(0));
    assert!(!pio.step_sm(0));
    assert_eq!(pio.sm[0].pc, 1);
    assert_eq!(
        pio.read32(FDEBUG).expect("fdebug") & FDEBUG_RXSTALL,
        FDEBUG_RXSTALL
    );
    assert_eq!(pio.read32(RXF0), Some(1), "drain one word");
    assert!(pio.step_sm(0));
    assert_eq!(pio.sm[0].pc, 2);
}

#[test]
fn mov_status_compares_the_tx_fifo_level() {
    let mut pio = Pio::new(0);
    // STATUS_N = 2，STATUS_SEL = 0（TX FIFO）。STATUS_N = 2 with TX FIFO selected.
    write_sm(&mut pio, 0, SM_EXECCTRL, EXECCTRL_RESET | 2);
    write_sm(&mut pio, 0, SM_INSTR, u32::from(mov(DEST_ISR, SRC_STATUS)));
    write_sm(&mut pio, 0, SM_INSTR, u32::from(push(false)));
    assert_eq!(pio.read32(RXF0), Some(u32::MAX));
    pio.write32(TXF0, 0xA);
    pio.write32(TXF0, 0xB);
    write_sm(&mut pio, 0, SM_INSTR, u32::from(mov(DEST_ISR, SRC_STATUS)));
    write_sm(&mut pio, 0, SM_INSTR, u32::from(push(false)));
    assert_eq!(pio.read32(RXF0), Some(0));
}

#[test]
fn irq_set_clear_and_relative_index() {
    let mut pio = Pio::new(0);
    write_sm(&mut pio, 0, SM_INSTR, u32::from(irq_instr(false, false, 3)));
    assert_eq!(pio.read32(IRQ), Some(1 << 3));
    write_sm(&mut pio, 0, SM_INSTR, u32::from(irq_instr(true, false, 3)));
    assert_eq!(pio.read32(IRQ), Some(0));
    // `irq rel, 3` on SM2 targets flag (3 + 2) & 3 = 1.
    // SM2 的 `irq rel, 3` 指向标志 (3 + 2) & 3 = 1。
    write_sm(
        &mut pio,
        2,
        SM_INSTR,
        u32::from(irq_instr(false, false, 0x13)),
    );
    assert_eq!(pio.read32(IRQ), Some(1 << 1));
}

#[test]
fn irq_wait_stalls_until_the_flag_is_cleared() {
    let mut pio = Pio::new(0);
    write_imem(&mut pio, 0, irq_instr(false, true, 5));
    write_imem(&mut pio, 1, nop());
    assert!(!pio.step_sm(0));
    assert_eq!(pio.sm[0].pc, 0);
    assert_eq!(pio.read32(IRQ), Some(1 << 5), "IRQ was raised");
    assert!(!pio.step_sm(0), "still waiting for the flag to clear");
    pio.write32(IRQ, 1 << 5);
    assert!(pio.step_sm(0));
    assert_eq!(pio.sm[0].pc, 1);
}

#[test]
fn set_x_and_y_are_observable_through_the_rx_fifo() {
    let mut pio = Pio::new(0);
    write_imem(&mut pio, 0, set(DEST_X, 7));
    write_imem(&mut pio, 1, mov(DEST_ISR, SRC_X));
    write_imem(&mut pio, 2, push(false));
    write_imem(&mut pio, 3, set(DEST_Y, 9));
    write_imem(&mut pio, 4, mov(DEST_ISR, SRC_Y));
    write_imem(&mut pio, 5, push(false));
    for _ in 0..6 {
        assert!(pio.step_sm(0));
    }
    assert_eq!(pio.read32(RXF0), Some(7));
    assert_eq!(pio.read32(RXF0), Some(9));
}

#[test]
fn addr_tracks_pc_and_exec_stalled_clears_on_retry() {
    let mut pio = Pio::new(0);
    write_imem(&mut pio, 0, nop());
    write_imem(&mut pio, 1, nop());
    write_imem(&mut pio, 2, pull(true));
    assert_eq!(pio.read32(SM0_CLKDIV + SM_ADDR), Some(0));
    assert!(pio.step_sm(0));
    assert_eq!(pio.read32(SM0_CLKDIV + SM_ADDR), Some(1));
    assert_eq!(
        pio.read32(SM0_CLKDIV + SM_INSTR),
        Some(u32::from(nop())),
        "INSTR reports the instruction at PC"
    );
    assert!(pio.step_sm(0));
    assert_eq!(pio.read32(SM0_CLKDIV + SM_ADDR), Some(2));
    assert!(!pio.step_sm(0));
    assert_eq!(
        pio.read32(SM0_CLKDIV + SM_EXECCTRL).expect("execctrl") & EXECCTRL_EXEC_STALLED,
        EXECCTRL_EXEC_STALLED
    );
    pio.write32(TXF0, 5);
    assert!(pio.step_sm(0));
    assert_eq!(pio.sm[0].execctrl & EXECCTRL_EXEC_STALLED, 0);
}

#[test]
fn wait_on_gpio_blocks_only_for_polarity_one() {
    let mut pio = Pio::new(0);
    write_imem(&mut pio, 0, wait_gpio(false, 0));
    write_imem(&mut pio, 1, wait_gpio(true, 0));
    write_imem(&mut pio, 2, nop());
    assert!(pio.step_sm(0), "WAIT 0 passes because pins read 0");
    assert_eq!(pio.sm[0].pc, 1);
    assert!(!pio.step_sm(0), "WAIT 1 blocks without a pin model");
    assert_eq!(pio.sm[0].pc, 1);
    // 引脚等待不属于 FIFO stall，不置位 FDEBUG。
    // A pin wait is not a FIFO stall and leaves FDEBUG untouched.
    assert_eq!(pio.read32(FDEBUG), Some(0));
}

#[test]
fn tick_runs_only_enabled_state_machines() {
    let mut pio = Pio::new(1);
    for slot in 0..8 {
        write_imem(&mut pio, slot, nop());
    }
    pio.write32(CTRL, 0b0011);
    pio.tick(3);
    assert_eq!(pio.read32(SM0_CLKDIV + SM_ADDR), Some(3));
    assert_eq!(pio.read32(SM0_CLKDIV + SM_STRIDE + SM_ADDR), Some(3));
    assert_eq!(pio.read32(SM0_CLKDIV + 2 * SM_STRIDE + SM_ADDR), Some(0));
}

#[test]
fn sm_restart_resets_pc_and_clears_the_stall() {
    let mut pio = Pio::new(0);
    write_imem(&mut pio, 0, nop());
    write_imem(&mut pio, 1, pull(true));
    assert!(pio.step_sm(0));
    assert!(!pio.step_sm(0));
    assert_ne!(pio.sm[0].execctrl & EXECCTRL_EXEC_STALLED, 0);
    pio.write32(CTRL, 1 | SM_RESTART);
    assert_eq!(pio.read32(SM0_CLKDIV + SM_ADDR), Some(0));
    assert_eq!(pio.sm[0].execctrl & EXECCTRL_EXEC_STALLED, 0);
    assert_eq!(pio.read32(CTRL), Some(1));
}
