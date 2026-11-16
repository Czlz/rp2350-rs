//! Unit tests for the shared bus layer / 共享总线层的单元测试
//!
//! The threaded tests live in [`super::concurrency`]; both files assert
//! properties that hold for **any** thread interleaving.
//! 线程测试位于 [`super::concurrency`]；两个文件都断言对**任意**线程交错都成立的性质。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::*;
use crate::bus::AddressBus;
use crate::memory::{RAM_BANKS, RamRegion};

pub(super) const RAM_BASE: Address = 0x2000_0000;

/// Two-word patterns used by the tearing tests / 撕裂测试使用的双字图案
pub(super) const PATTERN_A: u32 = 0xAAAA_AAAA;
/// Second pattern / 第二个图案
pub(super) const PATTERN_B: u32 = 0x5555_5555;

/// Store one word through the shared region trait / 经共享区域 trait 存储一个字
pub(super) fn store(region: &dyn SharedRegion, addr: Address, value: u32) {
    let mut cycles = 0;
    region
        .write_u32(addr, value, &mut cycles)
        .expect("shared store must succeed");
}

/// Load one word through the shared region trait / 经共享区域 trait 读取一个字
pub(super) fn load(region: &dyn SharedRegion, addr: Address) -> u32 {
    let mut cycles = 0;
    region
        .read_u32(addr, &mut cycles)
        .expect("shared load must succeed")
}

#[test]
fn bank_mapping_follows_the_word_interleave() {
    let ram = RamRegion::new("ram", RAM_BASE, 4 * RAM_BANKS * 3);
    for word in 0..RAM_BANKS * 3 {
        let addr = RAM_BASE + (word as u32) * 4;
        assert_eq!(ram.bank_of(addr), Some(word % RAM_BANKS));
    }
    assert_eq!(ram.bank_of(RAM_BASE - 1), None);
    assert_eq!(ram.bank_of(RAM_BASE + (4 * RAM_BANKS * 3) as u32), None);
}

#[test]
fn banked_storage_round_trips_every_word() {
    // 每个字写入其地址派生的值，再按扁平布局读回：分 bank 布局必须逐字节等价。
    // Write an address-derived value into every word and read it back flat: the
    // banked layout must be byte-identical to the old flat buffer.
    let size = 4 * RAM_BANKS * 5;
    let ram = RamRegion::new("ram", RAM_BASE, size);
    for word in 0..RAM_BANKS * 5 {
        store(
            &ram,
            RAM_BASE + (word as u32) * 4,
            0x0101_0101 * word as u32,
        );
    }
    let bytes = ram.bytes();
    assert_eq!(bytes.len(), size);
    for word in 0..RAM_BANKS * 5 {
        let at = word * 4;
        assert_eq!(
            u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes")),
            0x0101_0101 * word as u32
        );
    }
}

#[test]
fn shared_and_exclusive_buses_route_identically() {
    let mut exclusive = crate::memory::SimpleBus::new();
    exclusive
        .register_region(Box::new(RamRegion::new("ram", RAM_BASE, 0x1000)))
        .expect("ram");

    let ram_shared = Arc::new(RamRegion::new("ram", RAM_BASE, 0x1000));
    let flash_shared = Arc::new(crate::flash::FlashRegion::new("flash", 0x1000_0000, 0x1000));
    let handle = flash_shared.handle();
    let mut shared = SharedBus::new();
    shared
        .register_region(Arc::clone(&ram_shared) as Arc<dyn SharedRegion>)
        .expect("ram");
    shared
        .register_region(Arc::clone(&flash_shared) as Arc<dyn SharedRegion>)
        .expect("flash");

    let mut cycles = 0;
    for (addr, value) in [(RAM_BASE, 0x1234_5678u32), (RAM_BASE + 0x404, 0xDEAD_BEEF)] {
        shared.write_u32(addr, value, &mut cycles).expect("write");
        exclusive
            .write_u32(addr, value, &mut cycles)
            .expect("write");
        assert_eq!(
            shared.read_u32(addr, &mut cycles).expect("read"),
            exclusive.read_u32(addr, &mut cycles).expect("read")
        );
    }
    // Flash 读路径返回宿主写入的内容。
    // The flash read path returns what the host programmed.
    handle
        .write(0x1000_0000, &[0x11, 0x22, 0x33, 0x44])
        .unwrap();
    assert_eq!(
        shared.read_u32(0x1000_0000, &mut cycles).expect("flash"),
        0x4433_2211
    );
}

#[test]
fn shared_bus_rejects_overlap_and_unmaps_by_name() {
    let mut bus = SharedBus::new();
    bus.register_region(Arc::new(RamRegion::new("ram", RAM_BASE, 0x100)))
        .expect("first");
    assert!(
        bus.register_region(Arc::new(RamRegion::new("clash", RAM_BASE + 4, 0x100)))
            .is_err()
    );
    assert_eq!(bus.region_count(), 1);
    bus.unregister_region("ram");
    assert_eq!(bus.region_count(), 0);
    let mut cycles = 0;
    assert!(bus.read_u8(RAM_BASE, &mut cycles).is_err());
}

/// Region that reports a stall while `blocked` is set / 置位时报告 stall 的区域
struct StallingRegion {
    blocked: AtomicBool,
    reads: AtomicU64,
}

impl SharedRegion for StallingRegion {
    fn name(&self) -> &'static str {
        "stalling"
    }

    fn base(&self) -> Address {
        0x3000_0000
    }

    fn size(&self) -> usize {
        0x10
    }

    fn read_would_stall(&self, _addr: Address) -> bool {
        self.blocked.load(Ordering::Relaxed)
    }

    fn read_u8(&self, _addr: Address, cycles: &mut u64) -> BusResult<u8> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        *cycles += 1;
        Ok(0x5A)
    }

    fn write_u8(&self, _addr: Address, _value: u8, cycles: &mut u64) -> BusResult<()> {
        *cycles += 1;
        Ok(())
    }
}

#[test]
fn shared_bus_keeps_the_stall_gate() {
    let region = Arc::new(StallingRegion {
        blocked: AtomicBool::new(true),
        reads: AtomicU64::new(0),
    });
    let mut bus = SharedBus::new();
    bus.register_region(Arc::clone(&region) as Arc<dyn SharedRegion>)
        .expect("region");
    let mut cycles = 0;
    let error = bus.read_u32(0x3000_0000, &mut cycles).expect_err("stall");
    assert!(error.is_stall());
    assert_eq!(region.reads.load(Ordering::Relaxed), 0, "region untouched");
    assert_eq!(cycles, 0, "a stalled access books no cycles");
    region.blocked.store(false, Ordering::Relaxed);
    assert_eq!(bus.read_u8(0x3000_0000, &mut cycles).expect("read"), 0x5A);
    assert_eq!(region.reads.load(Ordering::Relaxed), 1);
}

#[test]
fn locked_region_bridges_an_exclusive_region() {
    let mut bus = SharedBus::new();
    bus.register_locked(Box::new(RamRegion::new_rom("rom", 0x0000_0000, 0x20)))
        .expect("rom");
    let mut cycles = 0;
    assert_eq!(bus.read_u32(0x0000_0000, &mut cycles).expect("read"), 0);
    let error = bus.write_u8(0x0000_0000, 1, &mut cycles).expect_err("deny");
    match error {
        CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::AccessDenied),
        other => panic!("unexpected error: {other:?}"),
    }
}
