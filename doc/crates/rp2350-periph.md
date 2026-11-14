# rp2350-periph — Public API / 公共 API

> Built-in RP2350 peripherals: PIO, HSTX, DMA, XIP (QMI/Flash/PSRAM), UART, SPI, I2C, PWM, ADC, SIO, Clocks, Timer, Watchdog, USB.
> RP2350 内置外设：PIO、HSTX、DMA、XIP（QMI/Flash/PSRAM）、UART、SPI、I2C、PWM、ADC、SIO、时钟、定时器、看门狗、USB。

---

## PIO register blocks / PIO 寄存器块

RP2350 has **three** PIO blocks, unlike RP2040's two. `BootPeripherals::mount`
installs one `Pio` instance per block at its AHB base address:

| Block | Base | Constant |
|-------|------|----------|
| PIO0 | `0x5020_0000` | `map::PIO0_BASE` |
| PIO1 | `0x5030_0000` | `map::PIO1_BASE` |
| PIO2 | `0x5040_0000` | `map::PIO2_BASE` |

RP2350 与 RP2040 的两块 PIO 不同，共有 **三个** PIO 块。`BootPeripherals::mount`
在每个块的 AHB 基地址各挂载一个 `Pio` 实例。

```rust
pub struct Pio { /* private register file + four state machines */ }

impl Pio {
    pub fn new(index: u8) -> Self;
    pub fn push_rx(&mut self, sm: usize, value: u32) -> bool;
    pub fn pop_tx(&mut self, sm: usize) -> Option<u32>;
    pub fn step_sm(&mut self, sm: usize) -> bool;
    pub fn tick(&mut self, cycles: u64);
}
```

`Pio` implements `MmioDevice` over a 4 KB window. `new(0/1/2)` selects
PIO0/PIO1/PIO2; any other index panics. The implementation is split into
`pio/mod.rs` (register block), `pio/regs.rs` (register layout, FIFOs, state),
`pio/sm.rs` (clocking, JMP/WAIT/IRQ/SET) and `pio/shift.rs`
(IN/OUT/PUSH/PULL/MOV).
`Pio` 在 4 KB 窗口上实现 `MmioDevice`。`new(0/1/2)` 选择 PIO0/PIO1/PIO2，
其它序号会 panic。实现拆分为 `pio/mod.rs`（寄存器块）、`pio/regs.rs`
（寄存器布局、FIFO、状态）、`pio/sm.rs`（时钟驱动与 JMP/WAIT/IRQ/SET）以及
`pio/shift.rs`（IN/OUT/PUSH/PULL/MOV）。

### Modelled registers / 已建模寄存器

- `CTRL` — `SM_ENABLE[3:0]` is read back; `SM_RESTART[7:4]` resets the addressed
  state machine's PC, shift counters and stall state (FIFOs are preserved);
  clock-restart strobes are accepted but have no effect because `CLKDIV` is 1:1.
  `CTRL` — 可读回 `SM_ENABLE[3:0]`；`SM_RESTART[7:4]` 复位对应状态机的 PC、
  移位计数与阻塞状态（FIFO 内容保留）；时钟重启脉冲被接受但无效果，因为
  `CLKDIV` 按 1:1 处理。
- `FSTAT` — reset `0x0F00_0F00`; per-SM `RXFULL[3:0]`, `RXEMPTY[11:8]`,
  `TXFULL[19:16]`, `TXEMPTY[27:24]` are derived live from FIFO levels, so they
  track instruction execution.
  `FSTAT` — 复位值 `0x0F00_0F00`；每个状态机的 `RXFULL`、`RXEMPTY`、
  `TXFULL`、`TXEMPTY` 由 FIFO 占用实时推导，随指令执行更新。
- `FDEBUG` — sticky `RXSTALL`/`RXUNDER`/`TXOVER`/`TXSTALL` flags,
  write-one-to-clear. Stalls raised by `PULL`/`OUT` (empty TX) and
  `PUSH`/autopush (full RX) set `TXSTALL`/`RXSTALL` for the stalled machine.
  `FDEBUG` — 粘滞 `RXSTALL`/`RXUNDER`/`TXOVER`/`TXSTALL` 标志，写 1 清零。
  `PULL`/`OUT`（TX 空）与 `PUSH`/自动压入（RX 满）造成的阻塞会为对应状态机
  置位 `TXSTALL`/`RXSTALL`。
