# rp2350-gdb — Public API / 公共 API

> GDB Remote Serial Protocol (RSP) protocol layer for RP2350 debugging.
> 用于 RP2350 调试的 GDB 远程串行协议（RSP）协议层。
>
> **Status**: 协议层、TCP 服务与 CLI 接线均已实现 — `rsp.rs` 提供包编解码（`$<payload>#<checksum>`、
> `+`/`-` 应答、`}` 二进制转义、十六进制工具）与无 socket 的命令分发器
> `Session<T>`；`CortexM33Target<B>` 与 CLI 的 `MachineTarget` 为两种目标适配器。
> `target_xml.rs` 提供静态 ARMv8-M `target.xml`，经 `qXfer:features:read` 按字节
> 分块提供给 GDB；`GdbServer`（`server.rs`）提供阻塞式 TCP 监听、顺序接受客户端
> 与字节级 RSP 帧循环；`rp2350 gdb --file <ELF> --port <N>` 直接启动服务。
>
> **Status** (EN): the protocol layer, the TCP server and the CLI wiring are
> implemented — `rsp.rs` provides packet framing (`$<payload>#<checksum>`),
> `+`/`-` acknowledgements, `}` binary escaping, hex helpers, and a socket-free
> dispatcher `Session<T>`. `CortexM33Target<B>` and the CLI `MachineTarget` are
> the two target adapters. `target_xml.rs` supplies the static ARMv8-M
> `target.xml`, chunked byte-wise over `qXfer:features:read`. `GdbServer`
> (`server.rs`) provides a blocking TCP listener, sequential client accept, and
> the byte-level RSP frame loop; `rp2350 gdb --file <ELF> --port <N>` starts the
> server directly.

---

## Key Types / 关键类型

### `GdbTarget` Trait

```rust
pub trait GdbTarget: Send {
    fn register_count(&self) -> u32;               // default: 17 (R0-R15 + XPSR)
    fn target_description(&self) -> Option<&'static str>; // default: ARMv8-M target.xml
    fn read_register(&mut self, register: u32) -> Result<u32, GdbError>;
    fn write_register(&mut self, register: u32, value: u32) -> Result<(), GdbError>;
    fn read_memory(&mut self, address: u32, length: usize) -> Result<Vec<u8>, GdbError>;
    fn write_memory(&mut self, address: u32, bytes: &[u8]) -> Result<(), GdbError>;
    fn set_breakpoint(&mut self, address: u32) -> Result<(), GdbError>;
    fn clear_breakpoint(&mut self, address: u32) -> Result<(), GdbError>;
    fn has_breakpoint(&self, address: u32) -> bool;
    fn supports_watchpoints(&self) -> bool;                    // default: false
    fn set_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError>;   // Z2
    fn clear_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError>;
    fn set_read_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError>;  // Z3
    fn clear_read_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError>;
    fn set_access_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError>; // Z4
    fn clear_access_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError>;
    fn watchpoint_hit(&self) -> Option<u32>;                   // default: None
    fn monitor_command(&mut self, command: &str) -> String;    // default: empty (= OK)
    fn thread_count(&self) -> usize;                           // default: 1
    fn thread_id(&self, index: usize) -> Option<ThreadId>;     // default: index+1
    fn thread_name(&self, thread: ThreadId) -> String;
    fn thread_state(&self, thread: ThreadId) -> Option<CpuState>;
    fn read_thread_register(&mut self, thread: ThreadId, register: u32)
        -> Result<u32, GdbError>;
    fn write_thread_register(&mut self, thread: ThreadId, register: u32, value: u32)
        -> Result<(), GdbError>;
    fn continue_thread(&mut self, thread: ThreadId) -> Result<StopReply, GdbError>;
    fn step_thread(&mut self, thread: ThreadId) -> Result<StopReply, GdbError>;
    fn stopped_thread(&self) -> ThreadId;                      // default: core0
    fn resume(&mut self, mode: ResumeMode) -> Result<StopReply, GdbError>;
    fn status(&self) -> TargetStatus;
}
```

