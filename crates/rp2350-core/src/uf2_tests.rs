use super::*;
use crate::memory::{RamRegion, SimpleBus};

/// Build one UF2 block / 构造一个 UF2 块
fn block(family: u32, addr: u32, payload: &[u8], number: u32, total: u32, flags: u32) -> Vec<u8> {
    let mut data = vec![0u8; UF2_BLOCK_SIZE];
    data[0..4].copy_from_slice(&UF2_MAGIC_START0.to_le_bytes());
    data[4..8].copy_from_slice(&UF2_MAGIC_START1.to_le_bytes());
    data[8..12].copy_from_slice(&flags.to_le_bytes());
    data[12..16].copy_from_slice(&addr.to_le_bytes());
    data[16..20].copy_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
    data[20..24].copy_from_slice(&number.to_le_bytes());
    data[24..28].copy_from_slice(&total.to_le_bytes());
    data[28..32].copy_from_slice(&family.to_le_bytes());
    data[32..32 + payload.len()].copy_from_slice(payload);
    data[UF2_BLOCK_SIZE - 4..].copy_from_slice(&UF2_MAGIC_END.to_le_bytes());
    data
}

fn arm_image(addr: u32, payload: &[u8]) -> Vec<u8> {
    block(0xE48B_FF59, addr, payload, 0, 1, UF2_FLAG_FAMILY_ID_PRESENT)
}

#[test]
fn family_ids_decode() {
    assert_eq!(Uf2Family::from_id(0xE48B_FF59), Uf2Family::Rp2350ArmSecure);
    assert_eq!(Uf2Family::from_id(0xE48B_FF5A), Uf2Family::Rp2350RiscV);
    assert_eq!(Uf2Family::from_id(0x68ED_2B88), Uf2Family::Rp2040);
    assert_eq!(Uf2Family::Rp2350RiscV.arch(), Some(Uf2Arch::RiscV));
    assert_eq!(Uf2Family::Rp2350ArmSecure.arch(), Some(Uf2Arch::Arm));
    assert_eq!(Uf2Family::Rp2350Absolute.arch(), None);
}

#[test]
fn region_routing() {
    assert_eq!(region_for(0x1000_0000), Some(Uf2Region::Flash));
    assert_eq!(region_for(0x10FF_FFFF), Some(Uf2Region::Flash));
    assert_eq!(region_for(0x2000_0000), Some(Uf2Region::Sram));
    assert_eq!(region_for(0x2008_1FFF), Some(Uf2Region::Sram));
    assert_eq!(region_for(0x4000_0000), None);
}

#[test]
fn parses_and_loads_flash_block() {
    let image = Uf2Image::from_bytes(&arm_image(0x1000_0000, &[1, 2, 3, 4])).unwrap();
    assert_eq!(image.family(), Uf2Family::Rp2350ArmSecure);
    assert_eq!(image.block_count(), 1);
    assert!(image.matches_arch(Uf2Arch::Arm));
    assert!(!image.matches_arch(Uf2Arch::RiscV));

    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new(
        "flash",
        UF2_FLASH_BASE,
        1024 * 1024,
    )))
    .unwrap();
    let report = image.load_into(&mut bus).unwrap();
    assert_eq!(report.blocks_written, 1);
    assert_eq!(report.min_address, 0x1000_0000);
    assert_eq!(report.max_address, 0x1000_0004);
    assert!(report.wrote_flash);
    assert!(!report.wrote_sram);

    let mut cycles = 0;
    assert_eq!(bus.read_u32(0x1000_0000, &mut cycles).unwrap(), 0x0403_0201);
}

#[test]
fn loads_sram_block() {
    let data = block(
        0xE48B_FF59,
        0x2000_0000,
        &[0xAA, 0xBB],
        0,
        1,
        UF2_FLAG_FAMILY_ID_PRESENT,
    );
    let image = Uf2Image::from_bytes(&data).unwrap();
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("sram", UF2_SRAM_BASE, 1024)))
        .unwrap();
    let report = image.load_into(&mut bus).unwrap();
    assert!(report.wrote_sram);
    assert!(!report.wrote_flash);
    assert_eq!(report.max_address, 0x2000_0002);
}

#[test]
fn skips_not_main_flash_blocks() {
    let data = block(
        0xE48B_FF59,
        0x1000_0000,
        &[1],
        0,
        2,
        UF2_FLAG_FAMILY_ID_PRESENT | UF2_FLAG_NOT_MAIN_FLASH,
    );
    let mut image_bytes = data;
    image_bytes.extend(block(
        0xE48B_FF59,
        0x1000_0001,
        &[2],
        1,
        2,
        UF2_FLAG_FAMILY_ID_PRESENT,
    ));
    let image = Uf2Image::from_bytes(&image_bytes).unwrap();
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("flash", UF2_FLASH_BASE, 1024)))
        .unwrap();
    let report = image.load_into(&mut bus).unwrap();
    assert_eq!(report.blocks_total, 2);
    assert_eq!(report.blocks_written, 1);
    assert_eq!(report.blocks_skipped, 1);
    let mut cycles = 0;
    assert_eq!(bus.read_u8(0x1000_0000, &mut cycles).unwrap(), 0);
    assert_eq!(bus.read_u8(0x1000_0001, &mut cycles).unwrap(), 2);
}

