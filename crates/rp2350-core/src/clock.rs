//! Simulation clock, frequency-aware time base and divider helpers
//! 仿真时钟、按频率积分的时间基与分频辅助
//!
//! [`TimeBase`] is the authoritative `cycles -> microseconds` integrator used by
//! every timing consumer. It keeps the executed cycle count, the whole
//! microseconds already settled and a sub-microsecond remainder, and it
//! **settles at the old frequency before a retune**, so a mid-run clock change
//! never loses or double-counts the cycles executed on either side.
//! [`TimeBase`] 是所有计时消费者使用的权威 `周期 -> 微秒` 积分器。它保存已执行
//! 周期数、已结算的整微秒与不足一微秒的余量，并在**改频前按旧频率结算**，因此
//! 运行中途改频既不会丢失也不会重复计算两侧执行的周期。
//!
//! [`SimulationClock`] is the legacy cumulative-cycle tracker kept for
//! compatibility; it reports frequency changes but does not integrate time.
//! [`SimulationClock`] 是为兼容保留的旧累计周期追踪器；它报告频率变化，但不做
//! 时间积分。

use super::types::CycleCount;

/// One microsecond in picoseconds / 一微秒对应的皮秒数
const PS_PER_US: u64 = 1_000_000;

/// System clock frequencies (hardware defaults for RP2350)
/// 系统时钟频率（RP2350 硬件默认值）
#[derive(Debug, Clone, Copy)]
pub struct ClockConfig {
    /// Reference clock (typically 12 MHz from XOSC) / 参考时钟（通常为 XOSC 12 MHz）
    pub refclk_hz: u32,
    /// System clock after PLL / PLL 后的系统时钟
    pub sysclk_hz: u32,
    /// Peripheral clock (sysclk / divider) / 外设时钟（sysclk / 分频）
    pub periphclk_hz: u32,
}

impl Default for ClockConfig {
    fn default() -> Self {
        Self {
            refclk_hz: 12_000_000,
            sysclk_hz: 150_000_000,
            periphclk_hz: 150_000_000,
        }
    }
}

/// Frequency-aware `cycles -> microseconds` integrator
/// 按频率积分的 `周期 -> 微秒` 时间基
///
/// # Exactness / 精确性
///
/// Within one frequency the integration is integer-exact: `micros` grows by
/// `(remainder + cycles) / hz` and the leftover cycles stay in `remainder`.
/// Before a retune the leftover is converted to a picosecond fraction at the
/// **old** rate, so a 150 MHz → 252 MHz switch keeps the partial microsecond.
/// 同一频率内积分是整数精确的：`micros` 增加 `(余量 + 周期) / hz`，余数留在
/// `remainder`。改频前余量按**旧**频率换算成皮秒小数，因此 150 MHz → 252 MHz
/// 的切换会保留不足一微秒的部分。
///
/// # Zero frequency / 零频率
///
/// A zero frequency is a stopped clock: executed cycles are still counted, no
/// time passes and one warning is emitted. 零频率表示时钟停止：仍统计已执行周期，
/// 但时间不前进，并只告警一次。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeBase {
    /// Executed cycles since start / 自启动以来已执行的周期
    cycles: u64,
    /// Settled whole microseconds / 已结算的整微秒
    micros: u64,
    /// Sub-microsecond numerator in `cycle × 1e6` units, always `< sysclk_hz`
    /// 不足一微秒的分子（`周期 × 1e6` 单位），恒 `< sysclk_hz`
    remainder: u64,
    /// Sub-microsecond picoseconds carried across retunes / 跨改频携带的皮秒小数
    frac_ps: u64,
    /// Current rate in Hz / 当前频率（Hz）
    sysclk_hz: u32,
    /// Zero-frequency warning latch / 零频告警锁存
    zero_warned: bool,
}

impl TimeBase {
    /// Create a time base at `sysclk_hz` / 以 `sysclk_hz` 创建时间基
    #[must_use]
    pub const fn new(sysclk_hz: u32) -> Self {
        Self {
            cycles: 0,
            micros: 0,
            remainder: 0,
            frac_ps: 0,
            sysclk_hz,
            zero_warned: false,
        }
    }

    /// Current rate in Hz / 当前频率（Hz）
    #[must_use]
    pub const fn sysclk_hz(&self) -> u32 {
        self.sysclk_hz
    }

    /// Total executed cycles / 已执行的周期总数
    #[must_use]
    pub const fn cycles(&self) -> u64 {
        self.cycles
    }

    /// Settled whole microseconds / 已结算的整微秒
    #[must_use]
    pub const fn micros(&self) -> u64 {
        self.micros
    }

