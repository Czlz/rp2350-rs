//! DeviceManager — central orchestration of virtual device plugins
//! DeviceManager — 虚拟设备插件的中央编排器
//!
//! Owns the plugin lifecycle, wiring table, GPIO routing, tick scheduling,
//! and MCP query API. All devices are loaded as independent plugins.
//! 拥有插件生命周期、接线表、GPIO 路由、tick 调度和 MCP 查询 API。
//! 所有设备作为独立插件加载。

// DeviceManager 等类型名称来自项目约定，文档标记反映了这些内部类型名称。
// DeviceManager and similar type names come from project conventions;
// doc markup reflects these internal type names.
#![allow(
    clippy::doc_markdown,
    clippy::missing_errors_doc,
    clippy::module_name_repetitions
)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rp2350_core::gpio::GpioPin;
use rp2350_plugin::PluginContext;

use super::device::DevicePlugin;
use super::error::{ConfigError, DeviceError};
use super::loader::{LoadedPlugin, PluginLoader};
use super::wiring::WiringTable;

/// Identifier type for registered devices / 注册设备的标识类型
pub type DeviceId = String;

/// A managed device instance / 受管理的设备实例
#[allow(dead_code)]
struct ManagedDevice {
    /// The device plugin / 设备插件
    plugin: Box<dyn DevicePlugin>,
    /// Source path if loaded from disk / 从磁盘加载时的源路径
    source_path: Option<PathBuf>,
    /// Library backing a dynamically loaded plugin / 动态加载插件背后的库
    ///
    /// Declared after `plugin` so the plugin (and its vtable) drops first.
    /// 声明在 `plugin` 之后，确保插件（及其 vtable）先于库被释放。
    library: Option<rp2350_ffi::SafeLibrary>,
}

/// Error returned by [`DeviceManager::apply_config`]
/// [`DeviceManager::apply_config`] 返回的错误
#[derive(Debug, thiserror::Error)]
pub enum ConfigApplyError {
    /// The requested device is not registered / 请求的设备未注册
    #[error("Device not found: {0} / 设备未找到: {0}")]
    DeviceNotFound(String),

    /// The plugin rejected the configuration / 插件拒绝了该配置
    #[error(transparent)]
    Invalid(#[from] ConfigError),
}

/// Device manager — the central hub for all virtual peripheral plugins
/// 设备管理器 — 所有虚拟外设插件的中央枢纽
pub struct DeviceManager {
    /// All registered devices keyed by ID / 所有已注册设备（按 ID 索引）
    devices: HashMap<DeviceId, ManagedDevice>,
    /// Wiring table mapping (device, port) ↔ GPIO pin
    /// 接线表映射 (设备, 端口) ↔ GPIO 引脚
    wiring: WiringTable,
    /// Plugin loader for dynamic library scanning
    /// 用于动态库扫描的插件加载器
    loader: PluginLoader,
}

impl DeviceManager {
    /// Create a new DeviceManager / 创建新的 DeviceManager
    #[must_use]
    pub fn new() -> Self {
        Self {
            devices: HashMap::new(),
            wiring: WiringTable::new(),
            loader: PluginLoader::default(),
        }
    }

    /// Register an already-constructed device plugin
    /// 注册已构建的设备插件
    pub fn register(
        &mut self,
        id: DeviceId,
        device: Box<dyn DevicePlugin>,
    ) -> Result<(), DeviceError> {
        if self.devices.contains_key(&id) {
            return Err(DeviceError::AlreadyExists(id));
        }
        self.devices.insert(
            id,
            ManagedDevice {
                plugin: device,
                source_path: None,
                library: None,
            },
        );
        Ok(())
    }

    /// Load a device plugin from a dynamic library and register it
    /// 从动态库加载设备插件并注册
    ///
    /// The library is kept alive for as long as the device remains registered.
    /// 只要设备保持注册状态，库就会保持存活。
    pub fn load_plugin(&mut self, id: DeviceId, path: impl AsRef<Path>) -> Result<(), DeviceError> {
        if self.devices.contains_key(&id) {
            return Err(DeviceError::AlreadyExists(id));
        }
        let LoadedPlugin { library, plugin } = self.loader.load_device(path.as_ref())?;
        self.devices.insert(
            id,
            ManagedDevice {
                plugin,
                source_path: Some(path.as_ref().to_path_buf()),
                library: Some(library),
            },
        );
        Ok(())
    }

    /// Remove a registered device / 移除已注册的设备
    pub fn unregister(&mut self, id: &DeviceId) -> Result<(), DeviceError> {
        let mut device = self
            .devices
            .remove(id)
            .ok_or_else(|| DeviceError::NotFound(id.clone()))?;
        device.plugin.shutdown();
        self.wiring.remove_device(id);
        Ok(())
    }

    /// Wire a device port to a GPIO pin / 将设备端口连接到 GPIO 引脚
    pub fn wire(
        &mut self,
        device_id: &str,
        port_index: usize,
        gpio_pin: u8,
    ) -> Result<(), DeviceError> {
        let device = self
            .devices
            .get(device_id)
            .ok_or_else(|| DeviceError::NotFound(device_id.to_string()))?;

        let ports = device.plugin.ports();
        if port_index >= ports.len() {
            return Err(DeviceError::PortIndexOutOfBounds(port_index));
        }

        let port_name = ports[port_index].name.clone();
        self.wiring
            .wire(device_id, &port_name, port_index, gpio_pin);
        Ok(())
    }

