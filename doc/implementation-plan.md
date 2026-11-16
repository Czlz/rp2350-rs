# V1 发布计划 / V1 release plan

本文件只保留**当前有效**的发布计划。历史阶段计划（Phase 1–10、M1–M5、GAP-01…20 的已完成项）
已删除，已完成工作的细节见 `doc/changelog.md` 与各 crate 文档。

- **V1 定义、任务拆分、验收命令、已知限制**：[`handoff.md`](handoff.md)
- **经验教训**：[`lessons.md`](lessons.md)
- **目标固件验收**：[`implementation-plan.md`](implementation-plan.md) 的 FV 清单见下
- **可玩性/示例**：[`playability-plan.md`](playability-plan.md)、[`../example/README.md`](../example/README.md)
- **多线程/时钟/性能证据**：[`threading-plan.md`](threading-plan.md)、[`performance.md`](performance.md)

## 1. 当前状态（2026-11-16）

- 40 个测试目标、1999 个测试全绿；fmt / clippy(pedantic) / doc 全绿。
- FV-1（RISC-V MicroPython）、FV-2（Arm MicroPython）、FV-3（CircuitPython）均 `stdout: 42`。
- emerald 300 M 基线：`time_us=1798617`、`dma=2199378`、`hstx words=5401204 underruns=0`、
  `sleep core0 steps=224994790 wakes=6`、`core1 Running`、8 条 `[irq]`、PC `0x1000B3AC`。
- 时钟按固件配置的 252 MHz（启动 150 MHz）；默认确定性模式真实游戏段 ≈2 fps，
  `--core-threads on` 3.5–6.5 fps（非位可复现）。

## 2. 发布工作包（详见 handoff §3）

| ID | 工作包 | 归属 | 估算 |
|----|--------|------|------|
| R1 | 发布卫生：LICENSE、README、CHANGELOG 1.0.0、版本号、过时文档清理 | 主会话 | 1 波 |
| R2 | 插件宿主接线（`--plugin-threads` + 真实设备循环） | cli + egui | 1–2 波 |
| R3 | V1 精度/限制声明（含 release notes 素材） | 主会话 | 与 R1 合并 |
| R4 | 稳定性：3 OS CI、30 次并行、2 G/30 min soak、线程模式长跑 | 主会话/子会话 | 1 波 |
| R5 | 存档/状态保存（建议纳入；不纳入需在 R3 排除） | cli + egui | 1–2 波 |
| R6 | 打 `v1.0.0` 标签并触发 release workflow | 主会话 | 0.5 波 |

顺序：R1 → R2 → R3 → R4 →（R5）→ R6。R1 与 R2 可并行（crate 不相交）。

## 3. 固件验证清单（V1 门禁）

> **门禁规则**：对应内核必须先跑通下列固件，才允许进入发布候选。

| ID | 固件 / Firmware | 架构 | 验证命令 | 状态 |
|----|-----------------|------|----------|------|
| FV-1 | `...RPI_PICO2-RISCV-20260824-v1.29.0.uf2` | Hazard3 | `repl --exec "print(6 * 7)"` → `42`；`run --arch riscv --strict-decode` 无未定义指令 | ✅ 通过 |
| FV-2 | `...RPI_PICO2-20260824-v1.29.0.uf2` | Cortex-M33 | `repl --exec "print(6 * 7)"` → `42`；`perfbench` 至少一个基准 | ✅ 通过 |
| FV-3 | `...adafruit-circuitpython-raspberry_pi_pico2-zh_Latn_pinyin-10.3.1.uf2` | Cortex-M33 | `repl --no-soft-reset --exec "print(6 * 7)"` → `42` | ✅ 通过 |
| FV-4 | `...\emerald.uf2`（示例） | Cortex-M33 ×2 | `run --max-cycles 300000000 --trace-steps 0`：`dma=2199378`、`hstx words=5401204 underruns=0`、`core1 Running`、8 条 `[irq]` | ✅ 通过 |

## 4. 变更记录

见 [`changelog.md`](changelog.md)。V1 发布时把 `[Unreleased]` 整理为 `[1.0.0]`。
