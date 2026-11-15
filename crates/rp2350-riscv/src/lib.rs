//! rp2350-riscv — Hazard3 RISC-V (RV32IMAC) processor emulation
//! rp2350-riscv — Hazard3 RISC-V（RV32IMAC）处理器仿真
//!
//! RP2350 can boot either an Arm Cortex-M33 or a Hazard3 RISC-V core. This crate
//! implements the machine-mode integer subset needed by firmware bring-up:
//! RV32I base instructions, the M (multiply/divide), A (atomics) and C
//! (compressed) extensions, machine CSRs, and a minimal `Xh3irq` external
//! interrupt controller.
//!
//! RP2350 可选择 Arm Cortex-M33 或 Hazard3 RISC-V 核心启动。本 crate 实现固件
//! 启动所需机器模式整数子集：RV32I 基础指令、M（乘除）、A（原子）与 C（压缩）
//! 扩展、机器模式 CSR，以及最小 `Xh3irq` 外部中断控制器。
//!
//! # Boot handoff / 启动交接
//!
//! Arm images use a vector table at flash address zero. RISC-V images instead
//! start with a picobin `IMAGE_DEF` header. The default RISC-V handoff reads:
//!
//! - entry PC from `0x1000_0004`
//! - initial SP from `0x1000_0020`
//!
//! after validating the picobin magic at `0x1000_0000`. Hosts may bypass this
//! layout with [`Hazard3Core::set_boot_vectors`].
//!
//! Arm 映像在 Flash 起始处使用向量表；RISC-V 映像以 picobin `IMAGE_DEF` 头开始。
//! 默认交接在验证 `0x1000_0000` 魔数后，从 `0x1000_0004` 读取入口 PC、从
//! `0x1000_0020` 读取初始 SP。宿主可用 [`Hazard3Core::set_boot_vectors`] 覆盖。

// RISC-V 指令执行涉及大量位域拼接与整数宽度转换，这是指令译码的固有特性。
// RISC-V execution performs heavy bit-field assembly and integer width
// conversions, which is inherent to instruction decoding.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::doc_markdown,
    clippy::module_name_repetitions,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

mod csr;
mod execute;
mod execute_c;
mod registers;

pub use registers::Rv32Registers;

use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;
use rp2350_core::types::{CpuId, CycleCount, IrqNumber};
use rp2350_cpu::{CpuArch, CpuCore, CpuState};

use csr::CsrFile;

/// Why a core is in [`CpuState::Sleeping`] / 核心处于 [`CpuState::Sleeping`] 的原因
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepReason {
    /// `WFI` with no enabled interrupt pending / 无使能挂起中断时的 `WFI`
    WaitForInterrupt,
    /// A load hit a retryable bus stall (blocking FIFO read)
    /// 载入遇到可重试的总线 stall（阻塞 FIFO 读）
    ///
    /// No guest-visible exception is raised: `PC` and the destination register
    /// keep their pre-instruction values and the load is replayed later.
    /// 不会产生客户机可见异常：`PC` 与目的寄存器保持指令执行前的值，载入稍后重放。
    BlockedRead,
}

/// Picobin header magic at the start of an RP2350 RISC-V image
/// RP2350 RISC-V 映像起始处的 picobin 头魔数
pub const RISCV_BOOT_MAGIC_ADDR: u32 = 0x1000_0000;

/// Expected picobin/boot2 header magic word / 期望的 picobin/boot2 头魔数字
pub const RISCV_BOOT_MAGIC: u32 = 0x7188_EBF2;

/// Address of the legacy entry point word in the boot2 header
/// boot2 头中旧式入口点字的地址
pub const RISCV_BOOT_ENTRY_ADDR: u32 = 0x1000_0004;

/// Address of the initial stack pointer word in the boot2 header
/// boot2 头中初始栈指针字的地址
pub const RISCV_BOOT_SP_ADDR: u32 = 0x1000_0024;

/// Start marker of a picobin `IMAGE_DEF` block / picobin `IMAGE_DEF` 块起始标记
pub const RISCV_PICOBIN_MARKER_START: u32 = 0xFFFF_DED3;

/// End marker of a picobin block / picobin 块结束标记
pub const RISCV_PICOBIN_MARKER_END: u32 = 0xAB12_3579;

