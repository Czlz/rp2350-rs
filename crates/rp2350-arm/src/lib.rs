//! rp2350-arm — ARM Cortex-M33 processor emulation
//!
//! Implements the ARMv8-M architecture: Thumb-16/32 instruction sets,
//! NVIC interrupt controller, exception handling, SysTick timer, and
//! debug components (FPB, DWT).
//!
//! 实现 ARMv8-M 架构：Thumb-16/32 指令集、NVIC 中断控制器、
//! 异常处理、SysTick 定时器和调试组件（FPB、DWT）。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]
// Cortex-M33 核心集成文件涉及 MMIO 寄存器仿真和类型转换，
// 这些是硬件仿真层的固有特性。
// Core integration file involves MMIO register emulation and casts
// inherent to hardware emulation layers.
#![allow(
    clippy::doc_markdown,
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
    clippy::identity_op,
    clippy::module_name_repetitions,
    clippy::missing_errors_doc,
    // 核心状态机用多个布尔标志描述等待/事件/缓存开关，这些是硬件状态的直接映射，
    // 拆成枚举反而会让 `step` 热路径多一次匹配。
    // The core state machine uses several boolean flags for wait/event/cache
    // switches; they map hardware bits directly, and folding them into enums
    // would add a match to the `step` hot path.
    clippy::struct_excessive_bools
)]

pub mod conditions;
pub mod decode_cache;
pub mod dual_core;
pub mod exceptions;
pub mod execute_thumb16;
pub mod execute_thumb32;
mod fetch_cache;
pub mod fpu;
pub mod nvic;
pub mod registers;
pub mod scr;
mod step_cached;
mod system_access;
pub mod systick;

/// Cortex-M33 register set / Cortex-M33 寄存器组
pub use registers::CortexM33Registers;

/// Dual-core scheduling scaffold / 双核调度脚手架
pub use dual_core::DualCoreM33;

use std::collections::BTreeSet;

use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;
use rp2350_core::types::{CpuId, CycleCount, IrqNumber};
use rp2350_cpu::{CpuArch, CpuCore, CpuState};

use conditions::ITBlockState;
use exceptions::EXC_SYSTICK;
use execute_thumb16::{SleepKind, StepResult, execute_thumb16};
use execute_thumb32::execute_thumb32;
use nvic::Nvic;
use systick::SysTick;

#[allow(dead_code)]
const PPB_BASE: u32 = 0xE000_E000;
#[allow(dead_code)]
const SYSTICK_BASE: u32 = 0xE000_E010;
#[allow(dead_code)]
const NVIC_BASE: u32 = 0xE000_E100;

/// `SCB_VTOR` absolute address / `SCB_VTOR` 绝对地址
const SCB_VTOR: u32 = 0xE000_ED08;

/// `SCB_CPACR` absolute address / `SCB_CPACR` 绝对地址
const SCB_CPACR: u32 = 0xE000_ED88;

/// `SCB_CFSR` absolute address / `SCB_CFSR` 绝对地址
const SCB_CFSR: u32 = 0xE000_ED28;

/// First exception number that maps to an external interrupt
/// 映射到外部中断的第一个异常号
const FIRST_EXTERNAL_EXCEPTION: u32 = 16;

/// Address of the initial stack pointer in the boot vector table
/// 启动向量表中初始栈指针的地址
const BOOT_SP_ADDR: u32 = 0x0000_0000;

/// Why a core is in [`CpuState::Sleeping`] / 核心处于 [`CpuState::Sleeping`] 的原因
///
/// The reason decides what can wake the core and is reported to frontends so a
/// sleeping machine can be diagnosed without guessing.
/// 原因决定什么能唤醒核心，并报告给前端，使睡眠中的机器无需猜测即可诊断。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepReason {
    /// `WFI` with `SLEEPDEEP = 0` and no interrupt pending
    /// `SLEEPDEEP = 0` 且无挂起中断时的 `WFI`
    WaitForInterrupt,
    /// `WFE` with `SLEEPDEEP = 0` and the event register clear
    /// `SLEEPDEEP = 0` 且事件寄存器为空时的 `WFE`
    WaitForEvent,
    /// `WFI`/`WFE` with `SCR.SLEEPDEEP = 1`
    /// `SCR.SLEEPDEEP = 1` 时的 `WFI`/`WFE`
    ///
    /// There is no separately powered domain in this model, so the core still
    /// enters [`CpuState::Sleeping`]. The payload keeps the instruction's wait
    /// condition: `wait_for_event = true` is a deep-sleeping `WFE`, `false` a
    /// deep-sleeping `WFI`. Hosts use the variant to report deep sleep without
    /// confusing it with an ordinary wait.
    /// 本模型没有独立掉电域，因此核心仍进入 [`CpuState::Sleeping`]。载荷保留指令的
    /// 等待条件：`wait_for_event = true` 为深度睡眠的 `WFE`，`false` 为深度睡眠的
    /// `WFI`。宿主可用该变体上报深度睡眠，而不会与普通等待混淆。
    SleepDeep {
        /// The instruction waits for an event (`WFE`) instead of an interrupt
        /// 该指令等待事件（`WFE`）而非中断
        wait_for_event: bool,
    },
    /// `SCR.SLEEPONEXIT` sleep entered by an exception return to Thread mode
    /// 由异常返回到 Thread 模式触发的 `SCR.SLEEPONEXIT` 睡眠
    ///
    /// The wake condition is the `WFI` one: any enabled pending interrupt
    /// resumes the core, while an event alone never does. `deep` mirrors
    /// `SCR.SLEEPDEEP` at the moment of the return.
    /// 唤醒条件与 `WFI` 相同：任何已使能的挂起中断都会恢复核心，单独的事件不会。
    /// `deep` 反映异常返回瞬间的 `SCR.SLEEPDEEP`。
    SleepOnExit {
        /// Whether `SCR.SLEEPDEEP` also requested deep sleep
        /// 是否同时由 `SCR.SLEEPDEEP` 请求深度睡眠
        deep: bool,
    },
    /// A load hit a retryable bus stall (blocking FIFO read)
    /// 载入遇到可重试的总线 stall（阻塞 FIFO 读）
    ///
    /// The instruction did **not** retire: `PC` and every register keep their
    /// pre-instruction values, so the exact same load is replayed once the
    /// device can serve it.
    /// 该指令**未**退役：`PC` 与所有寄存器保持指令执行前的值，因此设备能服务时重放的
    /// 是完全相同的载入。
    BlockedRead,
}

