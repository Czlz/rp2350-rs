# 交接文档 / Handoff（2026-11-16）

下一会话请先读本文件、`.trae/rules/project.md`、`.opencode/skills/firmware-verification/SKILL.md`
与 `doc/threading-plan.md`，再开始派发任务。

## 1. 项目目标与定位

- **通用 RP2350 模拟器**（Rust），不是某个固件的运行时。MicroPython Arm/RISC-V、CircuitPython、
  pokeemerald 只是验收客户机。
- **优先级：精确时钟 > 精确指令 > 功能/插件稳定 > 宿主吞吐**。
- **不做 JIT**、不做块编译、不跳过客户机指令或周期。性能只优化宿主侧重复工作。
- 示例专用常量（帧缓冲地址、引脚、I2S 接线、固件路径）只能出现在 `example/`、CLI 参数、GUI
  输入或 profile 文件，**不得进 crate 默认值**。

## 2. 当前状态（全部已提交，工作树干净）

- 最新提交：`9b3d7c0`（docs 同步）；主干 `main`。
- **40 个测试目标全绿**：arm **542**、periph **483**、cli **179 + 2 ignored / 89**、core **122**、
  egui **141**、gdb **91**、riscv **85**、virt-periph **74**、audio **20**、usb 53、usb-host 33+2、
  bootrom 21。
- 门禁：`fmt --check` / `clippy --all-targets --all-features -D warnings -W pedantic` /
  `test --all-features` / `doc --no-deps` 全绿。
- FV-1/2/3（RISC-V MP / Arm MP / CircuitPython）均 `stdout: 42`。
- emerald 300 M（默认确定性模式）：
  `time_us=1798617`、`dma=2199378`、`hstx words=5401204 underruns=0`、`sleep core0 steps=224994790
  wakes=6`、`core1 Running pc=0x1000035C`、8 条 `[irq]`、final PC `0x1000B3AC`。
- 时钟：固件把 `clk_sys` 配到 **252 MHz**（启动 150 MHz）；1 s 客户机时间 = 252 M 周期，
  60 fps 一帧 = **4.2 M 周期**。
- 帧率现实：真实游戏段 ≈8.5 M 步/秒 ≈ **2 fps（默认）**；`--core-threads on` 实测
  3.55–6.53 fps（130 帧 A/B：off 41.7 s → on 20.9 s，**2.00×**）。
  音频在 2–6 fps 下必然欠载（I2S 13 440 帧/秒），GUI 会显示 `produced/s` vs `need/s`，不伪造声音。

## 3. 已验证的关键事实（不要重复踩坑）

1. **PIO 周期计时本来是对的**：I2S 实测 **13 441.7 字/秒**（配置 13 440）。「2.7k」是把 tap 的
   **i16 样本数**除以**墙钟秒**造成的口径错误（1 字 = 2 样本；墙钟 ≠ 客户机时间）。
2. **音频曾经真的没数据**：HSTX FIFO 满 → DREQ 去断言 → 旧 `enabled_selects_dreq` 仍返回 true →
   `pump_dma` 把整批预算置 0 → PIO2 TX0（I2S）DMA 饿死。已修（DREQ 必须当前断言）。
3. **CPU 仍是「1 指令 = 1 周期」近似**（`steps == cycles`）。这是目前最大的精度近似，尚未做
   每指令周期成本模型；改动会波及所有计时，需单独立项。
4. 睡眠语义（WFI/WFE/SEV、阻塞 FIFO）已实现；emerald 用 `WFE` 睡眠（启动 1.5 s），
   MicroPython 也用 WFI。**不要**再以「某固件不用」为由裁剪语义。
5. GDB 裸 `stepi` 现在可用（`vCont` 混合列表）；RISC-V 无 core1，线程模式自动退化串行。
6. `--core-threads on` 只固定 64 步窗口边界，窗口内访存顺序依赖宿主调度 → **非位可复现**；
   验收/回归必须用默认 `off`。
7. PPB 读回 `CFSR/HFSR/MMFAR/BFAR` 看不到核内故障路径置位的位；`core.system_read` 是权威。
8. 全仓测试有**进程级全局 Flash 世代号**：涉及 `FlashRegion::new/write/erase` 的测试已加
   `test_support::flash_generation_lock()`；新增此类测试必须同样加锁，否则并行 flake。
