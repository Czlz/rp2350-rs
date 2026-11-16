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
│  rp2350-cpu (CPU abstraction)   rp2350-arm / rp2350-riscv impls  │
│  CpuCore CpuState CpuArch       Cortex-M33 | Hazard3 RV32IMAC    │
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
    fn system_write(&mut self, addr: u32, value: u32);
    fn system_read(&mut self, addr: u32) -> Option<u32>;
    fn pc(&self) -> u32;
    fn read_register(&self, index: u8) -> u32;
    fn write_register(&mut self, index: u8, value: u32);
    fn reset(&mut self);
    fn description(&self) -> String;

    // 带默认实现的方法（无状态核心可直接用）/ default-implemented methods:
    fn state(&self) -> CpuState { CpuState::Reset }
    fn xpsr(&self) -> u32 { 0 }
    fn total_cycles(&self) -> CycleCount { 0 }
}
```

#### Chip identification / 芯片识别

`rp2350-core::chip_family` provides `ChipFamily`, `ChipCapabilities` and the `ChipInfo` trait. `RP2350Chip` reports the RP2350 capability set (520 KB SRAM, 16 MB XIP window, Hazard3 available). `RP2040Chip` is a constructible, identifiable stub: `boot`, `step` and memory access all return `CoreError::UnsupportedChip` so callers see an explicit error instead of fabricated behaviour.
`rp2350-core::chip_family` 提供 `ChipFamily`、`ChipCapabilities` 与 `ChipInfo` trait。`RP2350Chip` 报告 RP2350 的能力集（520 KB SRAM、16 MB XIP 窗口、可用 Hazard3）。`RP2040Chip` 是可构造、可识别的桩：`boot`、`step` 与内存访问均返回 `CoreError::UnsupportedChip`，让调用方看到明确错误而不是伪造行为。

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
    fn log_info(&self, message: &str);
    fn log_warn(&self, message: &str);
    fn log_error(&self, message: &str);
    fn current_cycles(&self) -> u64;
    fn schedule_callback(&mut self, cycles: u64, tag: &str);
    // Internal: erased service access (use PluginContextExt for typed access)
    fn get_service_erased(&self) -> Option<&dyn Any>;
}

pub trait PluginContextExt: PluginContext {
    fn get_service<T: Any>(&self) -> Option<&T>;
}
impl<T: PluginContext + ?Sized> PluginContextExt for T {}
```

> GPIO 不经由 `PluginContext`：设备端口的读写由 `DeviceManager::wire` 接线、
> `DeviceManager::sync_gpio(&mut [GpioPin])` 在每次 tick 后把 Output 端口电平
> 写回 GPIO 引脚。插件视角的总线设备接口为 `BusDevice { base_address, size }`
> （`rp2350-plugin::context`）。
> GPIO does not go through `PluginContext`: device ports are wired via
> `DeviceManager::wire`, and `DeviceManager::sync_gpio(&mut [GpioPin])` writes
> Output port levels back to the GPIO pins after each tick. The plugin-side bus
> device interface is `BusDevice { base_address, size }`.

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
| BOOTROM | 0x0000_0000 | 0x0000_7FFF | 32KB | Synthetic boot ROM (read-only, host trap) |
| XIP Flash | 0x1000_0000 | 0x10FF_FFFF | 16MB | QSPI Flash XIP (cached) |
| XIP Flash (no-cache alias) | 0x1400_0000 | 0x14FF_FFFF | 16MB | Same flash bytes, non-cached window |
| XIP PSRAM | 0x1100_0000 | 0x11FF_FFFF | 16MB | PSRAM window on QMI CS1 (`rp2350-periph::psram::Psram`) |
| SRAM0-9 | 0x2000_0000 | 0x2008_1FFF | 520KB | Internal SRAM |
| APB | 0x4000_0000 | 0x4FFF_FFFF | 256MB | Peripherals |
| USB DPRAM | 0x5010_0000 | 0x5010_0FFF | 4KB | USB Controller RAM |
| USBCTRL REGS | 0x5011_0000 | 0x5011_0FFF | 4KB | USB SIE control/status |
| SIO | 0xD000_0000 | 0xD000_0FFF | 4KB | Single-cycle IO |
| PPB0 | 0xE000_0000 | 0xE00F_FFFF | 1MB | Private Peripheral Bus (Core 0) |
| PPB1 | 0xE100_0000 | 0xE10F_FFFF | 1MB | Private Peripheral Bus (Core 1) |

