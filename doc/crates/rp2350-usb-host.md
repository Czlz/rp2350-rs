# rp2350-usb-host — Public API / 公共 API

> Host-side USB bridge driving the simulated
> [`rp2350_usb::Controller`](rp2350-usb.md), plus a MicroPython raw REPL
> driver layered above the CDC bulk endpoints. It mirrors `rp2040py`'s
> `RawReplRunner` layering: bridge → CDC channel → byte link → protocol.
> 驱动模拟 [`rp2350_usb::Controller`](rp2350-usb.md) 的主机侧 USB 桥，以及在
> CDC 批量端点之上的 MicroPython raw REPL 驱动。分层方式与 `rp2040py` 的
> `RawReplRunner` 一致：bridge → CDC 通道 → 字节链路 → 协议。
>
> No CPU or clock is required: the guest firmware is simulated by callers
> through the controller's `host_*` methods, so the whole stack is testable
> headlessly.
> 无需 CPU 或时钟：客户机固件由调用方通过控制器的 `host_*` 方法模拟，因此
> 整个栈都可无头测试。
>
> **Status**: implemented — enumeration state machine, CDC channel, raw REPL
> protocol, perfbench driver, descriptor parsing.
> **状态**: 已实现 — 枚举状态机、CDC 通道、raw REPL 协议、perfbench 驱动、
> 描述符解析。

---

## Module Structure / 模块结构

| Module | Description / 描述 |
|--------|-------------------|
| `bridge` | `UsbHostBridge` — enumeration state machine, `HostProgress` / `HostError` |
| `cdc` | `CdcChannel` (buffered bulk channel), `CdcLink<F>` (`ByteLink` adapter) |
| `repl` | `ByteLink` trait, `RawReplSession`, protocol constants, `ExecOutput` / `ReplError` |
| `perfbench` | `assemble_bench_script`, `run_script`, `BenchResult` / `BenchOutcome` / `BenchError` |
| `descriptor` | `parse_device` / `parse_config`, `DeviceInfo` / `ConfigInfo` / `EndpointInfo` |

---

## Enumeration bridge / 枚举桥

```rust
pub const DEFAULT_SYSCLK_HZ: u32 = 150_000_000;   // UsbHostBridge::DEFAULT_SYSCLK_HZ

/// CDC bulk endpoints discovered during enumeration / 枚举发现的 CDC 批量端点
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CdcEndpoints {
    pub in_address: u8,        // bulk IN address (bit 7 set) / 批量 IN 地址（bit 7）
    pub in_max_packet: u16,
    pub out_address: u8,       // bulk OUT address / 批量 OUT 地址
    pub out_max_packet: u16,
    pub interface: u8,         // CDC data interface number / CDC 数据接口号
}

/// Result of one poll / 单次 poll 的结果
pub enum HostProgress {
    /// Run the guest and call again no earlier than this cycle
    /// 运行客户机，在该周期之后再调用
    Wait(u64),
    /// Enumeration finished, endpoints ready / 枚举完成且端点就绪
    Configured(CdcEndpoints),
    /// Enumeration failed / 枚举失败
    Failed(HostError),
    /// The device disappeared / 设备已消失
    Detached,
}

pub enum HostError {
    Timeout { request: u8 },               // control phase deadline missed / 控制阶段超时
    Stall { request: u8 },                 // device stalled a required request
    Descriptor(DescriptorError),
    Protocol(&'static str),
}

pub struct UsbHostBridge { /* private */ }
impl UsbHostBridge {
    pub fn new(controller: Controller) -> Self;
    pub fn set_sysclk_hz(&mut self, hz: u32);      // timing conversion clock
    pub fn device_info(&self) -> Option<DeviceInfo>;
    pub fn config_info(&self) -> Option<&ConfigInfo>;
    pub fn address(&self) -> u8;                   // address assigned by SET_ADDRESS
    pub fn controller(&self) -> &Controller;
    /// Human-readable enumeration snapshot for diagnostics / 供诊断用的枚举快照
    pub fn progress_line(&self) -> String;
    /// Advance enumeration by at most one USB transaction
    /// 至多推进一次 USB 事务
    pub fn poll(&mut self, now: u64) -> HostProgress;
}
```

