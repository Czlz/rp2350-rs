# 示例项目 / Example projects

本目录存放用本模拟器运行**真实固件/程序**的示例。每个示例一个子目录，包含：

- `README.md` — 目标固件（路径/大小/SHA-256）、运行命令、期望输出、**引脚配置**、
  **外设与插件依赖**；
- `doc/` — 该示例的验证报告、性能数据、截图；
- `tools/` — 示例专用脚本。

规则见 [`AGENTS.md`](../AGENTS.md) 的「示例项目目录（example/，强制）」：示例专用内容
不放主 `doc/`，主 `doc/` 只保留跨示例的通用文档并链接到此处。
This directory holds examples that drive **real firmware** with this emulator.
Each example has its own directory with a `README.md` (target firmware, run
commands, expected output, pin map, peripheral/plugin dependencies), `doc/`
(verification reports, screenshots) and `tools/`. Example-specific material must
not go into the repository `doc/` tree.

| 示例 / Example | 目标 / Target | 说明 / Notes |
|----------------|---------------|--------------|
| [gba](gba/README.md) | `emerald.uf2` (pokeemerald-rp2350) | 双核 Cortex-M33 + PIO + DMA 控制块环 + HSTX + USB CDC + I2S；HSTX 扫描输出真实游戏画面。报告：[gba-bringup-report.md](gba/doc/gba-bringup-report.md) / Two Cortex-M33 cores, PIO, DMA control-block ring, HSTX, USB CDC and I2S; HSTX scans out real game frames |

## 新增示例 / Adding an example

1. 建 `example/<name>/README.md`，按上表列出固件信息、运行命令、引脚表与外设依赖；
2. 验证数据/截图放 `example/<name>/doc/`，可复用脚本放 `example/<name>/tools/`；
3. 在本表加一行，并（如需）在根 `README.md` 与 `doc/implementation-plan.md` 用相对路径链接。