XIP 别名依据 pico-sdk `addressmap.h`（RP2350）：`XIP_BASE = 0x1000_0000`、
`XIP_END = 0x1400_0000`、`XIP_NOCACHE_NOALLOC_BASE = 0x1400_0000`、
`XIP_NOCACHE_NOALLOC_END = 0x1800_0000`、`XIP_MAINTENANCE_BASE = 0x1800_0000`。
缓存区 `0x1000_0000..0x1400_0000` 是四个 16 MB 的 XIP_MAIN 窗口，
`0x1100_0000` 属于 XIP_MAIN1（PSRAM/CS1 窗口），**不是** Flash CS0 的镜像，
因此不映射到 Flash。它由 `rp2350-periph::psram::Psram` 单独建模（`mount_psram`
挂载，默认 8 MB、上限 16 MB，客户机可读写），不随 `BootPeripherals::mount`
自动安装；未挂载时访问返回 `BusErrorKind::NoDevice`。本实现只补 Flash CS0 的
非缓存窗口 `0x1400_0000..0x1500_0000`，PSRAM 的非缓存别名
`0x1500_0000` 与 `0x1800_0000` 维护窗口暂不建模。

boot ROM 与 Flash 是两个独立地址区：厂商镜像不能随仓库分发，因此
`0x0000_0000..0x0000_7FFF` 由 `rp2350-bootrom` 提供合成只读镜像（magic 位于
`0x10`、函数表指针位于 `0x14`、`rom_table_lookup` 指针位于 `0x16`）。表项指向
`udf` 占位桩，宿主在指令步进前按 PC 拦截并执行等价语义（`rom_table_lookup`、
Flash 擦写、系统信息、重启等）。ARM 启动从 Flash XIP 基地址 `0x1000_0000`
读取 SP/PC，并把 VTOR 设为 `0x1000_0000`；`0x0000_0000` 不再镜像 Flash 向量。

### APB Peripheral Map / APB 外设映射

Stage 4 只建模启动路径需要的外设；基地址已按 RP2350 数据手册核对（见
`crates/rp2350-periph/src/map.rs`），未建模偏移由 `MmioRouter` 告警一次并读 0。

| Base | Peripheral | Stage 4 |
|------|------------|---------|
| 0x4000_0000 | SYSINFO | 未建模 |
| 0x4001_0000 | CLOCKS | 时钟树：10 个 `CLK_*` 通道，`CLK_*_SELECTED` 实时推导 |
| 0x4002_0000 | RESETS | 桩：`RESET_DONE` 恒读全 1 |
| 0x4002_8000 | IO_BANK0 | 桩：寄存器存储 |
| 0x4003_8000 | PADS_BANK0 | 桩：48 个 pad 复位值 0x56 |
| 0x4004_8000 | XOSC | 12 MHz 晶振：`CTRL`/`STATUS` 解码进时钟树 |
| 0x4005_0000 | PLL_SYS | 125/5/2 → 150 MHz，`LOCK` 实时 |
| 0x4005_8000 | PLL_USB | 100/5/5 → 48 MHz，`LOCK` 实时 |
| 0x4007_0000 | UART0 (PL011) | 桩：写 `DR` 进入宿主串口缓冲 |
| 0x400B_0000 | TIMER0 | 桩：共享微秒时基 + 4 个闹钟 |
| 0x400D_8000 | WATCHDOG | 桩：scratch 存储，`REASON` 读 0 |
| 0x400E_0000 | BOOTRAM | 2 KB 暂存 RAM + `WRITE_ONCE0/1` + `BOOTLOCK0..7`（读即认领、写释放），`STAT` 复位 0xFF |
| 0x4010_8000 | TICKS | 6 个滴答通道；使能后写 `CYCLES` 把该周期计入共享时基 |
| 0xD000_0000 | SIO | 桩：GPIO 输出/回环、FIFO、自旋锁 |
| 0xE000_0000 | PPB (Core 0) | 桩：`CPUID`、`VTOR` 存储 |

### MMIO Framework API / MMIO 框架接口

Stage 4 引入通用挂载接口，任何外部 crate（例如独立的 USB 设备控制器）
只需实现 `MmioDevice` 并用 `mount_device`（或 `MmioRouter::mount`）注册：

