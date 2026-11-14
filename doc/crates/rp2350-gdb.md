# rp2350-gdb — Public API / 公共 API

> GDB Remote Serial Protocol (RSP) protocol layer for RP2350 debugging.
> 用于 RP2350 调试的 GDB 远程串行协议（RSP）协议层。
>
> **Status**: 协议层、TCP 服务与 CLI 接线均已实现 — `rsp.rs` 提供包编解码（`$<payload>#<checksum>`、
> `+`/`-` 应答、`}` 二进制转义、十六进制工具）与无 socket 的命令分发器
> `Session<T>`；`CortexM33Target<B>` 与 CLI 的 `MachineTarget` 为两种目标适配器。
> `GdbServer`（`server.rs`）提供阻塞式 TCP 监听、顺序接受客户端与字节级 RSP 帧循环；
> `rp2350 gdb --file <ELF> --port <N>` 直接启动服务。
>
> **Status** (EN): the protocol layer, the TCP server and the CLI wiring are
> implemented — `rsp.rs` provides packet framing (`$<payload>#<checksum>`),
> `+`/`-` acknowledgements, `}` binary escaping, hex helpers, and a socket-free
> dispatcher `Session<T>`. `CortexM33Target<B>` and the CLI `MachineTarget` are
> the two target adapters. `GdbServer` (`server.rs`) provides a blocking TCP
> listener, sequential client accept, and the byte-level RSP frame loop;
> `rp2350 gdb --file <ELF> --port <N>` starts the server directly.

---

## Key Types / 关键类型

### `GdbTarget` Trait

```rust
pub trait GdbTarget: Send {
    fn register_count(&self) -> u32;               // default: 17 (R0-R15 + XPSR)
    fn read_register(&mut self, register: u32) -> Result<u32, GdbError>;
    fn write_register(&mut self, register: u32, value: u32) -> Result<(), GdbError>;
    fn read_memory(&mut self, address: u32, length: usize) -> Result<Vec<u8>, GdbError>;
    fn write_memory(&mut self, address: u32, bytes: &[u8]) -> Result<(), GdbError>;
    fn set_breakpoint(&mut self, address: u32) -> Result<(), GdbError>;
    fn clear_breakpoint(&mut self, address: u32) -> Result<(), GdbError>;
    fn has_breakpoint(&self, address: u32) -> bool;
    fn supports_watchpoints(&self) -> bool;                    // default: false
    fn set_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError>;
    fn clear_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError>;
    fn watchpoint_hit(&self) -> Option<u32>;                   // default: None
    fn resume(&mut self, mode: ResumeMode) -> Result<StopReply, GdbError>;
    fn status(&self) -> TargetStatus;
}
```

支持类型 / Supporting types：

- `ResumeMode`：`Continue` / `Step`
- `StopReply { reason: StopReason, pc: u32 }`；`signal()` 返回 GDB 信号号
- `StopReason`：`Breakpoint`、`SingleStep`、`Fault`、`Sleeping`、`Halted`、
  `Reset`、`InstructionLimit`、`Watchpoint`（均为 SIGTRAP，`Watchpoint` 输出
  `T05`）
- `TargetStatus { state, stop_reason, pc, total_cycles }`
- `GdbError`：`InvalidRegister`、`Bus(CoreError)`、`AddressOutOfRange`、
  `Unsupported(&'static str)`

监视点钩子默认返回 `GdbError::Unsupported` 且 `supports_watchpoints()` 为
`false`，因此未实现监视点的目标（如 `CortexM33Target`）无需改动，RSP 层也不会
向 GDB 声明该能力。
The watchpoint hooks default to `GdbError::Unsupported` with
`supports_watchpoints() == false`, so targets without watchpoint support (such as
`CortexM33Target`) need no changes and the RSP layer never advertises the
capability for them.

### `CortexM33Target<B>`

```rust
let target = CortexM33Target::new(core, bus)
    .with_instruction_budget(DEFAULT_INSTRUCTION_BUDGET);
```

`new()` 要求核心已启动（`CortexM33Core::boot`）。断点通过 PC 比较检测
（不写入 BKPT 指令），`Continue` 从当前断点恢复时会先跨过该指令。
`new()` expects a booted core (`CortexM33Core::boot`). Breakpoints are detected
by PC comparison (no BKPT patching); resuming from a breakpoint steps over the
current instruction first.

---

## RSP Protocol Layer / RSP 协议层

