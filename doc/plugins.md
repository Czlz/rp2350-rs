# Plugin Development Guide / 插件开发指南

> All virtual devices (first-party and third-party) live in the `plugins/` directory as independent crates.
> Two device plugins have landed so far: the reference LED plugin and the UART terminal;
> the others below are planned.
> 所有虚拟设备（第一方和第三方）作为独立 crate 存放在 `plugins/` 目录。
> 目前已有两个插件落地：参考 LED 插件与 UART 终端，其余为规划项。

---

## Directory Structure / 目录结构

```
plugins/
├── led/                       # LED plugin (first-party, landed / 已落地)
│   ├── Cargo.toml             #   cdylib + rlib
│   ├── src/
│   │   └── lib.rs
│   └── tests/
│       └── dynamic_load.rs    #   loads the built .dll via DeviceManager
├── uart-terminal/             # UART Terminal (first-party, landed / 已落地)
│   ├── Cargo.toml             #   cdylib + rlib
│   └── src/
│       ├── lib.rs             #   bit-level UART decoder / serializer
│       └── tests.rs           #   bit-timing tests (115200 / 9600)
├── tfcard/                    # (planned / 规划中) TF Card — SPI + SDIO
├── debug-probe/               # (planned / 规划中) Debug Probe
├── psram/                     # (planned / 规划中) PSRAM
├── qspi-flash/                # (planned / 规划中) QSPI Flash
└── third-party/               # (planned / 规划中) third-party plugins directory
```

---

## Plugin Cargo.toml Template / 插件 Cargo.toml 模板

Mirrors `plugins/led/Cargo.toml` (workspace member):
与 `plugins/led/Cargo.toml` 一致（workspace 成员）：

```toml
[package]
name = "rp2350-my-plugin"      # 第一方插件以 rp2350-*-plugin 命名
version.workspace = true
edition.workspace = true

[lib]
crate-type = ["cdylib", "rlib"]  # cdylib = 可加载插件；rlib = 供主机侧测试
name = "rp2350_my_plugin"        # output: rp2350_my_plugin.dll / .so / .dylib

[dependencies]
rp2350-plugin = { workspace = true }        # RpPlugin / PluginContext / ABI_VERSION
rp2350-virt-periph = { workspace = true }   # DevicePlugin / DevicePort / ConfigSchema
serde_json = { workspace = true }           # state_snapshot / apply_config

[dev-dependencies]
rp2350-core = { workspace = true }          # GpioPin, for DeviceManager-driven tests
```

> egui 依赖仅在实现 `render_ui()` 且启用 `gui` 特性时才需要；日志无需依赖 `log`
> crate —— 通过 `PluginContext::log_info/log_warn/log_error` 走主机日志系统。
> egui is only needed for `render_ui()` with the `gui` feature; logging needs no
> `log` dependency — go through `PluginContext::log_info/log_warn/log_error`.

---

## Reference Implementation / 参考实现：LED Plugin

This is the real `plugins/led/src/lib.rs` (abridged) — the only landed plugin,
and the template to copy:
这是真实的 `plugins/led/src/lib.rs`（节选）—— 目前唯一的落地插件，也是模板：

