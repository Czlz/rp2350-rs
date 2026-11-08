//! TICKS tick generator / TICKS 滴答发生器
//!
//! RP2350 exposes six tick generators (`PROC0`, `PROC1`, `TIMER0`, `TIMER1`,
//! `USB`, `ADC`) at `0x4010_8000`, each a `0x0C`-byte record: `CTRL` (+0x00),
//! `CYCLES` (+0x04) and `COUNT` (+0x08). The firmware programs `CYCLES` with the
//! period in processor cycles and sets `CTRL.ENABLE`; the counter then reloads
//! every period and the generated tick feeds blocks such as TIMER0.
//! RP2350 在 `0x4010_8000` 暴露六个滴答发生器（`PROC0`、`PROC1`、`TIMER0`、
//! `TIMER1`、`USB`、`ADC`），每个是 `0x0C` 字节的记录：`CTRL`（+0x00）、
//! `CYCLES`（+0x04）、`COUNT`（+0x08）。固件把以处理器周期为单位的周期写入
//! `CYCLES` 并置位 `CTRL.ENABLE`，计数器随后按周期重装，产生的滴答供 TIMER0
//! 等块使用。
//!
//! # Timing model / 时间模型
//!
//! Writing `CYCLES` while the channel is enabled pushes the whole period into the
//! shared [`CycleSink`], i.e. the simulated clock advances by the programmed
//! delay immediately instead of being burned by a busy loop. `COUNT` therefore
//! reads zero right after the write: the period has already elapsed in simulated
//! time. Writes to `CYCLES` while the channel is disabled are ignored, matching
//! the datasheet rule that the period only takes effect once `ENABLE` is set.
//! 通道使能时写 `CYCLES` 会把整个周期推入共享 [`CycleSink`]，即仿真时钟立即
//! 前进编程的延时量，而不是由忙等循环空转消耗。因此 `COUNT` 在写入后立即读到
//! 零：该周期在仿真时间里已经过去。通道未使能时对 `CYCLES` 的写入被忽略，
//! 与数据手册"周期只有在 `ENABLE` 置位后才生效"的规则一致。

use rp2350_core::mmio::{MmioDevice, MmioWindow};

use super::cycle::CycleSink;
use super::map;

/// Number of tick generator channels / 滴答发生器通道数量
const CHANNELS: u32 = 6;

/// Stride between channel records / 通道记录之间的步长
const CHANNEL_STRIDE: u32 = 0x0C;

/// `CTRL` offset inside a channel / 通道内 `CTRL` 的偏移
const CTRL: u32 = 0x00;

/// `CYCLES` offset inside a channel / 通道内 `CYCLES` 的偏移
const CYCLES: u32 = 0x04;

/// `COUNT` offset inside a channel / 通道内 `COUNT` 的偏移
const COUNT: u32 = 0x08;

/// `CTRL.ENABLE` mask / `CTRL.ENABLE` 掩码
const CTRL_ENABLE: u32 = 1 << 0;

/// `CTRL.RUNNING` mask (read-only) / `CTRL.RUNNING` 掩码（只读）
const CTRL_RUNNING: u32 = 1 << 1;

/// Width of the `CYCLES`/`COUNT` fields (bits [8:0]) / `CYCLES`/`COUNT` 字段宽度
const PERIOD_MASK: u32 = 0x1FF;

/// One tick generator channel / 单个滴答发生器通道
#[derive(Debug, Default, Clone, Copy)]
struct Channel {
    /// Latched period in processor cycles / 锁存的周期（处理器周期）
    cycles: u32,
    /// `CTRL.ENABLE` state / `CTRL.ENABLE` 状态
    enabled: bool,
}

/// TICKS register block / TICKS 寄存器块
#[derive(Debug)]
pub struct Ticks {
    sink: CycleSink,
    channels: [Channel; CHANNELS as usize],
}

impl Ticks {
    /// Create the block connected to the shared cycle sink
    /// 创建连接到共享周期汇的寄存器块
    #[must_use]
    pub fn new(sink: CycleSink) -> Self {
        Self {
            sink,
            channels: [Channel::default(); CHANNELS as usize],
        }
    }

    /// Channel owning `offset`, rejected outside the modelled records
    /// 拥有 `offset` 的通道；超出已建模的记录范围时返回 `None`
    fn channel(offset: u32) -> Option<usize> {
        let index = offset / CHANNEL_STRIDE;
        (index < CHANNELS).then_some(index as usize)
    }
}

impl MmioDevice for Ticks {
    fn name(&self) -> &str {
        "TICKS"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::TICKS_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        let index = Self::channel(offset)?;
        let channel = self.channels[index];
        Some(match offset % CHANNEL_STRIDE {
            CTRL => {
                let enable = u32::from(channel.enabled);
                enable | if channel.enabled { CTRL_RUNNING } else { 0 }
            }
            CYCLES => channel.cycles,
            // 周期已在写入时整体计入时基，剩余量因此为零。
            // The period was credited to the time base at write time, so none is left.
            COUNT => 0,
            _ => return None,
        })
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        let Some(index) = Self::channel(offset) else {
            return false;
        };
        let channel = &mut self.channels[index];
        match offset % CHANNEL_STRIDE {
            CTRL => channel.enabled = value & CTRL_ENABLE != 0,
            CYCLES => {
                // ENABLE 未置位时写入不生效。Ignored while ENABLE is clear.
                if channel.enabled {
                    let period = value & PERIOD_MASK;
                    channel.cycles = period;
                    self.sink.push(u64::from(period));
                }
            }
            // COUNT 只读。COUNT is read-only.
            COUNT => {}
            _ => return false,
        }
        true
    }

