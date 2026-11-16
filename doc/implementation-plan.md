# Implementation Plan / 实施计划

> **Version**: 3.1 | **Date**: 2026-11-14 | **Phase 1: ✅ COMPLETE** | **Phase 2: ✅ COMPLETE** | **验收主线 Stages 0–6: ✅**

> **当前验收主线 / Current acceptance mainline**: [reference-gap-analysis.md](reference-gap-analysis.md) 的
> Stages 0–6 —— 真实 MicroPython `RPI_PICO2-*.uf2` → USB CDC → raw REPL → `perfbench`。
> Stages 0–5 已落地（含 `rp2350-usb`/`rp2350-usb-host` 两个 crate 与 CLI `repl`/`perfbench`
> 子命令，超出本计划原始 Phase 1 清单）；Stage 6 的官方 `run-perfbench.py` 结果见
> [perfbench-report.md](perfbench-report.md)。另一条端到端验收主线是
> [example/gba/doc/gba-bringup-report.md](../example/gba/doc/gba-bringup-report.md)：`emerald.uf2`（pokeemerald-rp2350）
> 双核运行、HSTX 扫描输出真实游戏画面。
> The acceptance mainline runs through reference-gap-analysis Stages 0–6; stages 0–5
> have landed (including the `rp2350-usb` / `rp2350-usb-host` crates and the CLI
> `repl` / `perfbench` subcommands, beyond the original Phase 1 list), and the Stage 6
> official `run-perfbench.py` results are in [perfbench-report.md](perfbench-report.md).
> A second end-to-end mainline is [example/gba/doc/gba-bringup-report.md](../example/gba/doc/gba-bringup-report.md):
> `emerald.uf2` (pokeemerald-rp2350) on two cores with HSTX scanout of real game frames.

> **P1 功能进度 / P1 feature progress（2026-11-14）**
>
> | 领域 / Area | 状态 / Status | 已落地 / Landed | 待办 / Remaining |
> |---|---|---|---|
> | PIO | 🚧 | 3 blocks × 4 SMs 寄存器块、FIFO、指令存储；状态机执行（9 类指令、autopush/autopull、wrap、stall）；引脚映射、side-set、delay、CLKDIV、FIFO join、DBG_PADOUT/PADOE、共享句柄、IRQ 路由、主循环 tick、DMA DREQ 选择器 0..23 | PINCTRL FUNCSEL、跨块 IRQ 模式 1/3 |
> | GDB | ✅ | RSP 协议层、TCP 会话循环、CLI `gdb` 接线（`MachineTarget`）、写/读/访问监视点（`Z2`/`Z3`/`Z4`，`T05watch/rwatch/awatch`）、`target.xml`、`qRcmd` → `monitor reset/info/halt` | 多核 `vCont`、扩展 monitor 命令 |
> | MCP | ✅ | JSON-RPC 方法层（`initialize`/`tools/list`/`tools/call`）、HTTP 传输、CLI `mcp-serve`、Bearer 鉴权（`--auth-token`/环境变量） | TLS、插件热加载 |
> | 插件 / Plugins | ✅ | ABI、`DeviceManager::scan_and_load`/`reload`、CLI `list-devices`/`wire`/`--reload`/`--watch`、LED 与 UART Terminal 插件 | GUI 调色板/连线、接线持久化 |
> | DMA | 🚧 | 16 通道模型、控制块、AL 触发、byte/half/word、RING、CHAIN_TO、DREQ（含 PIO 0..23 与 `DREQ_HSTX=52`）、abort、IRQ、别名窗口、`TRANS_COUNT` 重载/`TRIGGER_SELF`/`ENDLESS`、`BSWAP`、`SNIFF`（CRC-32/CRC-16/奇偶/求和 + `BSWAP`/`OUT_REV`/`OUT_INV`）、`HIGH_PRIORITY`、节拍定时器 `(X/Y)*sys_clk`、3 通道环形链校验、CLI 每 64 步泵送 | `SNIFF` 其余 `CALC` 模式、周期精确仲裁、`PERFCTR` |
> | HSTX | 🚧 | APB 控制 + AHB FIFO、8 字 FIFO/WOF、RAW/TMDS/NOP 命令、像素时钟节拍（256 字行块 = 800 像素周期）、`DREQ_HSTX` FIFO 空间、宿主 drain/scanline、下溢 IRQ | DVI/TMDS 时序与编码、引脚/时钟、PIO 耦合 |
> | ARM ISA | ✅ | Thumb-16/32、FPv5-SP、单精度算术/比较/转换/VSEL/VRINT、FPCCR/LSPACT 惰性压栈与扩展帧、DSP 并行 SIMD/GE 标志、饱和/打包、折半/双乘/宽乘、`MSPLIM`/`PSPLIM` | 双精度算术、FPSCR 陷阱模型、`SMLALxy` |
> | 官方 perfbench | ✅ | `repl --stdio` + 本地 pyboard shim；Arm 与 RISC-V 各 24/24 通过 CPython 真值（[报告](perfbench-report.md)） | 可选：`-a 8` 方差、更大 N/M、外设类基准 |
> | 双核 / Dual-core | ✅ | 真实 core1 执行：SIO 按核视图与 `{0,0,1,vtor,sp,entry}` 启动握手、PSM `FRCE_OFF.PROC1` 复位/释放、按核 PPB 回放与 IRQ 路由、`Machine` 双核步进；SIO 硬件自旋锁 `SPINLOCK0..31`（读即取/写即放/`SPINLOCK_ST`）、每核独占监视器 + `CLREX`；emerald 的 `core1_render` 驱动 HSTX/PPU | `WFI`/`WFE`/`SEV` 睡眠（需先实现阻塞式 `SIO_FIFO_RD`/FIFO IRQ，见 `9a6b088`）、GDB 多核 `vCont`、跨核总线监听 |
>
> 状态标记：✅ 完成（P1 范围） · 🚧 部分完成 · ⏳ 未开始。
> Status markers: ✅ done for P1 scope · 🚧 partial · ⏳ not started.

