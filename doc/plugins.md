# Plugin Development Guide / 插件开发指南

> All virtual devices (first-party and third-party) live in the `plugins/` directory as independent crates.
> 所有虚拟设备（第一方和第三方）作为独立 crate 存放在 `plugins/` 目录。

---

## Directory Structure / 目录结构

```
plugins/
├── led/                       # LED plugin (first-party)
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
├── tfcard/                    # TF Card plugin — SPI + SDIO (first-party)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── spi_mode.rs
│       └── sdio_mode.rs
├── uart-terminal/             # UART Terminal plugin (first-party)
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
├── debug-probe/               # Debug Probe plugin (first-party)
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
├── psram/                     # PSRAM plugin (first-party)
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
├── qspi-flash/                # QSPI Flash plugin (first-party)
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
├── third-party/               # ★ Third-party plugins directory
│   └── my-custom-sensor/      # Your custom plugin here
│       ├── Cargo.toml
│       └── src/
│           └── lib.rs
├── user-guide.md              # This file (also available as doc/plugins.md)
└── README.md
```

---

## Plugin Cargo.toml Template / 插件 Cargo.toml 模板

```toml
[package]
name = "rp2350-plugin-led"
version = "0.1.0"
edition = "2024"
description = "Virtual LED plugin for RP2350 emulator"

[lib]
crate-type = ["cdylib"]       # ★ must be cdylib (dynamic library)
name = "led"                  # output: led.dll / libled.so / libled.dylib

[dependencies]
rp2350-plugin = { path = "../../crates/rp2350-plugin" }
rp2350-virt-periph = { path = "../../crates/rp2350-virt-periph" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Only if device has UI:
egui = "0.37"

# Only if device needs logging:
log = "0.4"

# Only if device needs disk I/O:
# anyhow = "1"
```

---

## Reference Implementations / 参考实现

### 1. LED Plugin — Simplest Device (no config, has UI)

```rust
// plugins/led/src/lib.rs

use rp2350_plugin::{declare_plugin, PluginContext, PluginMetadata, PluginError,
                    DeviceType, RpPlugin};
use rp2350_virt_periph::{DevicePlugin, DevicePort, PortDirection};

pub struct LedPlugin {
    metadata: PluginMetadata,
    ports: Vec<DevicePort>,
    level: bool,
    color: [f32; 3],
}

impl Default for LedPlugin {
    fn default() -> Self {
        Self {
            metadata: PluginMetadata {
                name: "LED".into(),
                version: "1.0.0".into(),
                author: "rp2350-rs team".into(),
                description: "Single-color LED via GPIO".into(),
                device_type: DeviceType::Led,
                icon_base64: String::new(),
            },
            ports: vec![
                DevicePort {
                    name: "ANODE".into(),
                    direction: PortDirection::Input,
                    group: "General".into(),
                    description: "LED anode (connect to GPIO output)".into(),
                    current_level: false,
                    wired: false,
                    gpio_pin: None,
                },
            ],
            level: false,
            color: [1.0, 0.0, 0.0], // red LED
        }
    }
}

impl RpPlugin for LedPlugin {
    fn metadata(&self) -> &PluginMetadata { &self.metadata }
    fn init(&mut self, _ctx: &mut dyn PluginContext) -> Result<(), PluginError> {
        Ok(())
    }
    fn tick(&mut self, _ctx: &mut dyn PluginContext, _cycles: u64) {
        // LED has no internal state machine — purely driven by GPIO
    }
    fn shutdown(&mut self) {}

    fn state_snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.metadata.name,
            "level": self.level,
            "color": self.color,
            "ports": self.ports.iter().map(|p| serde_json::json!({
                "name": p.name,
                "level": p.current_level,
                "wired": p.wired,
                "gpio_pin": p.gpio_pin,
            })).collect::<Vec<_>>(),
        })
    }
}

impl DevicePlugin for LedPlugin {
    fn ports(&self) -> &[DevicePort] { &self.ports }
    fn ports_mut(&mut self) -> &mut [DevicePort] { &mut self.ports }

    fn on_gpio_change(&mut self, port_index: usize, level: bool) {
        if port_index == 0 {
            self.level = level;
        }
    }

    fn has_ui(&self) -> bool { true }

    fn render_ui(&mut self, ui: &mut egui::Ui) {
        let brightness = if self.level { 1.0 } else { 0.15 };
        let color = egui::Color32::from_rgb(
            (self.color[0] * brightness * 255.0) as u8,
            (self.color[1] * brightness * 255.0) as u8,
            (self.color[2] * brightness * 255.0) as u8,
        );
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("●")
                    .color(color)
                    .size(24.0)
            );
            ui.label(&self.metadata.name);
            if self.level {
                ui.label("ON");
            } else {
                ui.label("OFF");
            }
        });
    }

    fn state_snapshot(&self) -> serde_json::Value {
        RpPlugin::state_snapshot(self)
    }
}

declare_plugin!(LedPlugin);
```

