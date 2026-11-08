# Changelog / 变更记录

> Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
> Version: [SemVer](https://semver.org/)

---

## [Unreleased]

### Added / 新增
- `rp2350-core`: `FlashRegion` XIP window (`0x1000_0000`, 16 MB) — guest writes return `AccessDenied`, host loaders use the new `MemoryRegion::load_u8` / `AddressBus::load_u8` path
- `rp2350-core`: generic MMIO framework (`MmioWindow`, `MmioDevice`, `MmioRegion`, `MmioRouter`, `RegisterFile`, `mount_device`) with fixed datasheet windows, overlap rejection and one-shot warnings for unmodelled offsets; 16/32-bit accesses dispatch as single register operations
- `rp2350-periph`: Stage-4 boot stubs — RESETS, CLOCKS, XOSC, PLL_SYS/USB, WATCHDOG, TIMER0, SIO, IO_BANK0, PADS_BANK0, UART0 and PPB, wired through `BootPeripherals::mount` with a shared microsecond `Timebase` and a host-visible `UartSink`
- `rp2350-cli`: `run --file <uf2|elf> [--arch arm|riscv] [--max-cycles N] [--trace-steps N]` — builds the Stage-4 bus (BootROM stub, XIP flash, SRAM, APB), boots ARM from `0x1000_0000`, prints a boot trace, drains UART0 output and reports the halt reason
- `rp2350-arm`: Thumb-16 decoder groups remapped to the ARMv8-M table (shift-immediate, MOV/CMP/ADD/SUB immediate, ALU/high-register, literal, register-offset, word/byte/halfword immediates, SP-relative), added `CBZ`/`CBNZ`/`POP`/`SETEND`/`CPS`/`BKPT`/`IT`, and fixed PC-relative branch/literal offsets for the pre-advanced PC
- `rp2350-core`: UF2 container loader (`uf2` module) — parses 512-byte blocks, decodes RP2040/RP2350 family IDs, routes blocks to the flash XIP or SRAM window via `AddressBus`, and rejects duplicate block numbers, bad magics, oversized payloads and unmapped targets
- `rp2350-core`: flat RAM-backed `SimpleBus`/`RamRegion`/`RomRegion` test bus used by boot and loader tests
- `rp2350-core`: `FirmwareImage` ELF loader for the boot path
- `rp2350-core`: `ChipFamily`, `ChipCapabilities` and `ChipInfo`; `RP2350Chip` reports RP2350 capabilities and `RP2040Chip` is a constructible stub whose operations return `CoreError::UnsupportedChip`
- `rp2350-riscv`: `Hazard3Core` implementing `CpuCore` for RV32IMAC — base/M/A/C instruction decode, machine-mode CSRs, trap entry/`mret`, Xh3irq-style external interrupts, and picobin boot (magic `0x7188EBF2`)
- `doc/reference-gap-analysis.md`: upstream capability comparison, verified MicroPython UF2 facts and phased landing plan (Stages 0–6)

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
- `rp2350-arm`: **Phase 2 core components** — `Condition` enum (15 ARM conditions), `ITBlockState` (1-4 instruction IT blocks), `execute_thumb16` (Thumb-16 decoder with ~90 ops), `execute_thumb32` (Thumb-32 decoder stub), `Nvic` (ISER/ICER/ISPR/ICPR/IABR/IPR/STIR, `find_highest_priority`), `SysTick` (24-bit down counter with reload/interrupt), `ExceptionHandler` (EXC_RETURN decode, stacking/unstacking, vector fetch, exception entry/exit)
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
- **Quality gate pass**: `cargo clippy --all-targets --all-features -- -D warnings -W clippy::pedantic` zero errors/warnings; `cargo test --all-features` 51/51 pass; `cargo doc --no-deps` zero warnings

### Fixed
- `conditions::execute_it`: instruction count formula `ctz(mask)+2` → `4-ctz(mask)` per ARM spec
- `conditions::advance`: mask bit indexing switched to LSB-shift for correct T/E pattern decoding
- `sysTick::tick`: counter reaching zero now triggers reload + interrupt in same tick cycle
- `servicing_irq` test: IRQs must be enabled (`write_iser`) before `find_highest_priority` can see them

### Design Decisions / 设计决策
- **Dyn-safe traits**: `CpuCore` uses `&mut dyn AddressBus` instead of generic `B` to enable trait objects
- **Dyn-safe plugins**: `PluginContext` splits into `PluginContext` (trait) + `PluginContextExt` (extension) for dyn compatibility
- **No duplicate state_snapshot**: `DevicePlugin` inherits `state_snapshot()` from `RpPlugin` — single authoritative state interface
- **PluginContext::BusDevice**: Minimal bus interface for plugins — full `MemoryRegion` stays in `rp2350-core`
- **Edition 2024**: All crates use Rust Edition 2024 with `resolver = "3"`
- **Feature gate `dep:` prefix**: `gui = ["dep:egui"]` for correct workspace feature resolution
- **Decouple exception handling**: `exceptions.rs` provides free functions (`enter_exception`, `exit_exception`, `decode_exc_return`) called by `CortexM33Core`, not methods on the core struct itself