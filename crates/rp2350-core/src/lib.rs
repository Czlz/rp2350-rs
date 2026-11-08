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
pub mod chip_family;
pub mod clock;
pub mod error;
pub mod firmware;
pub mod flash;
pub mod gpio;
pub mod irq;
pub mod memory;
pub mod mmio;
pub mod types;
pub mod uf2;

pub use bus::{AccessWidth, AddressBus, BusResult, MemoryRegion};
pub use chip::RP2350Chip;
pub use chip_family::{ChipCapabilities, ChipFamily, ChipInfo, RP2040Chip};
pub use clock::SimulationClock;
pub use error::{BusErrorKind, CoreError};
pub use firmware::{FirmwareImage, FirmwareSegment};
pub use flash::{FLASH_XIP_BASE, FLASH_XIP_NOCACHE_BASE, FLASH_XIP_SIZE, FlashHandle, FlashRegion};
pub use gpio::{GpioPin, PinDirection, PinFunction};
pub use irq::IrqMatrix;
pub use memory::{RamRegion, SimpleBus};
pub use mmio::{
    MMIO_REGISTER_SIZE, MmioDevice, MmioRegion, MmioRouter, MmioWindow, RegisterFile, mount_device,
};
pub use types::{Address, CpuId, CycleCount, GpioNumber, IrqNumber, PinNumber};
pub use uf2::{Uf2Arch, Uf2Block, Uf2Family, Uf2Image, Uf2LoadReport, Uf2Region};
