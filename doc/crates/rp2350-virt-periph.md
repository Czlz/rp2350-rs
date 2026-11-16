# rp2350-virt-periph — Public API / 公共 API

> Virtual Device Manager: lifecycle, GPIO wiring, tick scheduling and MCP state
> query for virtual device plugins. Does **NOT** contain device implementations.
> 虚拟设备管理器：虚拟外设插件的生命周期、GPIO 接线、tick 调度与 MCP 状态查询。
> **不包含**设备实现 —— 设备本体位于插件中。
>
> **Status**: 已实现 / implemented — `DeviceManager`（无参构造 / `with_latency` /
> `with_executor`）、`register` / `load_plugin` / `scan_and_load` / `reload` /
> `unregister` / `wire` / `sync_gpio` / `tick_all` / `shutdown_all` /
> `step` / `step_gpio` / `shutdown_threads` / `mcp_query_all` /
> `mcp_query_device` / `mcp_list_devices` / `ports_of` / `apply_config`、
> `PluginLoader`（目录扫描 + ABI 校验 + 损坏输入诊断）、`ReloadReport`、
> `WiringTable`、`DevicePlugin`/`DevicePort`、`ConfigSchema`、
> `PinSnapshot`/`PortBinding`/`ExecutorConfig`/`PublishSchedule`/`DeviceExecutor`
> 与固定延迟插件线程（MT-5）。见文末“规划中”。

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
| `executor` | `PinSnapshot`, `PortBinding`, `ExecutorConfig`, `PublishSchedule`, `StepReport`, `ExecutorError` |
| `executor_worker` | `Command`/`Response` mailbox, `WorkerContext`, worker loop (private side) |
| `executor_host` | `DeviceExecutor` — per-device worker handle, timeout, panic latch |
| `executor_device` | per-device inline/threaded host (`DeviceHost`) |
| `executor_manager` | `DeviceManager::step` / `step_gpio` / `shutdown_threads` |

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

In threaded mode the same call is served by the device worker
(`Command::QueryState`), and a degraded device answers with
`{"error": "...", "degraded": true}`.
线程化模式下该调用由设备工作线程服务（`Command::QueryState`）；已降级设备返回
`{"error": "...", "degraded": true}`。

---

## `DeviceManager` / 设备管理器

```rust
pub type DeviceId = String;

pub struct DeviceManager { /* devices, wiring, loader, executor, schedule */ }
impl DeviceManager {
    /// No arguments — devices are registered explicitly (synchronous default).
    /// 无参数 —— 设备通过显式注册加入（同步默认路径）。
    pub fn new() -> Self;

    /// Synchronous manager with a custom fixed propagation delay (epochs).
    /// 同步管理器，可指定固定传播延迟（epoch）。
    pub fn with_latency(latency_ticks: u64) -> Self;

    /// Opt-in threading: every device gets its own worker thread.
    /// 可选多线程：每台设备使用独立工作线程。
    pub fn with_executor(config: ExecutorConfig) -> Self;

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

    /// Tick all devices (synchronous path only) / 推进所有设备（仅同步路径）
    pub fn tick_all(&mut self, ctx: &mut dyn PluginContext, cycles: u64);

    /// Shutdown all devices / 关闭所有设备
    pub fn shutdown_all(&mut self);

    /// One fixed-delay epoch: dispatch to every device, collect outputs.
    /// 固定延迟的一个 epoch：向所有设备派发并收集输出。
    pub fn step(&mut self, epoch: u64, inputs: PinSnapshot, cycles: u64)
        -> Result<StepReport, StepError>;

    /// `step` plus GPIO sampling and masked write-back / `step` + GPIO 采样与掩码写回
    pub fn step_gpio(&mut self, epoch: u64, pins: &mut [GpioPin], cycles: u64)
        -> Result<StepReport, StepError>;

    /// Stop and join every plugin worker (threaded mode only).
    /// 停止并回收所有插件工作线程（仅线程化模式）。
    pub fn shutdown_threads(&mut self) -> usize;

    /// MCP API: `{ "<id>": <state_snapshot()>, ... }` / MCP 查询所有设备状态
    pub fn mcp_query_all(&mut self) -> serde_json::Value;

    /// MCP API: one device snapshot, `None` if unknown.
    /// MCP API：单个设备状态，未注册时返回 None。
    pub fn mcp_query_device(&mut self, id: &str) -> Option<serde_json::Value>;

    /// MCP API: `[{id, name, has_ui, tags, ports}]`, sorted by id.
    /// `ports` 按接线顺序（即 `wire` 使用的索引顺序）列出端口名。
    /// MCP API：设备清单（按 id 排序），`ports` 为接线顺序的端口名。
    pub fn mcp_list_devices(&self) -> Vec<serde_json::Value>;

    /// Port table of one device (inline or worker-hosted).
    /// 单个设备的端口表（内联或工作线程宿主）。
    pub fn ports_of(&self, id: &str) -> Option<&[DevicePort]>;

    /// MCP API: apply JSON config through `DevicePlugin::apply_config`.
    /// MCP API：通过 apply_config 应用 JSON 配置。
    pub fn apply_config(&mut self, id: &str, config: &serde_json::Value)
        -> Result<(), ConfigApplyError>;

    pub fn loader(&self) -> &PluginLoader;
    pub fn loader_mut(&mut self) -> &mut PluginLoader;
    pub fn has_devices(&self) -> bool;
    pub fn device_count(&self) -> usize;

    // Threaded-mode introspection / 线程化模式自检
    pub fn executor_config(&self) -> Option<ExecutorConfig>;
    pub fn latency_ticks(&self) -> u64;
    pub fn thread_count(&self) -> usize;
    pub fn degraded_devices(&self) -> Vec<DeviceId>;
    pub fn degraded_reasons(&self) -> Vec<(DeviceId, String)>;
    pub fn set_step_timeout(&mut self, id: &str, timeout: Duration) -> bool;
    pub fn driven_pins(&self) -> PinSnapshot;
}
impl Default for DeviceManager { /* == Self::new() */ }
```