#[test]
fn rejects_bad_magic() {
    let mut data = arm_image(0x1000_0000, &[1]);
    data[0] = 0;
    let err = Uf2Image::from_bytes(&data).unwrap_err();
    assert!(matches!(err, CoreError::FirmwareError(_)));
}

#[test]
fn rejects_short_file() {
    let err = Uf2Image::from_bytes(&[0u8; 100]).unwrap_err();
    assert!(matches!(err, CoreError::FirmwareError(_)));
}

#[test]
fn rejects_oversized_payload() {
    let mut data = arm_image(0x1000_0000, &[0u8; 4]);
    data[16..20].copy_from_slice(&u32::try_from(UF2_MAX_PAYLOAD + 1).unwrap().to_le_bytes());
    assert!(Uf2Image::from_bytes(&data).is_err());
}

#[test]
fn rejects_duplicate_block_numbers_within_family() {
    let mut bytes = arm_image(0x1000_0000, &[1]);
    bytes.extend(arm_image(0x1000_0100, &[2]));
    let err = Uf2Image::from_bytes(&bytes).unwrap_err();
    assert!(matches!(err, CoreError::FirmwareError(_)));
}

#[test]
fn allows_duplicate_numbers_across_families() {
    let mut bytes = block(
        0xE48B_FF57,
        0x10FF_FF00,
        &[0xEF],
        0,
        2,
        UF2_FLAG_FAMILY_ID_PRESENT,
    );
    bytes.extend(arm_image(0x1000_0000, &[1]));
    let image = Uf2Image::from_bytes(&bytes).unwrap();
    assert_eq!(image.family(), Uf2Family::Rp2350ArmSecure);
    assert_eq!(image.block_count(), 2);
}

#[test]
fn rejects_out_of_window_target() {
    let data = arm_image(0x4000_0000, &[1]);
    let image = Uf2Image::from_bytes(&data).unwrap();
    let mut bus = SimpleBus::new();
    let err = image.load_into(&mut bus).unwrap_err();
    assert!(matches!(err, CoreError::FirmwareError(_)));
}

#[test]
fn reports_unmapped_region_from_bus() {
    let data = arm_image(0x1000_0000, &[1]);
    let image = Uf2Image::from_bytes(&data).unwrap();
    let mut bus = SimpleBus::new();
    assert!(image.load_into(&mut bus).is_err());
}

/// Optional integration check against a real UF2 supplied through the
/// `RP2350_MICROPYTHON_UF2` environment variable.
/// 可选：通过 `RP2350_MICROPYTHON_UF2` 环境变量对真实 UF2 做集成校验。
#[test]
fn parses_real_uf2_when_available() {
    let Ok(path) = std::env::var("RP2350_MICROPYTHON_UF2") else {
        return;
    };
    let image = Uf2Image::from_file(std::path::Path::new(&path)).unwrap();
    assert!(image.block_count() > 1000);
    assert!(image.matches_arch(Uf2Arch::Arm));

    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new(
        "flash",
        UF2_FLASH_BASE,
        16 * 1024 * 1024,
    )))
    .unwrap();
    bus.register_region(Box::new(RamRegion::new("sram", UF2_SRAM_BASE, 520 * 1024)))
        .unwrap();
    let report = image.load_into(&mut bus).unwrap();
    println!(
        "UF2 family={} blocks={} written={} skipped={} range={:#010X}..{:#010X}",
        report.family,
        report.blocks_total,
        report.blocks_written,
        report.blocks_skipped,
        report.min_address,
        report.max_address
    );
    assert_eq!(report.family, Uf2Family::Rp2350ArmSecure);
    assert_eq!(report.blocks_total, 1292);
    assert_eq!(report.blocks_written, 1292);
    assert_eq!(report.blocks_skipped, 0);
    assert_eq!(report.min_address, 0x1000_0000);
    // The last block is an RP2350 absolute-family block at `0x10FF_FF00`.
    // 最后一块是位于 `0x10FF_FF00` 的 RP2350 绝对地址块。
    assert_eq!(report.max_address, 0x1100_0000);

    let mut cycles = 0;
    assert_eq!(bus.read_u32(0x1000_0000, &mut cycles).unwrap(), 0x2008_2000);
    assert_eq!(bus.read_u32(0x1000_0004, &mut cycles).unwrap(), 0x1000_015B);
}
