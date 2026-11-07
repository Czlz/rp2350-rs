# Changelog / 变更记录

> Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
> Version: [SemVer](https://semver.org/)

---

## [Unreleased]

### Planned / 计划中
- ARM Cortex-M33 dual-core simulation
- PIO (3 blocks, 4 SM each) complete instruction set
- HSTX pixel capture + egui video output
- QSPI Flash XIP + PSRAM XIP
- egui Virtual Electronics Lab with drag-wire GPIO connection
- Virtual peripherals: LED, TF Card (SPI+SDIO), UART Terminal, Debug Probe, PSRAM, QSPI Flash
- Plugin system (libloading + abi_stable)
- GDB RSP Server (multi-core, vCont, breakpoints, watchpoints)
- DSP extension (ARMv8-M)

---

## [0.1.0] — 2026-11-08

### Added
- Workspace scaffold with 11 crates (`cargo build` passes, zero errors)
- `rp2350-core`: `AddressBus` trait, `MemoryRegion` trait, `BusErrorKind`, `CoreError`, `RP2350Chip`, `SimulationClock`, `IrqMatrix` (256 IRQs), `GpioPin` (48 pins), base types
- `rp2350-cpu`: `CpuCore` trait (dyn-safe, no generic parameter), `CpuState` enum, `CpuArch` enum
- `rp2350-arm`: `CortexM33Core` stub implementing `CpuCore`, `CortexM33Registers` (R0-R15, MSP/PSP, XPSR, CONTROL, PRIMASK, BASEPRI, FAULTMASK)
- `rp2350-periph`: `Peripheral` trait (extends `MemoryRegion`), peripheral stubs
- `rp2350-virt-periph`: `DevicePlugin` trait (extends `RpPlugin`), `DevicePort`, `PortDirection`, `ConfigSchema`, `ConfigError`, `DeviceError`
- `rp2350-virt-periph`: `DeviceManager` full implementation (register, wire, unwire, tick_all, sync_gpio, mcp_query_all) with unit tests
- `rp2350-virt-periph`: `WiringTable` — bidirectional (device, port) ↔ GPIO pin mapping
- `rp2350-virt-periph`: `PluginLoader` — directory scanning + SafeLibrary loading with error handling
- `rp2350-plugin`: `RpPlugin` trait (metadata, init, tick, shutdown, state_snapshot), `PluginContext` trait (dyn-safe via `PluginContextExt`), `PluginMetadata` builder
- `rp2350-plugin`: `BusDevice` trait — minimal bus device interface for plugins
- `rp2350-ffi`: `SafeLibrary` RAII wrapper around `libloading::Library` (Send + Sync)
- `rp2350-gdb`: `GdbServer` stub, `GdbTarget` type alias
- `rp2350-egui`: `EmuApp` stub, `canvas`/`panels` module stubs, egui 0.37 integration
- `rp2350-cli`: `clap` CLI with `run`, `gdb`, `wire`, `list-devices`, `mcp-serve` subcommands
- `rp2350-mcp`: `McpServer` stub (query_all, configure), AgentFuse integration foundation
- GitHub Actions: CI (3-platform build/clippy/test/wasm-check) + Release (multi-target + WASM + GitHub Pages auto-deploy)
- Project documentation: `doc/README.md`, `doc/architecture.md`, `doc/dependencies.md`, `doc/changelog.md`, `doc/implementation-plan.md`, `doc/plugins.md`, crate-level docs, FFI spec
- Root `README.md` with features, quick start, crate structure, platform support
- `.gitignore` (target, IDE, OS artifacts, WASM outputs, plugin binaries)

### Design Decisions / 设计决策
- **Dyn-safe traits**: `CpuCore` uses `&mut dyn AddressBus` instead of generic `B` to enable trait objects
- **Dyn-safe plugins**: `PluginContext` splits into `PluginContext` (trait) + `PluginContextExt` (extension) for dyn compatibility
- **No duplicate state_snapshot**: `DevicePlugin` inherits `state_snapshot()` from `RpPlugin` — single authoritative state interface
- **PluginContext::BusDevice**: Minimal bus interface for plugins — full `MemoryRegion` stays in `rp2350-core`
- **Edition 2024**: All crates use Rust Edition 2024 with `resolver = "3"`
- **Feature gate `dep:` prefix**: `gui = ["dep:egui"]` for correct workspace feature resolution