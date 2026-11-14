//! `run` subcommand: build the Stage-4 bus, load firmware, boot and trace
//! `run` 子命令：构建 Stage 4 总线、加载固件、启动并输出轨迹

// 前端地址常量与映像偏移都在 32 位客户机地址空间内换算，这是模拟器前端的固有特性。
// Frontend address constants and image offsets are converted within the 32-bit
// guest address space, inherent to emulator-frontend address arithmetic.
#![allow(clippy::cast_possible_truncation)]

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::ValueEnum;
use rp2350_arm::CortexM33Core;
use rp2350_bootrom::Rp2350Bootrom;
use rp2350_core::types::CpuId;
use rp2350_core::{
    AddressBus, FLASH_XIP_BASE, FLASH_XIP_NOCACHE_BASE, FirmwareImage, FlashRegion, RamRegion,
    SimpleBus, Uf2Arch, Uf2Image, XipMaintenanceRegion,
};
use rp2350_cpu::{CpuCore, CpuState};
use rp2350_periph::BootPeripherals;
use rp2350_riscv::Hazard3Core;
use rp2350_usb::{Controller, USBCTRL_IRQ};

/// SRAM base address / SRAM 基地址
const SRAM_BASE: u32 = 0x2000_0000;

/// SRAM size (520 KB) / SRAM 大小（520 KB）
const SRAM_SIZE: usize = 520 * 1024;

/// Four-byte UF2 magic prefix / UF2 魔术前缀（四字节）
const UF2_MAGIC: [u8; 4] = [0x55, 0x46, 0x32, 0x0A];

/// Default instruction budget for `run` / `run` 的默认指令预算
pub const DEFAULT_RUN_MAX_CYCLES: u64 = 20_000_000;

/// Instructions between interrupt polls / 中断轮询间隔（指令数）
const PUMP_INTERVAL: u64 = 64;

/// Instructions between simulated-time advances / 推进模拟时间的指令间隔
const ADVANCE_INTERVAL: u64 = 10_000;

/// Maximum number of logged IRQ deliveries / 记录的中断投递条数上限
const IRQ_LOG_LIMIT: usize = 8;

/// Code window treated as one tight loop / 视为同一紧凑循环的代码窗口大小
const SPIN_WINDOW_BYTES: u32 = 32;

/// Consecutive steps inside one window before a spin warning is printed
/// 打印自旋告警前，PC 连续停留在同一代码窗口内的步数
const SPIN_REPORT_STEPS: u64 = 1_000_000;

/// Maximum number of spin warnings per run / 每次运行最多打印的自旋告警条数
const SPIN_REPORT_LIMIT: usize = 4;

/// Steps retained for the post-mortem PC trace / 停机后回溯保留的步数
const TAIL_TRACE_STEPS: usize = 32;

/// Reason reported when the instruction budget runs out
/// 指令预算耗尽时报告的原因
const BUDGET_REASON: &str = "instruction budget exhausted / 指令预算耗尽";

/// CPU architecture selection for `run` / `run` 的 CPU 架构选择
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchArg {
    /// Arm Cortex-M33 / Arm Cortex-M33
    Arm,
    /// Hazard3 RISC-V / Hazard3 RISC-V
    ///
    /// `riscv` is accepted as an alias so the documented FV-1 command works.
    /// 接受 `riscv` 别名，使文档中的 FV-1 命令可直接运行。
    #[value(alias = "riscv")]
    RiscV,
}

/// Options for a `run` invocation / `run` 调用选项
#[derive(Debug)]
pub struct RunOptions {
    /// Firmware file (UF2 or ELF) / 固件文件（UF2 或 ELF）
    pub file: PathBuf,
    /// Architecture override / 架构覆盖
    pub arch: Option<ArchArg>,
    /// Instruction budget / 指令预算
    pub max_cycles: u64,
    /// Number of steps printed as a boot trace / 作为启动轨迹打印的步数
    pub trace_steps: usize,
    /// Halt on undefined instructions instead of skipping / 遇到未定义指令即停机
    pub strict_decode: bool,
}

/// Construction options shared by `run`, `repl` and `perfbench`
/// `run`、`repl` 与 `perfbench` 共用的机器构造选项
#[derive(Debug, Clone, Copy)]
pub struct MachineOptions {
    /// Architecture override / 架构覆盖
    pub arch: Option<ArchArg>,
    /// Total instruction budget / 总指令预算
    pub max_cycles: u64,
    /// Halt on undefined instructions instead of skipping / 遇到未定义指令即停机
    pub strict_decode: bool,
}

impl Default for MachineOptions {
    fn default() -> Self {
        Self {
            arch: None,
            max_cycles: DEFAULT_RUN_MAX_CYCLES,
            strict_decode: false,
        }
    }
}

