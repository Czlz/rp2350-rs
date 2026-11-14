# rp2350-periph — Public API / 公共 API

> Built-in RP2350 peripherals: PIO, HSTX, DMA, XIP (QMI/Flash/PSRAM), UART, SPI, I2C, PWM, ADC, SIO, Clocks, Timer, Watchdog, USB.
> RP2350 内置外设：PIO、HSTX、DMA、XIP（QMI/Flash/PSRAM）、UART、SPI、I2C、PWM、ADC、SIO、时钟、定时器、看门狗、USB。

---

## PIO register blocks / PIO 寄存器块

RP2350 has **three** PIO blocks, unlike RP2040's two. `BootPeripherals::mount`
installs one shared block handle per block at its AHB base address:

| Block | Base | Constant |
|-------|------|----------|
| PIO0 | `0x5020_0000` | `map::PIO0_BASE` |
| PIO1 | `0x5030_0000` | `map::PIO1_BASE` |
| PIO2 | `0x5040_0000` | `map::PIO2_BASE` |

RP2350 与 RP2040 的两块 PIO 不同，共有 **三个** PIO 块。`BootPeripherals::mount`
在每个块的 AHB 基地址各挂载一个共享块句柄。

```rust
pub struct Pio { /* private register file + four state machines */ }
pub struct SharedPio { /* Arc<Mutex<Pio>> + two interrupt lines */ }

impl Pio {
    pub fn new(index: u8) -> Self;
    pub fn push_rx(&mut self, sm: usize, value: u32) -> bool;
    pub fn pop_tx(&mut self, sm: usize) -> Option<u32>;
    pub fn tx_fifo_empty(&self, sm: usize) -> Option<bool>;
    pub fn tx_fifo_full(&self, sm: usize) -> Option<bool>;
    pub fn rx_fifo_empty(&self, sm: usize) -> Option<bool>;
    pub fn rx_fifo_full(&self, sm: usize) -> Option<bool>;
    pub fn step_sm(&mut self, sm: usize) -> bool;
    pub fn tick(&mut self, cycles: u64);
    pub fn set_input_levels(&mut self, levels: u32);
    pub fn input_levels(&self) -> u32;
    pub fn set_input_level(&mut self, pin: u8, high: bool);
    pub fn output_levels(&self) -> u32;
    pub fn output_enables(&self) -> u32;
}

impl SharedPio {
    pub fn new(index: u8, irq: IrqAssert) -> Self;
    pub fn index(&self) -> u8;
    pub fn device(&self) -> Box<dyn MmioDevice>;
    pub fn with<R>(&self, f: impl FnOnce(&Pio) -> R) -> R;
    pub fn with_mut<R>(&self, f: impl FnOnce(&mut Pio) -> R) -> R;
    pub fn tick(&self, cycles: u64);
    pub fn sync_irqs(&self);
    pub fn irq_flags(&self) -> u32;
    pub fn enabled_mask(&self) -> u8;
    pub fn stalled_mask(&self) -> u8;
    pub fn pc(&self, sm: usize) -> Option<u8>;
    pub fn tx_fifo_empty(&self, sm: usize) -> Option<bool>;
    pub fn tx_fifo_full(&self, sm: usize) -> Option<bool>;
    pub fn rx_fifo_empty(&self, sm: usize) -> Option<bool>;
    pub fn rx_fifo_full(&self, sm: usize) -> Option<bool>;
    pub fn output_levels(&self) -> u32;
    pub fn output_enables(&self) -> u32;
}
```

`Pio` implements `MmioDevice` over a 4 KB window. `new(0/1/2)` selects
PIO0/PIO1/PIO2; any other index panics. The implementation is split into
`pio/mod.rs` (register block, pin model), `pio/regs.rs` (register layout, FIFOs,
state), `pio/shared.rs` (shared handle, MMIO proxy, IRQ routing), `pio/sm.rs`
(clocking, delay/side-set, JMP/WAIT/IRQ/SET) and `pio/shift.rs`
(IN/OUT/PUSH/PULL/MOV).
`Pio` 在 4 KB 窗口上实现 `MmioDevice`。`new(0/1/2)` 选择 PIO0/PIO1/PIO2，
其它序号会 panic。实现拆分为 `pio/mod.rs`（寄存器块、引脚模型）、
`pio/regs.rs`（寄存器布局、FIFO、状态）、`pio/shared.rs`（共享句柄、MMIO 代理、
中断路由）、`pio/sm.rs`（时钟驱动、延迟/side-set、JMP/WAIT/IRQ/SET）以及
`pio/shift.rs`（IN/OUT/PUSH/PULL/MOV）。

### Shared handles / 共享句柄

`BootPeripherals::mount` wraps each block in one [`SharedPio`] and mounts the
proxy returned by `SharedPio::device`. The proxy and the handle share **one**
register file (`Arc<Mutex<Pio>>`), so firmware MMIO writes are immediately
visible to the host and a host `tick` advances the state machines the guest
programmed — no register, FIFO or pin state is copied. `BootPeripherals::pio(i)`
returns the handle, `with_pio(i, f)` runs a closure against the register file
and republishes the IRQ lines, and `pump_pio_irqs()` refreshes all six lines
from the raw `IRQ` flags. `SharedPio::tick` acquires the lock once, returns
immediately when `CTRL.SM_ENABLE = 0`, and only rewrites the shared interrupt
lines when the `IRQ` flags changed.

`BootPeripherals::mount` 把每个块包装成一个 [`SharedPio`]，并挂载
`SharedPio::device` 返回的代理。代理与句柄共享**同一份**寄存器文件
（`Arc<Mutex<Pio>>`），因此固件的 MMIO 写入对宿主立即可见，宿主的 `tick`
推进的正是客户机编程的状态机 —— 寄存器、FIFO 与引脚状态都不会被复制。
`BootPeripherals::pio(i)` 返回句柄，`with_pio(i, f)` 在寄存器文件上执行闭包并
重新发布中断线，`pump_pio_irqs()` 按原始 `IRQ` 标志刷新全部六条线。
`SharedPio::tick` 只加锁一次，`CTRL.SM_ENABLE = 0` 时立即返回，且仅在 `IRQ`
标志变化时才改写共享中断线。

### Interrupt routing / 中断路由

