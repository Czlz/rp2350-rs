# rp2350-core — Public API / 公共 API

> Core crate: address bus, memory regions, MMIO framework, GPIO, IRQ matrix,
> simulation clock, chip topology, firmware loading (ELF/UF2), and the
> write-watchpoint diagnostic.
> 核心 crate：地址总线、内存区域、MMIO 框架、GPIO、中断矩阵、仿真时钟、
> 芯片拓扑、固件加载（ELF/UF2）与写监视点诊断。
>
> **Status**: 已实现 / implemented — bus, memory, flash XIP, MMIO, GPIO, IRQ,
> clock, chip family, ELF/UF2 loaders, `watch`, `xip_maintenance`.

---

## Module Structure / 模块结构

| Module | Description / 描述 |
|--------|-------------------|
| `bus` | `AddressBus` / `MemoryRegion` dyn-safe traits, `AccessWidth`, `BusResult` |
| `memory` | `SimpleBus` (flat router), `RamRegion` (writable / ROM) |
| `flash` | `FlashRegion` (XIP + nocache alias), `FlashHandle` (host-only) |
| `mmio` | `MmioDevice` / `MmioWindow` / `MmioRegion` / `MmioRouter`, `mount_device` |
| `xip_maintenance` | Write-only XIP_MAINTENANCE window (`0x1800_0000`, 64 MB) |
| `watch` | `RP2350_WATCH` / `RP2350_WATCHV` write-watchpoint diagnostic |
| `chip` | `RP2350Chip` — GPIO pins + IRQ matrix + clock |
| `chip_family` | `ChipFamily`, `ChipCapabilities`, `ChipInfo`, RP2040 stub |
| `gpio` | `GpioPin`, `PinDirection`, `PinFunction`, `GpioIntFlags` |
| `irq` | `IrqMatrix` (256 IRQ pending/enable/priority) |
| `clock` | `SimulationClock`, `ClockConfig` |
| `firmware` | ELF parsing: `FirmwareImage` / `FirmwareSegment` |
| `uf2` | UF2 parsing + loading: `Uf2Image` / `Uf2Block` / `Uf2Family` / `Uf2LoadReport` |
| `error` | `CoreError`, `BusErrorKind` |
| `types` | `Address`, `CpuId`, `CycleCount`, `GpioNumber`, `IrqNumber`, `PinNumber` |

---

## Address Bus / 地址总线

`AddressBus` and `MemoryRegion` are **dyn-safe traits** (no generics), so
regions can be stored as `Box<dyn MemoryRegion>` and cores take
`&mut dyn AddressBus`. All accesses carry a `&mut u64` cycle counter.
`AddressBus` 与 `MemoryRegion` 是 **dyn-safe trait**（无泛型），区域以
`Box<dyn MemoryRegion>` 存储，核心以 `&mut dyn AddressBus` 访问。
所有访问都带 `&mut u64` 周期计数器。

```rust
pub trait AddressBus: Send + Sync {
    fn register_region(&mut self, region: Box<dyn MemoryRegion>) -> BusResult<()>;
    fn unregister_region(&mut self, name: &str);
    fn find_region_mut(&mut self, addr: Address) -> Option<&mut dyn MemoryRegion>;
    // Default-implemented routing accessors, all taking `&mut u64 cycles`:
    //   read_u8/u16/u32(addr, cycles) -> BusResult<T>        (BusError::NoDevice if unmapped)
    //   write_u8/u16/u32(addr, value, cycles) -> BusResult<()>
    /// Host-side byte write that bypasses guest write protection (loaders).
    /// 宿主侧字节写入，绕过客户机写保护（加载器路径）。
    fn load_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()>;
}

pub trait MemoryRegion: Send + Sync {
    fn name(&self) -> &str;
    fn base(&self) -> Address;
    fn size(&self) -> usize;
    fn end(&self) -> Address;                 // default: base + size
    fn contains(&self, addr: Address) -> bool;
    fn read_u8(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u8>;
    fn write_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()>;
    // read_u16/u32, write_u16/u32: default implementations compose byte access
    fn load_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()>;
}
```

`write_u32` on the bus calls `watch::check_write` so the `RP2350_WATCH`
diagnostic sees every word store.
总线上的 `write_u32` 会调用 `watch::check_write`，使 `RP2350_WATCH` 诊断能看到
每一次整字存储。