/// A booted RP2350 machine that can be stepped by any frontend
/// 可被任意前端单步驱动的已启动 RP2350 机器
///
/// The machine owns the bus, synthetic boot ROM, CPU core, boot peripherals
/// and USB controller. `run` uses it with tracing; `repl` and `perfbench` use
/// the same object from a [`rp2350_usb_host::CdcLink`] cycle callback, so the
/// guest and the USB host bridge always share one timeline.
/// 机器持有总线、合成 boot ROM、CPU 核心、启动外设与 USB 控制器。`run` 使用
/// 它输出轨迹；`repl` 与 `perfbench` 从 [`rp2350_usb_host::CdcLink`] 的周期
/// 回调驱动同一个对象，因此客户机与 USB 主机桥共享同一条时间线。
pub struct Machine {
    /// Address bus with flash, SRAM and peripherals / 含 Flash、SRAM 与外设的地址总线
    pub bus: SimpleBus,
    /// Synthetic boot ROM and trap dispatcher / 合成 boot ROM 与陷阱分发器
    bootrom: Rp2350Bootrom,
    /// CPU core / CPU 核心
    core: Box<dyn CpuCore>,
    /// Boot peripheral handles / 启动外设句柄
    peripherals: BootPeripherals,
    /// USB controller shared with the host bridge / 与主机桥共享的 USB 控制器
    usb: Controller,
    /// Whether boot ROM traps apply to this architecture / boot ROM 陷阱是否适用
    bootrom_applicable: bool,
    /// Image summary printed by `run` / `run` 打印的映像摘要
    image_description: String,
    /// Boot vectors for ARM images / ARM 映像的启动向量
    boot_vectors: Option<(u32, u32)>,
    /// Executed steps (treated as cycles) / 已执行步数（按周期计）
    steps: u64,
    /// Instruction budget / 指令预算
    budget: u64,
    /// Steps already reflected in the peripheral timebase / 已计入外设时基的步数
    advanced: u64,
    /// Steps at the last interrupt pump / 上次中断泵送时的步数
    pumped: u64,
    /// Non-budget halt reason / 非预算原因的停机原因
    halt: Option<String>,
    /// Whether the peripheral timebase has been finalized / 外设时基是否已收尾
    finished: bool,
    /// Logged interrupt deliveries / 已记录的中断投递
    irqs: Vec<String>,
    /// Synchronous trap history (first entries) / 同步陷阱历史（前若干条）
    traps: Vec<String>,
    /// Last recorded synchronous `mcause` / 上次记录的同步 `mcause`
    last_trap_cause: u32,
    /// Buffered UART0 output / 已缓冲的 UART0 输出
    uart: Vec<u8>,
    /// Live notices (trace lines and spin warnings) / 实时通知（轨迹行与自旋告警）
    notices: Vec<String>,
    /// Start of the current spin window / 当前自旋窗口起点
    spin_base: u32,
    /// Consecutive steps inside the current spin window / 当前窗口内的连续步数
    spin_steps: u64,
    /// Number of spin warnings already printed / 已打印的自旋告警条数
    spin_reports: usize,
    /// Ring buffer of recent `(step, PC)` pairs for post-mortem traces
    /// 用于停机回溯的最近 `(步数, PC)` 环形缓冲
    recent: VecDeque<(u64, u32)>,
}

impl Machine {
    /// Build the bus, load firmware and boot the selected core
    /// 构建总线、加载固件并启动所选核心
    ///
    /// # Errors
    /// Returns an error when the firmware cannot be parsed or loaded, when a
    /// bus region cannot be registered, or when the core fails to boot.
    /// 固件无法解析或加载、总线区域注册失败，或核心启动失败时返回错误。
    pub fn new(bytes: &[u8], options: &MachineOptions) -> Result<Self> {
        let mut bus = SimpleBus::new();
        // 0x0000_0000..0x0000_7FFF 是独立的 boot ROM 地址区，与 Flash XIP 无关。
        // 这里挂载合成镜像与宿主陷阱桩，启动向量改为从 Flash XIP 基地址读取。
        // 0x0000_0000..0x0000_7FFF is the boot ROM window, separate from flash XIP.
        // Mount the synthetic image with host trap stubs here; boot vectors are now
        // read from the flash XIP base instead.
        let flash = FlashRegion::rp2350_xip();
        let bootrom = Rp2350Bootrom::new(flash.handle());
        bootrom.mount(&mut bus)?;
        // XIP 在缓存窗口 0x1000_0000 与非缓存别名 0x1400_0000 暴露同一份 Flash，
        // 两个区域共享底层存储。
        // XIP exposes the same flash through the cached window at 0x1000_0000 and
        // the non-cached alias at 0x1400_0000; both regions share one backing store.
        let flash_nocache = flash.alias("flash-xip-nocache", FLASH_XIP_NOCACHE_BASE);
        bus.register_region(Box::new(flash))?;
        bus.register_region(Box::new(flash_nocache))?;
        // 0x1800_0000..0x1C00_0000 是 XIP 维护窗口，写入即请求缓存维护，数据被丢弃。
        // Flash 辅助代码用它清理 XIP SRAM 的缓存行；本模拟器没有缓存模型，
        // 因此这里只吸收写入、读出 0，而不是让它落到 NoDevice。
        // 0x1800_0000..0x1C00_0000 is the XIP maintenance window: writes request
        // cache maintenance and are discarded. Flash helpers use it to clean cache
        // lines covering XIP SRAM; this emulator has no cache model, so the window
        // absorbs writes and reads as zero instead of hitting NoDevice.
        bus.register_region(Box::new(XipMaintenanceRegion::new("xip-maintenance")))?;
        bus.register_region(Box::new(RamRegion::new("sram", SRAM_BASE, SRAM_SIZE)))?;
        let peripherals = BootPeripherals::mount(&mut bus)?;
        // USBCTRL 挂载后，DPSRAM（0x5010_0000）与寄存器窗口（0x5011_0000，含
        // XOR/SET/CLR 别名）由控制器建模，固件对 SIE_CTRL（0x5011_004C）的
        // 读-改-写不再落到 NoDevice。控制器句柄保留到机器生命周期结束，供
        // `repl` / `perfbench` 的主机桥驱动。
        // After mounting USBCTRL, the DPSRAM window at 0x5010_0000 and the register
        // window at 0x5011_0000 (with XOR/SET/CLR aliases) are modelled, so the
        // firmware read-modify-write of SIE_CTRL (0x5011_004C) no longer hits
        // NoDevice. The controller stays alive for the machine lifetime and is
        // driven by the host bridge in `repl` / `perfbench`.
        let usb = rp2350_usb::mount(&mut bus).context("USB mount failed / USB 挂载失败")?;

        let (arch, elf_entry, image_description) = load_image(bytes, &mut bus, options.arch)?;

        let (core, boot_vectors): (Box<dyn CpuCore>, Option<(u32, u32)>) = match arch {
            ArchArg::Arm => {
                let mut arm = CortexM33Core::new(CpuId::Core0);
                arm.set_strict_decode(options.strict_decode);
                let vectors = if let Some(entry) = elf_entry {
                    // ELF 映像没有 Flash 向量表，使用标准 SRAM 栈顶与 ELF 入口。
                    // ELF images have no flash vector table; use the SRAM top and ELF entry.
                    let sp = SRAM_BASE + SRAM_SIZE as u32;
                    arm.boot_at(sp, entry, FLASH_XIP_BASE);
                    Some((sp, entry & !1))
                } else {
                    // UF2 的复位向量表位于 Flash XIP 基地址，VTOR 也指向那里。
                    // UF2 reset vectors live at the flash XIP base, and VTOR follows.
                    arm.boot_from(&mut bus, FLASH_XIP_BASE)
                        .context("core boot failed / 核心启动失败")?;
                    Some((arm.read_register(13), arm.pc()))
                };
                (Box::new(arm), vectors)
            }
            ArchArg::RiscV => {
                let mut riscv = Hazard3Core::new(CpuId::Core0);
                riscv
                    .init(&mut bus)
                    .context("core boot failed / 核心启动失败")?;
                // 同步安装合成 boot ROM 的致命异常向量：cause 2/5 进入宿主
                // 诊断桩，而不是客户机未填充的向量槽。
                // Install the synthetic boot ROM fatal-exception vectors so
                // causes 2/5 reach the host diagnostic stub instead of an
                // unfilled guest vector slot.
                riscv.set_fatal_trap_vector(bootrom.riscv_exception_vector());
                (Box::new(riscv), None)
            }
        };

        let spin_base = core.pc();
        Ok(Self {
            bus,
            bootrom,
            core,
            peripherals,
            usb,
            // 两种架构都使用合成 boot ROM：Arm 走表查询桩，RISC-V 走
            // 0x7DFA 槽位暴露的 rom_func_lookup 桩。
            // Both architectures use the synthetic boot ROM: Arm goes through
            // the table lookup stub, RISC-V through the rom_func_lookup stub
            // exposed at the 0x7DFA slot.
            bootrom_applicable: true,
            image_description,
            boot_vectors,
            steps: 0,
            budget: options.max_cycles,
            advanced: 0,
            pumped: 0,
            halt: None,
            finished: false,
            irqs: Vec::new(),
            traps: Vec::new(),
            // 复位后 mcause 为 0；用 0 作为初值可避免把复位值误记为陷阱。
            // mcause resets to 0; seeding with 0 avoids logging the reset value
            // as a trap.
            last_trap_cause: 0,
            uart: Vec::new(),
            notices: Vec::new(),
            spin_base,
            spin_steps: 0,
            spin_reports: 0,
            recent: VecDeque::with_capacity(TAIL_TRACE_STEPS),
        })
    }

