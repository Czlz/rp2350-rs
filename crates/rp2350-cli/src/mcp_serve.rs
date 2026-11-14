//! `mcp-serve` subcommand — MCP JSON-RPC server over the minimal HTTP transport
//! `mcp-serve` 子命令 —— 通过最小 HTTP 传输提供 MCP JSON-RPC 服务
//!
//! The subcommand binds a TCP listener, prints the actual address, installs a
//! Ctrl-C handler that flips a shutdown flag, and then blocks in
//! [`rp2350_mcp::http::serve`]. Endpoints: `POST /mcp` (JSON-RPC) and
//! `GET /health`. The device registry is empty in this process: `mcp-serve`
//! does not scan plugin directories and does not attach a running simulation.
//! 该子命令绑定 TCP 监听器、打印实际地址、安装翻转关停标志的 Ctrl-C 处理器，
//! 然后阻塞在 [`rp2350_mcp::http::serve`] 中。端点：`POST /mcp`（JSON-RPC）与
//! `GET /health`。本进程中的设备注册表为空：`mcp-serve` 不扫描插件目录，也不
//! 挂接运行中的仿真。

use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use anyhow::{Context, Result};
use rp2350_mcp::McpServer;

/// Default listen port for `mcp-serve` / `mcp-serve` 的默认监听端口
pub const DEFAULT_PORT: u16 = 8080;

/// Bind address used by `mcp-serve` (all interfaces) / `mcp-serve` 的绑定地址（所有网卡）
const BIND_ADDRESS: [u8; 4] = [0, 0, 0, 0];

/// Options for the `mcp-serve` subcommand / `mcp-serve` 子命令选项
#[derive(Debug)]
pub struct McpServeOptions {
    /// TCP port to listen on (0 asks the OS for a free port)
    /// 监听端口（0 表示由系统分配空闲端口）
    pub port: u16,
}

/// Bind the MCP HTTP listener / 绑定 MCP HTTP 监听器
///
/// # Errors
/// Returns an error when the address cannot be bound (port in use, no
/// permission). / 地址无法绑定（端口占用、权限不足）时返回错误。
pub fn bind(port: u16) -> Result<TcpListener> {
    let addr = SocketAddr::from((BIND_ADDRESS, port));
    TcpListener::bind(addr).with_context(|| {
        format!("cannot bind MCP listener on {addr} / 无法绑定 MCP 监听器: {addr}")
    })
}

/// Render the startup banner naming both endpoints / 渲染列出两个端点的启动横幅
#[must_use]
pub fn listening_message(local: SocketAddr) -> String {
    let mcp = rp2350_mcp::http::MCP_PATH;
    let health = rp2350_mcp::http::HEALTH_PATH;
    format!(
        "[mcp] listening on http://{local}{mcp} (POST JSON-RPC) / health: http://{local}{health}"
    )
}

/// Spawn the Ctrl-C handler that flips `shutdown` / 启动翻转 `shutdown` 的 Ctrl-C 处理器
fn spawn_ctrl_c_handler(shutdown: Arc<AtomicBool>) -> Result<()> {
    thread::Builder::new()
        .name("mcp-ctrl-c".to_string())
        .spawn(move || {
            // 独立的单线程运行时只用于等待 Ctrl-C；等待返回即代表用户要求停止。
            // A dedicated current-thread runtime only waits for Ctrl-C; a return
            // means the user asked the server to stop.
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    log::warn!("Ctrl-C handler unavailable / Ctrl-C 处理器不可用: {error}");
                    return;
                }
            };
            if let Err(error) = runtime.block_on(tokio::signal::ctrl_c()) {
                log::warn!("Ctrl-C handler failed / Ctrl-C 处理器失败: {error}");
            }
            shutdown.store(true, Ordering::Relaxed);
            log::info!("Ctrl-C received, stopping MCP server / 收到 Ctrl-C，正在停止 MCP 服务器");
        })
        .context("cannot spawn the Ctrl-C handler thread / 无法创建 Ctrl-C 处理线程")?;
    Ok(())
}

/// `mcp-serve` — run the MCP HTTP transport until Ctrl-C / 运行 MCP HTTP 传输直到 Ctrl-C
///
/// # Errors
/// Returns an error when the listener cannot be bound, the listener address
/// cannot be read, or the accept loop fails. / 监听器无法绑定、地址无法读取或
/// accept 循环失败时返回错误。
pub fn run(options: &McpServeOptions) -> Result<()> {
    let listener = bind(options.port)?;
    let local = listener
        .local_addr()
        .context("cannot read the bound MCP address / 无法读取 MCP 绑定地址")?;
    let server = McpServer::new(local.port());

    println!("{}", listening_message(local));
    println!("[mcp] press Ctrl-C to stop / 按 Ctrl-C 停止");

    let shutdown = Arc::new(AtomicBool::new(false));
    spawn_ctrl_c_handler(Arc::clone(&shutdown))?;

    let result = rp2350_mcp::http::serve(listener, &server, &shutdown);
    println!("[mcp] server stopped / MCP 服务器已停止");
    result.context("MCP HTTP transport failed / MCP HTTP 传输失败")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_zero_assigns_a_free_port() {
        let listener = bind(0).expect("bind an ephemeral port");
        let local = listener.local_addr().expect("listener address");
        assert_ne!(local.port(), 0, "port 0 must be replaced by a real port");
    }

    #[test]
    fn listening_message_mentions_both_endpoints() {
        let addr = SocketAddr::from(([127, 0, 0, 1], 18080));
        let message = listening_message(addr);
        assert!(message.contains("http://127.0.0.1:18080/mcp"));
        assert!(message.contains("http://127.0.0.1:18080/health"));
    }
}
