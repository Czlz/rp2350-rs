# Reference Gap Analysis & Landing Plan / 参考项目差距分析与落地计划

> **Version**: 1.0 | **Date**: 2026-11-08 | **Status**: Active
>
> **EN**: This document compares three upstream RP2350 emulators with `rp2350-rs`,
> records the verified facts about the target MicroPython firmware, and defines the
> phased plan used to close the gaps needed by the final acceptance target.
>
> **ZH**: 本文对照三个上游 RP2350 模拟器与 `rp2350-rs` 的能力，记录目标
> MicroPython 固件的实证结论，并给出为达成最终验收目标而制定的分阶段落地计划。

---

## 1. Executive Summary / 结论摘要

### 1.1 Current baseline / 当前基线

`rp2350-rs` already provides:

- `rp2350-core`: address bus, `MemoryRegion`/`AddressBus`, `RamRegion`/`SimpleBus`,
  chip topology, GPIO/IRQ/clock primitives, ELF `FirmwareImage` loading.
- `rp2350-cpu`: `CpuCore`/`CpuArch`/`CpuState` abstraction.
- `rp2350-arm`: Cortex-M33 with Thumb-16, partial Thumb-32, NVIC, SysTick,
  exception entry/exit, boot vector fetch from `0x0000_0000`.
- Plugin infrastructure (`RpPlugin`, `DevicePlugin`, dynamic loading) and an LED plugin.
- ELF loading and `.bss` zero-fill.

### 1.2 Key findings / 关键结论

1. **Target firmware uses USB CDC REPL, not UART REPL.**
   Verified in MicroPython `ports/rp2/mpconfigport.h`: `MICROPY_HW_ENABLE_UART_REPL`
   defaults to `0`, while `MICROPY_HW_USB_CDC` defaults to `1`. The board header
   `ports/rp2/boards/RPI_PICO2/mpconfigboard.h` does not override either.
   Therefore the stock `RPI_PICO2-*.uf2` prints nothing on UART0, and
   `run-perfbench.py` can only reach its REPL through the USB CDC device.
   / 目标固件默认只启用 USB CDC REPL，UART0 REPL 关闭；perfbench 必须经由 USB CDC。

2. **The target UF2 is an ARMv8-M image that boots directly from flash.**
   Verified: 661,504 bytes = 1,292 UF2 blocks; families `0xE48BFF59`
   (`RP2350_ARM_S`, 1,291 blocks) and `0xE48BFF57` (`RP2350_ABSOLUTE`, 1 block at
   `0x10FFFF00`). The first flash words are `SP = 0x20082000`,
   `Reset = 0x1000015B`, i.e. the vector table sits at `0x1000_0000` and there is no
   252-byte `boot2` stage. The ARM image occupies `0x1000_0000..0x1005_0B00`.
   / 目标 UF2 是 ARM 镜像，向量表直接位于 Flash 起始地址，无 boot2 阶段。

3. **`run-perfbench.py` needs a raw REPL, not a filesystem.**
   The driver composes `bm_*.py + perf_bench/benchrun.py + bm_run(N, M)` and
   executes it through the `pyboard` interface (`enter_raw_repl()` + `exec_()`).
   The device must provide `time.ticks_us`/`time.ticks_diff`; Python `float`
   (`complex` feature check) and Viper are optional and cause a SKIP when absent.
   / perfbench 通过 raw REPL 直接执行脚本，不依赖文件系统。

4. **The three references split the required work between them.**
   - `rp2350js_emulator`: RISC-V-only, no ARM, bootrom bypassed, rich peripheral set,
     family-aware UF2 loader.
   - `c1570/rp2350js`: both ARM and RISC-V, real bootrom, runs MicroPython ARM and
     RISC-V, comprehensive peripheral models, UF2/HEX loaders.
   - `0x4D44/picoem`: Rust, ARM-only, RP2350 + RP2040, real bootroms, but UART/SPI/
     I2C/DMA/timers are stubs and there is no USB model.

---

## 2. Reference Capability Matrix / 参考项目能力对照

