# rp2350-mcp — MCP Server (AgentFuse) / MCP 服务器

> **Crate**: `rp2350-mcp` | **Path**: `crates/rp2350-mcp` | **Status**: ✅ 方法层与最小 HTTP/1.1 传输已实现 / method layer + minimal HTTP/1.1 transport implemented — `McpServer::handle_request` 支持 `initialize` / `tools/list` / `tools/call`，并委托 `DeviceManager`；`http::serve` 提供 `POST /mcp` 与 `GET /health`；CLI `mcp-serve` 已接线。
>
> **Status** (EN): The JSON-RPC method layer (`src/server.rs`) implements `initialize`, `tools/list`, `tools/call` (`list_devices` / `query_all` / `query_device` / `configure`) over a shared `DeviceManager`. `src/http.rs` adds a minimal blocking HTTP/1.1 transport (`POST /mcp`, `GET /health`), and the CLI `mcp-serve` subcommand drives it.

---

## Overview / 概述

MCP (Model-Context-Protocol) JSON-RPC server for AgentFuse integration. Two
independent layers:

1. **Method layer** (`src/server.rs`): accepts a JSON-RPC request string and
   returns a JSON-RPC response string; transport-agnostic.
2. **HTTP transport** (`src/http.rs`): one request per connection, `POST /mcp`
   for JSON-RPC and `GET /health` for liveness. Only a body needs forwarding to
   `handle_request`, so a WASM WebSocket bridge can reuse the same method layer.

面向 AgentFuse 集成的 MCP (Model-Context-Protocol) JSON-RPC 服务器，分为两层：

1. **方法层**（`src/server.rs`）：输入 JSON-RPC 请求字符串，返回 JSON-RPC 响应字符串，
   与传输无关。
2. **HTTP 传输**（`src/http.rs`）：每连接一条请求，`POST /mcp` 承载 JSON-RPC，
   `GET /health` 用于存活探测。只需把报文体转交 `handle_request`，因此 WASM
   WebSocket 桥可复用同一方法层。

---

## HTTP transport / HTTP 传输

```text
agent → TCP → http::serve(listener, &server, &shutdown)
                 ├── GET  /health → 200 {"status":"ok", ...}
                 └── POST /mcp | /  → McpServer::handle_request(body) → application/json
```

| Method | Path | Status | Notes / 说明 |
|--------|------|--------|--------------|
| `POST` | `/mcp`（别名 `/`） | `200` / `400` / `202` | JSON-RPC 响应；非法 JSON 用 `400` 承载 `-32700` 载荷；通知返回 `202` 空体 |
| `GET` | `/health` | `200` | `{status, server, version, endpoint}` |
| 其他方法 | `/mcp`、`/health` | `405` | 附带 `Allow: POST` / `Allow: GET` |
| 其他路径 | — | `404` | 纯文本错误体 / plain-text body |

Headers / 头部：`Content-Type: application/json`、精确的 `Content-Length`、
`Connection: close`。请求侧要求 `Content-Length`（缺失 → `411`，
`Transfer-Encoding` → `501`），请求头上限 8 KiB（`431`），报文体上限 1 MiB
（`413`）；非法请求行、坏 `Content-Length`、截断报文 → `400`；
非 `HTTP/1.x` → `505`。所有 4xx/5xx 都不终止服务器，连接级错误仅写 `warn` 日志。

The transport never panics on bad input: parse/size failures become 4xx, and a
failing connection is logged and dropped while the accept loop keeps running.

### Serving / 启动方式

```rust
use std::sync::atomic::AtomicBool;

// 阻塞式 accept 循环；shutdown 由其他线程置真后返回（HTTP 服务不需要 tokio）
// Blocking accept loop; returns once another thread sets `shutdown`.
pub fn serve(listener: TcpListener, server: &McpServer, shutdown: &AtomicBool) -> std::io::Result<()>;
// 无停止标志的便捷入口 / convenience entry point without a shutdown flag
pub fn serve_blocking(listener: TcpListener, server: &McpServer) -> std::io::Result<()>;
// 单请求处理（测试/嵌入用） / single-request handling (tests, embedding)
pub fn handle_http(server: &McpServer, request: &HttpRequest) -> HttpResponse;
pub(crate) fn read_request<R: BufRead>(reader: &mut R) -> Result<HttpRequest, HttpError>;
```

`serve` 把监听器切到非阻塞模式、每 20 ms 轮询一次 `shutdown`，因此可在收到
Ctrl-C 后干净退出；每个连接同步处理，请求严格串行（共用一把 `DeviceManager` 锁），
单连接读写超时 10 s。
`serve` switches the listener to non-blocking mode and polls `shutdown` every
20 ms so Ctrl-C can stop it cleanly; connections are handled synchronously, so
requests are serialized through the `DeviceManager` lock, with a 10 s per
connection I/O timeout.

`HttpRequest` / `HttpResponse` / `HttpError` 为公开类型：请求含
`method` / `path`（已剥离查询串）/ `content_length` / `body`，响应含
`status` / `content_type` / `body` / `allow`；`HttpError` 带 `status` 与
`message`，`into_response()` 直接给出响应。
`HttpRequest` / `HttpResponse` / `HttpError` are public: requests carry
`method` / `path` (query stripped) / `content_length` / `body`; responses carry
`status` / `content_type` / `body` / `allow`; `HttpError` holds `status` and
`message` and renders itself via `into_response()`.

---

## Architecture / 架构

