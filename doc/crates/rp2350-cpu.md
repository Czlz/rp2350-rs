# rp2350-cpu — CPU Abstraction Layer / CPU 抽象层

> **Crate**: `rp2350-cpu` | **Path**: `crates/rp2350-cpu` | **Status**: ✅ 已实现 — trait 与两个实现（`rp2350-arm`、`rp2350-riscv`）均已落地。

---

## Overview / 概述

Defines the `CpuCore` trait — the dyn-safe contract that all CPU
implementations must satisfy. Core types include `CpuId`, `CpuArch`,
`CpuState`. Both shipping cores — `rp2350_arm::CortexM33Core` (see
[rp2350-arm.md](rp2350-arm.md)) and `rp2350_riscv::Hazard3Core` (see
[rp2350-riscv.md](rp2350-riscv.md)) — implement this trait, and
`rp2350-cli` selects between them at runtime (`--arch arm|riscv`).

定义 `CpuCore` trait —— 所有 CPU 实现必须满足的 dyn-safe 契约。核心类型包括
`CpuId`、`CpuArch`、`CpuState`。两个已交付核心 ——
`rp2350_arm::CortexM33Core`（见 [rp2350-arm.md](rp2350-arm.md)）与
`rp2350_riscv::Hazard3Core`（见 [rp2350-riscv.md](rp2350-riscv.md)）—— 均实现
该 trait，`rp2350-cli` 通过 `--arch arm|riscv` 在运行时选择。

---

## Key Types / 关键类型

### `CpuId` / `CpuArch` / `CpuState`

```rust
// CpuId lives in rp2350-core::types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CpuId { Core0, Core1 }   // 枚举而非 newtype

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuArch {
    ArmCortexM33,   // ARMv8-M Cortex-M33
    RiscVHazard3,   // Hazard3 RV32IMAC
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuState {
    Running,      // 正常运行
    Breakpoint,   // 触发断点
    Sleeping,     // 执行 WFI/WFE
    Faulted,      // 遇到故障
    Halted,       // 调试暂停
    Reset,        // 已复位 / 尚未启动
}
```

> Note: the earlier "Phase 1 skeleton" version of this document described
> `CpuState::LockedUp`/`Faulted(u32)` and a newtype `CpuId(pub u8)`; neither
> exists in the source. The RISC-V Hazard3 core is no longer a "Phase 10"
> plan — it is implemented and used by the CLI.
> 注意：本文档旧版（Phase 1 骨架）描述的 `CpuState::LockedUp`/`Faulted(u32)`
> 与 newtype `CpuId(pub u8)` 在源码中并不存在；RISC-V Hazard3 核心也不再是
> “Phase 10 计划”—— 已实现并被 CLI 使用。

---

## `CpuCore` Trait — CPU 核心契约

```rust
pub trait CpuCore: Send + Sync {
    fn cpu_id(&self) -> CpuId;
    fn arch(&self) -> CpuArch;

    /// Initialize the core (bootrom, reset vector, SP/PC)
    /// 初始化核心（bootrom、复位向量、SP/PC）
    fn init(&mut self, bus: &mut dyn AddressBus) -> Result<(), CoreError>;

    /// Execute `cycles` worth of instructions; returns actual cycles + new state.
    /// 执行相当于 `cycles` 个周期的指令；返回实际周期数与新状态。
    fn run_cycles(&mut self, bus: &mut dyn AddressBus, cycles: CycleCount)
        -> (CycleCount, CpuState);

    /// Execute a single instruction (debug step) / 执行单条指令（调试单步）
    fn step(&mut self, bus: &mut dyn AddressBus) -> Result<CpuState, CoreError>;

    /// Signal an interrupt to this core / 向此核心发送中断信号
    fn signal_interrupt(&mut self, irq: IrqNumber);

    /// IRQ currently being serviced / 正在服务的中断
    fn servicing_irq(&self) -> Option<IrqNumber>;

    fn pc(&self) -> u32;
    fn read_register(&self, index: u8) -> u32;
    fn write_register(&mut self, index: u8, value: u32);
    fn reset(&mut self);
    fn description(&self) -> String;

    /// Write to the architecture's system control space (NVIC, SysTick, SCB).
    /// 写入架构的系统控制空间（NVIC、SysTick、SCB）。
    /// Drivers replay captured private-bus writes here so interrupt enable and
    /// priority state reach the emulated core. Default: no-op.
    /// 驱动把从私有外设总线捕获的写回放到这里，使中断使能与优先级到达被模拟
    /// 核心。默认空操作。
    fn system_write(&mut self, addr: u32, value: u32) {}

    /// Read from the system control space; `None` if unsupported. Default `None`.
    /// 读取系统控制空间；不支持时返回 `None`。默认 `None`。
    fn system_read(&mut self, addr: u32) -> Option<u32> { None }

    /// Current execution state; default `CpuState::Reset`. / 当前执行状态
    fn state(&self) -> CpuState { CpuState::Reset }

    /// Program status register; default `0` (e.g. RISC-V has no XPSR). / XPSR
    fn xpsr(&self) -> u32 { 0 }

    /// Cumulative cycles since reset; default `0`. / 自复位累计周期数
    fn total_cycles(&self) -> CycleCount { 0 }
}
```

> **Design**: `&mut dyn AddressBus` instead of generic `B: AddressBus` — enables
> `Box<dyn CpuCore>` and trait objects. The five trailing methods have default
> implementations so simpler cores only implement what they track.
> **设计**：用 `&mut dyn AddressBus` 而非泛型 —— 支持 trait object。
> 末尾五个方法有默认实现，简单核心只需实现自己跟踪的部分。

---

## Implementations / 现有实现

| Core | Crate | Notes |
|------|-------|-------|
| `CortexM33Core` | `rp2350-arm` | Thumb-16/32, NVIC, SysTick, exceptions, FPU, CP0/CP4 coprocessors |
| `Hazard3Core` | `rp2350-riscv` | RV32IMAC + C, machine CSRs, trap/mret, Xh3irq, picobin boot |

The CLI's `run` machine wraps either core as `Box<dyn CpuCore>` and drives it
through `step()`; the register GUI panel and the GDB target adapter also
consume cores only through this trait.
CLI 的 `run` 机器把任一核心包装成 `Box<dyn CpuCore>` 并通过 `step()` 驱动；
寄存器 GUI 面板与 GDB 目标适配器同样只通过该 trait 访问核心。

---

## Dependencies / 依赖

| Crate | Why |
|-------|-----|
| `rp2350-core` | `AddressBus`, `CoreError`, `CycleCount`, `CpuId`, `IrqNumber` |

No external dependencies beyond workspace core types.

---

## Future Work / 后续计划

- [ ] `CpuRegisters` trait for register-level debug access
- [ ] `Coprocessor` trait 抽象（FPU/协处理器目前在各核心内部实现）
- [ ] RP2350 双核（Core1）宿主编排