```rust
// plugins/led/src/lib.rs

// 插件 ABI 通过 trait 对象指针跨越动态库边界，依赖同一编译器与 ABI 版本。
// The plugin ABI passes trait-object pointers across the dynamic library
// boundary, relying on the same compiler and ABI version.
#![allow(improper_ctypes_definitions)]

use rp2350_plugin::abi::ABI_VERSION;
use rp2350_plugin::{PluginContext, PluginError, PluginMetadata, RpPlugin};
use rp2350_virt_periph::{DevicePlugin, DevicePort};

const PLUGIN_ID: &str = "com.rp2350-rs.led";

pub struct LedPlugin {
    metadata: PluginMetadata,
    ports: Vec<DevicePort>,
    level: bool,
    transitions: u64,
}

impl LedPlugin {
    pub fn new() -> Self {
        Self {
            metadata: PluginMetadata::builder()
                .id(PLUGIN_ID)
                .name("LED")
                .version("0.1.0")
                .description("Reference single-LED virtual device / 参考单 LED 虚拟设备")
                .author("RP2350-rs Team")
                .abi_version(ABI_VERSION)
                .min_host_version("0.1.0")
                .has_ui(false)
                .tags(vec!["reference".to_string(), "gpio".to_string()])
                .build(),
            ports: vec![DevicePort::input(
                "LED",
                "GPIO-driven LED input / GPIO 驱动的 LED 输入",
            )],
            level: false,
            transitions: 0,
        }
    }
}

impl RpPlugin for LedPlugin {
    fn metadata(&self) -> &PluginMetadata { &self.metadata }

    fn init(&mut self, _ctx: &mut dyn PluginContext) -> Result<(), PluginError> {
        Ok(())
    }

    fn tick(&mut self, _ctx: &mut dyn PluginContext, _cycles: u64) {
        // LED 是纯电平驱动设备，无需随时间推进状态。
        // An LED is purely level-driven; no time-based state to advance.
    }

    fn shutdown(&mut self) {
        self.level = false;
        self.ports[0].level = false;
    }

    fn state_snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "port": "LED",
            "on": self.level,
            "transitions": self.transitions,
        })
    }
}

impl DevicePlugin for LedPlugin {
    fn ports(&self) -> &[DevicePort] { &self.ports }
    fn ports_mut(&mut self) -> &mut [DevicePort] { &mut self.ports }

    fn on_gpio_change(&mut self, port_index: usize, level: bool) {
        if port_index != 0 {
            return;
        }
        if self.level != level {
            self.transitions += 1;
        }
        self.level = level;
        self.ports[0].level = level;
    }
}

// ── ABI exports / ABI 导出（当前无导出宏，手工声明）──

#[unsafe(no_mangle)]
pub extern "C" fn rp2350_plugin_version() -> u32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn rp2350_plugin_is_device() -> u32 {
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn rp2350_device_plugin_create() -> *mut dyn DevicePlugin {
    Box::into_raw(Box::new(LedPlugin::new()))
}
```

> 没有 destroy 导出：宿主用 `Box::from_raw` 回收实例，`SafeLibrary` RAII 关闭库。
> No destroy export: the host reclaims the instance via `Box::from_raw`, and
> `SafeLibrary` RAII closes the library.

---

## Landed Example / 已落地示例：UART Terminal（`plugins/uart-terminal`）

A bit-level UART terminal: it samples the MCU's `TX` line, decodes
start bit + data bits (LSB first) + stop bit into bytes, and can shift bytes
back out on the `RX` line at the configured baud rate.
一个位级 UART 终端：采样 MCU 的 `TX` 线，把 起始位 + 数据位（LSB 优先）+ 停止位
解码为字节，并能按配置的波特率在 `RX` 线上把字节移出。

| Item / 项目 | Value / 值 |
|-------------|-----------|
| Device ID | `com.rp2350-rs.uart-terminal` |
| Name | `UART_TERMINAL` |
| Ports | `0 = TX` (`Input`, sampled from MCU GPIO) · `1 = RX` (`Output`, drives MCU GPIO) |
| GUI | `has_ui = false` (headless only / 纯无头) |
| Tags | `["uart", "terminal"]` |
| Library | `rp2350_uart_terminal.dll` / `librp2350_uart_terminal.so` / `.dylib` |

### Configuration / 配置

`config_schema()` properties (all optional / 全部可选)：

| Field | Type | Default | Meaning / 含义 |
|-------|------|---------|----------------|
| `baud` | integer | `115200` | Line speed, `1..=10_000_000` / 波特率 |
| `data_bits` | integer | `8` | Data bits per frame, `5..=8` / 每帧数据位 |
| `stop_bits` | integer | `1` | Stop bits per frame, `1..=2` / 每帧停止位 |
| `rx_line` | boolean | `true` | Idle (mark) level of the `RX` line / `RX` 空闲电平 |
| `clock_hz` | integer | `150000000` | Cycles per second used to derive bit timing / 由周期换算位时间的时钟频率 |

```jsonc
// apply_config example / apply_config 示例
{ "baud": 9600, "data_bits": 8, "stop_bits": 1, "rx_line": true }
```

