# 解释器热路径性能 / Interpreter hot-path performance

本文件记录 **P0-1 第一轮**、**P0-2 第二轮**、**P0-3a 第三轮**、**P0-3b 第四轮**
（`rp2350-cli` 批量步进）与 **P0-3c 第五轮**（`rp2350-arm` 译码结果缓存）的性能工作：
测量方法、基线、每项优化的收益、最终数字、剩余瓶颈与下一轮建议。目标与实际结果并列，
未达标处如实记录。
This document records the first (P0-1), second (P0-2), third (P0-3a), fourth
(P0-3b, batched stepping in `rp2350-cli`) and fifth (P0-3c decoded-instruction
cache in `rp2350-arm`) rounds of interpreter performance work: measurement
method, baseline, per-optimization gains, final numbers, remaining bottlenecks
and next-round suggestions. Targets and actual results are both listed, and any
shortfall is reported as measured.

关联文档 / Related: [`playability-plan.md`](playability-plan.md)（为什么需要 ≥50 M steps/s）、
[`crates/rp2350-core.md`](crates/rp2350-core.md)、[`crates/rp2350-cli.md`](crates/rp2350-cli.md)。

---

## 1. 测量方法 / Measurement method

```powershell
cargo build --release -p rp2350-cli
# 每次计时前先跑一遍预热（页缓存/文件缓存/分支预测器），随后取 3 次最小值与中位数。
target\release\rp2350-cli.exe run --file "<emerald.uf2>" --max-cycles 50000000 --trace-steps 0
target\release\rp2350-cli.exe run --file "<MicroPython PICO2.uf2>" --max-cycles 20000000 --trace-steps 0
```

- 固件 / Firmware：`E:\Source\mcu\pico\rp2350\gba\pokeemerald-rp2350\rp2350\hw\build_emerald\emerald.uf2`、
  `E:\Source\mcu\pico\python\micropython\RPI_PICO2-20260824-v1.29.0.uf2`。
- 计时口径是**进程墙钟**（`Stopwatch`），包含构建好的 release 二进制启动与固件加载。
  第三轮实测发现 emerald 这类 12 MB UF2 的装载曾占 **0.72~1.27 s**（见 7.2），
  修正了本文件早期「加载仅几十毫秒、可忽略」的判断。
- 该机器上还有并行会话编译同一 workspace，个别样本会偏慢；表中以 3~5 次中的**最小值**
  为主、中位数为辅，并保留全部样本。
- 诊断（`[run]`/`[tail]`/`[hstx]`/`[dma]`）内容在改动前后逐一比对，确认完全相同
  （同一步数下 final PC、DMA/HSTX 计数、core1 状态一致）。

微基准（仅在诊断阶段临时加入，交付前已删除）：在 `Machine::step` 内用 `rdtsc`
对 `core.step` 与各外设段打点，并用同一台机器上的 `b .` 自旋固件做对照。TSC
频率实测 **3.29 GHz**。

---

## 2. 基线 / Baseline（改动前）

| 场景 / Case | 步数 / Steps | 最小值 / min | 中位数 / median | 吞吐 / Throughput |
|---|---|---|---|---|
| emerald | 50 M | **14.21 s** | **14.87 s** | **3.52 / 3.36 M steps/s** |
| MicroPython (Arm) | 20 M | **5.12 s** | **5.41 s** | **3.91 / 3.70 M steps/s** |

（每个场景 3 次，预热后计时；表中给出最小与中位数。）
(Three runs per case after a warm-up; min and median are shown.)

主会话先前记录为 16.56 s（3.0 M steps/s），与本次 3 次中位数 14.87 s 同量级
（机器状态与并行负载不同）。

改动前的热路径成本分解（`rdtsc`，TSC 周期/步，emerald 10 M 步）：

| 区段 / Section | 周期/步 | 说明 |
|---|---|---|
| `core.step`（取指 + 译码 + 执行） | ~737 | 其中地址解析与 Flash 读锁占绝大部分 |
| `record_step` + `record_sync_trap` | ~55 | 环形缓冲 + `mcause` 读取 |
| DMA / PIO 泵送 | ~32 | 每 64 步一批，空闲探测 |
| 自旋检测 / PPB 回放 / IRQ / 时基 | ~76 | 每步检查，节流投递 |

进一步的逐调用计数（10 M 步）：

| 调用 / Call | 次数/步 | 周期/次 | 说明 |
|---|---|---|---|
| `SimpleBus::find_region_mut` | 2.9 | ~100 | 线性扫描 + 每个候选 2~3 次虚调用 |
| `FlashRegion::read_u8` | 2.74 | ~84 | 每次访问 2 次 `RwLock` 获取（`size()` + `read_store()`） |
| `FlashRegion::size()` | 7.2 | ~20 | 每次都加读锁 |
| `RamRegion::read_u8` / `write_u8` | 0.14 | ~46 | — |

取指路径（`find_region` + Flash 读锁）合计约 **520 周期/步**，是最大的一块可优化成本。

---

## 3. 逐项优化与收益 / Optimizations and gains

每项都以「其余不变、仅切换该项」的方式计时（emerald 10 M 步，3 次最小值）。

| # | 优化 / Optimization | 文件 / File | 前 / Before | 后 / After | 收益 / Gain |
|---|---|---|---|---|---|
| 1 | `SimpleBus` 区域桶缓存（`addr >> 24` 直接映射，256 桶） | `rp2350-core/src/memory.rs` | 3.462 s | 3.081 s | **−11 %** |
| 2 | `FlashRegion` 缓存长度 + 原生 `read_u16/u32`（一次共享存储获取） | `rp2350-core/src/flash.rs` | 3.081 s | 2.587 s | **−16 %** |
| 3 | 写监视门控（`AtomicBool`）+ `set_pc` 零开销 | `rp2350-core/src/watch.rs` | 2.587 s | 2.452 s | **−5 %** |
| 4 | `RamRegion` 原生 `read_u16/u32` + `write_u16/u32`（一次边界检查 + 一次拷贝） | `rp2350-core/src/memory.rs` | — | — | 数据密集负载收益（emerald 50 M 阶段以取指为主，单次 ±1 %，见下） |
| 5 | `MmioRouter` 4 KiB 页缓存（直接命中不再扫描子设备） | `rp2350-core/src/mmio.rs` | — | — | MMIO 轮询负载收益；emerald 50 M 阶段无 MMIO 流量，故本表未体现 |
| 6 | `deliver_irqs` 按位遍历（`trailing_zeros`） | `rp2350-cli/src/run.rs` | — | — | 每 64 步省一次 64 项扫描，语义与日志顺序不变 |

说明 / Notes：

- 第 4/5 项在 emerald 前 50 M 步（仍处于单核启动/自旋阶段，几乎不访问 SRAM 数据与
  MMIO）中无法体现；它们针对后续渲染与 MicroPython 解释器阶段，且不改变任何语义。
- 曾尝试给 `Machine::{step, step_cores, execute_core0_step, …}` 加 `#[inline]`，实测
  **无收益且略有回退**（代码膨胀 + 指令缓存压力），已回退。
- 曾尝试把 Flash 存储的 `std::sync::RwLock` 换成 `parking_lot::RwLock`，在噪声范围内
  无可测差异，已回退（保持 poison 语义与原实现一致）。
- 由于 `codegen-units = 1` + fat LTO，任意新增代码都会整体重排布局，单项收益存在
  ±5 % 的布局噪声；表中数字是多次复测后的稳定差值。

---

## 4. 最终数字 / Final numbers

| 场景 / Case | 步数 | 样本 (s) | 最小值 | 中位数 | 吞吐（最小值 / 中位数） |
|---|---|---|---|---|---|
| emerald | 50 M | 9.886 / 10.111 / 10.278 / 10.317 / 10.524 / 11.857 | **9.886 s** | 10.317 s | **5.06 / 4.85 M steps/s** |
| MicroPython (Arm) | 20 M | 3.768 / 3.874 / 3.932 | **3.768 s** | 3.874 s | **5.31 / 5.16 M steps/s** |

- emerald 50 M：**14.87 s → 9.89~10.32 s**，即 **1.50×（最小值）/ 1.44×（中位数）**；
  吞吐 **3.36 → 4.85~5.06 M steps/s**（+45~51 %）。
- MicroPython 20 M：**5.41 s → 3.87 s（1.40×）**，吞吐 **3.70 → 5.16 M steps/s（+39 %）**。
  该负载由 MicroPython 字节码解释器驱动，指令流密集、取指与 SRAM 数据访问各占相当
  比例，因此区域缓存与 Flash 取指优化的收益比 emerald 启动阶段更明显。
- 目标 **≤6.0 s（≥8.3 M steps/s）未达成**；差额与原因见下节。

功能回归（同一 release 二进制）：

| 门禁 / Gate | 结果 |
|---|---|
| FV-1（RISC-V MicroPython） | `[repl] banner` + `stdout: 42` ✅ |
| FV-2（Arm MicroPython） | `[repl] banner` + `stdout: 42` ✅ |
| FV-3（CircuitPython，`--no-soft-reset`） | `[repl] banner` + `stdout: 42` ✅ |
| emerald 300 M 步 | `hstx words=5401204 underruns=0 fifo=8 enabled=true wof=false unknown=0`、`core1 state=Running pc=0x10009D92 sp=0x2003D680 launches=1`、`dma=2195390 busy=0x0009`、无总线错误、预算耗尽正常停机（300 M 步，108.3 s） |
| `cargo test --all-features` | 全部通过（rp2350-arm 398、periph 347、cli 108+1 ignored、core 82 等） |
| `cargo fmt/clippy -D warnings -W clippy::pedantic/doc` | 全绿 |

