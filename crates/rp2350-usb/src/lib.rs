//! rp2350-usb — RP2350 USB full-speed device controller model
//! rp2350-usb — RP2350 全速 USB 设备控制器模型
//!
//! This crate models the device-side subset of the RP2350 USB controller that
//! MicroPython's TinyUSB port (`dcd_rp2040`) actually touches: the 4 KB DPSRAM
//! endpoint buffer block at `0x5010_0000`, the control register block at
//! `0x5011_0000` (including the XOR/SET/CLR atomic aliases), endpoint buffer
//! handshakes, and the device-mode interrupt aggregation.
//! 本 crate 建模 MicroPython TinyUSB 端口（`dcd_rp2040`）实际使用的 RP2350 USB
//! 设备侧子集：位于 `0x5010_0000` 的 4 KB DPSRAM 端点缓冲、位于 `0x5011_0000`
//! 的控制寄存器块（含 XOR/SET/CLR 原子别名）、端点缓冲握手以及设备模式中断聚合。
//!
//! The controller is split into three layers:
//! 控制器分为三层：
//!
//! - [`UsbCore`] — pure register/DPRAM state machine, no locking.
//!   [`UsbCore`] — 纯寄存器/DPRAM 状态机，不含锁。
//! - [`regions`] — `MemoryRegion` views used by the address bus.
//!   [`regions`] — 供地址总线使用的 `MemoryRegion` 视图。
//! - [`Controller`] — shared ownership facade plus the `Peripheral` contract.
//!   [`Controller`] — 共享所有权门面以及 `Peripheral` 契约。
//!
//! Host-side transactions are injected through `UsbCore::host_*` methods. They
//! are what an external host bridge drives; no CPU or clock is required to unit
//! test them.
//! 主机侧事务通过 `UsbCore::host_*` 方法注入，外部主机桥即驱动这些方法；单元
//! 测试无需 CPU 或时钟参与。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]
// USB 寄存器与 DPSRAM 仿真涉及地址、偏移与位宽之间的整数换算，这是硬件仿真层
// 的固有特性。
// USB register and DPSRAM emulation converts between addresses, offsets and
// widths, which is inherent to a hardware emulation layer.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::doc_markdown
)]

pub mod controller;
pub mod device;
pub mod endpoint;
pub mod mmio;
pub mod regions;
pub mod regs;
pub mod setup;

pub use controller::Controller;
pub use device::UsbCore;
pub use mmio::{DpramMmio, RegsMmio, decode_alias, mount_usb};
pub use regions::{DpramRegion, RegsRegion, UsbPeripheral};

/// Construct USBCTRL and mount both of its windows on `bus`
/// 构造 USBCTRL 并在 `bus` 上挂载它的两个窗口
///
/// This is the CLI-facing entry point: it registers the 4 KB DPSRAM window
/// (`0x5010_0000`) and the 16 KB register window with aliases
/// (`0x5011_0000`), then returns the [`Controller`] handle used by the
/// host-side bridge.
/// 这是面向 CLI 的入口：注册 4 KB DPSRAM 窗口（`0x5010_0000`）与带别名的
/// 16 KB 寄存器窗口（`0x5011_0000`），并返回主机桥使用的 [`Controller`] 句柄。
///
/// Register offsets are always readable once mounted; unmodelled bits read as
/// zero, so firmware `read-modify-write` cycles such as `0x5011_004C`
/// (`SIE_CTRL`) no longer produce a `NoDevice` bus error.
/// 挂载后所有寄存器偏移都可读；未建模位读作零，因此固件对 `0x5011_004C`
/// （`SIE_CTRL`）等寄存器的读-改-写不再产生 `NoDevice` 总线错误。
///
/// # Errors
/// Returns a bus error when either window is invalid or overlaps a region
/// already registered on `bus`.
/// 任一窗口非法或与 `bus` 上已注册区域重叠时返回总线错误。
pub fn mount(
    bus: &mut dyn rp2350_core::bus::AddressBus,
) -> rp2350_core::bus::BusResult<Controller> {
    let controller = Controller::new();
    controller.mount_mmio(bus)?;
    Ok(controller)
}
pub use regs::Direction;
pub use setup::SetupPacket;

use rp2350_core::types::Address;

/// USBCTRL DPSRAM base address / USBCTRL DPSRAM 基地址
pub const DPRAM_BASE: Address = 0x5010_0000;

/// USBCTRL DPSRAM size in bytes / USBCTRL DPSRAM 字节大小
pub const DPRAM_SIZE: usize = 0x1000;

/// USBCTRL_REGS base address / USBCTRL_REGS 基地址
pub const REGS_BASE: Address = 0x5011_0000;

/// Control register block size in bytes / 控制寄存器块字节大小
pub const REGS_BLOCK_SIZE: usize = 0x1000;

/// Address window covering RW/XOR/SET/CLR aliases / 覆盖 RW/XOR/SET/CLR 别名的地址窗口
pub const REGS_REGION_SIZE: usize = 0x4000;

/// Interrupt number of the USB controller on RP2350 / RP2350 上 USB 控制器的中断号
pub const USBCTRL_IRQ: u16 = 14;
