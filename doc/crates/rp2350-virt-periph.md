# rp2350-virt-periph — Public API / 公共 API

> Virtual Device Manager: lifecycle, GPIO wiring, tick scheduling and MCP state
> query for virtual device plugins. Does **NOT** contain device implementations.
> 虚拟设备管理器：虚拟外设插件的生命周期、GPIO 接线、tick 调度与 MCP 状态查询。
> **不包含**设备实现 —— 设备本体位于插件中。
>
> **Status**: 已实现 / implemented — `DeviceManager`（无参构造）、`register` /
> `load_plugin` / `scan_and_load` / `reload` / `unregister` / `wire` / `sync_gpio` /
> `tick_all` / `shutdown_all` / `mcp_query_all` / `mcp_query_device` /
> `mcp_list_devices` / `apply_config` / `devices`、`PluginLoader`（目录扫描 +
> ABI 校验 + 损坏输入诊断）、`ReloadReport`、`WiringTable`、`DevicePlugin`/`DevicePort`、
> `ConfigSchema`。见文末“规划中”。

---

## Module Structure / 模块结构

| Module | Description / 描述 |
|--------|-------------------|
| `manager` | `DeviceManager` — central orchestration |
| `loader` | `PluginLoader` — directory scan, input validation, ABI check, `LoadedPlugin` |
| `reload` | `Fingerprint`, `ReloadReport` — hot reload / 热重载 |
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

    /// Scan a directory, instantiate every device plugin and register it.
    /// ID = `metadata().id`, falling back to the library file stem; duplicate
    /// IDs and load failures are skipped with `log::warn!`.
    /// Returns `(device_id, library_path)` for each registered device.
    /// 扫描目录、实例化并注册其中所有设备插件。ID 取 `metadata().id`，为空时
    /// 退回库文件名主干；重复 ID 与加载失败仅 `log::warn!` 后跳过。
    /// 返回每个已注册设备的 `(设备 ID, 库路径)`。
    pub fn scan_and_load(&mut self, dir: impl AsRef<Path>) -> Vec<(DeviceId, PathBuf)>;

    /// One synchronous hot-reload pass over `dir`.
    /// 对 `dir` 执行一次同步热重载。
    ///
    /// Re-scans the directory, compares fingerprints, and classifies every
    /// change into `added` / `removed` / `reloaded` / `rewired` / `unwired` /
    /// `failed` (see “错误处理与重载语义”).
    /// 重新扫描目录、比较指纹，并把每项变化归类到 `added` / `removed` /
    /// `reloaded` / `rewired` / `unwired` / `failed`（见“错误处理与重载语义”）。
    pub fn reload(&mut self, dir: &Path) -> ReloadReport;

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

    /// MCP API: `[{id, name, has_ui, tags, ports}]`, sorted by id.
    /// `ports` 按接线顺序（即 `wire` 使用的索引顺序）列出端口名。
    /// MCP API：设备清单（按 id 排序），`ports` 为接线顺序的端口名。
    pub fn mcp_list_devices(&self) -> Vec<serde_json::Value>;

    /// MCP API: apply JSON config through `DevicePlugin::apply_config`.
    /// MCP API：通过 apply_config 应用 JSON 配置。
    pub fn apply_config(&mut self, id: &str, config: &serde_json::Value)
        -> Result<(), ConfigApplyError>;

    pub fn loader(&self) -> &PluginLoader;
    pub fn loader_mut(&mut self) -> &mut PluginLoader;
    pub fn has_devices(&self) -> bool;
    pub fn device_count(&self) -> usize;
    /// Iterate `(id, plugin)` over every registered device (unordered).
    /// 遍历所有已注册设备 `(id, plugin)`（顺序不确定）。
    pub fn devices(&self) -> impl Iterator<Item = (&DeviceId, &dyn DevicePlugin)> + '_;
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
    /// Platform plugin library extensions / 平台插件库扩展名
    /// (Windows `dll`; Linux `so`; macOS `dylib`+`so`).
    pub fn platform_extensions() -> &'static [&'static str];
    /// Scan one directory for platform plugin files, sorted by path.
    /// 扫描单个目录中的平台插件文件，结果按路径排序。
    pub fn scan_dir(dir: impl AsRef<Path>) -> Vec<PathBuf>;
    /// Scan search paths for platform plugin files (*.dll / *.so / *.dylib).
    /// 扫描搜索路径中的平台插件文件。
    pub fn scan(&self) -> Vec<PathBuf>;
    /// Load one library (no instantiation) / 仅加载一个库（不实例化）
    ///
    /// Validates first: path must exist, be a regular file and be non-empty.
    /// 先校验：路径必须存在、是普通文件且非空。
    pub fn load(&self, path: &Path) -> Result<SafeLibrary, DeviceError>;
    /// Load + input validation + ABI check + instantiate a device plugin.
    /// 加载、校验输入、检查 ABI 并实例化设备插件。
    pub fn load_device(&self, path: &Path) -> Result<LoadedPlugin, DeviceError>;
    /// Scan + load every library found (best effort; failures logged, no
    /// instantiation — use `DeviceManager::scan_and_load` for devices).
    /// 扫描并加载找到的所有库（尽力而为；失败记录日志；**不实例化设备** ——
    /// 设备级加载请用 `DeviceManager::scan_and_load`）。
    pub fn scan_and_load(&self) -> Vec<(PathBuf, SafeLibrary)>;
}
impl Default for PluginLoader { /* search path: "plugins" */ }

