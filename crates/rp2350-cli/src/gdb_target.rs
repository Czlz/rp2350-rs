//! GDB target adapter backed by the CLI [`Machine`]
//! 由 CLI [`Machine`] 支撑的 GDB 目标适配器
//!
//! `rp2350-gdb` exposes the transport and protocol layers only; the machine
//! that owns the bus, boot ROM, peripherals and CPU lives in [`crate::run`].
//! [`MachineTarget`] bridges the two: registers and memory go through the
//! machine's public accessors, while `resume` uses [`Machine::step`] (both
//! cores) or [`Machine::step_cores`] (a single selected core) so boot ROM
//! traps, interrupt pumping and the peripheral timebase stay active.
//! `rp2350-gdb` 只提供传输与协议层；持有总线、boot ROM、外设与 CPU 的机器位于
//! [`crate::run`]。[`MachineTarget`] 负责桥接：寄存器与内存走机器的公共访问器，
//! 恢复执行使用 [`Machine::step`]（双核）或 [`Machine::step_cores`]（单个选中
//! 核心），使 boot ROM 陷阱、中断泵送与外设时基保持有效。

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use rp2350_core::SimpleBus;
use rp2350_core::bus::AddressBus;
use rp2350_core::watch;
use rp2350_cpu::CpuState;
use rp2350_gdb::target::regs;
use rp2350_gdb::{
    GdbError, GdbServer, GdbTarget, ResumeMode, StopReason, StopReply, TargetStatus, ThreadId,
    WatchKind,
};

use crate::run::{Machine, MachineOptions};

/// Default instruction budget for one GDB `Continue`
/// 单次 GDB `Continue` 的默认指令预算
///
/// The limit keeps an accidental infinite loop from blocking the RSP server
/// forever; a client can simply issue `c` again to keep running.
/// 该上限防止意外无限循环永久阻塞 RSP 服务器；客户端再次发送 `c` 即可继续运行。
pub const DEFAULT_CONTINUE_BUDGET: u64 = 1_000_000;

/// Start of the SRAM window (`0x2000_0000`) / SRAM 窗口起始地址（`0x2000_0000`）
const SRAM_START: u32 = 0x2000_0000;
/// End of the 520 KiB SRAM window (`0x2008_2000`) / 520 KiB SRAM 窗口结束地址（`0x2008_2000`）
const SRAM_END: u32 = 0x2008_2000;

/// Whether reads of `length` bytes at `address` are free of side effects
/// `address` 起 `length` 字节的读取是否无副作用
///
/// Watchpoints sample memory on every executed instruction, so only SRAM is
/// accepted. Peripheral and USB windows are rejected: reading them can pop
/// FIFOs or advance device state, and the machine step counter must stay
/// untouched. Flash is not accepted either because guests cannot write it.
/// 监视点会在每条指令后采样内存，因此只接受 SRAM。外设与 USB 窗口被拒绝：读取
/// 它们可能弹出 FIFO 或推进设备状态，而机器步数计数必须保持不变。Flash 也不
/// 接受，因为客户机无法写入 Flash。
fn watchable(address: u32, length: usize) -> bool {
    let Ok(length) = u32::try_from(length) else {
        return false;
    };
    let Some(last) = address.checked_add(length.saturating_sub(1)) else {
        return false;
    };
    address >= SRAM_START && last < SRAM_END
}

/// Read `length` bytes without advancing the machine timeline
/// 读取 `length` 字节且不推进机器时间线
///
/// The local `cycles` counter absorbs the bus latency accounting, so snapshot
/// reads never change [`Machine::steps`] or the peripheral timebase.
/// 局部 `cycles` 计数器吸收总线延迟记账，因此快照读取不会改变
/// [`Machine::steps`] 或外设时基。
fn read_watch_bytes(bus: &mut SimpleBus, address: u32, length: usize) -> Result<Vec<u8>, GdbError> {
    let mut cycles = 0u64;
    let mut bytes = Vec::with_capacity(length);
    for offset in 0..length {
        let offset = u32::try_from(offset).map_err(|_| GdbError::AddressOutOfRange(address))?;
        let addr = address
            .checked_add(offset)
            .ok_or(GdbError::AddressOutOfRange(address))?;
        // 宿主采样不得触发读监视，否则监视点会自己命中自己。
        // Host sampling must not trip the read watch, or the watchpoint would
        // trigger on its own snapshot reads.
        bytes.push(watch::without_read_watch(|| {
            bus.read_u8(addr, &mut cycles)
        })?);
    }
    Ok(bytes)
}