> **P2 可玩性（当前阶段）/ Playability (current phase)**
>
> 目标：人类可以在模拟器里正常游玩 `emerald.uf2`（实时画面 + 键盘输入 + 音频）。
> 计划、硬指标与波次见 [playability-plan.md](playability-plan.md)；新缺口 GAP-13…GAP-17
> 已加入 skill 的 GAP 表。**一级阻塞是性能**：当前 3.0 M steps/s，实时需 ≥50 M steps/s。
> Goal: a human can play `emerald.uf2` in real time (video + keyboard + audio). See
> [playability-plan.md](playability-plan.md); the new gaps GAP-13…GAP-17 are in the skill's
> GAP table. **Performance is the primary blocker**: 3.0 M steps/s now vs ≥50 M needed.

> **任务队列 / Task queue（主会话拆分，子会话实现）**
>
> 规则见 `.trae/rules/project.md` 的「角色边界」「子会话使用规则」与
> 「定位：精确时钟与精确指令优先（强制）」：主会话只做分析、拆分、派发与验收；
> **一个任务 = 一个全新子会话**，禁止复用已完成的会话；**不做 JIT**，精度优先。
>
> | ID | 任务 / Task | 归属 / Crates | 状态 |
> |----|-------------|---------------|------|
> | GAP-01a/01b | `IO_BANK0` FUNCSEL + PIO/SIO 引脚矩阵 | periph | ✅ |
> | GAP-02/02b | GDB 多核线程、`vCont` 按核步进、混合动作列表 | gdb + cli | ✅ |
> | GAP-03 | ARM `SMLALxy` + `FPCCR` 行为位 | arm | ✅ |
> | GAP-04 | RISC-V 非法 CSR + ROM 表 + `misa.B` | riscv + bootrom | ✅ |
> | GAP-05 | USB 1 ms SOF + 双缓冲 OUT | usb-host + usb | ✅ |
> | GAP-06 | HSTX 原始路径计时 + 像素捕获 | periph | ✅ |
> | GAP-07 | 阻塞 FIFO/IRQ + `WFI`/`WFE`/`SEV` + 睡眠调度 | periph → arm/riscv/cli | ✅ |
> | GAP-08/08b | PSRAM 模型 + CLI `--psram` 挂载 | periph + cli | ✅ |
> | GAP-09 | egui 调色板 + 画布持久化 | egui | ✅ |
> | GAP-10 | MCP TLS 传输 | mcp | ⏸ 按需（需引入 rustls 依赖，调试服务器收益有限，默认不实施） |
> | GAP-11 | DMA `CALC` 6 模式 + `PERFCTR` + 总线记账 | periph | ✅ |
> | GAP-12 | 文档陈旧段落同步 | doc | 🚧 主会话 |
> | GAP-18 | 时钟精度（PLL/分频/超频） | periph + core + cli + usb-host | ✅（P4-1/P4-2） |
> | P1-1/2/3 | `play` worker、egui PlayView、示例 profile | cli + egui + example | ✅ |
> | P2-1/2/3 | I2S 捕获、`rp2350-audio`、GUI 音频输出 | periph + audio + egui | ✅ |
> | P3-1 | CLI 挂载 PSRAM（= GAP-08b） | cli | ✅ |
> | P3-3 | 长时间 soak（headless） | 主会话 | ✅ 2 G 步完整运行 + 30 分钟采样，工作集恒定（见 performance.md §10） |
> | P4-1 | 时钟树（XOSC/ROSC/PLL/CLOCKS + 超频） | periph + core | ✅ |
> | P4-2 | 动态频率接入全部计时消费者 | periph + core + cli + usb-host | ✅ |
> | P4-2b | SysTick/PendSV 异常号与向量 | arm + periph | ✅ |
> | P4-2c | CLI 转发 SysTick 挂起 | cli | ✅ |
> | P4-2d | SVCall/HardFault/NMI/故障升级 | arm | ✅ |
> | P4-2e | PPB 故障寄存器回放 + `SYSRESETREQ` 策略 | periph + cli | 🚧 |
> | P4-2f | `CCR` 陷阱（未对齐/除零）+ `SHPR1` | arm | 🚧 |
> | P4-3 | 时钟频率回归矩阵（150/252/降频/源切换）+ 文档 | 主会话 | ⏳ |
> | GAP-19 | MCP TLS（可选安全加固） | mcp | ⏳ |
> | GAP-20 | SysTick 投递时延收紧（10 064 步 → 逐周期） | periph + cli | ⏸ 可选（已知限制，~67 µs @150 MHz） |
> | MT-1 | 多线程测量与原型（逐核/锁成本/加速比） | core + cli | ✅ |
> | MT-2a | 线程安全总线（10 bank、Flash 免锁、MMIO 每设备锁） | core | ✅ |
> | MT-2b | TIMER 原子快照 | periph | ✅ |
> | MT-3a | 睡眠核快进（启动 56–66×） | cli + periph | ✅ |
> | MT-3b | 双核线程可选模式（整跑 1.97×） | cli + core + arm | ✅ |
> | MT-5 | 插件执行器线程化（+ 并行 flake 修复） | virt-periph | ✅ |
> | MTV-6 | 插件宿主接线（CLI/GUI 真正 step 插件 + `--plugin-threads`） | cli + egui | ⏳ 下一会话 T1 |
> | T2 | CPU 每指令周期成本（当前 steps==cycles 近似） | arm + cli | ⏳ 下一会话 T2（先设计/测量） |
> | T3 | 存档/状态保存与 flash 路径 | cli + egui | ⏳ 下一会话 T3 |
> | T4 | 长跑 soak 与静置机 A/B 复测 | 主会话 | ⏳ 下一会话 T4 |
>
> **交接 / Handoff**：下一会话请先读 [`handoff.md`](handoff.md)（自包含的定位、基线、代码地图、
> 验收命令、T1–T6 任务与经验教训）。