pub struct LoadedPlugin {
    pub plugin: Box<dyn DevicePlugin>, // dropped first / 先释放
    pub library: SafeLibrary,          // vtable lives here / vtable 位于此库内
}
```

> Field order matters: `plugin` is declared first so Rust drops the trait object
> **before** unloading the library that owns its vtable.
> 字段顺序很重要：`plugin` 声明在前，Rust 会先释放 trait 对象，再卸载拥有其
> vtable 的库。

`load_device` requires the exports `rp2350_plugin_version` (`ABI_VERSION`
match), `rp2350_plugin_is_device` (must return 1) and
`rp2350_device_plugin_create` (non-null device factory). See
[rp2350-plugin.md](rp2350-plugin.md) for the ABI.
`load_device` 要求导出 `rp2350_plugin_version`（匹配 `ABI_VERSION`）、
`rp2350_plugin_is_device`（返回 1）与 `rp2350_device_plugin_create`（非空设备工厂）。
ABI 细节见 [rp2350-plugin.md](rp2350-plugin.md)。

> Every rejected input produces `DeviceError::PluginLoadFailed` with the path and
> a bilingual reason; a failed load creates no device and leaks no library
> handle. See “错误处理与重载语义” for the full classification table.
> 每种被拒绝的输入都会返回带路径与双语原因的
> `DeviceError::PluginLoadFailed`；加载失败不会创建设备，也不会泄漏库句柄。
> 完整分类表见“错误处理与重载语义”。

### Directory Scan Flow / 目录扫描流程

```
DeviceManager::scan_and_load(dir)
  ├─ PluginLoader::scan_dir(dir)          # platform extensions, sorted by path
  └─ for each library path:
       loader.load_device(path)
         ├─ Ok(LoadedPlugin) → id = metadata().id || file stem
         │     ├─ id already registered → log::warn! + skip (library closed)
         │     └─ else insert ManagedDevice { plugin, library, source_path }
         └─ Err(e) → log::warn!("Failed to load device plugin …")