/// Installed watchpoint with its last sampled bytes / 已安装监视点及其最近采样字节
struct WatchEntry {
    /// Direction requested by GDB / GDB 请求的方向
    kind: WatchKind,
    /// Last sampled bytes; empty for read-only watchpoints
    /// 最近采样字节；只读监视点为空
    snapshot: Vec<u8>,
}

/// Options for the `gdb` subcommand / `gdb` 子命令选项
#[derive(Debug)]
pub struct GdbOptions {
    /// Firmware file (ELF) / 固件文件（ELF）
    pub file: PathBuf,
    /// GDB listen port / GDB 监听端口
    pub port: u16,
}

/// Debug target backed by a booted [`Machine`] / 由已启动 [`Machine`] 支撑的调试目标
///
/// Breakpoints are detected by PC comparison (no `BKPT` patching), matching the
/// library's `CortexM33Target` behaviour: resuming from a breakpoint first steps
/// over the instruction at the current PC.
/// 断点通过 PC 比较检测（不写入 `BKPT`），与库中的 `CortexM33Target` 一致：
/// 从断点恢复执行时会先跨过当前 PC 处的指令。
pub struct MachineTarget {
    /// Machine under debug / 被调试的机器
    machine: Machine,
    /// Instruction breakpoints / 指令断点
    breakpoints: BTreeSet<u32>,
    /// Installed write/read/access watchpoints, keyed by `(address, length)`
    /// 已安装的写/读/访问监视点，以 `(地址, 长度)` 为键
    watchpoints: BTreeMap<(u32, usize), WatchEntry>,
    /// Watchpoint address that caused the latest stop / 最近一次停止命中的监视点地址
    last_watch: Option<u32>,
    /// Reason recorded by the most recent resume / 最近一次恢复执行记录的原因
    last_stop: Option<StopReason>,
    /// Instruction budget for a single `Continue` / 单次 `Continue` 的指令预算
    instruction_budget: u64,
    /// Set by `monitor halt`; cleared by `monitor reset` / `monitor halt` 置位，`monitor reset` 清除
    halt_requested: bool,
    /// Thread targeted by the current resume and last stop reply
    /// 当前恢复目标及最近停止回复所属的线程
    last_thread: ThreadId,
}

impl MachineTarget {
    /// Wrap a booted machine into a debug target / 将已启动的机器封装为调试目标
    #[must_use]
    pub fn new(machine: Machine) -> Self {
        Self {
            machine,
            breakpoints: BTreeSet::new(),
            watchpoints: BTreeMap::new(),
            last_watch: None,
            last_stop: None,
            instruction_budget: DEFAULT_CONTINUE_BUDGET,
            halt_requested: false,
            last_thread: ThreadId::CORE0,
        }
    }

    /// Arm the core read watch for every read/access watchpoint
    /// 为所有读/访问监视点安装核心读监视
    ///
    /// The core module supports one `[start, end)` read range, so several
    /// watchpoints are merged into a single span; a hit is matched back to the
    /// closest installed entry when it is reported.
    /// 核心模块只支持一个 `[start, end)` 读区间，因此多个监视点被合并为一个跨度；
    /// 上报命中时会再匹配回具体的监视点条目。
    fn arm_read_range(&mut self) {
        let mut span: Option<(u32, u32)> = None;
        for (&(start, length), entry) in &self.watchpoints {
            if entry.kind == WatchKind::Write {
                continue;
            }
            let length = u32::try_from(length).unwrap_or(u32::MAX);
            let Some(end) = start.checked_add(length) else {
                continue;
            };
            span = Some(match span {
                Some((low, high)) => (low.min(start), high.max(end)),
                None => (start, end),
            });
        }
        watch::set_read_watch(span);
    }