Each block owns two interrupt lines at RP2350 numbers `15..=20`
(`irq::PIO0_IRQ_0` = 15 … `irq::PIO2_IRQ_1` = 20, table
`irq::PIO_IRQ_LINES[block][line]`), immediately after `USBCTRL_IRQ` (14) and
before `IO_IRQ_BANK0` (21). The eight `IRQ` flags are grouped as on silicon:
flags `0..=3` assert the block's `IRQ_0` line and flags `4..=7` assert `IRQ_1`
(the datasheet's `IRQ0_INTE`/`IRQ1_INTE` grouping). The model does not implement
those enable/force registers, so any set flag asserts its line directly. Every
path that can change a flag republishes the pair: the `IRQ` instruction executed
inside `tick`, `IRQ_FORCE` writes, `IRQ` write-one-to-clear writes, and every
`with_mut` call, so clearing a flag releases the line in the same operation.
`Pio` used directly (unit tests, standalone mounts) has no IRQ routing; only
`SharedPio` drives the shared `IrqAssert`.

每个块在 RP2350 中断号 `15..=20` 上拥有两条中断线（`irq::PIO0_IRQ_0` = 15 …
`irq::PIO2_IRQ_1` = 20，表为 `irq::PIO_IRQ_LINES[block][line]`），正好位于
`USBCTRL_IRQ`（14）之后、`IO_IRQ_BANK0`（21）之前。八个 `IRQ` 标志按硬件分组：
标志 `0..=3` 断言块的 `IRQ_0` 线，标志 `4..=7` 断言 `IRQ_1`（数据手册的
`IRQ0_INTE`/`IRQ1_INTE` 分组）。模型不实现这些使能/强制寄存器，因此任何已置位
标志都会直接断言对应线。所有可能改变标志的路径都会重新发布这一对线：`tick`
内执行的 `IRQ` 指令、`IRQ_FORCE` 写入、`IRQ` 写 1 清零写入，以及每次
`with_mut` 调用，因此清标志与释放线是同一次操作。直接使用 `Pio`（单元测试、
独立挂载）没有中断路由；只有 `SharedPio` 会驱动共享 `IrqAssert`。

### Tick semantics / tick 语义

`SharedPio::tick(cycles)` models `cycles` PIO clock cycles and matches
`Pio::tick`: each cycle gives every enabled machine one clock edge, `CLKDIV`
gates the machine cycle, and a pending instruction delay is burned before the
next fetch. The host decides the cycle budget; `rp2350-cli` hands one PIO cycle
per retired guest instruction (see `doc/crates/rp2350-cli.md`). `Pio::tick`
takes `&mut self` for standalone use, while the shared handle takes `&self`
because the register file lives behind the mutex.

`SharedPio::tick(cycles)` 建模 `cycles` 个 PIO 时钟周期，语义与 `Pio::tick`
一致：每个周期给每台已使能状态机一个时钟边沿，`CLKDIV` 决定是否产生机器周期，
待处理指令延迟在下次取指前消耗。周期预算由宿主决定；`rp2350-cli` 按每条退役的
客户机指令给出一个 PIO 周期（见 `doc/crates/rp2350-cli.md`）。独立使用时
`Pio::tick` 取 `&mut self`，共享句柄取 `&self`，因为寄存器文件位于互斥锁之后。

### Pin model / 引脚模型

The block owns three 32-bit pin vectors: input levels sampled by the state
machines, output levels driven by them, and output enables. All three reset to
zero, so a block with no host pin activity still behaves like the earlier
"no pins attached" model (`IN PINS`/`WAIT`/`JMP PIN` read 0).

- `set_input_levels(u32)` / `input_levels()` / `set_input_level(pin, high)`
  drive the sampled inputs from the host side; pins `>= 32` are ignored.
- `output_levels()` and `output_enables()` expose what `OUT`/`SET`/side-set
  wrote, and are mirrored in `DBG_PADOUT`/`DBG_PADOE`.
- Pin numbers wrap from 31 back to 0, as on the hardware.
- `GPIOBASE` is stored but not applied to the pin mapping yet, so PINCTRL pin
  numbers index this block's local 0..31 window.

该块持有三个 32 位引脚向量：状态机采样的输入电平、状态机驱动的输出电平与输出
使能。三者复位均为 0，因此没有主机侧引脚活动时仍保持早期“未接引脚”模型的语义
（`IN PINS`/`WAIT`/`JMP PIN` 读 0）。

- `set_input_levels(u32)` / `input_levels()` / `set_input_level(pin, high)`
  由主机侧驱动采样输入；`>= 32` 的引脚被忽略。
- `output_levels()` 与 `output_enables()` 暴露 `OUT`/`SET`/side-set 写入的
  结果，并反映在 `DBG_PADOUT`/`DBG_PADOE`。
- 引脚编号按硬件行为从 31 回绕到 0。
- `GPIOBASE` 仅存储、尚未作用于引脚映射，因此 PINCTRL 引脚号索引本块局部的
  0..31 窗口。

### Modelled registers / 已建模寄存器

- `CTRL` — `SM_ENABLE[3:0]` is read back; `SM_RESTART[7:4]` resets the addressed
  state machine's PC, shift counters, clock-divider phase and pending delay
  (FIFOs are preserved); `CLKDIV_RESTART[11:8]` clears that machine's divider
  phase.
  `CTRL` — 可读回 `SM_ENABLE[3:0]`；`SM_RESTART[7:4]` 复位对应状态机的 PC、
  移位计数、分频相位与剩余延迟（FIFO 内容保留）；`CLKDIV_RESTART[11:8]`
  清零对应状态机的分频相位。
- `FSTAT` — reset `0x0F00_0F00`; per-SM `RXFULL[3:0]`, `RXEMPTY[11:8]`,
  `TXFULL[19:16]`, `TXEMPTY[27:24]` are derived live from FIFO occupancy. A
  direction disabled by FIFO joining reports both full and empty, as the
  datasheet requires.
  `FSTAT` — 复位值 `0x0F00_0F00`；每个状态机的 `RXFULL`、`RXEMPTY`、
  `TXFULL`、`TXEMPTY` 由 FIFO 占用实时推导。被 FIFO 合并禁用的一侧同时报告
  “满”和“空”，与数据手册一致。
- `FDEBUG` — sticky `RXSTALL`/`RXUNDER`/`TXOVER`/`TXSTALL` flags,
  write-one-to-clear. Stalls raised by `PULL`/`OUT` (empty TX) and
  `PUSH`/autopush (full RX) set `TXSTALL`/`RXSTALL` for the stalled machine.
  `FDEBUG` — 粘滞 `RXSTALL`/`RXUNDER`/`TXOVER`/`TXSTALL` 标志，写 1 清零。
  `PULL`/`OUT`（TX 空）与 `PUSH`/自动压入（RX 满）造成的阻塞会为对应状态机
  置位 `TXSTALL`/`RXSTALL`。
- `FLEVEL` — 4-bit TX/RX occupancy per state machine, derived from the FIFOs; a
  joined FIFO reports up to 8 words.
  `FLEVEL` — 每个状态机的 4 位 TX/RX 占用，由 FIFO 推导；合并后的 FIFO 最多
  报告 8 字。
- `TXF0..TXF3` — each write pushes one word into the 4-deep TX FIFO (8 deep when
  `FJOIN_TX`); a write on a full FIFO drops the word and sets `FDEBUG.TXOVER`.
  `RXF0..RXF3` — each read pops one word from the 4-deep RX FIFO (8 deep when
  `FJOIN_RX`); a read on an empty FIFO returns zero and sets `FDEBUG.RXUNDER`.
  `TXF0..TXF3` — 每次写向 4 深度 TX FIFO（`FJOIN_TX` 时 8 深度）压入一个字；
  满时丢弃该字并置位 `FDEBUG.TXOVER`。`RXF0..RXF3` — 每次读从 4 深度 RX FIFO
  （`FJOIN_RX` 时 8 深度）取出一个字；空时返回 0 并置位 `FDEBUG.RXUNDER`。
- `IRQ` / `IRQ_FORCE` — eight PIO IRQ flags; `IRQ` is W1C, `IRQ_FORCE` sets
  the internal flags. The `IRQ` instruction sets/clears/waits on these flags.
  With `SharedPio`, flags `0..=3` drive the block's `IRQ_0` line and flags
  `4..=7` drive `IRQ_1` (see *Interrupt routing* above).
  `IRQ` / `IRQ_FORCE` — 8 个 PIO IRQ 标志；`IRQ` 写 1 清零，`IRQ_FORCE` 置位
  内部标志。`IRQ` 指令可置位/清零/等待这些标志。使用 `SharedPio` 时，标志
  `0..=3` 驱动本块 `IRQ_0` 线，标志 `4..=7` 驱动 `IRQ_1`（见上文
  *Interrupt routing / 中断路由*）。
- `INPUT_SYNC_BYPASS`, `GPIOBASE` (`bit 4`), `DBG_CFGINFO` (`0x1020_0404`),
  `DBG_PADOUT`/`DBG_PADOE` (sample the current output levels/enables).
  `INPUT_SYNC_BYPASS`、`GPIOBASE`（位 4）、`DBG_CFGINFO`（`0x1020_0404`）、
  `DBG_PADOUT`/`DBG_PADOE`（采样当前输出电平/输出使能）。
- `INSTR_MEM0..INSTR_MEM31` — 32 × 16-bit instruction slots. The hardware marks
  these write-only; this model permits readback for debugging.
  `INSTR_MEM0..INSTR_MEM31` — 32 × 16 位指令槽。硬件标称只写；本模型允许回读以便调试。
- `SM0..SM3` windows — `CLKDIV` (16.8 fixed point, reset `0x0001_0000`),
  `EXECCTRL` (reset `0x0001_F000`; `WRAP_TOP`/`WRAP_BOTTOM` at bits 16:12/11:7,
  `JMP_PIN` at 28:24, `SIDE_PINDIR` at 29, `SIDE_EN` at 30, `EXEC_STALLED` at
  bit 31), `SHIFTCTRL` (reset `0x000C_0000`; `FJOIN_RX` at 31, `FJOIN_TX` at
  30), `ADDR` (read-only, current PC), `INSTR` (reads the instruction at PC; a
  write executes it immediately), `PINCTRL` (reset `0x1400_0000`).
  `SM0..SM3` 窗口 — `CLKDIV`（16.8 定点，复位 `0x0001_0000`）、`EXECCTRL`
  （复位 `0x0001_F000`；`WRAP_TOP`/`WRAP_BOTTOM` 位于位 16:12/11:7，`JMP_PIN`
  位于 28:24，`SIDE_PINDIR` 位于 29，`SIDE_EN` 位于 30，`EXEC_STALLED` 位于
  位 31）、`SHIFTCTRL`（复位 `0x000C_0000`；`FJOIN_RX` 位于 31，`FJOIN_TX`
  位于 30）、`ADDR`（只读，当前 PC）、`INSTR`（读取 PC 处指令；写入会立即
  执行）、`PINCTRL`（复位 `0x1400_0000`）。
- `SMx_PINCTRL` fields — `OUT_BASE[4:0]`, `SET_BASE[9:5]`, `SIDESET_BASE[14:10]`,
  `IN_BASE[19:15]`, `OUT_COUNT[25:20]`, `SET_COUNT[28:26]`,
  `SIDESET_COUNT[31:29]` (clamped to the legal 0..=5).
  `SMx_PINCTRL` 位域 — `OUT_BASE[4:0]`、`SET_BASE[9:5]`、`SIDESET_BASE[14:10]`、
  `IN_BASE[19:15]`、`OUT_COUNT[25:20]`、`SET_COUNT[28:26]`、
  `SIDESET_COUNT[31:29]`（钳到合法范围 0..=5）。

`push_rx` feeds the RX FIFO from the device side and `pop_tx` drains the TX
FIFO as a state machine would; both are join-aware and return `false`/`None` on
a full/empty (or joined-away) FIFO and set the corresponding sticky debug flag.
`push_rx` 从设备侧向 RX FIFO 送入数据，`pop_tx` 按状态机行为取走 TX FIFO
数据；二者都感知 FIFO 合并，在 FIFO 满/空（或已被合并禁用）时分别返回
`false`/`None` 并置位对应粘滞调试标志。

### FIFO joining / FIFO 合并

`SHIFTCTRL.FJOIN_TX` (bit 30) makes the TX FIFO steal the RX storage and grow
to 8 words; `SHIFTCTRL.FJOIN_RX` (bit 31) does the same for RX. The joined-away
direction is disabled and then reads as both full and empty, and a change of
either join bit flushes both FIFOs, matching the datasheet. Setting both bits
at once is the useless-but-safe state where both directions read full and
empty. `FSTAT`, `FLEVEL`, `TXF*`/`RXF*`, `push_rx`/`pop_tx` and the
state-machine `PUSH`/`PULL`/`OUT` paths all follow the resulting capacity.

`SHIFTCTRL.FJOIN_TX`（位 30）让 TX FIFO 占用 RX 的存储并扩展到 8 字；
`SHIFTCTRL.FJOIN_RX`（位 31）对 RX 做同样的合并。被合并掉的方向被禁用，随后
同时读为“满”和“空”；任一位变化都会清空两个 FIFO，与数据手册一致。两位同时
置位是“无用但安全”的状态：两个方向都读为满和空。`FSTAT`、`FLEVEL`、
`TXF*`/`RXF*`、`push_rx`/`pop_tx` 以及状态机的 `PUSH`/`PULL`/`OUT` 路径都
按合并后的容量工作。

### DMA requests / DREQ / DMA 请求

Each state machine exposes one TX and one RX request line. `DMA
CTRL_TRIG.TREQ_SEL` selects them with the RP2350 table (`rp2350_periph::dreq`),
which differs from RP2040 because RP2350 adds a third block:

| Selectors | DREQ lines | Selectors | DREQ lines |
|-----------|------------|-----------|------------|
| `0..=3` | `PIO0_TX0..TX3` | `4..=7` | `PIO0_RX0..RX3` |
| `8..=11` | `PIO1_TX0..TX3` | `12..=15` | `PIO1_RX0..RX3` |
| `16..=19` | `PIO2_TX0..TX3` | `20..=23` | `PIO2_RX0..RX3` |

The level semantics follow the datasheet: `PIOx_TXy` is asserted while state
machine `y`'s TX FIFO is **not full** (the DMA channel may write a word), and
`PIOx_RXy` is asserted while its RX FIFO is **not empty** (the channel may read
a word). A joined FIFO is one 8-deep FIFO: the joined direction asserts its
request up to 8 words, while the joined-away direction reads as both full and
empty and therefore keeps its request deasserted. With both join bits set both
requests stay low.

每个状态机各有一条 TX 与一条 RX 请求线，由 `DMA CTRL_TRIG.TREQ_SEL` 按 RP2350
表（`rp2350_periph::dreq`）选择；RP2350 比 RP2040 多一个 PIO 块，因此编号不与
RP2040 兼容。电平语义遵循数据手册：`PIOx_TXy` 在状态机 `y` 的 TX FIFO **未满**
时断言（DMA 通道可写入一个字），`PIOx_RXy` 在其 RX FIFO **非空**时断言（通道可
读出一个字）。合并后的 FIFO 是一个 8 深度 FIFO：被合并的方向最多 8 字断言请求，
被合并掉的方向同时读为满和空，因此其请求保持去断言。两个 join 位同时置位时两条
请求都为低。

`Pio::tx_fifo_empty`/`tx_fifo_full`/`rx_fifo_empty`/`rx_fifo_full` (and the
[`SharedPio`] mirrors) are read-only views of the live FIFOs — the same state
`FSTAT`/`FLEVEL` derive from, so nothing is copied and a joined FIFO is judged
at its 8-word depth. Out-of-range machine indices return `None`.
`BootPeripherals::sync_pio_dreqs()` walks the three blocks, reads those four
queries under one lock per block and publishes a TX and an RX bitmask into the
DMA level array with `Dma::set_dreq_bank`; only selectors `0..=23` are touched.
`BootPeripherals::pump_dma(bus, units)` then republishes the levels and runs
the controller, advancing a channel paced by a PIO selector one unit at a time
so a FIFO that fills or drains inside the budget is noticed before the next
unit. `Dma::busy_selects_dreq(first, last)` is the probe that chooses that
granularity.
`Pio::tx_fifo_empty`/`tx_fifo_full`/`rx_fifo_empty`/`rx_fifo_full`（以及
[`SharedPio`] 上的同名方法）是实时 FIFO 的只读视图 —— 与 `FSTAT`/`FLEVEL`
同源，因此不复制状态，合并后的 FIFO 按 8 深度判定；越界状态机返回 `None`。
`BootPeripherals::sync_pio_dreqs()` 遍历三个块，每块一次加锁读取上述四个查询，
并用 `Dma::set_dreq_bank` 把 TX/RX 位掩码发布到 DMA 电平数组，只写选择器
`0..=23`。`BootPeripherals::pump_dma(bus, units)` 随后重新发布电平并运行控制器，
对由 PIO 选择器节拍的通道逐单元推进，从而在下一个单元前发现批次内被填满或排空的
FIFO；`Dma::busy_selects_dreq(first, last)` 是选择该粒度的探测接口。

### State-machine execution / 状态机执行

`step_sm(sm)` advances one machine by exactly one PIO clock edge. Every edge
first advances that machine's `CLKDIV` phase accumulator (1.0 or faster ticks
every cycle), then burns one pending instruction-delay cycle if present. Only
when both gates pass does the machine emit side-set and execute one
instruction; `true` means an instruction retired, `false` means the edge was
gated or spent on a delay. `step_sm` ignores `CTRL.SM_ENABLE` so tests can
drive a single machine; `tick(cycles)` applies the same edge to every enabled
machine.