```

- Enumeration always uses the explicit `dir` argument (`PluginLoader::scan_dir`,
  path-sorted); the `PluginLoader` configured through
  `DeviceManager::loader()` is only used for `load_device`, and its search paths
  are never mutated. 枚举始终使用显式 `dir` 参数（`PluginLoader::scan_dir`，按路径
  排序）；通过 `DeviceManager::loader()` 配置的搜索路径只用于
  `load_device`，不会被改动。
- Load order is deterministic (path-sorted), and one broken library never aborts
  the scan — it is logged and skipped.
  加载顺序确定（按路径排序），单个坏库不会中断扫描 —— 仅记录日志后跳过。
- The `SafeLibrary` stays inside `ManagedDevice` (declared after the plugin box)
  and is only closed when the device is dropped or unregistered.
  `SafeLibrary` 保存在 `ManagedDevice` 内（声明在插件 box 之后），仅在设备被
  释放或注销时关闭。
- No persistence: a CLI process releases every device on exit. See
  [rp2350-cli.md](rp2350-cli.md) for the one-shot `list-devices` / `wire`
  subcommands built on this method.
  无持久化：CLI 进程退出即释放所有设备。基于本方法的一次性 `list-devices` /
  `wire` 子命令见 [rp2350-cli.md](rp2350-cli.md)。

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
    /// Clone all entries of one device (hot-reload snapshot).
    /// 克隆某个设备的全部条目（热重载快照）。
    pub fn entries_for_device(&self, device_id: &str) -> Vec<WiringEntry>;
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

## Error Handling and Reload Semantics / 错误处理与重载语义

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReloadReport {
    pub added: Vec<String>,                 // [reload] + <path>
    pub removed: Vec<String>,               // [reload] - <path>
    pub reloaded: Vec<String>,              // [reload] ~ <path>
    pub rewired: Vec<String>,               // [reload] = <path>  (wiring kept)
    pub unwired: Vec<(String, String)>,     // [reload] x <path>: <reason>
    pub failed: Vec<(String, String)>,      // [reload] ! <path>: <error>
}
impl ReloadReport {
    pub fn is_empty(&self) -> bool;
    pub fn change_count(&self) -> usize;
    pub fn render_lines(&self) -> Vec<String>;  // entries + summary line
}
```

### Load error classification / 加载错误分类

Every **library** failure below returns `DeviceError::PluginLoadFailed`. The
rendered text is `Plugin load failed: <path>: <reason> / 插件加载失败: <path>: <reason>`;
the path is always present and the reason is bilingual (the OS loader's own
sentence is kept verbatim inside it). The duplicate-ID row is a manager-level
`DeviceError::AlreadyExists` raised before the library is opened.
以下每种**库**失败都返回 `DeviceError::PluginLoadFailed`。渲染文本为
`Plugin load failed: <路径>: <原因> / 插件加载失败: <路径>: <原因>`；路径始终存在，
原因中英双语（操作系统加载器自身的句子原样保留在其中）。重复 ID 一行是管理器级
`DeviceError::AlreadyExists`，在打开库之前返回。

| Input / 输入 | Detection / 检测 | Reason (English / 中文) |
|--------------|------------------|--------------------------|
| Missing path / 路径不存在 | `fs::metadata` | cannot access plugin file / 无法访问插件文件 |
| Directory or special file / 目录或特殊文件 | `metadata.is_file()` | not a regular file; directories cannot be plugins / 不是普通文件；目录不能作为插件加载 |
| Empty file / 空文件 | `metadata.len() == 0` | empty library file (0 bytes) / 空库文件（0 字节） |
| Random bytes / 随机字节 | OS loader | failed to open dynamic library / 打开动态库失败 |
| Truncated ELF/PE / 截断的 ELF/PE | OS loader | failed to open dynamic library / 打开动态库失败 |
| Non-`cdylib` ELF (`ET_REL`) / 非 cdylib ELF | OS loader | failed to open dynamic library / 打开动态库失败 |
| Missing `rp2350_plugin_version` | `find_symbol` | missing ABI symbol / 缺少 ABI 符号 |
| ABI mismatch / ABI 不匹配 | `version()` | incompatible plugin ABI: expected 1, got N / 插件 ABI 不兼容：期望 1，实际 N |
| Missing `rp2350_plugin_is_device` | `find_symbol` | missing ABI symbol / 缺少 ABI 符号 |
| `is_device() != 1` | call | plugin is not a device plugin / 插件不是设备插件 |
| Missing `rp2350_device_plugin_create` | `find_symbol` | missing ABI symbol / 缺少 ABI 符号 |
| Factory returns null / 工厂返回空指针 | pointer check | plugin factory returned a null pointer / 插件工厂返回空指针 |
| Duplicate registry ID / 重复注册 ID | manager pre-check | `DeviceError::AlreadyExists` (the library is not touched) / 不会触碰库 |

Guarantees / 保证：