    /// Install one watchpoint of `kind` / 安装一个指定类型的监视点
    fn install_watch(
        &mut self,
        address: u32,
        length: usize,
        kind: WatchKind,
    ) -> Result<(), GdbError> {
        if !watchable(address, length) {
            return Err(GdbError::Unsupported("watchpoints need SRAM windows"));
        }
        // 读监视点不需要写快照，采样开销留给写/访问监视点。
        // Read watchpoints need no value snapshot; sampling stays with write and
        // access watchpoints.
        let snapshot = if kind == WatchKind::Read {
            Vec::new()
        } else {
            let bus = &mut self.machine.bus;
            watch::without_read_watch(|| read_watch_bytes(bus, address, length))?
        };
        self.watchpoints
            .insert((address, length), WatchEntry { kind, snapshot });
        self.arm_read_range();
        Ok(())
    }

    /// Remove the watchpoint at `(address, length)` if it has `kind`
    /// 若 `(地址, 长度)` 处的监视点类型为 `kind` 则移除
    fn remove_watch(&mut self, address: u32, length: usize, kind: WatchKind) {
        if self
            .watchpoints
            .get(&(address, length))
            .is_some_and(|entry| entry.kind == kind)
        {
            self.watchpoints.remove(&(address, length));
        }
        self.arm_read_range();
    }

    /// Sample every write/access watchpoint and return the first changed entry
    /// 采样所有写/访问监视点并返回首个字节发生变化的条目
    ///
    /// The stored snapshot is refreshed on a hit so a later `continue` does not
    /// retrigger on the same value. Bus reads use a local cycle counter and are
    /// wrapped in [`watch::without_read_watch`], so neither the machine step
    /// counter nor the read watch is affected.
    /// 命中后刷新已存快照，使后续 `continue` 不会因同一值再次触发。总线读取使用
    /// 局部周期计数器，并包在 [`watch::without_read_watch`] 中，因此既不影响机器
    /// 步数计数，也不会触发读监视。
    fn changed_watchpoint(&mut self) -> Option<(u32, WatchKind)> {
        let bus = &mut self.machine.bus;
        let mut hit = None;
        for (&(address, length), entry) in &mut self.watchpoints {
            if entry.kind == WatchKind::Read {
                continue;
            }
            let Ok(observed) = watch::without_read_watch(|| read_watch_bytes(bus, address, length))
            else {
                continue;
            };
            if observed != entry.snapshot {
                entry.snapshot = observed;
                if hit.is_none() {
                    hit = Some((address, entry.kind));
                }
            }
        }
        hit
    }

    /// Match a pending guest read hit back to an installed read/access watchpoint
    /// 将待处理客户机读命中匹配回已安装的读/访问监视点
    fn read_watch_hit(&mut self) -> Option<(u32, WatchKind)> {
        let (address, _width) = watch::take_read_hit()?;
        self.watchpoints
            .iter()
            .find_map(|(&(start, length), entry)| {
                if entry.kind == WatchKind::Write {
                    return None;
                }
                let length = u32::try_from(length).unwrap_or(u32::MAX);
                let end = start.checked_add(length)?;
                (address >= start && address < end).then_some((address, entry.kind))
            })
    }

    /// Re-sample every write/access watchpoint to establish the pre-step baseline
    /// 重新采样所有写/访问监视点以建立步前基线
    fn refresh_watchpoints(&mut self) {
        let bus = &mut self.machine.bus;
        for (&(address, length), entry) in &mut self.watchpoints {
            if entry.kind == WatchKind::Read {
                continue;
            }
            if let Ok(observed) =
                watch::without_read_watch(|| read_watch_bytes(bus, address, length))
            {
                entry.snapshot = observed;
            }
        }
    }

    /// Build the stop reply for a watchpoint of `kind` / 构造监视点停止回复
    fn watch_stop(&mut self, address: u32, kind: WatchKind) -> StopReply {
        self.last_watch = Some(address);
        let reason = match kind {
            WatchKind::Write => StopReason::Watchpoint,
            WatchKind::Read => StopReason::ReadWatch,
            WatchKind::Access => StopReason::AccessWatch,
        };
        self.stop_reply(reason)
    }

