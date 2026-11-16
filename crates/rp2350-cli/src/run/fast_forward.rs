//! Sleep fast-forward for the batched execution path / 批量执行路径的睡眠快进
//!
//! When every runnable core is asleep and no interrupt, DMA, PIO, HSTX, ADC,
//! mailbox or clock event can fire, the machine state between two timebase
//! advances is frozen: only `steps` and the sleep counters move. This module
//! jumps over that interval in one operation instead of spinning one host
//! iteration per sleeping step, while keeping every boundary that the normal
//! path would have observed:
//!
//! - the next `advance_timebase` boundary is always reached by a normal step, so
//!   the timebase, `SysTick`, TIMER alarms, ADC and POWMAN see exactly the same
//!   cycle counts as the step-by-step path;
//! - the DMA/PIO/IRQ pump counters are folded by whole [`PUMP_INTERVAL`] periods,
//!   preserving their phase for the resumed path;
//! - sleep steps, sleep counters and the "both cores slept" notice are credited
//!   exactly as if each step had run;
//! - no guest instruction is skipped: a sleeping core retires nothing by
//!   definition, and any core that can run disables the fast-forward.
//!
//! 当所有可运行核都睡眠、且没有中断、DMA、PIO、HSTX、ADC、信箱或时钟事件能够
//! 触发时，两次时基推进之间的机器状态是冻结的：只有 `steps` 与睡眠计数在变。
//! 本模块用一次操作跳过该区间，而不是每个睡眠步跑一次宿主迭代，同时保留正常路径
//! 会观察到的每个边界：
//!
//! - 始终用一个正常步跨越下一个 `advance_timebase` 边界，因此时基、SysTick、
//!   TIMER alarm、ADC 与 POWMAN 看到的周期数与逐步路径完全一致；
//! - DMA/PIO/IRQ 泵送计数按整 [`PUMP_INTERVAL`] 周期折叠，恢复后的相位不变；
//! - 睡眠步、睡眠计数与「双核睡眠」通知按逐步执行精确记账；
//! - 不跳过任何客户机指令：睡眠核本来就不退役指令，任何可运行核都会关闭快进。

use rp2350_core::types::CpuId;
use rp2350_cpu::{CpuCore, CpuState};

use super::{ADVANCE_INTERVAL, Machine, PUMP_INTERVAL, SLEEP_STALL_STEPS};

impl Machine {
    /// Skip the next run of sleeping steps, returning the new step number
    /// 跳过接下来的一段睡眠步并返回新的步号
    ///
    /// The returned step number is always one less than the next timebase
    /// boundary (or `end`), so the caller's normal loop executes that boundary
    /// step itself. `None` means the machine is not quiescent, fast-forward is
    /// disabled, or there is nothing to skip.
    /// 返回的步号总是下一个时基边界（或 `end`）减一，因此调用方的正常循环会亲自
    /// 执行该边界步。`None` 表示机器不满足静默条件、快进已关闭，或没有可跳过的步。
    pub(super) fn sleep_fast_forward_skip(&mut self, steps: u64, end: u64) -> Option<u64> {
        if !self.sleep_fast_forward {
            return None;
        }
        self.steps = steps;
        // SIO 世代变化是 ARM 事件：先转发，再让守卫判断是否会唤醒 WFE 睡眠者。
        // An SIO generation change is an ARM event: forward it first, then let the
        // guard decide whether it would wake a WFE sleeper.
        self.sync_core_events();
        if !self.sleep_fast_forward_ready() {
            return None;
        }
        // 事件观察边界就是下一次时基推进：TIMER alarm、SysTick 与 ADC 都在该边界
        // 被采样，频率变化也在该边界生效，因此不允许跨过它。
        // The event observation boundary is the next timebase advance: TIMER
        // alarms, SysTick and ADC are sampled there and frequency changes take
        // effect there, so it must never be crossed.
        let boundary = self.advanced.saturating_add(ADVANCE_INTERVAL);
        let target = boundary.min(end);
        if target <= steps.saturating_add(1) {
            return None;
        }
        let landing = target - 1;
        let skipped = landing - steps;
        self.skip_sleeping_steps(skipped);
        fold_pump_counter(&mut self.pumped, landing);
        fold_pump_counter(&mut self.dma_pumped, landing);
        fold_pump_counter(&mut self.pio_pumped, landing);
        self.steps = landing;
        Some(landing)
    }

