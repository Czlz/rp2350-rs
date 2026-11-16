//! Optional two-thread core executor (`--core-threads on`)
//! 可选的双核线程执行器（`--core-threads on`）
//!
//! The default execution path advances core0 and core1 alternately on the
//! caller's thread. This module adds an **opt-in** mode where both Cortex-M33
//! cores run in their own host threads while the caller's thread coordinates
//! the peripherals. The default path is untouched and remains the acceptance
//! baseline.
//! 默认执行路径在调用方线程上交替推进 core0 与 core1。本模块新增**可选**模式：
//! 两个 Cortex-M33 核各自在宿主线程上执行，由调用方线程协调外设。默认路径保持不变，
//! 仍是验收基准。
//!
//! # Window model / 窗口模型
//!
//! Each window is `--core-window N` machine steps (default 64). Inside a window
//! the two cores run freely and every instruction is really executed. At the
//! window boundary the caller waits for both workers, settles the completed
//! window and starts the next one; the boundary work of the completed window
//! (DMA/PIO/HSTX pumps, core-1 mailbox handshakes, interrupt delivery, SIO/SEV
//! event forwarding and the 10 000-step timebase advance) then overlaps that
//! next window. Cross-core events therefore have a latency of at most two
//! windows.
//! 每个窗口为 `--core-window N` 个机器步（默认 64）。窗口内两核自由推进，每条
//! 指令都真实执行。窗口边界处调用方等待两个工作线程、结算刚完成的窗口并启动下一个；
//! 已完成窗口的边界工作（DMA/PIO/HSTX 泵送、core1 信箱握手、中断投递、SIO/SEV
//! 事件转发与 10 000 步时基推进）随后与下一个窗口重叠。因此跨核事件延迟最多两个
//! 窗口。
//!
//! # Determinism boundary / 确定性边界
//!
//! Window boundaries are fixed by the machine step counter, but the **order of
//! memory accesses inside a window is decided by the host scheduler**, so this
//! mode is **not bit-reproducible**. That is an explicit trade-off for
//! interactive speed; the default path stays deterministic.
//! 窗口边界由机器步数固定，但**窗口内内存访问的顺序取决于宿主调度**，因此该模式
//! **不是位可复现的**。这是为交互速度做的显式取舍；默认路径保持确定性。

mod worker;

use std::sync::Arc;

use anyhow::Result;
use rp2350_arm::CortexM33Core;
use rp2350_core::bus::AddressBus;
use rp2350_core::types::CpuId;
use rp2350_cpu::{CpuCore, CpuState};
use rp2350_usb::USBCTRL_IRQ;

use self::worker::{CoreCommand, WindowControl, WindowReport, spawn_worker};
use super::systick::log_systick;
use super::{ADVANCE_INTERVAL, CoreHandle, Machine, PUMP_INTERVAL, log_irq};

pub(super) use self::worker::replays_into_core;

/// How one threaded batch ended / 一次线程化批次的结束原因
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchOutcome {
    /// The requested step range was executed / 已执行请求的步数区间
    Reached,
    /// A core halted or requested a system reset / 核心停机或请求系统复位
    Halted,
    /// Both cores sit in a quiescent sleep / 两核处于静默睡眠
    Quiescent,
}

/// Signals the workers if the coordinator unwinds / 协调线程展开时通知工作线程
struct BatchStopGuard<'a>(&'a WindowControl);

impl Drop for BatchStopGuard<'_> {
    fn drop(&mut self) {
        self.0.signal_stop();
    }
}

/// Gated timing summary for one threaded batch (host diagnostic)
/// 一次线程化批次的按需计时摘要（宿主诊断）
struct BatchStats {
    /// Batch start / 批次起点
    started: std::time::Instant,
    /// Time spent waiting for both workers / 等待两个工作线程的时间
    waited: std::time::Duration,
    /// Time spent on boundary work after the wait / 等待之后边界工作的时间
    boundary: std::time::Duration,
    /// Completed windows / 已完成窗口数
    windows: usize,
    /// Whether `RP2350_THREAD_STATS` is set / 是否设置 `RP2350_THREAD_STATS`
    enabled: bool,
}

