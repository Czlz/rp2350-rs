//! IO_BANK0 `FUNCSEL`/`STATUS` tests / IO_BANK0 `FUNCSEL`/`STATUS` 测试

use rp2350_core::bus::AddressBus;
use rp2350_core::memory::SimpleBus;
use rp2350_core::mmio::MmioDevice;

use crate::IrqAssert;
use crate::boot::BootPeripherals;
use crate::gpio_pads::{GpioPads, PAD_COUNT, PAD_PUE};
use crate::io_bank0::{IoBank0, PinFunction};
use crate::map;
use crate::pio::Pio;
use crate::sio::Sio;

/// `STATUS.INFROMPAD`（bit 17）/ `STATUS.INFROMPAD` (bit 17)
const STATUS_INFROMPAD: u32 = 1 << 17;

/// PIO `CTRL` / PIO `CTRL`
const PIO_CTRL: u32 = 0x000;

/// PIO `TXF0` / PIO `TXF0`
const PIO_TXF0: u32 = 0x010;

/// PIO `INSTR_MEM0` / PIO `INSTR_MEM0`
const PIO_INSTR_MEM0: u32 = 0x048;

/// PIO `GPIOBASE` / PIO `GPIOBASE`
const PIO_GPIOBASE: u32 = 0x168;

/// PIO `SM0_PINCTRL` (`SM0_CLKDIV + 0x14`) / PIO `SM0_PINCTRL`
const PIO_SM0_PINCTRL: u32 = 0x0C8 + 0x14;

/// `PINCTRL.SET_BASE` bit position / `PINCTRL.SET_BASE` 位位置
const PINCTRL_SET_BASE: u32 = 5;

/// `PINCTRL.IN_BASE` bit position / `PINCTRL.IN_BASE` 位位置
const PINCTRL_IN_BASE: u32 = 15;

/// `PINCTRL.OUT_COUNT` bit position / `PINCTRL.OUT_COUNT` 位位置
const PINCTRL_OUT_COUNT: u32 = 20;

/// `PINCTRL.SET_COUNT` bit position / `PINCTRL.SET_COUNT` 位位置
const PINCTRL_SET_COUNT: u32 = 26;

/// `SET PINS, data` (`dest = 0`) / `SET PINS, data`
fn set_pins(data: u8) -> u32 {
    0xE000 | (u32::from(data) & 0x1F)
}

/// `SET PINDIRS, data` (`dest = 4`) / `SET PINDIRS, data`
fn set_pindirs(data: u8) -> u32 {
    0xE000 | (4 << 5) | (u32::from(data) & 0x1F)
}

/// `OUT PINS, count` (`dest = 0`) / `OUT PINS, count`
fn out_pins(count: u8) -> u32 {
    0x6000 | (u32::from(count) & 0x1F)
}

/// `OUT PINDIRS, count` (`dest = 4`) / `OUT PINDIRS, count`
fn out_pindirs(count: u8) -> u32 {
    0x6000 | (4 << 5) | (u32::from(count) & 0x1F)
}

/// `IN PINS, count` / `IN PINS, count`
fn in_pins(count: u8) -> u32 {
    0x4000 | (u32::from(count) & 0x1F)
}

/// Offset of `GPIOx_STATUS` / `GPIOx_STATUS` 偏移
const fn status(pin: u32) -> u32 {
    8 * pin
}

/// Offset of `GPIOx_CTRL` / `GPIOx_CTRL` 偏移
const fn ctrl(pin: u32) -> u32 {
    8 * pin + 4
}

#[test]
fn ctrl_resets_to_null_for_every_pin() {
    // 复位后全部 48 个 `CTRL.FUNCSEL` 都是 `NULL`（0x1F）。
    // After reset every one of the 48 `CTRL.FUNCSEL` fields is `NULL` (0x1F).
    let mut io = IoBank0::new();
    for pin in 0..PAD_COUNT {
        assert_eq!(io.read32(ctrl(pin)), Some(0x1F), "GPIO{pin}_CTRL reset");
        assert_eq!(io.funcsel(pin), Some(PinFunction::Null), "GPIO{pin}");
    }
}

