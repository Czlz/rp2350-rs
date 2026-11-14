# rp2350-virt-periph — Public API / 公共 API

> Virtual Device Manager: lifecycle, GPIO wiring, tick scheduling and MCP state
> query for virtual device plugins. Does **NOT** contain device implementations.
> 虚拟设备管理器：虚拟外设插件的生命周期、GPIO 接线、tick 调度与 MCP 状态查询。
> **不包含**设备实现 —— 设备本体位于插件中。
>
> **Status**: 已实现 / implemented — `DeviceManager`（无参构造）、`register` /
> `load_plugin` / `unregister` / `wire` / `sync_gpio` / `tick_all` /
> `shutdown_all` / `mcp_query_all` / `mcp_query_device` / `mcp_list_devices` /
> `apply_config`、`PluginLoader`（目录扫描 + ABI 校验）、
> `WiringTable`、`DevicePlugin`/`DevicePort`、`ConfigSchema`。见文末“规划中”。

---

## Module Structure / 模块结构

| Module | Description / 描述 |
|--------|-------------------|
| `manager` | `DeviceManager` — central orchestration |
| `loader` | `PluginLoader` — directory scan, ABI check, `LoadedPlugin` |
| `wiring` | `WiringTable` / `WiringEntry` — (device, port) ↔ GPIO pin |
| `device` | `DevicePlugin` trait, `DevicePort`, `PortDirection` |
| `config` | `ConfigSchema` — JSON config schema |
| `error` | `DeviceError`, `ConfigError` |

---

## `DevicePlugin` Trait / DevicePlugin 接口

```rust
/// Device plugin — extends RpPlugin with GPIO ports, optional config, optional GUI.
/// 设备插件 —— 扩展 RpPlugin，增加 GPIO 端口、可选配置、可选 GUI。
pub trait DevicePlugin: RpPlugin {
    fn ports(&self) -> &[DevicePort];
    fn ports_mut(&mut self) -> &mut [DevicePort];
    fn on_gpio_change(&mut self, port_index: usize, level: bool);

    fn config_schema(&self) -> Option<ConfigSchema> { None }
    fn apply_config(&mut self, config: &serde_json::Value) -> Result<(), ConfigError> { Ok(()) }
    fn has_ui(&self) -> bool { false }
    #[cfg(feature = "gui")]
    fn render_ui(&mut self, ui: &mut egui::Ui) {}   // gui feature only
}

pub struct DevicePort {
    pub name: String,
    pub direction: PortDirection,
    pub level: bool,
    pub description: String,
}
impl DevicePort {
    pub fn input(name: impl Into<String>, description: impl Into<String>) -> Self;
    pub fn output(name: impl Into<String>, description: impl Into<String>) -> Self;
    pub fn bidirectional(name: impl Into<String>, description: impl Into<String>) -> Self;
}
pub enum PortDirection { Input, Output, Bidirectional }
```

`state_snapshot()` comes from `RpPlugin` and is the authoritative headless/MCP
interface. Device logic must work when `render_ui()` is never called.
`state_snapshot()` 来自 `RpPlugin`，是无头/MCP 模式的权威接口。设备逻辑必须在
从不调用 `render_ui()` 的情况下正常工作。

---

## `DeviceManager` / 设备管理器