/// `PICOBIN_BLOCK_ITEM_1BS_ENTRY_POINT` item type / 入口点条目类型
pub const RISCV_PICOBIN_ENTRY_POINT_ITEM: u32 = 0x44;

/// Number of words scanned for a picobin block / 扫描 picobin 块的字数
const PICOBIN_SCAN_WORDS: u32 = 0x100;

/// Hazard3 RISC-V processor core / Hazard3 RISC-V 处理器核心
#[derive(Debug)]
pub struct Hazard3Core {
    /// Core ID / 核心 ID
    cpu_id: CpuId,
    /// Integer register file / 整数寄存器组
    registers: Rv32Registers,
    /// Machine-mode control and status registers / 机器模式控制状态寄存器
    csr: CsrFile,
    /// Current execution state / 当前执行状态
    state: CpuState,
    /// Cached boot vectors (SP, PC) / 缓存的启动向量（SP、PC）
    boot_vectors: Option<(u32, u32)>,
    /// Cumulative executed instructions since reset / 复位以来累计执行的指令数
    total_cycles: u64,
    /// Load reservation address for `lr.w`/`sc.w` / `lr.w`/`sc.w` 的保留地址
    reservation: Option<u32>,
    /// Optional host-installed vector table for fatal exceptions
    /// 宿主可选安装的致命异常向量表基地址
    fatal_trap_vector: Option<u32>,
    /// Reason the core is sleeping, `None` while running
    /// 核心睡眠原因，运行时为 `None`
    sleep_kind: Option<SleepReason>,
    /// Event signalled by the host (FIFO traffic, peer `SEV`) while asleep
    /// 宿主机在睡眠期间发出的事件（FIFO 流量、对端 `SEV`）
    sleep_event: bool,
    /// A `WFI` retired and asked for a sleep / 已退役的 `WFI` 请求睡眠
    sleep_requested: bool,
}

impl Hazard3Core {
    /// Create a new Hazard3 core in reset state / 创建一个处于复位状态的 Hazard3 核心
    #[must_use]
    pub fn new(cpu_id: CpuId) -> Self {
        Self {
            cpu_id,
            registers: Rv32Registers::new(),
            csr: CsrFile::new(),
            state: CpuState::Reset,
            boot_vectors: None,
            total_cycles: 0,
            reservation: None,
            fatal_trap_vector: None,
            sleep_kind: None,
            sleep_event: false,
            sleep_requested: false,
        }
    }

    /// Install a host-owned vector table for fatal exceptions
    /// 为致命异常安装宿主自有的向量表
    ///
    /// Causes 2 (illegal instruction) and 5 (load access fault) jump to
    /// `base + 4 * cause` instead of the guest's `mtvec`, so a host can
    /// terminate with a diagnostic report rather than loop on a vector slot
    /// the firmware never populated. Other traps keep the architectural
    /// `mtvec` behaviour.
    /// cause 2（非法指令）与 cause 5（加载访问错误）会跳到 `base + 4 * cause`，
    /// 而不是客户机的 `mtvec`，使宿主能够以诊断报告停机，而不是在固件从未填充
    /// 的向量槽里循环。其他陷阱保持架构规定的 `mtvec` 行为。
    pub fn set_fatal_trap_vector(&mut self, base: u32) {
        self.fatal_trap_vector = Some(base & !3);
    }

    /// Apply the optional fatal-exception vector to a trap target
    /// 将可选的致命异常向量应用到陷阱目标
    fn enter_trap(&mut self, cause: u32, tval: u32, epc: u32) -> u32 {
        let target = self.csr.take_trap(cause, tval, epc);
        if matches!(cause, 2 | 5) {
            if let Some(base) = self.fatal_trap_vector {
                return base.wrapping_add(4 * cause);
            }
        }
        target
    }

    /// Override the boot vectors used by [`CpuCore::init`] / 覆盖启动向量
    ///
    /// `sp` is the initial stack pointer and `pc` the first instruction address.
    /// `pc` has its low bit cleared, matching the alignment rule.
    /// `sp` 为初始栈指针，`pc` 为第一条指令地址；`pc` 最低位会被清零。
    pub fn set_boot_vectors(&mut self, sp: u32, pc: u32) {
        self.boot_vectors = Some((sp, pc & !1));
    }

