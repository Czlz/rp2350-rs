# 解释器热路径性能 / Interpreter hot-path performance

本文件记录 **P0-1 第一轮** 的性能工作：测量方法、基线、每项优化的收益、最终数字、
剩余瓶颈与下一轮建议。目标与实际结果并列，未达标处如实记录。
This document records the first round of P0-1 performance work: measurement
method, baseline, per-optimization gains, final numbers, remaining bottlenecks
and next-round suggestions. Targets and actual results are both listed, and the
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