Invalid values (wrong JSON type, `baud = 0`, `data_bits = 9`, `stop_bits = 0`,
`clock_hz = 0`, `baud > clock_hz`) return `ConfigError::InvalidType` /
`ConfigError::OutOfRange`; a rejected configuration never mutates the device.
非法值返回 `ConfigError::InvalidType` / `ConfigError::OutOfRange`；
被拒绝的配置不会改变设备状态。

### Timing / 时序

`PluginContext::current_cycles()` only exposes cycles, so the bit period is
`bit_cycles = clock_hz / baud`. The receiver samples at bit **centers**
(1.5 · bit_cycles after the observed start edge, then every `bit_cycles`),
and one `tick()` may cover several sample points — using absolute cycles keeps
chunked host stepping drift-free. At the default 150 MHz clock a 115200 baud
bit lasts 1302 cycles; steering `clock_hz` also lets hosts match any other
system clock.
`PluginContext::current_cycles()` 只暴露周期数，因此位周期为
`bit_cycles = clock_hz / baud`。接收端在位**中心**采样（观察到起始边沿后
1.5 · bit_cycles，之后每 `bit_cycles` 一次），一次 `tick()` 可跨过多个采样点；
采用绝对周期可避免主机分块推进产生漂移。默认 150 MHz 时钟下 115200 波特率的
一位为 1302 周期；调整 `clock_hz` 可匹配其它系统时钟。

A frame whose stop bit does not match the idle level is dropped and counted in
`framing_errors`. 停止位不等于空闲电平的帧会被丢弃并计入 `framing_errors`。

### State snapshot / 状态快照

```jsonc
{
  "name": "UART_TERMINAL",
  "baud": 115200, "data_bits": 8, "stop_bits": 1,
  "rx_line": true, "clock_hz": 150000000, "bit_cycles": 1302,
  "rx_bytes": 1,            // bytes decoded from TX / 从 TX 解码的字节数
  "tx_bytes": 0,            // bytes shifted out on RX / 从 RX 移出的字节数
  "last_byte": 65,          // last decoded byte, null before the first / 最近解码字节
  "last_tx_byte": null,     // last byte shifted out on RX / 最近移出字节
  "buffer_len": 1,          // retained receive bytes (max 4096) / 缓冲字节数
  "framing_errors": 0,
  "recent_hex": "41",       // last 16 bytes as hex / 最近 16 字节十六进制
  "recent_ascii": "A"       // last 16 bytes as printable ASCII / 可打印形式
}
```

### Sending bytes (device → host) / 发送字节（device → host）

`send_byte(byte)` / `send_bytes(&[u8])` queue bytes that the next `tick()`
calls shift out on the `RX` output port (start bit, LSB-first data, stop bit).
There is **no MCP entry point** for this direction yet, so it is reachable only
from library callers and tests; wiring `send_byte` into the MCP/CLI layer is
pending.
`send_byte(byte)` / `send_bytes(&[u8])` 将字节排入队列，后续 `tick()` 在 `RX`
输出端口移出（起始位、LSB 优先数据、停止位）。该方向**尚无 MCP 入口**，
目前只能由库调用方与测试触发；把 `send_byte` 接入 MCP/CLI 层待完成。

### Build & inspect / 编译与查看

```bash
cargo build --release -p rp2350-uart-terminal
cargo test -p rp2350-uart-terminal          # 11 tests / 11 个测试
cargo run --release -p rp2350-cli -- list-devices --plugin-dir target/release
# com.rp2350-rs.led / LED / has_ui=false / ports=[LED]
# com.rp2350-rs.uart-terminal / UART_TERMINAL / has_ui=false / ports=[TX,RX]
```

Wiring example (MCU GPIO4 → `TX`, MCU GPIO5 ← `RX`):
接线示例（MCU GPIO4 → `TX`，MCU GPIO5 ← `RX`）：

```bash
cargo run --release -p rp2350-cli -- wire --plugin-dir target/release \
      --device com.rp2350-rs.uart-terminal --port 0 --gpio 4
```

> Wiring is one-shot today (no persistence, no emulator attach) — same as the
> LED plugin. 与 LED 插件相同，接线目前是一次性的（无持久化、不挂接模拟器）。

---

## Planned Example / 规划示例：TF Card Plugin（dual mode + config）