#[test]
fn selecting_sio_pio_and_uart_round_trips() {
    // 通过访问器选择功能，注册表与 `funcsel()` 必须一致。
    // Selecting through the accessor must keep the register and `funcsel()` in
    // agreement.
    let mut io = IoBank0::new();
    for (pin, function) in [
        (0, PinFunction::Sio),
        (1, PinFunction::Pio0),
        (2, PinFunction::Pio2),
        (3, PinFunction::Uart),
        (4, PinFunction::Pio1),
    ] {
        io.set_funcsel(pin, function);
        assert_eq!(io.funcsel(pin), Some(function), "GPIO{pin} accessor");
        assert_eq!(
            io.read32(ctrl(pin)),
            Some(function.to_bits()),
            "GPIO{pin}_CTRL"
        );
    }

    // 直接写寄存器也必须被访问器解码。 / A raw register write must be decoded too.
    assert!(io.write32(ctrl(5), PinFunction::Hstx.to_bits()));
    assert_eq!(io.funcsel(5), Some(PinFunction::Hstx));
}

#[test]
fn funcsel_agrees_with_the_ctrl_field() {
    // 只有 bits[4:0] 参与解码，高位覆盖位不影响功能选择。
    // Only bits[4:0] take part in decoding; the upper override bits do not
    // change the function selection.
    let mut io = IoBank0::new();
    assert!(io.write32(ctrl(7), 0xFFFF_FFE6));
    assert_eq!(io.funcsel(7), Some(PinFunction::Pio0));

    for bits in 0..=0x1F {
        assert!(io.write32(ctrl(7), bits));
        let decoded = io.funcsel(7).expect("decoded");
        assert_eq!(decoded.to_bits(), bits, "FUNCSEL {bits:#04X}");
    }
}

#[test]
fn pins_outside_bank_zero_return_none() {
    // pin >= 48 不存在：访问器返回 `None`，写访问器被忽略。
    // Pins >= 48 do not exist: the accessor returns `None` and the setter is
    // ignored.
    let mut io = IoBank0::new();
    assert_eq!(io.funcsel(PAD_COUNT), None);
    assert_eq!(io.funcsel(63), None);
    io.set_funcsel(PAD_COUNT, PinFunction::Sio);
    assert_eq!(io.read32(ctrl(PAD_COUNT)), Some(0), "no stray write");

    // 边界上的 pin 47 仍然有效。 / Pin 47, the last pin, is still valid.
    io.set_funcsel(47, PinFunction::I2c);
    assert_eq!(io.funcsel(47), Some(PinFunction::I2c));
}

#[test]
fn undocumented_funcsel_values_are_reported() {
    // 12..=30 是未文档化编码，以 `Unknown` 回报；超出 5 位字段返回 `None`。
    // 12..=30 are undocumented encodings reported as `Unknown`; values that do
    // not fit the 5-bit field return `None`.
    let mut io = IoBank0::new();
    assert!(io.write32(ctrl(8), 0x0C));
    assert_eq!(io.funcsel(8), Some(PinFunction::Unknown(12)));
    assert!(io.write32(ctrl(8), 0x1E));
    assert_eq!(io.funcsel(8), Some(PinFunction::Unknown(30)));

    assert_eq!(PinFunction::from_bits(0x0C), Some(PinFunction::Unknown(12)));
    assert_eq!(PinFunction::Unknown(30).to_bits(), 30);
    assert_eq!(PinFunction::from_bits(0x20), None);
    assert_eq!(PinFunction::from_bits(u32::MAX), None, "out-of-field value");
}

#[test]
fn every_documented_encoding_round_trips() {
    // 全部 32 个 5 位编码都能解码并原样编码回去。
    // All 32 five-bit encodings decode and encode back unchanged.
    for bits in 0..=0x1F {
        let function = PinFunction::from_bits(bits).expect("5-bit value");
        assert_eq!(function.to_bits(), bits, "encoding {bits:#04X}");
    }
    assert_eq!(PinFunction::from_bits(0x1F), Some(PinFunction::Null));
    assert_eq!(PinFunction::from_bits(0x09), Some(PinFunction::Gpck));
    assert_eq!(PinFunction::from_bits(0x0A), Some(PinFunction::Usb));
    assert_eq!(PinFunction::from_bits(0x0B), Some(PinFunction::UartAux));
}