支持类型 / Supporting types：
- `ThreadId(u32)`：`CORE0 = 1`、`CORE1 = 2`；`number()` 返回线上编号
- `ResumeMode`：`Continue` / `Step`
- `WatchKind`：`Write`（`Z2`）/ `Read`（`Z3`）/ `Access`（`Z4`）
- `StopReply { reason: StopReason, pc: u32 }`；`signal()` 返回 GDB 信号号
- `StopReason`：`Breakpoint`、`SingleStep`、`Fault`、`Sleeping`、`Halted`、
  `Reset`、`InstructionLimit`、`Watchpoint`（`T05watch:`）、`ReadWatch`
  （`T05rwatch:`）、`AccessWatch`（`T05awatch:`）；前三类监视点均为 SIGTRAP
- `TargetStatus { state, stop_reason, pc, total_cycles }`
- `GdbError`：`InvalidRegister`、`Bus(CoreError)`、`AddressOutOfRange`、
  `Unsupported(&'static str)`

监视点钩子默认返回 `GdbError::Unsupported` 且 `supports_watchpoints()` 为
`false`，因此未实现监视点的目标（如 `CortexM33Target`）无需改动，RSP 层也不会
向 GDB 声明该能力；`Z3`/`Z4` 在只实现写监视点的目标上同样以 `E01` 干净失败。
`monitor_command` 默认返回空串，RSP 层把它映射为 `OK`（成功且无输出）。
The watchpoint hooks default to `GdbError::Unsupported` with
`supports_watchpoints() == false`, so targets without watchpoint support (such as
`CortexM33Target`) need no changes and the RSP layer never advertises the
capability for them; `Z3`/`Z4` fail cleanly with `E01` on a write-only target.
`monitor_command` defaults to an empty string, which the RSP layer maps to `OK`
(success, no output).

> 注意：`server.rs` 为 `Box<dyn GdbTarget + Send>` 提供了转发实现。新增 trait
> 方法时必须同步转发，否则 TCP 服务会静默回落到默认实现（监视点/monitor 失效）。
> Note: `server.rs` implements `GdbTarget` for `Box<dyn GdbTarget + Send>` by
> forwarding. Every new trait method must be forwarded there too, or the TCP
> server silently falls back to the default implementation.