Delay cycles come from the bits of the delay/side-set field that are not
allocated to side-set (`5 - SIDESET_COUNT`, or all five bits when
`SIDESET_COUNT = 0`). After an instruction retires, its delay is burned before
the next fetch, so an instruction with delay `N` occupies `N + 1` machine
cycles.

`step_sm(sm)` 让单台状态机推进恰好一个 PIO 时钟边沿。每个边沿先推进该机的
`CLKDIV` 相位累加器（1.0 或更快时每个周期都推进），再在存在剩余延迟时消耗一个
延迟周期。两道门都通过后才会输出 side-set 并执行一条指令；返回 `true` 表示指令
退休，返回 `false` 表示本边沿被分频拦住或用于消耗延迟。`step_sm` 忽略
`CTRL.SM_ENABLE` 以便测试单台状态机；`tick(cycles)` 对每台已启用状态机施加同样
的边沿。

延迟周期来自共享延迟/side-set 字段中未分配给 side-set 的位（`5 - SIDESET_COUNT`，
`SIDESET_COUNT = 0` 时为全部 5 位）。指令退休后先消耗其延迟再取下一条，因此带
延迟 `N` 的指令占用 `N + 1` 个机器周期。

`CLKDIV` is interpreted as `INT + FRAC/256` (16.8 fixed point). The model keeps
a per-machine phase accumulator in 1/256 clock units and emits a machine cycle
whenever it wraps. The reset value `0x0001_0000` is 1.0, so one PIO clock equals
one instruction by default; `0x0002_0000` halves the rate and `0x0001_8000`
gives 1.5. A divisor of 1.0 or below — including the invalid `INT = 0` — ticks
every cycle so a zero divisor cannot deadlock the model.