    /// Execute one instruction on the selected cores and map the result to a stop reason
    /// 在选中的核心上执行一条指令并将结果映射为停止原因
    fn single_step(&mut self, cores: (bool, bool)) -> StopReply {
        self.refresh_watchpoints();
        match self.machine.step_cores(cores.0, cores.1) {
            Ok(CpuState::Running) => {
                if let Some((address, kind)) = self.read_watch_hit() {
                    return self.watch_stop(address, kind);
                }
                if let Some((address, kind)) = self.changed_watchpoint() {
                    return self.watch_stop(address, kind);
                }
                if self.last_thread == ThreadId::CORE1 {
                    if let Some(reason) = self.core1_stop() {
                        return self.stop_reply(reason);
                    }
                }
                self.stop_reply(StopReason::SingleStep)
            }
            Ok(state) => self.stop_reply(self.reason_for_state(state)),
            Err(error) => {
                log::debug!("GDB single step failed: {error:?}");
                self.stop_reply(StopReason::Fault)
            }
        }
    }

    /// Run until a breakpoint, watchpoint, non-running state, or budget limit
    /// 运行直到断点、监视点、非运行状态或预算耗尽
    ///
    /// Only `cores` advance, so a single-core resume retires instructions with
    /// [`Machine::step_cores`] (one call per instruction) instead of
    /// [`Machine::run_cycles`]: it is slower because the machine's per-step
    /// peripheral path is entered from Rust for every instruction, but it keeps
    /// the paused core frozen. A whole-target resume keeps the dual-core
    /// `Machine::step` path and its throughput.
    /// 只推进 `cores` 选中的核心，因此单核恢复用 [`Machine::step_cores`] 逐条执行
    /// （每条指令一次调用），而不是 [`Machine::run_cycles`]：它更慢，因为每条指令
    /// 都要从 Rust 进入机器的每步外设路径，但能让暂停的核心保持冻结。整目标恢复
    /// 仍走双核 `Machine::step` 路径与其吞吐。
    fn continue_until_stop(&mut self, cores: (bool, bool)) -> StopReply {
        let mut budget = self.instruction_budget;
        // 首次迭代跳过 PC 检查，使从断点恢复执行时跨过当前指令，而不是立即
        // 再次停下。
        // The first iteration skips the PC check so resuming from a breakpoint
        // steps over the current instruction instead of stopping again.
        let mut resume_from_breakpoint = true;

        loop {
            if !resume_from_breakpoint {
                if let Some(thread) = self.breakpoint_thread_in(cores) {
                    self.last_thread = thread;
                    return self.stop_reply(StopReason::Breakpoint);
                }
            }
            resume_from_breakpoint = false;

            if budget == 0 || self.machine.budget_exhausted() {
                return self.stop_reply(StopReason::InstructionLimit);
            }
            budget -= 1;

            // Establish the pre-step baseline, then compare after the step so a
            // store anywhere inside the watched range is detected.
            // 先建立步前基线，步后比较，从而检测监视范围内的任意写入。
            self.refresh_watchpoints();
            match self.machine.step_cores(cores.0, cores.1) {
                Ok(CpuState::Running) => {
                    // 先查客户机读命中（读/访问监视点），再查写快照（写/访问）。
                    // Check guest read hits (read/access) before write snapshots
                    // (write/access).
                    if let Some((address, kind)) = self.read_watch_hit() {
                        return self.watch_stop(address, kind);
                    }
                    if let Some((address, kind)) = self.changed_watchpoint() {
                        return self.watch_stop(address, kind);
                    }
                    if self.last_thread == ThreadId::CORE1 {
                        if let Some(reason) = self.core1_stop() {
                            return self.stop_reply(reason);
                        }
                    }
                }
                Ok(state) => return self.stop_reply(self.reason_for_state(state)),
                Err(error) => {
                    log::debug!("GDB continue stopped by error: {error:?}");
                    return self.stop_reply(StopReason::Fault);
                }
            }
        }
    }

    /// `monitor reset`: restore the PC/SP captured at boot
    /// `monitor reset`：恢复到启动时记录的 PC/SP
    ///
    /// Only the program counter and stack pointer are rebuilt from the boot
    /// vectors; RAM contents, registers and watchpoints are left untouched.
    /// 只按启动向量重建程序计数器与栈指针；RAM、寄存器与监视点保持不变。
    fn monitor_reset(&mut self) -> String {
        self.halt_requested = false;
        let Some((sp, pc)) = self.machine.boot_vectors() else {
            return "reset: no boot vectors for this core\n".to_string();
        };
        self.machine
            .write_register(u8::try_from(regs::SP).unwrap_or_default(), sp);
        self.machine
            .write_register(u8::try_from(regs::PC).unwrap_or_default(), pc);
        format!("reset: pc={pc:#010X} sp={sp:#010X}\n")
    }