> **验收记录 / Acceptance（2026-11-16）**
>
> 全仓 `cargo fmt --check` / `clippy --all-targets --all-features -D warnings -W pedantic` /
> `cargo test --all-features`（**40 个测试目标**）/ `cargo doc --no-deps` 全绿；
> FV-1/2/3 均 `stdout: 42`。
>
> 当前测试规模：arm **512**、periph **435**、cli **127(+1 ignored)/86**、core **104**、
> riscv **85**、gdb **91**、egui **100**、virt-periph **51**、audio **13**、usb **53**、
> usb-host **33(+2)**、bootrom 21。
>
> 时钟与帧率（诚实口径）：固件把 `clk_sys` 配到 **252 MHz**（启动 150 MHz），
> 1 秒客户机时间 = 252 M 周期，60 fps 一帧 = **4.2 M 周期**；真实游戏段 8.5 M 步/s →
> **≈2 fps**。**不做 JIT**，优先精确时钟/精确指令/功能与插件稳定；性能数据按
> 300–500 M 步窗口报告（见 `doc/performance.md` §9/§9.1）。
>
> 并发上限 2（crate 不相交）；每波完成后主会话复跑四道门禁 + FV-1/2/3 + emerald
> 300M 冒烟，再派下一波。Concurrency is capped at 2 disjoint crates per wave; the main
> session re-runs the four gates plus FV-1/2/3 and the 300M-step emerald smoke after
> every wave before dispatching the next one.