```rust
pub struct MmioWindow { pub base: Address, pub size: usize }

pub trait MmioDevice: Send + Sync {
    fn name(&self) -> &str;
    fn window(&self) -> MmioWindow;
    fn read32(&mut self, offset: u32) -> Option<u32>;   // None = 未建模
    fn write32(&mut self, offset: u32, value: u32) -> bool; // false = 未建模
    fn reset(&mut self) {}
    fn irq_number(&self) -> Option<u16> { None }
}

pub fn mount_device(
    bus: &mut dyn AddressBus,
    device: Box<dyn MmioDevice>,
) -> BusResult<MmioWindow>;
```

- 地址窗口：设备在 `window()` 中声明数据手册的固定窗口；`mount_device` 校验
  4 字节对齐、大小为正且为 4 的倍数后注册到总线，重叠由总线返回 `Overlap`。
- `MmioRouter`：把一个大跨度（如 1 MB APB）按子设备窗口分发；未映射偏移只
  告警一次，读返回 0、写被忽略，避免固件因数据中止而停机。
- `MmioRegion`：`MmioDevice` → `MemoryRegion` 适配层，16/32 位访问按整字
  原子分发（不会拆成多次字节副作用）。
- ARM 侧 `CpuId` 等类型从 `rp2350_core` 复用，外部 crate 不需要依赖 CPU 后端。

### APB Atomic Aliases / APB 原子别名

RP2350 的 APB 寄存器按 4 KB 分块：块内前 4 KB 是寄存器本体，后三个 4 KB 窗口
对同一批寄存器做原子操作——`+0x1000` 为 XOR、`+0x2000` 为 SET、`+0x3000` 为 CLR。
`MmioRouter` 统一处理这些窗口，`MmioDevice` 不需要（也不允许）自己实现别名：

- 设备在 `window()` 里只声明本体窗口，且必须落在所属 `0x4000` 块的前 4 KB 内。
- 别名写折叠成对子设备的一次读-改-写：SET 为 `|=`、CLR 为 `&= !`、XOR 为 `^=`；
  底层仍走设备自己的 `read32`/`write32`，设备永远看不到别名偏移。
- 别名读返回底层寄存器的当前值。
- 挂载时若子窗口会跨越自己或其它子设备的别名跨度，`mount` 返回
  `Overlap`/`DeviceError`，不会静默共享。

pico-sdk 大量依赖这套别名（`hw_set_bits`/`hw_clear_bits` 会写 `RESETS`、
`PLL_*_PWR`、`CLOCKS_*_CTRL`、`XOSC_CTRL`），缺了它这些写会被静默丢弃，固件随后
在等待对应状态位时死等。`0x4004_A000` 这类地址不是新设备，而是 `XOSC` 的 SET
别名（`0x4004_8000 + 0x2000`）。

### Stalls and the Shared Cycle Sink / 停顿与共享周期汇

有些寄存器写入在硬件上不是存值，而是让处理器"等一段时间"（可编程延时、握手停顿）。
这类设备不做忙等空转，而是持有一个 `CycleSink`（`rp2350-periph/src/cycle.rs`），
把硬件会花费的周期数推入共享 `Timebase`：

- `CycleSink::advance(cycles, sysclk, refclk, generation)` 推进共享时基，
  `TIMER0`、`time_us()` 与上报的仿真时间立即反映这次"停顿"；
  `CycleSink::push_ref(cycles)` 用于已在 `clk_ref` 域的周期（TICKS）。
  `BootPeripherals::advance` 也走同一句柄。
- 句柄可克隆、跨设备复用，后续块直接使用，不需要改 `MmioDevice` trait，也不需要
  CPU 侧新增"取停顿周期"的接口。
- 首个使用者是 `TICKS`：通道使能后写 `CYCLES`，整段 `clk_ref` 周期一次性计入
  定时器域，随后读 `COUNT` 为 0（剩余量已耗尽），忙等循环因此立即通过而仿真时间
  正确前进。

`TICKS` 通道记录步长 `0x0C`（`CTRL`+0x00 / `CYCLES`+0x04 / `COUNT`+0x08），
`CTRL.ENABLE` 未置位时写 `CYCLES` 不生效、不推进时间，避免固件初始化阶段
（先写 `CYCLES`、后置 `ENABLE`）造成虚假的时间跳跃。

### Clock Data Flow / 时钟数据流

时钟树是唯一频率来源，寄存器写入沿 `ClockTree` 立即传播到所有导出接口：

