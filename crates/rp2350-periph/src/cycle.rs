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
//! TIMER0 与上报的运行时长时间保持一致。该抽象放在此处而不是某个设备内部，
//! 便于后续的块直接复用，不必改动 [`rp2350_core::mmio::MmioDevice`] trait。

use std::sync::Arc;

use parking_lot::Mutex;

use super::boot::Timebase;

/// Handle that advances the shared simulation time base
/// 推进共享仿真时基的句柄
///
/// Every clone shares one counter. [`Self::push`] is the "time passes" edge:
/// a device calls it with the number of cycles the hardware would spend.
/// 所有克隆共享同一计数器。[`Self::push`] 是"时间流逝"的入口：设备传入硬件
/// 会花费的周期数。
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

    /// Advance the shared time base by `cycles` / 将共享时基推进 `cycles` 个周期
    pub fn push(&self, cycles: u64) {
        self.timebase.lock().advance(cycles);
    }

    /// Total cycles accumulated so far / 目前累计的周期总数
    #[must_use]
    pub fn cycles(&self) -> u64 {
        self.timebase.lock().cycles()
    }

    /// Elapsed microseconds / 已经过的微秒数
    #[must_use]
    pub fn time_us(&self) -> u64 {
        self.timebase.lock().time_us()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sink() -> CycleSink {
        CycleSink::new(Arc::new(Mutex::new(Timebase::new(150))))
    }

    #[test]
    fn clones_share_one_counter() {
        let first = sink();
        let second = first.clone();
        first.push(150);
        second.push(150);
        assert_eq!(first.cycles(), 300);
        assert_eq!(second.time_us(), 2);
    }
}
