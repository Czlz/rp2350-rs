# 可玩性开发计划 / Playability plan

目标：让人类可以在模拟器里**正常操作游玩** `emerald.uf2`
（pokeemerald-rp2350），即：实时显示游戏画面、键盘/手柄输入、音频输出，并且稳定不崩溃。
Goal: a human can actually **play** `emerald.uf2` in the emulator — real-time video,
keyboard/gamepad input, audio output, and no crashes.

本文件是 `doc/implementation-plan.md` 的 P2 阶段细化；示例相关（固件路径、引脚、帧缓冲
地址）在 [`example/gba/README.md`](../example/gba/README.md)。

---

## 0. 现状与硬指标 / Current state and the hard numbers

| 指标 / Metric | 现状 / Now | 可玩所需 / Needed to play |
|---|---|---|
| 模拟吞吐 / Emulation throughput | **3.0 M steps/s**（emerald 50M 步 16.6 s） | ≥50 M steps/s（≈20 fps）…≥150 M steps/s（60 fps） |
| 每游戏帧步数 / Steps per game frame | ≈2.5 M（双核 + 软件 PPU） | 不变 |
| 画面输出 / Video | `g_fb` 在 SRAM（`0x20063DF4`，240×160 RGB565），仅 `run --dump` 导出 | GUI 实时纹理 |
| 输入 / Input | 固件读 GP0..9（低有效、上拉），模拟器可 `GpioPads` 注入 | 键盘/手柄映射 |
| 音频 / Audio | 固件 I2S（PIO2 + DMA）已运行，模拟器无采样出口 | 采样捕获 + 播放插件 |
| 稳定性 / Stability | 2 G 步无崩溃（`underruns=0 wof=false unknown=0`） | 长时间游玩 |

**结论**：性能是一级阻塞，必须先做；视频/输入/音频可并行推进。

**2026-11-15 更新（四轮性能后）**：吞吐 **3.0 → 28.5–30.2 M steps/s（≈10×）**，
`play` 实测 **12.4 fps**（每游戏帧 ≈2.5 M 步，客户机按 60 fps 自我配速）。60 fps 需
≥150 M steps/s（缺口 5×）。剩余成本 65–75 % 在 `rp2350-arm` 逐指令取指/译码，路线是
译码缓存（P0-3c）→ 必要时 JIT。

**关于 `WFI`（更正）**：emerald 用忙等实现（SDK `tight_loop_contents()` 是空函数），
这只说明该固件不走睡眠路径，**不构成「不需要睡眠语义」的理由**。本项目是通用 RP2350
模拟器（见 `.trae/rules/project.md` 的「通用性与指令精确」），`WFI`/`WFE`/`SEV` 与阻塞式
FIFO 是架构语义，必须实现；GAP-07 因此是**正确性任务**，保留在队列中并行推进，且不得
为了本示例的帧率而用「跳过自旋」之类的取巧手段替代。

## 1. 组件与依赖 / Components and dependencies

```
                 ┌──────────────┐  RGB565 frames   ┌──────────────┐
  emerald guest →│ Machine      │─────────────────→│ play worker  │→ triple buffer → egui PlayView
  (cli lib)      │ (cli lib)    │←─────────────────│ (cli lib)    │← button mask ← keyboard
                 └──────────────┘  input mask       └──────────────┘
                        │ audio words
                        ▼
                 ┌──────────────┐  i16 samples     ┌──────────────┐
                 │ PIO2 TX FIFO │─────────────────→│ audio plugin │→ cpal → speakers
                 └──────────────┘  (AudioSink)      └──────────────┘
```

- `rp2350-cli` 已拆出 **lib target**（`rp2350_cli::run::Machine`），`rp2350-egui`
  依赖即可复用同一台机器；`PlaySession`（`rp2350_cli::play`）提供后台帧与输入。
