# Implementation Plan / 实施计划

> **Version**: 3.0 | **Date**: 2026-11-08

---

## Phase 1 · Foundation (Week 1)

### Deliverables / 交付物

| ID | Task | Crate | Output |
|----|------|-------|--------|
| 1.1 | Workspace scaffold + Cargo.toml with dependencies | root | `cargo build --all` passes |
| 1.2 | `BusDevice` trait + `AtomicType` + `AddressBus` route logic | rp2350-core | Region routing with correct base/size match |
| 1.3 | `RP2350Chip` struct skeleton | rp2350-core | All fields initialized, no panic |
| 1.4 | `GpioPin` + `PinFunction` + `PinDirection` (48 pins) | rp2350-core | Unit tests: function switch, direction, output |
| 1.5 | `SimulationClock` + clock divider logic | rp2350-core | Unit tests: divider rollover |
| 1.6 | `IrqMatrix` (256 IRQs, pending/enabled) | rp2350-core | Unit tests: set/clear/get |
| 1.7 | `CpuCore` trait definition | rp2350-cpu | Trait compiles, doc tests pass |
| 1.8 | `Peripheral` trait (base for APB peripherals) | rp2350-periph | Trait compiles |
| 1.9 | `RpPlugin` trait + `PluginContext` trait + `declare_plugin!` macro | rp2350-plugin | Trait compiles, macro generates exports |
| 1.10 | `DevicePlugin` trait + `DevicePort` + `ConfigSchema` | rp2350-virt-periph | Trait definitions complete |
| 1.11 | `SafeLibrary` RAII wrapper (libloading) | rp2350-ffi | dlopen/dlclose works on all platforms |
| 1.12 | `PluginLoader` + `WiringTable` + `DeviceManager` skeleton | rp2350-virt-periph | scan/load/unload/wire works |
| 1.13 | `eframe` App skeleton with menu bar + status bar | rp2350-egui | Window launches, menu items clickable |
| 1.14 | `clap` CLI: `run`, `gdb`, `wire`, `list-devices`, `mcp-serve` subcommands | rp2350-cli | `--help` output correct |
| 1.15 | Git init + pre-commit hooks | root | `git commit` success |
| 1.16 | Fill `doc/README.md` + all crate stub docs | doc/ | `cargo doc` generates complete docs |

### Milestone M1 / 里程碑 M1

- `cargo build --all` 零错误
- `AddressBus` 能正确路由到不同地址区域
- egui 空窗口启动成功

---

## Phase 2 · ARM Cortex-M33 Basic (Week 2-3)

### Deliverables / 交付物

| ID | Task | Output |
|----|------|--------|
| 2.1 | `CortexM33Registers`: R0-R15, MSP/PSP, XPSR, CONTROL, PRIMASK, BASEPRI, FAULTMASK | registers.rs |
| 2.2 | Thumb-16 decoder + executor (~90 instructions) | execute_thumb16.rs |
| 2.3 | Thumb-32 basic decoder (~60 ALU + Load/Store, no DSP/FPU) | execute_thumb32.rs |
| 2.4 | Condition flags (NZCVQ) + IT block execution | conditions.rs |
| 2.5 | NVIC: ISER/ICER/ISPR/ICPR/IPR/STIR, priority grouping | nvic.rs |
| 2.6 | Exception entry/exit (stacking/unstacking, EXC_RETURN) | exceptions.rs |
| 2.7 | SysTick timer + PPB base registers (SCB, VTOR) | systick.rs + ppb.rs |
| 2.8 | `CortexM33Core` impl of `CpuCore` trait | core.rs |
| 2.9 | BOOTROM load + reset vector fetch + SP/PC init | chip boot sequence |
| 2.10 | egui register viewer panel (R0-R15 table) | registers.rs panel |
| 2.11 | `GdbTarget` trait + `ArmGdbTarget` stub | gdb target.rs |

### Milestone M2 / 里程碑 M2

- BOOTROM 能够执行到固件入口
- 基础 Thumb-16 指令集可运行 blink 固件
- egui 寄存器查看器实时刷新

---

## Phase 3 · Initial Six Virtual Peripherals (Week 3-5) ★

