//! Core worker thread of the `--core-threads on` executor
//! `--core-threads on` 执行器的核工作线程
//!
//! One worker owns exactly one `CortexM33Core` for the duration of a threaded
//! batch. It waits for the coordinator's next phase, executes the announced
//! window (every instruction is really executed), publishes a [`WindowReport`]
//! and increments the completion counter. The coordinator is the only thread
//! that touches peripherals, so device state changes stay serialised while the
//! two cores run concurrently inside a window.
//! 一个工作线程在一次线程化批次期间独占一个 `CortexM33Core`。它等待协调线程的
//! 下一个 phase，执行公布的窗口（每条指令都真实执行），发布 [`WindowReport`]
//! 并递增完成计数。外设只由协调线程触碰，因此一个窗口内两核并发执行，而设备状态
//! 变化保持串行。

use rp2350_arm::{CortexM33Core, SleepReason};
use rp2350_bootrom::Rp2350Bootrom;
use rp2350_core::shared::SharedBus;
use rp2350_core::types::CpuId;
use rp2350_cpu::{CpuCore, CpuState};
use rp2350_periph::sio::Sio;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::super::RecentSteps;
use super::super::bus::CoreBus;
use super::super::fast_forward::arm_core_has_pending_exception;

/// Per-machine worker counter, shared with [`ActiveThreadGuard`]
/// 与 [`ActiveThreadGuard`] 共享的每机器工作线程计数
pub(super) type WorkerCounter = Arc<AtomicUsize>;

/// Decrements the machine's worker counter even when the worker unwinds
/// 即使工作线程展开也会递减该机器的工作线程计数
struct ActiveThreadGuard(WorkerCounter);

impl ActiveThreadGuard {
    /// Count one new worker / 计入一个新的工作线程
    fn new(counter: WorkerCounter) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(counter)
    }
}

impl Drop for ActiveThreadGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// One command the coordinator queues for a core worker
/// 协调线程排队给核工作线程的一条命令
#[derive(Debug, Clone, Copy)]
pub(super) enum CoreCommand {
    /// Assert one NVIC interrupt line / 断言一条 NVIC 中断线
    SignalInterrupt(u16),
    /// Raise the ARMv8-M event register / 置位 ARMv8-M 事件寄存器
    SignalEvent,
    /// Forward one peripheral `SysTick` expiry / 转发一次外设 `SysTick` 到期
    PendSystick,
    /// Apply core-1 launch vectors / 应用 core1 启动向量
    BootAt {
        /// Initial stack pointer / 初始栈指针
        sp: u32,
        /// Reset vector / 复位向量
        entry: u32,
        /// Vector table base / 向量表基址
        vtor: u32,
    },
    /// Replace the core with a fresh reset-state core / 用全新复位态核心替换
    ResetCore,
}

/// State a worker reports at every window boundary / 工作线程在每个窗口边界上报的状态
#[derive(Debug, Clone)]
pub(super) struct WindowReport {
    /// Machine steps attempted in this window / 本窗口尝试的机器步数
    pub(super) executed: u32,
    /// Instructions actually retired / 实际退役的指令数
    pub(super) retired: u32,
    /// Sleep steps taken / 睡眠步数
    pub(super) sleep_steps: u64,
    /// State after the window / 窗口结束后的状态
    pub(super) state: CpuState,
    /// Sleep reason after the window / 窗口结束后的睡眠原因
    pub(super) sleep_reason: Option<SleepReason>,
    /// Whether the WFE event register is set / WFE 事件寄存器是否置位
    pub(super) event_pending: bool,
    /// Whether an enabled pending exception would wake the core
    /// 是否有已使能挂起异常会唤醒该核
    pub(super) pending_exception: bool,
    /// `SEV` instructions executed in this window / 本窗口执行的 `SEV` 指令数
    pub(super) events_sent: u32,
    /// Whether `AIRCR.SYSRESETREQ` was latched / 是否锁存了 `AIRCR.SYSRESETREQ`
    pub(super) reset_requested: bool,
    /// Non-budget halt reason / 非预算停机原因
    pub(super) halt: Option<String>,
    /// Program counter after the window / 窗口结束后的程序计数器
    pub(super) pc: u32,
}

