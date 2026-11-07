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

pub mod registers;

/// Cortex-M33 register set / Cortex-M33 寄存器组
pub use registers::CortexM33Registers;

use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;
use rp2350_core::types::{CpuId, CycleCount, IrqNumber};
use rp2350_cpu::{CpuArch, CpuCore, CpuState};

/// Cortex-M33 processor core
/// Cortex-M33 处理器核心
pub struct CortexM33Core {
    /// Core ID (Core0 or Core1) / 核心 ID
    cpu_id: CpuId,
    /// CPU registers / CPU 寄存器
    registers: CortexM33Registers,
    /// Current execution state / 当前执行状态
    state: CpuState,
    /// Pending IRQ number (if any) / 待处理的中断号（如果有）
    pending_irq: Option<IrqNumber>,
}

impl CortexM33Core {
    /// Create a new Cortex-M33 core in reset state
    /// 创建一个处于复位状态的 Cortex-M33 核心
    #[must_use]
    pub fn new(cpu_id: CpuId) -> Self {
        Self {
            cpu_id,
            registers: CortexM33Registers::new(),
            state: CpuState::Reset,
            pending_irq: None,
        }
    }
}

impl CpuCore for CortexM33Core {
    fn cpu_id(&self) -> CpuId {
        self.cpu_id
    }

    fn arch(&self) -> CpuArch {
        CpuArch::ArmCortexM33
    }

    fn init(&mut self, _bus: &mut dyn AddressBus) -> Result<(), CoreError> {
        self.state = CpuState::Running;
        Ok(())
    }

    fn run_cycles(&mut self, _bus: &mut dyn AddressBus, cycles: CycleCount) -> (CycleCount, CpuState) {
        // Phase 2: implement Thumb-16/32 instruction execution
        (cycles, self.state)
    }

    fn step(&mut self, _bus: &mut dyn AddressBus) -> Result<CpuState, CoreError> {
        // Phase 2: implement single-step
        Ok(self.state)
    }

    fn signal_interrupt(&mut self, irq: IrqNumber) {
        self.pending_irq = Some(irq);
    }

    fn servicing_irq(&self) -> Option<IrqNumber> {
        self.pending_irq
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
        self.registers = CortexM33Registers::new();
        self.state = CpuState::Reset;
        self.pending_irq = None;
    }

    fn description(&self) -> String {
        format!(
            "ARM Cortex-M33 Core{} (PC=0x{:08X}, {:?})",
            self.cpu_id as u8,
            self.pc(),
            self.state
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_core_is_in_reset() {
        let core = CortexM33Core::new(CpuId::Core0);
        assert_eq!(core.cpu_id(), CpuId::Core0);
        assert_eq!(core.state, CpuState::Reset);
    }
}