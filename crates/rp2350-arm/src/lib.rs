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
    clippy::missing_errors_doc
)]

pub mod conditions;
pub mod dual_core;
pub mod exceptions;
pub mod execute_thumb16;
pub mod execute_thumb32;
pub mod fpu;
pub mod nvic;
pub mod registers;
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
use execute_thumb16::{StepResult, execute_thumb16};
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
    /// Whether the unmodeled lazy FP stacking condition has been reported
    /// 是否已报告过未建模的惰性浮点压栈条件
    fp_stack_warned: bool,
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
            fp_stack_warned: false,
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
        self.fp_stack_warned = false;
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
    fn fetch_halfword(
        &mut self,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
    ) -> Result<u16, CoreError> {
        let hw = bus.read_u16(self.registers.pc, cycles)?;
        *cycles += 1;
        Ok(hw)
    }

    /// Handle SysTick register accesses via bus
    /// 通过总线处理 SysTick 寄存器访问
    fn handle_systick_access(
        &mut self,
        offset: u32,
        write: bool,
        value: u32,
        _cycles: &mut u64,
    ) -> Option<u32> {
        match offset {
            0x00 => {
                // SYST_CSR
                if write {
                    self.systick.csr = value & 0x7;
                    if value & (1 << 16) != 0 {
                        self.systick.csr &= !(1 << 16); // Clear COUNTFLAG
                    }
                }
                Some(self.systick.csr)
            }
            0x04 => {
                // SYST_RVR
                if write {
                    self.systick.write_rvr(value);
                }
                Some(self.systick.rvr)
            }
            0x08 => {
                // SYST_CVR
                if write {
                    self.systick.write_cvr(value);
                }
                Some(self.systick.read_cvr())
            }
            0x0C => {
                // SYST_CALIB (read-only)
                Some(self.systick.calib)
            }
            _ => None,
        }
    }

    /// Route a system control space access to the matching inner device
    /// 将系统控制空间访问路由到对应的内部设备
    fn route_system_access(&mut self, addr: u32, write: bool, value: u32) -> Option<u32> {
        if addr == SCB_VTOR {
            if write {
                // ARMv8-M：VTOR 低 7 位保留，向量表按 128 字节对齐。
                // ARMv8-M: VTOR bits[6:0] are reserved; the table is 128-byte aligned.
                self.vtor = value & !0x7F;
            }
            Some(self.vtor)
        } else if addr == SCB_CPACR {
            if write {
                // 建模 CP0-CP7（GPIOC/DCP）与 CP10-CP11（FPU）字段，
                // 其余协处理器键保持为零。
                // Model CP0-CP7 (GPIOC/DCP) and CP10-CP11 (FPU) fields;
                // the remaining coprocessor keys read as zero.
                self.registers.cpacr = value & fpu::cpacr::MODELED_MASK;
            }
            Some(self.registers.cpacr)
        } else if addr == SCB_CFSR {
            if write {
                // 简化为直接读写；真实 CFSR 为写 1 清零。
                // Modeled as a plain read/write; the real CFSR is write-one-to-clear.
                self.cfsr = value;
            }
            Some(self.cfsr)
        } else if (NVIC_BASE..NVIC_BASE + 0x400).contains(&addr) {
            self.handle_nvic_access(addr - NVIC_BASE, write, value)
        } else if (SYSTICK_BASE..SYSTICK_BASE + 0x10).contains(&addr) {
            let mut cycles = 0u64;
            self.handle_systick_access(addr - SYSTICK_BASE, write, value, &mut cycles)
        } else {
            None
        }
    }

    /// Handle NVIC register accesses via bus
    /// 通过总线处理 NVIC 寄存器访问
    fn handle_nvic_access(&mut self, offset: u32, write: bool, value: u32) -> Option<u32> {
        match offset {
            // ISERn: set-enable, write-1-to-set
            o @ 0x00..=0x1C => {
                let reg_idx = ((o - 0x00) >> 2) as usize;
                if reg_idx < 8 {
                    if write {
                        self.nvic.write_iser(reg_idx, value);
                    }
                    Some(self.nvic.read_iser(reg_idx))
                } else {
                    None
                }
            }
            // ICERn: clear-enable, write-1-to-clear
            o @ 0x80..=0x9C => {
                let reg_idx = ((o - 0x80) >> 2) as usize;
                if reg_idx < 8 {
                    if write {
                        self.nvic.write_icer(reg_idx, value);
                    }
                    Some(self.nvic.read_iser(reg_idx))
                } else {
                    None
                }
            }
            // ISPRn: set-pending, write-1-to-set
            o @ 0x100..=0x11C => {
                let reg_idx = ((o - 0x100) >> 2) as usize;
                if reg_idx < 8 {
                    if write {
                        self.nvic.write_ispr(reg_idx, value);
                    }
                    Some(self.nvic.read_ispr(reg_idx))
                } else {
                    None
                }
            }
            // ICPRn: clear-pending, write-1-to-clear
            o @ 0x180..=0x19C => {
                let reg_idx = ((o - 0x180) >> 2) as usize;
                if reg_idx < 8 {
                    if write {
                        self.nvic.write_icpr(reg_idx, value);
                    }
                    Some(self.nvic.read_ispr(reg_idx))
                } else {
                    None
                }
            }
            // IPRn: priority register, 4 bytes per IRQ, byte-accessible
            o @ 0x300..=0x3FC => {
                let irq_idx = ((o - 0x300) >> 2) as u16;
                if irq_idx < 256 {
                    let byte_idx = ((o - 0x300) & 3) as u16;
                    // Each IPR word covers 4 IRQs at byte granularity
                    // 每个 IPR 字以字节粒度覆盖 4 个 IRQ
                    let group_start = (irq_idx & !3u16) as usize;
                    let value_shift = (byte_idx * 8) as u8;
                    if write {
                        let mask = !(0xFFu32 << value_shift);
                        let existing = self.nvic.read_ipr_word(group_start / 4);
                        let new_val = (existing & mask) | ((value & 0xFF) << value_shift);
                        self.nvic.write_ipr_word(group_start / 4, new_val);
                    }
                    let word = self.nvic.read_ipr_word(group_start / 4);
                    Some((word >> value_shift) & 0xFF)
                } else {
                    None
                }
            }
            _ => None,
        }
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
            self.nvic.set_pending(EXC_SYSTICK);
        }

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
            execute_thumb32(
                instr32,
                &mut self.registers,
                bus,
                &mut self.nvic,
                &mut self.systick,
                &mut self.it_state,
                cycles,
            )?
        } else {
            self.registers.pc = self.registers.pc.wrapping_add(2);
            execute_thumb16(
                hw,
                &mut self.registers,
                bus,
                &mut self.nvic,
                &mut self.systick,
                &mut self.it_state,
                cycles,
            )?
        };

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
                // MSP/PSP 选择正确的 EXC_RETURN。
                // System exceptions default to 0xFFFF_FFF1; entering from
                // Thread mode selects the MSP/PSP-specific EXC_RETURN.
                self.registers.lr = if thread_psp { 0xFFFF_FFFD } else { 0xFFFF_FFF9 };
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
        // Only check if PRIMASK is 0 / 仅在 PRIMASK 为 0 时检查
        if self.registers.primask & 1 != 0 {
            return Ok(());
        }

        // 惰性浮点压栈未建模（FPCCR/LSPACT/FPCAR）。若中断到来时浮点单元
        // 已激活，真实硬件会按 FPCCR 压入 26 字扩展帧或推迟保存 S0-S15/FPSCR；
        // 本模拟器始终只压标准 8 字帧。此处显式报告一次未支持，不假装已保存
        // 浮点上下文（完整处理留待 Stage 5）。
        // Lazy FP stacking is unmodeled (FPCCR/LSPACT/FPCAR). If an interrupt
        // arrives while the FPU is active, real hardware stacks the 26-word
        // extended frame (or defers it). This emulator stacks only the standard
        // 8-word frame, so it reports the unsupported condition once instead
        // of pretending FP context was preserved (full handling is Stage 5).
        if self.registers.fpu.fpexc & fpu::fpexc::EN != 0 && !self.fp_stack_warned {
            self.fp_stack_warned = true;
            log::warn!(
                "exception entry with live FP context: lazy FP stacking \
                 (LSPACT/FPCAR/FPCCR) is not modeled; S0-S15 and FPSCR are not stacked / \
                 异常进入时浮点上下文活跃：未建模惰性压栈（LSPACT/FPCAR/FPCCR），\
                 S0-S15 与 FPSCR 不会被压栈"
            );
        }

        if let Some(irq) = self.nvic.find_highest_priority() {
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
        }

        Ok(())
    }

    /// Perform an exception return for a `BX LR` carrying `EXC_RETURN`
    /// 对携带 `EXC_RETURN` 的 `BX LR` 执行异常返回
    fn return_from_exception(
        &mut self,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
    ) -> Result<(), CoreError> {
        let Some(info) = exceptions::decode_exc_return(self.registers.lr) else {
            return Ok(());
        };
        let Some(active) = self.active_exceptions.pop() else {
            return Ok(());
        };
        exceptions::exit_exception(
            bus,
            &mut self.registers,
            &mut self.nvic,
            &info,
            u32::from(active),
            &mut self.it_state,
            cycles,
        )
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
            if self.state != CpuState::Running {
                break;
            }

            let mut step_cycles: u64 = 0;
            match self.step_instruction(bus, &mut step_cycles) {
                Ok(result) => {
                    if result == StepResult::ExceptionReturn {
                        if let Err(e) = self.return_from_exception(bus, &mut step_cycles) {
                            self.state = CpuState::Halted;
                            log::error!("CPU halted: {e:?}");
                            break;
                        }
                    }
                    if let Err(e) = self.check_pending_exceptions(bus, &mut step_cycles) {
                        self.state = CpuState::Halted;
                        log::error!("CPU halted: {e:?}");
                        break;
                    }
                    self.total_cycles += step_cycles;
                    consumed += 1;
                    remaining -= 1;
                }
                Err(e) => {
                    self.state = CpuState::Halted;
                    log::error!("CPU halted: {e:?}");
                    break;
                }
            }
        }

        (consumed, self.state)
    }

    fn step(&mut self, bus: &mut dyn AddressBus) -> Result<CpuState, CoreError> {
        if self.state != CpuState::Running {
            return Ok(self.state);
        }

        let mut cycle_count: u64 = 0;
        match self.step_instruction(bus, &mut cycle_count) {
            Ok(result) => {
                if result == StepResult::ExceptionReturn {
                    if let Err(e) = self.return_from_exception(bus, &mut cycle_count) {
                        self.state = CpuState::Halted;
                        return Err(e);
                    }
                }
                if let Err(e) = self.check_pending_exceptions(bus, &mut cycle_count) {
                    self.state = CpuState::Halted;
                    return Err(e);
                }
                self.total_cycles += cycle_count;
                Ok(self.state)
            }
            Err(e) => {
                self.state = CpuState::Halted;
                Err(e)
            }
        }
    }

    fn signal_interrupt(&mut self, irq: IrqNumber) {
        self.nvic.set_pending(irq);
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
