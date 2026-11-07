# rp2350-virt-periph — Public API / 公共 API

> Virtual Device Manager: plugin discovery, loading, wiring, and lifecycle. Does **NOT** contain device implementations — all devices live in `plugins/`.
> 虚拟设备管理器：插件发现、加载、接线和生命周期管理。**不包含**设备实现 —— 所有设备位于 `plugins/`。

---

## Role / 角色定位

```
┌─────────────────────────────────────────────────────────┐
│                   rp2350-virt-periph                     │
│                                                         │
│  ┌─────────────┐  ┌──────────────┐  ┌───────────────┐  │
│  │PluginLoader  │  │DeviceManager │  │ WiringTable   │  │
│  │              │  │              │  │               │  │
│  │ scan plugins/│  │ load/unload  │  │ port↔pin map  │  │
│  │ dlopen .dll  │  │ tick_all()   │  │ connect/disco │  │
│  │ verify ABI   │  │ bus_devices()│  │ save/load     │  │
│  └─────────────┘  └──────────────┘  └───────────────┘  │
│                                                         │
│  ┌──────────────────────────────────────────────────┐   │
│  │ DevicePlugin trait (defined here, extends         │   │
│  │   rp2350_plugin::RpPlugin)                       │   │
│  └──────────────────────────────────────────────────┘   │
│                                                         │
│  Plugins/ directory:                                    │
│  ├── led.dll          (DevicePlugin)                    │
│  ├── tfcard.dll       (DevicePlugin)                    │
│  ├── uart-terminal.dll(DevicePlugin)                    │
│  ├── debug-probe.dll  (DevicePlugin)                    │
│  ├── psram.dll        (DevicePlugin)                    │
│  ├── qspi-flash.dll   (DevicePlugin)                    │
│  └── third-party/                                      │
│      └── custom-sensor.dll  (DevicePlugin)              │
└─────────────────────────────────────────────────────────┘
```

---

## `DevicePlugin` Trait / DevicePlugin 接口

```rust
/// Device plugin — extends RpPlugin with GPIO ports, optional config, optional GUI.
/// 设备插件 —— 扩展 RpPlugin，增加 GPIO 端口、可选配置、可选 GUI。
///
/// # Headless requirement
/// All implementations MUST work correctly when `render_ui()` is never called.
/// GUI is purely a visualization layer; device logic must not depend on it.
/// 所有实现必须在 `render_ui()` 从未被调用时正常工作。
pub trait DevicePlugin: RpPlugin {
    // ── GPIO Ports (required) ──
    fn ports(&self) -> &[DevicePort];
    fn ports_mut(&mut self) -> &mut [DevicePort];
    fn on_gpio_change(&mut self, port_index: usize, level: bool);

    // ── Configuration (optional, JSON-based) ──
    fn config_schema(&self) -> Option<ConfigSchema> { None }
    fn apply_config(&mut self, config: &serde_json::Value) -> Result<(), ConfigError> { Ok(()) }
    fn current_config(&self) -> Option<serde_json::Value> { None }

    // ── GUI (optional, egui-dependent, never called in headless mode) ──
    fn has_ui(&self) -> bool { false }
    fn render_ui(&mut self, ui: &mut egui::Ui) {}

    // ── Headless/MCP state query (required) ──
    fn state_snapshot(&self) -> serde_json::Value;
}
```

Full trait docs: see [rp2350-plugin.md](rp2350-plugin.md).
完整 trait 文档：参见 [rp2350-plugin.md](rp2350-plugin.md)。

---

## `DeviceManager` / 设备管理器