impl BatchStats {
    /// Start a summary / 开始计时
    fn new() -> Self {
        Self {
            started: std::time::Instant::now(),
            waited: std::time::Duration::ZERO,
            boundary: std::time::Duration::ZERO,
            windows: 0,
            enabled: std::env::var_os("RP2350_THREAD_STATS").is_some(),
        }
    }
}

impl Drop for BatchStats {
    fn drop(&mut self) {
        if self.enabled {
            eprintln!(
                "[threads] windows={} total={:?} wait={:?} boundary={:?}",
                self.windows,
                self.started.elapsed(),
                self.waited,
                self.boundary
            );
        }
    }
}

impl Machine {
    /// Advance both cores with the optional two-thread executor
    /// 用可选的双线程执行器推进两个核心
    ///
    /// This has the same contract as [`Machine::run_steps`]: every machine step
    /// is counted, every instruction is executed, and the peripheral boundary
    /// work keeps its 64-step / 10 000-step cadence. Only the interleaving of
    /// the two cores inside a window differs.
    /// 契约与 [`Machine::run_steps`] 相同：每个机器步都被计数、每条指令都被执行、
    /// 外设边界工作保持 64 步 / 10 000 步节奏。只有窗口内两核的交错顺序不同。
    ///
    /// # Errors
    /// Threaded execution records core faults in the halt fields instead of
    /// returning them, so this currently never returns an error.
    /// 线程化执行把核心故障记录在停机字段中而不是返回错误，因此当前不会返回错误。
    pub(super) fn run_steps_threaded(&mut self, max_steps: u64) -> Result<CpuState> {
        if self.halt.is_some() || self.system_reset_pending {
            return Ok(CpuState::Halted);
        }
        let end = self.steps.saturating_add(max_steps).min(self.budget);
        if end <= self.steps {
            return Ok(CpuState::Halted);
        }
        while self.steps < end {
            if self.halt.is_some() || self.system_reset_pending {
                return Ok(CpuState::Halted);
            }
            // 只有一个核可运行时没有并行收益：启动阶段、core1 尚未启动、以及
            // 双核睡眠都由单线程批处理路径完成（它自己会快进睡眠）。
            // There is no parallel gain while only one core can run: boot,
            // a not-yet-launched core1 and double sleep stay on the serial
            // batch path, which fast-forwards sleeping steps itself.
            if !self.parallel_ready() {
                let chunk = (end - self.steps).clamp(1, ADVANCE_INTERVAL);
                let state = self.run_steps_serial(chunk)?;
                if state != CpuState::Running {
                    return Ok(state);
                }
                continue;
            }
            match self.run_threads_batch(end) {
                BatchOutcome::Reached => break,
                BatchOutcome::Halted => return Ok(CpuState::Halted),
                BatchOutcome::Quiescent => {}
            }
        }
        Ok(CpuState::Running)
    }

    /// Whether both cores can make progress concurrently / 两核是否都能并发推进
    ///
    /// One core must be `Running` and the peer must be `Running` or `Sleeping`
    /// (a sleeping peer can be woken by an interrupt). `Reset`/halted peers and
    /// a fully sleeping pair stay on the serial path.
    /// 必须有一个核 `Running`，另一个为 `Running` 或 `Sleeping`（睡眠对端可被中断
    /// 唤醒）。`Reset`/停机对端与双核全睡保持单线程路径。
    fn parallel_ready(&self) -> bool {
        let core0 = self.core.state();
        let Some(core1) = self.core1.as_ref().map(rp2350_cpu::CpuCore::state) else {
            return false;
        };
        let live = |state| matches!(state, CpuState::Running | CpuState::Sleeping);
        (core0 == CpuState::Running && live(core1)) || (core1 == CpuState::Running && live(core0))
    }

