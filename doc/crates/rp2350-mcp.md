# rp2350-mcp — MCP Server (AgentFuse) / MCP 服务器

> **Crate**: `rp2350-mcp` | **Path**: `crates/rp2350-mcp` | **Status**: ✅ 方法层已实现 / method layer implemented — `McpServer::handle_request` 支持 `initialize` / `tools/list` / `tools/call`，并委托 `DeviceManager`。**HTTP/TCP 传输与 CLI `mcp-serve` 接线尚未实现**。
>
> **Status** (EN): The JSON-RPC method layer is implemented (`src/server.rs`, `src/tests.rs`): `initialize`, `tools/list`, `tools/call` (`list_devices` / `query_all` / `query_device` / `configure`) delegate to a shared `DeviceManager`. HTTP/TCP transport and the CLI `mcp-serve` wiring are still out of scope.

---

## Overview / 概述

MCP (Model-Context-Protocol) JSON-RPC server for AgentFuse integration. The
current implementation is the **transport-agnostic method layer**: it accepts a
JSON-RPC request string and returns a JSON-RPC response string. A future HTTP
listener (or a WASM WebSocket bridge) only needs to forward bodies to
`handle_request`.
面向 AgentFuse 集成的 MCP (Model-Context-Protocol) JSON-RPC 服务器。当前实现为
**与传输无关的方法层**：输入 JSON-RPC 请求字符串，返回 JSON-RPC 响应字符串。
未来的 HTTP 监听器（或 WASM WebSocket 桥）只需把报文体转交给
`handle_request`。

---

## Architecture / 架构

```
AgentFuse IDE → (future HTTP/WS transport)
                     └── McpServer::handle_request(&str) -> String
                             ├── initialize
                             ├── tools/list
                             └── tools/call
                                     ├── list_devices → DeviceManager::mcp_list_devices()
                                     ├── query_all    → DeviceManager::mcp_query_all()
                                     ├── query_device → DeviceManager::mcp_query_device()
                                     └── configure    → DeviceManager::apply_config()
```

`McpServer` shares `Arc<parking_lot::Mutex<DeviceManager>>` with the host, so the
simulation keeps registering/wiring devices while MCP only reads state or
applies configuration.
`McpServer` 与宿主共享 `Arc<parking_lot::Mutex<DeviceManager>>`：仿真侧继续
注册/接线设备，MCP 侧只读取状态或应用配置。

---

## Public API / 公共 API

```rust
pub struct McpServer { /* port, running, Arc<Mutex<DeviceManager>> */ }

impl McpServer {
    /// Own empty DeviceManager / 自带空设备管理器
    pub fn new(port: u16) -> Self;
    /// Share the host's manager / 共享宿主的设备管理器
    pub fn with_manager(port: u16, manager: Arc<Mutex<DeviceManager>>) -> Self;

    pub fn manager(&self) -> Arc<Mutex<DeviceManager>>;  // shared handle / 共享句柄
    pub fn port(&self) -> u16;
    pub fn is_running(&self) -> bool;

    pub async fn start(&mut self) -> Result<(), std::io::Error>;  // 仅置标志 + 日志
    pub fn stop(&mut self);

    /// Handle one JSON-RPC request / 处理单个 JSON-RPC 请求
    pub fn handle_request(&self, request: &str) -> String;

    /// Direct (non-JSON-RPC) helpers / 直接调用辅助方法
    pub fn query_all(&self) -> serde_json::Value;
    pub fn configure(&mut self, device_id: &str, config: &serde_json::Value) -> Result<(), String>;
}
```

`new(port)` keeps the original signature and creates a private empty
`DeviceManager`; hosts that already own devices must use `with_manager`.
`new(port)` 保留原签名，内部创建空的私有 `DeviceManager`；已有设备的宿主必须使用
`with_manager`。

---

## JSON-RPC Methods / JSON-RPC 方法

| Method | Params | Result |
|--------|--------|--------|
| `initialize` | — | `{protocolVersion, capabilities.tools, serverInfo}` |
| `tools/list` | — | `{tools: [list_devices, query_all, query_device, configure]}` |
| `tools/call` | `{name, arguments}` | MCP tool result（见下 / see below） |
| `notifications/*` | — | 空字符串（通知无需响应 / notifications get no response） |

### `initialize`

```json
// Request
{ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }

// Response
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": "2024-11-05",
    "capabilities": { "tools": { "listChanged": false } },
    "serverInfo": { "name": "rp2350-mcp", "version": "0.1.0" }
  }
}
```

### `tools/list`

Returns four tools with JSON Schemas. `list_devices` / `query_all` take no
arguments; `query_device` requires `id`; `configure` requires `id` and `config`.
返回四个带 JSON Schema 的工具。`list_devices` / `query_all` 无参数；
`query_device` 需要 `id`；`configure` 需要 `id` 与 `config`。

### `tools/call`