- 帧源用**示例配置**（帧缓冲地址/格式）而不是 HSTX 像素流：HSTX 无扫描线/帧边界模型，
  而 `g_fb` 是固件每帧完整渲染的最终图像（已被 1.2 G 步的 Game Freak 帧验证）。
- 输入走 `GpioPads::set_external_pin`（GP0..9，低有效），与固件的 `gpio_get_all()` 一致。

## 2. 阶段与任务 / Phases and tasks

### P0 性能（一级阻塞，先做）

| ID | 任务 | 归属 | 验收 |
|----|------|------|------|
| P0-1 ✅ | 解释器热路径优化：区域桶缓存、Flash/RAM 原生宽读写、写监视门控、MMIO 页缓存、IRQ 位遍历 | `rp2350-core` + `rp2350-cli` | **实测 9.72 s / 5.14 M steps/s（3.36 → 5.14，1.5×）**；300M 计数器逐字段不变；未达 6 s，瓶颈定量在 `rp2350-arm`（详见 [performance.md](performance.md)） |
| P0-2 ✅ | 第二轮：`Nvic` 位图优先级（−43 %）、ROM 桩窗口过滤（−20 %）、通知门控（−14 %）、Flash 取指缓存、PPB 写世代门控、定长 `record_step` | `rp2350-arm` + `rp2350-core` + `rp2350-cli` | **实测 3.39 s / 14.8 M steps/s**（子会话最小 3.21 s / 15.6 M）；300M 计数器逐字段不变；累计 **4.6×** |
| P0-3a ✅ | 第三轮（`arm`+`core`）：纠正异常检查读数、UF2 装载 O(n²)→`HashSet`、批量 `load_bytes` | `rp2350-arm` + `rp2350-core` | **实测 2.45–2.51 s / 19.9–20.4 M/s**（启动 0.045 s）；300M 计数器逐字段不变；累计 **6.8×** |
| P0-3b ✅ | 第四轮（`cli`）：`run_steps` 批量步进（−27 %）、core1 门控（−11 %）、`record_sync_trap` 跳过；`play` 按键改每 4096 步、`read_block` 按字批量 | `rp2350-cli` | **实测 1.68–1.75 s / 28.5–29.8 M/s**（子会话最小 1.655 s / 30.2 M）；300M 15.8 s；`play` 12.4 fps |
| P0-3c ✅ | 第五轮（`arm`+`core`）：1024 条译码缓存（只缓存 Flash 窗口、`content_generation` 失效）+ 线程化分发；SRAM 仍走解释器 | `rp2350-arm` + `rp2350-core` | **实测 1.60–1.70 s / 29.5–31.2 M/s**（子会话 11 样本最小 **1.470 s / 34.0 M**；目标 1.2 s 未达，差 1.24×）；300M 16.3 s、计数器逐字段不变；10 项两核差分测试 |

### P1 画面 / Video