```
XOSC 12 MHz ──┐
ROSC ring/div ─┼─► PLL_CS.REFDIV + PLL_FBDIV_INT + PLL_PRIM
LPOSC 32 kHz ─┘        │
                       ├─► PLL_SYS ──┬─► CLK_SYS (SRC=AUX, AUXSRC=0) ──► CLK_PERI/HSTX (AUXSRC=0)
                       │             └─► CLK_PERI/HSTX (AUXSRC=1)
                       └─► PLL_USB ──┬─► CLK_USB / CLK_ADC (AUXSRC=0)
                                     └─► CLK_REF aux (AUXSRC=0)

CLK_REF: SRC=ROSC_PH / AUX / XOSC / LPOSC ──► CLK_SYS (SRC=CLK_REF)
CLK_GPOUTn: AUXSRC 0..=0xE（PLL、ROSC、XOSC、LPOSC、其它 CLK_*）
CLK_RTC: RP2350 无该分频器，RTC 直接使用 LPOSC
```

- `rp2350-periph`：`ClockTree`（`clock_tree.rs`）持有 XOSC/ROSC/PLL/10 个通道的
  寄存器镜像与解码；`CLOCKS`、`XOSC`、`PLL_SYS`、`PLL_USB`、`ROSC` 共享同一句柄
  （`BootPeripherals::mount` 创建，`clock_tree()` 暴露）。任何写入立即生效，
  `generation()` 单调递增供时序缓存失效。
- 初始状态是 **bootrom 等价**：XOSC 12 MHz、`PLL_SYS` 125/5/2 → 150 MHz、
  `CLK_SYS` = AUX/PLL_SYS；`PLL_USB` 100/5/5 → 48 MHz。`Clocks::reset()` 等恢复
  该状态，不是上电复位值。
- `rp2350-core`：`TimeBase` 是权威的按频率积分器，保存 `cycles`、已结算 `micros`
  与不足一微秒的分子；`advance(cycles, sysclk_hz)` 每次检查实时频率，改频前先按
  **旧**频率结算，零频率停止时间并只告警一次。`SimulationClock` 仅保留为旧的
  累计周期追踪器。
- `rp2350-periph`：`Timebase` 组合 CPU 域（`BootPeripherals::time_us`）与
  `clk_ref` 域（`TIMELR`/`TIMERAWH`，1 µs 节拍），并接收
  `ClockTree::generation()`。SysTick、ADC、POWMAN RTC、UART 与 HSTX 都持有共享
  时钟树/时基；PIO 的一个宿主 tick 是一个 `clk_sys` 周期，`CLKDIV`（16.8 定点）
  在其上分频，写入 `SMx_CLKDIV` 保留分频器相位；`CLKDIV_RESTART` 只重启分频器，`SM_RESTART` 复位 PC/移位/延迟/阻塞/相位；DMA pacing 按真实经过周期记账。
- 消费者与时钟对照见 [`crates/rp2350-periph.md`](crates/rp2350-periph.md) 的
  「Unified time base and consumers」表。SysTick 的唯一真相源是外设侧
  `SysTickCounter`：`SYST_CSR`/`SYST_RVR`/`SYST_CVR`、`COUNTFLAG`、实时时钟源与
  一次性到期锁存都在外设侧；CLI 每次中断泵送最多取走一次
  `BootPeripherals::take_systick_pending()` 并转发给
  `CortexM33Core::pend_systick()`，由核按系统异常 15（向量 `VTOR + 0x3C`）投递。
  机器运行时不回放 SysTick 寄存器，核内自计数路径只服务于没有外设的单元测试，
  因此不存在双计时。The peripheral `SysTickCounter` is the single source of
  truth (registers, `COUNTFLAG`, clock source and the one-shot expiry latch);
  the CLI forwards at most one latch per IRQ pump to `pend_systick()` and the
  core delivers system exception 15. SysTick registers are never replayed into
  the core, so nothing ticks twice.
- 精度测试：12 MHz + 125/5/2 = 150 MHz、252 MHz 超频（126/6/1）、`CLK_SYS` 切
  `REF` + DIV 3 = 4 MHz、`CLK_PERI` = PLL_SYS/2 = 75 MHz、`CLK_USB` 48 MHz、
  非法分频 0 Hz、`generation` 递增规则、总线级 252 MHz 与 `CLK_SYS_SELECTED`
  一致性、默认机器 `PERFCTR` 随 DMA 累加（见
  `crates/rp2350-periph/src/clock_tree.rs`、`clocks.rs`、`boot.rs`、`clock_time.rs`、
  `systick.rs` 单测）。

### Unmodelled Accesses / 未建模访问清单

以下地址在真实固件运行中落到"读 0 / 写忽略"的兜底路径（每个地址只告警一次）。
目前不阻塞已跑通的路径，但属于已知缺口：

