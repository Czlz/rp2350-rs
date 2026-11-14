//! Tests for the MCP JSON-RPC method layer / MCP JSON-RPC 方法层测试

use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_plugin::PluginContext;
use rp2350_plugin::PluginError;
use rp2350_plugin::PluginMetadata;
use rp2350_plugin::RpPlugin;
use rp2350_virt_periph::error::ConfigError;
use rp2350_virt_periph::{DeviceManager, DevicePlugin, DevicePort};
use serde_json::{Value, json};

use crate::McpServer;

/// Minimal configurable device used by the MCP tests
/// MCP 测试使用的最小可配置设备
struct TestDevice {
    /// Plugin metadata / 插件元数据
    metadata: PluginMetadata,
    /// Exposed ports / 暴露的端口
    ports: Vec<DevicePort>,
    /// Current LED level / 当前 LED 电平
    level: bool,
    /// Configurable interval / 可配置的间隔
    interval_ms: u64,
}

impl TestDevice {
    /// Build a device with a fixed metadata set / 构建带固定元数据的设备
    fn new() -> Self {
        Self {
            metadata: PluginMetadata::builder()
                .id("test.led")
                .name("Test LED")
                .version("0.1.0")
                .description("A test LED")
                .author("test")
                .abi_version(1)
                .min_host_version("0.1.0")
                .has_ui(true)
                .tags(vec!["led".into(), "test".into()])
                .build(),
            ports: vec![DevicePort::input("LED", "LED input port")],
            level: false,
            interval_ms: 100,
        }
    }
}

impl RpPlugin for TestDevice {
    fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    fn init(&mut self, _ctx: &mut dyn PluginContext) -> Result<(), PluginError> {
        Ok(())
    }

    fn tick(&mut self, _ctx: &mut dyn PluginContext, _cycles: u64) {}

    fn shutdown(&mut self) {}

    fn state_snapshot(&self) -> Value {
        json!({ "level": self.level, "interval_ms": self.interval_ms })
    }
}

impl DevicePlugin for TestDevice {
    fn ports(&self) -> &[DevicePort] {
        &self.ports
    }

    fn ports_mut(&mut self) -> &mut [DevicePort] {
        &mut self.ports
    }

    fn on_gpio_change(&mut self, _port_index: usize, level: bool) {
        self.level = level;
    }

    fn has_ui(&self) -> bool {
        true
    }

    fn apply_config(&mut self, config: &Value) -> Result<(), ConfigError> {
        let interval = config
            .get("interval_ms")
            .and_then(Value::as_u64)
            .ok_or_else(|| ConfigError::MissingField("interval_ms".to_string()))?;
        if !(1..=1000).contains(&interval) {
            return Err(ConfigError::OutOfRange {
                field: "interval_ms".to_string(),
                message: "expected 1..=1000".to_string(),
            });
        }
        self.interval_ms = interval;
        Ok(())
    }
}

/// Server with one registered `led-1` device / 注册了 `led-1` 设备的服务器
fn server_with_device() -> McpServer {
    let manager = Arc::new(Mutex::new(DeviceManager::new()));
    manager
        .lock()
        .register("led-1".into(), Box::new(TestDevice::new()))
        .unwrap();
    McpServer::with_manager(8765, manager)
}

/// Send a raw request string and parse the response / 发送原始请求并解析响应
fn request(server: &McpServer, method: &str, id: u64, params: Option<Value>) -> Value {
    let mut payload = json!({ "jsonrpc": "2.0", "id": id, "method": method });
    if let Some(params) = params {
        payload["params"] = params;
    }
    let response = server.handle_request(&payload.to_string());
    serde_json::from_str(&response).unwrap()
}

/// Invoke one tool and return the full JSON-RPC response
/// 调用单个工具并返回完整 JSON-RPC 响应
fn call_tool(server: &McpServer, id: u64, name: &str, arguments: &Value) -> Value {
    request(
        server,
        "tools/call",
        id,
        Some(json!({ "name": name, "arguments": arguments })),
    )
}

/// Decode the text payload of an MCP tool result / 解码 MCP 工具结果的文本载荷
fn tool_payload(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).unwrap()
}