---

## 固件验证清单 / Firmware Verification Checklist（内核门禁 / core gate）

> **门禁规则 / Gate rule**: 对应内核必须先跑通下列固件，才允许进入下一任务。
> **Gate rule**: the core owning each firmware must pass its gate command before
> the next task may start. 参考实现 / reference implementation: `E:\Source\mcu\pico\rp2350js`。
> 操作手册 / Playbook: `.opencode/skills/firmware-verification/SKILL.md`
> （验收命令、诊断步骤、子任务拆分模板与已知缺口 / gate commands, diagnostics, task template, known gaps）。

| ID | 固件 / Firmware | 架构 / Core | 验证命令 / Gate command | 状态 / Status |
|----|-----------------|-------------|--------------------------|---------------|
| FV-1 | `E:\Source\mcu\pico\python\micropython\RPI_PICO2-RISCV-20260824-v1.29.0.uf2`（MicroPython RISC-V） | Hazard3（`rp2350-riscv`） | `rp2350 repl --file <该文件> --exec "print(6 * 7)"` 输出 `42`；`run --strict-decode` 全程无未定义指令停机 | ✅ 通过 / passed（2026-11-14）— `[repl] USB CDC configured: IN=0x82/64 OUT=0x02/64 interface=1`、`[repl] banner`、`stdout: 42`（2,659,500 步 / 17,730 µs）；`run --arch riscv --strict-decode --max-cycles 50000000` 无未定义指令（终态 `PC=0x10046E64`）。根因修复：Zcmp/`C.LUI`/Zb\* 语义、Xh3irq 抢占与 NOIRQ、CSR 数组窗口索引 |
| FV-2 | `E:\Source\mcu\pico\python\micropython\RPI_PICO2-20260824-v1.29.0.uf2`（MicroPython Arm） | Cortex-M33（`rp2350-arm`） | `rp2350 repl --file <该文件> --exec "print(6 * 7)"` 输出 `42`；`perfbench` 至少一个基准出结果（回归项，随每次内核改动复跑） | ✅ 通过 / passed（2026-11-14）— `stdout: 42`；`bm_nqueens --n 40 --m 10` → `measured: ticks_us=19266 norm=100 result=10` |
| FV-3 | `E:\Source\mcu\pico\python\circuitpython\adafruit-circuitpython-raspberry_pi_pico2-zh_Latn_pinyin-10.3.1.uf2`（CircuitPython Arm） | Cortex-M33（`rp2350-arm`） | `rp2350 repl --file <该文件> --exec "print(6 * 7)"` 输出 `42`（配置成功 + banner） | ✅ 通过 / passed（2026-11-14）— 根因修复：16 位 `SXTB/SXTH/UXTB/UXTH` 不再更新 APSR（此前误置 Z 导致 TinyUSB 驱动循环提前退出并对 EP0 布防 STALL）。实测 `[repl] USB CDC configured: IN=0x82/64 OUT=0x02/64 interface=1`、`[repl] banner`、`stdout: 42` |

