# Architecture Design / 架构设计

> **Version**: 3.0 | **Date**: 2026-11-08 | **Status**: Review

---

## 1. Overall Architecture / 总体架构

```
┌─────────────────────────────────────────────────────────────────┐
│                        rp2350-egui (GUI)                         │
│  ┌──────────┬──────────────────┬────────────┬────────────────┐  │
│  │Left Panel│  Virtual Lab     │Right Panel │Bottom Terminal │  │
│  │(Plugins) │  Canvas (Drag+   │(Registers  │(UART Console)  │  │
│  │          │   Wire Connect)  │ HSTX Mem)  │                │  │
│  └──────────┴──────────────────┴────────────┴────────────────┘  │
├─────────────────────────────────────────────────────────────────┤
│                   rp2350-cli (CLI Entry Point)                    │
│  run | gdb | mcp-serve | screenshot | wire | list-devices       │
├───────────────────────┬─────────────────────────────────────────┤
│   rp2350-gdb (GDB)    │     rp2350-mcp (AgentFuse MCP Server)   │
│   server  protocol    │     server  query_all  configure        │
├───────────────────────┴─────────────────────────────────────────┤
│                   rp2350-virt-periph (DeviceManager)            │
│  PluginLoader │ WiringTable │ DeviceRegistry │ MCP Query API   │
│  (libloading) │ (pin↔port)  │ (Vec<Device>)  │ (JSON-RPC)      │
├─────────────────────────────────────────────────────────────────┤
│  Plugins/ directory:                                            │
│  ┌──────────┬──────────┬───────────────┬──────────┬──────────┐ │
│  │led.dll   │tfcard.dll│uart-term.dll  │psram.dll │flash.dll │ │
│  │RpPlugin  │RpPlugin  │RpPlugin       │RpPlugin  │RpPlugin  │ │
│  │DevicePlg │DevicePlg │DevicePlg      │DevicePlg │DevicePlg │ │
│  └──────────┴──────────┴───────────────┴──────────┴──────────┘ │
├─────────────────────────────────────────────────────────────────┤
│                     RP2350Chip (rp2350-core)                      │
│  ┌─────────┬──────────┬──────────┬──────────┬────────────────┐  │
│  │CortexM33│CortexM33 │  DMA     │   PIO    │  DeviceManager │  │
│  │  Core0  │  Core1   │16 ch     │ 3 blocks │  (wiring+bus)  │  │
│  ├─────────┴──────────┴──────────┴──────────┴────────────────┤  │
│  │                   Address Bus (统一地址空间)                  │  │
│  │  BOOTROM │ XIP Flash │ PSRAM │ SRAM │ APB Peripherals │  │  │
│  └────────────────────────────────────────────────────────────┘  │
├─────────────────────────────────────────────────────────────────┤
│  rp2350-periph (silicon peripherals)                            │
│  PIO HSTX DMA UART SPI I2C PWM ADC GPIO SIO CLOCKS              │
│  TIMER WATCHDOG USB QMI XIP_CTRL ...                            │
├─────────────────────────────────────────────────────────────────┤
│  rp2350-cpu (CPU abstraction)     rp2350-arm (Cortex-M33 impl)  │
│  CpuCore CpuState CpuArch         registers stub                │
├─────────────────────────────────────────────────────────────────┤
│  rp2350-plugin (base trait)        rp2350-ffi (SafeLibrary)     │
│  RpPlugin PluginContext            RAII libloading wrapper       │
└─────────────────────────────────────────────────────────────────┘
```

---

## 2. Core Trait System / 核心 Trait 体系

### 2.1 MemoryRegion + AddressBus — 总线设备

The fundamental abstraction for all memory-mapped components. Two-tier: `MemoryRegion` for individual regions, `AddressBus` for the address routing layer.
所有内存映射设备的基础抽象。两层架构：`MemoryRegion` 用于单个区域，`AddressBus` 用于地址路由层。

```rust
/// A single memory-mapped region (RP2350 datasheet address range).
/// 单个内存映射区域。
pub trait MemoryRegion: Send + Sync {
    fn name(&self) -> &str;
    fn base_address(&self) -> u32;
    fn size(&self) -> usize;
    fn contains(&self, addr: u32) -> bool;
    fn read_u8(&mut self, addr: u32, cycles: &mut u64) -> BusResult<u8>;
    fn write_u8(&mut self, addr: u32, value: u8, cycles: &mut u64) -> BusResult<()>;
    // Optional: override for wider access
    fn read_u16(&mut self, addr: u32, cycles: &mut u64) -> BusResult<u16> { ... }
    fn read_u32(&mut self, addr: u32, cycles: &mut u64) -> BusResult<u32> { ... }
    fn write_u16(&mut self, addr: u32, value: u16, cycles: &mut u64) -> BusResult<()> { ... }
    fn write_u32(&mut self, addr: u32, value: u32, cycles: &mut u64) -> BusResult<()> { ... }
}

/// The central address bus that routes accesses to the correct region.
/// 中央地址总线，将内存访问路由到正确区域。
pub trait AddressBus: Send + Sync {
    fn register_region(&mut self, region: Box<dyn MemoryRegion>) -> BusResult<()>;
    fn unregister_region(&mut self, name: &str);
    fn find_region_mut(&mut self, addr: Address) -> Option<&mut dyn MemoryRegion>;
    // read_u8/u16/u32, write_u8/u16/u32 with auto-route + default impls
}
```