impl SleepReason {
    /// Whether the reason describes `SCR.SLEEPDEEP` deep sleep
    /// 该原因是否描述 `SCR.SLEEPDEEP` 深度睡眠
    ///
    /// A `SLEEPONEXIT` sleep counts as deep only when `SLEEPDEEP` was set at the
    /// returning instruction. The electrical effect of deep sleep (power-down,
    /// clock gating) is not modeled; only the reason is distinguished.
    /// 只有异常返回瞬间 `SLEEPDEEP` 置位的 `SLEEPONEXIT` 睡眠才算深度睡眠。深度睡眠的
    /// 电气效果（掉电、时钟门控）未建模，仅区分原因。
    #[must_use]
    pub fn is_deep_sleep(self) -> bool {
        matches!(
            self,
            Self::SleepDeep { .. } | Self::SleepOnExit { deep: true }
        )
    }

    /// Whether the reason waits for the event register (`WFE` semantics)
    /// 该原因是否等待事件寄存器（`WFE` 语义）
    fn waits_for_event(self) -> bool {
        matches!(
            self,
            Self::WaitForEvent
                | Self::SleepDeep {
                    wait_for_event: true
                }
        )
    }
}

/// Cortex-M33 processor core
/// Cortex-M33 处理器核心
pub struct CortexM33Core {
    /// Core ID (Core0 or Core1) / 核心 ID
    cpu_id: CpuId,
    /// CPU registers / CPU 寄存器
    registers: CortexM33Registers,
    /// NVIC interrupt controller / NVIC 中断控制器
    nvic: Nvic,
    /// SysTick timer / SysTick 定时器
    systick: SysTick,
    /// Vector table offset register value / 向量表偏移寄存器值
    vtor: u32,
    /// Configurable Fault Status Register (SCB->CFSR) / 可配置故障状态寄存器
    cfsr: u32,
    /// IT block state machine / IT 块状态机
    it_state: ITBlockState,
    /// Current execution state / 当前执行状态
    state: CpuState,
    /// Pending IRQ number (if any) / 待处理的中断号（如果有）
    pending_irq: Option<IrqNumber>,
    /// Active external exception numbers, innermost last
    /// 活跃的外部异常号栈，最内层位于末尾
    active_exceptions: Vec<u16>,
    /// Boot vectors cached by `boot` as (initial MSP, reset vector)
    /// 由 `boot` 缓存的启动向量（初始 MSP、复位向量）
    boot_vectors: Option<(u32, u32)>,
    /// Vector table base cached with the boot vectors / 随启动向量一起缓存的向量表基址
    boot_vtor: u32,
    /// Cumulative cycles executed since the last reset
    /// 自上次复位以来累计执行的周期数
    total_cycles: u64,
    /// Halt instead of skipping when an undefined instruction is executed
    /// 遇到未定义指令时停机而不是跳过
    strict_decode: bool,
    /// Distinct undefined opcodes already reported, keyed by (width, opcode)
    /// 已报告过的未定义操作码，按（宽度, 操作码）去重
    undefined_seen: BTreeSet<(u8, u32)>,
    /// Flash instruction fetch cache / Flash 取指缓存
    fetch_cache: fetch_cache::FetchCache,
    /// Whether the flash fetch cache may serve fetches / 是否允许 Flash 取指缓存服务取指
    fetch_cache_enabled: bool,
    /// Decoded-instruction cache / 译码结果缓存
    decode_cache: decode_cache::DecodeCache,
    /// Whether the decoded-instruction cache may serve steps / 是否允许译码缓存服务单步
    decode_cache_enabled: bool,
    /// Reason the core is sleeping, `None` while running
    /// 核心睡眠原因，运行时为 `None`
    sleep_reason: Option<SleepReason>,
    /// System Control Register sleep bits (`SLEEPONEXIT`/`SLEEPDEEP`/`SEVONPEND`)
    /// 系统控制寄存器的睡眠位（`SLEEPONEXIT`/`SLEEPDEEP`/`SEVONPEND`）
    scr: u32,
    /// WFE event register / WFE 事件寄存器
    ///
    /// Set by `SEV` (this core and the peer), `SEVL`, exception entry and
    /// exception return, exactly the ARMv8-M event sources. A `WFE` that finds it
    /// set clears it and continues without sleeping.
    /// 由 `SEV`（本核与对端）、`SEVL`、异常进入与异常返回置位，正是 ARMv8-M 的事件源。
    /// `WFE` 发现它已置位时清除并继续执行，不进入睡眠。
    event_register: bool,
    /// `SEV` instructions executed since the last host query
    /// 自上次宿主查询以来执行的 `SEV` 指令数
    events_sent: u32,
}

impl CortexM33Core {
    /// Create a new Cortex-M33 core in reset state
    /// 创建一个处于复位状态的 Cortex-M33 核心
    #[must_use]
    pub fn new(cpu_id: CpuId) -> Self {
        Self {
            cpu_id,
            registers: CortexM33Registers::new(),
            nvic: Nvic::new(),
            systick: SysTick::new(),
            vtor: 0,
            cfsr: 0,
            it_state: ITBlockState::new(),
            state: CpuState::Reset,
            pending_irq: None,
            active_exceptions: Vec::new(),
            boot_vectors: None,
            boot_vtor: 0,
            total_cycles: 0,
            strict_decode: false,
            undefined_seen: BTreeSet::new(),
            fetch_cache: fetch_cache::FetchCache::new(),
            fetch_cache_enabled: false,
            decode_cache: decode_cache::DecodeCache::new(),
            decode_cache_enabled: false,
            sleep_reason: None,
            scr: 0,
            event_register: false,
            events_sent: 0,
        }
    }

    /// Load the boot vector table at address 0 and start execution
    /// 从地址 0 加载启动向量表并开始执行
    ///
    /// Reads the initial MSP from 0x0000_0000 and the reset vector from
    /// 0x0000_0004. Flash-booting systems should call [`Self::boot_from`] with
    /// the Flash XIP base instead, because the boot ROM lives at address 0.
    /// 从 0x0000_0000 读取初始 MSP、从 0x0000_0004 读取复位向量。从 Flash 启动
    /// 的系统应改用 [`Self::boot_from`] 并传入 Flash XIP 基地址，因为地址 0
    /// 属于 boot ROM。
    ///
    /// # Errors
    /// Returns a bus error if the vector table is not mapped on `bus`.
    /// 若向量表未映射到 `bus`，返回总线错误。
    pub fn boot(&mut self, bus: &mut dyn AddressBus) -> Result<(), CoreError> {
        self.boot_from(bus, BOOT_SP_ADDR)
    }