    /// Run one threaded batch until `end`, a halt or a quiescent sleep
    /// 运行一次线程化批次，直到 `end`、停机或静默睡眠
    fn run_threads_batch(&mut self, end: u64) -> BatchOutcome {
        if !matches!(self.core, CoreHandle::Arm(_)) {
            return BatchOutcome::Halted;
        }
        let dummy = CoreHandle::Arm(Box::new(CortexM33Core::new(CpuId::Core0)));
        let core0 = match std::mem::replace(&mut self.core, dummy) {
            CoreHandle::Arm(core) => core,
            CoreHandle::RiscV(core) => {
                self.core = CoreHandle::RiscV(core);
                return BatchOutcome::Halted;
            }
        };
        let Some(core1) = self.core1.take() else {
            self.core = CoreHandle::Arm(core0);
            self.core1 = Some(CortexM33Core::new(CpuId::Core1));
            return BatchOutcome::Halted;
        };
        let Some(bus) = self.bus.shared_handle() else {
            self.core = CoreHandle::Arm(core0);
            self.core1 = Some(core1);
            return BatchOutcome::Halted;
        };

        let bootrom = Arc::clone(&self.bootrom);
        let sio = self.peripherals.sio().clone();
        let sio_lock = Arc::clone(&self.sio_view_lock);
        let ppb_lock = Arc::clone(&self.ppb_lock);
        let strict_decode = self.strict_decode;
        let control = Arc::new(WindowControl::new());
        let worker_count = Arc::clone(&self.core_worker_count);
        let initial_recent = self.recent;
        let mut pending = [Vec::new(), Vec::new()];

        let (core0, core1, recent, outcome) = std::thread::scope(|scope| {
            let handle0 = spawn_worker(
                scope,
                CpuId::Core0,
                core0,
                Arc::clone(&bus),
                Arc::clone(&bootrom),
                sio.clone(),
                Arc::clone(&sio_lock),
                Arc::clone(&ppb_lock),
                strict_decode,
                initial_recent,
                Arc::clone(&control),
                Arc::clone(&worker_count),
            );
            let handle1 = spawn_worker(
                scope,
                CpuId::Core1,
                Box::new(core1),
                Arc::clone(&bus),
                Arc::clone(&bootrom),
                sio,
                Arc::clone(&sio_lock),
                Arc::clone(&ppb_lock),
                strict_decode,
                super::RecentSteps::new(),
                Arc::clone(&control),
                worker_count,
            );
            let _stop_guard = BatchStopGuard(&control);
            let outcome = self.threaded_loop(end, &control, &mut pending);
            control.signal_stop();
            let outcome0 = handle0.join().expect("core0 worker / core0 工作线程");
            let outcome1 = handle1.join().expect("core1 worker / core1 工作线程");
            (outcome0.core, outcome1.core, outcome0.recent, outcome)
        });

        self.core = CoreHandle::Arm(core0);
        self.core1 = Some(*core1);
        self.recent = recent;
        outcome
    }

    /// Boundary work for one completed window / 一个已完成窗口的边界工作
    ///
    /// Runs while the **next** window executes: the device handles and the
    /// shared bus are thread-safe, and both workers are already running, so the
    /// coordinator's DMA/PIO/HSTX/timebase work overlaps core execution. IRQ
    /// and event commands produced here are consumed two windows later; this is
    /// the documented extra latency of the pipelined boundary.
    /// 在**下一个**窗口执行期间运行：设备句柄与共享总线是线程安全的，且两个工作
    /// 线程已在运行，因此协调线程的 DMA/PIO/HSTX/时基工作与核执行重叠。这里产生的
    /// 中断/事件命令在两个窗口后生效；这就是流水化边界的文档化额外延迟。
    fn process_boundary(
        &mut self,
        report0: &WindowReport,
        report1: &WindowReport,
        pending: &mut [Vec<CoreCommand>; 2],
    ) {
        self.pump_dma_batch();
        self.pump_pio_batch();
        if self.steps - self.pumped >= PUMP_INTERVAL {
            self.peripherals.service_core1_peer();
            self.queue_irq_deliveries(pending, report0, report1);
            self.pumped = self.steps;
        }
        if self.steps - self.advanced >= ADVANCE_INTERVAL {
            self.advance_timebase();
        }
        self.prepare_core1_handshakes(&mut pending[1]);
        self.sync_core_events_threaded(pending, report0, report1);
    }

