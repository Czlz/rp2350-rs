//! Sleep accounting and future-wake tracking for the machine driver
//! 机器驱动器的睡眠记账与未来唤醒追踪
//!
//! A `WFI`/`WFE` retires one instruction and then parks the core without
//! consuming any further instruction budget. Simulated time still has to move,
//! otherwise a firmware waiting for a timer, DMA or USB interrupt could never
//! wake up. This module keeps that accounting (`sleep=`, the idle streak and the
//! `halt: sleeping` thresholds) and bridges a fired `TIMER0` alarm into the
//! NVIC while the timer model still reports its interrupt registers at the
//! wrong offsets.
//! `WFI`/`WFE` 退休一条指令后让核心停摆，不再消耗指令预算。模拟时间必须继续
//! 前进，否则等待定时器、DMA 或 USB 中断的固件永远无法唤醒。本模块负责这部分
//! 记账（`sleep=`、空闲连续时长与 `halt: sleeping` 阈值），并在定时器模型仍以
//! 错误偏移上报中断寄存器时，把已到期的 `TIMER0` alarm 桥接到 NVIC。

/// Halt reason reported when every core is asleep with no wake source
/// 所有核心都睡眠且没有唤醒源时报告的停机原因
///
/// `run` ends instead of burning the instruction budget on a machine that can
/// only make progress through an interrupt that will never arrive.
/// `run` 会直接结束，而不是把预算耗在一台只能靠永不到来的中断推进的机器上。
pub const SLEEPING_REASON: &str = "sleeping (no pending interrupt) / 睡眠（无挂起中断）";

/// Simulated microseconds of source-less sleep before `run` gives up
/// `run` 放弃前允许的无唤醒源睡眠模拟微秒数
///
/// 100 ms is long enough for a firmware's own timeouts to fire a peripheral
/// line, yet short enough that a genuinely dead wait is reported quickly.
/// 100 ms 足以让固件自身的超时触发外设中断线，又能让真正死掉的等待被快速报告。
pub const SLEEP_HALT_AFTER_US: u64 = 100_000;

/// Simulated microseconds of total sleep before `run` stops unconditionally
/// `run` 无条件停止前允许的睡眠模拟微秒总数
///
/// Safety valve so a heartbeat that never arrives cannot spin forever even when
/// a wake source (for example a peer core that keeps running) is reported.
/// 安全阀：即使始终报告存在唤醒源（例如一直在运行的对端核心），永不出现的心跳
/// 也无法让 `run` 无限空转。
pub const SLEEP_HARD_LIMIT_US: u64 = 10_000_000;

/// `TIMER0` register window base / `TIMER0` 寄存器窗口基地址
pub const TIMER0_BASE: u32 = 0x400B_0000;

/// `TIMER0.ALARM0` offset (alarms 1..3 follow at +4)
/// `TIMER0.ALARM0` 偏移（alarm 1..3 依次 +4）
pub const TIMER0_ALARM0: u32 = 0x10;

/// `TIMER0.ARMED` offset / `TIMER0.ARMED` 偏移
pub const TIMER0_ARMED: u32 = 0x20;

/// `TIMER0.TIMERAWL` offset / `TIMER0.TIMERAWL` 偏移
pub const TIMER0_TIMERAWL: u32 = 0x28;

/// `TIMER0` forced-interrupt bits / `TIMER0` 强制中断位
///
/// pico-sdk's alarm pool forces the alarm IRQ (`hw_set_bits(&timer->intf, …)`)
/// instead of arming the hardware alarm directly, then lets the handler program
/// it. Reading this offset back yields the forced bits in both the current and
/// the corrected register layout.
/// pico-sdk 的 alarm pool 并不直接武装硬件 alarm，而是先强制中断
/// （`hw_set_bits(&timer->intf, …)`），再由处理程序编程。无论当前还是修正后的
/// 寄存器布局，回读该偏移都能得到强制位。
pub const TIMER0_FORCE: u32 = 0x44;

/// Number of `TIMER0` alarm channels / `TIMER0` alarm 通道数
const TIMER0_ALARMS: usize = 4;

/// 32-bit "has the deadline been reached" comparison
/// 32 位「截止时间是否已到」比较
///
/// The timer counter and the alarm registers are 32-bit halves of a 64-bit
/// microsecond counter. Treating the difference as signed handles wrap-around
/// as long as the deadline is less than ~35 minutes away (the low 32 bits cover
/// 4294 s), which is the same assumption pico-sdk makes.
/// 计数器与 alarm 寄存器是 64 位微秒计数器的低 32 位。把差值按有符号处理即可
/// 处理回绕，前提是截止时间在约 35 分钟内（低 32 位覆盖 4294 秒），与 pico-sdk
/// 的假设一致。
#[must_use]
pub fn deadline_reached(deadline: u32, now: u32) -> bool {
    deadline != 0 && now.wrapping_sub(deadline) < 0x8000_0000
}