| ID | 任务 | 归属 | 验收 |
|----|------|------|------|
| P1-1 ✅ | `play` worker：独占线程运行 `Machine`，按墙钟节拍推进；发布 RGB565 帧（单槽最新帧/三缓冲语义）；接收按钮掩码；`--frames N` 无 GUI 测试模式 | `rp2350-cli`（新增 lib target + `play.rs`） | **实测 6.1–7.0 fps / 15.3–17.5 M steps/s**（emerald，`--steps-per-frame 2500000 --frames 20`，两次运行，退出码 0，20 帧记录 + 汇总）；按钮掩码单测（按下→低 / 松开→高 / 只动配置引脚）；`cargo test -p rp2350-cli` 126 passed / 1 ignored（新增 16 项 `play` 测试 + 1 项 CLI 解析） |
| P1-2 ✅ | egui PlayView：纹理上传（新帧才上传）+ 保持 3:2 缩放（≥1 时整数倍、<1 时小数）+ 键盘映射（方向键/Z/X/Enter/Backspace/S/A）+ 启动/停止/音频开关/状态栏；标签页「游戏」；`PlaySource`/`PlayStarter` 抽象保留，并在 `play_view/cli_source.rs` 提供真实适配器 `CliPlaySource`/`CliPlayStarter`（包装 `rp2350_cli::play::PlaySession`），`main.rs` 默认挂载。**已按通用性规则整改**：删除 `DEFAULT_FIRMWARE`/`DEFAULT_FRAME_BUFFER`/`DEFAULT_STEPS_PER_FRAME`，工具栏改为 profile 路径 + `Load profile`（`rp2350_cli::play::profile`）+ 固件/帧缓冲/每帧步数/按键掩码/音频块/SM/采样率/声道/容量全部可编辑，运行中置灰；`PlayRequest.audio` 改为 `Option<AudioRequest>` → `PlayConfig.audio`（`None` = 不捕获） | `rp2350-egui`（依赖 `rp2350-cli`） | 80×60/320×240/1280×720 无头渲染；键盘→掩码单测（GP0..GP9 逐位）；真实 `PlaySession` 适配器测试 5 项（首帧发布与掩码透传、统计单调、帧缓存复用、启动失败路径、`CliPlayStarter` 端到端无头出帧）；**profile/字段测试 14 项**（字段默认全空、缺 firmware/frame 拒绝启动、按键掩码十六/十进制/空、`audio=None` vs `Some(AudioRequest)` 逐字段透传、音频字段校验、`example/gba/profile.conf` 逐项比对（缺失则跳过）、非法 profile 报 `profile line N` 不改字段、空路径、`format_frame` 往返、真实 starter 未装 tap / tap 采样率声道 / 越界 PIO 块拒绝）；`cargo test -p rp2350-egui` **100 passed**；`cargo fmt -p rp2350-egui -- --check`、`clippy -D warnings -W pedantic`、`cargo doc --no-deps` 零警告；`rg "emerald\|0x20063DF4" crates/rp2350-egui/src` 无匹配（无生产示例常量）；依赖方向 `cargo tree -p rp2350-egui --depth 1` 含 `rp2350-cli`、`cargo tree -p rp2350-cli --depth 1` 不含 `rp2350-egui` |
| P1-3 ✅ | 示例配置已就位：`example/gba/profile.conf`（`key = value`：固件路径、帧缓冲、按钮引脚、I2S PIO 块/状态机/采样率）+ `rp2350-cli play --profile`；crate 内不再有示例默认值 | `example/gba` + `rp2350-cli` | 一条命令启动示例：`rp2350-cli play --profile example/gba/profile.conf --frames 20`；文档含 profile 格式表与按键表 |

### P2 音频 / Audio

| ID | 任务 | 归属 | 验收 |
|----|------|------|------|
| P2-1 ✅ | I2S 采样捕获：`AudioSink`/`AudioTap`，唯一接线点 `Pio::push_tx`（CPU 与 DMA 共用），`BootPeripherals::set_audio_tap` | `rp2350-periph` | periph 347 测试；CPU/DMA 两路径 + 端到端；未安装 tap 零开销 |
| P2-2 ✅ | 宿主输出 crate `rp2350-audio`（`cpal` 可选 feature，null 后端静默降级，有界队列 + 墙钟节奏，最近邻重采样） | `crates/rp2350-audio`（新） | 12/13 测试（`--all-features` 实测 backend=cpal）；`shutdown` 毫秒级；无设备不 panic |
| P2-3 ✅ | **GUI 已接线**：`PlayView` 勾选音频时用 `PlaySource::audio_source()` 取 worker 的 `AudioTap` 并启动 `AudioOutput`（容量 1 s / 96k 样本）；`stop`/`Drop` **先** join 音频线程再关会话；`audio-playback` feature 转发 `rp2350-audio/playback`；状态栏 `audio=<backend> underruns=<n>`，null 后端提示一次；运行中音频复选框置灰 | `rp2350-egui` | `cargo test -p rp2350-egui` **86 passed**（+6 音频接线）；`--features audio-playback` 86 passed；`backend ∈ {cpal, null}`；`stop()` 2 s 内 join；`Drop` 不 panic |