### Enumeration flow / 枚举流程

`poll(now)` is called from the host loop with the current simulated cycle
count. Stages are `Detached → Enumerating → Configured` (or `Failed`):
在主机循环中以当前模拟周期数调用 `poll(now)`。阶段为
`Detached → Enumerating → Configured`（或 `Failed`）：

1. **Detached**: waits until `controller.device_visible()` (firmware enabled
   MAIN_CTRL.CONTROLLER_EN and SIE_CTRL.PULLUP_EN), then injects
   `host_attach` + `host_bus_reset` and queues the first request.
   **Detached**：等待 `controller.device_visible()`（固件已使能
   MAIN_CTRL.CONTROLLER_EN 与 SIE_CTRL.PULLUP_EN），然后注入 `host_attach`
   + `host_bus_reset` 并把首个请求入队。
2. **Enumerating**: one control transfer at a time through the phases
   `Setup → Data → Status`, with simulated delays between tokens (20 µs
   after setup, 10 µs between data packets, 100 µs after a NAK). Request
   queue, in order: device header (8 B) → `SET_ADDRESS 1` → device descriptor
   (18 B) → config header (9 B) → full config descriptor (`wTotalLength`
   bytes) → string descriptors (`iManufacturer`/`iProduct`/`iSerialNumber`,
   optional) → `SET_CONFIGURATION 1` → CDC `SET_CONTROL_LINE_STATE 0x0003`
   (class request to interface 0, optional).
   **Enumerating**：一次一个控制传输，阶段为 `Setup → Data → Status`，令牌
   之间带模拟延时（Setup 后 20 µs，数据包间 10 µs，NAK 后 100 µs）。请求
   队列依次为：设备头（8 B）→ `SET_ADDRESS 1` → 设备描述符（18 B）→ 配置
   头（9 B）→ 完整配置描述符（`wTotalLength` 字节）→ 字符串描述符
   （`iManufacturer`/`iProduct`/`iSerialNumber`，可选）→
   `SET_CONFIGURATION 1` → CDC `SET_CONTROL_LINE_STATE 0x0003`（发往接口 0
   的类请求，可选）。
3. **Configured**: returns `Configured(CdcEndpoints)` every poll.
   **Configured**：每次 poll 都返回 `Configured(CdcEndpoints)`。

### Control-transfer termination / 控制传输的终止条件

An IN data stage ends on a **short packet or once `wLength` bytes have
arrived** — never on the device-side `BUF_CTRL_LAST` flag, which is a
hardware-internal buffer marker invisible to a real host. Earlier code
treated `BUF_CTRL_LAST` as the end condition, which made the first full
64-byte packet of a 75-byte configuration descriptor look complete: the host
moved to Status while the device was still awaiting another IN token, and
both sides deadlocked. OUT data stages end on `Ack.last` or when `wLength`
bytes have been sent.
IN 数据阶段以**短包或收满 `wLength`** 结束 —— 绝不使用设备侧的
`BUF_CTRL_LAST` 标志，它只是硬件内部的缓冲标记，真实主机根本看不到。早期
实现把它当作终止条件，导致 75 字节配置描述符的第一个 64 字节满包被判为
完成：主机进入 Status 阶段而固件仍在等下一个 IN 令牌，双方死锁。OUT 数据
阶段则以 `Ack.last` 或发送满 `wLength` 结束。

### Failure diagnostics / 失败诊断

Each request has a 25 ms deadline from its first token; on timeout or STALL
the bridge captures a transfer snapshot into `progress_line` (stage, phase,
request code, `w_value`/`w_length`, received/sent byte counts, queue depth),
so a stuck enumeration can tell whether the guest armed an endpoint at all.
Optional requests (strings, CDC line state) that fail are silently skipped.
每个请求自首个令牌起有 25 ms 期限；超时或 STALL 时桥会把传输现场记入
`progress_line`（阶段、传输阶段、请求码、`w_value`/`w_length`、已收/已发
字节数、队列深度），从而判断客户机是否布防过端点。可选请求（字符串、CDC
线状态）失败时会被静默跳过。