### `SimpleBus` / `RamRegion`

```rust
pub struct SimpleBus { /* regions: Vec<Box<dyn MemoryRegion>> */ }
impl SimpleBus { pub fn new() -> Self; pub fn region_count(&self) -> usize; }
// AddressBus impl: rejects overlapping regions (BusErrorKind::Overlap) and
// routes accesses in registration order.

pub struct RamRegion { /* name, base, writable, data: Vec<u8> */ }
impl RamRegion {
    pub fn new(name: impl Into<String>, base: Address, size: usize) -> Self;      // writable
    pub fn new_rom(name: impl Into<String>, base: Address, size: usize) -> Self;  // read-only
    pub fn bytes(&self) -> &[u8];
    pub fn is_writable(&self) -> bool;
}
// ROM writes from the guest return BusErrorKind::AccessDenied.
// 客户机写 ROM 返回 BusErrorKind::AccessDenied。
```

### Flash XIP / Flash XIP 窗口

```rust
pub const FLASH_XIP_BASE: Address = 0x1000_0000;
pub const FLASH_XIP_NOCACHE_BASE: Address = 0x1400_0000;  // XIP_NOCACHE_NOALLOC alias
pub const FLASH_XIP_SIZE: usize = 16 * 1024 * 1024;

pub struct FlashRegion { /* shared Arc<RwLock<Vec<u8>>> backing store */ }
impl FlashRegion {
    pub fn new(name: impl Into<String>, base: Address, size: usize) -> Self;
    pub fn rp2350_xip() -> Self;                        // standard 16 MB window
    pub fn alias(&self, name: impl Into<String>, base: Address) -> Self; // shares bytes
    pub fn bytes(&self) -> Vec<u8>;
    pub fn slice(&self, addr: Address, len: usize) -> Option<Vec<u8>>;
    pub fn handle(&self) -> FlashHandle;                // host-only erase/program path
    pub fn load_bytes(&mut self, addr: Address, bytes: &[u8]) -> BusResult<()>;
}

pub struct FlashHandle { /* base + shared store */ }
impl FlashHandle {
    pub fn base(&self) -> Address;
    pub fn size(&self) -> usize;
    pub fn read(&self, addr: Address, len: usize) -> BusResult<Vec<u8>>;
    pub fn write(&self, addr: Address, bytes: &[u8]) -> BusResult<()>;
    pub fn erase(&self, addr: Address, len: usize) -> BusResult<()>;  // fills 0xFF
}
```

Guest writes to flash return `AccessDenied`; only the loader
(`load_u8` / `load_bytes`) and a `FlashHandle` can program it. Clones and
`alias` windows share one backing store, so `0x1000_0000` and `0x1400_0000`
always agree.
客户机写 Flash 返回 `AccessDenied`；只有加载器（`load_u8`/`load_bytes`）和
`FlashHandle` 能编程。克隆与 `alias` 窗口共享底层存储，因此 `0x1000_0000` 与
`0x1400_0000` 始终一致。

### XIP maintenance window / XIP 维护窗口

```rust
pub const XIP_MAINTENANCE_BASE: Address = 0x1800_0000;
pub const XIP_MAINTENANCE_SIZE: usize = 0x0400_0000;      // 64 MB

pub struct XipMaintenanceRegion { /* name only */ }
impl XipMaintenanceRegion {
    pub fn new(name: impl Into<String>) -> Self;
}
```

Writes are accepted and discarded (cache maintenance, no cache model); reads
return 0. Out-of-window addresses report `NoDevice`.
写入被接受并丢弃（缓存维护；本模拟器无缓存模型），读取返回 0；
窗口外地址按 `NoDevice` 报错。

---

## MMIO framework / MMIO 框架

A device implements [`MmioDevice`] with a fixed window; `mount_device` wraps it
as a bus region. Devices living inside a larger span (e.g. the APB block) mount
into `MmioRouter`, which dispatches by offset and folds the RP2350 atomic
alias windows.
设备实现 [`MmioDevice`] 并声明固定窗口；`mount_device` 将其包装为总线区域。
位于更大跨度内（如 APB 块）的设备挂载到 `MmioRouter`，由其按偏移分发并折叠
RP2350 原子别名窗口。