`target_description()` 默认返回静态 ARMv8-M `target.xml`（见下文）；寄存器文件
不同的目标可返回 `None`，此时 `qXfer:features:read` 回复 `E00`。
`target_description()` defaults to the static ARMv8-M `target.xml` described
below; targets with a different register file may return `None`, which makes
`qXfer:features:read` answer `E00`.

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
| `qSupported[:features]` | `PacketSize=fff;qXfer:features:read+;QStartNoAckMode+;vContSupported+;hwbreak+[;watchpoint+]` | 特性协商；`watchpoint+` 仅在目标支持监视点时追加（写/读/访问三类共用，不区分）；声明 `qXfer:features:read` 后 GDB 会主动读取 `target.xml` |
| `?` | `T05thread:<n>;` | 附加查询（SIGTRAP，带当前线程） |
| `g` | 每寄存器 4 字节小端 hex | 寄存器批量读；按当前线程路由 |
| `G<hex>` | `OK` | 批量写；长度须为 4 的倍数且不超出寄存器数 |
| `p N` | 8 位小端 hex | 单个寄存器读；按当前线程路由 |
| `P N=<hex>` | `OK` | 单个寄存器写；按当前线程路由 |
| `m addr,len` | 小写 hex 字节串 | 长度上限 `MAX_PACKET_SIZE`（0x0fff） |
| `M addr,len:data` | `OK` | 数据字节数必须等于 `len` |
| `c` / `C<sig>` | `T<sig>thread:<n>;`（通常 `T05`） | `GdbTarget::resume(Continue)`；带 `Hc` 选择时只继续该线程 |
| `s` / `S<sig>` | `T<sig>thread:<n>;` | `GdbTarget::resume(Step)`；带 `Hc` 选择时只单步该线程 |
| `Z0,addr,kind` / `z0,addr,kind` | `OK` | 软件断点（kind 校验，忽略值） |
| `Z1,addr,kind` / `z1,addr,kind` | `OK` | 硬件断点（复用目标的断点模型） |
| `Z2,addr,kind` / `z2,addr,kind` | `OK` | 写监视点（`watch`）；`kind` 为字节数（1/2/4/8） |
| `Z3,addr,kind` / `z3,addr,kind` | `OK`（目标不支持时 `E01`） | 读监视点（`rwatch`）；同样的宽度校验与幂等语义 |
| `Z4,addr,kind` / `z4,addr,kind` | `OK`（目标不支持时 `E01`） | 访问监视点（`awatch`），读写均触发 |
| `qRcmd,<hex>` | `<hex 输出>` / `OK` | monitor 命令：`hex` 是 UTF-8 命令的十六进制；目标返回文本按 hex 回复，空文本回复 `OK` |
| `D` | `OK` | detach |
| `k` | 无回复 | 传输层应随后关闭连接 |
| `qC` | `QC<n>` | 当前线程（`Hg` 选择，否则最近停止线程） |
| `qAttached` | `1` | 已附加到现有目标 |
| `qfThreadInfo` | `m1,2` | 线程枚举首包（单核目标 `m1`） |
| `qsThreadInfo` | `l` | 枚举结束；`qfThreadInfo` 会重置游标 |
| `qThreadExtraInfo,<hex-id>` | 线程名的 hex（如 `636f726531` = `core1`） | 未知线程 `E01` |
| `Hg<id>` / `Hc<id>` | `OK`（越界 `E01`） | 记录寄存器/继续线程；`0`/`-1` 表示全部 |
| `T<id>` | `OK` / `E01` | 查询线程是否存活 |
| `vCont?` | `vCont;c;C;s;S;t` | 多线程动作能力 |
| `vCont;<action>[:<tid>][;...]` | `T<sig>thread:<n>;`、`E01` 或空包 | 见下文“线程模型与 `vCont`” |
| `QStartNoAckMode` | `OK` | 置位 `no_ack_mode()`，状态化 |
| `qXfer:features:read:target.xml:<off>,<len>` | `m<chunk>` / `l<chunk>` | 按字节切片的目标描述；`len=0`、`off` 到界或越界均返回空 `l` |
| `qXfer:features:read:<其他 annex>:...` | `E00` | 未提供的 annex；参数非法同样 `E00` |
| 未知 / 不支持的 `q`、`Q`、`v` | 空包 `""` | 符合 RSP 探测约定 |

写监视点命中时停止回复为 `T05watch:<hex-addr>;`，读监视点为
`T05rwatch:<hex-addr>;`，访问监视点为 `T05awatch:<hex-addr>;`；目标无法给出
命中地址时三者都退化为合法 `T05`。`Z2`/`Z3`/`Z4` 的 `kind` 不是 1/2/4/8 时返回
空包且不转发给目标；目标未实现对应类型时回复 `E01`。同一地址可同时安装写、读、
访问三种监视点，会话按 `(addr, kind)` 分别记账，重复插入/删除幂等。
A write-watchpoint stop is reported as `T05watch:<hex-addr>;`, a read watchpoint
as `T05rwatch:<hex-addr>;`, and an access watchpoint as
`T05awatch:<hex-addr>;`; all three degrade to a valid `T05` when the target
cannot name the hit address. A `kind` other than 1/2/4/8 returns an empty packet
and is never forwarded; an unimplemented kind replies `E01`. The same address can
carry a write, a read and an access watchpoint at once — the session books them
per `(addr, kind)`, and repeated inserts/removes are idempotent.

