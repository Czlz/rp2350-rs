# RP2350 Emulator — Rust (rp2350-rs)

> **EN**: A precise cycle-level emulator for the RP2350 microcontroller, implemented in Rust.
> **ZH**: 基于 Rust 实现的 RP2350 微控制器精确周期级模拟器。

---

## Project Overview / 项目总览

| Item | Detail |
|------|--------|
| **Chip** | RP2350 (Raspberry Pi Pico 2) |
| **ARM** | Cortex-M33：Thumb-16/32 + FPv5-SP FPU（单精度算术/比较/转换、FPCCR 惰性压栈）+ NVIC/异常/SysTick + GPIO/DCP 协处理器 + DSP/SIMD/饱和指令；真实双核 core0+core1（SIO/PSM/每核 PPB 与中断路由） |
| **RISC-V** | Hazard3 RV32IMAC + C（已实现，CLI `--arch riscv` 可用） |
| **USB** | 设备控制器模型 + 主机桥 + raw REPL：真实 MicroPython UF2 → 官方 `run-perfbench.py` 全链路可跑（Arm/RISC-V 各 24/24） |
| **PIO** | 3 blocks × 4 SMs：9 类指令、autopush/autopull、wrap、stall、FIFO join、引脚/side-set/delay/CLKDIV、共享句柄、IRQ 路由、DMA DREQ 门控、`FUNCSEL` 功能选择 |
| **DMA** | 16 通道 + 控制块 + AL 触发 + byte/half/word + RING/CHAIN_TO/DREQ/IRQ + sniffer（CRC/XOR/SUM）+ `BUSCTRL` 性能计数器 + CLI `Machine` 泵送（64 步批量仲裁） |
| **HSTX** | APB 控制 + AHB FIFO、8 字 FIFO/WOF、RAW/TMDS 命令、像素时钟节拍、宿主 drain/scanline、下溢 IRQ；无 TMDS 10 位编码/DVI 位时序 |
| **GUI** | egui 0.37 (eframe + wgpu) — PlayView（画面/键盘/音频/线程开关）+ 虚拟实验室连线画布 + 寄存器/DMA/HSTX/PIO 面板 |
| **Audio** | I2S 采样捕获（`AudioSink`/`AudioTap`）+ `rp2350-audio` 宿主后端（`cpal` 可选，null 降级） |
| **Debug** | GDB RSP：协议层 + TCP 服务 + CLI `gdb`（R0–R15/XPSR、内存、断点、resume、`Z2`/`Z3`/`Z4` 监视点、双核 `vCont`、`target.xml`）；线程化执行器模式下不支持监视点 |
| **MCP** | JSON-RPC 方法层 + HTTP 传输（`POST /mcp`、`GET /health`）+ CLI `mcp-serve` + Bearer 鉴权（无 TLS） |
| **Plugins** | libloading + abi_stable — 目录扫描、热重载（`--reload`/`--watch`）、线程化执行器、`run --plugin-dir/--wire/--plugin-threads` 挂接；LED 与 UART Terminal 插件 |
| **Platform** | Windows / Linux / macOS 原生；Web (WASM) 不属于 V1（见 `v1-scope.md`） |
| **Performance** | 默认确定性 ≈5–8.5 M 步/秒（按工作负载窗口，见 [`performance.md`](performance.md)）；`--core-threads on` 约 1.8–2.0×（非位可复现） |
| **Repository** | [github.com/Czlz/rp2350-rs](https://github.com/Czlz/rp2350-rs) |
| **Live Demo** | [czlz.github.io/rp2350-rs](https://czlz.github.io/rp2350-rs/) |

### Reference / 参考基准

- **Hardware**: RP2350 Datasheet v1.1
- **Architecture**: ARMv8-M Architecture Reference Manual
- **Software**: [rp2350js](https://github.com/c1570/rp2350js) — register maps, behavioral cross-validation

---

## Crate Structure / Crate 结构

```
rp2350-rs/
├── Cargo.toml                    # workspace root
├── crates/
│   ├── rp2350-core/              # address bus, memory regions, UF2/ELF loaders, MMIO framework, GPIO/IRQ/clock
│   ├── rp2350-cpu/               # CPU abstraction traits (CpuCore, CpuState, CpuArch)
│   ├── rp2350-arm/               # Cortex-M33 core implementation (Thumb-16/32, FPU, NVIC, exceptions)
│   ├── rp2350-riscv/             # Hazard3 core implementation (RV32IMAC + C, CSRs, Xh3irq)
│   ├── rp2350-bootrom/           # synthetic boot ROM images + host trap dispatch
│   ├── rp2350-periph/            # built-in silicon peripherals — Stage-4 boot stubs (Peripheral trait)
│   ├── rp2350-usb/               # USB device controller model (DPRAM, endpoint handshake, IRQ14)
│   ├── rp2350-usb-host/          # host-side bridge: enumeration, CDC, raw REPL, perfbench
│   ├── rp2350-audio/             # host audio backend (cpal optional, null fallback)
│   ├── rp2350-virt-periph/       # Virtual Device Manager (DevicePlugin, PluginLoader, WiringTable, DeviceManager)
│   ├── rp2350-plugin/            # plugin base trait (RpPlugin, PluginContext, PluginMetadata, ABI)
│   ├── rp2350-ffi/               # FFI safety wrapper (SafeLibrary — libloading RAII)
│   ├── rp2350-gdb/               # GDB RSP: protocol layer, TCP server, target adapters
│   ├── rp2350-egui/              # egui GUI: PlayView, Virtual Lab canvas, peripheral panels
│   ├── rp2350-cli/               # CLI: run / repl / play / perfbench / gdb / devices / mcp-serve
│   └── rp2350-mcp/               # MCP Server: JSON-RPC methods + HTTP transport
├── plugins/                      # ★ all virtual devices as independent plugins
│   ├── led/                      # first-party: LED
│   └── uart-terminal/            # first-party: UART terminal (TX bit decode / RX bit encode)
├── tests/
├── doc/
└── .github/
    └── workflows/                # CI (ci.yml) + Release (release.yml)
```

---

## Quick Start / 快速开始

```bash
# Build all crates
cargo build --all-features

# Run all tests
cargo test --all-features

# Run emulator with firmware
cargo run -q -p rp2350-cli -- run --file firmware.elf --arch arm

# Boot real MicroPython and open its USB CDC raw REPL / 启动真实 MicroPython 并打开 raw REPL
cargo run -q -p rp2350-cli -- repl --file RPI_PICO2-20260824-v1.29.0.uf2 --exec "print('hello')"

# Run a MicroPython performance benchmark / 运行 MicroPython 性能基准
cargo run -q -p rp2350-cli -- perfbench --file RPI_PICO2-20260824-v1.29.0.uf2 --bench bm_fannkuch.py --benchrun benchrun.py

# Start the GDB RSP server / 启动 GDB RSP 服务器
cargo run -q -p rp2350-cli -- gdb --port 3333 --file firmware.elf

# List device plugins / create a wiring entry / 列出设备插件、建立接线
cargo run -q -p rp2350-cli -- list-devices --plugin-dir target/release
cargo run -q -p rp2350-cli -- wire --plugin-dir target/release --device com.rp2350-rs.led --port 0 --gpio 25

# Start the MCP HTTP server / 启动 MCP HTTP 服务器
cargo run -q -p rp2350-cli -- mcp-serve --port 8080

# Example: boot the MicroPython RP2350 UF2
cargo run -q -p rp2350-cli -- run --file RPI_PICO2-20260824-v1.29.0.uf2 --trace-steps 40

# Quality gates (zero warning)
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings -W clippy::pedantic
cargo test --all-features
cargo doc --no-deps --document-private-items
```

---

## Key Documents / 核心文档

| Document | Description |
|----------|-------------|
| [architecture.md](architecture.md) | Architecture design, trait system, data flow |
| [v1-scope.md](v1-scope.md) | V1 范围/精度声明：包含与不包含、已知近似、确定性与线程模式、验证矩阵 |
| [images/README.md](images/README.md) | 截图资源与逐条复现命令（真实固件 REPL、GUI、游戏帧） |
| [implementation-plan.md](implementation-plan.md) | V1 release plan (work packages + firmware gates) |
| [lessons.md](lessons.md) | Lessons learned and engineering pitfalls |
| [playability-plan.md](playability-plan.md) | emerald 可玩性：吞吐、帧率、音频欠载与已知限制 |
| [threading-plan.md](threading-plan.md) | 双核线程执行器设计、测量与限制 |
| [performance.md](performance.md) | 按工作负载窗口的性能测量与优化记录 |
| [perfbench-report.md](perfbench-report.md) | Official `run-perfbench.py` results on both MicroPython firmware images |
| [example/](../example/README.md) | **示例项目**：GBA 等真实固件的运行说明、引脚/外设配置与验证报告（`example/<name>/doc/`）|
| [usb-device-model.md](usb-device-model.md) | USB device controller model + host bridge design |
| [dependencies.md](dependencies.md) | Third-party crate inventory, version, usage |
| [changelog.md](changelog.md) | Version changelog (Keep a Changelog) |
| [plugins.md](plugins.md) | Plugin directory structure, development guide, reference impls |
| [ffi/plugin-abi.md](ffi/plugin-abi.md) | Plugin ABI specification |
| [crates/rp2350-core.md](crates/rp2350-core.md) | Core crate public API |
| [crates/rp2350-arm.md](crates/rp2350-arm.md) | ARM Cortex-M33 crate public API |
| [crates/rp2350-periph.md](crates/rp2350-periph.md) | Peripherals crate public API |
| [crates/rp2350-virt-periph.md](crates/rp2350-virt-periph.md) | Virtual Device Manager (DevicePlugin trait, DeviceManager) |
| [crates/rp2350-plugin.md](crates/rp2350-plugin.md) | Plugin base trait (RpPlugin, PluginContext, ABI macros) |
| [crates/rp2350-gdb.md](crates/rp2350-gdb.md) | GDB server crate public API |
| [crates/rp2350-egui.md](crates/rp2350-egui.md) | egui GUI crate public API |
| [crates/rp2350-riscv.md](crates/rp2350-riscv.md) | RISC-V Hazard3 crate public API |
| [crates/rp2350-bootrom.md](crates/rp2350-bootrom.md) | Synthetic boot ROM crate public API |
| [crates/rp2350-usb.md](crates/rp2350-usb.md) | USB device controller crate public API |
| [crates/rp2350-usb-host.md](crates/rp2350-usb-host.md) | USB host bridge / raw REPL crate public API |
| [crates/rp2350-cli.md](crates/rp2350-cli.md) | CLI subcommands (`run`/`repl`/`perfbench`/`gdb`/`mcp-serve`) |
| [crates/rp2350-mcp.md](crates/rp2350-mcp.md) | MCP JSON-RPC methods + HTTP transport / Bearer auth |