```
AgentFuse IDE → HTTP/1.1 (POST /mcp)
                     └── http::serve → McpServer::handle_request(&str) -> String
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

    /// Async placeholder: flips `running` only / 异步占位：仅翻转标志
    pub async fn start(&mut self) -> Result<(), std::io::Error>;
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
`start()` / `stop()` remain the async-facing flag placeholders; the blocking
transport is `rp2350_mcp::http::serve`.
`new(port)` 保留原签名，内部创建空的私有 `DeviceManager`；已有设备的宿主必须使用
`with_manager`。`start()` / `stop()` 仍是面向异步宿主的标志占位；阻塞式传输由
`rp2350_mcp::http::serve` 提供。

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

Over HTTP / 通过 HTTP：

```rust
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use rp2350_mcp::McpServer;

let listener = TcpListener::bind(("127.0.0.1", 8080))?;
let server = McpServer::with_manager(listener.local_addr()?.port(), Arc::clone(&manager));
let shutdown = AtomicBool::new(false);   // 另一线程置真即停止 / set by another thread
rp2350_mcp::http::serve(listener, &server, &shutdown)?;
```

From a shell / 命令行验证：

```powershell
Invoke-RestMethod -Uri http://127.0.0.1:8080/health
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8080/mcp -ContentType 'application/json' `
  -Body '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'
```

---

## Limitations / 限制

- ⬜ 传输范围：无 TLS、无鉴权、无 SSE/流式、无 keep-alive、无分块编码；每连接一条
  请求后关闭，超时 10 s，请求头 ≤ 8 KiB、报文体 ≤ 1 MiB。
- ⬜ 单线程串行：`serve` 在 accept 线程内同步处理连接，每次调用锁住
  `DeviceManager`，未做并发/背压控制。
- ⬜ 不支持 JSON-RPC batch 数组（会回 `-32600`），也不支持 `Transfer-Encoding`（`501`）。
- ⬜ 无插件热加载；设备由宿主在进程内注册（`DeviceManager::register` / `load_plugin`）。
  CLI `mcp-serve` 的注册表为空，不扫描插件目录、也不挂接运行中的仿真。
- ⬜ MCP 子集：未实现 `resources/*`、`prompts/*`、采样（sampling）与客户端能力协商；
  `initialize` 不回显客户端 `protocolVersion`。
- ⬜ 通知（`notifications/*`）返回 `202` 空体，未更新任何会话状态。
- ⬜ `tools/call` 结果仅放入 `content[0].text`，未提供 `structuredContent`。

EN: The transport is deliberately minimal: no TLS, auth, SSE, keep-alive or
chunked bodies; one request per connection with a 10 s timeout, 8 KiB head and
1 MiB body limits. Connections are handled serially on the accept thread, so
requests share one manager mutex. JSON-RPC batch arrays and
`Transfer-Encoding` are rejected. There is no hot reload, the CLI server starts
with an empty registry, and the MCP surface remains a subset (no
resources/prompts/sampling, no client capability negotiation). Notifications
are acknowledged with `202` and an empty body; tool payloads are delivered as
text content only.

---

## Dependencies / 依赖

| Crate | Why |
|-------|-----|
| `serde` + `serde_json` | JSON-RPC 编解码 / protocol codec |
| `thiserror` | `HttpError` 派生 / `HttpError` derivation |
| `parking_lot` | 共享 `DeviceManager` 锁 / shared manager lock |
| `rp2350-virt-periph` | `DeviceManager` 查询与配置 API |
| `tokio` | 仅供异步宿主预留；阻塞传输不需要 / reserved for async hosts; the blocking transport does not need it |
| `log` | 生命周期日志 / lifecycle logging |
| `rp2350-plugin` (dev) | 测试用 `RpPlugin`/`DevicePlugin` 实现 |

---

## Tests / 测试

`src/tests.rs` 覆盖方法层（14 条），`src/http_tests.rs` 覆盖传输（12 条）：
请求头/报文解析、`413`/`431`/`400`/`505` 上限与错误路径、`initialize` 与
`tools/call` 往返、`/` 别名、`/health`、坏 JSON（`400` + `-32700`）、
未知路径 `404` / 方法 `405`、通知 `202`、缺 `Content-Length` `411`。传输测试使用
`127.0.0.1:0` + 后台线程，`AtomicBool` 关停后 join，全部离线可跑。
`src/tests.rs` covers the method layer (14 tests); `src/http_tests.rs` covers the
transport (12 tests): head/body parsing, `413`/`431`/`400`/`505` limits, the
`initialize` and `tools/call` round trips, the `/` alias, `/health`, malformed
JSON (`400` + `-32700`), unknown path `404` / wrong method `405`, notifications
(`202`) and a missing `Content-Length` (`411`). Transport tests bind
`127.0.0.1:0` on a background thread, stop it via an `AtomicBool` and join.

---

## WASM Note / WASM 注意事项

In WASM builds, the TCP-based HTTP server is replaced with a WebSocket-based JSON-RPC bridge. The same MCP protocol messages are tunneled through the browser's WebSocket connection.

在 WASM 构建中，基于 TCP 的 HTTP 服务器替换为基于 WebSocket 的 JSON-RPC 桥接。相同的 MCP 协议消息通过浏览器的 WebSocket 连接隧道传输。`handle_request` 与传输无关，可直接在 WebSocket 消息回调中复用。
`handle_request` is transport-agnostic and can be reused directly from a
WebSocket message callback.