`CLKDIV` 按 `INT + FRAC/256`（16.8 定点）解释。模型为每台状态机维护 1/256 时钟
单位的相位累加器，回绕时产生一个机器周期。复位值 `0x0001_0000` 为 1.0，因此
默认一个 PIO 时钟对应一条指令；`0x0002_0000` 减半，`0x0001_8000` 为 1.5。
分频比 ≤ 1.0（含非法的 `INT = 0`）时每周期都推进，避免零分频让模型死锁。

Side-set occupies the top `SIDESET_COUNT` bits of the shared field and is
emitted at the start of the instruction to `SIDESET_BASE..`; with
`EXECCTRL.SIDE_EN` the top bit of the field is instead an enable strobe and one
fewer bit carries data. `EXECCTRL.SIDE_PINDIR` redirects side-set to the output
enables. The reset configuration (`SIDESET_COUNT = 0`) treats the whole field
as delay, so existing programs are unaffected.

side-set 占用共享字段的高 `SIDESET_COUNT` 位，在指令起始处输出到
`SIDESET_BASE..`；置位 `EXECCTRL.SIDE_EN` 时字段最高位改为使能脉冲，数据位少
一位。`EXECCTRL.SIDE_PINDIR` 把 side-set 改写到输出使能。复位配置
（`SIDESET_COUNT = 0`）把整个字段当作延迟，因此既有程序不受影响。

| Instruction | Behaviour / 行为 |
|-------------|------------------|
| `JMP` | All conditions: always, `!x`, `x--`, `!y`, `y--`, `x!=y`, `pin` (samples `EXECCTRL.JMP_PIN`), `!osre`. `x--`/`y--` post-decrement / 支持全部条件：always、`!x`、`x--`、`!y`、`y--`、`x!=y`、`pin`（采样 `EXECCTRL.JMP_PIN`）、`!osre`；`x--`/`y--` 为后递减 |
| `WAIT` | `GPIO` samples its absolute pin index, `PIN` samples `IN_BASE + index` (both wrap at 31); IRQ source waits for the flag / `GPIO` 采样绝对引脚号，`PIN` 采样 `IN_BASE + index`（都在 31 回绕）；IRQ 源等待标志 |
| `IN` | Sources: pins (sampled from `IN_BASE`, masked by `IN_COUNT`), x, y, null, status, isr, osr; honours `SHIFTCTRL.IN_SHIFTDIR` / 源：pins（从 `IN_BASE` 采样并按 `IN_COUNT` 掩码）、x、y、null、status、isr、osr；遵循 `IN_SHIFTDIR` |
| `OUT` | Destinations: pins (drives `OUT_COUNT` pins from `OUT_BASE`), x, y, null, pindirs (output enables), pc, isr, exec / 目标：pins（从 `OUT_BASE` 起驱动 `OUT_COUNT` 个引脚）、x、y、null、pindirs（输出使能）、pc、isr、exec |
| `PUSH`/`PULL` | `block`, `iffull`/`ifempty`, and bit-4 `noerr` (suppresses the sticky stall flag) / 支持 `block`、`iffull`/`ifempty`，以及位 4 的 `noerr`（不置位粘滞 stall 标志） |
| `MOV` | Destinations: pins, x, y, exec, pc, isr, osr; sources: pins, x, y, null, status, isr, osr; op: none/invert/bit-reverse / 目标：pins、x、y、exec、pc、isr、osr；源：pins、x、y、null、status、isr、osr；运算：无/取反/位反转 |
| `IRQ` | Set, clear, wait and `rel` (SM-relative index) / 置位、清零、等待与 `rel`（状态机相对序号） |
| `SET` | pins (drives `SET_COUNT` pins from `SET_BASE`), x, y, pindirs (output enables) / 支持 pins（从 `SET_BASE` 起驱动 `SET_COUNT` 个引脚）、x、y、pindirs（输出使能） |

Shift-register semantics / 移位寄存器语义:

- `ISR` counts bits shifted in (`0..=32`) and `OSR` counts bits remaining
  (`0..=32`); both start at 0 (empty), matching the datasheet reset state.
  `ISR` 统计已移入位数（`0..=32`），`OSR` 统计剩余位数（`0..=32`）；两者初始
  为 0（空），与数据手册复位状态一致。
- `PUSH_THRESH`/`PULL_THRESH` (0 encodes 32) drive autopush (push once
  `isr_bits >= threshold`) and autopull (refill once `osr_bits <= 32 - threshold`).
  `PUSH_THRESH`/`PULL_THRESH`（0 表示 32）驱动自动压入（`isr_bits >= 阈值`
  时压入）与自动拉取（`osr_bits <= 32 - 阈值` 时补充）。
- An instruction that cannot complete (empty TX for `PULL`/`OUT`/autopull, full
  RX for `PUSH`/autopush) leaves `ADDR` unchanged, sets `EXECCTRL.EXEC_STALLED`
  and is retried on the next step; the bit clears when it retires.
  无法完成的指令（`PULL`/`OUT`/autopull 遇到空 TX，`PUSH`/autopush 遇到满 RX）
  保持 `ADDR` 不变、置位 `EXECCTRL.EXEC_STALLED`，并在下一步重试；退休时该位清零。
- Writes to `SMx_INSTR` execute the instruction without advancing the PC unless
  the instruction itself branches. `EXEC`/`MOV EXEC` nesting is limited to one
  level; a stalling nested instruction retires the outer instruction.
  写 `SMx_INSTR` 会执行该指令且不推进 PC（除非指令自身跳转）。`EXEC`/
  `MOV EXEC` 最多嵌套一层；内层指令阻塞时外层指令按已退休处理。

### Not modelled yet / 尚未建模

The PIO DREQ levels are modelled (see *DMA requests* above), but not
cycle-exactly: the DMA controller samples them per data unit and the host
decides when to republish (`Machine` does so inside each throttled DMA batch).
The RP2350 `RXFn_PUTGET*` registers and the
`IRQ0_INTE`/`IRQ0_INTF`/`IRQ0_INTS`/`IRQ1_INTE`/`IRQ1_INTF`/`IRQ1_INTS`
interrupt registers at `0x16C..0x184` are not modelled: the shared handle
asserts a line from the raw flag alone, without per-flag enables or forced
status. `GPIOBASE` is stored but never added to PINCTRL pin numbers, so every
block sees its own local GPIO 0..31 window, and no PINCTRL FUNCSEL crossbar
routes PIO pins to real GPIO pads. Cross-block `IRQ` routing (RP2350 index
modes 1/3) currently falls back to this block's flags.

PIO DREQ 电平已建模（见上文 *DMA requests*），但不是周期精确的：DMA 控制器按数据
单元采样，由宿主决定何时重新发布（`Machine` 在每个受节流的 DMA 批次内执行）。
RP2350 `RXFn_PUTGET*` 寄存器，以及 `0x16C..0x184` 的
`IRQ0_INTE`/`IRQ0_INTF`/`IRQ0_INTS`/`IRQ1_INTE`/`IRQ1_INTF`/`IRQ1_INTS`
中断寄存器尚未建模：共享句柄仅按原始标志断言中断线，没有逐标志使能与强制状态。
`GPIOBASE` 仅存储、不会加到 PINCTRL 引脚号上，因此每个块看到的是自己局部的
GPIO 0..31 窗口，也没有 PINCTRL FUNCSEL 交叉开关把 PIO 引脚接到真实 GPIO 焊盘。
跨块 `IRQ` 路由（RP2350 索引模式 1/3）目前回落到本块标志。

---

## HSTX / 高速发送

