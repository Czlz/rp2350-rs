//! Concurrent execution tests for the shared bus layer
//! 共享总线层的并发执行测试
//!
//! Every assertion below holds for **any** thread interleaving, so the suite
//! is deterministic under the default parallel test runner. Barriers only
//! raise the chance of real overlap; no assertion depends on it.
//! 每个断言对**任意**线程交错都成立，因此该套件在默认并行测试下是确定的。屏障只提高真实重叠的概率，没有断言依赖它。

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;

use super::tests::{PATTERN_A, PATTERN_B, RAM_BASE, load, store};
use super::*;
use crate::memory::RamRegion;
use crate::mmio::{MmioDevice, MmioRouter, MmioWindow};

#[test]
fn two_threads_write_different_banks_without_losing_updates() {
    let ram = Arc::new(RamRegion::new("ram", RAM_BASE, 0x1000));
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = [0u32, 1]
        .map(|bank| {
            let ram = Arc::clone(&ram);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let addr = RAM_BASE + bank * 4;
                let mut last = 0u32;
                for step in 0..4000u32 {
                    last = (step << 8) | bank;
                    store(&*ram, addr, last);
                }
                barrier.wait();
                last
            })
        })
        .into_iter()
        .collect();
    let last: Vec<u32> = handles
        .into_iter()
        .map(|handle| handle.join().expect("thread"))
        .collect();
    assert_eq!(load(&*ram, RAM_BASE), last[0]);
    assert_eq!(load(&*ram, RAM_BASE + 4), last[1]);
}

#[test]
fn two_threads_write_different_banks_into_different_buses() {
    // 两个线程各写自己的 bank 与自己的区域，总线路由也必须无数据竞争。
    // Two threads write their own bank in their own region; bus routing must be
    // race-free as well.
    let bus = Arc::new({
        let mut bus = SharedBus::new();
        bus.register_region(Arc::new(RamRegion::new("low", RAM_BASE, 0x100)))
            .expect("low");
        bus.register_region(Arc::new(RamRegion::new("high", RAM_BASE + 0x1000, 0x100)))
            .expect("high");
        bus
    });
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = [0u32, 0x1000]
        .map(|offset| {
            let bus = Arc::clone(&bus);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let mut cycles = 0;
                for step in 0..4000u32 {
                    bus.write_u32(RAM_BASE + offset + 8, step, &mut cycles)
                        .expect("write");
                }
                barrier.wait();
                bus.read_u32(RAM_BASE + offset + 8, &mut cycles)
                    .expect("read")
            })
        })
        .into_iter()
        .collect();
    let values: Vec<u32> = handles
        .into_iter()
        .map(|handle| handle.join().expect("thread"))
        .collect();
    assert_eq!(values, vec![3999, 3999]);
}

#[test]
fn same_bank_writes_are_serialised_and_never_tear() {
    let ram = Arc::new(RamRegion::new("ram", RAM_BASE, 0x40));
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = [PATTERN_A, PATTERN_B]
        .map(|pattern| {
            let ram = Arc::clone(&ram);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                for _ in 0..1500 {
                    // 屏障让两线程交替写入同一个 bank 的同一个字。
                    // The barrier alternates both threads on one word in one bank.
                    barrier.wait();
                    store(&*ram, RAM_BASE, pattern);
                    barrier.wait();
                    let observed = load(&*ram, RAM_BASE);
                    assert!(
                        observed == PATTERN_A || observed == PATTERN_B,
                        "a word must never be torn: {observed:#010X}"
                    );
                }
            })
        })
        .into_iter()
        .collect();
    for handle in handles {
        handle.join().expect("thread");
    }
    let final_value = load(&*ram, RAM_BASE);
    assert!(final_value == PATTERN_A || final_value == PATTERN_B);
}

#[test]
fn cross_bank_word_access_is_atomic_within_one_access() {
    // 非对齐字跨越两个 bank：一次访问同时持有两把 bank 锁，因此单次读永不撕裂。
    // An unaligned word crosses two banks; one access holds both bank locks, so
    // a single read is never torn.
    let ram = Arc::new(RamRegion::new("ram", RAM_BASE, 0x40));
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let writer = {
        let ram = Arc::clone(&ram);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            for step in 0..3000u32 {
                let pattern = if step % 2 == 0 { PATTERN_A } else { PATTERN_B };
                store(&*ram, RAM_BASE + 2, pattern);
                if step == 0 {
                    barrier.wait();
                }
            }
        })
    };
    barrier.wait();
    for _ in 0..3000 {
        let observed = load(&*ram, RAM_BASE + 2);
        assert!(
            observed == PATTERN_A || observed == PATTERN_B,
            "one access must not tear across banks: {observed:#010X}"
        );
    }
    writer.join().expect("writer");
}

