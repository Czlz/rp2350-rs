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
| MT-3 | **双核线程化**：`Machine` 拆为两台核线程 + 仲裁器；中断/事件跨线程投递；睡眠核不占调度 | `rp2350-cli` + `rp2350-arm` | FV-1/2/3 与 emerald 300 M 计数器一致；默认并行运行 30 次无 flake；吞吐报告 |
| MT-4 | **外设线程**：DMA/PIO/HSTX/TIMER 独立推进与 MMIO 邮箱；事件定序 | `rp2350-periph` + `rp2350-core` | 与单线程逐周期对照（同一时间点状态一致）；emerald HSTX `underruns=0` |
| MT-5 | **插件线程**：`virt-periph` 邮箱 + 固定延迟；超时/崩溃降级 | `rp2350-virt-periph` + `rp2350-plugin` | 与同步路径逐 pin 对照；重载/删除不崩溃；虚拟实验室示例 |
| MT-6 | **性能与回归收口**：长跑 soak、文档、`doc/performance.md` 新口径 | 主会话 | 数据 + 结论 |

依赖：MT-1 → MT-2 → MT-3 ∥ MT-4，MT-5 可与 MT-3 并行（crate 不相交）。

> **MT-1 已交付**：逐核/逐区域实测、锁微基准、双线程原型与 go/no-go 见 §6。
> MT-1 delivered: per-core/per-region measurements, lock microbenchmarks, the
> two-thread spike and the go/no-go decision are in §6.

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
