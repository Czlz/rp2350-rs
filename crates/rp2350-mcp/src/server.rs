//! MCP server method layer (JSON-RPC 2.0, no transport)
//! MCP 服务器方法层（JSON-RPC 2.0，不含传输）
//!
//! Implements `initialize` / `tools/list` / `tools/call` on top of a shared
//! [`DeviceManager`]. The minimal HTTP/1.1 transport lives in [`crate::http`];
//! CLI wiring lives in `rp2350-cli`.
//! 基于共享的 [`DeviceManager`] 实现 `initialize` / `tools/list` / `tools/call`。
//! 最小 HTTP/1.1 传输位于 [`crate::http`]；CLI 接线位于 `rp2350-cli`。

// 方法层自身不涉及 I/O；错误文档由 doc/crates 统一维护。
// The method layer performs no I/O; error docs live in doc/crates.
#![allow(
    clippy::doc_markdown,
    clippy::missing_errors_doc,
    clippy::unused_async,
    clippy::unused_async_trait_impl
)]

use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_virt_periph::DeviceManager;
use serde_json::{Map, Value, json};

/// JSON-RPC: invalid JSON payload / JSON-RPC：无效 JSON 载荷
const PARSE_ERROR: i64 = -32700;
/// JSON-RPC: structurally invalid request / JSON-RPC：请求结构无效
const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC: unknown method / JSON-RPC：未知方法
const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC: invalid parameters / JSON-RPC：参数无效
const INVALID_PARAMS: i64 = -32602;
/// MCP: requested device is not registered / MCP：请求的设备未注册
const DEVICE_NOT_FOUND: i64 = -32001;
/// MCP: device rejected the configuration / MCP：设备拒绝了配置
const CONFIG_ERROR: i64 = -32002;

/// MCP protocol revision advertised during `initialize`
/// `initialize` 握手时公布的 MCP 协议版本
const PROTOCOL_VERSION: &str = "2024-11-05";
/// Server name reported in `serverInfo` / `serverInfo` 中的服务器名称
const SERVER_NAME: &str = "rp2350-mcp";
/// Server version reported in `serverInfo` / `serverInfo` 中的服务器版本
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// MCP Server for AgentFuse integration
/// 用于 AgentFuse 集成的 MCP 服务器
///
/// The server shares the device registry with the host through
/// `Arc<Mutex<DeviceManager>>`; the host keeps registering/wiring devices while
/// the MCP layer only reads state and applies configuration.
/// 服务器通过 `Arc<Mutex<DeviceManager>>` 与宿主共享设备注册表；
/// 宿主继续注册/接线设备，MCP 层只读取状态并应用配置。
pub struct McpServer {
    /// Listen port / 监听端口
    port: u16,
    /// Whether the server is running / 服务器是否在运行
    running: bool,
    /// Shared device registry / 共享设备注册表
    manager: Arc<Mutex<DeviceManager>>,
}

impl McpServer {
    /// Create a new MCP server with its own empty [`DeviceManager`]
    /// 创建带独立空 [`DeviceManager`] 的 MCP 服务器
    ///
    /// Use [`McpServer::with_manager`] to share the host's registry.
    /// 需要共享宿主注册表时使用 [`McpServer::with_manager`]。
    #[must_use]
    pub fn new(port: u16) -> Self {
        Self::with_manager(port, Arc::new(Mutex::new(DeviceManager::new())))
    }

    /// Create a server backed by an existing device manager
    /// 创建由已有设备管理器支撑的服务器
    #[must_use]
    pub fn with_manager(port: u16, manager: Arc<Mutex<DeviceManager>>) -> Self {
        Self {
            port,
            running: false,
            manager,
        }
    }

    /// Shared handle to the device registry / 设备注册表的共享句柄
    #[must_use]
    pub fn manager(&self) -> Arc<Mutex<DeviceManager>> {
        Arc::clone(&self.manager)
    }

    /// Configured listen port / 配置的监听端口
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Whether [`McpServer::start`] has been called / 是否已调用 [`McpServer::start`]
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Start the MCP server / 启动 MCP 服务器
    ///
    /// This only flips the `running` flag and logs, for async hosts that own
    /// their own socket. The blocking HTTP transport is [`crate::http::serve`].
    /// 该方法仅翻转 `running` 标志并写日志，供自行持有 socket 的异步宿主使用；
    /// 阻塞式 HTTP 传输见 [`crate::http::serve`]。
    pub async fn start(&mut self) -> Result<(), std::io::Error> {
        self.running = true;
        log::info!("MCP server started on port {}", self.port);
        Ok(())
    }

    /// Stop the MCP server / 停止 MCP 服务器
    pub fn stop(&mut self) {
        self.running = false;
    }

    /// Query all device states (delegates to `DeviceManager::mcp_query_all`)
    /// 查询所有设备状态（委托 `DeviceManager::mcp_query_all`）
    #[must_use]
    pub fn query_all(&self) -> Value {
        self.manager.lock().mcp_query_all()
    }

    /// Configure a device (delegates to `DeviceManager::apply_config`)
    /// 配置设备（委托 `DeviceManager::apply_config`）
    ///
    /// Returns the rendered error as a string for backwards compatibility;
    /// JSON-RPC callers receive a structured error object instead.
    /// 为保持兼容返回字符串错误；JSON-RPC 调用方会收到结构化错误对象。
    pub fn configure(&mut self, device_id: &str, config: &Value) -> Result<(), String> {
        self.manager
            .lock()
            .apply_config(device_id, config)
            .map_err(|err| err.to_string())
    }

