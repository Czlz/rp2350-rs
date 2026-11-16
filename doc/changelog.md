# Changelog / 变更记录

> Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
> Version: [SemVer](https://semver.org/)

---

## [1.0.0] — 2026-11-16

> V1 发布口径见 [`v1-scope.md`](v1-scope.md)：包含/不包含项、已知近似、确定性与线程模式差异。

### Added / 新增
- `doc/images`: real-run screenshot assets + reproduction commands — Arm/RISC-V MicroPython and CircuitPython raw-REPL sessions, the Virtual Lab GUI, and the 1.2 G-step emerald frame (`example/gba/doc/emerald-frame-1.2g.png`); README now opens with a showcase section built from these captures.
- `rp2350-cli`: plugin host wiring for `run` — `--plugin-dir`, repeatable `--wire DEVICE:PORT:GPIO` and `--plugin-threads`; devices are driven once per 64-step pump window (`epoch = steps / 64`, elapsed steps passed as `cycles`), inline on the simulation thread or on per-device workers with the fixed publish latency, and `[plugins] devices=… epochs=… threads=…` plus the MCP state JSON are printed at exit. The host also injects one universal `{"clock_hz": <clk_peri>, "sys_clock_hz": <clk_sys>}` payload at construction and again whenever the clock-tree generation changes, so `uart-terminal` decodes at the live peripheral rate (48 MHz on the stock MicroPython image instead of its 150 MHz default). Without plugin flags no `DeviceManager` is built and the historical output is byte-identical; `Drop` reclaims worker threads on every exit and error path; 12 tests (driver/epoch/thread shutdown + clock injection with recording/rejecting mocks), incl. a 30-epoch inline/threaded pin-by-pin comparison and a 30× parallel run.
- `rp2350-core` / `rp2350-periph` / `rp2350-arm` / `rp2350-riscv` / `rp2350-cli`: GAP-07 sleep semantics — `BusErrorKind::Stall` + `CoreError::is_stall()` + `MemoryRegion::read_would_stall`/`MmioDevice::stall_on_read32` (default "never stalls"), `SIO_FIFO_RD` blocking reads with datasheet `FIFO_ST` `VLD`/`RDY`/`WOF`/`ROE` + write-one-to-clear and a per-core `SIO_IRQ_FIFO = VLD|WOF|ROE`, an `Sio::event_generation()` counter, real `WFI`/`WFE`/`SEV`/`SEVL` with an event register and exception-event sources, blocked-load replay without guest-visible exceptions, `Machine` sleep scheduling (`[run] sleep core0 steps=… wakes=… core1 steps=… wakes=…` plus a one-shot `[run] sleep stall` diagnostic) and `Machine::sleep_counters`/`sleep_state`; 6 core + 11 periph + 14 arm + 7 riscv + 7 CLI tests
- `rp2350-gdb` / `rp2350-cli`: read and access watchpoints — `Z3`/`Z4` RSP packets with `T05rwatch:`/`T05awatch:` stop replies, an installable SRAM read watch in `rp2350-core` (`watch::set_read_watch`/`check_read`/`take_read_hit` behind an `AtomicBool` fast gate) and `MachineTarget` hit ordering; `qRcmd` reaches `monitor reset`/`info`/`halt`; ~30 new tests plus a real `arm-none-eabi-gdb` `rwatch`/`awatch` smoke test
- `rp2350-periph` / `rp2350-cli`: dual-core execution — real Cortex-M33 core1 (`SIO_CPUID` per-core views, `{0,0,1,vtor,sp,entry}` launch handshake, PSM `FRCE_OFF.PROC1` reset/release, per-core PPB replay and IRQ routing), `Machine` steps both cores and reports `[run] core1 state=…`; pokeemerald's `core1_render` now runs and drives HSTX + PPU
- `rp2350-periph`: `gpio_pads::GpioPads` shared pad electrical model — `PADS_BANK0` pulls drive `SIO_GPIO_IN` when a pin is an input, host-injected external levels win; `BootPeripherals::pads()`; fixes the "all buttons held" RFU soft reset in the GBA firmware
- `rp2350-periph`: HSTX expander is now paced by the pixel clock — `2 * CSR.CLKDIV` `clk_sys` cycles per pixel, per-command word timing, a 256-word scanline block lasts 800 pixel periods; `output_words()`/`fifo_overrun()`/`fifo_not_full()`; removes the 20x over-draining that dropped words and desynchronised command/data phase
- `rp2350-periph`: `BUSCTRL` block at `0x4006_8000` — priority/ack, `PERFCTR_EN`, counter registers and SET/CLR/XOR aliases
- `rp2350-arm`: `MSPLIM`/`PSPLIM` — `MSR`/`MRS` support with reserved bits forced low (Pico SDK crt0 clears `MSPLIM` on both cores)
- `rp2350-cli`: mask-ROM write-drop view during USB enumeration, so TinyUSB's unconditional EP0 `endpoint_control` write at address 0 no longer faults; emerald's USB CDC banner is readable via `repl --stdio`
- `rp2350-periph`: DMA control-block ring fidelity — `TRANS_COUNT` reload vs working counter split, AL1/AL2/AL3 alias trigger rules, SDK-style `al1_ctrl` configuration, and a pump budget capped at the HSTX FIFO's free space so a batch can never overrun it; the emerald scanout ring streams three laps (42,912 words) verbatim
- `rp2350-arm`: halving, dual-multiply and wide-multiply DSP instructions — SHADD/SHSUB/SHASX/SHSAX, UHADD/UHSUB/UHASX/UHSAX, SMLAD/SMLSD, SMUAD/SMUSD, SMLALD/SMLSLD (X variants), SMULWB/T, SMLAWB/T, SMMLS/SMMLSR and SXTAB16/UXTAB16; 20 tests
- `rp2350-egui`: Virtual Lab wiring canvas — device/port model, wire validation, drag/connect/delete, pointer-anchored zoom and pan, tabbed view with preserved state; 17 tests
- `rp2350-periph`: GBA bring-up support — SET/CLR/XOR atomic alias windows for PIO/DMA, PIO selectors 0..23 and `DREQ_HSTX=52`, HSTX FIFO-room pacing/output counters, and DMA ring/AL3 fixes; PIO/DMA windows widened to 16 KB
- `rp2350-cli`: seed `SCB_VTOR` into the PPB window after Arm boot, fixing Pico SDK `runtime_init` copying an empty vector table; `Machine` drains HSTX output and reports `[run] hstx`/DMA snapshot diagnostics; `run --dump ADDR:LEN:PATH` exports memory ranges at halt
- `rp2350-gdb`: reply `m1`/`l` to `qfThreadInfo`/`qsThreadInfo`, fixing GDB's immediate "Thread 1 exited" after connecting
- `rp2350-periph`: PIO shared handles and IRQ routing — `SharedPio` (`Arc<Mutex<Pio>>` + MMIO proxy), `BootPeripherals::pio()`/`with_pio()`/`pump_pio_irqs()`, PIO0..2 IRQ lines 15..20 (flags 0..3 → IRQ_0, 4..7 → IRQ_1) and a `Machine` tick every 64 steps; 12 periph + 3 CLI tests
- `rp2350-arm`: saturating and packed SIMD instructions — SASX/SSAX/UASX/USAX, QADD/QSUB/QDADD/QDSUB, QADD8/16, QSUB8/16, UQADD/UQSUB, QASX/QSAX/UQASX/UQSAX, SEL and SSAT/SSAT16/USAT/USAT16 with Q/GE semantics; 23 tests
- `rp2350-egui`: read-only DMA/HSTX/PIO panels — busy masks, transfer counts, channel registers, FIFO levels, underrun, PIO SM addresses/pins/IRQs, `PeripheralSource` injection with placeholders; 14 tests
- `rp2350-gdb`: `qXfer:features:read:target.xml` — static ARMv8-M register description (`armv8-m.main` + `org.gnu.gdb.arm.m-profile`, validated with `arm-none-eabi-gdb 16.3`), byte-chunked `m`/`l` replies, `qXfer:features:read+` in `qSupported`, and 11 tests
- `rp2350-cli`: DMA pump in `Machine` — every 64 steps the machine runs up to 16 DMA data units through `Dma::run`, on the same path used by `step`/`trace_step`/boot-ROM traps/GDB single-steps; `dma_transfers()`/`dma_state()` diagnostics, idle fast path, bounded per-batch budget; 7 tests
- `rp2350-periph`: HSTX model — APB control window (`CSR`, `BIT0..7`, expanders) plus AHB FIFO window (`STAT`, `FIFO`), 8-word FIFO with WOF, RAW/RAW_REPEAT/TMDS/TMDS_REPEAT/NOP command decoding, documented WAIT extension, host `drain_output`/`take_scanline`, underrun accounting and model IRQ 46; 13 tests
- `rp2350-periph`: RP2350 DMA controller — 16 channels, control blocks, AL1/AL2/AL3 triggers, byte/half/word copies, address increments, RING wrap, CHAIN_TO, DREQ gating, abort, INTR/INTE/INTF/INTS and `BootPeripherals::dma()`; `Dma::run` is library-driven and the CLI pump is deferred; 20 tests
- `rp2350-gdb`: write watchpoints — RSP `Z2`/`z2` with 1/2/4/8-byte kinds, idempotent insert/remove, `Z3`/`Z4` fallback, `T05watch:<addr>;` stop replies, and CLI `MachineTarget` value-change detection limited to SRAM (snapshot reads do not advance machine cycles)
- `rp2350-virt-periph`: plugin hot reload — `Fingerprint`/`ReloadReport`, `DeviceManager::reload` (instance-before-library drop order), added/removed/reloaded/failed reporting, and CLI `list-devices --reload`/`--watch`; 20 new tests
- `rp2350-arm`: Cortex-M33 lazy FP context stacking — `FPCCR`/`FPCAR`/`FPDSCR`, 17-word FP context plus an alignment word, `LSPACT` deferral, first-FP-instruction commit, extension-frame `EXC_RETURN`, return-time reclaim/restore, nested commits, and corrected PSP thread `EXC_RETURN`; 18 new tests
- `rp2350-periph`: PIO pin model and timing — 32-bit input/output/output-enable levels, `PINCTRL` OUT/SET/IN/SIDESET base+count mapping, side-set (including optional `SIDE_PINDIR`), instruction delay cycles, 16.8 `CLKDIV` gating, `JMP PIN`/`WAIT PIN`/`WAIT GPIO`, `SET/OUT PINDIRS`, FIFO join (`FJOIN_TX`/`FJOIN_RX`, 4/8/0 depth) and real `DBG_PADOUT`/`DBG_PADOE` readback; 22 new tests
- `rp2350-mcp`: HTTP Bearer-token authentication — `Auth`/`HttpOptions`, constant-time token comparison, `401` + `WWW-Authenticate: Bearer` for `POST /mcp`, open `GET /health` with `auth` flag, CLI `mcp-serve --auth-token` / `RP2350_MCP_TOKEN`, token-redacting `Debug`; 15 new tests
- `doc/perfbench-report.md`: official `run-perfbench.py` results for the stock MicroPython Arm and RISC-V UF2s — 24/24 benchmarks pass the CPython truth check on both, driven through the new `repl --stdio` bridge; the run exposed and fixed the FPv5-SP arithmetic gap (Arm `bm_float`/`bm_fft`/`bm_chaos`)
- `rp2350-arm`: FPv5-SP single-precision arithmetic, comparison, conversion, `VSEL` and `VRINT` instructions — `VADD`/`VSUB`/`VMUL`/`VNMUL`/`VMLA`/`VMLS`/`VNMLA`/`VNMLS`/`VDIV`/`VSQRT`/`VABS`/`VNEG`, fused `VFMA`/`VFMS`/`VFNMA`/`VFNMS`, `VCMP`/`VCMPE`, integer↔float `VCVT`/`VCVTR`, `VMOV.F32 Sd,Sm`, condition selects and directed rounding; `VCMP` writes FPSCR N/Z/C/V and the cumulative exception bits are modelled
- `rp2350-arm`: `DualCoreM33` dual-core scheduling scaffold — owns core0/core1, gates core1 behind `release_core1` (vector-table probe + `boot_at`), steps each running core once per call, honours `run_cycles` budgets with early exit when both cores stop, and routes interrupts to each core's independent NVIC
- `rp2350-cli`: `repl --stdio` transparent USB CDC bridge — stdin → CDC OUT, CDC IN → stdout, `[repl]` diagnostics on stderr, bounded 64 KiB reader queue, 500 µs simulation steps, unlimited budget via `--max-cycles 0`, so the official `pyboard.py` / `run-perfbench.py` can drive the emulator through `exec:`
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
- `rp2350-mcp`: minimal HTTP/1.1 transport — `POST /mcp` (alias `/`) forwards the body to `handle_request`, `GET /health` reports status, with `400/404/405/411/413/431/501/505` handling, 8 KiB header / 1 MiB body limits, connection timeouts and a pollable shutdown flag
- `rp2350-cli`: `mcp-serve` starts the MCP HTTP transport (default port 8080) and prints both endpoints
- `plugins/uart-terminal`: second-party UART terminal plugin — `TX` (input) is sampled at the configured baud to decode start/data/stop bits LSB-first, `RX` (output) shifts bytes out with the same bit timing; `baud`/`data_bits`/`stop_bits`/`rx_line`/`clock_hz` configuration, framing-error accounting and a state snapshot (11 tests)

### Changed / 变更
- 仓库卫生：`.trae/`、`.opencode/` 等本地 agent/IDE 目录不再随仓库发布（规则迁移到根 `AGENTS.md`，固件验证手册迁移到 `doc/firmware-verification.md`）；删除重复的示例截图与空目录，README/示例文档同步更新。
- `rp2350-arm`: `check_pending_exceptions` now also honours `FAULTMASK` and `BASEPRI` (the same gate decides whether a sleep wake-up enters an interrupt); `CpuCore::step` on a sleeping core advances `SysTick`/`total_cycles` instead of returning immediately; `StepResult::Sleep` became `Sleep(SleepKind)` and `SEV`/`SEVL` report `SendEvent { local }`
- `rp2350-riscv`: `wfi` is a real wait instead of a `NOP`; `signal_interrupt` no longer forces `CpuState::Running` (the step's own wake rule decides), and a retryable load stall never enters a guest trap
- `rp2350-cli`: `run_steps`/`step_cores` treat `CpuState::Sleeping` as a normal, time-advancing step (only `Halted`/`Faulted`/`Reset` stop the machine), the executed-PC ring skips sleep steps, and `core1_running` includes a sleeping core 1 so it can be woken
- 文档结构：**示例项目专用内容迁出主 `doc/`**。`gba-bringup-report.md` 与游戏帧截图移到 `example/gba/doc/`，示例的固件信息、运行命令、引脚表、外设/插件依赖移到 `example/gba/README.md`，帧缓冲转 PNG 工具移到 `example/gba/tools/fb2png.py`；`AGENTS.md` 新增「示例项目目录（example/，强制）」规则，主 `doc/` 只保留跨示例的通用文档并用相对路径链接
- `doc/implementation-plan.md`：P1 进度表同步 —— PIO（DMA DREQ）、DMA（`TRANS_COUNT` 重载、3 通道环）、HSTX（像素时钟节拍、`DREQ_HSTX`）、ARM ISA（`MSPLIM`/`PSPLIM`、折半/双乘/宽乘）、双核（真实 core1 执行）状态更新；验收主线增加 GBA 示例链接
- `doc/changelog.md`：更新「Planned」列表（双核/HSTX/egui 画布/DSP/GDB 已完成，改为 PIO FUNCSEL、WFI/WFE 调度、PSRAM XIP、MCP TLS、多核 GDB `vCont`、TMDS 串行化）

### Fixed / 修复
- CI: rustc/clippy 1.100 的 `needless_range_loop` 命中 4 处（DMA `CHAN_ABORT`、PIO `CTRL`、NVIC 优先级扫描、USB `BUFF_STATUS`），改为固定数组 `iter()`/`iter_mut().enumerate()`，位序与扫描顺序不变；全 workspace `-D warnings -W clippy::pedantic` 重新通过。
- `rp2350-audio`: 后端探测等待从 1 s 放宽到 5 s，修复并行测试/慢声卡枚举时 `backend_detail()` 在 `start()` 返回后仍为 `None` 的偶发失败。
- `rp2350-virt-periph` 测试夹具：`rp2350_device_plugin_create` 改为返回 fat-pointer 空值（与 `DeviceCreateFn` 的 `*mut dyn Trait` ABI 一致），修复 Rust 1.100 下 `load_device_rejects_null_factory` 的 `STATUS_ACCESS_VIOLATION`。
- `rp2350-periph`: TIMER interrupt registers were at the RP2040-era offsets (`0x38/0x3C/0x40/0x44` instead of `0x3C/0x40/0x44/0x48`), `INTF` was treated as write-one-to-clear instead of a force mask, alarms never auto-disarmed and there was no `TIMER1`; all four are fixed, `INTS = (INTR | INTF) & INTE` drives the four lines and `TIMER1` at `0x400B_8000` shares the microsecond timebase with independent alarms/IRQs
- `rp2350-arm`: `enter_exception` never wrote `xPSR[8:0]` (IPSR), so `MRS IPSR` returned 0 and pico-sdk's `alarm_pool_irq_handler` selected the wrong timer/alarm and crashed with `Bus error 0x00757C08`; the exception number is now latched on entry
- **Re-landed**: the `WFI`/`WFE`/`SEV` sleep semantics reverted from `9a6b088` are implemented again in this release — blocking `SIO_FIFO_RD`, FIFO IRQ, `FIFO_ST` accounting and sleep scheduling landed first (see the GAP-07 entry above), so the core-1 launch handshake no longer stalls when core 0 sleeps. / `WFI`/`WFE`/`SEV` 睡眠语义已随阻塞 FIFO 与睡眠调度重新落地，见上方 GAP-07 条目。
- `rp2350-periph`: SIO hardware spinlocks were missing and `SPINLOCK_ST` read `0xFFFF_FFFF` (all free, the opposite of the reset value); `SPINLOCK0..31` at `0x100 + 4n` now claim on read, release on write, share state across cores and report the real bitmap; `Sio::spinlock_state()` diagnostics, 11 tests
- `rp2350-arm`: the exclusive monitor was a `thread_local!`, so both cores stepped on one host thread shared it and core0's `LDREX` could satisfy core1's `STREX`; the monitor now lives in `CortexM33Registers` (per core), `CLREX` (`0xF3BF_8F2F`) is implemented instead of decoding as undefined, and 9 tests cover cross-core isolation and size mismatches
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

### Verified / 验证（2026-11-15）

- **GBA 示例 ✅**：`emerald.uf2`（SHA-256 `5BEE593F…A9B4C4`）双核运行 2 G 步（13.3 s 模拟时间），`hstx words=128386654 underruns=0 wof=false unknown=0`、`core1 state=Running`、无总线错误；`g_fb` 为 32 色真实画面（Game Freak 标志）。详见 [example/gba/doc/gba-bringup-report.md](../example/gba/doc/gba-bringup-report.md)。
- **FV-1/2/3 ✅**：三个固件 `repl --exec "print(6 * 7)"` 均输出 `42`。
- **perfbench ✅**：官方 `run-perfbench.py` Arm 与 RISC-V 各 24/24 通过 CPython 真值（[报告](perfbench-report.md)）。

### Verified / 验证（2026-11-14）

- **FV-2 ✅**：`cargo run --release -p rp2350-cli -- repl --file RPI_PICO2-20260824-v1.29.0.uf2 --exec "print(6 * 7)"` → `stdout: 42`（2,224,500 步 / 14,830 µs）；`perfbench --bench bm_nqueens.py --benchrun benchrun.py --n 40 --m 10` → `measured: ticks_us=19266 norm=100 result=10`。
- **FV-3 ✅（2026-11-14）**：`cargo run --release -p rp2350-cli -- repl --file adafruit-circuitpython-...uf2 --no-soft-reset --max-cycles 300000000 --exec "print(6 * 7)"` → `[repl] USB CDC configured: IN=0x82/64 OUT=0x02/64 interface=1`、`[repl] banner`、`stdout: 42`（约 39.8M 步）。根因：16 位 `SXTB/SXTH/UXTB/UXTH` 误更新 APSR。
- **FV-1 ✅（2026-11-14）**：`cargo run --release -p rp2350-cli -- repl --file RPI_PICO2-RISCV-...uf2 --exec "print(6 * 7)"` → `[repl] USB CDC configured: IN=0x82/64 OUT=0x02/64 interface=1`、`[repl] banner`、`stdout: 42`（2,659,500 步 / 17,730 µs）；`run --arch riscv --strict-decode --max-cycles 50000000` 无未定义指令。修复 Zcmp、Xh3irq 抢占/NOIRQ、CSR 窗口索引与 boot2 数据查找槽。

### Planned / 已排期（不在 V1 范围）
- HSTX TMDS 10 位编码与 DVI 位时序；宿主侧像素捕获 API
- USB 全速位级时序、CRC 与位填充
- MCP TLS 传输
- 周期成本模型（当前 `steps == cycles`）、TrustZone/SAU、MPU 与 `MemManage`/`BusFault` 故障源
- 存档/状态保存（`--save-state`/`--load-state`、GUI Save/Load）——V1 明确排除
- `TIMER`/`TIMER1` 的 `PAUSE`/`DBGPAUSE`/`LOCKED` 语义
- 实时 60 fps 游玩（在「不做 JIT」前提下不可达）
- `SMLALxy`（`SMLALBB/BT/TB/TT`）与 `CP1-CP3`/`CP5-CP7` 协处理器建模

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