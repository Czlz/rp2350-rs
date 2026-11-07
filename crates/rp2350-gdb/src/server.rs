//! GDB RSP Server implementation (stub)
//! GDB RSP 服务器实现（桩）

use std::net::SocketAddr;

/// GDB Remote Serial Protocol server
/// GDB 远程串行协议服务器
pub struct GdbServer {
    /// Listen address / 监听地址
    addr: SocketAddr,
    /// Whether the server is running / 服务器是否在运行
    running: bool,
}

impl GdbServer {
    /// Create a new GDB server / 创建新的 GDB 服务器
    #[must_use]
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            running: false,
        }
    }

    /// Create with default address (0.0.0.0:3333) / 使用默认地址创建
    #[must_use]
    pub fn default_addr() -> Self {
        Self::new(SocketAddr::from(([0, 0, 0, 0], 3333)))
    }

    /// Start the GDB server (async stub) / 启动 GDB 服务器（异步桩）
    pub async fn start(&mut self) -> Result<(), std::io::Error> {
        self.running = true;
        log::info!("GDB server stub started on {}", self.addr);
        Ok(())
    }

    /// Stop the GDB server / 停止 GDB 服务器
    pub fn stop(&mut self) {
        self.running = false;
    }

    /// Check if the server is running / 检查服务器是否在运行
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Get the listen address / 获取监听地址
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}