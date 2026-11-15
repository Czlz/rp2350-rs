//! SIO hardware spinlock tests / SIO 硬件自旋锁测试
//!
//! Every test drives the real `Sio` MMIO window through [`MmioDevice`] so the
//! claim/release semantics are exercised exactly as firmware sees them.
//! 每个测试都通过 [`MmioDevice`] 驱动真实的 `Sio` MMIO 窗口，使取锁/释放语义与
//! 固件所见完全一致。

use rp2350_core::mmio::MmioDevice;
use rp2350_core::types::CpuId;

use crate::irq::IrqAssert;
use crate::sio::Sio;

/// `SPINLOCK0` offset on RP2350 / RP2350 的 `SPINLOCK0` 偏移
const SPINLOCK0: u32 = 0x100;

/// `SPINLOCK_ST` offset / `SPINLOCK_ST` 偏移
const SPINLOCK_ST: u32 = 0x05C;

/// Offset of `SPINLOCKn` / `SPINLOCKn` 的偏移
fn spinlock(n: u32) -> u32 {
    SPINLOCK0 + 4 * n
}

fn sio() -> Sio {
    Sio::new(IrqAssert::new())
}

#[test]
fn first_read_claims_the_lock() {
    let mut sio = sio();
    assert_eq!(sio.read32(SPINLOCK0), Some(1));
    assert_eq!(sio.read32(SPINLOCK_ST), Some(1));
    assert_eq!(sio.spinlock_state(), 1);
}

#[test]
fn a_second_read_reports_the_lock_as_taken() {
    let mut sio = sio();
    assert_eq!(sio.read32(spinlock(3)), Some(1 << 3));
    assert_eq!(
        sio.read32(spinlock(3)),
        Some(0),
        "the holder cannot re-claim / 持有者不能重复取锁"
    );
    assert_eq!(sio.read32(SPINLOCK_ST), Some(1 << 3));
}

#[test]
fn the_other_core_view_cannot_claim_a_held_lock() {
    let mut sio = sio();
    sio.select_core(CpuId::Core0);
    assert_eq!(sio.read32(SPINLOCK0), Some(1));

    // The spinlocks live outside the per-core view, so core 1 sees them held.
    // 自旋锁不属于按核视图，因此 core1 看到它们已被占用。
    sio.select_core(CpuId::Core1);
    assert_eq!(sio.read32(SPINLOCK0), Some(0));
    assert_eq!(sio.read32(SPINLOCK_ST), Some(1));
}

#[test]
fn writing_any_value_releases_the_lock() {
    let mut sio = sio();
    assert_eq!(sio.read32(spinlock(1)), Some(1 << 1));
    assert!(sio.write32(spinlock(1), 0xDEAD_BEEF));
    assert_eq!(sio.read32(SPINLOCK_ST), Some(0));
    assert_eq!(sio.spinlock_state(), 0);
    assert_eq!(sio.read32(spinlock(1)), Some(1 << 1));
}

#[test]
fn a_zero_write_also_releases_the_lock() {
    let mut sio = sio();
    assert_eq!(sio.read32(spinlock(2)), Some(1 << 2));
    assert!(sio.write32(spinlock(2), 0));
    assert_eq!(sio.read32(SPINLOCK_ST), Some(0));
}

#[test]
fn the_other_core_view_can_release() {
    let mut sio = sio();
    sio.select_core(CpuId::Core0);
    assert_eq!(sio.read32(SPINLOCK0), Some(1));

    sio.select_core(CpuId::Core1);
    assert!(sio.write32(SPINLOCK0, 1));
    assert_eq!(sio.read32(SPINLOCK_ST), Some(0));
    // core 1 can then take it even though core 0 claimed it first.
    // 尽管 core0 先取过，core1 现在也能取得该锁。
    assert_eq!(sio.read32(SPINLOCK0), Some(1));
}

#[test]
fn locks_are_independent_of_each_other() {
    let mut sio = sio();
    assert_eq!(sio.read32(spinlock(0)), Some(1));
    assert_eq!(sio.read32(spinlock(31)), Some(1 << 31));
    assert_eq!(
        sio.read32(spinlock(16)),
        Some(1 << 16),
        "a claim elsewhere leaves lock 16 free / 其他锁的占用不影响锁 16"
    );
    assert_eq!(sio.read32(SPINLOCK_ST), Some(1 | (1 << 16) | (1 << 31)));
}

#[test]
fn status_bitmap_tracks_several_locks() {
    let mut sio = sio();
    for n in [0u32, 5, 31] {
        assert_eq!(sio.read32(spinlock(n)), Some(1 << n));
    }
    assert_eq!(sio.read32(SPINLOCK_ST), Some(0x8000_0021));
    assert_eq!(sio.spinlock_state(), 0x8000_0021);

    assert!(sio.write32(spinlock(5), 0));
    assert_eq!(sio.read32(SPINLOCK_ST), Some(0x8000_0001));
}

#[test]
fn reset_clears_every_lock() {
    let mut sio = sio();
    assert_eq!(sio.read32(spinlock(0)), Some(1));
    assert_eq!(sio.read32(spinlock(7)), Some(1 << 7));

    sio.reset();
    assert_eq!(sio.read32(SPINLOCK_ST), Some(0));
    assert_eq!(sio.spinlock_state(), 0);
    assert_eq!(sio.read32(spinlock(7)), Some(1 << 7));
}

#[test]
fn select_core_does_not_touch_the_locks() {
    let mut sio = sio();
    sio.select_core(CpuId::Core0);
    assert_eq!(sio.read32(spinlock(4)), Some(1 << 4));

    sio.select_core(CpuId::Core1);
    assert_eq!(
        sio.spinlock_state(),
        1 << 4,
        "view switches must not clear locks / 切换视图不得清锁"
    );
    sio.select_core(CpuId::Core0);
    assert_eq!(sio.spinlock_state(), 1 << 4);
}

#[test]
fn spinlock_window_is_bounded_and_aligned() {
    let mut sio = sio();
    // Unaligned and below-window offsets are not spinlocks.
    // 未对齐与低于窗口的偏移都不是自旋锁。
    assert_eq!(sio.read32(SPINLOCK0 + 1), None);
    assert!(!sio.write32(SPINLOCK0 + 3, 1));
    assert_eq!(sio.read32(0x0FC), None);
    assert!(!sio.write32(0x0FC, 1));

    // `SPINLOCK_ST` is read-only, and the doorbell window at 0x180 is untouched.
    // `SPINLOCK_ST` 只读，0x180 的门铃窗口不受影响。
    assert!(!sio.write32(SPINLOCK_ST, 0));
    assert!(sio.write32(0x180, 0b1));
    assert_eq!(sio.spinlock_state(), 0);
}
