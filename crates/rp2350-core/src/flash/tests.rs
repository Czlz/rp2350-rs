//! Flash region tests: wide access, aliasing, host programming
//! Flash 区域测试：宽访问、别名、宿主编程

use super::*;
use crate::bus::AddressBus;
use crate::memory::SimpleBus;

#[test]
fn wide_reads_match_the_byte_path_and_book_the_same_cycles() {
    let mut flash = FlashRegion::new("flash", 0x1000_0000, 8);
    let mut cycles = 0;
    flash
        .load_bytes(0x1000_0000, &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66])
        .unwrap();
    assert_eq!(flash.read_u16(0x1000_0000, &mut cycles).unwrap(), 0x2211);
    assert_eq!(
        flash.read_u32(0x1000_0002, &mut cycles).unwrap(),
        0x6655_4433
    );
    assert_eq!(cycles, 6, "one cycle per byte, like the default path");
    // 末尾半字仍可读；越过末尾必须报 NoDevice。
    // The last halfword is readable; one byte past the end must report
    // NoDevice.
    assert_eq!(flash.read_u16(0x1000_0006, &mut cycles).unwrap(), 0);
    let err = flash.read_u16(0x1000_0007, &mut cycles).unwrap_err();
    match err {
        CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::NoDevice),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn size_and_alias_report_the_configured_length() {
    let flash = FlashRegion::new("flash", FLASH_XIP_BASE, 0x1000);
    assert_eq!(flash.size(), 0x1000);
    assert_eq!(MemoryRegion::end(&flash), FLASH_XIP_BASE + 0x1000);
    let alias = flash.alias("alias", FLASH_XIP_NOCACHE_BASE);
    assert_eq!(alias.size(), 0x1000);
    assert_eq!(MemoryRegion::end(&alias), FLASH_XIP_NOCACHE_BASE + 0x1000);
}

#[test]
fn guest_can_read_but_not_write() {
    let mut flash = FlashRegion::new("flash", 0x1000_0000, 64);
    let mut cycles = 0;
    flash
        .load_u8(0x1000_0004, 0xA5, &mut cycles)
        .expect("host load should succeed");
    assert_eq!(flash.read_u8(0x1000_0004, &mut cycles).unwrap(), 0xA5);

    // 客户机写入必须被拒绝。
    // Guest writes must be rejected.
    assert!(flash.write_u8(0x1000_0004, 0x00, &mut cycles).is_err());
}

#[test]
fn load_bytes_programs_a_range() {
    let mut flash = FlashRegion::new("flash", 0x1000_0000, 64);
    flash.load_bytes(0x1000_0010, &[1, 2, 3]).unwrap();
    assert_eq!(flash.slice(0x1000_0010, 3), Some(vec![1u8, 2, 3]));
    assert!(flash.load_bytes(0x1000_003F, &[1, 2]).is_err());
}

#[test]
fn alias_shares_flash_bytes_and_rejects_guest_writes() {
    let mut flash = FlashRegion::new("flash", FLASH_XIP_BASE, 64);
    let mut alias = flash.alias("flash-nocache", FLASH_XIP_NOCACHE_BASE);
    let mut cycles = 0;

    flash
        .load_u8(FLASH_XIP_BASE + 8, 0x5A, &mut cycles)
        .unwrap();
    assert_eq!(
        alias
            .read_u8(FLASH_XIP_NOCACHE_BASE + 8, &mut cycles)
            .unwrap(),
        0x5A
    );
    assert!(
        alias
            .write_u8(FLASH_XIP_NOCACHE_BASE + 8, 0, &mut cycles)
            .is_err()
    );
}

#[test]
fn handle_programs_every_window_and_erases_to_ff() {
    let mut flash = FlashRegion::new("flash", FLASH_XIP_BASE, 64);
    let mut alias = flash.alias("flash-nocache", FLASH_XIP_NOCACHE_BASE);
    let handle = flash.handle();
    let mut cycles = 0;

    handle.write(FLASH_XIP_BASE + 4, &[1, 2, 3]).unwrap();
    assert_eq!(
        alias
            .read_u8(FLASH_XIP_NOCACHE_BASE + 4, &mut cycles)
            .unwrap(),
        1
    );
    handle.erase(FLASH_XIP_BASE + 4, 3).unwrap();
    assert_eq!(
        flash.read_u8(FLASH_XIP_BASE + 4, &mut cycles).unwrap(),
        0xFF
    );
    assert_eq!(handle.read(FLASH_XIP_BASE + 4, 3).unwrap(), vec![0xFF; 3]);

    // 越界按 NoDevice 报错，客户机写入仍被拒绝。
    // Out-of-range reports NoDevice and guest writes stay rejected.
    assert!(handle.write(FLASH_XIP_BASE + 63, &[1, 2]).is_err());
    assert!(flash.write_u8(FLASH_XIP_BASE + 4, 0, &mut cycles).is_err());
}