    /// Human-readable image summary / 可读的映像摘要
    #[must_use]
    pub fn image_description(&self) -> &str {
        &self.image_description
    }

    /// Human-readable core description / 可读的核心描述
    #[must_use]
    pub fn core_description(&self) -> String {
        self.core.description()
    }

    /// Boot vectors for ARM images / ARM 映像的启动向量
    #[must_use]
    pub fn boot_vectors(&self) -> Option<(u32, u32)> {
        self.boot_vectors
    }

    /// Current program counter / 当前程序计数器
    #[must_use]
    pub fn pc(&self) -> u32 {
        self.core.pc()
    }

    /// Current stack pointer register / 当前栈指针寄存器
    #[must_use]
    pub fn sp(&self) -> u32 {
        self.core.read_register(13)
    }

    /// Executed steps (also used as simulated cycles) / 已执行步数（同时按周期计）
    #[must_use]
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// Current simulated cycle count / 当前模拟周期数
    #[must_use]
    pub fn now_cycles(&self) -> u64 {
        self.steps
    }

    /// Elapsed simulated microseconds / 已模拟的微秒数
    #[must_use]
    pub fn time_us(&self) -> u64 {
        // 外设时基按 10_000 步批量推进，诊断时把尚未推进的步数也算进来。
        // The peripheral timebase advances in 10_000-step batches; include the
        // not-yet-advanced steps so diagnostics report the current time.
        let pending = self.steps.saturating_sub(self.advanced);
        let pending_us = pending / u64::from(rp2350_periph::boot::DEFAULT_CYCLES_PER_US);
        self.peripherals.time_us().saturating_add(pending_us)
    }

    /// Total instruction budget / 总指令预算
    #[must_use]
    pub fn budget(&self) -> u64 {
        self.budget
    }

    /// Whether the instruction budget has been exhausted / 指令预算是否已耗尽
    #[must_use]
    pub fn budget_exhausted(&self) -> bool {
        self.steps >= self.budget
    }

    /// Non-budget halt reason, if the machine stopped / 非预算原因的停机原因
    #[must_use]
    pub fn halt_reason(&self) -> Option<&str> {
        self.halt.as_deref()
    }

    /// Shared USB controller / 共享 USB 控制器
    #[must_use]
    pub fn usb(&self) -> &Controller {
        &self.usb
    }

    /// Read a core system-control register (test/diagnostic hook)
    /// 读取核心系统控制寄存器（测试与诊断接口）
    #[cfg(test)]
    #[must_use]
    pub fn system_register(&mut self, addr: u32) -> Option<u32> {
        self.core.system_read(addr)
    }

    /// Logged interrupt deliveries / 已记录的中断投递
    #[must_use]
    pub fn irqs(&self) -> &[String] {
        &self.irqs
    }

    /// Synchronous trap history / 同步陷阱历史
    #[must_use]
    pub fn traps(&self) -> &[String] {
        &self.traps
    }