`qRcmd,<hex>` 是 monitor 通道：载荷是命令文本的十六进制（如 `info` 为
`696e666f`），回复是目标输出文本的十六进制；目标返回空串时回复 `OK`。非法十六
进制或非 UTF-8 载荷回复 `E01`，会话保持可用。未识别的命令由目标记录
`log::info!` 并返回空串，因此 GDB 不会因拼写错误报错。
`qRcmd,<hex>` is the monitor channel: the payload is the hex-encoded command text
(`info` is `696e666f`) and the reply is the hex-encoded target output; an empty
target string becomes `OK`. Malformed hex or non-UTF-8 payloads reply `E01` while
the session stays usable. Unknown commands are logged with `log::info!` and
return an empty string, so a typo never makes GDB error out.

错误映射：目标错误（非法寄存器、总线错误、地址越界）与非法参数均回复
`E01`，会话保持可用；未知命令返回空包。
Error mapping: target failures (invalid register, bus error, address overflow)
and malformed arguments reply `E01` while the session stays usable; unknown
commands return an empty packet.

### Threads and `vCont` / 线程模型与 `vCont`

线程编号由 `GdbTarget::thread_count`/`thread_id` 决定：默认单线程（`m1`），
双核目标暴露 `m1,2`，其中 1 是 core0（启动核）、2 是 core1。`qfThreadInfo`
的首个包列出全部线程 id，后续 `qsThreadInfo` 回复 `l` 结束；再次发送
`qfThreadInfo` 会重置游标。
Thread ids come from `GdbTarget::thread_count`/`thread_id`: the default is a
single thread (`m1`) and a dual-core target exposes `m1,2`, where 1 is core 0
(the boot core) and 2 is core 1. The first `qfThreadInfo` packet lists every id
and the following `qsThreadInfo` replies `l`, ending the list; a fresh
`qfThreadInfo` resets the cursor.

`Hg<n>` 选择 `g`/`G`/`p`/`P` 路由的线程（`0`/`-1` 表示任意线程，回退到最近
停止线程）；`Hc<n>` 选择 `c`/`s` 与 `vCont` 省略线程号时使用的线程。内存包
`m`/`M` 不区分线程（两个核心共享同一条总线）。`T<n>` 用于存活探测，返回
`OK` 或 `E01`。
`Hg<n>` selects the thread used by `g`/`G`/`p`/`P` (`0`/`-1` means any thread and
falls back to the last stopped thread); `Hc<n>` selects the thread for `c`/`s`
and for `vCont` actions without an explicit id. Memory packets `m`/`M` are
thread-agnostic because both cores share one bus. `T<n>` probes liveness and
replies `OK` or `E01`.

`vCont` 语法：`vCont;<action>[:<tid>][;...]`，动作可为 `c`/`C<sig>`（继续）、
`s`/`S<sig>`（单步）或 `t`（停止）。省略 `:<tid>`、`tid = -1`/`0` 均表示全部
线程；`c:1;c:2` 与裸 `c` 等价。继续与单步混用的列表无法由本目标满足，回复
`E01`；未知动作回复空包；越界线程号回复 `E01`。`vCont;t` 以
`T02thread:<n>;`（SIGINT）应答。
`vCont` syntax is `vCont;<action>[:<tid>][;...]` with `c`/`C<sig>` (continue),
`s`/`S<sig>` (step) or `t` (stop). A missing `:<tid>`, `tid = -1` and `tid = 0`
all mean every thread, and `c:1;c:2` is equivalent to a bare `c`. Mixed
continue/step lists cannot be satisfied by this target and reply `E01`; unknown
actions reply an empty packet and out-of-range ids reply `E01`. `vCont;t` answers
`T02thread:<n>;` (SIGINT).