---

## CDC channel / CDC 通道

```rust
pub struct CdcChannel { /* private */ }
impl CdcChannel {
    pub fn new(controller: Controller, endpoints: CdcEndpoints) -> Self;
    pub fn endpoints(&self) -> CdcEndpoints;
    pub fn readable_len(&self) -> usize;    // bytes in the receive buffer
    pub fn writable_pending(&self) -> usize; // bytes queued for transmission

    /// Drain device-to-host packets until the endpoint NAKs; returns bytes
    /// appended. Stops after a short packet (shorter than `in_max_packet`).
    /// 取走设备到主机的包直到端点 NAK；返回新增字节数。收到短包（小于
    /// `in_max_packet`）时停止。
    pub fn service(&mut self) -> usize;

    pub fn read_byte(&mut self) -> Option<u8>;
    pub fn read(&mut self, out: &mut [u8]) -> usize;
    /// Queue bytes and push as many packets as the device accepts
    /// 入队字节并按设备可接受量推送
    pub fn write(&mut self, data: &[u8]) -> usize;
    /// Push queued bytes until a NAK or an empty queue; returns bytes accepted
    /// 推送队列字节直到 NAK 或队列空；返回设备已接收的字节数
    pub fn flush(&mut self) -> usize;
}
```

The channel pumps bulk endpoints only when asked, so callers stay in control
of simulation time. `service()` loops on `host_in` (endpoint number =
`in_address & 0x0F`); `flush()` chunks the queue by `out_max_packet` and
calls `host_out` (endpoint `out_address & 0x0F`), stopping at the first
non-`Ack`.
通道仅在调用时搬运批量端点，时序由调用方掌握。`service()` 循环调用
`host_in`（端点号 = `in_address & 0x0F`）；`flush()` 按 `out_max_packet`
分块调用 `host_out`（端点 `out_address & 0x0F`），遇到非 `Ack` 即停止。

### `CdcLink` — `ByteLink` adapter / `ByteLink` 适配器

```rust
pub struct CdcLink<F: FnMut(u64)> { /* private */ }
impl<F: FnMut(u64)> CdcLink<F> {
    pub fn new(channel: CdcChannel, run_cycles: F) -> Self;   // 150 MHz default
    pub fn set_sysclk_hz(&mut self, hz: u32);
    pub fn channel(&self) -> &CdcChannel;
    pub fn channel_mut(&mut self) -> &mut CdcChannel;
}
impl<F: FnMut(u64)> ByteLink for CdcLink<F> { /* see the trait below */ }
```

`send` queues into the unbounded `CdcChannel` queue (every byte accepted);
`advance(micros)` converts microseconds to cycles at the configured clock,
calls `run_cycles(cycles)`, then flushes and services the channel — so the
guest and the host bridge always share one timeline.
`send` 写入无上限的 `CdcChannel` 队列（全部字节被接受）；`advance(micros)`
按配置时钟把微秒换算为周期，调用 `run_cycles(cycles)`，随后刷新并泵送通道
—— 客户机与主机桥因此共享同一条时间线。

---

## Raw REPL protocol / raw REPL 协议

Protocol semantics follow `lib/utils/pyexec.c` and `tools/pyboard.py`:
协议语义取自 `lib/utils/pyexec.c` 与 `tools/pyboard.py`：

