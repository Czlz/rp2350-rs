//! Shared PIO handle, host tick and IRQ-routing tests
//! 共享 PIO 句柄、宿主 tick 与中断路由测试

// 测试断言直接使用数据手册寄存器常量，通配导入保持命名直观。
// Tests assert against datasheet register constants, so the glob import keeps
// the naming faithful to the datasheet.
#![allow(clippy::wildcard_imports)]

use rp2350_core::bus::AddressBus;
use rp2350_core::memory::SimpleBus;
use rp2350_core::mmio::MmioDevice;

use super::regs::*;
use super::{PIO_BLOCKS, Pio, SharedPio};
use crate::BootPeripherals;
use crate::irq::{
    PIO_IRQ_LINES, PIO0_IRQ_0, PIO0_IRQ_1, PIO1_IRQ_0, PIO1_IRQ_1, PIO2_IRQ_0, PIO2_IRQ_1,
};
use crate::map;

/// `SET destination, data` / `SET destination, data`
fn set(dest: u8, data: u8) -> u16 {
    0xE000 | (u16::from(dest) << 5) | (u16::from(data) & 0x1F)
}

/// `IRQ set/clear/wait, field` / `IRQ set/clear/wait, field`
fn irq_instr(clear: bool, wait: bool, field: u8) -> u16 {
    let flags = if clear { 0x40 } else { 0 } | if wait { 0x20 } else { 0 };
    0xC000 | flags | (u16::from(field) & 0x1F)
}

/// `PULL block` / 阻塞式 `PULL`
fn pull_blocking() -> u16 {
    0x80A0
}

/// Mount the boot peripherals on a fresh bus / 在全新总线上挂载启动外设
fn mount() -> (SimpleBus, BootPeripherals) {
    let mut bus = SimpleBus::new();
    let peripherals = BootPeripherals::mount(&mut bus).expect("mount");
    (bus, peripherals)
}

/// Write one word through the bus / 通过总线写入一个字
fn write(bus: &mut SimpleBus, addr: u32, value: u32) {
    let mut cycles = 0u64;
    bus.write_u32(addr, value, &mut cycles).expect("bus write");
}

/// Read one word through the bus / 通过总线读取一个字
fn read(bus: &mut SimpleBus, addr: u32) -> u32 {
    let mut cycles = 0u64;
    bus.read_u32(addr, &mut cycles).expect("bus read")
}

/// Enable SM0 of a block through `CTRL` / 通过 `CTRL` 使能块的 SM0
fn enable_sm0(bus: &mut SimpleBus, base: u32) {
    write(bus, base + CTRL, 1);
}

#[test]
fn shared_handle_tick_advances_the_guest_program() {
    // 通过 MMIO 写入指令并使能后，宿主的 tick 必须推进同一批状态机。
    // After programming instructions and the enable bit through MMIO, the host
    // tick must advance those very state machines.
    let (mut bus, peripherals) = mount();
    let base = map::PIO0_BASE;
    write(&mut bus, base + INSTR_MEM0, u32::from(set(0, 1)));
    write(&mut bus, base + INSTR_MEM0 + 4, u32::from(set(4, 1)));
    enable_sm0(&mut bus, base);

    let pio: &SharedPio = peripherals.pio(0).expect("PIO0");
    assert_eq!(pio.index(), 0);
    assert_eq!(pio.enabled_mask(), 1);
    assert_eq!(pio.pc(0), Some(0));
    assert_eq!(pio.with(Pio::input_levels), 0);

    pio.tick(1);
    assert_eq!(pio.pc(0), Some(1));
    assert_eq!(pio.output_levels(), 1);
    assert_eq!(pio.output_enables(), 0);
    assert_eq!(read(&mut bus, base + SM0_CLKDIV + SM_ADDR), 1);
    assert_eq!(read(&mut bus, base + DBG_PADOUT), 1);

    pio.tick(1);
    assert_eq!(pio.pc(0), Some(2));
    assert_eq!(pio.output_levels(), 1);
    assert_eq!(pio.output_enables(), 1);
    assert_eq!(read(&mut bus, base + DBG_PADOE), 1);
    assert_eq!(read(&mut bus, base + CTRL), 1, "enable survives ticking");
}

