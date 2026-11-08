//! rp2350-gdb — GDB RSP (Remote Serial Protocol) server
//!
//! Implements a GDB server that exposes the simulated RP2350 as a debug target
//! over TCP (:3333). Supports multi-core debugging with vCont protocol.
//!
//! 实现 GDB 服务器，通过 TCP (:3333) 将模拟的 RP2350 暴露为调试目标。
//! 支持使用 vCont 协议的多核调试。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod server;

pub use server::GdbServer;