    /// Coordinator loop of one threaded batch / 一次线程化批次的协调循环
    fn threaded_loop(
        &mut self,
        end: u64,
        control: &WindowControl,
        pending: &mut [Vec<CoreCommand>; 2],
    ) -> BatchOutcome {
        let mut stats = BatchStats::new();
        let mut windows = 0usize;
        let mut previous: Option<(WindowReport, WindowReport)> = None;
        loop {
            if self.steps >= end {
                if let Some((report0, report1)) = previous.take() {
                    self.process_boundary(&report0, &report1, pending);
                }
                return BatchOutcome::Reached;
            }
            if self.halt.is_some() || self.system_reset_pending {
                return BatchOutcome::Halted;
            }
            let until_time = self
                .advanced
                .saturating_add(ADVANCE_INTERVAL)
                .saturating_sub(self.steps)
                .max(1);
            let chunk = (end - self.steps)
                .min(until_time)
                .min(u64::from(self.core_window.max(1)))
                .max(1);
            let base = self.steps;
            let iteration = std::time::Instant::now();
            windows = windows.saturating_add(1);
            control.start_window(chunk, base, pending);
            // 边界工作与**本**窗口重叠：两个工作线程已在执行，设备与共享总线都是
            // 线程安全的。它处理的是上一个窗口的边界，因此命令晚两个窗口生效。
            // Boundary work overlaps **this** window: both workers are already
            // executing and devices/bus are thread-safe. It handles the previous
            // window, so its commands take effect two windows later.
            let boundary_start = std::time::Instant::now();
            if let Some((report0, report1)) = previous.take() {
                self.process_boundary(&report0, &report1, pending);
            }
            let boundary = boundary_start.elapsed();
            control.wait_done(windows);
            let waited = iteration.elapsed();
            let Some(report0) = control.take_report(0) else {
                self.halt = Some(String::from(
                    "core0 worker reported no state / core0 工作线程未上报状态",
                ));
                return BatchOutcome::Halted;
            };
            let Some(report1) = control.take_report(1) else {
                self.halt = Some(String::from(
                    "core1 worker reported no state / core1 工作线程未上报状态",
                ));
                return BatchOutcome::Halted;
            };
            let advance = u64::from(report0.executed).min(chunk);
            self.steps += advance;
            self.sleep_steps[0] = self.sleep_steps[0].saturating_add(report0.sleep_steps);
            self.sleep_steps[1] = self.sleep_steps[1].saturating_add(report1.sleep_steps);
            self.note_core_sleep_state(0, report0.state == CpuState::Sleeping);
            self.note_core_sleep_state(1, report1.state == CpuState::Sleeping);
            self.note_both_sleeping(
                report0.state == CpuState::Sleeping && report1.state == CpuState::Sleeping,
                report0.retired > 0 || report1.retired > 0,
            );
            if report0.reset_requested || report1.reset_requested {
                self.system_reset_pending = true;
            }
            if let Some(halt) = &report0.halt {
                self.halt = Some(halt.clone());
            }
            if let Some(halt) = &report1.halt {
                self.core1_halt = Some(halt.clone());
            }
            if !matches!(report0.state, CpuState::Running | CpuState::Sleeping)
                && report0.halt.is_none()
            {
                self.halt = Some(format!("core entered state {:?}", report0.state));
            }
            if stats.enabled {
                stats.waited += waited;
                stats.boundary += boundary;
                stats.windows = stats.windows.saturating_add(1);
            }
            if self.halt.is_some() || self.system_reset_pending {
                return BatchOutcome::Halted;
            }
            if self.steps >= end {
                self.process_boundary(&report0, &report1, pending);
                return BatchOutcome::Reached;
            }
            if commands_free(pending) && self.threaded_quiescent(&report0, &report1) {
                self.process_boundary(&report0, &report1, pending);
                return BatchOutcome::Quiescent;
            }
            previous = Some((report0, report1));
        }
    }

