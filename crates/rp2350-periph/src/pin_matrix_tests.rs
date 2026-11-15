//! Bus-level pin-matrix tests / 总线级引脚矩阵测试
//!
//! These exercise the `IO_BANK0` `FUNCSEL` crossbar through `BootPeripherals`,
//! i.e. the same path a guest firmware takes: `PADS_BANK0` pulls, `SIO`/`PIO`
//! outputs and the shared pad handle.
//! 这些用例经 `BootPeripherals` 验证 `IO_BANK0` 的 `FUNCSEL` 交叉开关，即客户机
//! 固件走的同一条路径：`PADS_BANK0` 上下拉、`SIO`/`PIO` 输出与共享 pad 句柄。

use rp2350_core::bus::AddressBus;
use rp2350_core::memory::SimpleBus;

use crate::boot::BootPeripherals;
use crate::map;

/// `PADS_BANK0` word with `IE | PUE | SCHMITT` and no pull-down
/// `PADS_BANK0` 字：`IE | PUE | SCHMITT`，不启用下拉
const PAD_PULL_UP: u32 = 0x0000_005A;

#[test]
fn bus_wired_sio_drives_a_pad_selected_by_funcsel() {
    // 总线级：写 IO_BANK0 的 FUNCSEL、PADS_BANK0 的上拉与 SIO 的 OE/OUT 后，
    // 共享 pad 与 SIO 输入寄存器都反映合成后的电平。
    // Bus level: after programming FUNCSEL, a pull and SIO OE/OUT, the shared
    // pad and the SIO input register both report the resolved level.
    let mut bus = SimpleBus::new();
    let peripherals = BootPeripherals::mount(&mut bus).expect("mount");
    let mut cycles = 0u64;
    // GPIO0_CTRL.FUNCSEL = SIO (0x05)，GPIO1_CTRL 保持 NULL。
    // GPIO0_CTRL.FUNCSEL = SIO (0x05); GPIO1_CTRL stays NULL.
    bus.write_u32(map::IO_BANK0_BASE + 0x004, 0x05, &mut cycles)
        .expect("funsel 0");
    for pin in 0..2u32 {
        bus.write_u32(
            map::PADS_BANK0_BASE + 0x04 + pin * 4,
            PAD_PULL_UP,
            &mut cycles,
        )
        .expect("pull-up");
    }
    assert_eq!(
        peripherals.pads().input_levels() & 0b11,
        0b11,
        "both pads see their pull-up"
    );
    assert_eq!(
        bus.read_u32(map::SIO_BASE + 0x004, &mut cycles)
            .expect("gpio_in"),
        0b01,
        "only the routed pin reports the pad"
    );

    // SIO 驱动 GPIO0 为低，上拉被覆盖。
    // SIO drives GPIO0 low, overriding the pull-up.
    bus.write_u32(map::SIO_BASE + 0x030, 1, &mut cycles)
        .expect("gpio_oe");
    bus.write_u32(map::SIO_BASE + 0x010, 0, &mut cycles)
        .expect("gpio_out");
    assert_eq!(peripherals.pads().input_levels() & 1, 0);
    assert_eq!(
        bus.read_u32(map::IO_BANK0_BASE, &mut cycles)
            .expect("status0")
            & (1 << 17),
        0,
        "STATUS.INFROMPAD follows the driven pad"
    );
}

#[test]
fn bus_wired_pio_output_reaches_the_pad() {
    // 总线级：把 GPIO12 选给 PIO0，用 SET PINDIRS/SET PINS 驱动后 pad 变高；
    // 把 FUNCSEL 改回 NULL 后 pad 回到上拉。
    // Bus level: route GPIO12 to PIO0, drive it with SET PINDIRS/SET PINS and
    // the pad goes high; FUNCSEL = NULL returns it to the pull-up.
    let mut bus = SimpleBus::new();
    let peripherals = BootPeripherals::mount(&mut bus).expect("mount");
    let mut cycles = 0u64;
    bus.write_u32(map::IO_BANK0_BASE + 12 * 8 + 4, 0x06, &mut cycles)
        .expect("funsel pio0");
    bus.write_u32(
        map::PADS_BANK0_BASE + 0x04 + 12 * 4,
        PAD_PULL_UP,
        &mut cycles,
    )
    .expect("pull-up on 12");

    // SM0_PINCTRL：SET_BASE = 12、SET_COUNT = 1。
    // SM0_PINCTRL: SET_BASE = 12, SET_COUNT = 1.
    bus.write_u32(map::PIO0_BASE + 0x0DC, (12 << 5) | (1 << 26), &mut cycles)
        .expect("pinctrl");
    // SET PINDIRS, 1 / SET PINS, 1 / SET PINS, 0
    bus.write_u32(map::PIO0_BASE + 0x048, 0xE000 | (4 << 5) | 1, &mut cycles)
        .expect("imem0");
    bus.write_u32(map::PIO0_BASE + 0x04C, 0xE000 | 1, &mut cycles)
        .expect("imem1");
    bus.write_u32(map::PIO0_BASE + 0x050, 0xE000, &mut cycles)
        .expect("imem2");

    let block = peripherals.pio(0).expect("PIO0");
    block.with_mut(|pio| {
        assert!(pio.step_sm(0));
        assert!(pio.step_sm(0));
    });
    assert_eq!(
        peripherals.pads().input_levels() & (1 << 12),
        1 << 12,
        "PIO drives the pad high"
    );

    // 驱动为低时压过上拉，说明 pad 确实由 PIO 驱动。
    // Driving low beats the pull-up, proving the PIO owns the pad.
    block.with_mut(|pio| {
        assert!(pio.step_sm(0));
    });
    assert_eq!(
        peripherals.pads().input_levels() & (1 << 12),
        0,
        "PIO drives the pad low"
    );

    bus.write_u32(map::IO_BANK0_BASE + 12 * 8 + 4, 0x1F, &mut cycles)
        .expect("funsel null");
    assert_eq!(
        peripherals.pads().input_levels() & (1 << 12),
        1 << 12,
        "with FUNCSEL = NULL the pad returns to its pull-up"
    );
}