- `FLEVEL` — 4-bit TX/RX occupancy per state machine, derived from the FIFOs.
  `FLEVEL` — 每个状态机的 4 位 TX/RX 占用，由 FIFO 推导。
- `TXF0..TXF3` — each write pushes one word into the 4-deep TX FIFO; a write on
  a full FIFO drops the word and sets `FDEBUG.TXOVER`.
  `RXF0..RXF3` — each read pops one word from the 4-deep RX FIFO; a read on an
  empty FIFO returns zero and sets `FDEBUG.RXUNDER`.
  `TXF0..TXF3` — 每次写向 4 深度 TX FIFO 压入一个字；满时丢弃该字并置位
  `FDEBUG.TXOVER`。`RXF0..RXF3` — 每次读从 4 深度 RX FIFO 取出一个字；
  空时返回 0 并置位 `FDEBUG.RXUNDER`。
- `IRQ` / `IRQ_FORCE` — eight PIO IRQ flags; `IRQ` is W1C, `IRQ_FORCE` sets
  the internal flags. The `IRQ` instruction sets/clears/waits on these flags,
  but no processor interrupt line is routed yet.
  `IRQ` / `IRQ_FORCE` — 8 个 PIO IRQ 标志；`IRQ` 写 1 清零，`IRQ_FORCE` 置位
  内部标志。`IRQ` 指令可置位/清零/等待这些标志，但尚未接入处理器中断线。
- `INPUT_SYNC_BYPASS`, `GPIOBASE` (`bit 4`), `DBG_CFGINFO` (`0x1020_0404`),
  `DBG_PADOUT`/`DBG_PADOE` (read zero).
  `INPUT_SYNC_BYPASS`、`GPIOBASE`（位 4）、`DBG_CFGINFO`（`0x1020_0404`）、
  `DBG_PADOUT`/`DBG_PADOE`（读 0）。
- `INSTR_MEM0..INSTR_MEM31` — 32 × 16-bit instruction slots. The hardware marks
  these write-only; this model permits readback for debugging.
  `INSTR_MEM0..INSTR_MEM31` — 32 × 16 位指令槽。硬件标称只写；本模型允许回读以便调试。
- `SM0..SM3` windows — `CLKDIV` (reset `0x0001_0000`), `EXECCTRL` (reset
  `0x0001_F000`; `WRAP_TOP`/`WRAP_BOTTOM` at bits 16:12/11:7, `EXEC_STALLED`
  at bit 31), `SHIFTCTRL` (reset `0x000C_0000`), `ADDR` (read-only, current PC),
  `INSTR` (reads the instruction at PC; a write executes it immediately),
  `PINCTRL` (reset `0x1400_0000`).
  `SM0..SM3` 窗口 — `CLKDIV`（复位 `0x0001_0000`）、`EXECCTRL`（复位
  `0x0001_F000`；`WRAP_TOP`/`WRAP_BOTTOM` 位于位 16:12/11:7，`EXEC_STALLED`
  位于位 31）、`SHIFTCTRL`（复位 `0x000C_0000`）、`ADDR`（只读，当前 PC）、
  `INSTR`（读取 PC 处指令；写入会立即执行）、`PINCTRL`（复位 `0x1400_0000`）。

`push_rx` feeds the RX FIFO from the device side and `pop_tx` drains the TX
FIFO as a state machine would; both return `false`/`None` on a full/empty FIFO
and set the corresponding sticky debug flag.
`push_rx` 从设备侧向 RX FIFO 送入数据，`pop_tx` 按状态机行为取走 TX FIFO
数据；FIFO 满/空时分别返回 `false`/`None` 并置位对应粘滞调试标志。

### State-machine execution / 状态机执行

`step_sm(sm)` runs exactly one instruction of one machine and returns `true`
when it retired; it ignores `CTRL.SM_ENABLE` so tests can drive a single
machine. `tick(cycles)` runs every enabled machine once per cycle.
`step_sm(sm)` 让单台状态机执行恰好一条指令，指令退休时返回 `true`；它忽略
`CTRL.SM_ENABLE` 以便测试单台状态机。`tick(cycles)` 每个周期让所有已启用状态机
各执行一条指令。

