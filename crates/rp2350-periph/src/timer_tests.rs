//! TIMER0/TIMER1 model tests / TIMER0/TIMER1 模型测试

use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_core::mmio::MmioDevice;

use crate::boot::Timebase;
use crate::irq::IrqAssert;
use crate::map;
use crate::timer::{
    ALARM0_OFFSET, ARMED, INTE, INTF, INTR, INTS, TIMEHR, TIMELR, TIMERAWL, Timer0, Timer1,
};

/// Offset of `ALARM0` / `ALARM0` 偏移
const ALARM0: u32 = ALARM0_OFFSET;

/// One clock cycle per microsecond / 每微秒 150 个周期
const CYCLES_PER_US: u32 = 150;

/// Bootrom `clk_sys` / bootrom `clk_sys`
const SYSCLK_HZ: u32 = 150_000_000;

/// Bootrom `clk_ref` / bootrom `clk_ref`
const REFCLK_HZ: u32 = 12_000_000;

/// Both timer instances over one timebase and interrupt set
/// 共享一个时基与中断集合的两个定时器实例
struct Rig {
    /// TIMER0 instance / TIMER0 实例
    timer: Timer0,
    /// TIMER1 instance / TIMER1 实例
    timer1: Timer1,
    /// Shared timebase / 共享时基
    timebase: Arc<Mutex<Timebase>>,
    /// Shared interrupt lines / 共享中断线
    irq: IrqAssert,
}

impl Rig {
    /// Build the rig / 构建测试装置
    fn new() -> Self {
        let timebase = Arc::new(Mutex::new(Timebase::new(SYSCLK_HZ, REFCLK_HZ)));
        let irq = IrqAssert::new();
        Self {
            timer: Timer0::new(Arc::clone(&timebase), irq.clone()),
            timer1: Timer1::new(Arc::clone(&timebase), irq.clone()),
            timebase,
            irq,
        }
    }

    /// Advance simulated time by `micros` / 推进 `micros` 微秒模拟时间
    fn advance_us(&self, micros: u64) {
        self.timebase
            .lock()
            .advance(u64::from(CYCLES_PER_US) * micros, SYSCLK_HZ, REFCLK_HZ, 1);
    }
}

#[test]
fn register_offsets_match_the_datasheet() {
    // 数据手册 TIMER 块：INTR 0x3C、INTE 0x40、INTF 0x44、INTS 0x48。
    // Datasheet TIMER block: INTR 0x3C, INTE 0x40, INTF 0x44, INTS 0x48.
    use crate::timer::{DBGPAUSE, LOCKED, PAUSE, SOURCE, TIMEHW, TIMELW, TIMERAWH};
    assert_eq!((TIMEHW, TIMELW, TIMEHR, TIMELR), (0x00, 0x04, 0x08, 0x0C));
    assert_eq!(
        (ALARM0, ARMED, TIMERAWH, TIMERAWL),
        (0x10, 0x20, 0x24, 0x28)
    );
    assert_eq!((DBGPAUSE, PAUSE, LOCKED, SOURCE), (0x30, 0x34, 0x36, 0x38));
    assert_eq!((INTR, INTE, INTF, INTS), (0x3C, 0x40, 0x44, 0x48));
}

#[test]
fn counter_advances_with_timebase() {
    let mut rig = Rig::new();
    rig.advance_us(1_000);
    assert_eq!(rig.timer.read32(TIMERAWL), Some(1_000));
    assert_eq!(rig.timer.read32(TIMELR), Some(1_000));
}

#[test]
fn timer1_shares_the_microsecond_timebase() {
    // 两个块共享同一个 64 位计数器，只是寄存器/中断独立。
    // Both blocks share one 64-bit counter; only registers/interrupts differ.
    let mut rig = Rig::new();
    rig.advance_us(123_456);
    assert_eq!(rig.timer.read32(TIMERAWL), rig.timer1.read32(TIMERAWL));
    assert_eq!(rig.timer.read32(TIMEHR), rig.timer1.read32(TIMEHR));
    assert_eq!(rig.timer.read32(TIMERAWL), Some(123_456));
}

#[test]
fn irq_numbers_map_to_0_3_and_4_7() {
    let rig = Rig::new();
    assert_eq!(rig.timer.irq_number(), Some(map::TIMER0_IRQ_0));
    assert_eq!(rig.timer1.irq_number(), Some(map::TIMER1_IRQ_0));
    assert_eq!(
        [
            map::TIMER0_IRQ_0,
            map::TIMER0_IRQ_0 + 1,
            map::TIMER0_IRQ_0 + 2,
            map::TIMER0_IRQ_0 + 3,
        ],
        [0, 1, 2, 3]
    );
    assert_eq!(
        [
            map::TIMER1_IRQ_0,
            map::TIMER1_IRQ_1,
            map::TIMER1_IRQ_2,
            map::TIMER1_IRQ_3,
        ],
        [4, 5, 6, 7]
    );
}