`[run]`/`[tail]`/`[irq]`/`[watch]` 输出内容与触发条件均未变化；GDB 读写监视点
（`set_read_watch`/`take_read_hit`）行为不变。

---

## 5. 剩余瓶颈 / Remaining bottlenecks

emerald 50 M 步现在约 **9.9~10.3 s**，折算约 **650 TSC 周期/步**（3.29 GHz）。其中：

1. **ARM 解释器本身（`rp2350-arm`）≈ 430~465 TSC 周期/步（约 66~70 %）**。对照实验：同一台
   机器用 `b .` 自旋固件（一条 16 位指令、单次取指、无数据访问）经 `Machine::step`
   执行，**PRIMASK=1 时 96 ns/步（10.35 M steps/s）**，而 emerald 的循环体是多条
   Thumb-32（`ldr`/`str`/`mrs`/`cpsid`）加数据访问，**约 198 ns/步（5.06 M steps/s）**。
   即：即使把总线开销全部消掉，本阶段的物理上限约为 10 M steps/s，8.3 M steps/s 也
   只剩很小余量，而这部分成本在 `crates/rp2350-arm/src/{lib.rs,execute_thumb16.rs,
   execute_thumb32.rs}` 内，超出本轮文件白名单。
2. **`Nvic::find_highest_priority` 的 256 项线性扫描（`rp2350-arm/src/nvic.rs`）**。
   `CortexM33Core::check_pending_exceptions` 每步调用一次；`b .` 自旋固件在
   **PRIMASK=0 时 270~309 ns/步（3.2~3.7 M steps/s）**、PRIMASK=1 时 96~100 ns/步，
   即该扫描每步约 **560~680 TSC 周期**（2.8× 差距，实测同一固件同一循环）。
   emerald 的启动自旋循环内部恰好执行 `CPSID i`（`0x1000B238` 处 `0xB672`），因此本轮
   50 M 步基准基本不受影响；但**一旦中断打开（core1 启动后的渲染阶段、MicroPython
   REPL 等待键盘时），这一步就会成为首要成本**。300 M 步运行只有 2.77 M steps/s
   （中断已打开、双核 + HSTX/DMA 流量同时存在），与该扫描的代价方向一致，但要精确
   归因需在 `rp2350-arm` 内再加一次打点。改成「已使能 ∧ 已挂起」位掩码 +
   `trailing_zeros` 即可，约 10 行，但属于 `rp2350-arm`。
3. **Flash 取指仍每次获取一次共享存储读锁**（`RwLock` 对/解锁约 40~50 周期）。若改为
   「世代号 + 行缓存」，取指可再省约 8 %；但这会把写-读之间的强一致降级为
   「宿主单线程编程」假设，需要单独评估。
4. **`BootPeripherals::take_system_writes()` 每步一次 `Mutex` 加解锁**（实测约 15~20
   周期/步，占 ~3 %）。它在 `rp2350-periph`，且可用一个 `AtomicBool`「日志非空」门控
   去掉，同样超出本轮白名单。

### 下一轮建议 / Next round

按收益与文件归属排序：

| 优先级 | 改动 | 归属 | 预期 |
|---|---|---|---|
| P0 | `Nvic` 用 `enabled & pending & !active` 掩码 + `trailing_zeros` 取代 256 项扫描 | `rp2350-arm` | 中断打开时 **2~2.8×**；emerald 300 M 步 108 s → ~50 s |
| P0 | Thumb-32 译码热分支收敛（把常见 `ldr/str/b` 提到前段，冷指令走 `#[cold]` 函数） | `rp2350-arm` | 20~40 % |
| P1 | Flash 世代号 + 行缓存（去掉逐次读锁） | `rp2350-core` | ~8 % |
| P1 | `take_system_writes` 原子门控 | `rp2350-periph` | ~3 % |
| P2 | 双核分线程（core0/core1 各占一线程） | `rp2350-cli` | ~1.3~1.5×（core1 渲染阶段） |

若要达到 20 fps 游玩（≥50 M steps/s），在 P0 两项之后仍需 JIT 或帧级并行；
详见 [`playability-plan.md`](playability-plan.md) 的风险节。

---

## 6. 第二轮 / Round 2（P0-2）

第一轮把 emerald 50 M 步做到 9.7~10.3 s，并指出「ARM 解释器占 66~70 %」是主要
瓶颈。第二轮**先重新测量**，结论与第一轮的推测不同：在 50 M 步区间里真正最大
的一块是 **NVIC 的 256 项线性扫描**，而不是译码。
Round 1 brought emerald's 50 M steps to 9.7–10.3 s and blamed the ARM interpreter
(66–70 %). Round 2 re-measured first, and the result differs from that guess: the
largest single block over the 50 M-step window is the **NVIC 256-entry linear
scan**, not instruction decode.

### 6.1 重新分桶 / Re-measured profile

临时探针（交付前已删除）：`rdtsc` + 1/64 采样，桶边界为
`Machine::step → core.step → fetch → execute → post → check_pending_exceptions →
record_sync_trap → DMA → PIO → after_running_step → core1`。
Temporary probe (removed before delivery): `rdtsc` with 1-in-64 sampling.

改动前（emerald 50 M，TSC 周期/步，含约 15 周期/桶的探针开销）：

| 区段 / Section | 周期/步 | 说明 |
|---|---|---|
| `cli.enter`（预算检查、`pc()`、`record_step`、boot ROM 桩扫描） | 102 | 其中 `record_step` ≈ 37、桩表线性扫描占多数 |
| `arm.fetch`（`set_pc` + SysTick + 取指） | 81 | 每次取指一次 Flash 读锁 |
| `arm.exec`（译码 + 执行） | 62 | 热循环为 14 条 Thumb-16/32 |
| `arm.sched`（`check_pending_exceptions` + `total_cycles`） | **246** | **最大单项**：27.1 M/50 M 步（54 %）PRIMASK=0，每次调用都扫 256 项 |
| `record_sync_trap` / DMA / PIO / `after_running_step` / core1 | 35~41 各 | 多为固定调用开销；`after_running_step` 含每步一次 `Mutex` |
| 合计 | 711 | 与实测墙钟（650 周期/步）同量级 |

计数器证实了归因：`irq_checks=27131318`（PRIMASK=0 的步数）、
`irq_found=1`（整个 50 M 步只真正投递过一次中断）。也就是说**每步都在为一次
几乎不会发生的事件付出 400+ 周期**。

### 6.2 逐项 A/B / Per-item A/B

每项都以「其余不变、只切换该项」的方式测 3~5 次取最小值（emerald 50 M 步）。

| # | 优化 / Optimization | 文件 / File | 前 / Before | 后 / After | 收益 / Gain |
|---|---|---|---|---|---|
| 1 | `Nvic::find_highest_priority`：8×`u32` 候选位图 + 8 个优先级桶 + `trailing_zeros` | `rp2350-arm/src/nvic.rs` | 10.139 s | 5.771 s | **−43 %（1.76×）** |
| 2 | Flash 取指缓存（512 条直接映射 + 内容世代号失效） | `rp2350-arm/src/fetch_cache.rs`、`rp2350-core/src/flash.rs` | 5.771 s | 5.442 s | −6 %（探针桶 87→39 周期/步） |
| 3 | `record_step` 改为定长 32 条环形缓冲（去掉 `VecDeque`） | `rp2350-cli/src/run.rs` | 3.264 s¹ | 3.173 s¹ | −2.8 % |
| 4 | boot ROM 桩查询先用 ROM 窗口（`pc − ROM_BASE < ROM_SIZE`）过滤 | `rp2350-cli/src/run.rs` | 4.099 s¹ | 3.264 s¹ | **−20 %** |
| 5 | PPB 写世代门控 `take_system_writes`（每步一次的 `Mutex` 变一次比较） | `rp2350-core/src/memory.rs`、`rp2350-cli/src/run.rs` | 3.430 s¹ | 3.264 s¹ | **−5 %** |
| 6 | `run` 循环仅在 `notices_pending()` 时取走通知（空通知不再每步搬一次 `Vec`） | `rp2350-cli/src/run.rs` | 3.736 s¹ | 3.212 s¹ | **−14 %** |

¹ 第 3~6 项处于同一批改动：表中的数字都是「其余已启用、只关掉该项」的对照，
因此基线各不相同（第 4/5/6 项的前值分别对应关掉该项后的 4.099 / 3.430 / 3.736 s）。

Notes / 说明：

- 第 1 项收益远超第一轮预估（当时预计 2~2.8×，实测消掉了整段 43 %），因为
  emerald 启动循环里 `CPSID i`/`CPSIE i` 成对出现，50 M 步内有 54 % 的步
  PRIMASK=0。位图选择器仍保持「同优先级取 IRQ 号最小」与
  `PRIMASK`/`BASEPRI` 语义不变，并由伪随机矩阵与线性参考实现对表。
- 第 2 项的一致性论证：客户机写 Flash 被拒绝，因此 Flash 不可能被自修改；所有
  宿主侧变更（`FlashRegion::new/load_bytes`、`MemoryRegion::load_u8`、
  `FlashHandle::write/erase`）都递增 `flash::content_generation`，失配时整表清空。
  SRAM 完全不入缓存，自修改 SRAM 代码语义不变。缓存按区域半字读记同样的两个
  周期，`total_cycles` 不漂移（有单测对照「开/关缓存」逐周期相同）。
  缓存**默认关闭**，仅由 `Machine::new` 在确认 XIP 窗口由只读 `FlashRegion`
  支撑后打开。
