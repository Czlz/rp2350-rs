# 可玩性状态 / Playability status

> V1 总计划见 [`handoff.md`](handoff.md)；性能与多线程证据见
> [`performance.md`](performance.md)、[`threading-plan.md`](threading-plan.md)。
> 示例固件与引脚/按键表见 [`../example/gba/README.md`](../example/gba/README.md)。
> 本文件只保留**当前状态与限制**，不再维护已完成任务清单。

## 1. 当前状态

| 指标 | 默认（确定性） | `--core-threads on` |
|---|---|---|
| emerald 300 M 步墙钟 | ≈9–11 s | ≈5–6 s（1.8–2.0×） |
| 真实游戏段吞吐 | ≈8.5 M 步/秒 | 约 14.9–27.4 M 步/秒 |
| 帧率（4.2 M 周期/帧） | **≈2 fps** | **3.5–6.5 fps** |
| 稳定性 | 2 G 步 / 30 分钟采样工作集恒定（41–42.6 MB） | 连续 10 次运行无崩溃/挂起 |

- 时钟按固件配置：`clk_sys = 252 MHz`（启动 150 MHz）；1 s 客户机时间 = 252 M 周期，
  60 fps 一帧 = **4.2 M 周期**。
- 睡眠核快进（`--sleep-fast-forward`，默认 on）把启动睡眠段从 4.2 s 降到 0.075 s（56–66×），
  但不影响游戏段。
- 默认模式位可复现（300 M 计数器/stdout/SRAM 与基线逐字节一致）；`--core-threads on`
  只固定 64 步窗口边界，窗口内访存顺序依赖宿主调度，**非位可复现**。

## 2. 运行

```bash
# 无 GUI：逐帧统计
rp2350-cli play --profile example/gba/profile.conf --frames 130

# GUI（自动开始）
rp2350-egui --profile example/gba/profile.conf

# GUI 交互加速（非位可复现）
rp2350-egui --profile example/gba/profile.conf --core-threads --core-window 64
```

按键：`Z`=A、`X`=B、`Backspace`=Select、`Enter`=Start、方向键、`S`=R、`A`=L（画面获焦后生效）。

## 3. 已实现

- `rp2350-cli` lib + `play` worker（后台线程、`Arc<Frame>` 最新帧、按键掩码、`--frames N` headless）
- egui PlayView：纹理、3:2 缩放、键盘、音频输出（`cpal` 可选 / null 降级）、Reset、命令行直启、
  线程开关
- 示例配置：`example/gba/profile.conf`（固件/帧缓冲/每帧步数/按键/音频 PIO/采样率）；
  模拟器内**没有示例默认值**，无参数时明确报错
- I2S 捕获（`AudioSink`/`AudioTap`，CPU 与 DMA 共用 `Pio::push_tx`）+ `rp2350-audio` 输出
- `play` 每帧走 `run_steps` 批量，线程模式真正生效（130 帧 A/B 2.00×）

## 4. 已知限制

1. **帧率不足以实时游玩**：默认 ≈2 fps；无 JIT（项目决策），V1 不承诺 60 fps。
2. **音频必然欠载**：I2S 13 440 帧/秒，而模拟器只产出数百帧/秒；GUI 显示
   `produced/s` vs `need/s` 与 `underruns`，保持静音、不伪造、不循环旧样本。
3. `--core-threads on` 非位可复现；需要可复现时用默认 off。
4. 线程模式只对 Arm 且 core1 已启动生效；RISC-V/单核退化串行。
5. 音频在 GUI 中需要 `audio-playback` feature（cpal）；无声卡时 null 后端静默运行。
6. 没有存档/状态保存（V1 工作包 R5）；复位是宿主侧重启，不是存档恢复。
7. 示例固件帧缓冲/按键/I2S 接线都在 `example/gba/profile.conf`；换固件请复制该文件。
