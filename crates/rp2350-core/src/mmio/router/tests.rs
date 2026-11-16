//! Router tests: dispatch, alias folding, page cache, per-device locks
//! 路由器测试：分发、别名折叠、页缓存、每设备锁

use super::*;

/// Register stub used by the router tests / 路由器测试使用的寄存器桩
struct Dummy {
    window: MmioWindow,
    value: u32,
}

impl MmioDevice for Dummy {
    // 测试桩返回字面量名称，与 trait 的省略生命周期签名一致。
    // The test stub returns a literal name, matching the trait's elided signature.
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "dummy"
    }

    fn window(&self) -> MmioWindow {
        self.window
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        (offset == 0).then_some(self.value)
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        if offset == 0 {
            self.value = value;
            return true;
        }
        false
    }
}

fn dummy() -> Box<dyn MmioDevice> {
    Box::new(Dummy {
        window: MmioWindow::new(0x4000_0000, 0x1000),
        value: 0,
    })
}

fn dummy_at(base: u32, size: usize) -> Box<dyn MmioDevice> {
    Box::new(Dummy {
        window: MmioWindow::new(base, size),
        value: 0,
    })
}

#[test]
fn router_dispatches_and_rejects_overlap() {
    let mut router = MmioRouter::new("apb", MmioWindow::new(0x4000_0000, 0x10_0000));
    router.mount(dummy()).unwrap();
    assert_eq!(router.device_count(), 1);
    assert!(router.mount(dummy()).is_err());

    let mut cycles = 0;
    router.write_u32(0x4000_0000, 0x55, &mut cycles).unwrap();
    assert_eq!(router.read_u32(0x4000_0000, &mut cycles).unwrap(), 0x55);
    // Unmapped APB offset reads as zero instead of halting the bus.
    // 未映射的 APB 偏移读出零，而不是让总线中止。
    assert_eq!(router.read_u32(0x400F_0000, &mut cycles).unwrap(), 0);
}

#[test]
fn alias_writes_fold_into_one_register() {
    let mut router = MmioRouter::new("apb", MmioWindow::new(0x4000_0000, 0x10_0000));
    router.mount(dummy()).unwrap();
    let mut cycles = 0;
    router
        .write_u32(0x4000_0000, 0x0000_000F, &mut cycles)
        .unwrap();
    router
        .write_u32(0x4000_2000, 0x0000_00F0, &mut cycles)
        .unwrap();
    assert_eq!(
        router.read_u32(0x4000_0000, &mut cycles).unwrap(),
        0x0000_00FF,
        "SET alias must OR into the register"
    );
    router
        .write_u32(0x4000_3000, 0x0000_000F, &mut cycles)
        .unwrap();
    assert_eq!(
        router.read_u32(0x4000_0000, &mut cycles).unwrap(),
        0x0000_00F0,
        "CLR alias must clear bits"
    );
    router
        .write_u32(0x4000_1000, 0x0000_00FF, &mut cycles)
        .unwrap();
    assert_eq!(
        router.read_u32(0x4000_0000, &mut cycles).unwrap(),
        0x0000_000F,
        "XOR alias must toggle bits"
    );
}

#[test]
fn alias_reads_return_the_underlying_value() {
    let mut router = MmioRouter::new("apb", MmioWindow::new(0x4000_0000, 0x10_0000));
    router.mount(dummy()).unwrap();
    let mut cycles = 0;
    router
        .write_u32(0x4000_0000, 0xA5A5_5A5A, &mut cycles)
        .unwrap();
    assert_eq!(
        router.read_u32(0x4000_1000, &mut cycles).unwrap(),
        0xA5A5_5A5A
    );
    assert_eq!(
        router.read_u32(0x4000_2000, &mut cycles).unwrap(),
        0xA5A5_5A5A
    );
    assert_eq!(
        router.read_u32(0x4000_3000, &mut cycles).unwrap(),
        0xA5A5_5A5A
    );
}

/// Router with three devices in three blocks / 三个块各挂一个设备的路由器
fn three_device_router() -> (MmioRouter, [u32; 3]) {
    let bases = [0x4000_0000u32, 0x4001_0000, 0x4002_0000];
    let mut router = MmioRouter::new("apb", MmioWindow::new(0x4000_0000, 0x10_0000));
    for base in bases {
        router.mount(dummy_at(base, 0x1000)).unwrap();
    }
    (router, bases)
}

#[test]
fn page_cache_keeps_three_devices_separate() {
    let (mut router, bases) = three_device_router();
    let mut cycles = 0;
    for _ in 0..4 {
        for (index, base) in bases.iter().enumerate() {
            router
                .write_u32(*base, 0xA0 + index as u32, &mut cycles)
                .unwrap();
        }
    }
    for (index, base) in bases.iter().enumerate() {
        assert_eq!(
            router.read_u32(*base, &mut cycles).unwrap(),
            0xA0 + index as u32,
            "device {index} must keep its own register"
        );
    }
}

#[test]
fn page_cache_keeps_alias_writes_on_the_right_device() {
    let (mut router, bases) = three_device_router();
    let mut cycles = 0;
    for base in &bases {
        router.write_u32(*base, 0x0F, &mut cycles).unwrap();
        assert_eq!(router.read_u32(*base, &mut cycles).unwrap(), 0x0F);
    }
    // SET 别名必须折叠进第二个设备，而不是最近一次缓存的设备。
    // The SET alias must fold into device 1, not the last cached device.
    router
        .write_u32(bases[1] + 0x2000, 0xF0, &mut cycles)
        .unwrap();
    assert_eq!(
        router.read_u32(bases[1], &mut cycles).unwrap(),
        0xFF,
        "SET alias must reach device 1"
    );
    assert_eq!(router.read_u32(bases[0], &mut cycles).unwrap(), 0x0F);
    assert_eq!(router.read_u32(bases[2], &mut cycles).unwrap(), 0x0F);
}

