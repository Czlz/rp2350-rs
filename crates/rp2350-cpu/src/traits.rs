//! `CpuCore` trait — the contract all CPU implementations must satisfy
//! `CpuCore` trait — 所有 CPU 实现必须满足的契约

use rp2350_core::bus::AddressBus;
use rp2350_core::types::{CpuId, CycleCount, IrqNumber};

use super::{CpuArch, CpuState};

/// A CPU core capable of executing instructions and handling interrupts.
/// 能够执行指令并处理中断的 CPU 核心。
///
/// # Lifecycle / 生命周期
///
/// 1. `init()` — called once at simulation start / 仿真启动时调用一次
/// 2. `run_cycles(n)` — execute `n` cycles of instructions / 执行 n 个周期的指令
/// 3. `signal_interrupt(n)` — raise an IRQ for this core / 触发核心中断
/// 4. `reset()` — reset the core to power-on state / 复位核心到上电状态
pub trait CpuCore: Send + Sync {
    /// Return this core's ID / 返回此核心的 ID
    fn cpu_id(&self) -> CpuId;

    /// Return this core's architecture / 返回此核心的架构
    fn arch(&self) -> CpuArch;

    /// Initialize the core (bootrom, reset vector, SP/PC)
    /// 初始化核心（bootrom、复位向量、SP/PC）
    ///
    /// # Errors
    /// Returns `CoreError` if the bus read/write fails during initialization.
    /// 如果初始化期间总线读写失败，返回 `CoreError`。
    fn init(&mut self, bus: &mut dyn AddressBus) -> Result<(), rp2350_core::error::CoreError>;

    /// Execute `cycles` worth of instructions
    /// 执行相当于 `cycles` 个周期的指令
    ///
    /// Returns the actual number of cycles executed and the new CPU state.
    /// 返回实际执行的周期数和新 CPU 状态。
    fn run_cycles(
        &mut self,
        bus: &mut dyn AddressBus,
        cycles: CycleCount,
    ) -> (CycleCount, CpuState);

    /// Execute a single instruction (debug step)
    /// 执行单条指令（调试单步）
    ///
    /// # Errors
    /// Returns `CoreError` if instruction execution fails (e.g., `BusError`).
    /// 如果指令执行失败（如总线错误），返回 `CoreError`。
    fn step(&mut self, bus: &mut dyn AddressBus)
    -> Result<CpuState, rp2350_core::error::CoreError>;

    /// Signal an interrupt to this core / 向此核心发送中断信号
    fn signal_interrupt(&mut self, irq: IrqNumber);

    /// Check if an IRQ is currently being serviced / 检查 IRQ 是否正在服务中
    fn servicing_irq(&self) -> Option<IrqNumber>;

    /// Read the program counter (PC) / 读取程序计数器
    fn pc(&self) -> u32;

    /// Read a general-purpose register / 读取通用寄存器
    fn read_register(&self, index: u8) -> u32;

    /// Write a general-purpose register / 写入通用寄存器
    fn write_register(&mut self, index: u8, value: u32);

    /// Reset to power-on state / 复位到上电状态
    fn reset(&mut self);

    /// Get human-readable core description / 获取可读的核心描述
    fn description(&self) -> String;
}
