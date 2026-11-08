//! rp2350-periph — built-in silicon peripherals (PIO, HSTX, DMA, XIP...)
//!
//! Implements the `Peripheral` trait for all RP2350 hardware peripherals.
//! Each peripheral maps to its official address region from the datasheet.
//!
//! 实现 RP2350 所有硬件外设的 `Peripheral` trait。
//! 每个外设映射到其官方数据手册中指定的地址区域。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod traits;

pub use traits::Peripheral;