impl WindowReport {
    /// Snapshot the fields the coordinator reads at a window boundary
    /// 在窗口边界快照协调线程读取的字段
    pub(super) fn from_core(core: &mut CortexM33Core) -> Self {
        Self {
            executed: 0,
            retired: 0,
            sleep_steps: 0,
            state: core.state(),
            sleep_reason: core.sleep_reason(),
            event_pending: core.event_register(),
            pending_exception: arm_core_has_pending_exception(core),
            events_sent: 0,
            reset_requested: false,
            halt: None,
            pc: core.pc(),
        }
    }

    /// Refresh the end-of-window fields from the core / 从核心刷新窗口结束字段
    fn finish(&mut self, core: &mut CortexM33Core) {
        self.state = core.state();
        self.sleep_reason = core.sleep_reason();
        self.event_pending = core.event_register();
        self.pending_exception = arm_core_has_pending_exception(core);
        self.events_sent = core.take_events_sent();
        self.reset_requested |= core.take_system_reset_request();
        self.pc = core.pc();
    }
}

/// Cache-line padded atomic to keep the hot handshake words independent
/// 缓存行填充的原子量，使热握手字互不共享缓存行
#[repr(align(64))]
struct Padded<T>(T);

impl<T> Padded<T> {
    /// Wrap a value on its own cache line / 把值放到独立缓存行
    const fn new(value: T) -> Self {
        Self(value)
    }
}

/// Window mailbox and synchronisation shared by the coordinator and workers
/// 协调线程与两个工作线程共享的窗口信箱与同步
///
/// The barrier is a pair of atomics rather than `std::sync::Barrier`: with a
/// 64-step window the host would park and wake three threads over a million
/// times on a 300 M run, which costs far more than the window itself. Workers
/// spin with `spin_loop` and fall back to `yield_now` after a bounded number of
/// iterations, so a machine with fewer cores than threads still makes progress.
/// The handshake words sit on separate cache lines: two workers spin on
/// `phase` while the coordinator polls `done`, and false sharing there costs
/// more than the window itself.
/// 屏障由一对原子量实现，而不是 `std::sync::Barrier`：64 步窗口在 300 M 运行中
/// 会让三个线程停泊/唤醒上百万次，开销远超窗口本身。工作线程先自旋
/// (`spin_loop`)，超过固定次数后退让 (`yield_now`)，因此线程数多于核数时仍能推进。
/// 握手字位于独立缓存行：两个工作线程自旋读 `phase`，协调线程轮询 `done`，
/// 伪共享的开销会比窗口本身更大。
pub(super) struct WindowControl {
    /// Incremented by the coordinator when a window starts / 协调线程在窗口开始时递增
    phase: Padded<AtomicUsize>,
    /// Number of workers that finished a window, monotonically / 已完成窗口的工作线程累计数
    done: Padded<AtomicUsize>,
    /// Window length in machine steps / 窗口长度（机器步）
    chunk: Padded<AtomicU64>,
    /// Machine step number of the first step in the window / 窗口首步的机器步号
    base: Padded<AtomicU64>,
    /// Set by the coordinator to stop both workers / 协调线程置位以停止两个工作线程
    stop: Padded<AtomicBool>,
    /// Pending commands per core / 每核的待处理命令
    commands: [Mutex<Vec<CoreCommand>>; 2],
    /// Report slot per core / 每核的上报槽
    reports: [Mutex<Option<WindowReport>>; 2],
}

impl WindowControl {
    /// Create an empty mailbox for one threaded batch / 为一次线程化批次创建空信箱
    pub(super) fn new() -> Self {
        Self {
            phase: Padded::new(AtomicUsize::new(0)),
            done: Padded::new(AtomicUsize::new(0)),
            chunk: Padded::new(AtomicU64::new(0)),
            base: Padded::new(AtomicU64::new(0)),
            stop: Padded::new(AtomicBool::new(false)),
            commands: [Mutex::new(Vec::new()), Mutex::new(Vec::new())],
            reports: [Mutex::new(None), Mutex::new(None)],
        }
    }

