# rp2350-usb — Public API / 公共 API

> RP2350 USB full-speed **device-side** controller model. Covers the subset
> that MicroPython's TinyUSB port (`dcd_rp2040`) actually touches: the 4 KB
> DPSRAM endpoint buffer block, the control register block with its atomic
> aliases, endpoint buffer handshakes and device-mode interrupt aggregation.
> RP2350 全速 USB **设备侧**控制器模型。覆盖 MicroPython TinyUSB 端口
> （`dcd_rp2040`）实际使用的子集：4 KB DPSRAM 端点缓冲块、含原子别名的控制
> 寄存器块、端点缓冲握手与设备模式中断聚合。
>
> Design background: `doc/usb-device-model.md` records why the controller is
> modelled this way (host-driven transactions instead of a full SIE).
> 设计背景：`doc/usb-device-model.md` 记录了为什么这样建模控制器
> （主机驱动的事务，而不是完整的 SIE）。
>
> **Status**: implemented — device register model + host-side transaction
> injection. No SIE protocol, no host-mode controller, no DMA.
> **状态**: 已实现 — 设备寄存器模型 + 主机侧事务注入。无 SIE 协议、
> 无主机模式控制器、无 DMA。

---

## Module Structure / 模块结构

| Module | Description / 描述 |
|--------|-------------------|
| `lib` | `mount()`, address constants, `USBCTRL_IRQ` |
| `device` | `UsbCore` — pure register/DPRAM state machine (no locking) |
| `regs` | Register offsets, bit definitions, `Alias`, `Direction` |
| `endpoint` | `host_in` / `host_out` / `host_setup`, `InPacket` / `InResponse` / `OutResponse` |
| `setup` | `SetupPacket`, standard/CDC request and descriptor constants |
| `regions` | `DpramRegion` / `RegsRegion` / `UsbPeripheral` — `MemoryRegion` views |
| `mmio` | `DpramMmio` / `RegsMmio` / `mount_usb` / `decode_alias` — mainline `MmioDevice` adapters |
| `controller` | `Controller` — shared-ownership facade, `Peripheral` impl |

---

## Address map / 地址映射

```rust
pub const DPRAM_BASE: Address = 0x5010_0000;   // DPSRAM, 4 KB / DPSRAM，4 KB
pub const DPRAM_SIZE: usize = 0x1000;
pub const REGS_BASE: Address = 0x5011_0000;    // control registers / 控制寄存器
pub const REGS_BLOCK_SIZE: usize = 0x1000;     // RW shadow / RW 影子
pub const REGS_REGION_SIZE: usize = 0x4000;    // RW/XOR/SET/CLR aliases / 别名窗口
pub const USBCTRL_IRQ: u16 = 14;               // interrupt number / 中断号

pub fn mount(bus: &mut dyn AddressBus) -> BusResult<Controller>;
```

`mount` is the CLI-facing entry point: it creates a `Controller` and registers
both windows through the mainline `mount_device` API. After mounting, every
register offset is readable (unmodelled bits read as zero), so firmware
read-modify-write cycles such as `0x5011_004C` (`SIE_CTRL`) no longer produce
a `NoDevice` bus error.
`mount` 是面向 CLI 的入口：创建 `Controller` 并通过主线 `mount_device` API
注册两个窗口。挂载后所有寄存器偏移均可读（未建模位读作零），因此固件对
`0x5011_004C`（`SIE_CTRL`）等寄存器的读-改-写不再产生 `NoDevice` 总线错误。

---

## `UsbCore` — the state machine / 状态机

`UsbCore` owns the 4 KB DPSRAM and the register shadow. Guest MMIO accesses go
through `read_reg` / `write_reg`; host-side transactions go through the
`host_*` methods. No locking is involved, so it can be unit tested without a
CPU or clock.
`UsbCore` 持有 4 KB DPSRAM 与寄存器影子。客户机 MMIO 访问经由
`read_reg` / `write_reg`；主机侧事务经由 `host_*` 方法。核心不含锁，因此
无需 CPU 或时钟即可单测。

