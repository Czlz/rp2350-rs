# RP2350 Emulator — Rust (rp2350-rs)

> **EN**: A precise cycle-level emulator for the RP2350 microcontroller, implemented in Rust.
> **ZH**: 基于 Rust 实现的 RP2350 微控制器精确周期级模拟器。

---

## Project Overview / 项目总览

| Item | Detail |
|------|--------|
| **Chip** | RP2350 (Raspberry Pi Pico 2) |
| **Phase 1** | ARM Cortex-M33 dual-core (complete) |
| **Phase 2** | RISC-V Hazard3 dual-core (future) |
| **GUI** | egui 0.37 (eframe + wgpu) — Virtual Electronics Lab |
| **Debug** | GDB RSP Server (TCP :3333), multi-core vCont |
| **Plugins** | libloading + abi_stable — dynamic .dll/.so/.dylib loading |
| **Platform** | Windows / Linux / macOS / Web (WASM planned) |
| **Performance** | Target 200+ MCPS (single core, release build) |
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
│   ├── rp2350-core/              # chip topology, address bus, clock tree, IRQ matrix
│   ├── rp2350-cpu/               # CPU abstraction traits (CpuCore, CpuState, CpuArch)
│   ├── rp2350-arm/               # Cortex-M33 core implementation (registers, stub)
│   ├── rp2350-periph/            # built-in silicon peripherals (Peripheral trait)
│   ├── rp2350-virt-periph/       # Virtual Device Manager (DevicePlugin, PluginLoader, WiringTable, DeviceManager)
│   ├── rp2350-plugin/            # plugin base trait (RpPlugin, PluginContext, PluginMetadata)
│   ├── rp2350-ffi/               # FFI safety wrapper (SafeLibrary — libloading RAII)
│   ├── rp2350-gdb/               # GDB RSP Server (stub)
│   ├── rp2350-egui/              # egui GUI (Virtual Lab + canvas + panels — stub)
│   ├── rp2350-cli/               # CLI entry point (clap run/gdb/wire/list-devices/mcp-serve)
│   └── rp2350-mcp/               # MCP Server (AgentFuse integration — stub)
├── plugins/                      # ★ all virtual devices as independent plugins
│   ├── led/                      # first-party: LED
│   ├── tfcard/                   # first-party: TF Card (SPI + SDIO)
│   ├── uart-terminal/            # first-party: UART Terminal
│   ├── debug-probe/              # first-party: Debug Probe
│   ├── psram/                    # first-party: PSRAM
│   ├── qspi-flash/               # first-party: QSPI Flash
│   ├── third-party/              # ★ third-party plugin directory
│   └── user-guide.md             # plugin development guide
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
cargo run -p rp2350-cli -- run --file firmware.elf --arch arm --display

# Start GDB server only
cargo run -p rp2350-cli -- gdb --port 3333 --file firmware.elf

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
| [implementation-plan.md](implementation-plan.md) | Phased implementation plan with milestones |
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