#[test]
fn clone_and_bus_share_one_register_file() {
    // 句柄的克隆、总线适配器与宿主必须看到同一份寄存器与 FIFO。
    // Handle clones, the bus adapter and the host must observe one register
    // file and one set of FIFOs.
    let (mut bus, peripherals) = mount();
    let base = map::PIO1_BASE;
    let pio = peripherals.pio(1).expect("PIO1").clone();

    write(&mut bus, base + TXF0, 0x1234_5678);
    assert_eq!(pio.with_mut(|regs| regs.pop_tx(0)), Some(0x1234_5678));

    pio.with_mut(|regs| assert!(MmioDevice::write32(regs, INSTR_MEM0, 0xCAFE)));
    assert_eq!(read(&mut bus, base + INSTR_MEM0), 0xCAFE);
    assert_eq!(
        pio.with_mut(|regs| MmioDevice::read32(regs, INSTR_MEM0)),
        Some(0xCAFE)
    );
}

#[test]
fn irq_instruction_asserts_the_first_line() {
    // 标志 0..=3 由 IRQ_0 线承载。Flags 0..=3 ride the IRQ_0 line.
    let (mut bus, peripherals) = mount();
    let base = map::PIO0_BASE;
    write(
        &mut bus,
        base + INSTR_MEM0,
        u32::from(irq_instr(false, false, 3)),
    );
    enable_sm0(&mut bus, base);

    peripherals.pio(0).expect("PIO0").tick(1);

    let mask = peripherals.irq_mask();
    assert_ne!(mask & (1 << PIO0_IRQ_0), 0, "IRQ flag 3 asserts IRQ_0");
    assert_eq!(mask & (1 << PIO0_IRQ_1), 0, "IRQ_1 stays released");
    assert_eq!(peripherals.pio(0).expect("PIO0").irq_flags(), 1 << 3);
}

#[test]
fn irq_flags_four_to_seven_use_the_second_line() {
    // 标志 4..=7 由 IRQ_1 线承载。Flags 4..=7 ride the IRQ_1 line.
    let (mut bus, peripherals) = mount();
    let base = map::PIO0_BASE;
    write(
        &mut bus,
        base + INSTR_MEM0,
        u32::from(irq_instr(false, false, 5)),
    );
    enable_sm0(&mut bus, base);

    peripherals.pio(0).expect("PIO0").tick(1);

    let mask = peripherals.irq_mask();
    assert_ne!(mask & (1 << PIO0_IRQ_1), 0, "IRQ flag 5 asserts IRQ_1");
    assert_eq!(mask & (1 << PIO0_IRQ_0), 0, "IRQ_0 stays released");
}

#[test]
fn write_one_to_clear_and_force_track_the_lines() {
    // IRQ_FORCE 置位、写 1 清零必须同步更新共享线。
    // IRQ_FORCE assertions and write-one-to-clear releases must update the
    // shared lines.
    let (mut bus, peripherals) = mount();
    let base = map::PIO2_BASE;

    write(&mut bus, base + IRQ_FORCE, 0x0000_0081);
    let mask = peripherals.irq_mask();
    assert_ne!(mask & (1 << PIO2_IRQ_0), 0, "flag 0");
    assert_ne!(mask & (1 << PIO2_IRQ_1), 0, "flag 7");

    write(&mut bus, base + IRQ, 1 << 0);
    assert_eq!(
        peripherals.irq_mask() & (1 << PIO2_IRQ_0),
        0,
        "flag 0 cleared"
    );
    assert_ne!(peripherals.irq_mask() & (1 << PIO2_IRQ_1), 0, "flag 7 kept");

    write(&mut bus, base + IRQ, 1 << 7);
    assert_eq!(peripherals.irq_mask(), 0, "both lines released");
    assert_eq!(read(&mut bus, base + IRQ), 0);
}

#[test]
fn blocks_use_distinct_interrupt_lines() {
    // 三个块的线号互不混淆：只允许被置位的块断言自己的线。
    // The three blocks never cross lines: only the flagged block asserts its
    // own pair.
    let (mut bus, peripherals) = mount();
    let bases = [map::PIO0_BASE, map::PIO1_BASE, map::PIO2_BASE];
    write(&mut bus, bases[0] + IRQ_FORCE, 0x0000_0001);
    write(&mut bus, bases[1] + IRQ_FORCE, 0x0000_0001);
    write(&mut bus, bases[2] + IRQ_FORCE, 0x0000_0010);

    let mask = peripherals.irq_mask();
    assert_eq!(
        mask & 0x001F_F000,
        1 << PIO0_IRQ_0 | 1 << PIO1_IRQ_0 | 1 << PIO2_IRQ_1,
        "only the flagged lines are asserted"
    );
    assert_eq!(PIO_IRQ_LINES.len(), PIO_BLOCKS);

    for (index, base) in bases.iter().enumerate() {
        write(&mut bus, base + IRQ, 0x0000_00FF);
        for line in PIO_IRQ_LINES[index] {
            assert_eq!(peripherals.irq_mask() & (1 << line), 0, "line {line}");
        }
    }
}

