//! TIMER0/TIMER1 model tests / TIMER0/TIMER1 模型测试

use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use parking_lot::Mutex;
use rp2350_core::mmio::MmioDevice;

use crate::boot::Timebase;
use crate::irq::IrqAssert;
use crate::map;
use crate::timer::{
    ALARM0_OFFSET, ARMED, INTE, INTF, INTR, INTS, TIMEHR, TIMELR, TIMERAWH, TIMERAWL, Timer0,
    Timer1,
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

    /// Advance simulated time by executed `clk_sys` cycles
    /// 按已执行的 `clk_sys` 周期推进模拟时间
    fn advance_cycles(&self, cycles: u64) {
        self.timebase
            .lock()
            .advance(cycles, SYSCLK_HZ, REFCLK_HZ, 1);
    }

    /// Authoritative microsecond count (locked path) / 权威微秒数（加锁路径）
    fn now(&self) -> u64 {
        self.timebase.lock().timer_us()
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

#[test]
fn the_snapshot_matches_locked_reads_at_random_instants() {
    // 单线程随机推进：快照路径与设备锁路径（`Timebase::timer_us`）必须逐位一致，
    // 64 位两半与各别名同样一致。
    // Random single-threaded advances: the snapshot path and the locked path
    // (`Timebase::timer_us`) must agree bit for bit, including both halves and
    // every alias.
    let mut rig = Rig::new();
    let mut state = 0x1234_5678u32;
    for _ in 0..256 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        rig.advance_cycles(u64::from(state % 5_000));
        let locked = rig.now();
        assert_eq!(rig.timer.snapshot().now_us(), locked);
        assert_eq!(
            u64::from(rig.timer.read32(TIMELR).unwrap()),
            locked & 0xFFFF_FFFF
        );
        assert_eq!(
            u64::from(rig.timer.read32(TIMERAWL).unwrap()),
            locked & 0xFFFF_FFFF
        );
        assert_eq!(u64::from(rig.timer.read32(TIMEHR).unwrap()), locked >> 32);
        assert_eq!(
            u64::from(rig.timer1.read32(TIMERAWH).unwrap()),
            locked >> 32
        );
    }
}

#[test]
fn the_snapshot_follows_batched_and_ticks_driven_boundaries() {
    // 驱动器的 ADVANCE_INTERVAL 边界（10 000 步一次）与 TICKS 的 `CYCLES` 推入
    // 都必须刷新快照；MT-3a 的睡眠快进也在同一批边界结算。
    // The driver's ADVANCE_INTERVAL boundary (one advance per 10 000 steps) and
    // TICKS `CYCLES` pushes must both refresh the snapshot; MT-3a's sleep
    // fast-forward settles at the same batched boundary.
    let mut rig = Rig::new();
    for boundary in 0..64u32 {
        rig.advance_cycles(10_000);
        if boundary % 16 == 15 {
            rig.timebase.lock().push_ref_cycles(12);
        }
        rig.timer.refresh();
        rig.timer1.refresh();
        let locked = rig.now();
        assert_eq!(rig.timer.snapshot().now_us(), locked, "boundary {boundary}");
        assert_eq!(
            u64::from(rig.timer.read32(TIMERAWL).unwrap()),
            locked & 0xFFFF_FFFF
        );
        assert_eq!(u64::from(rig.timer.read32(TIMEHR).unwrap()), locked >> 32);
    }
    assert!(rig.now() > 4_000, "64 boundaries are more than 4 ms");
}

#[test]
fn timerawh_aliases_timehr_and_timerawl_aliases_timelr() {
    // 别名来自同一次快照读；高低半字必须属于同一个 64 位值。
    // The aliases come from the same snapshot read; the two halves must belong
    // to one 64-bit value.
    let mut rig = Rig::new();
    rig.advance_us(0x1_0000_0005);
    assert_eq!(rig.timer.read32(TIMERAWL), rig.timer.read32(TIMELR));
    assert_eq!(rig.timer.read32(TIMERAWH), rig.timer.read32(TIMEHR));
    assert_eq!(rig.timer.read32(TIMELR), Some(5));
    assert_eq!(rig.timer.read32(TIMEHR), Some(1));
    // TIMER1 共享同一计数器，因此别名值相同。
    // TIMER1 shares the counter, so its aliases read the same value.
    assert_eq!(rig.timer1.read32(TIMERAWL), Some(5));
    assert_eq!(rig.timer1.read32(TIMERAWH), Some(1));
}

#[test]
fn snapshot_alarm_hits_follow_arming_firing_and_acknowledgement() {
    let mut rig = Rig::new();
    // 武装但未到期：没有命中位。Armed but not due: no hit bit.
    assert!(rig.timer.write32(ALARM0, 10));
    assert_eq!(rig.timer.alarm_hits(), 0);
    rig.advance_us(20);
    rig.timer.refresh();
    assert_eq!(rig.timer.alarm_hits(), 1);
    assert_eq!(rig.timer.snapshot().timer0_alarm_hits(), 1);
    // 写 1 到 INTR 确认后快照同步清除。Acknowledging with a 1 clears the snapshot.
    assert!(rig.timer.write32(INTR, 1));
    assert_eq!(rig.timer.alarm_hits(), 0);
    // 重新武装到未来再解除武装：命中位保持为零。
    // Re-arming into the future and disarming keeps the hit bits clear.
    assert!(rig.timer.write32(ALARM0, 1_000));
    assert_eq!(rig.timer.alarm_hits(), 0);
    assert!(rig.timer.write32(ALARM0, 0));
    assert_eq!(rig.timer.alarm_hits(), 0);
    assert_eq!(rig.timer.read32(ARMED), Some(0));
}

#[test]
fn timer1_shares_the_counter_but_keeps_its_own_alarm_hits() {
    let mut rig = Rig::new();
    assert!(rig.timer1.write32(ALARM0, 10));
    rig.advance_us(20);
    rig.timer.refresh();
    rig.timer1.refresh();
    assert_eq!(rig.timer.alarm_hits(), 0, "TIMER0 has no armed alarm");
    assert_eq!(rig.timer1.alarm_hits(), 1);
    assert_eq!(rig.timer.read32(INTR), Some(0));
    assert_eq!(rig.timer1.read32(INTR), Some(1));
    assert_eq!(
        rig.timer.snapshot().now_us(),
        rig.timer1.snapshot().now_us()
    );
    assert_eq!(
        rig.timer.snapshot().now_us(),
        rig.timebase.lock().timer_us()
    );
}

#[test]
fn the_refresh_gate_samples_only_when_a_deadline_is_due() {
    // 中断泵在 64 步节拍上运行；没有到期截止时间时不得进入慢路径（不加块锁），
    // 到期后恰好采样一次，触发时刻与旧实现相同。
    // The interrupt pump runs on the 64-step tick; it must not enter the slow
    // path while no deadline is due, and samples exactly once when one is.
    let mut rig = Rig::new();
    assert!(rig.timer.write32(ALARM0, 1_000_000));
    let before = rig.timer.sample_count();
    let before1 = rig.timer1.sample_count();
    for _ in 0..1_000 {
        rig.timer.refresh();
        rig.timer1.refresh();
    }
    assert_eq!(rig.timer.sample_count(), before, "gate skips the slow path");
    assert_eq!(rig.timer1.sample_count(), before1);
    rig.advance_us(999_999);
    rig.timer.refresh();
    assert_eq!(
        rig.timer.sample_count(),
        before,
        "still one microsecond early"
    );
    assert_eq!(rig.timer.alarm_hits(), 0);
    rig.advance_us(1);
    rig.timer.refresh();
    assert_eq!(rig.timer.sample_count(), before + 1);
    assert_eq!(rig.timer.alarm_hits(), 1);
    // 已触发并解除武装，后续刷新不再采样。
    // Fired and disarmed: later refreshes sample no more.
    rig.advance_us(1_000);
    rig.timer.refresh();
    assert_eq!(rig.timer.sample_count(), before + 1);
    assert_eq!(rig.timer1.sample_count(), before1);
}

#[test]
fn an_alarm_armed_in_the_past_fires_on_the_write() {
    // 写入一个已经过去的截止时间必须立即命中（与旧实现相同），门槛不得吞掉它。
    // Arming a deadline that already passed must fire immediately (as before);
    // the gate must not swallow it.
    let mut rig = Rig::new();
    rig.advance_us(100);
    assert!(rig.timer.write32(INTE, 1));
    assert!(rig.timer.write32(ALARM0, 50));
    assert_eq!(rig.timer.alarm_hits(), 1);
    assert_eq!(rig.timer.read32(INTR), Some(1));
    assert_eq!(rig.timer.armed(), 0);
    assert_eq!(rig.irq.mask(), 1 << map::TIMER0_IRQ_0);
}

#[test]
fn reset_clears_the_snapshot_alarm_state() {
    let mut rig = Rig::new();
    assert!(rig.timer.write32(ALARM0, 10));
    rig.advance_us(20);
    rig.timer.refresh();
    assert_eq!(rig.timer.alarm_hits(), 1);
    rig.timer.reset();
    assert_eq!(rig.timer.alarm_hits(), 0);
    assert_eq!(rig.timer.read32(INTR), Some(0));
    assert_eq!(rig.timer.read32(ARMED), Some(0));
    assert_eq!(rig.irq.mask(), 0);
    // 复位后时间仍在前进：快照与加锁路径保持一致。
    // Time still advances after reset: the snapshot keeps matching the locked path.
    rig.advance_us(5);
    assert_eq!(rig.timer.snapshot().now_us(), rig.now());
}

#[test]
fn the_sixty_four_bit_counter_wraps_through_the_high_word() {
    // 越过 2^32 µs：低字回绕为 0、高字变成 1，快照与四个别名一致。
    // Crossing 2^32 µs: the low word wraps to zero, the high word becomes one,
    // and the snapshot agrees with all four aliases.
    let mut rig = Rig::new();
    rig.advance_us(4_294_967_296);
    let locked = rig.now();
    assert_eq!(locked, 0x1_0000_0000);
    assert_eq!(rig.timer.snapshot().now_us(), locked);
    assert_eq!(rig.timer.read32(TIMELR), Some(0));
    assert_eq!(rig.timer.read32(TIMEHR), Some(1));
    assert_eq!(rig.timer.read32(TIMERAWL), Some(0));
    assert_eq!(rig.timer.read32(TIMERAWH), Some(1));
}

#[test]
fn concurrent_timer_reads_never_take_the_timebase_lock() {
    // 主线程持有时间基互斥锁（计数器来源的“设备锁”）时，两个线程仍必须能读取
    // 计数器：读取只做原子加载。超时即视为失败，避免回归时挂死。
    // With the timebase mutex (the counter source's device lock) held by the
    // main thread, two threads must still read the counter: reads are atomic
    // loads only. A timeout is reported as a failure instead of hanging.
    let rig = Rig::new();
    rig.advance_us(1_234_567);
    let frozen = rig.now();
    let guard = rig.timebase.lock();
    let (sender, receiver) = mpsc::channel();
    for _ in 0..2 {
        let mut timer = rig.timer.clone();
        let sender = sender.clone();
        thread::spawn(move || {
            let mut values = Vec::with_capacity(100_000);
            for _ in 0..100_000 {
                let low = u64::from(timer.read32(TIMERAWL).unwrap());
                let high = u64::from(timer.read32(TIMERAWH).unwrap());
                values.push((high << 32) | low);
            }
            sender.send(values).expect("main thread is listening");
        });
    }
    drop(sender);
    let outcome = receiver.recv_timeout(Duration::from_secs(10));
    drop(guard);
    let values = outcome.expect("counter reads must not wait for the timebase lock");
    assert_eq!(values.len(), 100_000);
    assert!(values.iter().all(|&value| value == frozen));
}