Shows the patterns a config-bearing device will use — written against the
**current** API (note: no `declare_plugin!`, no `DeviceType`, no
`current_config`; `ConfigSchema` is JSON-Schema-shaped):
展示带配置设备将使用的模式 —— 基于**当前** API 编写（注意：没有
`declare_plugin!`、没有 `DeviceType`、没有 `current_config`；
`ConfigSchema` 为 JSON Schema 形态）：

```rust
// plugins/tfcard/src/lib.rs — planned / 规划中

use rp2350_plugin::abi::ABI_VERSION;
use rp2350_plugin::{PluginContext, PluginError, PluginMetadata, RpPlugin};
use rp2350_virt_periph::{ConfigError, ConfigSchema, DevicePlugin, DevicePort, PortDirection};

pub struct TfCardPlugin {
    metadata: PluginMetadata,
    ports_spi: Vec<DevicePort>,    // 4 ports / 4 个端口
    ports_sdio: Vec<DevicePort>,   // 6 ports / 6 个端口
    mode: TfCardMode,
    // ... full SD state machine / 完整 SD 状态机 ...
}

impl TfCardPlugin {
    pub fn new() -> Self {
        let ports_spi = vec![
            DevicePort::input("CS",   "Chip Select (active low) / 片选（低有效）"),
            DevicePort::input("MOSI", "Master Out, Slave In"),
            DevicePort::output("MISO", "Master In, Slave Out"),
            DevicePort::input("SCK",  "Serial Clock / 串行时钟"),
        ];
        let ports_sdio = vec![
            DevicePort::input("CLK", "Clock / 时钟"),
            DevicePort::bidirectional("CMD",  "Command/Response / 命令/响应"),
            DevicePort::bidirectional("DAT0", "Data line 0 / 数据线 0"),
            DevicePort::bidirectional("DAT1", "Data line 1 / 数据线 1"),
            DevicePort::bidirectional("DAT2", "Data line 2 / 数据线 2"),
            DevicePort::bidirectional("DAT3", "Data line 3 / 数据线 3"),
        ];
        Self {
            metadata: PluginMetadata::builder()
                .id("com.rp2350-rs.tfcard")
                .name("TF_CARD")
                .version("0.1.0")
                .description("Virtual TF/SD card (SPI + SDIO 4-bit) / 虚拟 TF/SD 卡")
                .author("RP2350-rs Team")
                .abi_version(ABI_VERSION)
                .min_host_version("0.1.0")
                .has_ui(true)
                .tags(vec!["storage".to_string()])
                .build(),
            ports_spi,
            ports_sdio,
            mode: TfCardMode::Spi,
        }
    }
}

impl RpPlugin for TfCardPlugin {
    fn metadata(&self) -> &PluginMetadata { &self.metadata }

    fn init(&mut self, ctx: &mut dyn PluginContext) -> Result<(), PluginError> {
        ctx.log_info("TF Card plugin initialized / TF 卡插件已初始化");
        Ok(())
    }

    fn tick(&mut self, _ctx: &mut dyn PluginContext, _cycles: u64) {
        // SD state machine tick / SD 状态机推进
    }

    fn shutdown(&mut self) {}

    fn state_snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "name": "TF_CARD",
            "mode": self.mode.as_str(),
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
        Some(
            ConfigSchema::new(
                "TF Card",
                "SD card interface settings / SD 卡接口设置",
            )
            .with_property(
                "mode",
                serde_json::json!({ "type": "string", "enum": ["spi", "sdio"] }),
                true,
            )
            .with_property(
                "image_path",
                serde_json::json!({ "type": "string" }),
                false,
            ),
        )
    }

    fn apply_config(&mut self, config: &serde_json::Value) -> Result<(), ConfigError> {
        if let Some(mode) = config.get("mode").and_then(|v| v.as_str()) {
            self.mode = match mode {
                "spi" => TfCardMode::Spi,
                "sdio" => TfCardMode::Sdio,
                other => {
                    return Err(ConfigError::InvalidType {
                        field: "mode".into(),
                        expected: "\"spi\" | \"sdio\"".into(),
                        actual: other.to_string(),
                    })
                }
            };
        }
        Ok(())
    }

    // GUI（可选）：render_ui 仅在 `gui` 特性下存在；无头模式永不调用。
    // GUI (optional): render_ui only exists with the `gui` feature;
    // never called in headless mode.
}

// 手工 ABI 导出，同 LED 插件 / manual ABI exports, same as the LED plugin
```