    /// Load a vector table at `vector_base` and start execution
    /// 从 `vector_base` 加载向量表并开始执行
    ///
    /// On RP2350 the ARM reset vector table lives at the Flash XIP base
    /// `0x1000_0000`, while `0x0000_0000` is the boot ROM. The table base is
    /// also programmed into VTOR so exception vectors resolve to flash.
    /// RP2350 的 ARM 复位向量表位于 Flash XIP 基地址 `0x1000_0000`，而
    /// `0x0000_0000` 是 boot ROM。表基址同时写入 VTOR，使异常向量指向 Flash。
    ///
    /// # Errors
    /// Returns a bus error if the vector table is not mapped on `bus`.
    /// 若向量表未映射到 `bus`，返回总线错误。
    pub fn boot_from(
        &mut self,
        bus: &mut dyn AddressBus,
        vector_base: u32,
    ) -> Result<(), CoreError> {
        let mut cycles = 0u64;
        let msp = bus.read_u32(vector_base, &mut cycles)?;
        let reset = bus.read_u32(vector_base + 4, &mut cycles)?;
        self.boot_at(msp, reset, vector_base);
        Ok(())
    }

    /// Start execution from explicit vectors and a vector table base
    /// 使用显式向量与向量表基址开始执行
    ///
    /// Images without a flash vector table (ELF builds) pass the SRAM top as
    /// `msp` and the ELF entry as `reset`. The values are cached so
    /// `CpuCore::reset` restarts the core without bus access.
    /// 没有 Flash 向量表的映像（ELF 构建）把 SRAM 栈顶作为 `msp`、ELF 入口作为
    /// `reset`。数值会被缓存，使 `CpuCore::reset` 无需总线访问即可重新启动核心。
    pub fn boot_at(&mut self, msp: u32, reset: u32, vector_base: u32) {
        self.boot_vectors = Some((msp, reset));
        self.boot_vtor = vector_base;
        self.apply_boot_vectors();
    }

    /// Select whether undefined instructions halt or are skipped
    /// 选择未定义指令是停机还是跳过
    ///
    /// In strict mode the next undefined instruction makes [`CpuCore::step`]
    /// return [`CoreError::UndefinedInstruction`] and leaves the core halted.
    /// The default is to warn once per distinct opcode and skip it.
    /// 严格模式下，下一条未定义指令会让 [`CpuCore::step`] 返回
    /// [`CoreError::UndefinedInstruction`] 并使核心停机。默认行为是对每个
    /// 不同的操作码告警一次后跳过。
    pub fn set_strict_decode(&mut self, strict: bool) {
        self.strict_decode = strict;
    }

    /// Report an undefined instruction, deduplicated by opcode
    /// 按操作码去重地报告未定义指令
    fn note_undefined(&mut self, pc: u32, opcode: u32, width: u8) -> Result<(), CoreError> {
        if self.strict_decode {
            return Err(CoreError::UndefinedInstruction { pc, opcode, width });
        }
        if self.undefined_seen.insert((width, opcode)) {
            log::warn!(
                "undefined instruction at PC {pc:#010X}: opcode {opcode:#010X} \
                 ({width} bytes), skipped / 未定义指令 PC {pc:#010X}: \
                 opcode {opcode:#010X}（{width} 字节），已跳过"
            );
        }
        Ok(())
    }

    /// Write the program status register (XPSR)
    /// 写入程序状态寄存器（XPSR）
    ///
    /// Debuggers use this entry point to restore flags and exception state.
    /// The value is stored verbatim; callers are responsible for keeping the
    /// Thumb bit (EPSR.T) set for Thumb execution.
    /// 调试器通过此入口恢复标志位与异常状态。数值按原样写入；调用方需自行
    /// 保持 Thumb 位（EPSR.T）置位以便执行 Thumb 指令。
    pub fn write_xpsr(&mut self, value: u32) {
        self.registers.xpsr = value;
    }

    /// Clear the register file, NVIC, SysTick and IT state
    /// 清零寄存器文件、NVIC、SysTick 和 IT 状态
    fn clear_core_state(&mut self) {
        self.registers = CortexM33Registers::new();
        self.nvic = Nvic::new();
        self.systick = SysTick::new();
        self.vtor = 0;
        self.cfsr = 0;
        self.it_state = ITBlockState::new();
        self.pending_irq = None;
        self.active_exceptions.clear();
        self.total_cycles = 0;
        self.undefined_seen.clear();
        self.sleep_reason = None;
        self.scr = 0;
        self.event_register = false;
        self.events_sent = 0;
    }

    /// Apply cached boot vectors, entering Running only if they exist
    /// 应用缓存的启动向量，仅当向量存在时进入运行状态
    fn apply_boot_vectors(&mut self) {
        self.clear_core_state();
        if let Some((msp, reset)) = self.boot_vectors {
            self.registers.msp = msp;
            self.registers.sp = msp;
            self.registers.pc = reset & !1;
            self.vtor = self.boot_vtor;
            // The RP2350 boot ROM enables CP10/CP11 (the FPU) before handing
            // control to an Arm image. This emulator enters directly at the
            // image entry point, so it must reproduce that post-ROM state;
            // a power-on register file still starts with CPACR cleared.
            // RP2350 boot ROM 在把控制权交给 Arm 映像前已使能 CP10/CP11（FPU）。
            // 本模拟器直接从映像入口开始执行，因此需要复现这一 post-ROM 状态；
            // 上电寄存器文件本身仍以 CPACR 清零启动。
            self.registers.cpacr = fpu::cpacr::CP10_CP11_MASK;
            self.state = CpuState::Running;
        } else {
            self.state = CpuState::Reset;
        }
    }

    /// Read memory at PC and return the 16-bit halfword
    /// 读取 PC 处的内存并返回 16 位半字
    ///
    /// When the machine has vouched that the flash XIP windows are backed by a
    /// read-only [`rp2350_core::FlashRegion`], flash fetches are served from
    /// [`fetch_cache::FetchCache`]; the cache books the same two cycles a region
    /// `read_u16` does, so cycle accounting is unchanged.
    /// 当机器已确认 Flash XIP 窗口由只读的 [`rp2350_core::FlashRegion`] 支撑时，
    /// Flash 取指由 [`fetch_cache::FetchCache`] 提供；缓存按区域 `read_u16` 记入
    /// 同样的两个周期，周期记账不变。
    fn fetch_halfword(
        &mut self,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
    ) -> Result<u16, CoreError> {
        let pc = self.registers.pc;
        if self.fetch_cache_enabled {
            if let Some(hw) = self.fetch_cache.lookup(pc) {
                // 与区域读一致：半字读取记 2 个周期，调用方再加 1。
                // Match the region read: a halfword books two cycles, the caller adds one.
                *cycles += 2;
                *cycles += 1;
                return Ok(hw);
            }
            let hw = bus.read_u16(pc, cycles)?;
            self.fetch_cache.store(pc, hw);
            *cycles += 1;
            return Ok(hw);
        }
        let hw = bus.read_u16(pc, cycles)?;
        *cycles += 1;
        Ok(hw)
    }