| Capability | rp2350js_emulator | c1570/rp2350js | 0x4D44/picoem | rp2350-rs (now) |
|---|---|---|---|---|
| Cortex-M33 | No (RISC-V fork) | Basic (runs MicroPython) | Cycle-accurate | Thumb-16 + partial Thumb-32 |
| Hazard3 RISC-V | Yes, RV32IMAC + Z* | Yes, RV32IMAC + Z* | No (out of scope) | No |
| Architecture selection | Fixed RISC-V | `coreArch: 'riscv' \| 'arm'` + OTP ARCHSEL | ARM only | Enum only (`CpuArch`) |
| Boot ROM | Bypassed | Real bootrom | Real bootrom | Bypassed (direct vector fetch) |
| UF2 loader | Yes, family-aware | Yes, family-agnostic | probe-rs loader reuse | **Missing** |
| Flash / XIP | 16 MB flat array, read-only XIP | QMI + XIP window, read-only | QMI + XIP, real bootrom polling | **Missing** |
| RP2040 | No | Yes (M33 path plus RP2040 peripherals) | Yes (dual Cortex-M0+) | **Missing** |
| UART (PL011) | Yes, RX/TX + IRQ + DREQ | Yes | Stub | **Missing** |
| USB (CDC) | Yes | Yes | No | **Missing** |
| TIMER (64-bit) | Yes | Yes | Stub | **Missing** |
| CLOCKS/PLL/XOSC/RESETS | Partial (`clk_sys` fixed) | Partial (PLL_SYS `*`, XOSC missing) | Working clock tree | **Missing** |
| SIO / GPIO | Yes + core1 rendezvous | Yes | Yes | GPIO model only |
| PIO / DMA | Yes / partial | Yes / partial | Working PIO, DMA stub | **Missing** |
| MicroPython boot | Not documented | Yes (ARM + RISC-V) | No | No |
| REPL host bridge | Host callbacks | GDB/MCP + serial | None | **Missing** |
| Implementation language | TypeScript | TypeScript | Rust | Rust |

---

## 3. Verified Target Firmware Facts / 目标固件实证

### 3.1 UF2 container / UF2 容器

| Property | Value |
|---|---|
| File | `E:\Source\mcu\pico\python\micropython\RPI_PICO2-20260824-v1.29.0.uf2` |
| Size | 661,504 bytes |
| Blocks | 1,292 x 512 bytes |
| Family IDs | `0xE48BFF59` (RP2350 ARM Secure) x 1,291; `0xE48BFF57` (Absolute) x 1 |
| Flash load range | `0x1000_0000 .. 0x1005_0B00`; last absolute block at `0x10FF_FF00` |
| Absolute block | `0x10FF_FF00`, 256 bytes of `0xEF` (erased padding) |
| First flash words | `0x2008_2000` (SP), `0x1000_015B` (Reset, Thumb) |
| Boot stage | None; vector table directly at flash base |

The companion RISC-V build (`RPI_PICO2-RISCV-20260824-v1.29.0.uf2`, 835,584 bytes,
family `0xE48BFF5A`) starts with a picobin `IMAGE_DEF`-style header at
`0x1000_0000`; its entry/SP are not in the ARM vector-table layout. RISC-V boot
therefore needs either a small `IMAGE_DEF` parser or an explicit host-provided
entry, and must not reuse the ARM vector-fetch convention.

### 3.2 perfbench requirements / perfbench 运行条件

From MicroPython `tests/run-perfbench.py` and `tests/perf_bench/benchrun.py`:

1. Host opens the device through `pyboard` and calls `enter_raw_repl()` then
   `exec_(script)`; the script is the benchmark file followed by `benchrun.py`
   and `bm_run(N, M)`.
2. Device must support MicroPython raw REPL (Ctrl-A / Ctrl-B / Ctrl-C framing,
   `OK`/`\x04` responses, `\x04` end-of-output).
3. `time.ticks_us`, `time.ticks_diff`, `print()` must work; results are read as
   `"<ticks> <norm> <result>"`.
4. `feature_check/complex.py` and `feature_check/native_check.py` are executed
   first; failing them only SKIPs the affected benchmarks, it does not abort.
5. No filesystem access is required by the driver; no `pyb`/`machine` calls are
   required by `benchrun.py`.

For the stock Pico 2 image, the channel in step 1 must be **USB CDC ACM**. A UART
REPL path only works with a custom build (`MICROPY_HW_ENABLE_UART_REPL=1`), so it is
a debugging aid, not the acceptance path.

---

## 4. Gap List / 差距清单与优先级

### P0 — required for any MicroPython boot / 启动必需

