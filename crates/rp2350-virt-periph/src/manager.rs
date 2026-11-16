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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rp2350_core::gpio::GpioPin;
use rp2350_plugin::PluginContext;

use super::device::{DevicePlugin, DevicePort};
use super::error::{ConfigError, DeviceError};
use super::executor::{ExecutorConfig, PinSnapshot, PublishSchedule};
use super::executor_device::DeviceHost;
use super::loader::{LoadedPlugin, PluginLoader};
use super::reload::{Fingerprint, scan_with_fingerprints};
use super::wiring::WiringTable;

/// Identifier type for registered devices / 注册设备的标识类型
pub type DeviceId = String;

/// A managed device instance / 受管理的设备实例
///
/// `pub(crate)` so the [`super::reload`] module can move instances in and out
/// during a hot reload. 对 crate 内可见，使 [`super::reload`] 模块能在热重载时
/// 增删实例。
pub(crate) struct ManagedDevice {
    /// The device plugin host (inline or worker thread) / 设备插件宿主
    pub(crate) host: DeviceHost,
    /// Source path if loaded from disk / 从磁盘加载时的源路径
    pub(crate) source_path: Option<PathBuf>,
    /// Library backing a dynamically loaded plugin / 动态加载插件背后的库
    ///
    /// Declared after `host` so the plugin (and its vtable) drops first.
    /// Only kept alive for RAII: it is never read, dropping it unloads the DLL.
    /// 声明在 `host` 之后，确保插件（及其 vtable）先于库被释放。
    /// 仅用于 RAII：不会被读取，释放它即卸载 DLL。
    #[allow(dead_code)]
    pub(crate) library: Option<rp2350_ffi::SafeLibrary>,
    /// Fingerprint recorded when the library was loaded / 加载库时记录的指纹
    pub(crate) fingerprint: Option<Fingerprint>,
}

impl ManagedDevice {
    /// Build a managed device, threading it when `executor` is configured
    /// 构造受管设备；`executor` 已配置时使用工作线程
    pub(crate) fn new(
        id: &DeviceId,
        plugin: Box<dyn DevicePlugin>,
        source_path: Option<PathBuf>,
        library: Option<rp2350_ffi::SafeLibrary>,
        fingerprint: Option<Fingerprint>,
        executor: Option<ExecutorConfig>,
    ) -> Self {
        let host = match executor {
            Some(config) => DeviceHost::threaded(id.clone(), plugin, Vec::new(), config),
            None => DeviceHost::inline(plugin),
        };
        Self {
            host,
            source_path,
            library,
            fingerprint,
        }
    }
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

    /// The plugin worker failed or is latched high-Z
    /// 插件工作线程失败或已锁定为高阻
    #[error(transparent)]
    Executor(#[from] crate::executor::ExecutorError),
}

/// Device manager — the central hub for all virtual peripheral plugins
/// 设备管理器 — 所有虚拟外设插件的中央枢纽
pub struct DeviceManager {
    /// All registered devices keyed by ID / 所有已注册设备（按 ID 索引）
    ///
    /// `BTreeMap` keeps iteration deterministic (ID order), which the stepped
    /// path relies on for reproducible output ordering.
    /// `BTreeMap` 使迭代顺序确定（按 ID），步进路径依赖它获得可复现的输出顺序。
    pub(crate) devices: BTreeMap<DeviceId, ManagedDevice>,
    /// Wiring table mapping (device, port) ↔ GPIO pin
    /// 接线表映射 (设备, 端口) ↔ GPIO 引脚
    pub(crate) wiring: WiringTable,
    /// Plugin loader for dynamic library scanning
    /// 用于动态库扫描的插件加载器
    pub(crate) loader: PluginLoader,
    /// Executor configuration; `None` keeps the synchronous default
    /// 执行器配置；`None` 保持同步默认路径
    pub(crate) executor: Option<ExecutorConfig>,
    /// Fixed-latency publish queue of the stepped path / 步进路径的固定延迟发布队列
    pub(crate) schedule: PublishSchedule,
    /// Last epoch accepted by [`DeviceManager::step`] / 步进接受的最后 epoch
    pub(crate) last_step_epoch: Option<u64>,
    /// Pins driven by at least one device output / 至少被一个设备输出驱动的引脚
    pub(crate) driven_pins: PinSnapshot,
}

impl DeviceManager {
    /// Create a new DeviceManager / 创建新的 DeviceManager
    #[must_use]
    pub fn new() -> Self {
        Self {
            devices: BTreeMap::new(),
            wiring: WiringTable::new(),
            loader: PluginLoader::default(),
            executor: None,
            schedule: PublishSchedule::new(super::executor::DEFAULT_LATENCY_TICKS),
            last_step_epoch: None,
            driven_pins: PinSnapshot::empty(),
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
            id.clone(),
            ManagedDevice::new(&id, device, None, None, None, self.executor),
        );
        self.refresh_bindings(&id);
        Ok(())
    }

