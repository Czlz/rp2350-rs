//! DeviceManager — central orchestration of virtual device plugins
//! DeviceManager — 虚拟设备插件的中央编排器
//!
//! Owns the plugin lifecycle, wiring table, GPIO routing, tick scheduling,
//! and MCP query API. All devices are loaded as independent plugins.
//! 拥有插件生命周期、接线表、GPIO 路由、tick 调度和 MCP 查询 API。
//! 所有设备作为独立插件加载。

use std::collections::HashMap;
use std::path::PathBuf;

use rp2350_core::gpio::GpioPin;
use rp2350_plugin::PluginContext;

use super::device::DevicePlugin;
use super::error::DeviceError;
use super::loader::PluginLoader;
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
    pub fn register(&mut self, id: DeviceId, device: Box<dyn DevicePlugin>) -> Result<(), DeviceError> {
        if self.devices.contains_key(&id) {
            return Err(DeviceError::AlreadyExists(id));
        }
        self.devices.insert(
            id,
            ManagedDevice {
                plugin: device,
                source_path: None,
            },
        );
        Ok(())
    }

    /// Remove a registered device / 移除已注册的设备
    pub fn unregister(&mut self, id: &DeviceId) -> Result<(), DeviceError> {
        self.devices
            .remove(id)
            .map(|_| {
                self.wiring.remove_device(id);
            })
            .ok_or_else(|| DeviceError::NotFound(id.clone()))?;
        Ok(())
    }

    /// Wire a device port to a GPIO pin / 将设备端口连接到 GPIO 引脚
    pub fn wire(&mut self, device_id: &str, port_index: usize, gpio_pin: u8) -> Result<(), DeviceError> {
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
                        super::device::PortDirection::Input | super::device::PortDirection::Bidirectional
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
                    super::device::PortDirection::Output | super::device::PortDirection::Bidirectional
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
    use super::device::{DevicePort, PortDirection};
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
}