```rust
pub struct UsbCore { /* private */ }
impl UsbCore {
    pub fn new() -> Self;
    pub fn reset(&mut self);

    // Register access / 寄存器访问
    pub fn read_reg(&mut self, offset: usize) -> u32;          // applies read side effects
    pub fn write_reg(&mut self, offset: usize, alias: Alias, value: u32);
    pub fn write_reg_masked(&mut self, offset: usize, alias: Alias, value: u32, mask: u32);
    pub fn intr(&self) -> u32;                                // raw interrupt status
    pub fn ints(&self) -> u32;                                // (INTR & INTE) | INTF
    pub fn irq_asserted(&self) -> bool;                       // INTS != 0

    // Status views / 状态视图
    pub fn address(&self) -> u8;                              // ADDR_ENDP[6:0]
    pub fn pullup_enabled(&self) -> bool;                     // SIE_CTRL.PULLUP_EN
    pub fn controller_enabled(&self) -> bool;                 // MAIN_CTRL.CONTROLLER_EN
    pub fn device_visible(&self) -> bool;                     // pull-up && controller
    pub fn transaction_ready(&self) -> bool;                  // CONNECTED && enabled

    // DPSRAM access (u8 / u16 / u32, little-endian) / DPSRAM 访问
    pub fn dpram(&self) -> &[u8];
    pub fn read_dpram_u8(&self, offset: usize) -> u8;
    pub fn write_dpram_u8(&mut self, offset: usize, value: u8);
    pub fn read_dpram_u16(&self, offset: usize) -> u16;
    pub fn write_dpram_u16(&mut self, offset: usize, value: u16);
    pub fn read_dpram_u32(&self, offset: usize) -> u32;
    pub fn write_dpram_u32(&mut self, offset: usize, value: u32);

    // Layout constants used by host bridges / 主机桥使用的布局常量
    pub const fn ep0_buffer() -> usize;         // 0x100 / 0x100
    pub const fn ep0_buffer_b() -> usize;       // EP0 buffer B / EP0 缓冲 B
    pub const fn ep_buf_ctrl_base() -> usize;   // 0x080 / 0x080
    pub const fn ep_ctrl_base() -> usize;       // 0x008 / 0x008
    pub const fn setup_packet_offset() -> usize; // 0x000 / 0x000

    // Bus events injected by the host / 主机注入的总线事件
    pub fn host_attach(&mut self);
    pub fn host_detach(&mut self);
    pub fn host_bus_reset(&mut self);
    pub fn host_sof(&mut self, frame: u16);
    pub fn next_half(&self, ep: usize, dir: Direction) -> bool;
}
```

### Register semantics / 寄存器语义

- `read_reg` applies read side effects: `INTR` and `INTS` are computed from
  `SIE_STATUS`, `BUFF_STATUS` and pending events rather than stored;
  reading `SOF_RD` clears the device-SOF pending flag; `SIE_STATUS` is
  returned raw.
  `read_reg` 带读副作用：`INTR` 与 `INTS` 由 `SIE_STATUS`、`BUFF_STATUS` 与
  挂起事件实时计算而非存储；读 `SOF_RD` 会清除设备 SOF 挂起标志；
  `SIE_STATUS` 原样返回。
- `write_reg` / `write_reg_masked` honour the selected `Alias`
  (`Rw` / `Xor` / `Set` / `Clear`). Several registers are write-one-to-clear:
  `BUFF_STATUS`, `EP_ABORT_DONE`, `EP_STATUS_STALL_NAK` and the `W1C_MASK`
  bits of `SIE_STATUS`. `INTR`/`INTS` ignore writes; `SOF_WR` keeps 11 bits;
  a nonzero `EP_ABORT` write aborts the selected endpoints immediately
  (clears their buffer-control halves, raises `EP_ABORT_DONE`).
  `write_reg` / `write_reg_masked` 遵循所选 `Alias`（`Rw`/`Xor`/`Set`/`Clear`）。
  多个寄存器为写 1 清零：`BUFF_STATUS`、`EP_ABORT_DONE`、
  `EP_STATUS_STALL_NAK` 以及 `SIE_STATUS` 的 `W1C_MASK` 位。`INTR`/`INTS`
  忽略写入；`SOF_WR` 保留 11 位；非零的 `EP_ABORT` 写会立即中止所选端点
  （清其缓冲控制半字并置位 `EP_ABORT_DONE`）。
