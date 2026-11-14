# rp2350-gdb — Public API / 公共 API

> GDB Remote Serial Protocol (RSP) protocol layer for RP2350 debugging.
> 用于 RP2350 调试的 GDB 远程串行协议（RSP）协议层。
>
> **Status**: 协议层已实现 — `rsp.rs` 提供包编解码（`$<payload>#<checksum>`、
> `+`/`-` 应答、`}` 二进制转义、十六进制工具）与无 socket 的命令分发器
> `RspSession<T>`；`CortexM33Target<B>` 为目标适配器。`GdbServer`
> （`server.rs`）仍为桩：TCP 监听与 CLI `gdb` 接线尚未实现。
>
> **Status** (EN): the protocol layer is implemented — `rsp.rs` provides packet
> framing (`$<payload>#<checksum>`), `+`/`-` acknowledgements, `}` binary
> escaping, hex helpers, and a socket-free dispatcher `RspSession<T>`.
> `CortexM33Target<B>` is the target adapter. `GdbServer` (`server.rs`) is still
> a stub: TCP listening and CLI `gdb` wiring are not implemented.

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
    fn resume(&mut self, mode: ResumeMode) -> Result<StopReply, GdbError>;
    fn status(&self) -> TargetStatus;
}
```

支持类型 / Supporting types：

- `ResumeMode`：`Continue` / `Step`
- `StopReply { reason: StopReason, pc: u32 }`；`signal()` 返回 GDB 信号号
- `StopReason`：`Breakpoint`、`SingleStep`、`Fault`、`Sleeping`、`Halted`、
  `Reset`、`InstructionLimit`
- `TargetStatus { state, stop_reason, pc, total_cycles }`
- `GdbError`：`InvalidRegister`、`Bus(CoreError)`、`AddressOutOfRange`

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

`rsp.rs` 不执行 I/O，所有函数与分发器均可直接单元测试。传输层（后续任务）
只需负责读写字节、解析 `+`/`-` 应答，并调用 `handle_frame` / `encode_packet`。
`rsp.rs` performs no I/O; every function and the dispatcher are directly
unit-testable. A future transport only reads/writes bytes, handles `+`/`-`
acknowledgements, and calls `handle_frame` / `encode_packet`.

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
    pub fn handle_packet(&mut self, payload: &str) -> Option<String>;
    pub fn handle_frame(&mut self, frame: &[u8]) -> Option<String>;
}
```

| Packet | Reply / 回复 | Notes |
|--------|--------------|-------|
| `qSupported[:features]` | `PacketSize=fff;QStartNoAckMode+;vContSupported+;hwbreak+` | 特性协商 |
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
| `D` | `OK` | detach |
| `k` | 无回复 | 传输层应随后关闭连接 |
| `qC` | `QC1` | 单核阶段固定线程 1 |
| `qAttached` | `1` | 已附加到现有目标 |
| `Hg<id>` / `Hc<id>` | `OK` | 线程选择空成功 |
| `vCont?` | `vCont;c;s` | 最小单线程能力 |
| `vCont;c[:id]` / `vCont;s[:id]` | `S05` | 单动作；多动作返回空包（多核待实现） |
| `QStartNoAckMode` | `OK` | 置位 `no_ack_mode()`，状态化 |
| 未知 / 不支持的 `q`、`Q`、`v` | 空包 `""` | 符合 RSP 探测约定 |

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

## Current Limitations / 当前限制

- 无 TCP 监听、无 `GdbServer` 会话循环、无 CLI `gdb` 子命令。
- 无 `qXfer:features:read` / `target.xml`，无 multiprocess、watchpoint（`Z2`-`Z4`）。
- `vCont` 仅单动作单线程；多核动作列表返回空包。
- 无 `qRcmd` monitor 命令、无 `qfThreadInfo` / `qsThreadInfo`。

- No TCP listener, no `GdbServer` session loop, no CLI `gdb` subcommand.
- No `qXfer:features:read` / `target.xml`, multiprocess, or watchpoints.
- `vCont` is single-action single-thread only; multi-core action lists return
  an empty packet.
- No `qRcmd` monitor commands or thread-list queries.

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

未来 TCP 接线后的目标用法（规划）：
Planned usage once the TCP transport lands:

```bash
cargo run -p rp2350-cli -- gdb --port 3333 --file firmware.elf
arm-none-eabi-gdb
(gdb) target remote :3333
(gdb) info registers
(gdb) stepi
(gdb) detach
```