| 地址范围 | 归属 | 固件行为 |
|---|---|---|
| `0x400D_0000`–`0x400D_0014` | QMI | 软复位路径读时序/格式寄存器 |
| `0x4004_0004` | PADS_QSPI `GPIO_QSPI_SCLK` | 读 pad 配置 |
| `0x4004_3008`–`0x4004_3014` | PADS_QSPI 之后 0x3000 的窗口（SVD 未列出） | 读 pad 配置 |

`0x400E_0000` 窗口已建模：前 2 KB 仍按普通 RAM 读写，所以固件把
`0x400E_0000..0x400E_0100` 拷到 SRAM `0x2000_4684` 的一次性搬运例程（守卫字节
`0x2000_5020`，随后 `0x1004_CF2C` 跳入副本）拿到的仍是这块暂存区的内容；窗口
尾部按数据手册实现 `WRITE_ONCE0/1`、`BOOTLOCK_STAT` 与 `BOOTLOCK0..7`，因此
`0x1003_2084` 处对 `BOOTLOCK7` 的轮询能一次认领成功。

### Interrupt Number Map / 中断号映射

中断号取自 pico-sdk `intctrl.h`（RP2350），也是 `crates/rp2350-periph/src/map.rs`
中 `*_IRQ` 常量的来源。外部 IRQ `n` 的异常号为 `16 + n`，向量位于
`VTOR + (16 + n) * 4`。

| IRQ | Peripheral | IRQ | Peripheral |
|-----|------------|-----|------------|
| 0..3 | TIMER0_IRQ_0..3 | 21 | IO_IRQ_BANK0 |
| 10..13 | DMA_IRQ_0..3 | 25 | SIO_IRQ_FIFO |
| **14** | **USBCTRL_IRQ** | 26 | SIO_IRQ_BELL |
| 15/16 | PIO0_IRQ_0/1 | 29 | SIO_IRQ_MTIMECMP |
| 17/18 | PIO1_IRQ_0/1 | 30 | CLOCKS_IRQ |
| 19/20 | PIO2_IRQ_0/1 | 31/32 | SPI0_IRQ / SPI1_IRQ |
| 33 | UART0_IRQ | 34 | UART1_IRQ |

### Interrupt Wiring / 中断接线

中断线不插桩到指令执行路径，而由驱动在三个位置搬运状态：

1. **NVIC/VTOR 写回放**：PPB 桩把落在 `0xE000_ED08`（VTOR）、`SCB_ICSR`、
   `SHPR2`/`SHPR3`、`SCR`、`STIR`、`CPACR` 与 `0xE000_E100..=0xE000_E4FF`
   （NVIC）的写记录到共享日志；驱动调用 `CpuCore::system_write` 回放到核。
   SysTick 寄存器（`0xE000_E010..=0xE000_E01F`）**不回放**：外设侧
   `SysTickCounter` 跟随实时时钟树，是计数与 `COUNTFLAG` 的唯一真相源。
2. **外设中断线轮询**：外设把断言状态写入共享 `IrqAssert`（64 位掩码），
   驱动读取 `BootPeripherals::irq_mask()` 并对每条断言的线调用
   `CpuCore::signal_interrupt`。
3. **SysTick 到期转发**：外设计数器的一次性到期锁存由
   `BootPeripherals::take_systick_pending()` 取走，驱动每次泵送最多转发一次给
   `CortexM33Core::pend_systick()`；核按系统异常 15（向量 `VTOR + 0x3C`）
   投递，并用 `[irq] core0 systick at PC=…` 记录该次转发。

核侧在每次 `step` 后检查 `PRIMASK` 与 NVIC 的使能/挂起/活跃状态，满足条件
即按 `16 + irq` 进入外部异常；SysTick（15）与 PendSV（14）是系统异常，
`rp2350-arm` 按（优先级，异常号）统一仲裁。活跃中断不会被再次选中，避免同一
电平在异常返回前自我抢占。

