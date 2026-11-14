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
pub struct Pio { /* private register file */ }

impl Pio {
    pub fn new(index: u8) -> Self;
    pub fn push_rx(&mut self, sm: usize, value: u32) -> bool;
    pub fn pop_tx(&mut self, sm: usize) -> Option<u32>;
}
```

`Pio` implements `MmioDevice` over a 4 KB window. `new(0/1/2)` selects
PIO0/PIO1/PIO2; any other index panics.
`Pio` 在 4 KB 窗口上实现 `MmioDevice`。`new(0/1/2)` 选择 PIO0/PIO1/PIO2，
其它序号会 panic。

### Modelled registers / 已建模寄存器

- `CTRL` — `SM_ENABLE[3:0]` is read back; restart/clock-restart strobes are
  self-clearing and not retained.
  `CTRL` — 可读回 `SM_ENABLE[3:0]`；restart/clock-restart 脉冲位自清零，不保留。
- `FSTAT` — reset `0x0F00_0F00`; per-SM `RXFULL[3:0]`, `RXEMPTY[11:8]`,
  `TXFULL[19:16]`, `TXEMPTY[27:24]` are derived from FIFO levels.
  `FSTAT` — 复位值 `0x0F00_0F00`；每个状态机的 `RXFULL`、`RXEMPTY`、
  `TXFULL`、`TXEMPTY` 由 FIFO 占用推导。
- `FDEBUG` — sticky `RXSTALL`/`RXUNDER`/`TXOVER`/`TXSTALL` flags, write-one-to-clear.
  `FDEBUG` — 粘滞 `RXSTALL`/`RXUNDER`/`TXOVER`/`TXSTALL` 标志，写 1 清零。
- `FLEVEL` — 4-bit TX/RX occupancy per state machine.
  `FLEVEL` — 每个状态机的 4 位 TX/RX 占用。
- `TXF0..TXF3` — each write pushes one word into the 4-deep TX FIFO; a write on
  a full FIFO drops the word and sets `FDEBUG.TXOVER`.
  `RXF0..RXF3` — each read pops one word from the 4-deep RX FIFO; a read on an
  empty FIFO returns zero and sets `FDEBUG.RXUNDER`.
  `TXF0..TXF3` — 每次写向 4 深度 TX FIFO 压入一个字；满时丢弃该字并置位
  `FDEBUG.TXOVER`。`RXF0..RXF3` — 每次读从 4 深度 RX FIFO 取出一个字；
  空时返回 0 并置位 `FDEBUG.RXUNDER`。
- `IRQ` / `IRQ_FORCE` — eight PIO IRQ flags; `IRQ` is W1C, `IRQ_FORCE` sets
  the internal flags. No processor interrupt line is routed yet.
  `IRQ` / `IRQ_FORCE` — 8 个 PIO IRQ 标志；`IRQ` 写 1 清零，`IRQ_FORCE` 置位
  内部标志。尚未接入处理器中断线。
- `INPUT_SYNC_BYPASS`, `GPIOBASE` (`bit 4`), `DBG_CFGINFO` (`0x1020_0404`),
  `DBG_PADOUT`/`DBG_PADOE` (read zero).
  `INPUT_SYNC_BYPASS`、`GPIOBASE`（位 4）、`DBG_CFGINFO`（`0x1020_0404`）、
  `DBG_PADOUT`/`DBG_PADOE`（读 0）。
- `INSTR_MEM0..INSTR_MEM31` — 32 × 16-bit instruction slots. The hardware marks
  these write-only; this model permits readback for debugging.
  `INSTR_MEM0..INSTR_MEM31` — 32 × 16 位指令槽。硬件标称只写；本模型允许回读以便调试。
- `SM0..SM3` windows — `CLKDIV` (reset `0x0001_0000`), `EXECCTRL`
  (reset `0x0001_F000`), `SHIFTCTRL` (reset `0x000C_0000`), `ADDR`
  (read-only, zero), `INSTR`, `PINCTRL` (reset `0x1400_0000`).
  `SM0..SM3` 窗口 — `CLKDIV`（复位 `0x0001_0000`）、`EXECCTRL`（复位
  `0x0001_F000`）、`SHIFTCTRL`（复位 `0x000C_0000`）、`ADDR`（只读，0）、
  `INSTR`、`PINCTRL`（复位 `0x1400_0000`）。

`push_rx` feeds the RX FIFO from the device side and `pop_tx` drains the TX
FIFO as a state machine would; both return `false`/`None` on a full/empty FIFO
and set the corresponding sticky debug flag.
`push_rx` 从设备侧向 RX FIFO 送入数据，`pop_tx` 按状态机行为取走 TX FIFO
数据；FIFO 满/空时分别返回 `false`/`None` 并置位对应粘滞调试标志。

### Not modelled yet / 尚未建模

State-machine execution, program counter movement, FIFO joining
(`SHIFTCTRL.FJOIN_*`), DMA handshakes, the RP2350 `RXFn_PUTGET*` registers, the
interrupt registers at `0x16C..0x184` and processor interrupt routing. Accesses
to these offsets follow the project policy: read zero, ignore the write and
emit one warning per address.
状态机执行、程序计数器推进、FIFO 合并（`SHIFTCTRL.FJOIN_*`）、DMA 握手、
RP2350 `RXFn_PUTGET*` 寄存器、`0x16C..0x184` 的中断寄存器以及处理器中断路由。
访问这些偏移沿用项目策略：读为 0、忽略写入、每个地址只警告一次。

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