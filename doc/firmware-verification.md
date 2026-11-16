# 固件验证 / Firmware Verification

本文件是固件门禁的**唯一操作手册**；进度与状态以
[`implementation-plan.md`](implementation-plan.md) 顶部「固件验证清单」为准。

## 何时使用

- 任何涉及固件启动、USB 枚举、raw REPL、`run`/`repl`/`perfbench` 的任务。
- 内核（`rp2350-arm` / `rp2350-riscv`）指令或启动路径改动。
- 需要判定 FV-1 / FV-2 / FV-3 门禁是否通过时。

## 固件清单与门禁

| ID | 固件（绝对路径） | 内核 | 通过标准 |
|----|------------------|------|----------|
| FV-1 | `E:\Source\mcu\pico\python\micropython\RPI_PICO2-RISCV-20260824-v1.29.0.uf2` | Hazard3（`rp2350-riscv`） | `repl --exec "print(6 * 7)"` 输出 `42` |
| FV-2 | `E:\Source\mcu\pico\python\micropython\RPI_PICO2-20260824-v1.29.0.uf2` | Cortex-M33（`rp2350-arm`） | 同上（回归门禁） |
| FV-3 | `E:\Source\mcu\pico\python\circuitpython\adafruit-circuitpython-raspberry_pi_pico2-zh_Latn_pinyin-10.3.1.uf2` | Cortex-M33（`rp2350-arm`） | 同上 |

命令模板（在仓库根目录执行）：

```powershell
cargo run --release -p rp2350-cli -- repl --file "<UF2>" --exec "print(6 * 7)"
```

- FV-1 需要 `--arch riscv`（`repl` 时由 UF2 family 自动判定；`run` 显式指定）。
- CircuitPython 建议加 `--no-soft-reset`（软复位握手未完全建模时）。
- **判定标准**：stdout 出现 `42` 且出现 `[repl] banner`。
  **仅 `run` 耗尽预算 / 未崩溃不算通过。**

## 当前状态（2026-11-16）

- **发布口径**：[`v1-scope.md`](v1-scope.md)（V1 包含/不包含、已知近似、验证矩阵）。
- **发布进度**：[`implementation-plan.md`](implementation-plan.md)（R1–R6、FV 清单）。
- **经验教训**：[`lessons.md`](lessons.md)。
- 40 个测试目标、2011 个测试全绿：arm 542、periph 483、cli 191+2 ignored/89、core 122、
  egui 141、gdb 91、riscv 85、virt-periph 74、audio 20、usb 53、usb-host 33+2、bootrom 21。
- FV-1/2/3 均 `stdout: 42`；emerald 300 M 基线：`time_us=1798617`、`dma=2199378`、
  `hstx words=5401204 underruns=0`、`sleep core0 steps=224994790 wakes=6`、`core1 Running`、
  8 条 `[irq]`、PC `0x1000B3AC`。
- 时钟：固件 `clk_sys=252 MHz`（启动 150 MHz），1 s 客户机时间 = 252 M 周期，60 fps 一帧 =
  4.2 M 周期；默认确定性模式真实游戏段 ≈2 fps，`--core-threads on` 约 3.5–6.5 fps（非位可复现）。
- 多线程状态：双核/插件执行器已实现；V1 剩余工作见 `doc/implementation-plan.md` §2（R2/R4/R6）。
- 已知近似：CPU `steps == cycles`（1 指令 = 1 周期）；PIO `CTRL/CLKDIV` 写入在下一个 64 步泵送
  边界生效；PPB 读回看不到核内故障位。

## 参考实现

`E:\Source\mcu\pico\rp2350js`（JS 版 RP2350 模拟器）。

- 忽略 `node_modules/`、`dist/`、`build/`；重点读 `src/`、`doc/`、`README`。
- 其根目录有 `bootrom_tmp.bin`：仅作**行为参考**，不要把来源不明的大体积 ROM 镜像提交进仓库。
- 优先查找：RISC-V 启动/bootrom 处理、USB 枚举与控制传输、时钟/PLL、SIO/GPIO、timer/tick、watchdog、ADC。
- 若仓库内有引用上述 UF2 的测试/脚本，先读它们。

## 主会话验收流程（强制）