Result envelope (MCP-compatible; the payload is serialized into `content[0].text`):

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "content": [{ "type": "text", "text": "{\"ok\":true,\"id\":\"led-1\"}" }],
    "isError": false
  }
}
```

Tool payloads:

| Tool | Arguments | Payload |
|------|-----------|---------|
| `list_devices` | — | `[{id, name, has_ui, tags}]`（按 id 排序 / sorted by id） |
| `query_all` | — | `{ "<id>": <state_snapshot>, ... }` |
| `query_device` | `{id}` | `{id, state: <state_snapshot>}` |
| `configure` | `{id, config}` | `{ok: true, id}` |

Request examples:

```json
{ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
  "params": { "name": "query_device", "arguments": { "id": "led-1" } } }

{ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
  "params": { "name": "configure", "arguments": { "id": "led-1", "config": { "interval_ms": 250 } } } }
```

`configure` 直接调用 `DevicePlugin::apply_config`；插件返回的 `ConfigError`
原样上报，不会回滚或缓存配置。
`configure` calls `DevicePlugin::apply_config` directly; a plugin `ConfigError`
is reported as-is with no rollback or caching of configurations.

---

## Errors / 错误

Successful responses carry `result`; failures carry `error: {code, message}` and
no `result`. Codes:

| Code | Meaning / 含义 |
|------|----------------|
| `-32700` | 非法 JSON / parse error |
| `-32600` | 非法请求（缺少 `method` 或 `jsonrpc` 非 `"2.0"`） |
| `-32601` | 未知方法 / unknown method |
| `-32602` | 参数无效、未知工具、缺少必填参数 / invalid params, unknown tool |
| `-32001` | 设备未注册（`query_device`） / device not found |
| `-32002` | 配置失败（未知设备或插件拒绝配置） / configure failed |

```json
// Unknown device
{ "jsonrpc": "2.0", "id": 2,
  "error": { "code": -32001, "message": "Device not found: led-9" } }
```

`configure` 失败统一使用 `-32002`，`message` 内区分未知设备与插件拒绝；
需要精确错误类型的调用方可直接用 `McpServer::configure` 或
`DeviceManager::apply_config`（返回 `ConfigApplyError`）。
`configure` failures always use `-32002`; the message distinguishes an unknown
device from a plugin rejection. Callers needing exact typed errors should use
`McpServer::configure` or `DeviceManager::apply_config` (`ConfigApplyError`).

---

## Usage / 使用示例

```rust
use std::sync::Arc;
use parking_lot::Mutex;
use rp2350_mcp::McpServer;
use rp2350_virt_periph::DeviceManager;

let manager = Arc::new(Mutex::new(DeviceManager::new()));
manager.lock().register("led-1".into(), Box::new(my_led))?;

let server = McpServer::with_manager(8765, Arc::clone(&manager));
let response = server.handle_request(
    r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"query_all"}}"#,
);
assert!(response.contains("led-1"));
```

---

## Limitations / 限制

- ⬜ 无 HTTP/TCP 传输：`start()` / `stop()` 仅维护 `running` 标志并写日志，不监听端口。
- ⬜ 无 CLI `mcp-serve` 接线；CLI 侧仍需自行创建 `McpServer` 并调用 `handle_request`。
- ⬜ 无插件热加载；设备由宿主在进程内注册（`DeviceManager::register` / `load_plugin`）。
- ⬜ MCP 子集：未实现 `resources/*`、`prompts/*`、采样（sampling）与客户端能力协商；
  `initialize` 不回显客户端 `protocolVersion`。
- ⬜ 通知（`notifications/*`）只返回空字符串，未更新任何会话状态。
- ⬜ `tools/call` 结果仅放入 `content[0].text`，未提供 `structuredContent`。
- ⬜ 单线程请求处理（每次调用锁住 `DeviceManager`），未做并发/背压控制。

EN: No HTTP/TCP listener, no CLI wiring, no hot reload. The MCP surface is a
subset (no resources/prompts/sampling, no client capability negotiation).
Notifications are acknowledged with an empty body. Tool payloads are delivered
as text content only. Requests are serialized through one manager mutex.

---

## Dependencies / 依赖

| Crate | Why |
|-------|-----|
| `serde` + `serde_json` | JSON-RPC 编解码 / protocol codec |
| `parking_lot` | 共享 `DeviceManager` 锁 / shared manager lock |
| `rp2350-virt-periph` | `DeviceManager` 查询与配置 API |
| `tokio` | 预留异步运行时 / reserved for the future transport |
| `log` | 生命周期日志 / lifecycle logging |
| `rp2350-plugin` (dev) | 测试用 `RpPlugin`/`DevicePlugin` 实现 |

---

## WASM Note / WASM 注意事项

In WASM builds, the TCP-based HTTP server is replaced with a WebSocket-based JSON-RPC bridge. The same MCP protocol messages are tunneled through the browser's WebSocket connection.

在 WASM 构建中，基于 TCP 的 HTTP 服务器替换为基于 WebSocket 的 JSON-RPC 桥接。相同的 MCP 协议消息通过浏览器的 WebSocket 连接隧道传输。`handle_request` 与传输无关，可直接在 WebSocket 消息回调中复用。
`handle_request` is transport-agnostic and can be reused directly from a
WebSocket message callback.
