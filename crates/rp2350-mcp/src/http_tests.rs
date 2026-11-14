//! Tests for the minimal HTTP/1.1 transport / 最小 HTTP/1.1 传输测试

use std::io::{BufReader, Cursor, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use serde_json::Value;

use crate::HttpOptions;
use crate::McpServer;
use crate::http::{self, HttpRequest, MAX_BODY_BYTES, MAX_HEAD_BYTES};

/// Bind a loopback listener on an ephemeral port / 在临时端口上绑定回环监听器
fn listener() -> TcpListener {
    TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).expect("bind loopback")
}

/// Spawn the transport on a background thread with an empty registry
/// 在后台线程启动传输（使用空设备注册表）
fn spawn_server() -> (
    SocketAddr,
    Arc<AtomicBool>,
    thread::JoinHandle<std::io::Result<()>>,
) {
    spawn_server_with_token(None)
}

/// Spawn the transport with an optional bearer token / 以可选 Bearer Token 启动传输
fn spawn_server_with_token(
    token: Option<&str>,
) -> (
    SocketAddr,
    Arc<AtomicBool>,
    thread::JoinHandle<std::io::Result<()>>,
) {
    let listener = listener();
    let addr = listener.local_addr().expect("listener address");
    let shutdown = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&shutdown);
    let options = HttpOptions::with_optional_token(token.map(str::to_string));
    let handle = thread::spawn(move || {
        let server = McpServer::new(addr.port());
        http::serve_with_options(listener, &server, &flag, &options)
    });
    (addr, shutdown, handle)
}

/// Stop the background server and assert a clean exit / 停止后台服务器并断言正常退出
fn stop(shutdown: &AtomicBool, handle: thread::JoinHandle<std::io::Result<()>>) {
    shutdown.store(true, Ordering::Relaxed);
    handle
        .join()
        .expect("server thread panicked")
        .expect("serve returned an error");
}

/// One raw HTTP response decoded by the test client / 测试客户端解码后的原始响应
struct RawResponse {
    /// Status code from the status line / 状态行中的状态码
    status: u16,
    /// `Content-Type` header / `Content-Type` 头
    content_type: String,
    /// `Content-Length` header / `Content-Length` 头
    content_length: Option<usize>,
    /// `WWW-Authenticate` header / `WWW-Authenticate` 头
    www_authenticate: Option<String>,
    /// Message body / 报文体
    body: String,
}

/// Send one raw request and decode the full response / 发送原始请求并解码完整响应
fn request(addr: SocketAddr, raw: &str) -> RawResponse {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .expect("write timeout");
    stream.write_all(raw.as_bytes()).expect("write request");
    let mut buffer = String::new();
    stream
        .read_to_string(&mut buffer)
        .expect("read full response");
    parse_response(&buffer)
}

/// Decode a response written by [`http::serve`] / 解码 [`http::serve`] 写出的响应
fn parse_response(raw: &str) -> RawResponse {
    let (head, body) = raw.split_once("\r\n\r\n").expect("header/body separator");
    let mut lines = head.lines();
    let status_line = lines.next().expect("status line");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .expect("status code")
        .parse::<u16>()
        .expect("numeric status");

    let mut content_type = String::new();
    let mut content_length = None;
    let mut www_authenticate = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-type") {
            content_type = value.trim().to_string();
        } else if name.eq_ignore_ascii_case("content-length") {
            content_length = Some(value.trim().parse::<usize>().expect("content length"));
        } else if name.eq_ignore_ascii_case("www-authenticate") {
            www_authenticate = Some(value.trim().to_string());
        }
    }

    RawResponse {
        status,
        content_type,
        content_length,
        www_authenticate,
        body: body.to_string(),
    }
}