- 第 4 项：合成 boot ROM 的桩表是 `Vec` 线性扫描且每步都执行；所有桩都位于
  `0x0000_0000..0x0000_8000`，先用窗口比较过滤等价于原语义（若跳过真实陷阱，
  PC 会立刻跑飞——实测被跳过的实验在 116 653 步即总线错误，证明该门控必须先于
  扫描）。
- 第 5 项：`take_system_writes` 本体在 `rp2350-periph`（本轮白名单外），因此改为
  在总线侧统计 PPB 写世代：PPB 写是回放队列的**唯一**来源，世代未变即队列必空。
  语义等价（同一步内的 PPB 写仍在下一条指令前回放）。
- 曾尝试把 `Nvic` 候选位图再压缩成「挂起摘要」单 bit，收益在噪声内，未采用。
- Thumb-16/32 译码本轮**未改**：重新测量后 `arm.exec` 只占 62/711 周期/步
  （≈9 %），把常见编码提前的理论收益 <3 %，不值得增加译码风险。

### 6.3 最终数字 / Final numbers

3 次预热后取 5 次最小值（同一 release 二进制，验收口径）：

| 场景 / Case | 步数 | 样本 (s) | 最小值 | 中位数 | 吞吐（最小值） |
|---|---|---|---|---|---|
| emerald | 50 M | 3.212 / 3.311 / 3.387 / 3.423 / 3.529（跨多轮；并行负载时可见 3.7 s） | **3.21 s** | 3.43 s | **15.6 M steps/s** |
| MicroPython (Arm) | 20 M | 1.205 / 1.210 / 1.268 / 1.455 | **1.21 s** | 1.21 s | **16.5 M steps/s** |
| emerald | 300 M | 22.29 / 23.35 | **22.29 s** | — | **13.5 M steps/s** |

- emerald 50 M：**9.72 s → 3.21~3.31 s（2.9~3.0×）**，吞吐 **5.14 → 15.6 M steps/s**。
  目标 **≤6.0 s 达成**，拉伸目标 **≤4 s 也达成**（余量约 0.7 s）。
- MicroPython 20 M：**3.77 s → 1.21 s（3.1×）**，吞吐 **5.31 → 16.6 M steps/s**。
- emerald 300 M：**108.3 s → 22.3~23.4 s（4.6~4.9×）**；中断与双核真正活跃后，
  第一轮点出的 NVIC 扫描代价被彻底移除。
- 与 20 fps 游玩目标（≥50 M steps/s）仍差 **3.2×**：剩余成本已经平摊到每步的
  取指/译码/执行/调度骨架，见下节。

### 6.4 不变量核对 / Invariant checks

| 检查 / Check | 结果 |
|---|---|
| emerald 50 M 全部 stdout（`[run]`/`[tail]`/`[regs]`/`[dma]`/`[hstx]`/`[irq]`/`[uart]`，49 行） | 与改动前**逐字节相同**（`Compare-Object` 无差异） |
| emerald 300 M `executed 300000000 steps`、`final PC=0x1000B3A6` | ✅ |
| `dma=2195390 busy=0x0009` | ✅ |
| `hstx words=5401204 underruns=0 fifo=8 enabled=true wof=false unknown=0` | ✅ |
| `core1 state=Running pc=0x10009D92 sp=0x2003D680 launches=1` | ✅ |
| 总线错误 | 无 |
| `RP2350_WATCH` / `RP2350_WATCHV` | 输出格式与条数不变（写钩子调用点未动；`SimpleBus` 的写覆写仍只在 `write_u32` 调一次 `check_write`，逐字节回退路径不变） |
| GDB 读/写监视、单步、断点 | `cargo test` 全绿（rp2350-gdb 78 项 + rp2350-cli 108 项） |
| FV-1 / FV-2 / FV-3 | 均 `[repl] banner` + `stdout: 42` ✅ |
| `cargo test --all-features` | 全部通过（36 个测试目标；rp2350-arm 411、core 82、cli 108+1 ignored、periph 347…） |
| `cargo fmt --check` / `clippy -D warnings -W clippy::pedantic` / `cargo doc --no-deps` | 全绿 |

新增测试 13 项：`Nvic` 7 项（伪随机矩阵对表线性参考、256 IRQ 全开、跨组同优先级、
IPR 字节写移动优先级桶、禁用/未挂起/活跃跳过、复位优先级为 0）、Flash 取指缓存
4 项（窗口/别名/非 Flash 地址、宿主改写失效、擦除失效、世代计数）、核心级 2 项
（宿主改写后新指令立即可见、开/关缓存逐周期与状态一致）。

### 6.5 剩余瓶颈 / Remaining bottlenecks

**约 650 → 211 TSC 周期/步**（3.29 GHz；50 M 步 / 3.212 s = 64.2 ns/步）。

最终分桶（探针单位，绝对值大致为「每桶 +15 周期的探针开销」，因此只看比例）：

| 区段 | 占比 | 真实成分 |
|---|---|---|
| `cli.enter`（预算检查、`pc()`、`record_step`、桩查询） | ~18 % | 每步一次 `run` 循环迭代、两次存储、虚调用 |
| 取指（`set_pc` + SysTick + 缓存/总线） | ~10 % | 缓存命中 + 一次原子读 + SysTick 检查 |
| 译码 + 执行 | ~11 % | 14 条指令/循环，含 3~4 次数据访问（dyn 总线 + 桶表） |
| `check_pending_exceptions` | ~9 % | 位图候选集 + 优先级桶（PRIMASK 短路仍在） |
| `record_sync_trap` / DMA / PIO / `after_running_step` / core1 | 各 ~8~9 % | 固定调用 + 节流比较（绝大多数为空转） |

折算掉探针开销后，**解释器本体（取指 + 译码 + 执行）约占 20~25 %**，其余 75~80 %
是「与指令无关的每步骨架」：循环迭代、`record_step`、`check_pending_exceptions`、
两次泵送节流、PPB 回放门控、core1 探测、通知检查。这也解释了为什么单靠译码优化
在第二轮几乎拿不到收益。

下一轮若继续做，收益顺序为：

| 优先级 | 改动 | 归属 | 预期 |
|---|---|---|---|
| P0 | 批量步进：`run` 循环内联 `step`/「N 步一批」，把每步骨架摊薄（逐指令 watch/GDB 语义需分支守卫） | `rp2350-cli` | 10~20 % |
| P1 | DMA/PIO/core1 泵送改为「事件计数 + 无事件跳过」，彻底去掉空转调用 | `rp2350-cli` + `rp2350-periph` | 3~5 % |
| P1 | 数据访问路径桶表内联（`AddressBus` 泛型化或专用 SRAM 快路径） | `rp2350-core` | 3~5 % |
| P2 | Thumb-16 常见编码查表（`arm.exec` 仅 ~11 %，上限有限） | `rp2350-arm` | <3 % |
| P2 | 双核分线程 | `rp2350-cli` | 1.3~1.5×（core1 渲染阶段） |

要真正进入 **≥50 M steps/s** 区间（20 fps 游玩），仍需 JIT 或帧级并行，
解释器微优化已接近上限；详见 [`playability-plan.md`](playability-plan.md)。

---

## 7. 第三轮 / Round 3（P0-3a：`rp2350-arm` + `rp2350-core`）

第二轮把 emerald 50 M 步做到 3.21~3.39 s，并把剩余成本归因为「每步骨架」
（取指/译码/调度/前端循环）。第三轮**先重新分桶**，结论与任务假设不同：
`check_pending_exceptions` 在第二轮位图选择器之后已经只要 **约 4~5 周期/步**，
真正的隐藏大头是**固件加载（启动）**，而不是每步解释器开销。
Round 2 brought emerald's 50 M steps to 3.21–3.39 s and blamed the per-step
skeleton. Round 3 re-measured first, and the task's premise was wrong:
`check_pending_exceptions` already costs only **~4–5 cycles/step** after round
2's bitmap selector. The real hidden cost was **firmware loading (startup)**.

### 7.1 重新分桶 / Re-measured profile

临时探针（交付前已删除）：`rdtsc` + 1/64 采样，并用一对空 `rdtsc` 标定探针自身
开销（实测 **33 周期**）。第一批数字没有扣除该开销，因此把每个桶都放大了约
30 周期——这正是第二轮「调度 40 周期」的来源。
Temporary probe (removed before delivery): `rdtsc` with 1-in-64 sampling plus an
empty-span calibration for the probe itself (**33 cycles**). The first numbers
did not subtract it, so every bucket was inflated by ~30 cycles — which is
exactly where round 2's "40 cycles for scheduling" came from.

标定后（emerald 50 M，TSC 周期/步；`ARM-only` 一列是同机微基准，直接驱动
`CortexM33Core::step`，不含 cli/外设）：

| 区段 / Section | ARM 侧 | 整机 / full machine | 说明 |
|---|---|---|---|
| 取指（缓存命中 + 世代号） | 3~5 | 3~5 | 取指缓存 10 M 步内 **miss=0** |
| 译码 + 执行（含数据访问） | 10~35 | 10~35 | 噪声较大；热循环 24 条指令/迭代 |
| `check_pending_exceptions` | **4~5** | 4~5 | PRIMASK=0 时也只要一次位图构造 |
| 单步其余部分 | ~1 | ~1 | usage-fault / undefined 检查 |
| cli 骨架（`pc()`/`record_step`/泵送节流/通知） | — | ~20 ns/步 | `rp2350-cli`，本轮不可改 |
| **合计** | **~27~30 ns/步** | **~50 ns/步** | 见 7.3 |