---

## Headless-Only Plugin / 纯无头插件（no GUI）

```rust
// plugins/my-protocol-analyzer/src/lib.rs — 例 / example
// A plugin that only collects data — no UI, no GPIO ports.

use rp2350_plugin::abi::ABI_VERSION;
use rp2350_plugin::{PluginContext, PluginError, PluginMetadata, RpPlugin};
use rp2350_virt_periph::{DevicePlugin, DevicePort};

pub struct ProtocolAnalyzer {
    metadata: PluginMetadata,
    packets: usize,
}

impl RpPlugin for ProtocolAnalyzer {
    fn metadata(&self) -> &PluginMetadata { &self.metadata }
    fn init(&mut self, _ctx: &mut dyn PluginContext) -> Result<(), PluginError> { Ok(()) }
    fn tick(&mut self, _ctx: &mut dyn PluginContext, _cycles: u64) { /* sniff bus */ }
    fn shutdown(&mut self) {}

    fn state_snapshot(&self) -> serde_json::Value {
        serde_json::json!({ "packets": self.packets })
    }
}

impl DevicePlugin for ProtocolAnalyzer {
    // No ports → on_gpio_change never called / 无端口 → on_gpio_change 永不调用
    fn ports(&self) -> &[DevicePort] { &[] }
    fn ports_mut(&mut self) -> &mut [DevicePort] { &mut [] }
    fn on_gpio_change(&mut self, _port_index: usize, _level: bool) {}
}

// 手工 ABI 导出：is_device 返回 1（它仍是 DevicePlugin）
// manual ABI exports: is_device returns 1 (it is still a DevicePlugin)
```

---

## Build & Install / 编译与安装

```bash
# Build the first-party plugin / 编译第一方插件
cargo build --release -p rp2350-led-plugin
# → target/release/rp2350_led_plugin.dll (Windows) / librp2350_led_plugin.so / .dylib
```

Loading today / 当前的加载方式：

**A. Directory scan (CLI) / 目录扫描（CLI）**

```bash
cargo build --release -p rp2350-led-plugin
cargo run --release -p rp2350-cli -- list-devices --plugin-dir target/release
# com.rp2350-rs.led / LED / has_ui=false / ports=[LED]
cargo run --release -p rp2350-cli -- wire --plugin-dir target/release \
      --device com.rp2350-rs.led --port 0 --gpio 25
# com.rp2350-rs.led / LED / port 0 / GPIO25
```

`list-devices` / `wire` call `DeviceManager::scan_and_load(dir)`: the directory is
enumerated by platform extension (`dll` / `so` / `dylib`), each library is
instantiated through `PluginLoader::load_device`, and the device ID becomes
`metadata().id` (falling back to the library file stem). Duplicate IDs and
libraries that fail to load are logged with `log::warn!` and skipped.
`list-devices` / `wire` 调用 `DeviceManager::scan_and_load(dir)`：按平台扩展名
（`dll` / `so` / `dylib`）枚举目录，经 `PluginLoader::load_device` 实例化每个库，
设备 ID 取 `metadata().id`（退回库文件名主干）。重复 ID 与加载失败的库会被
`log::warn!` 记录后跳过。

> **One-shot / 一次性**：these subcommands exit immediately after printing, so
> the devices, `SafeLibrary` handles and wiring are released at process exit.
> There is no wiring persistence and no emulator attach yet.
> 这些子命令打印后立即退出，设备、`SafeLibrary` 句柄与接线在进程结束时释放；
> 目前没有接线持久化，也不会挂接模拟器运行。

**B. Programmatic API / 程序化 API**

```rust
let mut dm = DeviceManager::new();                       // 默认搜索路径 "plugins"
dm.load_plugin("led0".to_string(),                       // id → DeviceError::AlreadyExists 重复即报错
               "target/release/rp2350_led_plugin.dll")?; // 内部走 PluginLoader::load_device
dm.wire("led0", 0, 25)?;                                 // 端口 0 → GPIO25

// 或整目录加载：ID 取自 metadata().id / or scan a whole directory (ID from metadata)
let loaded = dm.scan_and_load("target/release");         // Vec<(DeviceId, PathBuf)>
```

