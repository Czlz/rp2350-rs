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
    pub fn with_pads(index: u8, pads: GpioPads) -> Self;   // wired to the pin matrix
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
    pub fn sample_inputs(&mut self);
    pub fn sampled_inputs(&self) -> u32;
    pub fn publish_outputs(&self);
    pub fn output_levels(&self) -> u32;
    pub fn output_enables(&self) -> u32;
}

impl SharedPio {
    pub fn new(index: u8, irq: IrqAssert) -> Self;
    pub fn with_pads(index: u8, irq: IrqAssert, pads: GpioPads) -> Self;
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
    pub fn sampled_inputs(&self) -> u32;
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
zero, so a block with no pads wired up still behaves like the earlier
"no pins attached" model (`IN PINS`/`WAIT`/`JMP PIN` read 0).

- `set_input_levels(u32)` / `input_levels()` / `set_input_level(pin, high)` set
  the **host-side override** vector; pins `>= 32` are ignored. `sample_inputs()`
  recomputes what the machines see, exposed as `input_levels()` /
  `sampled_inputs()`.
- A block built with `Pio::with_pads` / `SharedPio::with_pads` samples only the
  pads whose `IO_BANK0.GPIOx_CTRL.FUNCSEL` equals `PIO0`/`PIO1`/`PIO2` for this
  block; every other pin reads 0. The host override is OR-ed in, so a host can
  still force a level on an unrouted pin.
- `output_levels()` and `output_enables()` expose what `OUT`/`SET`/side-set
  wrote, and are mirrored in `DBG_PADOUT`/`DBG_PADOE`. `publish_outputs()` hands
  the pair to the shared pin matrix, which only applies it to pads routed to
  this block, so `OUT`/`SET`/**side-set** (including `SIDE_PINDIR`) reach the pad.
- `GPIOBASE` (bit 4, i.e. base 0 or 16) shifts the local 0..31 pin window onto
  the real GPIO numbers for both sampling and publishing.
- Pin numbers wrap from 31 back to 0 inside the local window, as on the hardware.
- `SharedPio::tick` resamples the routed inputs before the batch and publishes the
  driven outputs after it, and `SharedPio::with_mut` does the same around each
  host edit; a direct `Pio` user calls `sample_inputs()`/`publish_outputs()`
  itself. A host that ticks in batches therefore sees pad changes at batch
  granularity.

该块持有三个 32 位引脚向量：状态机采样的输入电平、状态机驱动的输出电平与输出
使能。三者复位均为 0，因此没有接 pad 的块仍保持早期“未接引脚”模型的语义
（`IN PINS`/`WAIT`/`JMP PIN` 读 0）。

- `set_input_levels(u32)` / `input_levels()` / `set_input_level(pin, high)` 设置
  **宿主侧覆盖**向量；`>= 32` 的引脚被忽略。`sample_inputs()` 重新计算状态机
  看到的向量，由 `input_levels()` / `sampled_inputs()` 暴露。
- 通过 `Pio::with_pads` / `SharedPio::with_pads` 接到 pad 的块只采样
  `IO_BANK0.GPIOx_CTRL.FUNCSEL` 等于本块 `PIO0`/`PIO1`/`PIO2` 的 pad，其它引脚
  读 0；宿主覆盖向量按位或入，因此宿主仍可强制未路由引脚的电平。
- `output_levels()` 与 `output_enables()` 暴露 `OUT`/`SET`/side-set 写入的
  结果，并反映在 `DBG_PADOUT`/`DBG_PADOE`。`publish_outputs()` 把这一对发布到
  共享引脚矩阵，矩阵只作用于路由到本块的 pad，因此 `OUT`/`SET`/**side-set**
  （含 `SIDE_PINDIR`）都能到达 pad。
- `GPIOBASE`（位 4，即基址 0 或 16）把局部 0..31 引脚窗口平移到真实 GPIO 编号，
  采样与发布都适用。
- 局部窗口内的引脚编号按硬件行为从 31 回绕到 0。
- `SharedPio::tick` 在批次前重采样路由输入、批次后发布驱动输出，
  `SharedPio::with_mut` 在每次宿主编辑前后做同样的事；直接使用 `Pio` 的调用方
  自行调用 `sample_inputs()`/`publish_outputs()`。按批次 tick 的宿主因此以批次
  粒度看到 pad 变化。

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
status. `GPIOBASE` only models the documented 0/16 bases (bit 4) and applies to
both sampling and publishing. Cross-block `IRQ` routing (RP2350 index modes
1/3) currently falls back to this block's flags, and the pad side of the
`PINCTRL` crossbar only reaches the modelled `SIO`/`PIO` drivers — see
*Wiring and not-modelled routing* under `IO_BANK0`.

PIO DREQ 电平已建模（见上文 *DMA requests*），但不是周期精确的：DMA 控制器按数据
单元采样，由宿主决定何时重新发布（`Machine` 在每个受节流的 DMA 批次内执行）。
RP2350 `RXFn_PUTGET*` 寄存器，以及 `0x16C..0x184` 的
`IRQ0_INTE`/`IRQ0_INTF`/`IRQ0_INTS`/`IRQ1_INTE`/`IRQ1_INTF`/`IRQ1_INTS`
中断寄存器尚未建模：共享句柄仅按原始标志断言中断线，没有逐标志使能与强制状态。
`GPIOBASE` 只建模文档化的 0/16 基址（位 4），采样与发布都适用。跨块 `IRQ` 路由
（RP2350 索引模式 1/3）目前回落到本块标志；`PINCTRL` 交叉开关的 pad 侧只接到已
建模的 `SIO`/`PIO` 驱动器——详见 `IO_BANK0` 的
*Wiring and not-modelled routing*。

---

## I2S / Audio tap

Firmware that plays audio over an I2S DAC usually streams 32-bit words into a
PIO TX FIFO, one interleaved stereo frame per word, with the DMA channel paced by
the state machine's TX DREQ. The emulator does **not** model the I2S bit clock or
the shift register, but it can hand the pushed words to the host as `i16`
samples.

固件通过 I2S DAC 播放音频时，通常由 DMA 通道按状态机 TX DREQ 节拍，把每个立体
声帧一个的 32 位字写入 PIO TX FIFO。模拟器**不**建模 I2S 位时钟与移位寄存器，
但可以把压入的字以 `i16` 样本形式交给宿主。

### Wiring point / 接线点

`Pio::write32` 对 `TXF0..TXF3` 的写入与 DMA 经总线写同一组偏移，都汇聚到
`Pio::push_tx(index, value)`；因此只需在这个函数里检查一次已安装的 tap：

```text
CPU  str  →  TXF0..TXF3 (MMIO)  ─┐
DMA  run  →  bus write PIO TXF  ─┴→ Pio::push_tx(sm, word) → AudioTap → AudioSink
```

- `AudioTap::new(AudioSink::new(rate_hz, channels, capacity))` creates the
  shared sink; `Pio::set_audio_tap(tap, sm)` (or `SharedPio::set_audio_tap`,
  `BootPeripherals::set_audio_tap(block_index, sm, tap)`) installs it on exactly
  one `(block, sm)` pair. Other state machines and other blocks are not captured.
- `BootPeripherals::mount` installs **no** tap: capture is a host/example
  decision. Without a tap the push path performs a single `Option` check and the
  FIFO behaviour is unchanged.
- `BootPeripherals::audio_tap()` returns a clone of the most recently installed
  handle; clones share one sink, so a playback plugin can drain it concurrently.
- Resetting a PIO block keeps the tap wiring but clears the captured samples.
- 每个块只捕获一个状态机；`BootPeripherals::mount` **不**自动安装 tap，
  未安装时压入路径只做一次 `Option` 判断；复位保留接线并清空已捕获样本。

### Sample format / 样本格式

`AudioSink::push_word(word)` appends two `i16` samples:

| Bits | Sample | Role |
|------|--------|------|
| `[15:0]` | `(word & 0xFFFF) as u16 as i16` | first sample of the frame (left) |
| `[31:16]` | `(word >> 16) as u16 as i16` | second sample (right) |

`0x8000_7FFF` therefore becomes `[32767, -32768]`. `take_samples(max)` drains
oldest-first and `rate_hz`/`channels` are metadata for the playback plugin (the
pokeemerald firmware uses 13 440 Hz stereo). 低半字先于高半字播放，两半都是二
进制补码 `i16`；`take_samples` 从最旧开始取走。

### Capacity and drops / 容量与丢弃策略

The sink is a bounded `VecDeque<i16>`. When it already holds `capacity` samples
a push evicts the **oldest** sample and increments `dropped()`; it never blocks,
never panics and never grows past the capacity. `clear()` discards buffered
samples but keeps the cumulative drop counter, and a zero-capacity sink drops
every sample. 缓冲为有界 `VecDeque<i16>`：满时淘汰**最旧**样本并递增
`dropped()`，不阻塞、不 panic、不超容量；`clear()` 清空样本但保留累计丢样计数。

```rust
use rp2350_periph::{AudioSink, AudioTap, BootPeripherals};

let tap = AudioTap::new(AudioSink::new(13_440, 2, 32_768));
peripherals.set_audio_tap(2, 0, tap.clone());
// ... run the firmware / 运行固件 ...
let samples: Vec<i16> = tap.take_samples(4096);
```

### Not modelled / 未建模项

I2S bit timing (BCK/LRCK edges, the Philips one-bit channel delay), the PIO
`OUT`/autopull shift register details and the DAC itself are not modelled: the
capture hook observes FIFO **pushes**, not the serialised wire, so sample values
and order are exact while their real-time pacing is not. DMA pacing, FIFO depth
and DREQ gating still follow the PIO/DMA models above. I2S 位时序
（BCK/LRCK 边沿、Philips 一比特声道延迟）、PIO `OUT`/autopull 移位寄存器细节与
DAC 本身未建模：捕获钩子观察的是 FIFO **压入**而非串行线上数据，因此样本值与
顺序精确、实时节拍不精确。DMA 节拍、FIFO 深度与 DREQ 门控仍遵循上文的
PIO/DMA 模型。

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
  gates FIFO popping; `EXPAND_EN` (bit 1) enables command expansion. `CLKDIV`
  (bits 31:28) sets the pixel/shift clock and `N_SHIFTS` (bits 20:16) paces the
  raw path; `CLKPHASE`, `SHIFT` and the coupled-mode bits are stored and read
  back but have no timing effect.
- `BIT0..BIT7` (`+0x04..+0x20`, mask `0x0003_1F1F`) are storage only.
  `EXPAND_SHIFT` (`+0x24`, reset `0x0100_0100`) and `EXPAND_TMDS` (`+0x28`) drive
  the pixel capture path: `ENC_SHIFT` (bits 20:16) sets the pixels per word,
  `Lx_NBITS` (bits 7:5 / 15:13 / 23:21) and `Lx_ROT` (bits 4:0 / 12:8 / 20:16)
  split each pixel into lane symbols.
- `HSTX_FIFO_STAT` (`+0x00`) — `LEVEL[7:0]`, `FULL` (8), `EMPTY` (9) derived
  from the queue; `WOF` (10) sticky write-while-full flag, write-one-to-clear.
- `HSTX_FIFO_FIFO` (`+0x04`) — write-only; reads return zero. Writes while full
  drop the word and set `WOF`.
- `+0x30` `IRQ_STATUS` (W1C) and `+0x34` `IRQ_ENABLE` are **model-only**
  registers, because silicon has no HSTX interrupt registers. Reserved/read-only
  bits are masked, and unmodelled offsets follow the project one-shot warning
  policy (read 0, write ignored).

- `CSR`（`+0x00`，复位 `0x1005_0600`，写掩码 `0xFF1F_1F73`）——`EN`（位 0）
  控制 FIFO 弹出，`EXPAND_EN`（位 1）启用命令扩展。`CLKDIV`（位 31:28）决定
  像素/移位时钟，`N_SHIFTS`（位 20:16）为原始路径计时；`CLKPHASE`、`SHIFT` 与
  耦合模式位仅存储/回读，无时序效果。
- `BIT0..BIT7`（`+0x04..+0x20`，掩码 `0x0003_1F1F`）仅存储。
  `EXPAND_SHIFT`（`+0x24`，复位 `0x0100_0100`）与 `EXPAND_TMDS`（`+0x28`）驱动
  像素捕获：`ENC_SHIFT`（位 20:16）决定每字像素数，`Lx_NBITS`（位
  7:5 / 15:13 / 23:21）与 `Lx_ROT`（位 4:0 / 12:8 / 20:16）把每个像素切成
  三路符号。
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
| `0x3` | `hstx::CMD_TMDS_REPEAT` | consume one word, emit it `count` times (one per pixel period) / 消费一个字并按 `count` 个像素周期重复输出 |
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
happens — matching the datasheet's "raw FIFO data passed directly" mode. This
path **is** paced now: the word goes straight into the output shift register,
which is clocked at `clk_hstx / CSR.CLKDIV` and shifts `CSR.N_SHIFTS` times per
word, so one word occupies

```text
CSR.N_SHIFTS * ceil(CSR.CLKDIV * clk_sys / clk_hstx)   clk_sys cycles
```

(`ceil(CLKDIV * clk_sys / clk_hstx)` `clk_sys` per shift; `clk_hstx` is the
`CLK_HSTX` channel, not an assumed `clk_sys / 2`). The validated DVI setup
(`N_SHIFTS = 5`, `CLKDIV = 5`, 252 MHz `CLK_SYS`, 126 MHz `CLK_HSTX`) therefore
takes 50 `clk_sys` per raw word, and a word only reaches the host queue when its
shift period completes — `advance(49)` does not consume it, `advance(1)` does.
`N_SHIFTS = 0` is modelled as one shift (and `CLKDIV = 0` as one, matching the
pixel-clock derivation); the datasheet's "0 means shift 32 times" / "CLKDIV 0
means a period of 16" encodings are not modelled.
`EXPAND_EN = 0` 时每个 FIFO 字都是原始输出数据，不做命令解码，对应数据手册的
“原始 FIFO 数据直通”模式。该路径**已计时**：字直接进入输出移位寄存器，移位
寄存器由 `clk_hstx / CSR.CLKDIV` 驱动、每字移位 `CSR.N_SHIFTS` 次，因此每字占用

```text
CSR.N_SHIFTS * ceil(CSR.CLKDIV * clk_sys / clk_hstx)   个 clk_sys 周期
```

（每次移位为 `ceil(CLKDIV * clk_sys / clk_hstx)` 个 `clk_sys`；`clk_hstx` 取自
`CLK_HSTX` 通道，不是假定的 `clk_sys / 2`）。已验证的 DVI 配置（`N_SHIFTS = 5`、
`CLKDIV = 5`、`CLK_SYS` 252 MHz、`CLK_HSTX` 126 MHz）因此每字占 50 个 `clk_sys`；
字的移位周期结束后才进入宿主队列——`advance(49)` 不消费它，`advance(1)` 才消费。
`N_SHIFTS = 0` 按 1 次移位处理（`CLKDIV = 0` 按 1 处理，与像素时钟推导一致）；
数据手册中“0 表示移位 32 次”“CLKDIV 0 表示周期 16”的编码未建模。

### Host-side output / 宿主侧输出

`advance(cycles)` is paced by the pixel clock, not by the clock count of words:
the pixel period is `ceil(CLKDIV * clk_sys / clk_hstx)` `clk_sys` cycles (10
cycles with 252 MHz `CLK_SYS`, 126 MHz `CLK_HSTX` and `CLKDIV = 5`; 20 cycles
when `CLK_HSTX` is divided by four instead). Every popped word occupies the pixel
time of the command that owns it — one period for `RAW`/`RAW_REPEAT`,
`32 / ENC_SHIFT` periods per word for `TMDS`, `count` periods for a repeat
command. A 256-word scanline block therefore lasts exactly 800 pixel periods
(8000 `clk_sys` cycles at 252/126 MHz with `CLKDIV = 5`), which is one
640x480p60 scanline; that is what keeps the expander in step with the DMA ring
instead of draining the FIFO 20x too fast and dropping words.
`advance(cycles)` 由像素时钟而非「每周期一字」节拍：像素周期为
`ceil(CLKDIV * clk_sys / clk_hstx)` 个 `clk_sys` 周期（`CLK_SYS` 252 MHz、
`CLK_HSTX` 126 MHz、`CLKDIV = 5` 时为 10 周期；把 `CLK_HSTX` 再四分频则为 20
周期）。每个被弹出的字占用其所属命令的像素时长——`RAW`/`RAW_REPEAT` 一个周期，
`TMDS` 每字 `32 / ENC_SHIFT` 个周期，重复命令为 `count` 个周期。因此一个 256 字
扫描行块恰好持续 800 个像素周期（252/126 MHz、`CLKDIV = 5` 时 8000 个 `clk_sys`
周期），即一条 640x480p60 扫描线；这正是扩展器与 DMA 环保持同步、而不是快 20 倍
排空 FIFO 并丢字的原因。

`advance(cycles)` pops at most one FIFO word per word period while `EN` is set,
expands it and queues the resulting 32-bit output words. `drain_output()` takes
the whole queue; `take_scanline()` is an alias for scanline-oriented hosts, and
`take_output_count()` discards it while returning the word count.
`output_underrun()` counts FIFO starvation episodes while a command still needs
data, and `advance` sets `IRQ_UNDERRUN`. `advance` does nothing when `EN` is
clear or the FIFO is idle, so an idle HSTX never produces pixels.
`output_words()` counts expanded words, `fifo_not_full()` is the `DREQ_HSTX`
level, and `fifo_overrun()` reports the sticky `WOF`. `reset()` restores the
reset values and clears FIFOs, counters and the shared IRQ line.
`advance(cycles)` 在 `EN` 置位时按字周期至多弹出一个 FIFO 字，展开并把
32 位输出字放入队列。`drain_output()` 取走整个队列；`take_scanline()` 是面向
扫描线宿主的别名，`take_output_count()` 丢弃队列并返回字数。
`output_underrun()` 统计命令仍需数据时的 FIFO 断流次数，`advance` 会置位
`IRQ_UNDERRUN`。`EN` 清零或 FIFO 空闲时 `advance` 不做任何事，因此空闲 HSTX
不会产生像素。`output_words()` 统计已展开字数，`fifo_not_full()` 即
`DREQ_HSTX` 电平，`fifo_overrun()` 报告粘滞的 `WOF`。`reset()` 恢复复位值并
清空 FIFO、计数器与共享中断线。

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
// CLKDIV = 0 → 2 clk_sys per pixel → three RAW words need 6 cycles.
// CLKDIV = 0 → 每像素 2 个 clk_sys → 三个 RAW 字需要 6 个周期。
peripherals.advance(6);
assert_eq!(peripherals.hstx_mut().drain_output(), [0x0011_2233, 0x0044_5566, 0x0077_8899]);
```

### Pixel capture / 像素捕获

`take_pixels()` drains the TMDS data words out of the host queue and returns
`HstxPixel { lanes: [u8; 3] }` values; `pixels_emitted()` counts every pixel
handed out so far (monotonic, cleared by `reset()`). A
`CMD_TMDS`/`CMD_TMDS_REPEAT` word carries `32 / ENC_SHIFT` pixels (`1` when
`ENC_SHIFT = 0`), pixel 0 being the lowest `ENC_SHIFT` bits with each following
pixel rotating the word one slot further. `EXPAND_TMDS` splits a pixel into
three lane symbols — lane 0 blue, lane 1 green, lane 2 red:
`take_pixels()` 把宿主队列中的 TMDS 数据字取走并返回
`HstxPixel { lanes: [u8; 3] }`；`pixels_emitted()` 累计已交付的像素数（只增，
`reset()` 清零）。一个 `CMD_TMDS`/`CMD_TMDS_REPEAT` 字承载 `32 / ENC_SHIFT` 个
像素（`ENC_SHIFT = 0` 时为 1），像素 0 取最低 `ENC_SHIFT` 位，后续像素依次再旋转
一个槽位。`EXPAND_TMDS` 把像素切成三路通道符号——通道 0 蓝、通道 1 绿、通道 2 红：

```text
symbol = bits [7 : 8 - width] of word.rotate_right(Lx_ROT + pixel * ENC_SHIFT)
width  = Lx_NBITS + 1      (1..=8, "starting from bit 7 of the rotated data")
```

With the validated GBA firmware setup (`csr = EXPAND_EN | CLKDIV 5 | N_SHIFTS 5 |
SHIFT 2 | EN`, `expand_shift = ENC_N_SHIFTS 2 | ENC_SHIFT 16`,
`expand_tmds = L2 red 5b @ rot 8 | L1 green 6b @ rot 3 | L0 blue 5b @ rot 29`) the
word `0xF800_07E0` decodes to two pixels, `[0, 63, 0]` (green `0x07E0`) and
`[0, 0, 31]` (red `0xF800`) — RGB565. These symbols are **pre-encoding** values:
TMDS 10-bit coding, DC balancing, DVI/HDMI bit timing, `BITx` routing and pin
timing are still unmodelled. When the lane config cannot be decoded
(`EXPAND_TMDS` all zero, or `ENC_SHIFT = 0`) each pixel instead carries the low 8
bits of its raw slice in `lanes[0]` with `lanes[1]`/`lanes[2]` zero, so the
payload stays observable. Non-TMDS words (raw-mode words and
`CMD_RAW`/`CMD_RAW_REPEAT` sync payloads) stay in the queue for `drain_output()`,
and with `EXPAND_EN = 0` `take_pixels()` returns nothing because no word is TMDS
data.
在已验证的 GBA 固件配置下（`csr = EXPAND_EN | CLKDIV 5 | N_SHIFTS 5 | SHIFT 2 | EN`，
`expand_shift = ENC_N_SHIFTS 2 | ENC_SHIFT 16`，
`expand_tmds = L2 红 5 位 @ rot 8 | L1 绿 6 位 @ rot 3 | L0 蓝 5 位 @ rot 29`），
字 `0xF800_07E0` 解码为两个像素：`[0, 63, 0]`（绿 `0x07E0`）与 `[0, 0, 31]`
（红 `0xF800`），即 RGB565。这些符号是**编码前**取值：TMDS 10 位编码、直流平衡、
DVI/HDMI 位时序、`BITx` 路由与引脚时序仍未建模。当通道配置无法可靠解码
（`EXPAND_TMDS` 全 0，或 `ENC_SHIFT = 0`）时，每个像素改为在 `lanes[0]` 携带其
原始切片的低 8 位、`lanes[1]`/`lanes[2]` 为 0，使载荷仍可观察。非 TMDS 字
（原始模式字与 `CMD_RAW`/`CMD_RAW_REPEAT` 同步载荷）留在队列中供
`drain_output()` 取用；`EXPAND_EN = 0` 时没有 TMDS 数据，`take_pixels()` 返回空。

```rust
// Rebuild RGB565 from the captured lane symbols / 用捕获的通道符号重建 RGB565
for pixel in peripherals.hstx_mut().take_pixels() {
    let rgb565 = (u16::from(pixel.lanes[2]) << 11)
        | (u16::from(pixel.lanes[1]) << 5)
        | u16::from(pixel.lanes[0]);
    let _ = rgb565;
}
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
shift register bit ordering and `BITx` pin crossbar routing, GPIO pin mapping and
PIO coupled mode. TMDS command data words are kept unchanged in the word queue;
`take_pixels()` only splits them into pre-encoding lane symbols (see
`HstxPixel`), so a host can rebuild RGB565/RGB888 but cannot recover the
serialised TMDS stream. No scanline/frame boundary or DVI pixel timing is
provided, and `CMD_TMDS_REPEAT` emits one word copy per pixel period, so a
repeat run surfaces `count` decoded word copies. `DREQ_HSTX` (52) is modelled
and asserted while the FIFO has room.
DVI/HDMI 位时序、TMDS 编码器与 10 位符号打包、输出移位寄存器的位序与 `BITx`
引脚交叉开关、GPIO 引脚映射以及 PIO 耦合模式均未建模。TMDS 命令的数据字在字队列
中保持原样；`take_pixels()` 只把它们切成编码前的通道符号（见 `HstxPixel`），
因此宿主可以重建 RGB565/RGB888，但无法还原串行化的 TMDS 码流。模型不提供
扫描线/帧边界或 DVI 像素时序，且 `CMD_TMDS_REPEAT` 按每个像素周期输出一个字的
副本，因此一次重复运行会表现为 `count` 个已解码字副本。`DREQ_HSTX`（52）已建模，
FIFO 有空间时断言。

---

## DMA

`BootPeripherals::mount` installs one `Dma` at `0x5000_0000` (`map::DMA_BASE`,
window `map::DMA_SIZE` = 16 KB including the SET/CLR/XOR alias pages). Clones
share a single register file, so the bus-mounted device and the handles returned
by `BootPeripherals::dma()` / `dma_mut()` observe the same state.
`BootPeripherals::mount` 在 `0x5000_0000`（`map::DMA_BASE`，窗口
`map::DMA_SIZE` = 16 KB，含 SET/CLR/XOR 别名页）挂载一个 `Dma`。克隆共享同一个
寄存器文件，因此总线上的设备与 `BootPeripherals::dma()` / `dma_mut()` 返回的
句柄看到相同状态。

```rust
pub struct Dma { /* shared register file + IrqAssert + optional BusPerf */ }