    /// Take buffered UART0 output / 取走已缓冲的 UART0 输出
    pub fn take_uart_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.uart)
    }

    /// Take pending trace lines and spin warnings / 取走待输出的轨迹行与自旋告警
    pub fn take_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notices)
    }

    /// Return the last executed steps with their first halfword
    /// 返回最近执行的若干步及其首个半字
    ///
    /// The ring buffer keeps only `(step, PC)` pairs; opcodes are read lazily
    /// from the bus when a post-mortem trace is requested, so the hot execution
    /// path does not pay for an extra bus access per instruction.
    /// 环形缓冲只保存 `(步数, PC)`；仅在请求回溯轨迹时才从总线读取操作码，
    /// 因此热路径不会因每条指令多一次总线访问而变慢。
    #[must_use]
    pub fn recent_steps(&mut self) -> Vec<(u64, u32, u16)> {
        let recent: Vec<(u64, u32)> = self.recent.iter().copied().collect();
        let mut cycles = 0u64;
        recent
            .into_iter()
            .map(|(step, pc)| {
                let opcode = self.bus.read_u16(pc, &mut cycles).unwrap_or(0xFFFF);
                (step, pc, opcode)
            })
            .collect()
    }

    /// Read a general-purpose register for diagnostics / 为诊断读取通用寄存器
    #[must_use]
    pub fn register(&self, index: u8) -> u32 {
        self.core.read_register(index)
    }

    /// Dump a run of words from the bus, marking unmapped addresses
    /// 从总线转储一段连续字，对未映射地址做标记
    ///
    /// Returns `(address, value_or_marker)` pairs; unmapped reads report
    /// `0xFFFF_FFFF` so a missing region is visible at a glance.
    /// 返回 `(地址, 值或标记)`；未映射读返回 `0xFFFF_FFFF`，便于一眼看出
    /// 缺失区域。
    #[must_use]
    pub fn dump_memory(&mut self, addr: u32, words: usize) -> Vec<(u32, u32)> {
        let mut out = Vec::with_capacity(words);
        let mut cycles = 0u64;
        for i in 0..words {
            let a = addr.wrapping_add((i as u32) * 4);
            match self.bus.read_u32(a, &mut cycles) {
                Ok(value) => out.push((a, value)),
                Err(_) => out.push((a, 0xFFFF_FFFF)),
            }
        }
        out
    }

    /// Remember one executed PC for post-mortem diagnostics
    /// 记录一条已执行 PC 供停机后诊断
    fn record_step(&mut self, pc: u32) {
        if self.recent.len() == TAIL_TRACE_STEPS {
            self.recent.pop_front();
        }
        self.recent.push_back((self.steps, pc));
    }

    /// Record the first synchronous traps for post-mortem diagnostics
    /// 记录最先出现的同步陷阱供停机后诊断
    ///
    /// Interrupts (`mcause[31] == 1`) are excluded: they are normal traffic
    /// during enumeration. Only the first synchronous fault of each cause is
    /// kept, which is enough to locate a silent `ebreak` halt loop.
    /// 中断（`mcause[31] == 1`）被排除：枚举期间属于正常流量。每种原因只保留
    /// 首个同步故障，足以定位静默的 `ebreak` 停机循环。
    fn record_sync_trap(&mut self) {
        let cause = self.core.csr_read(0x342);
        if cause & 0x8000_0000 != 0 || cause == self.last_trap_cause {
            return;
        }
        self.last_trap_cause = cause;
        if self.traps.len() < IRQ_LOG_LIMIT {
            // 最近 PC 序列给出进入故障前的调用链，单看 mepc 无法区分「直接执行
            // ebreak」还是「某个断言函数跳到 ebreak」。
            // The recent PC sequence shows the call chain into the fault; mepc
            // alone cannot distinguish a direct ebreak from a panic helper.
            let history = self
                .recent
                .iter()
                .rev()
                .take(TAIL_TRACE_STEPS)
                .map(|(_, pc)| format!("{pc:#010X}"))
                .collect::<Vec<_>>()
                .join(" <- ");
            self.traps.push(format!(
                "[trap] step={} cause={cause:#X} mepc={:#010X} mtval={:#010X} hist={history}",
                self.steps,
                self.core.csr_read(0x341),
                self.core.csr_read(0x343)
            ));
        }
    }

    /// Execute one instruction (or one boot ROM trap)
    /// 执行一条指令（或一次 boot ROM 陷阱）
    ///
    /// # Errors
    /// Propagates bus errors raised while dispatching a boot ROM trap. CPU
    /// execution errors are captured as a halt reason so `run` keeps its
    /// historical "report and stop" behaviour.
    /// 传播 boot ROM 陷阱分发期间的总线错误。CPU 执行错误会记录为停机原因，
    /// 以保持 `run` 既有的“报告后停止”行为。
    pub fn step(&mut self) -> Result<CpuState> {
        self.execute_step(None)
    }

    /// Execute one instruction with a `run`-style trace line
    /// 执行一条指令并生成 `run` 风格的轨迹行
    ///
    /// # Errors
    /// Same conditions as [`Self::step`]. / 错误条件与 [`Self::step`] 相同。
    pub fn trace_step(&mut self, index: usize) -> Result<CpuState> {
        self.execute_step(Some(index))
    }

    /// Advance the guest by at most `cycles` instructions
    /// 最多推进 `cycles` 条指令
    ///
    /// This is the callback used by [`rp2350_usb_host::CdcLink`]: it runs the
    /// same boot-ROM dispatch, interrupt pump, timebase advance and UART
    /// collection as the `run` path, and stops at the machine budget.
    /// 这是 [`rp2350_usb_host::CdcLink`] 使用的回调：执行与 `run` 路径相同的
    /// boot ROM 分发、中断泵送、时基推进与 UART 收集，并在机器预算处停止。
    pub fn run_cycles(&mut self, cycles: u64) {
        let target = self.steps.saturating_add(cycles).min(self.budget);
        while self.steps < target && self.halt.is_none() {
            match self.step() {
                Ok(CpuState::Running) => {}
                Ok(_) => break,
                Err(error) => {
                    if self.halt.is_none() {
                        self.halt = Some(error.to_string());
                    }
                    self.finalize();
                    break;
                }
            }
        }
    }

    /// Flush pending simulated time and UART bytes / 收尾模拟时间与 UART 字节
    pub fn finalize(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.advance_timebase();
        self.uart.extend(self.peripherals.take_uart_output());
    }

    fn execute_step(&mut self, trace: Option<usize>) -> Result<CpuState> {
        if self.halt.is_some() || self.budget_exhausted() {
            return Ok(CpuState::Halted);
        }
        let pc = self.core.pc();
        self.record_step(pc);
        if self.bootrom_applicable && self.bootrom.handles(pc) {
            if let Some(index) = trace {
                self.notices
                    .push(format!("[trace] {index:03} PC={pc:#010X} bootrom-trap"));
            }
            if let Some(outcome) = self.bootrom.dispatch(&mut *self.core, &mut self.bus)? {
                self.steps += 1;
                self.record_sync_trap();
                if let Some(halt) = outcome.halt {
                    self.halt = Some(format!("bootrom halt: {halt}"));
                    self.finalize();
                    return Ok(CpuState::Halted);
                }
                // 陷阱步与原 run 路径一致：立即泵送中断，不在此处推进时基。
                // Trap steps mirror the original run path: pump interrupts now and
                // do not advance the timebase here.
                pump(
                    &mut *self.core,
                    &self.peripherals,
                    &self.usb,
                    &mut self.irqs,
                );
                return Ok(CpuState::Running);
            }
        }
        if let Some(index) = trace {
            let mut cycles = 0u64;
            let opcode = self.bus.read_u16(pc, &mut cycles).unwrap_or(0xFFFF);
            self.notices
                .push(format!("[trace] {index:03} PC={pc:#010X} op={opcode:#06X}"));
        }
        match self.core.step(&mut self.bus) {
            Ok(CpuState::Running) => {
                self.steps += 1;
                self.record_sync_trap();
                if trace.is_some() {
                    // 轨迹步与原 run 路径一致：每步泵送中断，不做自旋统计或时基推进。
                    // Trace steps mirror the original run path: pump every step and
                    // skip spin accounting and timebase advancement.
                    pump(
                        &mut *self.core,
                        &self.peripherals,
                        &self.usb,
                        &mut self.irqs,
                    );
                } else {
                    self.after_running_step();
                }
                Ok(CpuState::Running)
            }
            Ok(state) => {
                self.halt = Some(format!("core entered state {state:?}"));
                self.finalize();
                Ok(state)
            }
            Err(error) => {
                self.halt = Some(error.to_string());
                self.finalize();
                Ok(CpuState::Faulted)
            }
        }
    }

    fn after_running_step(&mut self) {
        // 紧凑自旋通常意味着固件在轮询一个永远不变的 MMIO 值（未建模设备静默
        // 读 0）或执行延时循环，会无声耗尽指令预算。PC 连续停在同一小窗口内
        // 达到阈值就告警一次，并给出窗口范围以便反汇编定位。
        // A tight spin usually means the firmware polls a register that never
        // changes (unmodelled devices read zero) or runs a delay loop, silently
        // burning the budget. Warn once when the PC stays inside one small window
        // and print the window so the spin point can be disassembled.
        let pc = self.core.pc();
        if pc.wrapping_sub(self.spin_base) < SPIN_WINDOW_BYTES {
            self.spin_steps += 1;
            if self.spin_steps == SPIN_REPORT_STEPS && self.spin_reports < SPIN_REPORT_LIMIT {
                self.spin_reports += 1;
                let registers = match self.core.arch() {
                    rp2350_cpu::CpuArch::RiscVHazard3 => format!(
                        "ra={:#010X} sp={:#010X} s0={:#010X} s1={:#010X} \
                         a0={:#010X} a1={:#010X} a2={:#010X} a3={:#010X} \
                         a4={:#010X} a5={:#010X}",
                        self.core.read_register(1),
                        self.core.read_register(2),
                        self.core.read_register(8),
                        self.core.read_register(9),
                        self.core.read_register(10),
                        self.core.read_register(11),
                        self.core.read_register(12),
                        self.core.read_register(13),
                        self.core.read_register(14),
                        self.core.read_register(15),
                    ),
                    rp2350_cpu::CpuArch::ArmCortexM33 => format!(
                        "r0={:#010X} r1={:#010X} r2={:#010X} r3={:#010X} sp={:#010X}",
                        self.core.read_register(0),
                        self.core.read_register(1),
                        self.core.read_register(2),
                        self.core.read_register(3),
                        self.core.read_register(13),
                    ),
                };
                self.notices.push(format!(
                    "[run] tight spin: PC stayed in {:#010X}..{:#010X} for {} steps, \
                     likely polling a value that never changes / 疑似轮询等待; {registers}",
                    self.spin_base,
                    self.spin_base + SPIN_WINDOW_BYTES,
                    self.spin_steps
                ));
            }
        } else {
            self.spin_base = pc;
            self.spin_steps = 0;
        }
        // 系统控制寄存器写入必须在下一条指令前生效：固件写 CPACR 后紧跟
        // `mrc p4`/VFP 指令，按 64 步批量回放会让核误判协处理器未使能并触发
        // NOCP UsageFault。中断线保持节流，避免电平中断被每一步重复投递。
        // System-control writes must land before the next instruction: firmware
        // writes CPACR immediately before `mrc p4`/VFP instructions, so a
        // 64-step batch replay makes the core raise a spurious NOCP UsageFault.
        // IRQ lines stay throttled to avoid re-asserting a level IRQ each step.
        replay_system_writes(&mut *self.core, &self.peripherals);
        if self.steps - self.pumped >= PUMP_INTERVAL {
            self.peripherals.service_core1_peer();
            deliver_irqs(
                &mut *self.core,
                &self.peripherals,
                &self.usb,
                &mut self.irqs,
            );
            self.pumped = self.steps;
        }
        if self.steps - self.advanced >= ADVANCE_INTERVAL {
            self.advance_timebase();
        }
    }

    fn advance_timebase(&mut self) {
        if self.steps > self.advanced {
            self.peripherals.advance(self.steps - self.advanced);
            self.advanced = self.steps;
            self.uart.extend(self.peripherals.take_uart_output());
        }
    }
}