    /// `monitor info`: core, stepping and DMA/HSTX summary
    /// `monitor info`：核心、步数与 DMA/HSTX 摘要
    fn monitor_info(&self) -> String {
        let mut lines = vec![format!(
            "core0: state={:?} pc={:#010X} sp={:#010X} steps={}",
            self.machine.cpu_state(),
            self.machine.pc(),
            self.machine.sp(),
            self.machine.steps()
        )];
        lines.push(
            match (self.machine.core1_state(), self.machine.core1_pc()) {
                (Some(state), Some(pc)) => format!(
                    "core1: state={state:?} pc={pc:#010X} sp={:#010X} launches={}",
                    self.machine.core1_sp().unwrap_or(0),
                    self.machine.core1_launch_count()
                ),
                _ => "core1: n/a".to_string(),
            },
        );
        lines.push(format!(
            "halt: {}",
            self.machine.halt_reason().unwrap_or("none")
        ));
        lines.push(self.machine.dma_state());
        lines.push(self.machine.hstx_state());
        lines.push(format!(
            "read_watch: {}",
            if self
                .watchpoints
                .values()
                .any(|entry| entry.kind != WatchKind::Write)
            {
                "armed"
            } else {
                "off"
            }
        ));
        lines.join("\n") + "\n"
    }

    /// Map a core state to a stop reason, preferring the machine budget
    /// 将核心状态映射为停止原因，优先报告机器预算耗尽
    fn reason_for_state(&self, state: CpuState) -> StopReason {
        if self.machine.budget_exhausted() {
            StopReason::InstructionLimit
        } else {
            stop_reason_for_state(state)
        }
    }

    /// Shared resume path for whole-target and per-thread requests
    /// 整目标与按线程恢复的共用路径
    ///
    /// `cores` selects which cores retire instructions: a whole-target request
    /// passes `(true, true)` so `c`/`s` keep the dual-core semantics of
    /// [`Machine::step`], while a `vCont;<action>:<tid>` or `Hc<n>` selection
    /// passes exactly the selected core. The reported thread always follows
    /// `thread`, so a core-1 stop replies `thread:2;`.
    /// `cores` 选择哪些核心退役指令：整目标请求传 `(true, true)`，使 `c`/`s`
    /// 保持 [`Machine::step`] 的双核语义；而 `vCont;<action>:<tid>` 或 `Hc<n>`
    /// 选择只传被选中的核心。上报线程始终跟随 `thread`，因此 core1 停止回复
    /// `thread:2;`。
    fn run(&mut self, mode: ResumeMode, thread: ThreadId, cores: (bool, bool)) -> StopReply {
        self.last_watch = None;
        // 丢弃停止期间宿主读取留下的读命中，只报告本次恢复内的访问。
        // Drop hits recorded by host reads while stopped; report only accesses
        // that happen during this resume.
        let _ = watch::take_read_hit();
        self.last_thread = thread;
        let reply = if self.halt_requested {
            self.stop_reply(StopReason::Halted)
        } else {
            match mode {
                ResumeMode::Step => self.single_step(cores),
                ResumeMode::Continue => self.continue_until_stop(cores),
            }
        };
        self.last_stop = Some(reply.reason);
        reply
    }

    /// Build a stop reply from the active thread's PC / 根据当前线程的 PC 构造停止回复
    fn stop_reply(&self, reason: StopReason) -> StopReply {
        StopReply {
            reason,
            pc: self.thread_pc(self.last_thread),
        }
    }

    /// Cores to advance for a resume request / 恢复请求要推进的核心
    ///
    /// Thread 1 steps only core 0 and thread 2 only core 1; the "any thread"
    /// id (`0`) and unknown ids fall back to advancing both cores.
    /// 线程 1 只步进 core0，线程 2 只步进 core1；“任意线程”编号（`0`）与未知
    /// 编号回退为双核推进。
    fn core_selection(thread: ThreadId) -> (bool, bool) {
        match thread {
            ThreadId::CORE0 => (true, false),
            ThreadId::CORE1 => (false, true),
            _ => (true, true),
        }
    }

