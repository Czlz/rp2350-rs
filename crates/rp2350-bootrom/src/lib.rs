//! rp2350-bootrom — synthetic boot ROM images and host-side trap dispatch
//!
//! Vendor boot ROM binaries cannot be redistributed with this repository, so the
//! emulator builds a synthetic image instead: the documented header fields are
//! present, table entries point at `udf` stubs, and the host executes the
//! equivalent behaviour when execution reaches a stub address.
//!
//! The crate is organised per chip. [`table`], [`image`], [`region`] and
//! [`trap`] are chip-agnostic; [`rp2350`] is the first concrete layout and
//! future RP2040 support can add a sibling module without touching the public
//! plumbing. Nothing here depends on the CLI, GDB server or GUI.
//!
//! 本仓库不能分发厂商 boot ROM 二进制，因此模拟器构建合成镜像：保留文档规定的
//! 头部字段，表项指向 `udf` 占位桩，执行到达桩地址时由宿主执行等价行为。
//!
//! crate 按芯片组织：[`table`]、[`image`]、[`region`]、[`trap`] 与芯片无关，
//! [`rp2350`] 是首个具体布局，未来 RP2040 可新增同级模块而不改动公共管线。
//! 本 crate 不依赖 CLI、GDB 服务器或 GUI。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod image;
pub mod region;
pub mod rp2350;
pub mod table;
pub mod trap;

pub use image::RomImageBuilder;
pub use region::RomRegion;
pub use rp2350::{ROM_BASE, ROM_MAGIC, ROM_SIZE, Rp2350Bootrom};
pub use table::{RomTableBuilder, TableCode, code, flags};
pub use trap::{HaltReason, TrapCall, TrapDispatcher, TrapHandler, TrapId, TrapOutcome};
