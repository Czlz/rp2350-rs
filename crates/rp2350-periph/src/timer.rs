//! TIMER0 / TIMER1 peripheral model / TIMER0 / TIMER1 外设模型
//!
//! Both RP2350 timer blocks share one 64-bit microsecond counter driven by the
//! shared [`Timebase`], which the simulation driver advances with executed
//! cycles. The datasheet clocks this counter from `clk_ref` through the TICKS
//! divider (`TICK = clk_ref / 1 MHz`), so [`Timebase::timer_us`] keeps a
//! reference-domain integrator: runtime `clk_sys` overclocking does not change
//! the microsecond rate. Each block owns its own `ALARM0..3`, `ARMED`, `INTR`,
//! `INTE`, `INTF` and `INTS` registers and its own interrupt lines. `INTR` is
//! write-one-to-clear raw status, `INTF` is a plain read/write force register
//! (pico-sdk's alarm pool pokes it with `hw_set_bits`), and
//! `INTS = (INTR | INTF) & INTE` drives the four alarm lines into the shared
//! [`IrqAssert`]. An alarm that reaches its deadline latches its `INTR` bit and
//! clears its `ARMED` bit, exactly like the hardware; it stays latched until
//! firmware writes a one to `INTR`.
//! 两个 RP2350 定时器块共享一个由共享 [`Timebase`] 驱动的 64 位微秒计数器，
//! 仿真驱动按已执行周期推进它。数据手册中该计数器由 `clk_ref` 经 TICKS 分频
//! （`TICK = clk_ref / 1 MHz`）驱动，因此 [`Timebase::timer_us`] 保留参考域积分器：
//! 运行期 `clk_sys` 超频不会改变微秒速率。每个块拥有各自的 `ALARM0..3`、
//! `ARMED`、`INTR`、`INTE`、`INTF`、`INTS` 寄存器与中断线。`INTR` 是写 1 清零的
//! 原始状态，`INTF` 是普通读写的强制位（pico-sdk 的 alarm pool 用 `hw_set_bits`
//! 置位），`INTS = (INTR | INTF) & INTE` 驱动四条 alarm 线进入共享 [`IrqAssert`]。
//! 到期的 alarm 会锁存自己的 `INTR` 位并清掉 `ARMED` 位，与硬件一致；在固件向
//! `INTR` 写 1 之前它保持锁存。
//!
//! TIMER0 is mounted at [`map::TIMER0_BASE`] and raises `TIMER0_IRQ_0..3`
//! (`0..=3`); TIMER1 is mounted at [`map::TIMER1_BASE`] and raises
//! `TIMER1_IRQ_0..3` (`4..=7`).
//! TIMER0 挂载在 [`map::TIMER0_BASE`]，产生 `TIMER0_IRQ_0..3`（`0..=3`）；
//! TIMER1 挂载在 [`map::TIMER1_BASE`]，产生 `TIMER1_IRQ_0..3`（`4..=7`）。
//!
//! # Lock-free counter reads / 无锁计数器读取
//!
//! `TIMELR`/`TIMEHR`/`TIMERAWL`/`TIMERAWH` are served from the shared
//! [`TimerSnapshot`] with one atomic load each. The snapshot is republished by
//! [`crate::clock_time::Timebase`] at every `clk_ref`-domain advance (executed
//! cycles, TICKS `CYCLES` pushes and the driver's batched tick boundary), while
//! the timebase mutex is held, so a read returns exactly the microsecond count
//! the locked path would return at the same instant. Register writes and alarm
//! bookkeeping still take the block's own locks; the alarm sample points are
//! unchanged, and the interrupt pump uses the atomic `next_deadline` gate plus
//! the cached masked status instead of locking when no deadline is due.
//! `TIMELR`/`TIMEHR`/`TIMERAWL`/`TIMERAWH` 由共享 [`TimerSnapshot`] 各用一次原子读
//! 服务。快照由 [`crate::clock_time::Timebase`] 在每次 `clk_ref` 域推进（执行周期、
//! TICKS 的 `CYCLES` 推入与驱动器的批量节拍边界）且仍持有时基互斥锁时重新发布，
//! 因此一次读取返回的正是加锁路径在同一时刻会返回的微秒数。寄存器写入与 alarm
//! 记账仍取块自身的锁；alarm 采样点保持不变，中断泵在没有到期截止时间时使用原子
//! `next_deadline` 门控与缓存的屏蔽状态，而不是加锁。

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use parking_lot::Mutex;
use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};
use rp2350_core::types::Address;