```rust
pub const MMIO_REGISTER_SIZE: u32 = 4;
pub const MMIO_BLOCK_SIZE: u32 = 0x4000;
pub const MMIO_ALIAS_XOR: u32 = 0x1000;
pub const MMIO_ALIAS_SET: u32 = 0x2000;
pub const MMIO_ALIAS_CLR: u32 = 0x3000;

pub struct MmioWindow { pub base: Address, pub size: usize }
// new(base, size) | end() | contains(addr) | offset(addr) -> Option<u32>
// validate(): 4-byte aligned, size % 4 == 0, no wrap
// validate()：4 字节对齐、大小为 4 的倍数、不回绕

pub trait MmioDevice: Send + Sync {
    fn name(&self) -> &str;
    fn window(&self) -> MmioWindow;
    fn read32(&mut self, offset: u32) -> Option<u32>;       // None = unmodelled
    fn write32(&mut self, offset: u32, value: u32) -> bool; // false = unmodelled
    fn reset(&mut self) {}
    fn irq_number(&self) -> Option<u16> { None }
}

pub struct RegisterFile { /* flat u32 words */ }
// new(size_bytes) | read(offset) -> Option<u32> | write(offset, value) -> bool
// fill(offset, value) | reset()

pub struct MmioRegion { /* wraps Box<dyn MmioDevice> + one-shot warn set */ }
// new(device) | device() -> &dyn MmioDevice | device_mut() | reset()
// MemoryRegion impl: byte/halfword accesses merge into the containing word;
// one 32-bit access is one register access. Unmodelled offsets read 0 / ignore
// writes and log a one-shot warning per address.

pub struct MmioRouter { /* name + window + child regions */ }
// new(name, window) | window() | device_count()
// mount(device) -> BusResult<MmioWindow>: child window must sit inside the
// router window, must not overlap another child or any alias span, and may only
// use the first 0x1000 bytes of its own block. Unmodelled router offsets read 0
// / ignore writes with a one-shot warning.

pub fn mount_device(bus: &mut dyn AddressBus, device: Box<dyn MmioDevice>)
    -> BusResult<MmioWindow>;
```

Alias semantics: within each `MMIO_BLOCK_SIZE` block, `+0x1000` is XOR,
`+0x2000` SET, `+0x3000` CLR over the register with the same low 12 bits.
Alias reads return the underlying value; alias writes fold into one
read-modify-write on the child, which never sees an alias offset.
别名语义：每个 `MMIO_BLOCK_SIZE` 块内 `+0x1000` 为 XOR、`+0x2000` 为 SET、
`+0x3000` 为 CLR，作用于低 12 位相同的寄存器。别名读返回底层值；别名写折叠成
对子设备的一次读-改-写，子设备不会看到别名偏移。

---

## Chip topology / 芯片拓扑

```rust
pub struct RP2350Chip {
    pub gpio_pins: [GpioPin; GPIO_PIN_COUNT],   // 48 pins (QFN-60)
    pub irq_matrix: IrqMatrix,
    pub clock: SimulationClock,
}
impl RP2350Chip {
    pub fn new() -> Self;
    pub fn gpio(&self, pin: usize) -> Option<&GpioPin>;
    pub fn gpio_mut(&mut self, pin: usize) -> Option<&mut GpioPin>;
    pub fn tick(&mut self, cycles: u64);        // advances the clock
}impl ChipInfo for RP2350Chip { /* family() = Rp2350, capabilities() */ }
```

`RP2350Chip` owns **GPIO, IRQ and clock state only** — there is no bus, CPU or
peripheral collection here; hosts (e.g. `rp2350-cli`) assemble the machine from
`SimpleBus` + `BootPeripherals` + a `CpuCore`.
`RP2350Chip` 只拥有 **GPIO、IRQ 与时钟状态** —— 不含总线、CPU 或外设集合；
宿主（如 `rp2350-cli`）用 `SimpleBus` + `BootPeripherals` + `CpuCore` 组装整机。

### Chip family / 芯片家族