### 2. TF Card Plugin — Complex Device (dual mode, config, has UI)

```rust
// plugins/tfcard/src/lib.rs

use rp2350_plugin::{declare_plugin, PluginContext, PluginMetadata, PluginError,
                    DeviceType, RpPlugin, LogLevel};
use rp2350_virt_periph::{DevicePlugin, DevicePort, PortDirection,
                          ConfigSchema, ConfigProperty, ConfigPropType, ConfigError};

pub struct TfCardPlugin {
    metadata: PluginMetadata,
    ports_spi: Vec<DevicePort>,    // 4 ports
    ports_sdio: Vec<DevicePort>,   // 6 ports
    mode: TfCardMode,
    sd_state: SdCardState,
    // ... full SD state machine ...
}

impl Default for TfCardPlugin {
    fn default() -> Self {
        let mut ports_spi = vec![
            DevicePort { name: "CS".into(),   direction: PortDirection::Input,  group: "SPI".into(),  description: "Chip Select (active low)".into(), current_level: true, wired: false, gpio_pin: None },
            DevicePort { name: "MOSI".into(), direction: PortDirection::Input,  group: "SPI".into(),  description: "Master Out, Slave In".into(), current_level: false, wired: false, gpio_pin: None },
            DevicePort { name: "MISO".into(), direction: PortDirection::Output, group: "SPI".into(),  description: "Master In, Slave Out".into(), current_level: false, wired: false, gpio_pin: None },
            DevicePort { name: "SCK".into(),  direction: PortDirection::Input,  group: "SPI".into(),  description: "Serial Clock".into(), current_level: false, wired: false, gpio_pin: None },
        ];
        let ports_sdio = vec![
            DevicePort { name: "CLK".into(),   direction: PortDirection::Input,         group: "SDIO".into(), description: "Clock".into(), current_level: false, wired: false, gpio_pin: None },
            DevicePort { name: "CMD".into(),   direction: PortDirection::Bidirectional, group: "SDIO".into(), description: "Command/Response".into(), current_level: false, wired: false, gpio_pin: None },
            DevicePort { name: "DAT0".into(),  direction: PortDirection::Bidirectional, group: "SDIO".into(), description: "Data line 0".into(), current_level: false, wired: false, gpio_pin: None },
            DevicePort { name: "DAT1".into(),  direction: PortDirection::Bidirectional, group: "SDIO".into(), description: "Data line 1".into(), current_level: false, wired: false, gpio_pin: None },
            DevicePort { name: "DAT2".into(),  direction: PortDirection::Bidirectional, group: "SDIO".into(), description: "Data line 2".into(), current_level: false, wired: false, gpio_pin: None },
            DevicePort { name: "DAT3".into(),  direction: PortDirection::Bidirectional, group: "SDIO".into(), description: "Data line 3".into(), current_level: false, wired: false, gpio_pin: None },
        ];

        Self {
            metadata: PluginMetadata {
                name: "TF_CARD".into(),
                version: "1.0.0".into(),
                author: "rp2350-rs team".into(),
                description: "Virtual TF/SD card (SPI + SDIO 4-bit)".into(),
                device_type: DeviceType::TfCard,
                icon_base64: String::new(),
            },
            ports_spi,
            ports_sdio,
            mode: TfCardMode::Spi,
            sd_state: SdCardState::Idle,
            // ...
        }
    }
}

impl RpPlugin for TfCardPlugin {
    fn metadata(&self) -> &PluginMetadata { &self.metadata }
    fn init(&mut self, ctx: &mut dyn PluginContext) -> Result<(), PluginError> {
        ctx.log(LogLevel::Info, "TF Card plugin initialized");
        Ok(())
    }
    fn tick(&mut self, _ctx: &mut dyn PluginContext, _cycles: u64) {
        // SD state machine tick
    }
    fn shutdown(&mut self) {}
    fn state_snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "name": "TF_CARD",
            "mode": self.mode.as_str(),
            "sd_state": format!("{:?}", self.sd_state),
            "ports": self.ports().iter().map(|p| serde_json::json!({
                "name": p.name,
                "level": p.current_level,
                "wired": p.wired,
            })).collect::<Vec<_>>(),
        })
    }
}

impl DevicePlugin for TfCardPlugin {
    fn ports(&self) -> &[DevicePort] {
        match self.mode {
            TfCardMode::Spi => &self.ports_spi,
            TfCardMode::Sdio => &self.ports_sdio,
        }
    }
    fn ports_mut(&mut self) -> &mut [DevicePort] {
        match self.mode {
            TfCardMode::Spi => &mut self.ports_spi,
            TfCardMode::Sdio => &mut self.ports_sdio,
        }
    }

    fn on_gpio_change(&mut self, port_index: usize, level: bool) {
        match self.mode {
            TfCardMode::Spi => self.handle_spi_gpio(port_index, level),
            TfCardMode::Sdio => self.handle_sdio_gpio(port_index, level),
        }
    }

    // ── Configuration ──
    fn config_schema(&self) -> Option<ConfigSchema> {
        Some(ConfigSchema {
            properties: vec![
                ConfigProperty {
                    name: "mode".into(),
                    description: "Interface mode (SPI or SDIO 4-bit)".into(),
                    prop_type: ConfigPropType::Enum,
                    default: Some(serde_json::json!("spi")),
                    enum_values: Some(vec!["spi".into(), "sdio".into()]),
                    min: None, max: None,
                },
                ConfigProperty {
                    name: "image_path".into(),
                    description: "Path to SD card disk image".into(),
                    prop_type: ConfigPropType::Path,
                    default: None,
                    enum_values: None, min: None, max: None,
                },
                ConfigProperty {
                    name: "block_size".into(),
                    description: "Block size in bytes".into(),
                    prop_type: ConfigPropType::Integer,
                    default: Some(serde_json::json!(512)),
                    enum_values: None,
                    min: Some(512.0), max: Some(4096.0),
                },
            ],
            required: vec!["image_path".into()],
        })
    }

    fn apply_config(&mut self, config: &serde_json::Value) -> Result<(), ConfigError> {
        if let Some(mode) = config.get("mode").and_then(|v| v.as_str()) {
            self.mode = match mode {
                "spi" => TfCardMode::Spi,
                "sdio" => TfCardMode::Sdio,
                _ => return Err(ConfigError::InvalidValue("mode".into())),
            };
        }
        if let Some(path) = config.get("image_path").and_then(|v| v.as_str()) {
            // self.load_image(Path::new(path))
        }
        Ok(())
    }

    fn current_config(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "mode": self.mode.as_str(),
        }))
    }

    // ── GUI ──
    fn has_ui(&self) -> bool { true }

    fn render_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("💾 TF Card");
        ui.separator();
        ui.label(format!("Mode: {:?}", self.mode));
        ui.label(format!("State: {:?}", self.sd_state));

        // Port status
        ui.collapsing("Ports", |ui| {
            for port in self.ports() {
                let icon = if port.current_level { "●" } else { "○" };
                ui.label(format!("{} {} {}", icon, port.name,
                    if port.wired { format!("→ GPIO{}", port.gpio_pin.unwrap_or(0)) }
                    else { "(unwired)".into() }
                ));
            }
        });

        // Mode switch button
        ui.horizontal(|ui| {
            if ui.button("SPI Mode").clicked() {
                self.mode = TfCardMode::Spi;
            }
            if ui.button("SDIO Mode").clicked() {
                self.mode = TfCardMode::Sdio;
            }
        });
    }

    fn state_snapshot(&self) -> serde_json::Value {
        RpPlugin::state_snapshot(self)
    }
}

declare_plugin!(TfCardPlugin);
```

