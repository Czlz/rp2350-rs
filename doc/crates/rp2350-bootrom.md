# rp2350-bootrom — Public API / 公共 API

> Synthetic RP2350 boot ROM images plus host-side trap dispatch. Vendor boot
> ROM binaries cannot be redistributed with this repository, so the emulator
> builds its own image: the documented header fields are present, table
> entries point at `udf` stubs, and the host executes the equivalent behaviour
> when execution reaches a stub address.
> 合成 RP2350 boot ROM 镜像与宿主侧陷阱分发。本仓库不能分发厂商 boot ROM
> 二进制，因此模拟器自行构建镜像：保留文档规定的头部字段，表项指向 `udf`
> 占位桩，执行到达桩地址时由宿主执行等价行为。
>
> **Status**: implemented — synthetic 32 KB RP2350 image, 8 ROM functions as
> trap stubs, `rom_table_lookup` and the two-level `FLASH_DEVINFO`
> indirection. The chip-agnostic plumbing (`table` / `image` / `region` /
> `trap`) is public and reusable for future chip layouts.
> **状态**: 已实现 — 32 KB RP2350 合成镜像、8 个 ROM 函数陷阱桩、
> `rom_table_lookup` 与两级 `FLASH_DEVINFO` 间接。与芯片无关的管线
> （`table` / `image` / `region` / `trap`) 是公共的，可被未来的芯片布局复用。

---

## Module Structure / 模块结构

| Module | Description / 描述 |
|--------|-------------------|
| `rp2350` | `Rp2350Bootrom`, `ROM_BASE`/`ROM_SIZE`/`ROM_MAGIC`, RP2350 trap semantics |
| `trap` | `TrapDispatcher` / `TrapHandler` / `TrapCall` / `TrapOutcome` / `TrapId` / `HaltReason` |
| `table` | `RomTableBuilder`, `TableCode`, `code`, `flags`, `lookup` — boot ROM table codec |
| `image` | `RomImageBuilder` — little-endian field writer, `udf #id` stub writer |
| `region` | `RomRegion` — read-only `MemoryRegion` over shared image bytes |

---

## `Rp2350Bootrom` / 合成 Boot ROM

```rust
pub const ROM_BASE: Address = 0x0000_0000;
pub const ROM_SIZE: usize = 0x8000;              // 32 KB window / 32 KB 窗口
pub const ROM_MAGIC: u32 = 0x0202_754D;          // 'M','u', magic v2, API v2

pub struct Rp2350Bootrom { /* private: image, dispatcher */ }
impl Rp2350Bootrom {
    /// `flash` is the host handle used by the flash erase/program functions;
    /// it writes the same backing store the guest reads through XIP.
    /// `flash` 是 Flash 擦除/编程函数使用的宿主句柄，写入客户机经 XIP 读取的
    /// 同一份底层存储。
    pub fn new(flash: FlashHandle) -> Self;

    /// Borrow the built image bytes / 借用已构建的镜像字节
    pub fn image(&self) -> &[u8];

    /// Register the read-only ROM window at `ROM_BASE` on `bus`
    /// 在 `bus` 的 `ROM_BASE` 注册只读 ROM 窗口
    pub fn mount(&self, bus: &mut dyn AddressBus) -> BusResult<()>;

    /// Report whether `pc` is a trap stub / 报告 `pc` 是否为陷阱占位桩
    pub fn handles(&self, pc: u32) -> bool;

    /// Dispatch the trap at the core's current PC; `Ok(None)` when the PC is
    /// not a stub, so the caller executes the instruction normally.
    /// 分发核心当前 PC 处的陷阱；PC 不是占位桩时返回 `Ok(None)`，调用方正常
    /// 执行指令。
    pub fn dispatch(&self, core: &mut dyn CpuCore, bus: &mut dyn AddressBus)
        -> Result<Option<TrapOutcome>, CoreError>;
}
```

