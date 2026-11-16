# 交接文档 / Handoff — 目标：V1 发布

下一会话以 **发布 V1** 为唯一目标。先读本文件、[`lessons.md`](lessons.md)、
`.trae/rules/project.md`、`.opencode/skills/firmware-verification/SKILL.md`。

---

## 1. V1 定义（建议，作为发布口径）

**V1.0 = 通用、确定性、指令精确（周期近似）的 RP2350 模拟器**

**包含**
- Arm Cortex-M33 + RISC-V Hazard3，可启动并运行 MicroPython（Arm/RISC-V）、CircuitPython、
  pokeemerald 真实固件
- 时钟树与运行时超频、睡眠/阻塞、异常/故障、DMA/PIO/HSTX/UART/ADC/RTC/USB CDC/PSRAM
- CLI（`run`/`repl`/`play`/`gdb`/`mcp-serve`/`list-devices`/`wire`）、GUI（PlayView + 虚拟实验室）、
  插件加载/热重载/线程化执行器、GDB 多核
- **默认确定性模式**；可选双核线程（交互加速，非位可复现）
- 明确文档化的已知近似与限制

**不包含（必须写进 release notes）**
- CPU 流水线级周期精确（当前 `steps == cycles`，1 指令≈1 周期）
- TrustZone 安全态、MPU/MemManage/BusFault 源
- HSTX TMDS 10 位编码/DVI 位时序、USB 全速位级时序/CRC/位填充
- 真实 ROM 函数行为（分区/OTP/链式装载；当前为桩）
- 实时 60 fps 与连续音频（无 JIT；默认≈2 fps，线程模式 3.5–6.5 fps）
- MCP TLS

> 如果要把「周期成本模型」「TrustZone/MPU」「实时游玩」纳入 V1，必须重新评估周期与风险：
> 周期模型 +3–5 波，TrustZone/MPU +4–8 波，实时游玩在「不做 JIT」前提下不可达。

---

## 2. 当前状态（2026-11-16，全部已提交）

- 最新提交 `139b304`（V1 文档整理前）；工作树干净。
- **40 个测试目标、1999 个测试全绿**（arm 542、periph 483、cli 179+2 ignored / 89、core 122、
  egui 141、gdb 91、riscv 85、virt-periph 74、audio 20、usb 53、usb-host 33+2、bootrom 21）。
- 四道门禁全绿；FV-1/2/3 均 `stdout: 42`。
- emerald 300 M 基线（默认确定性）：`time_us=1798617`、`dma=2199378`、
  `hstx words=5401204 underruns=0`、`sleep core0 steps=224994790 wakes=6`、
  `core1 Running pc=0x1000035C`、8 条 `[irq]`、final PC `0x1000B3AC`。
- 时钟：固件 `clk_sys=252 MHz`（启动 150 MHz）；1 s 客户机时间 = 252 M 周期；
  60 fps 一帧 = 4.2 M 周期。
- 音频：I2S 实测 13 441.7 字/秒（配置 13 440）；低帧率下必然欠载，GUI 显示
  `produced/s` vs `need/s`，不伪造。

---

## 3. V1 发布清单（必须全部打勾）

### R1 发布卫生（1 波，主会话可直接做大部分）
- [ ] 补 `LICENSE`（MIT OR Apache-2.0；README 已引用）
- [ ] 重写 `README.md` 特性表与状态（当前多处与实现相反：双核/PIO FUNCSEL/PSRAM/GDB 多核等）
- [ ] `doc/README.md` 索引更新（删除已删文档的链接，加入 handoff/lessons/V1 plan）
- [ ] `CHANGELOG.md`：把 29 KB 的 Unreleased 整理为 `1.0.0`（按 Added/Changed/Fixed/Removed）
- [ ] 版本 `0.1.0` → `1.0.0`（`[workspace.package]`）
- [ ] 删除过时文档（已删 `reference-gap-analysis.md`）；清理 `implementation-plan.md`
      的旧 Milestone/Phase；`doc/architecture.md:366` 的 PIO CLKDIV 旧语义
- [ ] 过时「未建模」条目：crate 文档中已实现的项（SIO 自旋锁、WFI/WFE、时钟树、PSRAM、
      DMA `CALC`/`PERFCTR`、插件线程、PIO 相位等）从「未建模」移到「已建模」或删除

### R2 插件宿主接线（MTV-6，1–2 波）
- [ ] `rp2350-cli`：`--plugin-threads` + 真实设备循环（scan → step → wiring 端口），
      或 GUI 虚拟实验室「运行」按钮
- [ ] 用 `DeviceManager::with_executor` + 固定 epoch 延迟；同步/线程化逐 pin 对照
- [ ] 验收：真实 LED/uart-terminal 在 CLI/GUI 可见输出；30 次并行无 flake

### R3 V1 精度/限制声明（与 R1 合并，1 波）
- [ ] 新增 `doc/v1-scope.md`（或 README 一节）：包含/不包含、已知近似、确定性与线程模式差异、
      性能与音频限制、验证范围（Windows 本地 + CI 3 OS）
- [ ] 在 README、handoff、CLI `--help`/GUI 关于页链接该声明