/// Build a `POST /mcp` request with a JSON body / 构造带 JSON 报文的 `POST /mcp`
fn post_mcp(body: &str) -> String {
    format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

/// Build a `POST /mcp` request carrying an `Authorization` header
/// 构造携带 `Authorization` 头的 `POST /mcp`
fn post_mcp_with_auth(body: &str, authorization: &str) -> String {
    format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: {authorization}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

/// JSON-RPC `initialize` request body used by the auth tests
/// 鉴权测试使用的 JSON-RPC `initialize` 报文体
const INITIALIZE_BODY: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;

#[test]
fn parses_request_line_headers_and_body() {
    let raw = "POST /mcp?trace=1 HTTP/1.1\r\nHost: localhost\r\n\
               Authorization: Bearer abc123\r\n\
               Content-Length: 13\r\n\r\n{\"jsonrpc\":2}";
    let mut reader = BufReader::new(Cursor::new(raw.as_bytes()));
    let request: HttpRequest = http::read_request(&mut reader).expect("valid request");

    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/mcp");
    assert_eq!(request.content_length, Some(13));
    assert_eq!(request.authorization.as_deref(), Some("Bearer abc123"));
    assert_eq!(request.body, br#"{"jsonrpc":2}"#);
}

#[test]
fn rejects_oversized_body_before_reading_it() {
    let raw = format!(
        "POST /mcp HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
        MAX_BODY_BYTES + 1
    );
    let mut reader = BufReader::new(Cursor::new(raw.as_bytes()));
    let error = http::read_request(&mut reader).expect_err("body must be rejected");
    assert_eq!(error.status, 413);
}

#[test]
fn rejects_oversized_head() {
    let raw = format!(
        "GET /health HTTP/1.1\r\nX-Pad: {}\r\n\r\n",
        "a".repeat(MAX_HEAD_BYTES)
    );
    let mut reader = BufReader::new(Cursor::new(raw.as_bytes()));
    let error = http::read_request(&mut reader).expect_err("head must be rejected");
    assert_eq!(error.status, 431);
}

#[test]
fn rejects_missing_request_line_and_bad_versions() {
    let mut empty = BufReader::new(Cursor::new(Vec::new()));
    assert_eq!(
        http::read_request(&mut empty)
            .expect_err("empty request")
            .status,
        400
    );

    let raw = "GET /health HTTP/2.0\r\n\r\n";
    let mut reader = BufReader::new(Cursor::new(raw.as_bytes()));
    assert_eq!(
        http::read_request(&mut reader)
            .expect_err("bad version")
            .status,
        505
    );
}

#[test]
fn post_initialize_round_trip() {
    let (addr, shutdown, handle) = spawn_server();
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let response = request(addr, &post_mcp(body));

    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "application/json");
    assert_eq!(
        response.content_length,
        Some(response.body.len()),
        "Content-Length must match the body"
    );

    let payload: Value = serde_json::from_str(&response.body).expect("JSON response");
    assert_eq!(payload["jsonrpc"], "2.0");
    assert_eq!(payload["id"], 1);
    assert_eq!(payload["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(payload["result"]["serverInfo"]["name"], "rp2350-mcp");

    stop(&shutdown, handle);
}

#[test]
fn post_tools_call_round_trip() {
    let (addr, shutdown, handle) = spawn_server();
    let body = r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"list_devices","arguments":{}}}"#;
    let response = request(addr, &post_mcp(body));

    assert_eq!(response.status, 200);
    let payload: Value = serde_json::from_str(&response.body).expect("JSON response");
    assert_eq!(payload["id"], 7);
    assert_eq!(payload["result"]["isError"], false);
    assert_eq!(
        payload["result"]["content"][0]["text"], "[]",
        "an empty registry lists no devices"
    );
    assert_eq!(payload["result"]["content"][0]["type"], "text");

    stop(&shutdown, handle);
}

#[test]
fn post_root_is_an_alias_for_mcp() {
    let (addr, shutdown, handle) = spawn_server();
    let body = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
    let raw = format!(
        "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let response = request(addr, &raw);

    assert_eq!(response.status, 200);
    let payload: Value = serde_json::from_str(&response.body).expect("JSON response");
    assert_eq!(
        payload["result"]["tools"].as_array().expect("tools").len(),
        4
    );

    stop(&shutdown, handle);
}

#[test]
fn health_endpoint_returns_json() {
    let (addr, shutdown, handle) = spawn_server();
    let response = request(addr, "GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n");

    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "application/json");
    let payload: Value = serde_json::from_str(&response.body).expect("JSON body");
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["server"], "rp2350-mcp");
    assert_eq!(payload["endpoint"], "/mcp");
    assert_eq!(payload["auth"], false, "auth is disabled without a token");

    stop(&shutdown, handle);
}

#[test]
fn auth_disabled_by_default_accepts_unauthenticated_requests() {
    let (addr, shutdown, handle) = spawn_server();

    let post = request(addr, &post_mcp(INITIALIZE_BODY));
    assert_eq!(post.status, 200);
    assert_eq!(post.www_authenticate, None);

    let health = request(addr, "GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert_eq!(health.status, 200);

    stop(&shutdown, handle);
}

#[test]
fn missing_authorization_returns_401_with_challenge() {
    let (addr, shutdown, handle) = spawn_server_with_token(Some("secret"));

    let response = request(addr, &post_mcp(INITIALIZE_BODY));
    assert_eq!(response.status, 401);
    assert_eq!(response.content_type, "application/json");
    assert_eq!(response.www_authenticate.as_deref(), Some("Bearer"));
    let payload: Value = serde_json::from_str(&response.body).expect("JSON 401 body");
    assert_eq!(payload["error"], "unauthorized");

    stop(&shutdown, handle);
}

#[test]
fn wrong_token_returns_401() {
    let (addr, shutdown, handle) = spawn_server_with_token(Some("secret"));

    let response = request(addr, &post_mcp_with_auth(INITIALIZE_BODY, "Bearer wrong"));
    assert_eq!(response.status, 401);
    assert_eq!(response.www_authenticate.as_deref(), Some("Bearer"));

    stop(&shutdown, handle);
}

#[test]
fn token_prefix_and_extra_data_are_rejected() {
    let (addr, shutdown, handle) = spawn_server_with_token(Some("secret"));

    for header in ["Bearer secre", "Bearer secret extra", "Bearersecret"] {
        let response = request(addr, &post_mcp_with_auth(INITIALIZE_BODY, header));
        assert_eq!(response.status, 401, "header {header:?} must be rejected");
    }

    stop(&shutdown, handle);
}

#[test]
fn correct_token_returns_200() {
    let (addr, shutdown, handle) = spawn_server_with_token(Some("secret"));

    let response = request(addr, &post_mcp_with_auth(INITIALIZE_BODY, "Bearer secret"));
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "application/json");
    let payload: Value = serde_json::from_str(&response.body).expect("JSON response");
    assert_eq!(payload["result"]["serverInfo"]["name"], "rp2350-mcp");

    stop(&shutdown, handle);
}

#[test]
fn bearer_scheme_and_header_name_are_case_insensitive() {
    let (addr, shutdown, handle) = spawn_server_with_token(Some("secret"));

    for header in ["bearer secret", "BEARER secret", "BeArEr secret"] {
        let response = request(addr, &post_mcp_with_auth(INITIALIZE_BODY, header));
        assert_eq!(response.status, 200, "header {header:?} must be accepted");
    }

    // 头名本身同样大小写不敏感 / header names are case-insensitive too.
    let raw = format!(
        "POST /mcp HTTP/1.1\r\nhost: localhost\r\nauthorization: bEaReR secret\r\n\
         Content-Length: {}\r\n\r\n{INITIALIZE_BODY}",
        INITIALIZE_BODY.len()
    );
    assert_eq!(request(addr, &raw).status, 200);

    stop(&shutdown, handle);
}

#[test]
fn health_stays_open_and_reports_auth_enabled() {
    let (addr, shutdown, handle) = spawn_server_with_token(Some("secret"));

    let response = request(addr, "GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert_eq!(response.status, 200);
    assert_eq!(response.www_authenticate, None);
    let payload: Value = serde_json::from_str(&response.body).expect("JSON body");
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["auth"], true);

    stop(&shutdown, handle);
}

#[test]
fn auth_keeps_405_and_404_routes_intact() {
    let (addr, shutdown, handle) = spawn_server_with_token(Some("secret"));

    let options = request(addr, "OPTIONS /mcp HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert_eq!(options.status, 405);
    assert!(options.body.contains("POST"));

    let wrong_method = request(addr, "GET /mcp HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert_eq!(wrong_method.status, 405);

    let unknown = request(addr, "GET /nope HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert_eq!(unknown.status, 404);

    stop(&shutdown, handle);
}

#[test]
fn bad_json_returns_400_with_parse_error_payload() {
    let (addr, shutdown, handle) = spawn_server();
    let response = request(addr, &post_mcp("{not json"));

    assert_eq!(response.status, 400);
    let payload: Value = serde_json::from_str(&response.body).expect("JSON error body");
    assert_eq!(payload["error"]["code"], -32700);
    assert_eq!(payload["id"], Value::Null);

    stop(&shutdown, handle);
}

#[test]
fn unknown_path_returns_404_and_wrong_method_405() {
    let (addr, shutdown, handle) = spawn_server();

    let missing = request(addr, "GET /nope HTTP/1.1\r\n\r\n");
    assert_eq!(missing.status, 404);
    assert!(missing.body.contains("/nope"));

    let wrong_method = request(addr, "GET /mcp HTTP/1.1\r\n\r\n");
    assert_eq!(wrong_method.status, 405);
    assert!(wrong_method.body.contains("POST"));

    stop(&shutdown, handle);
}

#[test]
fn notification_is_accepted_without_body() {
    let (addr, shutdown, handle) = spawn_server();
    let body = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    let response = request(addr, &post_mcp(body));

    assert_eq!(response.status, 202);
    assert_eq!(response.body, "");
    assert_eq!(response.content_length, Some(0));

    stop(&shutdown, handle);
}

#[test]
fn missing_content_length_on_post_returns_411() {
    let (addr, shutdown, handle) = spawn_server();
    let response = request(addr, "POST /mcp HTTP/1.1\r\nHost: localhost\r\n\r\n");

    assert_eq!(response.status, 411);

    stop(&shutdown, handle);
}
