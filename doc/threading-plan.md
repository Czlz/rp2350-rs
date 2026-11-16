# 多线程模拟计划 / Multithreaded emulation plan

目标：把 RP2350 的双核与外设/插件从「单线程轮流步进」改为**更接近真实硬件的并发模型**，
在保持指令级精确与可复现的前提下提高吞吐。**不做 JIT**，不为任何特定固件取巧。

Goal: model the RP2350's two cores, peripherals and device plugins concurrently (closer to the
real chip than strict instruction alternation) while keeping instruction accuracy and
determinism. No JIT and no firmware-specific tuning.

---

## 1. 现状 / Current state

| 组件 | 现状 | 与硬件的差距 |
|---|---|---|
| core0 / core1 | 同一线程里**交替各执行一条指令**（`Machine::step_cores`） | 真实芯片两核并行取指/执行，经总线仲裁访问 SRAM/Flash |
| 总线 / SRAM | 单线程直接读写，无锁 | 真实硬件有双主端口与仲裁；并发访问有确定优先级 |
| DMA / PIO / HSTX / TIMER | 在 `Machine` 的步进循环里按步数节流泵送 | 外设按自己的时钟域独立推进 |
| 插件（virt-periph） | 在模拟线程内同步调用 `step()` | 设备是独立硬件，输入/输出有传播延迟 |
| 音频 | 从 PIO TX FIFO 取样本，按墙钟播放 | 依赖模拟器达到实时吞吐（当前 2 fps → 必然欠载） |

实测：252 MHz 下每游戏帧 4.2 M 周期，真实游戏段 8.5 M 步/秒 ≈ **2 fps**；单步成本约
110–120 周期（宿主），其中 cli 骨架约 30、ARM 解释器约 70。插件在 emerald 中未启用，
因此**插件多线程不会提升该示例**，但对使用插件的场景（虚拟实验室）是必要的。

## 2. 目标架构 / Target architecture

```
            ┌──────────────┐   ┌──────────────┐
            │ core0 thread │   │ core1 thread │   每个核独立取指/译码/执行
            └──────┬───────┘   └──────┬───────┘
                   │  bus request (cycle-stamped)
            ┌──────▼──────────────────▼───────┐
            │      shared bus arbiter           │  按 (cycle, master) 定序；
            │  SRAM / Flash / PSRAM / MMIO      │  与 BUSCTRL 优先级一致
            └──────┬──────────────────┬────────┘
                   │ MMIO mailbox     │ IRQ / event
            ┌──────▼───────┐   ┌──────▼────────┐
            │ peripheral   │   │ plugin thread  │  固定传播延迟的 mailbox
            │ thread       │   │ (virt-periph)  │
            │ DMA/PIO/...  │   └───────────────┘
            └──────────────┘
```

### 2.1 双核

- 每核一个宿主线程，拥有自己的寄存器/流水线状态与取指缓存。
- 所有内存访问经 `SharedBus`：访问带**全局周期戳**；同一周期内按固定规则排序
  （默认 core0 < core1，可配置为与 `BUSCTRL` 的 `PROC0/PROC1` 优先级一致）。
- 跨核可见性：SIO FIFO、自旋锁、事件/中断按上述顺序线性化；**不引入数据竞争**（所有共享
  状态在仲裁器临界区内更新）。
- 确定性：同一程序在任意线程调度下产生相同的内存/寄存器/周期序列，因为**排序只由
  (cycle, master) 决定**，与宿主调度无关。

### 2.2 外设

- DMA/PIO/HSTX/TIMER 在独立线程按各自时钟域推进到下一个事件周期；核的 MMIO 访问通过
  邮箱在**该周期边界**交付（读需要等待返回值，写可异步但必须在下一事件前生效）。
- 事件（IRQ、FIFO、DMA 完成）由外设线程投递到核线程的事件队列，唤醒顺序同样按周期定序。

### 2.3 插件

- `virt-periph` 的 `DevicePlugin::step` 在插件线程执行；模拟器发送
  `{cycle, pin levels}` 批次，插件返回 `{cycle + latency, driven levels}`，
  `latency` 固定（例如 1 个外设 tick），因此**可复现**。
- 插件线程崩溃/超时不得拖死模拟：邮箱有超时与降级（记录诊断，设备视为高阻）。

### 2.4 音频

- 音频是「实时外设」：只有模拟吞吐 ≥ 客户机实时率时才能连续出声。
- 多线程目标是把游戏段从 8.5 M 步/秒提升到 ≥ 50 M（20 fps）甚至更高；在此之前 GUI 明确显示
  `underruns / produced per second / needed 13440 per second`，不伪造声音。

## 3. 任务拆分 / Tasks

| ID | 任务 | 归属 | 验收 |
|----|------|------|------|
| MT-1 | **测量与原型（spike）**：逐核耗时、共享总线访问占比、SIO/IRQ 频率；用 2 线程 + 周期定序仲裁器做最小原型，测端到端加速与确定性（300 M 计数器逐字段） | `rp2350-core` + `rp2350-cli`（临时插桩，交付前删除） | `doc/threading-plan.md` 增补实测表；原型加速比与 300 M 一致性证据；给出 go/no-go |
| MT-2 | **`SharedBus` 与仲裁器**：按 (cycle, master) 线性化 SRAM/Flash/MMIO；SIO/自旋锁/事件接入；单线程回归全绿 | `rp2350-core` + `rp2350-periph` | 单线程下 300 M 计数器不变；并发单元测试；无 `unsafe` 数据竞争 |
| MT-2a | **可共享总线层**：`SharedRegion`/`SharedAddressBus`/`SharedBus`、10 bank SRAM、Flash 免锁读、每设备 MMIO 锁、并发测试 | `rp2350-core` | ✅ 已交付（见 §7）：core 122 测试、12 个并发/契约测试、300 M 计数器不变 |
| MT-2b | **迁移与仲裁**：把 bootrom/usb/usb-host/periph 的区域改为 `SharedRegion`，CLI 组装 `SharedBus`，按 (cycle, master) 批量定序 | `rp2350-core` + `rp2350-periph` + 上述 crate | 单线程 300 M 计数器不变；双核原型接 `SharedBus` |
| MT-3 | **双核线程化**：`Machine` 拆为两台核线程 + 仲裁器；中断/事件跨线程投递；睡眠核不占调度 | `rp2350-cli` + `rp2350-arm` | FV-1/2/3 与 emerald 300 M 计数器一致；默认并行运行 30 次无 flake；吞吐报告 |
| MT-4 | **外设线程**：DMA/PIO/HSTX/TIMER 独立推进与 MMIO 邮箱；事件定序 | `rp2350-periph` + `rp2350-core` | 与单线程逐周期对照（同一时间点状态一致）；emerald HSTX `underruns=0` |
| MT-5 | **插件线程**：`virt-periph` 邮箱 + 固定延迟；超时/崩溃降级 | `rp2350-virt-periph` + `rp2350-plugin` | ✅ 已交付（见 §8）：与同步路径逐 pin 对照；重载/删除不崩溃 |
| MT-6 | **性能与回归收口**：长跑 soak、文档、`doc/performance.md` 新口径 | 主会话 | 数据 + 结论 |

依赖：MT-1 → MT-2 → MT-3 ∥ MT-4，MT-5 可与 MT-3 并行（crate 不相交）。

