//! PIO register-block tests / PIO 寄存器块测试

// 测试断言直接使用数据手册寄存器常量，通配导入保持命名直观。
// Tests assert against datasheet register constants, so the glob import keeps
// the naming faithful to the datasheet.
#![allow(clippy::wildcard_imports)]

use rp2350_core::bus::AddressBus;
use rp2350_core::memory::SimpleBus;
use rp2350_core::mmio::{MmioDevice, mount_device};

use super::regs::*;
use super::*;

#[test]
fn xor_alias_updates_clkdiv_and_keeps_the_divider_phase() {
    let mut pio = Pio::new(2);
    let clkdiv = SM0_CLKDIV + SM_STRIDE;
    assert!(pio.write32(clkdiv, 0x0001_0000));
    pio.sm[1].clock_phase = 7;
    // 原子别名做读-改-写；`SMx_CLKDIV` 没有重启位，相位累加器照常保留。
    // The atomic alias read-modify-writes; `SMx_CLKDIV` has no restart bit, so
    // the phase accumulator keeps running.
    assert!(pio.write32(clkdiv + 0x1000, 0x8000_0000));
    assert_eq!(pio.read32(clkdiv), Some(0x8001_0000));
    assert_eq!(pio.sm[1].clock_phase, 7);
}

#[test]
fn set_and_clear_aliases_update_pinctrl() {
    let mut pio = Pio::new(0);
    let pinctrl = SM0_CLKDIV + SM_PINCTRL;
    assert_eq!(pio.read32(pinctrl), Some(PINCTRL_RESET));
    assert!(pio.write32(pinctrl + 0x2000, 0x0000_0020));
    assert_eq!(pio.read32(pinctrl), Some(PINCTRL_RESET | 0x20));
    assert!(pio.write32(pinctrl + 0x3000, 0x0000_0020));
    assert_eq!(pio.read32(pinctrl), Some(PINCTRL_RESET));
}

#[test]
fn pio_window_covers_the_alias_regions() {
    let pio = Pio::new(2);
    assert_eq!(pio.window().size, PIO_WINDOW_SIZE);
    assert_eq!(PIO_WINDOW_SIZE, 0x4000);
}

#[test]
fn reset_values_match_the_datasheet() {
    let mut pio = Pio::new(0);
    assert_eq!(pio.read32(CTRL), Some(0));
    assert_eq!(pio.read32(FSTAT), Some(0x0F00_0F00));
    assert_eq!(pio.read32(FDEBUG), Some(0));
    assert_eq!(pio.read32(FLEVEL), Some(0));
    assert_eq!(pio.read32(IRQ), Some(0));
    assert_eq!(pio.read32(DBG_CFGINFO), Some(DBG_CFGINFO_VALUE));
    assert_eq!(pio.read32(GPIOBASE), Some(0));
    for index in 0..SM_COUNT {
        let base = SM0_CLKDIV + index as u32 * SM_STRIDE;
        assert_eq!(pio.read32(base + SM_CLKDIV), Some(CLKDIV_RESET));
        assert_eq!(pio.read32(base + SM_EXECCTRL), Some(EXECCTRL_RESET));
        assert_eq!(pio.read32(base + SM_SHIFTCTRL), Some(SHIFTCTRL_RESET));
        assert_eq!(pio.read32(base + SM_ADDR), Some(0));
        assert_eq!(pio.read32(base + SM_INSTR), Some(0));
        assert_eq!(pio.read32(base + SM_PINCTRL), Some(PINCTRL_RESET));
    }
}

#[test]
fn ctrl_reads_back_only_the_enable_bits() {
    let mut pio = Pio::new(0);
    assert!(pio.write32(CTRL, 0xFFFF_FFFF));
    assert_eq!(pio.read32(CTRL), Some(0x0000_000F));
    assert!(pio.write32(CTRL, 0x0000_0005));
    assert_eq!(pio.read32(CTRL), Some(5));
}

#[test]
fn sm_register_writes_apply_the_datasheet_masks() {
    let mut pio = Pio::new(1);
    assert!(pio.write32(SM0_CLKDIV, 0x1234_5678));
    assert_eq!(pio.read32(SM0_CLKDIV), Some(0x1234_5600));
    assert!(pio.write32(SM0_CLKDIV + SM_EXECCTRL, 0xFFFF_FFFF));
    assert_eq!(
        pio.read32(SM0_CLKDIV + SM_EXECCTRL),
        Some(EXECCTRL_WRITE_MASK)
    );
    assert!(pio.write32(SM0_CLKDIV + SM_SHIFTCTRL, 0xFFFF_FFFF));
    assert_eq!(pio.read32(SM0_CLKDIV + SM_SHIFTCTRL), Some(SHIFTCTRL_MASK));
    assert!(pio.write32(SM0_CLKDIV + SM_PINCTRL, 0xDEAD_BEEF));
    assert_eq!(pio.read32(SM0_CLKDIV + SM_PINCTRL), Some(0xDEAD_BEEF));
    // `SMx_ADDR` 只读，写入不改变 PC。`SMx_ADDR` is read-only.
    assert!(pio.write32(SM0_CLKDIV + SM_ADDR, 17));
    assert_eq!(pio.read32(SM0_CLKDIV + SM_ADDR), Some(0));
}