> 状态标记 / status markers: ✅ 通过 · 🚧 部分通过 · ⬜ 未通过。通过一条门禁后，
> 将其状态、日期与验证命令输出摘要记录到 [changelog.md](changelog.md)。
> After a gate passes, record its status, date and command output summary in
> [changelog.md](changelog.md).

### 门禁备注 / Gate notes

- **FV-2（通过 / passed）**：回归修复了 5 个真实内核缺陷 —— Thumb-16/32 `LDM`
  基址在加载列表时不写回、USB OUT `BUF_CTRL.LEN` 覆盖旧值、PSM 与 core1
  bootrom FIFO 握手、异常进入/返回保存 `ITSTATE`、Thumb-16 `ADDS/SUBS`
  完整 NZCV。命令输出见 changelog。
- **FV-3（通过 / passed，2026-11-14）**：`run --strict-decode --max-cycles 100000000`
  已完成 200 ms 首次启动延时、BOOTRAM boot2 拷贝并进入主循环，无未定义指令；
  时钟推进 100,000,000 步 / 666,666 µs。`repl` 的 `SET_CONFIGURATION` 状态阶段
  STALL 根因是 ARM 16 位 `SXTB/SXTH/UXTB/UXTH` 错误更新 APSR：CircuitPython
  的 `process_set_config` 依赖 `UXTB` 不改变 Z 标志，Z 被误置后 `bls` 提前跳出，
  TinyUSB 随即 `dcd_edpt_stall(EP0_OUT/IN)`。修复后实测
  `[repl] USB CDC configured: IN=0x82/64 OUT=0x02/64 interface=1`、
  `[repl] banner`、`stdout: 42`。同时修正 `rp2350-usb` 的 `EP_ABORT`
  位解码（`bit = 2*ep + dir`）与 SETUP 时清除 `EP_STALL_ARM`。
- **FV-1（通过 / passed，2026-11-14）**：启动路径与 USB REPL 全部打通。修复：
  boot2 `0x7DF8` 数据查找槽、Zb\*/Zcb/C.LUI/Zcmp 语义、Xh3irq 优先级/抢占/
  NOIRQ/`meinext.update` 认领、CSR 数组窗口改用原始操作数选窗、同步异常只走
  `mtvec` 基址、`repl --arch` 覆盖。实测 `[repl] USB CDC configured`、
  `[repl] banner`、`stdout: 42`。遗留改进项（非阻塞）：`repl` 失败路径诊断、
  `misa` B 位、其余 RISC-V ROM 表项。

---

## Phase 1 · Foundation (Week 1) ✅ COMPLETE

### Deliverables / 交付物