1. 独立复跑该任务的验收命令，保存关键输出（命令 + 末段输出）。
2. 抽查 diff：只允许白名单文件；检查语义是否符合数据手册/ARM ARM。
3. 运行四道门禁：
   ```powershell
   cargo fmt --all -- --check
   $env:CARGO_INCREMENTAL=0; cargo clippy --all-targets --all-features -- -D warnings -W clippy::pedantic
   cargo test --all-features
   cargo doc --no-deps
   ```
4. 通过后更新 `doc/implementation-plan.md` 清单状态与 `doc/changelog.md`（含命令输出摘要）。
5. 未通过：用失败证据开**新**短会话重派；禁止续用旧长会话。

### 精度优先与无 JIT（强制）

- **不做 JIT**、不做块级编译、不跳过客户机指令或周期来换吞吐；性能只能优化宿主侧重复工作。
- **时钟是一等语义**：`XOSC`/`ROSC`/`PLL_SYS`/`PLL_USB` 与 `CLK_REF`/`CLK_SYS`/`CLK_PERI`/
  `CLK_USB`/`CLK_ADC`/`CLK_RTC` 的源与分频必须建模，**支持运行时超频/降频**（PLL 与分频写入
  后立即生效），`cycles→µs`、`SysTick`、`TIMER`、PIO `CLKDIV`、HSTX 像素时钟、UART 波特率、
  ADC/RTC、DMA 节拍都必须使用实时频率，禁止写死 150 MHz。
- 频率相关改动必须带已知配置向量测试（12 MHz→150 MHz、252 MHz 超频、源切换、分频为 0/1）
  与至少一个端到端固件验证。
- 性能报告按工作负载窗口（如 emerald 300–500 M 步），不得用启动/睡眠阶段代表整体。

## 通用性与指令精确（强制）

模拟器是**通用 RP2350 模型**；MicroPython / CircuitPython / pokeemerald 只是验收客户机，
不是优化目标，也不能作为裁剪语义的理由。

- 禁止按固件哈希/文件名、固定地址、固定 PC、循环形态、固定步数特判；禁止跳过指令或周期、
  伪造输出。发现这类改动一律拒绝验收。
- 数据手册/ARM ARM 定义的语义（`WFI`/`WFE`/`SEV`、阻塞 FIFO、总线 stall、外设复位值…）
  即使当前示例固件用不到，也必须排期实现；示例固件「用不到」只能用于判优先级，不能用于
  判「不需要」。因此 GAP-07（睡眠语义 + 阻塞 FIFO）仍是正确性任务，不因 emerald 忙等而取消。
- 性能优化只能削减宿主侧重复工作（缓存、去虚化、批量记账），不得减少客户机工作或改变
  可观察时序。
- 示例专用常量（帧缓冲、引脚、音频 SM/采样率、固件路径）只能位于 `example/`、CLI 参数、
  GUI 输入或 profile；crate 默认值必须通用。
- 子任务报告必须写明通用性验证（除示例固件外，用哪个固件/单测覆盖），以及是否引入
  「只对某个固件成立」的假设。

## 角色边界：主会话只拆分/派发/验收（强制）

主会话**不写功能代码**，只做：

1. **分析**：收集缺陷证据（命令输出、PC、错误报文、数据手册条目），产出任务清单；
2. **拆分/派发**：每个任务一个**全新**子会话，提示词自包含（目标/证据/白名单/验收/约束/报告模板）；
3. **验收**：复跑门禁与固件/FV/示例冒烟，抽查 diff，提交并同步文档。

允许主会话直接改的只有：纯文档/规则/示例说明、`.gitignore`/配置、≤10 行的文字或常量修正。
发现新缺陷时**新增任务**，不要顺手改代码。

### 子会话禁止复用（强制，防上下文超长）

- **一个任务 = 一个全新子会话**；已完成/已验收的子会话不得用于任何后续工作。
- **禁止**给旧子会话发 follow-up 做「小修」「补充」「再来一次」；返工必须开新会话，
  并在新提示词里附上失败证据（命令、断言、PC/错误报文）。
- 子会话内部不得再拆子任务（禁止嵌套会话），一次只做自己白名单内的实现 + 测试 + 文档 + 门禁。
- 子会话提示词**不得**依赖「上一轮/历史对话」；必须能独立复现与验收。
- 复用旧会话、或主会话自行实现功能代码，均视为**违反项目规则**，验收时应拒绝并重派。