> `mcp_query_all` / `mcp_query_device` take `&mut self` because worker-hosted
> devices answer through their mailbox. `devices()` (an iterator over
> `&dyn DevicePlugin`) no longer exists: worker-hosted plugins are owned by
> their thread, so callers use `ports_of` / `mcp_list_devices` / `mcp_query_*`.
> `mcp_query_all` / `mcp_query_device` 使用 `&mut self`，因为工作线程设备通过邮箱
> 应答。`devices()`（返回 `&dyn DevicePlugin` 迭代器）已移除：工作线程独占插件，
> 调用方改用 `ports_of` / `mcp_list_devices` / `mcp_query_*`。

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
Synchronous path (default, unchanged):
  DeviceManager::sync_gpio(&mut [GpioPin])
    ├─ wiring.all() → for each entry with Input|Bidirectional port:
    │     device.on_gpio_change(port_index, gpio_pin.level())
    └─ for each device output|bidirectional port:
          wiring.find_by_device_port() → gpio_pin.set_input(port.level)

  DeviceManager::tick_all(ctx, cycles) → plugin.tick(ctx, cycles) per device

Fixed-delay path (`step`, works in both modes):
  DeviceManager::step(epoch, PinSnapshot, cycles)
    ├─ PublishSchedule::take_effective(epoch)   # outputs produced at epoch-latency
    ├─ phase 1: for every device (ID order):
    │     inline  → on_gpio_change* → tick → collect outputs (immediately)
    │     threaded→ Command::Step sent, no wait (workers run in parallel)
    ├─ phase 2: collect threaded responses in ID order (per-device timeout)
    ├─ combine per-device outputs with wired-OR
    └─ PublishSchedule::publish(epoch, combined) # effective at epoch+latency