All six devices are built as independent plugins in `plugins/`.
Loaded and managed by `DeviceManager` (rp2350-virt-periph).
Supports both GUI and headless/MCP mode.

### Deliverables / 交付物

| ID | Task | Crate | Output |
|----|------|-------|--------|
| **DeviceManager Completion** | | | |
| 3.1 | `DeviceManager` full impl: scan_and_load, wiring, notify, tick, MCP query | rp2350-virt-periph | All methods functional |
| 3.2 | `PluginLoader` full impl: platform-agnostic directory scan, ABI validation | rp2350-virt-periph | Cross-platform .dll/.so/.dylib loading |
| **LED Plugin** / 插件 `plugins/led/` | | | |
| 3.3 | `LedPlugin`: impl DevicePlugin (single port, on_gpio_change, state_snapshot) | plugins/led | Unit tests: GPIO → LED level |
| 3.4 | egui LED widget (glow circle, brightness, tooltip) | plugins/led | has_ui()=true, render_ui() working |
| **TF Card Plugin** / 插件 `plugins/tfcard/` | | | |
| 3.5 | `TfCardPlugin` SPI mode: CS/MOSI/MISO/SCK ports + byte-level state machine | plugins/tfcard | SPI init sequence (CMD0→CMD1→CMD58) |
| 3.6 | SD SPI command set: CMD0/1/8/17/24/58/ACMD41 | plugins/tfcard | FAT32 partition readable |
| 3.7 | SDIO 4-bit mode: CLK/CMD/DAT0-3 ports + 4-bit data transfer | plugins/tfcard | SDIO init → CMD2/3/7/6 → 4-bit mode |
| 3.8 | Disk image load/save + config_schema (mode, image_path, block_size) | plugins/tfcard | JSON config via apply_config() |
| 3.9 | egui TF Card widget (mode switch, capacity, status, file loader) | plugins/tfcard | has_ui()=true |
| **UART Terminal Plugin** / 插件 `plugins/uart-terminal/` | | | |
| 3.10 | `UartTermPlugin`: TX/RX buffers + baud rate emulation | plugins/uart-terminal | Bidirectional serial |
| 3.11 | egui terminal panel (VT100, scrollback, hex mode) | plugins/uart-terminal | has_ui()=true, terminal output + keyboard input |
| **Debug Probe Plugin** / 插件 `plugins/debug-probe/` | | | |
| 3.12 | `DebugProbePlugin`: register access + HW breakpoints + ITM trace | plugins/debug-probe | Debug operations via state_snapshot() |
| 3.13 | egui debug panel (core select, breakpoint list, step, trace view) | plugins/debug-probe | has_ui()=true |
| **PSRAM Plugin** / 插件 `plugins/psram/` | | | |
| 3.14 | `PsramPlugin`: APS6404L QSPI command set + latency simulation | plugins/psram | Jedec ID + Read/Write + Quad mode |
| 3.15 | config_schema (capacity_mb, qspi_mode) + state_snapshot (stats) | plugins/psram | JSON config + MCP query |
| 3.16 | egui PSRAM widget (capacity, mode, R/W statistics, QSPI ports) | plugins/psram | has_ui()=true |
| **QSPI Flash Plugin** / 插件 `plugins/qspi-flash/` | | | |
| 3.17 | `QspiFlashPlugin`: W25Q128 command set + program/erase state machine | plugins/qspi-flash | All QSPI commands functional |
| 3.18 | Binary file load/save + config_schema (capacity_mb, jedec_id) | plugins/qspi-flash | Firmware loading via JSON config |
| 3.19 | egui Flash widget (Jedec ID, status regs, sector map, stats) | plugins/qspi-flash | has_ui()=true |
| **Virtual Lab Canvas** / 虚拟实验室画布 | | | |
| 3.20 | `ChipViewWidget` — RP2350 package with 48 draggable pins | rp2350-egui | Chip body + pins drawn |
| 3.21 | `DragWire` system: palette drag → canvas placement → pin↔port wire creation | rp2350-egui | Full drag-wire UX |
| 3.22 | Bezier `Wire` rendering with signal animation (level = color) | rp2350-egui | Wires reflect GPIO state |
| 3.23 | Canvas layout save/load (JSON) via DeviceManager::wiring_to/from_json() | rp2350-egui | Layout persistence |
| **MCP & Headless Verification** / 无头与 MCP 验证 | | | |
| 3.24 | All six plugins pass `headless=true` test (no egui rendered) | plugins/* | state_snapshot() correct |
| 3.25 | MCP query_all test: wire LED→GPIO25, toggle, verify state_snapshot | tests/ | MCP round-trip passes |
| 3.26 | MCP configure test: change TF card mode SPI↔SDIO via apply_config() | tests/ | Config changes persist |

### Milestone M3 / 里程碑 M3

- egui 虚拟实验室中六个外设全部可拖拽、连线、实时响应用户操作
- 全部六个外设在无头模式下通过 `state_snapshot()` 可查询完整状态
- MCP `mcp_query_all` 返回所有设备状态聚合 JSON
- MCP `mcp_configure` 可修改设备配置（JSON → apply_config）
- TF 卡 SPI 模式和 SDIO 模式均可正常初始化并读写数据
- PSRAM / QSPI Flash 可通过 QSPI 命令正常访问
- 插件 ABI 设计验证通过：所有设备作为独立 `.dll` 加载/卸载/热替换

---

## Phase 4 · PIO + DMA (Week 6-7)

| ID | Task | Output |
|----|------|--------|
| 4.1 | PIO instruction set: JMP/WAIT/IN/OUT/PUSH/PULL/MOV/IRQ/SET | instr.rs + sm.rs |
| 4.2 | PIO FIFO (TX/RX, autopush/pull threshold, FJOIN_RX/FJOIN_TX) | fifo.rs |
| 4.3 | PIO timing (clkdiv counter, delay, side-set, stall) | sm.rs timing logic |
| 4.4 | PIO Assembler (text → binary 16-bit) | assembler.rs |
| 4.5 | PIO IRQ system (INTR, IRQ0/1_INTE/INTF/INTS, inter-PIO IRQ) | irq.rs |
| 4.6 | DMA 16-channel + chaining + DREQ handshake + byte/hword/word transfers | dma.rs |
| 4.7 | PIO TX/RX DREQ → DMA data flow | pio-dma integration |
| 4.8 | egui PIO status panel (instruction view, SM registers, FIFO levels) | pio_view.rs |
| 4.9 | Cross-validate PIO programs with rp2350js snapshots | tests/ |

### Milestone M4 / 里程碑 M4

- PIO 程序正确驱动 GPIO 波形 (blink via PIO)
- DMA 链式传输从 PIO RX FIFO → SRAM 正常
- `pioasm` 汇编器输出与 SDK 一致

---

## Phase 5 · HSTX + Display (Week 8)

| ID | Task | Output |
|----|------|--------|
| 5.1 | HSTX FIFO (8 lanes, CSR, CLKDIV, DMA DREQ) | hstx.rs |
| 5.2 | HSTXDecoder: RGB565/RGB888/YCbCr422/Raw8/Raw16 → RGBA8 | decoder.rs |
| 5.3 | HSTX → mpsc channel → egui texture upload | hstx_view.rs |
| 5.4 | Frame rate control (auto-scale, FPS counter) | hstx_view.rs |
| 5.5 | Screenshot → PNG (via `image` crate) | screenshot.rs |
| 5.6 | Headless mode: HSTX capture without GUI window | CLI flag |

### Milestone M5 / 里程碑 M5

- HSTX 像素流实时显示在 egui 面板中
- 截图导出 PNG 正常

---

## Phase 6 · XIP Integration (Week 9)

| ID | Task | Output |
|----|------|--------|
| 6.1 | QMI direct mode (CSR/TX/RX FIFO, bus busy flag, enable) | qmi.rs |
| 6.2 | XIP address window routing: 0x1000_0000 → Flash, 0x1100_0000 → PSRAM | xip mod |
| 6.3 | QSPI Flash XIP read → direct `data[addr]` for performance | flash XIP path |
| 6.4 | PSRAM XIP read/write → direct memory access | psram XIP path |
| 6.5 | Bootrom QMI polling loop emulation (CSR status bits) | QMI bootrom compat |
| 6.6 | Flash busy state propagation through QMI CSR | busy flag timing |

### Milestone M6 / 里程碑 M6

- 固件通过 XIP 窗口从 Flash 执行代码 (XIP execute-in-place)
- PSRAM 可通过 XIP 别名窗口读写

---

## Phase 7 · DSP + FPU + TrustZone (Week 10-11)

| ID | Task | Output |
|----|------|--------|
| 7.1 | ARMv8-M DSP: SADD8, SSUB8, USAD8, USADA8, SADD16, SSUB16, SHADD16, SHSUB16 | execute_dsp.rs |
| 7.2 | Saturating: SSAT, USAT, QADD, QSUB, QDADD, QDSUB | execute_dsp.rs |
| 7.3 | Multiply-accumulate: SMLAD, SMLSD, SMLALD, SMLSLD | execute_dsp.rs |
| 7.4 | Bitfield: BFC, BFI, SBFX, UBFX, RBIT, REV, CLZ | execute_dsp.rs |
| 7.5 | SDIV, UDIV, SMULL, UMULL, SMLAL, UMLAL | execute_dsp.rs |
| 7.6 | VFPv5-SP-D16: VADD, VSUB, VMUL, VNMUL, VDIV, VSQRT | execute_fpu.rs |
| 7.7 | VLDR, VSTR, VMOV (imm+reg), VCVT, VCMP, VCMPE | execute_fpu.rs |
| 7.8 | FPSCR exception flags (IOC/DZC/OFC/UFC/IXC/IDC) | fpu_helpers.rs |
| 7.9 | TrustZone: SAU, IDAU, Secure/NonSecure memory attribution | sau.rs |
| 7.10 | Security extension registers (MSP_NS, PSP_NS, MSPLIM, PSPLIM) | registers.rs |

### Milestone M7 / 里程碑 M7

- CMSIS-DSP 测试用例通过
- FPU 基本运算正确 (IEEE 754 single-precision)
- Secure/NonSecure 内存访问属性正确

---

## Phase 8 · GDB Server Complete (Week 12)

| ID | Task | Output |
|----|------|--------|
| 8.1 | GDB RSP protocol: all 'g/G/m/M/c/s/Z/z/v/q' packet handlers | protocol.rs |
| 8.2 | Multi-core support: Hg/Hc, vCont (c/s with thread id) | arm_target.rs |
| 8.3 | Software breakpoint (BKPT #0 injection + restore) | breakpoint.rs |
| 8.4 | Hardware breakpoint (FPB unit, 8 slots) | hw_breakpoint.rs |
| 8.5 | Watchpoint (DWT unit, 4 slots, read/write/access) | watchpoint.rs |
| 8.6 | Target XML for Cortex-M33 (VFP + system regs) | arm_target.xml |
| 8.7 | Monitor commands: pio, gpio, dma, hstx, registers, memory, info, trace | monitor.rs |
| 8.8 | ITM trace output via GDB (optional) | trace.rs |
| 8.9 | GDB server integration test (connect → bp → step → registers → detach) | tests/ |

### Milestone M8 / 里程碑 M8

- arm-none-eabi-gdb 可连接、设断点、单步、查看寄存器/内存、detach
- `monitor gpio` 等命令正确输出

---

## Phase 9 · Integration & Polish (Week 13-14)

| ID | Task | Output |
|----|------|--------|
| 9.1 | MicroPython ARM variant: boot to REPL via UART terminal | integration test |
| 9.2 | FreeRTOS context switch test | multi-task validation |
| 9.3 | Cycle accuracy calibration vs rp2350js (cross-validation) | accuracy report |
| 9.4 | Performance: instruction decode cache, batch execution, hot-path inlining | 100+ MCPS target |
| 9.5 | CLI daemon mode for CI/headless use | daemon.rs |
| 9.6 | Complete `cargo doc` API documentation | doc comments 100% |
| 9.7 | Quality gate: `cargo fmt`, `cargo clippy`, `cargo test`, `cargo doc` | all zero-warning |
| 9.8 | TF card plugin example documentation + build script | examples/tf_card_plugin/ |

### Milestone M9 / 里程碑 M9

- MicroPython REPL 通过 UART 终端交互
- 性能 ≥ 100 MCPS (release)
- 所有质量门禁通过

---

## Phase 10 · RISC-V (Future / 远期)

| ID | Task |
|----|------|
| 10.1 | RISC-V Hazard3: RV32IMAC + Zcb + Zba + Zbb + Zbs |
| 10.2 | RISC-V CSR registers + Machine mode |
| 10.3 | Xh3irq (platform interrupt controller) |
| 10.4 | RISC-V GDB Target |
| 10.5 | Heterogeneous dual-core (ARM + RISC-V) |
| 10.6 | RISC-V MicroPython boot test |

---

## Phase 11 · WASM / Web Publishing (Future / 远期)

| ID | Task | Output |
|----|------|--------|
| 11.1 | Add `wasm32-unknown-unknown` target to all crates | `cargo check --target wasm32` passes |
| 11.2 | Feature flags for device selection (`plugin-led`, `plugin-tfcard`, etc.) | Cargo features in root Cargo.toml |
| 11.3 | `eframe` wasm runner: `App::new()` → `wasm_bindgen(start)` | Browser loads wasm, shows egui |
| 11.4 | `trunk serve` dev workflow (`index.html` + wasm bundle) | Local dev server with HMR |
| 11.5 | Web-specific plugin loading: `DeviceManager::static_register()` | Static device registration without `libloading` |
| 11.6 | Virtual Lab Canvas in browser (WebGL2 / WebGPU backend) | Full virtual lab runs in browser |
| 11.7 | File I/O via `web-sys`: firmware upload dialog, disk image browse | Browser-native file picker |
| 11.8 | Layout persistence via `localStorage` | Wiring layout survives page refresh |
| 11.9 | CI: `trunk build --release` in GitHub Actions | Auto-publish to GitHub Pages |
| 11.10 | Deploy to GitHub Pages via `peaceiris/actions-gh-pages` | Public URL: `https://czlz.github.io/rp2350-rs/` |
| 11.11 | WASM binary optimization (`wasm-opt -Oz`) | Minimal `.wasm` size (target < 5MB) |
| 11.12 | `index.html` assets: manifest.json, favicon, service worker (PWA) | Installable PWA |

### Milestone M11 / 里程碑 M11

- RP2350 模拟器可通过浏览器访问，无需安装
- egui 虚拟实验室完整运行在 WebGL2/WebGPU 上
- 固件和磁盘镜像通过浏览器文件对话框加载
- GitHub Pages 自动部署，每次 push 更新

---

## Verification Matrix / 验证矩阵

### Test Per Type / 各类型测试

| Type | Count (min) | Coverage |
|------|-------------|----------|
| Unit tests | ≥ 1 per `pub fn` | All public functions, error paths |
| Doc tests | all `/// ```rust` examples | API examples compile + run |
| Integration tests | 20+ | Bootrom, interrupts, PIO, DMA, Flash, GDB |
| Snapshot tests | 10+ | Register state vs rp2350js |
| Firmware tests | 5+ | blink, hello_usb, MicroPython, FreeRTOS |

### Key Firmware Test Cases / 关键固件测试用例

| Firmware | Validates |
|----------|-----------|
| `blink_simple` | GPIO output, SysTick |
| `hello_usb` | USB CDC, UART |
| `pio_blink` | PIO + GPIO side-set |
| `pio_dma` | PIO + DMA data flow |
| `spi_sd_card` | SPI + TF card (SPI mode) |
| `sdio_sd_card` | SDIO + TF card (SDIO mode) |
| `psram_test` | PSRAM QSPI read/write |
| `flash_program` | Flash QSPI erase/program |
| `hstx_colorbar` | HSTX pixel output |
| `micropython` | Full system integration |
| `freertos_blinky` | Multi-tasking + context switch |

---

## Quality Gates Per Phase / 各阶段质量门禁

```bash
# Every phase must pass:
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings -W clippy::pedantic
cargo test --all-features
cargo doc --no-deps --document-private-items

# Phase 11+: additionally
rustup target add wasm32-unknown-unknown
cargo check --target wasm32-unknown-unknown --all-features
trunk build --release  # WASM production build
```

No `#[allow]` without bilingual justification comment.
无双语理由注释的 `#[allow]` 不允许。

All `unsafe` blocks must have `// SAFETY:` bilingual comment.
所有 `unsafe` 块必须有 `// SAFETY:` 双语注释。