The HSTX model covers the two datasheet register windows, the 8-entry
command/data FIFO, the command-expander subset and the host-side line-output
API. It does **not** encode DVI/HDMI pixels. `Hstx` implements no `Peripheral`
trait directly; [`Hstx::ctrl_device`] and [`Hstx::fifo_device`] expose the two
`MmioDevice` adapters, and clones share one register/FIFO state.
HSTX 模型覆盖数据手册的两个寄存器窗口、8 项命令/数据 FIFO、命令扩展器子集与
宿主侧行输出 API，**不**做 DVI/HDMI 像素编码。`Hstx` 不直接实现 `Peripheral`；
[`Hstx::ctrl_device`] 与 [`Hstx::fifo_device`] 暴露两个 `MmioDevice` 适配器，
克隆共享同一寄存器/FIFO 状态。

```rust
pub struct Hstx { /* shared register file + FIFO + command expander */ }

impl Hstx {
    pub fn new(irq: IrqAssert) -> Self;
    pub fn ctrl_device(&self) -> Box<dyn MmioDevice>;
    pub fn fifo_device(&self) -> Box<dyn MmioDevice>;
    pub fn push_command(&mut self, command: u32) -> bool;
    pub fn push_data(&mut self, word: u32) -> bool;
    pub fn advance(&self, cycles: u64);
    pub fn drain_output(&mut self) -> Vec<u32>;
    pub fn take_scanline(&mut self) -> Vec<u32>;
    pub fn output_underrun(&self) -> u64;
    pub fn fifo_level(&self) -> u8;
    pub fn output_level(&self) -> usize;
    pub fn enabled(&self) -> bool;
    pub fn unknown_commands(&self) -> u64;
    pub fn last_unknown_command(&self) -> Option<u32>;
    pub fn reset(&mut self);
}
```

### Windows / 窗口

| Window | Base | Constant | Contents |
|--------|------|----------|----------|
| APB control | `0x400C_0000` | `map::HSTX_CTRL_BASE` | `CSR`, `BIT0..BIT7`, `EXPAND_SHIFT`, `EXPAND_TMDS`, model IRQ registers |
| AHB FIFO | `0x5060_0000` | `map::HSTX_FIFO_BASE` | `STAT`, `FIFO` |

`BootPeripherals::mount` installs the control adapter into the APB router and
mounts the FIFO adapter standalone in AHB space. `BootPeripherals::hstx()` /
`hstx_mut()` return handles sharing that state, and `BootPeripherals::advance`
pumps `Hstx::advance`.
`BootPeripherals::mount` 把控制适配器装入 APB 路由器，并将 FIFO 适配器独立
挂载到 AHB 空间。`BootPeripherals::hstx()` / `hstx_mut()` 返回共享状态的句柄，
`BootPeripherals::advance` 会泵送 `Hstx::advance`。

### Register subset / 寄存器子集

- `CSR` (`+0x00`, reset `0x1005_0600`, write mask `0xFF1F_1F73`) — `EN` (bit 0)
  gates FIFO popping; `EXPAND_EN` (bit 1) enables command expansion. `CLKDIV`,
  `CLKPHASE`, `N_SHIFTS`, `SHIFT` and the coupled-mode bits are stored and read
  back but have no timing effect.
- `BIT0..BIT7` (`+0x04..+0x20`, mask `0x0003_1F1F`) and `EXPAND_SHIFT` (`+0x24`,
  reset `0x0100_0100`), `EXPAND_TMDS` (`+0x28`) are storage only;
  `EXPAND_SHIFT.ENC_SHIFT` (bits 20:16) drives the modelled TMDS word count.
- `HSTX_FIFO_STAT` (`+0x00`) — `LEVEL[7:0]`, `FULL` (8), `EMPTY` (9) derived
  from the queue; `WOF` (10) sticky write-while-full flag, write-one-to-clear.
- `HSTX_FIFO_FIFO` (`+0x04`) — write-only; reads return zero. Writes while full
  drop the word and set `WOF`.
- `+0x30` `IRQ_STATUS` (W1C) and `+0x34` `IRQ_ENABLE` are **model-only**
  registers, because silicon has no HSTX interrupt registers. Reserved/read-only
  bits are masked, and unmodelled offsets follow the project one-shot warning
  policy (read 0, write ignored).

- `CSR`（`+0x00`，复位 `0x1005_0600`，写掩码 `0xFF1F_1F73`）——`EN`（位 0）
  控制 FIFO 弹出，`EXPAND_EN`（位 1）启用命令扩展。`CLKDIV`、`CLKPHASE`、
  `N_SHIFTS`、`SHIFT` 与耦合模式位仅存储/回读，无时序效果。
- `BIT0..BIT7`（`+0x04..+0x20`，掩码 `0x0003_1F1F`）与 `EXPAND_SHIFT`
  （`+0x24`，复位 `0x0100_0100`）、`EXPAND_TMDS`（`+0x28`）仅存储；
  `EXPAND_SHIFT.ENC_SHIFT`（位 20:16）驱动建模的 TMDS 字数换算。
- `HSTX_FIFO_STAT`（`+0x00`）——`LEVEL[7:0]`、`FULL`（8）、`EMPTY`（9）由
  队列推导；`WOF`（10）为满写粘滞标志，写 1 清零。
- `HSTX_FIFO_FIFO`（`+0x04`）——只写，读回 0；满时写入丢弃该字并置位 `WOF`。
- `+0x30` `IRQ_STATUS`（写 1 清零）与 `+0x34` `IRQ_ENABLE` 是**仅模型**
  寄存器，因为芯片没有 HSTX 中断寄存器。保留位/只读位被掩码，未建模偏移
  沿用项目的一次性告警策略（读 0、写忽略）。

### Command encoding / 命令编码

Opcode lives in bits `[15:12]`, count in bits `[11:0]` — the layout used by the
`HSTX_CMD_*` macros in `pico-examples`. A zero count means zero words/cycles.
操作码位于位 `[15:12]`、计数位于位 `[11:0]`，与 `pico-examples` 的
`HSTX_CMD_*` 宏一致。计数 0 表示 0 个字/周期。

| Opcode | Constant | Behaviour / 行为 |
|--------|----------|------------------|
| `0x0` | `hstx::CMD_RAW` | consume `count` data words, emit each once / 消费 `count` 个数据字，各输出一次 |
| `0x1` | `hstx::CMD_RAW_REPEAT` | consume one data word, emit it `count` times / 消费一个字并输出 `count` 次 |
| `0x2` | `hstx::CMD_TMDS` | consume `ceil(count * ENC_SHIFT / 32)` words (or `count` when `ENC_SHIFT = 0`), pass through unchanged / 消费相应字数并原样透传 |
| `0x3` | `hstx::CMD_TMDS_REPEAT` | consume one word, emit it `count` times (TMDS word count) / 消费一个字并重复 |
| `0xE` | `hstx::CMD_WAIT` | **model extension**: wait `count` HSTX cycles, no output / **模型扩展**：等待，无输出 |
| `0xF` | `hstx::CMD_NOP` | no payload, no output / 无载荷、无输出 |

`CMD_WAIT` is not part of the hardware command set (which has no delay command);
opcode `0xE` is otherwise undefined, so the model reserves it and documents the
limitation. Unknown opcodes are consumed, counted in `unknown_commands()`, kept
in `last_unknown_command()`, logged once per opcode, flagged in `IRQ_STATUS` and
never panic or raise the completion IRQ.
`CMD_WAIT` 不属于硬件命令集（硬件没有延迟命令）；操作码 `0xE` 本来未定义，
模型保留它并记录该限制。未知操作码会被消费、计入 `unknown_commands()`、保存在
`last_unknown_command()`、按操作码只记录一次日志、在 `IRQ_STATUS` 置位，且
不会 panic，也不会触发完成中断。

With `EXPAND_EN = 0`, every FIFO word is raw output data and no command decoding
happens — matching the datasheet's "raw FIFO data passed directly" mode.
`EXPAND_EN = 0` 时每个 FIFO 字都是原始输出数据，不做命令解码，对应数据手册的
“原始 FIFO 数据直通”模式。

### Host-side output / 宿主侧输出