| ID | Gap | Reference evidence | Impact |
|---|---|---|---|
| P0-1 | UF2 parser and address routing | `rp2350js_emulator/demo/load-uf2-rp2350.ts`, `rp2350js/src/utils/load-uf2.ts` | Cannot load the only provided firmware format |
| P0-2 | Flash XIP read window at `0x1000_0000` | `rp2350js/src/peripherals/xip_rp2350.ts`, `rp2350js_emulator/src/rp2350.ts` | Code fetch and literal loads fail immediately |
| P0-3 | Flash direct-boot handoff (vector at `0x1000_0000`) | Target UF2 words 0/1 | Core cannot start the image |
| P0-4 | MMIO framework + RESETS/CLOCKS/XOSC/PLL stubs | picoem clock tree; rp2350js peripherals | MicroPython `set_sys_clock_khz` hangs without clock status bits |
| P0-5 | SIO/GPIO + IO_BANK0/PADS_BANK0 stubs | all three | SDK early init and `machine.Pin` fail |
| P0-6 | TIMER (64-bit) model | `rp2350js_emulator/src/peripherals/timer.ts` | `time.ticks_us` and alarms fail |
| P0-7 | UART0 (PL011) model | `rp2350js_emulator/src/peripherals/uart.ts` | Needed for diagnostics and custom UART REPL builds |
| P0-8 | Hazard3 RV32IMAC core | `rp2350js/src/riscv/*`, `rp2350js_emulator/src/riscv/*` | RISC-V variant cannot run |
| P0-9 | RP2040 chip stub with visible unsupported errors | picoem `rp2040-emu` | Cannot construct/identify RP2040; no explicit limitation surface |

### P1 — required for acceptance / 验收关键路径

| ID | Gap | Reference evidence | Impact |
|---|---|---|---|
| P1-1 | USB device controller + DPRAM + CDC endpoints | `rp2350js/src/peripherals/usb.js`, `rp2350js_emulator/src/peripherals/usb.ts` | Stock image has no other REPL channel |
| P1-2 | Host-side raw-REPL bridge (pyboard-compatible) | `run-perfbench.py` | Required to drive perfbench and capture results |
| P1-3 | Clock/timer accuracy sufficient for `ticks_us` | picoem clock tree | Perfbench timing must be monotonic and non-zero |
| P1-4 | Thumb-32 coverage audit against MicroPython | c1570/rp2350js "basic M33" | Unknown instruction halts boot |

### P2 — quality and completeness / 质量与完整性

| ID | Gap | Impact |
|---|---|---|
| P2-1 | Real bootrom image execution | Not needed for the target flash image; improves fidelity |
| P2-2 | QMI/XIP cache timing and flash program/erase semantics | Not needed to run from flash; needed by `rp2.Flash` |
| P2-3 | PIO/DMA/HSTX/ADC/SPI/I2C/PWM | Not needed by perfbench; needed by other firmware |
| P2-4 | RISC-V `IMAGE_DEF` parsing and Xh3irq routing | Needed for the RISC-V MicroPython variant |
| P2-5 | Dual-core rendezvous and SIO doorbells | MicroPython may start core1 for background tasks |

---

## 5. Phased Landing Plan / 分阶段落地计划

Each stage must pass the four quality gates before it is committed:
`cargo fmt --all -- --check`,
`cargo clippy --all-targets --all-features -- -D warnings -W clippy::pedantic`,
`cargo test --all-features`,
`cargo doc --no-deps`.

### Stage 0 — Gap analysis (this document) / 差距分析

Deliverable: this file, plus `doc/README.md` and `doc/changelog.md` index entries.
Acceptance: the document matches the repository state and the verified UF2 facts.

### Stage 1 — UF2 loader / UF2 加载器

Deliverables:

- `rp2350-core/src/uf2.rs`: block validation, family-ID decoding, duplicate and
  missing-block checks, target-address routing to flash/SRAM, load-range reporting.
- `FirmwareImage` auto-detection (`ELF` by magic, `UF2` by first-block magic).
- Unit tests with synthetic blocks; a manual parse of the real MicroPython UF2
  (1,292 blocks, expected family and range) recorded in the report.

Acceptance: the loader loads the target UF2 into a flat flash region, reports
`0x1000_0000..0x1005_0B00`, and rejects malformed/out-of-range images with a
`CoreError::FirmwareError` message.

Known limitation: no bootrom `IMAGE_DEF` interpretation in this stage.

### Stage 2 — Hazard3 RISC-V basic core / Hazard3 RISC-V 基本核心

Deliverables:

- New crate `crates/rp2350-riscv` implementing `CpuCore` for `CpuArch::RiscVHazard3`.
- RV32I base, M extension, A extension (`lr.w`/`sc.w` with a single-hart reservation),
  and C (compressed) decoding.