- `PluginLoader::scan()` / `scan_and_load()` enumerate candidate libraries without
  instantiating devices; `DeviceManager::scan_and_load` is the device-level entry
  point used by the CLI.
  `PluginLoader::scan()` / `scan_and_load()` 只枚举/加载库而**不实例化设备**；
  设备级入口是 `DeviceManager::scan_and_load`，CLI 即使用该入口。
- Hot reload landed as a synchronous `DeviceManager::reload` (see below); a
  persistent daemon and MCP wiring remain planned.
  热重载已以同步 `DeviceManager::reload` 落地（见下节）；常驻守护进程与 MCP
  接线仍为规划项。

---

## Hot Reload / 热重载

`DeviceManager::reload(dir)` re-scans the plugin directory and diffs it against
the libraries the manager already holds. It returns a `ReloadReport`:
`DeviceManager::reload(dir)` 重新扫描插件目录，并与管理器已持有的库做差异比较，
返回 `ReloadReport`：

```rust
pub struct ReloadReport {
    pub added: Vec<String>,              // newly loaded / 新加载
    pub removed: Vec<String>,            // file gone, device dropped / 文件消失，设备已移除
    pub reloaded: Vec<String>,           // changed, unloaded + loaded again / 已变化，卸载后重载
    pub failed: Vec<(String, String)>,   // path + error message / 路径 + 错误信息
}
```

```rust
let mut dm = DeviceManager::new();
dm.scan_and_load("target/release");                 // initial scan
let report = dm.reload(Path::new("target/release")); // rescan + diff
for line in report.render_lines() {
    println!("{line}");                             // "[reload] ~ …", "[reload] summary: …"
}
```

**How change detection works / 变更检测方式**

- Libraries are keyed by `normalized directory + file name`, so a renamed file
  legitimately shows up as `removed` + `added`.
  库以“规范化目录 + 文件名”为键，因此改名会正常表现为 `removed` + `added`。
- Each library records a `Fingerprint { len, modified }` at load time
  (`scan_with_fingerprints` exposes the same scanner for tests).
  每次加载都会记录 `Fingerprint { len, modified }`（`scan_with_fingerprints`
  对测试暴露同一扫描器）。
- A file whose length or modification time differs is **reloaded**; a file whose
  modification time cannot be read is treated as changed (fail-safe).
  长度或修改时间变化的文件会被**重载**；修改时间不可读时按“已变化”处理
  （失败安全）。
- Unchanged libraries keep their instance, device ID and registration order.
  未变化的库保持原实例、设备 ID 与注册顺序。
- In-process devices (`register`) and devices loaded from another directory are
  never touched. 进程内注册的设备以及来自其它目录的设备不受影响。

**Ordering and failure semantics / 顺序与失败语义**

- Removals are processed first, then changed libraries, then new ones.
  先处理删除，再处理变化，最后处理新增。
- For a removal or a reload the device instance is shut down and dropped
  **before** the library is unloaded (`ManagedDevice` drops `plugin` before
  `library` by field declaration order). 删除或重载时，设备实例先关闭并释放，
  然后才卸载库（`ManagedDevice` 按字段声明顺序先释放 `plugin` 再释放 `library`）。
- A library that fails to load is reported in `failed`; the manager stays usable
  and every other device keeps running. The failed library's device is *not*
  restored (it was already unloaded), and the next `reload` retries it.
  加载失败的库记入 `failed`；管理器继续可用，其它设备照常运行。失败库对应的
  设备**不会**恢复（库已卸载），下一次 `reload` 会重试。

**Constraints / 限制**

| Constraint / 限制 | Reason / 原因 |
|-------------------|---------------|
| Windows locks loaded DLLs / Windows 锁定已加载 DLL | The host cannot overwrite a plugin in place; rebuild to a new name, or drop the device first. 宿主无法原地覆盖插件；请重新构建为新文件名，或先释放设备。 |
| Drop before load / 必须先 drop 再 load | `libloading` refuses to map a second copy from a locked path; reload unloads the old library first. `libloading` 不会从被锁路径再映射一份；重载会先卸载旧库。 |
| Synchronous, no concurrency / 同步、无并发 | The host must not run `tick` / `on_gpio_change` / queries while `reload` runs; there is no lock inside the manager. 执行 `reload` 时宿主不得并发运行 `tick`、`on_gpio_change` 或查询；管理器内部没有锁。 |
| No state migration / 无状态迁移 | A reloaded plugin starts from a fresh instance; wiring is kept only when the device ID is unchanged. 重载后的插件是全新实例；仅当设备 ID 不变时接线才保留。 |
| File-level only / 仅文件级 | No cross-process replacement and no ABI change are detected beyond the existing ABI version check. 不做跨进程替换；ABI 变化仍只由现有 ABI 版本检查发现。 |