impl Dma {
    pub fn new(irq: IrqAssert) -> Self;
    pub fn with_perf(irq: IrqAssert, perf: BusPerf) -> Self;
    pub fn perf(&self) -> Option<&BusPerf>;
    pub fn bus_counters(&self) -> DmaBusCounters;
    pub fn run(&mut self, bus: &mut dyn AddressBus, max_transfers: u64) -> u64;
    pub fn advance(&mut self, cycles: u64);
    pub fn set_dreq(&self, dreq: u8, asserted: bool);
    pub fn set_dreq_bank(&self, first: u8, levels: u32);
    pub fn dreq(&self, dreq: u8) -> Option<bool>;
    pub fn busy_selects_dreq(&self, first: u8, last: u8) -> bool;
    pub fn abort(&mut self, mask: u16);
    pub fn busy_mask(&self) -> u16;
    pub fn channel_state(&self, index: usize) -> Option<DmaChannelState>;
}

pub struct DmaBusCounters {
    pub read_units: u64,    // completed reads / 完成的读单元
    pub write_units: u64,   // completed writes / 完成的写单元
    pub read_cycles: u64,   // bus cycles consumed by reads / 读占用的总线周期
    pub write_cycles: u64,  // bus cycles consumed by writes / 写占用的总线周期
    pub grants: u64,        // arbitration grants / 仲裁授予次数
}

