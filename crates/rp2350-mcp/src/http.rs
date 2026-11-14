//! Minimal HTTP/1.1 transport for the MCP JSON-RPC method layer
//! 面向 MCP JSON-RPC 方法层的最小 HTTP/1.1 传输
//!
//! One request per connection, `Connection: close`, no TLS, no chunked bodies,
//! no keep-alive and no SSE. [`serve`] blocks in an accept loop that polls a
//! shutdown flag, so a caller can stop it from another thread (CLI Ctrl-C).
//! 每连接一条请求、`Connection: close`、无 TLS、无分块传输、无 keep-alive、
//! 无 SSE。[`serve`] 阻塞在轮询关停标志的 accept 循环中，调用方可从其他线程
//! （CLI 的 Ctrl-C）停止它。
//!
//! Routes / 路由:
//!
//! | Method | Path | Behaviour / 行为 |
//! |--------|------|------------------|
//! | `POST` | `/mcp` (alias `/`) | JSON-RPC body → [`McpServer::handle_request`] |
//! | `GET` | `/health` | `200` with a small JSON object / 返回小型 JSON |
//!
//! 传输层不写错误文档，统一维护在 `doc/crates/rp2350-mcp.md`。
//! Transport error docs live in `doc/crates/rp2350-mcp.md`.
#![allow(
    clippy::doc_markdown,
    clippy::missing_errors_doc,
    // HTTP 类型名按惯例带 `Http` 前缀，重命名反而不清晰。
    // HTTP type names conventionally carry the `Http` prefix.
    clippy::module_name_repetitions,
    // 监听器按值传入以表达 socket 所有权的转移。
    // The listener is passed by value to express transfer of socket ownership.
    clippy::needless_pass_by_value
)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::McpServer;

/// Maximum size of the request head (request line + headers) in bytes
/// 请求头（请求行 + 头部字段）允许的最大字节数
pub const MAX_HEAD_BYTES: usize = 8 * 1024;

/// Maximum size of a request body in bytes / 请求体允许的最大字节数
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Path that accepts JSON-RPC over `POST` / 接受 `POST` JSON-RPC 的路径
pub const MCP_PATH: &str = "/mcp";

/// Path that reports liveness / 存活探测路径
pub const HEALTH_PATH: &str = "/health";

/// Per-connection read/write timeout / 单连接读写超时
const IO_TIMEOUT: Duration = Duration::from_secs(10);

/// Sleep between non-blocking `accept` polls / 非阻塞 `accept` 轮询间隔
const ACCEPT_POLL: Duration = Duration::from_millis(20);

/// `Content-Type` used for JSON payloads / JSON 载荷使用的 `Content-Type`
const JSON_CONTENT_TYPE: &str = "application/json";

/// `Content-Type` used for plain-text error bodies / 纯文本错误响应的 `Content-Type`
const TEXT_CONTENT_TYPE: &str = "text/plain; charset=utf-8";

/// Server name reported by `/health` / `/health` 报告的服务器名称
const SERVER_NAME: &str = "rp2350-mcp";

/// One parsed HTTP request / 一条解析后的 HTTP 请求
#[derive(Debug)]
pub struct HttpRequest {
    /// Request method verb (`POST`, `GET`, …) / 请求方法动词
    pub method: String,
    /// Request path without the query string / 去掉查询串的请求路径
    pub path: String,
    /// Declared `Content-Length`, if any / 声明的 `Content-Length`（若存在）
    pub content_length: Option<usize>,
    /// Raw request body / 原始请求体
    pub body: Vec<u8>,
}

/// One rendered HTTP response / 一条渲染后的 HTTP 响应
#[derive(Debug)]
pub struct HttpResponse {
    /// HTTP status code / HTTP 状态码
    pub status: u16,
    /// `Content-Type` header value / `Content-Type` 头值
    pub content_type: &'static str,
    /// Response body / 响应体
    pub body: String,
    /// Optional `Allow` header (only set for `405`) / 可选 `Allow` 头（仅 405）
    pub allow: Option<&'static str>,
}

impl HttpResponse {
    /// Build a JSON response / 构建 JSON 响应
    #[must_use]
    pub fn json(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: JSON_CONTENT_TYPE,
            body: body.into(),
            allow: None,
        }
    }

    /// Build a plain-text error response / 构建纯文本错误响应
    #[must_use]
    pub fn error(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            content_type: TEXT_CONTENT_TYPE,
            body: message.into(),
            allow: None,
        }
    }

    /// Build a `405` response carrying the expected method / 构建带期望方法的 405 响应
    #[must_use]
    pub fn method_not_allowed(allow: &'static str) -> Self {
        let mut response = Self::error(405, format!("method not allowed, expected {allow}"));
        response.allow = Some(allow);
        response
    }
}

/// HTTP-level failure that maps to a status code / 映射为状态码的 HTTP 层错误
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct HttpError {
    /// Status code to return / 要返回的状态码
    pub status: u16,
    /// Human-readable message / 人类可读消息
    pub message: String,
}

