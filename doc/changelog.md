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

## [0.1.0] — TBD

### Added
- Initial workspace scaffold
- `rp2350-core`: AddressBus, BusDevice trait, RP2350Chip, SimulationClock, IrqMatrix, GpioPin
- `rp2350-cpu`: CpuCore trait definition
- `rp2350-arm`: CortexM33Core skeleton
- `rp2350-periph`: Peripheral trait, basic peripheral stubs
- `rp2350-virt-periph`: VirtualLed, VirtualTfCard, VirtualUart, DebugProbe, PsramDevice, QspiFlashDevice
- `rp2350-plugin`: RpPlugin trait, PluginContext trait, PluginHost
- `rp2350-ffi`: SafeLibrary RAII wrapper
- `rp2350-gdb`: GdbTarget trait, GdbServer skeleton
- `rp2350-egui`: eframe App skeleton, menu bar, status bar
- `rp2350-cli`: clap CLI with `run` and `gdb` subcommands
- Project documentation in `doc/`