> **MT-1 已交付**：逐核/逐区域实测、锁微基准、双线程原型与 go/no-go 见 §6。
> MT-1 delivered: per-core/per-region measurements, lock microbenchmarks, the
> two-thread spike and the go/no-go decision are in §6.
>
> **MT-2a 已交付**：可共享总线层（不含线程），见 §7；原 MT-2 拆分后的迁移与仲裁
> 定序为 MT-2b。 / MT-2a delivered the shareable bus layer (no threads) in §7; the
> migration and arbitration half of the original MT-2 is now MT-2b.
>
> **MT-5 已交付**：每设备插件线程 + 固定延迟邮箱，默认仍为同步路径，见 §8。
> / MT-5 delivered the per-device plugin thread with a fixed-latency mailbox;
> the synchronous path stays the default (see §8).

## 4. 风险与回退 / Risks

- **锁竞争**：如果每次内存访问都要全局锁，可能比单线程更慢。缓解：按 SRAM bank 分片、
  每核私有取指缓存、批量窗口（例如每 64 周期一次屏障）。
- **确定性**：任何依赖宿主调度的顺序都会破坏可复现性。验收必须包含 30 次并行运行与
  300 M 计数器逐字段一致。
- **回退**：若 MT-1 显示加速 < 1.3×，保留单线程默认路径，把多线程作为可选
  `--threads` 模式；不得为了并行改变客户机可见语义。
- **插件语义**：固定传播延迟是建模选择，必须写进文档（真实器件也有传播延迟）。

## 5. 与「精确硬件」的关系 / Hardware fidelity

RP2350 的两核确实并行执行，共享总线有仲裁；外设按各自时钟域独立运行。当前的单线程
「交替各一条」是为了确定性而做的串行化，**多线程 + 周期定序仲裁**反而更接近硬件行为，
前提是顺序规则显式且可复现（不是「谁先抢到锁谁先」）。

---

## 6. MT-1 实测与原型 / MT-1 measurements and spike（2026-11-16）

本节所有数字均由**临时插桩/原型**测得，交付前已删除；默认路径语义未改，删除后
300 M 步的计数器逐字段复现基线（见 6.1）。测试机为 Windows/x86-64，`rdtsc` 实测
TSC **3.2937 GHz**，release 构建。

All numbers below come from temporary probes/prototypes that were deleted before
delivery; the default path is unchanged and reproduces the exact baseline after
removal. Host: Windows/x86-64, measured TSC **3.2937 GHz**, release build.

### 6.1 基线 / Baseline

```powershell
target\release\rp2350-cli.exe run --file "<emerald.uf2>" --max-cycles 300000000 --trace-steps 0
```

| 字段 / Field | 基线值 / Baseline |
|---|---|
| `executed` / `time_us` | 300 000 000 steps / **1 798 617 µs** |
| `dma` | **2 195 390**，`busy=0x0009` |
| `hstx` | **words=5 401 204**，`underruns=0`，`fifo=8` |
| `core1` | `state=Running pc=0x1000035C sp=0x2003D680 launches=1` |
| `[irq]` | 8 条（core0/core1 交替，IRQ 10） |
| 进程墙钟 / wall | **14.19 s**（静置机；并行会话负载下 18.22 s） |

插桩后同一 300 M 运行的计数器与基线完全一致（`time_us=1798617`、`dma=2195390`、
`hstx words=5401204`、`core1 Running`、8 条 `[irq]`），证明测量只增加宿主侧计数。

### 6.2 逐核与逐区域 / Per-core and per-region

emerald 300 M 步、252 MHz。时间用 1/1024 采样 `rdtsc`，乘以 1024 外推；访问计数为
精确值。探针运行墙钟 16.25 s（比基线慢约 14 %），因此**绝对时间按比例看，不要与
基线混用**。

| 指标 / Metric | core 0 | core 1 |
|---|---|---|
| retired instructions | **75 005 204** | **74 625 856** |
| sleep steps | **224 994 790** | 0 |
| `step_core1` 调用 / calls | — | 74 625 857（仅 1 次跳过） |
| 首次可运行步 / first live step | 0 | **≈225 M（模拟 0.893 s）** |
| 宿主时间（phase 1，采样外推） | **4.81 s** | **5.53 s** |
| phase 1 每指令宿主时间 | ≈64 ns | ≈74 ns |

关键结构：300 M 步分成两段。

| 阶段 / Phase | 步数 / steps | 墙钟（探针） | 内容 |
|---|---|---|---|
| phase 0 | 0 … ≈225 M | **4.99 s** | core0 全部在睡眠（0 条指令退役），core1 尚未启动 |
| phase 1 | ≈225 M … 300 M | **10.66 s** | 双核都活跃：core0 75.0 M 指令 + core1 74.6 M 指令 |

- phase 0 是**纯空转**：没有客户机指令、没有外设 DMA 工作，只有 core0 的睡眠步与
  6 次唤醒；仍花掉 30.7 % 的总墙钟（睡眠步约 13 ns/步）。
- phase 1 两核接近均衡（core0 45 %、core1 52 %、共享路径 3 %），这是双线程的最大机会。
- core0 的 75.0 M 条指令几乎全部落在 phase 1；其中最大的单一来源是
  `0x1000B394..0x1000B39C`（以及同形态的 `0x1000B3A4..0x1000B3AC`）的忙等循环：
  每轮 5 条指令、**2 次** `TIMER0` 读（先读 `TIMERAWH`=`0x400B0024`，再读
  `TIMERAWL`=`0x400B0028`，比较高字判断是否跨越 32 位回绕，分支回跳）。
  17.19 M 次 TIMER 读折算 ≈8.6 M 轮 × 5 条 ≈43 M 条指令，即 core0 **一半以上**的
  指令都在等硬件定时器，不是在跑游戏逻辑。

按区域统计（300 M 步累计；host = DMA/HSTX 泵）：

| 区域 / Region | core 0 | core 1 | host |
|---|---|---|---|
| Flash 读 / read（取指未命中） | 5 741 043 | 4 301 033 | 34 |
| Flash 写 / write | 0 | 0 | 0 |
| SRAM 读 / read | 11 475 469 | 34 021 585 | 2 195 390 |
| SRAM 写 / write | 206 898 | 2 101 189 | 4 |
| MMIO 读 / read（APB/PPB） | **17 187 549** | 1 315 972 | 0 |
| MMIO 写 / write | 1 299 | 4 562 | **2 195 390** |
| SIO FIFO 写 / `FIFO_WR` | 6 | 0 | 0 |
| SIO FIFO 读 / `FIFO_RD` | 6 | 0 | 0 |
| SIO `FIFO_ST` 读 / read | 24 | 0 | 0 |
| SIO 自旋锁 / spinlock | 0 | 0 | 0 |

- 每核每退役指令约 **0.46（core0）/ 0.56（core1）** 次总线访问；phase 1 内平均
  **≈1.08 次共享访问 / 机器步**，总线并不空闲。
- 取指缓存把 Flash 取指压到 0.06–0.08 次/指令；Flash 只读、客户机不可写，**无需锁**。
- SRAM 按 RP2350 的 10 bank 字交错（`(addr>>2) % 10`）统计：
  - core0：b6/b7 各 ≈5.74 M（合计 98 %），其余 bank 各 ≈25 K；
  - core1：b1/b2 各 ≈13.7 M（合计 76 %），b4–b7 各 ≈1.75 M，b8/b9 ≈0.43 M。
  - 按访问分布估算，两核**同时命中同一 bank 的概率仅 ≈6 %**；10 bank 分片可消除约
    94 % 的 SRAM 争用。两个热点（core0 b6/b7、core1 b1/b2）基本不相交。

跨核事件频率（300 M 步）：

| 事件 / Event | 次数 / count |
|---|---|
| `sync_core_events` 调用 | 300 000 000（每步一次） |
| SIO 事件世代变化 / generation changes | **18** |
| SIO FIFO 帧 / frames | 写 6、读 6 |
| IRQ 投递 / deliveries | core0 **4 555**、core1 **4 549**（含 SysTick 转发） |
| SIO 自旋锁 / spinlock | 0 |