    /// Scan the first flash page for a picobin `IMAGE_DEF` entry-point item
    /// 在 Flash 首页扫描 picobin `IMAGE_DEF` 的入口点条目
    ///
    /// The RP2350 boot2 header contains an `IMAGE_DEF` block with an item whose
    /// low byte is 0x44; the following two words are the boot2 entry point and
    /// initial stack pointer. Returns `None` when no block/item is present.
    /// RP2350 的 boot2 头包含 `IMAGE_DEF` 块，其中一个条目低字节为 0x44，其后
    /// 两个字是 boot2 入口点与初始栈指针。没有块/条目时返回 `None`。
    fn find_picobin_vectors(bus: &mut dyn AddressBus) -> Result<Option<(u32, u32)>, CoreError> {
        let mut cycles = 0u64;
        for word_index in 0..PICOBIN_SCAN_WORDS {
            let addr = RISCV_BOOT_MAGIC_ADDR.wrapping_add(word_index * 4);
            let word = bus.read_u32(addr, &mut cycles)?;
            if word != RISCV_PICOBIN_MARKER_START {
                continue;
            }
            let mut item = addr.wrapping_add(8);
            let end = addr.wrapping_add(0x180);
            while item < end {
                let item_word = bus.read_u32(item, &mut cycles)?;
                if item_word == RISCV_PICOBIN_MARKER_END {
                    break;
                }
                if item_word & 0xFF == RISCV_PICOBIN_ENTRY_POINT_ITEM {
                    let entry = bus.read_u32(item.wrapping_add(4), &mut cycles)?;
                    let sp = bus.read_u32(item.wrapping_add(8), &mut cycles)?;
                    return Ok(Some((sp, entry & !1)));
                }
                item = item.wrapping_add(4);
            }
        }
        Ok(None)
    }

    /// Load the boot vectors and enter the running state / 加载启动向量并进入运行状态
    ///
    /// # Errors
    /// Returns `CoreError::FirmwareError` when no explicit vectors were supplied
    /// and the bus does not contain a valid picobin block or legacy boot2 header.
    /// 未显式提供向量且总线既没有合法 picobin 块也没有旧式 boot2 头时，返回
    /// `CoreError::FirmwareError`。
    pub fn boot(&mut self, bus: &mut dyn AddressBus) -> Result<(), CoreError> {
        let (sp, pc) = if let Some(vectors) = self.boot_vectors {
            vectors
        } else if let Some(vectors) = Self::find_picobin_vectors(bus)? {
            vectors
        } else {
            let mut cycles = 0u64;
            let magic = bus.read_u32(RISCV_BOOT_MAGIC_ADDR, &mut cycles)?;
            if magic != RISCV_BOOT_MAGIC {
                return Err(CoreError::FirmwareError(format!(
                    "RISC-V boot: no picobin IMAGE_DEF block or boot2 magic in flash; \
                     read {magic:#010X} at {RISCV_BOOT_MAGIC_ADDR:#010X}, \
                     expected boot2 magic {RISCV_BOOT_MAGIC:#010X} / \
                     RISC-V 启动失败：Flash 中既无 picobin IMAGE_DEF 块也无 boot2 魔数"
                )));
            }
            let entry = bus.read_u32(RISCV_BOOT_ENTRY_ADDR, &mut cycles)?;
            let sp = bus.read_u32(RISCV_BOOT_SP_ADDR, &mut cycles)?;
            (sp, entry & !1)
        };
        self.boot_vectors = Some((sp, pc));
        self.apply_boot_vectors();
        Ok(())
    }

    /// Reset registers, CSRs and interrupt state / 复位寄存器、CSR 与中断状态
    fn clear_core_state(&mut self) {
        self.registers = Rv32Registers::new();
        self.csr = CsrFile::new();
        self.total_cycles = 0;
        self.reservation = None;
        self.sleep_kind = None;
        self.sleep_event = false;
        self.sleep_requested = false;
    }

    /// Apply cached boot vectors / 应用缓存的启动向量
    fn apply_boot_vectors(&mut self) {
        self.clear_core_state();
        if let Some((sp, pc)) = self.boot_vectors {
            self.registers.write(2, sp);
            self.registers.pc = pc;
            self.state = CpuState::Running;
        } else {
            self.state = CpuState::Reset;
        }
    }