`rsp.rs` 不执行 I/O，所有函数与分发器均可直接单元测试。TCP 传输层
（`server.rs`）负责读写字节、解析 `+`/`-` 应答，并调用 `Session::handle_packet`
/ `encode_packet`。
`rsp.rs` performs no I/O; every function and the dispatcher are directly
unit-testable. The TCP transport (`server.rs`) reads/writes bytes, handles
`+`/`-` acknowledgements, and calls `Session::handle_packet` / `encode_packet`.

### Packet Codec / 包编解码

```rust
use rp2350_gdb::{decode_packet, encode_packet, checksum, escape_binary, unescape_binary};

let frame = encode_packet("m20000000,4");   // "$m20000000,4#.."（小写校验和）
assert_eq!(decode_packet(frame.as_bytes())?, "m20000000,4");
assert_eq!(checksum(b"OK"), 0x9a);

let escaped = escape_binary(b"a$b#c}d*e"); // 保留字节转为 `}` + (byte ^ 0x20)
assert_eq!(unescape_binary(&escaped)?, b"a$b#c}d*e");
```

| Item | Behavior / 行为 |
|------|-----------------|
| `checksum(payload)` | 8 位回绕和 / wrapping 8-bit sum |
| `encode_packet(payload)` | `$<payload>#<2 位小写 hex>` |
| `decode_packet(frame)` | 严格校验起始 `$`、终止 `#`、两位 hex 校验和、UTF-8 |
| `escape_binary` / `unescape_binary` | `$`、`#`、`}`、`*` 的 `}` 转义 / RSP binary escaping |
| `encode_hex` / `decode_hex` | 小写输出；偶数长度与字符合法性校验 |
| `ACK` / `NACK` / `parse_ack` | `+` / `-` 应答字节与解析 |
| `ERROR_REPLY` | `E01`，非法包与目标错误的通用回复 |

非法帧（缺标记、校验和错误、非 UTF-8）由 `handle_frame` 回复 `E01`，不关闭
连接。`k` 是唯一不产生回复的命令（返回 `None`）。
Malformed frames are answered with `E01` by `handle_frame` instead of closing the
connection. `k` is the only command with no reply (returns `None`).

### Command Dispatcher / 命令分发

```rust
pub struct RspSession<T: GdbTarget> { /* target + no_ack_mode */ }

impl<T: GdbTarget> RspSession<T> {
    pub fn new(target: T) -> Self;
    pub fn no_ack_mode(&self) -> bool;
    pub fn target(&self) -> &T;
    pub fn reset_transport(&mut self);
    pub fn handle_packet(&mut self, payload: &str) -> Option<String>;
    pub fn handle_frame(&mut self, frame: &[u8]) -> Option<String>;
}
```

| Packet | Reply / 回复 | Notes |
|--------|--------------|-------|
| `qSupported[:features]` | `PacketSize=fff;QStartNoAckMode+;vContSupported+;hwbreak+[;watchpoint+]` | 特性协商；`watchpoint+` 仅在目标支持写监视点时追加 |
| `?` | `S05` | 附加查询（SIGTRAP） |
| `g` | 每寄存器 4 字节小端 hex | 寄存器批量读 |
| `G<hex>` | `OK` | 批量写；长度须为 4 的倍数且不超出寄存器数 |
| `p N` | 8 位小端 hex | 单个寄存器读 |
| `P N=<hex>` | `OK` | 单个寄存器写 |
| `m addr,len` | 小写 hex 字节串 | 长度上限 `MAX_PACKET_SIZE`（0x0fff） |
| `M addr,len:data` | `OK` | 数据字节数必须等于 `len` |
| `c` / `C<sig>` | `S<sig>`（通常 `S05`） | `GdbTarget::resume(Continue)` |
| `s` / `S<sig>` | `S<sig>` | `GdbTarget::resume(Step)` |
| `Z0,addr,kind` / `z0,addr,kind` | `OK` | 软件断点（kind 校验，忽略值） |
| `Z1,addr,kind` / `z1,addr,kind` | `OK` | 硬件断点（复用目标的断点模型） |
| `Z2,addr,kind` / `z2,addr,kind` | `OK` | 写监视点；`kind` 为字节数（1/2/4/8）。重复插入/删除幂等，仅在首次插入与最后一次删除时调用目标 |
| `Z3`/`Z4`（读/访问监视点） | 空包 | 未实现：不回 `OK`，GDB 可据此回退 |
| `D` | `OK` | detach |
| `k` | 无回复 | 传输层应随后关闭连接 |
| `qC` | `QC1` | 单核阶段固定线程 1 |
| `qAttached` | `1` | 已附加到现有目标 |
| `Hg<id>` / `Hc<id>` | `OK` | 线程选择空成功 |
| `vCont?` | `vCont;c;s` | 最小单线程能力 |
| `vCont;c[:id]` / `vCont;s[:id]` | `S05` 或 `T05watch:<addr>;` | 单动作；多动作返回空包（多核待实现） |
| `QStartNoAckMode` | `OK` | 置位 `no_ack_mode()`，状态化 |
| 未知 / 不支持的 `q`、`Q`、`v` | 空包 `""` | 符合 RSP 探测约定 |