use super::clock_time::Timebase;
use super::irq::IrqAssert;
use super::map;
use super::timer_snapshot::{TIMER0_SLOT, TIMER1_SLOT, TimerSnapshot};

/// Number of alarm channels / 闹钟通道数量
const ALARMS: usize = 4;

/// Offset of `TIMEHW` (write-only) / `TIMEHW` 偏移（只写）
pub const TIMEHW: u32 = 0x00;

/// Offset of `TIMELW` (write-only) / `TIMELW` 偏移（只写）
pub const TIMELW: u32 = 0x04;

/// Offset of `TIMEHR` / `TIMEHR` 偏移
pub const TIMEHR: u32 = 0x08;

/// Offset of `TIMELR` / `TIMELR` 偏移
pub const TIMELR: u32 = 0x0C;

/// Offset of `ALARM0` (alarms 1..3 follow at `+4`) / `ALARM0` 偏移（alarm 1..3 依次 `+4`）
pub const ALARM0_OFFSET: u32 = 0x10;

/// Offset of `ARMED` / `ARMED` 偏移
pub const ARMED: u32 = 0x20;

/// Offset of `TIMERAWH` / `TIMERAWH` 偏移
pub const TIMERAWH: u32 = 0x24;

/// Offset of `TIMERAWL` / `TIMERAWL` 偏移
pub const TIMERAWL: u32 = 0x28;

/// Offset of `DBGPAUSE` / `DBGPAUSE` 偏移
pub const DBGPAUSE: u32 = 0x30;

/// Offset of `PAUSE` / `PAUSE` 偏移
pub const PAUSE: u32 = 0x34;

/// Offset of `LOCKED` (16-bit, own word in this model) / `LOCKED` 偏移（16 位，本模型中独占一份状态）
pub const LOCKED: u32 = 0x36;

/// Offset of `SOURCE` / `SOURCE` 偏移
pub const SOURCE: u32 = 0x38;

/// Offset of `INTR` (raw status, write-one-to-clear) / `INTR` 偏移（原始状态，写 1 清零）
pub const INTR: u32 = 0x3C;

/// Offset of `INTE` (per-bit enable) / `INTE` 偏移（逐位使能）
pub const INTE: u32 = 0x40;

/// Offset of `INTF` (plain read/write force bits) / `INTF` 偏移（普通读写强制位）
pub const INTF: u32 = 0x44;

/// Offset of `INTS` (masked status) / `INTS` 偏移（屏蔽后的状态）
pub const INTS: u32 = 0x48;

/// Signed 32-bit "has the deadline been reached" test
/// 32 位有符号「截止时间是否已到」判断
///
/// The alarm registers hold the low 32 bits of the microsecond counter and the
/// hardware compares them as a wrapping signed difference, so a deadline less
/// than ~35 minutes away always resolves correctly (4294 s fit in 32 bits).
/// alarm 寄存器保存微秒计数器的低 32 位，硬件按回绕的有符号差值比较，因此截止
/// 时间在约 35 分钟内（32 位可覆盖 4294 秒）总能正确判断。
#[must_use]
fn reached(deadline: u32, now: u32) -> bool {
    now.wrapping_sub(deadline) < 0x8000_0000
}