- Machine-mode CSRs: `mstatus`, `mtvec`, `mepc`, `mcause`, `mie`, `mip`, `mscratch`,
  `mtval`, `misa`, `mhartid`, plus minimal `Xh3irq` `MEIEA/MEIPA/MEIFA/MEIPRA`.
- Direct-boot handoff: explicit `set_boot_vectors(sp, pc)` plus a documented default
  for the RISC-V picobin layout.

Acceptance: a hand-written RV32I/M/C test program executes and a trap/`ecall` round
trip works; the crate passes all four gates.

Known limitation: no user mode, PMP, `Zba/Zbb/Zbs/Zcb`, or full `IMAGE_DEF` boot.

### Stage 3 — RP2040 stub / RP2040 芯片桩

Deliverables:

- `ChipFamily`/`ChipCapabilities` in `rp2350-core`.
- `RP2040Chip::new()` is constructible and returns `ChipFamily::Rp2040`.
- Every unsupported RP2040 operation returns a visible
  `CoreError::UnsupportedChip("...")`; no silent success.

Acceptance: unit tests construct/identify RP2040 and assert the unsupported error
text; the RP2350 path is unchanged.

Known limitation: no Cortex-M0+ core, no RP2040 memory map, no RP2040 peripherals.

### Stage 4 — Flash/XIP, MMIO framework and boot peripherals / Flash 与启动外设

Deliverables:

- `FlashRegion` (XIP read window, host-writable, guest-write policy documented).
- MMIO register region abstraction for APB peripherals; unhandled accesses are
  logged once and return data-abort/zero per documented policy.
- Minimal models: RESETS, CLOCKS, XOSC, PLL_SYS, PLL_USB, WATCHDOG, TIMER,
  SIO, IO_BANK0, PADS_BANK0, UART0.
- ARM direct-boot from `0x1000_0000`; CLI `run --file <uf2> --arch arm`.
- Boot trace: PC sampling, unhandled-MMIO log, UART0 TX echo.

Acceptance (measured 2026-11-08): the ARM core boots from the flash vector table
through a 4 KB BootROM stub (`SP = 0x2008_2000`, `PC = 0x1000_015A`), programs
`SCB->VTOR` (`0xE000_ED08`) and enters the C runtime at `0x1000_0166`. A synthetic
UF2 that writes UART0 `DR` through the MMIO path yields host-visible `Hi\n`,
proving Flash XIP → `MmioRouter` → UART0 end to end.

Known limitation: on the stock MicroPython image execution stops inside the C
runtime because the Thumb-32 `op1 = 0b00` group (data-processing modified
immediate, `ADD`/`SUB SP` immediate) is not yet implemented by the ARM decoder
line. Measured trace: `mov.w r0, #0xD000_0000` is skipped, so the SIO `CPUID`
read returns the boot-stub mirror and the stub takes its core-1 re-entry path;
`sub.w sp, sp, #256` is skipped as well, leaving `SP` at the top of SRAM, so
`blx r0` faults at `0x2008_2000`. UART0 output therefore stays empty for the
stock image (USB REPL only).

`MmioDevice` / `mount_device` / `MmioRouter` are the public mounting contract for
out-of-tree devices; see `doc/architecture.md` §3.

### Stage 5 — USB CDC device + raw-REPL bridge / USB CDC 与 REPL 桥

> ✅ **已在本仓库落地（2026-11-14 同步）**：`rp2350-usb`（设备控制器模型：DPRAM/寄存器/别名/端点握手/IRQ14）
> 与 `rp2350-usb-host`（主机桥枚举 + CDC + raw REPL + perfbench），经 `MmioDevice` 挂载；
> CLI 子命令 `repl` / `perfbench`。原“独立子任务线”注记作废。
> ✅ **Landed in this repository (synced 2026-11-14)**: the `rp2350-usb` device controller
> model (DPRAM/regs/aliases/endpoint handshake/IRQ14) and `rp2350-usb-host` (host bridge
> enumeration + CDC + raw REPL + perfbench), mounted via `MmioDevice`; CLI subcommands
> `repl` / `perfbench`. The previous "separate sub-task line" note is obsolete.

Deliverables:

- USBCTRL_DPRAM at `0x5010_0000` and USBCTRL_REGS at `0x5011_0000`.
- Control endpoint handling for `GET_DESCRIPTOR`, `SET_ADDRESS`,
  `SET_CONFIGURATION`, CDC `SET_LINE_CODING`, `GET_LINE_CODING`.