- **No panic**: every rejected input becomes `Err`; the unsafe symbol lookups are
  guarded by `find_symbol` before any call, and every `unsafe` block carries a
  `// SAFETY:` note. 所有被拒绝的输入都变成 `Err`；`unsafe` 符号读取先经
  `find_symbol` 校验存在性，所有 `unsafe` 块都有 `// SAFETY:` 说明。
- **No leak**: on any failure the library handle is dropped and no device is
  created. 任何失败路径都会释放库句柄且不创建设备实例。
- **No stale vtable**: `LoadedPlugin`/`ManagedDevice` declare the plugin before
  the library so the trait object is destroyed while its vtable is mapped.
  `LoadedPlugin`/`ManagedDevice` 都先声明插件、后声明库，trait 对象在 vtable
  仍被映射时销毁。
- `DeviceManager::load_plugin(id, path)` uses the host-supplied ID and ignores
  `metadata().id`. The same path under two IDs yields two independent instances;
  the same ID twice returns `AlreadyExists` before the library is loaded.
  `load_plugin` 使用宿主给出的 ID，不使用 `metadata().id`。同一路径两个 ID 得到
  两个独立实例；同一 ID 两次在加载库之前返回 `AlreadyExists`。
- `scan_and_load` is best effort and idempotent: a broken library or a duplicate
  ID is logged and skipped, and re-scanning an unchanged directory registers
  nothing new.
  `scan_and_load` 是尽力而为且幂等的：坏库或重复 ID 记录日志后跳过；对未变化
  目录重复扫描不会新增注册。

### Reload policy / 重载策略

`DeviceManager::reload(dir)` is a **synchronous** rescan of `dir`; it never
panics and always returns a `ReloadReport`.
`DeviceManager::reload(dir)` 是**同步**重扫；绝不 panic，始终返回 `ReloadReport`。

| Situation / 情况 | Devices / 设备 | Wiring / 接线 | Report / 报告 |
|------------------|----------------|---------------|----------------|
| File unchanged / 文件未变 | kept / 保留 | kept / 保留 | empty / 空 |
| File deleted / 文件被删除 | shutdown + dropped / 关闭并释放 | removed / 清除 | `removed` |
| Replaced, same ABI & port table / 替换且 ABI/端口表一致 | new instance / 新实例 | re-applied to the same port indices / 按原端口索引重新应用 | `reloaded` + `rewired` |
| Replaced, port index/name changed / 端口索引或名称变化 | new instance / 新实例 | all entries dropped, reason recorded / 全部清除并记录原因 | `reloaded` + `unwired` |
| Replaced, device ID changed / 设备 ID 变化 | new instance / 新实例 | all entries dropped, reason recorded / 全部清除并记录原因 | `reloaded` + `unwired` |
| Replacement fails to load / 替换加载失败 | old instance already gone / 旧实例已释放 | removed / 清除 | `failed` |
| New library appears / 出现新库 | loaded + registered / 加载并注册 | — | `added` or `failed` |
| Library still invalid / 库仍然损坏 | unchanged / 不变 | unchanged / 不变 | `failed` (retried every pass) / 每次重试 |

Consistency invariant / 一致性不变量：after every `reload`, each entry returned by
`wiring().all()` points at an existing device returned by `devices()`, with
`port_index < ports().len()` and a matching `port_name`. Entries are only
re-applied after that check succeeds; otherwise the whole device wiring is
dropped and reported in `unwired`.
每次 `reload` 后，`wiring().all()` 的每个条目都指向 `devices()` 中真实存在的设备，
满足 `port_index < ports().len()` 且 `port_name` 一致；只有校验通过才会重新应用，
否则清除该设备的全部接线并记录到 `unwired`。

Devices registered in-process (`register`) or whose source path is outside `dir`
are never touched. Devices are always shut down **before** their library is
unloaded (mandatory on Windows, where loaded DLLs are locked).
进程内注册（`register`）或源路径位于 `dir` 之外的设备不受影响；设备总是在库卸载
**之前**关闭（Windows 上已加载 DLL 会被锁定，必须如此）。

### Known limitations / 已知限制