/// Build the bus, load firmware and execute until halt or budget exhaustion
/// 构建总线、加载固件并执行到停机或预算耗尽
///
/// # Errors
/// Returns an error when the firmware cannot be read, parsed or loaded, when a
/// bus region cannot be registered, or when the core fails to boot.
/// 固件无法读取、解析或加载，总线区域注册失败，或核心启动失败时返回错误。
pub fn run(options: &RunOptions) -> Result<()> {
    let bytes = std::fs::read(&options.file).with_context(|| {
        format!(
            "cannot read firmware / 无法读取固件: {}",
            options.file.display()
        )
    })?;

    let machine_options = MachineOptions {
        arch: options.arch,
        max_cycles: options.max_cycles,
        strict_decode: options.strict_decode,
    };
    let mut machine = Machine::new(&bytes, &machine_options)?;

    let mut stdout = std::io::stdout();
    writeln!(stdout, "[run] {}", machine.image_description())?;
    writeln!(stdout, "[run] core: {}", machine.core_description())?;
    if let Some((sp, pc)) = machine.boot_vectors() {
        writeln!(stdout, "[run] boot vectors: SP={sp:#010X} PC={pc:#010X}")?;
    }
    writeln!(
        stdout,
        "[run] started: PC={:#010X} SP={:#010X}",
        machine.pc(),
        machine.sp()
    )?;

    for index in 0..options.trace_steps {
        let state = machine.trace_step(index)?;
        drain_notices(&mut machine, &mut stdout)?;
        if state != CpuState::Running {
            break;
        }
    }
    while machine.halt_reason().is_none() && !machine.budget_exhausted() {
        machine.step()?;
        drain_notices(&mut machine, &mut stdout)?;
    }
    machine.finalize();

    let reason = machine
        .halt_reason()
        .map_or_else(|| BUDGET_REASON.to_string(), str::to_owned);
    writeln!(stdout, "[run] halt: {reason}")?;
    for (step, pc, opcode) in machine.recent_steps() {
        writeln!(stdout, "[tail] {step:06} PC={pc:#010X} op={opcode:#06X}")?;
    }
    writeln!(
        stdout,
        "[run] executed {steps} steps, {us} us simulated, final PC={pc:#010X}",
        steps = machine.steps(),
        us = machine.time_us(),
        pc = machine.pc()
    )?;
    writeln!(stdout, "[run] {}", register_snapshot(&machine))?;
    if !machine.irqs().is_empty() {
        writeln!(stdout, "[run] irq deliveries (first {IRQ_LOG_LIMIT}):")?;
        for line in machine.irqs() {
            writeln!(stdout, "{line}")?;
        }
    }
    let uart = machine.take_uart_output();
    writeln!(stdout, "[run] uart0: {} bytes", uart.len())?;
    if !uart.is_empty() {
        writeln!(stdout, "[run] uart0 output:")?;
        stdout.write_all(&uart)?;
        if !uart.ends_with(b"\n") {
            writeln!(stdout)?;
        }
    }
    stdout.flush()?;
    Ok(())
}