### 3. Headless-Only Plugin (no GUI)

```rust
// plugins/my-protocol-analyzer/src/lib.rs
// A plugin that only collects data — no UI, no GPIO ports.

use rp2350_plugin::{declare_plugin, PluginContext, PluginMetadata, PluginError, RpPlugin};
use rp2350_virt_periph::DevicePlugin;

pub struct ProtocolAnalyzer {
    metadata: PluginMetadata,
    packets: Vec<Packet>,
}

impl RpPlugin for ProtocolAnalyzer {
    fn metadata(&self) -> &PluginMetadata { &self.metadata }
    fn init(&mut self, _ctx: &mut dyn PluginContext) -> Result<(), PluginError> { Ok(()) }
    fn tick(&mut self, _ctx: &mut dyn PluginContext, _cycles: u64) { /* sniff bus */ }
    fn shutdown(&mut self) {}
    fn state_snapshot(&self) -> serde_json::Value {
        serde_json::json!({ "packets": self.packets.len() })
    }
}

impl DevicePlugin for ProtocolAnalyzer {
    fn ports(&self) -> &[DevicePort] { &[] }
    fn ports_mut(&mut self) -> &mut [DevicePort] { &mut [] }

    fn on_gpio_change(&mut self, _port_index: usize, _level: bool) {}
    // No ports → on_gpio_change never called

    fn has_ui(&self) -> bool { false }  // ★ Headless only
    // render_ui never called
    // Works perfectly in MCP mode

    fn state_snapshot(&self) -> serde_json::Value {
        RpPlugin::state_snapshot(self)
    }
}

declare_plugin!(ProtocolAnalyzer);
```