### 2.2 CpuCore — CPU 抽象

```rust
pub trait CpuCore: Send + Sync {
    fn cpu_id(&self) -> CpuId;
    fn arch(&self) -> CpuArch;
    fn init(&mut self, bus: &mut dyn AddressBus) -> Result<(), CoreError>;
    fn run_cycles(&mut self, bus: &mut dyn AddressBus, cycles: CycleCount) -> (CycleCount, CpuState);
    fn step(&mut self, bus: &mut dyn AddressBus) -> Result<CpuState, CoreError>;
    fn signal_interrupt(&mut self, irq: IrqNumber);
    fn servicing_irq(&self) -> Option<IrqNumber>;
    fn pc(&self) -> u32;
    fn read_register(&self, index: u8) -> u32;
    fn write_register(&mut self, index: u8, value: u32);
    fn reset(&mut self);
    fn description(&self) -> String;
}

### 2.3 RpPlugin — 基础插件契约

Defined in `rp2350-plugin` crate. All plugins implement this. Zero GUI dependency.
定义在 `rp2350-plugin` crate，所有插件均实现。零 GUI 依赖。

```rust
pub trait RpPlugin: Send + Sync {
    fn metadata(&self) -> &PluginMetadata;
    fn init(&mut self, ctx: &mut dyn PluginContext) -> Result<(), PluginError>;
    fn tick(&mut self, ctx: &mut dyn PluginContext, cycles: u64);
    fn shutdown(&mut self);
    fn state_snapshot(&self) -> serde_json::Value { json!({}) }
}
```

### 2.4 PluginContext — 插件上下文 (dyn-safe)

```rust
pub trait PluginContext: Send {
    fn log(&self, level: LogLevel, message: &str);
    fn gpio_read(&self, pin: u8) -> bool;
    fn gpio_write(&mut self, pin: u8, level: bool);
    fn gpio_set_direction(&mut self, pin: u8, direction: PinDirection);
    fn schedule_callback(&mut self, cycles: u64, tag: &str);
    // Internal: erased service access (use PluginContextExt for typed access)
    fn get_service_erased(&self) -> Option<&dyn Any>;
}

pub trait PluginContextExt: PluginContext {
    fn get_service<T: Any>(&self) -> Option<&T>;
}
impl<T: PluginContext + ?Sized> PluginContextExt for T {}
```

### 2.5 DevicePlugin — 设备插件契约

Defined in `rp2350-virt-periph` crate. Extends `RpPlugin` with GPIO ports, optional config, optional GUI.
定义在 `rp2350-virt-periph` crate。扩展 `RpPlugin`，增加 GPIO 端口、可选配置、可选 GUI。

```rust
pub trait DevicePlugin: RpPlugin {
    fn ports(&self) -> &[DevicePort];
    fn ports_mut(&mut self) -> &mut [DevicePort];
    fn on_gpio_change(&mut self, port_index: usize, level: bool);
    fn config_schema(&self) -> Option<ConfigSchema> { None }
    fn apply_config(&mut self, config: &serde_json::Value) -> Result<(), ConfigError> { Ok(()) }
    fn has_ui(&self) -> bool { false }
    #[cfg(feature = "gui")]
    fn render_ui(&mut self, ui: &mut egui::Ui) {}
}
```

> **Note**: `state_snapshot()` is inherited from `RpPlugin` — no duplicate definition. `DevicePlugin` does **not** redefine it, avoiding trait ambiguity.
> **注意**: `state_snapshot()` 从 `RpPlugin` 继承，无重复定义。`DevicePlugin` **不**重新定义它，避免 trait 歧义。

### 2.6 Trait Hierarchy / Trait 层次

```
RpPlugin (rp2350-plugin)
  ├── DevicePlugin (rp2350-virt-periph)
  │     ├── LedPlugin       (plugins/led)
  │     ├── TfCardPlugin    (plugins/tfcard)
  │     ├── UartTermPlugin  (plugins/uart-terminal)
  │     ├── DebugProbePlg   (plugins/debug-probe)
  │     ├── PsramPlugin     (plugins/psram)
  │     ├── QspiFlashPlg    (plugins/qspi-flash)
  │     └── CustomPlugin    (plugins/third-party/...)
  │
  └── Pure RpPlugin (headless-only, no ports, no GUI)
        ├── ProtocolAnalyzer (hypothetical)
        └── TraceCollector   (hypothetical)