#[test]
fn set_funcsel_keeps_the_other_ctrl_bits() {
    // `set_funcsel` 只改 bits[4:0]，输出/输入/输出使能覆盖位保持不变。
    // `set_funcsel` touches bits[4:0] only; the output/input/output-enable
    // override bits are preserved.
    let mut io = IoBank0::new();
    let overrides = 0xFFFF_FFE0;
    assert!(io.write32(ctrl(10), overrides | 0x05));
    io.set_funcsel(10, PinFunction::Pwm);
    assert_eq!(
        io.read32(ctrl(10)),
        Some(overrides | PinFunction::Pwm.to_bits())
    );
    assert_eq!(io.funcsel(10), Some(PinFunction::Pwm));

    // 再次设置为 `NULL` 同样只改低位。 / Setting `NULL` again is equally narrow.
    io.set_funcsel(10, PinFunction::Null);
    assert_eq!(io.read32(ctrl(10)), Some(overrides | 0x1F));
}

#[test]
fn status_infrompad_tracks_the_shared_pads() {
    // `STATUS.INFROMPAD` 实时跟随共享 pad 的上下拉与外部电平。
    // `STATUS.INFROMPAD` follows the shared pads' pulls and external levels.
    let pads = GpioPads::new();
    let mut io = IoBank0::with_pads(pads.clone());
    assert_eq!(io.read32(status(3)), Some(0), "reset pull-down reads low");

    pads.set_pad(3, PAD_PUE);
    assert_eq!(io.read32(status(3)), Some(STATUS_INFROMPAD));
    assert_eq!(io.read32(status(4)), Some(0), "other pins stay low");

    // 宿主注入低电平覆盖上拉。 / A host-driven low overrides the pull-up.
    pads.set_external_pin(3, false);
    assert_eq!(io.read32(status(3)), Some(0));

    // 高引脚（47）同样有效。 / The highest pin (47) works as well.
    pads.set_external_pin(47, true);
    assert_eq!(io.read32(status(47)), Some(STATUS_INFROMPAD));
}

#[test]
fn status_writes_are_ignored() {
    // `STATUS` 在硬件上只读：写入被确认但任何位都不改变。
    // `STATUS` is read-only in hardware: writes are acknowledged but no bit
    // changes.
    let pads = GpioPads::new();
    let mut io = IoBank0::with_pads(pads.clone());
    assert!(io.write32(status(0), 0xFFFF_FFFF));
    assert_eq!(io.read32(status(0)), Some(0));
    assert!(io.write32(status(0), 0x0002_0200));
    assert_eq!(io.read32(status(0)), Some(0), "OUTTOPAD/OETOPAD stay clear");

    // 即便写入被忽略，实时 pad 电平仍然出现在 INFROMPAD。
    // Even though the write is dropped, the live pad level still appears in
    // INFROMPAD.
    pads.set_pad(0, PAD_PUE);
    assert_eq!(io.read32(status(0)), Some(STATUS_INFROMPAD));
}

#[test]
fn reset_restores_null_and_clears_other_bits() {
    // `reset()` 恢复 `FUNCSEL = NULL` 并清掉其它位，但不复位共享 pad 状态。
    // `reset()` restores `FUNCSEL = NULL` and clears the other bits, without
    // resetting the shared pad state.
    let pads = GpioPads::new();
    let mut io = IoBank0::with_pads(pads.clone());
    io.set_funcsel(9, PinFunction::Sio);
    assert!(io.write32(ctrl(9), 0xFFFF_FFFF));
    pads.set_pad(9, PAD_PUE);

    io.reset();
    assert_eq!(io.read32(ctrl(9)), Some(0x1F), "CTRL back to NULL");
    assert_eq!(io.funcsel(9), Some(PinFunction::Null));
    assert_eq!(
        io.read32(status(9)),
        Some(STATUS_INFROMPAD),
        "pad state survives the block reset"
    );
}

#[test]
fn boot_peripherals_mount_multiple_blocks_on_shared_pads() {
    // `BOOT_PERIPHERALS` 挂载后，总线读写与新建块语义一致，且与
    // `BootPeripherals::pads()` 共享同一个 pad 句柄。
    // After `BOOT_PERIPHERALS` mounts, bus accesses match a fresh block and
    // share the `BootPeripherals::pads()` handle.
    let mut bus = SimpleBus::new();
    let peripherals = BootPeripherals::mount(&mut bus).expect("mount");
    let mut cycles = 0u64;

    assert_eq!(
        bus.read_u32(map::IO_BANK0_BASE + ctrl(0), &mut cycles)
            .expect("GPIO0_CTRL"),
        0x1F
    );
    bus.write_u32(
        map::IO_BANK0_BASE + ctrl(0),
        PinFunction::Sio.to_bits(),
        &mut cycles,
    )
    .expect("GPIO0_CTRL write");
    assert_eq!(
        bus.read_u32(map::IO_BANK0_BASE + ctrl(0), &mut cycles)
            .expect("GPIO0_CTRL"),
        0x05
    );

    peripherals.pads().set_pad(0, PAD_PUE);
    assert_eq!(
        bus.read_u32(map::IO_BANK0_BASE + status(0), &mut cycles)
            .expect("GPIO0_STATUS"),
        STATUS_INFROMPAD
    );
}