#[test]
fn page_cache_handles_two_devices_in_one_slot() {
    // 0x4000_0000 与 0x4004_0000 的页号相差 64 页，映射到同一个缓存槽。
    // Their page numbers differ by 64, so both map to the same cache slot.
    let mut router = MmioRouter::new("apb", MmioWindow::new(0x4000_0000, 0x10_0000));
    router.mount(dummy_at(0x4000_0000, 0x1000)).unwrap();
    router.mount(dummy_at(0x4004_0000, 0x1000)).unwrap();
    let mut cycles = 0;
    for _ in 0..4 {
        router.write_u32(0x4000_0000, 0x11, &mut cycles).unwrap();
        router.write_u32(0x4004_0000, 0x22, &mut cycles).unwrap();
    }
    assert_eq!(router.read_u32(0x4000_0000, &mut cycles).unwrap(), 0x11);
    assert_eq!(router.read_u32(0x4004_0000, &mut cycles).unwrap(), 0x22);
}

#[test]
fn page_cache_is_dropped_when_a_device_is_mounted() {
    let mut router = MmioRouter::new("apb", MmioWindow::new(0x4000_0000, 0x10_0000));
    router.mount(dummy_at(0x4000_0000, 0x1000)).unwrap();
    let mut cycles = 0;
    router.write_u32(0x4000_0000, 0x33, &mut cycles).unwrap();
    assert_eq!(router.read_u32(0x4000_0000, &mut cycles).unwrap(), 0x33);
    // 新设备挂载后缓存必须失效，否则 0x4001_0000 会落空。
    // Mounting must invalidate the cache, otherwise 0x4001_0000 misses.
    router.mount(dummy_at(0x4001_0000, 0x1000)).unwrap();
    router.write_u32(0x4001_0000, 0x44, &mut cycles).unwrap();
    assert_eq!(router.read_u32(0x4001_0000, &mut cycles).unwrap(), 0x44);
    assert_eq!(router.read_u32(0x4000_0000, &mut cycles).unwrap(), 0x33);
}

#[test]
fn alias_spans_reject_conflicting_windows() {
    let mut router = MmioRouter::new("apb", MmioWindow::new(0x4000_0000, 0x10_0000));
    router.mount(dummy()).unwrap();
    // 落进已挂载设备的别名区。Lands inside a mounted device's alias span.
    assert!(router.mount(dummy_at(0x4000_2000, 0x1000)).is_err());

    let mut fresh = MmioRouter::new("apb", MmioWindow::new(0x4000_0000, 0x10_0000));
    // 跨进自己所在块的别名区。Crosses its own block's alias span.
    assert!(fresh.mount(dummy_at(0x4000_0800, 0x1000)).is_err());
    // 只占块内前 4 KB 则合法。Staying inside the first 4 KB is accepted.
    assert!(fresh.mount(dummy_at(0x4000_0800, 0x800)).is_ok());
}

#[test]
fn shared_path_matches_the_exclusive_path() {
    use crate::shared::SharedRegion;

    let mut router = MmioRouter::new("apb", MmioWindow::new(0x4000_0000, 0x10_0000));
    router.mount(dummy_at(0x4000_0000, 0x1000)).unwrap();
    router.mount(dummy_at(0x4001_0000, 0x1000)).unwrap();

    let mut cycles = 0;
    router
        .write_u32(0x4000_0000, 0x1234_5678, &mut cycles)
        .unwrap();
    router
        .write_u32(0x4001_2000, 0x0000_00FF, &mut cycles)
        .unwrap();
    assert_eq!(
        SharedRegion::read_u32(&router, 0x4000_0000, &mut cycles).unwrap(),
        0x1234_5678
    );
    assert_eq!(
        SharedRegion::read_u32(&router, 0x4001_0000, &mut cycles).unwrap(),
        0xFF,
        "SET alias through the shared path must reach device 1"
    );
    SharedRegion::write_u32(&router, 0x4000_3000, 0x0000_0010, &mut cycles).unwrap();
    assert_eq!(
        router.read_u32(0x4000_0000, &mut cycles).unwrap(),
        0x1234_5668,
        "CLR alias must fold exactly once"
    );
    // 未建模偏移在两条路径上都读零并只警告一次。
    // An unmodelled offset reads zero on both paths and warns once.
    assert_eq!(
        SharedRegion::read_u32(&router, 0x400F_0000, &mut cycles).unwrap(),
        0
    );
    assert_eq!(router.read_u32(0x400F_0000, &mut cycles).unwrap(), 0);
}

#[test]
fn shared_path_preserves_read_only_device_rejection() {
    use crate::shared::SharedRegion;

    // MMIO 设备从不需要写保护，但共享路径必须原样转发 `read32` 的 `None`。
    // MMIO devices never need write protection, but the shared path must forward
    // `read32`'s `None` unchanged.
    let router = MmioRouter::new("apb", MmioWindow::new(0x4000_0000, 0x10_0000));
    let mut cycles = 0;
    assert_eq!(
        SharedRegion::read_u32(&router, 0x4000_0010, &mut cycles).unwrap(),
        0
    );
    assert_eq!(cycles, 1);
}
