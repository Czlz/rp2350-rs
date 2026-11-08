//! rp2350-plugin — base plugin contract for all RP2350 plugins
//!
//! Defines `RpPlugin` and `PluginContext` traits. All plugins (both pure-logic
//! and virtual devices) must implement `RpPlugin`. This crate has **zero** egui
//! dependency — rendering is handled exclusively by `DevicePlugin` in
//! `rp2350-virt-periph` via optional `has_ui()` / `render_ui()`.
//!
//! 定义 `RpPlugin` 和 `PluginContext` trait。所有插件（含纯逻辑插件和虚拟设备）
//! 都必须实现 `RpPlugin`。本 crate **零** egui 依赖 —— 渲染由
//! `rp2350-virt-periph` 中的 `DevicePlugin` 通过可选的 `has_ui()` / `render_ui()` 处理。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod context;
pub mod error;
pub mod metadata;
pub mod traits;

pub use context::{BusDevice, PluginContext, PluginContextExt};
pub use error::PluginError;
pub use metadata::PluginMetadata;
pub use traits::RpPlugin;