```

\* Inputs are delivered to every wired input/bidirectional port on every step,
with no merging, dropping or edge filtering.

---

## Threaded Executors / 线程化执行器（MT-5）

### Switch / 开关

- `DeviceManager::new()` — **synchronous default**, unchanged semantics, no
  threads; `sync_gpio` / `tick_all` remain the legacy drive path.
- `DeviceManager::with_executor(ExecutorConfig)` — each registered / loaded
  device is moved to **its own** worker thread. `register`, `load_plugin`,
  `scan_and_load`, `reload` and `unregister` all honour the mode.
- `ExecutorConfig { latency_ticks, step_timeout }`: propagation delay in
  simulation epochs (default 1 peripheral tick, clamped to ≥ 1) and the upper
  bound of one mailbox round trip (default **5 s**, `DEFAULT_STEP_TIMEOUT`).
  The timeout is a **stall bound**, not a scheduling knob: only a worker that
  genuinely hangs may hit it. Tests that verify degradation pass a short
  timeout explicitly (`ExecutorConfig::new(latency, Duration::from_millis(…))`);
  a normal `cargo test` / concurrent build must never latch a healthy device.

`DeviceManager::new()` 是**同步默认路径**（语义不变、无线程，`sync_gpio` /
`tick_all` 保留）；`with_executor` 让每台设备运行在**各自**工作线程上，注册、加载、
扫描、热重载、注销都遵循该模式。`ExecutorConfig` 指定传播延迟（仿真 epoch，默认 1
个外设 tick，最小 1）与单次邮箱往返上限（默认 **5 s**，`DEFAULT_STEP_TIMEOUT`）。
该超时是**卡死上限**而非调度参数：只有真正卡死的工作线程才应触发；需要验证降级的
测试显式传入短超时（`ExecutorConfig::new(latency, Duration::from_millis(…))`）。
正常 `cargo test` 或并行构建不得把健康设备误判为高阻。

`set_step_timeout(id, timeout)` overrides the bound of **one** worker-hosted
device (inline devices and unknown IDs return `false`), so a known-slow device
can be tightened without giving every healthy device the same tiny budget. It
takes effect on the next command; a device already high-Z stays high-Z until
`reload`. `set_step_timeout(id, timeout)` 覆盖**单台**工作线程设备的卡死上限
（内联设备与未知 ID 返回 `false`），可只收紧已知较慢的设备而不牵连健康设备；从
下一条命令生效，已高阻的设备在 `reload` 前保持高阻。

### Mailbox / 邮箱协议

```rust
pub enum Command {
    Step { epoch: u64, inputs: PinSnapshot, cycles: u64 },
    Configure { config: serde_json::Value },
    Reload { plugin: Box<dyn DevicePlugin>, bindings: Vec<PortBinding> },
    SetBindings { bindings: Vec<PortBinding> },
    QueryState,
    Shutdown,
}
pub enum Response {
    Stepped { epoch: u64, outputs: PinSnapshot },
    Ack,
    Reloaded { previous: Box<dyn DevicePlugin> },
    State { value: serde_json::Value },
    Shutdown { plugin: Box<dyn DevicePlugin> },
    Failed { epoch: Option<u64>, message: String },
}
```

Every command is a request with exactly one response, so the host can bound the
wait. The worker owns the plugin box: no host thread calls into a plugin while
it runs on a worker. `WorkerContext` supplies `current_cycles() == epoch` and
records (does not dispatch) `schedule_callback` requests.

每条命令都是“一次请求、恰好一个响应”，宿主因此可以有界等待。工作线程独占插件
box：插件在工作线程运行时，宿主线程不再调用它。`WorkerContext` 让
`current_cycles()` 等于该 epoch，并**记录**（不派发）`schedule_callback`。

### Delay semantics / 延迟语义

- `epoch` input snapshot → device → outputs published at
  `epoch + latency_ticks`; they appear in `StepReport::effective` and on GPIO at
  that epoch. The delay lives in `PublishSchedule` on the simulation thread and
  is counted in epochs, never in host time.
- `StepReport.produced` holds **one entry per registered device**, in ID order.
  A healthy device carries its output of the current epoch; a device that timed
  out, panicked or was already stopped carries `PinSnapshot::empty` (high-Z:
  it drives no pin) and is also listed in `StepReport.degraded`. A degraded
  device therefore never disappears silently from the report, and `effective`
  only ever contains the wired-OR of healthy, actively driven pins.
  `StepReport.effective` holds the wired-OR of outputs that matured now. Skipped
  epochs are released together (wired-OR), so no input is merged and no output
  is lost.
- Determinism: identical input sequences produce identical per-pin output
  sequences on the synchronous and threaded paths (tests compare them at
  latency 1 and 3).

`epoch` 的输入快照 → 设备 → 输出在 `epoch + latency_ticks` 发布，并在该 epoch
通过 `StepReport::effective` 与 GPIO 生效；延迟在模拟线程的 `PublishSchedule` 中
以 epoch 计量，与宿主时间无关。`produced` 为**每台已注册设备**按 ID 顺序保留一个
条目：健康设备携带当前 epoch 的输出；超时、panic 或已停止的设备携带
`PinSnapshot::empty`（高阻：不驱动任何引脚），并同时列入 `degraded`。因此降级
设备不会从报告里静默消失，`effective` 只包含健康设备实际驱动的引脚线或。跳过的
epoch 会一起释放（线或），因此不合并输入、不丢失输出。确定性：同一输入序列在同步
与线程化路径产生逐 pin 相同的输出序列（测试在延迟 1 与 3 下对照）。

### Failure handling / 失败降级

| Situation / 情况 | Behaviour / 行为 |
|------------------|------------------|
| Worker exceeds `step_timeout` (default **5 s**, `DEFAULT_STEP_TIMEOUT`) | device latched **high-Z**, listed in `StepReport::degraded` and kept in `produced` with an empty (no-drive) snapshot, stable diagnostic line; simulation continues / 设备锁定为**高阻**，列入 `degraded` 并在 `produced` 中保留空（不驱动）快照，诊断行稳定，仿真继续 |
| Plugin panics in `tick` / `apply_config` / `state_snapshot` | caught per command, worker poisoned, device high-Z; other devices unaffected / 按命令捕获，线程中毒、设备高阻，其它设备不受影响 |
| Plugin panics in `shutdown` | ignored, the plugin box is still returned / 忽略，仍交还插件 box |
| `shutdown_threads` / manager drop | sends `Shutdown`, joins with a bounded deadline, drops the plugin **before** the library / 发送 `Shutdown`，限期 join，插件先于库释放 |
| Worker never answers / 工作线程永不响应 | detached (`DeviceExecutor::is_detached`), library handle intentionally leaked so the vtable stays mapped / 分离，库句柄有意泄漏以保持 vtable 映射 |

The default five-second bound is a **stall detector**: parallel `cargo test`,
concurrent builds or a busy host may delay a worker by tens of milliseconds but
must never latch a healthy device high-Z. Degradation tests opt into a short
timeout explicitly. A high-Z device keeps its `produced` entry with an empty
snapshot, so `produced` always has one entry per registered device and callers
can tell "no output" (0 bits) from "device missing".
默认 5 秒上限是**卡死检测**：并行 `cargo test`、并发构建或繁忙宿主可能让工作线程
延迟数十毫秒，但绝不能把健康设备误锁为高阻。降级测试显式选择短超时。高阻设备在
`produced` 中保留空快照，因此 `produced` 始终为每台已注册设备保留一个条目，调用方
能区分「无输出（0 位）」与「设备缺失」。

`catch_unwind` only covers Rust-level unwinding. A `panic = "abort"` plugin, or
an unwind crossing a foreign (`extern "C"`) frame, aborts the process and cannot
be isolated — this is the pre-existing cdylib limitation.
`catch_unwind` 只覆盖 Rust 级展开；`panic = "abort"` 的插件，或展开穿过外部
（`extern "C"`）帧时仍会终止进程 —— 这是既有的 cdylib 限制。

Diagnostics / 诊断：`thread_count()`、`degraded_devices()`、
`degraded_reasons()`（每次查询内容一致）、`DeviceExecutor::timeouts()`。

### Reload / unregister in threaded mode / 线程化模式下的重载与注销

`reload` remains synchronous: the old worker is stopped and joined, the plugin
box is dropped, then the library is unloaded and the replacement loaded and
spawned on a fresh worker. Binding tables are rebuilt after `wire` / reload, so
wiring stays consistent. `unregister` stops and joins exactly one worker before
returning; afterwards `thread_count()` is immediately accurate, the wiring entry
is gone, and the next `step` never dispatches to the removed device.
`reload` 仍是同步操作：先停止并 join 旧工作线程、释放插件 box，再卸载旧库、加载
新库并在新线程上启动；`wire` / 重载后重建绑定表，接线保持一致。`unregister`
在返回前停止并 join 恰好一个工作线程；返回后 `thread_count()` 立即准确、接线条目
已移除，下一次 `step` 不会再向该设备派发。

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
    Executor(ExecutorError),                 // worker timeout / high-Z / 工作线程超时或高阻
}
```