- `IRQ aggregation`: `INTR` folds `BUFF_STATUS`/`EP_ABORT_DONE`/
  `EP_STATUS_STALL_NAK` bits and the `SIE_STATUS` conditions (BUS_RESET,
  SETUP_REC, TRANS_COMPLETE, VBUS_DETECTED, SUSPENDED, RESUME) plus the
  device-SOF and connect/disconnect pending flags; `INTS = (INTR & INTE) |
  INTF` and `irq_asserted` is simply `INTS != 0` — the host asserts
  `USBCTRL_IRQ` while this holds, and the condition disappears when firmware
  clears `INTS`.
  中断聚合：`INTR` 折叠 `BUFF_STATUS`/`EP_ABORT_DONE`/`EP_STATUS_STALL_NAK`
  位与 `SIE_STATUS` 条件（BUS_RESET、SETUP_REC、TRANS_COMPLETE、
  VBUS_DETECTED、SUSPENDED、RESUME），以及设备 SOF 与连接/断开挂起标志；
  `INTS = (INTR & INTE) | INTF`，`irq_asserted` 即 `INTS != 0` —— 满足时
  主机拉高 `USBCTRL_IRQ`，固件清 `INTS` 后条件自然撤销。

Register offsets are the RP2350 `USBCTRL_REGS` layout (note the interrupt
block differs from RP2040): `ADDR_ENDP 0x000`, `MAIN_CTRL 0x040`, `SOF_WR
0x044`, `SOF_RD 0x048`, `SIE_CTRL 0x04C`, `SIE_STATUS 0x050`, `BUFF_STATUS
0x058`, `EP_ABORT 0x060`, `EP_ABORT_DONE 0x064`, `INTR 0x08C`, `INTE 0x090`,
`INTF 0x094`, `INTS 0x098`, etc. — see the `regs` module for the full list.
寄存器偏移为 RP2350 的 `USBCTRL_REGS` 布局（注意中断块偏移与 RP2040 不同）：
`ADDR_ENDP 0x000`、`MAIN_CTRL 0x040`、`SOF_WR 0x044`、`SOF_RD 0x048`、
`SIE_CTRL 0x04C`、`SIE_STATUS 0x050`、`BUFF_STATUS 0x058`、
`EP_ABORT 0x060`、`EP_ABORT_DONE 0x064`、`INTR 0x08C`、`INTE 0x090`、
`INTF 0x094`、`INTS 0x098` 等 —— 完整列表见 `regs` 模块。

DPSRAM layout: SETUP packet at `0x000` (8 bytes), endpoint control registers
from `0x008`, endpoint buffer-control registers from `0x080`, EP0 buffer A at
`0x100` (B available when double-buffered), and general endpoint buffers at
guest-configured offsets.
DPSRAM 布局：SETUP 包在 `0x000`（8 字节）、端点控制寄存器自 `0x008`、端点
缓冲控制寄存器自 `0x080`、EP0 缓冲 A 在 `0x100`（双缓冲时可用 B），其余
端点缓冲位于客户机配置的偏移处。

---

## Host-side transactions / 主机侧事务

These methods simulate the host side of a transaction; an external host
bridge (see `rp2350-usb-host`) drives them. They require the device to be
"transaction ready" (`SIE_STATUS.CONNECTED` && controller enabled); otherwise
they answer `NoDevice` / `false`.
以下方法模拟事务的主机侧；外部主机桥（见 `rp2350-usb-host`）驱动它们。它们
要求设备处于“可事务”状态（`SIE_STATUS.CONNECTED` 且控制器已使能），否则
应答 `NoDevice` / `false`。