9. `play` 必须走 `run_steps` 批量（已改），否则 `--core-threads` 不生效。
10. 睡眠核快进（`--sleep-fast-forward`，默认 on）对 300 M 启动段 56–66×；关闭开关必须逐字节
    等同旧路径；RISC-V 不能快进（客户机可见 `mcycle`）。

## 4. 代码地图（关键文件）

| 领域 | 路径 |
|---|---|
| 总线/内存/MMIO/共享总线 | `crates/rp2350-core/src/{bus.rs,memory.rs,memory/**,mmio.rs,mmio/**,flash.rs,shared.rs,shared/**}` |
| 时钟树/时基 | `crates/rp2350-periph/src/{clock_tree.rs,clock_tree/**,clock_time.rs,timer_snapshot.rs}` |
| ARM 核/异常/睡眠 | `crates/rp2350-arm/src/{lib.rs,exceptions.rs,faults.rs,scb.rs,systick.rs,scr.rs,decode_cache.rs,fetch_cache.rs}` |
| RISC-V/PMP | `crates/rp2350-riscv/src/{lib.rs,pmp.rs,mem.rs,execute.rs,csr.rs}` |
| 外设 | `crates/rp2350-periph/src/{sio.rs,dma.rs,pio/**,hstx.rs,uart.rs,adc.rs,powman.rs,boot.rs}` |
| CLI/Play | `crates/rp2350-cli/src/{run.rs,run/{batch.rs,threads.rs,threads/**,fast_forward.rs,reset.rs,systick.rs},play.rs,play/**}` |
| GUI | `crates/rp2350-egui/src/{app.rs,play_view.rs,play_view/**,cli.rs}` |
| 插件 | `crates/rp2350-virt-periph/src/{manager.rs,executor*.rs,reload.rs,loader.rs}` |
| 音频 | `crates/rp2350-audio/src/**`、`crates/rp2350-periph/src/audio.rs` |
| 计划/实测 | `doc/{threading-plan.md,performance.md,playability-plan.md,implementation-plan.md}` |

## 5. 标准验收命令

```powershell
cargo fmt --all -- --check
$env:CARGO_INCREMENTAL=0; cargo clippy --all-targets --all-features -- -D warnings -W clippy::pedantic
cargo test --all-features
cargo doc --no-deps
cargo build --release -p rp2350-cli

# FV-1/2/3
target\release\rp2350-cli.exe repl --file "E:\Source\mcu\pico\python\micropython\RPI_PICO2-RISCV-20260824-v1.29.0.uf2" --exec "print(6 * 7)"
target\release\rp2350-cli.exe repl --file "E:\Source\mcu\pico\python\micropython\RPI_PICO2-20260824-v1.29.0.uf2" --exec "print(6 * 7)"
target\release\rp2350-cli.exe repl --file "E:\Source\mcu\pico\python\circuitpython\adafruit-circuitpython-raspberry_pi_pico2-zh_Latn_pinyin-10.3.1.uf2" --exec "print(6 * 7)" --no-soft-reset

# emerald 基线（默认确定性）
target\release\rp2350-cli.exe run --file "...\emerald.uf2" --max-cycles 300000000 --trace-steps 0

# 交互模式 A/B
target\release\rp2350-cli.exe play --profile example/gba/profile.conf --frames 130 --core-threads on
```

**注意**：验收期间不要 `Get-Process rp2350-cli | Stop-Process` 误杀长跑（本会话踩过）。
构建前如需覆盖 `target\release\rp2350-cli.exe`，先停掉占用进程或使用复制出的二进制。

## 6. 已完成的任务（本会话）

时钟树（252 MHz）+ 动态频率接入全部计时消费者；GAP-07 睡眠/阻塞 FIFO；SysTick/PendSV 异常号与
向量；SVCall/HardFault/NMI 与故障升级；`CCR` 陷阱与 FPU 对齐；PPB 故障/复位寄存器回放与
`SYSRESETREQ` 策略；RISC-V PMP/`mcountinhibit`/`mstatus`；PSRAM CLI 挂载；DMA `CALC`/`PERFCTR`；
插件加载器加固与执行器线程化；GDB `vCont` 混合列表；GUI PlayView（纹理/键盘/音频/复位/命令行
直启/线程开关）；睡眠核快进；TIMER 原子快照；双核线程可选模式；play 批量步进；PIO CLKDIV
相位/重启语义修正。

## 7. 下一会话任务清单（按优先级）

