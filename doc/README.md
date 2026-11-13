# RP2350 Emulator — Rust (rp2350-rs)

> **EN**: A precise cycle-level emulator for the RP2350 microcontroller, implemented in Rust.
> **ZH**: 基于 Rust 实现的 RP2350 微控制器精确周期级模拟器。

---

## Project Overview / 项目总览

| Item | Detail |
|------|--------|
| **Chip** | RP2350 (Raspberry Pi Pico 2) |
| **ARM** | Cortex-M33：Thumb-16/32 + FPv5-SP FPU + NVIC/异常/SysTick + GPIO/DCP 协处理器（core1 常驻 WFE，真双核排期中） |
| **RISC-V** | Hazard3 RV32IMAC + C（已实现，CLI `--arch riscv` 可用） |
| **USB** | 设备控制器模型 + 主机桥 + raw REPL：真实 MicroPython UF2 → `perfbench` 全链路可跑 |
| **GUI** | egui 0.37 (eframe + wgpu) — 寄存器面板（Virtual Electronics Lab 规划中） |
| **Debug** | GDB RSP Server (TCP :3333) — target 抽象层就绪，服务器为桩 |
| **Plugins** | libloading + abi_stable — dynamic .dll/.so/.dylib loading（LED 参考插件已就绪） |
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
│   ├── rp2350-core/              # address bus, memory regions, UF2/ELF loaders, MMIO framework, GPIO/IRQ/clock
│   ├── rp2350-cpu/               # CPU abstraction traits (CpuCore, CpuState, CpuArch)
│   ├── rp2350-arm/               # Cortex-M33 core implementation (Thumb-16/32, FPU, NVIC, exceptions)
│   ├── rp2350-riscv/             # Hazard3 core implementation (RV32IMAC + C, CSRs, Xh3irq)
│   ├── rp2350-bootrom/           # synthetic boot ROM images + host trap dispatch
│   ├── rp2350-periph/            # built-in silicon peripherals — Stage-4 boot stubs (Peripheral trait)
│   ├── rp2350-usb/               # USB device controller model (DPRAM, endpoint handshake, IRQ14)
│   ├── rp2350-usb-host/          # host-side bridge: enumeration, CDC, raw REPL, perfbench
│   ├── rp2350-virt-periph/       # Virtual Device Manager (DevicePlugin, PluginLoader, WiringTable, DeviceManager)
│   ├── rp2350-plugin/            # plugin base trait (RpPlugin, PluginContext, PluginMetadata, ABI)
│   ├── rp2350-ffi/               # FFI safety wrapper (SafeLibrary — libloading RAII)
│   ├── rp2350-gdb/               # GDB RSP (target trait + Cortex-M33 adapter; server stub)
│   ├── rp2350-egui/              # egui GUI (register panels; Virtual Lab planned)
│   ├── rp2350-cli/               # CLI entry point (clap: run/repl/perfbench; gdb/wire/list-devices/mcp-serve stubs)
│   └── rp2350-mcp/               # MCP Server (AgentFuse integration — stub)
├── plugins/                      # ★ all virtual devices as independent plugins
│   └── led/                      # first-party: LED（TF Card / UART Terminal / Debug Probe / PSRAM / QSPI Flash 规划中）
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

# Start GDB server only (stub / 桩)
cargo run -q -p rp2350-cli -- gdb --port 3333 --file firmware.elf

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
| [implementation-plan.md](implementation-plan.md) | Phased implementation plan with milestones |
| [reference-gap-analysis.md](reference-gap-analysis.md) | Upstream gap analysis, UF2/RISC-V/RP2040/USB plan |
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