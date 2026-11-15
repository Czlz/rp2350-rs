# rp2350-plugin — Public API / 公共 API

> Plugin framework: `RpPlugin` base trait + `DevicePlugin` extended trait
> (defined in `rp2350-virt-periph`).
> 插件框架：`RpPlugin` 基础 trait + `DevicePlugin` 扩展 trait
> （定义在 `rp2350-virt-periph`）。

---

## Trait Hierarchy / Trait 层次

```
rp2350-plugin (crate)
└── RpPlugin                 ← all plugins implement this (base contract)
     │                          mandatory: metadata, init, tick, shutdown
     │                          optional:  as_bus_device, state_snapshot (defaults provided)
     │
     └── DevicePlugin         ← implemented by virtual peripheral plugins
          (defined in rp2350-virt-periph)
          mandatory:  ports, ports_mut, on_gpio_change
          optional:   config_schema, apply_config, has_ui,
                      render_ui (only with the `gui` feature)
          inherited:  state_snapshot (default empty JSON — override recommended)
```

---

## Why Two Traits? / 为什么设计两个 trait？

| `RpPlugin` | `DevicePlugin` |
|------------|----------------|
| 不关心 GPIO | 必须声明 GPIO 端口 |
| 不关心 UI | 可选提供 egui 面板 |
| 不关心配置 schema | 可选暴露 JSON Schema 配置 |
| 用于纯逻辑插件（如协议分析器、trace 收集器） | 用于虚拟外设插件（LED 等） |

A plugin can implement `RpPlugin` without implementing `DevicePlugin` — such plugins work headless-only and don't appear in the virtual lab canvas.
插件可以实现 `RpPlugin` 而不实现 `DevicePlugin` —— 此类插件仅支持无头模式，不出现在虚拟实验室画布中。

---

## `RpPlugin` Trait / RpPlugin 基础接口

```rust
/// The base plugin contract. Every plugin (pure logic or device) implements this.
/// 基础插件契约。每个插件（纯逻辑或设备）都实现此 trait。
pub trait RpPlugin: Send + Sync {
    /// Return plugin metadata for discovery and display.
    /// 返回插件元数据用于发现和显示。
    fn metadata(&self) -> &PluginMetadata;

    /// Initialize the plugin with the given host context.
    /// 使用给定主机上下文初始化插件。
    /// Called once before any `tick()` calls.
    /// 在任何 `tick()` 调用之前调用一次。
    ///
    /// # Errors / 错误
    /// Returns `PluginError` if initialization fails. / 初始化失败时返回 `PluginError`。
    fn init(&mut self, ctx: &mut dyn PluginContext) -> Result<(), PluginError>;

    /// Advance the plugin's internal state by `cycles` simulation cycles.
    /// 将插件的内部状态推进 `cycles` 个仿真周期。
    fn tick(&mut self, ctx: &mut dyn PluginContext, cycles: u64);

    /// Shut down the plugin and release resources.
    /// 关闭插件并释放资源。
    /// Called once at the end of the plugin's lifecycle.
    /// 在插件生命周期结束时调用一次。
    fn shutdown(&mut self);

    // ── Optional callbacks (default impls) ──
    // 可选回调（有默认实现）

    /// If this plugin acts as a bus device, return a mutable reference.
    /// 如果此插件充当总线设备，返回可变引用。
    /// Default `None` — only memory-mapped peripherals override this.
    /// 默认 `None` —— 仅内存映射外设覆写。
    /// (Host-side bus wiring is a planned integration. / 主机侧总线接线为规划项。)
    fn as_bus_device(&mut self) -> Option<&mut dyn BusDevice> { None }

    /// Return complete plugin state as JSON (for MCP / headless query).
    /// 返回完整插件状态（JSON，用于 MCP / 无头查询）。
    /// Default: empty object. Override for meaningful data.
    /// 默认空对象。覆写以提供有意义的数据。
    fn state_snapshot(&self) -> serde_json::Value { serde_json::json!({}) }
}
```

### Design principles / 设计原则

1. **Headless-first** — the trait has zero dependency on egui or any rendering library; all methods must work in headless mode (MCP, CLI, CI).
   **无头优先** —— trait 零依赖 egui 或任何渲染库；所有方法必须在无头模式（MCP、CLI、CI）下正常工作。

