//! rp2350-egui — Virtual Electronics Lab GUI
//!
//! Provides the eframe-based GUI application: Virtual Lab canvas with
//! drag-and-drop GPIO wiring, HSTX video output panel, register/memory
//! viewers, debug controls, and virtual peripheral device widgets.
//!
//! Landed today: the register viewer plus the read-only DMA, HSTX and PIO
//! peripheral panels. They read snapshots through
//! [`app::PeripheralSource`] and render `not exposed` when the host machine
//! has not attached a source yet.
//!
//! 提供基于 eframe 的 GUI 应用：虚拟实验室画布（拖拽 GPIO 接线）、
//! HSTX 视频输出面板、寄存器/内存查看器、调试控件和虚拟外设小部件。
//!
//! 当前已落地：寄存器查看器，以及只读的 DMA、HSTX 与 PIO 外设面板。面板通过
//! [`app::PeripheralSource`] 读取快照，宿主机器尚未挂载来源时渲染
//! `not exposed`。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod app;
pub mod canvas;
pub mod panels;

pub use app::EmuApp;
