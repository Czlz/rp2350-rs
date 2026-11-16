# V1 发布计划 / V1 release plan

本文件只保留**当前有效**的发布计划与状态。历史阶段计划（Phase 1–10、M1–M5、已完成 GAP 项）
已删除，已完成工作的细节见 [`changelog.md`](changelog.md) 与各 crate 文档。

- **V1 范围/精度声明（发布口径）**：[`v1-scope.md`](v1-scope.md)
- **经验教训**：[`lessons.md`](lessons.md)
- **可玩性/示例**：[`playability-plan.md`](playability-plan.md)、[`../example/README.md`](../example/README.md)
- **多线程/时钟/性能证据**：[`threading-plan.md`](threading-plan.md)、[`performance.md`](performance.md)

## 1. 当前状态（2026-11-16）

- 40 个测试目标、**2011 个测试**全绿（arm 542、periph 483、cli 191+2 ignored / 89、core 122、
  egui 141、gdb 91、riscv 85、virt-periph 74、audio 20、usb 53、usb-host 33+2、bootrom 21…）；
  fmt / clippy(pedantic) / doc 全绿；连跑 30 次无 flake。
- FV-1（RISC-V MicroPython）、FV-2（Arm MicroPython）、FV-3（CircuitPython）均 `stdout: 42`。
- emerald 300 M 基线（默认确定性）：`time_us=1798617`、`dma=2199378`、`hstx words=5401204 underruns=0`、
  `sleep core0 steps=224994790 wakes=6`、`core1 Running`、8 条 `[irq]`、PC `0x1000B3AC`。
- emerald A/B（300 M，本机）：默认 8.87 s vs `--core-threads on` 4.79 s（1.85×）。
- soak：2 G 步 241 s，工作集恒定 42.6 MB、句柄 65；`--core-threads on` 20 G 步 / 25 min，
  线程 3、句柄 61 恒定，`hstx underruns=0`。
- 时钟按固件配置的 252 MHz（启动 150 MHz）；默认确定性模式真实游戏段 ≈2 fps，
  `--core-threads on` 3.5–6.5 fps（非位可复现）。

## 2. 发布工作包

| ID | 工作包 | 归属 | 状态 |
|----|--------|------|------|
| R1 | 发布卫生：LICENSE、README、CHANGELOG 1.0.0、版本号、过时文档清理 | 主会话 | ✅ 完成 |
| R2 | 插件宿主接线（`--plugin-threads` + 真实设备循环） | cli + egui 子会话 | ✅ 完成（仅 `run`；`repl`/`play` 未接线） |
| R3 | V1 精度/限制声明（`doc/v1-scope.md`，含 release notes 素材） | 主会话 | ✅ 完成（README/doc 索引/release notes 引用；CLI/GUI 内置链接列 V1 后） |
| R4 | 稳定性：3 OS CI、30 次并行、2 G/30 min soak、线程模式长跑 | 主会话 | 🟡 本地全部完成；3 OS CI 需推送后确认 |
| R5 | 存档/状态保存 | — | ❌ V1 明确排除（见 `v1-scope.md` §3.8） |
| R6 | 打 `v1.0.0` 标签并触发 release workflow | 主会话 | ⬜ 待执行 |

顺序：R1 → R2 → R3 → R4 → R6（R5 排除）。

### R1 发布卫生清单

- [x] `LICENSE` + `LICENSE-MIT` + `LICENSE-APACHE`
- [x] `README.md` 特性表与实现对齐（双核/FUNCSEL/PSRAM/GDB 多核/音频/GUI）
- [x] `doc/README.md` 索引更新（新增 v1-scope/playability/threading/performance，移除 handoff）
- [x] `CHANGELOG.md` → `doc/changelog.md`：`[Unreleased]` 收口为 `[1.0.0]`
- [x] 版本 `0.1.0` → `1.0.0`（`[workspace.package]`）
- [x] `doc/architecture.md` PIO `CLKDIV`/泵送边界语义补充
- [x] 过时「未建模」条目清理（`doc/crates/rp2350-cli.md`、`doc/crates/rp2350-periph.md`）

