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

**结论**：性能是唯一的一级阻塞（≈50× 差距），必须先做；视频/输入/音频可并行推进。

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
| P0-3a 🚧 | 第三轮（`arm`+`core`）：数据访问快路径内联、取指/译码与异常检查合并、`core` 侧桶表内联 | `rp2350-arm` + `rp2350-core` | emerald 50M ≤2.6 s（≥19 M steps/s）；300M 计数器逐字段不变 |
| P0-3b ⏳ | 第三轮（`cli`）：批量步进摊薄每步骨架、泵送事件门控、双核分线程评估 | `rp2350-cli` | emerald 50M ≤1.8 s（≥28 M steps/s） |
| P0-3c ⏳ | 决策点：记录 20/30/60 fps 可行性；若 <30 fps，评估块级 JIT / 帧级并行作为 P0-4 | 主会话 | `doc/performance.md` 结论 |

### P1 画面 / Video

| ID | 任务 | 归属 | 验收 |
|----|------|------|------|
| P1-1 ✅ | `play` worker：独占线程运行 `Machine`，按墙钟节拍推进；发布 RGB565 帧（单槽最新帧/三缓冲语义）；接收按钮掩码；`--frames N` 无 GUI 测试模式 | `rp2350-cli`（新增 lib target + `play.rs`） | **实测 6.1–7.0 fps / 15.3–17.5 M steps/s**（emerald，`--steps-per-frame 2500000 --frames 20`，两次运行，退出码 0，20 帧记录 + 汇总）；按钮掩码单测（按下→低 / 松开→高 / 只动配置引脚）；`cargo test -p rp2350-cli` 126 passed / 1 ignored（新增 16 项 `play` 测试 + 1 项 CLI 解析） |
| P1-2 | egui PlayView：纹理上传 + 缩放（保持 3:2）+ 键盘映射（方向键/Z/X/Enter/Backspace）；标签页「游戏」 | `rp2350-egui` | 80×60/320×240/1280×720 无头渲染；键盘→掩码单测；`cargo test -p rp2350-egui` 全绿 |
| P1-3 | 示例配置：`example/gba/profile.toml`（固件路径、帧缓冲/格式、按钮引脚、音频 SM）与 `README` 游玩步骤 | `example/gba` + 主会话 | 一条命令启动示例；文档含按键表 |

### P2 音频 / Audio

| ID | 任务 | 归属 | 验收 |
|----|------|------|------|
| P2-1 ✅ | I2S 采样捕获：`AudioSink`/`AudioTap`，唯一接线点 `Pio::push_tx`（CPU 与 DMA 共用），`BootPeripherals::set_audio_tap` | `rp2350-periph` | periph 347 测试；CPU/DMA 两路径 + 端到端；未安装 tap 零开销 |
| P2-2 ✅ | 宿主输出 crate `rp2350-audio`（`cpal` 可选 feature，null 后端静默降级，有界队列 + 墙钟节奏，最近邻重采样） | `crates/rp2350-audio`（新） | 12/13 测试（`--all-features` 实测 backend=cpal）；`shutdown` 毫秒级；无设备不 panic |

### P3 完善 / Polish

| ID | 任务 | 归属 | 验收 |
|----|------|------|------|
| P3-1 | GAP-08b：CLI 挂载 PSRAM 窗口（`run`/`repl`/`play`） | `rp2350-cli` | 总线路由测试 + FV |
| P3-2 | 存档/复位：`monitor reset` 在 GUI 可用；flash 存档路径可配置 | cli + egui | 手动冒烟 |
| P3-3 | 稳定性：2 小时游玩 soak（headless，模拟时间 ≥30 分钟），内存/句柄不增长 | 主会话 | 日志 + 结论 |

## 3. 波次与并发 / Waves

并发上限 2（crate 不相交），每波完成后主会话复跑四道门禁 + FV-1/2/3 + emerald 300M 冒烟。

1. **Wave 1 ✅**：P0-1（core+cli）‖ P2-1（periph）——均已验收提交
2. **Wave 2 ✅**：P0-2（arm+core+cli）‖ P2-2（`rp2350-audio` 新 crate）——均已验收提交
3. **Wave 3 🚧**：P0-3a（arm+core）‖ P1-1（cli lib + `play` worker + headless `--frames`）
4. **Wave 4**：P0-3b（cli 批量步进）‖ P1-2（egui PlayView，消费 P1-1 的 `PlaySession` API）
5. **Wave 5**：P1-3（示例 profile）+ P3-1（GAP-08b PSRAM 挂载）+ P3-3（soak）；P0-3c 决策点

## 4. 风险 / Risks

- **性能**：两轮后 14.8 M steps/s（累计 4.6×）。按每游戏帧 ≈2.5 M 步计算约 **6 fps**；30 fps 需 ≥75 M steps/s，60 fps 需 ≥150 M。P0-3a/b 之后若仍 <30 fps，实时游玩只能降级演示，需要 P0-3c 决策是否投入 JIT/帧级并行；
  备选方案：双核分线程（≈1.5×）、JIT（大工程）、或把 PPU 的 HSTX 路径换成直接帧输出
  （不改变访客代码，无法绕过）。结论写入 `doc/performance.md`。
- **GUI 架构**：`rp2350-cli` 已提供 lib target（`rp2350_cli::run::Machine` +
  `rp2350_cli::play::PlaySession`），P1-2 的 egui PlayView 直接依赖即可。
- **音频**：`cpal` 在无声卡/CI 环境会失败；插件必须可选、可静默。
- **示例耦合**：帧缓冲地址与按钮引脚属于示例配置，必须放 `example/gba/`，不写死进模拟器。
