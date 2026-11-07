# rp2350-plugin — Public API / 公共 API

> Plugin framework: `RpPlugin` base trait + `DevicePlugin` extended trait.
> 插件框架：`RpPlugin` 基础 trait + `DevicePlugin` 扩展 trait。

---

## Trait Hierarchy / Trait 层次

```
rp2350-plugin (crate)
└── RpPlugin                 ← all plugins implement this (base contract)
     │                          mandatory: metadata, init, tick, shutdown, state_snapshot
     │                          optional:  as_bus_device, irq_triggered
     │
     └── DevicePlugin         ← implemented by virtual peripheral plugins
          (defined in rp2350-virt-periph)
          mandatory:  ports, on_gpio_change, state_snapshot
          optional:   config_schema, apply_config, current_config
          optional:   has_ui, render_ui (requires egui)
```

---

## Why Two Traits? / 为什么设计两个 trait？

| `RpPlugin` | `DevicePlugin` |
|------------|----------------|
| 不关心 GPIO | 必须声明 GPIO 端口 |
| 不关心 UI | 可选提供 egui 面板 |
| 不关心配置 schema | 可选暴露 JSON Schema 配置 |
| 用于纯逻辑插件（如协议分析器、trace 收集器） | 用于虚拟外设插件（LED、TF 卡、PSRAM 等） |

A plugin can implement `RpPlugin` without implementing `DevicePlugin` — such plugins work headless-only and don't appear in the virtual lab canvas.
插件可以实现 `RpPlugin` 而不实现 `DevicePlugin` —— 此类插件仅支持无头模式，不出现在虚拟实验室画布中。

---

## `RpPlugin` Trait / RpPlugin 基础接口

```rust
/// Base plugin contract. All plugins MUST support headless mode.
/// 基础插件契约。所有插件必须支持无头模式。
pub trait RpPlugin: Send + Sync {
    /// Plugin metadata (name, version, author, description, device type).
    /// 插件元数据（名称、版本、作者、描述、设备类型）。
    fn metadata(&self) -> &PluginMetadata;

    /// Initialize with emulator context.
    /// 使用模拟器上下文初始化。
    /// Called once after loading the .dll/.so/.dylib.
    fn init(&mut self, ctx: &mut dyn PluginContext) -> Result<(), PluginError>;

    /// Per-cycle tick.
    /// 每周期 tick。
    /// `cycles` = number of system clock cycles elapsed since last tick.
    fn tick(&mut self, ctx: &mut dyn PluginContext, cycles: u64);

    /// Cleanup before unload.
    /// 卸载前清理。
    fn shutdown(&mut self);

    // ── Optional callbacks (all have default no-op) ──
    // 可选回调（均有默认空实现）

    /// Expose as a memory-mapped BusDevice.
    /// 暴露为内存映射的 BusDevice。
    /// Only needed if plugin registers on the APB address bus.
    fn as_bus_device(&mut self) -> Option<&mut dyn BusDevice> { None }

    /// IRQ notification from the chip.
    fn irq_triggered(&mut self, irq_num: u32) {}

    /// Headless/MCP state query. Must return valid JSON.
    /// 无头/MCP 状态查询。必须返回有效 JSON。
    /// Called by MCP tools to inspect plugin state without GUI.
    fn state_snapshot(&self) -> serde_json::Value { serde_json::json!({}) }
}
```

### Design principles / 设计原则

1. **Headless-first** — `state_snapshot()` is mandatory in spirit (default empty JSON), ensuring MCP tools can always query plugin state.
   **无头优先** —— `state_snapshot()` 实质上必须实现，确保 MCP 工具始终可查询插件状态。

2. **No egui in RpPlugin** — The base trait has zero GUI dependency. A pure-data plugin (e.g., performance counter, protocol analyzer) compiles without egui at all.
   **RpPlugin 不依赖 egui** —— 基础 trait 零 GUI 依赖。纯数据插件（如性能计数器、协议分析器）编译时完全不引入 egui。

3. **Optional BusDevice** — Only plugins that need to respond to memory-mapped register accesses need `as_bus_device()`. GPIO-only devices skip it.
   **可选 BusDevice** —— 仅需响应内存映射寄存器访问的插件才需实现 `as_bus_device()`。纯 GPIO 设备可跳过。

---

## `PluginContext` Trait / 插件上下文