## 子任务拆分模板

每个子任务提示词必须包含：

```
目标：<单一子系统目标>
现状证据：<命令 + 输出/PC/错误报文>
允许修改：<文件白名单，≤3 个文件>
验收命令：<可独立复跑的命令 + 预期输出>
停止条件：<超出白名单/需要跨 crate 大改 → 立即收口报告>
约束：无 git 提交；双语 doc 注释；单 .rs ≤ 500 行；unsafe 需 SAFETY
报告格式：状态 / 命令 / 关键输出 / 改动文件 / 新增测试 / 剩余卡点
```

crate 所有权参考（用于选择互不相交的任务）：

| 任务类型 | 主要 crate |
|----------|-----------|
| RISC-V 内核/启动 | `rp2350-riscv` + `rp2350-bootrom`（+ `rp2350-cli` 仅诊断输出） |
| ARM 内核/ISA | `rp2350-arm`（+ `rp2350-cpu` trait 默认方法） |
| USB 设备/主机桥 | `rp2350-usb` + `rp2350-usb-host` |
| 外设寄存器 | `rp2350-periph`（`map.rs`/`boot.rs` 挂载） |
| CLI/集成 | `rp2350-cli`（仅接线，不改内核语义） |

## 诊断手册

1. **启动轨迹**：`run --trace-steps 32 --strict-decode --max-cycles N`。
2. **未定义指令**：记录 PC/opcode；先区分「解码缺口」与「CSR/外设缺口」。
3. **首个偏离点**：在固件同一位置对照 `rp2350js` 的寄存器/内存行为，找第一个不一致。
4. **MMIO 告警**：`RUST_LOG=warn` 观察 `unimplemented MMIO access`；只有轮询循环依赖该地址时才算阻塞，按寄存器复位语义补桩。
5. **禁止**：按固件哈希/文件名特判、跳过指令、伪造门禁输出；必须是真实模拟行为。

## 已知缺口清单（2026-11-15 复核 / reviewed）

按 crate 归属排列；每项都已具备可复现证据或明确的数据手册条目，可直接拆成子任务。
Ordered by owning crate; every item has reproducible evidence or a datasheet reference
and can be dispatched as-is.