2. **`state_snapshot()` is the query surface** — default empty JSON, but MCP tools and headless inspection rely on meaningful overrides.
   **`state_snapshot()` 是查询接口** —— 默认空 JSON，但 MCP 工具和无头检查依赖有意义的覆写。

3. **Optional BusDevice** — only plugins that act as memory-mapped peripherals implement `as_bus_device()`. GPIO-only devices skip it.
   **可选 BusDevice** —— 仅充当内存映射外设的插件实现 `as_bus_device()`。纯 GPIO 设备可跳过。

---

## `PluginContext` Trait / 插件上下文

```rust
/// Minimal memory-mapped bus device interface (plugin perspective).
/// 内存映射总线设备的最小接口（插件视角）。
/// Full `MemoryRegion` capability lives in `rp2350-core` for crate-internal use.
/// 完整的 `MemoryRegion` 能力在 `rp2350-core` 中供 crate 内部使用。
pub trait BusDevice: Send {
    /// The device's base address on the bus / 设备在总线上的基地址
    fn base_address(&self) -> u32;
    /// The size of the device's address space / 设备地址空间的大小
    fn size(&self) -> usize;
}

/// The context provided to a plugin at initialization and during tick.
/// 插件初始化和 tick 时提供的上下文。
/// Provides access to the simulation environment without exposing internals.
/// 提供对仿真环境的访问，但不暴露内部细节。
pub trait PluginContext: Send {
    // ── Logging ──
    /// Log a message through the host's logging system
    /// (info / warn / error levels).
    /// 通过主机的日志系统记录消息（info / warn / error 三个级别）。
    fn log_info(&self, message: &str);
    fn log_warn(&self, message: &str);
    fn log_error(&self, message: &str);

    // ── Timing ──
    /// Current simulation time in cycles / 当前仿真时间（周期数）
    fn current_cycles(&self) -> u64;

    /// Request that the host schedule a callback after `cycles` ticks.
    /// 请求主机在 `cycles` 个 tick 后调度一个回调。
    /// (Interface reserved — host-side dispatch is not wired yet.
    ///  接口已预留 —— 主机侧调度尚未接入。)
    fn schedule_callback(&mut self, cycles: u64, tag: &str);

    /// Access a typed service from the host (e.g. an external bus reference).
    /// 从主机访问一个类型化服务（如外部总线引用）。
    /// Returns `None` if the service is not available. / 服务不可用时返回 `None`。
    #[doc(hidden)]
    fn get_service_erased(&self) -> Option<&dyn Any>;
}

/// Extension trait for typed service access / 类型化服务访问的扩展 trait
/// (blanket-implemented for every `PluginContext` / 对所有 `PluginContext` 自动实现)
pub trait PluginContextExt: PluginContext {
    /// Access a typed service from the host / 从主机访问类型化服务
    fn get_service<T: Any>(&self) -> Option<&T> {
        self.get_service_erased().and_then(|s| s.downcast_ref::<T>())
    }
}
```

### What the context does NOT provide / 上下文不提供的能力

- **No GPIO operations** — GPIO interaction is not part of `PluginContext`. Device plugins declare
  `DevicePort`s and the host drives them via `DeviceManager::wire` + `DeviceManager::sync_gpio`.
  GPIO 交互不属于 `PluginContext`：设备插件声明 `DevicePort`，
  由主机通过 `DeviceManager::wire` + `DeviceManager::sync_gpio` 驱动。
- **No bus registration** — a plugin exposes a bus device via `RpPlugin::as_bus_device()`;
  host-side address-bus wiring is a planned integration.
  总线注册：插件通过 `RpPlugin::as_bus_device()` 暴露总线设备；
  主机侧地址总线接线为规划项。
- **No direct memory access** — the current ABI gives plugins no read/write access to chip
  address space. / 当前 ABI 下插件不能直接读写芯片地址空间。

---

## `PluginMetadata`