/// Idle ticks attributed to sleeping cores / 记入睡眠核心的空闲节拍
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SleepStats {
    /// Sleep ticks since reset / 复位以来的睡眠节拍数
    ticks: u64,
    /// Ticks at which a wake source was last observed
    /// 最近一次观察到唤醒源时的节拍数
    idle_since: u64,
}

impl SleepStats {
    /// Create a zeroed counter / 创建清零的计数器
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ticks: 0,
            idle_since: 0,
        }
    }

    /// Attribute `ticks` idle ticks at once / 一次记入 `ticks` 个空闲节拍
    ///
    /// Saturating because a long idle run must never wrap the clock back.
    /// 使用饱和加法，长时间空闲运行不会让时钟回绕。
    pub fn advance(&mut self, ticks: u64) {
        self.ticks = self.ticks.saturating_add(ticks);
    }

    /// Sleep ticks accumulated so far / 已累计的睡眠节拍
    #[must_use]
    pub fn ticks(self) -> u64 {
        self.ticks
    }

    /// Restart the source-less streak / 重新开始无唤醒源的连续计时
    pub fn note_wake_source(&mut self) {
        self.idle_since = self.ticks;
    }

    /// Ticks slept since the last observed wake source
    /// 自上次观察到唤醒源以来睡眠的节拍数
    #[must_use]
    pub fn idle_ticks(self) -> u64 {
        self.ticks.saturating_sub(self.idle_since)
    }
}

/// Tracks fired `TIMER0` alarms so the driver can deliver their NVIC lines
/// 追踪已到期的 `TIMER0` alarm，使驱动器能投递其中断线
///
/// The peripheral timer decides which alarm fired from `INTR`, which the current
/// model exposes at an RP2040-era offset; while that mismatch exists the timer's
/// own IRQ line never asserts, so a `WFE` waiting for a future alarm would never
/// wake. [`AlarmWake`] observes the guest-visible alarm/force registers and
/// reports each deadline exactly once. Once the timer model is corrected the
/// line is asserted as well; delivering the same NVIC pending bit twice is
/// harmless because it is a set operation.
/// 外设定时器通过 `INTR` 判断哪个 alarm 到期，而当前模型把它放在 RP2040 时代的
/// 偏移上；在该偏差存在期间定时器中断线永不断言，等待未来 alarm 的 `WFE` 便无法
/// 唤醒。[`AlarmWake`] 观察客户机可见的 alarm/force 寄存器，并把每个截止时间
/// 恰好报告一次。定时器模型修正后中断线也会断言；重复设置同一个 NVIC 挂起位是
/// 幂等的，因此没有副作用。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AlarmWake {
    /// Alarm value already delivered per channel / 每通道已投递的 alarm 值
    delivered: [u32; TIMER0_ALARMS],
    /// Forced-interrupt bits observed on the previous poll / 上次轮询观察到的强制中断位
    forced: u32,
}

impl AlarmWake {
    /// Create an empty tracker / 创建空追踪器
    #[must_use]
    pub const fn new() -> Self {
        Self {
            delivered: [0; TIMER0_ALARMS],
            forced: 0,
        }
    }

    /// Report whether an alarm or a forced request is still outstanding
    /// 报告是否仍有未完成的 alarm 或强制请求
    #[must_use]
    pub fn has_future_wakeup(now: u32, alarms: [u32; TIMER0_ALARMS], forced: u32) -> bool {
        forced != 0 || alarms.iter().any(|alarm| deadline_reached(*alarm, now))
    }