    /// Return the highest-priority pending external IRQ allowed to trap
    /// 返回当前允许触发陷阱的最高优先级外部中断
    fn pending_interrupt(&self) -> Option<IrqNumber> {
        let globally_enabled = self.csr.mstatus & csr::MSTATUS_MIE != 0;
        let external_enabled = self.csr.mie & csr::MIE_MEIE != 0;
        if !globally_enabled || !external_enabled {
            return None;
        }
        // Xh3irq 只允许 meiea 使能、优先级不低于当前 preempt 的挂起位触发
        // 陷阱；meinext 的可见性由 ppreempt 决定，陷阱门控由 preempt 决定。
        // Xh3irq only traps for pending bits enabled through meiea whose
        // priority is at least the current preempt level; meinext visibility
        // uses ppreempt, while trap gating uses preempt.
        let irq = self.csr.xh3irq.best_visible(self.csr.xh3irq.ppreempt())?;
        if self.csr.xh3irq.priority_of(irq) >= self.csr.xh3irq.preempt() {
            Some(irq)
        } else {
            None
        }
    }

    /// Enter the external interrupt handler / 进入外部中断处理程序
    fn take_interrupt(&mut self, irq: IrqNumber) {
        self.csr.xh3irq.enter(irq);
        // 不在此处重算 MEINEXT：处理程序要靠它识别刚进入的 IRQ；下一次
        // 软件写 MEINEXT.UPDATE 时才会重新选择候选。
        // Do not recompute MEINEXT here: the handler relies on it to identify
        // the IRQ just entered; the next MEINEXT.UPDATE write reselects.
        let handler = self.csr.take_trap(0x8000_000B, 0, self.registers.pc);
        self.registers.pc = handler;
    }

    /// Execute one instruction or take a pending interrupt / 执行一条指令或响应中断
    fn step_internal(&mut self, bus: &mut dyn AddressBus) -> Result<(), CoreError> {
        if let Some(irq) = self.pending_interrupt() {
            self.take_interrupt(irq);
            self.total_cycles += 1;
            return Ok(());
        }
        self.execute_one(bus)?;
        if self.state == CpuState::Sleeping {
            // 被 stall 的载入已经让核心睡眠；本步不重复计时（睡眠步由
            // `CpuState::Sleeping` 的推进路径计时）。
            // A stalled load already put the core to sleep; this step books no
            // extra time (the sleeping path advances the clock instead).
            return Ok(());
        }
        if std::mem::take(&mut self.sleep_requested) {
            self.state = CpuState::Sleeping;
            self.sleep_kind = Some(SleepReason::WaitForInterrupt);
        }
        self.csr.tick();
        self.total_cycles += 1;
        Ok(())
    }

    /// Whether a sleeping core has a reason to resume / 睡眠核心是否有理由恢复
    ///
    /// RISC-V `wfi` resumes on any enabled interrupt (taken at the next step) and
    /// on a host-signalled event, which models the pico-sdk mapping of `__wfe()`
    /// to `wfi` for the inter-core mailbox. A blocked load is always retried
    /// because the retry is side-effect free.
    /// RISC-V 的 `wfi` 在任一使能中断（下一步被响应）或宿主事件到来时恢复，后者用于
    /// 建模 pico-sdk 把 `__wfe()` 映射为 `wfi` 的核间信箱握手。阻塞载入总是重试，因为
    /// 重试没有副作用。
    fn wake_ready(&self) -> bool {
        match self.sleep_kind {
            Some(SleepReason::BlockedRead) | None => true,
            Some(SleepReason::WaitForInterrupt) => {
                self.pending_interrupt().is_some() || self.sleep_event
            }
        }
    }

    /// Reason the core is sleeping, if it is / 核心的睡眠原因（若在睡眠）
    #[must_use]
    pub fn sleep_reason(&self) -> Option<SleepReason> {
        self.sleep_kind
    }