#[test]
fn a_future_alarm_reads_armed_and_does_not_assert() {
    let mut rig = Rig::new();
    assert!(rig.timer.write32(ALARM0, 10));
    rig.timer.write32(INTE, 1);
    rig.timer.refresh();
    assert_eq!(rig.timer.read32(ARMED), Some(1));
    assert_eq!(rig.timer.read32(INTR), Some(0));
    assert_eq!(rig.timer.read32(INTS), Some(0));
    assert_eq!(rig.irq.mask(), 0);
}

#[test]
fn a_reaching_alarm_sets_intr_and_disarms_itself() {
    // 到期即置 INTR、清 ARMED（硬件自动解除武装）。
    // Reaching the deadline sets INTR and clears ARMED (hardware auto-disarm).
    let mut rig = Rig::new();
    rig.timer.write32(ALARM0, 10);
    rig.advance_us(20);
    assert_eq!(rig.timer.read32(INTR), Some(1));
    assert_eq!(rig.timer.read32(ARMED), Some(0));
    assert_eq!(rig.timer.armed(), 0);
    assert_eq!(rig.timer.raw_status(), 1);
    // 保持锁存，直到固件向 INTR 写 1。 / Latched until firmware writes 1 to INTR.
    rig.advance_us(1_000);
    assert_eq!(rig.timer.read32(INTR), Some(1));
}

#[test]
fn intr_is_write_one_to_clear_and_does_not_refire() {
    let mut rig = Rig::new();
    rig.timer.write32(ALARM0, 10);
    rig.advance_us(20);
    assert_eq!(rig.timer.read32(INTR), Some(1));
    assert!(rig.timer.write32(INTR, 1));
    assert_eq!(rig.timer.read32(INTR), Some(0));
    // 已解除武装的 alarm 不会再自触发。 / A disarmed alarm does not re-fire.
    rig.advance_us(1_000);
    assert_eq!(rig.timer.read32(INTR), Some(0));
}

#[test]
fn enabled_alarm_asserts_its_line_until_acknowledged() {
    let mut rig = Rig::new();
    rig.timer.write32(ALARM0, 10);
    rig.timer.write32(INTE, 1);
    rig.advance_us(20);
    rig.timer.refresh();
    assert_eq!(rig.irq.mask(), 1 << map::TIMER0_IRQ_0);
    rig.timer.write32(INTR, 1);
    assert_eq!(rig.irq.mask(), 0);
}

#[test]
fn intf_forces_ints_and_the_irq_line() {
    // pico-sdk 的 alarm pool 用 hw_set_bits(&intf) 强制中断。
    // pico-sdk's alarm pool forces the IRQ with hw_set_bits(&intf).
    let mut rig = Rig::new();
    rig.timer.write32(INTE, 1 << 3);
    rig.timer.write32(INTF, 1 << 3);
    assert_eq!(rig.timer.read32(INTS), Some(1 << 3));
    assert_eq!(rig.irq.mask(), 1 << (map::TIMER0_IRQ_0 + 3));
    // 清掉强制位后释放。 / Clearing the force bit releases the line.
    rig.timer.write32(INTF, 0);
    assert_eq!(rig.timer.read32(INTS), Some(0));
    assert_eq!(rig.irq.mask(), 0);
}

#[test]
fn intf_is_a_plain_read_write_register_not_w1c() {
    let mut rig = Rig::new();
    rig.timer.write32(INTF, 0b0100);
    rig.timer.write32(INTF, 0b0001);
    // 普通写入覆盖旧值；W1C 会留下 0b0101。 / Plain write replaces; W1C would leave 0b0101.
    assert_eq!(rig.timer.read32(INTF), Some(0b0001));
    rig.timer.write32(INTF, 0);
    assert_eq!(rig.timer.read32(INTF), Some(0));
}

#[test]
fn intf_force_is_masked_by_inte() {
    let mut rig = Rig::new();
    rig.timer.write32(INTF, 0xF);
    assert_eq!(rig.timer.read32(INTS), Some(0));
    rig.timer.refresh();
    assert_eq!(rig.irq.mask(), 0);
    // 使能后强制位立即出现在 INTS 与中断线上。 / Enabling exposes the force bits.
    rig.timer.write32(INTE, 0b0010);
    assert_eq!(rig.timer.read32(INTS), Some(0b0010));
    assert_eq!(rig.irq.mask(), 1 << (map::TIMER0_IRQ_0 + 1));
}

#[test]
fn intr_and_intf_combine_and_separate() {
    let mut rig = Rig::new();
    rig.timer.write32(ALARM0, 10);
    rig.timer.write32(INTE, 1);
    rig.advance_us(20);
    assert_eq!(rig.timer.read32(INTR), Some(1));
    // 再叠加强制位：INTS 仍为 1。 / Add a forced bit: INTS stays 1.
    rig.timer.write32(INTF, 1);
    assert_eq!(rig.timer.read32(INTS), Some(1));
    // 清 INTR 后 INTF 仍强制，INTS 与中断线保持。 / W1C on INTR leaves the force.
    rig.timer.write32(INTR, 1);
    assert_eq!(rig.timer.read32(INTS), Some(1));
    assert_eq!(rig.irq.mask(), 1 << map::TIMER0_IRQ_0);
    // 清 INTF 后彻底释放。 / Clearing INTF releases everything.
    rig.timer.write32(INTF, 0);
    assert_eq!(rig.timer.read32(INTS), Some(0));
    assert_eq!(rig.irq.mask(), 0);
}