`apply_config` 由 `DeviceManager` 负责查找设备（`DeviceNotFound`），再调用
`DevicePlugin::apply_config` 并把 `ConfigError` 包装为 `Invalid`（内联路径）或
经邮箱返回 `Executor`（线程化路径）。
`apply_config` resolves the device in the manager (`DeviceNotFound`) and then
delegates to `DevicePlugin::apply_config`, wrapping `ConfigError` as `Invalid`
on the inline path, or reporting `Executor` through the mailbox on the threaded
path.

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
`wiring().all()` points at an existing device reported by `ports_of()`, with
`port_index < ports().len()` and a matching `port_name`. Entries are only
re-applied after that check succeeds; otherwise the whole device wiring is
dropped and reported in `unwired`.
每次 `reload` 后，`wiring().all()` 的每个条目都指向 `ports_of()` 中真实存在的
设备，满足 `port_index < ports().len()` 且 `port_name` 一致；只有校验通过才会
重新应用，否则清除该设备的全部接线并记录到 `unwired`。

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
- **Threads cannot be killed.** A plugin that never returns from `tick` leaves a
  detached worker behind; `DeviceExecutor::is_detached()` reports it and the
  library handle is intentionally leaked so the plugin vtable stays mapped. Do
  not unload such a library manually. **线程无法被终止。** 永不从 `tick` 返回的
  插件会留下已分离的工作线程；`is_detached()` 报告该状态，库句柄被有意泄漏以保持
  插件 vtable 映射，请勿手动卸载该库。
