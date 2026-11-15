//! Tests for `RamRegion` and `SimpleBus` / `RamRegion` 与 `SimpleBus` 的测试
//!
//! Split out of `memory.rs` so both files stay inside the project's per-file
//! code budget; the assertions are unchanged.
//! 从 `memory.rs` 拆出，使两个文件都落在项目的单文件代码预算内；断言内容不变。

use super::*;

#[test]
fn ram_round_trip_byte() {
    let mut ram = RamRegion::new("ram", 0x2000_0000, 16);
    let mut cycles = 0;
    ram.write_u8(0x2000_0003, 0xA5, &mut cycles).unwrap();
    assert_eq!(ram.read_u8(0x2000_0003, &mut cycles).unwrap(), 0xA5);
    assert_eq!(ram.bytes()[3], 0xA5);
}

#[test]
fn ram_word_round_trip_uses_byte_accessors() {
    let mut ram = RamRegion::new("ram", 0x2000_0000, 16);
    let mut cycles = 0;
    ram.write_u32(0x2000_0004, 0xDEAD_BEEF, &mut cycles)
        .unwrap();
    assert_eq!(ram.read_u32(0x2000_0004, &mut cycles).unwrap(), 0xDEAD_BEEF);
}

#[test]
fn ram_rejects_out_of_range() {
    let mut ram = RamRegion::new("ram", 0x2000_0000, 16);
    let mut cycles = 0;
    assert!(ram.read_u8(0x2000_0010, &mut cycles).is_err());
}

#[test]
fn ram_wide_writes_land_at_the_last_full_span() {
    let mut ram = RamRegion::new("ram", 0x2000_0000, 8);
    let mut cycles = 0;
    ram.write_u16(0x2000_0006, 0xBEEF, &mut cycles).unwrap();
    ram.write_u32(0x2000_0000, 0x1122_3344, &mut cycles)
        .unwrap();
    assert_eq!(ram.bytes(), &[0x44, 0x33, 0x22, 0x11, 0, 0, 0xEF, 0xBE]);
    // 涓ゆ瀹藉啓鍚勮婊″搴︿釜鍛ㄦ湡銆? Each wide store books its full width.
    assert_eq!(cycles, 6);
    assert_eq!(ram.read_u32(0x2000_0000, &mut cycles).unwrap(), 0x1122_3344);
    assert_eq!(ram.read_u16(0x2000_0006, &mut cycles).unwrap(), 0xBEEF);
    assert_eq!(cycles, 12);
}

#[test]
fn ram_wide_write_past_the_end_matches_the_byte_path() {
    // 榛樿瀹炵幇鍏堝啓鑳藉啓涓嬬殑瀛楄妭锛屽啀鍦ㄨ秺鐣屽鎶?NoDevice锛涘揩閫熻矾寰勫繀椤讳竴鑷淬€?        // The default stores the bytes that fit, then reports NoDevice; the fast
    // path must behave identically.
    let mut ram = RamRegion::new("ram", 0x2000_0000, 4);
    let mut cycles = 0;
    let err = ram.write_u16(0x2000_0003, 0x1122, &mut cycles).unwrap_err();
    match err {
        CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::NoDevice),
        other => panic!("unexpected error: {other:?}"),
    }
    assert_eq!(ram.bytes(), &[0, 0, 0, 0x22]);
    // 涓庨€愬瓧鑺傝矾寰勪竴鑷达細涓や釜 `write_u8` 閮藉厛璁板懆鏈燂紝绗簩涓殢鍚庤秺鐣屻€?        // Matches the byte path: both `write_u8` calls book a cycle first, the
    // second then reports the out-of-range address.
    assert_eq!(cycles, 2);
}

#[test]
fn ram_wide_read_past_the_end_reports_no_device() {
    let mut ram = RamRegion::new("ram", 0x2000_0000, 4);
    let mut cycles = 0;
    let err = ram.read_u32(0x2000_0002, &mut cycles).unwrap_err();
    match err {
        CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::NoDevice),
        other => panic!("unexpected error: {other:?}"),
    }
    // 涓庨€愬瓧鑺傝矾寰勪竴鑷达細鍓嶄袱涓瓧鑺傛垚鍔燂紝绗笁娆″瓧鑺傝鍦ㄨ鍛ㄦ湡鍚庤秺鐣屻€?        // Matches the byte path: two bytes succeed, the third byte read books a
    // cycle and then reports the out-of-range address.
    assert_eq!(cycles, 3);
}

