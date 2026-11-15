//! DMA pacing timers / DMA 节拍定时器
//!
//! `TIMER0..3` (`0x440..=0x44C`) are four (X/Y) fractional pacing timers. The
//! register layout follows the RP2350 header: `X` (the dividend) in
//! `bits[31:16]` and `Y` (the divisor) in `bits[15:0]`. Each timer produces
//! `DREQ_DMA_TIMERn` pulses at the rate `(X / Y) * sys_clk`: with `X == Y` every
//! DMA cycle yields one pulse, with `X = 1, Y = 4` one pulse every four cycles.
//! [`super::Dma::advance`] accumulates `X * cycles` into a fractional accumulator
//! and queues `accumulator / Y` pulses, keeping the remainder for the next call.
//! Each queued pulse is a single request: the first channel transfer that
//! samples the asserted `DREQ_DMA_TIMER0..3` (`59..=62`) selector consumes one
//! pulse, and the level stays asserted while pulses remain. `X == 0` or
//! `Y == 0` never asserts the request (and avoids a division by zero); the reset
//! value is zero, so firmware that never programs a pacing timer sees no
//! request from these selectors.
//! `TIMER0..3`（`0x440..=0x44C`）是四个 (X/Y) 小数节拍定时器。寄存器布局遵循
//! RP2350 头文件：`X`（被除数）在 `bits[31:16]`，`Y`（除数）在 `bits[15:0]`。
//! 每个定时器以 `(X / Y) * sys_clk` 的速率产生 `DREQ_DMA_TIMERn` 脉冲：`X == Y`
//! 时每个 DMA 周期一个脉冲，`X = 1, Y = 4` 时每四个周期一个。`Dma::advance`
//! 把 `X * cycles` 累加到小数累加器，排队 `累加器 / Y` 个脉冲并保留余数供下次
//! 使用。每个排队脉冲对应一次请求：第一个采样到 `DREQ_DMA_TIMER0..3`（`59..=62`）
//! 的通道传输消费一个脉冲，仍有剩余脉冲时电平保持断言。`X == 0` 或 `Y == 0`
//! 时永不断言（同时避免除零）；复位值为 0，因此从未编程节拍定时器的固件在这些
//! 选择器上不会看到请求。
//!
//! The timer writes the shared level array that [`super::Dma::set_dreq`] also
//! drives, so a host republishing a DREQ bank that overlaps `59..=62` would
//! override a pending level.
//! 定时器写入 [`super::Dma::set_dreq`] 也会驱动的共享电平数组，因此宿主若重新
//! 发布与 `59..=62` 重叠的 DREQ 组，会覆盖待处理的电平。

use super::{DREQ_TIMER0, DmaState, TIMER_COUNT};

/// Divisor (`Y`) field mask / 除数（`Y`）字段掩码
const DIVISOR_MASK: u32 = 0x0000_FFFF;

/// Dividend (`X`) field shift / 被除数（`X`）字段移位
const DIVIDEND_SHIFT: u32 = 16;

/// Dividend (`X`) of one timer register / 定时器寄存器的被除数（`X`）
fn dividend(register: u32) -> u32 {
    register >> DIVIDEND_SHIFT
}

/// Divisor (`Y`) of one timer register / 定时器寄存器的除数（`Y`）
fn divisor(register: u32) -> u32 {
    register & DIVISOR_MASK
}

/// Timer index behind a DREQ selector / DREQ 选择器对应的定时器序号
///
/// Returns `None` for selectors outside `DREQ_DMA_TIMER0..=DREQ_DMA_TIMER3`.
/// 对 `DREQ_DMA_TIMER0..=DREQ_DMA_TIMER3` 之外的选择器返回 `None`。
pub(super) fn timer_index(selector: u8) -> Option<usize> {
    let index = usize::from(selector.checked_sub(DREQ_TIMER0)?);
    (index < TIMER_COUNT).then_some(index)
}

/// Advance all timers by `cycles` DMA cycles / 按 `cycles` 个 DMA 周期推进定时器
///
/// Pulses are queued; the level is asserted whenever at least one pulse is
/// pending. The fractional remainder is carried to the next call.
/// 脉冲进入队列；只要还有待处理脉冲，电平就保持断言。小数余数结转到下次调用。
pub(super) fn advance(state: &mut DmaState, cycles: u64) {
    for index in 0..TIMER_COUNT {
        let selector = usize::from(DREQ_TIMER0) + index;
        let register = state.timer[index];
        let x = u64::from(dividend(register));
        let y = u64::from(divisor(register));
        if x == 0 || y == 0 {
            // X == 0 或 Y == 0：永不断言，同时清空累加器与待处理脉冲。
            // X == 0 or Y == 0 never asserts; the fractional state is cleared.
            state.timer_acc[index] = 0;
            state.timer_pending[index] = 0;
            state.dreq[selector] = false;
            continue;
        }
        let divisor = u128::from(y);
        let total = state.timer_acc[index] + u128::from(x) * u128::from(cycles);
        let pulses = u32::try_from(total / divisor).unwrap_or(u32::MAX);
        state.timer_acc[index] = total % divisor;
        state.timer_pending[index] = state.timer_pending[index].saturating_add(pulses);
        state.dreq[selector] = state.timer_pending[index] != 0;
    }
}

/// Consume one queued pulse after a channel accepted it / 通道接受后消费一个排队脉冲
///
/// Does nothing when no pulse is pending, so a host-driven level is never
/// clobbered.
/// 没有待处理脉冲时不做任何事，因此不会覆盖宿主驱动的电平。
pub(super) fn consume(state: &mut DmaState, index: usize) {
    if index >= TIMER_COUNT || state.timer_pending[index] == 0 {
        return;
    }
    state.timer_pending[index] -= 1;
    state.dreq[usize::from(DREQ_TIMER0) + index] = state.timer_pending[index] != 0;
}

/// Apply a `TIMERx` register write / 应用一次 `TIMERx` 寄存器写入
///
/// Restarts the fractional accumulator and drops any queued pulse; the new
/// `X`/`Y` take effect on the next [`advance`].
/// 重启小数累加器并丢弃已排队的脉冲；新的 `X`/`Y` 在下次 [`advance`] 生效。
pub(super) fn program(state: &mut DmaState, index: usize) {
    if index >= TIMER_COUNT {
        return;
    }
    state.timer_acc[index] = 0;
    state.timer_pending[index] = 0;
    state.dreq[usize::from(DREQ_TIMER0) + index] = false;
}