```rust
/// Emulator context provided to plugins.
/// 提供给插件的模拟器上下文。
pub trait PluginContext {
    // ── GPIO operations ──
    /// Drive an RP2350 GPIO output level.
    /// 驱动 RP2350 GPIO 输出电平。
    fn gpio_write(&mut self, pin: u8, level: bool);

    /// Read RP2350 GPIO input level.
    /// 读取 RP2350 GPIO 输入电平。
    fn gpio_read(&self, pin: u8) -> bool;

    /// Set RP2350 GPIO pin direction.
    /// 设置 RP2350 GPIO 引脚方向。
    fn gpio_set_direction(&mut self, pin: u8, output: bool);

    // ── Address bus ──
    /// Register a BusDevice at an APB address range.
    /// 在 APB 地址范围注册 BusDevice。
    fn register_bus_device(&mut self, base: u32, size: u32, device: Box<dyn BusDevice>);

    /// Read byte from chip address space (any region).
    /// 从芯片地址空间读取字节（任意区域）。
    fn mem_read_u8(&self, addr: u32) -> u8;

    /// Write byte to chip address space (any region).
    /// 向芯片地址空间写入字节（任意区域）。
    fn mem_write_u8(&mut self, addr: u32, value: u8);

    // ── Interrupts ──
    /// Trigger an IRQ to the chip.
    /// 触发中断到芯片。
    fn trigger_irq(&mut self, irq: u32, core: Option<usize>);

    // ── Timing ──
    /// Simulation time in nanoseconds.
    /// 模拟时间（纳秒）。
    fn sim_time_ns(&self) -> u64;

    /// System clock frequency in Hz.
    /// 系统时钟频率（Hz）。
    fn sys_clk_hz(&self) -> u32;

    // ── Logging ──
    /// Log a message (appears in egui log panel or CLI output).
    /// 记录日志（出现在 egui 日志面板或 CLI 输出）。
    fn log(&self, level: LogLevel, msg: &str);
}

pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
```

### `PluginMetadata`

```rust
pub struct PluginMetadata {
    /// Unique plugin name.
    pub name: String,
    /// Semantic version "MAJOR.MINOR.PATCH".
    pub version: String,
    /// Author name / organization.
    pub author: String,
    /// Short description (shown in plugin list).
    pub description: String,
    /// Device type for egui widget palette.
    pub device_type: DeviceType,
    /// Icon for virtual lab palette (64x64 PNG, base64-encoded).
    /// Empty string = use default icon.
    pub icon_base64: String,
}

pub enum DeviceType {
    Led,
    TfCard,
    UartTerminal,
    DebugProbe,
    Psram,
    QspiFlash,
    HstxDisplay,
    Button,
    SevenSegment,
    Custom(String),
}
```

---

## `DevicePlugin` Trait / 设备插件接口

```rust
// Defined in: rp2350-virt-periph crate
// 定义在 rp2350-virt-periph crate

/// Device plugin — extends RpPlugin with GPIO ports, optional config, and optional GUI.
/// 设备插件 —— 扩展 RpPlugin，增加 GPIO 端口、可选配置和可选 GUI。
///
/// # Headless requirement / 无头要求
/// All `DevicePlugin` implementations MUST work correctly when `render_ui()` is never called.
/// GUI is purely a visualization layer; device logic must not depend on it.
/// 所有 `DevicePlugin` 实现必须在 `render_ui()` 从未被调用时正常工作。
/// GUI 纯粹是可视化层；设备逻辑不得依赖 GUI。
pub trait DevicePlugin: RpPlugin {
    // ── GPIO Ports (required) ──

    /// Port definitions for virtual lab canvas wiring.
    /// 虚拟实验室画布连线的端口定义。
    fn ports(&self) -> &[DevicePort];

    /// GPIO level change notification.
    /// Called when a wired RP2350 GPIO pin changes level.
    /// `port_index` = index into `self.ports()`.
    /// `level`     = new GPIO level.
    fn on_gpio_change(&mut self, port_index: usize, level: bool);

    // ── Configuration (optional, JSON-based, no GUI dependency) ──

    /// JSON Schema describing configurable parameters.
    /// Returns None if device has no configuration.
    /// 描述可配置参数的 JSON Schema。无配置时返回 None。
    fn config_schema(&self) -> Option<ConfigSchema> { None }

    /// Apply configuration from JSON value.
    /// Can be called from CLI, MCP, or GUI config panel.
    /// 从 JSON 值应用配置。可从 CLI、MCP 或 GUI 配置面板调用。
    fn apply_config(&mut self, config: &serde_json::Value) -> Result<(), ConfigError> {
        let _ = config;
        Ok(())
    }

    /// Current configuration as JSON.
    /// 当前配置（JSON 格式）。
    fn current_config(&self) -> Option<serde_json::Value> { None }

    // ── GUI (optional, egui-dependent) ──

    /// Whether this device has a GUI panel.
    /// 此设备是否有 GUI 面板。
    /// - true  → `render_ui()` will be called each frame in GUI mode.
    /// - false → device is headless-only, no panel shown.
    fn has_ui(&self) -> bool { false }

    /// Render the device's egui panel.
    /// 渲染设备的 egui 面板。
    /// Only called when `has_ui()` returns true AND running in GUI mode.
    /// In headless mode this is never called.
    /// 仅在 `has_ui()` 返回 true 且运行在 GUI 模式时调用。
    /// 无头模式下永远不会调用。
    fn render_ui(&mut self, ui: &mut egui::Ui) {}

    // ── Headless/MCP state query (required) ──

    /// Full device state as JSON for MCP tools and headless inspection.
    /// MCP 工具和无头检查用的完整设备状态（JSON）。
    ///
    /// Must include at minimum:
    /// - device name, mode, status
    /// - all port states (level, direction)
    /// - any statistics (bytes read/written, etc.)
    fn state_snapshot(&self) -> serde_json::Value;
}
```