| ID | 领域 / Area | 缺口 / Gap | 归属 crate |
|----|-------------|-----------|-----------|
| GAP-01 | 引脚复用 | `IO_BANK0.GPIOx_CTRL.FUNCSEL` 未解释：PIO/UART/PWM 输出不会按功能选择驱动 pad，PIO 输入也只看宿主注入电平 | `rp2350-periph` |
| GAP-02 | GDB 多核 | 线程表只有单线程；`vCont` 只接受 `c`/`s`，无法按核 resume/stop，`qThreadInfo` 不暴露 core1 | `rp2350-gdb` + `rp2350-cli` |
| GAP-03 | ARM DSP | `SMLALxy`（`SMLALBB/BT/TB/TT`）未实现；`FPCCR.LSPENS/CLRONRET/CLRONRETS/TS/USER/THREAD` 只存储；`CP1-CP3/CP5-CP7` 未建模（当前按未定义） | `rp2350-arm` |
| GAP-04 | RISC-V | 未建模 CSR 读 0 而非触发非法 CSR 异常；RISC-V ROM 表项只实现 `rom_func_lookup` 所需子集；`misa` 未置 B 位 | `rp2350-riscv` |
| GAP-05 | USB 时序 | 主机桥未按 1 ms 调度 SOF（`host_sof()` 已有但 `poll()` 不调用）；双缓冲 OUT、CRC/位填充、全速时序未建模 | `rp2350-usb-host` + `rp2350-usb` |
| GAP-06 | HSTX | TMDS 串行化/10 位符号打包/DVI 位时序未建模；`EXPAND_EN=0` 原始透传不计时；无像素捕获 API（宿主只能拿 32 位字） | `rp2350-periph` |
| GAP-07 | 睡眠语义（**正确性，必做**） | `WFI`/`WFE`/`SEV` 曾在 `9a6b088` 实现但回退：`SIO_FIFO_RD` 非阻塞，`multicore_fifo_pop_blocking` 在睡眠中无法完成 core1 启动握手；需先做阻塞式 FIFO 读 + FIFO IRQ + 总线 stall/重试 | `rp2350-periph` → `rp2350-arm`/`rp2350-riscv`/`rp2350-cli` |
| GAP-08 | PSRAM | XIP PSRAM 后端未实现（`rp2350-virt-periph` 有模型但未接入 QMI 地址窗口） | `rp2350-virt-periph` + `rp2350-periph` |
| GAP-09 | egui 虚拟实验室 | 无设备调色板；画布状态不持久化（保存/加载、自动布局、撤销） | `rp2350-egui` |
| GAP-10 | MCP | 无 TLS 传输（当前仅明文 HTTP + Bearer） | `rp2350-mcp` |
| GAP-11 | DMA 细节 | `SNIFF_CTRL.CALC` `0x4..=0xD` 未建模；仲裁为轮转而非周期精确；`PERFCTR`/`BUSCTRL` 性能计数器不累加 | `rp2350-periph` |
| GAP-13 | 性能 | **已改进到 5.14 M steps/s**（emerald 50M 步 9.72 s，1.5×）：区域桶缓存、Flash/RAM 原生宽读写、写监视门控、MMIO 页缓存已落地；剩余瓶颈在 `rp2350-arm` 解释器（≈66–70 %）与 `Nvic` 256 项优先级扫描（详见 `doc/performance.md`） | `rp2350-arm` + `rp2350-core` + `rp2350-cli` |
| GAP-14 | 画面输出 | 无 GUI 实时显示：`g_fb` 只能 `--dump` 导出；缺帧源/纹理/节拍 | `rp2350-cli` + `rp2350-egui` |
| GAP-15 | 输入 | 键盘/手柄→GP0..9 无映射 | `rp2350-egui` + `rp2350-cli` |
| GAP-16 | 音频 | 采样捕获**已完成**（`AudioSink`/`AudioTap` → `Pio::push_tx`）；仍缺宿主播放后端（`rp2350-audio`，`cpal` 可选 + null 降级） | `rp2350-audio`（宿主后端） |
| GAP-18 | 时钟精度 | `CLOCKS` 只是寄存器桩：PLL_SYS/PLL_USB、`CLK_*_DIV`、源切换未建模，`cycles→µs` 与 HSTX/PIO/TIMER/UART 计时写死 150 MHz；**运行时超频/降频不生效** | `rp2350-periph` + `rp2350-core` + `rp2350-cli` |
| GAP-17 | 示例配置 | 帧缓冲地址/格式、按钮引脚、音频 SM 未形成可运行的示例 profile | `example/gba` |
| GAP-12 | 文档陈旧 | `doc/crates/rp2350-cli.md` 的 SIO/睡眠「未建模」段落已过时（自旋锁已实现、睡眠已回退）；`doc/crates/rp2350-periph.md` PIO IRQ 寄存器段落需复核 | `doc/`（主会话直接改） |

优先级建议：GAP-01/02 先做（影响真实固件的引脚与调试体验），随后 GAP-03/04（ISA 完整性）、
GAP-06/07（显示与多核调度）、GAP-05/08（外设/存储）、GAP-09/10（工具链）、GAP-11/12 收尾。

## 常用命令

```powershell
# FV-1 启动诊断
cargo run --release -p rp2350-cli -- run --file "E:\Source\mcu\pico\python\micropython\RPI_PICO2-RISCV-20260824-v1.29.0.uf2" --arch riscv --strict-decode --max-cycles 50000000 --trace-steps 32

# FV-3 启动基线
cargo run --release -p rp2350-cli -- run --file "E:\Source\mcu\pico\python\circuitpython\adafruit-circuitpython-raspberry_pi_pico2-zh_Latn_pinyin-10.3.1.uf2" --strict-decode --max-cycles 100000000

# 单 crate 快速回归
cargo test -p rp2350-riscv -p rp2350-bootrom
cargo test -p rp2350-usb-host -p rp2350-usb
cargo test -p rp2350-arm
```

## 禁止事项

- 子会话内执行 `git add/commit/push`（主会话统一提交）。
- 用 `run` 未崩溃替代 `repl` 输出 `42` 作为门禁通过。
- 针对固件文件名/哈希做特判或跳过指令。
- 在一个子会话里同时做多条门禁的长期开发（应拆分并新开会话）。
