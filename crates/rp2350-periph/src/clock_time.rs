//! Unified peripheral time base / 统一的外设时基
//!
//! One [`Timebase`] is shared by TIMER0/TIMER1, TICKS and every host-side
//! diagnostic. It keeps two integrators:
//!
//! - the **CPU** integrator converts executed `clk_sys` cycles into uptime
//!   microseconds with [`rp2350_core::TimeBase`];
//! - the **reference** integrator converts the same cycles into `clk_ref`
//!   ticks and then into microseconds. RP2350's TIMER0/1 microsecond counter is
//!   clocked from `clk_ref` through a TICKS divider (`TICK = clk_ref / 1 MHz`),
//!   so this is the authoritative `TIMELR`/`TIMERAWH` value.
//!
//! Because both domains derive from the same XOSC the two microsecond counts
//! agree in every valid configuration, while the reference domain stays exact
//! when `clk_sys` is retuned.
//! 一个 [`Timebase`] 由 TIMER0/TIMER1、TICKS 与所有宿主诊断共享，内部保留两个
//! 积分器：
//!
//! - **CPU** 积分器用 [`rp2350_core::TimeBase`] 把已执行的 `clk_sys` 周期换算为
//!   运行时间微秒；
//! - **参考** 积分器把同样的周期换算为 `clk_ref` 节拍，再得到微秒。RP2350 的
//!   TIMER0/1 微秒计数器由 `clk_ref` 经 TICKS 分频（`TICK = clk_ref / 1 MHz`）
//!   驱动，因此它才是 `TIMELR`/`TIMERAWH` 的权威值。
//!
//! 两个域都来自同一晶振，因此在所有有效配置下微秒数一致；而当 `clk_sys` 改频时，
//! 参考域仍保持精确。

use rp2350_core::TimeBase;

/// Unified CPU + reference-domain time base / 统一的 CPU + 参考域时基
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timebase {
    /// Executed `clk_sys` cycles to uptime microseconds / 已执行 `clk_sys` 周期到运行时间微秒
    cpu: TimeBase,
    /// `clk_ref` ticks to timer microseconds / `clk_ref` 节拍到定时器微秒
    reference: TimeBase,
    /// Fractional numerator for `clk_sys` -> `clk_ref` conversion
    /// `clk_sys` -> `clk_ref` 换算的小数分子
    ref_acc: u128,
    /// Current reference frequency / 当前参考频率
    refclk_hz: u32,
    /// Clock-tree generation observed by the last advance / 上次推进观察到的时钟树世代
    generation: u32,
}

impl Timebase {
    /// Create a time base from the live frequencies / 以实时频率创建时基
    #[must_use]
    pub const fn new(sysclk_hz: u32, refclk_hz: u32) -> Self {
        Self {
            cpu: TimeBase::new(sysclk_hz),
            reference: TimeBase::new(refclk_hz),
            ref_acc: 0,
            refclk_hz,
            generation: 0,
        }
    }

    /// Advance both domains by executed `clk_sys` cycles
    /// 按已执行的 `clk_sys` 周期推进两个域
    ///
    /// `generation` is the clock tree's write generation. A change means a
    /// frequency-affecting register was written; the CPU integrator settles the
    /// carried remainder at the old rate inside [`TimeBase::advance`] before it
    /// adopts `sysclk_hz`.
    /// `generation` 是时钟树的写入世代。变化意味着有影响频率的寄存器被写入；
    /// CPU 积分器在 [`TimeBase::advance`] 内部先按旧频率结算余量，再采用
    /// `sysclk_hz`。
    pub fn advance(&mut self, cycles: u64, sysclk_hz: u32, refclk_hz: u32, generation: u32) {
        self.generation = generation;
        if refclk_hz != self.refclk_hz {
            // 参考域改频同样先结算小数部分。The reference domain settles too.
            self.reference.set_sysclk_hz(refclk_hz);
            self.refclk_hz = refclk_hz;
        }
        self.cpu.advance(cycles, sysclk_hz);
        if sysclk_hz == 0 || refclk_hz == 0 {
            // 时钟停止时不会产生参考节拍，剩余小数保留到恢复后。
            // A stopped clock produces no reference ticks; the fraction is kept.
            return;
        }
        let total = self.ref_acc + u128::from(cycles) * u128::from(refclk_hz);
        let rate = u128::from(sysclk_hz);
        let ticks = u64::try_from(total / rate).unwrap_or(u64::MAX);
        self.ref_acc = total % rate;
        self.reference.advance(ticks, refclk_hz);
    }