#[test]
fn cross_bank_halves_can_be_observed_separately() {
    // 两次独立访问之间另一线程可以写入，因此高半字与低半字可能来自不同图案：
    // 这是与真实总线一致的非原子语义（每个字是一次独立传输）。
    // Another thread may write between two separate accesses, so the two
    // halfwords can come from different patterns: this is the documented
    // non-atomic behaviour of separate transfers on a real bus.
    let ram = Arc::new(RamRegion::new("ram", RAM_BASE, 0x40));
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let writer = {
        let ram = Arc::clone(&ram);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            for step in 0..3000u32 {
                let pattern = if step % 2 == 0 { PATTERN_A } else { PATTERN_B };
                store(&*ram, RAM_BASE + 2, pattern);
                if step == 0 {
                    barrier.wait();
                }
            }
        })
    };
    barrier.wait();
    let mut mixes = 0u32;
    for _ in 0..3000 {
        let mut cycles = 0;
        // 低半字在 bank 0，高半字在 bank 1，两次独立访问。
        // The low halfword lives in bank 0 and the high halfword in bank 1.
        let low = SharedRegion::read_u16(&*ram, RAM_BASE + 2, &mut cycles).expect("low");
        let high = SharedRegion::read_u16(&*ram, RAM_BASE + 4, &mut cycles).expect("high");
        assert!(
            low == 0xAAAA || low == 0x5555,
            "low half must be one pattern: {low:#06X}"
        );
        assert!(
            high == 0xAAAA || high == 0x5555,
            "high half must be one pattern: {high:#06X}"
        );
        if low != high {
            mixes += 1;
        }
    }
    writer.join().expect("writer");
    // 混合并非必需（宿主可能从不交错），但绝不允许出现无效半字。
    // A mix is not required (the host may never interleave), but an invalid
    // halfword is never allowed.
    assert!(mixes <= 3000);
}

#[test]
fn flash_reads_stay_consistent_while_the_host_writes() {
    let flash = Arc::new(crate::flash::FlashRegion::new(
        "flash",
        crate::flash::FLASH_XIP_BASE,
        0x100,
    ));
    let handle = flash.handle();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let start_generation = crate::flash::content_generation();
    let writer = {
        let handle = handle.clone();
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            for step in 0..2000u32 {
                if step == 0 {
                    barrier.wait();
                }
                handle
                    .erase(crate::flash::FLASH_XIP_BASE, 8)
                    .expect("erase");
                handle
                    .write(crate::flash::FLASH_XIP_BASE, &[0x5A; 8])
                    .expect("write");
            }
        })
    };
    barrier.wait();
    for _ in 0..2000 {
        let word = load(&*flash, crate::flash::FLASH_XIP_BASE);
        assert!(
            word == 0 || word == u32::MAX || word == 0x5A5A_5A5A,
            "flash read must observe a whole word: {word:#010X}"
        );
    }
    writer.join().expect("writer");
    assert!(
        crate::flash::content_generation() != start_generation,
        "host writes must publish a new generation"
    );
}

#[test]
fn flash_bulk_snapshot_is_consistent_per_word() {
    let flash = Arc::new(crate::flash::FlashRegion::new(
        "flash",
        crate::flash::FLASH_XIP_BASE,
        0x40,
    ));
    let handle = flash.handle();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let writer = {
        let handle = handle.clone();
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            for step in 0..2000u32 {
                let byte = if step % 2 == 0 { 0x11 } else { 0x22 };
                handle
                    .write(crate::flash::FLASH_XIP_BASE, &[byte; 16])
                    .expect("write");
                if step == 0 {
                    barrier.wait();
                }
            }
        })
    };
    barrier.wait();
    for _ in 0..2000 {
        let snapshot = flash
            .slice(crate::flash::FLASH_XIP_BASE, 16)
            .expect("snapshot");
        // 快照按字拼装，因此每个 4 字节组必须是同一个图案的完整一份。
        // The snapshot is assembled word by word, so every 4-byte group must be
        // one complete pattern.
        for chunk in snapshot.chunks_exact(4) {
            let word = u32::from_le_bytes(chunk.try_into().expect("four bytes"));
            assert!(
                word == 0x1111_1111 || word == 0x2222_2222,
                "snapshot group must not mix generations: {word:#010X}"
            );
        }
    }
    writer.join().expect("writer");
}