Typical host loop (as used by `rp2350-cli`'s `Machine`): before stepping,
check `handles(core.pc())`; if it matches, call `dispatch` — a trap step
consumes one cycle, increments the instruction counter, and either returns the
guest through `lr` or halts with a `HaltReason`.
典型宿主循环（如 `rp2350-cli` 的 `Machine`）：步进前先查
`handles(core.pc())`；命中则调用 `dispatch` —— 陷阱步消耗一个周期、增加指令
计数，随后要么经 `lr` 返回客户机，要么以 `HaltReason` 停机。

Note: in the CLI the boot ROM traps only apply to **Arm** cores
(`bootrom_applicable` is false for RISC-V); Arm boot vectors are also read
from the flash XIP base, not from the ROM window.
注意：CLI 中 boot ROM 陷阱只对 **Arm** 核生效（RISC-V 时
`bootrom_applicable` 为 false）；Arm 启动向量同样读取自 Flash XIP 基地址，
而不是 ROM 窗口。

---

## Synthetic image layout / 合成镜像布局

Header fields follow the pico-sdk `bootrom_constants.h` offsets:
头部字段遵循 pico-sdk `bootrom_constants.h` 的偏移：

| Offset | Field / 字段 | Value / 值 |
|--------|-------------|------------|
| `0x10` | Magic word / magic 字 | `0x0202_754D` (byte `0x13` = boot ROM API version 2) |
| `0x14` | Function table offset / 函数表偏移 | `0x7C00` |
| `0x16` | `rom_table_lookup` pointer / 指针 | `0x74 \| 1` (Thumb) |
| `0x7C00` | Function/data table / 函数数据表 | built by `RomTableBuilder` |
| `0x7D00` | `FLASH_DEVINFO` word / 设备信息字 | `0x0C00` (CS0 = 16 MB, code 0xC at bits 11:8) |
| `0x7D04` | `FLASH_DEVINFO` pointer literal / 指针字面量 | `0x0000_7D00` |

Every callable function lives at a small `udf #id` stub: `rom_table_lookup`
at `0x74` (trap id 0) and the eight ROM functions at
`0x80 + 2 * (id - 1)`. The dispatcher keys on the stub address; the immediate
is only a debugging aid.
所有可调用函数都位于小的 `udf #id` 占位桩：`rom_table_lookup` 在 `0x74`
（陷阱 id 0），八个 ROM 函数在 `0x80 + 2 * (id - 1)`。分发器以桩地址为键，
立即数仅作调试线索。

### Boot ROM table codec / Boot ROM 表编解码

An entry is `u16 tag`, `u16 flags`, then one `u16` value per set flag bit in
ascending order; `tag == 0` terminates the table. Function pointers carry the
Thumb bit, data pointers are even.
表项为 `u16 tag`、`u16 flags`，随后按置位标志位升序每个标志写一个 `u16`
值；`tag == 0` 终止表。函数指针带 Thumb 位，数据指针为偶数。

```rust
pub type TableCode = u16;
pub const fn code(c1: u8, c2: u8) -> TableCode;   // 'G','S' -> 0x5347

pub mod flags {
    pub const FUNC_ARM_SEC: u16 = 0x0004;
    pub const FUNC_ARM_NONSEC: u16 = 0x0010;
    pub const DATA: u16 = 0x0040;
}

pub struct RomTableBuilder { /* private */ }
impl RomTableBuilder {
    pub fn new() -> Self;
    pub fn push_func(&mut self, tag: TableCode, secure: u16, nonsecure: u16);
    pub fn push_data(&mut self, tag: TableCode, pointer: u16);
    pub fn finish(self) -> Vec<u8>;              // appends the zero terminator
}

/// Returns the raw `u16` stored for the first matching flag bit in `mask`.
/// 返回 `mask` 中第一个命中标志位对应的原始 `u16`。
pub fn lookup(table: &[u8], code: TableCode, mask: u16) -> Option<u16>;
```

`FLASH_DEVINFO` uses the same two-level indirection as the real ROM: the data
entry tagged `FD` points at the 32-bit literal at `0x7D04`, which in turn
points at the 16-bit info word at `0x7D00` (`0x0C00` — CS0 size code 0xC =
16 MB, CS1 absent).
`FLASH_DEVINFO` 使用与真实 ROM 相同的两级间接：`FD` 数据表项指向 `0x7D04`
处的 32 位字面量，字面量再指向 `0x7D00` 处的 16 位信息字（`0x0C00` ——
CS0 容量码 0xC = 16 MB，无 CS1）。

---

## Trap framework / 陷阱框架

```rust
pub struct TrapId(pub u16);

/// Arguments captured from `r0`, `r1`, `r2`, `r3` when a trap fires
/// 陷阱触发时从 `r0`、`r1`、`r2`、`r3` 捕获的参数
pub struct TrapCall { pub id: TrapId, pub args: [u32; 4] }

/// Result: value written back to `r0`, approximate cycles, optional halt
/// 结果：写回 `r0` 的值、近似周期数、可选停机原因
pub struct TrapOutcome { pub result: u32, pub cycles: u32, pub halt: Option<HaltReason> }
impl TrapOutcome {
    pub const fn void(cycles: u32) -> Self;      // result = 0
    pub const fn value(result: u32, cycles: u32) -> Self;
}

pub enum HaltReason {
    /// The guest called `rom_reboot` / 客户机调用了 `rom_reboot`
    Reboot { flags: u32, delay_ms: u32, p0: u32, p1: u32 },
}

pub trait TrapHandler: Send + Sync {
    fn invoke(&self, call: TrapCall, bus: &mut dyn AddressBus, cycles: &mut u64)
        -> Result<TrapOutcome, CoreError>;
}

pub struct TrapDispatcher { /* private */ }
impl TrapDispatcher {
    pub fn new(handler: Arc<dyn TrapHandler>) -> Self;
    pub fn register(&mut self, pc: u32, id: TrapId);
    pub fn handles(&self, pc: u32) -> bool;
    pub fn dispatch(&self, core: &mut dyn CpuCore, bus: &mut dyn AddressBus)
        -> Result<Option<TrapOutcome>, CoreError>;
}
```

`dispatch` semantics: it reads `r0..r3` into the `TrapCall`, invokes the
handler, adds the handler's own bus cycles to `outcome.cycles`, writes
`outcome.result` back to `r0` and returns through `lr`
(`PC = lr & !1`). The interface depends only on `CpuCore` and `AddressBus`,
so the CLI, GDB server and GUI can share it.
`dispatch` 语义：把 `r0..r3` 读入 `TrapCall`，调用处理器，把处理器自身的总线
周期累加进 `outcome.cycles`，把 `outcome.result` 写回 `r0` 并经 `lr` 返回
（`PC = lr & !1`）。该接口只依赖 `CpuCore` 与 `AddressBus`，CLI、GDB 服务器
与 GUI 均可复用。

---

## The eight ROM functions / 八个 ROM 函数

Stub addresses are `0x80 + 2 * (id - 1)`; tags are two-character codes:
占位桩地址为 `0x80 + 2 * (id - 1)`；标签为两字符编码：

| Trap / 陷阱 | id | Tag / 标签 | Stub / 桩地址 | Behaviour / 行为 |
|-------------|----|-----------|---------------|------------------|
| `rom_connect_internal_flash` | 1 | `IF` | `0x80` | no-op, returns 0 (32 cycles) |
| `rom_flash_exit_xip` | 2 | `EX` | `0x82` | no-op, returns 0 (32 cycles) |
| `rom_flash_range_erase` | 3 | `RE` | `0x84` | erases flash — see below / 见下 |
| `rom_flash_range_program` | 4 | `RP` | `0x86` | programs flash — see below / 见下 |
| `rom_flash_flush_cache` | 5 | `FC` | `0x88` | no-op, returns 0 (16 cycles) |
| `rom_get_sys_info` | 6 | `GS` | `0x8A` | fills the buffer, see below / 填充缓冲区，见下 |
| `rom_reboot` | 7 | `RB` | `0x8C` | **halts** with `HaltReason::Reboot{flags, delay_ms, p0, p1}` |
| `rom_bootrom_state_reset` | 8 | `SR` | `0x8E` | no-op, returns 0 (8 cycles) |

Unknown trap ids return `BOOTROM_ERROR_INVALID_ARG` = `0xFFFF_FFEC`.
未知陷阱 id 返回 `BOOTROM_ERROR_INVALID_ARG` = `0xFFFF_FFEC`。

### Flash offsets are relative to flash start / Flash 偏移相对 Flash 起始

`flash_range_erase(offset, count, block_size)` and
`flash_range_program(offset, src_ptr, count)` take **flash offsets**, not XIP
addresses, exactly like the pico-sdk API: a caller passing `0x0010_0000`
means the second megabyte. The handler translates to the shared backing store
by adding `flash.base()` (the XIP base, `0x1000_0000`), so guest reads through
XIP see the programmed bytes.
`flash_range_erase(offset, count, block_size)` 与
`flash_range_program(offset, src_ptr, count)` 接受的是 **Flash 偏移**而非 XIP
地址，与 pico-sdk API 一致：调用者传 `0x0010_0000` 指第二个 MB。处理器加上
`flash.base()`（XIP 基地址 `0x1000_0000`）换算到共享底层存储，因此客户机经
XIP 能读到编程后的字节。

- `flash_range_erase`: `count` is rounded up to `block_size`
  (`block_size.max(1)`); cost is `max(count / 256, 64)` cycles.
  `flash_range_erase`：`count` 按 `block_size` 上取整（`block_size.max(1)`）；
  开销为 `max(count / 256, 64)` 周期。
- `flash_range_program`: copies `count` bytes byte-by-byte from guest memory
  at `src_ptr`; cost is `max(count / 16, 16)` cycles.
  `flash_range_program`：从客户机 `src_ptr` 逐字节复制 `count` 字节；
  开销为 `max(count / 16, 16)` 周期。

### `rom_get_sys_info(buffer, capacity, flags)` / 系统信息

`flags` is a mask of the supported `SYS_INFO_*` queries:
`flags` 是受支持的 `SYS_INFO_*` 查询的掩码：

| Bit / 位 | Query / 查询 | Words filled / 填充字 |
|---------|--------------|----------------------|
| 0x0001 | `SYS_INFO_CHIP_INFO` | 3 words, all 0 |
| 0x0002 | `SYS_INFO_CRITICAL` | 1 word, 0 |
| 0x0004 | `SYS_INFO_CPU_INFO` | `0x0000_0100` (Arm, type 0) |
| 0x0008 | `SYS_INFO_FLASH_DEV_INFO` | `0x0000_0C00` (CS0 = 16 MB) |
| 0x0010 | `SYS_INFO_BOOT_RANDOM` | 4 fixed words (`0x1234_5678 …`) |
| 0x0020 | `SYS_INFO_NONCE` | 2 words, 0 |
| 0x0040 | `SYS_INFO_BOOT_INFO` | 4 words (`BOOT_TYPE_NORMAL` = 0, then 0s) |

The first buffer word is always the answered-flags mask. If `capacity` is
smaller than the required word count, the function returns
`BOOTROM_ERROR_BUFFER_TOO_SMALL` = `0xFFFF_FFF3` without writing; otherwise
the return value is the number of words written.
缓冲区第一个字始终是已应答标志的掩码。若 `capacity` 小于所需字数，函数返回
`BOOTROM_ERROR_BUFFER_TOO_SMALL` = `0xFFFF_FFF3` 且不写入；否则返回写入的
字数。

### `rom_table_lookup(code, mask)` / 表查找

Reads `r0` = two-character `code`, `r1` = flag `mask`, and returns the
matching 16-bit table pointer (Thumb bit included) or `0` when not found.
读取 `r0` = 两字符 `code`、`r1` = 标志 `mask`，返回匹配的 16 位表指针（含
Thumb 位）；未找到返回 `0`。

---

## Image and region builders / 镜像与区域构建器

```rust
pub struct RomImageBuilder { /* private */ }
impl RomImageBuilder {
    pub fn new(size: usize) -> Self;             // zeroed image / 清零镜像
    pub fn write_u16(&mut self, offset: usize, value: u16);   // little-endian
    pub fn write_u32(&mut self, offset: usize, value: u32);   // little-endian
    pub fn write_slice(&mut self, offset: usize, bytes: &[u8]);
    /// Thumb `udf #id` stub: `0xDE00 | (id & 0xFF)`
    /// Thumb `udf #id` 占位桩：`0xDE00 | (id & 0xFF)`
    pub fn trap_stub(&mut self, offset: usize, id: u16);
    pub fn finish(self) -> Vec<u8>;
}

