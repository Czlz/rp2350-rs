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
| `memory` | `SimpleBus` (flat router), `RamRegion` (writable / ROM); tests live in `memory/tests.rs` |
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

`SimpleBus` overrides `write_u8` / `write_u16` / `write_u32` / `load_u8` for one
extra side effect: guest writes into the ARMv8-M PPB window
(`0xE000_0000..0xE010_0000`) advance a monotonic counter exposed by
[`SimpleBus::ppb_write_epoch`]. Those are the only writes that feed the core's
system-register replay queue, so a frontend that sees an unchanged epoch can
skip the drain (and its mutex) entirely. Routing, error and watchpoint
semantics are byte-for-byte the trait defaults.
`SimpleBus` 覆写 `write_u8` / `write_u16` / `write_u32` / `load_u8`，只增加一项
副作用：客户机对 ARMv8-M PPB 窗口（`0xE000_0000..0xE010_0000`）的写会推进
[`SimpleBus::ppb_write_epoch`] 暴露的单调计数器。这些写正是核系统寄存器回放队列的
唯一来源，因此前端发现世代未变即可完全跳过回放（及其互斥锁）。路由、错误与监视点
语义与 trait 默认实现逐字节一致。

### `SimpleBus` / `RamRegion`

```rust
pub struct SimpleBus { /* regions: Vec<Box<dyn MemoryRegion>>, buckets: [RegionBucket; 256] */ }
impl SimpleBus { pub fn new() -> Self; pub fn region_count(&self) -> usize; }
// AddressBus impl: rejects overlapping regions (BusErrorKind::Overlap) and
// routes accesses through a region cache (see below).

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

#### Region cache / 区域缓存

```text
bucket = addr >> 24                        // 256 buckets, one per 16 MiB
bucket[b] = { start, end, index }          // validated span + region index
```

Every instruction fetch and data access resolves an address to a region, so the
lookup is the hottest operation in the interpreter. `SimpleBus` keeps a
direct-mapped table of 256 buckets keyed by the top address byte; a hit costs a
table load and two comparisons instead of a linear scan over dynamic objects.
A bucket holds one span, so two regions inside the same 16 MiB bucket simply
overwrite each other and the next lookup falls back to the scan — the cache can
never change which region serves an address. `register_region` /
`unregister_region` drop the whole table.

`RamRegion` implements `read_u16` / `read_u32` / `write_u16` / `write_u32`
natively: one bounds check and one slice copy instead of one `Vec::get` per
byte. An access whose span leaves the buffer, a read-only region, and every
store taken while a write watch is armed fall back to the byte-wise path, so
per-byte error, cycle and diagnostic semantics are unchanged.

每次取指与每次数据访问都要把地址解析到区域，因此该查找是解释器最热的操作。
`SimpleBus` 按地址最高字节索引 256 个桶直接映射缓存：命中只需一次表读取加两次
比较，而不是遍历动态对象。一个桶只保存一个跨度，同一 16 MiB 桶内的两个区域会
互相覆盖，下一次查找自动回退到线性扫描，因此缓存绝不改变地址由哪个区域响应。
`register_region` / `unregister_region` 会整体丢弃缓存表。

`RamRegion` 原生实现 `read_u16`/`read_u32`/`write_u16`/`write_u32`：一次边界
检查加一次切片拷贝，而不是每字节一次 `Vec::get`。跨度越界、只读区域，以及已
安装写监视时的所有存储都回退到逐字节路径，因此逐字节的错误、周期与诊断语义
完全不变。

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

Every host-side mutation publishes a generation number:

```rust
/// Current flash content generation / 当前 Flash 内容世代号
pub fn content_generation() -> u32;
```

`FlashRegion::new`, `FlashRegion::load_bytes`, the `MemoryRegion::load_u8` path,
`FlashHandle::write` and `FlashHandle::erase` each bump it (relaxed atomic
`fetch_add`). Readers can therefore cache flash bytes and revalidate with one
relaxed load instead of taking the shared-store lock per access;
`rp2350-arm`'s flash instruction fetch cache is the only consumer today. The
counter is monotonic and process-wide, so a mismatch simply discards the cache.
每次宿主侧变更都会发布世代号：`FlashRegion::new`、`FlashRegion::load_bytes`、
`MemoryRegion::load_u8` 路径、`FlashHandle::write` 与 `FlashHandle::erase` 各递增
一次（relaxed 原子 `fetch_add`）。因此读取方可缓存 Flash 字节，并用一次 relaxed
读取重新验证，而不必每次访问都获取共享存储锁；目前唯一使用者是 `rp2350-arm`
的 Flash 取指缓存。计数器是进程级单调量，失配即丢弃整个缓存。

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
pub const MMIO_PAGE_SLOTS: usize = 64;                 // router page cache

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

#### Device page cache / 设备页缓存

```text
slot  = (addr >> 12) & 63                 // 64 directly-mapped slots
slot[s] = { start, end, index + 1 }       // validated child span, 0 = empty
```

`MmioRouter::resolve` used to scan every mounted child, which is what firmware
polling one register pays on every access. Direct (non-alias) hits are now
cached per 4 KiB page: a hit is one tag compare plus two range checks. The tag
stores the child's window span, so a slot that is reused by another page can
only miss and re-scan — it can never return the wrong device. Alias accesses
still take the scan and are never cached, because they must fold an operation
into the child. `mount` drops every slot.

`MmioRouter::resolve` 原先要遍历所有已挂载子设备，固件轮询单个寄存器时每次访问
都要付这个代价。现在直接（非别名）命中会按 4 KiB 页缓存：命中只需一次标签比较
加两次范围检查。标签保存子设备的窗口跨度，因此被其他页复用的槽只会未命中并
重新扫描，绝不会返回错误设备。别名访问仍走扫描且不进缓存，因为它们必须把操作
折叠到子设备上。`mount` 会丢弃全部槽。

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
pub fn set_write_watch(range: Option<(u32, u32)>);   // arm/disarm the address range
pub fn set_write_value(value: Option<u32>);          // arm/disarm the value watch
pub fn write_watch_armed() -> bool;                  // gate query for native wide stores
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

#### Write gate / 写门控

Arming either write channel sets a process-wide `AtomicBool`; `init()` leaves it
clear when neither variable is present. `set_pc` and `check_write` return on one
relaxed load while it is clear, so a normal run pays no thread-local access per
store or per instruction. `RamRegion`'s native wide stores consult
[`write_watch_armed`] and take the per-byte path while a watch is armed, which
reproduces the diagnostic exactly. The gate is sticky for the same reason the
read gate is: the watched range lives in thread-local storage, so one thread
disarming its own watch must not make another thread skip its checks.

任一条写通道安装后即置位一个进程级 `AtomicBool`；两个变量都不存在时 `init()`
保持其清零。清零状态下 `set_pc` 与 `check_write` 只做一次 relaxed load 就返回，
因此正常运行时每次存储与每条指令都不必访问 thread-local。`RamRegion` 的原生宽
存储会查询 [`write_watch_armed`]，在监视安装期间改走逐字节路径，从而精确复现
诊断输出。门控与读门控一样只置位不清零：监视区间位于 thread-local，一个线程
撤除自己的监视不得让其他线程跳过检查。


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
