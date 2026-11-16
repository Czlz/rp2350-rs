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