```rust
/// Central registry for all loaded virtual device plugins.
/// 所有已加载虚拟设备插件的中央注册表。
pub struct DeviceManager {
    /// Plugin loader (libloading wrapper).
    pub loader: PluginLoader,

    /// All loaded devices.
    pub devices: Vec<LoadedDevice>,

    /// GPIO wiring table: (device_name, port_index) → rp2350_pin.
    /// 可运行时修改（GUI 拖线 / CLI wire 命令 / MCP wire 工具）。
    pub wiring: WiringTable,

    /// Headless mode flag.
    pub headless: bool,

    /// Plugin search directories.
    pub plugin_dirs: Vec<PathBuf>,
}

/// A loaded device with its library handle (RAII).
pub struct LoadedDevice {
    /// Plugin name (from metadata).
    pub name: String,
    /// Device type (from metadata).
    pub device_type: DeviceType,
    /// The plugin instance (as DevicePlugin).
    pub plugin: Box<dyn DevicePlugin>,
    /// RAII library handle — auto dlclose on drop.
    _library: SafeLibrary,
    /// Whether this is a first-party or third-party plugin.
    pub origin: PluginOrigin,
}

pub enum PluginOrigin {
    FirstParty,   // from plugins/ root
    ThirdParty,   // from plugins/third-party/
}
```

### DeviceManager Methods / 方法

```rust
impl DeviceManager {
    /// Create a new device manager.
    /// `plugin_dirs` — directories to scan for .dll/.so/.dylib files.
    /// `headless`    — if true, never call render_ui().
    pub fn new(plugin_dirs: Vec<PathBuf>, headless: bool) -> Self;

    // ── Plugin Loading ──

    /// Scan all plugin directories and load every compatible plugin.
    /// Returns a list of load results (successes and failures).
    /// 扫描所有插件目录并加载每个兼容插件。
    pub fn scan_and_load(&mut self) -> Vec<PluginLoadResult>;

    /// Load a single plugin by absolute path.
    /// 按绝对路径加载单个插件。
    pub fn load_plugin(&mut self, path: &Path) -> Result<PluginLoadResult>;

    /// Unload a plugin by name.
    /// 按名称卸载插件。
    pub fn unload_plugin(&mut self, name: &str);

    /// Unload all plugins.
    pub fn unload_all(&mut self);

    /// Get a device by name.
    pub fn device(&self, name: &str) -> Option<&dyn DevicePlugin>;

    /// Get a device mutably by name.
    pub fn device_mut(&mut self, name: &str) -> Option<&mut dyn DevicePlugin>;

    // ── Wiring ──

    /// Wire a device port to an RP2350 GPIO pin.
    /// `device_name` — plugin metadata name.
    /// `port_index`  — index into device.ports().
    /// `gpio_pin`    — RP2350 GPIO pin number (0-47).
    /// 将设备端口连接到 RP2350 GPIO 引脚。
    pub fn wire(&mut self, device_name: &str, port_index: usize, gpio_pin: u8) -> Result<()>;

    /// Remove a wire.
    /// 移除连线。
    pub fn unwire(&mut self, device_name: &str, port_index: usize);

    /// Remove all wires for a device.
    /// 移除设备的所有连线。
    pub fn unwire_all(&mut self, device_name: &str);

    /// Get all wires as JSON (for MCP / save).
    /// 获取所有连线（JSON 格式）。
    pub fn wiring_to_json(&self) -> serde_json::Value;

    /// Load wiring from JSON (for MCP / restore).
    /// 从 JSON 加载连线。
    pub fn wiring_from_json(&mut self, json: &serde_json::Value) -> Result<()>;

    // ── Runtime ──

    /// Notify all devices of a GPIO pin level change.
    /// Called by the emulator core every cycle for any changed GPIO.
    /// 通知所有设备 GPIO 引脚电平变化。
    /// 由模拟器内核在 GPIO 变化时每周期调用。
    pub fn notify_gpio_change(&mut self, gpio_pin: u8, level: bool);

    /// Set GPIO level on RP2350 side from a device output port.
    /// Used by DevicePlugin impls that drive GPIO output → RP2350 input.
    /// 从设备输出端口设置 RP2350 侧 GPIO 电平。
    pub fn device_drives_gpio(&mut self, device_name: &str, port_index: usize, level: bool);

    /// Tick all loaded devices.
    /// 驱动所有已加载设备的 tick。
    pub fn tick_all(&mut self, ctx: &mut dyn PluginContext, cycles: u64);

    // ── Bus Device Collection ──

    /// Collect all bus devices from plugins that implement as_bus_device().
    /// Returns list of (base_addr, size, &mut dyn BusDevice).
    /// 收集所有实现 as_bus_device() 的插件的总线设备。
    pub fn collect_bus_devices(&mut self) -> Vec<(u32, u32, &mut dyn BusDevice)>;

    // ── GUI ──

    /// Render all device UIs (only devices where has_ui() == true).
    /// No-op in headless mode.
    /// 渲染所有设备 UI（仅 has_ui() == true 的设备）。
    /// 无头模式下为空操作。
    pub fn render_all_uis(&mut self, ctx: &egui::Context);

    /// Get list of devices with their UI visibility for palette display.
    /// 获取设备列表及其 UI 可见性（用于组件面板显示）。
    pub fn device_palette_entries(&self) -> Vec<DevicePaletteEntry>;

    // ── MCP / Headless API ──

    /// Query all device states as a JSON object.
    /// Primary API for MCP tools and headless CLI.
    /// MCP 工具和无头 CLI 的主要 API。
    pub fn mcp_query_all(&self) -> serde_json::Value {
        let mut states = serde_json::Map::new();
        for device in &self.devices {
            states.insert(device.name.clone(), device.plugin.state_snapshot());
        }
        serde_json::json!({
            "devices": states,
            "wiring": self.wiring_to_json(),
            "device_count": self.devices.len(),
            "headless": self.headless,
        })
    }

    /// Query a single device state.
    /// 查询单个设备状态。
    pub fn mcp_query_device(&self, name: &str) -> Option<serde_json::Value>;

    /// Apply configuration to a device (from MCP or CLI).
    /// 对设备应用配置（来自 MCP 或 CLI）。
    pub fn mcp_configure(&mut self, name: &str, config: &serde_json::Value) -> Result<()>;

    // ── Persistence ──

    /// Save current wiring layout to file.
    /// 保存当前接线布局到文件。
    pub fn save_wiring(&self, path: &Path) -> Result<()>;

    /// Load wiring layout from file.
    /// 从文件加载接线布局。
    pub fn load_wiring(&mut self, path: &Path) -> Result<()>;
}
```