#[test]
fn disabled_block_tick_is_a_no_op() {
    // 未使能时宿主 tick 不得执行指令、不得置位标志或中断线。
    // A host tick on a disabled block must not execute instructions or raise
    // flags and lines.
    let (mut bus, peripherals) = mount();
    let base = map::PIO0_BASE;
    write(
        &mut bus,
        base + INSTR_MEM0,
        u32::from(irq_instr(false, false, 1)),
    );

    let pio = peripherals.pio(0).expect("PIO0");
    pio.tick(64);

    assert_eq!(pio.enabled_mask(), 0);
    assert_eq!(pio.pc(0), Some(0));
    assert_eq!(pio.irq_flags(), 0);
    assert_eq!(peripherals.irq_mask(), 0);
}

#[test]
fn enabled_and_stalled_masks_track_the_machines() {
    let (mut bus, peripherals) = mount();
    let base = map::PIO0_BASE;
    write(&mut bus, base + INSTR_MEM0, u32::from(pull_blocking()));
    enable_sm0(&mut bus, base);

    let pio = peripherals.pio(0).expect("PIO0");
    pio.tick(1);
    assert_eq!(pio.enabled_mask(), 1);
    assert_eq!(pio.stalled_mask(), 1, "empty TX stalls PULL");
    assert_eq!(pio.pc(0), Some(0));

    write(&mut bus, base + TXF0, 0xABCD);
    pio.tick(1);
    assert_eq!(pio.stalled_mask(), 0);
    assert_eq!(pio.pc(0), Some(1));
}

#[test]
fn pio_handle_bounds_are_checked() {
    // 越界序号不得 panic，也不得执行闭包。
    // Out-of-range indices must not panic and must not run the closure.
    let (_bus, peripherals) = mount();
    assert!(peripherals.pio(0).is_some());
    assert!(peripherals.pio(PIO_BLOCKS - 1).is_some());
    assert!(peripherals.pio(PIO_BLOCKS).is_none());
    assert_eq!(peripherals.with_pio(PIO_BLOCKS, |_| 1u8), None);

    let mut ran = false;
    assert_eq!(peripherals.with_pio(2, |pio| pio.input_levels()), Some(0));
    assert!(peripherals.with_pio(0, |_| ran = true).is_some());
    assert!(ran);
}

#[test]
fn reset_releases_the_block_lines() {
    // 复位清空 IRQ 标志并释放两条线。
    // Reset clears the IRQ flags and releases both lines.
    let (mut bus, peripherals) = mount();
    let base = map::PIO1_BASE;
    write(&mut bus, base + IRQ_FORCE, 0x0000_00F0);
    assert_ne!(peripherals.irq_mask() & (1 << PIO1_IRQ_1), 0);

    let pio = peripherals.pio(1).expect("PIO1");
    pio.with_mut(MmioDevice::reset);

    assert_eq!(pio.irq_flags(), 0);
    assert_eq!(peripherals.irq_mask() & 0x001F_F000, 0);
    assert_eq!(pio.enabled_mask(), 0);
}

#[test]
fn pump_pio_irqs_is_a_deterministic_resync() {
    // 重同步不得改变已经正确的线状态。
    // Resynchronising must not change an already correct line state.
    let (mut bus, peripherals) = mount();
    write(&mut bus, map::PIO0_BASE + IRQ_FORCE, 0x0000_000F);
    peripherals.pump_pio_irqs();
    assert_ne!(peripherals.irq_mask() & (1 << PIO0_IRQ_0), 0);

    write(&mut bus, map::PIO0_BASE + IRQ, 0x0000_000F);
    peripherals.pump_pio_irqs();
    assert_eq!(peripherals.irq_mask(), 0);
}