    /// Load a device plugin from a dynamic library and register it
    /// 从动态库加载设备插件并注册
    ///
    /// The library is kept alive for as long as the device remains registered.
    /// The registry ID is supplied by the host; the plugin's own metadata ID is
    /// **not** used here (only [`DeviceManager::scan_and_load`] and
    /// [`DeviceManager::reload`] derive it from metadata). Loading the same path
    /// twice under two different IDs therefore creates two independent
    /// instances; loading it twice under the same ID fails before the library is
    /// touched.
    /// 库在设备注册期间保持存活。注册 ID 由宿主给出，这里**不**使用插件自身的
    /// 元数据 ID（只有 [`DeviceManager::scan_and_load`] 与
    /// [`DeviceManager::reload`] 会从元数据推导）。因此同一路径用两个不同 ID
    /// 加载会得到两个独立实例；用同一 ID 加载两次会在触碰库之前失败。
    ///
    /// # Errors
    ///
    /// Returns [`DeviceError::AlreadyExists`] for a duplicate ID and
    /// [`DeviceError::PluginLoadFailed`] when the path is missing, is a
    /// directory, is empty, is not a loadable library, lacks an ABI symbol, or
    /// reports an incompatible ABI version.
    /// 重复 ID 返回 [`DeviceError::AlreadyExists`]；路径不存在、是目录、为空、
    /// 不是可加载库、缺少 ABI 符号或 ABI 版本不兼容时返回
    /// [`DeviceError::PluginLoadFailed`]。
    pub fn load_plugin(&mut self, id: DeviceId, path: impl AsRef<Path>) -> Result<(), DeviceError> {
        if self.devices.contains_key(&id) {
            return Err(DeviceError::AlreadyExists(id));
        }
        let path = path.as_ref();
        let LoadedPlugin { library, plugin } = self.loader.load_device(path)?;
        self.devices.insert(
            id.clone(),
            ManagedDevice::new(
                &id,
                plugin,
                Some(path.to_path_buf()),
                Some(library),
                Fingerprint::read(path),
                self.executor,
            ),
        );
        self.refresh_bindings(&id);
        Ok(())
    }

    /// Scan a directory and register every device plugin it contains
    /// 扫描目录并注册其中所有的设备插件
    ///
    /// Libraries are enumerated by [`PluginLoader::scan_dir`] (platform
    /// extensions, sorted by path) and instantiated through
    /// [`PluginLoader::load_device`]. The device ID is the plugin's
    /// `metadata().id` when non-empty, otherwise the library file stem.
    /// Duplicate IDs and libraries that fail to load are skipped with a
    /// `log::warn!` entry — one bad plugin never aborts the scan.
    ///
    /// 库由 [`PluginLoader::scan_dir`] 枚举（按平台扩展名，路径排序），并通过
    /// [`PluginLoader::load_device`] 实例化。设备 ID 优先取插件的
    /// `metadata().id`，为空时退回库文件名主干。重复 ID 与加载失败的库会被
    /// 跳过并记录 `log::warn!` —— 单个坏插件不会中断整个扫描。
    ///
    /// Returns `(device_id, library_path)` for every registered device.
    /// 返回每个已注册设备的 `(设备 ID, 库路径)`。
    #[must_use]
    pub fn scan_and_load(&mut self, dir: impl AsRef<Path>) -> Vec<(DeviceId, PathBuf)> {
        let dir = dir.as_ref();
        let mut registered = Vec::new();
        for (path, fingerprint) in scan_with_fingerprints(dir) {
            match self.insert_loaded(&path, Some(fingerprint)) {
                Ok(id) => registered.push((id, path)),
                Err(error) => log::warn!(
                    "Failed to load device plugin {}: {} / 设备插件加载失败",
                    path.display(),
                    error
                ),
            }
        }
        registered
    }

