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
pub mod exceptions;
pub mod execute_thumb16;
pub mod execute_thumb32;
pub mod nvic;
pub mod registers;
pub mod systick;

/// Cortex-M33 register set / Cortex-M33 寄存器组
pub use registers::CortexM33Registers;

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

/// Address of the initial stack pointer in the boot vector table
/// 启动向量表中初始栈指针的地址
const BOOT_SP_ADDR: u32 = 0x0000_0000;

/// Address of the reset vector in the boot vector table
/// 启动向量表中复位向量的地址
const BOOT_PC_ADDR: u32 = 0x0000_0004;

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
    /// IT block state machine / IT 块状态机
    it_state: ITBlockState,
    /// Current execution state / 当前执行状态
    state: CpuState,
    /// Pending IRQ number (if any) / 待处理的中断号（如果有）
    pending_irq: Option<IrqNumber>,
    /// Boot vectors cached by `boot` as (initial MSP, reset vector)
    /// 由 `boot` 缓存的启动向量（初始 MSP、复位向量）
    boot_vectors: Option<(u32, u32)>,
    /// Cumulative cycles executed since the last reset
    /// 自上次复位以来累计执行的周期数
    total_cycles: u64,
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
            it_state: ITBlockState::new(),
            state: CpuState::Reset,
            pending_irq: None,
            boot_vectors: None,
            total_cycles: 0,
        }
    }

    /// Load the boot vector table and start execution
    /// 加载启动向量表并开始执行
    ///
    /// Reads the initial MSP from 0x0000_0000 and the reset vector from
    /// 0x0000_0004, caches both, then applies them to the register file. The
    /// cached values let `CpuCore::reset` restart the core without bus access.
    /// 从 0x0000_0000 读取初始 MSP、从 0x0000_0004 读取复位向量，缓存后写入
    /// 寄存器文件。缓存值使得 `CpuCore::reset` 无需总线访问即可重新启动核心。
    ///
    /// # Errors
    /// Returns a bus error if the vector table is not mapped on `bus`.
    /// 若向量表未映射到 `bus`，返回总线错误。
    pub fn boot(&mut self, bus: &mut dyn AddressBus) -> Result<(), CoreError> {
        let mut cycles = 0u64;
        let msp = bus.read_u32(BOOT_SP_ADDR, &mut cycles)?;
        let reset = bus.read_u32(BOOT_PC_ADDR, &mut cycles)?;
        self.boot_vectors = Some((msp, reset));
        self.apply_boot_vectors();
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
        self.it_state = ITBlockState::new();
        self.pending_irq = None;
        self.total_cycles = 0;
    }

    /// Apply cached boot vectors, entering Running only if they exist
    /// 应用缓存的启动向量，仅当向量存在时进入运行状态
    fn apply_boot_vectors(&mut self) {
        self.clear_core_state();
        if let Some((msp, reset)) = self.boot_vectors {
            self.registers.msp = msp;
            self.registers.sp = msp;
            self.registers.pc = reset & !1;
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
    #[allow(dead_code)]
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

    /// Handle NVIC register accesses via bus
    /// 通过总线处理 NVIC 寄存器访问
    #[allow(dead_code)]
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

        let result = if instr_type >= 0b11101 {
            // Thumb-32: fetch second halfword
            // Thumb-32：获取第二个半字
            self.registers.pc = self.registers.pc.wrapping_add(2);
            let hw2 = self.fetch_halfword(bus, cycles)?;
            // ARM 体系参考手册约定：第一个半字位于高 16 位，第二个半字位于低 16 位。
            // ARM ARM convention: first halfword in bits[31:16], second in bits[15:0].
            let instr32 = (hw as u32) << 16 | hw2 as u32;
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

        if let Some(irq) = self.nvic.find_highest_priority() {
            exceptions::enter_exception(
                bus,
                &mut self.registers,
                &mut self.nvic,
                irq as u32,
                0, // Default VTOR = 0
                cycles,
            )?;
        }

        Ok(())
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
                Ok(_) => {
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
            Ok(_) => {
                self.check_pending_exceptions(bus, &mut cycle_count)?;
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
