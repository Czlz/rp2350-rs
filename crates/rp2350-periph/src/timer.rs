//! TIMER0 peripheral model / TIMER0 外设模型
//!
//! A 64-bit microsecond counter runs off the shared [`Timebase`], which the
//! simulation driver advances with executed cycles. Alarm registers, `ARMED`,
//! `INTR`, `INTE`, `INTF` and `INTS` are modelled; interrupt delivery to the
//! NVIC is not wired in Stage 4.
//! 64 位微秒计数器由共享的 [`Timebase`] 驱动，仿真驱动按已执行周期推进它。
//! 闹钟寄存器、`ARMED`、`INTR`、`INTE`、`INTF` 与 `INTS` 均已建模；Stage 4
//! 尚未将中断送达 NVIC。

use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::boot::Timebase;
use super::map;

/// Number of alarm channels / 闹钟通道数量
const ALARMS: usize = 4;

/// Offset of the first alarm register / 首个闹钟寄存器的偏移
const ALARM0_OFFSET: u32 = 0x10;

/// TIMER0 register block / TIMER0 寄存器块
#[derive(Debug)]
pub struct Timer0 {
    timebase: Arc<Mutex<Timebase>>,
    regs: RegisterFile,
    /// Alarms already acknowledged through `INTR`. 已通过 `INTR` 确认的闹钟。
    cleared: u32,
}

impl Timer0 {
    /// Create the timer connected to `timebase` / 创建连接到 `timebase` 的定时器
    #[must_use]
    pub fn new(timebase: Arc<Mutex<Timebase>>) -> Self {
        Self {
            timebase,
            regs: RegisterFile::new(map::DEVICE_WINDOW_SIZE),
            cleared: 0,
        }
    }

    /// Current 64-bit microsecond counter / 当前 64 位微秒计数器
    fn now_us(&self) -> u64 {
        self.timebase.lock().time_us()
    }

    /// Alarms whose deadline has passed / 已到期的闹钟
    fn fired(&self) -> u32 {
        let now = self.now_us();
        let mut fired = 0;
        for index in 0..ALARMS {
            let offset = ALARM0_OFFSET + (index as u32) * 4;
            if let Some(alarm) = self.regs.read(offset) {
                // 复位值 0 表示该闹钟未被设置。A reset value of 0 means unset.
                if alarm != 0 && u64::from(alarm) <= now {
                    fired |= 1 << index;
                }
            }
        }
        fired
    }
}

impl MmioDevice for Timer0 {
    fn name(&self) -> &str {
        "TIMER0"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::TIMER0_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        let now = self.now_us();
        match offset {
            // TIMEHR / TIMELR: latched read of the free-running counter.
            // TIMEHR / TIMELR：自由运行计数器的锁存读取。
            0x08 | 0x24 => Some((now >> 32) as u32),
            0x0C | 0x28 => Some(now as u32),
            // ARMED: bit N set when alarm N is still ahead of the counter.
            // ARMED：当闹钟 N 尚未到期时置位对应位。
            0x20 => {
                let mut armed = 0;
                for index in 0..ALARMS {
                    let offset = ALARM0_OFFSET + (index as u32) * 4;
                    if let Some(alarm) = self.regs.read(offset) {
                        if alarm != 0 && u64::from(alarm) > now {
                            armed |= 1 << index;
                        }
                    }
                }
                Some(armed)
            }
            // TIMERAWH / TIMERAWL share the unlatched counter values above.
            // TIMERAWH / TIMERAWL 与非锁存计数器数值使用上面的同一分支。
            // INTR: fired and not acknowledged. INTR：已到期且未确认。
            0x38 => Some(self.fired() & !self.cleared),
            // INTS: fired, unacknowledged and enabled. INTS：已到期、未确认且已使能。
            0x44 => {
                let inte = self.regs.read(0x3C).unwrap_or(0);
                Some(self.fired() & !self.cleared & inte)
            }
            _ => self.regs.read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        match offset {
            // TIMEHW / TIMELW are write-only in hardware. TIMEHW / TIMELW 在硬件中只写。
            0x00 | 0x04 => true,
            // INTF is write-one-to-clear. INTF 为写 1 清零。
            0x40 => {
                self.cleared |= value;
                true
            }
            _ => self.regs.write(offset, value),
        }
    }

    fn reset(&mut self) {
        self.regs.reset();
        self.cleared = 0;
    }

    fn irq_number(&self) -> Option<u16> {
        Some(map::TIMER0_IRQ_0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timer() -> (Timer0, Arc<Mutex<Timebase>>) {
        let timebase = Arc::new(Mutex::new(Timebase::new(150)));
        (Timer0::new(Arc::clone(&timebase)), timebase)
    }

    #[test]
    fn counter_advances_with_timebase() {
        let (mut timer, timebase) = timer();
        timebase.lock().advance(150 * 1_000);
        assert_eq!(timer.read32(0x28), Some(1_000));
    }

    #[test]
    fn alarm_fires_and_clears() {
        let (mut timer, timebase) = timer();
        assert!(timer.write32(0x10, 10));
        assert_eq!(timer.read32(0x20), Some(1));
        timebase.lock().advance(150 * 20);
        assert_eq!(timer.read32(0x38), Some(1));
        assert!(timer.write32(0x40, 0xF));
        assert_eq!(timer.read32(0x38), Some(0));
    }
}