```rust
pub type DeviceId = String;

pub struct DeviceManager { /* devices, wiring, loader */ }
impl DeviceManager {
    /// No arguments — devices are registered explicitly.
    /// 无参数 —— 设备通过显式注册加入。
    pub fn new() -> Self;

    /// Register an already-constructed plugin (in-process).
    /// 注册已构建的插件（进程内）。
    pub fn register(&mut self, id: DeviceId, device: Box<dyn DevicePlugin>)
        -> Result<(), DeviceError>;

    /// Load a plugin from a dynamic library (dlopen path kept alive with the device).
    /// 从动态库加载插件（库与设备一同保持存活）。
    pub fn load_plugin(&mut self, id: DeviceId, path: impl AsRef<Path>)
        -> Result<(), DeviceError>;

    /// Remove a device: calls `shutdown()` and drops its wiring entries.
    /// 移除设备：调用 shutdown() 并清除其接线。
    pub fn unregister(&mut self, id: &DeviceId) -> Result<(), DeviceError>;

    /// Wire a device port to a GPIO pin (replaces an existing wire for the port).
    /// 将设备端口连接到 GPIO 引脚（同端口已有接线会被替换）。
    pub fn wire(&mut self, device_id: &str, port_index: usize, gpio_pin: u8)
        -> Result<(), DeviceError>;

    /// Wiring table reference / 接线表引用
    pub fn wiring(&self) -> &WiringTable;

    /// Bidirectional GPIO sync (called by the host once per simulation step):
    ///   GPIO output level → wired input ports (`on_gpio_change`);
    ///   device output port level → GPIO pin input (`set_input`).
    /// 双向 GPIO 同步（宿主每个模拟步调用一次）：
    ///   GPIO 输出电平 → 已接线输入端口（on_gpio_change）；
    ///   设备输出端口电平 → GPIO 引脚输入（set_input）。
    pub fn sync_gpio(&mut self, gpio_pins: &mut [GpioPin]);

    /// Tick all devices / 推进所有设备
    pub fn tick_all(&mut self, ctx: &mut dyn PluginContext, cycles: u64);

    /// Shutdown all devices / 关闭所有设备
    pub fn shutdown_all(&mut self);

    /// MCP API: `{ "<id>": <state_snapshot()>, ... }` / MCP 查询所有设备状态
    pub fn mcp_query_all(&self) -> serde_json::Value;

    /// MCP API: one device snapshot, `None` if unknown.
    /// MCP API：单个设备状态，未注册时返回 None。
    pub fn mcp_query_device(&self, id: &str) -> Option<serde_json::Value>;

    /// MCP API: `[{id, name, has_ui, tags}]`, sorted by id.
    /// MCP API：设备清单（按 id 排序）。
    pub fn mcp_list_devices(&self) -> Vec<serde_json::Value>;

    /// MCP API: apply JSON config through `DevicePlugin::apply_config`.
    /// MCP API：通过 apply_config 应用 JSON 配置。
    pub fn apply_config(&mut self, id: &str, config: &serde_json::Value)
        -> Result<(), ConfigApplyError>;

    pub fn loader(&self) -> &PluginLoader;
    pub fn loader_mut(&mut self) -> &mut PluginLoader;
    pub fn has_devices(&self) -> bool;
    pub fn device_count(&self) -> usize;
}
impl Default for DeviceManager { /* == Self::new() */ }
```

> There is no `new(plugin_dirs, headless)` constructor and no
> `LoadedDevice{name, device_type}` / `PluginOrigin` type in the source. Devices
> are identified by the `DeviceId` key, and the library handle stays private
> inside `ManagedDevice`.
> 源码中没有 `new(plugin_dirs, headless)` 构造器，也没有
> `LoadedDevice{name, device_type}` / `PluginOrigin` 类型。设备以 `DeviceId`
> 键标识，库句柄私有保存在 `ManagedDevice` 内。

---

## `PluginLoader` / 插件加载器

```rust
pub struct PluginLoader { /* search_paths: Vec<PathBuf> */ }
impl PluginLoader {
    pub fn new() -> Self;
    pub fn add_search_path(&mut self, path: impl AsRef<Path>);
    /// Scan search paths for platform plugin files (*.dll / *.so / *.dylib).
    /// 扫描搜索路径中的平台插件文件。
    pub fn scan(&self) -> Vec<PathBuf>;
    /// Load one library (no instantiation) / 仅加载一个库（不实例化）
    pub fn load(&self, path: &Path) -> Result<SafeLibrary, DeviceError>;
    /// Load + ABI check + instantiate a device plugin.
    /// 加载、校验 ABI 并实例化设备插件。
    pub fn load_device(&self, path: &Path) -> Result<LoadedPlugin, DeviceError>;
    /// Scan + load every library found (best effort; failures logged).
    /// 扫描并加载找到的所有库（尽力而为；失败仅记录日志）。
    pub fn scan_and_load(&self) -> Vec<(PathBuf, SafeLibrary)>;
}
impl Default for PluginLoader { /* search path: "plugins" */ }

pub struct LoadedPlugin {
    pub library: SafeLibrary,        // must outlive `plugin` (vtable lives here)
    pub plugin: Box<dyn DevicePlugin>,
}
```

`load_device` requires the exports `rp2350_plugin_version` (`ABI_VERSION`
match), `rp2350_plugin_is_device` (must return 1) and
`rp2350_device_plugin_create` (non-null device factory). See
[rp2350-plugin.md](rp2350-plugin.md) for the ABI.
`load_device` 要求导出 `rp2350_plugin_version`（匹配 `ABI_VERSION`）、
`rp2350_plugin_is_device`（返回 1）与 `rp2350_device_plugin_create`（非空设备工厂）。
ABI 细节见 [rp2350-plugin.md](rp2350-plugin.md)。

