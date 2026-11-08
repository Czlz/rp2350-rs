//! rp2350-core — RP2350 `SoC` topology, address bus, GPIO, IRQ matrix, clock simulation
//!
//! This crate provides the foundational types and simulation infrastructure for the
//! RP2350 microcontroller. It defines the address space layout, GPIO pin model,
//! interrupt routing matrix, and the chip topology struct that connects all
//! peripherals together.
//!
//! 本 crate 提供 RP2350 微控制器的基础类型和仿真基础设施，定义地址空间布局、
//! GPIO 引脚模型、中断路由矩阵，以及连接所有外设的芯片拓扑结构。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod bus;
pub mod chip;
pub mod clock;
pub mod error;
pub mod gpio;
pub mod irq;
pub mod types;

pub use bus::{AddressBus, BusResult, MemoryRegion};
pub use chip::RP2350Chip;
pub use clock::SimulationClock;
pub use error::{BusErrorKind, CoreError};
pub use gpio::{GpioPin, PinDirection, PinFunction};
pub use irq::IrqMatrix;
pub use types::{Address, CycleCount, GpioNumber, IrqNumber, PinNumber};