### R2 验收标准（已复跑）

- `run --plugin-dir <DIR> --wire <DEVICE:PORT:GPIO> --plugin-threads` 在 64 步泵送边界驱动插件 ✅
- 内联与线程模式逐 epoch 引脚一致（30 epoch 对照）；退出时 `thread_count()==0` ✅
- 不传插件参数时与旧路径逐字节等价 ✅（2000 万步输出逐行相同，`[plugins]` 行为空）
- 真实插件（LED / UART Terminal）在 CLI 输出中可见 ✅（`devices=2 epochs=5170 threads=2` + 状态 JSON；
  该 MicroPython 窗口未翻转 GP25，故 `transitions=0` 属预期）
- 限制：仅 `run` 接线；宿主时钟（`clock_hz=clk_peri`、`sys_clock_hz=clk_sys`）已按世代注入，
  改频后 ≤64 步延迟生效，见 `v1-scope.md` §4

### R4 执行清单

- [x] 3 OS CI 重新跑绿：CI #9（rustc/clippy 1.100）Windows / Linux / macOS 全部成功
- 注：Release 的 `aarch64-apple-darwin` 作业曾因 GitHub macOS 容量不足排队 15 分钟后被取消（非代码失败），重试发布。
- [x] `cargo test --all-features` 默认并行连跑 30 次无 flake（30/30 通过）；最终代码再补 10/10 通过
- [x] 最终二进制 soak：2 G 步 + 30 分钟采样（241 s，工作集恒定 42.6 MB、句柄 65；
      `hstx words=128386654 underruns=0`、`dma=52359254`）
- [x] `--core-threads on` 长跑 30 分钟（线程/句柄泄漏）：20 G 步 / 25 min，线程数 3、句柄 61 全程恒定，
      工作集 42.4 MB（末段回落 12.9 MB）；`hstx words=1430561194 underruns=0`、`dma=583488906`
- [ ] 静置机复测 300 M A/B（宿主负载波动 10–35 %）：默认 8.87 s / `--core-threads on` 4.79 s（1.85×，本机空闲窗口）

### R6 发布动作

- [ ] 打 `v1.0.0` 标签，触发 release workflow（4 个原生目标二进制 + 第一方插件；无 WASM/Pages）
- [ ] Release notes 使用 `v1-scope.md` 的范围声明 + `changelog.md` 的 `[1.0.0]` 条目

## 3. 固件验证清单（V1 门禁）

> **门禁规则**：对应内核必须先跑通下列固件，才允许进入发布候选。

| ID | 固件 / Firmware | 架构 | 验证命令 | 状态 |
|----|-----------------|------|----------|------|
| FV-1 | `...RPI_PICO2-RISCV-20260824-v1.29.0.uf2` | Hazard3 | `repl --exec "print(6 * 7)"` → `42`；`run --arch riscv --strict-decode` 无未定义指令 | ✅ 通过 |
| FV-2 | `...RPI_PICO2-20260824-v1.29.0.uf2` | Cortex-M33 | `repl --exec "print(6 * 7)"` → `42`；`perfbench` 至少一个基准 | ✅ 通过 |
| FV-3 | `...adafruit-circuitpython-raspberry_pi_pico2-zh_Latn_pinyin-10.3.1.uf2` | Cortex-M33 | `repl --no-soft-reset --exec "print(6 * 7)"` → `42` | ✅ 通过 |
| FV-4 | `...\emerald.uf2`（示例） | Cortex-M33 ×2 | `run --max-cycles 300000000 --trace-steps 0`：`dma=2199378`、`hstx words=5401204 underruns=0`、`core1 Running`、8 条 `[irq]` | ✅ 通过 |

## 4. 变更记录

见 [`changelog.md`](changelog.md)。`[1.0.0]` 已由原 `[Unreleased]` 收口。
