# Changelog / 变更记录

> Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
> Version: [SemVer](https://semver.org/)

---

## [Unreleased]

### Added / 新增
- `rp2350-bootrom`: synthetic 32 KB boot ROM for RP2350 — documented header fields (`0x10` magic/version, `0x14` table pointer, `0x16` `rom_table_lookup`), ARM-secure/non-secure table entries and `udf` stubs dispatched on the host before each instruction step; `rom_table_lookup`, `rom_get_sys_info`, `rom_reboot`, `rom_bootrom_state_reset` and the flash erase/program functions operate on the shared flash backing store
- `rp2350-core`: `FlashHandle` host-only programming handle sharing the `FlashRegion` bytes (guest writes still return `AccessDenied`)
- `rp2350-arm`: `boot_from(vector_base)` and `boot_at(sp, pc, vector_base)` — boot vectors and VTOR can come from the flash XIP base instead of address 0
- `rp2350-core`: `FlashRegion` XIP window (`0x1000_0000`, 16 MB) — guest writes return `AccessDenied`, host loaders use the new `MemoryRegion::load_u8` / `AddressBus::load_u8` path
- `rp2350-core`: generic MMIO framework (`MmioWindow`, `MmioDevice`, `MmioRegion`, `MmioRouter`, `RegisterFile`, `mount_device`) with fixed datasheet windows, overlap rejection and one-shot warnings for unmodelled offsets; 16/32-bit accesses dispatch as single register operations
- `rp2350-periph`: Stage-4 boot stubs — RESETS, CLOCKS, XOSC, PLL_SYS/USB, WATCHDOG, TIMER0, SIO, IO_BANK0, PADS_BANK0, UART0 and PPB, wired through `BootPeripherals::mount` with a shared microsecond `Timebase` and a host-visible `UartSink`
- `rp2350-cli`: `run --file <uf2|elf> [--arch arm|riscv] [--max-cycles N] [--trace-steps N]` — builds the Stage-4 bus (synthetic boot ROM, XIP flash, SRAM, APB), boots ARM from `0x1000_0000` with `VTOR = 0x1000_0000`, prints a boot trace, drains UART0 output and reports the halt reason
- `rp2350-arm`: Thumb-16 decoder groups remapped to the ARMv8-M table (shift-immediate, MOV/CMP/ADD/SUB immediate, ALU/high-register, literal, register-offset, word/byte/halfword immediates, SP-relative), added `CBZ`/`CBNZ`/`POP`/`SETEND`/`CPS`/`BKPT`/`IT`, and fixed PC-relative branch/literal offsets for the pre-advanced PC
- `rp2350-core`: UF2 container loader (`uf2` module) — parses 512-byte blocks, decodes RP2040/RP2350 family IDs, routes blocks to the flash XIP or SRAM window via `AddressBus`, and rejects duplicate block numbers, bad magics, oversized payloads and unmapped targets
- `rp2350-core`: flat RAM-backed `SimpleBus`/`RamRegion`/`RomRegion` test bus used by boot and loader tests
- `rp2350-core`: `FirmwareImage` ELF loader for the boot path
- `rp2350-core`: `ChipFamily`, `ChipCapabilities` and `ChipInfo`; `RP2350Chip` reports RP2350 capabilities and `RP2040Chip` is a constructible stub whose operations return `CoreError::UnsupportedChip`
- `rp2350-riscv`: `Hazard3Core` implementing `CpuCore` for RV32IMAC — base/M/A/C instruction decode, machine-mode CSRs, trap entry/`mret`, Xh3irq-style external interrupts, and picobin boot (magic `0x7188EBF2`)
- `doc/reference-gap-analysis.md`: upstream capability comparison, verified MicroPython UF2 facts and phased landing plan (Stages 0–6)
- `rp2350-periph`: `IrqAssert` — shared 64-bit interrupt-line mask that peripherals assert and the driver polls
- `rp2350-periph`: TIMER0 alarm model — four alarms compared against the shared microsecond `Timebase`, `INTR`/`INTE`/`INTF`/`INTS` semantics and IRQ lines 0..3
- `rp2350-periph`: SIO FIFO and doorbell model — core-to-core FIFO queues, `FIFO_ST` flags, `SIO_IRQ_FIFO` / `SIO_IRQ_BELL` assertions and `DOORBELL_OUT_SET/CLR` + `IN_SET/CLR`
- `rp2350-periph`: PPB records NVIC, SysTick and VTOR writes into a shared log for replay into the core
- `rp2350-cpu`: `CpuCore::system_write` / `CpuCore::system_read` hooks for system control space accesses
- `rp2350-cli`: interrupt pump — replays PPB writes via `system_write`, delivers asserted lines via `signal_interrupt` and prints `[run] irq deliveries`
- `rp2350-core`: `FlashRegion::alias` — a second read-only XIP window over the same backing store, used for the RP2350 non-cached flash alias at `0x1400_0000..0x14FF_FFFF`
- `doc/architecture.md`: verified IRQ number map (TIMER0 0..3, USBCTRL 14, IO_BANK0 21, SIO 25/26, UART0 33) and interrupt wiring description
- `rp2350-arm`: Thumb-32 FPv5-SP data transfers — `VMOV`/`VLDR`/`VSTR`/`VLDM`/`VSTM`/`VPUSH`/`VPOP`/`VMSR`/`VMRS` (d-register view, VFP immediate expansion, PC-relative `VLDR`), plus the GPIO coprocessor CP0 (`MCR`/`MRC`/`MCRR`/`MRRC` with SET/CLR/XOR alias writes) and a side-effect-free double-coprocessor CP4 placeholder
- `rp2350-arm`: CFSR storage with synchronous NOCP `UsageFault` entry (rewound PC, standard frame, MSP/PSP-aware `EXC_RETURN`), post-boot-ROM CPACR state, and CPACR modeling for CP0–CP7 + CP10–CP11 with FPv5-SP access governed by CP10 alone
- `rp2350-core`: `watch` module — optional `RP2350_WATCH=START-END` / `RP2350_WATCHV=VALUE` store watchpoint attributing writes to the current PC (hooked into `AddressBus::write_u32` and `RamRegion::write_u8`)
- `rp2350-core`: `XipMaintenanceRegion` — write-only cache-maintenance window at `0x1800_0000..0x1BFF_FFFF` (writes accepted and discarded, reads return zero)
- `rp2350-periph`: PPB replays `SCB_CPACR` writes into the core, matching `runtime_init`'s FPU enable
- `rp2350-usb-host`: `progress_line()` enumeration diagnostics (stage/phase/request/byte counts/queue depth) and a failed-transfer snapshot preserved after errors
- `rp2350-periph`: PSM (`0x4001_8000`) power-state model with `FRCE_ON`/`FRCE_OFF`/`WDSEL`/`DONE`, a shared core-1 release flag (clearing `FRCE_OFF.PROC1` triggers the boot ROM handshake) and `BootPeripherals::service_core1_peer` (drain-and-ack + FIFO echo)
- `rp2350-periph`: `BootRam` seeds the first BOOTRAM word with a `bx lr` boot2 copy-out stub, so `flash_init_boot2_copyout` returns immediately while flash remains XIP-readable in this model
- `rp2350-arm`: Thumb-32 DSP multiply coverage — `SMULBB/SMULBT/SMULTB/SMULTT` and the accumulating `SMLABB/SMLABT/SMLATB/SMLATT` forms (Q flag on 32-bit overflow), plus `SMMUL/SMMLAR/SMMLA/SMMLAR` high-word modular multiply
- `rp2350-arm`: extend-and-add `SXTAB/SXTAH/UXTAB/UXTAH` with the 8/16/24-bit rotate field (accumulating forms leave APSR unchanged; the 32-bit plain `SXTB/SXTH/UXTB/UXTH` forms update N/Z, while the 16-bit encodings remain flag-free)
- `rp2350-arm`: `MRS`/`MSR BASEPRI_MAX` (sysm `0x12`) with raise-only priority-mask semantics
- `rp2350-riscv`: picobin `IMAGE_DEF` parser — scans the first flash page for marker `0xffffded3` and the `0x44` entry-point item, loading the boot2 entry and initial SP (target RISC-V MicroPython: entry `0x10000036`, SP `0x20082000`)
- `rp2350-cli`: `run` and `repl` now service the resident core-1 peer every 64 steps (and on every trace/boot-ROM trap step), so SDK `multicore_reset_core1` handshakes complete
- `rp2350-cli`: `repl` and `perfbench` subcommands — USB CDC raw REPL over a booted MicroPython UF2 (`--exec`, or `bm_*.py` via shared `benchrun.py` with N/M repetitions); `run.rs` refactored into a reusable `Machine` with register snapshots, a bounded `recent_steps` trace, per-batch PPB system-write replay, the XIP maintenance window and SRAM-top exception stack mapping
- `rp2350-bootrom`: RISC-V synthetic boot ROM — the 16-bit pointer at `0x7DFA` points to the `0x75C2` `rom_func_lookup` stub; RISC-V function stubs (`IF`/`IX`/`RE`/`RP`/`FC`/`CX`/`GS`/`RB`/`SR`) and architecture-aware trap dispatch (`a0..a3` arguments, `ra` return)
- `rp2350-riscv`: Xh3irq model (`meiea`/`meipa`/`meifa`/`meipra` 16-bit windows, `meinext`/`meicontext`), Zba/Zbb/Zbs instructions (`sh1add`/`sh2add`/`sh3add`, `andn`/`orn`/`xnor`, `bset`/`bclr`/`binv`/`bext`, `min`/`max`, `rol`/`ror`), plus 2-byte-aligned `mepc` and `mret` preserving NOIRQ
- `rp2350-riscv`: Zb\* semantics completion — `pack`/`packh` (`rs2 = 0` selects `zext.h`), `clz`/`ctz`/`cpop`, `sext.b`/`sext.h`, `orc.b`, `rev8`, `brev8`, and the immediate bit ops; Zcb `C.ZEXT.B`/`C.ZEXT.H`, `C.SEXT.B`/`C.SEXT.H`, `C.NOT`, `C.MUL`; `C.LUI` sign-extension; synchronous exceptions use the `mtvec` base while only interrupts vectorize
- `rp2350-bootrom`: RISC-V data lookup slot `0x7DF8 → 0x75FE` and an exception vector table at `0x7E00` (cause 2/5 → `0x7F00` diagnostic stub that prints `mcause`/`mepc`/`mtval` and halts cleanly)
- `rp2350-riscv`: Zcmp `cm.push`/`cm.pop`/`cm.popret`(z) with register lists and `spimm`; Xh3irq priority/preemption (`meinext.update` claims the pending bit, `ppreempt`/`pppreempt` stack, NOIRQ preserved across `mret`) and CSR array windows selected from the raw operand (a `csrrc meiea` read-modify-write can no longer clear another window's IRQ)
- `rp2350-cli`: `repl --arch arm|riscv` override plus UF2-family architecture detection, so the RISC-V MicroPython image reaches the raw REPL
- `rp2350-periph`: PIO state-machine execution — `JMP`/`WAIT`/`IN`/`OUT`/`PUSH`/`PULL`/`MOV`/`IRQ`/`SET`, shift registers with autopush/autopull thresholds, wrap and `EXEC_STALLED`/`FDEBUG` stall reporting, plus a `pio/{mod,regs,sm,shift}` module split (16 new tests)
- `rp2350-gdb`: TCP session loop — listener/accept, byte-level RSP framing with `+`/`-` acknowledgement, `QStartNoAckMode` negotiation, `Session::reset_transport()` per connection and six real TCP end-to-end tests
- `rp2350-cli`: `gdb` subcommand now starts the RSP server — a `MachineTarget` adapter exposes registers (R0-R15 + XPSR), bus memory, PC-compare breakpoints and `resume` through `Machine::step` (boot ROM traps, interrupt pumping and the peripheral timebase stay active), with a 1,000,000-instruction continue budget
- `rp2350-mcp`: JSON-RPC method layer — `initialize`, `tools/list` and `tools/call` (`list_devices`, `query_all`, `query_device`, `configure`) delegated to `DeviceManager`, with JSON-RPC error codes for parse/method/parameter/device/config failures
- `rp2350-virt-periph`: `mcp_query_device`, `mcp_list_devices` and `apply_config` on `DeviceManager` (plus `ConfigApplyError`) backing the MCP method layer
- `rp2350-arm`: DSP packed SIMD — `SADD8`/`SSUB8`/`SADD16`/`SSUB16` and the `UADD`/`USUB` variants with per-lane GE flags in `APSR[19:16]`, `SXTB16`/`UXTB16` with the rotate field, and `USAD8`/`USADA8`; `MRS APSR` / `MSR APSR_g|nzcvqg` expose GE
- `rp2350-virt-periph`: `DeviceManager::scan_and_load(dir)` — instantiates every device plugin in a directory (metadata ID or file stem, duplicate/failure skipped with a warning) and `PluginLoader::scan_dir` helper
- `rp2350-cli`: `list-devices` and `wire` subcommands now scan a plugin directory, build a real `DeviceManager` and print device/wiring tables (one-shot process semantics)

### Fixed / 修复
- `rp2350-core`: the non-cached XIP alias at `0x1400_0000` was unmapped, so firmware executing from the `XIP_NOCACHE_NOALLOC` window failed with `BusError::NoDevice`; it now reads the same flash bytes as the cached window
- `rp2350-arm`: external interrupts used the IRQ number as the exception number and fetched the vector from `VTOR + irq * 4`; they now use exception number `16 + irq`
- `rp2350-arm`: `SCB_VTOR` writes now reach the core (replayed from the PPB log) and the exception vector fetch uses the programmed offset
- `rp2350-arm`: NVIC active/pending bits were indexed with the exception number (offset by 16); they now use the IRQ number
- `rp2350-arm`: `find_highest_priority` skips active interrupts so an asserted level cannot preempt its own handler
- `rp2350-periph`: SIO register offsets switched from the RP2040 layout to the RP2350 interleaved layout (`GPIO_HI_OUT` 0x014, `OUT_SET` 0x018, `GPIO_OE` 0x030, `OE_SET` 0x038)
- `doc/architecture.md`: USB DPRAM corrected from 16 KB to 4 KB (`0x5010_0000..0x5010_0FFF`), USBCTRL IRQ documented as 14, SIO window corrected from 512 B to 4 KB (`0xD000_0000..0xD000_0FFF`)
- `rp2350-arm`: PC writes now follow BXWritePC/LoadWritePC semantics — `POP {.., pc}`/`LDM`/`LDR`/`BX` carrying an `EXC_RETURN` value trigger the exception return sequence instead of masking bit 0 into a runaway branch
- `rp2350-arm`: SP updates route through `regs.write(13)` so the MSP/PSP shadow banks stay in sync; exception returns no longer unstack from a stale `msp` (real MicroPython previously restored PC/SP from the SRAM top `0x2008_2000`)
- `rp2350-bootrom`: `flash_range_erase`/`flash_range_program` arguments are flash-relative offsets and are translated by the XIP base before touching the backing store (offset `0x1000` used to land at absolute address `0x1000`)
- `rp2350-usb-host`: control IN data stages end only on a short packet or once `wLength` bytes arrive — `BUF_CTRL_LAST` is a device-internal flag invisible to the host, and treating it as the end condition deadlocked 75-byte configuration descriptor fetches
- `rp2350-arm`: Thumb-16 `LDMIA` with the base register in the load list keeps the loaded value instead of writing back the post-increment address (GCC's three-word word-copy idiom in MicroPython's lexer reader was being corrupted); the Thumb-32 `LDM` write-back follows the same Armv8-M rule
- `rp2350-arm`: Thumb-16 `ADDS/SUBS` (register and 3-bit immediate) now update C and V as well as N/Z, restoring the 64-bit carry chain used by CircuitPython's delay loop
- `rp2350-arm`: exception entry/return saves and restores `EPSR.IT` in the stacked xPSR (`xPSR[26:25,15:10]`) and clears the handler's IT state; an interrupt taken inside an IT block no longer causes the first handler instruction or the resumed instruction to be skipped
- `rp2350-usb`: OUT buffer control writes now clear stale `BUF_CTRL.LEN` before storing the received byte count; the firmware had armed OUT with a 64-byte length, so every short CDC packet looked full-sized and trailing DPRAM zeros were read as data
- `rp2350-arm`: 16-bit `SXTB/SXTH/UXTB/UXTH` no longer update APSR — the erroneous Z update made CircuitPython's `process_set_config` loop exit early (`bls`), so TinyUSB stalled EP0 and `SET_CONFIGURATION` never completed (FV-3 blocker)
- `rp2350-riscv`: `LB`/`LH` now sign-extend to XLEN (the previous implementation effectively zero-extended); `RESET_DONE` reports the implemented bit mask and tracks reset-register state
- `rp2350-usb`: `EP_ABORT`/`EP_ABORT_DONE` bits decode as `2*ep + dir` (there is no separate direction bit), so aborting EP0 IN no longer clears EP0 OUT; `EP_STALL_ARM` EP0 bits are cleared when a SETUP packet arrives

### Verified / 验证（2026-11-14）

- **FV-2 ✅**：`cargo run --release -p rp2350-cli -- repl --file RPI_PICO2-20260824-v1.29.0.uf2 --exec "print(6 * 7)"` → `stdout: 42`（2,224,500 步 / 14,830 µs）；`perfbench --bench bm_nqueens.py --benchrun benchrun.py --n 40 --m 10` → `measured: ticks_us=19266 norm=100 result=10`。
- **FV-3 ✅（2026-11-14）**：`cargo run --release -p rp2350-cli -- repl --file adafruit-circuitpython-...uf2 --no-soft-reset --max-cycles 300000000 --exec "print(6 * 7)"` → `[repl] USB CDC configured: IN=0x82/64 OUT=0x02/64 interface=1`、`[repl] banner`、`stdout: 42`（约 39.8M 步）。根因：16 位 `SXTB/SXTH/UXTB/UXTH` 误更新 APSR。
- **FV-1 ✅（2026-11-14）**：`cargo run --release -p rp2350-cli -- repl --file RPI_PICO2-RISCV-...uf2 --exec "print(6 * 7)"` → `[repl] USB CDC configured: IN=0x82/64 OUT=0x02/64 interface=1`、`[repl] banner`、`stdout: 42`（2,659,500 步 / 17,730 µs）；`run --arch riscv --strict-decode --max-cycles 50000000` 无未定义指令。修复 Zcmp、Xh3irq 抢占/NOIRQ、CSR 窗口索引与 boot2 数据查找槽。

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