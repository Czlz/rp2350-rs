//! TIMER0 / TIMER1 peripheral model / TIMER0 / TIMER1 外设模型
//!
//! Both RP2350 timer blocks share one 64-bit microsecond counter driven by the
//! shared [`Timebase`], which the simulation driver advances with executed
//! cycles. Each block owns its own `ALARM0..3`, `ARMED`, `INTR`, `INTE`, `INTF`
//! and `INTS` registers and its own interrupt lines. `INTR` is write-one-to-clear
//! raw status, `INTF` is a plain read/write force register (pico-sdk's alarm pool
//! pokes it with `hw_set_bits`), and `INTS = (INTR | INTF) & INTE` drives the
//! four alarm lines into the shared [`IrqAssert`]. An alarm that reaches its
//! deadline latches its `INTR` bit and clears its `ARMED` bit, exactly like the
//! hardware; it stays latched until firmware writes a one to `INTR`.
//! 两个 RP2350 定时器块共享一个由共享 [`Timebase`] 驱动的 64 位微秒计数器，
//! 仿真驱动按已执行周期推进它。每个块拥有各自的 `ALARM0..3`、`ARMED`、`INTR`、
//! `INTE`、`INTF`、`INTS` 寄存器与中断线。`INTR` 是写 1 清零的原始状态，`INTF`
//! 是普通读写的强制位（pico-sdk 的 alarm pool 用 `hw_set_bits` 置位），
//! `INTS = (INTR | INTF) & INTE` 驱动四条 alarm 线进入共享 [`IrqAssert`]。到期
//! 的 alarm 会锁存自己的 `INTR` 位并清掉 `ARMED` 位，与硬件一致；在固件向
//! `INTR` 写 1 之前它保持锁存。
//!
//! TIMER0 is mounted at [`map::TIMER0_BASE`] and raises `TIMER0_IRQ_0..3`
//! (`0..=3`); TIMER1 is mounted at [`map::TIMER1_BASE`] and raises
//! `TIMER1_IRQ_0..3` (`4..=7`).
//! TIMER0 挂载在 [`map::TIMER0_BASE`]，产生 `TIMER0_IRQ_0..3`（`0..=3`）；
//! TIMER1 挂载在 [`map::TIMER1_BASE`]，产生 `TIMER1_IRQ_0..3`（`4..=7`）。

use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};
use rp2350_core::types::Address;

use super::boot::Timebase;
use super::irq::IrqAssert;
use super::map;

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

/// State shared by one timer block and its clones / 一个定时器块及其克隆共享的状态
#[derive(Debug, Clone)]
struct TimerBlock {
    /// Datasheet name (`TIMER0`/`TIMER1`) / 数据手册名称（`TIMER0`/`TIMER1`）
    name: &'static str,
    /// Register window base / 寄存器窗口基地址
    base: Address,
    /// First interrupt line (`TIMERx_IRQ_0`) / 首条中断线（`TIMERx_IRQ_0`）
    irq_0: u16,
    /// Shared 64-bit timebase / 共享的 64 位时基
    timebase: Arc<Mutex<Timebase>>,
    /// Flat register file (`ALARM0..3`, `INTE`, `INTF`, `PAUSE`, ...) / 扁平寄存器文件
    regs: Arc<Mutex<RegisterFile>>,
    /// Latched raw status bits / 锁存的原始状态位
    intr: Arc<Mutex<u32>>,
    /// Latched armed bits (cleared when an alarm fires) / 锁存的武装位（alarm 到期即清）
    armed: Arc<Mutex<u32>>,
    /// `LOCKED` value, kept apart from its 16-bit neighbour `PAUSE` / `LOCKED` 值，与 16 位邻居 `PAUSE` 分开保存
    locked: Arc<Mutex<u16>>,
    /// Shared interrupt lines / 共享中断线
    irq: IrqAssert,
}

impl TimerBlock {
    /// Build one block / 构建一个块
    fn new(
        name: &'static str,
        base: Address,
        irq_0: u16,
        timebase: Arc<Mutex<Timebase>>,
        irq: IrqAssert,
    ) -> Self {
        Self {
            name,
            base,
            irq_0,
            timebase,
            regs: Arc::new(Mutex::new(RegisterFile::new(map::DEVICE_WINDOW_SIZE))),
            intr: Arc::new(Mutex::new(0)),
            armed: Arc::new(Mutex::new(0)),
            locked: Arc::new(Mutex::new(0)),
            irq,
        }
    }

    /// Current 64-bit microsecond counter / 当前 64 位微秒计数器
    fn now_us(&self) -> u64 {
        self.timebase.lock().time_us()
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
    /// `INTR` to acknowledge it. / 到期的 alarm 置位 `INTR` 并清掉 `ARMED`，因此
    /// 每次武装只投递一次；固件需要向 `INTR` 写 1 来确认。
    fn sample(&self) {
        let now = self.now_low();
        let deadlines = {
            let regs = self.regs.lock();
            let mut deadlines = [0u32; ALARMS];
            for (index, slot) in deadlines.iter_mut().enumerate() {
                *slot = regs.read(ALARM0_OFFSET + (index as u32) * 4).unwrap_or(0);
            }
            deadlines
        };
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
    fn refresh(&self) {
        self.sample();
        let ints = self.ints();
        for index in 0..ALARMS {
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
            INTR => *self.intr.lock() &= !value,
            LOCKED => *self.locked.lock() = value as u16,
            ALARM0_OFFSET..=0x1F => {
                let _ = self.regs.lock().write(offset, value);
                let bit = 1u32 << ((offset - ALARM0_OFFSET) / 4);
                if value == 0 {
                    *self.armed.lock() &= !bit;
                } else {
                    *self.armed.lock() |= bit;
                }
            }
            // INTF is a plain read/write force register, not W1C. / INTF 是普通
            // 读写强制位寄存器，不是 W1C。
            _ => {
                let _ = self.regs.lock().write(offset, value);
            }
        }
        // Any register write can change the alarm set or the mask; refresh.
        // 任何寄存器写都可能改变闹钟集合或屏蔽，因此刷新中断线。
        self.refresh();
        true
    }

    /// Restore power-on defaults / 恢复上电默认值
    fn reset(&mut self) {
        self.regs.lock().reset();
        *self.intr.lock() = 0;
        *self.armed.lock() = 0;
        *self.locked.lock() = 0;
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
    ($ty:ty, $name:literal, $base:expr, $irq0:expr) => {
        impl $ty {
            /// Create the timer connected to `timebase` and `irq`
            /// 创建连接到 `timebase` 与 `irq` 的定时器
            #[must_use]
            pub fn new(timebase: Arc<Mutex<Timebase>>, irq: IrqAssert) -> Self {
                Self {
                    inner: TimerBlock::new($name, $base, $irq0, timebase, irq),
                }
            }

            /// Recompute the asserted alarm lines / 重新计算断言的 alarm 线
            pub fn refresh(&self) {
                self.inner.refresh();
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

timer_instance!(Timer0, "TIMER0", map::TIMER0_BASE, map::TIMER0_IRQ_0);
timer_instance!(Timer1, "TIMER1", map::TIMER1_BASE, map::TIMER1_IRQ_0);
