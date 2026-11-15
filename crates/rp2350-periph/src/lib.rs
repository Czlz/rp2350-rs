//! rp2350-periph — built-in silicon peripherals (PIO, HSTX, DMA, XIP...)
//!
//! Implements the `Peripheral` trait for all RP2350 hardware peripherals.
//! Each peripheral maps to its official address region from the datasheet.
//!
//! 实现 RP2350 所有硬件外设的 `Peripheral` trait。
//! 每个外设映射到其官方数据手册中指定的地址区域。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]
// Stage 4 外设桩把寄存器地址直接换算为偏移，这是内存映射仿真层的固有特性。
// The Stage-4 peripheral stubs turn register addresses into offsets directly,
// which is inherent to the memory-mapped emulation layer.
#![allow(clippy::cast_possible_truncation)]
// 外设桩的文档统一使用数据手册中的大写外设/寄存器名（如 IO_BANK0），
// 且设备名称始终是字面量，因此放宽这两条 pedantic 检查。
// Peripheral-stub docs use datasheet uppercase peripheral/register names (e.g. IO_BANK0),
// and device names are always string literals, so these pedantic lints are relaxed.
#![allow(clippy::doc_markdown, clippy::unnecessary_literal_bound)]

pub mod adc;
mod alias;
pub mod audio;
pub mod boot;
pub mod bootram;
pub mod busctrl;
pub mod clock_tree;
pub mod clocks;
pub mod cycle;
pub mod dma;
pub mod dreq;
pub mod gpio_pads;
pub mod hstx;
pub mod hstx_pixels;
pub mod io_bank0;
pub mod io_qspi;
pub mod irq;
pub mod map;
pub mod pads_bank0;
pub mod pads_qspi;
pub mod pio;
pub mod pll;
pub mod powman;
pub mod ppb;
pub mod psm;
pub mod psram;
pub mod pwm;
pub mod resets;
pub mod rosc;
pub mod sha256;
pub mod sio;
pub mod ticks;
pub mod timer;
pub mod traits;
pub mod trng;
pub mod uart;
pub mod watchdog;
pub mod xip_qmi;
pub mod xosc;

pub use audio::{AudioSink, AudioTap};
pub use boot::{BootPeripherals, Core1Launch, Timebase};
pub use clock_tree::{ClockTree, PllId};
pub use cycle::CycleSink;
pub use dma::Dma;
pub use hstx::Hstx;
pub use irq::IrqAssert;
pub use pio::{Pio, SharedPio};
pub use traits::Peripheral;
pub use uart::UartSink;

#[cfg(test)]
mod audio_tests;
#[cfg(test)]
mod busctrl_tests;
#[cfg(test)]
mod dma_tests;
#[cfg(test)]
mod hstx_tests;
#[cfg(test)]
mod io_bank0_tests;
#[cfg(test)]
mod pin_matrix_tests;
#[cfg(test)]
mod psram_tests;
#[cfg(test)]
mod sio_spinlock_tests;
#[cfg(test)]
mod timer_tests;

use rp2350_core::mmio::MmioRegion;

impl Peripheral for MmioRegion {
    fn peripheral_name(&self) -> &str {
        self.name()
    }

    fn reset(&mut self) {
        MmioRegion::reset(self);
    }

    fn irq_number(&self) -> Option<u16> {
        self.device().irq_number()
    }
}
