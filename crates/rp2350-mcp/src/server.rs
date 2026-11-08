//! MCP Server implementation (stub)
//! MCP 服务器实现（桩）

// MCP 桩代码尚未实现 .await 调用，异步签名和文档标记为未来扩展预留。
// MCP stub has no .await calls yet; async sigs and doc markup reserved for future.
#![allow(
    clippy::doc_markdown,
    clippy::missing_errors_doc,
    clippy::unused_async,
    clippy::unused_async_trait_impl
)]

use serde_json::Value;

/// MCP Server for AgentFuse integration
/// 用于 AgentFuse 集成的 MCP 服务器
pub struct McpServer {
    /// Listen port / 监听端口
    port: u16,
    /// Whether the server is running / 服务器是否在运行
    running: bool,
}

impl McpServer {
    /// Create a new MCP server / 创建新的 MCP 服务器
    #[must_use]
    pub fn new(port: u16) -> Self {
        Self {
            port,
            running: false,
        }
    }

    /// Start the MCP server / 启动 MCP 服务器
    pub async fn start(&mut self) -> Result<(), std::io::Error> {
        self.running = true;
        log::info!("MCP server stub started on port {}", self.port);
        Ok(())
    }

    /// Stop the MCP server / 停止 MCP 服务器
    pub fn stop(&mut self) {
        self.running = false;
    }

    /// Query all device states (MCP mcp_query_all)
    /// 查询所有设备状态
    #[must_use]
    pub fn query_all(&self) -> Value {
        // Phase 3: delegate to DeviceManager::mcp_query_all()
        serde_json::json!({"status": "stub", "devices": {}})
    }

    /// Configure a device (MCP mcp_configure)
    /// 配置设备
    pub fn configure(&mut self, _device_id: &str, _config: &Value) -> Result<(), String> {
        // Phase 3: delegate to DeviceManager
        Ok(())
    }
}
