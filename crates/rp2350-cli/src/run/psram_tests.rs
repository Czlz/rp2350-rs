//! PSRAM window wiring tests / PSRAM 窗口接线测试
//!
//! These tests cover the front-end half of the XIP chip-select-1 window: the
//! size parser used by `--psram`/the profile key, and the fact that
//! [`Machine::new`] only registers `0x1100_0000` when a size was requested.
//! 本文件覆盖 XIP 片选 1 窗口的前端部分：`--psram`/profile 键使用的尺寸解析器，
//! 以及 [`Machine::new`] 只在请求了大小后才注册 `0x1100_0000` 这一行为。

// 窗口大小常量在 32 位客户机地址空间内换算，这是前端地址算术的固有特性。
// The window size constant is converted inside the 32-bit guest address space,
// inherent to front-end address arithmetic.
#![allow(clippy::cast_possible_truncation)]

use super::*;
use crate::test_support::infinite_loop_elf;
use rp2350_core::bus::MemoryRegion;
use rp2350_core::error::{BusErrorKind, CoreError};
use rp2350_periph::map::{PSRAM_BASE, PSRAM_MAX_SIZE};
use rp2350_periph::psram::Psram;

/// Flash XIP word that the fixture loads at the bottom of the CS0 window
/// 夹具装载在 CS0 窗口底部的 Flash XIP 半字
const FIXTURE_FIRST_HALFWORD: u16 = 0xE7FE;

/// Build an Arm machine with the requested PSRAM window
/// 构建带指定 PSRAM 窗口的 Arm 机器
fn machine_with_psram(size_bytes: Option<usize>) -> Machine {
    let elf = infinite_loop_elf();
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles: 10_000,
        strict_decode: false,
        psram: size_bytes.map(|size_bytes| PsramOptions { size_bytes }),
    };
    Machine::new(&elf, &options).expect("machine")
}

/// Extract the error kind from a bus failure / 从总线失败中取出错误类型
fn bus_error_kind(error: CoreError) -> BusErrorKind {
    match error {
        CoreError::BusError { kind, .. } => kind,
        other => panic!("expected a bus error, got {other:?}"),
    }
}

/// Read one byte through the machine bus, expecting a bus error
/// 通过机器总线读取一个字节，并期望总线错误
fn expect_byte_error(machine: &mut Machine, addr: u32) -> BusErrorKind {
    let mut cycles = 0u64;
    bus_error_kind(
        machine
            .bus
            .read_u8(addr, &mut cycles)
            .expect_err("access must fail"),
    )
}

/// Ask the PSRAM region itself for a byte beyond its window
/// 直接向 PSRAM 区域请求窗口之外的字节
///
/// The bus reports an unregistered address as `NoDevice`; the region's own
/// out-of-window path is the `DeviceError` documented by `rp2350-periph`, so
/// both layers are asserted separately.
/// 总线上未注册的地址报 `NoDevice`；区域自身的越界路径才是 `rp2350-periph`
/// 文档里说的 `DeviceError`，因此两层分别断言。
fn expect_region_device_error(psram: &mut Psram, addr: u32) -> BusErrorKind {
    let mut cycles = 0u64;
    bus_error_kind(psram.read_u8(addr, &mut cycles).expect_err("out-of-window"))
}

#[test]
fn parses_plain_decimal_and_hex_byte_counts() {
    assert_eq!(
        PsramOptions::parse("8388608").expect("decimal").size_bytes,
        8 * 1024 * 1024
    );
    assert_eq!(
        PsramOptions::parse("0x800000").expect("hex").size_bytes,
        8 * 1024 * 1024
    );
    assert_eq!(
        PsramOptions::parse(" 4096 ").expect("spaces").size_bytes,
        4096
    );
}

#[test]
fn parses_k_and_m_suffixes_as_powers_of_two() {
    for (spec, expected) in [
        ("1024K", 1024 * 1024),
        ("8k", 8 * 1024),
        ("16KiB", 16 * 1024),
        ("8M", 8 * 1024 * 1024),
        ("8mib", 8 * 1024 * 1024),
        ("16MB", 16 * 1024 * 1024),
    ] {
        assert_eq!(
            PsramOptions::parse(spec).expect(spec).size_bytes,
            expected,
            "spec={spec}"
        );
    }
}

#[test]
fn rejects_zero_and_negative_sizes() {
    for spec in ["0", "0K", "0M", "0x0", "-1", "-8M"] {
        assert!(
            PsramOptions::parse(spec).is_err(),
            "`{spec}` must be rejected"
        );
    }
}

#[test]
fn rejects_misspelled_and_empty_sizes() {
    for spec in ["", "  ", "8G", "8MB!", "eight", "M", "8.5M"] {
        assert!(
            PsramOptions::parse(spec).is_err(),
            "`{spec}` must be rejected"
        );
    }
}

#[test]
fn rejects_sizes_that_overflow_u64_or_usize() {
    // 18 446 744 073 709 551 615 K 远超 u64，必须被拒绝而不是回绕。
    // 18446744073709551615 K overflows u64 and must be rejected, never wrap.
    assert!(PsramOptions::parse("18446744073709551615K").is_err());
    assert!(PsramOptions::parse("99999999999999999999").is_err());
}