    /// Observe one snapshot and return the alarm channels whose IRQ is due
    /// 观察一次快照并返回应投递中断的 alarm 通道（位 0..3）
    ///
    /// A channel is reported when its forced bit rises (the alarm pool asking
    /// the handler to program a hardware alarm) or when its newly programmed
    /// deadline is reached. `delivered` remembers the last deadline value, so a
    /// handler that leaves the register alone cannot cause an interrupt storm.
    /// 通道在两种情况下上报：强制位置位（alarm pool 要求处理程序编程硬件 alarm），
    /// 或其新写入的截止时间已到。`delivered` 记住上次的截止值，因此保持寄存器不变
    /// 的处理程序不会造成中断风暴。
    pub fn observe(&mut self, now: u32, alarms: [u32; TIMER0_ALARMS], forced: u32) -> u32 {
        let mut due = 0u32;
        for (index, alarm) in alarms.iter().enumerate() {
            let bit = 1u32 << index;
            if alarm == &0 {
                self.delivered[index] = 0;
                continue;
            }
            if deadline_reached(*alarm, now) && self.delivered[index] != *alarm {
                self.delivered[index] = *alarm;
                due |= bit;
            }
        }
        // 只有新出现的强制位才投递一次：处理程序清位后再次置位算新一轮。
        // Deliver only newly raised force bits; a bit cleared by the handler and
        // raised again counts as a new request.
        due |= forced & !self.forced;
        self.forced = forced;
        due & 0xF
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_counter_is_zero() {
        let mut stats = SleepStats::new();
        assert_eq!(stats.ticks(), 0);
        assert_eq!(stats.idle_ticks(), 0);
        stats.advance(1);
        assert_eq!(stats.ticks(), 1);
    }

    #[test]
    fn ticks_saturate_instead_of_wrapping() {
        let mut stats = SleepStats {
            ticks: u64::MAX - 1,
            idle_since: 0,
        };
        stats.advance(64);
        assert_eq!(stats.ticks(), u64::MAX);
    }

    #[test]
    fn advance_adds_a_whole_batch() {
        let mut stats = SleepStats::new();
        stats.advance(64);
        stats.advance(64);
        assert_eq!(stats.ticks(), 128);
        assert_eq!(stats.idle_ticks(), 128);
    }

    #[test]
    fn wake_source_restarts_the_idle_streak() {
        let mut stats = SleepStats::new();
        for _ in 0..10 {
            stats.advance(1);
        }
        assert_eq!(stats.idle_ticks(), 10);
        stats.note_wake_source();
        assert_eq!(stats.idle_ticks(), 0);
        stats.advance(1);
        assert_eq!(stats.idle_ticks(), 1);
    }

    #[test]
    fn sleeping_reason_mentions_the_missing_wake_source() {
        assert!(SLEEPING_REASON.contains("sleeping"));
        assert!(SLEEPING_REASON.contains("no pending interrupt"));
    }

    #[test]
    fn deadline_reached_handles_wrap_around() {
        assert!(!deadline_reached(0, 100));
        assert!(!deadline_reached(200, 100));
        assert!(deadline_reached(100, 100));
        assert!(deadline_reached(100, 200));
        // 已回绕：截止 0xFFFF_FFF0，当前 0x10。 / Wrapped: deadline near u32::MAX.
        assert!(deadline_reached(0xFFFF_FFF0, 0x10));
        assert!(!deadline_reached(0x10, 0xFFFF_FFF0));
    }

    #[test]
    fn a_fired_alarm_is_reported_once() {
        let mut wake = AlarmWake::new();
        let alarms = [1_000, 0, 0, 0];
        assert!(!AlarmWake::has_future_wakeup(500, alarms, 0));
        assert_eq!(wake.observe(500, alarms, 0), 0);
        assert!(AlarmWake::has_future_wakeup(1_000, alarms, 0));
        assert_eq!(wake.observe(1_000, alarms, 0), 1);
        // 同一截止值不会重复投递。 / The same deadline is not redelivered.
        assert_eq!(wake.observe(2_000, alarms, 0), 0);
        // 处理程序改写新截止值后重新计时。 / A new deadline arms a new delivery.
        let next = [3_000, 0, 0, 0];
        assert_eq!(wake.observe(2_000, next, 0), 0);
        assert_eq!(wake.observe(3_000, next, 0), 1);
    }

    #[test]
    fn forced_bits_raise_their_channel_once() {
        let mut wake = AlarmWake::new();
        let alarms = [0; 4];
        assert!(AlarmWake::has_future_wakeup(0, alarms, 0b1000));
        assert_eq!(wake.observe(0, alarms, 0b1000), 0b1000);
        assert_eq!(wake.observe(0, alarms, 0b1000), 0);
        // 处理程序清位后再次强制算新一轮。 / Cleared then re-forced is new.
        assert_eq!(wake.observe(0, alarms, 0), 0);
        assert_eq!(wake.observe(0, alarms, 0b1000), 0b1000);
    }

    #[test]
    fn disarming_an_alarm_forgets_its_deadline() {
        let mut wake = AlarmWake::new();
        assert_eq!(wake.observe(1_000, [1_000, 0, 0, 0], 0), 1);
        assert_eq!(wake.observe(1_100, [0, 0, 0, 0], 0), 0);
        // 重新武装同一个数值时再次投递。 / Re-arming the same value delivers again.
        assert_eq!(wake.observe(1_200, [1_100, 0, 0, 0], 0), 1);
    }
}