    /// Get a reference to the wiring table / 获取接线表引用
    #[must_use]
    pub fn wiring(&self) -> &WiringTable {
        &self.wiring
    }

    /// Notify all devices about GPIO pin changes and sync output levels
    /// 通知所有设备 GPIO 引脚变化并同步输出电平
    pub fn sync_gpio(&mut self, gpio_pins: &mut [GpioPin]) {
        // Input → Device: read GPIO output level, feed into device input ports
        // Input → Device: 读取 GPIO 输出电平，馈入设备输入端口
        for entry in self.wiring.all() {
            if let Some(device) = self.devices.get_mut(&entry.device_id) {
                let ports = device.plugin.ports();
                if entry.port_index < ports.len()
                    && matches!(
                        ports[entry.port_index].direction,
                        super::device::PortDirection::Input
                            | super::device::PortDirection::Bidirectional
                    )
                {
                    if let Some(pin) = gpio_pins.get_mut(entry.gpio_pin as usize) {
                        device.plugin.on_gpio_change(entry.port_index, pin.level());
                    }
                }
            }
        }

        // Device → Output: read device output port level, write into GPIO pin
        // Device → Output: 读取设备输出端口电平，写入 GPIO 引脚
        for (device_id, managed) in &mut self.devices {
            let ports = managed.plugin.ports();
            let _snapshot = managed.plugin.state_snapshot();

            // For each output port, check the wiring and update GPIO
            for (i, port) in ports.iter().enumerate() {
                if matches!(
                    port.direction,
                    super::device::PortDirection::Output
                        | super::device::PortDirection::Bidirectional
                ) {
                    if let Some(gpio_pin) = self.wiring.find_by_device_port(device_id, i) {
                        if let Some(pin) = gpio_pins.get_mut(gpio_pin as usize) {
                            pin.set_input(port.level);
                        }
                    }
                }
            }
        }
    }

    /// Tick all devices / 推进所有设备的时钟
    pub fn tick_all(&mut self, ctx: &mut dyn PluginContext, cycles: u64) {
        for managed in self.devices.values_mut() {
            managed.plugin.tick(ctx, cycles);
        }
    }

    /// Shutdown all devices / 关闭所有设备
    pub fn shutdown_all(&mut self) {
        for managed in self.devices.values_mut() {
            managed.plugin.shutdown();
        }
    }

    /// Query all device states (MCP API) / 查询所有设备状态（MCP API）
    #[must_use]
    pub fn mcp_query_all(&self) -> serde_json::Value {
        let mut states = serde_json::Map::new();
        for (id, managed) in &self.devices {
            states.insert(id.clone(), managed.plugin.state_snapshot());
        }
        serde_json::Value::Object(states)
    }

    /// MCP API: query one device state; `None` when the ID is unknown
    /// MCP API：查询单个设备状态；ID 不存在时返回 `None`
    #[must_use]
    pub fn mcp_query_device(&self, id: &str) -> Option<serde_json::Value> {
        self.devices
            .get(id)
            .map(|managed| managed.plugin.state_snapshot())
    }

    /// MCP API: list devices as `[{id, name, has_ui, tags}]`, sorted by ID
    /// MCP API：列出设备（id/name/has_ui/tags），按 ID 排序
    #[must_use]
    pub fn mcp_list_devices(&self) -> Vec<serde_json::Value> {
        let mut entries: Vec<serde_json::Value> = self
            .devices
            .iter()
            .map(|(id, managed)| {
                let metadata = managed.plugin.metadata();
                serde_json::json!({
                    "id": id,
                    "name": metadata.name,
                    "has_ui": managed.plugin.has_ui(),
                    "tags": metadata.tags,
                })
            })
            .collect();
        // HashMap 迭代顺序不稳定，按 ID 排序保证 MCP 输出可复现。
        // HashMap iteration order is unstable; sort by ID for reproducible MCP output.
        entries.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        entries
    }

    /// MCP API: apply a JSON configuration to one device
    /// MCP API：向单个设备应用 JSON 配置
    ///
    /// # Errors
    ///
    /// Returns [`ConfigApplyError::DeviceNotFound`] for an unknown ID and
    /// [`ConfigApplyError::Invalid`] when the plugin rejects the configuration.
    /// 未知 ID 返回 [`ConfigApplyError::DeviceNotFound`]，插件拒绝配置时返回
    /// [`ConfigApplyError::Invalid`]。
    pub fn apply_config(
        &mut self,
        id: &str,
        config: &serde_json::Value,
    ) -> Result<(), ConfigApplyError> {
        let managed = self
            .devices
            .get_mut(id)
            .ok_or_else(|| ConfigApplyError::DeviceNotFound(id.to_string()))?;
        managed.plugin.apply_config(config)?;
        Ok(())
    }

