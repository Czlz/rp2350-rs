# RP2350-rs 项目规则 / Project rules

## 项目概况

| 项目 | 值 |
|------|-----|
| 名称 | RP2350-rs |
| 定位 | 精确周期 ARM Cortex-M33 / RISC-V 双核模拟器 |
| 语言 | Rust Edition 2024（`resolver = "3"`） |
| 结构 | Cargo Workspace，16 个 crate + `plugins/{led,uart-terminal}` |
| 仓库 | git@github.com:Czlz/rp2350-rs.git |
| 文档目录 | `doc/`（与代码同步维护） |

## AI 开发模式（强制）：主会话只拆分/派发/验收，禁止复用子会话

禁止把长期开发塞进单一子会话（上下文会线性膨胀且验收困难）。所有开发按以下循环执行：

1. **拆分**：主会话把目标拆成单子系统任务（每任务只涉及一个 crate 或一对紧密耦合 crate），
   每个任务必须附：目标、现状证据（命令输出/PC/错误报文）、允许修改的文件白名单、
   验收命令与预期输出、停止条件。
2. **派发**：每任务开启一个**全新**子会话（自包含提示词，不依赖历史会话）；子会话不得执行任何 git 操作。
3. **验收**：主会话独立复跑验收命令、抽查 diff、运行四道质量门禁（见下节）。
4. **收口**：通过 → 更新 `doc/implementation-plan.md` 清单/`doc/changelog.md` 并记录；
   不通过 → 用失败证据开**新**短会话重派，禁止续用旧长会话。
5. **并行**：仅允许 crate 不相交的任务并行，并发 ≤ 2。

#### 定位：精确时钟与精确指令优先（强制）/ Precision first

本项目的优先级是 **周期/时钟精确、指令精确、功能与插件稳定**，其次才是宿主吞吐。

- **不做 JIT**：不以宿主代码生成/块编译换取速度。性能只能在「宿主侧重复工作」上优化
  （缓存、去虚化、批量记账、事件门控），客户机的每条指令、每个周期都必须照常执行。
- **时钟是第一类语义**：`XOSC`/`ROSC`/`PLL_SYS`/`PLL_USB`、`CLK_REF`/`CLK_SYS`/`CLK_PERI`/
  `CLK_USB`/`CLK_ADC`/`CLK_RTC` 的源选择与分频必须建模，并让依赖它的计时（`SysTick`、
  `TIMER`、PIO `CLKDIV`、HSTX 像素时钟、UART 波特率、ADC/RTC、DMA 节拍、`cycles→µs`）
  跟随运行时变化。**支持运行时超频/降频**（固件改写 PLL/分频后立即生效），不得写死 150 MHz。
- 频率/时序改动必须提供「已知配置向量」测试（例如 12 MHz XOSC → 150 MHz、252 MHz 超频），
  以及至少一个真实固件或合成固件的端到端验证。
- 性能数据必须按**工作负载区间**报告（例如 emerald 300–500 M 步窗口），不得用启动/睡眠
  阶段的吞吐代表整体。

Precision first: cycle- and instruction-accurate behaviour, stable features and plugins come
before host throughput. **No JIT** — every guest instruction and cycle is still executed; only
host-side duplicated work may be optimised. Clocks are first-class: XOSC/ROSC/PLL_SYS/PLL_USB
and the CLK_REF/CLK_SYS/CLK_PERI/CLK_USB/CLK_ADC/CLK_RTC sources and dividers must be modelled,
runtime overclocking must take effect immediately, and every timing consumer (SysTick, TIMER,
PIO CLKDIV, HSTX pixel clock, UART baud, ADC/RTC, DMA pacing, cycles-to-microseconds) must
follow the live frequency instead of a hardcoded 150 MHz. Frequency work needs known-vector tests
(12 MHz XOSC to 150 MHz, 252 MHz overclock) plus an end-to-end firmware check, and performance
numbers must be reported per workload window rather than from startup or sleep phases.

## 通用性与指令精确（强制）/ Generality and instruction accuracy

本项目是**通用 RP2350 模拟器**，不是某个示例固件的定制运行时。MicroPython、CircuitPython、
pokeemerald 等固件只是**验收用的真实客户机**，不是优化目标，也不是语义裁剪的依据。

