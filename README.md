# rp2350-rs — RP2350 Emulator in Rust

[![CI](https://github.com/Czlz/rp2350-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/Czlz/rp2350-rs/actions/workflows/ci.yml)
[![Deploy](https://github.com/Czlz/rp2350-rs/actions/workflows/release.yml/badge.svg)](https://github.com/Czlz/rp2350-rs/actions/workflows/release.yml)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)

> **EN**: A precise cycle-level emulator for the RP2350 microcontroller (Raspberry Pi Pico 2), implemented in Rust.  
> **ZH**: 基于 Rust 实现的 RP2350 微控制器精确周期级模拟器（树莓派 Pico 2）。

**🚀 Live Demo (Web):** [czlz.github.io/rp2350-rs](https://czlz.github.io/rp2350-rs/)

---

## Features / 特性

> 状态标注 / Status: ✅ 已实现 implemented · 🚧 部分 partial · ⏳ 规划 planned（阶段计划见 [implementation-plan](doc/implementation-plan.md) 与 [reference-gap-analysis](doc/reference-gap-analysis.md)）

- ✅ **ARM Cortex-M33** — Thumb-16/32 解码器、FPv5-SP FPU（单精度算术/比较/转换/VSEL/VRINT、FPCCR 惰性压栈）、NVIC/异常/SysTick、GPIO(CP0)/DCP(CP4) 协处理器、DSP 并行 SIMD（SADD/SSUB/UADD/USUB、SXTB16/UXTB16、USAD8/USADA8）
- ✅ **RISC-V Hazard3** — RV32IMAC + C 压缩指令、machine CSRs、Xh3irq 外部中断、picobin 启动
- ✅ **USB 全链路** — 设备控制器模型 + 主机桥枚举 + raw REPL：真实 MicroPython UF2 可启动、进入 REPL 并跑通官方 `run-perfbench.py`（Arm/RISC-V 各 24/24）
- ✅ **合成 bootrom + XIP Flash** — 32 KB 合成 ROM（`rom_table_lookup`/flash 擦写/sysinfo/reboot）、16 MB Flash XIP、非缓存别名与维护窗口
- ✅ **启动外设桩** — RESETS/CLOCKS/XOSC/PLL/TIMER/SIO/IO_BANK0/PADS/UART0/PPB/BOOTRAM/TICKS/POWMAN/IO_QSPI/XIP_QMI
- 🚧 **PIO** — 3 blocks × 4 SMs 寄存器块 + 状态机执行（JMP/WAIT/IN/OUT/PUSH/PULL/MOV/IRQ/SET、autopush/autopull、wrap、stall）；PINCTRL 引脚映射、side-set、delay、CLKDIV、FIFO join 已接入；DMA 握手/IRQ 路由/主循环 tick 待补
- ✅ **GDB RSP** — 协议层 + TCP 服务 + CLI `gdb` 接线（R0–R15/XPSR、总线内存、PC 断点、resume）；watchpoint/多核 `vCont`/`target.xml` 待补
- ✅ **MCP** — JSON-RPC 方法层 + HTTP/1.1 传输（`POST /mcp`、`GET /health`）+ CLI `mcp-serve` + Bearer 鉴权
- ✅ **插件系统** — libloading 动态加载 `.dll`/`.so`/`.dylib`，目录扫描 + CLI `list-devices`/`wire`，LED 与 UART Terminal 插件
- 🚧 **双核** — core1 常驻 WFE（SIO FIFO/门铃语义保留），`DualCoreM33` 调度脚手架已落地；PSM/SIO/CLI 接线待补
- ⏳ **DMA**（16 通道链式 + DREQ）、**HSTX** 视频输出、**Virtual Electronics Lab**（egui 拖线连线）、**PSRAM** — 规划中

---

## Quick Start / 快速开始

```bash
# Build all crates
cargo build --release

# Run emulator with firmware (ELF) / 运行模拟器（ELF）
cargo run -p rp2350-cli -- run --file firmware.elf

# Boot a real MicroPython UF2 with boot trace / 启动真实 MicroPython UF2 并打印启动轨迹
cargo run -p rp2350-cli -- run --file RPI_PICO2-*.uf2 --arch arm --trace-steps 40

# RISC-V (Hazard3) firmware / RISC-V（Hazard3）固件
cargo run -p rp2350-cli -- run --file firmware.elf --arch riscv

# Boot real MicroPython and open its USB CDC raw REPL / 启动真实 MicroPython 并打开 USB CDC raw REPL
cargo run -p rp2350-cli -- repl --file RPI_PICO2-*.uf2 --exec "print('hello')"

# Run a MicroPython performance benchmark / 运行 MicroPython 性能基准
cargo run -p rp2350-cli -- perfbench --file RPI_PICO2-*.uf2 --bench bm_fannkuch.py --benchrun benchrun.py

# Start the GDB RSP server / 启动 GDB RSP 服务器
cargo run -p rp2350-cli -- gdb --port 3333 --file firmware.elf

# List device plugins and create a wiring entry / 列出设备插件并建立接线
cargo run -p rp2350-cli -- list-devices --plugin-dir target/release
cargo run -p rp2350-cli -- wire --plugin-dir target/release --device com.rp2350-rs.led --port 0 --gpio 25

# Start the MCP HTTP server (POST /mcp, GET /health) / 启动 MCP HTTP 服务器
cargo run -p rp2350-cli -- mcp-serve --port 8080

# Run all tests
cargo test --all-features

# Quality gates
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo doc --no-deps
```

---

## Browser Demo / 浏览器演示

```bash
# Install trunk (WASM bundler)
cargo install trunk

# Build and serve
trunk serve

# Open http://localhost:8080
```

---

## Project Structure / 项目结构

```
rp2350-rs/
├── crates/
│   ├── rp2350-core/              # address bus, memory regions, UF2/ELF loaders, MMIO framework, GPIO/IRQ/clock
│   ├── rp2350-cpu/               # CPU abstraction traits (CpuCore, CpuState, CpuArch)
│   ├── rp2350-arm/               # Cortex-M33: Thumb-16/32, FPv5-SP FPU, NVIC, exceptions, coprocessors
│   ├── rp2350-riscv/             # Hazard3: RV32IMAC + C, machine CSRs, Xh3irq, picobin boot
│   ├── rp2350-bootrom/           # synthetic 32 KB boot ROM + host trap dispatch
│   ├── rp2350-periph/            # Stage-4 boot peripheral stubs (RESETS/CLOCKS/.../XIP_QMI)
│   ├── rp2350-usb/               # USB device controller model (DPRAM, endpoint handshake, IRQ14)
│   ├── rp2350-usb-host/          # host-side bridge: enumeration, CDC, raw REPL, perfbench
│   ├── rp2350-virt-periph/       # DeviceManager, DevicePlugin, WiringTable, plugin loading
│   ├── rp2350-plugin/            # Base plugin trait (RpPlugin, PluginContext, ABI exports)
│   ├── rp2350-ffi/               # SafeLibrary RAII wrapper (libloading)
│   ├── rp2350-gdb/               # GDB RSP: protocol layer, TCP server, target adapters
│   ├── rp2350-egui/              # egui GUI (register panels; Virtual Lab planned)
│   ├── rp2350-cli/               # CLI: run / repl / perfbench / gdb / list-devices / wire / mcp-serve
│   └── rp2350-mcp/               # MCP Server: JSON-RPC methods + HTTP transport
├── plugins/
│   ├── led/                      # LED virtual device plugin (first-party)
│   └── uart-terminal/            # UART terminal plugin (bit-timed TX decode / RX encode)
└── doc/                          # design documents (architecture, plan, APIs)
```

---

## Platform Support / 平台支持

| Platform | Status | Notes |
|----------|--------|-------|
| Windows (x86_64) | ✅ Native | `.dll` plugin loading |
| Linux (x86_64) | ✅ Native | `.so` plugin loading |
| macOS (arm64/x86_64) | ✅ Native | `.dylib` plugin loading |
| Web (WASM) | 🚧 Planned | Static plugin linking via features |

---

## Documentation / 文档

Full design documents in `doc/`:
- [Architecture](doc/architecture.md) — trait system, data flow, address space
- [Implementation Plan](doc/implementation-plan.md) — phased tasks with milestones
- [Reference Gap Analysis](doc/reference-gap-analysis.md) — 验收主线（Stages 0–6）：真实 MicroPython UF2 → USB CDC → perfbench
- [perfbench Report](doc/perfbench-report.md) — official `run-perfbench.py` results on the Arm and RISC-V MicroPython images
- [USB Device Model](doc/usb-device-model.md) — USB 设备控制器与主机桥设计
- [Plugin Guide](doc/plugins.md) — how to create virtual device plugins
- [API Reference](doc/crates/) — per-crate public API documentation

---

## License / 许可证

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

---

## Acknowledgments / 致谢

- [rp2350js](https://github.com/c1570/rp2350js) — reference implementation for register maps and behavioral verification
- RP2350 Datasheet v1.1
- ARMv8-M Architecture Reference Manual