/// `SIO.GPIO_IN` offset / `SIO.GPIO_IN` 偏移
const SIO_GPIO_IN: u32 = 0x004;

/// `SIO.GPIO_OUT` offset / `SIO.GPIO_OUT` 偏移
const SIO_GPIO_OUT: u32 = 0x010;

/// `SIO.GPIO_OE` offset / `SIO.GPIO_OE` 偏移
const SIO_GPIO_OE: u32 = 0x030;

/// `PIO.SM0_EXECCTRL` offset / `PIO.SM0_EXECCTRL` 偏移
const PIO_SM0_EXECCTRL: u32 = 0x0C8 + 0x04;

/// `PIO.RXF0` offset / `PIO.RXF0` 偏移
const PIO_RXF0: u32 = 0x020;

/// Build a SIO sharing `pads` / 创建共享 `pads` 的 SIO
fn sio_on(pads: &GpioPads) -> Sio {
    Sio::with_peer_irq_and_pads(IrqAssert::new(), IrqAssert::new(), pads.clone())
}

/// Build a PIO block wired to `pads` with `program` in IMEM
/// 创建接到 `pads` 的 PIO 块，并把 `program` 写入 IMEM
fn pio_on(index: u8, pads: &GpioPads, program: &[u32]) -> Pio {
    let mut pio = Pio::with_pads(index, pads.clone());
    for (slot, instr) in program.iter().enumerate() {
        assert!(
            pio.write32(PIO_INSTR_MEM0 + slot as u32 * 4, *instr),
            "IMEM{slot}"
        );
    }
    pio
}

#[test]
fn funcsel_sio_output_drives_the_pad() {
    // `FUNCSEL = SIO` 且 `GPIO_OE` 置位时，pad 由 SIO 输出锁存驱动。
    // With `FUNCSEL = SIO` and `GPIO_OE` set, the SIO latch drives the pad.
    let pads = GpioPads::new();
    let mut io = IoBank0::with_pads(pads.clone());
    let mut sio = sio_on(&pads);
    io.set_funcsel(4, PinFunction::Sio);

    assert!(sio.write32(SIO_GPIO_OE, 1 << 4));
    assert!(sio.write32(SIO_GPIO_OUT, 1 << 4));
    assert_eq!(pads.input_levels() & (1 << 4), 1 << 4, "pad driven high");
    assert_eq!(
        io.read32(status(4)),
        Some(STATUS_INFROMPAD),
        "STATUS.INFROMPAD follows the driven pad"
    );
    assert_eq!(sio.read32(SIO_GPIO_IN), Some(1 << 4));

    assert!(sio.write32(SIO_GPIO_OUT, 0));
    assert_eq!(pads.input_levels(), 0, "pad driven low");
}

#[test]
fn sio_oe_clear_returns_the_pad_to_its_pull() {
    // `GPIO_OE` 清零后 SIO 不再驱动 pad，上拉重新可见。
    // With `GPIO_OE` clear SIO no longer drives the pad and the pull is visible.
    let pads = GpioPads::new();
    let mut io = IoBank0::with_pads(pads.clone());
    let mut sio = sio_on(&pads);
    io.set_funcsel(4, PinFunction::Sio);
    pads.set_pad(4, PAD_PUE);

    assert_eq!(pads.input_levels(), 1 << 4, "pull-up visible while idle");
    assert!(sio.write32(SIO_GPIO_OE, 1 << 4));
    assert!(sio.write32(SIO_GPIO_OUT, 0));
    assert_eq!(pads.input_levels(), 0, "driven low beats the pull-up");
    assert!(sio.write32(SIO_GPIO_OE, 0));
    assert_eq!(pads.input_levels(), 1 << 4, "pull-up returns with OE clear");
}