```rust
pub const BANNER: &[u8] = b"raw REPL; CTRL-B to exit\r\n>";  // full banner + prompt
pub const BANNER_LINE: &[u8] = b"raw REPL; CTRL-B to exit\r\n";
pub const SOFT_REBOOT: &[u8] = b"soft reboot\r\n";
pub const CTRL_A: u8 = 0x01;   // enter raw REPL / 进入 raw REPL
pub const CTRL_B: u8 = 0x02;   // leave raw REPL / 退出 raw REPL
pub const CTRL_C: u8 = 0x03;   // interrupt / 中断
pub const CTRL_D: u8 = 0x04;   // execute or soft reset / 执行或软复位
pub const DEFAULT_TIMEOUT_MICROS: u64 = 10_000_000;
pub const DEFAULT_STEP_MICROS: u64 = 100;

pub trait ByteLink {
    fn send(&mut self, bytes: &[u8]) -> usize;     // accepted byte count
    fn receive(&mut self, out: &mut [u8]) -> usize;
    fn advance(&mut self, micros: u64);            // let the device run
    fn now_micros(&self) -> u64;
}

pub struct ExecOutput { pub stdout: Vec<u8>, pub stderr: Vec<u8> }
impl ExecOutput { pub fn is_success(&self) -> bool; }   // stderr is empty

pub enum ReplError {
    Timeout { marker: &'static str },   // marker not seen before the deadline
    Protocol { reason: &'static str },
}

pub struct RawReplSession<L: ByteLink> { /* private */ }
impl<L: ByteLink> RawReplSession<L> {
    pub fn new(link: L) -> Self;
    pub fn with_timeout(link: L, timeout_micros: u64) -> Self;
    pub fn link(&self) -> &L;
    pub fn link_mut(&mut self) -> &mut L;
    pub fn set_step_micros(&mut self, micros: u64);

    /// `\r\x03` interrupt / 中断
    pub fn interrupt(&mut self) -> Result<(), ReplError>;
    /// `\r\x01`, wait for the banner, optionally Ctrl-D soft reset
    /// `\r\x01`，等待 banner，可选 Ctrl-D 软复位
    pub fn enter_raw_repl(&mut self, soft_reset: bool) -> Result<(), ReplError>;
    /// Send source + `\x04`, read `OK`, stdout and stderr up to `\x04`
    /// 发送源码 + `\x04`，读取 `OK`、以 `\x04` 结尾的 stdout 与 stderr
    pub fn exec_raw(&mut self, source: &[u8]) -> Result<ExecOutput, ReplError>;
    /// `\r\x02` return to the friendly REPL / 返回友好 REPL
    pub fn exit_raw_repl(&mut self) -> Result<(), ReplError>;
    /// Ctrl-D soft reset through the raw REPL / 通过 raw REPL 软复位
    pub fn soft_reset(&mut self) -> Result<(), ReplError>;
}
```

"Blocking" means bounded waiting in **simulated time**: every retry loop
advances the link by `step_micros` (default 100 µs) and checks a simulated
deadline (default 10 s), so sessions never sleep on the host and stay
deterministic. `enter_raw_repl(true)` mirrors `pyboard.py`'s default flow:
banner → Ctrl-D → `soft reboot\r\n` → banner again.
“阻塞”指在**模拟时间**上的有界等待：每个重试循环按 `step_micros`（默认
100 µs）推进链路并检查模拟期限（默认 10 s），因此会话不会让主机真正睡眠，
且保持确定性。`enter_raw_repl(true)` 与 `pyboard.py` 默认流程一致：
banner → Ctrl-D → `soft reboot\r\n` → 再次 banner。

---

## perfbench driver / 基准驱动

Mirrors `tests/run-perfbench.py`: concatenates the benchmark file with
`tests/perf_bench/benchrun.py`, appends `bm_run(N, M)` and expects the device
to print one `time norm result` line, where `time` is in `time.ticks_us()`
units and `norm` is time divided by `N * M`.
复现 `tests/run-perfbench.py`：把基准文件与 `tests/perf_bench/benchrun.py`
拼接，追加 `bm_run(N, M)`，并期望设备打印一行 `time norm result`；`time`
以 `time.ticks_us()` 为单位，`norm` 为时间除以 `N * M`。

```rust
pub fn assemble_bench_script(benchmark: &[u8], benchrun: &[u8], n: u32, m: u32) -> Vec<u8>;
// = "import sys\nsys.path.remove('')\n\n" + benchmark + benchrun + "bm_run(N, M)\n"

pub fn run_script<L: ByteLink>(session: &mut RawReplSession<L>, script: &[u8])
    -> Result<BenchOutcome, BenchError>;

pub struct BenchResult { pub ticks_us: i64, pub norm: i64, pub result: String }
pub enum BenchOutcome { Skip, Measured(BenchResult) }
pub enum BenchError {
    Repl(ReplError),      // protocol failure / 协议失败
    Fatal(String),        // non-empty stderr (device exception) / stderr 非空
    BadOutput(String),    // stdout not matching the contract / 输出不符约定
}
```