```rust
pub enum ChipFamily { Rp2040, Rp2350 }
pub struct ChipCapabilities {
    pub family: ChipFamily,
    pub flash_bytes: usize,   // RP2350: 16 MB
    pub sram_bytes: usize,    // RP2350: 520 KB
    pub gpio_pins: usize,     // RP2350: 48
    pub hazard3: bool,
    pub usb_device: bool,
    pub dual_core: bool,
}
pub trait ChipInfo { fn family(&self) -> ChipFamily; fn capabilities(&self) -> ChipCapabilities; }

// RP2040Chip: every operational entry point returns CoreError::UnsupportedChip
// RP2040Chip：所有操作入口均返回 CoreError::UnsupportedChip
pub struct RP2040Chip;
impl RP2040Chip {
    pub const fn new() -> Self;
    pub fn boot(&mut self, bus: &mut dyn AddressBus) -> Result<(), CoreError>;
    pub fn step(&mut self, bus: &mut dyn AddressBus) -> Result<(), CoreError>;
    pub fn read_u32(&self, address: Address) -> Result<u32, CoreError>;
    pub fn write_u32(&mut self, address: Address, value: u32) -> Result<(), CoreError>;
}```

### GPIO / 中断 / 时钟

```rust
pub enum PinDirection { Input, Output }
pub enum PinFunction { Null, Sio, F1, F2, F3, F4, F5, Rts }
bitflags! { pub struct GpioIntFlags: u8 { LEVEL_LOW | LEVEL_HIGH | EDGE_FALL | EDGE_RISE } }

pub struct GpioPin { /* private fields */ }
// new(number: PinNumber) | number() -> PinNumber | direction() | set_direction(dir)
// function() | set_function(func) | set_output(level: bool) | level() -> bool
// level(): output or external input level; set_input(level) drives it externally
// (e.g. DeviceManager); set_pull_up/pull_down/input_enable(enable: bool)
// interrupt_pending() -> bool | clear_interrupt(flags: GpioIntFlags)

pub struct IrqMatrix { /* pending/enabled/priority: IRQ_COUNT each */ }
// new() | is_pending(irq) | is_enabled(irq) | is_active(irq) = pending && enabled
// set_pending(irq) | clear_pending(irq) | enable(irq) | disable(irq)
// set_priority(irq, pri: u8) (lower = higher) | priority(irq) -> u8
// find_highest_active() -> Option<IrqNumber>

pub struct ClockConfig { pub refclk_hz: u32, pub sysclk_hz: u32, pub periphclk_hz: u32 }
// Default: 12 MHz refclk, 150 MHz sysclk / periphclk
pub struct SimulationClock { /* total_cycles + config */ }
// new() | with_config(config) | total_cycles() -> CycleCount | config() -> ClockConfig
// tick(cycles: CycleCount) | systick_value(reload, enable) -> u32 (24-bit view)
// peripheral_cycles(target_hz, ticks) -> CycleCount
```

---

## Firmware loading / 固件加载