    /// Enable or disable the flash instruction fetch cache
    /// 启用或关闭 Flash 取指缓存
    ///
    /// The cache is **off by default**. Callers enable it only after mapping a
    /// read-only [`rp2350_core::FlashRegion`] over the XIP windows: the cache
    /// then relies on the two invariants that make it exact — guest writes to
    /// flash are rejected, and host-side mutations bump
    /// [`rp2350_core::flash::content_generation`]. A bus that maps *writable*
    /// memory inside `0x1000_0000..0x1100_0000` or
    /// `0x1400_0000..0x1500_0000` must leave the cache off so self-modifying
    /// code there stays visible immediately.
    /// 该缓存**默认关闭**。调用方只有在 XIP 窗口上映射了只读的
    /// [`rp2350_core::FlashRegion`] 后才应启用：此时它依赖两条不变量保证精确——
    /// 客户机写 Flash 会被拒绝，宿主侧变更会递增
    /// [`rp2350_core::flash::content_generation`]。若总线上
    /// `0x1000_0000..0x1100_0000` 或 `0x1400_0000..0x1500_0000` 内映射的是
    /// *可写*内存，则必须保持关闭，使该处的自修改代码立即可见。
    pub fn set_flash_fetch_cache(&mut self, enabled: bool) {
        self.fetch_cache_enabled = enabled;
        // 译码缓存依赖与取指缓存完全相同的不变量（XIP 窗口只读 + 世代号失效），
        // 因此同一个开关同时启用两者；需要单独对照实验时可随后调用
        // [`Self::set_decode_cache`]。
        // The decoded-instruction cache relies on exactly the same invariants as
        // the fetch cache (read-only XIP windows plus generation invalidation),
        // so one switch enables both. A caller that needs them independently can
        // follow up with [`Self::set_decode_cache`].
        self.decode_cache_enabled = enabled;
    }

    /// Enable or disable the decoded-instruction cache
    /// 启用或关闭译码结果缓存
    ///
    /// The cache stores, per flash `PC`, the instruction length, the raw word
    /// and the dispatch tag, so the hot path skips the second fetch, the width
    /// decision and the opcode-class chain. It requires the same preconditions
    /// as [`Self::set_flash_fetch_cache`]: flash XIP windows backed by a
    /// read-only region whose host-side mutations bump
    /// [`rp2350_core::flash::content_generation`]. `SRAM` is never cached, so
    /// self-modifying `SRAM` code keeps working with the cache on.
    /// 该缓存按 Flash `PC` 保存指令长度、原始指令字与分派标签，使热路径跳过第二次
    /// 取指、宽度判定与操作码类别链。前置条件与 [`Self::set_flash_fetch_cache`] 相同：
    /// XIP 窗口由只读区域支撑，且宿主侧变更会递增
    /// [`rp2350_core::flash::content_generation`]。`SRAM` 永不缓存，因此缓存开启时
    /// 自修改 `SRAM` 代码仍然正确。
    pub fn set_decode_cache(&mut self, enabled: bool) {
        self.decode_cache_enabled = enabled;
    }

    /// Number of decode-cache generation flushes (diagnostics)
    /// 译码缓存的世代清空次数（诊断用）
    #[must_use]
    pub fn decode_cache_flushes(&self) -> u32 {
        self.decode_cache.generation_flushes()
    }

    /// Execute one instruction (step)
    /// 执行一条指令（单步）
    fn step_instruction(
        &mut self,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
    ) -> Result<StepResult, CoreError> {
        // Refresh the diagnostic program counter before each instruction so a
        // memory watchpoint can attribute stores to their instruction.
        // 每条指令前刷新诊断用的程序计数器，使内存监视点能把存储归因于具体指令。
        rp2350_core::watch::set_pc(self.registers.pc);

        // SysTick tick (1 cycle per instruction fetch)
        // SysTick 计时（每条指令获取 1 个周期）
        if self.systick.tick(1) {
            self.set_pending_irq(EXC_SYSTICK);
        }

        // 被 stall 的载入必须完整回滚：PC 与 IT 状态回到指令前，寄存器由访问本身保证
        // 未被写入（所有载入都是先取值再写回目的寄存器）。
        // A stalled load must roll back completely: PC and IT state return to the
        // pre-instruction values, and the registers are untouched because every
        // load evaluates the access before writing its destination.
        let saved_pc = self.registers.pc;
        let saved_it = self.it_state;

        let result = self.execute_current(bus, cycles);

        match result {
            Err(error) if error.is_stall() => {
                self.registers.pc = saved_pc;
                self.it_state = saved_it;
                // 重放的指令只付一次周期：本次尝试不计入任何周期。
                // The replayed instruction pays once: this attempt books nothing.
                *cycles = 0;
                self.sleep_reason = Some(SleepReason::BlockedRead);
                self.state = CpuState::Sleeping;
                Ok(StepResult::Ok)
            }
            Ok(result) => {
                match result {
                    // 等待指令在退役之后才决定是否挂起，因此它本身绝不会被跳过。
                    // A wait instruction decides on suspension only after it
                    // retires, so it is never skipped.
                    StepResult::Sleep(kind) => self.apply_sleep_request(kind),
                    StepResult::SendEvent { local } => {
                        self.event_register = true;
                        if !local {
                            self.events_sent = self.events_sent.saturating_add(1);
                        }
                    }
                    _ => {}
                }
                Ok(result)
            }
            Err(error) => Err(error),
        }
    }

    /// Run the cached or fully decoded path for the instruction at `PC`
    /// 执行 `PC` 处指令的缓存或完整译码路径
    fn execute_current(
        &mut self,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
    ) -> Result<StepResult, CoreError> {
        // 译码缓存命中：一次查表取代两次取指、宽度判定与分发链。仅在 IT 块之外
        // 使用，块内指令仍走完整译码器以保留槽位消费与标志抑制语义。
        // A decode-cache hit replaces the two fetches, the width decision and the
        // dispatch chain with one table lookup. Only used outside an IT block:
        // instructions inside one still run the full decoder so the slot
        // consumption and flag suppression stay exactly as they were.
        if self.decode_cache_enabled && !self.it_state.in_it_block() {
            let pc = self.registers.pc;
            if let Some(entry) = self.decode_cache.lookup(pc) {
                return self.step_cached(bus, cycles, entry);
            }
        }

        self.step_decoded(bus, cycles)
    }