`advance(cycles)` pops at most one FIFO word per HSTX cycle while `EN` is set,
expands it and queues the resulting 32-bit output words. `drain_output()` takes
the whole queue; `take_scanline()` is an alias for scanline-oriented hosts.
`output_underrun()` counts FIFO starvation episodes while a command still needs
data, and `advance` sets `IRQ_UNDERRUN`. `advance` does nothing when `EN` is
clear or the FIFO is empty, so an idle HSTX never produces pixels. `reset()`
restores the reset values and clears FIFOs, counters and the shared IRQ line.
`advance(cycles)` 在 `EN` 置位时每个 HSTX 周期至多弹出一个 FIFO 字，展开并把
32 位输出字放入队列。`drain_output()` 取走整个队列；`take_scanline()` 是面向
扫描线宿主的别名。`output_underrun()` 统计命令仍需数据时的 FIFO 断流次数，
`advance` 会置位 `IRQ_UNDERRUN`。`EN` 清零或 FIFO 空闲时 `advance` 不做任何
事，因此空闲 HSTX 不会产生像素。`reset()` 恢复复位值并清空 FIFO、计数器与
共享中断线。

```rust
use rp2350_core::bus::AddressBus;
use rp2350_core::memory::SimpleBus;
use rp2350_periph::{BootPeripherals, hstx, map};

let mut bus = SimpleBus::new();
let mut peripherals = BootPeripherals::mount(&mut bus)?;
let mut cycles = 0u64;

bus.write_u32(map::HSTX_CTRL_BASE, hstx::CSR_EN | hstx::CSR_EXPAND_EN, &mut cycles)?;
bus.write_u32(map::HSTX_FIFO_BASE + 0x04, hstx::CMD_RAW | 3, &mut cycles)?;
for word in [0x0011_2233, 0x0044_5566, 0x0077_8899] {
    bus.write_u32(map::HSTX_FIFO_BASE + 0x04, word, &mut cycles)?;
}
peripherals.advance(4);
assert_eq!(peripherals.hstx_mut().drain_output(), [0x0011_2233, 0x0044_5566, 0x0077_8899]);
```

### Interrupts / 中断

Silicon has no dedicated HSTX interrupt line. The model borrows `SPARE_IRQ_0`
(`map::HSTX_IRQ` = 46) and defines `IRQ_STATUS`/`IRQ_ENABLE` at control offsets
`+0x30`/`+0x34` with `IRQ_UNDERRUN` (0), `IRQ_COMMAND_DONE` (1) and
`IRQ_UNKNOWN_COMMAND` (2). Any enabled status bit asserts the shared
`IrqAssert` line, so `BootPeripherals::irq_mask()` reports it; status bits are
write-one-to-clear.
芯片没有专用 HSTX 中断线。模型借用 `SPARE_IRQ_0`（`map::HSTX_IRQ` = 46），在
控制窗口 `+0x30`/`+0x34` 定义 `IRQ_STATUS`/`IRQ_ENABLE`，位为 `IRQ_UNDERRUN`
（0）、`IRQ_COMMAND_DONE`（1）与 `IRQ_UNKNOWN_COMMAND`（2）。任一使能状态位
置位即断言共享 `IrqAssert` 线，`BootPeripherals::irq_mask()` 可观察到；状态位
写 1 清零。

### Not modelled / 未建模项

DVI/HDMI bit timing, the TMDS encoder and 10-bit symbol packing, the output
shift register/clock generator (`CLKDIV`, `CLKPHASE`, `N_SHIFTS`, `SHIFT`),
`BITx` pin crossbar routing, GPIO pin mapping, `EXPAND_TMDS` lane configuration,
HSTX DMA handshakes (DREQ) and PIO coupled mode. TMDS data words are passed
through unchanged; only the consumed word count is derived from
`EXPAND_SHIFT.ENC_SHIFT`. No scanline/frame boundary, pixel format or
`HstxDecoder` is provided. Hardware FIFO timing is approximated as one word per
HSTX cycle.
DVI/HDMI 位时序、TMDS 编码器与 10 位符号打包、输出移位寄存器/时钟发生器
（`CLKDIV`、`CLKPHASE`、`N_SHIFTS`、`SHIFT`）、`BITx` 引脚交叉开关、GPIO
引脚映射、`EXPAND_TMDS` 通道配置、HSTX DMA 握手（DREQ）与 PIO 耦合模式均未
建模。TMDS 数据字原样透传，只有消费字数由 `EXPAND_SHIFT.ENC_SHIFT` 推导。
不提供扫描线/帧边界、像素格式或 `HstxDecoder`。硬件 FIFO 时序近似为每个
HSTX 周期一个字。

---

## DMA

`BootPeripherals::mount` installs one `Dma` at `0x5000_0000` (`map::DMA_BASE`,
window `map::DMA_SIZE` = 4 KB). Clones share a single register file, so the
bus-mounted device and the handles returned by `BootPeripherals::dma()` /
`dma_mut()` observe the same state.
`BootPeripherals::mount` 在 `0x5000_0000`（`map::DMA_BASE`，窗口
`map::DMA_SIZE` = 4 KB）挂载一个 `Dma`。克隆共享同一个寄存器文件，因此总线上
的设备与 `BootPeripherals::dma()` / `dma_mut()` 返回的句柄看到相同状态。

```rust
pub struct Dma { /* shared register file + IrqAssert */ }

impl Dma {
    pub fn new(irq: IrqAssert) -> Self;
    pub fn run(&mut self, bus: &mut dyn AddressBus, max_transfers: u64) -> u64;
    pub fn set_dreq(&self, dreq: u8, asserted: bool);
    pub fn set_dreq_bank(&self, first: u8, levels: u32);
    pub fn dreq(&self, dreq: u8) -> Option<bool>;
    pub fn busy_selects_dreq(&self, first: u8, last: u8) -> bool;
    pub fn abort(&mut self, mask: u16);
    pub fn busy_mask(&self) -> u16;
    pub fn channel_state(&self, index: usize) -> Option<DmaChannelState>;
}

pub struct DmaChannelState {
    pub read_addr: u32,
    pub write_addr: u32,
    pub trans_count: u32,
    pub ctrl: u32,
    pub busy: bool,
    pub enabled: bool,
    pub chain_to: u8,
    pub treq_sel: u8,
    pub ring_size: u8,
    pub irq_quiet: bool,
}
```

### Register map / 寄存器映射

| Offset | Register | Notes / 说明 |
|--------|----------|--------------|
| `0x000..=0x3FF` | `CH0..CH15` | 16 channels × `0x40` bytes / 16 通道 × `0x40` 字节 |
| `+0x00` | `READ_ADDR` | read address / 读地址 |
| `+0x04` | `WRITE_ADDR` | write address / 写地址 |
| `+0x08` | `TRANS_COUNT` | remaining count (`[27:0]`) / 剩余计数（`[27:0]`） |
| `+0x0C` | `CTRL_TRIG` | control + trigger / 控制 + 触发 |
| `+0x10..=0x3C` | `AL1_*`/`AL2_*`/`AL3_*` | read/write/control aliases; `AL1_TRANS_COUNT_TRIG`, `AL2_WRITE_ADDR_TRIG`, `AL3_READ_ADDR_TRIG` also start the channel on a non-zero write / 读/写/控制别名；上述三个 `*_TRIG` 别名的非零写入同时启动通道 |
| `0x400` | `INTR` | raw 16-bit status, W1C / 16 位原始状态，写 1 清零 |
| `0x404`/`0x414`/`0x424`/`0x434` | `INTE0..3` | per-line enable / 每条线的使能 |
| `0x408`/`0x418`/`0x428`/`0x438` | `INTF0..3` | per-line force / 每条线的强制位 |
| `0x40C`/`0x41C`/`0x42C`/`0x43C` | `INTS0..3` | `(INTR & INTEn) \| INTFn`, also W1C through `INTR` / 状态，写 `INTR` 同样可清 |
| `0x440..=0x44C` | `TIMER0..3` | pacing X/Y storage / 节拍 X/Y 存储 |
| `0x450` | `MULTI_CHAN_TRIGGER` | start the selected channels / 启动选中通道 |
| `0x454`/`0x458` | `SNIFF_CTRL`/`SNIFF_DATA` | storage only / 仅存储 |
| `0x460` | `FIFO_LEVELS` | reads 0 / 读 0 |
| `0x464` | `CHAN_ABORT` | clear `BUSY` on the selected channels / 清除选中通道的 `BUSY` |
| `0x468` | `N_CHANNELS` | reads 16 / 读 16 |