#[test]
fn instruction_memory_holds_32_sixteen_bit_words() {
    let mut pio = Pio::new(2);
    for slot in 0..IMEM_WORDS {
        let offset = INSTR_MEM0 + slot as u32 * 4;
        assert!(pio.write32(offset, 0xFFFF_0000 | slot as u32));
        assert_eq!(pio.read32(offset), Some(slot as u32));
    }
    assert_eq!(pio.read32(INSTR_MEM31), Some(31));
    // SMx_INSTR 读取 PC 处的指令。SMx_INSTR reads the instruction at PC.
    assert!(pio.write32(INSTR_MEM0, 0x1234));
    assert_eq!(pio.read32(SM0_CLKDIV + SM_INSTR), Some(0x1234));
}

#[test]
fn tx_fifo_pushes_four_words_then_reports_full_and_overrun() {
    let mut pio = Pio::new(0);
    for word in 1..=4u32 {
        assert!(pio.write32(TXF0, word));
    }
    assert_eq!(
        pio.read32(FSTAT).expect("fstat") & (1 << 16),
        1 << 16,
        "SM0 TXFULL"
    );
    assert_eq!(pio.read32(FLEVEL).expect("flevel") & 0xF, 4);
    assert!(pio.write32(TXF0, 5));
    assert_eq!(
        pio.read32(FLEVEL).expect("flevel") & 0xF,
        4,
        "overflow word is dropped"
    );
    assert_eq!(
        pio.read32(FDEBUG).expect("fdebug") & FDEBUG_TXOVER,
        FDEBUG_TXOVER
    );
}

#[test]
fn tx_fifo_pull_returns_words_in_order_and_tracks_empty() {
    let mut pio = Pio::new(0);
    for word in 0x11..=0x14 {
        pio.write32(TXF0, word);
    }
    for word in 0x11..=0x14 {
        assert_eq!(pio.pop_tx(0), Some(word));
    }
    assert_eq!(pio.pop_tx(0), None);
    assert_eq!(pio.read32(FLEVEL), Some(0));
    assert_eq!(
        pio.read32(FSTAT).expect("fstat") & (1 << 24),
        1 << 24,
        "SM0 TXEMPTY"
    );
}

#[test]
fn rx_fifo_pops_in_order_and_underflow_sets_fdebug() {
    let mut pio = Pio::new(0);
    for word in 0x21..=0x24 {
        assert!(pio.push_rx(2, word));
    }
    assert_eq!(
        pio.read32(FSTAT).expect("fstat") & (1 << 2),
        1 << 2,
        "SM2 RXFULL"
    );
    for word in 0x21..=0x24 {
        assert_eq!(pio.read32(RXF0 + 8), Some(word));
    }
    assert_eq!(pio.read32(RXF0 + 8), Some(0));
    assert_eq!(
        pio.read32(FDEBUG).expect("fdebug") & (FDEBUG_RXUNDER << 2),
        FDEBUG_RXUNDER << 2,
        "SM2 RXUNDER"
    );
}

#[test]
fn fdebug_stall_bits_are_write_one_to_clear() {
    let mut pio = Pio::new(0);
    pio.fdebug = FDEBUG_TXSTALL | FDEBUG_RXSTALL;
    assert!(pio.write32(FDEBUG, 0));
    assert_eq!(pio.read32(FDEBUG), Some(FDEBUG_TXSTALL | FDEBUG_RXSTALL));
    assert!(pio.write32(FDEBUG, FDEBUG_RXSTALL));
    assert_eq!(pio.read32(FDEBUG), Some(FDEBUG_TXSTALL));
    assert!(pio.write32(FDEBUG, FDEBUG_TXSTALL));
    assert_eq!(pio.read32(FDEBUG), Some(0));
}

#[test]
fn irq_flags_are_forced_and_write_one_cleared() {
    let mut pio = Pio::new(0);
    assert!(pio.write32(IRQ_FORCE, 0x5A));
    assert_eq!(pio.read32(IRQ), Some(0x5A));
    assert!(pio.write32(IRQ, 0x08));
    assert_eq!(pio.read32(IRQ), Some(0x52));
}

#[test]
fn reset_restores_power_on_defaults() {
    let mut pio = Pio::new(2);
    pio.write32(CTRL, 0x0F);
    pio.write32(TXF3, 0xAA);
    pio.write32(INSTR_MEM0, 0x1234);
    pio.fdebug = FDEBUG_TXSTALL;
    MmioDevice::reset(&mut pio);
    assert_eq!(pio.read32(CTRL), Some(0));
    assert_eq!(pio.read32(FSTAT), Some(0x0F00_0F00));
    assert_eq!(pio.read32(INSTR_MEM0), Some(0));
    assert_eq!(pio.read32(FDEBUG), Some(0));
}

#[test]
fn bus_mount_exposes_the_register_block() {
    let mut bus = SimpleBus::new();
    mount_device(&mut bus, Box::new(Pio::new(1))).expect("mount");
    let mut cycles = 0;
    assert_eq!(
        bus.read_u32(map::PIO1_BASE + DBG_CFGINFO, &mut cycles)
            .expect("cfg"),
        DBG_CFGINFO_VALUE
    );
    bus.write_u32(map::PIO1_BASE + INSTR_MEM0 + 8, 0x1234, &mut cycles)
        .expect("imem write");
    assert_eq!(
        bus.read_u32(map::PIO1_BASE + INSTR_MEM0 + 8, &mut cycles)
            .expect("imem read"),
        0x1234
    );
}

#[test]
#[should_panic(expected = "PIO index")]
fn invalid_pio_index_is_rejected() {
    let _ = Pio::new(3);
}