    /// Cycles not yet converted at the current rate / 当前频率下尚未换算的周期
    ///
    /// The value is a numerator in `cycle × 1e6` units over the current rate, so
    /// dividing it by [`Self::sysclk_hz`] yields the fractional microsecond.
    /// 该值是当前频率下以 `周期 × 1e6` 为单位的分子；除以 [`Self::sysclk_hz`]
    /// 即得小数微秒。
    #[must_use]
    pub const fn remainder(&self) -> u64 {
        self.remainder
    }

    /// Retune **after settling** the carried remainder at the old rate
    /// 按旧频率结算携带的余量后改频
    ///
    /// Passing the current rate is a no-op, so callers may hand in the live
    /// clock value unconditionally. 传入当前频率时不做任何事，调用方可以直接传入
    /// 实时时钟值。
    pub fn set_sysclk_hz(&mut self, hz: u32) {
        if hz == self.sysclk_hz {
            return;
        }
        self.settle_remainder();
        self.sysclk_hz = hz;
        if hz == 0 && !self.zero_warned {
            self.zero_warned = true;
            log::warn!(
                "clock: sysclk fell to 0 Hz; simulated time stands still until it is restored / \
                 时钟：sysclk 变为 0 Hz，恢复前仿真时间停止前进"
            );
        }
    }

    /// Advance by `cycles` executed at `sysclk_hz` / 按 `sysclk_hz` 推进 `cycles` 个周期
    ///
    /// The rate is checked on every call, which makes a generation change in the
    /// clock tree visible without a separate notification. 每次调用都检查频率，
    /// 因此时钟树世代变化无需额外通知即可生效。
    pub fn advance(&mut self, cycles: u64, sysclk_hz: u32) {
        if sysclk_hz != self.sysclk_hz {
            self.set_sysclk_hz(sysclk_hz);
        }
        self.cycles = self.cycles.saturating_add(cycles);
        if self.sysclk_hz == 0 {
            if !self.zero_warned {
                self.zero_warned = true;
                log::warn!(
                    "clock: sysclk is 0 Hz; executed cycles are counted but time does not advance / \
                     时钟：sysclk 为 0 Hz；仍统计执行周期，但时间不前进"
                );
            }
            return;
        }
        let total = u128::from(self.remainder) + u128::from(cycles) * u128::from(PS_PER_US);
        let rate = u128::from(self.sysclk_hz);
        // 整微秒进入 `micros`，余数留到下次；u128 避免长窗口溢出。
        // Whole microseconds land in `micros`; the remainder waits for the next
        // call. u128 keeps long windows from overflowing.
        self.micros = self
            .micros
            .saturating_add(u64::try_from(total / rate).unwrap_or(u64::MAX));
        self.remainder = u64::try_from(total % rate).unwrap_or(0);
    }

    /// Elapsed microseconds including the sub-microsecond remainder
    /// 已过去的微秒数（含不足一微秒的余量）
    #[must_use]
    pub fn time_us(&self) -> u64 {
        self.project(0, self.sysclk_hz)
    }

    /// Microseconds after running `extra` more cycles at `hz` (read-only)
    /// 在 `hz` 下再执行 `extra` 周期后的微秒数（只读）
    ///
    /// Used by diagnostics that report the current time before the next batched
    /// advance. 供诊断在下一次批量推进之前报告当前时间。
    #[must_use]
    pub fn project_us(&self, extra: u64, hz: u32) -> u64 {
        self.project(extra, hz)
    }

    /// Simulate an advance without mutating the time base / 不改状态地模拟一次推进
    fn project(&self, extra: u64, hz: u32) -> u64 {
        let mut micros = self.micros;
        let mut frac_ps = u128::from(self.frac_ps);
        let mut remainder = u128::from(self.remainder);
        let mut rate = self.sysclk_hz;
        if hz != rate {
            // 与真实改频一致：先按旧频率结算余量。Settle at the old rate first.
            if rate > 0 {
                frac_ps += remainder * u128::from(PS_PER_US) / u128::from(rate);
            }
            remainder = 0;
            rate = hz;
        }
        if rate == 0 {
            return micros
                .saturating_add(u64::try_from(frac_ps / u128::from(PS_PER_US)).unwrap_or(0));
        }
        let total = remainder + u128::from(extra) * u128::from(PS_PER_US);
        let rate128 = u128::from(rate);
        micros = micros.saturating_add(u64::try_from(total / rate128).unwrap_or(u64::MAX));
        remainder = total % rate128;
        frac_ps += remainder * u128::from(PS_PER_US) / rate128;
        micros.saturating_add(u64::try_from(frac_ps / u128::from(PS_PER_US)).unwrap_or(0))
    }