---

## `WiringTable` / 接线表

```rust
pub struct WiringEntry {
    pub device_id: String,
    pub port_name: String,
    pub port_index: usize,
    pub gpio_pin: u8,
}
#[derive(Serialize, Deserialize)]
pub struct WiringTable { /* entries: Vec<WiringEntry> */ }
impl WiringTable {
    pub fn new() -> Self;
    pub fn wire(&mut self, device_id: &str, port_name: &str, port_index: usize, gpio_pin: u8);
    pub fn unwire(&mut self, device_id: &str, port_name: &str, port_index: usize);
    pub fn remove_device(&mut self, device_id: &str);
    pub fn find_by_gpio(&self, gpio_pin: u8) -> Vec<&WiringEntry>;
    pub fn find_by_device_port(&self, device_id: &str, port_index: usize) -> Option<u8>;
    pub fn all(&self) -> &[WiringEntry];
    pub fn clear(&mut self);
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
}
```

Multiple ports may share one GPIO pin; rewiring the same (device, port)
replaces its entry.
多个端口可共享同一引脚；对同一 (设备, 端口) 重新接线会替换原条目。

---

## GPIO Sync Flow / GPIO 同步流程

```
Host step (每步一次):
  DeviceManager::sync_gpio(&mut [GpioPin])
    ├─ wiring.all() → for each entry with Input|Bidirectional port:
    │     device.on_gpio_change(port_index, gpio_pin.level())
    └─ for each device output|bidirectional port:
          wiring.find_by_device_port() → gpio_pin.set_input(port.level)

Device tick (设备推进):
  DeviceManager::tick_all(ctx, cycles) → plugin.tick(ctx, cycles) per device
```

---

## Errors / 错误类型

```rust
pub enum DeviceError {
    NotFound(String),                 // device not found / 设备未找到
    AlreadyExists(String),            // duplicate id / 重复 ID
    InitFailed(String),
    InvalidWiring(String),
    PluginLoadFailed(String),         // missing symbol / ABI mismatch / ...
    PortIndexOutOfBounds(usize),
}
pub enum ConfigError { MissingField(String), InvalidType { field, expected, actual }, OutOfRange { field, message } }
pub enum ConfigApplyError {
    DeviceNotFound(String),                  // manager-level: unknown id / 未知设备
    Invalid(ConfigError),                    // plugin rejected config / 插件拒绝配置
}
```

`apply_config` 由 `DeviceManager` 负责查找设备（`DeviceNotFound`），再调用
`DevicePlugin::apply_config` 并把 `ConfigError` 包装为 `Invalid`。
`apply_config` resolves the device in the manager (`DeviceNotFound`) and then
delegates to `DevicePlugin::apply_config`, wrapping `ConfigError` as `Invalid`.

---

## Usage Example / 使用示例

```rust
use rp2350_virt_periph::{DeviceManager, DevicePort};

let mut dm = DeviceManager::new();
dm.register("led-1".into(), Box::new(LedPlugin::new()))?;
dm.wire("led-1", 0, 25)?;                      // port 0 → GPIO 25

dm.sync_gpio(&mut chip.gpio_pins);             // every simulation step
dm.tick_all(&mut ctx, cycles);

let states = dm.mcp_query_all();               // {"led-1": {"level": true}}
dm.unregister("led-1")?;                       // shutdown + unwire
```

---

## 规划中 / Not yet implemented

- ⬜ `DeviceManager` 层的自动扫描与一键加载（`scan_and_load` 目前只在
  `PluginLoader` 上，且不实例化设备）；目录约定 `plugins/`、
  `plugins/third-party/` 及 first-party/third-party 来源标记。
- ⬜ 接线 JSON 持久化（`WiringTable` 已可序列化，但没有保存/加载文件入口）。
- ⬜ CLI `wire` / `list-devices` / `mcp-serve` 子命令仍为桩（见
  [rp2350-cli.md](rp2350-cli.md)）。
- ⬜ Manager-level auto scan-and-load, wiring-file persistence, and the CLI
  wiring subcommands are planned; the `PluginLoader` scan/loading and in-process
  registration above are what exists today.
