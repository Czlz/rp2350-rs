# rp2350-rs — RP2350 Emulator in Rust

[![CI](https://github.com/Czlz/rp2350-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/Czlz/rp2350-rs/actions/workflows/ci.yml)
[![Deploy](https://github.com/Czlz/rp2350-rs/actions/workflows/release.yml/badge.svg)](https://github.com/Czlz/rp2350-rs/actions/workflows/release.yml)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)

> **EN**: A precise cycle-level emulator for the RP2350 microcontroller (Raspberry Pi Pico 2), implemented in Rust.  
> **ZH**: 基于 Rust 实现的 RP2350 微控制器精确周期级模拟器（树莓派 Pico 2）。

**🚀 Live Demo (Web):** [czlz.github.io/rp2350-rs](https://czlz.github.io/rp2350-rs/)

---

## Features / 特性

- **Dual-core ARM Cortex-M33** with Thumb-16/32, DSP, FPU (RISC-V in future)
- **PIO** state machine emulation (3 blocks × 4 SMs) with assembler
- **HSTX** pixel stream output with real-time egui video display
- **DMA** 16-channel with chaining and DREQ handshake
- **QSPI Flash / PSRAM** XIP support
- **GDB RSP Server** with multi-core debugging (vCont protocol)
- **Plugin system** — virtual devices as `.dll`/`.so` dynamic libraries
- **Virtual Electronics Lab** — drag-and-drop GPIO wiring with egui

---

## Quick Start / 快速开始

```bash
# Build all crates
cargo build --release

# Run emulator with firmware
cargo run -p rp2350-cli -- run --file firmware.elf

# Start with GDB server
cargo run -p rp2350-cli -- gdb --port 3333 --file firmware.elf

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
│   ├── rp2350-core/              # chip topology, address bus, GPIO, IRQ, clocks
│   ├── rp2350-cpu/               # CPU abstraction traits
│   ├── rp2350-arm/               # Cortex-M33 implementation
│   ├── rp2350-periph/            # silicon peripherals (PIO, HSTX, DMA, XIP...)
│   ├── rp2350-virt-periph/       # Virtual Device Manager (DevicePlugin trait, wiring)
│   ├── rp2350-plugin/            # Base plugin trait (RpPlugin, PluginContext)
│   ├── rp2350-ffi/               # SafeLibrary RAII wrapper (libloading)
│   ├── rp2350-gdb/               # GDB RSP Server
│   ├── rp2350-egui/              # egui GUI (Virtual Lab + HSTX + panels)
│   ├── rp2350-cli/               # CLI entry point
│   └── rp2350-mcp/               # MCP Server (AgentFuse integration)
├── plugins/
│   ├── led/                      # LED virtual device plugin
│   ├── tfcard/                   # TF Card (SPI + SDIO) plugin
│   ├── uart-terminal/            # UART Terminal plugin
│   ├── debug-probe/              # Debug Probe plugin
│   ├── psram/                    # PSRAM plugin
│   ├── qspi-flash/               # QSPI Flash plugin
│   └── third-party/              # third-party plugin directory
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