    /// Convert the carried cycles into a picosecond fraction at the old rate
    /// 按旧频率把携带的周期换算为皮秒小数
    fn settle_remainder(&mut self) {
        if self.sysclk_hz == 0 {
            self.remainder = 0;
            return;
        }
        let rate = u128::from(self.sysclk_hz);
        let add = u128::from(self.remainder) * u128::from(PS_PER_US) / rate;
        self.frac_ps = self
            .frac_ps
            .saturating_add(u64::try_from(add).unwrap_or(u64::MAX));
        self.remainder = 0;
        if self.frac_ps >= PS_PER_US {
            self.micros = self.micros.saturating_add(self.frac_ps / PS_PER_US);
            self.frac_ps %= PS_PER_US;
        }
    }
}

impl Default for TimeBase {
    fn default() -> Self {
        Self::new(ClockConfig::default().sysclk_hz)
    }
}

/// Simulation clock tracker
/// 仿真时钟追踪器
///
/// Tracks the total cycle count and provides tick advancement with
/// configurable clock frequency. Timing consumers should use [`TimeBase`];
/// this type only keeps the historical cumulative-cycle API.
/// 追踪总周期计数，支持可配置时钟频率的 tick 推进。计时消费者应使用 [`TimeBase`]；
/// 本类型只保留历史累计周期接口。
pub struct SimulationClock {
    /// Total cycles elapsed since simulation start
    /// 自仿真启动以来经过的总周期数
    total_cycles: CycleCount,
    /// Clock configuration / 时钟配置
    config: ClockConfig,
}

impl SimulationClock {
    /// Create a new simulation clock with default RP2350 configuration
    /// 使用默认 RP2350 配置创建仿真时钟
    #[must_use]
    pub fn new() -> Self {
        Self {
            total_cycles: 0,
            config: ClockConfig::default(),
        }
    }

    /// Create with custom clock config / 使用自定义时钟配置创建
    #[must_use]
    pub fn with_config(config: ClockConfig) -> Self {
        Self {
            total_cycles: 0,
            config,
        }
    }

    /// Get total cycles elapsed / 获取经过的总周期数
    #[must_use]
    pub fn total_cycles(&self) -> CycleCount {
        self.total_cycles
    }

    /// Get current clock config / 获取当前时钟配置
    #[must_use]
    pub fn config(&self) -> ClockConfig {
        self.config
    }

    /// Update the system clock frequency / 更新系统时钟频率
    ///
    /// [`TimeBase`] is the frequency-aware integrator; this setter only updates
    /// the value reported by this legacy tracker.
    /// [`TimeBase`] 才是按频率积分的时间基；本 setter 只更新旧追踪器报告的值。
    pub fn set_sysclk_hz(&mut self, hz: u32) {
        self.config.sysclk_hz = hz;
    }

    /// Update the peripheral clock frequency / 更新外设时钟频率
    pub fn set_periphclk_hz(&mut self, hz: u32) {
        self.config.periphclk_hz = hz;
    }

    /// Update the reference clock frequency / 更新参考时钟频率
    pub fn set_refclk_hz(&mut self, hz: u32) {
        self.config.refclk_hz = hz;
    }

    /// Advance the simulation by `n` system clock cycles
    /// 将仿真推进 n 个系统时钟周期
    pub fn tick(&mut self, cycles: CycleCount) {
        self.total_cycles += cycles;
    }

    /// Read the `SysTick` counter value (24-bit down-counter)
    /// 读取 `SysTick` 计数器值（24 位递减计数器）
    #[must_use]
    pub fn systick_value(&self, reload: u32, enable: bool) -> u32 {
        if !enable || reload == 0 {
            return 0;
        }
        #[allow(clippy::cast_possible_truncation)]
        let elapsed = (self.total_cycles % u64::from(reload + 1)) as u32;
        reload - elapsed
    }

    /// Calculate peripheral clock divider cycles for a given peripheral frequency
    /// 计算外设时钟分频器周期数
    #[must_use]
    pub fn peripheral_cycles(&self, target_hz: u32, ticks: CycleCount) -> CycleCount {
        if target_hz == 0 || self.config.periphclk_hz == 0 {
            return 0;
        }
        ticks * u64::from(target_hz) / u64::from(self.config.periphclk_hz)
    }
}