#[test]
fn rom_wide_write_is_denied_and_changes_nothing() {
    let mut rom = RamRegion::new_rom("rom", 0x0000_0000, 8);
    let mut cycles = 0;
    let err = rom
        .write_u32(0x0000_0000, 0xFFFF_FFFF, &mut cycles)
        .unwrap_err();
    match err {
        CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::AccessDenied),
        other => panic!("unexpected error: {other:?}"),
    }
    assert_eq!(rom.bytes(), &[0; 8]);
}

#[test]
fn wide_writes_keep_the_byte_wise_path_while_a_write_watch_is_armed() {
    // 鍐欑洃瑙嗗紑鍚椂鏀硅蛋閫愬瓧鑺傝矾寰勶紝瀛樺偍缁撴灉涓庡懆鏈熸暟涓嶅彉銆?        // With a write watch armed the byte-wise path runs; the stored value and
    // the cycle count stay the same.
    let mut ram = RamRegion::new("ram", 0x2000_0000, 16);
    crate::watch::set_write_watch(Some((0x2000_0000, 0x2000_0010)));
    assert!(crate::watch::write_watch_armed());
    let mut cycles = 0;
    ram.write_u32(0x2000_0004, 0x1122_3344, &mut cycles)
        .unwrap();
    ram.write_u16(0x2000_0008, 0xAABB, &mut cycles).unwrap();
    crate::watch::set_write_watch(None);
    assert!(!crate::watch::write_watch_armed());
    assert_eq!(&ram.bytes()[4..10], &[0x44, 0x33, 0x22, 0x11, 0xBB, 0xAA]);
    assert_eq!(cycles, 6);
}

#[test]
fn read_watch_records_one_hit_per_access_with_its_width() {
    let mut ram = RamRegion::new("ram", 0x2000_0000, 16);
    let mut cycles = 0;
    crate::watch::set_read_watch(Some((0x2000_0000, 0x2000_0010)));

    ram.read_u8(0x2000_0001, &mut cycles).unwrap();
    assert_eq!(crate::watch::take_read_hit(), Some((0x2000_0001, 1)));
    ram.read_u16(0x2000_0002, &mut cycles).unwrap();
    assert_eq!(crate::watch::take_read_hit(), Some((0x2000_0002, 2)));
    ram.read_u32(0x2000_0004, &mut cycles).unwrap();
    assert_eq!(crate::watch::take_read_hit(), Some((0x2000_0004, 4)));
    // 鏈彇璧板墠鏈夋柊鐨勮鍛戒腑鏃跺彧淇濈暀鏈€鏂颁竴娆°€?        // A newer hit replaces an unconsumed one.
    ram.read_u8(0x2000_0000, &mut cycles).unwrap();
    ram.read_u8(0x2000_0003, &mut cycles).unwrap();
    assert_eq!(crate::watch::take_read_hit(), Some((0x2000_0003, 1)));

    crate::watch::set_read_watch(None);
    assert_eq!(cycles, 9, "byte cycles must match the default path");
}

#[test]
fn rom_rejects_writes() {
    let mut rom = RamRegion::new_rom("rom", 0x0000_0000, 16);
    let mut cycles = 0;
    let err = rom.write_u8(0x0000_0000, 1, &mut cycles).unwrap_err();
    match err {
        CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::AccessDenied),
        other => panic!("unexpected error: {other:?}"),
    }
    assert!(!rom.is_writable());
}

#[test]
fn bus_routes_to_region() {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("ram", 0x2000_0000, 64)))
        .unwrap();
    let mut cycles = 0;
    bus.write_u32(0x2000_0000, 0x1234_5678, &mut cycles)
        .unwrap();
    assert_eq!(bus.read_u32(0x2000_0000, &mut cycles).unwrap(), 0x1234_5678);
}

