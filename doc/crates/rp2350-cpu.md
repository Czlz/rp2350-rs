# rp2350-cpu — CPU Abstraction Layer / CPU 抽象层

> **Crate**: `rp2350-cpu` | **Path**: `crates/rp2350-cpu` | **Status**: ✅ Skeleton (Phase 1)

---

## Overview / 概述

Defines the `CpuCore` trait — the dyn-safe contract that all CPU implementations must satisfy. Core types include `CpuId`, `CpuArch`, `CpuState`, `IrqNumber`, and `CoreError`. This crate is the **sole dependency** of `rp2350-core` for CPU-related abstraction.

定义 `CpuCore` trait —— 所有 CPU 实现必须满足的 dyn-safe 契约。核心类型包括 `CpuId`、`CpuArch`、`CpuState`、`IrqNumber` 和 `CoreError`。此 crate 是 `rp2350-core` 的 **唯一 CPU 抽象依赖**。

---

## Key Types / 关键类型

### `CpuId` — CPU 标识

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CpuId(pub u8);
```

### `CpuArch` — CPU 架构

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuArch {
    CortexM33,
    Hazard3RiscV,
}
```

### `CpuState` — CPU 状态

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuState {
    Running,
    Halted,
    Sleeping,
    LockedUp,
    Faulted(u32),
}
```

### `CpuCore` Trait — CPU 核心契约

```rust
pub trait CpuCore: Send + Sync {
    fn cpu_id(&self) -> CpuId;
    fn arch(&self) -> CpuArch;
    fn init(&mut self, bus: &mut dyn AddressBus) -> Result<(), CoreError>;
    fn run_cycles(&mut self, bus: &mut dyn AddressBus, cycles: CycleCount) -> (CycleCount, CpuState);
    fn step(&mut self, bus: &mut dyn AddressBus) -> Result<CpuState, CoreError>;
    fn signal_interrupt(&mut self, irq: IrqNumber);
    fn servicing_irq(&self) -> Option<IrqNumber>;
    fn pc(&self) -> u32;
    fn read_register(&self, index: u8) -> u32;
    fn write_register(&mut self, index: u8, value: u32);
    fn reset(&mut self);
    fn description(&self) -> String;
}
```

> **Design**: Uses `&mut dyn AddressBus` instead of generic `B: AddressBus` — enables `Vec<Box<dyn CpuCore>>` and trait objects.
> **设计**: 使用 `&mut dyn AddressBus` 而非泛型 `B: AddressBus` —— 使得 `Vec<Box<dyn CpuCore>>` 和 trait object 成为可能。

---

## Dependencies / 依赖

| Crate | Why |
|-------|-----|
| `rp2350-core` (types only) | `Address` BusType, `CoreError`, `CycleCount`, `IrqNumber` |

No external dependencies beyond workspace core types.

---

## Future Work / 后续计划

- [ ] `CpuRegisters` trait for register-level debug access
- [ ] `Coprocessor` trait for FPU / DSP extension
- [ ] RISC-V Hazard3 `CpuCore` impl (Phase 10)