    /// Remove a registered device / 移除已注册的设备
    ///
    /// In threaded mode the worker is stopped and joined before this returns
    /// (bounded by the shutdown deadline), the plugin box is dropped, and the
    /// wiring entries are removed. After the call, [`DeviceManager::thread_count`]
    /// and the next [`DeviceManager::step`] never observe the device.
    /// 线程化模式下本调用返回前会停止并 join 工作线程（受关闭期限约束）、释放插件
    /// box 并移除接线条目；返回后 [`DeviceManager::thread_count`] 与下一次
    /// [`DeviceManager::step`] 都不会再看到该设备。
    pub fn unregister(&mut self, id: &DeviceId) -> Result<(), DeviceError> {
        if self.drop_device_instance(id) {
            Ok(())
        } else {
            Err(DeviceError::NotFound(id.clone()))
        }
    }

    /// Wire a device port to a GPIO pin / 将设备端口连接到 GPIO 引脚
    ///
    /// In threaded mode the new binding is pushed to the device worker before the
    /// call returns, so the next step already sees it.
    /// 线程化模式下新绑定会在返回前推送到设备工作线程，下一次步进即生效。
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

        let ports = device.host.ports();
        if port_index >= ports.len() {
            return Err(DeviceError::PortIndexOutOfBounds(port_index));
        }

        let port_name = ports[port_index].name.clone();
        self.wiring
            .wire(device_id, &port_name, port_index, gpio_pin);
        self.refresh_bindings(device_id);
        Ok(())
    }

    /// Get a reference to the wiring table / 获取接线表引用
    #[must_use]
    pub fn wiring(&self) -> &WiringTable {
        &self.wiring
    }