- **禁止针对特定固件取巧**：不得按固件哈希/文件名/版本、固定地址、固定 PC、循环形态、
  固定步数做特判；不得跳过指令、跳过周期、伪造结果或让输出「看起来对」。
- **禁止以「该固件用不到」为由省略语义**：`WFI`/`WFE`/`SEV` 睡眠、阻塞式 FIFO/总线 stall、
  未实现外设的复位语义等，只要数据手册/ARM ARM 定义了，就必须建模并按优先级排期。
  示例固件的忙等实现只能说明当前瓶颈在哪，不能说明某项语义可以不做。
- **性能优化必须保语义**：任何提速都不能改变客户机可见行为（寄存器/内存/周期/中断/时序）。
  可接受的是消除宿主侧重复工作（缓存、去虚化、批量记账），不可接受的是减少客户机工作。
- **示例常量不得进入模拟器**：帧缓冲地址与格式、按键引脚、I2S 的 PIO 块/状态机/采样率、
  固件路径等，只能出现在 `example/`、CLI 参数、GUI 用户输入或 profile 文件里；
  crate 的默认值必须是通用的（未提供时要求用户显式给出，或留空并提示）。
- 新增/修改功能时，若只能在某个固件上验证，必须在报告里说明「通用性风险」与
  「其它固件如何验证」。

Generality is mandatory: this is a general RP2350 emulator, not a runtime for one example.
Firmware such as MicroPython, CircuitPython or pokeemerald are real guest programs used for
acceptance, never a reason to skip semantics or to special-case a hash, address, PC or loop
shape. Missing semantics defined by the datasheet or the ARM ARM (WFI/WFE/SEV, blocking
FIFO reads and bus stalls, reset values of unmodelled peripherals) stay on the roadmap no
matter whether the current example exercises them. Optimisations may only remove host-side
duplicated work, never guest-visible work. Example-specific constants (frame buffer address,
button pins, I2S PIO block/state machine/rate, firmware paths) belong to `example/`, CLI
options, GUI input or profile files, never to crate defaults.

## 角色边界（强制）

- **主会话只做三件事**：① 缺陷/需求分析与任务拆解；② 派发自包含的子会话任务；
  ③ 独立验收（复跑门禁/固件/FV/示例冒烟）、提交与文档收口。
- 主会话**不直接实现功能代码**；只有以下情况可由主会话直接改：纯文档/规则/示例说明、
  `.gitignore`/配置、以及 ≤10 行的文字或常量修正。
- 主会话发现新缺陷时**新增任务**，不得自行顺手修改（避免绕过白名单与验收）。

### 子会话使用规则（强制，防止上下文超长）

- **一个任务 = 一个全新子会话**。禁止把已完成/已验收的子会话用于后续任务。
- 禁止用 `sessionID` 复活旧子会话做「小修」；返工必须开**新**会话，并把失败证据
  （命令输出、断言、PC/错误报文）写进新任务的提示词。
- 单个子会话只允许：读白名单文件、实施改动、跑自己任务的测试/门禁、写报告；不得再拆分它自己的
  子任务（避免嵌套与上下文翻倍）。
- 提示词必须自包含：目标、已确认证据、文件白名单、验收命令、约束（禁用 PowerShell 整文件读写
  含中文的源文件、禁止 git 写操作）、报告模板。**不得**依赖「上一轮对话」或历史上下文。
- 子会话一次性产出：实现 + 测试 + 文档 + 门禁结果；主会话只做复跑与提交。

**任务硬上限**：净改动 ≤ ~300 行、文件 ≤ 3 个、无需全仓探索；超出即收口并拆出新任务。
固件验证门禁与诊断手册见 [`doc/firmware-verification.md`](doc/firmware-verification.md)。

## Workspace 成员

