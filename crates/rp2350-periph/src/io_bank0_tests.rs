//! IO_BANK0 `FUNCSEL`/`STATUS` tests / IO_BANK0 `FUNCSEL`/`STATUS` 测试

use rp2350_core::bus::AddressBus;
use rp2350_core::memory::SimpleBus;
use rp2350_core::mmio::MmioDevice;

use crate::boot::BootPeripherals;
use crate::gpio_pads::{GpioPads, PAD_COUNT, PAD_PUE};
use crate::io_bank0::{IoBank0, PinFunction};
use crate::map;

/// `STATUS.INFROMPAD`（bit 17）/ `STATUS.INFROMPAD` (bit 17)
const STATUS_INFROMPAD: u32 = 1 << 17;

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