/// Deadline that will be reached first, or `0` when nothing is armed
/// 最先到期的截止时间；没有武装项时返回 `0`
///
/// The choice maximizes the wrapping signed distance `now - deadline`, which is
/// exactly the ordering [`reached`] uses. All distances shift by the same amount
/// as time advances, so the selected deadline stays the first one to become due
/// until an arming write or a firing sample changes the set. The interrupt pump
/// therefore only needs `reached(next_deadline, now)` to know whether any slow
/// path can possibly have work to do.
/// 该选择最大化回绕有符号差值 `now - deadline`，正是 [`reached`] 使用的顺序。所有
/// 差值随时间同步平移，因此选中的截止时间在下次武装写入或采样触发之前始终保持
/// 「最先到期」。中断泵因此只需 `reached(next_deadline, now)` 就能判断慢路径是否
/// 可能有工作。
/// 「最先到期」的选择需要把回绕差值按位重解释为有符号数，这正是 [`reached`]
/// 使用的硬件语义。
/// The "first to become due" selection reinterprets the wrapping difference as a
/// signed value by bit, which is exactly the hardware semantics [`reached`] uses.
#[allow(clippy::cast_possible_wrap)]
#[must_use]
fn nearest_deadline(deadlines: &[u32; ALARMS], armed: u32, now: u32) -> u32 {
    let mut next = 0u32;
    let mut best = i32::MIN;
    for (index, &deadline) in deadlines.iter().enumerate() {
        if armed & (1 << index) == 0 || deadline == 0 {
            continue;
        }
        let distance = now.wrapping_sub(deadline) as i32;
        if distance > best {
            best = distance;
            next = deadline;
        }
    }
    next
}

/// State shared by one timer block and its clones / 一个定时器块及其克隆共享的状态
#[derive(Debug, Clone)]
struct TimerBlock {
    /// Datasheet name (`TIMER0`/`TIMER1`) / 数据手册名称（`TIMER0`/`TIMER1`）
    name: &'static str,
    /// Register window base / 寄存器窗口基地址
    base: Address,
    /// First interrupt line (`TIMERx_IRQ_0`) / 首条中断线（`TIMERx_IRQ_0`）
    irq_0: u16,
    /// Slot inside the shared snapshot (`0` = TIMER0, `1` = TIMER1)
    /// 共享快照内的槽位（`0` = TIMER0，`1` = TIMER1）
    slot: usize,
    /// Lock-free mirror of the shared microsecond counter / 共享微秒计数器的无锁镜像
    snapshot: Arc<TimerSnapshot>,
    /// Flat register file (`ALARM0..3`, `INTE`, `INTF`, `PAUSE`, ...) / 扁平寄存器文件
    regs: Arc<Mutex<RegisterFile>>,
    /// Latched raw status bits / 锁存的原始状态位
    intr: Arc<Mutex<u32>>,
    /// Latched armed bits (cleared when an alarm fires) / 锁存的武装位（alarm 到期即清）
    armed: Arc<Mutex<u32>>,
    /// `LOCKED` value, kept apart from its 16-bit neighbour `PAUSE` / `LOCKED` 值，与 16 位邻居 `PAUSE` 分开保存
    locked: Arc<Mutex<u16>>,
    /// Lines this block last republished (`INTS` mirror) / 本块上次发布的中断线（`INTS` 镜像）
    published: Arc<AtomicU32>,
    /// Number of slow-path samples (tests prove the gate skips them)
    /// 慢路径采样次数（测试用其证明门控会跳过采样）
    #[cfg(test)]
    samples: Arc<AtomicU32>,
    /// Shared interrupt lines / 共享中断线
    irq: IrqAssert,
}

impl TimerBlock {
    /// Build one block / 构建一个块
    // 公共构造函数保持「拥有 `Arc`」的签名（boot 与测试都传自己的克隆），块只取
    // 快照句柄，之后不再持有该 `Arc`。
    // The public constructors keep the owned-`Arc` signature (boot and tests pass
    // their own clones); the block only extracts the snapshot handle.
    #[allow(clippy::needless_pass_by_value)]
    fn new(
        name: &'static str,
        base: Address,
        irq_0: u16,
        slot: usize,
        timebase: Arc<Mutex<Timebase>>,
        irq: IrqAssert,
    ) -> Self {
        // 快照由时基拥有；构造时取一次句柄，此后计数读取不再碰时基锁。
        // The time base owns the snapshot; take the handle once so counter reads
        // never touch the timebase lock again.
        let snapshot = timebase.lock().snapshot();
        Self {
            name,
            base,
            irq_0,
            slot,
            snapshot,
            regs: Arc::new(Mutex::new(RegisterFile::new(map::DEVICE_WINDOW_SIZE))),
            intr: Arc::new(Mutex::new(0)),
            armed: Arc::new(Mutex::new(0)),
            locked: Arc::new(Mutex::new(0)),
            published: Arc::new(AtomicU32::new(0)),
            #[cfg(test)]
            samples: Arc::new(AtomicU32::new(0)),
            irq,
        }
    }