```rust
pub struct InPacket { /* private: data[64], len */ }
impl InPacket {
    pub fn bytes(&self) -> &[u8];   // captured payload / 捕获的负载
    pub const fn len(&self) -> usize;
    pub const fn is_empty(&self) -> bool;
    pub last: bool;                 // device marked the buffer as transfer-last
                                    // 设备将该缓冲标记为传输最后一个
}

pub enum InResponse { NoDevice, Nak, Stall, Data(InPacket) }
pub enum OutResponse { NoDevice, Nak, Stall, Ack { last: bool } }

impl UsbCore {
    /// IN token: consumes the next armed double-buffer half, clears
    /// `AVAIL|FULL`, raises `BUFF_STATUS`, returns the payload.
    /// IN 令牌：消费双缓冲轮转中下一个已布防半，清除 `AVAIL|FULL`，置位
    /// `BUFF_STATUS` 并返回负载。
    pub fn host_in(&mut self, ep: usize) -> InResponse;

    /// OUT token: copies up to 64 bytes into the armed buffer, marks it full,
    /// raises `BUFF_STATUS`. A short packet (or armed `LAST`) ends the transfer.
    /// OUT 令牌：把最多 64 字节复制进已布防缓冲并标记为满，置位
    /// `BUFF_STATUS`。短包（或已布防 `LAST`）结束本次传输。
    pub fn host_out(&mut self, ep: usize, data: &[u8]) -> OutResponse;

    /// SETUP packet: writes 8 bytes to DPSRAM `0x000`, aborts pending EP0
    /// buffers, raises `SETUP_REC`. Returns `false` when not attached.
    /// SETUP 包：把 8 字节写入 DPSRAM `0x000`，中止未完成的 EP0 缓冲并置位
    /// `SETUP_REC`。设备未附着时返回 `false`。
    pub fn host_setup(&mut self, packet: &SetupPacket) -> bool;

    pub fn setup_packet(&self) -> SetupPacket;   // read back from DPSRAM
    pub fn acknowledge_setup(&mut self);         // clear SIE_STATUS.SETUP_REC
}
```

Priority order in each transaction: STALL wins over data (checked across both
buffer halves), then the armed half is picked in double-buffer rotation order
(`next_half`), then `Nak` when nothing is armed or the buffer offset is
unconfigured. `host_out` also treats a packet shorter than 64 bytes as the
end of the transfer (`Ack { last: true }`).
每个事务的优先顺序：先查 STALL（跨两个缓冲半检查，优先于数据），再按双缓冲
轮转顺序（`next_half`）选择已布防半；未布防或缓冲偏移未配置时返回 `Nak`。
`host_out` 还会把短于 64 字节的包视为传输结束（`Ack { last: true }`）。

The host-side bus events mirror the hardware conditions a device would see:
`host_attach` raises `VBUS_DETECTED | CONNECTED`; `host_detach` clears
`SIE_STATUS` and latches a connect/disconnect interrupt (only after an
earlier attach); `host_bus_reset` clears `ADDR_ENDP`, all endpoint
control/buffer-control registers and the rotation state, then raises
`SIE_STATUS.BUS_RESET`; `host_sof` stores the 11-bit frame and sets the
device-SOF pending flag.
主机侧总线事件复现硬件上设备可见的条件：`host_attach` 置
`VBUS_DETECTED | CONNECTED`；`host_detach` 清 `SIE_STATUS` 并锁存一次
连接/断开中断（仅在之前发生附着后）；`host_bus_reset` 清除 `ADDR_ENDP`、
全部端点控制/缓冲控制寄存器与轮转状态，然后置位 `SIE_STATUS.BUS_RESET`；
`host_sof` 保存 11 位帧号并置设备 SOF 挂起标志。

---

## `SetupPacket` / SETUP 包

