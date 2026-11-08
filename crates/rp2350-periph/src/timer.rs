//! TIMER0 peripheral model / TIMER0 外设模型
//!
//! A 64-bit microsecond counter runs off the shared [`Timebase`], which the
//! simulation driver advances with executed cycles. Alarm registers, `ARMED`,
//! `INTR`, `INTE`, `INTF` and `INTS` are modelled, and the four alarm lines are
//! asserted into a shared [`IrqAssert`] so the driver can deliver them to the
//! NVIC. Alarm 0 uses [`map::TIMER0_IRQ_0`]; alarms 1..3 use the next lines.
//! 64 位微秒计数器由共享的 [`Timebase`] 驱动，仿真驱动按已执行周期推进它。
//! 闹钟寄存器、`ARMED`、`INTR`、`INTE`、`INTF` 与 `INTS` 均已建模，四个闹钟
//! 线断言到共享的 [`IrqAssert`]，由驱动送达 NVIC。闹钟 0 使用
//! [`map::TIMER0_IRQ_0`]，闹钟 1..3 使用其后的中断线。

use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::boot::Timebase;
use super::irq::IrqAssert;
use super::map;

/// Number of alarm channels / 闹钟通道数量
const ALARMS: usize = 4;

/// Offset of the first alarm register / 首个闹钟寄存器的偏移
const ALARM0_OFFSET: u32 = 0x10;

/// `INTR` offset / `INTR` 偏移
const INTR: u32 = 0x38;

/// `INTE` offset / `INTE` 偏移
const INTE: u32 = 0x3C;

/// `INTF` offset / `INTF` 偏移
const INTF: u32 = 0x40;

/// `INTS` offset / `INTS` 偏移
const INTS: u32 = 0x44;

/// TIMER0 register block / TIMER0 寄存器块
///
/// Clones share state; [`crate::BootPeripherals`] keeps a clone to recompute the
/// asserted lines whenever the timebase advances.
/// 克隆共享状态；[`crate::BootPeripherals`] 保留一个克隆，在时基推进时重新
/// 计算断言的中断线。
#[derive(Debug, Clone)]
pub struct Timer0 {
    timebase: Arc<Mutex<Timebase>>,
    regs: Arc<Mutex<RegisterFile>>,
    /// Alarms already acknowledged through `INTR`. 已通过 `INTR` 确认的闹钟。
    cleared: Arc<Mutex<u32>>,
    irq: IrqAssert,
}

impl Timer0 {
    /// Create the timer connected to `timebase` and `irq`
    /// 创建连接到 `timebase` 与 `irq` 的定时器
    #[must_use]
    pub fn new(timebase: Arc<Mutex<Timebase>>, irq: IrqAssert) -> Self {
        Self {
            timebase,
            regs: Arc::new(Mutex::new(RegisterFile::new(map::DEVICE_WINDOW_SIZE))),
            cleared: Arc::new(Mutex::new(0)),
            irq,
        }
    }

    /// Current 64-bit microsecond counter / 当前 64 位微秒计数器
    fn now_us(&self) -> u64 {
        self.timebase.lock().time_us()
    }

    /// Alarms whose deadline has passed / 已到期的闹钟
    fn fired(&self) -> u32 {
        let now = self.now_us();
        let regs = self.regs.lock();
        let mut fired = 0;
        for index in 0..ALARMS {
            let offset = ALARM0_OFFSET + (index as u32) * 4;
            if let Some(alarm) = regs.read(offset) {
                // 复位值 0 表示该闹钟未被设置。A reset value of 0 means unset.
                if alarm != 0 && u64::from(alarm) <= now {
                    fired |= 1 << index;
                }
            }
        }
        fired
    }

    /// `INTS`: fired, unacknowledged and enabled / `INTS`：到期、未确认且使能
    fn ints(&self) -> u32 {
        let inte = {
            let regs = self.regs.lock();
            regs.read(INTE).unwrap_or(0)
        };
        self.fired() & !*self.cleared.lock() & inte
    }

    /// Recompute the asserted IRQ lines from the current counter and registers
    /// 依据当前计数器与寄存器重新计算断言的中断线
    pub fn refresh(&self) {
        let ints = self.ints();
        for index in 0..ALARMS {
            let line = map::TIMER0_IRQ_0 + index as u16;
            self.irq.set(line, ints & (1 << index) != 0);
        }
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
                let regs = self.regs.lock();
                let mut armed = 0;
                for index in 0..ALARMS {
                    let offset = ALARM0_OFFSET + (index as u32) * 4;
                    if let Some(alarm) = regs.read(offset) {
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
            INTR => Some(self.fired() & !*self.cleared.lock()),
            // INTS: fired, unacknowledged and enabled. INTS：已到期、未确认且已使能。
            INTS => Some(self.ints()),
            _ => self.regs.lock().read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        match offset {
            // TIMEHW / TIMELW are write-only in hardware. TIMEHW / TIMELW 在硬件中只写。
            0x00 | 0x04 => return true,
            // INTF is write-one-to-clear. INTF 为写 1 清零。
            INTF => *self.cleared.lock() |= value,
            _ => {
                let _ = self.regs.lock().write(offset, value);
            }
        }
        // Any register write can change the alarm set; refresh the lines.
        // 任何寄存器写都可能改变闹钟集合，因此刷新中断线。
        self.refresh();
        true
    }

    fn reset(&mut self) {
        self.regs.lock().reset();
        *self.cleared.lock() = 0;
        self.refresh();
    }

    fn irq_number(&self) -> Option<u16> {
        Some(map::TIMER0_IRQ_0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timer() -> (Timer0, Arc<Mutex<Timebase>>, IrqAssert) {
        let timebase = Arc::new(Mutex::new(Timebase::new(150)));
        let irq = IrqAssert::new();
        (
            Timer0::new(Arc::clone(&timebase), irq.clone()),
            timebase,
            irq,
        )
    }

    #[test]
    fn counter_advances_with_timebase() {
        let (mut timer, timebase, _irq) = timer();
        timebase.lock().advance(150 * 1_000);
        assert_eq!(timer.read32(0x28), Some(1_000));
    }

    #[test]
    fn alarm_fires_and_clears() {
        let (mut timer, timebase, _irq) = timer();
        assert!(timer.write32(0x10, 10));
        assert_eq!(timer.read32(0x20), Some(1));
        timebase.lock().advance(150 * 20);
        assert_eq!(timer.read32(INTR), Some(1));
        assert!(timer.write32(INTF, 0xF));
        assert_eq!(timer.read32(INTR), Some(0));
    }

    #[test]
    fn enabled_alarm_asserts_shared_irq_line() {
        let (mut timer, timebase, irq) = timer();
        assert!(timer.write32(0x10, 10));
        assert!(timer.write32(INTE, 1));
        timer.refresh();
        assert_eq!(irq.mask(), 0);
        timebase.lock().advance(150 * 20);
        timer.refresh();
        assert_eq!(irq.mask(), 1 << map::TIMER0_IRQ_0);
        assert!(timer.write32(INTF, 1));
        assert_eq!(irq.mask(), 0);
    }

    #[test]
    fn clones_share_state() {
        let (timer, timebase, _irq) = timer();
        let mut peer = timer.clone();
        let mut writer = timer;
        assert!(writer.write32(0x10, 5));
        timebase.lock().advance(150 * 6);
        peer.refresh();
        assert_eq!(peer.read32(INTR), Some(1));
    }
}