| ID | Task | Crate | Status |
|----|------|-------|--------|
| 1.1 | Workspace scaffold + Cargo.toml with dependencies | root | ✅ |
| 1.2 | `MemoryRegion` trait + `AddressBus` trait + `BusErrorKind` + route logic | rp2350-core | ✅ |
| 1.3 | `RP2350Chip` struct skeleton (gpio, irq, clock, cores) | rp2350-core | ✅ |
| 1.4 | `GpioPin` + `PinFunction` + `PinDirection` + `PortDirection` (48 pins) | rp2350-core | ✅ |
| 1.5 | `SimulationClock` + clock divider logic | rp2350-core | ✅ |
| 1.6 | `IrqMatrix` (256 IRQs, pending/enabled/priority routing) | rp2350-core | ✅ |
| 1.7 | `CpuCore` trait + `CpuState` + `CpuArch` enums | rp2350-cpu | ✅ |
| 1.8 | `Peripheral` trait (base for APB peripherals) | rp2350-periph | ✅ |
| 1.9 | `RpPlugin` trait + `PluginContext` trait + `PluginMetadata` builder | rp2350-plugin | ✅ |
| 1.10 | `DevicePlugin` trait + `DevicePort` + `PortDirection` + `ConfigSchema` | rp2350-virt-periph | ✅ |
| 1.11 | `SafeLibrary` RAII wrapper (libloading) | rp2350-ffi | ✅ |
| 1.12 | `PluginLoader` + `WiringTable` + `DeviceManager` (full impl + tests) | rp2350-virt-periph | ✅ |
| 1.13 | `eframe` App skeleton with menu bar + status bar (stub) | rp2350-egui | ✅ |
| 1.14 | `clap` CLI: `run`, `gdb`, `wire`, `list-devices`, `mcp-serve` subcommands | rp2350-cli | ✅ |
| 1.15 | `GdbServer` skeleton + `McpServer` skeleton | rp2350-gdb, rp2350-mcp | ✅ |
| 1.16 | Git init + `.gitignore` + `README.md` + GitHub Actions CI/CD | root | ✅ |

### Milestone M1 / 里程碑 M1 ✅

- ✅ `cargo build` 零错误 (11 crates + workspace)
- ✅ `AddressBus` 路由抽象完整（`find_region_mut` + read/write auto-route）
- ✅ `DeviceManager` 完整实现（注册/接线/tick/mcp_query_all）+ 单元测试
- ✅ `WiringTable` 双向 (device, port) ↔ GPIO pin 映射
- ✅ `PluginLoader` 目录扫描 + SafeLibrary 加载
- ✅ `SafeLibrary` RAII 包装 + Send/Sync
- ✅ egui 空窗口桩代码就绪（Phase 3 实现）
- ✅ CLI 子命令桩全部就绪
- ✅ GitHub Actions CI (三平台) + Release (多目标 + WASM + Pages) 工作流

---

## Phase 2 · ARM Cortex-M33 Basic (Week 2-3) ✅ COMPLETE

### Deliverables / 交付物

| ID | Task | Output | Status |
|----|------|--------|--------|
| 2.1 | `CortexM33Registers`: R0-R15, MSP/PSP, XPSR, CONTROL, PRIMASK, BASEPRI, FAULTMASK | registers.rs | ✅ |
| 2.2 | Thumb-16 decoder + executor (~90 instructions) | execute_thumb16.rs | ✅ |
| 2.3 | Thumb-32 decoder（~60 ALU + Load/Store；另含 FPv5-SP FPU 与 GPIO(CP0)/DCP(CP4) 协处理器；DSP/SIMD 已补 SMUL/SMLA/SMMUL、并行加减、SXTB16/UXTB16 与 USAD8/USADA8） | execute_thumb32.rs | ✅ |
| 2.4 | Condition flags (NZCVQ) + IT block execution | conditions.rs | ✅ |
| 2.5 | NVIC: ISER/ICER/ISPR/ICPR/IPR/STIR, priority grouping | nvic.rs | ✅ |
| 2.6 | Exception entry/exit (stacking/unstacking, EXC_RETURN) | exceptions.rs | ✅ |
| 2.7 | SysTick timer + PPB base registers (SCB, VTOR) | systick.rs | ✅ |
| 2.8 | `CortexM33Core` impl of `CpuCore` trait | core.rs (lib.rs) | ✅ |
| 2.9 | BOOTROM load + reset vector fetch + SP/PC init | chip boot sequence | ✅（合成 bootrom + `CortexM33Core::boot`） |
| 2.10 | egui register viewer panel (R0-R15 table) | panels.rs | ✅（可无头渲染测试） |
| 2.11 | `GdbTarget` trait + Cortex-M33 adapter | gdb target.rs / cortex_m33.rs | ✅（P1：RSP 协议层 + TCP 会话循环已落地；CLI 接线规划中 / RSP protocol + TCP session loop landed; CLI wiring planned） |