#[test]
fn mounted_window_serves_guest_reads_and_writes() {
    let mut machine = machine_with_psram(Some(64 * 1024));
    let mut cycles = 0u64;
    machine
        .bus
        .write_u32(PSRAM_BASE, 0xDEAD_BEEF, &mut cycles)
        .expect("word write");
    machine
        .bus
        .write_u8(PSRAM_BASE + 0x10, 0x5A, &mut cycles)
        .expect("byte write");
    assert_eq!(
        machine.bus.read_u32(PSRAM_BASE, &mut cycles).expect("read"),
        0xDEAD_BEEF
    );
    assert_eq!(
        machine
            .bus
            .read_u8(PSRAM_BASE + 0x10, &mut cycles)
            .expect("byte read"),
        0x5A
    );
    // 窗口清零启动：未写过的字节读 0。 / Zeroed window: untouched bytes read 0.
    assert_eq!(
        machine
            .bus
            .read_u8(PSRAM_BASE + 0x20, &mut cycles)
            .expect("read"),
        0
    );
}

#[test]
fn window_ends_at_the_requested_size() {
    let mut machine = machine_with_psram(Some(64 * 1024));
    let mut cycles = 0u64;
    assert!(
        machine
            .bus
            .read_u8(PSRAM_BASE + 0xFFFF, &mut cycles)
            .is_ok(),
        "last byte of the 64 KiB window must be readable"
    );
    // 总线上窗口之外没有设备；区域自身的越界访问报 DeviceError。
    // Beyond the window the bus has no device; the region itself reports
    // `DeviceError` for the same address.
    assert_eq!(
        expect_byte_error(&mut machine, PSRAM_BASE + 0x1_0000),
        BusErrorKind::NoDevice
    );
    assert_eq!(
        expect_region_device_error(&mut Psram::new(64 * 1024), PSRAM_BASE + 0x1_0000),
        BusErrorKind::DeviceError
    );
}

#[test]
fn flash_xip_window_is_unaffected_by_the_psram_mount() {
    let mut machine = machine_with_psram(Some(64 * 1024));
    let mut cycles = 0u64;
    machine
        .bus
        .write_u8(PSRAM_BASE, 0x11, &mut cycles)
        .expect("psram write");
    // CS0 Flash 仍提供固件首条指令，且没有被 PSRAM 写入影响。
    // CS0 flash still serves the fixture's first instruction word.
    assert_eq!(
        machine
            .bus
            .read_u16(FLASH_XIP_BASE, &mut cycles)
            .expect("flash fetch"),
        FIXTURE_FIRST_HALFWORD
    );
    // 非缓存别名指向同一份 Flash。 / The non-cached alias shares the same flash.
    assert_eq!(
        machine
            .bus
            .read_u16(FLASH_XIP_NOCACHE_BASE, &mut cycles)
            .expect("flash alias fetch"),
        FIXTURE_FIRST_HALFWORD
    );
}

#[test]
fn without_a_request_the_window_stays_no_device() {
    // 回归：不给 `--psram` 时 `0x1100_0000` 必须与引入 PSRAM 选项前完全一致。
    // Regression: without `--psram` the window must behave exactly as before.
    let mut machine = machine_with_psram(None);
    assert_eq!(
        expect_byte_error(&mut machine, PSRAM_BASE),
        BusErrorKind::NoDevice
    );
    let mut cycles = 0u64;
    assert_eq!(
        bus_error_kind(
            machine
                .bus
                .write_u8(PSRAM_BASE, 1, &mut cycles)
                .expect_err("unmapped write")
        ),
        BusErrorKind::NoDevice
    );
    // 默认选项同样不挂载。 / The default options do not mount it either.
    assert!(MachineOptions::default().psram.is_none());
}

#[test]
fn oversized_requests_clamp_to_the_architectural_maximum() {
    // 32 MiB 超过 16 MiB 架构上限：钳制并只警告一次，绝不 panic 或截断到 0。
    // 32 MiB is above the 16 MiB ceiling: clamp with a one-shot warning, never
    // panic and never truncate the window to zero.
    let mut machine = machine_with_psram(Some(32 * 1024 * 1024));
    let mut cycles = 0u64;
    assert!(
        machine
            .bus
            .read_u8(PSRAM_BASE + PSRAM_MAX_SIZE as u32 - 1, &mut cycles)
            .is_ok(),
        "the clamped window must reach the architectural limit"
    );
    assert_eq!(
        expect_byte_error(&mut machine, PSRAM_BASE + PSRAM_MAX_SIZE as u32),
        BusErrorKind::NoDevice
    );
    assert_eq!(
        expect_region_device_error(
            &mut Psram::new(32 * 1024 * 1024),
            PSRAM_BASE + PSRAM_MAX_SIZE as u32
        ),
        BusErrorKind::DeviceError
    );
}

#[test]
fn unaligned_requests_clamp_instead_of_failing_the_boot() {
    // 1000 字节未按 4 KiB 对齐，`Psram` 把它钳制到上限而不是让启动失败。
    // 1000 bytes is not 4 KiB aligned; `Psram` clamps it instead of failing boot.
    let mut machine = machine_with_psram(Some(1000));
    let mut cycles = 0u64;
    assert!(
        machine
            .bus
            .read_u8(PSRAM_BASE + PSRAM_MAX_SIZE as u32 - 1, &mut cycles)
            .is_ok()
    );
}
