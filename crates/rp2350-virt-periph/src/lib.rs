//! rp2350-virt-periph — Virtual Device Manager
//!
//! Manages the lifecycle, wiring, and GPIO routing for all virtual peripheral
//! plugins. Implements the `DevicePlugin` trait which extends `RpPlugin` with
//! GPIO port management, optional configuration, and optional GUI rendering.
//!
//! 管理所有虚拟外设插件的生命周期、接线和 GPIO 路由。实现 `DevicePlugin` trait，
//! 该 trait 扩展了 `RpPlugin`，增加了 GPIO 端口管理、可选配置和可选 GUI 渲染。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod config;
pub mod device;
pub mod error;
pub mod loader;
pub mod manager;
pub mod wiring;

pub use config::ConfigSchema;
pub use device::{DevicePlugin, DevicePort};
pub use error::DeviceError;
pub use loader::PluginLoader;
pub use manager::DeviceManager;
pub use wiring::{WiringEntry, WiringTable};