    /// Announce the next window and move queued commands into the mailboxes
    /// 公布下一个窗口，并把排队命令移入信箱
    pub(super) fn start_window(&self, chunk: u64, base: u64, pending: &mut [Vec<CoreCommand>; 2]) {
        self.chunk.0.store(chunk, Ordering::Relaxed);
        self.base.0.store(base, Ordering::Relaxed);
        for (index, queue) in pending.iter_mut().enumerate() {
            let mut slot = self.commands[index]
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            slot.clear();
            slot.append(queue);
        }
        // Release: every field above is visible to a worker that observes the
        // new phase. / Release：观察到新 phase 的工作线程能看到上述所有字段。
        self.phase.0.fetch_add(1, Ordering::Release);
    }

    /// Wait until both workers have finished the window started last
    /// 等待两个工作线程完成最近启动的窗口
    pub(super) fn wait_done(&self, windows: usize) {
        let expected = windows.saturating_mul(2);
        let mut spins = 0u32;
        while self.done.0.load(Ordering::Acquire) < expected {
            if spins < 1024 {
                std::hint::spin_loop();
            } else {
                std::thread::yield_now();
            }
            spins = spins.saturating_add(1);
        }
    }

    /// Whether the coordinator asked the workers to stop / 协调线程是否要求停止
    pub(super) fn should_stop(&self) -> bool {
        self.stop.0.load(Ordering::Acquire)
    }

    /// Ask both workers to exit and wake them from the phase wait
    /// 要求两个工作线程退出并唤醒 phase 等待
    pub(super) fn signal_stop(&self) {
        self.stop.0.store(true, Ordering::Release);
        self.phase.0.fetch_add(1, Ordering::Release);
    }

    /// Window length and base step for this window / 本窗口长度与首步步号
    fn window(&self) -> (u64, u64) {
        (
            self.chunk.0.load(Ordering::Acquire),
            self.base.0.load(Ordering::Acquire),
        )
    }

    /// Current phase value / 当前 phase 值
    fn phase(&self) -> usize {
        self.phase.0.load(Ordering::Acquire)
    }

    /// Report that one worker finished a window / 报告一个工作线程完成窗口
    fn finish_window(&self) {
        self.done.0.fetch_add(1, Ordering::Release);
    }

    /// Drain the commands queued for `index` / 取走排队给 `index` 的命令
    fn take_commands(&self, index: usize) -> Vec<CoreCommand> {
        let mut slot = self.commands[index]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::take(&mut *slot)
    }

    /// Publish one worker report / 发布一个工作线程上报
    fn put_report(&self, index: usize, report: WindowReport) {
        let mut slot = self.reports[index]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = Some(report);
    }