---

## `PluginLoader` / 插件加载器

```rust
/// Low-level plugin loader (libloading wrapper).
/// 底层插件加载器（libloading 封装）。
pub struct PluginLoader {
    /// Directories to scan for plugins.
    pub search_dirs: Vec<PathBuf>,
    /// ABI version required.
    pub required_abi_version: u32,
}

impl PluginLoader {
    /// Create new loader with default search dirs.
    /// Default: ["./plugins/", "./plugins/third-party/"]
    pub fn new() -> Self;

    /// Add a search directory.
    pub fn add_search_dir(&mut self, dir: PathBuf);

    /// Scan a directory for plugin files.
    /// 扫描目录中的插件文件。
    /// File pattern: *.dll (Windows), *.so (Linux), *.dylib (macOS).
    pub fn scan_dir(&self, dir: &Path) -> Vec<PathBuf>;

    /// Try to load a plugin file. Validates ABI version.
    /// 尝试加载插件文件。验证 ABI 版本。
    ///
    /// # Safety
    /// The plugin DLL must export:
    /// - `rp2350_plugin_create` → `*mut dyn RpPlugin`
    /// - `rp2350_plugin_destroy` → `fn(*mut dyn RpPlugin)`
    /// - `rp2350_plugin_version` → `u32`
    /// - `rp2350_plugin_is_device` → `u32` (optional, 1 = DevicePlugin)
    pub unsafe fn load(&self, path: &Path) -> Result<RawPlugin, PluginLoadError>;
}
```

---

## `WiringTable` / 接线表