`CTRL_TRIG` fields follow the RP2350 layout: `EN[0]`, `HIGH_PRIORITY[1]`,
`DATA_SIZE[3:2]`, `INCR_READ[4]`, `INCR_WRITE[6]`, `RING_SIZE[11:8]`,
`RING_SEL[12]`, `CHAIN_TO[16:13]`, `TREQ_SEL[22:17]`, `IRQ_QUIET[23]`,
`BSWAP[24]`, `SNIFF_EN[25]`, `BUSY[26]`, `WRITE_ERROR[29]` (W1C),
`READ_ERROR[30]` (W1C), `AHB_ERROR[31]`. `BUSY` is read-only; every other
writable bit is replaced on a `CTRL_TRIG` write. `EN` is retained after
completion (the SDK clears it explicitly); writing `CTRL_TRIG` with `EN = 0`
clears both `EN` and `BUSY`.
`CTRL_TRIG` 位域遵循 RP2350 布局……`BUSY` 只读；其余可写位在写 `CTRL_TRIG`
时被替换。完成传输后 `EN` 保留（SDK 会显式清除）；写 `EN = 0` 会同时清除
`EN` 与 `BUSY`。

### Transfer semantics / 传输语义

- Writing `CTRL_TRIG` (or an ALx control alias) with `EN` set and a non-zero
  value sets `BUSY` when `TRANS_COUNT > 0`; a zero count completes immediately.
  `AL1_TRANS_COUNT_TRIG` / `AL2_WRITE_ADDR_TRIG` / `AL3_READ_ADDR_TRIG` and
  `MULTI_CHAN_TRIGGER` also start the selected channels.
  写 `CTRL_TRIG`（或 ALx 控制别名）且 `EN` 置位、值非零时，若
  `TRANS_COUNT > 0` 则置 `BUSY`；计数为 0 时立即完成。
  `AL1_TRANS_COUNT_TRIG` / `AL2_WRITE_ADDR_TRIG` / `AL3_READ_ADDR_TRIG` 与
  `MULTI_CHAN_TRIGGER` 也会启动选中通道。
- `Dma::run(bus, max_transfers)` visits channels round-robin and performs one
  unit per eligible channel step: read `DATA_SIZE` (`0`=byte, `1`=halfword,
  `2`=word, reserved `3` treated as word) from `READ_ADDR`, write it to
  `WRITE_ADDR`, then advance `READ_ADDR` if `INCR_READ` and `WRITE_ADDR` if
  `INCR_WRITE` by the unit width.
  `Dma::run(bus, max_transfers)` 按轮转顺序访问通道，每个可执行步搬运一个
  单元：按 `DATA_SIZE`（`0`=字节、`1`=半字、`2`=字，保留值 `3` 按字处理）从
  `READ_ADDR` 读取并写入 `WRITE_ADDR`；随后按 `INCR_READ`/`INCR_WRITE` 以
  单元宽度推进地址。
- `RING_SIZE` is the log2 ring byte count. With a non-zero size, the read side
  wraps when `RING_SEL = 0` and the write side when `RING_SEL = 1`, using
  `(addr & !mask) | ((addr + step) & mask)`.
  `RING_SIZE` 是环字节数的 log2。非零时，`RING_SEL = 0` 环绕读侧、
  `RING_SEL = 1` 环绕写侧，公式为 `(addr & !mask) | ((addr + step) & mask)`。
- `TRANS_COUNT` decrements after each unit and saturates at zero; reaching zero
  clears `BUSY`, sets `INTR[channel]` unless `IRQ_QUIET`, and starts
  `CHAIN_TO`. The return value of `run` counts attempted units, so a driver can
  cap DMA work per CPU timeslice.
  `TRANS_COUNT` 每个单元后递减并在零处饱和；到零时清除 `BUSY`、在未置
  `IRQ_QUIET` 时置位 `INTR[channel]`，并启动 `CHAIN_TO`。`run` 的返回值是
  尝试的单元数，驱动可据此限制每个 CPU 时间片内的 DMA 工作量。
- `CHAN_ABORT` / `Dma::abort(mask)` clears `BUSY` but keeps `EN`, the addresses
  and the remaining count, matching the datasheet; a new trigger restarts the
  channel from the current state.
  `CHAN_ABORT` / `Dma::abort(mask)` 清除 `BUSY`，但保留 `EN`、地址与剩余计数，
  与数据手册一致；再次触发会从当前状态重启通道。
- A bus read or write error latches `READ_ERROR` / `WRITE_ERROR`, clears
  `BUSY` and raises no completion interrupt.
  总线读/写错误会锁存 `READ_ERROR` / `WRITE_ERROR`、清除 `BUSY`，且不产生
  完成中断。

### DREQ / DREQ 请求

`TREQ_SEL = 0x3F` (`TREQ_PERMANENT`) always transfers. Every other selector
samples the host-driven level array; all 64 inputs reset asserted so firmware
that never programs a DREQ still progresses. `set_dreq(sel, false)` pauses
channels using that selector and `set_dreq(sel, true)` resumes them on the next
`run`; selectors `>= 64` are ignored. `set_dreq_bank(first, levels)` writes
eight consecutive inputs from one bitmask and one lock, which is how the PIO
blocks publish four TX and four RX levels at once. The levels are sampled per
`Dma::run` call (and per unit inside `BootPeripherals::pump_dma` for PIO-paced
channels), not continuously. Selectors `0x3B..=0x3E` are the pacing timers and
are stored but do not gate transfers yet.
`TREQ_SEL = 0x3F`（`TREQ_PERMANENT`）始终可传输。其他选择器采样宿主驱动的
电平数组；64 个输入复位时均断言，因此未编程 DREQ 的固件仍能推进。
`set_dreq(sel, false)` 暂停使用该选择器的通道，`set_dreq(sel, true)` 在下一次
`run` 时恢复；`>= 64` 的选择被忽略。`set_dreq_bank(first, levels)` 用一次加锁从
位掩码写入连续八个输入，PIO 块据此一次发布四个 TX 与四个 RX 电平。电平在每次
`Dma::run` 调用时采样（PIO 节拍的通道在 `BootPeripherals::pump_dma` 内逐单元
采样），不是连续采样。`0x3B..=0x3E` 是节拍定时器选择器，仅存储，尚未门控传输。

The RP2350 PIO selectors are `0..=23` (`PIO0_TX0` … `PIO2_RX3`), with
`PIOx_TXy` asserted while the TX FIFO is not full and `PIOx_RXy` while the RX
FIFO is not empty; the numbering and join behaviour are documented under
*PIO → DMA requests*. `BootPeripherals::sync_pio_dreqs()` publishes them and
`BootPeripherals::pump_dma()` uses `busy_selects_dreq(0, 23)` to refresh the
levels between single units. Non-PIO selectors are still reset-asserted and are
not driven by SPI/UART/ADC/HSTX/timers.
RP2350 的 PIO 选择器为 `0..=23`（`PIO0_TX0` … `PIO2_RX3`）：`PIOx_TXy` 在 TX
FIFO 未满时断言，`PIOx_RXy` 在 RX FIFO 非空时断言；编号与 join 行为见
*PIO → DMA requests*。`BootPeripherals::sync_pio_dreqs()` 负责发布，
`BootPeripherals::pump_dma()` 用 `busy_selects_dreq(0, 23)` 在逐单元之间刷新
电平。非 PIO 选择器仍为复位断言状态，尚未由 SPI/UART/ADC/HSTX/定时器驱动。

### Chaining and interrupts / 链式与中断

`CHAIN_TO` is a 4-bit target; a target equal to the channel itself disables
chaining (the hardware reset value). Completion of a channel starts the target,
including when `IRQ_QUIET` suppresses the interrupt. The four RP2350 DMA lines
are `map::DMA_IRQ_0..3` = `10..13`. For each line
`INTSn = (INTR & INTEn) | INTFn`; any non-zero status asserts the shared
`IrqAssert` bit, so `BootPeripherals::irq_mask()` already contains DMA
interrupts without extra wiring. `INTR` and `INTSn` are write-one-to-clear;
`INTEn` / `INTFn` accept the low 16 bits.
`CHAIN_TO` 是 4 位目标；目标等于通道自身时禁用链式（硬件复位值）。通道完成后
启动目标通道，即使 `IRQ_QUIET` 抑制了中断。RP2350 的四条 DMA 线为
`map::DMA_IRQ_0..3` = `10..13`。对每条线
`INTSn = (INTR & INTEn) | INTFn`；任一非零状态都会断言共享 `IrqAssert` 的
对应位，因此 `BootPeripherals::irq_mask()` 无需额外接线即可看到 DMA 中断。
`INTR` 与 `INTSn` 为写 1 清零；`INTEn` / `INTFn` 只接受低 16 位。