### R4 稳定性与跨平台（1 波）
- [ ] 3 OS CI 重新跑绿（CI 最近未在本会话验证；工作流已存在）
- [ ] `cargo test --all-features` 默认并行连跑 30 次无 flake
- [ ] 最终二进制 soak：2 G 步 + 30 分钟采样（工作集曾恒定 41–42.6 MB）
- [ ] `--core-threads on` 长跑 30 分钟（线程/句柄泄漏）
- [ ] 静置机复测 300 M A/B（宿主负载波动 10–35 %）

### R5 存档/状态（建议纳入；如不纳入需在 R3 中明确排除）
- [ ] `--save-state/--load-state`（SRAM/外设/时钟/核寄存器冷快照，带版本号）或至少
      flash 镜像保存/加载
- [ ] GUI Save/Load（无文件对话框时可配置路径）；加载失败不破坏当前会话

### R6 发布动作（0.5 波）
- [ ] 打 `v1.0.0` 标签，触发 release workflow（多目标二进制 + WASM + Pages）
- [ ] Release notes 使用 R3 的范围声明 + CHANGELOG

**顺序**：R1 → R2 → R3 → R4 →（R5）→ R6。R1 与 R2 可并行（crate 不相交）。

---

## 4. 验收命令（每次提交/每波）

```powershell
cargo fmt --all -- --check
$env:CARGO_INCREMENTAL=0; cargo clippy --all-targets --all-features -- -D warnings -W clippy::pedantic
cargo test --all-features
cargo doc --no-deps
cargo build --release -p rp2350-cli

# FV-1/2/3
target\release\rp2350-cli.exe repl --file "...RPI_PICO2-RISCV-20260824-v1.29.0.uf2" --exec "print(6 * 7)"
target\release\rp2350-cli.exe repl --file "...RPI_PICO2-20260824-v1.29.0.uf2" --exec "print(6 * 7)"
target\release\rp2350-cli.exe repl --file "...adafruit-circuitpython-raspberry_pi_pico2-zh_Latn_pinyin-10.3.1.uf2" --exec "print(6 * 7)" --no-soft-reset

# emerald 基线 / 交互 A/B
target\release\rp2350-cli.exe run --file "...\emerald.uf2" --max-cycles 300000000 --trace-steps 0
target\release\rp2350-cli.exe play --profile example/gba/profile.conf --frames 130 --core-threads on
```

⚠️ 不要 `Stop-Process rp2350-cli` 误杀长跑；覆盖 `target\release\*.exe` 前先停占用进程。
⚠️ 涉及 `FlashRegion` 的新测试必须持 `test_support::flash_generation_lock()`。

---

## 5. 代码地图（V1 相关）

| 领域 | 路径 |
|---|---|
| 总线/内存/共享 | `crates/rp2350-core/src/{bus,memory/**,mmio/**,flash,shared/**}` |
| 时钟/时基 | `crates/rp2350-periph/src/{clock_tree**,clock_time,timer_snapshot}` |
| ARM/RISC-V | `crates/rp2350-arm/src/**`、`crates/rp2350-riscv/src/**` |
| 外设 | `crates/rp2350-periph/src/{sio,dma,pio/**,hstx,uart,adc,powman,boot}` |
| CLI/Play | `crates/rp2350-cli/src/{run,run/**,play,play/**}` |
| GUI | `crates/rp2350-egui/src/{app,play_view**,cli}` |
| 插件 | `crates/rp2350-virt-periph/src/{manager,executor*,reload,loader}` |
| 计划/证据 | `doc/{handoff,lessons,playability-plan,threading-plan,performance,architecture}.md` |

---

## 6. 硬约束（违反即拒绝验收）

- 通用 RP2350 模拟器；**精确时钟 > 精确指令 > 功能/插件稳定 > 吞吐**；**不做 JIT**；
  不跳过客户机指令/周期；不按固件哈希/地址/PC/循环形态特判。
- 示例常量只在 `example/`、CLI、GUI 输入、profile。
- 默认确定性是验收基准；交互加速必须可关闭且关闭时逐字节等同旧路径。
- 一个任务 = 一个全新子会话；禁止 follow-up 复用；子会话禁 git 写操作。
- 主会话只分析/拆分/派发/验收；每波 ≤2 个 crate 不相交任务；完成后复跑门禁 + FV + emerald。

---

## 7. 经验教训

见 [`lessons.md`](lessons.md)（语义优先、测量口径、全局状态测试、超时语义、长跑进程、
发布卫生、子会话工作流等 18 条）。

---

## 8. 已知限制（V1 release notes 必须包含）

1. CPU `steps == cycles`（1 指令≈1 周期），非流水线级精确。
2. 无 TrustZone/SAU、无 MPU/MemManage/BusFault 源。
3. HSTX 无 TMDS 10 位编码/DVI 位时序；USB 无位级时序/CRC/位填充。
4. bootrom 为合成/桩实现，ROM 函数行为不完整。
5. 默认 ≈2 fps；`--core-threads on` 3.5–6.5 fps 且窗口内非位可复现；音频因此常欠载。
6. MCP 仅 HTTP + Bearer，无 TLS。
7. PIO `CTRL/CLKDIV` 写入在下一个 64 步泵送边界生效；PPB 读回看不到核内故障位。
8. RISC-V 无 core1，线程模式自动退化串行；GDB 监视点在线程模式不支持。
