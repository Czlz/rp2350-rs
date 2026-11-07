//! rp2350-mcp — MCP Server for AgentFuse AI integration
//!
//! Provides a JSON-RPC MCP server that exposes the emulator state
//! to AI agents. Supports querying device states, configuring devices,
//! and controlling the simulation — all without GUI.
//!
//! 提供 JSON-RPC MCP 服务器，将模拟器状态暴露给 AI 代理。
//! 支持查询设备状态、配置设备和控制仿真 — 全部无需 GUI。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod server;

pub use server::McpServer;