#[test]
fn bus_serves_flash_through_nocache_alias() {
    let mut bus = SimpleBus::new();
    let flash = FlashRegion::new("flash", FLASH_XIP_BASE, 64);
    let alias = flash.alias("flash-nocache", FLASH_XIP_NOCACHE_BASE);
    bus.register_region(Box::new(flash)).unwrap();
    bus.register_region(Box::new(alias)).unwrap();

    let mut cycles = 0;
    bus.load_u8(FLASH_XIP_BASE, 0x42, &mut cycles).unwrap();
    assert_eq!(
        bus.read_u8(FLASH_XIP_NOCACHE_BASE, &mut cycles).unwrap(),
        0x42
    );
}

#[test]
fn bus_load_bypasses_write_protection() {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(FlashRegion::rp2350_xip()))
        .unwrap();
    let mut cycles = 0;
    bus.load_u8(FLASH_XIP_BASE, 0x42, &mut cycles).unwrap();
    assert_eq!(bus.read_u8(FLASH_XIP_BASE, &mut cycles).unwrap(), 0x42);
    assert!(bus.write_u8(FLASH_XIP_BASE, 0x00, &mut cycles).is_err());
}

#[test]
fn bulk_load_matches_the_byte_path_and_rejects_guest_writes() {
    let mut bulk = FlashRegion::new("bulk", FLASH_XIP_BASE, 64);
    let mut bytewise = FlashRegion::new("byte", FLASH_XIP_BASE, 64);
    let payload: Vec<u8> = (0..32u8).map(|value| value.wrapping_mul(3)).collect();

    let mut bulk_cycles = 0u64;
    MemoryRegion::load_bytes(&mut bulk, FLASH_XIP_BASE + 8, &payload, &mut bulk_cycles).unwrap();
    let mut byte_cycles = 0u64;
    for (offset, byte) in payload.iter().enumerate() {
        bytewise
            .load_u8(FLASH_XIP_BASE + 8 + offset as u32, *byte, &mut byte_cycles)
            .unwrap();
    }
    // 相同字节、相同周期（每字节一个周期）。
    // Same bytes, same cycle count (one cycle per byte).
    assert_eq!(
        bulk.slice(FLASH_XIP_BASE, 64),
        bytewise.slice(FLASH_XIP_BASE, 64)
    );
    assert_eq!(bulk_cycles, byte_cycles);
    assert_eq!(bulk_cycles, payload.len() as u64);

    // 客户机写入仍被拒绝，越界批量写入与逐字节路径报同样的错误。
    // Guest writes stay rejected; an out-of-range bulk write reports the
    // same error as the byte path.
    let mut cycles = 0;
    assert!(bulk.write_u8(FLASH_XIP_BASE + 8, 0, &mut cycles).is_err());
    let bulk_error =
        MemoryRegion::load_bytes(&mut bulk, FLASH_XIP_BASE + 56, &[0; 16], &mut cycles)
            .unwrap_err();
    let byte_error = bytewise
        .load_u8(FLASH_XIP_BASE + 56 + 8, 0, &mut cycles)
        .unwrap_err();
    let kind = |error: &CoreError| match error {
        CoreError::BusError { kind, .. } => *kind,
        other => panic!("unexpected error: {other:?}"),
    };
    assert_eq!(kind(&bulk_error), kind(&byte_error));
}

#[test]
fn bus_bulk_load_serves_flash_through_the_host_channel() {
    let mut bus = SimpleBus::new();
    let flash = FlashRegion::new("flash", FLASH_XIP_BASE, 64);
    let handle = flash.handle();
    bus.register_region(Box::new(flash)).unwrap();
    let payload: Vec<u8> = (0..48u8).collect();
    let mut cycles = 0u64;
    bus.load_bytes(FLASH_XIP_BASE + 4, &payload, &mut cycles)
        .unwrap();
    assert_eq!(cycles, payload.len() as u64);
    assert_eq!(handle.read(FLASH_XIP_BASE + 4, 48).unwrap(), payload);
    assert_eq!(bus.read_u8(FLASH_XIP_BASE + 4, &mut cycles).unwrap(), 0);
}