- **Panics cannot be caught across the `cdylib` boundary.** If a plugin panics in
  `metadata()`, `tick()`, `state_snapshot()`, the factory or any other export,
  the unwind crosses an `extern "C"` frame and aborts the process. The loader can
  reject bad inputs, but it cannot make a panicking plugin safe — **plugins must
  not panic**; return `PluginError` instead.
  **跨 `cdylib` 边界的 panic 无法捕获。** 插件若在 `metadata()`、`tick()`、
  `state_snapshot()`、工厂或其它导出中 panic，展开会穿过 `extern "C"` 帧并终止
  进程。加载器能拒绝坏输入，但不能让会 panic 的插件变安全 —— **插件自身不得
  panic**，错误应通过 `PluginError` 返回。
- The ABI check is version number + symbol presence only. Two libraries built by
  different compilers with the same `ABI_VERSION` can have incompatible trait
  object layouts; the host cannot detect that. Build host and plugins with the
  same toolchain. ABI 校验只看版本号与符号存在性；不同编译器、相同
  `ABI_VERSION` 的 trait 对象布局可能不兼容，宿主无法检测。主机与插件必须使用
  同一工具链。
- A factory that returns a non-null but invalid pointer is trusted by the host
  (`Box::from_raw`), so calling into it is undefined behaviour. This is inherent
  to the raw-pointer ABI; treat plugins as trusted code.
  工厂返回非空但无效的指针时宿主会信任它（`Box::from_raw`），后续调用即未定义
  行为。这是裸指针 ABI 的固有限制；插件应视为可信代码。
- Loading a library executes its platform initializers before any ABI check; a
  malicious library can run code at `dlopen`/`LoadLibrary` time.
  加载库会在任何 ABI 校验之前执行其平台初始化代码；恶意库可在
  `dlopen`/`LoadLibrary` 阶段运行代码。
- The OS loader's error sentence is platform/locale dependent; only the
  surrounding bilingual framing is stable.
  操作系统加载器的错误句子随平台/语言变化；只有外层双语框架是稳定的。
- Change detection uses length + modification time. A same-length overwrite
  within the filesystem timestamp granularity can be missed; touch the file or
  re-register to force a reload.
  变更检测使用长度 + 修改时间；在文件系统时间戳粒度内等长覆盖可能漏检，可 touch
  文件或重新注册以强制重载。
- Reload tracks devices **per file**. When the same library path was loaded under
  several host IDs with `load_plugin`, the lexicographically smallest ID is
  tracked (deterministic) — unregister the duplicates before relying on hot
  reload. On Windows a DLL path maps to one code module, so a second handle would
  keep returning the old code anyway.
  热重载**按文件**跟踪设备。同一库路径被 `load_plugin` 以多个宿主 ID 加载时，只
  跟踪字典序最小的 ID（确定性）——依赖热重载前请先注销多余实例。Windows 上同一
  DLL 路径只映射一份代码模块，第二个句柄本来也会继续指向旧代码。
- On Windows a loaded DLL is locked by the OS: deleting/replacing it while a
  process has it loaded requires the manager to unload first, which `reload`
  does; an external process still holding the file open makes the reload fail
  with `failed` rather than crashing.
  Windows 上已加载 DLL 被系统锁定：在其被加载时删除/替换需要先卸载，`reload`
  已这样做；外部进程若仍持有该文件，重载会归类为 `failed` 而不是崩溃。

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

- ⬜ 目录约定扩展：`plugins/third-party/` 及 first-party/third-party 来源标记；
  当前 `scan_and_load` 只扫描单个显式目录（按平台扩展名）。
  Source tagging (first-party vs third-party) and nested `plugins/third-party/`
  discovery are not implemented; `scan_and_load` scans one explicit directory.
- ⬜ 接线 JSON 持久化（`WiringTable` 已可序列化，但没有保存/加载文件入口）。
  Wiring persistence — the table is serializable but no file I/O entry point.
- ⬜ 长期设备守护进程（`reload` 已实现，但没有跨进程设备守护或状态持久化）。
  No long-lived device daemon: hot reload exists, but device state is not
  persisted across processes.
- ⬜ `mcp-serve` 子命令仍为桩（`mcp_query_all` / `mcp_query_device` /
  `mcp_list_devices` 已可用于未来的 MCP 服务器）。
  `mcp-serve` remains a stub; the MCP query API is ready for a future server.
