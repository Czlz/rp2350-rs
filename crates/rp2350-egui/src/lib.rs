//! rp2350-egui — Virtual Electronics Lab GUI
//!
//! Provides the eframe-based GUI application: Virtual Lab canvas with
//! drag-and-drop GPIO wiring, HSTX video output panel, register/memory
//! viewers, debug controls, and virtual peripheral device widgets.
//!
//! Landed today: the register viewer, the read-only DMA, HSTX and PIO
//! peripheral panels, and the Virtual Lab wiring canvas (device nodes, ports,
//! drag-to-wire, delete, pan and zoom). The canvas ships a built-in device
//! palette and persists to a dependency-free text format ([`CanvasState::to_text`]
//! / [`CanvasState::from_text`]). The panels read snapshots through
//! [`app::PeripheralSource`] and render `not exposed` when the host machine has
//! not attached a source yet. The canvas is an editor model only: it does not
//! drive the simulator.
//!
//! 提供基于 eframe 的 GUI 应用：虚拟实验室画布（拖拽 GPIO 接线）、
//! HSTX 视频输出面板、寄存器/内存查看器、调试控件和虚拟外设小部件。
//!
//! 当前已落地：寄存器查看器、只读的 DMA、HSTX 与 PIO 外设面板，以及虚拟实验室
//! 连线画布（设备节点、端口、拖拽连线、删除、平移与缩放）。画布内置设备调色板，
//! 并以无额外依赖的文本格式保存/加载（[`CanvasState::to_text`] /
//! [`CanvasState::from_text`]）。面板通过 [`app::PeripheralSource`] 读取快照，
//! 宿主机器尚未挂载来源时渲染 `not exposed`。画布仅为编辑模型：不驱动仿真。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod app;
pub mod canvas;
pub mod canvas_io;
mod canvas_view;
pub mod palette;
pub mod panels;
pub mod play_view;

#[cfg(test)]
mod canvas_tests;

pub use app::EmuApp;
pub use canvas::CanvasState;
pub use canvas_io::CanvasParseError;
pub use palette::DevicePalette;
pub use play_view::{
    CliPlaySource, CliPlayStarter, PlayFrame, PlayRequest, PlaySource, PlayStarter, PlayStats,
    PlayView,
};