    /// Take one worker report, if any / 取走一个工作线程上报（若有）
    pub(super) fn take_report(&self, index: usize) -> Option<WindowReport> {
        self.reports[index]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

/// Everything a worker returns when the batch ends / 批次结束时工作线程返回的全部内容
pub(super) struct WorkerOutcome {
    /// The core the worker owned / 工作线程持有的核心
    pub(super) core: Box<CortexM33Core>,
    /// Executed-PC ring of core 0 / core0 的已执行 PC 环形缓冲
    pub(super) recent: RecentSteps,
}

/// Signals the coordinator even if the window body panics
/// 即使窗口主体 panic 也会通知协调线程
// 工作线程接收 12 个参数：核、总线、bootrom、SIO 视图锁、PPB 锁、统计计数等
// 都是不同所有权来源，打包成结构体只会把相同的字段搬一次，不改变可读性。
// The worker takes 12 parameters (core, bus, bootrom, SIO view lock, PPB lock,
// counters ...) with distinct ownership sources; packing them into a struct
// would only move the same fields once and hurt readability.
#[allow(clippy::too_many_arguments, clippy::large_types_passed_by_value)]
pub(super) fn spawn_worker<'scope>(
    scope: &'scope std::thread::Scope<'scope, '_>,
    id: CpuId,
    core: Box<CortexM33Core>,
    bus: Arc<SharedBus>,
    bootrom: Arc<Rp2350Bootrom>,
    sio: Sio,
    sio_lock: Arc<Mutex<()>>,
    ppb_lock: Arc<Mutex<()>>,
    strict_decode: bool,
    recent: RecentSteps,
    control: Arc<WindowControl>,
    workers: WorkerCounter,
) -> std::thread::ScopedJoinHandle<'scope, WorkerOutcome> {
    let index = match id {
        CpuId::Core0 => 0,
        CpuId::Core1 => 1,
    };
    scope.spawn(move || {
        let _active = ActiveThreadGuard::new(workers);
        let mut core = core;
        let mut recent = recent;
        let mut seen = 0usize;
        loop {
            wait_for_phase(&control, seen);
            seen = control.phase();
            if control.should_stop() {
                break;
            }
            let commands = control.take_commands(index);
            apply_commands(&mut core, &commands, strict_decode);
            let (chunk, base) = control.window();
            let report = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_window(
                    &mut core,
                    &bus,
                    &bootrom,
                    &sio,
                    &sio_lock,
                    &ppb_lock,
                    id,
                    chunk,
                    base,
                    &mut recent,
                )
            }))
            .unwrap_or_else(|_| WindowReport {
                executed: 0,
                retired: 0,
                sleep_steps: 0,
                state: CpuState::Faulted,
                sleep_reason: None,
                event_pending: false,
                pending_exception: true,
                events_sent: 0,
                reset_requested: false,
                halt: Some(String::from("core worker panicked / 核工作线程 panic")),
                pc: core.pc(),
            });
            control.put_report(index, report);
            control.finish_window();
        }
        WorkerOutcome { core, recent }
    })
}

/// Wait until the coordinator publishes a phase different from `seen`
/// 等待协调线程发布与 `seen` 不同的 phase
fn wait_for_phase(control: &WindowControl, seen: usize) {
    let mut spins = 0u32;
    while control.phase() == seen {
        if spins < 1024 {
            std::hint::spin_loop();
        } else {
            std::thread::yield_now();
        }
        spins = spins.saturating_add(1);
    }
}

/// Apply the coordinator's commands before the next window
/// 在下一个窗口前应用协调线程的命令
fn apply_commands(core: &mut CortexM33Core, commands: &[CoreCommand], strict_decode: bool) {
    for command in commands {
        match *command {
            CoreCommand::SignalInterrupt(irq) => core.signal_interrupt(irq),
            CoreCommand::SignalEvent => core.signal_event(),
            CoreCommand::PendSystick => core.pend_systick(),
            CoreCommand::BootAt { sp, entry, vtor } => core.boot_at(sp, entry, vtor),
            CoreCommand::ResetCore => {
                let mut fresh = CortexM33Core::new(CpuId::Core1);
                fresh.set_strict_decode(strict_decode);
                fresh.set_flash_fetch_cache(true);
                *core = fresh;
            }
        }
    }
}