    /// Current 64-bit microsecond counter (lock-free)
    /// 当前 64 位微秒计数器（无锁）
    fn now_us(&self) -> u64 {
        self.snapshot.now_us()
    }

    /// Current counter as compared with the alarm registers / 与 alarm 寄存器比较用的当前计数
    fn now_low(&self) -> u32 {
        self.now_us() as u32
    }

    /// Latch every armed alarm whose deadline has passed
    /// 锁存所有已到期且仍武装的 alarm
    ///
    /// A firing alarm sets its `INTR` bit and clears its `ARMED` bit, so the
    /// alarm is delivered exactly once per arming — firmware must write a one to
    /// `INTR` to acknowledge it. The fired bits are also published to the
    /// snapshot (`alarm_hits`) so hosts can observe the latch without locks.
    /// 到期的 alarm 置位 `INTR` 并清掉 `ARMED`，因此每次武装只投递一次；固件需要
    /// 向 `INTR` 写 1 来确认。命中的位同时发布到快照（`alarm_hits`），宿主因此
    /// 可以无锁观察该锁存。
    fn sample(&self) {
        #[cfg(test)]
        self.samples.fetch_add(1, Ordering::Relaxed);
        let now = self.now_low();
        let deadlines = {
            let regs = self.regs.lock();
            let mut deadlines = [0u32; ALARMS];
            for (index, slot) in deadlines.iter_mut().enumerate() {
                *slot = regs.read(ALARM0_OFFSET + (index as u32) * 4).unwrap_or(0);
            }
            deadlines
        };
        let fired = {
            let mut armed = self.armed.lock();
            let mut fired = 0u32;
            for (index, &deadline) in deadlines.iter().enumerate() {
                let bit = 1u32 << index;
                if *armed & bit != 0 && deadline != 0 && reached(deadline, now) {
                    fired |= bit;
                }
            }
            if fired != 0 {
                *armed &= !fired;
                *self.intr.lock() |= fired;
            }
            fired
        };
        self.snapshot.publish_alarm_hits(self.slot, fired);
        self.publish_derived();
    }

    /// Republish the nearest armed deadline and masked status
    /// 重新发布最近的已武装截止时间与屏蔽状态
    fn publish_derived(&self) {
        let now = self.now_low();
        let mut deadlines = [0u32; ALARMS];
        let (enable, force) = {
            let regs = self.regs.lock();
            for (index, slot) in deadlines.iter_mut().enumerate() {
                *slot = regs.read(ALARM0_OFFSET + (index as u32) * 4).unwrap_or(0);
            }
            (regs.read(INTE).unwrap_or(0), regs.read(INTF).unwrap_or(0))
        };
        let armed = *self.armed.lock();
        let raw = *self.intr.lock();
        self.snapshot
            .publish_next_deadline(self.slot, nearest_deadline(&deadlines, armed, now));
        self.snapshot
            .publish_ints(self.slot, (raw | force) & enable);
    }

    /// Masked status: `(INTR | INTF) & INTE` / 屏蔽后的状态：`(INTR | INTF) & INTE`
    fn ints(&self) -> u32 {
        let (mask, force) = {
            let regs = self.regs.lock();
            (regs.read(INTE).unwrap_or(0), regs.read(INTF).unwrap_or(0))
        };
        let raw = *self.intr.lock();
        (raw | force) & mask
    }

    /// Recompute the asserted IRQ lines / 重新计算断言的中断线
    ///
    /// The fast path is lock-free: the snapshot says whether any armed deadline
    /// can be due (`next_deadline`) and what the masked status is (`ints`). Only
    /// a due deadline enters [`Self::sample`], and only lines whose level
    /// changed are written to the shared [`IrqAssert`]. Alarm firing instants are
    /// exactly the old ones, because a deadline can only fire once it is due.
    /// 快路径无锁：快照给出门控值 `next_deadline`（是否有已武装截止时间可能到期）
    /// 与屏蔽状态 `ints`。只有真的到期才进入 [`Self::sample`]，并且只把电平发生
    /// 变化的中断线写入共享 [`IrqAssert`]。alarm 的触发时刻与旧实现完全相同，
    /// 因为截止时间只有在到期之后才可能触发。
    fn refresh(&self) {
        let next = self.snapshot.next_deadline(self.slot);
        if next != 0 && reached(next, self.now_low()) {
            self.sample();
        }
        let ints = self.snapshot.ints(self.slot);
        let previous = self.published.swap(ints, Ordering::AcqRel);
        let mut changed = previous ^ ints;
        while changed != 0 {
            let index = changed.trailing_zeros();
            changed &= changed - 1;
            let line = self.irq_0 + index as u16;
            self.irq.set(line, ints & (1 << index) != 0);
        }
    }