/// Format a one-line register snapshot for post-mortem diagnostics
/// 生成停机后寄存器快照的单行文本
///
/// A bad indirect branch is far easier to diagnose when the registers that
/// produced the target are visible. Some emulator faults (for example an
/// ARM-state `BLX` target) report the *branch target* as a "bus address", so
/// the register file is the only way to tell a memory fault from an INVSTATE.
/// 错误的间接跳转若能同时看到产生目标的寄存器就很容易定位。部分模拟器故障
/// （例如 `BLX` 跳到 ARM 态地址）会把「跳转目标」当作「总线地址」上报，
/// 只有寄存器现场才能区分内存故障与 INVSTATE。
#[must_use]
pub fn register_snapshot(machine: &Machine) -> String {
    let mut parts = Vec::with_capacity(18);
    match machine.core.arch() {
        rp2350_cpu::CpuArch::RiscVHazard3 => {
            for index in 0..16u8 {
                parts.push(format!(
                    "X{index}={:#010X}",
                    machine.core.read_register(index)
                ));
            }
            parts.push(format!("SP={:#010X}", machine.core.read_register(2)));
            parts.push(format!("RA={:#010X}", machine.core.read_register(1)));
            parts.push(format!("PC={:#010X}", machine.core.pc()));
            parts.push(format!("MTVEC={:#010X}", machine.core.csr_read(0x305)));
            parts.push(format!("MCAUSE={:#010X}", machine.core.csr_read(0x342)));
            parts.push(format!("MEPC={:#010X}", machine.core.csr_read(0x341)));
            parts.push(format!("MTVAL={:#010X}", machine.core.csr_read(0x343)));
        }
        rp2350_cpu::CpuArch::ArmCortexM33 => {
            for index in 0..13u8 {
                parts.push(format!(
                    "R{index}={:#010X}",
                    machine.core.read_register(index)
                ));
            }
            parts.push(format!("SP={:#010X}", machine.core.read_register(13)));
            parts.push(format!("LR={:#010X}", machine.core.read_register(14)));
            parts.push(format!("PC={:#010X}", machine.core.pc()));
        }
    }
    format!("[regs] {}", parts.join(" "))
}

/// Dump `count` words starting at `addr` as one diagnostic line
/// 从 `addr` 起转储 `count` 个字，输出为单行诊断
///
/// Unmapped words print as `????` instead of aborting, so the dump stays
/// useful when the fault address itself is bogus.
/// 未映射的字打印为 `????` 而不是中断转储，因此即使出错地址本身非法也仍可用。
fn memory_line(machine: &mut Machine, label: &str, addr: u32, count: usize) -> String {
    let mut words = Vec::with_capacity(count);
    for index in 0..count {
        let slot = addr.wrapping_add((index as u32) * 4);
        let mut cycles = 0u64;
        match machine.bus.read_u32(slot, &mut cycles) {
            Ok(value) => words.push(format!("{value:08X}")),
            Err(_) => words.push(String::from("????????")),
        }
    }
    format!("[mem] {label} {addr:#010X}: {}", words.join(" "))
}

/// Format the post-mortem PC history for frontends that stop on a fault
/// 为因故障停机的前端格式化停机前的 PC 轨迹
///
/// `repl` and `perfbench` do not print a step trace, so without this hook a
/// guest-side bus fault is only visible as a final PC. Showing the last
/// [`TAIL_TRACE_STEPS`] instructions makes the offending branch locatable, the
/// register snapshot shows how its operands were produced, and the memory dump
/// of `R4`/`SP` reveals corrupted structures.
/// `repl` 与 `perfbench` 不输出逐步轨迹，客户机总线故障只剩一个最终 PC；
/// 打印停机前 [`TAIL_TRACE_STEPS`] 条指令即可定位罪魁分支，寄存器快照说明
/// 操作数从何而来，`R4`/`SP` 的内存转储则暴露被破坏的结构体。
pub fn fault_tail_lines(machine: &mut Machine) -> Vec<String> {
    let (object, stack, object_label) = match machine.core.arch() {
        // RISC-V has no r4 frame pointer; s0 (x8) is the usual object base
        // saved across calls, and x2 is the stack pointer.
        // RISC-V 没有 r4 帧指针；跨调用保存的对象基址通常是 s0（x8），
        // 栈指针是 x2。
        rp2350_cpu::CpuArch::RiscVHazard3 => (
            machine.core.read_register(8),
            machine.core.read_register(2),
            "S0",
        ),
        rp2350_cpu::CpuArch::ArmCortexM33 => (
            machine.core.read_register(4),
            machine.core.read_register(13),
            "R4",
        ),
    };
    let mut lines = vec![register_snapshot(machine)];
    lines.push(memory_line(machine, object_label, object, 12));
    lines.push(memory_line(machine, "SP", stack, 12));
    // 若对象首字是 SRAM 指针，再转储它指向的锁/状态字。
    // If the object's first word is an SRAM pointer, dump the lock/state word
    // it points at.
    let mut cycles = 0u64;
    let pointer = machine
        .bus
        .read_u32(object, &mut cycles)
        .unwrap_or(0xFFFF_FFFF);
    let target = pointer & !3;
    if target & 0xFFF0_0000 == 0x2000_0000 {
        lines.push(memory_line(machine, "OBJ>", target, 8));
    }
    lines.extend(
        machine
            .recent_steps()
            .into_iter()
            .map(|(step, pc, opcode)| format!("[tail] {step:06} PC={pc:#010X} op={opcode:#06X}")),
    );
    lines
}