---

## Build & Install / 编译与安装

```bash
# Build all first-party plugins
cargo build --release --workspace
# → target/release/led.dll, tfcard.dll, uart-terminal.dll, ...

# Copy to plugins/ directory (or configure cargo build output dir)
cp target/release/*.dll plugins/

# Build a single third-party plugin
cd plugins/third-party/my-plugin
cargo build --release
cp target/release/my_plugin.dll ../../plugins/third-party/

# The emulator auto-discovers plugins on startup
rp2350 run firmware.elf
# [INFO] Loaded plugin: LED (plugins/led.dll)
# [INFO] Loaded plugin: TF_CARD (plugins/tfcard.dll)
# [INFO] Loaded plugin: UART_TERMINAL (plugins/uart-terminal.dll)
```

---

## ABI Compatibility Checklist / ABI 兼容性检查清单

For a plugin to load successfully, it must:
插件要成功加载，必须：

| Requirement | Check |
|-------------|-------|
| Export `rp2350_plugin_create` → `*mut dyn RpPlugin` | `declare_plugin!` macro ensures |
| Export `rp2350_plugin_destroy` → `fn(*mut dyn RpPlugin)` | `declare_plugin!` macro ensures |
| Export `rp2350_plugin_version` → `u32` matching host ABI | Currently version = 1 |
| Export `rp2350_plugin_is_device` → `u32` (1 = DevicePlugin) | `declare_device_plugin!` macro ensures |
| Plugin struct is `Send + Sync` | Required by trait bounds |
| Plugin struct implements `Default` | Required by `declare_plugin!` |
| Same `RpPlugin` trait definition as host | Must use same `rp2350-plugin` crate version |
| Same `DevicePlugin` trait definition as host | Must use same `rp2350-virt-periph` crate version |

---

## Testing Plugins / 测试插件

```rust
// plugins/tfcard/tests/integration_test.rs

#[test]
fn test_tfcard_spi_init() {
    let mut dm = DeviceManager::new(vec![PathBuf::from("../../target/release")], true);
    dm.load_plugin(Path::new("../../target/release/tfcard.dll")).unwrap();

    let snapshot = dm.mcp_query_device("TF_CARD").unwrap();
    assert_eq!(snapshot["mode"], "spi");
    assert_eq!(snapshot["sd_state"], "Idle");
}

#[test]
fn test_tfcard_configure_via_mcp() {
    let mut dm = DeviceManager::new(vec![PathBuf::from("../../target/release")], true);
    dm.load_plugin(Path::new("../../target/release/tfcard.dll")).unwrap();

    dm.mcp_configure("TF_CARD", &serde_json::json!({"mode": "sdio"})).unwrap();

    let snapshot = dm.mcp_query_device("TF_CARD").unwrap();
    assert_eq!(snapshot["mode"], "sdio");
}
```

---

## Best Practices / 最佳实践

1. **Headless-first** — Always test your plugin with `headless: true` first. GUI is optional decoration.
   **无头优先** — 始终先用 `headless: true` 测试插件。GUI 是可选装饰。

2. **Config via JSON** — Never depend on egui for configuration. `apply_config()` must accept JSON from any source.
   **通过 JSON 配置** — 永远不要依赖 egui 进行配置。`apply_config()` 必须接受来自任何来源的 JSON。

3. **State snapshot is canonical** — `state_snapshot()` output is the authoritative state representation for MCP, testing, and logging.
   **状态快照是规范性的** — `state_snapshot()` 输出是 MCP、测试和日志记录的唯一权威状态表示。

4. **No globals** — Plugin state must be fully self-contained. Multiple instances of the same plugin must work.
   **无全局状态** — 插件状态必须完全自包含。同一插件的多个实例必须能同时工作。

5. **Minimal dependencies** — If your plugin doesn't need egui, don't depend on it. Compile time and DLL size matter.
   **最少依赖** — 如果插件不需要 egui，就不要依赖它。编译时间和 DLL 大小很重要。

6. **Pin direction awareness** — An Input port reads from RP2350 GPIO output. An Output port drives RP2350 GPIO input. Double-check your port directions.
   **引脚方向意识** — Input 端口从 RP2350 GPIO 输出读取。Output 端口驱动 RP2350 GPIO 输入。仔细检查端口方向。