//! Lock-free TIMER state snapshot / 无锁 TIMER 状态快照
//!
//! The TIMER microsecond counter is sampled by firmware in tight polling loops
//! (`TIMERAWL`/`TIMELR` reads), so it must be readable without taking the
//! timebase mutex or any of the block's own register locks. [`TimerSnapshot`]
//! is the shared view that makes that possible:
//!
//! * **The writer** is [`crate::clock_time::Timebase`]: every `clk_ref`-domain
//!   advance (executed cycles, a TICKS `CYCLES` push, or the batched advance at
//!   the simulation driver's tick boundary) republishes the exact 64-bit
//!   microsecond count with a `Release` store, **under the same mutex that
//!   guards the timebase**. A reader therefore can never see a published value
//!   that belongs to a later timebase state than the one it observes through
//!   the locked path.
//! * **The readers** are the TIMER blocks (for `TIMELR`/`TIMEHR`/`TIMERAWL`/
//!   `TIMERAWH`) and the interrupt pump. They only perform atomic loads.
//! * The alarm comparison result is published per block as well: the nearest
//!   armed deadline (`next_deadline`, used as a cheap "could anything be due?"
//!   gate), the bits latched by the most recent sample (`alarm_hits`) and the
//!   masked status `(INTR | INTF) & INTE` that the interrupt pump republishes
//!   onto the IRQ lines. These are host-side derived values: they are recomputed
//!   by the owning block at its existing sample points, so alarm timing is
//!   unchanged.
//!
//! 固件会在紧凑轮询循环里反复读 `TIMERAWL`/`TIMELR`，因此该计数器必须能在不获取
//! 时基互斥锁、也不获取块内寄存器锁的情况下读取。[`TimerSnapshot`] 就是为此提供的
//! 共享视图：
//!
//! * **写者是** [`crate::clock_time::Timebase`]：每次 `clk_ref` 域推进（执行的周期、
//!   TICKS 的 `CYCLES` 推入，或仿真驱动在节拍边界的批量推进）都会以 `Release`
//!   存储发布精确的 64 位微秒计数，**并且是在保护时基的同一把互斥锁内完成**。
//!   因此读者绝不会看到与加锁路径所观察到的时基状态不对应的发布值。
//! * **读者是** TIMER 块（用于 `TIMELR`/`TIMEHR`/`TIMERAWL`/`TIMERAWH`）与中断泵，
//!   它们只做原子读取。
//! * alarm 比较结果也按块发布：最近的已武装截止时间（`next_deadline`，用作「是否
//!   可能有到期项」的廉价门控）、最近一次采样锁存的位（`alarm_hits`），以及中断泵
//!   用来重新发布中断线的屏蔽状态 `(INTR | INTF) & INTE`。这些是宿主侧派生值，
//!   由所属块在既有采样点重算，因此 alarm 时序不变。

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Number of TIMER blocks sharing one snapshot / 共享一份快照的 TIMER 块数量
pub(crate) const TIMER_BLOCKS: usize = 2;

/// Slot index of TIMER0 / TIMER0 的槽位下标
pub(crate) const TIMER0_SLOT: usize = 0;

/// Slot index of TIMER1 / TIMER1 的槽位下标
pub(crate) const TIMER1_SLOT: usize = 1;

/// Lock-free view of the shared TIMER counter and alarm state
/// 共享 TIMER 计数器与 alarm 状态的无锁视图
///
/// All fields are plain atomics: readers never take a mutex. Writes happen
/// under the owning lock (the timebase mutex for `now_us`, the block's register
/// locks for the derived per-block state), so each stored value is a complete
/// sample.
/// 所有字段都是普通原子量：读者从不加锁。写入发生在所属锁内（`now_us` 在时基锁内，
/// 每块的派生状态在块自身的寄存器锁内），因此每个存储值都是一次完整采样。
#[derive(Debug, Default)]
pub struct TimerSnapshot {
    /// 64-bit `clk_ref`-domain microsecond counter / 64 位 `clk_ref` 域微秒计数器
    now_us: AtomicU64,
    /// Nearest armed deadline per block (`0` = nothing armed)
    /// 每块最近的已武装截止时间（`0` 表示没有武装项）
    next_deadline: [AtomicU32; TIMER_BLOCKS],
    /// Alarm bits latched by the most recent sample per block
    /// 每块最近一次采样锁存的 alarm 位
    alarm_hits: [AtomicU32; TIMER_BLOCKS],
    /// Cached `(INTR | INTF) & INTE` per block / 每块缓存的 `(INTR | INTF) & INTE`
    ints: [AtomicU32; TIMER_BLOCKS],
}

impl TimerSnapshot {
    /// Create an all-zero snapshot / 创建一个全零快照
    ///
    /// A fresh snapshot reads `now_us == 0` and reports no armed alarm, which is
    /// exactly the power-on state of the timer blocks.
    /// 新快照读出的 `now_us == 0` 且没有任何已武装 alarm，正是定时器块的上电状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Current 64-bit microsecond counter / 当前 64 位微秒计数器
    ///
    /// This is the same value the locked path returns at the same simulated
    /// instant; it is the authoritative `TIMELR`/`TIMEHR` content.
    /// 该值与加锁路径在**同一仿真时刻**返回的值相同，是 `TIMELR`/`TIMEHR` 的权威内容。
    #[must_use]
    pub fn now_us(&self) -> u64 {
        self.now_us.load(Ordering::Acquire)
    }

    /// TIMER0 alarm bits latched by the most recent sample
    /// TIMER0 最近一次采样锁存的 alarm 位
    #[must_use]
    pub fn timer0_alarm_hits(&self) -> u32 {
        self.alarm_hits(TIMER0_SLOT)
    }