```

---

## 3. Address Space Layout / 地址空间布局

| Region | Start | End | Size | Device |
|--------|-------|-----|------|--------|
| BOOTROM | 0x0000_0000 | 0x0000_7FFF | 32KB | BootROM |
| XIP Flash | 0x1000_0000 | 0x10FF_FFFF | 16MB | QSPI Flash XIP |
| XIP PSRAM | 0x1100_0000 | 0x11FF_FFFF | 16MB | PSRAM XIP alias |
| SRAM0-9 | 0x2000_0000 | 0x2008_1FFF | 520KB | Internal SRAM |
| APB | 0x4000_0000 | 0x4FFF_FFFF | 256MB | Peripherals |
| USB DPRAM | 0x5010_0000 | 0x5010_3FFF | 16KB | USB Controller RAM |
| SIO | 0xD000_0000 | 0xD000_01FF | 512B | Single-cycle IO |
| PPB0 | 0xE000_0000 | 0xE00F_FFFF | 1MB | Private Peripheral Bus (Core 0) |
| PPB1 | 0xE100_0000 | 0xE10F_FFFF | 1MB | Private Peripheral Bus (Core 1) |

### APB Peripheral Map / APB 外设映射

| Offset | Base | Peripheral |
|--------|------|------------|
| 0x00000 | 0x4000_0000 | SYSINFO |
| 0x04000 | 0x4000_4000 | SYSCFG |
| 0x08000 | 0x4000_8000 | CLOCKS |
| 0x10000 | 0x4001_0000 | PSM |
| 0x14000 | 0x4001_4000 | HSTX_CTRL |
| 0x18000 | 0x4001_8000 | HSTX_FIFO |
| 0x20000 | 0x4002_0000 | RESETS |
| 0x30000 | 0x4003_0000 | PIO0 |
| 0x34000 | 0x4003_4000 | XIP_CTRL (QSPI) |
| 0x38000 | 0x4003_8000 | XIP_CTRL |
| 0x3C000 | 0x4003_C000 | XIP_QMI |
| 0x40000 | 0x4004_0000 | PIO1 |
| 0x50000 | 0x4005_0000 | PIO2 |
| 0xD0000 | 0x400D_0000 | DMA |
| 0xE0000 | 0x400E_0000 | USB CTRL |

---

## 4. Execution Loop / 执行循环

```
┌─ Simulator::run() ──────────────────────────────────────────┐
│                                                               │
│  for batch in 0..BATCH_SIZE:                                  │
│                                                               │
│    ┌─ core0.step(chip) ─┐   ┌─ core1.step(chip) ─┐           │
│    │ fetch → decode →   │   │ fetch → decode →   │           │
│    │ execute → wb      │   │ execute → wb      │           │
│    └────────────────────┘   └────────────────────┘           │
│                                                               │
│    peripherals.tick(cycles)   // APB peripherals              │
│    device_manager.tick_all()  // all loaded DevicePlugins     │
│    device_manager.notify_gpio() // GPIO → device routing      │
│    irq.process_pending()      // interrupt dispatch           │
│                                                               │
│    if debug.halt_requested: break                             │
│    if hstx.frame_ready: display.render(frame)                 │
│                                                               │
│  total_cycles += batch                                       │
└───────────────────────────────────────────────────────────────┘
```

Multi-core interleaving: core0 → core1 per cycle, configurable ratio.
多核交错执行：每周期 core0 → core1，交错比例可配置。

---

## 5. Data Flow / 数据流

### 5.1 CPU Memory Access

```
CPU ld/st → RP2350Chip::read_u32(addr)
  → AddressBus::route(addr)
    ├── 0x0000_0000-0x0000_7FFF → BOOTROM
    ├── 0x1000_0000-0x10FF_FFFF → XIP → QMI → QspiFlash.read()
    ├── 0x1100_0000-0x11FF_FFFF → XIP → QMI → Psram.read()
    ├── 0x2000_0000-0x2008_1FFF → SRAM
    ├── 0x4000_0000-0x4FFF_FFFF → APB peripherals
    ├── 0x5010_0000-0x5010_3FFF → USB DPRAM
    ├── 0xD000_0000-0xD000_01FF → SIO
    └── 0xE000_0000-0xE10F_FFFF → PPB (per-core)
