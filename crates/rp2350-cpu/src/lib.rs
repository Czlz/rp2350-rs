//! rp2350-cpu — CPU core abstraction traits
//!
//! Defines the `CpuCore` trait that all CPU implementations must satisfy.
//! This trait abstracts instruction fetch, decode, execute, and interrupt
//! handling, allowing both ARM Cortex-M33 and RISC-V Hazard3 backends.
//!
//! 定义所有 CPU 实现必须满足的 `CpuCore` trait，抽象指令取指、
//! 解码、执行和中断处理，同时支持 ARM Cortex-M33 和 RISC-V Hazard3 后端。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod traits;

pub use traits::CpuCore;

/// CPU execution result after `run_cycles` or `step`
/// CPU 执行 `run_cycles` 或 `step` 后的执行结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuState {
    /// CPU is running normally / CPU 正常运行
    Running,
    /// CPU hit a breakpoint / CPU 触发了断点
    Breakpoint,
    /// CPU executed WFI/WFE (waiting for event) / CPU 执行了 WFI/WFE（等待事件）
    Sleeping,
    /// CPU encountered a fault / CPU 遇到故障
    Faulted,
    /// CPU halted (debug halt request) / CPU 已暂停（调试暂停请求）
    Halted,
    /// CPU reset / CPU 已复位
    Reset,
}

/// CPU architecture type / CPU 架构类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuArch {
    /// ARM Cortex-M33 (ARMv8-M)
    ArmCortexM33,
    /// RISC-V Hazard3 (RV32IMAC)
    RiscVHazard3,
}