    /// Apply the ARMv8-M wait semantics for a retired `WFI`/`WFE`
    /// 对刚退役的 `WFI`/`WFE` 应用 ARMv8-M 等待语义
    ///
    /// `WFI` never suspends while an enabled interrupt is pending (the ARM ARM
    /// guarantees it wakes on a pending interrupt even when `PRIMASK` masks it),
    /// `WFE` continues immediately when the event register is set and clears it,
    /// and both continue immediately when an exception is already pending
    /// because exception entry itself is an event source. `SCR.SLEEPDEEP` only
    /// changes the recorded reason: this model has no power domain to shut down.
    /// 只要存在已使能的挂起中断，`WFI` 就不会挂起（ARM ARM 保证即使 `PRIMASK` 屏蔽它，
    /// 挂起中断也会唤醒 `WFI`）；`WFE` 在事件寄存器已置位时立即继续并清除它；两者在
    /// 已有挂起异常时都立即继续，因为异常进入本身就是一个事件源。`SCR.SLEEPDEEP` 只
    /// 改变记录的原因：本模型没有可关闭的掉电域。
    fn apply_sleep_request(&mut self, kind: SleepKind) {
        let deep = self.scr & scr::SLEEPDEEP != 0;
        match kind {
            SleepKind::WaitForInterrupt => {
                if self.nvic.has_runnable() {
                    return;
                }
                self.sleep_reason = Some(if deep {
                    SleepReason::SleepDeep {
                        wait_for_event: false,
                    }
                } else {
                    SleepReason::WaitForInterrupt
                });
                self.state = CpuState::Sleeping;
            }
            SleepKind::WaitForEvent => {
                if self.event_register {
                    self.event_register = false;
                    return;
                }
                if self.nvic.has_runnable() {
                    return;
                }
                self.sleep_reason = Some(if deep {
                    SleepReason::SleepDeep {
                        wait_for_event: true,
                    }
                } else {
                    SleepReason::WaitForEvent
                });
                self.state = CpuState::Sleeping;
            }
        }
    }

    /// Enter the `SLEEPONEXIT` sleep after an exception return to Thread mode
    /// 异常返回到 Thread 模式后进入 `SLEEPONEXIT` 睡眠
    ///
    /// The instruction at the restored `PC` has not run yet and must not run
    /// until the sleep ends. A runnable interrupt keeps the core awake, exactly
    /// like `WFI`, so the pending exception can be taken in the same step.
    /// `SCR.SLEEPDEEP` again only changes the reported reason.
    /// 恢复出的 `PC` 处指令尚未执行，睡眠结束前也不能执行。存在可运行中断时核心保持
    /// 唤醒，与 `WFI` 完全一致，因此挂起异常可在同一步内被响应。`SCR.SLEEPDEEP`
    /// 同样只改变上报原因。
    fn apply_sleep_on_exit(&mut self) {
        if self.nvic.has_runnable() {
            return;
        }
        let deep = self.scr & scr::SLEEPDEEP != 0;
        self.sleep_reason = Some(SleepReason::SleepOnExit { deep });
        self.state = CpuState::Sleeping;
    }

    /// Set an interrupt pending and raise the `SEVONPEND` event on a transition
    /// 置位中断挂起位，并在发生 0→1 跳变时按 `SEVONPEND` 置位事件
    ///
    /// Only a 0-to-1 transition of the pending bit sends the event, whether the
    /// interrupt is enabled or masked (ARM ARM `SEVONPEND`). Repeated
    /// assertions of a still-pending line must not keep the event permanently
    /// set, otherwise `WFE` would stop waiting.
    /// 只有挂起位发生 0→1 跳变才发送事件，无论中断是否使能或被屏蔽（ARM ARM
    /// `SEVONPEND`）。对仍然挂起的中断线重复断言不得让事件永久置位，否则 `WFE`
    /// 将不再等待。
    fn set_pending_irq(&mut self, irq: IrqNumber) {
        if usize::from(irq) >= 256 {
            return;
        }
        if self.scr & scr::SEVONPEND != 0 && !self.nvic.is_pending(irq) {
            self.event_register = true;
        }
        self.nvic.set_pending(irq);
    }

    /// Raise the `SEVONPEND` event when a register write sets new pending bits
    /// 寄存器写入置位新的挂起位时按 `SEVONPEND` 置位事件
    ///
    /// `newly` is the write-1-to-set difference (`value & !ISPR`), so multiple
    /// interrupts pended by one `ISPR` write produce one event, and writing an
    /// already-pending bit produces none.
    /// `newly` 是写 1 置位的差集（`value & !ISPR`），因此一次 `ISPR` 写入挂起多个
    /// 中断只产生一个事件，而写已挂起的位不产生事件。
    fn note_pending_bits(&mut self, newly: u32) {
        if newly != 0 && self.scr & scr::SEVONPEND != 0 {
            self.event_register = true;
        }
    }

    /// Whether a sleeping core has a reason to resume / 睡眠核心是否有理由恢复
    ///
    /// `WFI` (and the `SLEEPONEXIT` sleep, whose wake rule is the `WFI` one)
    /// ignores the event register but resumes as soon as any enabled interrupt
    /// is pending, masked or not. `WFE` resumes on an event or a pending
    /// interrupt, and `SLEEPDEEP` keeps the wait condition of the instruction
    /// that requested it. A blocked load is always retried: the retry is
    /// side-effect free, so a missing event cannot deadlock it.
    /// `WFI`（以及唤醒规则与 `WFI` 相同的 `SLEEPONEXIT` 睡眠）忽略事件寄存器，但只要
    /// 有已使能的挂起中断就恢复，无论是否被屏蔽。`WFE` 在事件或挂起中断到来时恢复，
    /// `SLEEPDEEP` 保留请求它的指令的等待条件。阻塞载入总是重试：重试没有副作用，
    /// 因此缺少事件也不会死锁。
    fn wake_ready(&self) -> bool {
        match self.sleep_reason {
            Some(SleepReason::WaitForInterrupt | SleepReason::SleepOnExit { .. }) => {
                self.nvic.has_runnable()
            }
            Some(
                SleepReason::WaitForEvent
                | SleepReason::SleepDeep {
                    wait_for_event: true,
                },
            ) => self.event_register || self.nvic.has_runnable(),
            Some(SleepReason::SleepDeep {
                wait_for_event: false,
            }) => self.nvic.has_runnable(),
            Some(SleepReason::BlockedRead) | None => true,
        }
    }

