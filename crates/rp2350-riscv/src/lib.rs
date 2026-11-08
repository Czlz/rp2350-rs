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

/// Picobin header magic at the start of an RP2350 RISC-V image
/// RP2350 RISC-V 映像起始处的 picobin 头魔数
pub const RISCV_BOOT_MAGIC_ADDR: u32 = 0x1000_0000;

/// Expected picobin magic word / 期望的 picobin 魔数字
pub const RISCV_BOOT_MAGIC: u32 = 0x7188_EBF2;

/// Address of the entry point word in the picobin header
/// picobin 头中入口点字的地址
pub const RISCV_BOOT_ENTRY_ADDR: u32 = 0x1000_0004;

/// Address of the initial stack pointer word in the picobin header
/// picobin 头中初始栈指针字的地址
pub const RISCV_BOOT_SP_ADDR: u32 = 0x1000_0020;

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
    /// Pending external IRQ mask (bits 0..63) / 待处理外部中断掩码
    pending_irqs: u64,
    /// Cached boot vectors (SP, PC) / 缓存的启动向量（SP、PC）
    boot_vectors: Option<(u32, u32)>,
    /// Cumulative executed instructions since reset / 复位以来累计执行的指令数
    total_cycles: u64,
    /// Load reservation address for `lr.w`/`sc.w` / `lr.w`/`sc.w` 的保留地址
    reservation: Option<u32>,
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
            pending_irqs: 0,
            boot_vectors: None,
            total_cycles: 0,
            reservation: None,
        }
    }

    /// Override the boot vectors used by [`CpuCore::init`] / 覆盖启动向量
    ///
    /// `sp` is the initial stack pointer and `pc` the first instruction address.
    /// `pc` has its low bit cleared, matching the alignment rule.
    /// `sp` 为初始栈指针，`pc` 为第一条指令地址；`pc` 最低位会被清零。
    pub fn set_boot_vectors(&mut self, sp: u32, pc: u32) {
        self.boot_vectors = Some((sp, pc & !1));
    }

    /// Load the boot vectors and enter the running state / 加载启动向量并进入运行状态
    ///
    /// # Errors
    /// Returns `CoreError::FirmwareError` when no explicit vectors were supplied
    /// and the bus does not contain a valid picobin header at flash base.
    /// 未显式提供向量且总线在 Flash 起始处没有合法 picobin 头时，返回
    /// `CoreError::FirmwareError`。
    pub fn boot(&mut self, bus: &mut dyn AddressBus) -> Result<(), CoreError> {
        let (sp, pc) = if let Some(vectors) = self.boot_vectors {
            vectors
        } else {
            let mut cycles = 0u64;
            let magic = bus.read_u32(RISCV_BOOT_MAGIC_ADDR, &mut cycles)?;
            if magic != RISCV_BOOT_MAGIC {
                return Err(CoreError::FirmwareError(format!(
                    "RISC-V boot: picobin magic {magic:#010X} at {RISCV_BOOT_MAGIC_ADDR:#010X}, \
                     expected {RISCV_BOOT_MAGIC:#010X} / RISC-V 启动头魔数不匹配"
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
        self.pending_irqs = 0;
        self.total_cycles = 0;
        self.reservation = None;
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

    /// Return the highest pending external IRQ / 返回最高优先级的待处理外部中断
    fn pending_interrupt(&self) -> Option<IrqNumber> {
        let globally_enabled = self.csr.mstatus & csr::MSTATUS_MIE != 0;
        let external_enabled = self.csr.mie & csr::MIE_MEIE != 0;
        if !globally_enabled || !external_enabled {
            return None;
        }
        (0..64)
            .find(|bit| self.pending_irqs & (1u64 << bit) != 0)
            .map(|bit| u16::try_from(bit).unwrap_or(u16::MAX))
    }

    /// Enter the external interrupt handler / 进入外部中断处理程序
    fn take_interrupt(&mut self, irq: IrqNumber) {
        if irq < 64 {
            self.pending_irqs &= !(1u64 << irq);
        }
        if self.pending_irqs == 0 {
            self.csr.mip &= !csr::MIP_MEIP;
        }
        self.csr.meipa = u32::from(irq);
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
        self.csr.tick();
        self.total_cycles += 1;
        Ok(())
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
        while remaining > 0 && self.state == CpuState::Running {
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
        if self.state != CpuState::Running {
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
            self.pending_irqs |= 1u64 << irq;
            self.csr.mip |= csr::MIP_MEIP;
        }
        if self.state == CpuState::Sleeping {
            self.state = CpuState::Running;
        }
    }

    fn servicing_irq(&self) -> Option<IrqNumber> {
        self.pending_interrupt()
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