Wiring note: the wiring table is keyed by device ID, so a reloaded library that
keeps its `metadata().id` keeps its wiring; a library that changes its ID loses it.
接线说明：接线表以设备 ID 为键，因此重载后保持 `metadata().id` 的库会保留接线；
改变 ID 的库会丢失接线。

CLI usage / CLI 用法:

```bash
# One reload pass, print the report, exit / 执行一次重载，打印报告后退出
rp2350 list-devices --plugin-dir target/release --reload

# Keep polling (once per second) until Ctrl-C / 每秒轮询直到 Ctrl-C
rp2350 list-devices --plugin-dir target/release --watch
```

---

## ABI Compatibility Checklist / ABI 兼容性检查清单

For a plugin to load successfully, it must:
插件要成功加载，必须：

| Requirement | Check |
|-------------|-------|
| Export `rp2350_device_plugin_create` → `*mut dyn DevicePlugin` | 手工 `#[unsafe(no_mangle)]` 导出（参考 `plugins/led/src/lib.rs`） |
| Export `rp2350_plugin_version` → `u32` matching host ABI | Currently version = 1 |
| Export `rp2350_plugin_is_device` → `u32` (1 = DevicePlugin) | 手工 `#[unsafe(no_mangle)]` 导出 |
| Crate declares `#![allow(improper_ctypes_definitions)]` | `*mut dyn DevicePlugin` crosses the dylib boundary |
| Plugin struct is `Send + Sync` | Required by trait bounds |
| Same `RpPlugin` trait definition as host | Must use same `rp2350-plugin` crate version |
| Same `DevicePlugin` trait definition as host | Must use same `rp2350-virt-periph` crate version |

> 没有 destroy 导出：宿主用 `Box::from_raw` 回收实例，`SafeLibrary` RAII 关闭库。
> No destroy export: the host reclaims the instance via `Box::from_raw`, and
> `SafeLibrary` RAII closes the library.

---

## Testing Plugins / 测试插件

Real pattern from `plugins/led/src/lib.rs` (in-process, no dylib needed):
真实模式，来自 `plugins/led/src/lib.rs`（进程内测试，无需动态库）：

```rust
#[test]
fn device_manager_drives_led() {
    let mut manager = DeviceManager::new();
    manager
        .register("led0".to_string(), Box::new(LedPlugin::new()))
        .unwrap();
    manager.wire("led0", 0, 25).unwrap();

    let mut pins =
        std::array::from_fn::<GpioPin, 48, _>(|i| GpioPin::new(u8::try_from(i).unwrap()));
    pins[25].set_direction(PinDirection::Output);
    pins[25].set_output(true);
    manager.sync_gpio(&mut pins);

    let query = manager.mcp_query_all();
    assert_eq!(query["led0"]["on"], true);
    assert_eq!(query["led0"]["transitions"], 1);
}
```

End-to-end dylib loading is covered by `plugins/led/tests/dynamic_load.rs`.
端到端动态库加载由 `plugins/led/tests/dynamic_load.rs` 覆盖。

Planned MCP-facing assertions (not yet implemented / 规划中，尚未实现)：
`mcp_configure` / `mcp_query_device` are planned DeviceManager additions; today
queries go through `mcp_query_all()`.
`mcp_configure` / `mcp_query_device` 为规划中的 DeviceManager 能力；目前查询统一走
`mcp_query_all()`。

---

## Best Practices / 最佳实践

1. **Headless-first** — develop and test through the headless path (CLI / MCP / in-process tests). GUI is optional decoration.
   **无头优先** — 通过无头路径（CLI / MCP / 进程内测试）开发和测试。GUI 是可选装饰。

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