### `DevicePort`

```rust
/// A connectable port on a virtual device.
/// 虚拟设备上可连线的端口。
pub struct DevicePort {
    /// Port name (e.g. "CS", "MOSI", "SCK", "CMD").
    pub name: String,
    /// Signal direction.
    pub direction: PortDirection,
    /// Visual group (e.g. "SPI", "QSPI", "SDIO", "General").
    pub group: String,
    /// Description tooltip.
    pub description: String,
    /// Current level (updated by DeviceManager on GPIO change).
    pub current_level: bool,
    /// Whether port is currently wired to an RP2350 GPIO pin.
    pub wired: bool,
    /// Connected RP2350 GPIO pin (set by DeviceManager).
    pub gpio_pin: Option<u8>,
}

pub enum PortDirection {
    Input,          // Device reads this signal from RP2350 GPIO output
    Output,         // Device drives this signal onto RP2350 GPIO input
    Bidirectional,  // Device both reads and drives
}
```

---

## Plugin Export Macros / 插件导出宏

```rust
/// Declare the required ABI exports for a plugin.
/// 声明插件必需的 ABI 导出。
///
/// # Example
/// ```
/// use rp2350_plugin::declare_plugin;
///
/// pub struct MyPlugin { /* impl RpPlugin */ }
///
/// declare_plugin!(MyPlugin);
/// ```
///
/// # Generated exports / 生成的导出:
/// - `rp2350_plugin_create() → *mut dyn RpPlugin`
/// - `rp2350_plugin_version() → u32`  (ABI version)
#[macro_export]
macro_rules! declare_plugin {
    ($plugin_type:ty) => {
        #[no_mangle]
        pub extern "C" fn rp2350_plugin_create() -> *mut dyn $crate::RpPlugin {
            let plugin: $plugin_type = Default::default();
            Box::into_raw(Box::new(plugin))
        }

        #[no_mangle]
        pub extern "C" fn rp2350_plugin_destroy(ptr: *mut dyn $crate::RpPlugin) {
            if !ptr.is_null() {
                unsafe {
                    let _ = Box::from_raw(ptr);
                }
            }
        }

        #[no_mangle]
        pub extern "C" fn rp2350_plugin_version() -> u32 {
            1 // ABI version
        }
    };
}

/// Declare exports for a DevicePlugin.
/// 声明 DevicePlugin 的 ABI 导出。
#[macro_export]
macro_rules! declare_device_plugin {
    ($plugin_type:ty) => {
        declare_plugin!($plugin_type);

        #[no_mangle]
        pub extern "C" fn rp2350_plugin_is_device() -> u32 {
            1 // signals this is a DevicePlugin, not just RpPlugin
        }
    };
}
```

---

## Plugin Loading Flow / 插件加载流程

```
DeviceManager::scan_and_load()
     │
     ├── Walk plugins/ directory
     ├── For each .dll/.so/.dylib:
     │     ├── SafeLibrary::new(path)     // dlopen/LoadLibrary
     │     ├── dlsym("rp2350_plugin_version") → check ABI compat
     │     ├── dlsym("rp2350_plugin_create") → Box<dyn RpPlugin>
     │     ├── dlsym("rp2350_plugin_is_device") → if 1, cast to DevicePlugin
     │     ├── plugin.init(ctx)
     │     └── Store in DeviceManager.devices[]
     │
     └── Return Vec<PluginLoadResult>

GUI mode:
     │
     ├── For each device where has_ui() == true:
     │     └── Create egui panel, call render_ui() each frame

Headless/MCP mode:
     │
     ├── Call device.state_snapshot() → aggregate into MCP response
     └── No egui dependency at all
```

---

## Usage Mode Matrix / 使用模式矩阵

| Mode | `RpPlugin` | `DevicePlugin` | GUI available? | `render_ui()` called? | `state_snapshot()` |
|------|-----------|----------------|----------------|----------------------|-------------------|
| **GUI** | ✓ | ✓ | Yes | if `has_ui()` | Yes (for info panel) |
| **Headless CLI** | ✓ | ✓ | No | Never | Yes (for --dump-state) |
| **MCP Server** | ✓ | ✓ | No | Never | Yes (primary API) |
| **CI/Test** | ✓ | ✓ | No | Never | Yes (for assertions) |