```
crates/
├── rp2350-core/        # 芯片基类: AddressBus, MemoryRegion, RP2350Chip, GpioPin, IrqMatrix, SimulationClock
├── rp2350-cpu/         # CpuCore trait (dyn-safe), CpuState, CpuArch
├── rp2350-arm/         # ARM Cortex-M33 核心实现
├── rp2350-riscv/       # Hazard3 RISC-V 核心实现（RV32IMAC+Zcb）
├── rp2350-periph/      # BootPeripherals: GPIO/UART0/Timer/PSM/BOOTRAM 等
├── rp2350-bootrom/     # 合成 Boot ROM（Arm/RISC-V 陷阱桩）
├── rp2350-usb/         # USB 控制器设备模型（DPRAM/寄存器）
├── rp2350-usb-host/    # USB 主机桥 + CDC + MicroPython raw REPL + perfbench
├── rp2350-audio/       # 宿主音频后端（cpal 可选 / null 降级）
├── rp2350-plugin/      # RpPlugin trait, PluginContext, PluginMetadata
├── rp2350-virt-periph/ # DevicePlugin, DeviceManager, WiringTable, 线程化执行器/热重载
├── rp2350-ffi/         # SafeLibrary RAII (libloading)
├── rp2350-gdb/         # GDB RSP server（断点/监视点/双核 vCont）
├── rp2350-egui/        # egui GUI（PlayView + 虚拟实验室 + 外设面板）
├── rp2350-cli/         # clap CLI（run/repl/play/perfbench/gdb/devices/mcp-serve）
└── rp2350-mcp/         # MCP server（JSON-RPC + HTTP + Bearer）
```

## 质量门禁（每次提交前必过，零容忍）

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings -W clippy::pedantic
cargo test --all-features
cargo doc --no-deps
```

要求：clippy 零错误零警告，测试全部通过，doc 零警告。

## 固件验证门禁（内核开发准入条件）

对应内核必须先跑通下列固件，才允许进入下一开发任务：

| ID | 固件 | 内核 | 通过标准 |
|----|------|------|----------|
| FV-1 | RISC-V MicroPython UF2 | Hazard3 | `repl --exec "print(6 * 7)"` 输出 `42` |
| FV-2 | Arm MicroPython UF2 | Cortex-M33 | 同上（回归门禁，持续复跑） |
| FV-3 | Arm CircuitPython UF2 | Cortex-M33 | 同上 |

- 固件路径、验收命令、当前状态与诊断步骤见 skill `firmware-verification`；
  清单原文见 `doc/implementation-plan.md` 顶部「固件验证清单」。
- 状态标记只允许 ✅（实测通过并附命令输出）/ 🚧 / ⬜；**禁止把「run 未崩溃」当成通过**。

## 代码规范（强制）

- **单 .rs 文件 ≤ 500 行代码**（注释不计）；超限即拆 `types.rs` / `traits.rs` / `impl_*.rs` / `error.rs`。
- **所有 `pub` 项写中英双语文档注释**；复杂逻辑双行内注释。
- **`unsafe` 块必须附双语 `// SAFETY:` 说明**。
- **文件头**：库文件加 `#![deny(unsafe_op_in_unsafe_fn)]` + `#![warn(missing_docs)]`。
- **错误处理**：库用 `thiserror`，应用（CLI）用 `anyhow`。
- **编码前先判断是否需要 trait 抽象**（多实现、外部依赖、需 mock）；需要则先写 trait 再写 impl。
- **每个 `pub fn` ≥ 1 测试**，错误分支必须覆盖；单元测试同文件 `#[cfg(test)]`，集成测试放 `tests/`，文档测试 Examples 必须可跑。

## Workspace 依赖管理

