# 解释器热路径性能 / Interpreter hot-path performance

本文件记录 **P0-1 第一轮** 与 **P0-2 第二轮** 的性能工作：测量方法、基线、每项优化
的收益、最终数字、剩余瓶颈与下一轮建议。目标与实际结果并列，未达标处如实记录。
This document records the first (P0-1) and second (P0-2) rounds of interpreter
performance work: measurement method, baseline, per-optimization gains, final
numbers, remaining bottlenecks and next-round suggestions. Targets and actual
results are both listed, and any shortfall is reported as measured.

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
- 计时口径是**进程墙钟**（`Stopwatch`），包含构建好的 release 二进制启动与固件加载；
  固件加载仅占几十毫秒，可忽略。
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