pub struct DmaChannelState {
    pub read_addr: u32,
    pub write_addr: u32,
    pub trans_count: u32,
    pub trans_count_mode: u8,
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
| `+0x08` | `TRANS_COUNT` | remaining count (`[27:0]`) + transfer mode (`[31:28]`) / 剩余计数（`[27:0]`）+ 传输模式（`[31:28]`） |
| `+0x0C` | `CTRL_TRIG` | control + trigger / 控制 + 触发 |
| `+0x10..=0x3C` | `AL1_*`/`AL2_*`/`AL3_*` | read/write/count/control aliases. Only `CTRL_TRIG` and the `*_TRIG` aliases (`AL1_TRANS_COUNT_TRIG`, `AL2_WRITE_ADDR_TRIG`, `AL3_READ_ADDR_TRIG`) trigger; `AL1_CTRL`/`AL2_CTRL`/`AL3_CTRL` are plain, non-triggering writes / 读/写/计数/控制别名。只有 `CTRL_TRIG` 与 `*_TRIG` 别名（`AL1_TRANS_COUNT_TRIG`、`AL2_WRITE_ADDR_TRIG`、`AL3_READ_ADDR_TRIG`）会触发；`AL1_CTRL`/`AL2_CTRL`/`AL3_CTRL` 是普通的非触发写入 |
| `0x400` | `INTR` | raw 16-bit status, W1C / 16 位原始状态，写 1 清零 |
| `0x404`/`0x414`/`0x424`/`0x434` | `INTE0..3` | per-line enable / 每条线的使能 |
| `0x408`/`0x418`/`0x428`/`0x438` | `INTF0..3` | per-line force / 每条线的强制位 |
| `0x40C`/`0x41C`/`0x42C`/`0x43C` | `INTS0..3` | `(INTR & INTEn) \| INTFn`, also W1C through `INTR` / 状态，写 `INTR` 同样可清 |
| `0x440..=0x44C` | `TIMER0..3` | pacing timers: `X` dividend in `[31:16]`, `Y` divisor in `[15:0]` / 节拍定时器：`[31:16]` 为被除数 `X`，`[15:0]` 为除数 `Y` |
| `0x450` | `MULTI_CHAN_TRIGGER` | start the selected channels / 启动选中通道 |
| `0x454`/`0x458` | `SNIFF_CTRL`/`SNIFF_DATA` | `EN[0]`, `DMACH[4:1]`, `CALC[8:5]`, `BSWAP[9]`, `OUT_REV[10]`, `OUT_INV[11]`; seed + accumulator / 控制与种子兼累加器 |
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
`CTRL_TRIG` 位域遵循 RP2350 布局：`EN[0]`、`HIGH_PRIORITY[1]`、`DATA_SIZE[3:2]`、
`INCR_READ[4]`、`INCR_WRITE[6]`、`RING_SIZE[11:8]`、`RING_SEL[12]`、
`CHAIN_TO[16:13]`、`TREQ_SEL[22:17]`、`IRQ_QUIET[23]`、`BSWAP[24]`、
`SNIFF_EN[25]`、`BUSY[26]`、`WRITE_ERROR[29]`（写 1 清零）、`READ_ERROR[30]`
（写 1 清零）、`AHB_ERROR[31]`。`BUSY` 只读；其余可写位在写 `CTRL_TRIG`
时被替换。完成传输后 `EN` 保留（SDK 会显式清除）；写 `EN = 0` 会同时清除
`EN` 与 `BUSY`。

### Transfer semantics / 传输语义

- `TRANS_COUNT` (`0x08`) and the non-triggering count aliases (`AL2_TRANS_COUNT`,
  `AL3_TRANS_COUNT`) latch a **rewrite** value together with the `[31:28]`
  transfer mode. A trigger copies the count into the working count that
  `Dma::run` decrements; reads of `TRANS_COUNT` and
  `DmaChannelState::trans_count` return the working count, while
  `DmaChannelState::trans_count_mode` exposes the latched mode. Writing the
  count while a channel is busy therefore only changes the value used by the
  next start, exactly like the hardware reload register.
  `TRANS_COUNT`（`0x08`）与非触发计数别名（`AL2_TRANS_COUNT`、`AL3_TRANS_COUNT`）
  锁存**重载**值以及 `[31:28]` 传输模式。触发器把计数装载到 `Dma::run` 递减的
  工作计数；读 `TRANS_COUNT` 与 `DmaChannelState::trans_count` 返回工作计数，
  `DmaChannelState::trans_count_mode` 暴露锁存的模式。因此通道忙碌时写计数只改变
  下一次启动使用的值，与硬件重载寄存器一致。
- Triggers are `CTRL_TRIG`, `AL1_TRANS_COUNT_TRIG`, `AL2_WRITE_ADDR_TRIG`,
  `AL3_READ_ADDR_TRIG` and `MULTI_CHAN_TRIGGER`. A trigger starts only a channel
  whose `EN` is set and which is not already busy; `MULTI_CHAN_TRIGGER` skips
  disabled channels and a trigger on a busy channel neither restarts it nor
  reloads its count. `AL1_CTRL`/`AL2_CTRL`/`AL3_CTRL` never trigger — this is
  what `dma_channel_configure(..., false)` relies on. A zero-length trigger
  completes immediately: it raises `INTR` unless `IRQ_QUIET` and then follows
  `CHAIN_TO`; a zero write to a `*_TRIG` alias is a *null trigger* that raises
  the completion interrupt when `IRQ_QUIET` is set.
  触发器为 `CTRL_TRIG`、`AL1_TRANS_COUNT_TRIG`、`AL2_WRITE_ADDR_TRIG`、
  `AL3_READ_ADDR_TRIG` 与 `MULTI_CHAN_TRIGGER`。触发器只启动 `EN` 置位且不忙碌
  的通道；`MULTI_CHAN_TRIGGER` 跳过禁用通道，忙碌通道上的触发既不重启也不重载
  计数。`AL1_CTRL`/`AL2_CTRL`/`AL3_CTRL` 永不触发——`dma_channel_configure(...,
  false)` 正依赖这一点。零长度触发立即完成：未置 `IRQ_QUIET` 时置位 `INTR`，
  随后跟踪 `CHAIN_TO`；向 `*_TRIG` 别名写零是*空触发*，在置位 `IRQ_QUIET` 时
  置位完成中断。
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
  `(addr & !mask) | ((addr + step) & mask)`. The wrap is applied after every
  unit, so a `TRANS_COUNT` larger than the ring re-streams the window (the HSTX
  line blocks use `RING_SIZE = 10` with `count = 512` to emit a 1024 B line
  twice).
  `RING_SIZE` 是环字节数的 log2。非零时，`RING_SEL = 0` 环绕读侧、
  `RING_SEL = 1` 环绕写侧，公式为 `(addr & !mask) | ((addr + step) & mask)`。
  环绕在每个单元之后应用，因此大于环的 `TRANS_COUNT` 会重复流出窗口（HSTX 行块
  用 `RING_SIZE = 10` 与 `count = 512` 把 1024 B 行输出两次）。
- `TRANS_COUNT` decrements after each unit and saturates at zero; reaching zero
  clears `BUSY`, sets `INTR[channel]` unless `IRQ_QUIET`, and starts
  `CHAIN_TO`. The return value of `run` counts attempted units, so a driver can
  cap DMA work per CPU timeslice.
  `TRANS_COUNT` 每个单元后递减并在零处饱和；到零时清除 `BUSY`、在未置
  `IRQ_QUIET` 时置位 `INTR[channel]`，并启动 `CHAIN_TO`。`run` 的返回值是
  尝试的单元数，驱动可据此限制每个 CPU 时间片内的 DMA 工作量。
- `TRANS_COUNT.MODE` (`[31:28]`) selects the completion behaviour:
  `0x0` **NORMAL** (above), `0x1` **TRIGGER_SELF** re-triggers the channel from
  the reload count after the completion IRQ and `CHAIN_TO` have been handled
  (so a single trigger keeps a self-paced channel alive), and `0xF` **ENDLESS**
  freezes the working count: it never decrements, never completes, never chains
  and never raises an interrupt until `CHAN_ABORT`. Addresses and the ring wrap
  keep advancing in ENDLESS mode. Any other value is treated as NORMAL.
  `TRANS_COUNT.MODE`（`[31:28]`）选择完成行为：`0x0` **NORMAL**（同上）、
  `0x1` **TRIGGER_SELF** 在处理完成中断与 `CHAIN_TO` 后用重载计数重新触发自身
  （因此一次触发即可让自节拍通道持续运行）、`0xF` **ENDLESS** 冻结工作计数：
  不递减、永不完成、不链式、不产生中断，直到 `CHAN_ABORT`。ENDLESS 下地址与
  环绕仍继续推进。其它值按 NORMAL 处理。
- `CTRL_TRIG.BSWAP` transforms every unit on the write side: byte transfers are
  unaffected, halfwords swap their two bytes (`0x1234` → `0x3412`) and words
  reverse all four bytes (`0x1122_3344` → `0x4433_2211`).
  `CTRL_TRIG.BSWAP` 在写侧变换每个单元：字节传输不受影响，半字交换两个字节
  （`0x1234` → `0x3412`），字反转四个字节（`0x1122_3344` → `0x4433_2211`）。
- `CTRL_TRIG.SNIFF_EN` feeds the observed channel's read units into the checksum
  engine. All three conditions are required: `SNIFF_CTRL.EN[0]` is set, the
  channel carries `CTRL_TRIG.SNIFF_EN`, and the channel index equals
  `SNIFF_CTRL.DMACH[4:1]`. The sniffer sits **downstream of the channel byte
  swap**, so it observes the already `CTRL_TRIG.BSWAP`-transformed unit and
  `SNIFF_CTRL.BSWAP[9]` then reverses it again — with both bits set the two
  swaps cancel, matching the datasheet note. `SNIFF_DATA` (`0x458`) is both the
  seed written by software (for example `0xFFFF_FFFF` before CRC-32) and the
  running accumulator: each observed unit updates it, a read applies only
  `OUT_REV[10]` (bit reverse) and `OUT_INV[11]` (complement) to the presented
  value, and a read never clears it (only a write sets the accumulator).
  `SNIFF_CTRL.CALC[8:5]` follows `hardware/regs/dma.h`: `0x0` CRC-32/IEEE
  (reflected polynomial `0xEDB8_8320`, i.e. the 802.3/zlib bit order), `0x1` the
  same CRC with bit-reversed input bytes, `0x2` CRC-16-CCITT (`0x1021`), `0x3`
  CRC-16-CCITT with bit-reversed input (`0x8408`), `0xE` the XOR reduction
  (parity of all data bits, result 0 or 1) and `0xF` a 32-bit addition checksum
  (`wrapping_add`, masked to the unit width). `0x4..=0xD` are reserved by the
  header — no other algorithms exist — and leave the accumulator unchanged.
  Known vectors (all over the bytes of `"123456789"`): seed `0xFFFF_FFFF`,
  `CALC = 0x0`, `OUT_INV = 1` → `0xCBF4_3926`; seed `0xFFFF_FFFF`,
  `CALC = 0x1` → `0xE767_6EC0` (bit reverse of the CRC-32/BZIP2 register
  `0x0376_E6E7`); seed `0xFFFF`, `CALC = 0x2` → `0x29B1` and `CALC = 0x3` →
  `0x8D94` (bit reverse of `0x29B1`).
  `CTRL_TRIG.SNIFF_EN` 把被观察通道读出的数据送入校验引擎。需要同时满足三个
  条件：`SNIFF_CTRL.EN[0]` 置位、通道置 `CTRL_TRIG.SNIFF_EN`、且通道号等于
  `SNIFF_CTRL.DMACH[4:1]`。嗅探器位于**通道字节交换之后**，因此它观察的是已按
  `CTRL_TRIG.BSWAP` 变换过的单元，`SNIFF_CTRL.BSWAP[9]` 再反转一次——两个位同时
  置位时相互抵消，与数据手册说明一致。`SNIFF_DATA`（`0x458`）既是软件写入的种子
  （例如 CRC-32 前写 `0xFFFF_FFFF`），也是运行累加器：每次观察到的单元都会更新
  它；读出只对展示值应用 `OUT_REV[10]`（位反转）与 `OUT_INV[11]`（取反），且读出
  从不清零（只有写入才会设置累加器）。`SNIFF_CTRL.CALC[8:5]` 遵循
  `hardware/regs/dma.h`：`0x0` CRC-32/IEEE（反射多项式 `0xEDB8_8320`，即
  802.3/zlib 位序）、`0x1` 同上但输入字节位反转、`0x2` CRC-16-CCITT（`0x1021`）、
  `0x3` 输入位反转的 CRC-16-CCITT（`0x8408`）、`0xE` 全部数据位异或折叠（奇偶，
  结果 0/1）、`0xF` 32 位加法校验和（`wrapping_add`，按单元宽度掩码）。
  `0x4..=0xD` 在头文件中为保留值——不存在其它算法——累加器保持不变。
  已知向量（数据均为 `"123456789"` 的字节）：种子 `0xFFFF_FFFF`、`CALC = 0x0`、
  `OUT_INV = 1` → `0xCBF4_3926`；种子 `0xFFFF_FFFF`、`CALC = 0x1` → `0xE767_6EC0`
  （CRC-32/BZIP2 寄存器 `0x0376_E6E7` 的位反转）；种子 `0xFFFF`、`CALC = 0x2` →
  `0x29B1`、`CALC = 0x3` → `0x8D94`（`0x29B1` 的位反转）。
- `CTRL_TRIG.HIGH_PRIORITY` changes arbitration: in every scheduling round all
  eligible high-priority channels are served before any normal channel, while
  channels of the same priority keep round-robin order. A busy high-priority
  channel therefore starves normal channels (this model does not limit the
  number of high-priority issues per round).
  `CTRL_TRIG.HIGH_PRIORITY` 改变仲裁：每个调度轮次中所有合格的高优先级通道
  都先于任何普通通道被服务，同优先级通道保持轮转顺序。因此忙碌的高优先级通道
  会让普通通道饿死（本模型不限制每轮的高优先级发射次数）。
- `CHAN_ABORT` / `Dma::abort(mask)` clears `BUSY` but keeps `EN`, the addresses
  and the remaining count, matching the datasheet; a new trigger restarts the
  channel and reloads `TRANS_COUNT` from the rewrite value (the partial count is
  not resumed).
  `CHAN_ABORT` / `Dma::abort(mask)` 清除 `BUSY`，但保留 `EN`、地址与剩余计数，
  与数据手册一致；再次触发会重启通道并从重载值重新装载 `TRANS_COUNT`（不会续接
  部分计数）。
- A bus read or write error latches `READ_ERROR` / `WRITE_ERROR`, clears
  `BUSY` and raises no completion interrupt.
  总线读/写错误会锁存 `READ_ERROR` / `WRITE_ERROR`、清除 `BUSY`，且不产生
  完成中断。

### Bus accounting / 总线记账

Every executed unit performs one bus read and one bus write, and both report the
bus cycles they consumed through the `cycles` out-parameter of
`AddressBus`. `Dma::bus_counters` publishes those totals per master together
with the number of arbitration grants `Dma::run` handed out, so a host can see
how much of the bus the DMA claimed without a cycle-exact arbiter:
每个执行单元执行一次总线读与一次总线写，二者都通过 `AddressBus` 的 `cycles`
出参回报消耗的总线周期。`Dma::bus_counters` 按主设备发布这些总计以及
`Dma::run` 发出的仲裁授予次数，因此宿主无需周期精确仲裁器也能看到 DMA 占用了
多少总线：

```rust
pub struct DmaBusCounters {
    pub read_units: u64,
    pub write_units: u64,
    pub read_cycles: u64,
    pub write_cycles: u64,
    pub grants: u64,
}
```

`Dma::with_perf(irq, busctrl.perf())` additionally reports every completed
access into the `BUSCTRL` performance counters: the access address is attributed
to a crossbar downstream port (`busctrl::port_of_address`: ROM, XIP main 0/1,
the ten SRAM banks, APB, fast peripheral) and `PERFSELn = ACCESS` counts one,
`PERFSELn = STALL_UPSTREAM` adds the consumed cycles. `Dma::new` leaves the
controller unwired, which is what `BootPeripherals::mount` uses: the counters
then store `PERFSEL`/`PERFCTR_EN` correctly but nothing increments them. Wiring
`Dma` to `BusCtrl` inside `BootPeripherals` needs a change in `boot.rs`.
`Dma::with_perf(irq, busctrl.perf())` 还会把每次完成的访问上报到 `BUSCTRL`
性能计数器：访问地址被归属到交叉开关下游端口（`busctrl::port_of_address`：
ROM、XIP main 0/1、十个 SRAM 存储体、APB、快速外设），`PERFSELn = ACCESS`
计 1，`PERFSELn = STALL_UPSTREAM` 累加消耗的周期。`Dma::new` 不接线，这正是
`BootPeripherals::mount` 目前的用法：计数器仍能正确保存 `PERFSEL`/`PERFCTR_EN`，
但不会有任何递增。要让 `BootPeripherals` 内的 `Dma` 与 `BusCtrl` 接线需要在
`boot.rs` 中改动一行。

### DREQ / DREQ 请求

`TREQ_SEL = 0x3F` (`TREQ_PERMANENT`) always transfers. Every other selector
samples the host-driven level array; all inputs reset asserted except the pacing
timer selectors, so firmware that never programs a DREQ still progresses. `set_dreq(sel, false)` pauses
channels using that selector and `set_dreq(sel, true)` resumes them on the next
`run`; selectors `>= 64` are ignored. `set_dreq_bank(first, levels)` writes
eight consecutive inputs from one bitmask and one lock, which is how the PIO
blocks publish four TX and four RX levels at once. The levels are sampled per
`Dma::run` call (and per unit inside `BootPeripherals::pump_dma` for PIO-paced
channels), not continuously. Selectors `0x3B..=0x3E` (`DREQ_DMA_TIMER0..3`)
are driven by the four pacing timers described below.
`TREQ_SEL = 0x3F`（`TREQ_PERMANENT`）始终可传输。其他选择器采样宿主驱动的
电平数组；除节拍定时器选择器外，输入复位时均断言，因此未编程 DREQ 的固件仍能
推进。
`set_dreq(sel, false)` 暂停使用该选择器的通道，`set_dreq(sel, true)` 在下一次
`run` 时恢复；`>= 64` 的选择被忽略。`set_dreq_bank(first, levels)` 用一次加锁从
位掩码写入连续八个输入，PIO 块据此一次发布四个 TX 与四个 RX 电平。电平在每次
`Dma::run` 调用时采样（PIO 节拍的通道在 `BootPeripherals::pump_dma` 内逐单元
采样），不是连续采样。`0x3B..=0x3E`（`DREQ_DMA_TIMER0..3`）由下述四个节拍
定时器驱动。

The pacing timers are `TIMER0..3` (`0x440..=0x44C`): `X` (dividend) is in
`bits[31:16]` and `Y` (divisor) in `bits[15:0]`, giving a request rate of
`(X / Y) * sys_clk` — `X == Y` requests every DMA cycle and `X = 1, Y = 4`
every four cycles. `Dma::advance(cycles)` adds `X * cycles` to a fractional
accumulator, queues `accumulator / Y` pulses and keeps the remainder for the
next call. The asserted level carries one queued pulse per transfer: the first
channel transfer that samples `DREQ_DMA_TIMER0..3` (`59..=62`) consumes one
pulse, and the level stays asserted while pulses remain. `X = 0` or `Y = 0`
never asserts (the latter also avoids a division by zero); both fields reset to
zero, so firmware that never programs a timer sees no request.
Writing `TIMERx` clears the fractional accumulator and queued pulses.
节拍定时器为 `TIMER0..3`（`0x440..=0x44C`）：`X`（被除数）在 `bits[31:16]`，
`Y`（除数）在 `bits[15:0]`，请求速率为 `(X / Y) * sys_clk`——`X == Y` 时每个
DMA 周期一次，`X = 1, Y = 4` 时每四个周期一次。`Dma::advance(cycles)` 把
`X * cycles` 累加到小数累加器，排队 `累加器 / Y` 个脉冲并保留余数供下次调用。
断言电平每放行一次传输消费一个排队脉冲：第一个采样到 `DREQ_DMA_TIMER0..3`
（`59..=62`）的通道传输消费一个，仍有剩余脉冲时电平保持断言。`X = 0` 或
`Y = 0` 时永不断言（后者同时避免除零）；两个字段的复位值都是 0，因此从未编程
定时器的固件不会看到请求。写 `TIMERx` 会清空小数累加器与已排队脉冲。

The RP2350 PIO selectors are `0..=23` (`PIO0_TX0` … `PIO2_RX3`), with
`PIOx_TXy` asserted while the TX FIFO is not full and `PIOx_RXy` while the RX
FIFO is not empty; the numbering and join behaviour are documented under
*PIO → DMA requests*. `BootPeripherals::sync_pio_dreqs()` publishes them and
`BootPeripherals::pump_dma()` uses `busy_selects_dreq(0, 23)` to refresh the
levels between single units. `DREQ_HSTX` (52) is published by
`sync_hstx_dreq()` and the pacing timers drive `59..=62`; the SPI/UART/ADC
selectors stay reset-asserted and are not driven by their peripherals.
RP2350 的 PIO 选择器为 `0..=23`（`PIO0_TX0` … `PIO2_RX3`）：`PIOx_TXy` 在 TX
FIFO 未满时断言，`PIOx_RXy` 在 RX FIFO 非空时断言；编号与 join 行为见
*PIO → DMA requests*。`BootPeripherals::sync_pio_dreqs()` 负责发布，
`BootPeripherals::pump_dma()` 用 `busy_selects_dreq(0, 23)` 在逐单元之间刷新
电平。`DREQ_HSTX`(52) 由 `sync_hstx_dreq()` 发布，节拍定时器驱动 `59..=62`；
SPI/UART/ADC 选择器仍为复位断言状态，尚未由对应外设驱动。

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

### Control-block ring / 控制块环

The HSTX scanout uses the classic three-channel SDK ring, which the model
reproduces end-to-end (`dma_tests::hstx_control_block_ring_streams_three_laps`):
`DATA` streams a 1024 B line block into the HSTX FIFO (`RING_SIZE = 10`,
`TREQ_SEL = DREQ_HSTX`, `CHAIN_TO = CB_COUNT`); `CB_COUNT` streams the next
`TRANS_COUNT` word from a 1024 B ring into `DATA.AL3_TRANS_COUNT`
(`CHAIN_TO = CB_ADDR`); `CB_ADDR` streams the next read address from a second
1024 B ring into `DATA.AL3_READ_ADDR_TRIG`, which triggers `DATA` again. The CB
channels keep `TRANS_COUNT = 1` forever, so the reload-on-trigger rule is what
keeps the ring alive; a single missing reload stalls scanout permanently.
HSTX 扫描输出使用 SDK 经典的三通道环，模型端到端复现
（`dma_tests::hstx_control_block_ring_streams_three_laps`）：`DATA` 把 1024 B
行块流入 HSTX FIFO（`RING_SIZE = 10`、`TREQ_SEL = DREQ_HSTX`、
`CHAIN_TO = CB_COUNT`）；`CB_COUNT` 从 1024 B 环流出一个 `TRANS_COUNT` 字到
`DATA.AL3_TRANS_COUNT`（`CHAIN_TO = CB_ADDR`）；`CB_ADDR` 从第二个 1024 B 环
流出一个读地址到 `DATA.AL3_READ_ADDR_TRIG`，从而再次触发 `DATA`。CB 通道的
`TRANS_COUNT` 永远保持 1，因此“触发时重载”是环持续运行的关键；漏掉一次重载
就会让扫描输出永久停摆。

### Not modelled / 未建模项

The per-channel debug registers at `0x800..=0xBFF`, AHB error recovery beyond
clearing `BUSY`, and cycle-exact bus/arbitration timing are not modelled.
Contention, downstream stalls and preemption need a shared multi-master bus
model, so `BUSCTRL.PERFSEL` events `STALL_DOWNSTREAM`/`ACCESS_CONTESTED` stay 0
and a DMA access is never stalled by CPU traffic. `SNIFF_CTRL.CALC` values
`0x4..=0xD` are reserved in `hardware/regs/dma.h` (only `0x0..=0x3`, `0xE` and
`0xF` exist) and leave the accumulator unchanged. The DMA device never
intercepts CPU bus traffic: transfers happen only inside `Dma::run`.
`0x800..=0xBFF` 的通道调试寄存器、除清除 `BUSY` 之外的 AHB 错误恢复，以及周期
精确的总线/仲裁时序未建模。争用、下游 stall 与抢占需要共享的多主设备总线模型，
因此 `BUSCTRL.PERFSEL` 的 `STALL_DOWNSTREAM`/`ACCESS_CONTESTED` 事件保持 0，
DMA 访问也不会被 CPU 流量 stall。`SNIFF_CTRL.CALC` 的 `0x4..=0xD` 在
`hardware/regs/dma.h` 中为保留值（只存在 `0x0..=0x3`、`0xE`、`0xF`），累加器
保持不变。DMA 设备不拦截 CPU 总线流量：传输只在 `Dma::run` 内部发生。

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

`pump_dma` applies two pacing rules so a batch cannot overrun a FIFO:

- A **busy** PIO-paced channel (`busy_selects_dreq(0, 23)`) advances one unit per
  `dma.run` call, because a PIO FIFO can fill inside the batch.
- When any **enabled** channel selects `DREQ_HSTX` (52), the batch budget is
  capped at `Hstx::fifo_room()`, so at most that many words can enter the 8-word
  HSTX FIFO no matter how the round-robin cursor interleaves other channels.
  `enabled_selects_dreq` deliberately ignores `BUSY`: a channel is enabled before
  `dma.run` marks it busy, and using the busy flag here let a batch run with a
  stale DREQ level, overfilling the FIFO, dropping a word (`WOF`) and
  desynchronising the HSTX command/data phase.
  `pump_dma` 用两条节拍规则避免批次冲垮 FIFO：忙碌的 PIO 节拍通道逐单元执行；
  只要有已使能通道选中 `DREQ_HSTX`(52)，批次预算就被限制为 `Hstx::fifo_room()`。
  `enabled_selects_dreq` 有意忽略 `BUSY`：通道在 `dma.run` 置 busy 之前就已使能，
  若用 busy 判定，整批会用陈旧 DREQ 电平执行、灌满 FIFO、丢字（`WOF`）并让 HSTX
  命令/数据相位失步。

`pump_dma` also advances the pacing timers at the start of every call with
`Dma::advance(64)` (the fixed `DMA_PUMP_CYCLES` budget), so `TIMER0..3` queue
`(X * 64) / Y` pulses per pump without the host threading CPU cycle deltas
through the hook. The cadence is approximate but deterministic, and a reset
timer (`X = 0`) never asserts, so timer-less firmware is unaffected.
`pump_dma` 还在每次调用开始时用 `Dma::advance(64)`（固定的 `DMA_PUMP_CYCLES`
预算）推进节拍定时器，因此 `TIMER0..3` 每次泵送排队 `(X * 64) / Y` 个脉冲，
无需宿主把 CPU 周期差传入钩子。节奏是近似但确定的；复位的定时器（`X = 0`）
永不断言，因此不使用定时器的固件不受影响。

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

## TIMER / 定时器

RP2350 exposes two identical timer blocks. Both share **one** 64-bit
microsecond counter driven by the reference domain of the simulation timebase
(`Timebase::timer_us`): the datasheet clocks `TIMELR`/`TIMERAWH` from `clk_ref`
through the TICKS divider (`TICK = clk_ref / 1 MHz`), so the microsecond rate is
independent of runtime `clk_sys` overclocking. `ALARM0..3`, `ARMED`, `INTR`,
`INTE`, `INTF` and `INTS` are per block.
RP2350 提供两个相同的定时器块。它们共享**同一个** 64 位微秒计数器（由仿真时基的
参考域 `Timebase::timer_us` 驱动）：数据手册中 `TIMELR`/`TIMERAWH` 由 `clk_ref` 经
TICKS 分频（`TICK = clk_ref / 1 MHz`），因此微秒速率与运行期 `clk_sys` 超频无关。
`ALARM0..3`、`ARMED`、`INTR`、`INTE`、`INTF`、`INTS` 均按块独立。

| Block | Base | Alarm lines |
| --- | --- | --- |
| TIMER0 | `0x400B_0000` | `TIMER0_IRQ_0..3` = `0..=3` |
| TIMER1 | `0x400B_8000` | `TIMER1_IRQ_0..3` = `4..=7` |

### Register offsets / 寄存器偏移

| Offset | Register | Model |
| --- | --- | --- |
| `0x00` | `TIMEHW` | write-only, reads as 0 / 只写，读回 0 |
| `0x04` | `TIMELW` | write-only, reads as 0 / 只写，读回 0 |
| `0x08` | `TIMEHR` | high 32 bits of the counter / 计数器高 32 位 |
| `0x0C` | `TIMELR` | low 32 bits of the counter / 计数器低 32 位 |
| `0x10..=0x1C` | `ALARM0..3` | 32-bit deadline, low half of the counter / 32 位截止值，计数器低半部 |
| `0x20` | `ARMED` | bit `n` set while `ALARMn` is armed / `ALARMn` 武装期间置位 |
| `0x24` | `TIMERAWH` | unlatched high half / 非锁存高半部 |
| `0x28` | `TIMERAWL` | unlatched low half / 非锁存低半部 |
| `0x30` | `DBGPAUSE` | storage / 存储 |
| `0x34` | `PAUSE` | storage / 存储 |
| `0x36` | `LOCKED` | 16-bit storage, kept apart from `PAUSE` / 16 位存储，与 `PAUSE` 分开保存 |
| `0x38` | `SOURCE` | storage / 存储 |
| `0x3C` | `INTR` | raw status, **write-one-to-clear** / 原始状态，**写 1 清零** |
| `0x40` | `INTE` | per-alarm enable / 逐 alarm 使能 |
| `0x44` | `INTF` | **plain read/write force bits** / **普通读写强制位** |
| `0x48` | `INTS` | masked status / 屏蔽后的状态 |

### Interrupt semantics / 中断语义

```text
INTS = (INTR | INTF) & INTE
```

* `INTR` reports latched alarm hits. Firmware acknowledges by writing a one to
  the bit; writing a zero leaves it unchanged. / `INTR` 报告锁存的 alarm 命中，
  固件向对应位写 1 确认，写 0 不变。
* `INTF` is a force register, not write-one-to-clear. pico-sdk's alarm pool
  raises an interrupt with `hw_set_bits(&timer->intf, 1u << n)` and drops it
  with `hw_clear_bits`; the model stores those bits verbatim, so a forced bit
  keeps `INTS` asserted even after `INTR` is acknowledged. / `INTF` 是强制位
  寄存器，不是写 1 清零。pico-sdk 的 alarm pool 用
  `hw_set_bits(&timer->intf, 1u << n)` 触发中断、用 `hw_clear_bits` 撤销；
  模型按普通读写保存这些位，因此即使 `INTR` 已被确认，强制位仍让 `INTS` 保持
  断言。
* `INTS` bit `n` drives the line `TIMERx_IRQ_n`; a disabled bit (`INTE` = 0)
  never asserts, even when forced. / `INTS` 第 `n` 位驱动 `TIMERx_IRQ_n`；
  未使能的位（`INTE` 为 0）即使被强制也不会断言。

### Alarm auto-disarm / alarm 自动解除武装

Writing a non-zero deadline arms alarm `n` (`ARMED.n = 1`); writing zero
disarms it. When the low 32 bits of the counter reach the deadline (signed
wrap-around comparison, the same rule pico-sdk uses), the model latches
`INTR.n = 1` and clears `ARMED.n`, so each arming delivers exactly one interrupt
until firmware acknowledges `INTR` and programs the next deadline. A disarmed
alarm never re-fires on its own.
写入非零截止值会武装 alarm `n`（`ARMED.n = 1`），写 0 则解除武装。当计数器低
32 位到达截止值（按有符号回绕比较，与 pico-sdk 相同），模型锁存 `INTR.n = 1`
并清掉 `ARMED.n`，因此每次武装只投递一次中断，直到固件确认 `INTR` 并写入下一个
截止值。已解除武装的 alarm 不会自行再次触发。

### Handles / 句柄

`BootPeripherals::mount` installs both blocks into the APB router and exposes
`timer()` / `timer1()`. Both handles share state with the bus devices;
`refresh()` samples the counters and republishes the eight alarm lines, and
`BootPeripherals::advance` / `irq_mask` do that on every pump so a `WFI`
waiting on a future deadline wakes on time.
`BootPeripherals::mount` 把两个块安装到 APB 路由器，并暴露 `timer()` /
`timer1()`。两个句柄与总线设备共享状态；`refresh()` 采样计数器并重新发布八条
alarm 线，`BootPeripherals::advance` / `irq_mask` 在每次泵送时执行该操作，使
等待未来截止值的 `WFI` 按时唤醒。

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

### PSRAM window (XIP CS1) / PSRAM 窗口（XIP CS1）

`psram::Psram` is the built-in model of the external PSRAM that RP2350 reaches
through QMI chip select 1. It is a bus-mounted `MemoryRegion`, not a QMI
register block, and is installed independently of `BootPeripherals::mount`:

```rust
use rp2350_core::memory::SimpleBus;
use rp2350_periph::psram::mount_psram;

let mut bus = SimpleBus::new();
let psram = mount_psram(&mut bus, 8 * 1024 * 1024).expect("psram window");
// `psram` shares the bytes that the bus serves at PSRAM_BASE.
```

`psram::Psram` 是内置的外部 PSRAM 模型，对应 RP2350 经 QMI 片选 1 访问的器件。
它是挂到总线上的 `MemoryRegion`（不是 QMI 寄存器块），挂载与
`BootPeripherals::mount` 相互独立：

- **Base and size / 基址与大小**: `PSRAM_BASE = 0x1100_0000`
  (`XIP_MAIN1`, not a mirror of the CS0 flash window at `0x1000_0000`) with
  `PSRAM_MAX_SIZE = 16 MB`. `Psram::zeroed()` builds the 8 MB window fitted by
  common PSRAM boards; `Psram::new(size)` takes any 4 KiB aligned size in
  `1..=PSRAM_MAX_SIZE`. Invalid sizes (zero, unaligned, oversized) clamp to the
  16 MB ceiling with a one-shot `log::warn!`, never to a silently truncated size.
- **基址与大小**: `PSRAM_BASE = 0x1100_0000`（`XIP_MAIN1`，不是
  `0x1000_0000` 处 CS0 Flash 窗口的镜像），`PSRAM_MAX_SIZE = 16 MB`。
  `Psram::zeroed()` 生成常见 PSRAM 板的 8 MB 窗口；`Psram::new(size)` 接受
  `1..=PSRAM_MAX_SIZE` 内任意 4 KiB 对齐大小。非法大小（0、未对齐、超上限）会
  钳制到 16 MB 上限并只 `log::warn!` 一次，绝不会静默截断。
- **QMI CS1 / QMI 片选 1**: `M1_TIMING`, `M1_RFMT`, `M1_RCMD` (and the other
  `M1_*` words) remain plain storage in `xip_qmi`; the PSRAM window does not
  interpret CS1 timing, latency or command encodings.
- **QMI 片选 1**: `M1_TIMING`、`M1_RFMT`、`M1_RCMD` 等 `M1_*` 字在 `xip_qmi`
  中仍是普通存储；PSRAM 窗口不解释 CS1 时序、延迟或命令编码。
- **Read/write semantics / 读写语义**: guest reads and writes are byte
  addressable and cost one bus cycle per byte (halfword two, word four, matching
  `RamRegion`). Out-of-window accesses return `BusErrorKind::DeviceError`.
  `Psram::read_only(size)` builds a window whose writes return
  `BusErrorKind::AccessDenied`. Clones share one backing store, so the handle
  returned by `mount_psram` observes bus traffic in both directions.
- **读写语义**: 客户机读写按字节可寻址，每字节一个总线周期（半字两个、字四个，
  与 `RamRegion` 一致）。窗口外访问返回 `BusErrorKind::DeviceError`。
  `Psram::read_only(size)` 构造的窗口写返回 `BusErrorKind::AccessDenied`。
  克隆共享同一份底层存储，因此 `mount_psram` 返回的句柄可双向观察总线流量。
- **Unmounted / 未挂载**: `0x1100_0000` stays unmapped unless `mount_psram` (or
  another region) is registered, so access reports `BusErrorKind::NoDevice`.
- **未挂载**: 除非调用 `mount_psram`（或注册其它区域），`0x1100_0000` 保持未映射，
  访问返回 `BusErrorKind::NoDevice`。
- **Not modelled / 未建模**: PSRAM timing (`M1_TIMING` latency/RXDELAY),
  command formats, CS1 address remapping (`M1_ADDR`), the non-cached PSRAM alias
  at `0x1500_0000`, the `0x1800_0000` maintenance window, and QMI arbitration
  with CS0 flash.
- **未建模**: PSRAM 时序（`M1_TIMING` 的延迟/RXDELAY）、命令格式、CS1 地址重映射
  （`M1_ADDR`）、`0x1500_0000` 处的非缓存 PSRAM 别名、`0x1800_0000` 维护窗口，
  以及与 CS0 Flash 之间的 QMI 仲裁。

---

## UART

`uart::Uart0` is the mounted PL011 model. Transmit bytes written to `UARTDR`
are appended to the shared `UartSink` with the simulated microsecond of the
write. `UARTIBRD`, `UARTFBRD` and `UARTLCR_H` are stored, and the line rate is
derived from the live `clk_peri`:

```text
baud = clk_peri * 4 / (64 * IBRD + FBRD)
bit  = 1 / baud
```

```rust
pub struct UartSink { /* bytes + last_byte_us */ }
impl UartSink {
    pub fn push(&mut self, byte: u8);          // no timestamp / 无时间戳
    pub fn push_at(&mut self, byte: u8, time_us: u64);
    pub fn last_byte_us(&self) -> Option<u64>;
    pub fn len(&self) -> usize;
    pub fn take(&mut self) -> Vec<u8>;
}

pub struct Uart0 { /* sink + timebase + clock tree + registers */ }
impl Uart0 {
    pub fn new(sink: Arc<Mutex<UartSink>>) -> Self;
    pub fn with_clock(sink, timebase, tree: ClockTree) -> Self;
    pub fn baud_divisor(&self) -> u64;          // 64*IBRD + FBRD
    pub fn baud_rate(&self) -> u32;             // 0 until programmed
    pub fn bit_time_ns(&self) -> Option<u64>;
    pub fn last_byte_us(&self) -> Option<u64>;
}
```

**Not modelled / 未建模**：byte-level shifting and the ten bit periods a real
UART spends on the wire, TX FIFO occupancy, receive data, framing/parity errors
and interrupts. A byte enters the sink immediately (with its timestamp); only
the baud rate and timestamp mapping follow the real peripheral clock, so host
logs and throughput estimates are meaningful while wire-level timing is not.
**不建模**：字节级移位、真实 UART 在线上花费的十个位周期、TX FIFO 占用、接收
数据、帧/校验错误与中断。字节立即进入 sink（带时间戳）；只有波特率与时间戳映射
跟随真实外设时钟，因此宿主日志与吞吐估计有意义，但位级时序不是。

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

### Pad electrical model / pad 电气模型

`gpio_pads::GpioPads` is the shared source of truth between `PADS_BANK0`
programming, `IO_BANK0` routing and the `SIO` input register. A pin reads, in
order of priority: the peripheral selected by `FUNCSEL` while its output enable
is set (SIO latch or PIO `PADOUT`/`PADOE`), else the host-driven external level
if the host drives that pin, else its programmed pull-up (`PUE`) or pull-down
(`PDE`). All pins reset to pull-down, so a firmware that calls `gpio_pull_up` on
a button input sees a high level and an unpressed button rather than a stuck-low
"all buttons held" reading.
`gpio_pads::GpioPads` 是 `PADS_BANK0` 配置、`IO_BANK0` 路由与 `SIO` 输入寄存器
之间共享的唯一事实来源。引脚电平按优先级解析：先是 `FUNCSEL` 选中且输出使能
有效的外设（SIO 锁存或 PIO 的 `PADOUT`/`PADOE`），其次是宿主驱动该引脚时的外部
电平，最后是配置的上拉（`PUE`）或下拉（`PDE`）。所有引脚复位为下拉，因此对按键
输入调用 `gpio_pull_up` 的固件读到高电平（未按下），而不是被拉低的「所有按键都
按下」。

```rust
pub struct GpioPads { /* external levels, driven mask, pulls, FUNCSEL, drivers */ }
impl GpioPads {
    pub fn new() -> Self;
    pub fn set_pad(&self, pin: u32, value: u32);          // PADS_BANK0 word
    pub fn set_external(&self, levels: u64);              // host drive, all pins
    pub fn set_external_pin(&self, pin: u32, high: bool);
    pub fn set_funcsel(&self, pin: u32, bits: u32);       // IO_BANK0 routing
    pub fn funcsel(&self, pin: u32) -> Option<u32>;
    pub fn funcsel_mask(&self, bits: u32) -> u64;
    pub fn set_driver(&self, selector: u32, levels: u64, enables: u64);
    pub fn sample_for(&self, selector: u32, base: u32) -> u32;
    pub fn input_levels(&self) -> u64;
    pub fn reset(&self);
}
```

`BootPeripherals::pads()` hands out the handle so a host can inject button
levels; `PADS_BANK0` writes feed `set_pad`, `IO_BANK0` writes feed `set_funcsel`,
`SIO`/`PIO` feed `set_driver`, and `SIO_GPIO_IN`/`GPIO_HI_IN` plus the PIO
`IN PINS` source read the resolved levels.
`BootPeripherals::pads()` 提供句柄供宿主注入按键电平；`PADS_BANK0` 写入调用
`set_pad`，`IO_BANK0` 写入调用 `set_funcsel`，`SIO`/`PIO` 调用 `set_driver`，
`SIO_GPIO_IN`/`GPIO_HI_IN` 与 PIO 的 `IN PINS` 源则读取解析后的电平。

---

## IO_BANK0

```rust
pub enum PinFunction {
    Hstx, Spi, Uart, I2c, Pwm, Sio, Pio0, Pio1, Pio2, Gpck, Usb, UartAux,
    Null, Unknown(u32),
}

impl PinFunction {
    pub const fn from_bits(bits: u32) -> Option<Self>;
    pub const fn to_bits(self) -> u32;
}

pub struct IoBank0 { /* register file + shared GpioPads */ }
impl IoBank0 {
    pub fn new() -> Self;
    pub fn with_pads(pads: GpioPads) -> Self;
    pub fn pads(&self) -> &GpioPads;                 // shared pin matrix / 共享引脚矩阵
    pub fn funcsel(&self, pin: u32) -> Option<PinFunction>;
    pub fn set_funcsel(&mut self, pin: u32, function: PinFunction);
    pub fn pins_for(&self, function: PinFunction) -> u64;  // pad bitmap / pad 位图
}
```

### FUNCSEL / 引脚功能选择

`IO_BANK0` sits at `map::IO_BANK0_BASE` (`0x4002_8000`). Each of the 48 pins
owns a `GPIOx_STATUS` word at `0x000 + 8*n` (read-only) and a `GPIOx_CTRL` word
at `0x004 + 8*n`. `CTRL.FUNCSEL` (bits `[4:0]`) selects the peripheral that
owns the pin and **resets to `0x1F` (`NULL`)** on every pin; the constructor
writes that reset value explicitly instead of relying on zeroed storage, and
`reset()` restores it while clearing `OUTOVER`/`INOVER`/`OEOVER`/`IRQOVER`.

`IO_BANK0` 位于 `map::IO_BANK0_BASE`（`0x4002_8000`）。48 个引脚各自拥有
`0x000 + 8*n` 的只读 `GPIOx_STATUS` 和 `0x004 + 8*n` 的 `GPIOx_CTRL`。
`CTRL.FUNCSEL`（bits `[4:0]`）选择拥有该引脚的外设，**每个引脚的复位值都是
`0x1F`（`NULL`）**；构造函数显式写入该复位值而不依赖零初始化，`reset()` 会恢复
它并清掉 `OUTOVER`/`INOVER`/`OEOVER`/`IRQOVER`。

| `FUNCSEL` | `PinFunction` | Peripheral / 外设 |
|-----------|---------------|-------------------|
| `0x00` | `Hstx` | HSTX |
| `0x01` | `Spi` | SPI |
| `0x02` | `Uart` | UART0 |
| `0x03` | `I2c` | I2C |
| `0x04` | `Pwm` | PWM |
| `0x05` | `Sio` | SIO |
| `0x06` | `Pio0` | PIO0 |
| `0x07` | `Pio1` | PIO1 |
| `0x08` | `Pio2` | PIO2 |
| `0x09` | `Gpck` | GPCK / XIP CS1 / trace |
| `0x0A` | `Usb` | USB |
| `0x0B` | `UartAux` | UART1 (`UART_AUX`) |
| `0x1F` | `Null` | none / 无 |
| `0x0C..=0x1E` | `Unknown(bits)` | undocumented / 未文档化 |

`PinFunction::from_bits` returns `None` only for values that do not fit the
5-bit field (above `0x1F`); undocumented 5-bit values stay observable as
`Unknown(bits)` so a host can distinguish "pin does not exist" from "firmware
programmed an undocumented function". `IoBank0::funcsel` returns `None` for
`pin >= 48`; `IoBank0::set_funcsel` ignores such pins and preserves every
`CTRL` bit outside `FUNCSEL`. Every write to a `GPIOx_CTRL` register — through
the accessor or the bus — also publishes the field into the shared `GpioPads`
matrix, and `reset()` puts all 48 pads back to `NULL`.
`PinFunction::from_bits` 仅在数值超出 5 位字段（大于 `0x1F`）时返回 `None`；
未文档化的 5 位值以 `Unknown(bits)` 保留可观测性，使宿主能区分「引脚不存在」与
「固件写入了未文档化功能」。`IoBank0::funcsel` 对 `pin >= 48` 返回 `None`；
`IoBank0::set_funcsel` 忽略这类引脚，并保留 `CTRL` 中 `FUNCSEL` 之外的每一位。
每次写 `GPIOx_CTRL`（经访问器或总线）都会把该字段发布到共享 `GpioPads` 矩阵，
`reset()` 把全部 48 个 pad 恢复为 `NULL`。

### STATUS / 状态寄存器

`GPIOx_STATUS` is read-only in hardware: writes are acknowledged (`write32`
returns `true`) but change nothing. `read32` overlays the live pad level onto
`INFROMPAD` (bit 17), taken from the shared `gpio_pads::GpioPads` handle that
`PADS_BANK0` programs and `SIO`/`PIO` drive. `OETOPAD` (bit 13) and `OUTTOPAD`
(bit 9) stay `0` because the pad-output status bits are not modelled.
`GPIOx_STATUS` 在硬件上是只读的：写入会被确认（`write32` 返回 `true`）但不改变
任何位。`read32` 会把实时 pad 电平叠加到 `INFROMPAD`（第 17 位），电平取自
`PADS_BANK0` 配置、`SIO`/`PIO` 驱动的共享 `gpio_pads::GpioPads` 句柄。
`OETOPAD`（第 13 位）与 `OUTTOPAD`（第 9 位）仍为 `0`，因为 pad 输出状态位未建模。

### Wiring and routing / 接线与路由

`BootPeripherals::mount` builds one `GpioPads` handle and passes it to
`IoBank0::with_pads`, `PadsBank0::with_pads`, `Sio` and all three
`SharedPio::with_pads` blocks. That handle is the pin matrix: `IO_BANK0`
publishes every `CTRL.FUNCSEL` into it (`IoBank0::pads()` /
`IoBank0::pins_for`), `PADS_BANK0` programs the pulls, `SIO` publishes
`GPIO_OE`/`GPIO_OUT`, each PIO block publishes its output levels/enables, and
every reader resolves one level per pad.
`BootPeripherals::mount` 只创建一个 `GpioPads` 句柄，并交给
`IoBank0::with_pads`、`PadsBank0::with_pads`、`Sio` 与三个
`SharedPio::with_pads` 块。该句柄就是引脚矩阵：`IO_BANK0` 把每个
`CTRL.FUNCSEL` 发布进去（`IoBank0::pads()` / `IoBank0::pins_for`），
`PADS_BANK0` 配置上下拉，`SIO` 发布 `GPIO_OE`/`GPIO_OUT`，每个 PIO 块发布自己
的输出电平与使能，所有读者为每个 pad 解析出唯一电平。

Routing rules, highest priority first / 路由规则（优先级从高到低）：

1. **`FUNCSEL == SIO` (5)** and the matching `GPIO_OE` bit set → the pad is driven
   by the SIO `GPIO_OUT` latch (`GPIO_HI_*` for pins 32..47). `SIO_GPIO_IN` reports
   the resolved pad level only on these bits.
2. **`FUNCSEL == PIO0/1/2` (6/7/8)** → the pad is driven by that block's
   `output_levels`/`output_enables`, i.e. whatever `OUT`/`SET`/side-set (including
   `SIDE_PINDIR`) wrote, mapped through `PINCTRL` `OUT_BASE`/`SET_BASE`/
   `SIDESET_BASE` + counts and `GPIOBASE`. The same pins feed `IN PINS`, `WAIT`
   and `JMP PIN`; unrouted pins read 0.
3. **Other functions** (HSTX/SPI/UART/I2C/PWM/USB/GPCK/UART1) have **no modelled
   pad driver**, so the pad keeps its pull or host-driven external level.
4. **`NULL`** (the reset value) — nothing drives; pads float to pull/external.

1. **`FUNCSEL == SIO`（5）** 且对应 `GPIO_OE` 位置位 → pad 由 SIO 的 `GPIO_OUT`
   锁存驱动（引脚 32..47 用 `GPIO_HI_*`）。`SIO_GPIO_IN` 只在这些位上报告解析后
   的 pad 电平。
2. **`FUNCSEL == PIO0/1/2`（6/7/8）** → pad 由该块的
   `output_levels`/`output_enables` 驱动，即 `OUT`/`SET`/side-set（含
   `SIDE_PINDIR`）写入的值，经 `PINCTRL` 的 `OUT_BASE`/`SET_BASE`/
   `SIDESET_BASE` 与计数、以及 `GPIOBASE` 映射。同一批引脚供 `IN PINS`、`WAIT`
   与 `JMP PIN` 采样；未路由的引脚读 0。
3. **其它功能**（HSTX/SPI/UART/I2C/PWM/USB/GPCK/UART1）**没有建模 pad 驱动器**，
   pad 保持上下拉或宿主驱动的外部电平。
4. **`NULL`**（复位值）——无人驱动，pad 浮到上下拉/外部电平。

Only one `FUNCSEL` wins per pad: the matrix always reads the **current** field, so
a pad handed from PIO to SIO stops following the PIO on that write. Real silicon
would see two drivers fighting; firmware must keep one function per pin at a
time. Write arbitration between peripherals is therefore not modelled.
每个 pad 只有一个 `FUNCSEL` 生效：矩阵始终读取**当前**字段，因此把 pad 从 PIO
切到 SIO 的那次写入之后它就不再跟随 PIO。真实芯片上会出现两个驱动器相争；固件
必须保证同一时刻每个引脚只有一个功能。因此外设间的写仲裁未建模。

`STATUS.OETOPAD` (bit 13) and `OUTTOPAD` (bit 9) still read 0, and IO_BANK0
interrupt routing (`PROC0_*`/`PROC1_*`) is plain storage.
`STATUS.OETOPAD`（第 13 位）与 `OUTTOPAD`（第 9 位）仍读 0，IO_BANK0 的中断路由
（`PROC0_*`/`PROC1_*`）仍仅为普通存储。

---

## CLOCKS / clock tree / 时钟树

`clocks::Clocks`, `pll::Pll`, `xosc::Xosc` and `rosc::Rosc` share one
`clock_tree::ClockTree` (`BootPeripherals::clock_tree`). The tree decodes
register fields into frequencies; the MMIO blocks only store the raw register
image. Any accepted write to `PLL_CS`/`PLL_PWR`/`PLL_FBDIV_INT`/`PLL_PRIM`,
`XOSC_CTRL`, `ROSC_CTRL`/`ROSC_DIV`, or `CLK_*_CTRL`/`CLK_*_DIV` takes effect
immediately — runtime overclocking/underclocking needs no extra call.
`clocks::Clocks`、`pll::Pll`、`xosc::Xosc` 与 `rosc::Rosc` 共享同一棵
`clock_tree::ClockTree`（`BootPeripherals::clock_tree`）。频率换算由时钟树完成，
MMIO 块只保存原始寄存器镜像。对 `PLL_CS`/`PLL_PWR`/`PLL_FBDIV_INT`/`PLL_PRIM`、
`XOSC_CTRL`、`ROSC_CTRL`/`ROSC_DIV`、`CLK_*_CTRL`/`CLK_*_DIV` 的写入立即生效，
运行时超频/降频无需额外调用。

### Register map / 寄存器映射

| Block | Base | Registers modelled |
|-------|------|--------------------|
| `CLOCKS` | `0x4001_0000` | `CLK_GPOUTn/REF/SYS/PERI/HSTX/USB/ADC_CTRL/DIV/SELECTED`; other registers are storage |
| `XOSC` | `0x4004_8000` | `CTRL` (ENABLE `0xFAB`/`0xD1E`, `FREQ_RANGE 0xAA0..0xAA3`), `STATUS` (`STABLE`/`ENABLED`/`BADWRITE`/range) |
| `PLL_SYS` | `0x4005_0000` | `CS` (REFDIV/BYPASS/LOCK), `PWR`, `FBDIV_INT`, `PRIM` |
| `PLL_USB` | `0x4005_8000` | same / 同上 |
| `ROSC` | `0x400E_8000` | `CTRL` (ENABLE/FREQ_RANGE), `DIV` (`0xAA00 + div`), `STATUS`, `RANDOMBIT` |

`CLK_*_SELECTED` is derived live: `CLK_REF`/`CLK_SYS` report `1 << SRC`, every
other slice reports `0x1` because it has no glitchless mux (per
`hardware/regs/clocks.h`). Switching is instantaneous in the model; the
datasheet's "may briefly show all-0s" window is not modelled.
`CLK_*_SELECTED` 实时推导：`CLK_REF`/`CLK_SYS` 报告 `1 << SRC`，其余通道按
`hardware/regs/clocks.h` 报告 `0x1`（没有无毛刺选择器）。模型切换瞬时完成，不建模
数据手册所述「切换期间可能短暂全 0」。

### Formulas / 公式

```
vco      = (xosc_hz / refdiv) * fbdiv          # PLL_CS.REFDIV, PLL_FBDIV_INT
pll_out  = vco / postdiv1 / postdiv2           # PLL_PRIM.POSTDIV1/2
clk      = source / (div_int + div_frac/65536) # CLK_*_CTRL.SRC/AUXSRC + CLK_*_DIV
```

`DIV.INT = 0` means `max + 1` of the field width: 65536 for `CLK_SYS`/
`CLK_GPOUTn`, 256 for `CLK_REF`, 16 for `CLK_USB`/`CLK_ADC`, 4 for
`CLK_PERI`/`CLK_HSTX`. Only `CLK_SYS` and `CLK_GPOUTn` have `DIV.FRAC`.
`DIV.INT = 0` 表示字段宽度的 `max + 1`：`CLK_SYS`/`CLK_GPOUTn` 为 65536，
`CLK_REF` 为 256，`CLK_USB`/`CLK_ADC` 为 16，`CLK_PERI`/`CLK_HSTX` 为 4。
只有 `CLK_SYS` 与 `CLK_GPOUTn` 带 `DIV.FRAC`。

Source tables follow the header exactly: `CLK_SYS` aux `0 = PLL_SYS`,
`1 = PLL_USB`, `2 = ROSC`, `3 = XOSC`; `CLK_PERI` aux `0 = CLK_SYS`,
`1 = PLL_SYS`, `2 = PLL_USB`, `3 = ROSC`, `4 = XOSC`; `CLK_USB`/`CLK_ADC` aux
`0 = PLL_USB`, `1 = PLL_SYS`, `2 = ROSC`, `3 = XOSC`. GPIN0/1 and
`OTP_CLK2FC` are not driven and read 0 Hz.
源表与头文件完全一致：`CLK_SYS` 辅助源 `0 = PLL_SYS`、`1 = PLL_USB`、
`2 = ROSC`、`3 = XOSC`；`CLK_PERI` 为 `0 = CLK_SYS`、`1 = PLL_SYS`、
`2 = PLL_USB`、`3 = ROSC`、`4 = XOSC`；`CLK_USB`/`CLK_ADC` 为
`0 = PLL_USB`、`1 = PLL_SYS`、`2 = ROSC`、`3 = XOSC`。GPIN0/1 与
`OTP_CLK2FC` 未接线，读为 0 Hz。

### Post-bootrom initial state / bootrom 之后的初始状态

`BootPeripherals::mount` (and `Clocks::new`) start from the **bootrom-equivalent**
state, not the power-on reset values:
`BootPeripherals::mount`（以及 `Clocks::new`）从 **bootrom 等价**状态启动，而不是
上电复位值：

| Item | Value |
|------|-------|
| XOSC | 12 MHz, enabled, `STABLE` |
| `PLL_SYS` | REFDIV 1, FBDIV 125, POSTDIV 5×2 → 150 MHz |
| `PLL_USB` | REFDIV 1, FBDIV 100, POSTDIV 5×5 → 48 MHz |
| `CLK_REF` | XOSC, DIV 1 → 12 MHz |
| `CLK_SYS` | AUX/PLL_SYS, DIV 1 → 150 MHz |
| `CLK_PERI`/`CLK_HSTX` | `CLK_SYS`, DIV 1 → 150 MHz |
| `CLK_USB`/`CLK_ADC` | `PLL_USB`, DIV 1 → 48 MHz |

`Clocks::reset()`, `Pll::reset()`, `Xosc::reset()` and `Rosc::reset()` restore
this state. Consequently a `CLOCKS` reset readback shows `CLK_REF_CTRL = 2`,
`CLK_SYS_CTRL = 0x01`, `PLL_SYS.FBDIV_INT = 125`, not the datasheet's power-on
values.
`Clocks::reset()`、`Pll::reset()`、`Xosc::reset()` 与 `Rosc::reset()` 都恢复该
状态；因此 `CLOCKS` 复位后读回 `CLK_REF_CTRL = 2`、`CLK_SYS_CTRL = 0x01`、
`PLL_SYS.FBDIV_INT = 125`，而非数据手册的上电值。

### Overclocking example (252 MHz) / 超频示例（252 MHz）

The emerald firmware runs at 252 MHz. `set_sys_clock_khz(252000, true)` searches
`fbdiv` downwards from 320 and accepts the first in-range VCO that divides
exactly, which is FBDIV 126 → VCO 1512 MHz and POSTDIV1 6 / POSTDIV2 1:
emerald 固件运行在 252 MHz。`set_sys_clock_khz(252000, true)` 从 320 向下搜索
`fbdiv`，取第一个落在 VCO 范围内且能整除的配置：FBDIV 126 → VCO 1512 MHz，
POSTDIV1 6 / POSTDIV2 1：

```text
PLL_SYS.FBDIV_INT = 126
PLL_SYS.PRIM      = (6 << 16) | (1 << 12)
=> PLL_SYS = 12 MHz * 126 / (6 * 1) = 252 MHz
=> CLK_SYS (AUX/PLL_SYS, DIV 1)   = 252 MHz
```

Measured on the emerald firmware at 300 M steps: `CLK_SYS_SELECTED = 0x2`,
`PLL_SYS.FBDIV_INT = 126`, `PLL_SYS.PRIM = 0x0006_1000`,
`sysclk_hz() = 252000000`, matching the guest's own `clk_sys 252000000 Hz`
banner. The firmware later moves `CLK_PERI` to `PLL_USB` (48 MHz) and `CLK_HSTX`
to `CLK_SYS / 2` (126 MHz), both visible through the same tree.
在 emerald 固件 300 M 步时实测：`CLK_SYS_SELECTED = 0x2`、
`PLL_SYS.FBDIV_INT = 126`、`PLL_SYS.PRIM = 0x0006_1000`、
`sysclk_hz() = 252000000`，与客户机自己打印的 `clk_sys 252000000 Hz` 一致。
固件随后把 `CLK_PERI` 切到 `PLL_USB`（48 MHz）、`CLK_HSTX` 切到
`CLK_SYS / 2`（126 MHz），同样可通过时钟树观察。

```rust
let tree = peripherals.clock_tree();
// Guest writes go through the bus; the host API is live as well.
assert_eq!(peripherals.sysclk_hz(), 252_000_000);
```

### Invalid configurations / 非法配置

`REFDIV = 0`, `POSTDIV1/2 = 0` and `FBDIV_INT` outside `16..=320` make the PLL
output read 0 Hz and emit one `log::warn!` per class; a disabled XOSC makes
every PLL read 0 Hz. Undefined `AUXSRC` codes (GPIN, `OTP_CLK2FC`, reserved
values) read 0 Hz. No configuration panics.
`REFDIV = 0`、`POSTDIV1/2 = 0`、`FBDIV_INT` 超出 `16..=320` 时 PLL 输出读
0 Hz，每类错误只输出一次 `log::warn!`；XOSC 关闭时所有 PLL 读 0 Hz。未定义的
`AUXSRC`（GPIN、`OTP_CLK2FC`、保留值）读 0 Hz。任何配置都不会 panic。

### Generation counter / 世代计数器

`ClockTree::generation()` increments on every accepted frequency-affecting
write (`CLK_*_CTRL`/`CLK_*_DIV`, PLL `CS`/`PWR`/`FBDIV_INT`/`PRIM`,
`XOSC_CTRL`, `ROSC_CTRL`/`ROSC_DIV`, host crystal/ring changes). Read-only
`CLK_*_SELECTED`, PLL interrupt registers, `XOSC_STARTUP` and other unrelated
writes leave it unchanged. P4-2 uses it to invalidate derived timing caches.
`ClockTree::generation()` 在每次被接受的影响频率的写入后递增
（`CLK_*_CTRL`/`CLK_*_DIV`、PLL `CS`/`PWR`/`FBDIV_INT`/`PRIM`、`XOSC_CTRL`、
`ROSC_CTRL`/`ROSC_DIV`、宿主的晶振/环形频率设置）。只读的 `CLK_*_SELECTED`、
PLL 中断寄存器、`XOSC_STARTUP` 等无关写入不改变它。P4-2 用它失效派生的时序缓存。

### Host API / 宿主接口

| API | Meaning |
|-----|---------|
| `ClockTree::sysclk_hz/refclk_hz/periclk_hz/hstxclk_hz/usbclk_hz/adcclk_hz` | live channel frequencies / 实时通道频率 |
| `ClockTree::rtcclk_hz` | RTC source (`LPOSC`, 32.768 kHz nominal) / RTC 源 |
| `ClockTree::pll_sys_hz/pll_usb_hz`, `pll_locked` | PLL outputs and lock / PLL 输出与锁定 |
| `ClockTree::roscclk_hz`, `channel_hz(n)` | ROSC output and any channel / ROSC 输出与任意通道 |
| `BootPeripherals::clock_tree`, `sysclk_hz` | mounted tree and `CLK_SYS` / 已挂载的树与 `CLK_SYS` |
| `BootPeripherals::refclk_hz/periclk_hz/hstxclk_hz/usbclk_hz/adcclk_hz/rtcclk_hz` | live frequencies for consumers / 消费者使用的实时频率 |
| `Xosc::set_frequency_hz`, `ClockTree::set_rosc_hz` | board-specific crystal/ring / 板级晶振/环形频率 |

### Unified time base and consumers (P4-2) / 统一时间基与消费者（P4-2）

`rp2350-core::TimeBase` integrates executed `clk_sys` cycles into microseconds.
It keeps `cycles`, settled `micros` and a sub-microsecond numerator, and it
**settles the carried fraction at the old rate before a retune**, so a guest
write that changes `CLK_SYS` cannot lose or double-count cycles. A zero
frequency stops time (cycles are still counted) and warns once.
`rp2350-core::TimeBase` 把已执行的 `clk_sys` 周期积分为微秒。它保存 `cycles`、
已结算的 `micros` 与不足一微秒的分子，并在**改频前按旧频率结算携带的小数**，
因此客户机改写 `CLK_SYS` 不会丢失或重复计算周期。零频率会停止时间（仍统计周期）
并只告警一次。

`rp2350-periph::Timebase` composes two integrators: the CPU domain (uptime,
`BootPeripherals::time_us`) and the `clk_ref` domain (`TIMELR`/`TIMERAWH`, fed by
executed cycles converted with the live `clk_sys`/`clk_ref` ratio). All
consumers in the mounted machine share it:

| Consumer | Clock / 时钟 | Behaviour / 行为 |
|----------|--------------|------------------|
| `cycles → µs` | `clk_sys` | `BootPeripherals::time_us`, `Machine::time_us` / 运行时间 |
| TIMER0/1 | `clk_ref / TICK` | 1 µs counter, same µs at 150/252 MHz / 1 µs 计数 |
| `SYST_CSR.CLKSOURCE=1` | `clk_sys` | countdown in processor cycles / 处理器周期 |
| `SYST_CSR.CLKSOURCE=0` | `clk_ref / 2` | countdown in reference cycles / 参考周期 |
| PIO state machines | `clk_sys / CLKDIV` | integer+fractional 16.8 divider, write restarts it / 16.8 分频，写即重启 |
| HSTX pixels | `CLK_HSTX / CSR.CLKDIV` | `ceil(CLKDIV·clk_sys/clk_hstx)` cycles/pixel / 每像素周期 |
| UART0 baud | `clk_peri` | `baud = clk_peri·4/(64·IBRD+FBRD)` |
| ADC conversion | `clk_adc` | 96 `clk_adc` cycles → `ceil(96·clk_sys/clk_adc)` CPU cycles |
| POWMAN RTC | `clk_rtc` (LPOSC) | 32.768 kHz ticks drive milliseconds / 毫秒计数 |
| DMA pacing | `clk_sys` | `(X/Y)·sys_clk`, credited with real elapsed cycles / 真实经过周期 |

`ClockTree::generation()` is passed to `Timebase::advance`, so a frequency write
settles at the old rate on the next batch. `SimulationClock` remains only as a
legacy cumulative-cycle tracker.
`ClockTree::generation()` 会传入 `Timebase::advance`，因此频率写入在下一批次先按
旧频率结算。`SimulationClock` 仅作为旧的累计周期追踪器保留。

SysTick exception delivery / SysTick 异常投递：the peripheral counter owns timing,
`COUNTFLAG`, reload and the one-shot expiry latch
(`BootPeripherals::take_systick_pending`), including the live source selection
(`CLKSOURCE=1` → `clk_sys`, `CLKSOURCE=0` → `clk_ref/2`). The core models the
architectural target: `rp2350-arm` keeps SysTick's pending bit in `SCB_ICSR`
(exception 15, vector `VTOR + 0x3C`) instead of NVIC bit 15 (exception 31), and
exposes `CortexM33Core::pend_systick()` for a driver. `rp2350-cli` now closes the
loop: every IRQ pump takes the latch at most once and forwards it to
`pend_systick()`, logging `[irq] core0 systick at PC=…`; SysTick registers are
never replayed into the core, so the peripheral counter is the single counting
source. PPB writes to `ICSR`, `SHPR2` and `SHPR3` are still replayed into the
core, so firmware-side PendSV pending requests and system-exception priorities
take effect.
外设计数器负责计时、`COUNTFLAG`、重载与一次性到期锁存
（`BootPeripherals::take_systick_pending`），并跟随实时源选择
（`CLKSOURCE=1` → `clk_sys`，`CLKSOURCE=0` → `clk_ref/2`）。核侧正确建模架构
目标：`rp2350-arm` 把 SysTick 挂起位放在 `SCB_ICSR`（异常 15，向量
`VTOR + 0x3C`），而不是 NVIC 位 15（异常 31），并提供
`CortexM33Core::pend_systick()` 供驱动使用。`rp2350-cli` 现已闭环：每次中断泵送
最多取走一次锁存并转发给 `pend_systick()`，同时记录
`[irq] core0 systick at PC=…`；SysTick 寄存器从不回放到核，因此外设计数器是唯一
计数源。PPB 对 `ICSR`、`SHPR2`、`SHPR3` 的写入仍会回放到核，因此固件侧的
PendSV 挂起请求与系统异常优先级立即生效。

### Not modelled / 未建模

XOSC/ROSC startup delay and counters, ROSC drive-strength recalibration
(`FREQA/FREQB` do not rescale the nominal ring), glitchless-mux transition
windows, `CLOCKS.FC0` frequency counter, `CLK_SYS_RESUS`, GPIN0/1 and DFTCLK
muxing.
XOSC/ROSC 启动延时与计数器、ROSC 驱动强度重标定（`FREQA/FREQB` 不改变标称环形
频率）、无毛刺选择器切换窗口、`CLOCKS.FC0` 频率计数器、`CLK_SYS_RESUS`、
GPIN0/1 与 DFTCLK 选择。

---

## BUSCTRL

`busctrl::BusCtrl` models the `BUSCTRL` block at `map::BUSCTRL_BASE`
(`0x4006_8000`): the `BUS_PRIORITY` master priority hints, the `BUS_PRIORITY_ACK`
handshake, `PERFCTR_EN`, and the two bus-fabric performance counters
`PERFCTR0/1` with their `PERFSEL0/1` selectors. The standard SET/CLR/XOR aliases
fold onto the 4 KB block. Firmware writes it during display bring-up
(`bus_ctrl_hw->priority`).
`busctrl::BusCtrl` 建模 `map::BUSCTRL_BASE`（`0x4006_8000`）处的 `BUSCTRL` 块：
`BUS_PRIORITY` 主设备优先级提示、`BUS_PRIORITY_ACK` 握手、`PERFCTR_EN`，以及
两个总线矩阵性能计数器 `PERFCTR0/1` 与其 `PERFSEL0/1` 选择器。标准
SET/CLR/XOR 别名折叠到 4 KB 块上。固件在显示初始化时写入它
（`bus_ctrl_hw->priority`）。

```rust
pub struct BusCtrl { /* register file + BusPerf bank */ }
pub struct BusPerf { /* cloneable Arc<Mutex<counters>> */ }

pub enum BusPort { SiobProc1, SiobProc0, Apb, FastPeri, Sram(u8), XipMain1, XipMain0, Rom }

impl BusCtrl {
    pub fn new() -> Self;                     // private counter bank / 私有计数器组
    pub fn with_perf(perf: BusPerf) -> Self;  // shared with Dma / 与 Dma 共享
    pub fn perf(&self) -> BusPerf;
    pub fn priority(&self) -> u32;
    pub fn dma_bus_cycles(&self) -> u64;
    pub fn dma_grants(&self) -> u64;
    pub fn dma_occupancy(&self, total_cycles: u64) -> u32; // per mille / 千分比
    pub fn perf_snapshot(&self) -> BusPerfSnapshot;
}

impl BusPerf {
    pub fn record_read(&self, address: u32, cycles: u64);
    pub fn record_write(&self, address: u32, cycles: u64);
    pub fn record_grant(&self);
    pub fn snapshot(&self) -> BusPerfSnapshot;
    pub fn occupancy(&self, total_cycles: u64) -> u32;
}
```

### Register map / 寄存器映射

| Offset | Register | Model / 模型 |
|--------|----------|--------------|
| `0x00` | `BUS_PRIORITY` | `PROC0[0]`, `PROC1[4]`, `DMA_R[8]`, `DMA_W[12]`, mask `0x1111` / 四个优先级提示位，掩码 `0x1111` |
| `0x04` | `BUS_PRIORITY_ACK` | reads 1 (the model latches immediately) / 读 1（模型立即锁存） |
| `0x08` | `PERFCTR_EN` | bit 0 gates both counters / bit0 门控两个计数器 |
| `0x0C` | `PERFCTR0` | 24-bit saturating counter, write any value to clear / 24 位饱和计数器，写任意值清零 |
| `0x10` | `PERFSEL0` | 7-bit event select, reset `0x1F` (`sram6_access`) / 7 位事件选择，复位 `0x1F` |
| `0x14` | `PERFCTR1` | second counter / 第二个计数器 |
| `0x18` | `PERFSEL1` | second selector / 第二个选择器 |

### Performance events / 性能事件

`PERFSELn` selects one of `0x00..=0x43` event codes: code `n` means port `n / 4`
and kind `n % 4`, with kinds `STALL_UPSTREAM`, `STALL_DOWNSTREAM`,
`ACCESS_CONTESTED` and `ACCESS`. The port order is the datasheet order
(`siob_proc1`, `siob_proc0`, `apb`, `fastperi`, `sram9..=sram0`, `xip_main1`,
`xip_main0`, `rom`); `busctrl::BusPort::event_code` builds the code and
`busctrl::port_of_address` attributes an address.
`PERFSELn` 从 `0x00..=0x43` 的事件码中选择一个：码 `n` 表示端口 `n / 4`、种类
`n % 4`，种类依次为 `STALL_UPSTREAM`、`STALL_DOWNSTREAM`、`ACCESS_CONTESTED`、
`ACCESS`。端口顺序即数据手册顺序（`siob_proc1`、`siob_proc0`、`apb`、
`fastperi`、`sram9..=sram0`、`xip_main1`、`xip_main0`、`rom`）；
`busctrl::BusPort::event_code` 生成事件码，`busctrl::port_of_address` 做地址归属。

Implemented events (fed by the DMA master through `Dma::with_perf`):
已实现事件（由 `Dma::with_perf` 接线的 DMA 主设备提供）：

- `ACCESS` — one completed DMA read or write attributed to the port by address
  (SRAM banks are decoded from the 520 KiB layout, XIP `0x1000_0000`/`0x1100_0000`
  to main 0/1, `0x4000_0000` to APB, `0x5000_0000` to the fast peripherals).
  `ACCESS` — 每次完成的 DMA 读/写按地址归属到端口（SRAM 存储体按 520 KiB 布局
  解码，XIP 的 `0x1000_0000`/`0x1100_0000` 归属 main 0/1，`0x4000_0000` 归属
  APB，`0x5000_0000` 归属快速外设）。
- `STALL_UPSTREAM` — the bus cycles that access consumed (the `cycles`
  out-parameter), a proxy for how long the master held the fabric.
  `STALL_UPSTREAM` — 该访问消耗的总线周期（`cycles` 出参），作为主设备占用总线
  矩阵时长的代理指标。

Not implemented: `STALL_DOWNSTREAM` and `ACCESS_CONTESTED` stay 0 because both
need a cycle-exact multi-master arbiter, and `siob_proc0/1` events stay 0 because
CPU accesses do not pass through the peripheral models. Counters saturate at
`0x00FF_FFFF`; only a write to `PERFCTR` clears them (changing `PERFSEL` keeps
the accumulated value, exactly as the datasheet specifies — it documents no
other clear).
未实现：`STALL_DOWNSTREAM` 与 `ACCESS_CONTESTED` 保持 0，因为二者需要周期精确的
多主设备仲裁器；`siob_proc0/1` 事件也保持 0，因为 CPU 访问不经过外设模型。
计数器在 `0x00FF_FFFF` 饱和；只有写 `PERFCTR` 才清零（改变 `PERFSEL` 会保留
已累积值，与数据手册一致——手册没有规定其它清零方式）。

### Accounting and occupancy / 记账与占用率

`BusPerf` also keeps monotonic host-side totals that are not registers:
`dma_read_units`, `dma_write_units`, `dma_cycles`, `grants` and per-port
`port_accesses`/`port_cycles`. They survive `BusCtrl::reset`. `Dma::bus_counters`
reports the same master totals even when no counter bank is wired, and
`BusCtrl::dma_occupancy(total_cycles)` returns the DMA share of a caller-supplied
cycle window in parts per thousand (saturating at 1000), which is how
DMA-versus-CPU contention becomes observable.
`BusPerf` 还保存不映射寄存器的单调宿主侧总计：`dma_read_units`、
`dma_write_units`、`dma_cycles`、`grants` 以及每端口 `port_accesses`/
`port_cycles`。它们会在 `BusCtrl::reset` 后保留。`Dma::bus_counters` 即使未接线
计数器组也会报告同样的主设备总计；`BusCtrl::dma_occupancy(total_cycles)` 返回
DMA 在调用方给出的周期窗口中的千分比占用（在 1000 饱和），从而让 DMA 与 CPU 的
争用可观测。

### Not modelled / 未建模项

Only DMA-master events are countable; CPU masters, downstream stalls and
contested accesses require a cycle-exact shared bus model. `BUS_PRIORITY` is
stored (and reported acknowledged immediately) but it does not change arbitration
because the model has a single DMA master and no preemption.
只有 DMA 主设备事件可统计；CPU 主设备、下游 stall 与争用访问需要周期精确的共享
总线模型。`BUS_PRIORITY` 会被保存（并立即报告已确认），但不改变仲裁，因为模型只有
一个 DMA 主设备且没有抢占。

---

## SIO

```rust
pub struct Sio { /* shared state, core-0 IRQ mask, core-1 IRQ mask, view */ }
impl Sio {
    pub fn new(irq: IrqAssert) -> Self;
    pub fn with_peer_irq(irq: IrqAssert, core1_irq: IrqAssert) -> Self;
    pub fn select_core(&self, core: CpuId);
    pub fn selected_core(&self) -> CpuId;
    pub fn post_to_core(&self, value: u32);          // core-1 → core-0 frame
    pub fn ring_core(&self, mask: u8);               // core-1 → core-0 doorbell
    pub fn take_from_core(&self) -> Vec<u32>;        // drain core-0 → core-1 frames
    pub fn pop_from_core(&self) -> Option<u32>;      // one core-0 → core-1 frame
    pub fn peer_bells(&self) -> u8;
    pub fn core1_irq_mask(&self) -> u64;
    pub fn spinlock_state(&self) -> u32;             // held-lock bitmap / 已占用锁位图
    pub fn refresh_irq(&self);
    // FIFO blocking reads and sleep wake-up sources / FIFO 阻塞读与睡眠唤醒源
    pub fn fifo_rx_ready(&self, core: CpuId) -> bool;
    pub fn fifo_read_blocking(&self, core: CpuId) -> Option<u32>;
    pub fn fifo_status(&self, core: CpuId) -> u32;
    pub fn fifo_irq_asserted(&self, core: CpuId) -> bool;
    pub fn event_generation(&self) -> u32;
}
```

Modelled registers: `CPUID`, `GPIO_IN/OUT/OUT_SET/OUT_CLR/OUT_XOR`, the `GPIO_HI_*`
equivalents, `GPIO_OE` and its aliases, `FIFO_ST`, `FIFO_WR`, `FIFO_RD`,
`SPINLOCK_ST`, the 32 `SPINLOCK0..31` registers at `0x100 + 4*n`, and
`DOORBELL_OUT/IN_SET/CLR` at `0x180`.
已建模寄存器：`CPUID`、`GPIO_IN/OUT/OUT_SET/OUT_CLR/OUT_XOR`、对应的
`GPIO_HI_*`、`GPIO_OE` 及其别名、`FIFO_ST`、`FIFO_WR`、`FIFO_RD`、
`SPINLOCK_ST`、位于 `0x100 + 4*n` 的 32 个 `SPINLOCK0..31`，以及 `0x180`
的 `DOORBELL_OUT/IN_SET/CLR`。

### Pad routing / pad 路由

The SIO output latch and enables are published into the shared pin matrix on
every `GPIO_OUT`/`GPIO_OE` (and `GPIO_HI_*`) write, but the matrix applies them
only to pads whose `IO_BANK0.FUNCSEL` selects `SIO` (5). `GPIO_IN` mirrors that
gate: bits routed to `SIO` report the resolved pad level (latch, host level or
pull), while every other bit keeps the SIO latch loopback (`out & oe`) and reads
0 while the driver is off. `GPIO_HI_IN` does the same for pins 32..47.
SIO 的输出锁存与使能在每次写 `GPIO_OUT`/`GPIO_OE`（以及 `GPIO_HI_*`）时发布到
共享引脚矩阵，但矩阵只把它们应用到 `IO_BANK0.FUNCSEL` 选中 `SIO`（5）的 pad。
`GPIO_IN` 遵循同一门控：路由到 `SIO` 的位报告解析后的 pad 电平（锁存、宿主电平
或上下拉），其它位保持 SIO 锁存回环（`out & oe`），驱动器关闭时读 0。
`GPIO_HI_IN` 对引脚 32..47 同理。

### Hardware spinlocks / 硬件自旋锁

RP2350 moves the spinlocks to `0x100 + 4*n` (RP2040 had them at `0x000`), so
`SPINLOCK0` is `0x100` and `SPINLOCK31` is `0x17C`. Reading a lock address tries
to claim it: if it was free the read returns `1 << n` and the lock becomes held,
otherwise the read returns `0`. A write of **any** value releases the lock.
`SPINLOCK_ST` (`0x05C`, read-only) returns the claim bitmap with one bit per
lock, `1 = held`, reset value `0`.
RP2350 把自旋锁移到 `0x100 + 4*n`（RP2040 在 `0x000`），因此 `SPINLOCK0`
是 `0x100`、`SPINLOCK31` 是 `0x17C`。读取锁地址即尝试取锁：空闲时返回
`1 << n` 并置为占用，否则返回 `0`。写入**任意**值即释放。`SPINLOCK_ST`
（`0x05C`，只读）返回占用位图，每位对应一个锁，`1 = 已锁`，复位值 `0`。

Lock state lives outside the per-core view: `select_core` only redirects the FIFO,
doorbells and `CPUID`, so either core's view reads and releases the same locks.
The whole `SioState` sits behind one mutex, so the claim is a single
check-and-set step — the sequential-execution stand-in for the hardware's atomic
claim. "Core 0 wins a simultaneous claim" therefore means only that a driver
stepping core 0 first lets it reach a free lock first; the model deliberately
adds no concurrency semantics. [`Sio::spinlock_state`] exposes the same bitmap
without a bus access for tests and the CLI.
锁状态位于按核视图之外：`select_core` 只改变 FIFO、门铃与 `CPUID` 的方向，
两个核心视图读写的是同一组锁。全部 `SioState` 位于同一个互斥锁之后，因此取锁
是一步完成的检查并置位——这是顺序执行模型对硬件原子取锁的替代。所谓「core0 在
同时争抢时获胜」，在这里仅意味着先推进 core0 的驱动器先到达空闲锁；模型有意
不引入并发语义。[`Sio::spinlock_state`] 绕过总线暴露同一份位图，供测试与 CLI
使用。

### Per-core view / 按核视图

The SIO window is per core on real silicon: both cores see `0xD000_0000`, but
`FIFO_WR` feeds the *peer's* receive queue and `FIFO_RD` drains what the peer
wrote. Because one bus serves both cores there is a single `Sio` device, and
`select_core` flips its direction (one relaxed atomic store shared by every
clone, including the bus-mounted device). A driver that steps two cores calls
`select_core(Core1)` around core 1's instruction; the doorbells and `CPUID` flip
with it, so core 1's `CPUID` reads 1 and its `DOORBELL_OUT_SET` rings core 0.
真实芯片上 SIO 窗口是按核区分的：两个核心都在 `0xD000_0000` 看到它，但
`FIFO_WR` 压入**对端**的接收队列、`FIFO_RD` 排空对端写入的数据。由于一条总线
同时服务两个核心，这里只有一个 `Sio` 设备，由 `select_core` 翻转方向（一次
relaxed 原子存储，所有克隆共享，含总线挂载的设备）。推进双核的驱动器在 core1
的指令前后调用 `select_core(Core1)`；门铃与 `CPUID` 一并翻转，因此 core1 读到
`CPUID == 1`，其 `DOORBELL_OUT_SET` 敲响 core 0。

### Interrupt endpoints / 中断端点

`SIO_IRQ_FIFO` (25) and `SIO_IRQ_BELL` (26) are per-core despite sharing line
numbers: `Sio::new` publishes core 0's levels into the shared `IrqAssert`, and a
second mask ([`Sio::core1_irq_mask`]) carries core 1's. `BootPeripherals`
exposes both views (`irq_mask` for core 0, `core1_irq_mask` for core 1), so a
FIFO frame pushed by core 0 only pends core 1 and vice versa.
`SIO_IRQ_FIFO`（25）与 `SIO_IRQ_BELL`（26）虽然线号相同，但按核独立：
`Sio::new` 把 core0 的电平发布到共享 `IrqAssert`，core1 的电平放在第二个掩码
（[`Sio::core1_irq_mask`]）中。`BootPeripherals` 同时暴露两个视图（core0 用
`irq_mask`，core1 用 `core1_irq_mask`），因此 core0 压入的 FIFO 帧只会挂起
core1，反之亦然。

### FIFO blocking reads, sticky status and sleep events / FIFO 阻塞读、粘滞状态与睡眠事件

`FIFO_ST` implements the datasheet fields exactly (`SIO_FIFO_ST_BITS = 0x0F`):

| Bit | Field | Meaning / 含义 |
|-----|-------|----------------|
| 0 | `VLD` | RX FIFO not empty / 接收 FIFO 非空 |
| 1 | `RDY` | TX FIFO not full (reset value 1) / 发送 FIFO 未满（复位值 1） |
| 2 | `WOF` | sticky: wrote while full / 粘滞：写满 |
| 3 | `ROE` | sticky: read while empty / 粘滞：读空 |

`WOF` and `ROE` are `WC`: writing 1 clears them, writing 0 leaves them set, and
every other bit of the register is read-only. `SIO_IRQ_FIFO` for a core is the
logical OR of **its own** `VLD`, `WOF` and `ROE`, exactly as the datasheet
defines it, so a dropped write or a dropped read also raises the line. The NVIC
`ISER` bit is the enable/mask stage; there is no separate SIO interrupt-enable
register in the RP2350 register map (verified against the SDK's
`hardware/regs/sio.h`: the only SIO endpoints are `SIO_IRQ_FIFO` and
`SIO_IRQ_BELL`, and there is no `PROC0_INTE`/`PROC0_INTS` pair).

A read of `FIFO_RD` is a **blocking read**: while the receiving FIFO is empty the
bus reports `BusErrorKind::Stall` (through `MmioDevice::stall_on_read32` →
`MemoryRegion::read_would_stall`) and the access never reaches the device, so no
frame is consumed and no flag is latched. The CPU retries the same load when the
data arrives (see `rp2350-arm` / `rp2350-riscv`). A sub-word read bypasses the
word gate and keeps the datasheet's "ignored read + ROE" behaviour, which is
also what a direct `Sio::read32` call does.

`event_generation()` is a monotonic counter bumped by every state change that can
complete a blocked access or wake a sleeping core: a FIFO push or pop, a
doorbell change, and any change of the two per-core interrupt levels. A driver
that sleeps cores compares it against its own snapshot and turns a change into an
ARM event (`CortexM33Core::signal_event`), which is what makes the SDK's
`while (!multicore_fifo_rvalid()) __wfe();` handshake work.
`fifo_read_blocking(core)` is the host-side contract of the same operation: it
pops one frame when available and returns `None` **without** latching `ROE` or
consuming anything when the queue is empty.

`FIFO_ST` 精确实现数据手册字段（`SIO_FIFO_ST_BITS = 0x0F`）：`VLD`、`RDY`、
`WOF`、`ROE`。`WOF`/`ROE` 是写 1 清除的粘滞位，寄存器其余位只读。某核的
`SIO_IRQ_FIFO` 是该核**自身** `VLD`、`WOF`、`ROE` 的逻辑或，与数据手册一致，
因此被丢弃的写或读同样拉高该线。使能/屏蔽阶段由 NVIC `ISER` 承担；RP2350 寄存器
表中没有独立的 SIO 中断使能寄存器（已用 SDK `hardware/regs/sio.h` 核对：SIO 只有
`SIO_IRQ_FIFO` 与 `SIO_IRQ_BELL` 两个端点，并不存在 `PROC0_INTE`/`PROC0_INTS`）。

`FIFO_RD` 的读是**阻塞读**：接收 FIFO 为空时总线报告 `BusErrorKind::Stall`
（经 `MmioDevice::stall_on_read32` → `MemoryRegion::read_would_stall`），访问根本
不会到达设备，因此不消费帧、也不锁存任何标志。数据到达后 CPU 重放同一条载入
（见 `rp2350-arm` / `rp2350-riscv`）。非整字读绕过整字门控，保持数据手册的
「忽略空读 + ROE」行为，直接调用 `Sio::read32` 时也是如此。

`event_generation()` 是单调计数器，任何可能完成阻塞访问或唤醒睡眠核的状态变化都会
递增它：FIFO 压入/弹出、门铃变化，以及两条按核中断电平的任何变化。让核心睡眠的
驱动器把它与自身快照比较，并把变化转成 ARM 事件
（`CortexM33Core::signal_event`），这正是 SDK 的
`while (!multicore_fifo_rvalid()) __wfe();` 握手能工作的原因。
`fifo_read_blocking(core)` 是同一操作的宿主侧契约：有帧时弹出，队列为空时返回
`None` 且**不**锁存 `ROE`、不消费任何东西。

### Not modelled yet / 尚未建模

`INTERP0/1`, the TMDS encoder (`TMDS_CTRL`/`TMDS_WDATA`/`TMDS_*POP*`),
`PERI_NONSEC`, `RISCV_SOFTIRQ`, the RISC-V `MTIME*` block, and
exclusive/`LDREX`/`STREX` monitors. The FIFO is 8 entries deep per direction with
sticky `WOF`/`ROE`; the spinlocks are 32 bits wide because the RP2350 block
exposes exactly `SPINLOCK0..31`.
`INTERP0/1`、TMDS 编码器（`TMDS_CTRL`/`TMDS_WDATA`/`TMDS_*POP*`）、
`PERI_NONSEC`、`RISCV_SOFTIRQ`、RISC-V `MTIME*` 块，以及独占访问监视器均未
建模。每方向 FIFO 深度 8，带粘滞 `WOF`/`ROE`；自旋锁为 32 位，因为 RP2350
该块只提供 `SPINLOCK0..31`。

---

## PSM and BOOTRAM / 电源状态机与 BOOTRAM

```rust
pub struct Psm { /* FRCE_ON, FRCE_OFF, WDSEL, DONE + release/reset flags */ }
impl Psm {
    pub fn new() -> Self;
    pub fn with_core1_release(flag: Arc<AtomicBool>) -> Self;
    pub fn with_core1_flags(release: Arc<AtomicBool>, reset: Arc<AtomicBool>) -> Self;
}

pub struct Core1Launch { pub vtor: u32, pub sp: u32, pub entry: u32 }

impl BootPeripherals {
    pub fn service_core1_peer(&self);            // boot-ROM echo + PSM ack
    pub fn core1_launch_pending(&self) -> bool;
    pub fn take_core1_launch(&self) -> Option<Core1Launch>;
    pub fn core1_launch_count(&self) -> u64;
    pub fn core1_launched(&self) -> bool;
    pub fn take_core1_reset(&self) -> bool;
    pub fn reset_core1_boot(&self);
    pub fn sio(&self) -> &Sio;
    pub fn irq_mask(&self) -> u64;               // core-0 view
    pub fn core1_irq_mask(&self) -> u64;         // core-1 view
}
```

`Psm` models `PSM_FRCE_ON`, `PSM_FRCE_OFF`, `PSM_WDSEL` and the derived
`PSM_DONE` status. `FRCE_OFF.PROC1` drives two one-shot flags:
`0 → 1` latches a **hard reset** (the driver drops core 1's context and any
pending launch), `1 → 0` latches a **release** (the boot-ROM peer drains core
1's mailbox and pushes `0` to core 0). `multicore_reset_core1()` performs exactly
that set-then-clear pair and waits for the `0`.
`Psm` 模型实现 `PSM_FRCE_ON`、`PSM_FRCE_OFF`、`PSM_WDSEL` 与派生的 `PSM_DONE`
状态。`FRCE_OFF.PROC1` 驱动两个一次性标志：`0 → 1` 锁存**硬复位**（驱动器丢弃
core1 上下文与挂起的启动信息），`1 → 0` 锁存**释放**（boot ROM 对端清空 core1
信箱并向 core0 推入 `0`）。`multicore_reset_core1()` 正是先置位再清除并等待该
`0`。

### Core-1 launch protocol / core1 启动协议

pico-sdk `multicore_launch_core1_raw` writes the six-word stream
`{0, 0, 1, vtor, sp, entry}` to `SIO_FIFO_WR` and waits for each word to be
echoed through `SIO_FIFO_RD`. `BootPeripherals::service_core1_peer` is the
resident core-1 boot ROM peer: it consumes one word at a time from core 0's
outbox (`Sio::pop_from_core`), echoes it back (`Sio::post_to_core`) and watches a
rolling six-word window. When the window matches `{0, 0, 1, …}` the launch is
recorded — `vtor`, `sp` and `entry` become a `Core1Launch` — and the echo stub
**retires**: from that point on the FIFO belongs to the real core 1 and the peer
never touches it again. `take_core1_launch` hands the vectors to the driver,
which probes the vector table and calls `CortexM33Core::boot_at(sp, entry, vtor)`.
pico-sdk `multicore_launch_core1_raw` 向 `SIO_FIFO_WR` 写入六字流
`{0, 0, 1, vtor, sp, entry}`，并逐字等待其经 `SIO_FIFO_RD` 回显。
`BootPeripherals::service_core1_peer` 就是常驻的 core1 boot ROM 对端：它从
core0 的发送队列逐字取出（`Sio::pop_from_core`）、回显（`Sio::post_to_core`），
并维护一个六字滚动窗口。窗口匹配 `{0, 0, 1, …}` 时记录启动信息（`vtor`、`sp`、
`entry` 组成 `Core1Launch`），随后**回显桩退役**：此后 FIFO 属于真实的 core1，
对端不再触碰。`take_core1_launch` 把向量交给驱动器，驱动器先探测向量表再调用
`CortexM33Core::boot_at(sp, entry, vtor)`。

`entry` 在 SDK 2.x 中指 Flash 中的 `core1_trampoline`（`pop {r0,r1,pc}`），
它的栈上已有 `{entry, stack_base, core1_wrapper}`，因此 core1 直接跳到
`core1_wrapper` 并执行每核初始化。这些初始化里的 `rom_func_lookup`（表项
`'SR'`）也必须经过合成 boot ROM 的陷阱分发，否则 core1 会执行到 `udf` 占位桩。
`entry` points at the flash-resident `core1_trampoline` (`pop {r0,r1,pc}`) in
SDK 2.x; its stack already holds `{entry, stack_base, core1_wrapper}`, so core 1
lands in `core1_wrapper` and runs the per-core initializers. Their
`rom_func_lookup` (`'SR'` table entry) must also go through the synthetic boot
ROM trap dispatch, otherwise core 1 walks into the `udf` stubs.

`BootRam` exposes the scratch RAM at `0x400E_0000`; its first word is a
`bx lr` boot2 copy-out stub so pico-sdk `flash_init_boot2_copyout` can return
while this model keeps flash XIP readable.
`BootRam` 暴露 `0x400E_0000` 的暂存 RAM；首字为 `bx lr` boot2 拷出桩，使
pico-sdk 的 `flash_init_boot2_copyout` 能立即返回，而本模型保持 Flash XIP 可读。

---

## PPB (private peripheral bus) / 私有外设总线

`ppb::Ppb` owns the 4 KB window at `0xE000_0000`. `CPUID` reports a Cortex-M33
part number, the SysTick registers are backed by `SysTickCounter` (which follows
the live clock tree), and every other word is storage so a firmware `ldr`/`str`
never faults on an unmodelled SCB/MPU field.
`ppb::Ppb` 拥有 `0xE000_0000` 处的 4 KB 窗口。`CPUID` 报告 Cortex-M33 部件号，
SysTick 寄存器由跟随实时时钟树的 `SysTickCounter` 支撑，其余字都是存储，因此固件
对未建模 SCB/MPU 字段的 `ldr`/`str` 不会触发故障。

### Replay list / 回放清单

Writes to registers whose behaviour lives in `rp2350-arm` are appended to a
shared log (`SystemWriteLog`) and drained by the driver through
`CpuCore::system_write` before the next instruction (PPB write epoch) or at the
boot-ROM trap pump. Firmware-side configuration therefore takes effect
immediately, exactly like a real store into the system control space.
行为属于 `rp2350-arm` 的寄存器写入会追加到共享日志（`SystemWriteLog`），驱动在
下一条指令前（PPB 写世代）或 boot ROM 陷阱泵送时通过 `CpuCore::system_write`
排空。因此固件侧配置立即生效，与真实写入系统控制空间一致。

| Register / 寄存器 | Address / 地址 | Replayed semantics / 回放语义 |
|-------------------|----------------|-------------------------------|
| `SCB_VTOR` | `0xE000_ED08` | Vector table base (low 7 bits reserved) / 向量表基址（低 7 位保留） |
| `SCB_ICSR` | `0xE000_ED04` | NMI/PendSV/SysTick pending set/clear / NMI/PendSV/SysTick 挂起置位与清除 |
| `SCB_AIRCR` | `0xE000_ED0C` | `VECTKEY` gate, `PRIS`, `SYSRESETREQ`/`VECTRESET` latches / `VECTKEY` 门控、`PRIS`、复位锁存 |
| `SCB_SCR` | `0xE000_ED10` | `SLEEPONEXIT`/`SLEEPDEEP`/`SEVONPEND` / 睡眠与事件位 |
| `SCB_SHPR2` | `0xE000_ED1C` | SVCall priority / SVCall 优先级 |
| `SCB_SHPR3` | `0xE000_ED20` | PendSV and SysTick priorities / PendSV 与 SysTick 优先级 |
| `SCB_SHCSR` | `0xE000_ED24` | `MEMFAULTENA`/`BUSFAULTENA`/`USGFAULTENA`; `*ACT`/`SVCALLPENDED` are live core views / 三个故障使能；活跃与挂起位为核内实时视图 |
| `SCB_CFSR` | `0xE000_ED28` | MMFSR/BFSR/UFSR status, write-one-to-clear / 故障状态，写 1 清零 |
| `SCB_HFSR` | `0xE000_ED2C` | `VECTTBL`/`FORCED`/`DEBUGEVT`, write-one-to-clear / 写 1 清零 |
| `SCB_CPACR` | `0xE000_ED88` | CP0–CP7 (GPIOC/DCP) and CP10–CP11 (FPU) keys / 协处理器访问键 |
| `SCB_STIR` | `0xE000_EF00` | Software-triggered external interrupt / 软件触发外部中断 |
| NVIC | `0xE000_E100..=0xE000_E4FF` | `ISER`/`ICER`/`ISPR`/`ICPR`/`IPR` / NVIC 使能、挂起与优先级 |

SysTick (`0xE000_E010..=0xE000_E01C`), `CPUID` and every unmodelled SCB/MPU word
stay local to the PPB: replaying the SysTick counter would make it tick twice.
SysTick（`0xE000_E010..=0xE000_E01C`）、`CPUID` 与所有未建模 SCB/MPU 字只留在
PPB 内：回放 SysTick 计数器会让它重复计时。

### Read-back shadow / 读回影子

`SHCSR`, `CFSR`, `HFSR` and `AIRCR` are architecturally owned by the core, so the
PPB keeps a **cache only** and applies the same write rules the core applies.
`SHCSR` stores the three fault enables (`0x0007_0000`); `CFSR`/`HFSR` are
write-one-to-clear (writing 0 changes nothing); an `AIRCR` write is accepted only
when `bits[31:16] == 0x05FA` and then stores `PRIS` only. `AIRCR` always reads
`VECTKEYSTAT = 0xFA05_0000` and the write-only reset requests read as zero. The
cache never becomes a second source of truth: fault escalation, live `*ACT`/
`SVCALLPENDED` views and the reset latch are computed inside `rp2350-arm`.
`SHCSR`、`CFSR`、`HFSR` 与 `AIRCR` 架构上属于核，因此 PPB 只保留**缓存**，并套用
核的写入规则。`SHCSR` 保存三个故障使能（`0x0007_0000`）；`CFSR`/`HFSR` 写 1
清零（写 0 不变）；`AIRCR` 仅当 `bits[31:16] == 0x05FA` 时接受写入，且只保存
`PRIS`。`AIRCR` 恒读 `VECTKEYSTAT = 0xFA05_0000`，只写的复位请求读 0。缓存绝不
成为第二真相源：故障升级、`*ACT`/`SVCALLPENDED` 实时视图与复位锁存都在
`rp2350-arm` 内计算。

A write without `VECTKEY` is ignored by the core and leaves both the cache and the
reset latch unchanged, which is why `firmware` disassembly that clears `PRIS`
with a bare `0x0000_xxxx` store has no effect.
无 `VECTKEY` 的写入被核忽略，缓存与复位锁存都不变，因此用裸 `0x0000_xxxx` 写入
清 `PRIS` 的固件不会有任何效果。

### System reset request / 系统复位请求

`AIRCR.SYSRESETREQ` is latched by `FaultState` and read through
`CortexM33Core::take_system_reset_request()` (one-shot; repeated polls return
`false`). `rp2350-cli::run::Machine::take_system_reset_request()` polls both
Cortex-M33 cores and stops advancing the machine, precisely so a frontend can
act before the guest executes more pre-reset code. `run` rebuilds the machine
from the same image at most 3 times, while `repl`/`play` report and stop; the
policy is documented in [`rp2350-cli.md`](rp2350-cli.md).
`AIRCR.SYSRESETREQ` 由 `FaultState` 锁存，经
`CortexM33Core::take_system_reset_request()` 读取（一次性；重复轮询返回
`false`）。`rp2350-cli::run::Machine::take_system_reset_request()` 轮询两个
Cortex-M33 核并停止推进机器，正是为了让前端在客户机执行更多复位前代码之前响应。
`run` 用同一映像最多重建 3 次，`repl`/`play` 报告并停止；策略见
[`rp2350-cli.md`](rp2350-cli.md)。

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