### P3 完善 / Polish

| ID | 任务 | 归属 | 验收 |
|----|------|------|------|
| P3-1 ✅ | GAP-08b：CLI 挂载 PSRAM 窗口（`run`/`repl`/`play`/`gdb`） | `rp2350-cli` | `--psram SIZE`（字节/`K`/`M`/`MiB`）+ profile `psram` 键；未给出时不注册（`0x1100_0000` 仍 `NoDevice`），启动错误走 `anyhow`（超限只警告一次）；`cargo test -p rp2350-cli` 115 + 86 绿，FV-1/2/3 `stdout: 42`，emerald 300M 冒烟计数器不变 |
| P3-2 | 存档/复位：`monitor reset` 在 GUI 可用；flash 存档路径可配置 | cli + egui | 手动冒烟 |
| P3-3 | 稳定性：2 小时游玩 soak（headless，模拟时间 ≥30 分钟），内存/句柄不增长 | 主会话 | 日志 + 结论 |

## 3. 波次与并发 / Waves

并发上限 2（crate 不相交），每波完成后主会话复跑四道门禁 + FV-1/2/3 + emerald 300M 冒烟。

1. **Wave 1 ✅**：P0-1（core+cli）‖ P2-1（periph）——均已验收提交
2. **Wave 2 ✅**：P0-2（arm+core+cli）‖ P2-2（`rp2350-audio` 新 crate）——均已验收提交
3. **Wave 3 ✅**：P0-3a（arm+core）‖ P1-1（cli lib + `play` worker + headless `--frames`）
4. **Wave 4 ✅**：P0-3b（cli 批量步进/门控 + play 输入粒度）‖ P1-2（egui PlayView 已接线到真实 `PlaySession`）
5. **Wave 5 ✅**：P0-3c（arm 译码缓存）‖ P2-3（GUI 音频）+ 示例配置整改（`profile.conf` / CLI `--profile` / GUI 无默认值）
6. **Wave 6 🚧**：GAP-07 ✅（`WFI`/`WFE`/`SEV` + 阻塞 FIFO + 睡眠调度，**正确性**）‖ GAP-11 ✅ 部分（DMA `SNIFF` 向量核对 + `PERFCTR`/`PERFSEL` 事件计数 + 总线记账；周期精确仲裁仍待）
7. **Wave 7 🚧**：GAP-08b ✅（CLI 挂载 PSRAM）+ P3-2（存档/复位）+ GAP-12（文档）；P3-3（soak）最后

## 3.6 精度任务（当前最高优先级）/ Precision tasks

| ID | 任务 | 归属 | 状态 |
|----|------|------|------|
| P4-1 | `CLOCKS` 时钟树：XOSC/ROSC/PLL_SYS/PLL_USB（REFDIV/FBDIV/PDIV/VDIV/LOCK）、`CLK_*_CTRL`/`CLK_*_DIV`、源切换与实时频率 API + 已知向量测试（12 MHz→150 MHz、252 MHz 超频） | `rp2350-periph` | ⏳ 待派发 |
| P4-2 | 动态频率接入全部计时消费者：`cycles→µs`、SysTick、TIMER、PIO `CLKDIV`、HSTX 像素时钟、UART 波特率、ADC/RTC、DMA 节拍；端到端固件验证（含 252 MHz 固件样例） | `rp2350-periph` + `rp2350-core` + `rp2350-cli` | ⏳ 待派发（依赖 P4-1） |
| P4-3 | 时钟/频率回归与文档：超频/降频/源切换的 300M 计数器对照、`doc/architecture.md` 时钟章节 | 主会话 | ⏳ |

## 3.5 通用性任务（与帧率无关，必须推进）/ Generality tasks

