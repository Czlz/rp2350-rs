//! Synthetic ROM flash/sys-info trap tests / 合成 ROM Flash 与系统信息陷阱测试

use super::*;
use rp2350_core::bus::AddressBus;
use rp2350_core::{FLASH_XIP_BASE, FlashRegion, RamRegion, SimpleBus};

/// Test SRAM base address / 测试用 SRAM 基地址
const SRAM_BASE: u32 = 0x2000_0000;

#[test]
fn sys_info_reports_boot_info() {
    let flash = FlashRegion::rp2350_xip();
    let handler = Rp2350TrapHandler {
        image: Arc::new(build_image()),
        flash: flash.handle(),
        boot_random: [1, 2, 3, 4],
    };
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("sram", SRAM_BASE, 0x1000)))
        .unwrap();
    let mut cycles = 0u64;

    let outcome = handler
        .invoke(
            TrapCall {
                id: TrapId(Trap::SysInfo.id()),
                args: [SRAM_BASE, 5, SYS_INFO_BOOT_INFO, 0],
            },
            &mut bus,
            &mut cycles,
        )
        .unwrap();
    assert_eq!(outcome.result, 5);
    assert_eq!(
        bus.read_u32(SRAM_BASE, &mut cycles).unwrap(),
        SYS_INFO_BOOT_INFO
    );
    assert_eq!(bus.read_u32(SRAM_BASE + 4, &mut cycles).unwrap(), BOOT_WORD);
}

#[test]
fn flash_program_and_erase_share_the_guest_store() {
    let flash = FlashRegion::rp2350_xip();
    let handle = flash.handle();
    let handler = Rp2350TrapHandler {
        image: Arc::new(build_image()),
        flash: handle.clone(),
        boot_random: [0; 4],
    };
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("sram", SRAM_BASE, 0x100)))
        .unwrap();
    let mut cycles = 0u64;
    for (index, byte) in [0xDE_u8, 0xAD, 0xBE, 0xEF].iter().enumerate() {
        bus.write_u8(SRAM_BASE + index as u32, *byte, &mut cycles)
            .unwrap();
    }

    handler
        .invoke(
            TrapCall {
                id: TrapId(Trap::RangeProgram.id()),
                args: [0, SRAM_BASE, 4, 0],
            },
            &mut bus,
            &mut cycles,
        )
        .unwrap();
    assert_eq!(
        handle.read(FLASH_XIP_BASE, 4).unwrap(),
        vec![0xDE, 0xAD, 0xBE, 0xEF]
    );

    handler
        .invoke(
            TrapCall {
                id: TrapId(Trap::RangeErase.id()),
                args: [0, 4, 4, 0xD8],
            },
            &mut bus,
            &mut cycles,
        )
        .unwrap();
    assert_eq!(handle.read(FLASH_XIP_BASE, 4).unwrap(), vec![0xFF; 4]);
}

#[test]
fn flash_api_offsets_are_relative_to_the_xip_base() {
    // pico-sdk `flash_range_erase`/`flash_range_program` take offsets, so
    // offset 0x1000 must land at FLASH_XIP_BASE + 0x1000, not 0x1000.
    // pico-sdk 的擦写 API 接受偏移，因此 0x1000 必须落在 XIP 基地址 + 0x1000。
    let flash = FlashRegion::rp2350_xip();
    let handle = flash.handle();
    let handler = Rp2350TrapHandler {
        image: Arc::new(build_image()),
        flash: handle.clone(),
        boot_random: [0; 4],
    };
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("sram", SRAM_BASE, 0x100)))
        .unwrap();
    let mut cycles = 0u64;
    bus.write_u8(SRAM_BASE, 0x5A, &mut cycles).unwrap();

    handler
        .invoke(
            TrapCall {
                id: TrapId(Trap::RangeProgram.id()),
                args: [0x1000, SRAM_BASE, 1, 0],
            },
            &mut bus,
            &mut cycles,
        )
        .unwrap();
    assert_eq!(handle.read(FLASH_XIP_BASE + 0x1000, 1).unwrap(), vec![0x5A]);

    handler
        .invoke(
            TrapCall {
                id: TrapId(Trap::RangeErase.id()),
                args: [0x1000, 1, 1, 0x20],
            },
            &mut bus,
            &mut cycles,
        )
        .unwrap();
    assert_eq!(handle.read(FLASH_XIP_BASE + 0x1000, 1).unwrap(), vec![0xFF]);
}