结论：emerald 的跨核通信几乎为零，SIO/IRQ 用**事件驱动 + 原子信箱**即可，
不需要无锁队列或复杂仲裁；真正的成本在每步的总线访问与 MMIO 轮询。

### 6.3 锁成本微基准 / Lock microbenchmarks

单线程无竞争、4 M 次 32 位 SRAM 读；两线程各 4 M 次。`ns/访问` 为总墙钟 ÷ 总访问数。

| 方案 / Case | ns / access |
|---|---|
| 无锁 `SimpleBus`（基线） | **5.10** |
| `std::sync::Mutex` 全局锁，单线程 | 6.92 |
| `parking_lot::Mutex` 全局锁，单线程 | 6.77 |
| 10 个 SRAM bank 分片，单线程 | **4.87** |
| `std` 全局锁，2 线程争用 | **36.81** |
| `parking_lot` 全局锁，2 线程争用 | **8.97** |
| 10 bank 分片，2 线程无交集 | **9.44** |
| `std::sync::Barrier` 每步两次等待（2 线程） | **483.54 / 轮** |

- 无竞争锁只贵 ~1.8 ns；两线程全局 `std::Mutex` 贵 **+31.7 ns/访问（5.3×）**。
- `parking_lot` 的自适应自旋把两线程争用压到 **+2.2 ns/访问**，分片为 **+4.6 ns**。
- 每步屏障约 **484 ns/轮**，相当于每步多 240 ns；**绝不能做逐周期/逐步屏障**，
  只能按批次窗口（64–256 步）定序。

### 6.4 最小双线程原型 / Two-thread spike

合成工作负载：两核各跑 Flash 中的 4 指令计数循环
`LDR r1,[pc,#8]; LDR r0,[r1]; ADDS r0,#1; STR r0,[r1]; B .`，每核 20 M 条指令
（合计 40 M），每 4 条指令 1 次 SRAM 读 + 1 次写；取指全部命中各自的 Flash 缓存。
单线程基线 = 当前语义（两核交替各一条）；所有变量均核对计数器 = 5 000 000。
下表的“加速比”= 单线程墙钟 ÷ 该方案墙钟，取 3 次最好值。

| 方案 / Case | 墙钟 / wall | 总吞吐 | 加速比 |
|---|---|---|---|
| 单线程交替（当前语义） | 943–1032 ms | 39–42 M instr/s | 1.00× |
| 双线程 + 私有总线（理想上限） | 439.9 ms | 90.9 M instr/s | **2.20×** |
| 双线程 + 10 bank 分片 SRAM + 私有 Flash | **412.7 ms** | **96.9 M instr/s** | **2.32×** |
| 双线程 + 全局 `std::Mutex` 总线 | 0.79–1.41 s | 28–51 M instr/s | **0.68–1.23×** |
| 双线程 + 全局 `parking_lot::Mutex` | 0.85–1.43 s | 28–47 M instr/s | **0.67–1.22×** |
| 双线程 + 逐步轮转（严格交替） | 3.11–4.53 s | 8.8–12.9 M instr/s | **0.21–0.32×** |

