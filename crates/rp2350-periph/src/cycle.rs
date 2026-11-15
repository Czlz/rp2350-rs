//! Shared cycle sink for peripherals that make time pass
//! 供"会让时间前进"的外设使用的共享周期汇
//!
//! Some register writes cost processor time instead of just storing a value
//! (a programmed delay, a stall, a handshake). Those devices hold a
//! [`CycleSink`] clone and push the cycles into the shared [`Timebase`], so the
//! simulated clock, TIMER0 and the reported uptime stay consistent. The
//! abstraction lives here rather than inside one device so later blocks can
//! reuse it without touching the [`rp2350_core::mmio::MmioDevice`] trait.
//! 有些寄存器写入不是单纯存值，而是消耗处理器时间（可编程延时、停顿、握手）。
//! 这类设备持有 [`CycleSink`] 克隆，把周期数推入共享 [`Timebase`]，使仿真时钟、
//! TIMER0 与上报的运行时间保持一致。该抽象放在此处而不是某个设备内部，便于后续
//! 的块直接复用，不必改动 [`rp2350_core::mmio::MmioDevice`] trait。
//!
//! Two domains exist: [`CycleSink::advance`] pushes executed `clk_sys` cycles
//! (converted with the live frequencies and clock-tree generation), while
//! [`CycleSink::push_ref`] pushes cycles that are already counted in `clk_ref`
//! (a TICKS `CYCLES` write).
//! 这里有两个域：[`CycleSink::advance`] 推入已执行的 `clk_sys` 周期（按实时频率
//! 与时钟树世代换算），[`CycleSink::push_ref`] 推入已按 `clk_ref` 计数的周期
//! （TICKS 的 `CYCLES` 写入）。

use std::sync::Arc;

use parking_lot::Mutex;

use super::clock_time::Timebase;

/// Handle that advances the shared simulation time base
/// 推进共享仿真时基的句柄
///
/// Every clone shares one counter. [`Self::advance`] is the "time passes" edge:
/// a device calls it with the number of `clk_sys` cycles the hardware would
/// spend. 所有克隆共享同一计数器。[`Self::advance`] 是"时间流逝"的入口：设备
/// 传入硬件会花费的 `clk_sys` 周期数。
#[derive(Debug, Clone)]
pub struct CycleSink {
    timebase: Arc<Mutex<Timebase>>,
}

impl CycleSink {
    /// Create a sink over an existing time base / 在已有时间基准上创建周期汇
    #[must_use]
    pub fn new(timebase: Arc<Mutex<Timebase>>) -> Self {
        Self { timebase }
    }

    /// Advance the time base by executed `clk_sys` cycles
    /// 按已执行的 `clk_sys` 周期推进时基
    ///
    /// `sysclk_hz`/`refclk_hz` are the live clock-tree values and `generation`
    /// is its write generation, so a retune settles at the old frequency.
    /// `sysclk_hz`/`refclk_hz` 是实时时钟树值，`generation` 是其写入世代，因此
    /// 改频会先按旧频率结算。
    pub fn advance(&self, cycles: u64, sysclk_hz: u32, refclk_hz: u32, generation: u32) {
        self.timebase
            .lock()
            .advance(cycles, sysclk_hz, refclk_hz, generation);
    }

    /// Push cycles that are already in the `clk_ref` domain / 推入已在 `clk_ref` 域的周期
    pub fn push_ref(&self, cycles: u64) {
        self.timebase.lock().push_ref_cycles(cycles);
    }

    /// Total `clk_sys` cycles accumulated so far / 目前累计的 `clk_sys` 周期总数
    #[must_use]
    pub fn cycles(&self) -> u64 {
        self.timebase.lock().cycles()
    }

    /// Elapsed microseconds (`clk_sys` domain) / 已过去的微秒数（`clk_sys` 域）
    #[must_use]
    pub fn time_us(&self) -> u64 {
        self.timebase.lock().time_us()
    }

    /// TIMER microsecond counter (`clk_ref` domain) / TIMER 微秒计数器（`clk_ref` 域）
    #[must_use]
    pub fn timer_us(&self) -> u64 {
        self.timebase.lock().timer_us()
    }

    /// Uptime after `extra` more `clk_sys` cycles (read-only)
    /// 再执行 `extra` 个 `clk_sys` 周期后的运行时间（只读）
    #[must_use]
    pub fn project_time_us(&self, extra: u64, sysclk_hz: u32) -> u64 {
        self.timebase.lock().project_time_us(extra, sysclk_hz)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sink() -> CycleSink {
        CycleSink::new(Arc::new(Mutex::new(Timebase::new(150_000_000, 12_000_000))))
    }

    #[test]
    fn clones_share_one_counter() {
        let first = sink();
        let second = first.clone();
        first.advance(150, 150_000_000, 12_000_000, 1);
        second.advance(150, 150_000_000, 12_000_000, 1);
        assert_eq!(first.cycles(), 300);
        assert_eq!(second.time_us(), 2);
        assert_eq!(second.timer_us(), 2);
    }

    #[test]
    fn reference_pushes_only_move_the_timer_domain() {
        let sink = sink();
        sink.push_ref(12);
        assert_eq!(sink.timer_us(), 1);
        assert_eq!(sink.time_us(), 0);
    }
}