impl Default for SimulationClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_increments() {
        let mut clock = SimulationClock::new();
        assert_eq!(clock.total_cycles(), 0);
        clock.tick(100);
        assert_eq!(clock.total_cycles(), 100);
        clock.tick(50);
        assert_eq!(clock.total_cycles(), 150);
    }

    #[test]
    fn peripheral_cycles_scaling() {
        let clock = SimulationClock::new();
        // periphclk = 150MHz, target = 75MHz → 1/2 rate
        let periph = clock.peripheral_cycles(75_000_000, 100);
        assert_eq!(periph, 50);
    }

    #[test]
    fn zero_frequency_returns_zero() {
        let clock = SimulationClock::new();
        assert_eq!(clock.peripheral_cycles(0, 100), 0);
    }

    #[test]
    fn set_sysclk_hz_updates_the_reported_config() {
        let mut clock = SimulationClock::new();
        clock.tick(42);
        clock.set_sysclk_hz(252_000_000);
        assert_eq!(clock.config().sysclk_hz, 252_000_000);
        assert_eq!(clock.config().refclk_hz, 12_000_000);
        assert_eq!(clock.total_cycles(), 42);
    }

    #[test]
    fn set_periphclk_hz_drives_peripheral_scaling() {
        let mut clock = SimulationClock::new();
        clock.set_periphclk_hz(252_000_000);
        assert_eq!(clock.peripheral_cycles(126_000_000, 100), 50);
        clock.set_refclk_hz(24_000_000);
        assert_eq!(clock.config().refclk_hz, 24_000_000);
    }

    #[test]
    fn time_base_integrates_exactly_at_one_rate() {
        // 150 MHz：150 周期 = 1 µs，余数保持为 0。
        // 150 MHz: 150 cycles = 1 µs and the remainder stays zero.
        let mut time = TimeBase::new(150_000_000);
        time.advance(150, 150_000_000);
        assert_eq!(time.time_us(), 1);
        assert_eq!(time.remainder(), 0);
        time.advance(75, 150_000_000);
        assert_eq!(time.time_us(), 1, "half a microsecond is still fractional");
        time.advance(75, 150_000_000);
        assert_eq!(time.time_us(), 2);
        assert_eq!(time.cycles(), 300);
        assert_eq!(time.micros(), 2);
    }

    #[test]
    fn retune_settles_the_remainder_at_the_old_rate() {
        // 150 MHz 下 75 周期 = 0.5 µs；切到 252 MHz 后 126 周期 = 0.5 µs，
        // 两者合计正好 1 µs。如果余量丢失，结果会是 0。
        // 75 cycles at 150 MHz = 0.5 µs; 126 cycles at 252 MHz = 0.5 µs. Added
        // together they are exactly 1 µs; losing the remainder would report 0.
        let mut time = TimeBase::new(150_000_000);
        time.advance(75, 150_000_000);
        assert_eq!(time.time_us(), 0);
        time.advance(126, 252_000_000);
        assert_eq!(time.sysclk_hz(), 252_000_000);
        assert_eq!(time.time_us(), 1);
    }

    #[test]
    fn a_150_to_252_switch_keeps_the_elapsed_microseconds() {
        // 150 MHz 下 1 µs，然后 252 MHz 下 1 µs：总时间 2 µs，周期 402。
        // 1 µs at 150 MHz then 1 µs at 252 MHz: 2 µs total over 402 cycles.
        let mut time = TimeBase::new(150_000_000);
        time.advance(150, 150_000_000);
        time.advance(252, 252_000_000);
        assert_eq!(time.time_us(), 2);
        assert_eq!(time.cycles(), 402);
        // 同一段周期若被误当成 150 MHz 会得到 2.68 µs，说明改频确实生效。
        // Treating the same cycles as 150 MHz would give 2.68 µs, proving the
        // retune took effect.
        assert_eq!(TimeBase::new(150_000_000).project_us(402, 150_000_000), 2);
        assert_eq!(TimeBase::new(252_000_000).project_us(402, 252_000_000), 1);
    }

    #[test]
    fn zero_frequency_stops_time_but_counts_cycles() {
        let mut time = TimeBase::new(150_000_000);
        time.advance(150, 150_000_000);
        time.advance(10_000, 0);
        assert_eq!(time.time_us(), 1, "a stopped clock produces no time");
        assert_eq!(time.cycles(), 10_150, "executed cycles are still counted");
        time.advance(150, 150_000_000);
        assert_eq!(time.time_us(), 2, "time resumes without retroactive credit");
    }

    #[test]
    fn project_us_matches_advance_without_mutating() {
        let mut time = TimeBase::new(150_000_000);
        time.advance(75, 150_000_000);
        assert_eq!(time.project_us(75, 150_000_000), 1);
        assert_eq!(time.time_us(), 0, "projection is read-only");
        assert_eq!(time.project_us(126, 252_000_000), 1);
    }

    #[test]
    fn one_second_needs_the_live_rate_in_cycles() {
        // 1 秒模拟时间在 150 MHz 下是 150M 周期，在 252 MHz 下是 252M 周期。
        // One simulated second is 150M cycles at 150 MHz and 252M at 252 MHz.
        let mut slow = TimeBase::new(150_000_000);
        slow.advance(150_000_000, 150_000_000);
        assert_eq!(slow.time_us(), 1_000_000);
        let mut fast = TimeBase::new(252_000_000);
        fast.advance(252_000_000, 252_000_000);
        assert_eq!(fast.time_us(), 1_000_000);
    }
}