    /// TIMER1 alarm bits latched by the most recent sample
    /// TIMER1 最近一次采样锁存的 alarm 位
    #[must_use]
    pub fn timer1_alarm_hits(&self) -> u32 {
        self.alarm_hits(TIMER1_SLOT)
    }

    /// Alarm bits latched by the most recent sample of `slot`
    /// `slot` 最近一次采样锁存的 alarm 位
    pub(crate) fn alarm_hits(&self, slot: usize) -> u32 {
        self.alarm_hits[slot].load(Ordering::Acquire)
    }

    /// Publish the exact microsecond count / 发布精确的微秒计数
    ///
    /// Called by [`crate::clock_time::Timebase`] while it holds the timebase
    /// mutex, so the store is ordered against every locked observer.
    /// 由 [`crate::clock_time::Timebase`] 在持有时基互斥锁时调用，因此该存储相对
    /// 所有加锁观察者都是有序的。
    pub(crate) fn publish_now(&self, now_us: u64) {
        self.now_us.store(now_us, Ordering::Release);
    }

    /// Nearest armed deadline for `slot` (`0` = nothing armed)
    /// `slot` 最近的已武装截止时间（`0` 表示没有武装项）
    pub(crate) fn next_deadline(&self, slot: usize) -> u32 {
        self.next_deadline[slot].load(Ordering::Acquire)
    }

    /// Publish the nearest armed deadline of `slot` / 发布 `slot` 最近的已武装截止时间
    pub(crate) fn publish_next_deadline(&self, slot: usize, deadline: u32) {
        self.next_deadline[slot].store(deadline, Ordering::Release);
    }

    /// Cached masked status of `slot` / `slot` 缓存的屏蔽状态
    pub(crate) fn ints(&self, slot: usize) -> u32 {
        self.ints[slot].load(Ordering::Acquire)
    }

    /// Publish the cached masked status of `slot` / 发布 `slot` 缓存的屏蔽状态
    pub(crate) fn publish_ints(&self, slot: usize, ints: u32) {
        self.ints[slot].store(ints, Ordering::Release);
    }

    /// Record the alarm bits latched by one sample / 记录一次采样锁存的 alarm 位
    pub(crate) fn publish_alarm_hits(&self, slot: usize, hits: u32) {
        self.alarm_hits[slot].store(hits, Ordering::Release);
    }

    /// Clear every latched alarm bit of `slot` (new arming state)
    /// 清除 `slot` 的全部锁存 alarm 位（新的武装状态）
    pub(crate) fn clear_alarm_hits(&self, slot: usize) {
        self.alarm_hits[slot].store(0, Ordering::Release);
    }

    /// Clear the acknowledged bits of `slot` (`INTR` write-one-to-clear)
    /// 清除 `slot` 已确认的位（`INTR` 写 1 清零）
    pub(crate) fn acknowledge_alarm_hits(&self, slot: usize, value: u32) {
        self.alarm_hits[slot].fetch_and(!value, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_snapshot_is_the_power_on_state() {
        let snapshot = TimerSnapshot::new();
        assert_eq!(snapshot.now_us(), 0);
        assert_eq!(snapshot.timer0_alarm_hits(), 0);
        assert_eq!(snapshot.timer1_alarm_hits(), 0);
        assert_eq!(snapshot.next_deadline(TIMER0_SLOT), 0);
        assert_eq!(snapshot.ints(TIMER1_SLOT), 0);
    }

    #[test]
    fn publishing_a_counter_value_is_visible_to_every_reader() {
        let snapshot = TimerSnapshot::new();
        snapshot.publish_now(0x1_0000_0002);
        snapshot.publish_now(0x1_0000_0003);
        assert_eq!(snapshot.now_us(), 0x1_0000_0003);
        assert_eq!((snapshot.now_us() >> 32) as u32, 1);
        assert_eq!(snapshot.now_us() as u32, 3);
    }

    #[test]
    fn alarm_bits_are_per_block_and_acknowledged_by_value() {
        let snapshot = TimerSnapshot::new();
        snapshot.publish_alarm_hits(TIMER0_SLOT, 0b0101);
        snapshot.publish_alarm_hits(TIMER1_SLOT, 0b1000);
        assert_eq!(snapshot.timer0_alarm_hits(), 0b0101);
        assert_eq!(snapshot.timer1_alarm_hits(), 0b1000);
        snapshot.acknowledge_alarm_hits(TIMER0_SLOT, 0b0001);
        assert_eq!(snapshot.timer0_alarm_hits(), 0b0100);
        snapshot.clear_alarm_hits(TIMER0_SLOT);
        assert_eq!(snapshot.timer0_alarm_hits(), 0);
        assert_eq!(snapshot.timer1_alarm_hits(), 0b1000, "TIMER1 untouched");
    }

    #[test]
    fn derived_state_is_kept_per_block() {
        let snapshot = TimerSnapshot::new();
        snapshot.publish_next_deadline(TIMER0_SLOT, 1_000);
        snapshot.publish_next_deadline(TIMER1_SLOT, 2_000);
        snapshot.publish_ints(TIMER0_SLOT, 0b0011);
        snapshot.publish_ints(TIMER1_SLOT, 0b1100);
        assert_eq!(snapshot.next_deadline(TIMER0_SLOT), 1_000);
        assert_eq!(snapshot.next_deadline(TIMER1_SLOT), 2_000);
        assert_eq!(snapshot.ints(TIMER0_SLOT), 0b0011);
        assert_eq!(snapshot.ints(TIMER1_SLOT), 0b1100);
    }
}