    /// Set the host event flag (FIFO traffic, peer `SEV`)
    /// 置位宿主事件标志（FIFO 流量、对端 `SEV`）
    ///
    /// Hazard3 has no architectural WFE event register, but the pico-sdk builds
    /// `__wfe()` into `wfi` for RISC-V, so the inter-core mailbox handshake needs
    /// a wake source that is not an interrupt.
    /// Hazard3 没有架构层面的 WFE 事件寄存器，但 pico-sdk 在 RISC-V 上把 `__wfe()` 编译
    /// 为 `wfi`，因此核间信箱握手需要一个非中断的唤醒源。
    pub fn signal_event(&mut self) {
        self.sleep_event = true;
    }

    /// Whether the host event flag is pending (diagnostics)
    /// 宿主事件标志是否挂起（诊断）
    #[must_use]
    pub fn event_pending(&self) -> bool {
        self.sleep_event
    }
}

impl CpuCore for Hazard3Core {
    fn cpu_id(&self) -> CpuId {
        self.cpu_id
    }

    fn arch(&self) -> CpuArch {
        CpuArch::RiscVHazard3
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
        while remaining > 0 && matches!(self.state, CpuState::Running | CpuState::Sleeping) {
            match self.step_internal(bus) {
                Ok(()) => {
                    consumed += 1;
                    remaining -= 1;
                }
                Err(error) => {
                    self.state = CpuState::Faulted;
                    log_fault(&error);
                    break;
                }
            }
        }
        (consumed, self.state)
    }

    fn step(&mut self, bus: &mut dyn AddressBus) -> Result<CpuState, CoreError> {
        if self.state == CpuState::Sleeping {
            // 睡眠期间时间照常推进：CSR 计数与总周期都前进。
            // Time keeps moving while asleep: the CSR clock and the total cycle
            // counter advance.
            self.csr.tick();
            self.total_cycles = self.total_cycles.saturating_add(1);
            if !self.wake_ready() {
                return Ok(CpuState::Sleeping);
            }
            self.sleep_kind = None;
            self.sleep_event = false;
            self.state = CpuState::Running;
        } else if self.state != CpuState::Running {
            return Ok(self.state);
        }
        match self.step_internal(bus) {
            Ok(()) => Ok(self.state),
            Err(error) => {
                self.state = CpuState::Faulted;
                Err(error)
            }
        }
    }

    fn signal_interrupt(&mut self, irq: IrqNumber) {
        if irq < 64 {
            let window = usize::from(irq) / 16;
            self.csr.xh3irq.pending[window] |= 1u16 << (irq % 16);
            self.csr.refresh_xh3irq();
        }
        // 挂起位本身不会改状态：`step` 在睡眠时按 `pending_interrupt` 判定唤醒，
        // 与 `wfi` 的「中断使能才唤醒」规则一致。
        // Signalling only marks the bit: `step` decides the wake from
        // `pending_interrupt`, matching wfi's "only enabled interrupts wake" rule.
    }

    fn servicing_irq(&self) -> Option<IrqNumber> {
        self.pending_interrupt()
    }

    fn pc(&self) -> u32 {
        self.registers.pc
    }

    fn set_pc(&mut self, pc: u32) {
        // RISC-V has no architectural PC register; register 15 is a5, so the
        // dedicated field must be updated instead.
        // RISC-V 没有架构层面的 PC 寄存器；寄存器 15 是 a5，因此必须改为更新
        // 专用的 PC 字段。
        self.registers.pc = pc;
    }

    fn csr_read(&self, addr: u32) -> u32 {
        self.csr.read(addr, 0)
    }

    fn read_register(&self, index: u8) -> u32 {
        self.registers.read(index)
    }

    fn write_register(&mut self, index: u8, value: u32) {
        self.registers.write(index, value);
    }

    fn reset(&mut self) {
        self.apply_boot_vectors();
    }

    fn description(&self) -> String {
        format!(
            "Hazard3 RISC-V Core{} (PC=0x{:08X}, {:?})",
            self.cpu_id as u8,
            self.pc(),
            self.state
        )
    }

    fn state(&self) -> CpuState {
        self.state
    }

    fn total_cycles(&self) -> CycleCount {
        self.total_cycles
    }
}

/// Log a fatal execution error without pulling in a logging backend
/// 记录致命执行错误，不引入日志后端依赖
fn log_fault(error: &CoreError) {
    eprintln!("Hazard3 core halted: {error}");
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests_csr.rs"]
mod tests_csr;

#[cfg(test)]
#[path = "tests_sleep.rs"]
mod tests_sleep;