    /// Whether the machine can skip sleeping steps without changing guest state
    /// 机器是否可以在不改变客户机状态的前提下跳过睡眠步
    ///
    /// Every check is a "no event can fire here" proof. An asserted interrupt
    /// line, a pending `SysTick`, an in-flight DMA/PIO/HSTX/ADC unit or a pending
    /// core-1 mailbox handshake all make the interval observable, so the caller
    /// must fall back to the step-by-step path.
    /// 每项检查都是一次「此处不会触发事件」的证明。已断言的中断线、挂起的
    /// `SysTick`、进行中的 DMA/PIO/HSTX/ADC 单元，或待处理的 core1 信箱握手都会让
    /// 该区间可观察，因此调用方必须回退到逐步路径。
    fn sleep_fast_forward_ready(&mut self) -> bool {
        // Hazard3's mcycle/minstret tick per sleep step; only the ARM cores can
        // batch the clock without changing a guest-visible CSR.
        // Hazard3 的 mcycle/minstret 每个睡眠步前进；只有 ARM 核能在不改变客户机
        // 可见 CSR 的前提下批量记账。
        if !self.core.supports_sleep_fast_forward() || self.core.state() != CpuState::Sleeping {
            return false;
        }
        // 阻塞载入必须按设备可服务的时刻重放，不能按时间批处理。
        // A blocked load must be replayed when the device can serve it.
        if self.core.sleep_blocks_on_read() {
            return false;
        }
        if self.core.sleep_waits_for_event() && self.core.sleep_event_pending() {
            return false;
        }
        // 已转发到核内的系统异常挂起位（例如 SysTick 的 PENDSTSET）会让核在下一步
        // 唤醒，外部中断线守卫看不到它，因此必须单独检查。
        // A forwarded system-exception latch inside the core (for example
        // SysTick's PENDSTSET) wakes it on the very next step, which the external
        // line guards cannot see.
        if self.core.sleep_has_pending_exception() {
            return false;
        }
        if let Some(core1) = self.core1.as_mut() {
            match core1.state() {
                // 另一核仍在运行：绝不快进。 / A peer that can run forbids it.
                CpuState::Running => return false,
                CpuState::Sleeping => {
                    if matches!(
                        core1.sleep_reason(),
                        Some(rp2350_arm::SleepReason::BlockedRead)
                    ) {
                        return false;
                    }
                    let waits_for_event = matches!(
                        core1.sleep_reason(),
                        Some(
                            rp2350_arm::SleepReason::WaitForEvent
                                | rp2350_arm::SleepReason::SleepDeep {
                                    wait_for_event: true
                                }
                        )
                    );
                    if waits_for_event && core1.event_register() {
                        return false;
                    }
                    if arm_core_has_pending_exception(core1) {
                        return false;
                    }
                }
                _ => {}
            }
        }
        // 时钟树世代变化（改频）以时基推进为边界：推进前退避到逐步路径，由下一次
        // `advance` 按实时频率结算。
        // A clock-tree generation change (retune) is bounded by the timebase
        // advance: back off before it and let the next `advance` settle the live
        // rates.
        if self.peripherals.clock_generation() != self.clock_generation_seen {
            return false;
        }
        // core1 启动/复位/释放握手与信箱帧都必须在固定泵送点消费。
        // Core-1 launch/reset/release handshakes and mailbox frames are consumed
        // on fixed pump boundaries.
        let peripherals = &self.peripherals;
        if peripherals.core1_reset_pending()
            || peripherals.core1_launch_pending()
            || peripherals.core1_release_pending()
            || peripherals.sio().fifo_rx_ready(CpuId::Core1)
        {
            return false;
        }
        // 任一断言的中断线都会在下一个 64 步泵送点改变 NVIC 挂起位。
        // Any asserted interrupt line changes NVIC pending bits at the next
        // 64-step pump.
        if peripherals.irq_mask() != 0 || peripherals.core1_irq_mask() != 0 {
            return false;
        }
        if self.usb.irq_asserted() {
            return false;
        }
        // SysTick 的一次性到期锁存在下一个泵送点被转发为异常 15。
        // The one-shot SysTick latch is forwarded as exception 15 on the next pump.
        if peripherals.systick_pending() {
            return false;
        }
        // DMA/PIO/HSTX/ADC 的活动都有逐周期副作用，不能按整段推进。
        // DMA/PIO/HSTX/ADC activity has per-cycle side effects and cannot be
        // batched across.
        if peripherals.dma().busy_mask() != 0 {
            return false;
        }
        if self.pio_busy_mask() != 0 {
            return false;
        }
        if peripherals.hstx().enabled() {
            return false;
        }
        if peripherals.adc().busy() {
            return false;
        }
        true
    }