```rust
/// Bidirectional port ↔ GPIO pin wiring.
/// 双向端口 ↔ GPIO 引脚接线。
pub struct WiringTable {
    /// (device_name, port_index) → gpio_pin
    port_to_pin: HashMap<(String, usize), u8>,
    /// gpio_pin → Vec<(device_name, port_index)>
    pin_to_ports: HashMap<u8, Vec<(String, usize)>>,
}

impl WiringTable {
    pub fn new() -> Self;

    /// Add a wire. One pin can connect to multiple ports.
    /// 添加连线。一个引脚可连接多个端口。
    pub fn connect(&mut self, device: &str, port_index: usize, gpio_pin: u8);

    /// Remove a wire.
    pub fn disconnect(&mut self, device: &str, port_index: usize);

    /// Remove all wires for a GPIO pin.
    pub fn disconnect_pin(&mut self, gpio_pin: u8);

    /// Get the GPIO pin wired to a port.
    pub fn get_pin(&self, device: &str, port_index: usize) -> Option<u8>;

    /// Get all ports wired to a GPIO pin.
    pub fn get_ports(&self, gpio_pin: u8) -> &[(String, usize)];

    /// Check if a port is wired.
    pub fn is_wired(&self, device: &str, port_index: usize) -> bool;

    /// Export as JSON.
    pub fn to_json(&self) -> serde_json::Value;

    /// Import from JSON.
    pub fn from_json(&mut self, json: &serde_json::Value) -> Result<()>;

    /// Clear all wires.
    pub fn clear(&mut self);
}
```

---

## GPIO Notification Flow / GPIO 通知流程

```
RP2350 GPIO pin changes (firmware writes SIO → GPIO_OUT)
     │
     ▼
RP2350Chip::notify_gpio_change(pin, level)
     │
     ▼
DeviceManager::notify_gpio_change(pin, level)
     │
     ├── wiring.get_ports(pin) → [(device, port_index), ...]
     │
     ├── For each wired port:
     │     ├── device.ports_mut()[port_index].current_level = level
     │     └── device.on_gpio_change(port_index, level)
     │
     └── egui canvas → update pin color on chip + port color on device

Device output (device drives GPIO):
     │
     ▼
DeviceManager::device_drives_gpio(device_name, port_index, level)
     │
     ├── wiring.get_pin(device, port_index) → gpio_pin
     ├── ctx.gpio_write(gpio_pin, level)
     │     → RP2350 GpioPin::input_level updated
     │     → Visible in GPIO_IN register
     └── egui canvas → update wire color
```

---

## Configuration Flow / 配置流程

```
┌─ GUI Mode ───────────────────────┐  ┌─ Headless/MCP Mode ──────────┐
│                                   │  │                              │
│ Right-click device → "Configure"  │  │ MCP tool:                    │
│     │                             │  │   mcp_configure("tfcard",    │
│     ▼                             │  │     {mode: "sdio"})          │
│ device.config_schema()            │  │     │                        │
│     │  → JSON Schema              │  │     ▼                        │
│     ▼                             │  │ dm.mcp_configure(            │
│ egui generates form from schema   │  │   "tfcard", &config)         │
│ User fills form → JSON            │  │     │                        │
│     │                             │  │     ▼                        │
│     ▼                             │  │ device.config_schema()       │
│ device.apply_config(&json)        │  │   → validate JSON            │
│     │                             │  │     │                        │
│     ▼                             │  │     ▼                        │
│ device.on_config_applied()        │  │ device.apply_config(&json)   │
└───────────────────────────────────┘  └──────────────────────────────┘
```

Configuration is ALWAYS JSON-based. GUI is merely a visual JSON editor on top of `config_schema()`. The same `apply_config()` path works in headless mode via MCP.
配置始终基于 JSON。GUI 只是 `config_schema()` 之上的可视化 JSON 编辑器。同样的 `apply_config()` 路径在无头模式下通过 MCP 工作。

---

## CLI Usage / CLI 使用

