# rp2350-mcp — MCP Server (AgentFuse) / MCP 服务器

> **Crate**: `rp2350-mcp` | **Path**: `crates/rp2350-mcp` | **Status**: ✅ 方法层与最小 HTTP/1.1 传输已实现 / method layer + minimal HTTP/1.1 transport implemented — `McpServer::handle_request` 支持 `initialize` / `tools/list` / `tools/call`，并委托 `DeviceManager`；`http::serve` 提供 `POST /mcp` 与 `GET /health`，可通过 [`Auth`] 启用 Bearer Token 鉴权；CLI `mcp-serve` 已接线。
>
> **Status** (EN): The JSON-RPC method layer (`src/server.rs`) implements `initialize`, `tools/list`, `tools/call` (`list_devices` / `query_all` / `query_device` / `configure`) over a shared `DeviceManager`. `src/http.rs` adds a minimal blocking HTTP/1.1 transport (`POST /mcp`, `GET /health`) with optional Bearer-token authentication (`src/auth.rs`), and the CLI `mcp-serve` subcommand drives it.

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
| `POST` | `/mcp`（别名 `/`） | `200` / `400` / `202` / `401` | JSON-RPC 响应；非法 JSON 用 `400` 承载 `-32700` 载荷；通知返回 `202` 空体；启用鉴权且凭据缺失/错误时 `401` |
| `GET` | `/health` | `200` | `{status, server, version, endpoint, auth}`，始终开放 / always open |
| 其他方法 | `/mcp`、`/health` | `405` | 附带 `Allow: POST` / `Allow: GET`（鉴权不改变该行为） |
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
// 带鉴权选项的入口 / entry point with authentication options
pub fn serve_with_options(listener: TcpListener, server: &McpServer,
                          shutdown: &AtomicBool, options: &HttpOptions) -> std::io::Result<()>;
// 无停止标志的便捷入口 / convenience entry points without a shutdown flag
pub fn serve_blocking(listener: TcpListener, server: &McpServer) -> std::io::Result<()>;
pub fn serve_blocking_with_options(listener: TcpListener, server: &McpServer,
                                   options: &HttpOptions) -> std::io::Result<()>;
// 单请求处理（测试/嵌入用） / single-request handling (tests, embedding)
pub fn handle_http(server: &McpServer, request: &HttpRequest) -> HttpResponse;
pub fn handle_http_with_options(server: &McpServer, request: &HttpRequest,
                                options: &HttpOptions) -> HttpResponse;
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
`method` / `path`（已剥离查询串）/ `content_length` / `authorization` / `body`，
响应含 `status` / `content_type` / `body` / `allow` / `www_authenticate`；
`HttpError` 带 `status` 与 `message`，`into_response()` 直接给出响应。
`HttpRequest` / `HttpResponse` / `HttpError` are public: requests carry
`method` / `path` (query stripped) / `content_length` / `authorization` / `body`;
responses carry `status` / `content_type` / `body` / `allow` /
`www_authenticate`; `HttpError` holds `status` and `message` and renders itself
via `into_response()`.

---

## Authentication / 鉴权

Auth is a single static bearer token; there are no users, roles, scopes or
rate limits. Configure it with [`HttpOptions`] / [`Auth`]:
鉴权只有一枚静态 Bearer Token，没有用户、角色、作用域或限流。通过
[`HttpOptions`] / [`Auth`] 配置：

```rust
use rp2350_mcp::{Auth, HttpOptions};

let disabled = HttpOptions::new();                       // auth: false
let enabled  = HttpOptions::with_token("s3cret");        // auth: true
let token = std::env::var("RP2350_MCP_TOKEN").ok();
let from_env = HttpOptions::with_optional_token(token);
// 空字符串 / None 均视为关闭鉴权 / both `None` and `""` disable auth
assert!(!HttpOptions::with_optional_token(Some(String::new())).auth_enabled());
```