### Quality Gate (Current) / 质量门禁（当前）

| Gate | Status |
|------|--------|
| `cargo fmt --all -- --check` | ✅ |
| `cargo clippy --all-targets --all-features -- -D warnings -W clippy::pedantic` | ✅ zero errors/warnings |
| `cargo test --all-features` | ✅ 465/465 pass |
| `cargo doc --no-deps` | ✅ zero warnings |

### Milestone M2 / 里程碑 M2 ✅

- ✅ BOOTROM 能够执行到固件入口（合成 bootrom + Stage-4 总线）
- ✅ 基础 Thumb-16 指令集可运行 blink 固件
- ✅ egui 寄存器查看器（`panels.rs`，含无头渲染测试）

---

## Phase 2.5 · Multicore (core1) Strategy / 多核（core1）策略

**Decision (adopted option a): single-core execution with a resident core1.**
**决策（采纳方案 a）：单核执行 + core1 常驻。**

当前阶段不实现第二核心的真实指令调度。core1 被建模为“常驻 WFE”：它不取指、
不执行，但 SIO 的 FIFO 与门铃（doorbell）语义完整保留，使 core0 的
launch / FIFO 握手能够正常完成，而不会因为“对端永不消费 FIFO”而永久阻塞。

At this stage the second core is not actually scheduled. core1 is modelled as a
*resident WFE*: it fetches and executes nothing, but the SIO FIFO and doorbell
semantics are preserved so core0's launch/FIFO handshakes can complete instead
of blocking forever on a core1 that never consumes the FIFO.

设计要点 / Design points:

- **SIO FIFO 语义保留**：`FIFO_WR` / `FIFO_RD` / `FIFO_ST` 按真实硬件行为建模
  （写满、读空、VLD/RDY 位），core1 视为持续的 FIFO 消费者。
  FIFO register behaviour (full/empty, VLD/RDY) is modelled; core1 acts as a
  steady FIFO consumer.
- **门铃保留**：`DOORBELL_OUT_SET` / `DOORBELL_IN_CLR` 等寄存器可读写，保留
  跨核中断请求语义。
  Doorbell registers stay readable/writable to preserve cross-core IRQ request
  semantics.
- **阻塞检测告警**：若 core0 在 `FIFO_RD` 上连续空转超过阈值（与现有自旋告警
  同一套窗口机制），打印一次告警，附带 PC、FIFO 状态与累计步数，便于定位
  “等待 core1 消费但 core1 未建模”的握手死锁。
  If core0 spins on `FIFO_RD` past a threshold (reusing the existing tight-loop
  spin detector), emit a single warning with PC, FIFO state and step count, so
  a core1-consumption deadlock is easy to locate.
- **后续升级路径**：若固件确实依赖 core1 并行推进（而非仅握手），再引入真正的
  双核调度（社区方案 b），届时复用同一 SIO 模型。
  If firmware later depends on true core1 parallelism (not just handshakes),
  upgrade to real dual-core scheduling (option b) and reuse the same SIO model.

> **2026-11-14 更新 / Update**: 真双核调度已排入 P1 开发计划（用户决策），升级路径维持本节设计。
> Real dual-core scheduling is scheduled into the P1 development plan; the upgrade
> path in this section stays as designed.

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