- 统一在 `[workspace.dependencies]` + `[workspace.package]` 管理。
- 子 crate 用 `{ workspace = true }` 引用。
- 本地 crate 用 `path = "crates/<name>"` 声明，如 `rp2350-core = { path = "crates/rp2350-core" }`。
- 依赖默认 [crates.io](https://crates.io/) 最新稳定版。
- 新模块先 `cargo new --lib crates/<name>` 分包，禁止堆砌逻辑。

## 关键架构决策

- **Dyn-safe traits**：`CpuCore` 使用 `&mut dyn AddressBus`（非泛型 `B`），支持 trait object。
- **`PluginContext` 拆分**：`PluginContext`(trait) + `PluginContextExt`(extension) 解决 dyn 兼容。
- **`DevicePlugin` 继承 `RpPlugin`**，不重复定义 `state_snapshot()`。
- **异常处理解耦**：`exceptions.rs` 提供自由函数（`enter_exception` / `exit_exception` / `decode_exc_return`），由 `CortexM33Core` 调用，而非核心结构体方法。

## Clippy allow 策略

硬件仿真代码（指令解码器、MMIO 寄存器、异常处理）会产生固有 Clippy 警告，**统一在文件顶部模块级 `#![allow(...)]` 抑制**，附双语理由注释，不破坏可读性。典型 pattern：

```rust
// 指令解码器涉及大量位操作、幻数和类型转换，
// 这些是 ARM 指令集仿真的固有特性。
// Instruction decoder involves extensive bit operations, magic numbers,
// and type casts inherent to ARM ISA emulation.
#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::missing_errors_doc,
    clippy::doc_markdown,
    clippy::unusual_byte_groupings,
    clippy::unreadable_literal
)]
```

`#[allow]` 必附双语理由。

## 动态库（libloading）

- 调 `.so`/`.dll`/`.dylib` 必须用 `libloading`，封装为独立 FFI crate（`rp2350-ffi`）。
- RAII 管理 `Library`，禁止 `#[link]`。
- 发布默认静态编译；`--release` + `lto = "fat"` + `codegen-units = 1`。

## 文档维护（doc/ 目录）

- 代码变更时同步更新 `doc/`，禁止代码与文档脱节。
- 必含文件：
  - `doc/README.md` — 项目总览、crate 结构、快速开始（中英双语）
  - `doc/architecture.md` — 模块划分、trait 抽象、数据流
  - `doc/dependencies.md` — 第三方依赖清单、版本、用途、动态库加载说明
  - `doc/changelog.md` — 版本变更记录（Keep a Changelog 格式）
  - `doc/v1-scope.md` — V1 发布范围、精度声明与已知限制（发布口径）
  - `doc/implementation-plan.md` — 实施计划（含各 Phase 交付物与状态）
  - `doc/crates/<name>.md` — 每个 crate 的公共 API 与使用示例
- 涉及 `libloading` 的 crate 必须额外写 `doc/ffi/<name>.md`。
- 主 `doc/` **只放跨示例的通用文档**；示例项目专用内容见下节。

## 示例项目目录（example/，强制）

示例项目（用本模拟器跑某个真实固件/程序）的一切**专用**材料放在
`example/<name>/`，不得混入主 `doc/`：

```
example/<name>/
├── README.md        # 目标固件/程序、运行命令、期望输出、引脚配置、外设/插件依赖
├── doc/             # 该示例的验证报告、性能数据、截图（如 gba-bringup-report.md）
└── tools/           # 示例专用脚本（如帧缓冲转 PNG）
```

规则：

- **主程序与外设配置**（启动流程、DMA/PIO/HSTX 使用方式）、**插件外设依赖配置**、
  **引脚配置表**写在 `example/<name>/README.md`，不写进主 `doc/`。
- **验证报告/截图**放 `example/<name>/doc/`；主 `doc/` 只允许用相对路径
  （`example/<name>/...`）链接，禁止复制内容。
- 新增示例必须先建 `example/<name>/README.md`（含引脚与外设依赖表），再写代码/报告。
- 示例引用的固件不在本仓库时，README 必须写明绝对路径、大小与 SHA-256。

Example projects keep **all** their specific material under `example/<name>/`:
`README.md` (target firmware, run commands, expected output, pin map, peripheral
and plugin dependencies), `doc/` (verification reports, screenshots) and
`tools/`. The main `doc/` tree holds cross-example design documents only and may
link into `example/` but never copy its content.

## Git 规范

- 默认初始化 Git，代码变动时详细记录提交变动内容。
- **禁止主动提交**：除非用户明确要求，不执行 `git commit` / `git push`。
- `.gitignore` 已忽略：`/target/`、IDE 目录、OS 工件、WASM 输出、插件二进制、`HANDOFF.md`、
  `.opencode/`、`.trae/`（本地 agent/IDE 配置，不随仓库发布）。

## 当前进度

以 `doc/implementation-plan.md`（含固件验证清单）为唯一进度来源；本文件不重复维护进度快照。

## 常见命令

```bash
cargo build                          # 构建
cargo test --all-features            # 全量测试
cargo test -p rp2350-arm             # 单 crate 测试
cargo clippy --all-targets --all-features -- -D warnings -W clippy::pedantic  # 严格 Clippy
cargo doc --no-deps --open           # 文档
cargo fmt --all                      # 格式化
cargo new --lib crates/<name>        # 新增 crate
```