    /// Read one register / 读取一个寄存器
    fn read32(&mut self, offset: u32) -> Option<u32> {
        match offset {
            // TIMEHW/TIMELW are write-only and read as zero.
            // TIMEHW/TIMELW 只写，读回为零。
            TIMEHW | TIMELW => Some(0),
            // 计数器四个别名都走无锁快照，每次读仍是一次独立采样。
            // All four counter aliases come from the lock-free snapshot; each
            // read is still one independent sample.
            TIMEHR | TIMERAWH => Some((self.now_us() >> 32) as u32),
            TIMELR | TIMERAWL => Some(self.now_us() as u32),
            ARMED => {
                self.sample();
                Some(*self.armed.lock())
            }
            INTR => {
                self.sample();
                Some(*self.intr.lock())
            }
            INTE => self.regs.lock().read(INTE),
            INTF => self.regs.lock().read(INTF),
            INTS => {
                self.sample();
                Some(self.ints())
            }
            LOCKED => Some(u32::from(*self.locked.lock())),
            _ => self.regs.lock().read(offset),
        }
    }

    /// Write one register / 写入一个寄存器
    fn write32(&mut self, offset: u32, value: u32) -> bool {
        match offset {
            // Writing TIME moves the counter on hardware; the model keeps the
            // cycle-driven timebase authoritative. / 硬件写 TIME 会移动计数器；
            // 本模型以周期驱动的时基为准。
            TIMEHW | TIMELW => return true,
            // INTR is write-one-to-clear. / INTR 写 1 清零。
            INTR => {
                *self.intr.lock() &= !value;
                self.snapshot.acknowledge_alarm_hits(self.slot, value);
            }
            LOCKED => *self.locked.lock() = value as u16,
            ALARM0_OFFSET..=0x1F => {
                let _ = self.regs.lock().write(offset, value);
                let bit = 1u32 << ((offset - ALARM0_OFFSET) / 4);
                if value == 0 {
                    *self.armed.lock() &= !bit;
                } else {
                    *self.armed.lock() |= bit;
                }
                // 新的武装集合：上一次命中的锁存位不再适用。
                // New arming set: hits latched by the previous one no longer apply.
                self.snapshot.clear_alarm_hits(self.slot);
            }
            // INTF is a plain read/write force register, not W1C. / INTF 是普通
            // 读写强制位寄存器，不是 W1C。
            _ => {
                let _ = self.regs.lock().write(offset, value);
            }
        }
        // The write may have changed the alarm set or the mask: republish the
        // derived atomics first so `refresh` sees the new state, then refresh.
        // 写入可能改变闹钟集合或屏蔽，因此先重新发布派生原子量使 `refresh` 看到
        // 新状态，然后再刷新中断线。
        self.publish_derived();
        self.refresh();
        true
    }

    /// Restore power-on defaults / 恢复上电默认值
    fn reset(&mut self) {
        self.regs.lock().reset();
        *self.intr.lock() = 0;
        *self.armed.lock() = 0;
        *self.locked.lock() = 0;
        self.snapshot.clear_alarm_hits(self.slot);
        self.publish_derived();
        self.refresh();
    }
}

/// TIMER0 register block / TIMER0 寄存器块
///
/// Mounted at [`map::TIMER0_BASE`]. Clones share state; [`crate::BootPeripherals`]
/// keeps a clone to recompute the asserted lines whenever the timebase advances.
/// 挂载在 [`map::TIMER0_BASE`]。克隆共享状态；[`crate::BootPeripherals`] 保留一个
/// 克隆，在时基推进时重新计算断言的中断线。
#[derive(Debug, Clone)]
pub struct Timer0 {
    /// Shared block state / 共享的块状态
    inner: TimerBlock,
}