#[test]
fn bus_reports_unmapped_address() {
    let mut bus = SimpleBus::new();
    let mut cycles = 0;
    let err = bus.read_u8(0x4000_0000, &mut cycles).unwrap_err();
    match err {
        CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::NoDevice),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn bus_rejects_overlapping_regions() {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("a", 0x2000_0000, 64)))
        .unwrap();
    let err = bus
        .register_region(Box::new(RamRegion::new("b", 0x2000_0010, 64)))
        .unwrap_err();
    match err {
        CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::Overlap),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn bus_unregister_by_name() {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("ram", 0x2000_0000, 64)))
        .unwrap();
    assert_eq!(bus.region_count(), 1);
    bus.unregister_region("ram");
    assert_eq!(bus.region_count(), 0);
}

#[test]
fn bus_routes_regions_that_share_one_bucket() {
    // 0x2000_0000 涓?0x20F0_0000 钀藉湪鍚屼竴涓?16 MiB 妗讹細缂撳瓨蹇呴』鍥為€€鍒版壂鎻忥紝
    // 涓ゆ璁块棶閮借钀藉埌鍚勮嚜鐨勫尯鍩熴€?        // Both live in the same 16 MiB bucket: the cache must fall back to the
    // scan and each access must still reach its own region.
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("low", 0x2000_0000, 0x100)))
        .unwrap();
    bus.register_region(Box::new(RamRegion::new("high", 0x20F0_0000, 0x100)))
        .unwrap();
    let mut cycles = 0;
    for _ in 0..4 {
        bus.write_u32(0x2000_0010, 0x1111_1111, &mut cycles)
            .unwrap();
        bus.write_u32(0x20F0_0010, 0x2222_2222, &mut cycles)
            .unwrap();
    }
    assert_eq!(bus.read_u32(0x2000_0010, &mut cycles).unwrap(), 0x1111_1111);
    assert_eq!(bus.read_u32(0x20F0_0010, &mut cycles).unwrap(), 0x2222_2222);
}

#[test]
fn bus_cache_does_not_survive_unregister() {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("ram", 0x2000_0000, 0x100)))
        .unwrap();
    let mut cycles = 0;
    bus.write_u32(0x2000_0000, 0x1234_5678, &mut cycles)
        .unwrap();
    assert_eq!(bus.read_u32(0x2000_0000, &mut cycles).unwrap(), 0x1234_5678);
    bus.unregister_region("ram");
    // 鎾ら櫎鍚庡悓涓€鍦板潃蹇呴』鎶?NoDevice锛岃€屼笉鏄懡涓檲鏃х紦瀛樸€?        // After removal the same address must report NoDevice, not a stale hit.
    let err = bus.read_u32(0x2000_0000, &mut cycles).unwrap_err();
    match err {
        CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::NoDevice),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn bus_cache_picks_up_a_newly_registered_region() {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("first", 0x2000_0000, 0x100)))
        .unwrap();
    let mut cycles = 0;
    let err = bus.read_u8(0x3000_0000, &mut cycles).unwrap_err();
    match err {
        CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::NoDevice),
        other => panic!("unexpected error: {other:?}"),
    }
    bus.register_region(Box::new(RamRegion::new("second", 0x3000_0000, 0x100)))
        .unwrap();
    bus.write_u8(0x3000_0004, 0x5A, &mut cycles).unwrap();
    assert_eq!(bus.read_u8(0x3000_0004, &mut cycles).unwrap(), 0x5A);
}

#[test]
fn bus_bulk_load_matches_the_byte_path_byte_for_byte() {
    // 批量宿主写入必须与逐字节路径写入同样的字节、记同样的周期。
    // A host bulk load must write the same bytes and book the same cycles as
    // the byte-wise path.
    let mut bulk_bus = SimpleBus::new();
    let mut byte_bus = SimpleBus::new();
    bulk_bus
        .register_region(Box::new(RamRegion::new("ram", 0x2000_0000, 64)))
        .unwrap();
    byte_bus
        .register_region(Box::new(RamRegion::new("ram", 0x2000_0000, 64)))
        .unwrap();
    let payload: Vec<u8> = (0..40u8).collect();

    let mut bulk_cycles = 0u64;
    bulk_bus
        .load_bytes(0x2000_0008, &payload, &mut bulk_cycles)
        .unwrap();
    let mut byte_cycles = 0u64;
    for (offset, byte) in payload.iter().enumerate() {
        byte_bus
            .load_u8(0x2000_0008 + offset as u32, *byte, &mut byte_cycles)
            .unwrap();
    }
    assert_eq!(bulk_cycles, byte_cycles);
    assert_eq!(bulk_cycles, payload.len() as u64);

    let mut read_cycles = 0u64;
    for offset in 0..payload.len() {
        let addr = 0x2000_0008 + offset as u32;
        assert_eq!(
            bulk_bus.read_u8(addr, &mut read_cycles).unwrap(),
            byte_bus.read_u8(addr, &mut read_cycles).unwrap()
        );
    }
    // 未覆盖的尾部保持为零。
    // The untouched tail stays zero.
    assert_eq!(bulk_bus.read_u8(0x2000_0007, &mut read_cycles).unwrap(), 0);
}