### Not modelled / 未建模项

`BSWAP` byte swapping, `SNIFF_EN`/`SNIFF_CTRL`, `HIGH_PRIORITY` arbitration,
`TRANS_COUNT` transfer modes (`[31:28]`), the reload/working-count split (a
`TRANS_COUNT` write also replaces the remaining count while a channel is busy),
the per-channel debug registers at `0x800..=0xBFF`, DMA pacing timers, AHB
error recovery beyond clearing `BUSY` and the APB SET/CLR/XOR global aliases.
Reads of unmodelled offsets return zero with a one-shot warning and writes are
ignored. The DMA device never intercepts CPU bus traffic: transfers happen only
inside `Dma::run`.
`BSWAP` 字节交换、`SNIFF_EN`/`SNIFF_CTRL`、`HIGH_PRIORITY` 仲裁、
`TRANS_COUNT` 传输模式（`[31:28]`）、重载值与工作计数的拆分（写
`TRANS_COUNT` 会在通道忙碌时直接替换剩余计数）、`0x800..=0xBFF` 的通道调试
寄存器、DMA 节拍定时器、除清除 `BUSY` 之外的 AHB 错误恢复，以及 APB
SET/CLR/XOR 全局别名均未建模。未建模偏移读出 0 并只警告一次，写入被忽略。
DMA 设备不拦截 CPU 总线流量：传输只在 `Dma::run` 内部发生。

### Host driving / 宿主驱动方式

Program the channel through the MMIO window, then pump `run` from the
simulation loop. `BootPeripherals::advance` intentionally does not execute DMA
transfers. For a channel paced by a PIO FIFO, call
`BootPeripherals::sync_pio_dreqs()` before `Dma::run`, or let
`BootPeripherals::pump_dma(bus, units)` do both; `rp2350-cli`'s `Machine` uses
the latter at its throttled DMA pump point, so `TREQ_SEL = PIO0_TX0` only writes
while the TX FIFO has room.
通过 MMIO 窗口编程通道，然后在仿真循环中泵送 `run`。
`BootPeripherals::advance` 有意不执行 DMA 传输。由 PIO FIFO 节拍的通道应在
`Dma::run` 之前调用 `BootPeripherals::sync_pio_dreqs()`，或直接使用
`BootPeripherals::pump_dma(bus, units)` 一并完成；`rp2350-cli` 的 `Machine` 在
受节流的 DMA 泵送点使用后者，因此 `TREQ_SEL = PIO0_TX0` 只会在 TX FIFO 有空位时
写入。

```rust
use rp2350_core::memory::SimpleBus;
use rp2350_periph::{BootPeripherals, map};

let mut bus = SimpleBus::new();
let mut peripherals = BootPeripherals::mount(&mut bus)?;
let mut cycles = 0u64;
let base = map::DMA_BASE;
bus.write_u32(base + 0x00, 0x2000_0000, &mut cycles)?; // READ_ADDR
bus.write_u32(base + 0x04, 0x2000_1000, &mut cycles)?; // WRITE_ADDR
bus.write_u32(base + 0x08, 64, &mut cycles)?;          // TRANS_COUNT
bus.write_u32(base + 0x0C, 0x007E_0051, &mut cycles)?; // EN + INCR + permanent DREQ
let moved = peripherals.dma_mut().run(&mut bus, 64);
assert_eq!(moved, 64);
```

---

## XIP (QMI + Flash + PSRAM backend)

```rust
pub struct XipController {
    pub qmi: Qmi,
    pub flash: QspiFlash,
    pub psram: Psram,
}

impl XipController {
    pub fn xip_read_u32(&self, addr: u32) -> u32;
    pub fn xip_write_u32(&mut self, addr: u32, value: u32);
    pub fn tick(&mut self, cycles: u64);
}
```

See [rp2350-virt-periph.md](rp2350-virt-periph.md) for PSRAM and QSPI Flash device details.
PSRAM 和 QSPI Flash 设备详情参见 [rp2350-virt-periph.md](rp2350-virt-periph.md)。

---

## UART

```rust
pub struct RpUart {
    pub index: u8,
    pub tx_fifo: VecDeque<u8>,
    pub rx_fifo: VecDeque<u8>,
    pub baud_rate_div: u32,
    pub ctrl: UartCtrl,
    pub line_ctrl: UartLineCtrl,
    pub irq_mask: UartIrqMask,
    pub irq_status: UartIrqStatus,
}

impl RpUart {
    pub fn push_rx(&mut self, byte: u8);
    pub fn pop_tx(&mut self) -> Option<u8>;
    pub fn rx_fifo_full(&self) -> bool;
    pub fn tx_fifo_empty(&self) -> bool;
}
```

---

## SPI

```rust
pub struct RpSpi {
    pub index: u8,
    pub tx_fifo: VecDeque<u8>,
    pub rx_fifo: VecDeque<u8>,
    pub ctrl: SpiCtrl,
    pub clock_div: u32,
}

impl RpSpi {
    pub fn push_rx(&mut self, byte: u8);
    pub fn pop_tx(&mut self) -> Option<u8>;
}
```

---

## GPIO (SIO-based)

```rust
pub struct RpGpio {
    pub pins: [GpioPin; 48],
    pub out_set: u32,
    pub out_clr: u32,
    pub out_xor: u32,
    pub oe_set: u32,
    pub oe_clr: u32,
    pub oe_xor: u32,
    pub in_reg: u32,
    pub irq_summary: u16,
}

impl RpGpio {
    pub fn set_output(&mut self, pin: u8, value: bool);
    pub fn get_input(&self, pin: u8) -> bool;
    pub fn set_function(&mut self, pin: u8, function: PinFunction);
    pub fn update_irq(&mut self);
}
```

---

## SIO

```rust
pub struct RpSio {
    pub cpu_id: u32,
    pub gpio_in: u32,
    pub gpio_out: u32,
    pub gpio_oe: u32,
    pub fifo_st: SioFifoStatus,
    pub fifo_wr: u32,
    pub fifo_rd: u32,
    pub spinlocks: [u32; 32],
    pub doorbell_out_set: u32,
    pub doorbell_in: u32,
    pub interp: [Interpolator; 2],
    pub tmds: TmdsEncoder,
}
```

---

## PSM and BOOTRAM / 电源状态机与 BOOTRAM

```rust
pub struct Psm { /* FRCE_ON, FRCE_OFF, WDSEL, DONE */ }
impl Psm {
    pub fn new() -> Self;
    pub fn with_core1_release(flag: Arc<AtomicBool>) -> Self;
}

impl BootPeripherals {
    pub fn service_core1_peer(&self); // FIFO echo + core-1 boot-ROM ack
    pub fn sio(&self) -> &Sio;
}
```

`Psm` models `PSM_FRCE_ON`, `PSM_FRCE_OFF`, `PSM_WDSEL` and the derived
`PSM_DONE` status; clearing `FRCE_OFF.PROC1` latches a core-1 release event
that the simulation driver turns into the boot ROM's drain-and-`0` handshake.
`BootRam` exposes the scratch RAM at `0x400E_0000`; its first word is a
`bx lr` boot2 copy-out stub so pico-sdk `flash_init_boot2_copyout` can return
while this model keeps flash XIP readable.
PSM 模型实现 `PSM_FRCE_ON`、`PSM_FRCE_OFF`、`PSM_WDSEL` 与派生的 `PSM_DONE`
状态；清除 `FRCE_OFF.PROC1` 会置位 core1 释放事件，仿真驱动器据此执行
boot ROM 的“清空信箱并回送 0”握手。`BootRam` 暴露 `0x400E_0000` 的暂存
RAM；首字为 `bx lr` boot2 拷出桩，使 pico-sdk 的
`flash_init_boot2_copyout` 能立即返回，而本模型保持 Flash XIP 可读。

---

## USB Controller

```rust
pub struct RpUsbController {
    pub main_ctrl: u32,
    pub sie_ctrl: u32,
    pub sie_status: u32,
    pub buffer_ctrl: u32,
    pub buffer_select: u32,
    pub endpoints: [UsbEndpoint; 16],
    pub cdc: Option<UsbCdc>,
    pub dpram: Box<[u8]>,
}

pub struct UsbCdc {
    pub tx_buf: VecDeque<u8>,
    pub rx_buf: VecDeque<u8>,
    pub connected: bool,
    pub line_coding: CdcLineCoding,
}
```