#[test]
fn ints_reads_the_0x48_offset_with_the_masked_value() {
    let mut rig = Rig::new();
    rig.timer.write32(INTE, 0b1010);
    rig.timer.write32(INTF, 0b0011);
    // INTS = (INTR | INTF) & INTE = 0b0010。
    // INTS = (INTR | INTF) & INTE = 0b0010.
    assert_eq!(INTS, 0x48);
    assert_eq!(rig.timer.read32(0x48), Some(0b0010));
}

#[test]
fn timer1_alarms_and_lines_are_independent_of_timer0() {
    let mut rig = Rig::new();
    rig.timer1.write32(ALARM0, 10);
    rig.timer1.write32(INTE, 1);
    rig.advance_us(20);
    rig.timer1.refresh();
    assert_eq!(rig.timer1.read32(INTR), Some(1));
    assert_eq!(rig.timer.read32(INTR), Some(0), "TIMER0 untouched");
    assert_eq!(rig.irq.mask(), 1 << map::TIMER1_IRQ_0);
    // TIMER0 的线 0..3 全部未断言。 / TIMER0's lines 0..3 stay clear.
    assert_eq!(rig.irq.mask() & 0xF, 0);
    rig.timer1.write32(INTR, 1);
    assert_eq!(rig.irq.mask(), 0);
}

#[test]
fn forcing_timer0_does_not_force_timer1() {
    let mut rig = Rig::new();
    rig.timer.write32(INTE, 0xF);
    rig.timer1.write32(INTE, 0xF);
    rig.timer.write32(INTF, 0b0001);
    assert_eq!(rig.timer.read32(INTS), Some(0b0001));
    assert_eq!(rig.timer1.read32(INTS), Some(0));
    assert_eq!(rig.irq.mask(), 1 << map::TIMER0_IRQ_0);
}

#[test]
fn writing_zero_to_an_alarm_disarms_it() {
    let mut rig = Rig::new();
    rig.timer.write32(ALARM0, 10);
    assert_eq!(rig.timer.read32(ARMED), Some(1));
    rig.timer.write32(ALARM0, 0);
    assert_eq!(rig.timer.read32(ARMED), Some(0));
    rig.advance_us(50);
    assert_eq!(rig.timer.read32(INTR), Some(0));
}

#[test]
fn reset_clears_alarms_status_and_lines() {
    let mut rig = Rig::new();
    rig.timer.write32(ALARM0, 10);
    rig.timer.write32(INTE, 1);
    rig.timer.write32(INTF, 0xF);
    assert_ne!(rig.irq.mask(), 0);
    rig.timer.reset();
    assert_eq!(rig.timer.read32(ARMED), Some(0));
    assert_eq!(rig.timer.read32(INTF), Some(0));
    assert_eq!(rig.timer.read32(INTS), Some(0));
    assert_eq!(rig.irq.mask(), 0);
}

#[test]
fn clones_share_state() {
    let Rig {
        timer, timebase, ..
    } = Rig::new();
    let mut peer = timer.clone();
    let mut writer = timer;
    assert!(writer.write32(ALARM0, 5));
    timebase
        .lock()
        .advance(u64::from(CYCLES_PER_US) * 6, SYSCLK_HZ, REFCLK_HZ, 2);
    peer.refresh();
    assert_eq!(peer.read32(INTR), Some(1));
}

#[test]
fn the_timer_alarm_hits_the_same_microsecond_at_150_and_252_mhz() {
    // TIMER 的 µs 计数器由 clk_ref 驱动：同样的 1 000 µs 截止时间在 150 MHz 与
    // 252 MHz 下都在 TIMELR == 1 000 时命中。
    // The TIMER microsecond counter is clocked from clk_ref: the same 1000 µs
    // deadline is reached at TIMELR == 1000 under both 150 and 252 MHz.
    for (sysclk, cycles_per_us) in [(150_000_000u32, 150u64), (252_000_000, 252)] {
        let timebase = Arc::new(Mutex::new(Timebase::new(sysclk, REFCLK_HZ)));
        let mut timer = Timer0::new(Arc::clone(&timebase), IrqAssert::new());
        assert!(timer.write32(ALARM0, 1_000));
        // 999 µs 不命中。 / 999 µs does not fire.
        timebase
            .lock()
            .advance(cycles_per_us * 999, sysclk, REFCLK_HZ, 1);
        assert_eq!(timer.read32(TIMERAWL), Some(999));
        assert_eq!(timer.read32(INTR), Some(0));
        // 第 1 000 µs 命中。 / The 1000th microsecond fires.
        timebase.lock().advance(cycles_per_us, sysclk, REFCLK_HZ, 1);
        assert_eq!(timer.read32(TIMERAWL), Some(1_000));
        assert_eq!(timer.read32(INTR), Some(1), "{sysclk} Hz");
    }
}