    /// Get a reference to the plugin loader / 获取插件加载器引用
    #[must_use]
    pub fn loader(&self) -> &PluginLoader {
        &self.loader
    }

    /// Get a mutable reference to the plugin loader / 获取插件加载器可变引用
    pub fn loader_mut(&mut self) -> &mut PluginLoader {
        &mut self.loader
    }

    /// Check if any devices are registered / 检查是否有已注册设备
    #[must_use]
    pub fn has_devices(&self) -> bool {
        !self.devices.is_empty()
    }

    /// Number of registered devices / 已注册设备数量
    #[must_use]
    pub fn device_count(&self) -> usize {
        self.devices.len()
    }
}

impl Default for DeviceManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DevicePort;
    use crate::error::ConfigError;
    use rp2350_plugin::PluginContext;
    use rp2350_plugin::RpPlugin;
    use rp2350_plugin::error::PluginError;
    use rp2350_plugin::metadata::PluginMetadata;

    /// A minimal test device plugin / 最小测试设备插件
    struct TestLed {
        metadata: PluginMetadata,
        ports: Vec<DevicePort>,
        level: bool,
    }

    impl RpPlugin for TestLed {
        fn metadata(&self) -> &PluginMetadata {
            &self.metadata
        }
        fn init(&mut self, _ctx: &mut dyn PluginContext) -> Result<(), PluginError> {
            Ok(())
        }
        fn tick(&mut self, _ctx: &mut dyn PluginContext, _cycles: u64) {}
        fn shutdown(&mut self) {}
        fn state_snapshot(&self) -> serde_json::Value {
            serde_json::json!({
                "level": self.level,
            })
        }
    }

    impl DevicePlugin for TestLed {
        fn ports(&self) -> &[DevicePort] {
            &self.ports
        }
        fn ports_mut(&mut self) -> &mut [DevicePort] {
            &mut self.ports
        }
        fn on_gpio_change(&mut self, _port_index: usize, level: bool) {
            self.level = level;
        }
        fn apply_config(&mut self, config: &serde_json::Value) -> Result<(), ConfigError> {
            let level = config
                .get("level")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| ConfigError::MissingField("level".to_string()))?;
            self.level = level;
            Ok(())
        }
    }

    impl TestLed {
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
                    .has_ui(false)
                    .tags(vec!["led".into(), "test".into()])
                    .build(),
                ports: vec![DevicePort::input("LED", "LED input port")],
                level: false,
            }
        }
    }

    #[test]
    fn register_and_wire_device() {
        let mut dm = DeviceManager::new();
        dm.register("led-1".into(), Box::new(TestLed::new()))
            .unwrap();
        dm.wire("led-1", 0, 25).unwrap();
        assert_eq!(dm.wiring().len(), 1);
    }

    #[test]
    fn mcp_query_empty() {
        let dm = DeviceManager::new();
        let result = dm.mcp_query_all();
        assert_eq!(result.as_object().unwrap().len(), 0);
    }

    #[test]
    fn mcp_query_with_device() {
        let mut dm = DeviceManager::new();
        dm.register("led-1".into(), Box::new(TestLed::new()))
            .unwrap();
        let result = dm.mcp_query_all();
        assert!(result.get("led-1").is_some());
    }

    #[test]
    fn mcp_query_device_hit_and_miss() {
        let mut dm = DeviceManager::new();
        dm.register("led-1".into(), Box::new(TestLed::new()))
            .unwrap();
        assert_eq!(
            dm.mcp_query_device("led-1").unwrap()["level"],
            serde_json::json!(false)
        );
        assert!(dm.mcp_query_device("missing").is_none());
    }

    #[test]
    fn mcp_list_devices_reports_metadata() {
        let mut dm = DeviceManager::new();
        dm.register("led-1".into(), Box::new(TestLed::new()))
            .unwrap();
        let devices = dm.mcp_list_devices();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0]["id"], "led-1");
        assert_eq!(devices[0]["name"], "Test LED");
        assert_eq!(devices[0]["has_ui"], false);
        assert_eq!(devices[0]["tags"], serde_json::json!(["led", "test"]));
    }

    #[test]
    fn apply_config_updates_device() {
        let mut dm = DeviceManager::new();
        dm.register("led-1".into(), Box::new(TestLed::new()))
            .unwrap();
        dm.apply_config("led-1", &serde_json::json!({ "level": true }))
            .unwrap();
        assert_eq!(
            dm.mcp_query_device("led-1").unwrap()["level"],
            serde_json::json!(true)
        );
    }

    #[test]
    fn apply_config_reports_errors() {
        let mut dm = DeviceManager::new();
        dm.register("led-1".into(), Box::new(TestLed::new()))
            .unwrap();
        let missing_device = dm.apply_config("missing", &serde_json::json!({ "level": true }));
        assert!(matches!(
            missing_device,
            Err(ConfigApplyError::DeviceNotFound(_))
        ));

        let invalid_config = dm.apply_config("led-1", &serde_json::json!({}));
        assert!(matches!(
            invalid_config,
            Err(ConfigApplyError::Invalid(ConfigError::MissingField(_)))
        ));
    }
}