写监视点命中时停止回复为 `T05watch:<hex-addr>;`；目标无法给出命中地址时退化为
合法 `T05`。`Z2` 的 `kind` 不是 1/2/4/8 或目标未实现监视点时不会误报成功：
前者返回空包，后者回复 `E01`。
A write-watchpoint stop is reported as `T05watch:<hex-addr>;`; when the target
cannot name the hit address it degrades to a valid `T05`. `Z2` never claims
success falsely: unsupported widths return an empty packet, and targets that
implement no watchpoints answer `E01`.

错误映射：目标错误（非法寄存器、总线错误、地址越界）与非法参数均回复
`E01`，会话保持可用；未知命令返回空包。
Error mapping: target failures (invalid register, bus error, address overflow)
and malformed arguments reply `E01` while the session stays usable; unknown
commands return an empty packet.

`QStartNoAckMode` 注意：传输层必须在发送 `OK` 之后才进入无应答模式；分发器只
记录协商状态。
`QStartNoAckMode` note: the transport must enter no-ack mode only after sending
`OK`; the dispatcher only records the negotiated flag.

---

## TCP Transport / TCP 传输

`server.rs` 实现阻塞式 TCP 服务。`GdbServer` 持有监听地址与运行标志；目标以
`Box<dyn GdbTarget + Send>` 传入，因此服务端无需知道具体 CPU 后端。
`server.rs` implements the blocking TCP transport. `GdbServer` owns the listen
address and a running flag; the target is supplied as
`Box<dyn GdbTarget + Send>`, so the server never needs the concrete CPU type.

```rust
pub struct GdbServer { /* addr + running */ }

impl GdbServer {
    pub fn new(addr: SocketAddr) -> Self;
    pub fn default_addr() -> Self;                  // 0.0.0.0:3333
    pub fn serve(&mut self, target: Box<dyn GdbTarget + Send>) -> io::Result<()>;
    pub fn serve_with(&mut self, listener: &TcpListener,
                      target: Box<dyn GdbTarget + Send>) -> io::Result<()>;
    pub fn stop(&mut self);
    pub fn is_running(&self) -> bool;
    pub fn addr(&self) -> SocketAddr;
}
```

| Method | Behavior / 行为 |
|--------|-----------------|
| `serve` | 绑定 `addr`（端口 0 时回填实际端口），顺序 `accept`，客户端断开后继续接受；监听器错误时返回。Binds `addr`, accepts clients sequentially, keeps accepting after disconnects, returns on listener errors. |
| `serve_with` | 服务调用方提供的已绑定监听器，处理**一个**连接后返回；测试用 `127.0.0.1:0` 获取端口。Serves one connection on a caller-supplied listener (tests bind `127.0.0.1:0`). |
| `stop` | 仅清除运行标志；阻塞中的 `serve` 在返回时观察。Clears the running flag only; a blocked `serve` observes it on return. |
| `addr` / `is_running` | 监听地址（`serve` 后为实际地址）/ 运行状态。Listen address (actual after `serve`) / running state. |

### Frame Loop / 帧循环

每个连接一个 `BufReader` + 克隆的写端，顺序处理：

1. 扫描到 `$` 起始；帧外字节（含 GDB 的 `+`/`-` 应答）被丢弃。
2. 读到未转义的 `#` 后再读两位校验和；`}` 转义字节不会终止帧。
3. 校验通过 → 非无应答模式先回 `+`，再分发并回 `$<reply>#<checksum>`。
4. 校验失败 → 回 `-`，**连接保持打开**，等待 GDB 重发。
5. `QStartNoAckMode` → 仅对请求包回 `+`，随后发送 `OK`；之后不再回 `+`。
6. `k` → 回 `+`、无回复、关闭连接。
7. 客户端 EOF / 连接重置 → 视为正常断开，服务端继续接受下一个客户端。