/// TIMER1 register block / TIMER1 寄存器块
///
/// Mounted at [`map::TIMER1_BASE`]. It shares the 64-bit microsecond counter with
/// [`Timer0`] but has independent alarm and interrupt registers and raises
/// `TIMER1_IRQ_0..3` (`4..=7`).
/// 挂载在 [`map::TIMER1_BASE`]。它与 [`Timer0`] 共享 64 位微秒计数器，但 alarm
/// 与中断寄存器独立，并产生 `TIMER1_IRQ_0..3`（`4..=7`）。
#[derive(Debug, Clone)]
pub struct Timer1 {
    /// Shared block state / 共享的块状态
    inner: TimerBlock,
}

/// Define the public surface of one timer instance / 定义一个定时器实例的公共接口
macro_rules! timer_instance {
    ($ty:ty, $name:literal, $base:expr, $irq0:expr, $slot:expr) => {
        impl $ty {
            /// Create the timer connected to `timebase` and `irq`
            /// 创建连接到 `timebase` 与 `irq` 的定时器
            ///
            /// The block takes the time base's lock-free snapshot handle once,
            /// at construction; every later counter read is one atomic load.
            /// 该块在构造时取一次时基的无锁快照句柄；此后每次计数器读取都是一次
            /// 原子加载。
            #[must_use]
            pub fn new(timebase: Arc<Mutex<Timebase>>, irq: IrqAssert) -> Self {
                Self {
                    inner: TimerBlock::new($name, $base, $irq0, $slot, timebase, irq),
                }
            }

            /// Recompute the asserted alarm lines / 重新计算断言的 alarm 线
            pub fn refresh(&self) {
                self.inner.refresh();
            }

            /// Lock-free snapshot shared with the time base / 与时基共享的无锁快照
            ///
            /// Hosts can read `now_us` and the per-block derived state without
            /// taking the timebase mutex or the register locks.
            /// 宿主可以读取 `now_us` 与每块派生状态，而无需获取时基互斥锁或寄存器锁。
            #[must_use]
            pub fn snapshot(&self) -> &TimerSnapshot {
                self.inner.snapshot.as_ref()
            }

            /// Alarm bits latched by this block's most recent sample
            /// 本块最近一次采样锁存的 alarm 位
            #[must_use]
            pub fn alarm_hits(&self) -> u32 {
                self.inner.snapshot.alarm_hits($slot)
            }

            /// Latched raw status bits (`INTR`) / 锁存的原始状态位（`INTR`）
            #[must_use]
            pub fn raw_status(&self) -> u32 {
                *self.inner.intr.lock()
            }

            /// Latched armed bits (`ARMED`) / 锁存的武装位（`ARMED`）
            #[must_use]
            pub fn armed(&self) -> u32 {
                *self.inner.armed.lock()
            }

            /// Slow-path samples taken so far (tests prove the gate skips them)
            /// 目前执行的慢路径采样次数（测试用其证明门控会跳过采样）
            #[cfg(test)]
            pub(crate) fn sample_count(&self) -> u32 {
                self.inner.samples.load(Ordering::Relaxed)
            }
        }

        impl MmioDevice for $ty {
            fn name(&self) -> &str {
                self.inner.name
            }

            fn window(&self) -> MmioWindow {
                MmioWindow::new(self.inner.base, map::DEVICE_WINDOW_SIZE)
            }

            fn read32(&mut self, offset: u32) -> Option<u32> {
                self.inner.read32(offset)
            }

            fn write32(&mut self, offset: u32, value: u32) -> bool {
                self.inner.write32(offset, value)
            }

            fn reset(&mut self) {
                self.inner.reset();
            }

            fn irq_number(&self) -> Option<u16> {
                Some(self.inner.irq_0)
            }
        }
    };
}

timer_instance!(
    Timer0,
    "TIMER0",
    map::TIMER0_BASE,
    map::TIMER0_IRQ_0,
    TIMER0_SLOT
);
timer_instance!(
    Timer1,
    "TIMER1",
    map::TIMER1_BASE,
    map::TIMER1_IRQ_0,
    TIMER1_SLOT
);