/// Write queued trace lines and spin warnings / 输出排队的轨迹行与自旋告警
fn drain_notices(machine: &mut Machine, stdout: &mut dyn Write) -> Result<()> {
    for notice in machine.take_notices() {
        writeln!(stdout, "{notice}")?;
    }
    Ok(())
}

/// Detect the format, load it and return its architecture and summary
/// 检测格式、加载映像并返回其架构与摘要
fn load_image(
    bytes: &[u8],
    bus: &mut SimpleBus,
    override_arch: Option<ArchArg>,
) -> Result<(ArchArg, Option<u32>, String)> {
    if bytes.starts_with(&UF2_MAGIC) {
        let image = Uf2Image::from_bytes(bytes).context("UF2 parse failed / UF2 解析失败")?;
        let family = image.family();
        let detected = family.arch().map_or(ArchArg::Arm, |arch| match arch {
            Uf2Arch::Arm => ArchArg::Arm,
            Uf2Arch::RiscV => ArchArg::RiscV,
        });
        let arch = override_arch.unwrap_or(detected);
        let report = image
            .load_into(bus)
            .context("UF2 load failed / UF2 加载失败")?;
        let description = format!(
            "uf2: family={family} blocks={}/{} flash={} sram={} range={:#010X}..{:#010X}",
            report.blocks_written,
            report.blocks_total,
            report.wrote_flash,
            report.wrote_sram,
            report.min_address,
            report.max_address
        );
        Ok((arch, None, description))
    } else {
        let image =
            FirmwareImage::from_elf_bytes(bytes).context("ELF parse failed / ELF 解析失败")?;
        image
            .load_into(bus)
            .context("ELF load failed / ELF 加载失败")?;
        let arch = override_arch.unwrap_or(ArchArg::Arm);
        let description = format!(
            "elf: entry={:#010X} segments={}",
            image.entry_point,
            image.segments.len()
        );
        Ok((arch, Some(image.entry_point), description))
    }
}

/// Replay PPB system writes and deliver asserted interrupt lines
/// 将 PPB 系统写入回放到核，并投递已断言的中断线
///
/// Used on boot ROM traps and traced steps, where the original `run` loop
/// pumped every step. The untraced hot path uses the two halves separately so
/// system writes take effect immediately while IRQ lines stay throttled.
/// 用于 boot ROM 陷阱与轨迹步（原 `run` 循环每步都泵送）。无轨迹的热路径分开
/// 调用两半：系统写入立即生效，中断线保持节流。
fn pump(
    core: &mut dyn CpuCore,
    peripherals: &BootPeripherals,
    usb: &Controller,
    irq_log: &mut Vec<String>,
) {
    peripherals.service_core1_peer();
    replay_system_writes(core, peripherals);
    deliver_irqs(core, peripherals, usb, irq_log);
}

/// Replay PPB system writes into the core
/// 将 PPB 系统写入回放到核
fn replay_system_writes(core: &mut dyn CpuCore, peripherals: &BootPeripherals) {
    for (addr, value) in peripherals.take_system_writes() {
        core.system_write(addr, value);
    }
}

/// Deliver asserted interrupt lines / 投递已断言的中断线
fn deliver_irqs(
    core: &mut dyn CpuCore,
    peripherals: &BootPeripherals,
    usb: &Controller,
    irq_log: &mut Vec<String>,
) {
    let mask = peripherals.irq_mask();
    for irq in 0..64u16 {
        if mask & (1u64 << irq) == 0 {
            continue;
        }
        core.signal_interrupt(irq);
        log_irq(irq_log, irq, core.pc());
    }
    // USB 控制器把 INTR & INTE 聚合到 INTS；固件清 INTS 后条件自然撤销。
    // The USB controller aggregates INTR & INTE into INTS; the condition
    // disappears when firmware clears INTS.
    if usb.irq_asserted() {
        core.signal_interrupt(USBCTRL_IRQ);
        log_irq(irq_log, USBCTRL_IRQ, core.pc());
    }
}