Per connection the server scans for `$` (discarding out-of-frame bytes such as
GDB `+`/`-` acks), reads through the first unescaped `#` plus two checksum
digits, acknowledges valid frames with `+` before replying, rejects bad
checksums with `-` without closing, honours `QStartNoAckMode`, and closes on `k`
or client EOF.

每个新连接都会调用 `Session::reset_transport()` 清除上一客户端协商的无应答
模式；目标状态（内存、寄存器、断点）保留，因为 `Session` 在整个 `serve`
调用期间共享。
Every new connection calls `Session::reset_transport()` to clear the previous
client's no-ack negotiation; target state (memory, registers, breakpoints)
survives because one `Session` is shared for the whole `serve` call.

---

## Current Limitations / 当前限制

- CLI `gdb` 已接线（`--file <ELF>`、默认端口 3333）；无 `--arch` 覆盖，RISC-V 目标读数 16（XPSR）返回 `E01`。
- 单客户端串行：`serve` 在 `serve_connection` 返回前不接受新连接；无并发/多客户端。
- 无 TLS、无 `qXfer:features:read` / `target.xml`，无 multiprocess。
- 仅写监视点（`Z2`/`z2`）：读监视点 `rwatch`（`Z3`）与访问监视点 `awatch`
  （`Z4`）未实现并返回空包；库层 `CortexM33Target` 未实现监视点，只有 CLI
  `MachineTarget` 提供（且仅限 SRAM）。不使用 DWT 数据断点寄存器。
- `vCont` 仅单动作单线程；多核动作列表返回空包。
- 无 `qRcmd` monitor 命令、无 `qfThreadInfo` / `qsThreadInfo`。
- `stop()` 只清标志，不能中断阻塞中的 `accept`（需要连接一次才退出）。

- The CLI `gdb` subcommand is wired (`--file <ELF>`, default port 3333); there
  is no `--arch` override, and register 16 (XPSR) returns `E01` on RISC-V
  targets.
- One client at a time: `serve` does not accept the next connection until the
  current `serve_connection` returns; no concurrency.
- No TLS, no `qXfer:features:read` / `target.xml`, no multiprocess.
- Write watchpoints only (`Z2`/`z2`): read watchpoints (`Z3`) and access
  watchpoints (`Z4`) are unimplemented and answer with an empty packet. The
  library `CortexM33Target` implements no watchpoints; only the CLI
  `MachineTarget` does, and only for SRAM. No DWT data-breakpoint registers are
  used.
- `vCont` is single-action single-thread only; multi-core action lists return
  an empty packet.
- No `qRcmd` monitor commands or thread-list queries.
- `stop()` only clears a flag; it cannot interrupt a blocked `accept` (one
  connection is needed before `serve` returns).

---

## Usage Example / 使用示例

```rust
use rp2350_gdb::{GdbTarget, RspSession, encode_packet};

let mut session = RspSession::new(target);
assert_eq!(session.handle_packet("?").as_deref(), Some("S05"));
assert_eq!(session.handle_packet("g").is_some(), true);

let frame = encode_packet("m20000000,4");
let reply = session.handle_frame(frame.as_bytes()).unwrap();
```

从库启动 TCP 服务：
Starting the TCP server from the library:

```rust
use std::net::SocketAddr;
use rp2350_gdb::{GdbServer, GdbTarget};

let addr: SocketAddr = "127.0.0.1:3333".parse()?;
let mut server = GdbServer::new(addr);
// Blocks; Ctrl-C (or another thread) is required to leave `serve`.
// 阻塞；需 Ctrl-C 或其他线程才能离开 `serve`。
server.serve(Box::new(target))?;
```

`serve_with` 便于测试：先 `TcpListener::bind("127.0.0.1:0")`，把监听器交给
后台线程，再用返回的端口连接客户端。
`serve_with` is the test entry point: bind `127.0.0.1:0`, hand the listener to a
background thread, then connect a client to the reported port.

CLI 用法 / CLI usage：

```bash
cargo run -p rp2350-cli -- gdb --port 3333 --file firmware.elf
arm-none-eabi-gdb
(gdb) target remote :3333
(gdb) info registers
(gdb) stepi
(gdb) watch counter          # 写监视点（CLI 适配器仅支持 SRAM）
(gdb) continue               # 命中后返回 T05watch:<addr>;
(gdb) detach
```