    /// Program counter of `thread`, falling back to core 0
    /// `thread` 的程序计数器；无 core1 时回退到 core0
    fn thread_pc(&self, thread: ThreadId) -> u32 {
        match thread {
            ThreadId::CORE1 => self.machine.core1_pc().unwrap_or_else(|| self.machine.pc()),
            _ => self.machine.pc(),
        }
    }

    /// First core whose PC sits on an instruction breakpoint
    /// 首个 PC 位于指令断点上的核心
    ///
    /// Only the cores selected for this resume are inspected, so a breakpoint
    /// on the frozen core never ends a single-core resume.
    /// 只检查本次恢复选中的核心，因此冻结核心上的断点不会结束单核恢复。
    fn breakpoint_thread_in(&self, cores: (bool, bool)) -> Option<ThreadId> {
        if cores.0 && self.breakpoints.contains(&self.machine.pc()) {
            return Some(ThreadId::CORE0);
        }
        if !cores.1 {
            return None;
        }
        let core1_pc = self.machine.core1_pc()?;
        self.breakpoints
            .contains(&core1_pc)
            .then_some(ThreadId::CORE1)
    }

    /// Report a core-1 stop when only core 1 stopped during a resume
    /// 恢复期间仅 core1 停止时上报 core1
    ///
    /// Core 1 stays in `Reset` until the SIO FIFO launch protocol hands it
    /// vectors; `Running` means it is still healthy.
    /// core1 在 SIO FIFO 启动协议交出向量前保持 `Reset`；`Running` 表示仍然正常。
    fn core1_stop(&self) -> Option<StopReason> {
        let state = self.machine.core1_state()?;
        (state != CpuState::Running).then(|| self.reason_for_state(state))
    }
}

/// Map a core state returned by `step` to a stop reason
/// 将 `step` 返回的核心状态映射为停止原因
fn stop_reason_for_state(state: CpuState) -> StopReason {
    match state {
        CpuState::Running => StopReason::SingleStep,
        CpuState::Breakpoint => StopReason::Breakpoint,
        CpuState::Sleeping => StopReason::Sleeping,
        CpuState::Faulted => StopReason::Fault,
        CpuState::Halted => StopReason::Halted,
        CpuState::Reset => StopReason::Reset,
    }
}

/// Read a core-1 register through the read-only diagnostics of [`Machine`]
/// 通过 [`Machine`] 的只读诊断读取 core1 寄存器
///
/// [`Machine`] publishes no per-core register file (only `core1_pc`,
/// `core1_sp` and `core1_state`), so PC and SP are real values while the other
/// general-purpose registers read as zero; writes are rejected by
/// [`MachineTarget::write_thread_register`].
/// [`Machine`] 没有公开按核寄存器组（只有 `core1_pc`、`core1_sp` 与
/// `core1_state`），因此 PC 与 SP 是真实值，其余通用寄存器读为 0；写入由
/// [`MachineTarget::write_thread_register`] 拒绝。
fn core1_register(machine: &Machine, register: u32) -> Result<u32, GdbError> {
    match register {
        regs::PC => machine
            .core1_pc()
            .ok_or(GdbError::InvalidRegister(register)),
        regs::SP => machine
            .core1_sp()
            .ok_or(GdbError::InvalidRegister(register)),
        regs::R0..=regs::XPSR => Ok(0),
        _ => Err(GdbError::InvalidRegister(register)),
    }
}

impl GdbTarget for MachineTarget {
    fn read_register(&mut self, register: u32) -> Result<u32, GdbError> {
        match register {
            regs::R0..=regs::PC => Ok(self
                .machine
                .read_register(u8::try_from(register).unwrap_or_default())),
            regs::XPSR => Ok(self.machine.xpsr()),
            _ => Err(GdbError::InvalidRegister(register)),
        }
    }