停止回复统一带线程字段：普通停止为 `T05thread:1;`，监视点停止为
`T05watch:<addr>;thread:2;`（字段顺序不敏感）。线程号来自
`GdbTarget::stopped_thread`，`MachineTarget` 在 core1 断点/core1 停机时上报 2。
Every stop reply carries the thread field: a normal stop is `T05thread:1;` and a
watchpoint stop is `T05watch:<addr>;thread:2;` (field order is not significant).
The number comes from `GdbTarget::stopped_thread`; `MachineTarget` reports 2
when a core-1 breakpoint or core-1 stop is what ended the resume.

> 单线程选择只影响**上报线程与 PC**：没有按核步进 API 的目标（如
> `MachineTarget`，其 `Machine::step` 各推进 core0/core1 一条指令）会退化为
> 双核同步步进。`step_thread(2)` 时 core0 也会前进一条指令，这一点由
> `crates/rp2350-cli/src/gdb_target/tests.rs` 锁定。
> A single-thread selection only affects the **reported thread and PC**: targets
> without a per-core step API (such as `MachineTarget`, whose `Machine::step`
> retires one core-0 *and* one core-1 instruction) degrade to lock-step dual-core
> stepping. `step_thread(2)` also advances core 0 by one instruction; the CLI
> adapter tests lock this behaviour down.

`QStartNoAckMode` 注意：传输层必须在发送 `OK` 之后才进入无应答模式；分发器只
记录协商状态。
`QStartNoAckMode` note: the transport must enter no-ack mode only after sending
`OK`; the dispatcher only records the negotiated flag.

---

## Target Description / 目标描述

`target_xml.rs` 持有静态 `target.xml`（纯 ASCII），`Session` 负责解析
`qXfer:features:read`。GDB 在 `qSupported` 看到 `qXfer:features:read+` 后会
自动读取该文档，无需 `set architecture`。
`target_xml.rs` owns the static, pure-ASCII `target.xml`; `Session` parses
`qXfer:features:read`. Once `qSupported` advertises
`qXfer:features:read+`, GDB fetches the document automatically and no
`set architecture` is required.

```xml
<?xml version="1.0"?>
<!DOCTYPE target SYSTEM "gdb-target.dtd">
<target>
  <architecture>armv8-m.main</architecture>
  <feature name="org.gnu.gdb.arm.m-profile">
    <reg name="r0" bitsize="32" regnum="0" type="data_ptr"/>
    <!-- r1-r12 同形 / same shape for r1-r12 -->
    <reg name="sp" bitsize="32" regnum="13" type="data_ptr"/>
    <reg name="lr" bitsize="32" regnum="14" type="int"/>
    <reg name="pc" bitsize="32" regnum="15" type="code_ptr"/>
    <reg name="xpsr" bitsize="32" regnum="16" type="int"/>
  </feature>
</target>
```

### Register Numbers / 寄存器编号

| regnum | XML name | `regs` | 说明 / Notes |
|--------|----------|--------|--------------|
| 0-12 | `r0`-`r12` | `R0`-`R12` | 通用寄存器，`data_ptr` / general purpose, `data_ptr` |
| 13 | `sp` | `SP` | 当前栈指针（MSP/PSP 由 CONTROL 选择）/ active stack pointer |
| 14 | `lr` | `LR` | 链接寄存器 / link register |
| 15 | `pc` | `PC` | 程序计数器，`code_ptr` / program counter |
| 16 | `xpsr` | `XPSR` | 组合状态寄存器 / combined status register |

### Chunking Semantics / 分块语义

请求格式 `qXfer:features:read:target.xml:<offset>,<length>`，偏移与长度均为
十六进制、以**字节**计。回复前缀 `m` 表示还有后续数据，`l` 表示这是最后一块
（数据可能为空）。XML 为纯 ASCII，因此字节偏移与字符偏移一致。
The request is `qXfer:features:read:target.xml:<offset>,<length>`; both numbers
are hexadecimal **byte** counts. The `m` prefix means more data follows, `l`
means this is the final chunk (possibly empty). The document is pure ASCII, so
byte and character offsets coincide.