> **P1 进度 / P1 progress（2026-11-14）**：MCP 方法层与最小 HTTP 传输均已落地 ——
> `McpServer` 的 `initialize`/`tools/list`/`tools/call`（`list_devices`/`query_all`/
> `query_device`/`configure`）委托 `DeviceManager`；`POST /mcp`（别名 `/`）与
> `GET /health` 提供 HTTP 传输，CLI `mcp-serve` 已接线（默认 8080）。剩余：TLS/鉴权、
> 插件热加载。
> The MCP method layer and a minimal HTTP transport have landed: McpServer implements
> initialize/tools/list/tools/call (list_devices, query_all, query_device, configure) on
> top of DeviceManager, with `POST /mcp` (alias `/`) and `GET /health` served over HTTP;
> the CLI `mcp-serve` subcommand is wired (default 8080). TLS/authentication and plugin
> hot reload remain.

> **P1 进度 / P1 progress（2026-11-14）**：插件系统端到端已打通 ——
> `DeviceManager::scan_and_load(dir)` 自动实例化目录内设备插件，CLI `list-devices`
> 与 `wire` 已接入真实设备管理器（一次性进程语义，已写入文档）；`mcp_list_devices`
> 补充端口信息；第二方插件 `uart-terminal` 已落地（TX 位采样解码、RX 位时序输出、
> baud/data_bits/stop_bits 配置、状态快照，11 条测试）。剩余：GUI 调色板/连线、
> 接线持久化、热替换。
> The plugin system is now wired end to end: DeviceManager::scan_and_load(dir)
> instantiates device plugins from a directory, and the CLI list-devices / wire
> subcommands drive a real manager (one-shot process semantics documented);
> mcp_list_devices now includes port information, and the second-party `uart-terminal`
> plugin has landed (TX bit sampling, RX bit-timed output, baud/data_bits/stop_bits
> configuration, state snapshot, 11 tests). GUI palette/wiring, wiring persistence and
> hot replacement remain.

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

> **P1 进度 / P1 progress（2026-11-14）**：4.1（指令集）与 4.2（FIFO/autopush/autopull）
> 已落地基础执行；4.3 部分（`CLKDIV` 固定 1:1，delay/side-set 待补）；4.4–4.9 待办。
> `Pio::tick` 尚未接入仿真主循环，当前由 `step_sm`/`tick` 单元测试驱动。
> 4.1 and 4.2 have a working base (instruction execution plus FIFO/autopush/autopull);
> 4.3 is partial (`CLKDIV` fixed at 1:1, delay/side-set pending); 4.4-4.9 remain.
> `Pio::tick` is not yet wired into the simulation loop; it is exercised through unit tests.

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

> **P1 进度 / P1 progress（2026-11-14）**：8.1 的 RSP 协议层、TCP 会话循环与 CLI
> `gdb` 接线（`MachineTarget`，端到端验证）已落地（`rsp.rs`/`server.rs`/`gdb_target.rs`，
> 37 + 21 条测试）；8.2–8.9 待办。
> 8.1's RSP protocol layer, TCP session loop and the CLI `gdb` wiring (`MachineTarget`,
> end-to-end verified) have landed (`rsp.rs`/`server.rs`/`gdb_target.rs`, 37 + 21 tests);
> 8.2-8.9 remain.

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

## Phase 10 · RISC-V (🔄 部分提前完成 / partially landed ahead of schedule)

> `rp2350-riscv` crate 已实现并被 CLI `run --arch riscv` 使用（原计划远期，2026-11 前提前落地）。

| ID | Task | Status |
|----|------|--------|
| 10.1 | RISC-V Hazard3: RV32IMAC + C 压缩指令（Zcb/Zba/Zbb/Zbs 未实现） | ✅ 基础 / ⏳ Z 扩展 |
| 10.2 | RISC-V CSR registers + Machine mode（trap/`mret`） | ✅ |
| 10.3 | Xh3irq (platform interrupt controller) | ✅ |
| 10.4 | RISC-V GDB Target | ⬜ |
| 10.5 | Heterogeneous dual-core (ARM + RISC-V) | ⬜ |
| 10.6 | RISC-V MicroPython boot test | ⬜ |

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