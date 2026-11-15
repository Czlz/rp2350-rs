//! PSRAM window (XIP CS1) tests / PSRAM 窗口（XIP CS1）测试
//!
//! Covers the byte/word access semantics, cycle accounting, size clamping,
//! write protection and bus integration at `0x1100_0000`, plus coexistence with
//! the flash XIP window and the boot peripheral set.
//! 覆盖字节/字访问语义、周期记账、大小钳制、写保护与 `0x1100_0000` 上的总线集成，
//! 以及与 Flash XIP 窗口、启动外设集合的共存。

use rp2350_core::bus::{AddressBus, MemoryRegion};
use rp2350_core::error::{BusErrorKind, CoreError};
use rp2350_core::flash::{FLASH_XIP_BASE, FlashRegion};
use rp2350_core::memory::SimpleBus;

use crate::boot::BootPeripherals;
use crate::map;
use crate::psram::{DEFAULT_PSRAM_SIZE, PSRAM_BASE, PSRAM_MAX_SIZE, Psram, mount_psram};

/// Assert that `result` failed with the expected bus error kind
/// 断言 `result` 以指定的总线错误类型失败
fn assert_bus_kind<T: std::fmt::Debug>(result: Result<T, CoreError>, expected: BusErrorKind) {
    match result {
        Err(CoreError::BusError { kind, .. }) => assert_eq!(kind, expected),
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn byte_round_trip_counts_one_cycle_each_way() {
    let mut psram = Psram::new(4096);
    let mut cycles = 0u64;
    assert_eq!(psram.name(), "psram");
    assert_eq!(psram.base(), PSRAM_BASE);

    psram.write_u8(PSRAM_BASE + 7, 0xA5, &mut cycles).unwrap();
    assert_eq!(cycles, 1, "one byte write costs one cycle");
    assert_eq!(psram.read_u8(PSRAM_BASE + 7, &mut cycles).unwrap(), 0xA5);
    assert_eq!(cycles, 2, "one byte read costs one cycle");
}

#[test]
fn halfword_and_word_round_trip() {
    let mut psram = Psram::new(4096);
    let mut cycles = 0u64;

    psram
        .write_u16(PSRAM_BASE + 0x10, 0xBEEF, &mut cycles)
        .unwrap();
    let after_write = cycles;
    assert_eq!(
        psram.read_u16(PSRAM_BASE + 0x10, &mut cycles).unwrap(),
        0xBEEF
    );
    assert_eq!(cycles - after_write, 2, "halfword costs two byte cycles");

    psram
        .write_u32(PSRAM_BASE + 0x20, 0xDEAD_BEEF, &mut cycles)
        .unwrap();
    let after_word_write = cycles;
    assert_eq!(
        psram.read_u32(PSRAM_BASE + 0x20, &mut cycles).unwrap(),
        0xDEAD_BEEF
    );
    assert_eq!(cycles - after_word_write, 4, "word costs four byte cycles");

    // 小端字节序：低字节位于低地址。
    // Little endian: the low byte sits at the low address.
    assert_eq!(psram.read_u8(PSRAM_BASE + 0x20, &mut cycles).unwrap(), 0xEF);
}

#[test]
fn out_of_range_accesses_report_device_error() {
    let mut psram = Psram::new(4096);
    let mut cycles = 0u64;
    let end = PSRAM_BASE + 4096;

    assert_bus_kind(psram.read_u8(end, &mut cycles), BusErrorKind::DeviceError);
    assert_bus_kind(
        psram.write_u8(end, 1, &mut cycles),
        BusErrorKind::DeviceError,
    );
    assert_bus_kind(
        psram.read_u8(PSRAM_BASE - 1, &mut cycles),
        BusErrorKind::DeviceError,
    );
    // 最后一个字越界：前半可用、后半必须报错。
    // A word straddling the end must fail even though its first half is mapped.
    assert_bus_kind(
        psram.read_u32(end - 2, &mut cycles),
        BusErrorKind::DeviceError,
    );
}

#[test]
fn len_and_is_empty_report_the_window() {
    let psram = Psram::new(8192);
    assert_eq!(psram.len(), 8192);
    assert_eq!(psram.size(), 8192);
    assert!(!psram.is_empty());
    assert!(psram.is_writable());
}

#[test]
fn read_only_window_denies_writes() {
    let mut psram = Psram::read_only(4096);
    let mut cycles = 0u64;
    assert!(!psram.is_writable());
    assert_bus_kind(
        psram.write_u8(PSRAM_BASE, 0x5A, &mut cycles),
        BusErrorKind::AccessDenied,
    );
    // 只读窗口仍可读取（初始为 0）。
    // The read-only window stays readable (initially zero).
    assert_eq!(psram.read_u8(PSRAM_BASE, &mut cycles).unwrap(), 0);
}

#[test]
fn invalid_sizes_clamp_to_the_maximum_window() {
    // 0、未对齐、超过上限都钳制到 16 MB 上限。
    // Zero, unaligned and oversized requests clamp to the 16 MB ceiling.
    for requested in [0usize, 4095, 4097, PSRAM_MAX_SIZE + 4096] {
        let psram = Psram::new(requested);
        assert_eq!(
            psram.len(),
            PSRAM_MAX_SIZE,
            "requested {requested:#X} must clamp"
        );
    }
    // 合法大小保持原样。 / A valid size is kept as requested.
    assert_eq!(Psram::new(4096).len(), 4096);
}

#[test]
fn new_window_is_zero_filled() {
    let mut psram = Psram::new(4096);
    let mut cycles = 0u64;
    for offset in (0..4096).step_by(4) {
        assert_eq!(
            psram.read_u32(PSRAM_BASE + offset, &mut cycles).unwrap(),
            0,
            "offset {offset:#X} must reset to zero"
        );
    }
}

#[test]
fn zeroed_defaults_to_eight_megabytes() {
    let mut psram = Psram::zeroed();
    let mut cycles = 0u64;
    assert_eq!(DEFAULT_PSRAM_SIZE, 8 * 1024 * 1024);
    assert_eq!(psram.len(), DEFAULT_PSRAM_SIZE);
    assert_eq!(psram.read_u8(PSRAM_BASE, &mut cycles).unwrap(), 0);
    assert_eq!(
        psram
            .read_u8(PSRAM_BASE + DEFAULT_PSRAM_SIZE as u32 - 1, &mut cycles)
            .unwrap(),
        0
    );
}

#[test]
fn bus_routes_psram_window_at_its_base() {
    let mut bus = SimpleBus::new();
    let mut psram = mount_psram(&mut bus, 4096).expect("mount psram");
    let mut cycles = 0u64;

    bus.write_u32(PSRAM_BASE + 0x100, 0x1234_5678, &mut cycles)
        .expect("psram write");
    assert_eq!(
        bus.read_u32(PSRAM_BASE + 0x100, &mut cycles)
            .expect("psram read"),
        0x1234_5678
    );

    // 宿主句柄与总线区域共享同一份字节。
    // The host handle and the bus region share the same bytes.
    assert_eq!(
        psram.read_u8(PSRAM_BASE + 0x100, &mut cycles).unwrap(),
        0x78
    );
    psram
        .write_u8(PSRAM_BASE + 0x104, 0xC3, &mut cycles)
        .unwrap();
    assert_eq!(
        bus.read_u8(PSRAM_BASE + 0x104, &mut cycles)
            .expect("guest read"),
        0xC3
    );
}

#[test]
fn psram_and_flash_windows_are_independent() {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(FlashRegion::new("flash", FLASH_XIP_BASE, 4096)))
        .expect("mount flash");
    let _psram = mount_psram(&mut bus, 4096).expect("mount psram");
    let mut cycles = 0u64;

    bus.load_u8(FLASH_XIP_BASE, 0x5A, &mut cycles)
        .expect("flash load");
    bus.write_u32(PSRAM_BASE, 0xFFFF_FFFF, &mut cycles)
        .expect("psram write");

    assert_eq!(
        bus.read_u8(FLASH_XIP_BASE, &mut cycles)
            .expect("flash read"),
        0x5A
    );
    assert_eq!(
        bus.read_u8(FLASH_XIP_BASE + 1, &mut cycles)
            .expect("flash read"),
        0
    );
    assert_eq!(
        bus.read_u32(PSRAM_BASE, &mut cycles).expect("psram read"),
        0xFFFF_FFFF
    );
    // Flash 仍拒绝客户机写入，而 PSRAM 接受。
    // Flash still rejects guest writes while PSRAM accepts them.
    assert_bus_kind(
        bus.write_u8(FLASH_XIP_BASE, 0, &mut cycles),
        BusErrorKind::AccessDenied,
    );
}

#[test]
fn unmounted_psram_window_reports_no_device() {
    let mut bus = SimpleBus::new();
    let mut cycles = 0u64;
    assert_bus_kind(bus.read_u8(PSRAM_BASE, &mut cycles), BusErrorKind::NoDevice);
    assert_bus_kind(
        bus.write_u8(PSRAM_BASE, 1, &mut cycles),
        BusErrorKind::NoDevice,
    );
}

#[test]
fn psram_coexists_with_boot_peripherals_in_any_mount_order() {
    for psram_first in [false, true] {
        let mut bus = SimpleBus::new();
        let mut cycles = 0u64;
        if psram_first {
            mount_psram(&mut bus, 4096).expect("mount psram");
            let _peripherals = BootPeripherals::mount(&mut bus).expect("mount peripherals");
            verify_coexistence(&mut bus, &mut cycles);
        } else {
            let _peripherals = BootPeripherals::mount(&mut bus).expect("mount peripherals");
            mount_psram(&mut bus, 4096).expect("mount psram");
            verify_coexistence(&mut bus, &mut cycles);
        }
    }
}

/// Check that both the PSRAM and the APB peripherals still answer
/// 检查 PSRAM 与 APB 外设都仍能正常响应
fn verify_coexistence(bus: &mut SimpleBus, cycles: &mut u64) {
    bus.write_u32(PSRAM_BASE + 0x200, 0xCAFE_F00D, cycles)
        .expect("psram write");
    assert_eq!(
        bus.read_u32(PSRAM_BASE + 0x200, cycles)
            .expect("psram read"),
        0xCAFE_F00D
    );
    // UART0 的 `UARTFR` 是 APB 路由器的只读状态寄存器，可作为共存探针。
    // UART0's `UARTFR` is a read-only APB router status register, a suitable
    // coexistence probe.
    assert_eq!(
        bus.read_u32(map::UART0_BASE + 0x18, cycles)
            .expect("uart0 fr"),
        0x90
    );
}

#[test]
fn qmi_cs1_registers_stay_plain_storage_beside_the_psram_window() {
    let mut bus = SimpleBus::new();
    let _peripherals = BootPeripherals::mount(&mut bus).expect("mount peripherals");
    let _psram = mount_psram(&mut bus, 4096).expect("mount psram");
    let mut cycles = 0u64;

    // M1_TIMING (0x14) 与 M1_RCMD (0x1C) 只是普通存储，写入可回读。
    // M1_TIMING (0x14) and M1_RCMD (0x1C) are plain storage and read back.
    bus.write_u32(map::XIP_QMI_BASE + 0x14, 0x0004_0202, &mut cycles)
        .expect("m1 timing");
    bus.write_u32(map::XIP_QMI_BASE + 0x1C, 0xEB, &mut cycles)
        .expect("m1 rcmd");
    assert_eq!(
        bus.read_u32(map::XIP_QMI_BASE + 0x14, &mut cycles)
            .expect("m1 timing read"),
        0x0004_0202
    );
    assert_eq!(
        bus.read_u32(map::XIP_QMI_BASE + 0x1C, &mut cycles)
            .expect("m1 rcmd read"),
        0xEB
    );
    // QMI 写入不得泄漏到 PSRAM 窗口。
    // QMI writes must not leak into the PSRAM window.
    assert_eq!(
        bus.read_u32(PSRAM_BASE, &mut cycles).expect("psram read"),
        0
    );
}

#[test]
fn clone_shares_the_backing_store() {
    let mut window = Psram::new(4096);
    let mut clone = window.clone();
    let mut cycles = 0u64;

    clone.write_u8(PSRAM_BASE + 3, 0x7E, &mut cycles).unwrap();
    assert_eq!(window.read_u8(PSRAM_BASE + 3, &mut cycles).unwrap(), 0x7E);
    window.write_u8(PSRAM_BASE + 3, 0x81, &mut cycles).unwrap();
    assert_eq!(clone.read_u8(PSRAM_BASE + 3, &mut cycles).unwrap(), 0x81);
}