```rust
/// Plugin metadata exposed to the host for discovery and display.
/// 暴露给主机的插件元数据，用于发现和显示。
pub struct PluginMetadata {
    /// Unique plugin identifier (e.g. "com.rp2350-rs.led") / 唯一插件标识符
    pub id: String,
    /// Human-readable plugin name / 人类可读的插件名称
    pub name: String,
    /// Plugin version (semver) / 插件版本（语义版本）
    pub version: String,
    /// Brief description / 简短描述
    pub description: String,
    /// Plugin author / 插件作者
    pub author: String,
    /// ABI version this plugin was compiled against / 此插件编译时使用的 ABI 版本
    pub abi_version: u32,
    /// Minimum host version required / 最低主机版本要求
    pub min_host_version: String,
    /// Whether this plugin has a GUI component / 此插件是否有 GUI 组件
    pub has_ui: bool,
    /// Tags for categorization and search / 分类和搜索标签
    pub tags: Vec<String>,
}
```

Metadata is built with `PluginMetadata::builder()` → `PluginMetadataBuilder`
(setters `id` / `name` / `version` / `description` / `author` / `abi_version` /
`min_host_version` / `has_ui` / `tags`, then `build()`).
There is **no** `DeviceType` enum and no icon field in the current ABI — the egui
palette currently keys off `DevicePlugin::ports()` and `has_ui()`.
元数据通过 `PluginMetadata::builder()` 构建。当前 ABI 中**没有** `DeviceType`
枚举和图标字段；egui 调色板目前依赖 `DevicePlugin::ports()` 与 `has_ui()`。

---

## `DevicePlugin` Trait / 设备插件接口

```rust
// Defined in: rp2350-virt-periph crate
// 定义在 rp2350-virt-periph crate

/// Device plugin — extends RpPlugin with GPIO ports, optional config, and optional GUI.
/// 设备插件 —— 扩展 RpPlugin，增加 GPIO 端口、可选配置和可选 GUI。
///
/// # Headless requirement / 无头要求
/// All device logic must work without `render_ui()` ever being called.
/// `state_snapshot()` (inherited from `RpPlugin`) is the sole authoritative
/// interface used by MCP and headless mode.
/// 所有设备逻辑必须在从不调用 `render_ui()` 的情况下正常工作。
/// 继承自 `RpPlugin` 的 `state_snapshot()` 是 MCP 和无头模式使用的唯一权威接口。
pub trait DevicePlugin: RpPlugin {
    // ── GPIO Ports (required) ──

    /// The list of I/O ports this device exposes.
    /// 此设备暴露的 I/O 端口列表。
    fn ports(&self) -> &[DevicePort];

    /// Mutable access to the port list (the host updates port levels via this).
    /// 端口列表的可变访问（主机借此更新端口电平）。
    fn ports_mut(&mut self) -> &mut [DevicePort];

    /// GPIO level change notification.
    /// Called when a wired RP2350 GPIO pin changes level.
    /// `port_index` = index into `self.ports()`.
    /// `level`     = new GPIO level.
    /// GPIO 电平变化通知。连接的 RP2350 GPIO 引脚电平变化时调用。
    fn on_gpio_change(&mut self, port_index: usize, level: bool);

    // ── Configuration (optional, JSON-based, no GUI dependency) ──

    /// JSON Schema describing configurable parameters.
    /// Returns None if device has no configuration.
    /// 描述可配置参数的 JSON Schema。无配置时返回 None。
    fn config_schema(&self) -> Option<ConfigSchema> { None }

    /// Apply configuration from JSON value.
    /// Can be called from CLI, MCP, or GUI config panel.
    /// 从 JSON 值应用配置。可从 CLI、MCP 或 GUI 配置面板调用。
    ///
    /// # Errors / 错误
    /// Returns `ConfigError` if the configuration is invalid. / 配置无效时返回 `ConfigError`。
    fn apply_config(&mut self, config: &serde_json::Value) -> Result<(), ConfigError> { ... }

    // ── GUI (optional, egui-dependent, `gui` feature only) ──

    /// Whether this device has a GUI panel.
    /// If `false`, `render_ui()` will never be called.
    /// 此设备是否有 GUI 面板。为 false 时 `render_ui()` 永远不会被调用。
    fn has_ui(&self) -> bool { false }

    /// Render the device's egui panel.
    /// Only called when `has_ui()` returns true AND the `gui` feature is enabled.
    /// In headless mode this is never called.
    /// 渲染设备的 egui 面板。仅在 `has_ui()` 为 true 且启用 `gui` 特性时调用。
    /// 无头模式下永远不会调用。
    #[cfg(feature = "gui")]
    fn render_ui(&mut self, ui: &mut egui::Ui) {}

    // state_snapshot() is inherited from RpPlugin (default: empty JSON).
    // Override it — it is the MCP / headless query interface.
    // state_snapshot() 继承自 RpPlugin（默认空 JSON）。请覆写它 ——
    // 它是 MCP / 无头查询接口。
}
```