| 情况 / Case | 回复 / Reply |
|-------------|--------------|
| `offset + length < len` | `m<length 字节>` |
| `offset + length >= len` | `l<剩余字节>` |
| `offset == len` | `l`（空 / empty） |
| `offset > len` | `l`（空 / empty，按协议选择而非 `E00`） |
| `length == 0` | `l`（空；`m` + 空数据会让客户端死循环） |
| 未知 annex / 参数非法 | `E00` |
| 目标 `target_description() == None` | `E00` |

越界偏移返回空 `l` 而不是 `E00`：GDB 把 `E00` 当作传输错误并可能重试，空 `l`
则干净地表示文档结束。Over-long offsets return an empty `l` rather than
`E00`: GDB treats `E00` as a transfer error and may retry, while an empty `l`
cleanly terminates the document.

### GDB Usage / GDB 用法

```bash
cargo run -p rp2350-cli -- gdb --port 3333 --file firmware.elf
arm-none-eabi-gdb -ex "target remote :3333" -ex "info registers"
# 手工验证 / manual check:
(gdb) maintenance packet qXfer:features:read:target.xml:0,fff
# -> l<?xml version="1.0"?>...
```

The description uses `<architecture>armv8-m.main</architecture>` and the
`org.gnu.gdb.arm.m-profile` feature. This combination was validated with
`arm-none-eabi-gdb 16.3`; the earlier `armv8-m` / `m-system` pair was rejected
by GDB (`unknown architecture`, then `Architecture rejected target-supplied
description`), so the names above are the ones GDB actually accepts. The
`m-profile` feature normally also carries `msp`/`psp`/`control`/`primask`/
`basepri`/`faultmask`; those are omitted because the target cannot read them by
register number yet.
描述使用 `<architecture>armv8-m.main</architecture>` 与
`org.gnu.gdb.arm.m-profile` 特性。该组合已用 `arm-none-eabi-gdb 16.3` 实测；
此前的 `armv8-m`/`m-system` 组合会被 GDB 拒绝（先报 `unknown architecture`，
再报 `Architecture rejected target-supplied description`），因此以上名称是 GDB
真正接受的组合。`m-profile` 通常还包含 `msp`/`psp`/`control`/`primask`/
`basepri`/`faultmask`，这些寄存器暂不声明，因为目标还不能按编号读取它们。

### Not Exposed / 未暴露

- `msp` / `psp` / `control`（`org.gnu.gdb.arm.m-profile`）：目标目前无法按编号
  读取，因此不声明；GDB 不会尝试访问不存在的寄存器。
  `msp` / `psp` / `control` (`org.gnu.gdb.arm.m-profile`) are omitted because
  the target cannot read them by number; GDB will not probe registers that the
  description does not list.
- FPU（`s0`-`s31`、`fpscr`）、DWT 比较器、ETM 寄存器均不在描述中。
  No FPU (`s0`-`s31`, `fpscr`), DWT comparator, or ETM registers are described.

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
- 无 TLS、无 multiprocess。
- `qXfer:features:read` 仅提供静态 `target.xml`；无动态 XML 生成，无 FPU/DWT/ETM
  寄存器，也未声明 `msp`/`psp`/`control`（目标无法按编号读取）。
- 监视点由目标适配器实现：`MachineTarget` 支持写（`Z2`）、读（`Z3`）与访问
  （`Z4`）三类，且仅接受 SRAM；库层 `CortexM33Target` 仍未实现监视点。不使用
  DWT 数据断点寄存器。
- 核心层读监视只支持**一个** `[start, end)` 区间：多个读/访问监视点被合并成一个
  跨度，命中后再匹配回具体条目。
- 读监视覆盖 SRAM 区的所有读，包括取指与 core1 的读（地址落在区间内时）；宿主侧
  读取（GDB `m` 包与写监视点快照）被显式抑制，不会自我触发。
- 性能：未安装过读监视的进程每次 SRAM 读只多一次 relaxed 原子 load；安装后每次读
  多一次 thread-local 判断。门控只置位不复位（区间按线程存放），因此不会出现另一
  线程撤除监视导致本线程漏报；写监视点仍按指令采样。