/// Record the latest interrupt deliveries in a bounded ring
/// 在有界环形缓冲中记录最近的中断投递
fn log_irq(irq_log: &mut Vec<String>, irq: u16, pc: u32) {
    if irq_log.len() == IRQ_LOG_LIMIT {
        irq_log.remove(0);
    }
    irq_log.push(format!("[irq] signal {irq} at PC={pc:#010X}"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{cpacr_write_elf, infinite_loop_elf};

    /// `SCB_CPACR` absolute address / `SCB_CPACR` 绝对地址
    const SCB_CPACR: u32 = 0xE000_ED88;

    #[test]
    fn machine_boots_and_stops_at_the_budget() {
        let elf = infinite_loop_elf();
        let options = MachineOptions {
            arch: Some(ArchArg::Arm),
            max_cycles: 5,
            strict_decode: false,
        };
        let mut machine = Machine::new(&elf, &options).expect("machine");
        assert_eq!(
            machine.boot_vectors(),
            Some((SRAM_BASE + SRAM_SIZE as u32, 0x1000_0000))
        );
        assert_eq!(machine.pc(), 0x1000_0000);

        machine.run_cycles(3);
        assert_eq!(machine.steps(), 3);
        assert!(!machine.budget_exhausted());

        machine.run_cycles(100);
        assert_eq!(machine.steps(), 5);
        assert!(machine.budget_exhausted());
        assert_eq!(machine.halt_reason(), None);

        machine.run_cycles(100);
        assert_eq!(machine.steps(), 5);
        assert_eq!(machine.pc(), 0x1000_0000);
        assert_eq!(machine.time_us(), 0);
    }

    #[test]
    fn trace_step_reports_the_branch_opcode() {
        let elf = infinite_loop_elf();
        let options = MachineOptions {
            arch: Some(ArchArg::Arm),
            max_cycles: 10,
            strict_decode: false,
        };
        let mut machine = Machine::new(&elf, &options).expect("machine");
        let state = machine.trace_step(0).expect("trace step");
        assert_eq!(state, CpuState::Running);
        let notices = machine.take_notices();
        assert_eq!(notices.len(), 1);
        assert!(notices[0].starts_with("[trace] 000 PC=0x10000000 op=0xE7FE"));
    }

    #[test]
    fn recent_steps_keep_a_bounded_pc_history() {
        let elf = infinite_loop_elf();
        let options = MachineOptions {
            arch: Some(ArchArg::Arm),
            max_cycles: 40,
            strict_decode: false,
        };
        let mut machine = Machine::new(&elf, &options).expect("machine");
        machine.run_cycles(40);
        let tail = machine.recent_steps();
        // 预算小于回溯窗口时，缓冲只保留实际执行过的步数。
        // When the budget is smaller than the trace window, the ring buffer
        // only holds the steps that actually ran.
        let expected = usize::try_from(machine.steps())
            .unwrap()
            .min(TAIL_TRACE_STEPS);
        assert_eq!(tail.len(), expected);
        assert_eq!(tail[0].0, machine.steps() - expected as u64);
        assert!(
            tail.iter()
                .all(|&(_, pc, opcode)| pc == 0x1000_0000 && opcode == 0xE7FE)
        );
    }

    #[test]
    fn ppb_writes_reach_the_core_before_the_next_instruction() {
        // 固件在写 CPACR 后紧接协处理器指令；若系统写入按 64 步批量回放，
        // 这里第 3 步的写入要到第 64 步才生效，核内 CPACR 仍是上电默认值。
        // Firmware writes CPACR right before a coprocessor instruction. With a
        // 64-step batch replay, the step-3 write would not land until step 64
        // and the in-core CPACR would still hold its power-on default.
        let elf = cpacr_write_elf();
        let options = MachineOptions {
            arch: Some(ArchArg::Arm),
            max_cycles: 8,
            strict_decode: false,
        };
        let mut machine = Machine::new(&elf, &options).expect("machine");
        assert_eq!(
            machine.system_register(SCB_CPACR),
            Some(0x00F0_0000),
            "power-on CPACR must model the boot ROM's FPU enable / 上电 CPACR 应复现 boot ROM 的 FPU 使能"
        );

        machine.run_cycles(4);
        assert_eq!(machine.steps(), 4);
        assert_eq!(machine.halt_reason(), None);
        assert_eq!(
            machine.system_register(SCB_CPACR),
            Some(0x0030_0303),
            "CPACR write must be replayed on the very next step / CPACR 写入必须在下一步前回放"
        );
    }

    #[test]
    fn sram_top_is_mapped_for_exception_stack_frames() {
        // MicroPython 把 MSP 放在 520 KB SRAM 顶端，软复位/异常压栈会访问
        // 顶端下方几百字节。若某个窗口抢先覆盖或 SRAM 尺寸算错，固件会在
        // raw REPL 握手阶段莫名报 NoDevice。
        // MicroPython parks MSP at the top of the 520 KB SRAM, so soft resets
        // and exception stacking touch the few hundred bytes below the top.
        // A clobbering window or a wrong SRAM size would surface as a spurious
        // NoDevice during the raw REPL handshake.
        use rp2350_core::bus::AddressBus;

        let elf = infinite_loop_elf();
        let machine = Machine::new(&elf, &MachineOptions::default()).expect("machine");
        let mut bus = machine.bus;
        let top = SRAM_BASE + SRAM_SIZE as u32;
        for addr in [top - 0x108, top - 0x20, top - 4] {
            let region = bus
                .find_region_mut(addr)
                .unwrap_or_else(|| panic!("no region for {addr:#010X}"));
            assert_eq!(region.name(), "sram", "unexpected owner of {addr:#010X}");
            let mut cycles = 0;
            bus.write_u32(addr, 0xA5A5_A5A5, &mut cycles)
                .unwrap_or_else(|e| panic!("write {addr:#010X}: {e:?}"));
            assert_eq!(bus.read_u32(addr, &mut cycles).expect("read"), 0xA5A5_A5A5);
        }
    }

    #[test]
    fn image_summary_keeps_the_run_format() {
        let elf = infinite_loop_elf();
        let machine = Machine::new(&elf, &MachineOptions::default()).expect("machine");
        assert!(
            machine
                .image_description()
                .starts_with("elf: entry=0x10000001")
        );
        assert_eq!(machine.boot_vectors().map(|(_, pc)| pc), Some(0x1000_0000));
    }

    /// Build one 512-byte UF2 block with a family ID / 构造一个带 family ID 的 UF2 块
    fn uf2_block(family: u32, address: u32, payload: &[u8]) -> Vec<u8> {
        let mut block = vec![0u8; 512];
        block[0..4].copy_from_slice(&0x0A32_4655u32.to_le_bytes());
        block[4..8].copy_from_slice(&0x9E5D_5157u32.to_le_bytes());
        // Family-ID-present flag. / 置位 family ID 存在标志。
        block[8..12].copy_from_slice(&0x0000_2000u32.to_le_bytes());
        block[12..16].copy_from_slice(&address.to_le_bytes());
        block[16..20].copy_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
        block[20..24].copy_from_slice(&0u32.to_le_bytes());
        block[24..28].copy_from_slice(&1u32.to_le_bytes());
        block[28..32].copy_from_slice(&family.to_le_bytes());
        block[32..32 + payload.len()].copy_from_slice(payload);
        block[508..512].copy_from_slice(&0x0AB1_6F30u32.to_le_bytes());
        block
    }

    #[test]
    fn uf2_family_selects_the_cpu_architecture() {
        // RISC-V family must auto-select Hazard3; the explicit override wins.
        // RISC-V family 必须自动选择 Hazard3；显式覆盖优先。
        let riscv = uf2_block(0xE48B_FF5A, FLASH_XIP_BASE, &[0u8; 4]);
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(RamRegion::new("flash", FLASH_XIP_BASE, 0x1000)))
            .expect("flash region");
        let (arch, entry, description) = load_image(&riscv, &mut bus, None).expect("load");
        assert_eq!(arch, ArchArg::RiscV);
        assert_eq!(entry, None);
        assert!(description.contains("RISC-V"));
        let (arch, _, _) = load_image(&riscv, &mut bus, Some(ArchArg::Arm)).expect("load override");
        assert_eq!(arch, ArchArg::Arm);

        let arm = uf2_block(0xE48B_FF59, FLASH_XIP_BASE, &[0u8; 4]);
        let (arch, _, _) = load_image(&arm, &mut bus, None).expect("load arm");
        assert_eq!(arch, ArchArg::Arm);
    }
}