    fn write_register(&mut self, register: u32, value: u32) -> Result<(), GdbError> {
        match register {
            regs::R0..=regs::PC => {
                self.machine
                    .write_register(u8::try_from(register).unwrap_or_default(), value);
                Ok(())
            }
            regs::XPSR => {
                // RISC-V cores have no XPSR: report the write as unsupported
                // instead of silently dropping it.
                // RISC-V 核心没有 XPSR：把写入报告为不支持，而不是静默丢弃。
                if self.machine.write_xpsr(value) {
                    Ok(())
                } else {
                    Err(GdbError::InvalidRegister(register))
                }
            }
            _ => Err(GdbError::InvalidRegister(register)),
        }
    }

    fn read_memory(&mut self, address: u32, length: usize) -> Result<Vec<u8>, GdbError> {
        // GDB 的 `m` 包是宿主侧读取，不得触发读监视。
        // GDB `m` packets are host-side reads and must not trip the read watch.
        watch::without_read_watch(|| {
            let mut bytes = Vec::with_capacity(length);
            let mut cycles = 0u64;

            for offset in 0..length {
                let offset =
                    u32::try_from(offset).map_err(|_| GdbError::AddressOutOfRange(address))?;
                let addr = address
                    .checked_add(offset)
                    .ok_or(GdbError::AddressOutOfRange(address))?;
                bytes.push(self.machine.bus.read_u8(addr, &mut cycles)?);
            }

            Ok(bytes)
        })
    }

    fn write_memory(&mut self, address: u32, bytes: &[u8]) -> Result<(), GdbError> {
        let mut cycles = 0u64;

        for (offset, byte) in bytes.iter().copied().enumerate() {
            let offset = u32::try_from(offset).map_err(|_| GdbError::AddressOutOfRange(address))?;
            let addr = address
                .checked_add(offset)
                .ok_or(GdbError::AddressOutOfRange(address))?;
            self.machine.bus.write_u8(addr, byte, &mut cycles)?;
        }

        Ok(())
    }

    fn set_breakpoint(&mut self, address: u32) -> Result<(), GdbError> {
        self.breakpoints.insert(address);
        Ok(())
    }

    fn clear_breakpoint(&mut self, address: u32) -> Result<(), GdbError> {
        self.breakpoints.remove(&address);
        Ok(())
    }

    fn has_breakpoint(&self, address: u32) -> bool {
        self.breakpoints.contains(&address)
    }

    fn supports_watchpoints(&self) -> bool {
        true
    }

