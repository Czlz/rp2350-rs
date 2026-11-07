# rp2350-core — Public API / 公共 API

> Core crate: chip topology, address bus, clock tree, IRQ matrix, GPIO pins.
> 核心 crate：芯片拓扑、地址总线、时钟树、IRQ 矩阵、GPIO 引脚。

---

## Key Types / 关键类型

### `AddressBus`

A unified address space router that dispatches reads/writes to registered `BusDevice` instances.
统一地址空间路由器，将读写请求分发到已注册的 `BusDevice` 实例。

```rust
impl AddressBus {
    /// Register a device at a specific address range.
    /// 在指定地址范围注册设备。
    pub fn register_device(&mut self, base: u32, size: u32, device: Box<dyn BusDevice>) -> Result<(), BusError>;

    /// Read 32-bit word from address.
    pub fn read_u32(&self, addr: u32) -> u32;
    pub fn read_u16(&self, addr: u32) -> u16;
    pub fn read_u8(&self, addr: u32) -> u8;

    /// Write 32-bit word to address.
    pub fn write_u32(&mut self, addr: u32, value: u32);
    pub fn write_u16(&mut self, addr: u32, value: u16);
    pub fn write_u8(&mut self, addr: u32, value: u8);

    /// Write with atomic operation (for APB alias region).
    /// 原子操作写入（用于 APB 别名区域）。
    pub fn write_atomic(&mut self, addr: u32, value: u32, atomic_type: AtomicType);

    /// Tick all registered devices.
    pub fn tick(&mut self, cycles: u64);
}
```

### `RP2350Chip`

The top-level chip struct holding all state.
顶层芯片结构体，持有所有状态。

```rust
pub struct RP2350Chip {
    pub bus: AddressBus,
    pub cores: Vec<Box<dyn CpuCore>>,
    pub current_core: usize,
    pub sram: Box<[u8]>,
    pub xip_flash: QspiFlash,
    pub psram: Psram,
    pub pio: Vec<RpPio>,
    pub hstx: Hstx,
    pub dma: DmaController,
    pub gpio: Vec<GpioPin>,
    pub qspi: Vec<GpioPin>,
    pub clock: ClockTree,
    pub irq: IrqMatrix,
    pub plugin_host: PluginHost,
    pub debug: DebugContext,
}

impl RP2350Chip {
    /// Execute N cycles, return actual cycles executed.
    /// 执行 N 个周期，返回实际执行周期数。
    pub fn step_batch(&mut self, max_cycles: u64) -> u64;

    /// Full chip reset.
    pub fn reset(&mut self);

    /// Load firmware from ELF/IHEX file.
    /// 从 ELF/IHEX 文件加载固件。
    pub fn load_firmware(&mut self, path: &Path) -> Result<LoadResult>;
}
```

### `GpioPin`

```rust
pub struct GpioPin {
    pub index: u8,
    pub function: PinFunction,   // SIO / PIO0 / PIO1 / PIO2 / UART / SPI / ...
    pub direction: PinDirection, // Input / Output
    pub value: bool,
    pub output_enable: bool,
    pub pull_up: bool,
    pub pull_down: bool,
    pub input_sync_bypass: bool,
    pub irq_override: bool,
}

impl GpioPin {
    pub fn get_level(&self) -> bool;
    pub fn set_function(&mut self, function: PinFunction);
    pub fn set_direction(&mut self, direction: PinDirection);
    pub fn set_output(&mut self, value: bool);
    pub fn output_value(&self) -> bool;
    pub fn output_enabled(&self) -> bool;
}
```

### `ClockTree`

```rust
pub struct ClockTree {
    pub clk_sys: u32,       // System clock (Hz), default 125 MHz
    pub clk_peri: u32,      // Peripheral clock (Hz)
    pub clk_usb: u32,       // USB clock (48 MHz)
    pub clk_adc: u32,       // ADC clock (48 MHz)
    pub clk_hstx: u32,      // HSTX clock
    pub cycle_counter: u64, // Total simulation cycles
}
```

### `IrqMatrix`

```rust
pub struct IrqMatrix {
    pub pending: [AtomicBool; 256],  // IRQ pending flags
    pub enabled: [AtomicBool; 256],  // IRQ enable masks
}

impl IrqMatrix {
    pub fn set_irq(&self, irq: u32, value: bool);
    pub fn get_irq(&self, irq: u32) -> bool;
    pub fn process_pending(&self, cores: &mut [Box<dyn CpuCore>]);
}
```

---

## Usage Example / 使用示例

```rust
use rp2350_core::{RP2350Chip, AddressBus, GpioPin, PinFunction};

// Create chip instance
let mut chip = RP2350Chip::new(CoreArch::Arm)?;

// Load firmware
chip.load_firmware(Path::new("firmware.elf"))?;

// Execute 1 million cycles
let cycles = chip.step_batch(1_000_000);
println!("Executed {} cycles, PC = 0x{:08X}", cycles, chip.cores[0].pc());

// Read GPIO state
let gpio25 = &chip.gpio[25];
println!("GPIO25: fn={:?}, dir={:?}, val={}", gpio25.function, gpio25.direction, gpio25.value);
```