#[test]
fn a_peripheral_driver_overrides_the_external_level() {
    // 外设驱动优先于宿主外部电平；取消功能选择后又回到外部电平。
    // The peripheral driver wins over the host level; deselecting it returns to
    // the host level.
    let pads = GpioPads::new();
    let mut io = IoBank0::with_pads(pads.clone());
    let mut sio = sio_on(&pads);
    io.set_funcsel(7, PinFunction::Sio);
    pads.set_external_pin(7, false);
    assert_eq!(pads.input_levels(), 0, "host drives low");

    assert!(sio.write32(SIO_GPIO_OE, 1 << 7));
    assert!(sio.write32(SIO_GPIO_OUT, 1 << 7));
    assert_eq!(pads.input_levels(), 1 << 7, "driver beats the host");

    io.set_funcsel(7, PinFunction::Uart);
    assert_eq!(pads.input_levels(), 0, "unmodelled UART does not drive");
}

#[test]
fn unmodelled_functions_leave_the_pad_to_its_pull() {
    // HSTX/SPI/UART/I2C/PWM/USB/GPCK 尚未建模 pad 驱动器。
    // HSTX/SPI/UART/I2C/PWM/USB/GPCK have no modelled pad driver yet.
    let pads = GpioPads::new();
    let mut io = IoBank0::with_pads(pads.clone());
    pads.set_pad(5, PAD_PUE);
    for function in [
        PinFunction::Hstx,
        PinFunction::Spi,
        PinFunction::Uart,
        PinFunction::I2c,
        PinFunction::Pwm,
        PinFunction::Gpck,
        PinFunction::Usb,
        PinFunction::UartAux,
        PinFunction::Null,
    ] {
        io.set_funcsel(5, function);
        assert_eq!(pads.input_levels(), 1 << 5, "{function:?} keeps the pull");
    }
}

#[test]
fn switching_funcsel_hands_the_pad_from_pio_to_sio() {
    // 切到 SIO 之后 PIO 不再驱动该 pad，切回 PIO 后恢复驱动。
    // After switching to SIO the PIO stops driving; switching back restores it.
    let pads = GpioPads::new();
    let mut io = IoBank0::with_pads(pads.clone());
    let mut pio = pio_on(0, &pads, &[set_pindirs(1), set_pins(1)]);
    pio.write32(
        PIO_SM0_PINCTRL,
        (8 << PINCTRL_SET_BASE) | (1 << PINCTRL_SET_COUNT),
    );
    io.set_funcsel(8, PinFunction::Pio0);
    assert!(pio.step_sm(0));
    assert!(pio.step_sm(0));
    pio.publish_outputs();
    assert_eq!(pads.input_levels(), 1 << 8, "PIO drives the pad");

    io.set_funcsel(8, PinFunction::Sio);
    assert_eq!(pads.input_levels(), 0, "PIO no longer owns the pad");
    io.set_funcsel(8, PinFunction::Pio0);
    assert_eq!(pads.input_levels(), 1 << 8, "PIO drives it again");
}

#[test]
fn reset_clears_every_pad_function() {
    // `IoBank0::reset` 把全部 `FUNCSEL` 恢复为 `NULL`，pad 回到上下拉/外部电平。
    // `IoBank0::reset` restores every `FUNCSEL` to `NULL`, so pads fall back to
    // their pulls and host levels.
    let pads = GpioPads::new();
    let mut io = IoBank0::with_pads(pads.clone());
    let mut sio = sio_on(&pads);
    for pin in [0, 9, 47] {
        io.set_funcsel(pin, PinFunction::Sio);
        pads.set_pad(pin, PAD_PUE);
    }
    assert!(sio.write32(SIO_GPIO_OE, 0xFFFF_FFFF));
    assert!(sio.write32(SIO_GPIO_OUT, 0xFFFF_FFFF));
    assert_eq!(
        pads.input_levels() & ((1 << 0) | (1 << 9) | (1 << 47)),
        (1 << 0) | (1 << 9) | (1 << 47)
    );

    io.reset();
    for pin in 0..PAD_COUNT {
        assert_eq!(pads.funcsel(pin), Some(0x1F), "pin {pin} back to NULL");
    }
    let pulled = (1 << 0) | (1 << 9) | (1 << 47);
    assert_eq!(pads.input_levels(), pulled, "pads fall back to their pulls");
}