    fn set_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        self.install_watch(address, length, WatchKind::Write)
    }

    fn clear_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        self.remove_watch(address, length, WatchKind::Write);
        Ok(())
    }

    fn set_read_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        self.install_watch(address, length, WatchKind::Read)
    }

    fn clear_read_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        self.remove_watch(address, length, WatchKind::Read);
        Ok(())
    }

    fn set_access_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        self.install_watch(address, length, WatchKind::Access)
    }

    fn clear_access_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        self.remove_watch(address, length, WatchKind::Access);
        Ok(())
    }

    fn watchpoint_hit(&self) -> Option<u32> {
        self.last_watch
    }

    fn monitor_command(&mut self, command: &str) -> String {
        match command {
            "reset" => self.monitor_reset(),
            "info" => self.monitor_info(),
            "halt" => {
                self.halt_requested = true;
                format!("halted: pc={:#010X}\n", self.machine.pc())
            }
            other => {
                // 未识别命令静默接受：GDB 会收到 `OK`，不会因拼写错误中断会话。
                // Unknown commands are accepted silently: GDB receives `OK` so a
                // typo never breaks the session.
                log::info!("unsupported monitor command: {other}");
                String::new()
            }
        }
    }

    fn resume(&mut self, mode: ResumeMode) -> Result<StopReply, GdbError> {
        // Plain `c`/`s` means "the whole target": both cores advance, and the
        // reply reports core 0 unless a core-1 breakpoint overrides it.
        // 普通 `c`/`s` 表示“整个目标”：双核都推进，回复上报 core0，除非 core1
        // 断点改写上报线程。
        Ok(self.run(mode, ThreadId::CORE0, (true, true)))
    }

    fn continue_thread(&mut self, thread: ThreadId) -> Result<StopReply, GdbError> {
        Ok(self.run(ResumeMode::Continue, thread, Self::core_selection(thread)))
    }

    fn step_thread(&mut self, thread: ThreadId) -> Result<StopReply, GdbError> {
        Ok(self.run(ResumeMode::Step, thread, Self::core_selection(thread)))
    }

    fn stopped_thread(&self) -> ThreadId {
        self.last_thread
    }

    fn thread_count(&self) -> usize {
        // RISC-V builds have no core 1, so they keep a single-thread table.
        // RISC-V 构建没有 core1，因此保持单线程表。
        if self.machine.core1_state().is_some() {
            2
        } else {
            1
        }
    }

    fn thread_id(&self, index: usize) -> Option<ThreadId> {
        match index {
            0 => Some(ThreadId::CORE0),
            1 if self.machine.core1_state().is_some() => Some(ThreadId::CORE1),
            _ => None,
        }
    }

    fn thread_name(&self, thread: ThreadId) -> String {
        match thread {
            ThreadId::CORE0 => "core0".to_string(),
            ThreadId::CORE1 => "core1".to_string(),
            other => format!("thread {}", other.number()),
        }
    }

    fn thread_state(&self, thread: ThreadId) -> Option<CpuState> {
        match thread {
            ThreadId::CORE0 => Some(self.machine.cpu_state()),
            ThreadId::CORE1 => self.machine.core1_state(),
            _ => None,
        }
    }

    fn read_thread_register(&mut self, thread: ThreadId, register: u32) -> Result<u32, GdbError> {
        match thread {
            ThreadId::CORE0 => self.read_register(register),
            ThreadId::CORE1 => core1_register(&self.machine, register),
            other => Err(GdbError::InvalidRegister(other.number())),
        }
    }

    fn write_thread_register(
        &mut self,
        thread: ThreadId,
        register: u32,
        value: u32,
    ) -> Result<(), GdbError> {
        match thread {
            ThreadId::CORE0 => self.write_register(register, value),
            ThreadId::CORE1 => Err(GdbError::Unsupported(
                "core1 registers are read-only / core1 寄存器只读",
            )),
            other => Err(GdbError::InvalidRegister(other.number())),
        }
    }

    fn status(&self) -> TargetStatus {
        TargetStatus {
            state: self.machine.cpu_state(),
            stop_reason: self.last_stop,
            pc: self.machine.pc(),
            // Steps double as simulated cycles in the machine timeline.
            // 在机器时间线中步数同时充当模拟周期。
            total_cycles: self.machine.steps(),
        }
    }
}

/// Load the firmware, boot a machine and serve GDB clients until the listener fails
/// 加载固件、启动机器并提供 GDB 客户端服务，直到监听器出错
///
/// # Errors
/// Returns an error when the firmware cannot be read, parsed or booted, or when
/// the TCP listener fails.
/// 固件无法读取、解析或启动，或 TCP 监听器失败时返回错误。
pub fn run(options: &GdbOptions) -> Result<()> {
    let bytes = std::fs::read(&options.file).with_context(|| {
        format!(
            "cannot read firmware / 无法读取固件: {}",
            options.file.display()
        )
    })?;

    // `u64::MAX` keeps the machine from halting on its own budget: GDB decides
    // when to stop, and each `Continue` has its own instruction cap.
    // `u64::MAX` 避免机器因自身预算停机：由 GDB 决定何时停止，单次 `Continue`
    // 另有指令上限。
    let machine_options = MachineOptions {
        arch: None,
        max_cycles: u64::MAX,
        strict_decode: false,
    };
    let machine = Machine::new(&bytes, &machine_options)?;

    println!("[gdb] {}", machine.image_description());
    println!("[gdb] core: {}", machine.core_description());
    println!(
        "[gdb] started: PC={:#010X} SP={:#010X}",
        machine.pc(),
        machine.sp()
    );

    let addr = SocketAddr::from(([0, 0, 0, 0], options.port));
    let mut server = GdbServer::new(addr);
    // The address is exact for every non-zero port; `GdbServer` logs the actual
    // bound address once the listener exists.
    // 除端口 0 外该地址即实际地址；监听器建立后 `GdbServer` 会记录实际绑定地址。
    println!("[gdb] listening on {} (Ctrl-C to stop)", server.addr());

    let result = server.serve(Box::new(MachineTarget::new(machine)));
    println!(
        "[gdb] server stopped (last listen address {})",
        server.addr()
    );
    result.context("GDB server failed / GDB 服务器失败")?;
    Ok(())
}

#[cfg(test)]
mod tests;