    /// Push `clk_ref` cycles directly (TICKS `CYCLES` writes)
    /// 直接推入 `clk_ref` 周期（TICKS 的 `CYCLES` 写入）
    ///
    /// TICKS programs an already-divided `clk_ref` period, so these cycles are
    /// already in the reference domain and must not be converted again.
    /// TICKS 编程的是已按 `clk_ref` 计数的周期，因此这些周期已在参考域中，不能
    /// 再次换算。
    pub fn push_ref_cycles(&mut self, cycles: u64) {
        self.reference.advance(cycles, self.refclk_hz);
    }

    /// Elapsed uptime microseconds (`clk_sys` domain) / 已运行的微秒数（`clk_sys` 域）
    #[must_use]
    pub fn time_us(&self) -> u64 {
        self.cpu.time_us()
    }

    /// TIMER0/1 microsecond counter (`clk_ref` domain) / TIMER0/1 微秒计数器（`clk_ref` 域）
    #[must_use]
    pub fn timer_us(&self) -> u64 {
        self.reference.time_us()
    }

    /// Executed cycles / 已执行的周期
    #[must_use]
    pub const fn cycles(&self) -> u64 {
        self.cpu.cycles()
    }

    /// Current `clk_sys` rate as last seen by [`Timebase::advance`]
    /// 上次 [`Timebase::advance`] 看到的 `clk_sys` 频率
    #[must_use]
    pub const fn sysclk_hz(&self) -> u32 {
        self.cpu.sysclk_hz()
    }

    /// Current `clk_ref` rate / 当前 `clk_ref` 频率
    #[must_use]
    pub const fn refclk_hz(&self) -> u32 {
        self.refclk_hz
    }

    /// Clock-tree generation observed by the last advance / 上次推进观察到的时钟树世代
    #[must_use]
    pub const fn generation(&self) -> u32 {
        self.generation
    }

    /// Uptime after `extra` more cycles at `sysclk_hz` (read-only)
    /// 在 `sysclk_hz` 下再执行 `extra` 周期后的运行时间（只读）
    #[must_use]
    pub fn project_time_us(&self, extra: u64, sysclk_hz: u32) -> u64 {
        self.cpu.project_us(extra, sysclk_hz)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_and_reference_domains_agree_at_12_and_150_mhz() {
        // 150 MHz 执行 150 000 周期：CPU 1000 µs；12 MHz 参考域得到 12 000 个
        // 节拍，同样 1000 µs。
        // 150_000 cycles at 150 MHz: 1000 µs of uptime; the 12 MHz reference
        // domain gets 12_000 ticks, also 1000 µs.
        let mut time = Timebase::new(150_000_000, 12_000_000);
        time.advance(150_000, 150_000_000, 12_000_000, 1);
        assert_eq!(time.time_us(), 1_000);
        assert_eq!(time.timer_us(), 1_000);
    }

    #[test]
    fn overclocking_keeps_the_timer_microsecond_count() {
        // 252 MHz 执行 252 000 周期仍是 1000 µs，参考域与 CPU 域都一致。
        // 252_000 cycles at 252 MHz are still 1000 µs in both domains.
        let mut time = Timebase::new(252_000_000, 12_000_000);
        time.advance(252_000, 252_000_000, 12_000_000, 2);
        assert_eq!(time.time_us(), 1_000);
        assert_eq!(time.timer_us(), 1_000);
    }

    #[test]
    fn mid_run_retune_keeps_the_partial_microsecond() {
        // 150 MHz 下 75 周期 = 0.5 µs，切到 252 MHz 后 126 周期 = 0.5 µs，
        // 合计 1 µs；参考域同样按旧频率结算。
        // 75 cycles at 150 MHz = 0.5 µs, then 126 cycles at 252 MHz = 0.5 µs:
        // 1 µs total. The reference domain settles at the old rate too.
        let mut time = Timebase::new(150_000_000, 12_000_000);
        time.advance(75, 150_000_000, 12_000_000, 1);
        time.advance(126, 252_000_000, 12_000_000, 2);
        assert_eq!(time.time_us(), 1);
        assert_eq!(time.timer_us(), 1);
        assert_eq!(time.cycles(), 201);
        assert_eq!(time.generation(), 2);
    }

    #[test]
    fn ticks_ref_cycles_advance_the_timer_domain_only() {
        // TICKS 的 `CYCLES` 已是 `clk_ref` 周期，只推进定时器域。
        // TICKS `CYCLES` is already in `clk_ref` cycles and only moves the timer
        // domain.
        let mut time = Timebase::new(150_000_000, 12_000_000);
        time.push_ref_cycles(12);
        assert_eq!(time.timer_us(), 1);
        assert_eq!(time.time_us(), 0);
        assert_eq!(time.cycles(), 0);
    }

    #[test]
    fn project_time_matches_the_next_advance() {
        let mut time = Timebase::new(150_000_000, 12_000_000);
        time.advance(75, 150_000_000, 12_000_000, 1);
        assert_eq!(time.project_time_us(75, 150_000_000), 1);
        assert_eq!(time.time_us(), 0);
    }
}