    fn reset(&mut self) {
        self.channels = [Channel::default(); CHANNELS as usize];
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use parking_lot::Mutex;

    use super::*;
    use crate::boot::Timebase;

    /// Offset of a channel register / 通道寄存器的偏移
    fn reg(channel: u32, reg: u32) -> u32 {
        channel * CHANNEL_STRIDE + reg
    }

    fn ticks() -> (Ticks, CycleSink) {
        let sink = CycleSink::new(Arc::new(Mutex::new(Timebase::new(150))));
        (Ticks::new(sink.clone()), sink)
    }

    #[test]
    fn cycles_write_while_disabled_is_ignored() {
        let (mut ticks, sink) = ticks();
        assert!(ticks.write32(reg(0, CYCLES), 12));
        assert_eq!(sink.cycles(), 0);
        assert_eq!(ticks.read32(reg(0, CYCLES)), Some(0));
        assert_eq!(ticks.read32(reg(0, COUNT)), Some(0));
    }

    #[test]
    fn enabled_cycles_write_advances_the_shared_timebase() {
        let (mut ticks, sink) = ticks();
        assert!(ticks.write32(reg(0, CTRL), CTRL_ENABLE));
        assert!(ticks.write32(reg(0, CYCLES), 12));
        assert_eq!(sink.cycles(), 12);
        assert_eq!(ticks.read32(reg(0, CYCLES)), Some(12));
        // 周期已整体计入时基，剩余计数为零。The period was fully credited already.
        assert_eq!(ticks.read32(reg(0, COUNT)), Some(0));
    }

    #[test]
    fn ctrl_reports_enable_and_running() {
        let (mut ticks, _sink) = ticks();
        assert!(ticks.write32(reg(0, CTRL), 0xFFFF_FFFF));
        assert_eq!(ticks.read32(reg(0, CTRL)), Some(CTRL_ENABLE | CTRL_RUNNING));
        assert!(ticks.write32(reg(0, CTRL), 0));
        assert_eq!(ticks.read32(reg(0, CTRL)), Some(0));
    }

    #[test]
    fn channel_period_is_masked_to_nine_bits() {
        let (mut ticks, sink) = ticks();
        assert!(ticks.write32(reg(0, CTRL), CTRL_ENABLE));
        assert!(ticks.write32(reg(0, CYCLES), 0xFFFF_FFFF));
        assert_eq!(ticks.read32(reg(0, CYCLES)), Some(PERIOD_MASK));
        assert_eq!(sink.cycles(), u64::from(PERIOD_MASK));
    }

    #[test]
    fn count_is_read_only() {
        let (mut ticks, sink) = ticks();
        assert!(ticks.write32(reg(0, CTRL), CTRL_ENABLE));
        assert!(ticks.write32(reg(0, CYCLES), 5));
        assert!(ticks.write32(reg(0, COUNT), 0x1FF));
        assert_eq!(ticks.read32(reg(0, COUNT)), Some(0));
        assert_eq!(sink.cycles(), 5);
    }

    #[test]
    fn channels_are_independent() {
        let (mut ticks, sink) = ticks();
        // 只使能 TIMER0 通道（索引 2）。Enable only the TIMER0 channel (index 2).
        assert!(ticks.write32(reg(2, CTRL), CTRL_ENABLE));
        assert!(ticks.write32(reg(2, CYCLES), 12));
        // 未使能通道的写入不推进时间。A disabled channel does not advance time.
        assert!(ticks.write32(reg(1, CYCLES), 99));
        assert_eq!(sink.cycles(), 12);
        assert_eq!(ticks.read32(reg(1, CYCLES)), Some(0));
    }

    #[test]
    fn offsets_past_the_last_channel_are_unmodelled() {
        let (mut ticks, _sink) = ticks();
        let beyond = CHANNELS * CHANNEL_STRIDE;
        assert_eq!(ticks.read32(beyond), None);
        assert!(!ticks.write32(beyond, 1));
        // 通道内未使用的偏移同样未建模。Unused offsets inside a record are unmodelled too.
        assert_eq!(ticks.read32(reg(0, 0x0A)), None);
    }

    #[test]
    fn reset_clears_channels() {
        let (mut ticks, _sink) = ticks();
        assert!(ticks.write32(reg(0, CTRL), CTRL_ENABLE));
        assert!(ticks.write32(reg(0, CYCLES), 12));
        ticks.reset();
        assert_eq!(ticks.read32(reg(0, CTRL)), Some(0));
        assert_eq!(ticks.read32(reg(0, CYCLES)), Some(0));
    }
}