```rust
pub struct FirmwareSegment { pub address: Address, pub data: Vec<u8>, pub mem_size: usize }
pub struct FirmwareImage { pub entry_point: Address, pub segments: Vec<FirmwareSegment> }
impl FirmwareImage {
    pub fn from_elf_bytes(bytes: &[u8]) -> Result<Self, CoreError>;
    pub fn load_into(&self, bus: &mut dyn AddressBus) -> Result<(), CoreError>;
}
// Keeps PT_LOAD segments only; load_into uses the host write path (load_u8) so
// it can program read-only flash, and zero-fills the .bss tail (mem_size).

pub const UF2_BLOCK_SIZE: usize = 512;
pub const UF2_MAX_PAYLOAD: usize = 476;
pub const UF2_MAGIC_START0: u32 = 0x0A32_4655;   // "UF2\n"
pub const UF2_MAGIC_START1: u32 = 0x9E5D_5157;
pub const UF2_MAGIC_END: u32 = 0x0AB1_6F30;

pub enum Uf2Arch { Arm, RiscV }
pub enum Uf2Family { Rp2040, Rp2350ArmSecure, Rp2350RiscV, Rp2350ArmNonSecure,
                     Rp2350Absolute, Rp2350Data, NoFamily, Unknown(u32) }
// Family IDs: Rp2040 0x68ED2B88/0xE48BFF56, Rp2350ArmSecure 0xE48BFF59,
// Rp2350RiscV 0xE48BFF5A, Rp2350ArmNonSecure 0xE48BFF5B,
// Rp2350Absolute 0xE48BFF57, Rp2350Data 0xE48BFF58.
impl Uf2Family { pub fn from_id(id: u32) -> Self; pub fn id(self) -> u32;
                 pub fn arch(self) -> Option<Uf2Arch>; pub fn is_rp2350(self) -> bool; }

pub enum Uf2Region { Flash, Sram }
pub fn region_for(addr: Address) -> Option<Uf2Region>;  // flash 0x1000_0000..0x1100_0000, SRAM 0x2000_0000..0x2008_2000

pub struct Uf2Block { pub block_number: u32, pub total_blocks: u32, pub flags: u32,
                      pub family: Uf2Family, pub target_addr: Address, pub payload: Vec<u8> }
impl Uf2Block { pub fn parse(bytes: &[u8]) -> Result<Self, CoreError>;
                pub fn is_loadable(&self) -> bool; pub fn end_addr(&self) -> Option<Address>; }

pub struct Uf2LoadReport { pub family: Uf2Family, pub blocks_total: usize, pub blocks_written: usize,
                           pub blocks_skipped: usize, pub min_address: Address, pub max_address: Address,
                           pub wrote_flash: bool, pub wrote_sram: bool }
pub struct Uf2Image { /* blocks + primary family */ }
impl Uf2Image {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CoreError>;
    pub fn from_file(path: &Path) -> Result<Self, CoreError>;
    pub fn family(&self) -> Uf2Family;
    pub fn blocks(&self) -> &[Uf2Block];
    pub fn block_count(&self) -> usize;
    pub fn matches_arch(&self, arch: Uf2Arch) -> bool;
    pub fn load_into(&self, bus: &mut dyn AddressBus) -> Result<Uf2LoadReport, CoreError>;
}
```

The loader validates the container only; RP2350 `IMAGE_DEF` metadata is **not**
interpreted.
加载器只校验容器；**不**解析 RP2350 `IMAGE_DEF` 元数据。

---

## Write watchpoint / 写监视点诊断

```rust
pub fn init();                       // parse RP2350_WATCH / RP2350_WATCHV from env
pub fn set_pc(pc: u32);              // refreshed once per instruction step
pub fn check_write(addr: u32, value: u32);
```

`RP2350_WATCH=START-END` (hex) prints every write whose address falls in
`[START, END)` together with the PC of the writing instruction;
`RP2350_WATCHV=VALUE` watches for a specific written value instead. Output goes
to stderr as `[watch] pc=… addr=… value=…`. Without the variables this is a
no-op.
`RP2350_WATCH=START-END`（十六进制）打印地址落入 `[START, END)` 的每次写及其
PC；`RP2350_WATCHV=VALUE` 改为监视特定写值。输出到 stderr，格式为
`[watch] pc=… addr=… value=…`。未设置变量时为空操作。

---

## Errors / 错误类型

```rust
pub enum BusErrorKind { NoDevice, Unaligned, AccessDenied, Overlap, DeviceError }
pub enum CoreError {
    BusError { addr: Address, kind: BusErrorKind },
    InvalidGpio(String),
    InvalidIrq(u16),
    ClockError(String),
    FirmwareError(String),
    UnsupportedChip(String),
    UndefinedInstruction { pc: Address, opcode: u32, width: u8 },
}
```

---

## Usage Example / 使用示例

```rust
use rp2350_core::{AddressBus, RamRegion, SimpleBus, FlashRegion, Uf2Image};

let mut bus = SimpleBus::new();
let flash = FlashRegion::rp2350_xip();
bus.register_region(Box::new(flash.alias("flash-xip-nocache", 0x1400_0000)))?;
bus.register_region(Box::new(flash))?;              // cached XIP window
bus.register_region(Box::new(RamRegion::new("sram", 0x2000_0000, 520 * 1024)))?;

// Load a UF2 through the host write path (programs read-only flash)
let image = Uf2Image::from_file(std::path::Path::new("firmware.uf2"))?;
let report = image.load_into(&mut bus)?;

let mut cycles = 0u64;
let pc0 = bus.read_u32(0x1000_0000, &mut cycles)?;  // first XIP word
```