- Two bulk endpoints with IN/OUT packet buffers and `BUFF_STATUS`/`INTR` semantics.
- Host bridge that emulates the `pyboard` serial protocol over the emulated CDC
  endpoints (raw REPL framing, `exec_`, output capture).

Acceptance: the image enumerates as a CDC ACM device and prints the MicroPython
REPL banner; Ctrl-C/Ctrl-D work.
Known limitation: no mass storage, HID, or USB suspend/resume fidelity.

### Stage 6 — MicroPython boot and perfbench / MicroPython 启动与基准测试

Deliverables:

- Boot bring-up fixes (clocks, interrupts, Thumb-32 gaps, heap).
- `perfbench` runner: load `bm_*.py` + `benchrun.py`, call `bm_run(N, M)`, parse
  `"<ticks> <norm> <result>"`, and report per-benchmark status.
- Documented result: which benchmarks complete, which SKIP (no `complex`/Viper),
  and which fail with the exact halt reason.

Acceptance: at least one `bm_*.py` benchmark returns a numeric result through the
emulated REPL; any remaining failure is reported with evidence, not inferred.

---

## 6. Design Decisions / 设计要点

### 6.1 Memory map constants / 内存映射常量

| Region | Base | Size | Notes |
|---|---|---|---|
| Flash XIP | `0x1000_0000` | 16 MB modelled | Read-only to the guest; host loader writes it |
| SRAM | `0x2000_0000` | 520 KB | `0x2000_0000..0x2008_2000` |
| APB peripherals | `0x4000_0000` | - | RESETS, CLOCKS, TIMER, UART0, ... |
| USB DPRAM | `0x5010_0000` | 4 KB | Endpoint buffers and setup packets |
| USB regs | `0x5011_0000` | 4 KB | SIE control/status |
| SIO | `0xD000_0000` | 4 KB | GPIO, FIFO, spinlocks |

### 6.2 Boot handoff / 启动交接

- **ARM (target image)**: read `SP` from `0x1000_0000` and `PC` from `0x1000_0004`;
  this is the RP2350 direct-flash-boot convention used by the provided UF2.
- **ELF**: keep the existing entry-point/segment flow.
- **RISC-V**: explicit host-provided or parsed `IMAGE_DEF` entry; never apply the
  ARM vector layout.
- **RP2040**: unsupported; return `CoreError::UnsupportedChip`.

### 6.3 Stub policy / 桩实现策略

1. A stub must be constructible and identifiable.
2. Any unimplemented capability must return a visible error or log a one-shot
   "unimplemented MMIO" warning with the address and PC; never silently pretend.
3. Register reads return documented reset values (not random data) so firmware
   polling loops can make progress; polling loops still need emulator-side timeouts.
4. Documents must label every stub and its limitation.

### 6.4 No vendoring / 不引入第三方源码

The reference repositories are cloned outside the workspace (system temp) for study
only. No third-party source is copied into this repository; implementations are
written from the documented behavior and validated with unit tests.

---

## 7. Risks & Open Questions / 风险与待定问题

| Risk | Mitigation |
|---|---|
| USB device model is the acceptance bottleneck | Implement control + CDC bulk endpoints first; defer MSC/HID |
| Clock tree hangs (`while (!xosc_stable)`) | Return stable status bits immediately; add loop timeouts |
| Thumb-32 coverage unknown | Add a PC-sampled fault trace; expand decoders driven by the boot trace |
| perfbench absolute scores meaningless | Report completion and relative consistency, not silicon-comparable MCPS |
| RISC-V `IMAGE_DEF` format undocumented here | Stage 2 uses explicit entry; Stage 6 parses the real header |
| Concurrent edits to `crates/rp2350-egui` | Never touch that crate; stage only files owned by this work |

---

## 8. Documentation Sync Checklist / 文档同步清单

- [x] `doc/README.md`: link this document.
- [x] `doc/architecture.md`: chip family, Flash/XIP, MMIO, USB rows.
- [x] `doc/implementation-plan.md`: replace Phase 10 "future" with the staged plan.
- [ ] `doc/crates/rp2350-core.md`: UF2 loader and RP2040 stub API.
- [ ] `doc/crates/rp2350-riscv.md`: new crate API and limitations.
- [x] `doc/changelog.md`: one entry per stage.
- [x] `README.md`: quick-start commands for UF2 loading and RISC-V selection.