    /// Notify all devices about GPIO pin changes and sync output levels
    /// 通知所有设备 GPIO 引脚变化并同步输出电平
    ///
    /// This is the **synchronous** path and only touches inline devices. Devices
    /// hosted on worker threads are skipped: drive them with
    /// [`DeviceManager::step`], which keeps the fixed propagation delay.
    /// 这是**同步**路径，只作用于内联设备。工作线程上的设备会被跳过：请用
    /// [`DeviceManager::step`] 驱动它们，以保持固定传播延迟。
    pub fn sync_gpio(&mut self, gpio_pins: &mut [GpioPin]) {
        // Input → Device: read GPIO output level, feed into device input ports
        // Input → Device: 读取 GPIO 输出电平，馈入设备输入端口
        for entry in self.wiring.all() {
            if let Some(device) = self.devices.get_mut(&entry.device_id) {
                let ports = device.host.ports();
                if entry.port_index < ports.len()
                    && matches!(
                        ports[entry.port_index].direction,
                        super::device::PortDirection::Input
                            | super::device::PortDirection::Bidirectional
                    )
                {
                    if let Some(pin) = gpio_pins.get(entry.gpio_pin as usize) {
                        let level = pin.level();
                        device.host.notify_gpio(entry.port_index, level);
                    }
                }
            }
        }

        // Device → Output: read device output port level, write into GPIO pin
        // Device → Output: 读取设备输出端口电平，写入 GPIO 引脚
        for (device_id, managed) in &mut self.devices {
            let ports = managed.host.ports();

            // For each output port, check the wiring and update GPIO
            for (i, port) in ports.iter().enumerate() {
                if matches!(
                    port.direction,
                    super::device::PortDirection::Output
                        | super::device::PortDirection::Bidirectional
                ) {
                    if let Some(gpio_pin) = self.wiring.find_by_device_port(device_id, i) {
                        if let Some(pin) = gpio_pins.get_mut(gpio_pin as usize) {
                            if let Some(level) = managed.host.output_level(i) {
                                pin.set_input(level);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Tick all devices / 推进所有设备的时钟
    ///
    /// Synchronous path only; worker-hosted devices are advanced by
    /// [`DeviceManager::step`]. 仅同步路径；工作线程上的设备由
    /// [`DeviceManager::step`] 推进。
    pub fn tick_all(&mut self, ctx: &mut dyn PluginContext, cycles: u64) {
        for managed in self.devices.values_mut() {
            managed.host.tick_inline(ctx, cycles);
        }
    }

    /// Shutdown all devices / 关闭所有设备
    pub fn shutdown_all(&mut self) {
        for managed in self.devices.values_mut() {
            managed.host.shutdown_in_place();
        }
    }

    /// Query all device states (MCP API) / 查询所有设备状态（MCP API）
    ///
    /// Takes `&mut self` because worker-hosted devices answer through their
    /// mailbox. 工作线程上的设备通过邮箱应答，因此需要 `&mut self`。
    #[must_use]
    pub fn mcp_query_all(&mut self) -> serde_json::Value {
        let mut states = serde_json::Map::new();
        for (id, managed) in &mut self.devices {
            states.insert(id.clone(), managed.host.state_snapshot());
        }
        serde_json::Value::Object(states)
    }

    /// MCP API: query one device state; `None` when the ID is unknown
    /// MCP API：查询单个设备状态；ID 不存在时返回 `None`
    #[must_use]
    pub fn mcp_query_device(&mut self, id: &str) -> Option<serde_json::Value> {
        self.devices
            .get_mut(id)
            .map(|managed| managed.host.state_snapshot())
    }

    /// Port table of one device, `None` when the ID is unknown
    /// 单个设备的端口表；ID 不存在时返回 `None`
    ///
    /// Works for inline and worker-hosted devices: the threaded host caches the
    /// port table captured at registration / reload.
    /// 内联与工作线程设备都可用：线程化宿主缓存注册/重载时捕获的端口表。
    #[must_use]
    pub fn ports_of(&self, id: &str) -> Option<&[DevicePort]> {
        self.devices.get(id).map(|managed| managed.host.ports())
    }

    /// MCP API: list devices as `[{id, name, has_ui, tags, ports}]`, sorted by ID
    /// MCP API：列出设备（id/name/has_ui/tags/ports），按 ID 排序
    ///
    /// `ports` holds the port names in wiring order, i.e. the index accepted by
    /// [`DeviceManager::wire`]. `ports` 为按接线顺序排列的端口名，即
    /// [`DeviceManager::wire`] 使用的索引顺序。
    #[must_use]
    pub fn mcp_list_devices(&self) -> Vec<serde_json::Value> {
        let mut entries: Vec<serde_json::Value> = self
            .devices
            .iter()
            .map(|(id, managed)| {
                let metadata = managed.host.metadata();
                let ports: Vec<&str> = managed
                    .host
                    .ports()
                    .iter()
                    .map(|port| port.name.as_str())
                    .collect();
                serde_json::json!({
                    "id": id,
                    "name": metadata.name,
                    "has_ui": managed.host.has_ui(),
                    "tags": metadata.tags,
                    "ports": ports,
                })
            })
            .collect();
        // `BTreeMap` 已按 ID 排序；显式排序使输出契约与容器实现无关。
        // `BTreeMap` is already ID-ordered; sort explicitly so the contract does
        // not depend on the container.
        entries.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        entries
    }

    /// MCP API: apply a JSON configuration to one device
    /// MCP API：向单个设备应用 JSON 配置
    ///
    /// Threaded devices apply the configuration on their worker thread.
    /// 线程化设备在工作线程上应用配置。
    ///
    /// # Errors
    ///
    /// Returns [`ConfigApplyError::DeviceNotFound`] for an unknown ID,
    /// [`ConfigApplyError::Invalid`] when the plugin rejects the configuration,
    /// and [`ConfigApplyError::Executor`] when a worker times out or is high-Z.
    /// 未知 ID 返回 [`ConfigApplyError::DeviceNotFound`]，插件拒绝配置时返回
    /// [`ConfigApplyError::Invalid`]，工作线程超时或已高阻时返回
    /// [`ConfigApplyError::Executor`]。
    pub fn apply_config(
        &mut self,
        id: &str,
        config: &serde_json::Value,
    ) -> Result<(), ConfigApplyError> {
        let managed = self
            .devices
            .get_mut(id)
            .ok_or_else(|| ConfigApplyError::DeviceNotFound(id.to_string()))?;
        managed.host.configure(id, config)
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

/// Resolve the device ID: plugin metadata first, library file stem as fallback
/// 解析设备 ID：优先插件元数据，退回库文件名主干
pub(crate) fn device_id_for(metadata_id: &str, path: &Path) -> DeviceId {
    if metadata_id.is_empty() {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string()
    } else {
        metadata_id.to_string()
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
        let mut dm = DeviceManager::new();
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
        assert_eq!(devices[0]["ports"], serde_json::json!(["LED"]));
    }

    /// Create a fresh temporary directory for scan tests
    /// 为扫描测试创建全新的临时目录
    fn temp_dir(tag: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("rp2350-virt-periph-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn scan_and_load_empty_dir_returns_empty() {
        let dir = temp_dir("scan-empty");
        let mut dm = DeviceManager::new();
        assert_eq!(dm.scan_and_load(&dir), Vec::<(DeviceId, PathBuf)>::new());
        assert_eq!(dm.device_count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_and_load_missing_dir_returns_empty() {
        let dir = temp_dir("scan-missing");
        let missing = dir.join("does-not-exist");
        let mut dm = DeviceManager::new();
        assert_eq!(dm.scan_and_load(missing), Vec::<(DeviceId, PathBuf)>::new());
        assert_eq!(dm.device_count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn device_id_prefers_metadata_then_file_stem() {
        let path = Path::new("/tmp/plugins/rp2350_led_plugin.dll");
        assert_eq!(
            device_id_for("com.rp2350-rs.led", path),
            "com.rp2350-rs.led"
        );
        assert_eq!(device_id_for("", path), "rp2350_led_plugin");
    }

    /// Locate the built LED plugin library in the workspace target directory
    /// 在工作区 target 目录中定位已构建的 LED 插件库
    fn locate_led_plugin() -> Option<PathBuf> {
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
        let name = if cfg!(target_os = "windows") {
            "rp2350_led_plugin.dll"
        } else if cfg!(target_os = "macos") {
            "librp2350_led_plugin.dylib"
        } else {
            "librp2350_led_plugin.so"
        };
        ["debug", "release", "debug/deps"]
            .into_iter()
            .map(|profile| target.join(profile).join(name))
            .find(|path| path.is_file())
    }

    #[test]
    fn scan_and_load_skips_duplicate_plugin() {
        let Some(source) = locate_led_plugin() else {
            eprintln!(
                "SKIP: LED plugin library not built; run `cargo build -p rp2350-led-plugin` \
                 / 未构建 LED 插件，跳过"
            );
            return;
        };
        let dir = temp_dir("scan-duplicate");
        let file_name = source.file_name().expect("library file name");
        std::fs::copy(&source, dir.join(file_name)).expect("copy plugin library");

        let mut dm = DeviceManager::new();
        let first = dm.scan_and_load(&dir);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].0, "com.rp2350-rs.led");
        assert_eq!(dm.device_count(), 1);

        // 第二次扫描必须跳过重复 ID，且不影响已注册设备。
        // A second scan must skip the duplicate ID without disturbing the device.
        let second = dm.scan_and_load(&dir);
        assert_eq!(second, Vec::<(DeviceId, PathBuf)>::new());
        assert_eq!(dm.device_count(), 1);
        dm.wire("com.rp2350-rs.led", 0, 25).expect("wire LED port");
        assert_eq!(dm.wiring().len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
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