| Case / 情形 | Behaviour / 行为 |
|-------------|------------------|
| Token 未配置 | `POST /mcp` 不检查 `Authorization`，行为与旧版一致 / no header required |
| 缺失 `Authorization` | `401` + `WWW-Authenticate: Bearer` + `{"error":"unauthorized"}` |
| 方案名大小写变体 | `Bearer` / `bearer` / `BEARER` 均接受；Token 精确匹配 / scheme is case-insensitive, token is exact |
| Token 错误 / 前缀 / 多余字符 | `401`（不区分「未知 Token」与「错误 Token」） / same `401` for unknown and wrong tokens |
| `GET /health` | 始终 `200`，响应含 `"auth": true/false`，不回显 Token / always open, reports `auth` only |

比较使用 [`auth::constant_time_eq`]：循环次数取两串最大长度，不在首个不同
字节处提前返回，因此响应时间不泄露匹配前缀。
The comparison uses [`auth::constant_time_eq`]: it iterates `max(len(left),
len(right))` times with no early exit, so timing does not reveal the matched
prefix. [`Auth`]'s `Debug` output is redacted, so tokens never reach logs or
startup banners.

`POST /mcp` 的鉴权在方法检查之后、报文校验之前执行，所以 `GET /mcp` 仍是
`405`、未知路径仍是 `404`、未授权请求也不会暴露 `Content-Length` 要求
（`411` 仅在通过鉴权后出现）。
Authentication runs after the method check and before body validation, so
`GET /mcp` still returns `405`, unknown paths still return `404`, and
unauthenticated callers never learn the `Content-Length` requirement (`411`
only appears once authorized).

```powershell
# 未带 Token → 401 / no token → 401
curl.exe -i -X POST http://127.0.0.1:18080/mcp -H "Content-Type: application/json" `
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'
# 带 Token → 200 / with token → 200
curl.exe -i -X POST http://127.0.0.1:18080/mcp -H "Authorization: Bearer s3cret" `
  -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'
```

> **No TLS / 无 TLS**：Token 以明文 HTTP 传输，任何能看到流量的中间人都可复用。
> 生产/远程部署请在前面挂反向代理（nginx/Caddy）终止 TLS 并转发到
> `127.0.0.1`，或只监听本机回环。CLI 当前固定绑定 `0.0.0.0`，启用 Token 时
> 请自行限制网络可达范围。
> The token travels in cleartext HTTP and can be replayed by anyone who can
> observe the connection. For production or remote use terminate TLS in a
> reverse proxy (nginx/Caddy) in front of a loopback-bound server. The CLI binds
> `0.0.0.0`, so restrict network reachability when a token is enabled.

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

### Auth types / 鉴权类型

```rust
pub struct Auth { /* token: Option<String> */ }   // Debug 已脱敏 / redacted Debug
impl Auth {
    pub fn disabled() -> Self;                     // accept everything / 全部放行
    pub fn bearer(token: impl Into<String>) -> Self;
    pub fn from_optional(token: Option<String>) -> Self;  // None/"" → disabled
    pub fn is_enabled(&self) -> bool;
    pub fn authorize(&self, header: Option<&str>) -> bool;
}

pub struct HttpOptions { pub auth: Auth }
impl HttpOptions {
    pub fn new() -> Self;                          // default: auth disabled
    pub fn with_token(token: impl Into<String>) -> Self;
    pub fn with_optional_token(token: Option<String>) -> Self;
    pub fn auth_enabled(&self) -> bool;
}

