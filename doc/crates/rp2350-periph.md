# rp2350-periph — Public API / 公共 API

> Built-in RP2350 peripherals: PIO, HSTX, DMA, XIP (QMI/Flash/PSRAM), UART, SPI, I2C, PWM, ADC, SIO, Clocks, Timer, Watchdog, USB.
> RP2350 内置外设：PIO、HSTX、DMA、XIP（QMI/Flash/PSRAM）、UART、SPI、I2C、PWM、ADC、SIO、时钟、定时器、看门狗、USB。

---

## PIO (Programmable I/O) / 可编程 IO

```rust
pub struct RpPio {
    pub index: u8,
    pub sm: [StateMachine; 4],
    pub instr_mem: [u16; 32],
    pub irq: PioIrq,
    pub gpio_base: u16,
    pub fifo_join: FifoJoin,
    pub clkdiv: [u32; 4],
    pub input_sync_bypass: u32,
    pub dbg_padout: u32,
    pub dbg_padoe: u32,
}

impl RpPio {
    pub fn tick(&mut self, clk_sys: u32, gpio: &[GpioPin]);
    pub fn set_gpio_levels(&self, gpio: &mut [GpioPin]);
    pub fn read_sm_register(&self, sm_index: u8, offset: u32) -> u32;
    pub fn write_sm_register(&mut self, sm_index: u8, offset: u32, value: u32);
}
```

### StateMachine

```rust
pub struct StateMachine {
    pub pc: u8,
    pub isr: u32, pub isr_count: u8,
    pub osr: u32, pub osr_count: u8,
    pub x: u32, pub y: u32,
    pub tx_fifo: PioFifo,  // 4 deep
    pub rx_fifo: PioFifo,  // 4 deep
    pub exec_ctrl: SmExecCtrl,
    pub shift_ctrl: SmShiftCtrl,
    pub pin_ctrl: SmPinCtrl,
    pub delay_counter: u32,
    pub clkdiv_counter: u32,
    pub wait_state: WaitCondition,
    pub push_thresh: u8,
    pub pull_thresh: u8,
}
```

### PIO Assembler / PIO 汇编器

```rust
pub fn pio_assemble(source: &str) -> Result<Vec<u16>, AsmError>;
```

Example:
```asm
.program blink
    set pindirs, 1
loop:
    set pins, 1    ; GPIO high
    set pins, 0    ; GPIO low
    jmp loop
```

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