External IRQ lines are polled into each NVIC, while the peripheral SysTick
counter is the single owner of `SYST_CSR`/`COUNTFLAG` and its one-shot expiry
latch. The driver forwards that latch to `CortexM33Core::pend_systick()`, which
enters system exception 15 at `VTOR + 0x3C`; SysTick registers are never
replayed into the core, so the core-side counter cannot double-count.

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
│    device_manager.sync_gpio(&mut pins) // GPIO→device routing │
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
    ├── 0x0000_0000-0x0000_7FFF → BOOTROM (synthetic, read-only, host trap)
    ├── 0x1000_0000-0x10FF_FFFF → XIP → QMI → QspiFlash.read()
    ├── 0x1400_0000-0x14FF_FFFF → XIP no-cache alias → same Flash bytes
    ├── 0x1100_0000-0x11FF_FFFF → PSRAM window (CS1) → psram::Psram
    ├── 0x2000_0000-0x2008_1FFF → SRAM
    ├── 0x4000_0000-0x4FFF_FFFF → APB peripherals
    ├── 0x5010_0000-0x5010_0FFF → USB DPRAM (4 KB)
    ├── 0x5011_0000-0x5011_0FFF → USBCTRL REGS (4 KB)
    ├── 0xD000_0000-0xD000_0FFF → SIO
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
RP2350Chip GPIO 引脚电平（每 tick 由驱动读取）
     │
     ▼
DeviceManager::sync_gpio(&mut [GpioPin])          // 每个 tick 后调用 / called after each tick
     │
     ├── Input → Device（GPIO 输出电平 → 设备输入端口）:
     │     for entry in wiring.all():                      // WiringTable 全部接线
     │       device = devices[entry.device_id]
     │       port   = device.ports()[entry.port_index]
     │       if port.direction ∈ {Input, Bidirectional}:
     │           device.on_gpio_change(entry.port_index, gpio_pin.level())
     │           （插件在自己的实现里更新 port.level —— 主机不直接写端口字段）
     │           (the plugin updates its own port.level — the host never writes port fields)
     │
     ├── Device → Output（设备输出端口 → GPIO 输入电平）:
     │     for (device_id, device) in devices:
     │       for (i, port) in device.ports().enumerate():
     │         if port.direction ∈ {Output, Bidirectional}
     │            && let Some(gpio) = wiring.find_by_device_port(device_id, i):
     │              gpio_pin.set_input(port.level)         // 写回 RP2350 GPIO_IN
     │
     └── GUI 模式：egui 面板读取 GPIO / 端口状态刷新画布（仅可视化）
         (GUI mode: egui panels read GPIO / port state to refresh the canvas)

Headless/MCP:
  DeviceManager::mcp_query_all() → { "<device_id>": state_snapshot(), ... }
  （按 id 汇总每个设备的 state_snapshot；`mcp_query_device` 单设备查询为规划项）
  (aggregates each device's state_snapshot by id; per-device `mcp_query_device` is planned)
```

### 5.3 HSTX Video Pipeline

> ⏳ **规划中（未实现）**：本节是 Phase 5 的目标设计，`rp2350-periph` 目前没有 HSTX 模块。
> ⏳ **Planned (not implemented)**: this section is the Phase 5 target design; `rp2350-periph` has no HSTX module yet.

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
  插件加载：DeviceManager::load_plugin / PluginLoader::scan_and_load
       │
       ▼
  DeviceManager::new()                    // 无参构造；目录扫描在 PluginLoader
       │
       ▼
  DeviceManager::load_plugin(id, path)   // 按路径逐个加载（内部 PluginLoader::load_device）
       ├── Walk plugins/ directory
       ├── For each .dll/.so/.dylib:
       │     ├── SafeLibrary::new(path)            // dlopen / LoadLibrary
       │     ├── dlsym("rp2350_plugin_version")    // 必须等于 ABI_VERSION (1)
       │     ├── dlsym("rp2350_device_plugin_create")     // → Box<dyn DevicePlugin>
       │     ├── dlsym("rp2350_plugin_is_device")  // → 非 1 拒绝（仅设备插件）
       │     ├── Box::from_raw(create())           // 实例化 Box<dyn DevicePlugin>
       │     └── DeviceManager::register(id, plugin) // 存入设备表
       └── 任一符号缺失/版本不符则返回 DeviceError::PluginLoadFailed

（GUI 渲染与 MCP 集成为规划项 / GUI + MCP wiring planned）:
  For each device:                 For each device:
    if has_ui():                     state_snapshot() → JSON
      render_ui() each frame         exposed via MCP JSON-RPC
                                     also --dump-state CLI flag

Runtime (every tick):
  DeviceManager::tick_all(ctx, cycles)
       ├── For each device:
       │     └── device.plugin.tick(ctx, cycles)
       │
  DeviceManager::sync_gpio(&mut [GpioPin])
       ├── Lookup WiringTable for (device, port) → pin
       ├── Call device.on_gpio_change(port_idx, level)
       └── Update device port state

On shutdown:
  DeviceManager::unregister(&id)
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