#[test]
fn initialize_handshake() {
    let server = server_with_device();
    let response = request(&server, "initialize", 1, None);
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(response["result"]["serverInfo"]["name"], "rp2350-mcp");
    assert!(response["result"]["capabilities"]["tools"].is_object());
}

#[test]
fn tools_list_exposes_four_tools() {
    let server = server_with_device();
    let response = request(&server, "tools/list", 2, None);
    let tools = response["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["list_devices", "query_all", "query_device", "configure"]
    );
}

#[test]
fn list_devices_reports_metadata() {
    let server = server_with_device();
    let response = call_tool(&server, 3, "list_devices", &json!({}));
    let devices = tool_payload(&response);
    assert_eq!(devices[0]["id"], "led-1");
    assert_eq!(devices[0]["name"], "Test LED");
    assert_eq!(devices[0]["has_ui"], true);
    assert_eq!(devices[0]["tags"], json!(["led", "test"]));
}

#[test]
fn query_all_delegates_to_manager() {
    let server = server_with_device();
    let response = call_tool(&server, 4, "query_all", &json!({}));
    let states = tool_payload(&response);
    assert_eq!(states["led-1"]["level"], false);
    assert_eq!(states["led-1"]["interval_ms"], 100);
}

#[test]
fn query_device_hit_returns_state() {
    let server = server_with_device();
    let response = call_tool(&server, 5, "query_device", &json!({ "id": "led-1" }));
    let device = tool_payload(&response);
    assert_eq!(device["id"], "led-1");
    assert_eq!(device["state"]["interval_ms"], 100);
}

#[test]
fn query_device_miss_returns_error() {
    let server = server_with_device();
    let response = call_tool(&server, 6, "query_device", &json!({ "id": "missing" }));
    assert_eq!(response["error"]["code"], -32001);
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing")
    );
    assert!(response.get("result").is_none());
}

#[test]
fn configure_success_updates_device() {
    let server = server_with_device();
    let response = call_tool(
        &server,
        7,
        "configure",
        &json!({ "id": "led-1", "config": { "interval_ms": 250 } }),
    );
    assert_eq!(
        tool_payload(&response),
        json!({ "ok": true, "id": "led-1" })
    );

    let state = server.query_all();
    assert_eq!(state["led-1"]["interval_ms"], 250);
}

#[test]
fn configure_rejects_invalid_value() {
    let server = server_with_device();
    let response = call_tool(
        &server,
        8,
        "configure",
        &json!({ "id": "led-1", "config": { "interval_ms": 5000 } }),
    );
    assert_eq!(response["error"]["code"], -32002);
    // 插件仍保留旧值 / the plugin keeps its previous value
    assert_eq!(server.query_all()["led-1"]["interval_ms"], 100);
}

#[test]
fn configure_unknown_device_returns_error() {
    let server = server_with_device();
    let response = call_tool(
        &server,
        9,
        "configure",
        &json!({ "id": "missing", "config": { "interval_ms": 10 } }),
    );
    assert_eq!(response["error"]["code"], -32002);
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing")
    );
}

#[test]
fn unknown_method_returns_error() {
    let server = server_with_device();
    let response = request(&server, "bogus/method", 10, None);
    assert_eq!(response["error"]["code"], -32601);
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("bogus/method")
    );
}

#[test]
fn invalid_json_returns_parse_error() {
    let server = server_with_device();
    let response = server.handle_request("{not json");
    let parsed: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(parsed["error"]["code"], -32700);
    assert_eq!(parsed["id"], Value::Null);
}

#[test]
fn missing_tool_argument_returns_invalid_params() {
    let server = server_with_device();
    let response = call_tool(&server, 11, "query_device", &json!({}));
    assert_eq!(response["error"]["code"], -32602);
}

#[test]
fn unknown_tool_returns_invalid_params() {
    let server = server_with_device();
    let response = call_tool(&server, 12, "bogus_tool", &json!({}));
    assert_eq!(response["error"]["code"], -32602);
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("bogus_tool")
    );
}

#[test]
fn notification_has_no_response() {
    let server = server_with_device();
    let response =
        server.handle_request(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    assert_eq!(response, "");
}