### `DevicePort`

```rust
/// A single I/O port on a virtual device.
/// 虚拟设备上的单个 I/O 端口。
pub struct DevicePort {
    /// Port name (e.g. "MOSI", "SCK", "LED") / 端口名称
    pub name: String,
    /// Signal direction / 信号方向
    pub direction: PortDirection,
    /// Current signal level (updated via `ports_mut`) / 当前信号电平（经 `ports_mut` 更新）
    pub level: bool,
    /// Port description for UI tooltip / 端口描述（UI 提示用）
    pub description: String,
}

pub enum PortDirection {
    /// Signal flows into the device from GPIO pin / 信号从 GPIO 引脚流入设备
    Input,
    /// Signal flows out of the device to GPIO pin / 信号从设备流出到 GPIO 引脚
    Output,
    /// Signal can flow in either direction / 信号可双向流动
    Bidirectional,
}

// Constructors / 构造器：
//   DevicePort::input(name, description)
//   DevicePort::output(name, description)
//   DevicePort::bidirectional(name, description)
```

> Wiring state (which GPIO pin a port is connected to) lives in the host's
> `WiringTable`, not on the port itself.
> 接线状态（端口连接到哪个 GPIO 引脚）保存在主机的 `WiringTable` 中，不在端口上。

---

## Plugin ABI Exports / 插件 ABI 导出

当前**没有**导出宏（`declare_plugin!` / `declare_device_plugin!` 为规划项）：插件以
`#[unsafe(no_mangle)] pub extern "C"` 手工导出三个 ABI 符号，参考实现
`plugins/led/src/lib.rs`。
There are currently **no** export macros (`declare_plugin!` /
`declare_device_plugin!` are planned): a plugin declares the three ABI symbols
manually, as the reference implementation `plugins/led/src/lib.rs` does.

```rust
use rp2350_plugin::abi::ABI_VERSION;

/// Return the ABI version this plugin was compiled against
/// 返回此插件编译时使用的 ABI 版本
#[unsafe(no_mangle)]
pub extern "C" fn rp2350_plugin_version() -> u32 {
    ABI_VERSION
}

/// Report that this library exports a device plugin
/// 报告此库导出的是设备插件
#[unsafe(no_mangle)]
pub extern "C" fn rp2350_plugin_is_device() -> u32 {
    1
}

/// Create a boxed device plugin for the host
/// 为主机创建 boxed 设备插件
#[unsafe(no_mangle)]
pub extern "C" fn rp2350_device_plugin_create() -> *mut dyn DevicePlugin {
    Box::into_raw(Box::new(MyPlugin::new()))
}
```

> 没有 destroy 导出：宿主用 `Box::from_raw` 回收实例，`SafeLibrary` RAII 关闭库。
> No destroy export: the host reclaims the instance via `Box::from_raw`, and the
> `SafeLibrary` RAII guard closes the library.

---

## Plugin Loading Flow / 插件加载流程