```bash
# GUI mode — auto-loads all plugins from plugins/
rp2350 run firmware.elf

# Headless mode — no window, just simulation
rp2350 run firmware.elf --headless

# Load specific plugins
rp2350 run firmware.elf --plugin-dir ./my-plugins/

# List loaded devices
rp2350 list-devices

# Wire a port (headless)
rp2350 wire --device "LED0" --port 0 --pin 25

# Save/load wiring layout
rp2350 wire save layout.json
rp2350 wire load layout.json

# MCP server mode (headless, exposes JSON-RPC)
rp2350 mcp-serve --port 9000

# MCP client example query
curl -X POST http://localhost:9000/ -d '{
  "method": "mcp_query_all",
  "params": {}
}'
# → {"devices": {"LED0": {"level": true, ...}, "TF_CARD": {"mode": "spi", ...}}, ...}
```

---

## Error Types / 错误类型

```rust
#[derive(Debug, thiserror::Error)]
pub enum ManagerError {
    #[error("Plugin not found: {0}")]
    PluginNotFound(String),
    #[error("Plugin already loaded: {0}")]
    PluginAlreadyLoaded(String),
    #[error("Port index out of range: device {device} port {port}")]
    PortOutOfRange { device: String, port: usize },
    #[error("Port already wired: {device}:{port}")]
    PortAlreadyWired { device: String, port: usize },
    #[error("GPIO pin {0} conflict (already in use)")]
    GpioConflict(u8),
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("Load error: {0}")]
    LoadError(#[from] PluginLoadError),
}

#[derive(Debug, thiserror::Error)]
pub enum PluginLoadError {
    #[error("File not found: {0}")]
    NotFound(PathBuf),
    #[error("Not a valid plugin: {0}")]
    InvalidPlugin(String),
    #[error("ABI version mismatch: plugin {0}, host {1}")]
    AbiMismatch(u32, u32),
    #[error("Init failed: {0}")]
    InitFailed(String),
    #[error("Library error: {0}")]
    Library(#[from] libloading::Error),
}
```

---

## Dependency Graph / 依赖图

```
rp2350-virt-periph
├── rp2350-plugin        (RpPlugin trait, PluginContext, PluginMetadata)
├── rp2350-ffi           (SafeLibrary)
├── rp2350-core          (BusDevice trait, GpioPin)
├── libloading
├── thiserror
├── serde + serde_json
├── egui (for render_ui signature only — headless mode still compiles egui)
└── log
```

---

## `ConfigSchema` / 配置 Schema

```rust
/// JSON Schema for device configuration.
/// Used by GUI auto-form generator and MCP validation.
/// 设备配置的 JSON Schema。用于 GUI 自动表单生成和 MCP 验证。
pub struct ConfigSchema {
    pub properties: Vec<ConfigProperty>,
    pub required: Vec<String>,
}

pub struct ConfigProperty {
    pub name: String,
    pub description: String,
    pub prop_type: ConfigPropType,
    pub default: Option<serde_json::Value>,
    pub enum_values: Option<Vec<String>>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

pub enum ConfigPropType {
    String,
    Integer,
    Float,
    Boolean,
    Enum,
    Path,       // file path, GUI shows file picker
    PinNumber,  // GPIO pin number (0-47), GUI shows pin selector
}
```

---

## Example: TF Card Config Schema / 示例：TF 卡配置 Schema

```json
{
  "properties": [
    {
      "name": "mode",
      "description": "Interface mode",
      "type": "Enum",
      "default": "spi",
      "enum_values": ["spi", "sdio"]
    },
    {
      "name": "image_path",
      "description": "Disk image file",
      "type": "Path",
      "default": ""
    },
    {
      "name": "block_size",
      "description": "Block size in bytes",
      "type": "Integer",
      "default": 512,
      "min": 512,
      "max": 4096
    }
  ],
  "required": ["image_path"]
}
```

This schema works in GUI (auto-rendered as form) and MCP (JSON-RPC validation):
此 schema 在 GUI（自动渲染为表单）和 MCP（JSON-RPC 验证）中均有效：

```bash
# Headless configure via MCP:
rp2350 mcp-configure --device "TF_CARD" --config '{"mode":"sdio","image_path":"/path/to/sdcard.img"}'
```