```rust
pub struct SetupPacket {
    pub bm_request_type: u8,
    pub b_request: u8,
    pub w_value: u16,
    pub w_index: u16,
    pub w_length: u16,
}
impl SetupPacket {
    pub const fn from_bytes(bytes: &[u8; 8]) -> Self;   // little-endian
    pub const fn to_bytes(self) -> [u8; 8];
    pub const fn direction(self) -> Direction;          // bmRequestType.bit7
    pub const fn request_type(self) -> u8;              // bmRequestType[6:5]
    pub const fn is_standard(self) -> bool;
    pub const fn is_class(self) -> bool;
    pub const fn recipient(self) -> u8;                 // bmRequestType[4:0]
    pub const fn standard_in(request: u8, value: u16, index: u16, length: u16) -> Self;
    pub const fn standard_out(request: u8, value: u16, index: u16, length: u16) -> Self;
    pub const fn class_out(request: u8, value: u16, index: u16, length: u16) -> Self;
}

pub mod request { /* GET_STATUS 0x00, CLEAR_FEATURE 0x01, SET_FEATURE 0x03,
                     SET_ADDRESS 0x05, GET_DESCRIPTOR 0x06, SET_DESCRIPTOR 0x07,
                     GET_CONFIGURATION 0x08, SET_CONFIGURATION 0x09,
                     GET_INTERFACE 0x0A, SET_INTERFACE 0x0B,
                     CDC_SET_LINE_CODING 0x20, CDC_GET_LINE_CODING 0x21,
                     CDC_SET_CONTROL_LINE_STATE 0x22, CDC_SEND_BREAK 0x23 */ }
pub mod descriptor { /* DEVICE 0x01, CONFIGURATION 0x02, STRING 0x03,
                       INTERFACE 0x04, ENDPOINT 0x05 */ }
```

---

## Bus integration / 总线集成

Two layers of adapters share one `UsbCore` through
`Arc<Mutex<UsbCore>>` (parking-lot mutex):
两层适配器通过 `Arc<Mutex<UsbCore>>`（parking-lot 互斥锁）共享同一个
`UsbCore`：

```rust
// MemoryRegion views (bus-facing) / MemoryRegion 视图（面向总线）
pub struct DpramRegion { /* private */ }     // 4 KB DPSRAM at 0x5010_0000
pub struct RegsRegion { /* private */ }      // 16 KB register window at 0x5011_0000
pub struct UsbPeripheral { /* private */ }   // combined DPRAM + registers
// name() returns "USBCTRL"; size() spans DPRAM + the alias window.

// Mainline MmioDevice adapters / 主线 MmioDevice 适配器
pub struct DpramMmio { /* private */ }       // MmioWindow(DPRAM_BASE, 4 KB)
pub struct RegsMmio { /* private */ }        // MmioWindow(REGS_BASE, 16 KB)
pub fn decode_alias(offset: u32) -> Option<(usize, Alias)>;
pub fn mount_usb(bus: &mut dyn AddressBus, core: Arc<Mutex<UsbCore>>) -> BusResult<()>;
```

`decode_alias` maps a register-window offset onto `offset & 0xFFF` plus the
alias selected by `offset >> 12`: `0x0000..0x0FFF` RW, `0x1000..0x1FFF` XOR,
`0x2000..0x2FFF` SET, `0x3000..0x3FFF` CLR. Alias writes fold into one
write on the underlying register; alias reads return the plain value.
`decode_alias` 把寄存器窗口偏移映射为 `offset & 0xFFF` 加上由 `offset >> 12`
选择的别名：`0x0000..0x0FFF` 为 RW、`0x1000..0x1FFF` 为 XOR、
`0x2000..0x2FFF` 为 SET、`0x3000..0x3FFF` 为 CLR。别名写折叠为对底层寄存器
的一次写入；别名读返回普通值。

Byte and halfword accesses through the `MemoryRegion`/`MmioDevice` wrappers
merge into the containing word, so guest `strb`/`strh` sequences behave
consistently.
经由 `MemoryRegion`/`MmioDevice` 包装器的字节与半字访问会合并到所在字，
客户机的 `strb`/`strh` 序列因此行为一致。

---

## `Controller` / 共享控制器句柄