    /// Queue the boundary interrupts for the next window
    /// 把边界中断排入下一个窗口
    ///
    /// The text emitted here matches [`super::deliver_irqs`] so `[irq]` lines
    /// stay recognisable in both modes. The only difference is that delivery
    /// happens at the next window start instead of inside the current step.
    /// 这里的文本与 [`super::deliver_irqs`] 一致，使 `[irq]` 行在两种模式下都可识别。
    /// 唯一区别是投递发生在下一个窗口开始，而不是当前步内。
    fn queue_irq_deliveries(
        &mut self,
        pending: &mut [Vec<CoreCommand>; 2],
        report0: &WindowReport,
        report1: &WindowReport,
    ) {
        if self.peripherals.take_systick_pending() {
            pending[0].push(CoreCommand::PendSystick);
            log_systick(&mut self.irqs, report0.pc);
        }
        let mut mask = self.peripherals.irq_mask();
        while mask != 0 {
            let irq = mask.trailing_zeros() as u16;
            mask &= mask - 1;
            pending[0].push(CoreCommand::SignalInterrupt(irq));
            log_irq(&mut self.irqs, "", irq, report0.pc);
        }
        if self.usb.irq_asserted() {
            pending[0].push(CoreCommand::SignalInterrupt(USBCTRL_IRQ));
            log_irq(&mut self.irqs, "", USBCTRL_IRQ, report0.pc);
        }
        let mut mask = self.peripherals.core1_irq_mask();
        while mask != 0 {
            let irq = mask.trailing_zeros() as u16;
            mask &= mask - 1;
            pending[1].push(CoreCommand::SignalInterrupt(irq));
            log_irq(&mut self.irqs, "core1 ", irq, report1.pc);
        }
    }

    /// Consume core-1 reset/launch handshakes at a window boundary
    /// 在窗口边界消费 core1 复位/启动握手
    fn prepare_core1_handshakes(&mut self, pending: &mut Vec<CoreCommand>) {
        if self.peripherals.core1_reset_pending() && self.peripherals.take_core1_reset() {
            self.peripherals.reset_core1_boot();
            self.core1_halt = None;
            pending.push(CoreCommand::ResetCore);
        }
        if self.peripherals.core1_launch_pending() {
            let Some(launch) = self.peripherals.take_core1_launch() else {
                return;
            };
            let mut cycles = 0u64;
            if self.bus.read_u32(launch.vtor, &mut cycles).is_err()
                || self
                    .bus
                    .read_u32(launch.vtor.wrapping_add(4), &mut cycles)
                    .is_err()
            {
                self.core1_halt = Some(format!("core1 vtor {:#010X} unreadable", launch.vtor));
                return;
            }
            pending.push(CoreCommand::BootAt {
                sp: launch.sp,
                entry: launch.entry,
                vtor: launch.vtor,
            });
            self.core1_halt = None;
            self.core1_launches = self.core1_launches.saturating_add(1);
        }
    }