- 线程模型上限为两核（core0/core1）；`CortexM33Target` 仍是单线程目标（默认
  `thread_count() == 1`）。`MachineTarget` 暴露两线程，但 core1 寄存器只读且仅
  PC/SP 为真实值（其余读 0），单线程 `vCont`/`Hc` 选择退化为双核同步步进。
- 继续与单步混用的 `vCont` 动作列表返回 `E01`；不支持 `vCont;r`（range step）。
- `qRcmd` monitor 命令由目标定义；库层默认全部返回 `OK`（无输出）。
- `stop()` 只清标志，不能中断阻塞中的 `accept`（需要连接一次才退出）。

- The CLI `gdb` subcommand is wired (`--file <ELF>`, default port 3333); there
  is no `--arch` override, and register 16 (XPSR) returns `E01` on RISC-V
  targets.
- One client at a time: `serve` does not accept the next connection until the
  current `serve_connection` returns; no concurrency.
- No TLS, no multiprocess.
- `qXfer:features:read` serves only the static `target.xml`; there is no dynamic
  XML generation, no FPU/DWT/ETM registers, and no `msp`/`psp`/`control`
  (the target cannot read them by number).
- Watchpoints are implemented by target adapters: `MachineTarget` supports write
  (`Z2`), read (`Z3`) and access (`Z4`) and only for SRAM; the library
  `CortexM33Target` still implements none. No DWT data-breakpoint registers are
  used.
- The core read watch covers exactly **one** `[start, end)` range: several
  read/access watchpoints are merged into a single span and matched back to the
  concrete entry on a hit.
- The read watch sees every SRAM read, including instruction fetches and core-1
  reads whose address falls inside the range. Host-side reads (GDB `m` packets
  and write-watchpoint snapshots) are explicitly suppressed and never
  self-trigger.
- Performance: a process that never armed a read watch pays one extra relaxed
  atomic load per SRAM read; once armed, every read pays one thread-local check.
  The gate is sticky (the range lives per thread), so another thread disarming
  its watch can never make this thread miss hits. Write watchpoints keep
  sampling once per instruction.
- The thread model tops out at two cores (core 0/core 1); `CortexM33Target` stays
  a single-thread target (default `thread_count() == 1`). `MachineTarget`
  exposes two threads, but core-1 registers are read-only and only PC/SP carry
  real values (everything else reads 0); a single-thread `vCont`/`Hc` selection
  degrades to lock-step dual-core stepping.
- Mixed continue/step `vCont` action lists reply `E01`; `vCont;r` (range step) is
  not supported.
- `qRcmd` monitor commands are defined by the target; the library default
  answers everything with `OK` (no output).
- `stop()` only clears a flag; it cannot interrupt a blocked `accept` (one
  connection is needed before `serve` returns).

---

## Usage Example / 使用示例

```rust
use rp2350_gdb::{GdbTarget, RspSession, encode_packet};

let mut session = RspSession::new(target);
assert_eq!(session.handle_packet("?").as_deref(), Some("T05thread:1;"));
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
(gdb) watch counter          # 写监视点 Z2（CLI 适配器仅支持 SRAM）
(gdb) continue               # 命中后返回 T05watch:<addr>;
(gdb) rwatch *(unsigned int*)0x20081FF0   # 读监视点 Z3
(gdb) continue               # 命中后返回 T05rwatch:20081ff0;
(gdb) awatch *(unsigned int*)0x20081FF0   # 访问监视点 Z4（读写均触发）
(gdb) continue               # 命中后返回 T05awatch:20081ff0;
(gdb) monitor info           # qRcmd：core0/core1/步数/DMA/HSTX 摘要
(gdb) monitor halt           # 之后的 c/s 立即以 SIGINT 停止
(gdb) monitor reset          # 恢复启动向量的 PC/SP
(gdb) detach
```