`run_script` executes the script through `exec_raw`, fails fast on non-empty
stderr, treats exact output `SKIP` as `BenchOutcome::Skip`, and otherwise
parses the whitespace-separated `time norm result…` line (the result field
keeps text values such as `None`).
`run_script` 经 `exec_raw` 执行脚本，stderr 非空立即失败；输出恰为 `SKIP`
时返回 `BenchOutcome::Skip`；否则解析空白分隔的 `time norm result…` 行
（result 字段保留文本值，如 `None`）。

---

## Descriptor parsing / 描述符解析

```rust
pub struct EndpointInfo {
    pub address: u8,           // includes the direction bit / 含方向位
    pub max_packet_size: u16,  // wMaxPacketSize, masked to 0x07FF / 掩码 0x07FF
    pub bulk: bool,            // bmAttributes[1:0] == 2 / 为批量传输
}
impl EndpointInfo { pub const fn number(&self) -> u8; pub const fn is_in(&self) -> bool; }

pub struct DeviceInfo {
    pub vendor_id: u16, pub product_id: u16, pub device_version: u16,
    pub max_packet_size0: u8, pub configurations: u8,
    pub manufacturer: u8, pub product: u8, pub serial: u8,
}

pub struct ConfigInfo {
    pub total_length: u16, pub value: u8,
    pub cdc_in: Option<EndpointInfo>,        // bulk IN on the data interface
    pub cdc_out: Option<EndpointInfo>,       // bulk OUT on the data interface
    pub comm_interface: Option<u8>,          // CDC control interface (class 0x02)
    pub data_interface: Option<u8>,          // CDC data interface (class 0x0A)
}
impl ConfigInfo {
    pub fn require_cdc_in(&self) -> Result<EndpointInfo, DescriptorError>;
    pub fn require_cdc_out(&self) -> Result<EndpointInfo, DescriptorError>;
}

pub enum DescriptorError {
    Truncated, UnexpectedType, MissingCdcDataInterface, MissingCdcEndpoint,
}

pub fn parse_device(bytes: &[u8]) -> Result<DeviceInfo, DescriptorError>;  // 18 bytes
pub fn parse_config(bytes: &[u8]) -> Result<ConfigInfo, DescriptorError>;  // full config
```

`parse_config` walks the descriptor chain, records CDC class 0x02 (control)
and 0x0A (data) interfaces, and claims the first bulk IN/OUT endpoints that
follow while the current interface is the data class. A configuration
without a data interface or without both bulk endpoints is rejected.
`parse_config` 遍历描述符链，记录 CDC 类 0x02（控制）与 0x0A（数据）接口，
并在当前接口为数据类时认领紧随其后的首个批量 IN/OUT 端点。缺少数据接口或
缺少任一批量端点的配置会被拒绝。

---

## Usage Example / 使用示例

```rust
use rp2350_usb::Controller;
use rp2350_usb_host::{CdcChannel, CdcLink, RawReplSession, UsbHostBridge};

let controller = Controller::new();          // shared with the bus side
let mut bridge = UsbHostBridge::new(controller.clone());

// In the host loop (now = current simulated cycles):
let endpoints = loop {
    match bridge.poll(now) {
        rp2350_usb_host::HostProgress::Configured(endpoints) => break endpoints,
        rp2350_usb_host::HostProgress::Wait(at) => now = at,
        _ => panic!("enumeration failed: {}", bridge.progress_line()),
    }
};

let channel = CdcChannel::new(controller, endpoints);
let mut link = CdcLink::new(channel, |cycles| machine.run_cycles(cycles));
let mut session = RawReplSession::new(link);
session.enter_raw_repl(true)?;
let output = session.exec_raw(b"print('hello')")?;
assert_eq!(output.stdout, b"hello\r\n");
```