/// Execute one window and report what the core did
/// 执行一个窗口并上报核心的实际进展
#[allow(clippy::too_many_arguments)]
fn run_window(
    core: &mut CortexM33Core,
    bus: &SharedBus,
    bootrom: &Rp2350Bootrom,
    sio: &Sio,
    sio_lock: &Mutex<()>,
    ppb_lock: &Mutex<()>,
    id: CpuId,
    chunk: u64,
    base: u64,
    recent: &mut RecentSteps,
) -> WindowReport {
    let record_recent = id == CpuId::Core0;
    let mut report = WindowReport::from_core(core);
    for offset in 0..chunk {
        if !matches!(core.state(), CpuState::Running | CpuState::Sleeping) {
            break;
        }
        let step = base.saturating_add(offset);
        let pc = core.pc();
        let mut adapter = CoreBus {
            bus,
            core: id,
            sio,
            sio_lock,
            ppb_lock,
            ppb_writes: Vec::new(),
        };
        let in_rom_window = pc.wrapping_sub(rp2350_bootrom::ROM_BASE)
            < u32::try_from(rp2350_bootrom::ROM_SIZE).unwrap_or(u32::MAX);
        let mut trapped = false;
        if in_rom_window && bootrom.handles(pc) {
            match bootrom.dispatch(core, &mut adapter) {
                Ok(Some(outcome)) => {
                    replay_ppb(core, &mut adapter);
                    report.executed = report.executed.saturating_add(1);
                    report.retired = report.retired.saturating_add(1);
                    if record_recent {
                        recent.push(step, pc);
                    }
                    if let Some(halt) = outcome.halt {
                        report.halt = Some(format!("bootrom halt: {halt}"));
                        break;
                    }
                    trapped = true;
                }
                Ok(None) => {}
                Err(error) => {
                    report.halt = Some(error.to_string());
                    break;
                }
            }
        }
        if trapped {
            if core.take_system_reset_request() {
                report.reset_requested = true;
            }
            continue;
        }
        match core.step(&mut adapter) {
            Ok(CpuState::Running) => {
                replay_ppb(core, &mut adapter);
                report.executed = report.executed.saturating_add(1);
                report.retired = report.retired.saturating_add(1);
                if record_recent {
                    recent.push(step, pc);
                }
            }
            Ok(CpuState::Sleeping) => {
                replay_ppb(core, &mut adapter);
                report.executed = report.executed.saturating_add(1);
                report.sleep_steps = report.sleep_steps.saturating_add(1);
            }
            Ok(state) => {
                report.state = state;
                break;
            }
            Err(error) => {
                report.halt = Some(error.to_string());
                break;
            }
        }
        if core.take_system_reset_request() {
            report.reset_requested = true;
        }
    }
    report.finish(core);
    report
}

/// Replay the PPB writes the adapter captured into the owning core
/// 把适配器捕获的 PPB 写入回放到所属核心
fn replay_ppb(core: &mut CortexM33Core, adapter: &mut CoreBus<'_>) {
    for (addr, value) in adapter.take_ppb_writes() {
        core.system_write(addr, value);
    }
}

/// Whether a PPB write must be replayed into the writing core
/// PPB 写入是否必须回放到写入核心
///
/// Mirrors `rp2350_periph::ppb::is_replayed`, which is private to that crate.
/// The list is architectural (ARMv8-M system control space plus the RP2350
/// `CPACR`), not firmware-specific.
/// 与 `rp2350_periph::ppb::is_replayed` 一致（后者是该 crate 私有项）。该清单是
/// 架构性的（ARMv8-M 系统控制空间加 RP2350 `CPACR`），与具体固件无关。
pub(crate) fn replays_into_core(addr: u32) -> bool {
    matches!(
        addr,
        0xE000_ED08 // SCB_VTOR
            | 0xE000_ED04 // SCB_ICSR
            | 0xE000_ED0C // SCB_AIRCR
            | 0xE000_ED24 // SCB_SHCSR
            | 0xE000_ED28 // SCB_CFSR
            | 0xE000_ED2C // SCB_HFSR
            | 0xE000_ED14 // SCB_CCR
            | 0xE000_ED18 // SCB_SHPR1
            | 0xE000_ED1C // SCB_SHPR2
            | 0xE000_ED20 // SCB_SHPR3
            | 0xE000_ED34 // SCB_MMFAR
            | 0xE000_ED38 // SCB_BFAR
            | 0xE000_ED10 // SCB_SCR
            | 0xE000_EF00 // SCB_STIR
            | 0xE000_ED88 // SCB_CPACR
    ) || (0xE000_E100..=0xE000_E4FF).contains(&addr) // NVIC
}