#[test]
fn pins_for_reports_each_function() {
    // `IoBank0::pins_for` 与共享矩阵的选择器掩码一致。
    // `IoBank0::pins_for` agrees with the shared matrix selector mask.
    let pads = GpioPads::new();
    let mut io = IoBank0::with_pads(pads.clone());
    io.set_funcsel(2, PinFunction::Pio0);
    io.set_funcsel(5, PinFunction::Pio0);
    io.set_funcsel(6, PinFunction::Sio);
    io.set_funcsel(7, PinFunction::Pio2);

    assert_eq!(io.pins_for(PinFunction::Pio0), (1 << 2) | (1 << 5));
    assert_eq!(io.pins_for(PinFunction::Sio), 1 << 6);
    assert_eq!(io.pins_for(PinFunction::Pio2), 1 << 7);
    assert_eq!(io.pins_for(PinFunction::Pio1), 0);
    assert_eq!(
        io.pins_for(PinFunction::Null),
        (1u64 << PAD_COUNT) - 1 - ((1 << 2) | (1 << 5) | (1 << 6) | (1 << 7))
    );
    assert_eq!(
        pads.funcsel_mask(PinFunction::Pio0.to_bits()),
        1 << 2 | 1 << 5
    );
    assert_eq!(io.pads().funcsel(7), Some(PinFunction::Pio2.to_bits()));
}

#[test]
fn sio_gpio_in_reads_only_its_own_pins() {
    // `GPIO_IN` 只在 `FUNCSEL == SIO` 的位上返回 pad 电平，其它位保持回环/0。
    // `GPIO_IN` reports the pad only where `FUNCSEL == SIO`; other bits keep
    // the latch loopback and read 0.
    let pads = GpioPads::new();
    let mut io = IoBank0::with_pads(pads.clone());
    let mut sio = sio_on(&pads);
    for pin in 0..3 {
        pads.set_pad(pin, PAD_PUE);
    }
    io.set_funcsel(0, PinFunction::Sio);
    io.set_funcsel(1, PinFunction::Pio0);

    assert_eq!(sio.read32(SIO_GPIO_IN), Some(1), "only GPIO0 is routed");
    assert_eq!(pads.input_levels() & 0b111, 0b111, "pads are all high");

    // 关闭功能选择后同一位读 0。Clearing the function makes the bit read 0 again.
    io.set_funcsel(0, PinFunction::Null);
    assert_eq!(sio.read32(SIO_GPIO_IN), Some(0));
}

#[test]
fn pio_inputs_come_from_the_routed_pads() {
    // PIO 的 `IN PINS` 采样 `FUNCSEL` 选中本块的 pad；未选中的引脚读 0。
    // PIO `IN PINS` samples the pads routed to the block; unrouted pins read 0.
    let pads = GpioPads::new();
    let mut io = IoBank0::with_pads(pads.clone());
    pads.set_pad(2, PAD_PUE);
    pads.set_pad(3, PAD_PUE);
    io.set_funcsel(2, PinFunction::Pio0);
    io.set_funcsel(3, PinFunction::Sio);

    let mut pio = pio_on(0, &pads, &[in_pins(4), 0x8000]);
    pio.write32(PIO_SM0_PINCTRL, 2 << PINCTRL_IN_BASE);
    // 单步路径由宿主显式刷新采样，`SharedPio::tick` 会自动完成。
    // The host refreshes the sample explicitly on the single-step path;
    // `SharedPio::tick` does it automatically.
    pio.sample_inputs();
    assert!(pio.step_sm(0));
    assert_eq!(pio.sampled_inputs(), 1 << 2, "only GPIO2 is routed");
    assert!(pio.step_sm(0));
    assert_eq!(
        pio.read32(PIO_RXF0),
        Some(0x1000_0000),
        "ISR[31] is GPIO2, GPIO3 reads 0"
    );
}

#[test]
fn pio_blocks_only_see_their_own_pads() {
    // PIO0 看不到路由给 PIO1 的 pad，PIO1 看得到；未选中的引脚读 0。
    // PIO0 cannot see a pad routed to PIO1, PIO1 can; unrouted pins read 0.
    let pads = GpioPads::new();
    let mut io = IoBank0::with_pads(pads.clone());
    pads.set_pad(5, PAD_PUE);
    io.set_funcsel(5, PinFunction::Pio1);

    let mut pio0 = pio_on(0, &pads, &[]);
    let mut pio1 = pio_on(1, &pads, &[]);
    pio1.sample_inputs();
    pio0.sample_inputs();
    assert_eq!(pio1.sampled_inputs(), 1 << 5);
    assert_eq!(pio0.sampled_inputs(), 0, "PIO0 was not selected");
    assert_eq!(pads.sample_for(PinFunction::Pio1.to_bits(), 0), 1 << 5);
    assert_eq!(pads.sample_for(PinFunction::Pio0.to_bits(), 0), 0);
}