impl HttpError {
    /// Build an error with a status code / 构建带状态码的错误
    #[must_use]
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    /// Render the error as an HTTP response / 将错误渲染为 HTTP 响应
    #[must_use]
    pub fn into_response(self) -> HttpResponse {
        HttpResponse::error(self.status, self.message)
    }
}

/// Serve HTTP requests on `listener` until `shutdown` becomes `true`
/// 在 `listener` 上提供 HTTP 服务，直到 `shutdown` 变真
///
/// The listener is switched to non-blocking mode and polled every 20 ms; each
/// connection is handled synchronously on the calling thread, so requests are
/// strictly serialized (one `DeviceManager` lock).
/// 监听器切换为非阻塞模式并按 20 ms 轮询；每个连接在调用线程上同步处理，
/// 因此请求严格串行（共用一把 `DeviceManager` 锁）。
pub fn serve(
    listener: TcpListener,
    server: &McpServer,
    shutdown: &AtomicBool,
) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    let local = listener.local_addr()?;
    log::info!("MCP HTTP transport listening on http://{local}{MCP_PATH}");

    loop {
        if shutdown.load(Ordering::Relaxed) {
            log::info!("MCP HTTP transport stopping / MCP HTTP 传输正在停止");
            return Ok(());
        }

        match listener.accept() {
            Ok((stream, peer)) => {
                log::debug!("MCP HTTP connection from {peer}");
                // 单个连接的失败只记录日志，不终止服务。
                // A single failed connection is logged, never fatal.
                if let Err(error) = handle_connection(stream, server) {
                    log::warn!("MCP HTTP connection from {peer} failed: {error}");
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(ACCEPT_POLL);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

/// Serve forever (no shutdown flag) on `listener` / 在 `listener` 上永久服务
///
/// Convenience wrapper for hosts that stop the process instead of the loop.
/// 供「以结束进程代替停止循环」的宿主使用。
pub fn serve_blocking(listener: TcpListener, server: &McpServer) -> std::io::Result<()> {
    serve(listener, server, &AtomicBool::new(false))
}

/// Handle one accepted connection and write one response
/// 处理一条已接受的连接并写出一个响应
fn handle_connection(stream: std::net::TcpStream, server: &McpServer) -> std::io::Result<()> {
    // accept 得到的 socket 在部分平台会继承非阻塞标志，这里显式恢复阻塞模式。
    // Accepted sockets can inherit the non-blocking flag on some platforms.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;

    let mut reader = BufReader::new(stream);
    let response = match read_request(&mut reader) {
        Ok(request) => handle_http(server, &request),
        Err(error) => error.into_response(),
    };

    let mut stream = reader.into_inner();
    write_response(&mut stream, &response)
}

/// Read and parse one HTTP request / 读取并解析一条 HTTP 请求
pub(crate) fn read_request<R: BufRead>(reader: &mut R) -> Result<HttpRequest, HttpError> {
    let head = read_head(reader)?;
    let parsed = parse_head(&head)?;
    let body = match parsed.content_length {
        Some(length) => read_body(reader, length)?,
        None => Vec::new(),
    };

    Ok(HttpRequest {
        method: parsed.method,
        path: parsed.path,
        content_length: parsed.content_length,
        body,
    })
}

/// Read the request head (everything up to the blank line) / 读取请求头
fn read_head<R: BufRead>(reader: &mut R) -> Result<String, HttpError> {
    let mut head = String::new();
    let mut line = String::new();

    loop {
        line.clear();
        let remaining = MAX_HEAD_BYTES.saturating_sub(head.len());
        let read = read_line_bounded(reader, &mut line, remaining)?;
        if read == 0 {
            return Err(HttpError::new(400, "unexpected end of request head"));
        }
        // 空行结束头部，不纳入 head 内容。
        // The blank line terminates the head and is not part of it.
        if line == "\r\n" || line == "\n" {
            if head.is_empty() {
                return Err(HttpError::new(400, "empty request"));
            }
            return Ok(head);
        }
        head.push_str(&line);
        if head.len() > MAX_HEAD_BYTES {
            return Err(HttpError::new(
                431,
                format!("request head exceeds {MAX_HEAD_BYTES} bytes"),
            ));
        }
    }
}

/// Read one line without exceeding `remaining` bytes / 在剩余字节数内读取一行
fn read_line_bounded<R: BufRead>(
    reader: &mut R,
    line: &mut String,
    remaining: usize,
) -> Result<usize, HttpError> {
    // 多读 1 字节以检测超限，避免无界分配。
    // Read one byte past the budget so overflow is detected without unbounded allocation.
    let limit = u64::try_from(remaining)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    (&mut *reader)
        .take(limit)
        .read_line(line)
        .map_err(|error| HttpError::new(400, format!("malformed request head: {error}")))
}

/// Parsed pieces of the request head / 请求头的解析结果
struct ParsedHead {
    /// Request method / 请求方法
    method: String,
    /// Request path (query stripped) / 请求路径（已去掉查询串）
    path: String,
    /// Declared `Content-Length` / 声明的 `Content-Length`
    content_length: Option<usize>,
}

/// Parse the request line and the headers we care about / 解析请求行与关心的头部
fn parse_head(head: &str) -> Result<ParsedHead, HttpError> {
    let mut lines = head.lines();
    let request_line = lines
        .next()
        .ok_or_else(|| HttpError::new(400, "missing request line"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| HttpError::new(400, "missing request method"))?;
    let target = parts
        .next()
        .ok_or_else(|| HttpError::new(400, "missing request target"))?;
    let version = parts
        .next()
        .ok_or_else(|| HttpError::new(400, "missing HTTP version"))?;
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return Err(HttpError::new(
            505,
            format!("unsupported HTTP version: {version}"),
        ));
    }

    // 查询串不参与路由；`/mcp?x=1` 与 `/mcp` 等价。
    // Query strings do not take part in routing (`/mcp?x=1` == `/mcp`).
    let path = target.split('?').next().unwrap_or(target).to_string();
    let mut content_length = None;

    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(HttpError::new(400, "malformed header line"));
        };
        let name = name.trim();
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            let parsed = value
                .parse::<usize>()
                .map_err(|_| HttpError::new(400, format!("invalid Content-Length: {value}")))?;
            content_length = Some(parsed);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(HttpError::new(
                501,
                "Transfer-Encoding is not supported; send Content-Length",
            ));
        }
    }

    Ok(ParsedHead {
        method: method.to_string(),
        path,
        content_length,
    })
}

/// Read exactly `length` body bytes / 精确读取 `length` 字节的报文体
fn read_body<R: Read>(reader: &mut R, length: usize) -> Result<Vec<u8>, HttpError> {
    if length > MAX_BODY_BYTES {
        return Err(HttpError::new(
            413,
            format!("request body exceeds {MAX_BODY_BYTES} bytes"),
        ));
    }
    let mut body = vec![0u8; length];
    reader
        .read_exact(&mut body)
        .map_err(|error| HttpError::new(400, format!("truncated request body: {error}")))?;
    Ok(body)
}

/// Route one parsed request to its handler / 将一条解析后的请求路由到处理器
#[must_use]
pub fn handle_http(server: &McpServer, request: &HttpRequest) -> HttpResponse {
    if request.path == HEALTH_PATH {
        return if request.method == "GET" {
            HttpResponse::json(200, health_body())
        } else {
            HttpResponse::method_not_allowed("GET")
        };
    }

    if request.path == MCP_PATH || request.path == "/" {
        return if request.method == "POST" {
            handle_rpc(server, request)
        } else {
            HttpResponse::method_not_allowed("POST")
        };
    }

    HttpResponse::error(404, format!("no route for {}", request.path))
}

/// Handle one JSON-RPC body / 处理一条 JSON-RPC 报文体
fn handle_rpc(server: &McpServer, request: &HttpRequest) -> HttpResponse {
    if request.content_length.is_none() {
        return HttpResponse::error(411, "Content-Length is required for POST");
    }

    let Ok(text) = std::str::from_utf8(&request.body) else {
        return HttpResponse::error(400, "request body is not valid UTF-8");
    };

    let response = server.handle_request(text);
    if response.is_empty() {
        // JSON-RPC 通知无响应体；用 202 表示已接收。
        // JSON-RPC notifications produce no body; 202 signals "accepted".
        return HttpResponse::json(202, String::new());
    }

    // 非法 JSON 用 400 承载 JSON-RPC -32700 载荷（MCP Streamable HTTP 惯例）。
    // Malformed JSON uses 400 while still carrying the JSON-RPC -32700 payload
    // (MCP Streamable HTTP convention).
    if serde_json::from_str::<serde_json::Value>(text).is_err() {
        return HttpResponse::json(400, response);
    }

    HttpResponse::json(200, response)
}

/// JSON body of `GET /health` / `GET /health` 的 JSON 响应体
fn health_body() -> String {
    serde_json::json!({
        "status": "ok",
        "server": SERVER_NAME,
        "version": env!("CARGO_PKG_VERSION"),
        "endpoint": MCP_PATH,
    })
    .to_string()
}

/// Write one response and close the connection / 写出响应并关闭连接
fn write_response(
    stream: &mut std::net::TcpStream,
    response: &HttpResponse,
) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        response.status,
        reason_phrase(response.status),
        response.content_type,
        response.body.len(),
    );
    if let Some(allow) = response.allow {
        head.push_str("Allow: ");
        head.push_str(allow);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");

    stream.write_all(head.as_bytes())?;
    stream.write_all(response.body.as_bytes())?;
    stream.flush()
}

/// Reason phrase for a status code / 状态码对应的原因短语
fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        505 => "HTTP Version Not Supported",
        _ => "Unknown",
    }
}