/// Single-register device used by the MMIO concurrency tests
/// MMIO 并发测试使用的单寄存器设备
struct Counter {
    window: MmioWindow,
    value: AtomicU32,
}

impl Counter {
    fn new(base: Address) -> Self {
        Self {
            window: MmioWindow::new(base, 0x1000),
            value: AtomicU32::new(0),
        }
    }

    fn value(&self) -> u32 {
        self.value.load(Ordering::Relaxed)
    }
}

impl MmioDevice for Counter {
    fn name(&self) -> &'static str {
        "counter"
    }

    fn window(&self) -> MmioWindow {
        self.window
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        (offset == 0).then(|| self.value())
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        if offset == 0 {
            self.value.store(value, Ordering::Relaxed);
            return true;
        }
        false
    }
}

/// Router with two independent single-register devices / 两个独立单寄存器设备的路由器
fn two_device_router() -> Arc<MmioRouter> {
    let mut router = MmioRouter::new("apb", MmioWindow::new(0x4000_0000, 0x10_0000));
    router
        .mount(Box::new(Counter::new(0x4000_0000)))
        .expect("A");
    router
        .mount(Box::new(Counter::new(0x4001_0000)))
        .expect("B");
    Arc::new(router)
}

#[test]
fn two_mmio_devices_run_concurrently() {
    let router = two_device_router();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = [(0x4000_0000u32, 0x1111u32), (0x4001_0000, 0x2222)]
        .map(|(base, tag)| {
            let router = Arc::clone(&router);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let mut cycles = 0;
                for step in 0..3000u32 {
                    router
                        .write_u32(base, tag + step, &mut cycles)
                        .expect("write");
                }
                barrier.wait();
                router.read_u32(base, &mut cycles).expect("read")
            })
        })
        .into_iter()
        .collect();
    let values: Vec<u32> = handles
        .into_iter()
        .map(|handle| handle.join().expect("thread"))
        .collect();
    assert_eq!(values, vec![0x1111 + 2999, 0x2222 + 2999]);
}

#[test]
fn one_mmio_device_serialises_concurrent_alias_writes() {
    // SET 别名是「读-改-写」，同一设备锁内完成才不会丢失另一线程的置位。
    // A SET alias is a read-modify-write; it only avoids losing the other
    // thread's bits when both halves run inside one device lock.
    let router = two_device_router();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = [0x0000_FFFFu32, 0xFFFF_0000]
        .map(|bits| {
            let router = Arc::clone(&router);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let mut cycles = 0;
                barrier.wait();
                for _ in 0..2000 {
                    router
                        .write_u32(0x4000_2000, bits, &mut cycles)
                        .expect("set alias");
                }
            })
        })
        .into_iter()
        .collect();
    for handle in handles {
        handle.join().expect("thread");
    }
    let mut cycles = 0;
    assert_eq!(
        router.read_u32(0x4000_0000, &mut cycles).expect("read"),
        0xFFFF_FFFF,
        "both bit groups must survive the concurrent alias writes"
    );
}

#[test]
fn ppb_epoch_counts_every_write_across_threads() {
    let bus = Arc::new({
        let mut bus = SharedBus::new();
        bus.register_region(Arc::new(RamRegion::new("ppb", 0xE000_0000, 0x100)))
            .expect("ppb");
        bus
    });
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = [0u32, 0x40]
        .map(|offset| {
            let bus = Arc::clone(&bus);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let mut cycles = 0;
                barrier.wait();
                for step in 0..1000u32 {
                    bus.write_u8(0xE000_0000 + offset + (step & 0x3F), 1, &mut cycles)
                        .expect("write");
                }
            })
        })
        .into_iter()
        .collect();
    for handle in handles {
        handle.join().expect("thread");
    }
    assert_eq!(
        SharedAddressBus::ppb_write_epoch(&*bus),
        2000,
        "every PPB write must be counted exactly once"
    );
    // 非 PPB 写不推进世代。A non-PPB write must not advance the epoch.
    let mut cycles = 0;
    let mut other = SharedBus::new();
    other
        .register_region(Arc::new(RamRegion::new("ram", RAM_BASE, 0x100)))
        .expect("ram");
    other.write_u32(RAM_BASE, 1, &mut cycles).expect("write");
    assert_eq!(other.ppb_write_epoch(), 0);
}