#[test]
fn bus_bulk_load_past_the_end_reports_the_same_error() {
    let mut bulk_bus = SimpleBus::new();
    let mut byte_bus = SimpleBus::new();
    bulk_bus
        .register_region(Box::new(RamRegion::new("ram", 0x2000_0000, 8)))
        .unwrap();
    byte_bus
        .register_region(Box::new(RamRegion::new("ram", 0x2000_0000, 8)))
        .unwrap();
    let mut cycles = 0u64;
    let bulk_error = bulk_bus
        .load_bytes(0x2000_0004, &[1, 2, 3, 4, 5], &mut cycles)
        .unwrap_err();
    // 逐字节参考路径：每个字节一次 `load_u8`。
    // Byte-wise reference path: one `load_u8` per byte.
    let mut byte_error = None;
    for (offset, byte) in [1u8, 2, 3, 4, 5].iter().enumerate() {
        if let Err(error) = byte_bus.load_u8(0x2000_0004 + offset as u32, *byte, &mut cycles) {
            byte_error = Some(error);
            break;
        }
    }
    let byte_error = byte_error.expect("the byte path must hit the same fault");
    let kind = |error: &CoreError| match error {
        CoreError::BusError { kind, .. } => *kind,
        other => panic!("unexpected error: {other:?}"),
    };
    assert_eq!(kind(&bulk_error), kind(&byte_error));
    // 越界前落地的字节与逐字节路径一致。
    // Bytes that fit before the fault land exactly as in the byte path.
    for addr in [0x2000_0004, 0x2000_0007] {
        assert_eq!(
            bulk_bus.read_u8(addr, &mut cycles).unwrap(),
            byte_bus.read_u8(addr, &mut cycles).unwrap()
        );
    }
}

#[test]
fn empty_bulk_load_is_a_no_op_even_when_unmapped() {
    // 默认实现空循环、不解析地址；特化实现必须保持同样的语义。
    // The default implementation loops zero times and never resolves the
    // address; the specialised path must preserve that.
    let mut bus = SimpleBus::new();
    let mut cycles = 0u64;
    bus.load_bytes(0x4000_0000, &[], &mut cycles).unwrap();
    assert_eq!(cycles, 0);
}

#[test]
fn bus_bulk_load_denies_a_read_only_region() {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new_rom("rom", 0x0000_0000, 16)))
        .unwrap();
    let mut cycles = 0u64;
    let error = bus
        .load_bytes(0x0000_0000, &[1, 2, 3], &mut cycles)
        .unwrap_err();
    match error {
        CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::AccessDenied),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn bulk_load_falls_back_to_the_byte_path_while_a_write_watch_is_armed() {
    // 安装写监视时批量写入必须逐字节回退，否则诊断会漏报每个字节。
    // With a write watch armed the bulk path must fall back per byte, or the
    // diagnostic would miss stored bytes.
    let mut bulk = RamRegion::new("ram", 0x2000_0000, 16);
    let mut byte = RamRegion::new("ram", 0x2000_0000, 16);
    crate::watch::set_write_value(Some(0x5A5A_5A5A));
    assert!(crate::watch::write_watch_armed());
    let mut bulk_cycles = 0u64;
    let mut byte_cycles = 0u64;
    bulk.load_bytes(0x2000_0004, &[0x5A; 6], &mut bulk_cycles)
        .unwrap();
    for offset in 0..6u32 {
        byte.load_u8(0x2000_0004 + offset, 0x5A, &mut byte_cycles)
            .unwrap();
    }
    crate::watch::set_write_value(None);
    assert!(!crate::watch::write_watch_armed());
    assert_eq!(bulk.bytes(), byte.bytes());
    assert_eq!(bulk_cycles, byte_cycles);
    assert_eq!(bulk_cycles, 6);
}