    /// Handle one JSON-RPC request and return the response body
    /// 处理单个 JSON-RPC 请求并返回响应体
    ///
    /// Supports `initialize`, `tools/list`, `tools/call` and MCP
    /// `notifications/*` (which produce an empty response). All other methods
    /// return a JSON-RPC error object.
    /// 支持 `initialize`、`tools/list`、`tools/call` 与 MCP `notifications/*`
    /// （返回空响应）。其余方法返回 JSON-RPC 错误对象。
    #[must_use]
    pub fn handle_request(&self, request: &str) -> String {
        let request = match serde_json::from_str::<Value>(request) {
            Ok(request) => request,
            Err(err) => {
                return error_response(
                    &Value::Null,
                    &RpcError::new(PARSE_ERROR, format!("Parse error: {err}")),
                );
            }
        };

        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let version_ok = request
            .get("jsonrpc")
            .is_none_or(|value| value.as_str() == Some("2.0"));
        if !version_ok {
            let err = RpcError::new(INVALID_REQUEST, "Invalid request: jsonrpc must be \"2.0\"");
            return error_response(&id, &err);
        }

        let Some(method) = request.get("method").and_then(Value::as_str) else {
            let err = RpcError::new(INVALID_REQUEST, "Invalid request: missing method");
            return error_response(&id, &err);
        };

        // MCP 通知没有 id，也不需要响应。
        // MCP notifications carry no id and must not receive a response.
        if method.starts_with("notifications/") {
            return String::new();
        }

        match self.dispatch(method, request.get("params")) {
            Ok(result) => success_response(&id, &result),
            Err(err) => error_response(&id, &err),
        }
    }

    /// Dispatch one method to its handler / 将方法分派到对应处理器
    fn dispatch(&self, method: &str, params: Option<&Value>) -> Result<Value, RpcError> {
        match method {
            "initialize" => Ok(initialize_result()),
            "tools/list" => Ok(tools_list_result()),
            "tools/call" => self.call_tool(params),
            other => Err(RpcError::new(
                METHOD_NOT_FOUND,
                format!("Method not found: {other}"),
            )),
        }
    }

    /// Execute one MCP tool / 执行单个 MCP 工具
    fn call_tool(&self, params: Option<&Value>) -> Result<Value, RpcError> {
        let params =
            params.ok_or_else(|| RpcError::new(INVALID_PARAMS, "tools/call requires params"))?;
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::new(INVALID_PARAMS, "tools/call requires a string 'name'"))?;
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));

        match name {
            "list_devices" => {
                let devices = self.manager.lock().mcp_list_devices();
                Ok(tool_result(&Value::Array(devices)))
            }
            "query_all" => {
                let states = self.manager.lock().mcp_query_all();
                Ok(tool_result(&states))
            }
            "query_device" => {
                let id = required_str(&arguments, "id")?;
                match self.manager.lock().mcp_query_device(id) {
                    Some(state) => Ok(tool_result(&json!({ "id": id, "state": state }))),
                    None => Err(RpcError::new(
                        DEVICE_NOT_FOUND,
                        format!("Device not found: {id}"),
                    )),
                }
            }
            "configure" => {
                let id = required_str(&arguments, "id")?;
                let config = arguments
                    .get("config")
                    .ok_or_else(|| RpcError::new(INVALID_PARAMS, "configure requires 'config'"))?;
                self.manager
                    .lock()
                    .apply_config(id, config)
                    .map_err(|err| {
                        RpcError::new(CONFIG_ERROR, format!("Failed to configure '{id}': {err}"))
                    })?;
                Ok(tool_result(&json!({ "ok": true, "id": id })))
            }
            other => Err(RpcError::new(
                INVALID_PARAMS,
                format!("Unknown tool: {other}"),
            )),
        }
    }
}

/// A JSON-RPC error payload / JSON-RPC 错误载荷
struct RpcError {
    /// JSON-RPC error code / JSON-RPC 错误码
    code: i64,
    /// Human-readable message / 人类可读消息
    message: String,
}

impl RpcError {
    /// Build an error payload / 构建错误载荷
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// `initialize` result: minimal MCP handshake / `initialize` 结果：最小 MCP 握手
fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
    })
}

/// `tools/list` result: the four exposed tools / `tools/list` 结果：四个暴露的工具
fn tools_list_result() -> Value {
    json!({
        "tools": [
            {
                "name": "list_devices",
                "description": "List registered virtual devices (id, name, has_ui, tags).",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }
            },
            {
                "name": "query_all",
                "description": "Query state snapshots of all registered devices.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }
            },
            {
                "name": "query_device",
                "description": "Query one device state snapshot by id.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "id": { "type": "string" } },
                    "required": ["id"]
                }
            },
            {
                "name": "configure",
                "description": "Apply a JSON configuration to one device.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "config": { "type": "object" }
                    },
                    "required": ["id", "config"]
                }
            }
        ]
    })
}

/// Extract a required string argument / 提取必填字符串参数
fn required_str<'a>(arguments: &'a Value, field: &str) -> Result<&'a str, RpcError> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::new(INVALID_PARAMS, format!("Missing string argument '{field}'")))
}

/// Wrap a payload in an MCP tool result (`content` + `isError`)
/// 将载荷包装为 MCP 工具结果（`content` + `isError`）
fn tool_result(payload: &Value) -> Value {
    json!({
        "content": [{ "type": "text", "text": payload.to_string() }],
        "isError": false,
    })
}

/// Render a successful JSON-RPC response / 渲染成功的 JSON-RPC 响应
fn success_response(id: &Value, result: &Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

/// Render a JSON-RPC error response / 渲染 JSON-RPC 错误响应
fn error_response(id: &Value, error: &RpcError) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": error.code, "message": error.message },
    })
    .to_string()
}