| ID | 任务 | 归属 | 状态 |
|----|------|------|------|
| GAP-07 | 阻塞式 `SIO_FIFO_RD`/FIFO IRQ + 总线 stall，重新落地 `WFI`/`WFE`/`SEV` 睡眠语义 | periph → arm/riscv/cli | ✅ 已完成（`BusErrorKind::Stall` + `MemoryRegion::read_would_stall`、`SIO_FIFO_ST` 粘滞位与写 1 清除、事件世代号、ARM/RISC-V 睡眠状态机、`[run] sleep` 统计） |
| GAP-08b | CLI 挂载 PSRAM 窗口（模型已完成，地址可配置） | cli | ✅ 已完成（`MachineOptions::psram` + `PsramOptions::parse`；`--psram`/profile `psram` 覆盖 `run`/`repl`/`play`/`gdb`；未启用时 `0x1100_0000` 仍 `NoDevice`；超限/未对齐按 periph 规则钳制到 16 MiB 并只警告一次；挂载失败映射为启动错误） |
| GAP-11 | DMA `SNIFF` 其余模式 / `PERFCTR` / 周期精确仲裁 | periph | ✅ 部分完成（`SNIFF_CTRL.CALC` 已按 `dma.h` 核对：仅 `0x0..=0x3`/`0xE`/`0xF` 有定义并有标准向量，`0x4..=0xD` 为保留值；嗅探改为观察通道 BSWAP 之后的单元；`BUSCTRL.PERFCTR0/1`+`PERFSEL0/1` 按端口累加 DMA `ACCESS`/`STALL_UPSTREAM`；`DmaBusCounters` 记账 DMA 占用周期与授予；**剩余**：`STALL_DOWNSTREAM`/`ACCESS_CONTESTED`、CPU 主设备事件与真正抢占需周期精确模型，`BootPeripherals` 内 `Dma`↔`BusCtrl` 接线需 `boot.rs` 一行） |
| GAP-10 | MCP TLS 传输 | mcp | ⏳ |
| GAP-12 | 陈旧文档段落同步 | doc | ⏳ |

## 4. 风险 / Risks

- **性能与定位（2026-11-15 决定）**：总步数吞吐 29.5–34 M/s 主要来自启动段（睡眠/自旋）；**真实游戏段 8.0–8.5 M steps/s ≈ 3.3 fps**（`doc/performance.md` §9）。**不做 JIT**：优先级是精确时钟/精确指令/功能与插件稳定；吞吐只优化宿主侧（缓存、去虚化、事件门控）。60 fps 不在当前目标内，3.3 fps 作为「运行真实固件」的当前水平记录；
  备选方案：双核分线程（≈1.5×）、JIT（大工程）、或把 PPU 的 HSTX 路径换成直接帧输出
  （不改变访客代码，无法绕过）。结论写入 `doc/performance.md`。
- **GUI 架构**：`rp2350-cli` 提供 lib target（`rp2350_cli::run::Machine` +
  `rp2350_cli::play::PlaySession`）。依赖方向已修正为 **egui → CLI**：删除了
  `rp2350-cli` 中从未被源码使用的 `rp2350-egui` 依赖（它曾造成包循环），
  `rp2350-egui` 直接依赖 `rp2350-cli`，在 `play_view/cli_source.rs` 提供
  `CliPlaySource` / `CliPlayStarter`，`main.rs` 默认挂载。`PlaySource` +
  `PlayStarter` 抽象保留，仅用于测试注入与其它宿主。证据：
  `cargo tree -p rp2350-egui --depth 1` 含 `rp2350-cli`，
  `cargo tree -p rp2350-cli --depth 1` 不含 `rp2350-egui`。
- **音频**：`cpal` 在无声卡/CI 环境会失败；插件必须可选、可静默。
- **示例耦合**：帧缓冲地址与按钮引脚属于示例配置，必须放 `example/gba/`，不写死进模拟器。