    /// Whether an enabled interrupt may be taken right now
    /// 当前是否允许进入已使能的中断
    ///
    /// `PRIMASK`, `FAULTMASK` and `BASEPRI` all gate exception entry; the same
    /// gate decides whether a wake-up turns into exception entry, so a sleeping
    /// core never takes an interrupt it should keep masked.
    /// `PRIMASK`、`FAULTMASK` 与 `BASEPRI` 都会门控异常进入；唤醒是否变成异常进入也用
    /// 同一门控，因此睡眠核心绝不会进入本应保持屏蔽的中断。
    fn irq_gate_open(&self, irq: IrqNumber) -> bool {
        if self.registers.primask & 1 != 0 || self.registers.faultmask & 1 != 0 {
            return false;
        }
        // BASEPRI == 0 表示不屏蔽；否则优先级数值 >= BASEPRI 的中断被屏蔽。
        // BASEPRI == 0 disables the mask; otherwise priorities numerically at or
        // above BASEPRI are masked.
        self.nvic.basepri_allows(irq, self.registers.basepri)
    }

    /// Advance a sleeping core by one cycle without retiring an instruction
    /// 让睡眠核心推进一个周期而不退役指令
    ///
    /// Time keeps moving: `SysTick` and the cumulative cycle counter advance, so
    /// a sleeping core still generates the timer exception a real one would.
    /// Returns `true` when the core resumed and the caller must execute the
    /// instruction at `PC`; returns `false` while it stays asleep. When the wake
    /// is caused by a takeable interrupt the exception is entered here and the
    /// caller must not execute another instruction in the same step.
    /// 时间照常推进：`SysTick` 与累计周期计数都前进，因此睡眠核心仍会产生真实硬件会产生的
    /// 定时异常。核心恢复时返回 `true`，调用方必须执行 `PC` 处的指令；仍在睡眠时返回
    /// `false`。若唤醒来自可进入的中断，这里直接进入异常，调用方不得在同一步再执行指令。
    fn service_sleep(&mut self, bus: &mut dyn AddressBus) -> Result<bool, CoreError> {
        let mut cycles = 0u64;
        self.total_cycles = self.total_cycles.saturating_add(1);
        if self.systick.tick(1) {
            self.set_pending_irq(EXC_SYSTICK);
        }
        if !self.wake_ready() {
            return Ok(false);
        }
        let reason = self.sleep_reason.take();
        self.state = CpuState::Running;
        // WFE/阻塞载入消费掉唤醒它的事件；WFI（含 SLEEPONEXIT 与浅层/深层 WFI）
        // 不消费，因为它根本不看事件寄存器。
        // WFE and a blocked load consume the event that woke them; WFI (including
        // SLEEPONEXIT and both shallow/deep WFI) does not look at the event
        // register at all.
        if reason.is_some_and(|reason| {
            reason.waits_for_event() || matches!(reason, SleepReason::BlockedRead)
        }) {
            self.event_register = false;
        }
        if self.nvic.has_runnable() {
            if let Some(irq) = self.nvic.find_highest_priority() {
                if self.irq_gate_open(irq) {
                    let exception_number = FIRST_EXTERNAL_EXCEPTION + u32::from(irq);
                    exceptions::enter_exception(
                        bus,
                        &mut self.registers,
                        &mut self.nvic,
                        exception_number,
                        self.vtor,
                        &mut self.it_state,
                        &mut cycles,
                    )?;
                    self.active_exceptions.push(exception_number as u16);
                    // 异常进入是事件源；进入后事件寄存器置位（ARMv8-M）。
                    // Exception entry is an event source: set the register.
                    self.event_register = true;
                    self.total_cycles = self.total_cycles.saturating_add(cycles);
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// Reason the core is sleeping, if it is / 核心的睡眠原因（若在睡眠）
    #[must_use]
    pub fn sleep_reason(&self) -> Option<SleepReason> {
        self.sleep_reason
    }

    /// Set this core's event register (an ARMv8-M event, e.g. `SEV` or FIFO I/O)
    /// 置位本核的事件寄存器（ARMv8-M 事件，例如 `SEV` 或 FIFO I/O）
    ///
    /// A host calls this when an event source outside the core changed state — a
    /// FIFO frame, a doorbell, a peer `SEV`. It never wakes a `WFI` sleeper,
    /// which is exactly the ARM ARM rule that only `WFE` consumes events.
    /// 宿主在核外事件源改变状态时调用它——FIFO 帧、门铃、对端 `SEV`。它绝不唤醒 `WFI`
    /// 睡眠者，这正是 ARM ARM「只有 `WFE` 消费事件」的规则。
    pub fn signal_event(&mut self) {
        self.event_register = true;
    }

    /// Whether the event register is set (diagnostics) / 事件寄存器是否置位（诊断）
    #[must_use]
    pub fn event_register(&self) -> bool {
        self.event_register
    }

    /// Take the number of `SEV` instructions executed since the last call
    /// 取走自上次调用以来执行的 `SEV` 指令数
    ///
    /// `SEV` sets the event register of *every* core in the multiprocessor; a
    /// host forwards this count to the peer core so its `signal_event` runs.
    /// `SEV` 置位多处理器中**每个**核心的事件寄存器；宿主把这个计数转发给对端核心，使
    /// 其执行 `signal_event`。
    pub fn take_events_sent(&mut self) -> u32 {
        std::mem::take(&mut self.events_sent)
    }

    /// Full fetch/decode/execute path, used on a decode-cache miss
    /// 完整取指/译码/执行路径，用于译码缓存未命中
    ///
    /// This is the pre-cache interpreter body. It also fills the decode cache
    /// after a flash fetch so the next visit to this `PC` takes the fast path.
    /// 这是加入缓存之前的解释器主体。它还会在 Flash 取指之后填充译码缓存，使该 `PC`
    /// 的下一次执行走快路径。
    #[inline]
    fn step_decoded(
        &mut self,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
    ) -> Result<StepResult, CoreError> {
        // Fetch halfword
        // 获取半字
        let hw = self.fetch_halfword(bus, cycles)?;
        let old_pc = self.registers.pc;

        // Determine if Thumb-16 or Thumb-32
        // 判断是 Thumb-16 还是 Thumb-32
        let instr_type = (hw >> 11) & 0x1F;
        // Raw opcode and width, retained so undefined instructions can be
        // reported with enough context / 保留原始操作码与宽度，便于报告未定义指令
        let mut raw_opcode = u32::from(hw);
        let mut width = 2u8;

        let result = if instr_type >= 0b11101 {
            // Thumb-32: fetch second halfword
            // Thumb-32：获取第二个半字
            self.registers.pc = self.registers.pc.wrapping_add(2);
            let hw2 = self.fetch_halfword(bus, cycles)?;
            // ARM 体系参考手册约定：第一个半字位于高 16 位，第二个半字位于低 16 位。
            // ARM ARM convention: first halfword in bits[31:16], second in bits[15:0].
            let instr32 = (hw as u32) << 16 | hw2 as u32;
            raw_opcode = instr32;
            width = 4;
            self.registers.pc = old_pc.wrapping_add(4);
            self.prepare_fpu_instruction(bus, cycles, instr32)?;
            let result = execute_thumb32(
                instr32,
                &mut self.registers,
                bus,
                &mut self.nvic,
                &mut self.systick,
                &mut self.it_state,
                cycles,
            )?;
            if self.decode_cache_enabled && decode_cache::in_flash_window(old_pc) {
                self.decode_cache
                    .store(decode_cache::decode32(old_pc, instr32));
            }
            result
        } else {
            self.registers.pc = self.registers.pc.wrapping_add(2);
            let result = execute_thumb16(
                hw,
                &mut self.registers,
                bus,
                &mut self.nvic,
                &mut self.systick,
                &mut self.it_state,
                cycles,
            )?;
            if self.decode_cache_enabled && decode_cache::in_flash_window(old_pc) {
                self.decode_cache.store(decode_cache::decode16(old_pc, hw));
            }
            result
        };

        self.finish_instruction(bus, cycles, old_pc, raw_opcode, width, result)
    }

    /// Lazy FP stacking and `CONTROL.FPCA` before the first VFP instruction
    /// 首条 VFP 指令之前的惰性浮点压栈与 `CONTROL.FPCA`
    ///
    /// A handler's first VFP instruction (arithmetic, transfers, loads/stores,
    /// system registers) commits the interrupted context's S0-S15 and FPSCR to
    /// the area reserved by FPCAR at exception entry and clears LSPACT, so each
    /// frame is saved exactly once. Inactive when no lazy frame is pending. The
    /// same hook raises CONTROL.FPCA (the ARMv8-M "FP context active" bit)
    /// whenever CPACR grants access; the CLRONRET path of exception return
    /// clears it again.
    /// 处理程序的第一条 VFP 指令（算术、搬移、加载/存储、系统寄存器访问）会把被中断
    /// 上下文的 S0-S15 与 FPSCR 写入异常进入时由 FPCAR 预留的区域并清除 LSPACT，
    /// 保证每个栈帧只提交一次。FPU 未活跃时该分支不生效。同一路径还负责置位
    /// CONTROL.FPCA（ARMv8-M 的「浮点上下文活跃」位）：只要 CPACR 允许访问，执行
    /// 浮点指令就会置位，之后由异常返回的 CLRONRET 路径清除。
    #[inline]
    fn prepare_fpu_instruction(
        &mut self,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
        instr32: u32,
    ) -> Result<(), CoreError> {
        if fpu::is_fpu_encoding(instr32) {
            if self.registers.fp_access_enabled() {
                self.registers.control |= registers::CONTROL_FPCA;
            }
            if self.registers.fpu.lspact() {
                exceptions::commit_lazy_fp_context(bus, &mut self.registers, cycles)?;
            }
        }
        Ok(())
    }

    /// Shared tail: synchronous UsageFault entry and undefined-instruction report
    /// 共享尾部：同步用法故障进入与未定义指令报告
    #[inline]
    fn finish_instruction(
        &mut self,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
        old_pc: u32,
        raw_opcode: u32,
        width: u8,
        result: StepResult,
    ) -> Result<StepResult, CoreError> {
        // 同步用法故障（当前仅 NOCP）：设置 CFSR 并立即进入 UsageFault。
        // Synchronous UsageFault (currently only NOCP): set CFSR and enter
        // the UsageFault handler immediately with a standard stack frame.
        if self.registers.take_usage_fault() {
            self.cfsr |= exceptions::CFSR_UFSR_NOCP;
            let in_handler = !self.active_exceptions.is_empty();
            let thread_psp = self.registers.control & 0x2 != 0;
            exceptions::enter_exception(
                bus,
                &mut self.registers,
                &mut self.nvic,
                u32::from(exceptions::EXC_USAGE_FAULT),
                self.vtor,
                &mut self.it_state,
                cycles,
            )?;
            if !in_handler {
                // 系统异常默认写入 0xFFFF_FFF1；从 Thread 模式进入时按
                // MSP/PSP 选择正确的 EXC_RETURN，并保留 enter_exception 根据
                // 帧类型写入的 bit 4（0 = 扩展帧）。
                // System exceptions default to 0xFFFF_FFF1; entering from
                // Thread mode selects the MSP/PSP-specific EXC_RETURN while
                // preserving the frame-type bit 4 written by enter_exception.
                let extended = self.registers.lr & 0x10 == 0;
                self.registers.lr = if thread_psp { 0xFFFF_FFFD } else { 0xFFFF_FFF9 };
                if extended {
                    self.registers.lr &= !0x10;
                }
            }
            self.active_exceptions.push(exceptions::EXC_USAGE_FAULT);
            return Ok(StepResult::Ok);
        }

        if matches!(result, StepResult::Undefined) {
            self.note_undefined(old_pc, raw_opcode, width)?;
        }

        Ok(result)
    }

    /// Check for pending interrupts and initiate exception entry if needed
    /// 检查挂起的中断并在需要时发起异常进入
    fn check_pending_exceptions(
        &mut self,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
    ) -> Result<(), CoreError> {
        // 没有「已使能且挂起」的中断时无需构造候选位图。
        // Nothing enabled is pending, so building the candidate bitmaps is wasted.
        if !self.nvic.has_runnable() {
            return Ok(());
        }

        if let Some(irq) = self.nvic.find_highest_priority() {
            // PRIMASK/FAULTMASK/BASEPRI 都门控异常进入；睡眠唤醒使用同一个门控，
            // 因此睡眠核心不会进入本应保持屏蔽的中断。
            // PRIMASK/FAULTMASK/BASEPRI all gate exception entry, and the sleep
            // wake path uses the same gate, so a sleeping core never takes an
            // interrupt it should keep masked.
            if !self.irq_gate_open(irq) {
                return Ok(());
            }
            // 外部中断 n 对应异常号 16 + n，向量位于 VTOR + (16 + n) * 4。
            // External interrupt n maps to exception number 16 + n, so its
            // vector lives at VTOR + (16 + n) * 4.
            let exception_number = FIRST_EXTERNAL_EXCEPTION + u32::from(irq);
            exceptions::enter_exception(
                bus,
                &mut self.registers,
                &mut self.nvic,
                exception_number,
                self.vtor,
                &mut self.it_state,
                cycles,
            )?;
            self.active_exceptions.push(exception_number as u16);
            // 异常进入是 ARMv8-M 的事件源。 / Exception entry is an event source.
            self.event_register = true;
        }

        Ok(())
    }

    /// Perform an exception return for a `BX LR` carrying `EXC_RETURN`
    /// 对携带 `EXC_RETURN` 的 `BX LR` 执行异常返回
    ///
    /// Returns whether the return target is Thread mode, which is the only case
    /// where `SCR.SLEEPONEXIT` may suspend the core.
    /// 返回目标是否为 Thread 模式；只有该情况可能由 `SCR.SLEEPONEXIT` 挂起核心。
    fn return_from_exception(
        &mut self,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
    ) -> Result<bool, CoreError> {
        let Some(info) = exceptions::decode_exc_return(self.registers.lr) else {
            return Ok(false);
        };
        let Some(active) = self.active_exceptions.pop() else {
            return Ok(false);
        };
        exceptions::exit_exception(
            bus,
            &mut self.registers,
            &mut self.nvic,
            &info,
            u32::from(active),
            &mut self.it_state,
            cycles,
        )?;
        // 异常返回同样是 ARMv8-M 的事件源。 / Exception return is an event source.
        self.event_register = true;
        Ok(!info.to_handler)
    }

    /// Execute one instruction, or advance a sleeping core by one cycle
    /// 执行一条指令，或让睡眠核心推进一个周期
    ///
    /// This is the single body shared by [`CpuCore::step`] and
    /// [`CpuCore::run_cycles`], so both entry points have identical sleep,
    /// stall-retry and interrupt semantics.
    /// 这是 [`CpuCore::step`] 与 [`CpuCore::run_cycles`] 共享的唯一实现，因此两个入口
    /// 的睡眠、stall 重试与中断语义完全一致。
    fn advance(&mut self, bus: &mut dyn AddressBus) -> Result<CpuState, CoreError> {
        if self.state == CpuState::Sleeping {
            if !self.service_sleep(bus)? {
                // 仍在睡眠，或刚刚在唤醒时进入了中断：本步不退役指令。
                // Still asleep, or an exception was entered on wake: no
                // instruction retires in this step.
                return Ok(self.state);
            }
        } else if self.state != CpuState::Running {
            return Ok(self.state);
        }

        let mut cycle_count: u64 = 0;
        let result = match self.step_instruction(bus, &mut cycle_count) {
            Ok(result) => result,
            Err(error) => {
                self.state = CpuState::Halted;
                return Err(error);
            }
        };
        // 被 stall 的载入把核心置为 Sleeping 并保持 PC 不变；此时不能做异常返回或
        // 中断投递，否则会在「未执行的指令」上叠加异常。
        // A stalled load puts the core to sleep with PC unchanged; no exception
        // return or interrupt delivery may then run on top of an unexecuted
        // instruction.
        if self.state == CpuState::Running {
            if result == StepResult::ExceptionReturn {
                let to_thread = self.return_from_exception(bus, &mut cycle_count)?;
                // SLEEPONEXIT 只在返回到 Thread 模式时生效；Handler→Handler 返回
                // 继续执行被抢占的处理程序。
                // SLEEPONEXIT only applies to a return to Thread mode; a
                // Handler-to-Handler return resumes the preempted handler.
                if to_thread && self.scr & scr::SLEEPONEXIT != 0 {
                    self.apply_sleep_on_exit();
                }
            }
            if self.state == CpuState::Running {
                self.check_pending_exceptions(bus, &mut cycle_count)?;
            }
        }
        self.total_cycles = self.total_cycles.saturating_add(cycle_count);
        Ok(self.state)
    }
}

impl CpuCore for CortexM33Core {
    fn cpu_id(&self) -> CpuId {
        self.cpu_id
    }

    fn arch(&self) -> CpuArch {
        CpuArch::ArmCortexM33
    }

    fn init(&mut self, bus: &mut dyn AddressBus) -> Result<(), CoreError> {
        self.boot(bus)
    }

    fn run_cycles(
        &mut self,
        bus: &mut dyn AddressBus,
        cycles: CycleCount,
    ) -> (CycleCount, CpuState) {
        let mut remaining = cycles;
        let mut consumed: CycleCount = 0;

        while remaining > 0 {
            if !matches!(self.state, CpuState::Running | CpuState::Sleeping) {
                break;
            }
            match self.advance(bus) {
                Ok(_) => {
                    // 睡眠步同样推进机器时间，因此也计入一个周期槽。
                    // A sleep step still advances machine time, so it takes a slot.
                    consumed += 1;
                    remaining -= 1;
                }
                Err(e) => {
                    log::error!("CPU halted: {e:?}");
                    break;
                }
            }
        }

        (consumed, self.state)
    }

    fn step(&mut self, bus: &mut dyn AddressBus) -> Result<CpuState, CoreError> {
        self.advance(bus)
    }

    fn signal_interrupt(&mut self, irq: IrqNumber) {
        self.set_pending_irq(irq);
    }

    fn system_write(&mut self, addr: u32, value: u32) {
        let _ = self.route_system_access(addr, true, value);
    }

    fn system_read(&mut self, addr: u32) -> Option<u32> {
        self.route_system_access(addr, false, 0)
    }

    fn servicing_irq(&self) -> Option<IrqNumber> {
        self.nvic.find_highest_priority()
    }

    fn pc(&self) -> u32 {
        self.registers.pc
    }

    fn read_register(&self, index: u8) -> u32 {
        self.registers.read(index)
    }

    fn write_register(&mut self, index: u8, value: u32) {
        self.registers.write(index, value);
    }

    fn reset(&mut self) {
        // 重新应用缓存的启动向量；若核心从未 boot 过则保持 Reset 状态。
        // Re-apply cached boot vectors; stay in Reset if the core never booted.
        self.apply_boot_vectors();
    }

    fn description(&self) -> String {
        format!(
            "ARM Cortex-M33 Core{} (PC=0x{:08X}, {:?})",
            self.cpu_id as u8,
            self.pc(),
            self.state
        )
    }

    fn state(&self) -> CpuState {
        self.state
    }

    fn xpsr(&self) -> u32 {
        self.registers.xpsr
    }

    fn total_cycles(&self) -> CycleCount {
        self.total_cycles
    }
}

#[cfg(test)]
mod tests;