```

### 5.2 GPIO Signal Flow

```
Firmware: gpio_put(pin, 1) or gpio_set_function(pin, PIO0)
     │
     ▼
GpioPin::set_function() → updates pin state
     │
     ├── PIO SM reads pin input → SM input path
     ├── SIO output → pin.value → GPIO_OUT register
     │
     ▼
RP2350Chip::notify_gpio_change(pin, level)
     │
     ▼
DeviceManager::notify_gpio_change(pin, level)
     │
     ├── WiringTable::get_ports(pin) → [(device_name, port_idx), ...]
     │
     ├── For each wired port:
     │     ├── device.ports_mut()[port_idx].current_level = level
     │     ├── device.on_gpio_change(port_idx, level)
     │     └── (device drives output if PortDirection::Output)
     │           └── DeviceManager::device_drives_gpio(device, port, level)
     │                 └── ctx.gpio_write(pin, level) → RP2350 GPIO_IN
     │
     └── egui lab canvas → update pin color + port color + wire color

Headless/MCP:
  DeviceManager::mcp_query_device("LED") → {"level": true, "gpio_pin": 25}
  DeviceManager::mcp_query_all() → all device states aggregate
```

### 5.3 HSTX Video Pipeline

```
Firmware: DMA → HSTX FIFO write (8 lanes, 32-bit each)
     │
     ▼
Hstx::write_u32() → frame_buffer.push(pixel_data)
     │
     ▼ (frame end or half-buffer)
HstxDecoder::decode(raw_u32, format) → RGBA8 Vec
     │  formats: RGB565, RGB888, YCbCr422, Raw8, Raw16
     │
     ▼ (via mpsc channel to egui thread)
egui::ColorImage → TextureHandle → ui.image()
     │
     ▼
wgpu swapchain → window present
```

---

## 6. Plugin Loading Flow / 插件加载流程

```
Startup:
  CLI: rp2350 run firmware.elf [--headless] [--mcp-port 9000]
       │
       ▼
  DeviceManager::new(plugin_dirs, headless)
       │
       ▼
  DeviceManager::scan_and_load()
       ├── Walk plugins/ directory
       ├── For each .dll/.so/.dylib:
       │     ├── SafeLibrary::new(path)            // dlopen / LoadLibrary
       │     ├── dlsym("rp2350_plugin_version")    // check ABI
       │     ├── dlsym("rp2350_plugin_create")     // → Box<dyn RpPlugin>
       │     ├── dlsym("rp2350_plugin_is_device")  // → if 1, cast to DevicePlugin
       │     ├── plugin.init(ctx)                  // init with emulator ctx
       │     └── Register in devices[]             // store LoadedDevice
       └── Return Vec<PluginLoadResult>

GUI mode:                        Headless/MCP mode:
  For each device:                 For each device:
    if has_ui():                     state_snapshot() → JSON
      render_ui() each frame         exposed via MCP JSON-RPC
                                     also --dump-state CLI flag

Runtime (every cycle):
  DeviceManager::tick_all(ctx, cycles)
       ├── For each device:
       │     └── device.plugin.tick(ctx, cycles)
       │
  DeviceManager::notify_gpio_change(pin, level)
       ├── Lookup WiringTable for (device, port) → pin
       ├── Call device.on_gpio_change(port_idx, level)
       └── Update device port state

On shutdown:
  DeviceManager::unload_all()
       ├── For each device: plugin.shutdown()
       └── SafeLibrary::drop()  // RAII: dlclose / FreeLibrary
```

---

## 7. Error Handling Strategy / 错误处理策略

| Layer | Crate | Strategy |
|-------|-------|----------|
| Library | all lib crates | `thiserror` — derive `Error` for typed error enums |
| CLI binary | rp2350-cli | `anyhow::Result` — with `context()` for chain |
| FFI boundary | rp2350-ffi | Custom `FfiError` wrapping `libloading::Error` |
| Plugin ABI | rp2350-plugin | `PluginError` enum — all variants cross FFI-safe |
| GDB protocol | rp2350-gdb | `GdbError` — protocol-level errors + IO failures |

---

## 8. Multi-Core Debug Model / 多核调试模型

```
GDB Thread Model:
  thread 1 = core0 (Cortex-M33)
  thread 2 = core1 (Cortex-M33)

GDB Commands:
  Hc-1           → select core0 as current
  Hc-2           → select core1 as current
  vCont;c        → continue all cores
  vCont;s:1      → single-step core0 only
  vCont;c:2      → continue core1 only
  ?              → halt reason (core-specific)

Monitor Commands:
  monitor pio    → dump PIO state (all blocks)
  monitor gpio   → dump GPIO pin states
  monitor dma    → dump DMA channel states
  monitor hstx   → dump HSTX frame info
  monitor info   → system info summary
```