| Instruction | Behaviour / 行为 |
|-------------|------------------|
| `JMP` | All conditions: always, `!x`, `x--`, `!y`, `y--`, `x!=y`, `pin` (reads 0, never taken), `!osre`. `x--`/`y--` post-decrement / 支持全部条件：always、`!x`、`x--`、`!y`、`y--`、`x!=y`、`pin`（读 0，不跳转）、`!osre`；`x--`/`y--` 为后递减 |
| `WAIT` | GPIO/PIN sources sample 0 without a pin model, so polarity 1 blocks forever; IRQ source waits for the flag / 未接引脚时 GPIO/PIN 恒读 0，极性 1 永久阻塞；IRQ 源等待标志 |
| `IN` | Sources: pins (0), x, y, null, status, isr, osr; honours `SHIFTCTRL.IN_SHIFTDIR` and `IN_COUNT` / 源：pins(0)、x、y、null、status、isr、osr；遵循 `IN_SHIFTDIR` 与 `IN_COUNT` |
| `OUT` | Destinations: pins (discarded), x, y, null, pindirs (discarded), pc, isr, exec / 目标：pins（丢弃）、x、y、null、pindirs（丢弃）、pc、isr、exec |
| `PUSH`/`PULL` | `block`, `iffull`/`ifempty`, and bit-4 `noerr` (suppresses the sticky stall flag) / 支持 `block`、`iffull`/`ifempty`，以及位 4 的 `noerr`（不置位粘滞 stall 标志） |
| `MOV` | Destinations: x, y, exec, pc, isr, osr; sources: x, y, null, status, isr, osr; op: none/invert/bit-reverse / 目标：x、y、exec、pc、isr、osr；源：x、y、null、status、isr、osr；运算：无/取反/位反转 |
| `IRQ` | Set, clear, wait and `rel` (SM-relative index) / 置位、清零、等待与 `rel`（状态机相对序号） |
| `SET` | x, y; pins/pindirs writes are discarded without a pin model / 支持 x、y；无引脚模型时 pins/pindirs 写入被丢弃 |

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

GPIO/PINCTRL pin mapping and side-set output (pins read 0 and writes are
discarded), delay cycles, `CLKDIV` (treated as 1:1), FIFO joining
(`SHIFTCTRL.FJOIN_*`), DMA handshakes, the RP2350 `RXFn_PUTGET*` registers, the
interrupt registers at `0x16C..0x184` and processor interrupt routing.
Cross-block `IRQ` routing (RP2350 index modes 1/3) currently falls back to this
block's flags.
GPIO/PINCTRL 引脚映射与 side-set 输出（引脚读 0、写入丢弃）、延迟周期、
`CLKDIV`（按 1:1 处理）、FIFO 合并（`SHIFTCTRL.FJOIN_*`）、DMA 握手、
RP2350 `RXFn_PUTGET*` 寄存器、`0x16C..0x184` 的中断寄存器以及处理器中断路由
尚未建模。跨块 `IRQ` 路由（RP2350 索引模式 1/3）目前回落到本块标志。

---

## HSTX / 高速发送

```rust
pub struct Hstx {
    pub csr: u32,
    pub clkdiv: u32,
    pub fifo_levels: [u8; 8],
    pub frame_buffer: Vec<u32>,         // raw 32-bit pixel words
    pub pixel_count: usize,
    pub pixel_read: usize,
    pub frame_sender: Option<mpsc::Sender<DecodedFrame>>,
    pub dma_ready: bool,
    pub frame_ready_cycles: u64,
}

impl Hstx {
    pub fn consume_pixels(&mut self, count: usize) -> &[u32];
    pub fn frame_ready(&self) -> bool;
    pub fn snapshot(&self) -> Vec<u32>;
    pub fn pixel_count(&self) -> usize;
}

pub struct HstxDecoder {
    pub format: PixelFormat,
    pub width: u32,
    pub height: u32,
    pub bytes_per_pixel: u8,
}

impl HstxDecoder {
    pub fn decode(&self, raw: &[u32]) -> Vec<u8>;  // RGBA8 output
}
```

---

## DMA

```rust
pub struct DmaController {
    pub channels: [DmaChannel; 16],
    pub dreq_states: [bool; DREQ_MAX],
    pub timer: Option<DmaTimer>,
}

pub struct DmaChannel {
    pub read_addr: u32,
    pub write_addr: u32,
    pub trans_count: u32,
    pub ctrl: DmaCtrl,
    pub ctrl_trig: DmaCtrl,
    pub active: bool,
    pub stalled: bool,
    pub chain_to: u8,
}

impl DmaController {
    pub fn tick(&mut self, cycles: u64, chip: &mut RP2350Chip);
    pub fn set_dreq(&mut self, dreq: DREQChannel, asserted: bool);
    pub fn abort_channel(&mut self, ch: usize);
}
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