pub struct RomRegion { /* private: name, base, data: Arc<Vec<u8>> */ }
impl RomRegion {
    pub fn new(name: impl Into<String>, base: Address, data: Arc<Vec<u8>>) -> Self;
    pub fn bytes(&self) -> &[u8];
}
impl MemoryRegion for RomRegion { /* guest AND host writes -> AccessDenied */ }
```

`RomRegion` rejects both guest and host writes — a boot ROM never changes at
runtime, so even `load_u8` is refused.
`RomRegion` 同时拒绝客户机与宿主写入 —— boot ROM 在运行时不会改变，因此
`load_u8` 也会被拒绝。

---

## Usage Example / 使用示例

```rust
use rp2350_bootrom::{Rp2350Bootrom, ROM_BASE};
use rp2350_core::{FlashRegion, SimpleBus};
use rp2350_cpu::CpuCore;

let flash = FlashRegion::rp2350_xip();
let bootrom = Rp2350Bootrom::new(flash.handle());

let mut bus = SimpleBus::new();
bootrom.mount(&mut bus)?;                    // ROM window at 0x0000_0000
bus.register_region(Box::new(flash))?;       // XIP window shares the store

// In the execution loop:
if bootrom.handles(core.pc()) {
    if let Some(outcome) = bootrom.dispatch(&mut core, &mut bus)? {
        if let Some(halt) = outcome.halt {
            eprintln!("halted: {halt}");     // e.g. rom_reboot
        }
    }
}
```
