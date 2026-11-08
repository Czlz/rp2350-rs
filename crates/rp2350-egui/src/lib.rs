//! rp2350-egui — Virtual Electronics Lab GUI
//!
//! Provides the eframe-based GUI application: Virtual Lab canvas with
//! drag-and-drop GPIO wiring, HSTX video output panel, register/memory
//! viewers, debug controls, and virtual peripheral device widgets.
//!
//! 提供基于 eframe 的 GUI 应用：虚拟实验室画布（拖拽 GPIO 接线）、
//! HSTX 视频输出面板、寄存器/内存查看器、调试控件和虚拟外设小部件。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod app;
pub mod canvas;
pub mod panels;

pub use app::EmuApp;