### T1 插件宿主接线（MTV-6）— 中优先
**问题**：CLI/GUI 都没有真正 `step` 插件（`rp2350-cli/src/devices.rs` 只做 scan/list/reload/wire），
所以线程化执行器只在库层可用。
**做**：在 `rp2350-cli` 增加 `--plugin-threads` 与一个真正的设备循环（或在 GUI 虚拟实验室提供
「运行」按钮），用 `DeviceManager::with_executor` + 固定 epoch 延迟驱动插件；把结果接到现有
`wiring()` 端口；加同步/线程化对照与长跑测试。
**验收**：真实 LED/uart-terminal 插件在 GUI/CLI 可见输出；`--plugin-threads` 与同步逐 pin 一致；
30 次并行无 flake；40 目标全绿。

### T2 CPU 每指令周期成本（精度，建议优先于更多性能）
**问题**：`steps == cycles`，所有计时按「1 指令 = 1 周期」。Cortex-M33 实际 1–3+ 周期
（load/store、分支、乘法等）。
**做**：先做**测量/设计**（不改默认）：统计 emerald/MicroPython 的指令类别分布与若按真实周期
记账后的时间差异；给出分阶段方案（例如先对 load/store 记 2 周期）。改动必须保留一个
「1 周期兼容模式」用于对照，避免一次性推翻所有基线。
**验收**：设计文档 + 合成基准 + 明确回归策略；**不要**直接改默认而不做对照。

### T3 闪存/状态存档（P3-2 剩余）
**问题**：GUI 复位已做，但存档/状态保存与 flash 路径未实现。
**做**：`--save-state/--load-state`（冷状态快照：SRAM/外设/时钟/核寄存器），或至少 flash 镜像
保存/加载；GUI 增加 Save/Load（无文件对话框时可配置路径）。
**约束**：状态格式必须带版本号；加载失败不得破坏当前会话。

### T4 长跑 soak 与性能复测（低风险）
- 2 G 步完整运行 + 30 分钟采样（工作集曾恒定 41–42.6 MB）。
- `--core-threads on` 长跑 30 分钟（检查线程/句柄泄漏）。
- 静置机复测 300 M A/B（本机宿主负载波动 10–35 %）。

### T5 可选：MCP TLS（GAP-10）
引入 rustls/tokio-rustls + 证书加载；默认关闭。若不打算做，在计划中保持「按需」。

### T6 文档遗留
- `doc/architecture.md:366` 仍写「写 `SMx_CLKDIV` 立即重启分频器」，与新语义（保留相位）不符。
- `crates/rp2350-cli` 的 `pump_pio_batch` 注释「每个交入周期就是一个 PIO 时钟」应改为
  「一个 `clk_sys` 周期」。
- `doc/implementation-plan.md` 的 GAP 表可补 MT-* 任务状态。

## 8. 经验与教训（本会话真实踩过）

1. **不要用「该固件不用」裁剪语义**（我曾把 WFI 划出路线，被用户纠正）。数据手册/ARM ARM
   定义了就必须做，示例固件只决定优先级。
2. **测量口径要写清**：帧率、字速率、样本数、墙钟/客户机时间混用会产生 5× 假象
   （PIO 13.4k 被误报 2.7k；play 12 fps 其实是启动睡眠段）。
3. **全局状态的测试必须隔离**（Flash 世代号 flake：30/30 才可信）。
4. **超时是调度上限还是卡死上限要分清**（插件 500 ms 超时在 16 线程负载下误杀健康 worker）。
5. **长跑进程不要被验收命令误杀**；构建会锁 `target\release\*.exe`，用复制二进制做 soak。
6. **默认确定性是验收基准**：任何「交互加速」必须可关闭，且关闭时逐字节等同旧路径
   （睡眠快进、双核线程都如此）。
7. 子会话经常按规范纠正提示词（NMI 层级、PMP M 模式、MMFAR 读写、PIO 口径）——**这是好事**，
   验收时以数据手册/ARM ARM 为准，不要强迫子会话服从错误提示。
8. 中文源文件不要用 PowerShell 整文件读写（会破坏 UTF-8）；用 `edit`/`write`。

## 9. 硬约束速查（违反即拒绝验收）

- 不做 JIT；不跳过指令/周期；不按固件哈希/地址/PC/循环形态特判。
- 示例常量只在 `example/`、CLI、GUI 输入、profile。
- 每任务一个全新子会话；禁止 follow-up 复用；子会话禁 git 写操作。
- 主会话只分析/拆分/派发/验收；功能代码由子会话实现。
- 每波 ≤2 个 crate 不相交任务；完成后主会话复跑门禁 + FV + emerald。