```
DeviceManager::load_plugin(id, path)     // 单个库文件加载（内部走 PluginLoader::load_device）
     │
     ├── id 已注册 → DeviceError::AlreadyExists
     ├── PluginLoader::load(path)                             // 输入校验 + dlopen / LoadLibrary
     │     ├── 不存在 / 目录 / 0 字节 → PluginLoadFailed（含路径）
     │     └── 随机字节 / 截断 ELF / 非 cdylib → PluginLoadFailed（含路径）
     ├── find_symbol("rp2350_plugin_version") → 必须等于 ABI_VERSION (1)
     ├── find_symbol("rp2350_plugin_is_device") → 非 1 拒绝（仅设备插件）
     ├── find_symbol("rp2350_device_plugin_create") → 调用工厂
     ├── 工厂返回空指针 → DeviceError::PluginLoadFailed
     └── Box::from_raw(create()) → LoadedPlugin { plugin, library }
              （plugin 先声明：trait 对象先于库释放，vtable 始终有效）

目录扫描（实例化并注册）：
     DeviceManager::scan_and_load(dir)
       ├── scan_with_fingerprints(dir) → 按平台扩展名 + 路径排序
       └── 每个库：load_device → id = metadata().id || 文件名主干
             ├── 重复 ID → log::warn! + 跳过（库被关闭）
             └── 其它错误 → log::warn! + 跳过（坏库不中断扫描）

热重载：
     DeviceManager::reload(dir) → ReloadReport
       ├── removed  文件消失：shutdown → 清除接线 → 卸载库
       ├── reloaded 指纹变化：快照接线 → 释放旧实例/库 → 重新加载
       │     ├── 端口索引/名称一致 → rewired（接线保留）
       │     └── 端口表或设备 ID 变化 → unwired（接线清除 + 原因）
       ├── added    新库 → 加载并注册（失败记入 failed）
       └── failed   加载失败：只影响该库，其余设备继续工作

卸载：
     DeviceManager::unregister(id) → shutdown() → 移除接线 →
     ManagedDevice 连同 SafeLibrary 一起 drop（库在设备注销时才关闭）

GUI mode / GUI 模式：
     has_ui() == true 的设备 → egui 面板，每帧调用 render_ui()

Headless/MCP mode / 无头与 MCP 模式：
     state_snapshot() → 由 mcp_query_all() 汇总进 MCP 响应；完全不依赖 egui
```

> **Loader guarantees and limits / 加载器保证与限制**
>
> - 所有非法输入（路径不存在、目录、空文件、随机字节、截断文件、缺符号、
>   ABI 不匹配、工厂返回空指针）都返回 `DeviceError::PluginLoadFailed`，消息
>   包含路径与双语原因，不 panic、不泄漏库句柄。
> - 只有「版本号 + 三个导出符号存在性」可校验；相同 `ABI_VERSION` 但不同编译器
>   构建的库仍可能布局不兼容，必须使用同一工具链。
> - **跨 `cdylib` 的 panic 无法捕获**：插件 panic 会穿过 `extern "C"` 帧终止
>   进程，因此插件自身必须不 panic，错误应返回 `PluginError`。
> - 工厂返回非空但无效的裸指针无法校验，宿主按可信代码处理（固有 ABI 限制）。
>
> - Every invalid input (missing path, directory, empty file, random bytes,
>   truncated file, missing symbol, ABI mismatch, null factory) returns
>   `DeviceError::PluginLoadFailed` with the path and a bilingual reason; no
>   panic, no leaked library handle.
> - Only the version number and the presence of the three exports can be
>   validated; same-`ABI_VERSION` libraries built by different compilers may
>   still be layout-incompatible, so one toolchain is required.
> - **Panics cannot be caught across the `cdylib` boundary**: a panicking plugin
>   unwinds through an `extern "C"` frame and aborts the process. Plugins must
>   not panic and should return `PluginError` instead.
> - A non-null invalid raw pointer from the factory cannot be validated; the host
>   trusts plugins as trusted code (inherent ABI limit).

---

## Usage Mode Matrix / 使用模式矩阵

| Mode | `RpPlugin` | `DevicePlugin` | GUI available? | `render_ui()` called? | `state_snapshot()` |
|------|-----------|----------------|----------------|----------------------|-------------------|
| **GUI** | ✓ | ✓ | Yes | if `has_ui()` | Yes (for info panel) |
| **Headless CLI** | ✓ | ✓ | No | Never | Yes (for --dump-state) |
| **MCP Server** | ✓ | ✓ | No | Never | Yes (primary API) |
| **CI/Test** | ✓ | ✓ | No | Never | Yes (for assertions) |