（加速比按**同一轮次内**的单线程墙钟计算，取 3 轮中的最好值；墙钟/吞吐取各方案
自己的最好样本。全局锁的波动来自宿主调度，不是测量误差。）
(Speedups use the single-thread wall clock from the same round; wall/throughput
show each case's best sample. Global-lock variance comes from host scheduling.)

- 全局锁方案不仅慢，而且**方差极大**（0.68–1.23×），因为它把两核的每次访问都串行化；
  该合成负载每 4 条指令就有 2 次共享访问，是 emerald phase 1（≈1.05 次/步）的
  乐观代理。
- 10 bank 分片已接近私有总线的理想上限；这是 MT-2 必须达到的形态。
- **锁等待/争用开销**：以同轮“私有总线”为无锁基线，全局 `std::Mutex` 每访问多
  **17.5–48 ns**（20 M 次访问共 0.35–0.97 s），全局 `parking_lot` 多 **20–49 ns**；
  这就是锁等待与串行化的合计成本。直接在每次 `lock()` 外包 `Instant` 会被
  ≈55 ns/次的计时器开销淹没，故未采用。

**「按 (cycle, master) 定序」在原型中的近似**：原型没有真实周期戳，共享总线按
互斥锁的**到达顺序**线性化（宿主调度相关，不确定）；最后一个用例用两个原子标志做
**严格轮转**（core0 一步、core1 一步），这是 (cycle, master) 定序的逐指令特例。
它把吞吐压到 0.21–0.32×，证明逐指令定序不可行；可接受的近似是**按批次窗口定序**：
每个窗口内各核只访问私有/独享资源，窗口边界按 `(cycle, master)` 合并共享写与事件。

### 6.5 Amdahl 估算 / Speedup estimate

用 6.2 的实测拆分（phase 0 4.99 s 串行；phase 1 core0 4.81 s、core1 5.53 s、
共享 0.32 s），phase 1 共享访问 ≈80.7 M 次（1.08 次/步）：

| 方案 / Case | phase 1 | 整个 300 M | 游戏窗口（phase 1） |
|---|---|---|---|
| 理想（零锁、零调度开销） | 5.53 s | **1.54×** | **1.93×** |
| 10 bank 分片（+4 ns/访问） | 5.85 s | **1.50×** | **1.82×** |
| 全局锁（+17.5 至 +40 ns/访问） | 6.9–8.8 s | **1.18–1.36×** | **1.22–1.53×** |
| 理想 + 睡眠核不占宿主（phase 0 ≈0） | 5.53 s | **≈2.8×** | 1.93× |

- **整个 300 M 运行**被 phase 0 的 30.7 % 空转封顶；即使完美双线程也只有 ≈1.54×。
- **游戏窗口**（phase 1）才是双线程的目标区间：分片后 ≈1.8×，与 6.4 原型的 2.2×
  同向（原型更偏总线密集，故绝对值略高）。
- 全局锁在最好情况下刚过 1.3×，在争用/调度不利时跌到 1.2× 以下；**不能作为 MT-2
  的交付形态**。
- phase 0 的 225 M 步是“core0 睡眠 + core1 未启动”。若 MT-3 真正实现计划里的
  “睡眠核不占调度”（等到下一事件、把步数/周期一次性推进并补齐外设泵），整个运行
  可望到 ≈2.8×；但这是**语义决策**，必须同时定义 DMA/PIO/HSTX/TIMER 的追赶规则。

### 6.6 go / no-go

- **no-go：单一全局互斥锁总线。** 原型 0.68–1.23×，emerald 估算 1.18–1.36×，
  方差大且低于/贴着 1.3× 门槛。
- **go：10 bank 分片 SRAM + 每区域/每设备锁 + 只读 Flash 免锁 + 批量定序。**
  原型 2.32×，emerald 估算整跑 ≈1.5×、游戏窗口 ≈1.8×。
- **条件**：MT-2 必须先交付分片总线并通过 300 M 计数器逐字段一致；MT-3 不得以
  逐指令/逐周期握手实现定序。满足这两条才进入默认 `--threads` 之外的产品化。

### 6.7 对 MT-2 / MT-3 的建议 / Adjustments

1. **MT-2：不要交付全局锁。** 最小形态是 10 个 SRAM bank 锁（字交错，
   `(addr>>2)%10`）+ 一个 APB/MMIO 锁 + Flash 只读免锁。emerald 的同 bank 冲突
   概率仅 ≈6 %，分片收益接近理想。
2. **MT-2：MMIO 轮询必须走低开销路径。** core0 在 phase 1 有 17.19 M 次
   `TIMER0` 读，占其总线访问的 **50 %**；任何“每次读一次跨线程信箱往返”的设计都会
   增加 ≥17 s。应让 TIMER 的 `TIMERAWH/TIMERAWL` 由共享原子快照服务，或使用 core1
   不会触碰的独立设备锁。
3. **MT-2：Flash 取指不加锁。** 客户机写 Flash 被拒绝，宿主写有世代号；每核已有
   独立 `FetchCache`，可以直接读私有/只读副本。
4. **MT-3：批量窗口 64–256 步。** `PUMP_INTERVAL=64` 是现成边界；每 64 步一次
   定序握手摊到 7.6 ns/步（相对 47 ns/步基线 ≈16 %），256 步降到 ≈1.9 ns/步。
   逐指令轮转实测 0.21–0.32×，不可接受。
5. **MT-3：优先处理睡眠核。** phase 0 的 30.7 % 是纯睡眠空转；若“睡眠核不占调度”
   落地，整跑上限从 1.5× 提到 ≈2.8×，收益大于核心并行本身。需先定义事件驱动的
   时钟/外设追赶规则，保证 `time_us`、DMA/HSTX 计数不变。
6. **无锁队列暂不必要。** SIO 世代变化 18 次、IRQ 投递 4.5 K 次/300 M 步；原子
   信箱 + 批量事件队列足够。只有 MT-4 的外设线程需要更高的队列吞吐。
7. **锁实现选择**：`parking_lot` 在纯争用微基准明显更好（8.97 vs 36.81 ns），但
   端到端原型中两者都受“整条总线在一个临界区”拖累；先分片，再谈锁算法。

### 6.8 风险与测量局限 / Risks and limits

- **确定性未验证**：原型按锁到达顺序线性化，宿主调度相关；MT-3 必须用 30 次并行
  运行 + 300 M 计数器逐字段一致来证明。批量窗口定序是当前唯一可行路径。
- **测量噪声**：并行会话/后台进程会让全局锁用例在 0.68–1.23× 间波动；分片/私有
  用例稳定在 2.0–2.3×。表中已给区间而非单点。
- **插桩开销**：计时运行比基线慢 ≈14 %，访问计数运行慢 ≈11–15 %；因此报告比例
  与相对拆分，不用探针绝对时间替代基线。
- **通用性**：本轮只测了 Arm emerald。core0 的 TIMER 忙等是**该客户机**的行为，
  不代表 MicroPython/CircuitPython/其它 Arm 或 Hazard3 负载；MT-2/MT-3 的结论
  （分片、批量窗口、MMIO 低开销）是通用设计约束，但加速比数字只对 emerald 成立。
- **未测**：真实跨线程 IRQ 投递延迟、SIO FIFO 阻塞语义、WFE/WFI 睡眠的线程化唤醒；
  这些属于 MT-3/MT-4 的验收项。

---

## 7. MT-2a 交付记录：可共享总线层（2026-11-16）
## 7. MT-2a delivery: shareable bus layer (2026-11-16)

本节记录 MT-2a（“把总线层改为线程安全、可并发的形态”）的交付内容、与原始任务书的
偏差、实测数据与 MT-3 注意事项。**本轮没有引入任何线程**：默认 `run`/`repl` 路径
仍然是单线程独占总线，新增的并发层是附加类型。
This section records what MT-2a delivered, where it deviates from the original task
list, the measurements, and the MT-3 follow-ups. **No thread is introduced yet**:
the default `run`/`repl` path still uses the single-threaded exclusive bus; the
concurrent layer is additive.

### 7.1 交付内容 / Delivered

| 项 / Item | 实现 / Implementation |
|---|---|
| 共享区域契约 | `shared::SharedRegion`（`&self` 访问器 + `read_would_stall` + `load_u8/load_bytes`），`Send + Sync` |
| 共享总线路由 | `shared::SharedAddressBus`（只要求 `find_region` + `ppb_write_epoch`，其余为默认实现） |
| 可共享句柄 | `shared::SharedBus`（`Vec<Arc<dyn SharedRegion>>` + 256 个 `AtomicU16` 桶标签 + `AtomicU64` PPB 世代）；经 `Arc<SharedBus>` 跨线程共享 |
| 旧区域迁移桥 | `shared::LockedRegion`（`Mutex<Box<dyn MemoryRegion>>`，每区域一把锁） |
| SRAM 分 bank | `RamRegion` 改为 10 个 `parking_lot::Mutex<Vec<u8>>`，`bank = ((addr-base)>>2) % 10`；独占路径用 `Mutex::get_mut`（零原子操作） |
| Flash 免锁读 | `FlashRegion` 底层改为 `Arc<FlashStore>`（`Vec<AtomicU32>`，relaxed）；读者只做一次普通加载；宿主写用 CAS/整字 store，并发布 `content_generation` |
| MMIO 每设备锁 | `MmioRouter` 改为 `Vec<Mutex<MmioRegion>>` + 只读 `Vec<MmioWindow>`；页缓存改为打包的 `AtomicU64` 标签 |
| 单线程零开销钩子 | `MemoryRegion::read_would_stall_mut`（默认转发 `read_would_stall`），让路由器用 `get_mut` 回答 stall，不给轮询路径加锁 |
| 并发测试 | `shared/concurrency.rs`（10 个线程测试）、`shared/tests.rs`（6 个单元测试） |

### 7.2 与任务书的偏差（重要）/ Deviation (important)

任务书第 1 条要求把 `MemoryRegion` / `AddressBus` 的读写方法从 `&mut self` 改成
`&self`。**本轮没有这样做**，因为这两个 trait 的实现者分布在白名单之外的 crate：

- `MemoryRegion`：`rp2350-bootrom/src/region.rs`、`rp2350-usb/src/regions.rs`(3 处)、
  `rp2350-usb-host/src/mask_rom.rs`、`rp2350-periph/src/psram.rs`；
- `AddressBus`：`rp2350-arm/src/execute_thumb32/test_support.rs`。

改动 trait 签名会让这 4 个 crate 直接编译失败，与本轮“不得修改其它 crate、四道门禁
全绿”的硬约束冲突。因此本轮改为**附加**一层 `SharedRegion` / `SharedAddressBus`：
调用点（`&mut dyn AddressBus`）零改动，独占路径零成本，并发路径由新类型承担。
迁移这些实现属于 MT-2b（新增任务），完成后再考虑是否把两个 trait 合一。
The task list asked for `&mut self` → `&self` on `MemoryRegion` / `AddressBus`. That
was **not** done: implementors live in four crates outside this task's whitelist
(`bootrom`, `usb`, `usb-host`, `periph` for `MemoryRegion`; `rp2350-arm` for
`AddressBus`). Changing the signatures would break those crates and the "all gates
green, no other crate touched" constraint. This round therefore adds a parallel
`SharedRegion` / `SharedAddressBus` layer: call sites and the exclusive path are
unchanged, and the concurrent path lives in the new types. Migrating those
implementors is MT-2b.

### 7.3 锁模型与不变量 / Locking model and invariants

- 每区域自带锁粒度：SRAM = 每 bank 一把；Flash = 无锁；MMIO 路由器 = 每子设备一把；
  `LockedRegion` = 每区域一把；`SharedBus` 自身无锁（只有原子桶标签）。
- **锁顺序**：bank 锁始终按 bank 下标升序获取；一次访问最多同时持有两把 bank 锁，
  且持锁期间不再获取其它锁。MMIO 访问最多持有**一把**设备锁；路由器的警告集合
  只在未建模访问时单独加锁，且绝不跨设备锁持有。
- **跨 bank 语义**：单次宽访问在它所触及的全部 bank 锁内完成，因此相对其它**总线
  访问**是原子的；但两次独立访问（例如先读低半字再读高半字）可能看到不同时刻的两个
  半字——与真实总线「每个字一次传输」一致。文档写在 `doc/crates/rp2350-core.md`。
- **写监视**：已安装 `RP2350_WATCH` 时，共享路径仍逐字节上报，但整段写入在同一把
  bank 锁内完成（不放松诊断语义，也不产生可观察的撕裂）。
- **Flash**：单字访问原子；跨字跨度可能混合世代，批量快照（`bytes`/`slice`/
  `FlashHandle::read`）在 `content_generation` 变化时重试。
- **MMIO 别名**：SET/CLR/XOR 的读-改-写在同一把设备锁内完成，并发别名写不丢位。
- **挂载**：所有 `register_region` / `mount` 都取 `&mut self`，即发生在总线被
  `Arc` 共享之前；`SharedBus` 的桶标签与路由器页标签都在挂载时整体失效。

### 7.4 并发测试 / Concurrency tests

默认并行测试下连跑 20 次全绿（`cargo test -p rp2350-core --all-features`）。
Green 20× under the default parallel runner. `shared/concurrency.rs` holds the ten
threaded tests; the two bus-contract tests at the end of the table live in
`shared/tests.rs`.
默认并行测试下连跑 20 次全绿；十个线程测试在 `shared/concurrency.rs`，表中最后两个
总线契约测试在 `shared/tests.rs`。

| 测试 / Test | 断言 / Property |
|---|---|
| `two_threads_write_different_banks_without_losing_updates` | 不同 bank 并发写，各自最后一个值完整保留 |
| `two_threads_write_different_banks_into_different_buses` | 同一条 `SharedBus` 的两个区域并发写，路由无竞争 |
| `same_bank_writes_are_serialised_and_never_tear` | 同一 bank 同一字屏障交替写/读，读到的永远是完整图案 |
| `cross_bank_word_access_is_atomic_within_one_access` | 跨 bank 非对齐字单次读不撕裂 |
| `cross_bank_halves_can_be_observed_separately` | 两次独立半字读只能看到两个合法图案的半字 |
| `flash_reads_stay_consistent_while_the_host_writes` | Flash 并发读 + `FlashHandle` 擦/写：字不撕裂、世代推进 |
| `flash_bulk_snapshot_is_consistent_per_word` | 批量快照每 4 字节组来自同一世代 |
| `two_mmio_devices_run_concurrently` | 两个设备各自并发写，互不等待、结果正确 |
| `one_mmio_device_serialises_concurrent_alias_writes` | 同设备并发 SET 别名写，最终位并集完整 |
| `ppb_epoch_counts_every_write_across_threads` | 跨线程 PPB 写世代按次计数 |
| `shared_bus_keeps_the_stall_gate` | 共享总线保留 stall 门控（区域未被触碰、不记周期） |
| `locked_region_bridges_an_exclusive_region` | `LockedRegion` 转发读写与只读拒绝 |

另有一个**并发环境**下的既有语义测试：`memory/tests.rs` 的写监视/宽写语义测试会
全局安装写监视，并发测试必须在其同时运行时仍然成立（已修复共享 `write_u16/u32`
在监视开启时的原子性，见 7.3）。

### 7.5 性能 / Performance

**微基准（`crates/rp2350-core/examples/bus_bench.rs`，交付前删除）**：同一进程内
模拟 emerald phase 1 的访问混合（30 M RAM 读写 + 18 M MMIO 轮询 + 10 M Flash 取指），
同机交替运行、取 8 次最小值：

| 场景 / Case | MT-2a 前 | MT-2a 后 | 变化 |
|---|---|---|---|
| `ram-mixed`（4 读 : 1 写，随机字） | 5.41 ns/op | **5.19 ns/op** | −4.1 % |
| `mmio-poll`（15 设备路由器轮询一个寄存器） | 28.26 ns/op | **27.29 ns/op** | −3.4 % |
| `flash-fetch`（半字取指） | 5.36 ns/op | **5.29 ns/op** | −1.3 % |

**端到端（emerald，release，CPU 时间，同机交替）**：

| 场景 | 前 / Before | 后 / After |
|---|---|---|
| 50 M 步墙钟最小值（3 次，验收口径） | 0.786 s | 0.803 s（+2.2 %） |
| 50 M 步 CPU 时间最小值（20 次交替） | 0.781 s | 0.828 s（+6 %） |
| 50 M 步 p25 / 中位数（CPU） | 0.828 / 0.859 s | 0.875 / 0.906 s |
| 300 M 步 CPU 时间最小值（噪声大） | 15.88–16.56 s | 17.7–18.2 s |

**结论与局限**：50 M 步窗口是**纯睡眠**（`sleep steps=49 894 568`、0 条指令退役），
本轮改动没有任何代码在该路径上执行。作为对照，只给**旧**核心加一个从未调用的
`pub fn`（纯代码布局扰动）就使同一指标从 0.781 s 变成 0.828 s（+6 %）——说明该窗口
的差异来自二进制布局/对齐噪声，而不是总线语义；微基准（真正测总线）显示 −3～−4 %。
300 M 的绝对时间在本机被并行会话负载拉到 15.9–22.7 s 区间，无法分辨 <5 % 的差异，
因此只作为“无数量级退化”的证据。**建议主会话在静置机上复测 50 M×3 与 300 M**。
The 50 M window is pure sleep (zero retired instructions), and nothing this round runs
on that path. As a control, adding a never-called `pub fn` to the **old** core (pure
code-layout perturbation) moved the same metric from 0.781 s to 0.828 s (+6 %), so the
end-to-end delta is layout noise; the bus microbenchmark (which actually exercises the
changed code) is 3–4 % faster. The 300 M wall clock on this host varied 15.9–22.7 s
because of concurrent sessions, so it only rules out an order-of-magnitude regression.

### 7.6 计数器与固件回归 / Counters and firmware regression

emerald 300 M（同一 UF2、同一宿主）：

| 字段 | 值 |
|---|---|
| `time_us` | **1 798 617** |
| `dma` / `busy` | **2 199 378** / `0x0009` |
| `hstx` | **words=5 401 204 underruns=0 fifo=8** |
| `core1` | `Running pc=0x1000035C sp=0x2003D680 launches=1` |
| `[irq]` | 8 条（core0/core1 交替，IRQ 10） |
| final PC | `0x1000B3AC` |

与**改动前的同一工作树**逐字段一致。与 §6.1 记录的基线有两点差异，均非本轮引入：
`dma` 2 195 390 → 2 199 378、final PC `0x1000B3AA` → `0x1000B3AC`。并行任务修改了
`crates/rp2350-periph/src/dma.rs`（`enabled_selects_dreq` 现在要求 DREQ 已断言），
用改动前的核心源码在同一工作树重建后同样得到 `dma=2 199 378`、PC `0x1000B3AC`，因此
该差异来自外设侧改动。验收时应以本节数值为准（或等该并行任务合并后重定基线）。
Identical to the pre-change working tree field by field. The two differences against
§6.1 (`dma` 2 195 390 → 2 199 378, final PC `0x1000B3AA` → `0x1000B3AC`) come from the
concurrent `rp2350-periph/src/dma.rs` change, not from this round: rebuilding the old
core in the same tree reproduces them.

### 7.7 对 MT-3 的注意事项 / Notes for MT-3

1. **先建 `SharedBus`，再拆线程**：CLI 需要把 SRAM/Flash/APB 注册为
   `Arc<RamRegion>` / `Arc<FlashRegion>` / `Arc<MmioRouter>`；尚未迁移的区域
   （PSRAM、USB、bootrom ROM）用 `SharedBus::register_locked`，它们各自仍是一把锁。
   注意 `RamRegion::bytes()` 现在是复制（分 bank 存储没有连续切片）。
2. **不要逐周期/逐步加锁**：MT-1 已证明逐步屏障 ≈484 ns/轮。批量窗口 64–256 步
   （`PUMP_INTERVAL=64` 是现成边界）仍是唯一可行路径。
3. **跨 bank 非原子是文档化的语义**：定序器不需要为两次独立访问之间的 bank 混合
   做额外工作；但任何“用两次窄读拼一个宽值”的宿主诊断代码必须在共享总线上改为
   宽访问。
4. **睡眠核**仍是最大收益（phase 0 占 30.7 %）；`SharedRegion` 不改变睡眠路径。
5. **`read_would_stall` 与 `read_would_stall_mut` 必须同时正确**：新区域若在共享
   实现里加锁，应同时覆盖独占变体，否则单线程轮询路径会多一次虚调用或一次锁。
6. **迁移旧 trait**（可选）：若 MT-3 决定统一到 `&self`，先把 4 个外部实现改成
   `SharedRegion`，再删除 `MemoryRegion`，避免一次性改 5 个 crate。
7. **共享路径的 MMIO 读当前取两次设备锁**：一次用于 `read_would_stall` 门控、一次
   用于实际读。单线程路径已用 `read_would_stall_mut` 完全避开加锁；若 MT-3 的
   profiling 显示共享路径的 MMIO 轮询成为瓶颈，可让 `SharedBus` 在同一把设备锁内
   完成 stall 检查与读（需要给 `SharedRegion` 增加一个“带门控的读”入口）。

---

## 8. MT-3a 交付记录：睡眠快进（2026-11-16）

## 8. MT-3a delivery: sleep fast-forward (2026-11-16)

本节记录 MT-3a（把「全睡且无事件」的时钟步批量推进）的实现、不变量、开关、测试与
实测。**不引入线程**，也不跳过任何客户机指令或周期：睡眠步仍计入 `steps`/`time_us`，
只是宿主不再为每个睡眠步执行一次机器循环。
This section records MT-3a (batching clock-only steps while every runnable core sleeps)
including invariants, the switch, tests and measurements. **No thread is introduced**
and no guest instruction or cycle is skipped: sleeping steps still advance
`steps`/`time_us`; the host simply stops running one machine-loop iteration per step.

### 8.1 判定与边界 / Guard and boundaries

| 条件 / Condition | 检查 / Check |
|---|---|
| core0 睡眠且非阻塞载入 | `CpuState::Sleeping` 且 `SleepReason != BlockedRead` |
| core1 不可运行 | 无 / `Reset` / `Sleeping`；睡眠时也须非阻塞载入、无待处理事件、无挂起异常 |
| 核内挂起异常 | `NVIC.ISER & NVIC.ISPR`（两个 enable/pending 字）、`SCB_ICSR` 的 NMI/PendSV/SysTick 锁存、`SHCSR.SVCALLPENDED` 全空 |
| 外设中断 | `BootPeripherals::irq_mask`、`core1_irq_mask`、`USBCTRL.INTS` 全 0，且无待转发 SysTick 锁存 |
| 外设活动 | `Dma::busy_mask()==0`、`pio_busy_mask()==0`、HSTX 关闭、ADC 未转换 |
| core1 握手 | `core1_reset_pending`/`core1_launch_pending`/`core1_release_pending`/`fifo_rx_ready(Core1)` 全假 |

事件观察边界是下一次 `ADVANCE_INTERVAL`（10 000 步）时基推进：TIMER alarm、SysTick、
ADC/POWMAN 与时钟树 `generation` 都只在该边界被采样。快进停在 `boundary - 1`，正常
循环亲自执行边界步；边界后新锁存的 alarm/SysTick 由下一次守卫看到并回退到逐步路径，
再由 64 步泵送在**同一步**唤醒。DMA/PIO/IRQ 节流计数按整 64 步折叠，保留之后每次
泵送的相位；睡眠步、`sleep_steps`、「双核睡眠」通知按逐步执行记账。

The observation boundary is the next 10 000-step timebase advance (TIMER alarms,
SysTick, ADC/POWMAN and clock-tree generation are sampled there). The skip lands on
`boundary - 1`; the normal loop executes the boundary step, the next guard sees any
newly latched alarm/SysTick and falls back, and the 64-step pump wakes on the same step
as before. Pump counters fold by whole 64-step periods; sleep steps and the double-sleep
notice are credited exactly.

### 8.2 开关 / Switch

- `run --sleep-fast-forward on|off`（默认 `on`，接受 `true/false`、`1/0`）。
- `RP2350_NO_SLEEP_FF=1|true` 对全部前端关闭；显式 CLI 值优先。
- `Machine::set_sleep_fast_forward`/`sleep_fast_forward`/`sleep_fast_forward_steps`。
  `off` 时 `run_steps` 的守卫立即返回，行为与 MT-3a 之前逐字节一致。

### 8.3 测试 / Tests

`run/fast_forward_tests.rs` 新增 12 个 A/B 测试（每例同固件、FF on vs off，比较
steps/time_us/PC/16 寄存器/SRAM 64 KiB/SysTick/TIMER/睡眠计数/DMA/HSTX/`[irq]`/
`[tail]`/通知）：

| 测试 / Test | 覆盖 / Property |
|---|---|
| `timer_alarm_sleep_is_fast_forwarded_and_matches_step_by_step` | WFI + TIMER alarm；跳过 >10 K 步且状态一致 |
| `repeated_segments_never_drift_from_the_step_by_step_path` | 12 段分段推进逐段一致 |
| `a_running_peer_blocks_fast_forward` | core1 运行 ⇒ 跳过 0 步 |
| `both_sleeping_cores_are_fast_forwarded_with_identical_state` | 双核睡 + 1 M 步睡眠通知逐字节相同 |
| `a_systick_expiry_wakes_at_the_same_step` | SysTick 到期在同一步唤醒 |
| `a_clock_change_while_sleeping_does_not_cross_the_boundary` | 睡眠中改 `CLK_SYS_DIV`，时基边界结算一致 |
| `dma_activity_blocks_fast_forward` | 忙 DMA ⇒ 跳过 0 步且传输计数一致 |
| `pio_activity_blocks_fast_forward` | 使能 PIO SM ⇒ 跳过 0 步 |
| `an_irq_asserted_between_segments_wakes_on_the_same_boundary` | 睡眠中置位中断 ⇒ 同一 64 步泵送唤醒 |
| `the_disabled_switch_never_skips_and_is_deterministic` | 关闭时跳过 0 步且两次运行一致，并与开启结果相同 |
| `a_blocking_fifo_read_is_never_fast_forwarded` | `BlockedRead` 逐步重试，帧到达后只消费一次 |
| `the_wake_step_never_crosses_the_next_event` | 32 步粒度扫描唤醒点，两机同段醒来且睡眠计数相同 |

### 8.4 计数器与性能 / Counters and performance

emerald 300 M（FF on 与 off 逐字段相同）：`time_us=1 798 617`、`dma=2 199 378`、
`hstx words=5 401 204 underruns=0 fifo=8`、`sleep core0 steps=224 994 790 wakes=6`、
`core1 Running pc=0x1000035C`、8 条 `[irq]`、final PC `0x1000B3AC`。
225 M：`time_us=1 500 000`、`sleep core0 steps=224 894 568 wakes=0`、`core1 Reset`、
final PC `0x1000B262`。FF on/off 的 300 M `run` stdout（56 行）逐字节相同，SRAM
520 KiB 的 SHA-256 相同，证明快进不改变客户机可见状态。

| 窗口 / Window | FF off（最小墙钟 / CPU） | FF on（最小墙钟 / CPU） | 加速比 |
|---|---|---|---|
| 0–225 M（启动睡眠） | 4.204 s / 4.078 s | **0.075 s / 0.062 s** | **≈56× / 66×** |
| 0–300 M（整跑） | 20.105 s / 18.734 s | **13.674 s / 12.578 s** | **1.47× / 1.49×** |

- 启动加速比在多轮测量中稳定在 51–66×；整跑在 1.33–1.49×（同轮交错最小值）。
- 启动阶段 224 994 790 个睡眠步中约 99.99 % 被跳过。MT-1 的 phase 0 占墙钟 30.7 %
  （上限 1.44×），本轮 off 运行的启动份额为 21–22 %（游戏段在本机更慢）；实测同轮
  比值 1.33–1.49× 说明绝对时间受宿主热/负载漂移影响。判据以同轮比值 + 逐字节计数器
  一致为准；300 M 窗口的 1.5× 需要 phase 1 同时提速（MT-3 双核线程化）。
- 游戏段不进入快进分支（有核可运行即回退），路径与 MT-3a 之前逐指令相同；分段
  差值在长跑热/负载漂移范围内，需静置机复测。
- 完整数据与口径见 `doc/performance.md` §11。

### 8.5 局限与后续 / Limits

- **仅 Arm Cortex-M33**：Hazard3 的 `mcycle`/`minstret` 每个睡眠步前进且是客户机可见
  CSR，`rp2350-riscv` 未提供批量记账入口，因此 RISC-V 保持逐步睡眠（FV-1 的步数/
  模拟时间与改动前完全相同）。
- **宿主诊断 `CpuCore::total_cycles()`** 只随真正执行的核步前进：快进跳过的睡眠步没有
  折叠进该核内计数（`rp2350-arm` 没有批量入口）。该值不是客户机可见寄存器（无 DWT
  CYCCNT 模型），也不出现在 `[run]` 报告中；`steps`、`time_us` 与全部外设计数不受影响。
- 若将来引入外设线程（MT-4），本模块的守卫即为「外设活动 ⇒ 退出快进」的现成契约。
- 通用性：判定基于架构状态（NVIC/SCB/外设忙闲），不依赖固件哈希、地址或固定步数；
  MicroPython（FV-2）、CircuitPython（FV-3）的 USB 启动路径已复跑 `stdout: 42`。

---

## 9. MT-5 交付记录：插件线程（2026-11-16）

## 9. MT-5 delivery: plugin threads (2026-11-16)

### 8.1 交付内容 / Delivered

| 内容 | 位置 |
|---|---|
| 邮箱协议 `Command` / `Response`、`WorkerContext`、工作线程循环（panic 隔离） | `crates/rp2350-virt-periph/src/executor_worker.rs` |
| 每设备执行器 `DeviceExecutor`（超时、高阻锁定、限期 join、分离诊断） | `crates/rp2350-virt-periph/src/executor_host.rs` |
| 内联/线程化双宿主 `DeviceHost`、绑定表与方向投递 | `crates/rp2350-virt-periph/src/executor_device.rs` |
| `PinSnapshot` / `PortBinding` / `ExecutorConfig` / `PublishSchedule` / `StepReport` / `StepError` | `crates/rp2350-virt-periph/src/executor.rs` |
| `DeviceManager::with_executor` / `with_latency` / `step` / `step_gpio` / `shutdown_threads` / 线程诊断 | `crates/rp2350-virt-periph/src/executor_manager.rs` |
| 管理器的内联/线程化查询、`ports_of`、`ConfigApplyError::Executor` | `manager.rs`、`reload.rs` |
| 19 项确定性/故障/性能测试（51 → 70） | `src/tests/executor.rs`、`executor_faults.rs`、`fixtures.rs` |

### 8.2 与任务书的差异 / Deviations

1. **没有 `DevicePlugin::step`**：现有 ABI 只有 `RpPlugin::tick` + `DevicePlugin::on_gpio_change`
   + `ports()`。MT-5 把「插件步进」定义为
   `deliver_inputs → tick(cycles) → collect_outputs` 三步，未改动 ABI。
   There is no `DevicePlugin::step`; the ABI only has `tick` + `on_gpio_change` +
   `ports()`. MT-5 defines a device step as `deliver_inputs → tick → collect_outputs`
   and leaves the ABI untouched.
2. **每个设备一个执行器**（任务书允许二选一）：卡死/panic 只影响该设备，3 台设备
   各自按 epoch 推进；管理器分两阶段（先派发、再收集），因此多设备是**真并行**，
   而不是串行等待每个邮箱。
   One executor per device (allowed by the task): a hung or panicking device only
   affects itself, and the manager dispatches to all devices before collecting, so
   multiple devices really run in parallel.
3. **`--plugin-threads` 未加**：`rp2350-cli` / `rp2350-egui` 不在本任务白名单内，
   开关以 `DeviceManager::with_executor` 形式提供；宿主接线留待后续任务。
   No `--plugin-threads`: the CLI and egui crates are outside this task's whitelist;
   the switch is `DeviceManager::with_executor` and host wiring is a follow-up.
4. **`mcp_query_all` / `mcp_query_device` 改为 `&mut self`**：线程化设备必须经邮箱
   查询；`devices()` 迭代器移除，替换为 `ports_of()`（内联与线程化都可用）。
   The MCP queries take `&mut self` and `devices()` was replaced by `ports_of()`,
   because worker-hosted plugins are only reachable through the mailbox.
5. **`BTreeMap` 取代 `HashMap` 存设备**：迭代顺序按 ID，`step` / `tick_all` /
   `mcp_query_all` 的输出顺序不再依赖哈希种子。
   Devices moved from `HashMap` to `BTreeMap` so iteration, stepping and MCP output
   are ID-ordered instead of hash-order dependent.

### 8.3 延迟语义 / Latency semantics

- 设备在 `epoch` 看到输入快照；其输出在 `epoch + latency_ticks` 生效
  （默认 1 个外设 tick，可配置，最小 1）。延迟队列 `PublishSchedule` 在**模拟线程**
  上以 epoch 计量，不使用宿主时间。
- `StepReport::produced` 是各设备本 epoch 产生的输出（可逐设备断言互不串扰）；
  `StepReport::effective` 是已成熟输出的线或，`step_gpio` 只把 `driven_pins`
  覆盖的引脚写回 GPIO。
- 被跳过的 epoch 在下次调用时一并释放（线或），不合并输入、不丢失输出。
- 确定性对照：同一输入序列，同步与线程化路径在 latency 1 与 3 下逐 pin 相同
  （`sync_and_threaded_paths_match_pin_for_pin`、`latency_three_shifts_outputs_on_both_paths`），
  且真实 LED `cdylib` 两条路径状态一致（`led_plugin_matches_between_paths`）。

### 8.4 超时 / panic 降级 / Degradation

| 情况 | 行为 |
|---|---|
| `step_timeout` 内无响应 | 设备锁定高阻，`StepReport::degraded` + 稳定诊断行；后续步进不再等待 |
| 插件 `tick`/`apply_config`/`state_snapshot` panic | 工作线程按命令 `catch_unwind`，标记中毒，设备高阻；其它设备不受影响 |
| 关闭期限内不响应 | 工作线程分离（`is_detached()`），库句柄**有意泄漏**保持 vtable 映射（Rust 无法 kill 线程） |
| 关闭正常 | 先 `Shutdown` 取回插件 box 并 join，再释放库（Windows DLL 锁） |

跨 `cdylib` 的 panic 限制保持不变：`panic = "abort"` 或展开穿越 `extern "C"` 帧
仍会终止进程。诊断可通过 `thread_count()` / `degraded_devices()` /
`degraded_reasons()` / `DeviceExecutor::timeouts()` 获取。

### 8.5 对照测试与性能 / Comparison and performance

测试（`cargo test -p rp2350-virt-periph --all-features`，**74 passed**，新增 4/替换 1）：

| 类别 | 测试 |
|---|---|
| 编码/延迟 | `pin_snapshot_encodes_gpio_levels`、`publish_schedule_holds_fixed_latency`、`default_executor_config_is_a_stall_bound` |
| 同步 vs 线程化 | `sync_and_threaded_paths_match_pin_for_pin`、`latency_three_shifts_outputs_on_both_paths`、`led_plugin_matches_between_paths` |
| 多设备独立 | `three_devices_keep_independent_outputs`、`unregister_stops_only_its_worker`、`unregister_joins_before_returning` |
| 方向/接线 | `port_direction_controls_delivery_and_publish`、`step_gpio_only_drives_wired_outputs`、`unwired_device_still_advances` |
| 协议 | `non_monotonic_epoch_is_rejected`、`synchronous_default_spawns_no_threads` |
| 故障 | `timeout_latches_device_high_z_without_stopping_simulation`、`panic_is_isolated_to_the_failing_device` |
| 重载/关闭 | `reload_swaps_worker_hosted_device_and_keeps_wiring`、`shutdown_threads_reclaims_all_workers`、`dropping_threaded_manager_stops_workers` |
| 配置 | `configure_reaches_the_worker_thread` |
| 并行/负载 | `parallel_dispatch_is_proven_by_rendezvous_plugins`、`sixteen_host_threads_do_not_starve_device_workers`、`heavy_plugins_match_sequential_state` |

性能（debug 构建、3 台重插件、每 tick 400 000 次整数运算、10 epoch、16 核）：
同步 ~112 ms、线程化 ~44 ms（比值为**信息性**输出，不再设硬断言）。宿主时间比值
度量的是机器负载而非执行器，`cargo test` 与并行构建会污染它；真正的并行派发由
`parallel_dispatch_is_proven_by_rendezvous_plugins` 用三方屏障确定性证明（串行派发
会死锁并触发超时），重插件用例只断言两条路径状态一致且无设备降级。
Debug build, 3 heavy plugins (400 000 integer ops per tick), 10 epochs, 16 cores:
~112 ms vs ~44 ms. The wall-clock ratio is **informational** and no longer asserted,
because it measures machine load rather than the executor; parallel dispatch is
proven deterministically by the rendezvous-barrier test, and the heavy case only
asserts identical final state with no degraded device.

### 8.6 门禁 / Gates

```
cargo test -p rp2350-virt-periph --all-features     74 passed
cargo fmt -p rp2350-virt-periph -- --check          clean
cargo clippy -p rp2350-virt-periph --all-targets --all-features -- -D warnings -W clippy::pedantic  clean
cargo doc -p rp2350-virt-periph --no-deps           clean
list-devices --plugin-dir target/debug              com.rp2350-rs.led / LED / has_ui=false / ports=[LED]
FV-1 / FV-2 / FV-3                                  [repl] stdout: 42
```

`cargo test --all-features`：**40 个 target 全绿**（含 `rp2350-periph` 的
`boot::tests::core1_release_pending_tracks_the_ack_owed_to_the_mailbox`；此前那次
失败来自并行任务的未提交 `boot.rs` 改动，现已在对方工作区修复）。
The workspace run is **green across all 40 targets**, including the
`rp2350-periph` boot test that previously failed due to a concurrent task's
uncommitted change.

### 8.7 并行测试 flake 根因与修复（2026-11-16）

**症状 / Symptom.** 默认并行 `cargo test -p rp2350-virt-periph --all-features`
偶发 69/70：`tests::executor_faults::unregister_stops_only_its_worker` 在
`report.produced.len()` 处 **left: 1, right: 2**；单独运行或低负载全绿。

**复现 / Reproduction.** 空闲 30/30 全绿；用 20 个自旋进程压满 16 个逻辑核后
跑 10 轮，出现 2 次失败（`heavy_plugins_*`：`heavy-c.ticks` 为 `Null`，即设备已
降级；另一轮是宿主时间比值断言）。修复前失败输出：
`left: Number(11) / right: Null` 与 `parallel < sequential` 断言失败。

**根因 / Root cause.** `ExecutorConfig::default().step_timeout` 原为 **500 ms**。
它是**宿主调度上限**而不是卡死上限：并行测试/构建让一个健康 worker 超过 500 ms
拿不到 CPU，执行器按设计把设备锁为高阻，`StepReport.produced` 又只收录成功设备，
于是输出少一台。测量排除了其它猜测：

- `unregister` **已同步 join**：`DeviceExecutor::shutdown` 收到 `Shutdown` 后
  `join` 再返回；`tracked` 存活计数在 `unregister` 返回时已从 3 减到 2。
- 派发/收集没有竞态：`step` 在 `&mut self` 上先派发当前 `BTreeMap` 的全部设备，
  再按 ID 收集；`unregister` 同步移除条目，不会向已注销设备派发。
- `PublishSchedule` 以 epoch 计量，epoch 0 不跳过任何设备。
- 真正失败标志来自 `degraded_reasons()`：`... timed out after 500 ms ... high-Z`。

**修复 / Fix.**

1. 默认 `step_timeout` 提高到 **5 s**（`DEFAULT_STEP_TIMEOUT`），只用于检测真正
   卡死；降级用例 `timeout_latches_device_high_z_...` 在接线完成后用
   `set_step_timeout("slow", 400 ms)` 只收紧卡死设备，健康设备仍享 5 s 余量
   （此前把 400 ms 套在整个管理器上，连健康设备的 `set_bindings` 都会被误判）。
2. `StepReport.produced` 改为**每台已注册设备一个条目**：高阻设备携带
   `PinSnapshot::empty()`（不驱动任何引脚）并同时出现在 `degraded`，不再静默消失。
3. 测试改用生产默认配置；新增 `parallel_dispatch_is_proven_by_rendezvous_plugins`
   （三方屏障确定性证明先派发后收集）、`sixteen_host_threads_do_not_starve_device_workers`
   （16 宿主线程压力下 8 设备全部产出）、`unregister_joins_before_returning`
   （返回即 join 并释放插件）、`default_executor_config_is_a_stall_bound`。
4. 删除宿主时间比值硬断言（它度量机器负载而非执行器）；重插件用例只保留状态一致性
   与「无降级」断言，墙钟比值仅打印。

**验证 / Verification.** 默认并行连跑 30/30 全绿；`--test-threads=1` 全绿、
`--test-threads=4` 5/5 全绿；20 个自旋进程压力下共 15 轮全绿。

### 8.8 剩余卡点 / Remaining

1. 宿主接线：CLI `--plugin-threads` 与虚拟实验室（多设备）尚未启用线程化。
2. 长时间运行（soak）与真实重插件（如 uart-terminal）的线程化吞吐未测。
3. 无法终止卡死线程 → 分离 + 泄漏库句柄；若需要硬隔离必须进程外执行。
4. 工作线程 panic 隔离仅覆盖 Rust 展开；`panic = "abort"` / 跨 `extern "C"` 不支持。

