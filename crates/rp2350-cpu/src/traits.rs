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

    /// Write to the architecture's system control space (NVIC, `SysTick`, SCB).
    /// 写入架构的系统控制空间（NVIC、`SysTick`、SCB）。
    ///
    /// Drivers replay bus writes captured from the private peripheral bus here
    /// so interrupt-enable and priority state reach the emulated core.
    /// 驱动把从私有外设总线捕获的写操作回放到这里，使中断使能与优先级状态
    /// 到达被模拟的核心。
    fn system_write(&mut self, _addr: u32, _value: u32) {}

    /// Read from the architecture's system control space.
    /// 读取架构的系统控制空间。
    ///
    /// Defaults to `None` for cores without a system control space.
    /// 对于没有系统控制空间的核心，默认返回 `None`。
    #[must_use]
    fn system_read(&mut self, _addr: u32) -> Option<u32> {
        None
    }

    /// Return the current execution state / 返回当前执行状态
    ///
    /// Defaults to [`CpuState::Reset`] for cores that do not track state.
    /// 对于不跟踪状态的核心，默认返回 [`CpuState::Reset`]。
    #[must_use]
    fn state(&self) -> CpuState {
        CpuState::Reset
    }

    /// Read the program status register (XPSR) / 读取程序状态寄存器（XPSR）
    ///
    /// Defaults to `0` for cores without an XPSR (e.g. RISC-V).
    /// 对于没有 XPSR 的核心（例如 RISC-V），默认返回 `0`。
    #[must_use]
    fn xpsr(&self) -> u32 {
        0
    }

    /// Return cumulative cycles executed since reset / 返回自复位以来累计执行的周期数
    ///
    /// Defaults to `0` for cores that do not track cycle counts.
    /// 对于不跟踪周期计数的核心，默认返回 `0`。
    #[must_use]
    fn total_cycles(&self) -> CycleCount {
        0
    }
}