- **Threaded mode assumes plugins are `Send`.** `RpPlugin` already requires
  `Send + Sync`, so a plugin that compiles for the synchronous path also compiles
  for the threaded one; a plugin that keeps thread-local state must copy it into
  the worker explicitly. **线程化模式假定插件为 `Send`。** `RpPlugin` 已要求
  `Send + Sync`，因此能用于同步路径的插件也能用于线程化路径；依赖线程局部状态的
  插件必须自行把状态复制到工作线程。
- `sync_gpio` / `tick_all` are synchronous-path APIs: worker-hosted devices are
  skipped. Use `step` / `step_gpio` in threaded mode.
  `sync_gpio` / `tick_all` 属于同步路径：工作线程设备会被跳过；线程化模式请用
  `step` / `step_gpio`。

---

## Usage Example / 使用示例

```rust
use rp2350_virt_periph::{DeviceManager, DevicePort, ExecutorConfig, PinSnapshot};

let mut dm = DeviceManager::new();
dm.register("led-1".into(), Box::new(LedPlugin::new()))?;
dm.wire("led-1", 0, 25)?;                      // port 0 → GPIO 25

dm.sync_gpio(&mut chip.gpio_pins);             // every simulation step (sync path)
dm.tick_all(&mut ctx, cycles);

let states = dm.mcp_query_all();               // {"led-1": {"level": true}}
dm.unregister("led-1")?;                       // shutdown + unwire

// Threaded mode: same wiring, fixed one-epoch propagation delay.
let mut dm = DeviceManager::with_executor(ExecutorConfig::default());
dm.register("led-1".into(), Box::new(LedPlugin::new()))?;
dm.wire("led-1", 0, 25)?;
let mut pins = chip.gpio_pins.clone();
let mut inputs = PinSnapshot::empty();
inputs.set(25, true);
let report = dm.step_gpio(0, &mut pins, 1)?;   // effective at epoch 1
assert_eq!(report.degraded, Vec::<String>::new());
let states = dm.mcp_query_device("led-1");
dm.shutdown_threads();                         // join every worker
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
- ⬜ 宿主接线（Host wiring）：`rp2350-cli` 与 `rp2350-egui` 尚未启用
  `DeviceManager::with_executor`（MT-5 的白名单不含这两个 crate），因此
  `--plugin-threads` 开关与虚拟实验室的“多设备并行”接入仍待后续任务；线程化模式目前
  仅由 crate API 与 19 项测试覆盖。
  Neither `rp2350-cli` nor `rp2350-egui` enables `DeviceManager::with_executor`
  yet (MT-5's file whitelist excludes both crates), so a `--plugin-threads` flag
  and the virtual-lab multi-device hookup remain a follow-up task; the threaded
  mode is currently exercised through the crate API and 19 tests.
