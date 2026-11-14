//! rp2350-usb-host — host-side USB bridge and raw REPL driver
//! rp2350-usb-host — 主机侧 USB 桥与 raw REPL 驱动
//!
//! This crate sits on the host side of [`rp2350_usb::Controller`]. It is
//! layered the same way as `rp2040py`'s `RawReplRunner`:
//! 本 crate 位于 [`rp2350_usb::Controller`] 的主机侧，分层方式与 `rp2040py`
//! 的 `RawReplRunner` 一致：
//!
//! - [`bridge`] enumerates the device and discovers CDC bulk endpoints.
//!   [`bridge`] 执行枚举并发现 CDC 批量端点。
//! - [`cdc`] turns the bulk endpoints into a buffered byte channel.
//!   [`cdc`] 将批量端点转换为带缓冲的字节通道。
//! - [`repl`] implements the MicroPython raw REPL protocol over any
//!   [`repl::ByteLink`], so the protocol is independent of the transport.
//!   [`repl`] 在任意 [`repl::ByteLink`] 之上实现 MicroPython raw REPL 协议，
//!   因此协议与传输方式解耦。
//! - [`perfbench`] mirrors `tests/run-perfbench.py`'s `time norm result` flow.
//!   [`perfbench`] 复现 `tests/run-perfbench.py` 的 `time norm result` 流程。
//! - [`mask_rom`] provides the EP0 compatibility view of the boot ROM window.
//!   [`mask_rom`] 提供 EP0 兼容所需的 boot ROM 窗口视图。
//!
//! No CPU or clock is required: the guest firmware is simulated by callers
//! through the controller's `host_*` methods.
//! 无需 CPU 或时钟：客户机固件由调用方通过控制器的 `host_*` 方法模拟。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]
// 主机桥涉及端点地址、包长与周期/微秒换算，这是 USB 协议层的固有特性。
// The host bridge converts endpoint addresses, packet lengths and
// cycle/microsecond units, which is inherent to a USB protocol layer.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
    clippy::doc_markdown
)]

pub mod bridge;
pub mod cdc;
pub mod descriptor;
pub mod mask_rom;
pub mod perfbench;
pub mod repl;

pub use bridge::{CdcEndpoints, HostError, HostProgress, UsbHostBridge};
pub use cdc::{CdcChannel, CdcLink};
pub use descriptor::{ConfigInfo, DescriptorError, DeviceInfo, EndpointInfo};
pub use mask_rom::MaskRom;
pub use perfbench::{BenchError, BenchOutcome, BenchResult, assemble_bench_script};
pub use repl::{ByteLink, ExecOutput, RawReplSession, ReplError};