pub fn auth::constant_time_eq(left: &str, right: &str) -> bool;
pub const auth::AUTH_SCHEME: &str = "Bearer";
pub const auth::WWW_AUTHENTICATE: &str = "Bearer";
pub const auth::TOKEN_ENV: &str = "RP2350_MCP_TOKEN";
```

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

- ⬜ 传输范围：无 TLS、无 SSE/流式、无 keep-alive、无分块编码；每连接一条
  请求后关闭，超时 10 s，请求头 ≤ 8 KiB、报文体 ≤ 1 MiB。鉴权仅支持单枚静态
  Bearer Token（无用户/角色/限流）。
- ⬜ 无 TLS：Token 与 JSON-RPC 明文传输，远程部署需反向代理终止 TLS。
  单进程内 Token 比较为常量时间，但网络层没有任何机密性保证。
- ⬜ 单线程串行：`serve` 在 accept 线程内同步处理连接，每次调用锁住
  `DeviceManager`，未做并发/背压控制。
- ⬜ 不支持 JSON-RPC batch 数组（会回 `-32600`），也不支持 `Transfer-Encoding`（`501`）。
- ⬜ 无插件热加载；设备由宿主在进程内注册（`DeviceManager::register` / `load_plugin`）。
  CLI `mcp-serve` 的注册表为空，不扫描插件目录、也不挂接运行中的仿真。
- ⬜ MCP 子集：未实现 `resources/*`、`prompts/*`、采样（sampling）与客户端能力协商；
  `initialize` 不回显客户端 `protocolVersion`。
- ⬜ 通知（`notifications/*`）返回 `202` 空体，未更新任何会话状态。
- ⬜ `tools/call` 结果仅放入 `content[0].text`，未提供 `structuredContent`。

EN: The transport is deliberately minimal: no TLS, SSE, keep-alive or
chunked bodies; one request per connection with a 10 s timeout, 8 KiB head and
1 MiB body limits. Authentication is a single static bearer token — no users,
roles or rate limits — and tokens cross the wire in cleartext, so remote
deployments need a TLS-terminating reverse proxy. Connections are handled
serially on the accept thread, so requests share one manager mutex. JSON-RPC
batch arrays and `Transfer-Encoding` are rejected. There is no hot reload, the
CLI server starts with an empty registry, and the MCP surface remains a subset
(no resources/prompts/sampling, no client capability negotiation).
Notifications are acknowledged with `202` and an empty body; tool payloads are
delivered as text content only.

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

`src/tests.rs` 覆盖方法层（14 条），`src/http_tests.rs` 覆盖传输（20 条），
`src/auth.rs` 覆盖鉴权（7 条）：请求头/报文解析、`413`/`431`/`400`/`505` 上限与
错误路径、`initialize` 与 `tools/call` 往返、`/` 别名、`/health`（含 `auth`
字段）、坏 JSON（`400` + `-32700`）、未知路径 `404` / 方法 `405`、通知 `202`、
缺 `Content-Length` `411`；鉴权覆盖未配置时的开放行为、缺失/错误/前缀/长度
Token 的 `401`（含 `WWW-Authenticate: Bearer`）、scheme 与头名大小写变体、
`/health` 开放、常量时间比较（相同/不同/长度不同）。传输测试使用
`127.0.0.1:0` + 后台线程，`AtomicBool` 关停后 join，全部离线可跑。
`src/tests.rs` covers the method layer (14 tests), `src/http_tests.rs` the
transport (20 tests) and `src/auth.rs` authentication (7 tests): head/body
parsing, `413`/`431`/`400`/`505` limits, the `initialize` and `tools/call`
round trips, the `/` alias, `/health` (including `auth`), malformed JSON
(`400` + `-32700`), unknown path `404` / wrong method `405`, notifications
(`202`) and a missing `Content-Length` (`411`). Auth tests cover the open
default, `401` for missing/wrong/prefix/length tokens (with
`WWW-Authenticate: Bearer`), scheme and header-name case variants, the open
health endpoint, and constant-time comparison. Transport tests bind
`127.0.0.1:0` on a background thread, stop it via an `AtomicBool` and join.

---

## WASM Note / WASM 注意事项

In WASM builds, the TCP-based HTTP server is replaced with a WebSocket-based JSON-RPC bridge. The same MCP protocol messages are tunneled through the browser's WebSocket connection.

在 WASM 构建中，基于 TCP 的 HTTP 服务器替换为基于 WebSocket 的 JSON-RPC 桥接。相同的 MCP 协议消息通过浏览器的 WebSocket 连接隧道传输。`handle_request` 与传输无关，可直接在 WebSocket 消息回调中复用。
`handle_request` is transport-agnostic and can be reused directly from a
WebSocket message callback.