#[test]
fn pio_out_and_set_mappings_follow_gpiobase() {
    // GPIOBASE = 16 时 PINCTRL 的局部引脚 2/3 映射到 pad 18/19。
    // With GPIOBASE = 16, PINCTRL local pins 2/3 map to pads 18/19.
    let pads = GpioPads::new();
    let mut io = IoBank0::with_pads(pads.clone());
    let mut pio = pio_on(0, &pads, &[0x80A0, out_pins(2), out_pindirs(2)]);
    io.set_funcsel(18, PinFunction::Pio0);
    io.set_funcsel(19, PinFunction::Pio0);
    // 局部引脚 2 也选给 PIO0，用来证明驱动落在 18 而不是 2。
    // Local pin 2 is also selected for PIO0 to prove the driver lands on 18.
    io.set_funcsel(2, PinFunction::Pio0);
    // pad 18 上拉，用来证明被驱动为低时确实压过上拉。
    // Pad 18 is pulled up so a driven low proves the driver wins.
    pads.set_pad(18, PAD_PUE);

    assert!(pio.write32(PIO_GPIOBASE, 0x10));
    pio.write32(PIO_SM0_PINCTRL, 2 | (2 << PINCTRL_OUT_COUNT));
    assert!(pio.write32(PIO_TXF0, 0xFFFF_FFFF));
    // 用 `CTRL.SM_ENABLE` + `tick` 走批量路径；宿主在批次前后刷新引脚矩阵。
    // Enable through `CTRL.SM_ENABLE` and use `tick`; the host refreshes the pin
    // matrix around the batch.
    assert!(pio.write32(PIO_CTRL, 1));
    pio.sample_inputs();
    pio.tick(3);
    pio.publish_outputs();

    assert_eq!(pio.output_levels() & 0b1100, 0b1100, "local pins 2..3");
    assert_eq!(
        pads.sample_for(PinFunction::Pio0.to_bits(), 16),
        0b1100,
        "GPIOBASE shifts the window onto pads 18/19"
    );
    assert_eq!(pads.input_levels() & (1 << 19), 1 << 19, "pad 19 high");
    assert_eq!(pads.input_levels() & (1 << 18), 1 << 18, "pad 18 high");
    assert_eq!(
        pads.input_levels() & (1 << 2),
        0,
        "local pin 2 does not drive pad 2"
    );
}

#[test]
fn pio_side_set_pindir_drives_the_pad() {
    // `EXECCTRL.SIDE_PINDIR` 的 side-set 输出使能同样落到 pad。
    // Side-set with `EXECCTRL.SIDE_PINDIR` reaches the pad as well.
    let pads = GpioPads::new();
    let mut io = IoBank0::with_pads(pads.clone());
    // PINCTRL：SET_BASE = 12、SET_COUNT = 1、SIDESET_BASE = 12、SIDESET_COUNT = 1。
    // PINCTRL: SET_BASE = 12, SET_COUNT = 1, SIDESET_BASE = 12, SIDESET_COUNT = 1.
    let pinctrl = (12 << PINCTRL_SET_BASE) | (1 << PINCTRL_SET_COUNT) | (12 << 10) | (1 << 29);
    let mut pio = pio_on(0, &pads, &[set_pins(1) | (0b1_0000 << 8)]);
    pio.write32(PIO_SM0_PINCTRL, pinctrl);
    // `EXECCTRL.SIDE_PINDIR`（bit 29）让 side-set 驱动方向。
    // `EXECCTRL.SIDE_PINDIR` (bit 29) routes side-set into the direction bits.
    pio.write32(PIO_SM0_EXECCTRL, 0x2001_F000);
    io.set_funcsel(12, PinFunction::Pio0);

    assert!(pio.step_sm(0));
    pio.publish_outputs();
    assert_eq!(pio.output_enables() & (1 << 12), 1 << 12);
    assert_eq!(pads.input_levels(), 1 << 12, "pad 12 driven high");
}
