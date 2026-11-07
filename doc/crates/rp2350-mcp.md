# rp2350-mcp — MCP Server (AgentFuse) / MCP 服务器

> **Crate**: `rp2350-mcp` | **Path**: `crates/rp2350-mcp` | **Status**: ✅ Skeleton (Phase 1)

---

## Overview / 概述

MCP (Model-Context-Protocol) JSON-RPC server for AgentFuse integration. Exposes emulator state, device queries, and configuration capabilities over HTTP. Designed for headless operation — the authoritative integration point between the emulator and AI-assisted development workflows.

面向 AgentFuse 集成的 MCP (Model-Context-Protocol) JSON-RPC 服务器。通过 HTTP 暴露模拟器状态、设备查询和配置能力。设计用于无头操作 —— 模拟器与 AI 辅助开发工作流之间的权威集成点。

---

## API Methods / API 方法

### `query_all` — 查询所有设备状态

```json
// Request
{ "jsonrpc": "2.0", "method": "query_all", "id": 1 }

// Response
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "devices": [
      { "id": "led", "name": "User LED", "state": { "level": true, "gpio_pin": 25 } },
      { "id": "uart0", "name": "UART Terminal 0", "state": { "tx_pin": 0, "rx_pin": 1, "baud": 115200 } }
    ],
    "cpu": { "pc": 268435456, "state": "running", "cycles": 1024000 },
    "gpio_summary": { "pins_active": 5, "pins_total": 48 }
  }
}
```

### `query_device` — 查询单个设备

```json
// Request
{ "jsonrpc": "2.0", "method": "query_device", "params": { "id": "led" }, "id": 2 }

// Response
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": { "id": "led", "name": "User LED", "state": { "level": true, "gpio_pin": 25 } }
}
```

### `configure` — 配置设备参数

```json
// Request
{ "jsonrpc": "2.0", "method": "configure", "params": { "id": "uart0", "config": { "baud": 9600 } }, "id": 3 }

// Response
{ "jsonrpc": "2.0", "id": 3, "result": { "ok": true } }
```

### `list_devices` — 列出所有已加载设备

```json
// Request
{ "jsonrpc": "2.0", "method": "list_devices", "id": 4 }

// Response
{
  "jsonrpc": "2.0",
  "id": 4,
  "result": [
    { "id": "led", "name": "User LED", "type": "DevicePlugin", "has_ui": false },
    { "id": "uart0", "name": "UART Terminal 0", "type": "DevicePlugin", "has_ui": true }
  ]
}
```

---

## Architecture / 架构

```
AgentFuse IDE → HTTP POST /mcp → McpServer
                                    ├── DeviceManager::mcp_query_all()
                                    ├── DeviceManager::mcp_query_device()
                                    ├── DeviceManager::apply_config()
                                    └── Response JSON
```

---

## Usage Scenarios / 使用场景

### 1. AI-Assisted Debugging / AI 辅助调试

AgentFuse can query the emulator state mid-simulation, correlate it with source code, and suggest fixes or explain behavior.

AgentFuse 可以在仿真中途查询模拟器状态，将其与源码关联，并建议修复或解释行为。

### 2. Automated Testing / 自动化测试

CI pipelines can start the emulator in headless mode with MCP, run test firmware, and assert device states via JSON-RPC.

CI 流水线可以在无头模式下启动模拟器（带 MCP），运行测试固件，并通过 JSON-RPC 断言设备状态。

### 3. Remote Lab / 远程实验室

The MCP server can be exposed over a network, enabling remote device interaction without GUI.

MCP 服务器可以通过网络暴露，实现无需 GUI 的远程设备交互。

---

## Dependencies / 依赖

| Crate | Why |
|-------|-----|
| `tokio` | Async HTTP server runtime |
| `serde` + `serde_json` | JSON-RPC protocol |
| `rp2350-virt-periph` | DeviceManager query API |
| `log` | Server lifecycle logging |

---

## WASM Note / WASM 注意事项

In WASM builds, the TCP-based HTTP server is replaced with a WebSocket-based JSON-RPC bridge. The same MCP protocol messages are tunneled through the browser's WebSocket connection.

在 WASM 构建中，基于 TCP 的 HTTP 服务器替换为基于 WebSocket 的 JSON-RPC 桥接。相同的 MCP 协议消息通过浏览器的 WebSocket 连接隧道传输。