    /// Credit `skipped` sleeping steps to the machine and both sleep counters
    /// 把 `skipped` 个睡眠步记入机器与两个睡眠计数器
    ///
    /// Mirrors `run_steps`' per-step bookkeeping: `steps`, the per-core sleep
    /// counters, the fast-forward diagnostic and the "both cores slept" stall
    /// notice. `recent`, the spin detector and the wake counters stay untouched
    /// because a sleeping step never touches them.
    /// 复现 `run_steps` 的逐步记账：`steps`、每核睡眠计数、快进诊断与「双核睡眠」
    /// 停滞通知。`recent`、自旋检测与唤醒计数保持不变，因为睡眠步从不触碰它们。
    fn skip_sleeping_steps(&mut self, skipped: u64) {
        let core0_sleeping = self.core.state() == CpuState::Sleeping;
        let core1_sleeping = self
            .core1
            .as_ref()
            .is_some_and(|core| core.state() == CpuState::Sleeping);
        if core0_sleeping {
            self.sleep_steps[0] = self.sleep_steps[0].saturating_add(skipped);
        }
        if core1_sleeping {
            self.sleep_steps[1] = self.sleep_steps[1].saturating_add(skipped);
        }
        self.sleep_fast_forward_steps = self.sleep_fast_forward_steps.saturating_add(skipped);
        self.steps = self.steps.saturating_add(skipped);
        if core0_sleeping && core1_sleeping {
            let before = self.both_sleeping_steps;
            self.both_sleeping_steps = before.saturating_add(skipped);
            // 与 `note_both_sleeping` 一样，阈值只在跨越时触发一次；文本使用阈值
            // 本身，保证与逐步路径逐字节相同。
            // Like `note_both_sleeping`, the threshold fires exactly once when it
            // is crossed; the text uses the threshold itself so it is byte-identical
            // to the step-by-step path.
            if before < SLEEP_STALL_STEPS && self.both_sleeping_steps >= SLEEP_STALL_STEPS {
                self.notices.push(format!(
                    "[run] sleep stall: both cores slept for {SLEEP_STALL_STEPS} steps with no event; \
                     only the clock advanced / 双核无事件睡眠 {SLEEP_STALL_STEPS} 步，仅时钟推进"
                ));
            }
        } else {
            self.both_sleeping_steps = 0;
        }
    }
}

/// Whether an ARM core has a pending exception (the `WFI` wake condition)
/// ARM 核是否有挂起异常（`WFI` 唤醒条件）
///
/// Reads the same architectural state the core's `has_pending_exception` uses:
/// `NVIC.ISER & NVIC.ISPR` plus the SCB `PENDSTSET`/`PENDSVSET`/`NMIPENDSET` and
/// `SHCSR.SVCALLPENDED` latches. Only the first two NVIC enable/pending words
/// exist on RP2350 (external IRQs `0..=63`).
/// 读取核内 `has_pending_exception` 使用的同一份架构状态：`NVIC.ISER & NVIC.ISPR`
/// 以及 SCB 的 `PENDSTSET`/`PENDSVSET`/`NMIPENDSET` 与 `SHCSR.SVCALLPENDED` 锁存。
/// RP2350 只有前两个 NVIC 使能/挂起字（外部中断 `0..=63`）。
pub(super) fn arm_core_has_pending_exception(core: &mut rp2350_arm::CortexM33Core) -> bool {
    /// `SCB_ICSR` absolute address / `SCB_ICSR` 绝对地址
    const ICSR: u32 = 0xE000_ED04;
    /// `SCB_SHCSR` absolute address / `SCB_SHCSR` 绝对地址
    const SHCSR: u32 = 0xE000_ED24;
    /// `ICSR` pending latches: NMI, `PendSV` and `SysTick` / `ICSR` 挂起锁存
    const ICSR_PENDING: u32 = (1 << 31) | (1 << 28) | (1 << 26);
    /// `SHCSR.SVCALLPENDED` / `SHCSR.SVCALLPENDED`
    const SVCALLPENDED: u32 = 1 << 15;
    let enable_lo = core.system_read(0xE000_E100).unwrap_or(0);
    let enable_hi = core.system_read(0xE000_E104).unwrap_or(0);
    let pending_lo = core.system_read(0xE000_E200).unwrap_or(0);
    let pending_hi = core.system_read(0xE000_E204).unwrap_or(0);
    let icsr = core.system_read(ICSR).unwrap_or(0);
    let shcsr = core.system_read(SHCSR).unwrap_or(0);
    ((enable_lo & pending_lo) | (enable_hi & pending_hi)) != 0
        || icsr & ICSR_PENDING != 0
        || shcsr & SVCALLPENDED != 0
}

/// Fold one pump throttle counter across whole pump periods
/// 把一个泵送节流计数按整周期折叠
///
/// The counter records the last step at which the pump ran. During the skipped
/// interval the pump provably had no effect, so moving it forward by whole
/// [`PUMP_INTERVAL`] periods preserves the phase of every later pump exactly.
/// 该计数记录泵送上次运行的步号。跳过区间内泵送已被证明没有作用，因此按整
/// [`PUMP_INTERVAL`] 周期前移即可精确保留之后每次泵送的相位。
fn fold_pump_counter(counter: &mut u64, landing: u64) {
    let folds = landing.saturating_sub(*counter) / PUMP_INTERVAL;
    *counter = (*counter).saturating_add(folds.saturating_mul(PUMP_INTERVAL));
}