计数器（emerald 前 20 M 步）：`PRIMASK=0` 仅占 **3.3 %**（第二轮窗口是 54 %），
整段只投递 1 次中断；也就是说本轮基准里异常扫描本来就不是热点。
Counters over the first 20 M steps: `PRIMASK=0` only **3.3 %** of steps (round
2's window had 54 %) with one delivery, so the exception scan was never the hot
spot in this benchmark.

`Machine`/`run` 每步固定做的工作（预算检查、`pc()`、`record_step`、boot-ROM
窗口比较、`core.step`、`record_sync_trap`、DMA/PIO 节流比较、自旋检测、PPB
世代比较）合计约 **20 ns/步**，并且全部在 `rp2350-cli`（本轮白名单外）。

### 7.2 逐项 A/B / Per-item A/B

所有 50 M 数字都是**同一时段交错运行、取 3 次最小值**（并行会话编译会带来
±15 % 抖动）。

| # | 优化 / Optimization | 文件 / File | 前 / Before | 后 / After | 收益 / Gain |
|---|---|---|---|---|---|
| 1 | 异常门控：`Nvic::has_runnable`（`ISER & ISPR` 的保守缓存，全部写入口维护）+ `check_pending_exceptions` 短路 | `rp2350-arm/src/{nvic.rs,lib.rs}` | 27.24 ns/步¹ | 26.06 ns/步¹ | 上限 **−1.2 ns/步**（端到端 <0.2 %，见下） |
| 2 | UF2 重复块检查：`Vec::contains`（O(n²)，4.8 万块）→ `HashSet` | `rp2350-core/src/uf2.rs` | 3.711 s | 3.032 s | **−0.68 s（−18 %）** |
| 3 | UF2 整块宿主写入：逐字节 `load_u8`（每字节一次写锁 + 一次世代递增）→ `AddressBus::load_bytes`（一次路由/锁/世代） | `rp2350-core/src/{bus.rs,memory.rs,flash.rs,uf2.rs}` | 2.634 s | 2.502 s | −0.13 s（−5 %） |
| 4 | ELF 段加载与 `.bss` 零填充同样走 `load_bytes`（4 KiB 分块） | `rp2350-core/src/firmware.rs` | — | — | 与 #3 同源；emerald 走 UF2，故本表未体现 |

¹ 第 1 项只能在 ARM-only 微基准上测出：把 `check_pending_exceptions` **整体短路**
（收益上限）也只省 1.2 ns/步。端到端 50 M 的差异低于噪声，因此如实记为「语义精确、
零风险，但收益极小」。任务给出的「每步 40 周期」是未标定探针的读数。

启动开销（`run --max-cycles 1000`，含进程启动 + 读 24 MB UF2 + 解析 + 装载）：

| 版本 / Build | 样本 (s) | 最小值 |
|---|---|---|
| 原状（线性查重 + 逐字节装载） | 1.268 / 0.749 / 0.762 / 0.717 / 0.820 | **0.72 s** |
| 仅哈希查重（逐字节装载） | 0.184 / 0.166 / 0.155 / 0.149 | **0.149 s** |
| 本轮最终（哈希 + 批量装载） | 0.088 / 0.050 / 0.046 / 0.045 | **0.045 s** |

即启动从 **0.72~1.27 s 降到 0.045 s**：12 MB 映像的 4.8 万块线性查重是
O(n²)（十亿级比较），逐字节装载则要 1230 万次总线往返（每次一次写锁）。
That is, startup drops from **0.72–1.27 s to 0.045 s**: the 48 k-block linear
duplicate scan is O(n²) (billions of comparisons) and the byte-wise load costs
12.3 M bus round trips, each taking the flash write lock.

一致性论证 / Exactness：

- `load_bytes` 的 trait 默认实现逐字节转发 `load_u8`；`SimpleBus` 只把整段路由
  一次，`FlashRegion`/`RamRegion` 才做批量拷贝。越界跨度回退逐字节，因此错误类型
  与「越界前字节已落下」的行为不变；写监视安装期间 `RamRegion` 强制逐字节
  （诊断逐字节上报）；`FlashRegion` 仍拒绝客户机写，宿主侧每次连续写入只递增一次
  内容世代（判定条件仍是「变了就失效」，取指缓存语义不变）；PPB 写世代在跨度可能
  与 PPB 窗口相交时仍逐字节推进。
- 哈希查重与线性查重判定完全相同（同一 `(family, block_number)` 键），错误报文
  也相同；`cargo test` 的 `rejects_duplicate_block_numbers_within_family` /
  `allows_duplicate_numbers_across_families` 覆盖。
- `RP2350_WATCH=20000000-20082000` 下，改动前后 230 316 条 `[watch]` 输出逐字节
  相同（用只差 UF2 装载路径的两个二进制对照）。

### 7.3 最终数字 / Final numbers

同一 release 二进制，预热后 8 次取最小值（并列全部样本）：

| 场景 / Case | 步数 | 样本 (s) | 最小值 | 中位数 | 吞吐（最小值） |
|---|---|---|---|---|---|
| emerald | 50 M | 2.640 / 2.441 / 2.440 / 2.453 / 2.442 / 2.414 / 2.548 / 2.632 | **2.414 s** | 2.442 s | **20.7 M steps/s** |
| emerald | 300 M | 20.52 | **20.52 s** | — | 14.6 M steps/s |

（同一二进制更早一轮 5 次样本为 2.512 / 2.420 / 2.573 / 2.629 / 2.652，最小值
2.420 s；并行负载下会看到 2.6 s 以上。）

- emerald 50 M：**3.50 s → 2.414 s（1.45×）**（本轮开头同机同口径基线
  3.502 / 3.527 / 3.501 s；第二轮记录的最好值 3.212 s）。目标 **≤2.6 s 达成**，
  最小值 2.414 s、中位数 2.442 s，余量约 0.16 s。
- emerald 300 M：**22.29 s → 20.52 s（1.09×）**。300 M 只付一次启动，因此收益主要
  来自启动省下的 0.7~1.2 s；运行期每步成本本轮基本未变（ARM 侧只多了异常门控），
  其余差异在并行负载噪声内。
- ARM-only 微基准（同机、emerald 自旋循环 2000 万步、直接调 `CortexM33Core::step`）：
  ~27~30 ns/步 → 整机 ~50 ns/步，即 **ARM 解释器约占 55 %，cli + 外设骨架约 45 %**。

### 7.4 不变量核对 / Invariant checks

| 检查 / Check | 结果 |
|---|---|
| emerald 50 M 全部 stdout（49 行） | 与改动前**逐字节相同**（`Compare-Object` 无差异） |
| emerald 300 M `executed 300000000 steps`、`final PC=0x1000B3A6` | ✅ |
| `dma=2195390 busy=0x0009` | ✅ |
| `hstx words=5401204 underruns=0 fifo=8 enabled=true wof=false unknown=0` | ✅ |
| `core1 state=Running pc=0x10009D92 sp=0x2003D680 launches=1` | ✅ |
| 8 条 `[irq]` | ✅ |
| 总线错误 | 无 |
| `RP2350_WATCH` / `RP2350_WATCHV` | 输出逐字节不变（见 7.2） |
| FV-1 / FV-2 / FV-3 | 均 `[repl] banner` + `stdout: 42` ✅ |
| `cargo test --all-features` | 全绿（38 个测试目标；rp2350-arm 419、core 89、cli 73、periph 347…） |
| `cargo fmt --check` / `clippy -D warnings -W clippy::pedantic` / `cargo doc --no-deps` | 全绿 |

新增测试 15 项：`Nvic` 3 项（门控写入口全覆盖、活跃时保守、与线性参考对表）、
核心级 5 项（未使能不投递、PRIMASK 屏蔽、ICPR 关闸、SysTick 置挂起、混合序列
逐步一致性）、`rp2350-core` 7 项（总线批量与逐字节等价、越界同错、只读拒绝、
写监视回退；Flash 批量与逐字节同字节/同周期；总线经宿主通道写 Flash；
ELF `load_into` 与逐字节参考逐字节一致）。

### 7.5 剩余瓶颈 / Remaining bottlenecks

启动已从 **0.72~1.27 s 压到 0.045 s**，50 M 的墙钟现在几乎全部是运行期：

| 组成 | 每步 | 占比 | 归属 |
|---|---|---|---|
| cli 骨架（`pc()`、`record_step`、boot-ROM 窗口、`core.step` 分派、`record_sync_trap`、DMA/PIO 节流、自旋检测、PPB 世代） | ~20 ns | ~40 % | `rp2350-cli` |
| ARM 解释器（取指 + 译码 + 执行 + 数据访问） | ~27~30 ns | ~55 % | `rp2350-arm` + `rp2350-core` |
| 外设/MMIO（定时器读、HSTX/DMA 批次） | 少数 ns | ~5 % | `rp2350-periph` |

本轮未做、并说明理由：

- **SRAM 取指缓存**：emerald 基准全程从 Flash 取指，实测取指缓存 **miss=0**；
  SRAM 入缓存还必须在每次存储时失效（自修改代码），收益不可测而风险显著，故不做。
- **数据访问快路径**（去掉第二次 `dyn` 调用）：热循环约 0.35 次数据访问/步，
  每次最多省 5~8 周期，端到端 <1 %，与改动风险不成比例。
- **译码优化**：分桶显示 `dec+exec` 波动 10~35 周期/步，噪声很大；第二轮结论
  （收益 <3 %）仍然成立。

### 7.6 下一轮建议 / Next round

| 优先级 | 改动 | 归属 | 预期 |
|---|---|---|---|
| P0 | `run` 循环把 N 步合成一次 `step`（逐指令 watch/GDB 语义用分支守卫），把 ~20 ns/步的 cli 骨架摊薄 | `rp2350-cli` | 20~30 % |
| P0 | MMIO 读路径（定时器轮询每步约 0.1 次、每次数百周期）：设备内联 + 只读寄存器快照 | `rp2350-periph` | 5~10 % |
| P1 | 启动路径其余分配（4.8 万个 `Vec` 载荷）改为一次性 arena | `rp2350-core` | 启动再省 ~0.02 s（已很小） |
| P2 | Thumb-32 热分支收敛（`fpu::try_execute`/`coproc::try_execute` 前置判断） | `rp2350-arm` | <3 % |

若要达到 20 fps 游玩（≥50 M steps/s），在 cli 批量步进之后仍需 JIT 或帧级并行；
详见 [`playability-plan.md`](playability-plan.md) 的风险节。

---

## 8. 第四轮 / Round 4（P0-3b：`rp2350-cli`）

第三轮把 50 M 步做到 2.41 s，并把 ~40 % 的时间归给「cli 骨架」（`pc()`、
`record_step`、泵送节流、自旋检测、PPB 世代比较、core1 探测）。本轮**只改
`rp2350-cli`**：先重新分桶确认骨架的真实构成，再用批量步进把它摊薄，顺带修复
`play` 的两处体验缺陷（按键每帧采样的输入延迟、逐字节整屏拷贝）。
Round 3 brought 50 M steps to 2.41 s and attributed ~40 % of the time to the
"cli skeleton". Round 4 only touches `rp2350-cli`: it re-profiles that skeleton,
amortizes it with batched stepping, and fixes two `play` experience defects
(per-frame button sampling and a byte-wise full-screen copy).

### 8.1 重新分桶 / Re-measured profile

临时探针（交付前已删除）：`rdtsc` 每次采样只包住**一个**区段（每 64 步采样一次），
另有一次空跨度采样标定探针自身开销。空跨度实测 **35~37 周期**，因此单段读数
= 真实值 + 35 左右；短于此的区段无法区分。
Temporary probe (removed before delivery): one `rdtsc` pair around a **single**
section per run, sampled every 64 steps, plus an empty-span calibration.
The empty span measured **35–37 cycles**, so each section reading is the real
value + ~35 and anything shorter is below the resolution.

改动前（emerald 20 M 步，`run` 路径；周期/步，TSC 3.29 GHz）：

| 区段 / Section | 探针读数 | 扣除标定 | 说明 |
|---|---|---|---|
| 空跨度（标定） | 35.3 / 37.2 | 0 | 探针本身的开销 |
| `core.step`（取指+译码+执行+数据访问） | 106.2 / 121.1 | **~71 / ~84** | 两次独立采样；占整步的 40~45 % |
| `pc()` + `record_step` | 37.9 | **< 1** | 单段短于探针分辨率 |
| `record_sync_trap` | 35.7 | **< 1** | 同上 |
| DMA/PIO 泵送（节流后） | 36.2 | **< 1** | 同上 |
| `after_running_step`（自旋+PPB+IRQ 节流+时基） | 36.3 | **< 1** | 同上 |
| `step_core1`（core1 未运行时） | 37.8 | **< 1** | 同上 |

单段都低于探针分辨率，但它们的**合计**可以从两条路径的差看出来：

- 把上述六段按 `step()` 的顺序写进一个扁平循环（`prof` 探针循环）：
  **148.6 周期/步**；
- 真实的 `run` 前端循环（`machine.step()?` + `notices_pending()` + 外层
  `halt_reason()`/`budget_exhausted()` 检查）：50 M 步 2.83~2.86 s =
  **186 周期/步**。

即：约 **35 周期/步**花在 `Machine::step` → `step_cores_with_trace` →
`execute_core0_step` → `after_running_step` 的调用层与外层循环上，其余约
**65 周期/步**是那五段自身的合计。这解释了为什么「每步骨架 ~20 ns」难以靠单点
优化拿掉：它由五六个 5~15 周期的短函数调用组成。
Each section alone is shorter than the probe resolution, but their sum is
visible by comparing two whole loops: the same six pieces written as one flat
loop cost **148.6 cycles/step**, while the real `run` front-end loop
(`step()?` + `notices_pending()` + the outer halt/budget checks) costs
**186 cycles/step**. That is ~35 cycles/step in call layers plus ~65 cycles/step
across the five short sections.

### 8.2 逐项 A/B / Per-item A/B

同一 release 二进制、同一时段交错运行 3 次取最小值（emerald 50 M 步）。
三项都通过临时环境变量开关（交付前已删除）。

| # | 优化 / Optimization | 文件 / File | 前 / Before | 后 / After | 收益 / Gain |
|---|---|---|---|---|---|
| 1 | 批量步进 `Machine::run_steps`（扁平循环 + 步边界提升） | `run/batch.rs` | 2.464 s¹ | 1.802 s¹ | **−27 %** |
| 2 | core1 门控（仅在 `Running` 或 PSM 复位 / FIFO 启动挂起时调用 `step_core1`） | `run/batch.rs` | 2.025 s | 1.802 s | **−11 %** |
| 3 | Arm 跳过 `record_sync_trap`（`csr_read(0x342)` 恒为 0） | `run/batch.rs` | 1.828 s | 1.802 s | **−1.4 %**（噪声内） |
| 4 | PC 缓存（下一步的 PC = 上一步结束时的 PC，省一次 `core.pc()`） | — | 1.791 s | 1.802 s | **无收益，已回退** |

¹ 第 1 项是整项开关：前值 = 用 `machine.step()` 逐指令跑同一个 `run`（同一二进制、
同一时段），后值 = 用 `run_steps` 一次跑完。前后差 **0.66 s ≈ 43 周期/步**，与
8.1 测出的调用层 35 周期 + 五个短段的一部分吻合。

说明 / Notes：

- 第 1 项的语义约束是**逐事件步号不变**：DMA/PIO 批次仍每 64 步一次、中断投递
  仍每 64 步一次（`service_core1_peer` + `deliver_irqs`）、时基仍每 10 000 步一次，
  且 core1 的步进被挪到「边界工作之后」以保持原顺序（`service_core1_peer` 能在
  同一步挂起 core1 启动，原路径就是在同一步消费它）。这一条曾在第一版实现里弄错
  （core1 启动晚一步，300 M 步最终 PC 偏移 6 条指令），由逐步对照工具定位后修正，
  现已在 300 M 步上逐字节一致（见 8.4）。
- 仍然逐指令进行的工作：预算/停机检查（步号精确）、`RecentSteps` 环形缓冲
  （`[tail]`/故障回溯）、PPB 回放世代比较（CPACR 必须在下一条指令前生效）、
  自旋检测（`[run] tight spin` 的步号与文本不变）、RISC-V 的 `mcause` 探测。
  `record_step` **没有**加门控：`run` 总会打印 `[tail]`，故障回溯也依赖它，加
  `AtomicBool` 只会多一次原子读，故保持无锁环形缓冲。
- 第 2 项收益最大的一块是 `step_core1` 的 `SIO select_core`/状态查询调用层；
  未运行时它没有任何副作用（复位与启动都各自有 relaxed 守卫）。
- 第 4 项实测无收益（1.791 vs 1.802 在噪声内），按「只做实测有效」原则回退。

### 8.3 `play` 体验修复 / Play fixes

| # | 缺陷 / Defect | 修复 / Fix | 效果 / Effect |
|---|---|---|---|
| a | 按键**每帧**采样一次：8 fps 下输入延迟 ~120 ms，帧内快速点按被吞掉 | 每 **4096 客户机步**采样一次（与停止标志同一节拍），一帧按 4096 步切片执行 | 延迟从 ~120 ms 降到 ~0.14 ms 客户机时间 |
| b | `read_block` **逐字节**读整屏：76 800 次总线往返 | 对齐主体用 `AddressBus::read_u32`（每次 4 字节），不对齐的头/短尾/失败宽读回退字节读 | 每帧 ~19 200 次总线读；`play` 8.26 → **11.7~12.5 fps** |

`read_block` 的等价性：宽读失败时从该字起退回逐字节读，因此返回前缀与逐字节遍历
完全一致（含跨越 SRAM 末尾的情形）；有单测覆盖 4 种对齐 × 11 种长度与越界前缀。

### 8.4 最终数字 / Final numbers

同一 release 二进制，预热后 5 次（并列出全部样本）：

| 场景 / Case | 步数 | 样本 (s) | 最小值 | 中位数 | 吞吐（最小值 / 中位数） |
|---|---|---|---|---|---|
| emerald | 50 M | 1.879 / 1.689 / 1.704 / 1.723 / 1.655 | **1.655 s** | 1.704 s | **30.2 / 29.3 M steps/s** |
| emerald | 300 M | 15.80 | **15.80 s** | — | **19.0 M steps/s** |
| `play --frames 20` | 50 M | 两轮：11.74 / 12.46 fps | — | — | **29.4 / 31.2 M steps/s** |

- emerald 50 M：**2.83~2.86 s（本轮开头同机同口径）→ 1.655 s（最小值）**，
  **−41 %**，吞吐 **17.7 → 30.2 M steps/s**。目标 **≤1.8 s（≥28 M steps/s）
  达成**，余量约 8 %；中位数 1.704 s 也在目标内。
- emerald 300 M：**20.52 s（第三轮）→ 15.80 s**，吞吐 14.6 → 19.0 M steps/s。
  300 M 区间的 cli 骨架占比本来就比 50 M 小（中断、双核与 DMA 都在跑），因此
  收益（−23 %）小于 50 M 的 −41 %。
- `play`：**8.26 → 11.7~12.5 fps**（+42~51 %），同时修掉 120 ms 输入延迟。

### 8.5 不变量核对 / Invariant checks

| 检查 / Check | 结果 |
|---|---|
| emerald 50 M 全部 stdout（49 行） | 与改动前**逐字节相同**（`Compare-Object` 无差异） |
| emerald 300 M 全部 stdout | 与「逐指令 `step()`」构建**逐字节相同**（`Compare-Object` 无差异） |
| 300 M `final PC=0x1000B3A6`、`dma=2195390 busy=0x0009`、`hstx words=5401204 underruns=0 fifo=8 enabled=true wof=false unknown=0`、`core1 state=Running pc=0x10009D92 sp=0x2003D680 launches=1`、8 条 `[irq]` | 全部 ✅（逐字段相同） |
| 总线错误 | 无 |
| `RP2350_WATCH=20000000-20082000`（50 M 步） | stdout 与 stderr（237 MB、约 970 万条 `[watch]`）与逐指令构建 **SHA-256 相同** |
| 逐步对照（同一进程内两台机器，每 64 步比一次 PC/core1 PC/DMA/HSTX/节流游标/IRQ 条数） | 300 M 步**无分歧** |
| `--trace-steps` / `--dump` / `gdb` / `repl` | 输出格式不变；GDB 与 `--trace-steps` 仍走逐指令 `step`/`trace_step` |
| FV-1 / FV-2 / FV-3 | 均 `[repl] banner` + `stdout: 42` ✅ |
| `cargo test --all-features` | 全绿（40 个测试目标） |
| `cargo fmt --check` / `clippy -D warnings -W clippy::pedantic` / `cargo doc --no-deps` | 全绿 |

新增测试 10 项：批量步进与逐指令逐步一致（计数器循环、双核都在跑）、批中途中断
（PIO `IRQ set`）投递相同、DMA 空闲后能再次唤醒、预算精确停在预算步、故障回溯
（`[tail]` 内容相同、含故障取指共 3 条）、`read_block` 批量 vs 逐字节（4 种对齐 ×
11 种长度、越界前缀、全未映射）、按键在帧首生效、按键在帧内被重新采样。

### 8.6 剩余瓶颈 / Remaining bottlenecks

50 M 步现在 **1.655~1.70 s ≈ 109~112 周期/步**：

| 组成 | 每步 | 占比 | 归属 |
|---|---|---|---|
| ARM 解释器（`core.step`：取指 + 译码 + 执行 + 数据访问） | **~71~84 周期** | **~65~75 %** | `rp2350-arm` + `rp2350-core` |
| cli 骨架（`pc()`、`record_step`/环形缓冲、ROM 窗口、PPB 世代、自旋比较、循环与边界摊薄） | ~28~40 周期 | ~25~35 % | `rp2350-cli`（本轮） |

- 本轮把 cli 骨架从 ~100 周期/步（占 55 %）压到 ~30 周期/步（占 25~35 %），
  剩余部分已经是「每条指令必须做」的最小集合：一次 PC 读、一次环形缓冲写、
  一次 PPB 世代原子读、一次自旋窗口比较。
- 再要提升只能动解释器本体或改架构：`core.step` 的 `dyn AddressBus` 数据访问、
  Thumb-32 热分支、以及双核分线程。cli 侧继续优化的理论上限约 **−15 %**
  （1.66 s → ~1.45 s），达不到 20 fps（≥50 M steps/s）所需的 3.4× 差距。

### 8.7 下一轮建议 / Next round

| 优先级 | 改动 | 归属 | 预期 |
|---|---|---|---|
| P0 | `AddressBus` 去虚化（`CortexM33Core::step` 泛型化或 SRAM/Flash 专用快路径） | `rp2350-arm` + `rp2350-core` | 10~20 % |
| P0 | MMIO 读路径（定时器轮询 + 只读寄存器快照） | `rp2350-periph` | 5~10 % |
| P1 | 双核分线程（core0/core1 各占一线程，300 M 区间） | `rp2350-cli` | 1.3~1.5×（渲染阶段） |
| P2 | `record_step` 仅在需要时写入（`play`/`repl` 无 `[tail]` 读者时可关） | `rp2350-cli` | <2 % |

要进入 ≥50 M steps/s（20 fps 游玩）仍需 JIT 或帧级并行，详见
[`playability-plan.md`](playability-plan.md)。

---

## 9. 第五轮 / Round 5（P0-3c：`rp2350-arm` 译码结果缓存）

第四轮把 50 M 步做到 1.66~1.70 s（~30 M steps/s），并把 **65~75 %** 的时间归给
`core.step`（取指 + 译码 + 执行）。本轮**只改 `rp2350-arm`**：先用 PC 直方图确认
「同一小段指令被反复取指/译码」，再把每条指令的**长度、原始指令字、分派标签与
操作数字段**放进一张按 `PC` 索引的直接映射表，命中时一次查表取代两次取指、宽度判定
与操作码类别链。
Round 4 brought emerald's 50 M steps to 1.66–1.70 s (~30 M steps/s) and attributed
**65–75 %** of the time to `core.step`. Round 5 touches only `rp2350-arm`: a PC
histogram first confirms that a tiny instruction window is re-fetched and
re-decoded millions of times, then a direct-mapped table keyed by `PC` stores the
instruction length, the raw word, a dispatch tag and the operand fields, so a hit
replaces two fetches, the width decision and the opcode-class chain with one tag
compare.

### 9.1 热点测量 / Hot-spot measurement

临时探针（交付前已删除）：在 `step_instruction` 里按 `(pc >> 1) & 0xFFFF` 累加
计数并记录代表 `PC`，5 M 步后输出 top-N、覆盖区间与类别直方图。
Temporary probe (removed before delivery): counts per `(pc >> 1) & 0xFFFF` bucket
plus a representative `PC`, dumped after 5 M steps.

emerald 前 5 M 步（**只有 1530 个不同 PC**）：

| 排名 | PC | 步数占比 | 指令 |
|---|---|---|---|
| 1–24 | `0x1000B222..0x1000B262`（每 2 字节一个） | 各 **4.079 %**，合计 **97.9 %** | `sleep_ms` 的 WFE 自旋循环，24 条指令 |

覆盖区间 / Coverage：

| top-N 桶 | 覆盖步数 |
|---|---|
| 4 | 16.3 % |
| 8 | 32.6 % |
| **16** | **65.3 %** |
| **32** | **99.91 %** |
| 64 | 99.95 % |
| 1024 | 99.99 % |

即：整个窗口是**一段 0x42 字节（24 条指令）的循环**（`ldr`/`cmp`/`bhi`/`bcs`/`bne`
采样 SIO 与 TIMERAWL，然后 `cpsid`/`mrs`/`ldaexb`/`strexb`/`dmb`/`stlb`/`msr`/`wfe`），
被反复执行 5 M / 24 ≈ 208 000 次。**1024 条表项（2 KiB 代码无冲突）远大于所需**，
但保留余量给真实负载中更分散的代码。

类别直方图（5 M 步）/ Class histogram：16 位 `01101`（LDR 立即数）16.3 %、
`11010|11011`（B&lt;cond&gt;）21.0 %、`01000`（数据处理寄存器）12.9 %、
`11100`（B）4.1 %、`10110`（CPS）4.1 %、`10111`（hint/WFE）4.1 %；
32 位 `0xE8xx`（`ldaexb`/`strexb`/`stlb`）12.2 %、`0xF3xx`（`mrs`/`dmb`/`msr`）
12.2 %、`0xF1xx`/`0xF0xx`（`cmp.w`/`movs.w`）12.2 %。

### 9.2 缓存设计 / Cache design

`crates/rp2350-arm/src/decode_cache.rs`：

```rust
pub struct Entry {            // 16 bytes
    pub tag: u32,             // PC the entry was filled from (0 = empty)
    pub word: u32,            // raw instruction word (hw1 in bits 31:16)
    pub aux: u32,             // branch target or literal address
    pub kind: Kind,           // dispatch tag (u8)
    pub group: u8,            // Thumb-32 group id, or the condition code
    pub len: u8,              // 2 or 4
}
```

- **索引**：`(pc >> 1) & 1023`，1024 条 × 16 B = 16 KiB（与 4 KiB 的取指缓存
  组成 L1/L2 两级：命中走译码缓存，未命中落回取指缓存再到总线）。
- **命中**：`tag == pc` 即命中（条目只能由 `store` 建立，而 `store` 拒绝窗口外地址，
  因此标签相等已隐含「来自 Flash」，不需要窗口判定与有效位）。
- **分派标签**：`B16`/`BCond16`/`LdrLiteral16`/`LdrWordImm5`/`StrWordImm5`/
  `DpRegister16`/`Hint16`/`Cps16`/`Group32`/`Slow16`/`Slow32`。`Group32` 存的是
  `execute_thumb32` 本来会在 FPU/协处理器探测与 `op1`/`op2` 判定之后选择的**同一个
  整数组**，热路径直接调用该组函数（`execute_group_tag`）。
- **字段预提取**：分支目标 `pc + 4 + offset` 与 `LDR (literal)` 地址在**填充时**
  算好（条目按 `PC` 索引，因此恒定），命中时不再做符号扩展。
- **执行体唯一**：`execute_thumb16::fast` 里的热点分支（`LDR`/`STR` 立即数、
  数据处理寄存器、`B`、`B<cc>`、hint、`CPS`、文字池）由**译码器与快路径共用**，
  因此两条路径不可能分叉；`Group32` 直接调用既有的组函数。缓存只保存**如何分派**，
  不保存指令**做什么**。
- **周期精确**：命中按取指路径记同样的周期（16 位 3 个、32 位 6 个），并保留原有的
  逐分支记账——**已执行的分支不额外记周期**（通用分支在共享尾部之前就返回）。
  这一点在差分测试里被发现并修正（第一版多记了 1 个周期，`total_cycles` 立刻偏离）。
- **语义边界**：`IT` 块内的指令一律走完整译码器（槽位消费与
  `setflags = !InITBlock()` 抑制不动）；FPU/协处理器编码（`0xEC..0xEE`、`0xFE`
  前缀）保留在慢路径，惰性浮点压栈（`FPCAR`/`LSPACT`/`CONTROL.FPCA`）与 NOCP
  用法故障逻辑完全不变。

**失效**：Flash 侧沿用 `flash::content_generation`（失配即整表清空）；
**SRAM 完全不缓存**（见 9.6 的理由与代价分析），因此自修改 `SRAM` 代码由构造保证
正确。

### 9.3 逐项 A/B / Per-item A/B

本轮所有数字都是**同一 release 二进制、同一时段交错运行**，脚本用环境变量切换译码
缓存开关（探针与开关交付前已删除）。50 M 步最小/中位数：

| # | 优化 / Optimization | 文件 | 前 / Before | 后 / After | 收益 |
|---|---|---|---|---|---|
| 1 | 融合取指：一条指令一次查表（取代 1.4 次半字取指查找） | `decode_cache.rs`、`lib.rs` | 1.85 s¹ | 1.66 s¹ | −10 % |
| 2 | 分派标签 + 整数组函数指针（跳过 FPU/协处理器探测与 `op1`/`op2` 链） | `decode_cache.rs`、`execute_thumb32.rs` | 1.66 s¹ | 1.59 s¹ | −4 % |
| 3 | 热点 16 位分支与字段预提取（`B`/`B<cc>`/`LDR`/`STR`/DP-reg/hint/CPS） | `execute_thumb16/fast.rs` | 1.59 s¹ | 1.56 s¹ | −2 % |
| 4 | 命中路径去掉重复的窗口判定与有效位（标签相等已隐含有效性） | `decode_cache.rs` | 1.60 s | 1.56 s | −2.5 % |
| — | **合计** | | **1.85 s** | **1.56 s** | **−16 %** |

¹ 第 1~3 项是同一批改动里的三个可分别关闭的部分，前后值来自逐步回退的快速测量
（±3 % 抖动），只用于判断「哪一项值得保留」；第 4 项是独立的开关对照。**合计**一行
才是同二进制、同口径下的可信数字（关闭全部译码缓存 vs 全开）。

微基准（同机、直接驱动 `CortexM33Core::step`，emerald 自旋循环 10 M 步，取 3 次
最小值；单位 TSC 周期，3.29 GHz；在本轮中段测得，之后把命中路径拆成独立模块只带来
噪声级变化）：

| 配置 / Configuration | 周期/步 | 说明 |
|---|---|---|
| 无任何缓存 | 115~118 | 每次取指两次半字总线读 |
| 仅 Flash 取指缓存 | 83~86 | 32 位指令取两次 |
| 取指 + 译码缓存 | **52.0~52.2** | 一条指令一次查表 |

即 `core.step` 本体 **−37 %**；端到端只有 −16 %，因为 `rp2350-cli` 骨架（~30 周期/步）
与本轮无关。

### 9.4 最终数字 / Final numbers

| 场景 / Case | 步数 | 样本 (s) | 最小值 | 吞吐（最小值） |
|---|---|---|---|---|
| emerald | 50 M | 1.735 / 1.470 / 1.502 / 1.508 / 1.531 / 1.552 / 1.559 / 1.573 / 1.608 / 1.634 / 1.684 | **1.470 s** | **34.0 M steps/s** |
| emerald | 300 M | 16.30 | **16.30 s** | **18.4 M steps/s** |
| `play --frames 20` | 50 M | 13.1~13.6 fps（32.8~33.9 M steps/s） | — | **13.6 fps** |

- emerald 50 M：**1.66~1.75 s → 1.47~1.61 s（−7~11 % vs 第四轮记录的最好值）**，同二进制
  开关对照（同一时段交错的另一组样本）为 **1.85 → 1.56 s（−16 %）**，吞吐
  **27.0 → 32.0~34.0 M steps/s**。两组数字的差异来自并行会话的负载抖动
  （该机器实测 ±10 %），A/B 只应看同一组内的前后差。
- emerald 300 M：同二进制开关对照 **19.0 → 16.3 s（−14 %）**；中断打开、代码足迹
  更大后命中率下降，收益比 50 M 小。
- `play`：**12.4 → 13.6 fps（+10 %）**。
- **目标 ≤1.2 s（≥42 M steps/s）未达成**：实测 1.47 s / 34.0 M steps/s，差 **1.24×**。
  原因见 9.6。

### 9.5 不变量核对 / Invariant checks

| 检查 / Check | 结果 |
|---|---|
| emerald 6 M 步全部 stdout（与改动前同一命令逐字节比较） | **无差异**（`Compare-Object` 空） |
| emerald 300 M `final PC=0x1000B3A6`、`dma=2195390 busy=0x0009`、`hstx words=5401204 underruns=0 fifo=8 enabled=true wof=false unknown=0`、`core1 state=Running pc=0x10009D92 sp=0x2003D680 launches=1` | 全部 ✅（逐字段相同） |
| 8 条 `[irq]`、无总线错误 | ✅ |
| `RP2350_WATCH` / `RP2350_WATCHV` | 写钩子调用点未动；`SimpleBus::write_u32` 仍只调一次 `check_write`，逐字节回退路径不变 |
| `--strict-decode` / `undefined` 行为 | 差分测试覆盖（strict 与宽容两种模式，未定义指令结果与 `undefined_seen` 去重相同） |
| IT 块、`watch`、`trace`、GDB 单步/断点 | 块内指令仍走完整译码器；四者都用同一条 `step`，只是被加速 |
| FV-1 / FV-2 / FV-3 | 均 `[repl] banner` + `stdout: 42` ✅ |
| `cargo test --all-features` | 全绿（rp2350-arm 438、core 90、periph 347…；40 个测试目标） |
| `cargo fmt --check` / `clippy -D warnings -W clippy::pedantic` / `cargo doc --no-deps` | 全绿 |

新增测试 **17 项**：

- `decode_cache.rs` 7 项：窗口内可缓存 / `SRAM` 不可缓存、字段往返、条件分支预计算
  （`cond == 0xF` 不进快路径）、Thumb-32 组号与解码树一致（含 FPU/协处理器前缀与
  未定义编码保持慢路径）、宿主 Flash 写触发清空、同索引冲突替换、标签字节往返。
- `tests/decode_cache.rs` 10 项（**差分**：同程序跑两台核心，逐步比较
  `pc`/寄存器/`xpsr`/`primask`/`faultmask`/`control`/`total_cycles`/独占监视器/
  `FPSCR` 与总线内容）：
  ① 全部特化的 Thumb-16 编码（16 种 DP-reg 运算、`LDR`/`STR`/`LDRB`/`STRB`/
  `LDRH`/`STRH`、`MRS`/`MSR`、`CPS`/`WFE`/`YIELD`/`NOP`、`PUSH`/`POP`）；
  ② 分支与 IT 块（已执行/未执行、IT 内标志抑制）；③ Thumb-32 各组
  （`movw`、`movs.w`、`cmp.w`、`ldr.w`/`str.w`、`ldm.w`、`ldaexb`/`strexb`、`mrs`/
  `msr`、`dmb`、`lsl.w`、`and.w`、`mul.w`）；④ 未定义指令（strict 与宽容）；
  ⑤ 宿主 Flash 改写立即生效；⑥ `SRAM` 代码永不缓存、自修改立即生效；
  ⑦ 非对齐 PC 取指；⑧ 32 位指令跨 Flash 窗口末尾；⑨ 伪随机 120 条指令流（600 步）
  逐步等价；⑩ 热循环填充后持续命中。

### 9.6 剩余瓶颈 / Remaining bottlenecks

50 M 步现在 **1.47~1.61 s ≈ 97~106 周期/步**（TSC 3.29 GHz）：

| 组成 | 每步 | 占比 | 归属 |
|---|---|---|---|
| ARM 解释器（`core.step`：一次查表 + 执行 + 数据访问） | **~70 周期** | ~68 % | `rp2350-arm` |
| cli 骨架（`pc()`、`record_step`、PPB 世代、自旋/泵送节流、循环摊薄） | ~30 周期 | ~29 % | `rp2350-cli`（第四轮已定稿） |
| 外设/MMIO | 少数 | ~3 % | `rp2350-periph` |

译码缓存已经把「取指 + 译码 + 字段提取」从 ~30 周期压到 ~10；剩下的 ~40 周期/步是
**每条指令都必须做的固定工作**：`watch::set_pc`、`systick.tick`、一次世代号原子读、
`Kind` 匹配、条件求值、`take_usage_fault`、`check_pending_exceptions` 的短路、
`total_cycles` 累加，以及三次函数调用层。实测这些项**逐个都不显著**（关掉 `watch`/
`systick`/`check_pending_exceptions` 在整机上都是噪声内），但合起来就是解释器的地板。

热循环里另外 0.29 次/步的数据访问（含 2 次 `TIMERAWL` MMIO 读与独占访问）经
`dyn AddressBus` → 桶表 → 设备虚调用，每次 ~25~40 周期，合计 ~10 周期/步，属于
`rp2350-core`/`rp2350-periph`。

本轮**未做**：

- **SRAM 译码缓存**：实测负载都不在 `SRAM` 执行（emerald 全程 Flash XIP）。要做到
  精确失效需在热路径上查询按 16 字节行的世代表（`0x2000_0000..0x2008_2000` 需
  ~33 k 项），每步多 4~6 周期，而它只在 SRAM 代码上才有收益；`RamRegion` 若改成
  「每次写都递增全局世代」则会被自旋锁的 `stlb` 每帧清空几十次。故只缓存 Flash
  窗口，`SRAM` 由构造保持精确（见 `doc/crates/rp2350-core.md`）。
- **MMIO 读路径**（定时器轮询每次数百周期）：在 `rp2350-periph`，本轮白名单外。
- **cli 骨架再摊薄**：`rp2350-cli`，第四轮已到最小必需集合。

### 9.7 JIT 可行性与路线 / JIT feasibility

当前 **32~34 M steps/s（1.47~1.56 s / 50 M）**，`play` 13.1~13.6 fps（每帧 2.5 M 步）。

| 目标 | 需要吞吐 | 相对当前 | 每步预算 |
|---|---|---|---|
| 30 fps（每帧 2.5 M 步） | 75 M steps/s | **2.2~2.3×** | 44 周期/步 |
| 60 fps（每帧 2.5 M 步） | 150 M steps/s | **4.4~4.7×** | 22 周期/步 |
| 20 fps（旧目标，≥50 M/s） | 50 M steps/s | **1.5~1.6×** | 66 周期/步 |

判断：

1. **1.5~1.6×（20 fps）仍然只靠解释器就可能达到**：把上面列的固定成本（`watch`
   门控、`Kind` 匹配、条件求值、`take_usage_fault`、`check_pending_exceptions`
   短路）合并成一次「快照比较」，并给 `movs`/`adds`/`subs`/`push`/`pop`/`ldrb` 等
   常见 16 位编码补上快路径（emerald 热循环已全覆盖，但 MicroPython 字节码解释器
   未覆盖），估计还能拿到 15~25 %（1.47 → 1.15~1.25 s）。这正好是**本轮目标**
   （≤1.2 s）的量级，仍有 1.2~1.3× 的不确定性。
2. **2.2~2.3×（30 fps）需要「块级」执行**：把 N 条指令的取指/译码一次性完成，并在
   块内把 `pc`/`steps` 的更新折叠成一次（**必须保持逐步可观测**：`watch`、GDB、
   `--trace-steps`、`[tail]` 需要按步回放，只能在这些调试开关关闭时才允许折叠）。
   预计 1.8~2.2×，风险在于中断/时基粒度必须保持逐条不变。
3. **4.4~4.7×（60 fps）需要 JIT**：把 Thumb 译成宿主机器码的**基本块**，guest 寄存器
   常驻宿主寄存器，块尾用「同一份 `check_pending_exceptions` + `cycles` 记账」收尾。
   可行性的关键前提在本模拟器里已经具备：
   - **Flash 只读且带世代号**：`content_generation` 足以使译好的块失效，因此可以
     做「按地址缓存 + 世代校验」的持久代码缓存，不需要处理自修改代码（自修改只可能
     发生在 `SRAM`，那条路径可以继续解释执行）。
   - **指令语义已用差分测试固定**：快路径与完整译码器逐步等价，JIT 可以复用同一批
     测试作为「解释 vs JIT」的对照。
   - 主要工作量在**异常入口/出口**（stacks frame、EXC_RETURN、惰性浮点）与
     **MMIO/独占访问**的边界：这些必须强制回解释器。
   预计 4~8×（块级 JIT 的常见区间），是唯一能达到 60 fps 的路线；帧级并行（双核
   分线程）只能拿到 1.3~1.5×。

---

## 9. 真实游戏段吞吐（GAP-07 睡眠语义之后，主会话实测）

GAP-07 落地后 emerald 的启动延时（`sleep_ms(1500)`）真正进入 `WFE` 睡眠：**睡眠步不退休
指令，但计入步数预算并推进时基**，因此同一步数对应的客户机时间不变。各阶段实测：

| 区间 / Window | 步数 | 墙钟 | 吞吐 | 说明 |
|---|---|---|---|---|
| 0–50 M | 50 M | **0.84 s** | 59 M/s | 全部为启动睡眠（`sleep core0 steps=49.9 M`） |
| MicroPython 20 M | 20 M | **0.34–0.37 s** | 54–60 M/s | 启动阶段大量 `WFI` |
| 0–300 M | 300 M | 14.3 s | 21 M/s | 含 225 M 睡眠步 |
| **300–500 M** | 200 M | 25.1 s | **8.0 M/s** | **真实游戏段（无睡眠）** |
| **500–800 M** | 300 M | 35.1 s | **8.5 M/s** | 真实游戏段 |
| 0–1.2 G | 1200 M | 119.5 s | 10 M/s | 含启动 + 游戏 |

**结论（重要更正）**：此前记录的「`play` 12.4/12.9 fps」是在**启动阶段**（前 50 M 步，
几乎全是睡眠/自旋）测得的，不代表游戏画面帧率。真实游戏段约 **8.0–8.5 M steps/s**，
按每游戏帧 ≈2.5 M 步计算是 **≈3.3 fps**。

- 60 fps 需要 ≥150 M steps/s → 约 **18×**；30 fps 需要 ≥75 M → 约 **9×**。
- 解释器微优化（16 位快路径、块级执行）最多再给 1.5–2×，**无法达到 30/60 fps**；
  **必须 JIT**（或帧级并行 + JIT 组合）。
- 睡眠语义本身是**正确性收益**：MicroPython 启动从 16.5 M/s 提升到 54–60 M/s（不再空转），
  emerald 启动从 30 M/s 提升到 59 M/s，且 1.2 G 步的帧缓冲与改动前 **SHA-256 完全相同**
  （`EC9D6262…F8C9CE`），证明客户机可见行为未变。

下一步性能工作的度量口径改为「300–500 M 窗口的吞吐」，避免再被启动段的睡眠/自旋误导。

### 9.1 时钟树接入后的最终口径（P4-2）

`clk_sys` 现在按固件写入的 **252 MHz** 计时（启动阶段仍是 150 MHz）：

| 关系 | 数值 |
|---|---|
| 252 MHz 下 1 秒客户机时间 | 252 M 周期 |
| 60 fps 一帧（16.67 ms） | **4.2 M 周期** |
| emerald 300 M 步 | 1.7986 s（前 225 M 步为 150 MHz 启动睡眠） |
| emerald 300 → 360 M 步（60 M） | +0.238 s → **252 M 周期/秒**（与频率一致） |

按 4.2 M 周期/帧与真实游戏段 8.5 M 步/秒计算，**实际游戏帧率约 2 fps**；此前记录的
3.3 fps 是按 150 MHz 假设的 2.5 M 步/帧算出的，已过时。

本项目**不做 JIT**，因而不会以跳过客户机工作换取帧率；当前目标是把「精确时钟 + 精确
指令 + 功能/插件稳定」做好，帧率作为已知限制记录。


---

## 10. P4-2 时钟接入后的重新基线 / Rebaseline after P4-2

时钟树接入后，`steps` 仍然是一条指令一个 `clk_sys` 周期，但**每秒墙钟/模拟时间对应的
步数随实时频率变化**。因此旧的「同样 300 M 步、逐字段相同」判据不再成立：300 M 步在
252 MHz 下只覆盖约 1.19 s，而在 150 MHz 下覆盖 2.0 s。新的比较口径是**相同模拟时间**
（`time_us`），并同时报告 `steps`、`cycles`、`time_us`。

After P4-2 a step is still one `clk_sys` cycle, but the number of steps per second of
**simulated** time follows the live frequency. The old "same 300 M steps, identical
counters" baseline is no longer meaningful: 300 M steps cover ~1.19 s at 252 MHz versus
2.0 s at 150 MHz. Comparisons now use the same `time_us` and report `steps`, `cycles` and
`time_us` together.

| 频率 / Rate | 1 s 模拟时间所需步数 / Steps per simulated second |
|---|---|
| 150 MHz | 150 000 000 |
| 252 MHz | 252 000 000 |

**合成固件 150 vs 252 MHz（同一段写 `PLL_SYS` 的 Thumb 代码）**
**Synthetic firmware, same PLL-writing Thumb code at 150 vs 252 MHz**

| 频率 | 运行步数 | 模拟时间 | 每步覆盖 |
|---|---|---|---|
| 150 MHz (`FBDIV=125`, `/5/2`) | 1 000 000 | **6 666 µs** | 6.67 ns |
| 252 MHz (`FBDIV=126`, `/6/1`) | 1 000 000 | **3 968 µs** | 3.97 ns |

（`run/tests.rs::the_same_guest_code_runs_slower_in_simulated_time_at_252_mhz`；
10 ms 对照由 `ten_milliseconds_of_simulated_time_needs_the_live_rate_in_steps`
覆盖：1.5 M 步与 2.52 M 步都得到 10 000 µs。）

**emerald（固件在启动睡眠后把时钟升到 252 MHz）**

| 运行 | steps | time_us | 说明 |
|---|---|---|---|
| `--max-cycles 300000000` | 300 000 000 | **1 798 617**（1.80 s） | 前约 225 M 步仍是 150 MHz 的 `sleep_ms(1500)`，之后 252 MHz |
| `--max-cycles 360000000` | 360 000 000 | **2 036 712**（2.04 s） | 跨过 2 s 模拟时间；`hstx underruns=0`、`core1 Running`、8 条 `[irq]` |

- 300 M 步的 `dma=2 195 390`、`hstx words=5 401 204`、`underruns=0`；
  360 M 步为 `dma=3 958 856`、`hstx words=9 746 080`、`underruns=0`。
- 启动睡眠 `sleep core0 steps=224 994 790` 正好对应 150 MHz 下的 1.5 s；接入前同一段
  固件在模型时间里要 2.5 s 才走完，这是 P4-2 最直接的端到端证据。
- **不要**再用「300 M 步计数器相同」做回归判据；改用「跑到 `time_us >= 2_000_000`」
  或一组固定 `time_us` 检查点，并附上当时的 `steps` 与频率。