```rust
#[derive(Clone)]
pub struct Controller { /* private: core: Arc<Mutex<UsbCore>> */ }
impl Controller {
    pub fn new() -> Self;
    pub fn shared_core(&self) -> Arc<Mutex<UsbCore>>;
    pub fn with_core<R>(&self, f: impl FnOnce(&mut UsbCore) -> R) -> R;

    // Region factories / 区域工厂
    pub fn dpram_region(&self) -> DpramRegion;
    pub fn regs_region(&self) -> RegsRegion;
    pub fn peripheral_region(&self) -> UsbPeripheral;
    pub fn mmio_dpram(&self) -> DpramMmio;
    pub fn mmio_regs(&self) -> RegsMmio;
    pub fn mount_mmio(&self, bus: &mut dyn AddressBus) -> BusResult<()>;

    // Diagnostics / 诊断
    pub fn irq_asserted(&self) -> bool;
    pub fn device_visible(&self) -> bool;
    pub fn reset(&self);

    // Host-side passthrough / 主机侧直通
    pub fn host_attach(&self);
    pub fn host_detach(&self);
    pub fn host_bus_reset(&self);
    pub fn host_sof(&self, frame: u16);
    pub fn host_setup(&self, packet: &SetupPacket) -> bool;
    pub fn host_in(&self, ep: usize) -> InResponse;
    pub fn host_out(&self, ep: usize, data: &[u8]) -> OutResponse;
}

impl Peripheral for Controller { /* name "USBCTRL", irq_number = Some(USBCTRL_IRQ) */ }
impl MemoryRegion for Controller { /* delegates to the combined region */ }
```

Because `Controller` is `Clone`, the bus side, a host bridge and diagnostics
can all hold handles to the same core. `rp2350-cli` keeps one controller
alive for the whole machine lifetime and drives it from the USB host bridge
during `repl` / `perfbench`.
`Controller` 可 `Clone`，因此总线侧、主机桥与诊断都能持有同一核心的句柄。
`rp2350-cli` 让一个控制器存活整个机器生命周期，并在 `repl`/`perfbench`
期间由 USB 主机桥驱动。

---

## Not implemented / 未实现清单

- **SIE protocol / host-mode controller**: the device never synthesizes
  tokens itself; every transaction is host-injected via `host_*`.
  **SIE 协议 / 主机模式控制器**：设备从不自行合成令牌；每个事务都由主机经
  `host_*` 注入。
- **DMA / chained transfers**: not modelled.
  **DMA / 链式传输**：未建模。
- **FS/HS, isochronous timing, frame-accurate scheduling**: the model is
  event-based; SOF only latches the frame number.
  **FS/HS、等时传输时序、帧级调度**：模型基于事件；SOF 只锁存帧号。
- **PHY, USBPHY_DIRECT/USBPHY_TRIM registers**: storage only via the generic
  register path.
  **PHY、USBPHY_DIRECT/USBPHY_TRIM 寄存器**：仅经通用寄存器路径存储。

---

## Usage Example / 使用示例

```rust
use rp2350_core::SimpleBus;
use rp2350_usb::{Controller, SetupPacket, USBCTRL_IRQ, regs};

let mut bus = SimpleBus::new();
let controller = rp2350_usb::mount(&mut bus)?;

// Firmware enables the controller and the pull-up through the bus;
// the host bridge only injects bus events and transactions.
controller.with_core(|core| {
    core.write_reg(regs::MAIN_CTRL, regs::Alias::Rw, regs::MAIN_CTRL_CONTROLLER_EN);
    core.write_reg(regs::SIE_CTRL, regs::Alias::Rw, regs::SIE_CTRL_PULLUP_EN);
});
assert!(controller.device_visible());

controller.host_attach();
controller.host_bus_reset();
let setup = SetupPacket::standard_in(0x06, 0x0100, 0, 18);   // GET_DESCRIPTOR
assert!(controller.host_setup(&setup));
assert!(controller.irq_asserted());                           // SETUP_REQ raised
```