    /// Forward SIO/SEV events between the cores at a window boundary
    /// 在窗口边界在核心之间转发 SIO/SEV 事件
    fn sync_core_events_threaded(
        &mut self,
        pending: &mut [Vec<CoreCommand>; 2],
        report0: &WindowReport,
        report1: &WindowReport,
    ) {
        let generation = self.peripherals.sio_event_generation();
        if generation != self.sio_events_seen {
            self.sio_events_seen = generation;
            pending[0].push(CoreCommand::SignalEvent);
            pending[1].push(CoreCommand::SignalEvent);
        }
        if report0.events_sent > 0 {
            pending[1].push(CoreCommand::SignalEvent);
        }
        if report1.events_sent > 0 {
            pending[0].push(CoreCommand::SignalEvent);
        }
    }

    /// Whether both cores sit in a sleep that the fast-forward guard can skip
    /// 两核是否都处于快进守卫可跳过的睡眠
    ///
    /// Mirrors [`super::fast_forward::Machine::sleep_fast_forward_ready`] but
    /// reads the core-side facts from the last window report, because the real
    /// cores are owned by the worker threads while this runs.
    /// 与 [`super::fast_forward::Machine::sleep_fast_forward_ready`] 对应，但核内事实
    /// 取自最近一次窗口上报，因为此函数运行时真实核心由工作线程持有。
    fn threaded_quiescent(&mut self, report0: &WindowReport, report1: &WindowReport) -> bool {
        if !self.sleep_fast_forward {
            return false;
        }
        if report0.state != CpuState::Sleeping {
            return false;
        }
        if matches!(
            report0.sleep_reason,
            Some(rp2350_arm::SleepReason::BlockedRead)
        ) {
            return false;
        }
        if waits_for_event(report0.sleep_reason) && report0.event_pending {
            return false;
        }
        if report0.pending_exception {
            return false;
        }
        match report1.state {
            CpuState::Running => return false,
            CpuState::Sleeping => {
                if matches!(
                    report1.sleep_reason,
                    Some(rp2350_arm::SleepReason::BlockedRead)
                ) {
                    return false;
                }
                if waits_for_event(report1.sleep_reason) && report1.event_pending {
                    return false;
                }
                if report1.pending_exception {
                    return false;
                }
            }
            _ => {}
        }
        if self.peripherals.clock_generation() != self.clock_generation_seen {
            return false;
        }
        let peripherals = &self.peripherals;
        if peripherals.core1_reset_pending()
            || peripherals.core1_launch_pending()
            || peripherals.core1_release_pending()
            || peripherals.sio().fifo_rx_ready(CpuId::Core1)
        {
            return false;
        }
        if peripherals.irq_mask() != 0 || peripherals.core1_irq_mask() != 0 {
            return false;
        }
        if self.usb.irq_asserted() || peripherals.systick_pending() {
            return false;
        }
        if peripherals.dma().busy_mask() != 0 || self.pio_busy_mask() != 0 {
            return false;
        }
        if peripherals.hstx().enabled() || peripherals.adc().busy() {
            return false;
        }
        true
    }

    /// Number of core worker threads alive right now (host diagnostic)
    /// 当前存活的核工作线程数（宿主诊断）
    ///
    /// Zero outside a threaded batch proves the workers were joined and no
    /// handle leaked; it is per machine, so concurrently running machines do
    /// not pollute the value.
    /// 线程化批次之外为零，证明工作线程已 join、没有句柄泄漏；计数按机器隔离，
    /// 并发运行的其它机器不会污染该值。
    #[must_use]
    pub fn active_core_threads(&self) -> usize {
        self.core_worker_count
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Whether a sleep reason waits for the WFE event register
/// 睡眠原因是否等待 WFE 事件寄存器
fn waits_for_event(reason: Option<rp2350_arm::SleepReason>) -> bool {
    matches!(
        reason,
        Some(
            rp2350_arm::SleepReason::WaitForEvent
                | rp2350_arm::SleepReason::SleepDeep {
                    wait_for_event: true
                }
        )
    )
}

/// Whether no coordinator command is queued for either core
/// 是否没有排队给任一核心的协调命令
fn commands_free(pending: &[Vec<CoreCommand>; 2]) -> bool {
    pending.iter().all(Vec::is_empty)
}
