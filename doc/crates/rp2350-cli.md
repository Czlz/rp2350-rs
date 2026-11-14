# rp2350-cli — Public API / 公共 API

> Command-line front end for the RP2350 simulator. Builds the machine bus,
> boots Arm Cortex-M33 or Hazard3 RISC-V firmware, and drives it either as a
> traced `run`, an interactive USB CDC raw REPL (`repl`), or a benchmark
> harness (`perfbench`).
> RP2350 模拟器的命令行前端。构建机器总线，启动 Arm Cortex-M33 或 Hazard3
> RISC-V 固件，并按三种模式驱动：带轨迹的 `run`、交互式 USB CDC raw REPL
> （`repl`）、基准测试（`perfbench`）。
>
> **Status**: implemented — `run` / `repl` / `perfbench` / `gdb` /
> `list-devices` / `wire` / `mcp-serve`.
> **状态**: 已实现 — `run` / `repl` / `perfbench` / `gdb` / `list-devices` /
> `wire` / `mcp-serve`。

---

## Subcommands / 子命令

Global flags: `--verbose` (log level `debug` instead of `info`) and
`--headless` (accepted on every subcommand; the CLI has no GUI today).
全局标志：`--verbose`（日志级别从 `info` 变为 `debug`）与 `--headless`
（所有子命令都接受；目前 CLI 本身没有 GUI）。

### `run` — traced execution / 带轨迹运行

```text
rp2350 run --file <FIRMWARE> [--arch arm|riscv] [--max-cycles N]
           [--trace-steps N] [--strict-decode]
```

| Option / 选项 | Default / 默认 | Meaning / 含义 |
|---------------|----------------|----------------|
| `--file` | required | Firmware, UF2 or ELF / 固件，UF2 或 ELF |
| `--arch` | auto | Override the detected architecture / 覆盖检测到的架构 |
| `--max-cycles` | `20_000_000` | Instruction budget / 指令预算 |
| `--trace-steps` | `16` | Boot-trace steps printed / 打印的启动轨迹步数 |
| `--strict-decode` | off | Halt on undefined instructions / 未定义指令即停机 |

`run` prints the image summary, core description and boot vectors, executes
the first `trace_steps` steps as `[trace]` lines, then runs to halt or budget
exhaustion. The report ends with the halt reason, a 32-step `[tail]` PC
history, the simulated time, up to 8 logged IRQ deliveries, and any buffered
UART0 output.
`run` 打印映像摘要、核心描述与启动向量，先按 `[trace]` 行执行前
`trace_steps` 步，然后运行至停机或预算耗尽。报告以停机原因、32 步 `[tail]`
PC 轨迹、模拟时间、最多 8 条已记录的中断投递以及缓冲的 UART0 输出收尾。

### `repl` — USB CDC raw REPL / USB CDC raw REPL

```text
rp2350 repl --file <FIRMWARE> [--exec "CODE" | --stdio] [--max-cycles N]
            [--enum-cycles N] [--no-soft-reset] [--strict-decode]
            [--arch arm|riscv]
```

| Option / 选项 | Default / 默认 | Meaning / 含义 |
|---------------|----------------|----------------|
| `--file` | required | Firmware (MicroPython/CircuitPython UF2 or ELF) / 固件 |
| `--exec` | interactive | Python source to run once; conflicts with `--stdio` / 只执行一次的 Python 源码；与 `--stdio` 冲突 |
| `--stdio` | off | Transparent stdin/stdout bridge to the CDC bulk endpoints; conflicts with `--exec` / 在 stdin/stdout 与 CDC 批量端点之间透明桥接；与 `--exec` 冲突 |
| `--max-cycles` | `300_000_000` | Total simulated instruction budget; `0` means unlimited for `repl` / 总指令预算；`repl` 中 `0` 表示不限制 |
| `--enum-cycles` | `100_000_000` | Budget before USB enumeration must finish / USB 枚举需在此预算内完成 |
| `--no-soft-reset` | off | Skip the Ctrl-D soft reset on entry (ignored by `--stdio`) / 进入时跳过 Ctrl-D 软复位（`--stdio` 下忽略） |
| `--strict-decode` | off | Halt on undefined instructions / 未定义指令即停机 |
| `--arch` | from UF2 family | Override the core architecture / 覆盖核心架构（`riscv` 为别名） |

The subcommand boots the machine, runs the guest under the USB host bridge
until `HostProgress::Configured` (within `--enum-cycles`), enters the raw
REPL and then either executes `--exec` and prints stdout/stderr, or runs an
interactive line-based session. Enumeration failures print the host bridge's
`progress_line` snapshot plus the fault-tail diagnostics.
该子命令启动机器，在 USB 主机桥下运行客户机直至
`HostProgress::Configured`（须在 `--enum-cycles` 内），进入 raw REPL，随后
要么执行 `--exec` 并打印 stdout/stderr，要么进入交互式按行会话。枚举失败
会打印主机桥的 `progress_line` 快照与故障尾部诊断。

#### `--stdio` transparent bridge / 透明桥接

With `--stdio` the CLI stops driving the raw REPL protocol after enumeration:
stdin bytes are forwarded verbatim to the CDC OUT endpoint and CDC IN bytes are
written verbatim to stdout. The CLI never sends Ctrl-C/Ctrl-A/Ctrl-B/Ctrl-D on
its own, so the peer (`pyboard.py`, `run-perfbench.py`, or a script) owns the
whole protocol.
使用 `--stdio` 时，CLI 在枚举完成后不再驱动 raw REPL 协议：stdin 字节原样
送到 CDC OUT 端点，CDC IN 字节原样写到 stdout。CLI 不会自行发送
Ctrl-C/Ctrl-A/Ctrl-B/Ctrl-D，协议完全由对端（`pyboard.py`、
`run-perfbench.py` 或脚本）掌控。

- **stdout purity / stdout 纯净性**：every `[repl]` diagnostic (image/core
  summary, endpoint line, final state, fault dumps) goes to stderr. stdout
  carries only guest bytes, so it can be piped into a serial-protocol client
  without filtering.
  所有 `[repl]` 诊断（映像/核心摘要、端点行、最终状态、故障转储）都走
  stderr；stdout 只有客户机字节，可直接接入串口协议客户端而无需过滤。
- **Transport / 传输**：a background thread performs blocking stdin reads and
  feeds a bounded queue (16 slots × 4 KiB = 64 KiB in flight); the main loop
  sends queued bytes, drains CDC IN to stdout and flushes, then advances the
  simulation by `STDIO_CHUNK_MICROS` (500 µs, i.e. 75 000 guest cycles at
  150 MHz). Smaller steps lower latency but add host loop overhead; 100–1000 µs
  is the useful range.
  后台线程阻塞读取 stdin，并送入有界队列（16 槽 × 4 KiB = 64 KiB 在途）；
  主循环发送队列字节、把 CDC IN 排空到 stdout 并 flush，然后按
  `STDIO_CHUNK_MICROS`（500 微秒，150 MHz 下 75 000 个客户机周期）推进模拟。
  更小的步长降低延迟但增加宿主循环开销；100–1000 微秒是有效范围。
- **EOF and budget / EOF 与预算**：after stdin EOF the bridge sends every
  queued byte and keeps simulating for a 50 ms (simulated) drain window so a
  late banner/prompt is not truncated, then exits with a stdin-byte count on
  stderr. The default `--max-cycles 300_000_000` is about two simulated seconds
  and may be too small for long benchmark sessions; pass `--max-cycles 0` for
  an unlimited budget (`u64::MAX`). This `0` interpretation applies to `repl`
  (both `--exec` and `--stdio`); `repl --exec` keeps its historical default of
  `300_000_000`.
  stdin EOF 后桥接会把队列字节全部送出，并继续模拟 50 毫秒（模拟时间）的
  排空窗口，避免截断迟到的 banner/提示符，然后在 stderr 打印已处理的 stdin
  字节数并退出。默认 `--max-cycles 300_000_000` 约等于两秒模拟时间，长时间
  基准可能不够；传 `--max-cycles 0` 表示不限制预算（`u64::MAX`）。该 `0`
  语义仅适用于 `repl`（`--exec` 与 `--stdio`）；`repl --exec` 的默认值仍为
  `300_000_000`。
- **Limitations / 限制**：no PTY, no real serial port, no hardware flow
  control (only the 64 KiB bounded queue and OS pipe backpressure), no TLS and
  no multi-client. `--stdio` expects a byte-oriented pipe/file; interactive
  console line editing and Ctrl-C signal handling are the caller's job, and
  Windows terminal compatibility is not emulated (use a pipe or a wrapper that
  redirects stdin/stdout). `--no-soft-reset` is ignored because the CLI does
  not enter raw mode.
  无 PTY、无真实串口、无硬件流控（只有 64 KiB 有界队列与操作系统管道背压）、
  无 TLS、无多客户端。`--stdio` 期望面向字节的管道/文件；交互式控制台的行
  编辑与 Ctrl-C 信号处理由调用方负责，不模拟 Windows 终端兼容性（请使用管道
  或重定向 stdin/stdout 的包装器）。`--no-soft-reset` 会被忽略，因为 CLI 不
  进入 raw 模式。

```powershell
# Windows PowerShell: drive pyboard.py through the stdio bridge
python tools\pyboard.py --device "exec:E:\Source\mcu\pico\rp2350-rs\target\release\rp2350-cli.exe repl --file E:\Source\mcu\pico\python\micropython\RPI_PICO2-20260824-v1.29.0.uf2 --stdio --max-cycles 0" -c "print(6 * 7)"

# run-perfbench.py with the same target / 用同一目标运行 run-perfbench.py
python tests\run-perfbench.py -t "exec:E:\...\rp2350-cli.exe repl --file firmware.uf2 --stdio --max-cycles 0" tests\perf_bench\bm_float.py
```

> **Note / 注意**：`exec:` support and its quoting rules depend on the
> `pyboard.py` version and host shell; any wrapper that pipes this process's
> stdin/stdout works, including a small `.cmd`/shell script.
> `exec:` 支持与引号规则取决于 `pyboard.py` 版本与宿主 shell；任何把本进程
> stdin/stdout 接成管道的包装器也可用（包括小型 `.cmd`/shell 脚本）。

### `perfbench` — MicroPython benchmarks / MicroPython 基准

```text
rp2350 perfbench --file <FIRMWARE> --bench bm_xxx.py --benchrun benchrun.py
                 [--n N] [--m M] [--max-cycles N] [--enum-cycles N]
                 [--no-soft-reset] [--strict-decode]
```

| Option / 选项 | Default / 默认 | Meaning / 含义 |
|---------------|----------------|----------------|
| `--file` | required | Firmware (MicroPython UF2 or ELF) / 固件 |
| `--bench` | required | Benchmark file (`bm_*.py`) / 基准文件 |
| `--benchrun` | required | Shared `benchrun.py` helper / 共享辅助文件 |
| `--n` | `1` | `N` in `bm_run(N, M)` / `bm_run(N, M)` 的 N |
| `--m` | `1` | `M` in `bm_run(N, M)` / `bm_run(N, M)` 的 M |
| `--max-cycles` / `--enum-cycles` / `--no-soft-reset` / `--strict-decode` | as `repl` | same semantics (`--max-cycles 0` is not remapped here) / 语义同 `repl`（此处不把 `--max-cycles 0` 重映射为不限制） |

The script is assembled by
`rp2350_usb_host::assemble_bench_script` (benchmark + benchrun +
`bm_run(N, M)`) and executed through the same enumeration + raw REPL path as
`repl`; the device's `time norm result` line is parsed into a
`BenchResult` (`SKIP` short-circuits).
脚本由 `rp2350_usb_host::assemble_bench_script`（基准 + benchrun +
`bm_run(N, M)`）拼装，并经与 `repl` 相同的枚举 + raw REPL 路径执行；设备的
`time norm result` 行被解析为 `BenchResult`（`SKIP` 直接短路）。

### `gdb` — GDB remote debug server / GDB 远程调试服务器

```text
rp2350 gdb --file <ELF> [--port 3333]
```

| Option / 选项 | Default / 默认 | Meaning / 含义 |
|---------------|----------------|----------------|
| `--file` | required | Firmware (ELF; `--arch` is not exposed here) / 固件（ELF；此处不提供 `--arch`） |
| `--port` | `3333` | TCP listen port (`0.0.0.0`) / TCP 监听端口（绑定 `0.0.0.0`） |

`gdb` reads the ELF, builds the same `Machine` as `run`, prints the image summary,
core description and start vectors, then hands the machine to
`GdbServer`/`MachineTarget` and blocks in `serve`. The adapter forwards GDB
register numbers `0..=15` to the core register file and `16` to XPSR; memory
packets go through `Machine::bus` byte accesses. Breakpoints are PC comparisons
(no `BKPT` patching), and `Continue` runs at most 1 000 000 instructions per
request so an accidental infinite loop cannot block the server.
`gdb` 读取 ELF、构建与 `run` 相同的 `Machine`，打印映像摘要、核心描述与启动
向量，然后把机器交给 `GdbServer`/`MachineTarget` 并阻塞在 `serve`。适配器把
GDB 寄存器号 `0..=15` 转发到核心寄存器组、`16` 转发到 XPSR；内存包通过
`Machine::bus` 的字节访问完成。断点按 PC 比较（不写入 `BKPT`），单次
`Continue` 最多执行 1 000 000 条指令，避免意外死循环阻塞服务器。

```bash
cargo run -p rp2350-cli -- gdb --file firmware.elf --port 3333
# 另一个终端 / another terminal
arm-none-eabi-gdb firmware.elf
(gdb) target remote 127.0.0.1:3333
(gdb) break main
(gdb) continue
(gdb) info registers
```

**Behaviour / 行为**

- `resume` 通过 `Machine::step` 执行，因此 boot ROM 陷阱、中断泵送、PPB 系统
  写入回放与外设时基都保持有效；`StopReply` 的 `Breakpoint`/`SingleStep`/
  `Fault`/`Sleeping`/`Halted`/`Reset`/`InstructionLimit` 映射为对应 GDB 信号
  （`S05` / `S0b` / `S02`）。
  `resume` uses `Machine::step`, so boot ROM traps, interrupt pumping, PPB
  replay and the peripheral timebase stay active; stop reasons map to GDB
  signals (`S05` / `S0b` / `S02`).
- 机器自身预算固定为 `u64::MAX`，停机时机完全由 GDB 的 `c`/`s` 与断点决定；
  `--max-cycles` 未暴露。
  The machine budget is pinned to `u64::MAX`; stopping is driven only by GDB
  `c`/`s` requests and breakpoints (no `--max-cycles` flag).
- 客户端断开后 `serve` 继续接受下一个连接，目标状态（内存、寄存器、断点）保留，
  仅无应答模式协商会被重置。
  After a client disconnects `serve` keeps accepting; target state (memory,
  registers, breakpoints) survives and only the no-ack negotiation is reset.

**Limitations / 限制**

- 无 `--arch` 覆盖：ELF 默认按 Cortex-M33 启动。RISC-V 核心没有 XPSR，
  `p10`/`P10`（寄存器 16）会回复 `E01`。
  No `--arch` override: ELF boots as Cortex-M33. RISC-V has no XPSR, so
  `p10`/`P10` (register 16) replies `E01`.
- 单客户端串行、无 TLS、无 `target.xml`（`qXfer:features:read`）、无 watchpoint
  （`Z2`-`Z4`）、无多核 `vCont`、无 `qRcmd` monitor 命令；细节见
  `doc/crates/rp2350-gdb.md`。
  Single sequential client, no TLS, no `target.xml`, no watchpoints
  (`Z2`-`Z4`), no multi-core `vCont`, no `qRcmd` monitor commands.
- 不做 Flash 编程：Flash 窗口对客户机写保护，`M`/`load` 写 Flash 会以 `E01`
  失败；SRAM 与已映射的可写 MMIO 可正常写入。
  No flash programming: the flash window rejects guest writes, so `M`/`load`
  into flash fails with `E01`; SRAM and writable mapped MMIO work normally.
- 打印的监听地址在端口非 0 时即实际地址；端口 `0` 的实际端口由服务器日志给出。
  The printed address is the actual one for non-zero ports; for port `0` the
  assigned port is reported by the server log.
### `list-devices` — enumerate device plugins / 枚举设备插件

```text
rp2350 list-devices [--plugin-dir <DIR>]
```

| Option / 选项 | Default / 默认 | Meaning / 含义 |
|---------------|----------------|----------------|
| `--plugin-dir` | `plugins` | Directory scanned for plugin libraries / 扫描插件库的目录 |

Scans the directory (platform extensions `dll`/`so`/`dylib`, path-sorted),
instantiates every library through `DeviceManager::scan_and_load`, and prints
one line per device:

```text
com.rp2350-rs.led / LED / has_ui=false / ports=[LED]
```

Format: `id / name / has_ui=<bool> / ports=[<comma-separated port names>]`;
`ports=[-]` means the device exposes no ports. An empty (but existing)
directory prints `no device plugins found in <DIR>`; a missing directory is an
error (exit code 1). Port order is the index order accepted by `wire`.
扫描目录（平台扩展名 `dll`/`so`/`dylib`，按路径排序），经
`DeviceManager::scan_and_load` 实例化每个库，并每台设备打印一行，格式为
`id / name / has_ui=<bool> / ports=[<逗号分隔端口名>]`；`ports=[-]` 表示无端口。
目录存在但为空时打印 `no device plugins found in <DIR>`；目录不存在则报错
（退出码 1）。端口顺序即 `wire` 接受的索引顺序。

### `wire` — wire one port and print the table / 接线并打印接线表

```text
rp2350 wire --plugin-dir <DIR> --device <ID> --port <INDEX> --gpio <PIN>
```

| Option / 选项 | Default / 默认 | Meaning / 含义 |
|---------------|----------------|----------------|
| `--plugin-dir` | `plugins` | Directory scanned for plugin libraries / 扫描插件库的目录 |
| `--device` (`-d`) | required | Device ID (from `list-devices`) / 设备 ID |
| `--port` (`-p`) | required | Port index on the device / 设备端口索引 |
| `--gpio` (`-g`) | required | RP2350 GPIO pin number / GPIO 引脚号 |

Scans the directory, resolves the device ID, calls `DeviceManager::wire` and
prints the resulting wiring table:

```text
com.rp2350-rs.led / LED / port 0 / GPIO25
note: devices and wiring live only in this process and are released on exit / ...
```

Errors (exit code 1): missing plugin directory, unknown device ID (the message
lists the IDs that were found), or out-of-bounds port index.
错误（退出码 1）：插件目录缺失、设备 ID 未找到（错误信息会列出已发现的 ID）、
端口索引越界。

> **One-shot process / 一次性进程**：`list-devices` and `wire` construct the
> `DeviceManager`, act, then exit — the devices, their dynamic libraries and the
> wiring table are released at process exit. Nothing is persisted and no
> emulator run is attached, so `wire` is currently a wiring/dry-run inspection
> tool. Wiring persistence and a long-lived host (GUI/MCP) are future work.
> `list-devices` 与 `wire` 构建管理器、执行动作后立即退出 —— 设备、动态库与
> 接线表在进程结束时全部释放。当前**不持久化**，也不会挂接到模拟器运行，
> 因此 `wire` 实际是接线/空跑检查工具。接线持久化与常驻宿主（GUI/MCP）为后续工作。

### `mcp-serve` — MCP server over HTTP / HTTP 上的 MCP 服务器

```text
rp2350 mcp-serve [--port 8080] [--auth-token <TOKEN>]
```

| Option / 选项 | Default / 默认 | Meaning / 含义 |
|---------------|----------------|----------------|
| `--port` (`-p`) | `8080` | TCP listen port on `0.0.0.0`; `0` picks a free port / `0.0.0.0` 上的 TCP 监听端口；`0` 由系统分配 |
| `--auth-token` | env `RP2350_MCP_TOKEN` | Bearer token required on `POST /mcp`; empty flag or variable = auth off / `POST /mcp` 要求的 Bearer Token；空标志或空变量 = 关闭鉴权 |

Binds the listener, prints the actual base address and the auth state (never the
token), installs a Ctrl-C handler and then blocks in
`rp2350_mcp::http::serve_with_options`:
绑定监听器、打印实际基地址与鉴权状态（绝不打印 Token）、安装 Ctrl-C 处理器，
然后阻塞在 `rp2350_mcp::http::serve_with_options` 中：

```text
[mcp] listening on http://0.0.0.0:18080/mcp (POST JSON-RPC) / health: http://0.0.0.0:18080/health
[mcp] auth: enabled
[mcp] press Ctrl-C to stop / 按 Ctrl-C 停止
```

| Endpoint | Method | Behaviour / 行为 |
|----------|--------|------------------|
| `/mcp`（别名 `/`） | `POST` | JSON-RPC 报文体 → `McpServer::handle_request`，返回 `application/json`；启用鉴权时缺少/错误 Token → `401` |
| `/health` | `GET` | `200 {"status":"ok","server":"rp2350-mcp","version":…,"endpoint":"/mcp","auth":true/false}`，始终开放 |

#### Authentication / 鉴权

- **Token 来源 / token source**：`--auth-token TOKEN` 优先，其次环境变量
  `RP2350_MCP_TOKEN`；两者都为空（未设置或空字符串）则关闭鉴权。因此
  `--auth-token ""` 可显式覆盖环境变量。
  `--auth-token TOKEN` wins over `RP2350_MCP_TOKEN`; absent/empty for both
  disables auth, so `--auth-token ""` explicitly overrides the environment.
- **请求 / request**：`POST /mcp`（含 `/` 别名）必须携带
  `Authorization: Bearer <TOKEN>`；方案名大小写不敏感、Token 精确匹配。
  缺失或错误时返回 `401 Unauthorized`、`WWW-Authenticate: Bearer` 与
  `{"error":"unauthorized"}`。比较为常量时间，不泄露匹配前缀。
  `POST /mcp` (and `/`) requires `Authorization: Bearer <TOKEN>`; the scheme
  is case-insensitive and the token must match exactly. Missing/wrong
  credentials yield `401` with `WWW-Authenticate: Bearer` and
  `{"error":"unauthorized"}`; comparison is constant-time.
- **开放探活 / open probe**：`GET /health` 永不鉴权，便于容器 liveness 探针；
  其 JSON 仅报告 `"auth": true/false`，不回显 Token。
  `GET /health` is never protected; its JSON only reports `"auth"`.
- **错误方法/路径 / errors**：`GET /mcp` 仍为 `405`、未知路径仍为 `404`，
  鉴权不改变这些路由语义。
  `GET /mcp` still returns `405` and unknown paths `404`; auth does not change
  routing semantics.
- **无 TLS / no TLS**：Token 与 JSON-RPC 明文传输。远程部署请用 nginx/Caddy 等
  反向代理终止 TLS 并转发到回环地址；`mcp-serve` 固定绑定 `0.0.0.0`，启用
  Token 时应限制网络可达范围。不要通过命令行传 Token（会留在 shell 历史/进程
  列表中），优先用环境变量或仅本机访问。
  The token crosses cleartext HTTP: terminate TLS in a reverse proxy for remote
  use and restrict reachability. Prefer the environment variable over
  `--auth-token`, which can leak through shell history and process listings.

```powershell
# 启用鉴权 / auth enabled
$env:RP2350_MCP_TOKEN = 's3cret'      # 或用 --auth-token s3cret
rp2350 mcp-serve --port 18080
# 另一个终端 / another terminal
Invoke-RestMethod -Uri http://127.0.0.1:18080/health
try {
  Invoke-RestMethod -Method Post -Uri http://127.0.0.1:18080/mcp -ContentType 'application/json' `
    -Body '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'
} catch { $_.Exception.Response.StatusCode.value__ }   # 401
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:18080/mcp -ContentType 'application/json' `
  -Headers @{ Authorization = 'Bearer s3cret' } `
  -Body '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'
```

**Stopping / 停止语义**：`Ctrl-C` 置真内部的 `AtomicBool` 关停标志，accept 循环
在 20 ms 轮询点返回，`serve` 打印 `[mcp] server stopped` 后进程正常退出（退出码
0）。没有 `--stop`/管理端点，也不做守护化；测试与嵌入场景可直接调用
`rp2350_mcp::http::serve(listener, &server, &shutdown)`（无鉴权）或
`serve_with_options(..., &options)`。
`Ctrl-C` sets an internal `AtomicBool`; the accept loop returns on its next 20 ms
poll, `serve` prints `[mcp] server stopped`, and the process exits with code 0.
There is no stop endpoint or daemonization; tests and embedders call
`rp2350_mcp::http::serve(listener, &server, &shutdown)` (no auth) or
`serve_with_options(..., &options)` directly.

**Scope / 范围**：本子命令**不加载插件**、也不启动仿真，注册表为空，因此
`list_devices` 返回 `[]`、`query_device` 返回 `-32001`。鉴权仅支持单枚静态
Bearer Token；无 TLS、无用户/角色/限流、无 SSE、无 keep-alive。请求头上限
8 KiB、报文体上限 1 MiB，非法 JSON 返回 `400`（载荷为 JSON-RPC `-32700`），
未知路径 `404`，错误方法 `405`。
The subcommand loads no plugins and starts no simulation, so the registry is
empty (`list_devices` → `[]`, `query_device` → `-32001`). Authentication is a
single static bearer token only: no TLS, users/roles/rate limiting, SSE or
keep-alive. 8 KiB head and 1 MiB body limits; malformed JSON → `400` carrying a
JSON-RPC `-32700` payload, unknown paths → `404`, wrong methods → `405`.

---

## `Machine` — the shared front-end / 共享前端

`run.rs` defines the machine every subcommand uses:
`run.rs` 定义了所有子命令共用的机器：

```rust
pub const DEFAULT_RUN_MAX_CYCLES: u64 = 20_000_000;

pub enum ArchArg { Arm, RiscV }               // clap ValueEnum

pub struct MachineOptions {
    pub arch: Option<ArchArg>,               // architecture override
    pub max_cycles: u64,                     // instruction budget
    pub strict_decode: bool,
}

pub struct Machine { /* bus, bootrom, core, peripherals, usb … */ }
impl Machine {
    pub fn new(bytes: &[u8], options: &MachineOptions) -> Result<Self>;
    pub fn step(&mut self) -> Result<CpuState>;
    pub fn trace_step(&mut self, index: usize) -> Result<CpuState>;  // prints [trace]
    pub fn run_cycles(&mut self, cycles: u64);   // CdcLink cycle callback
    pub fn finalize(&mut self);                  // flush timebase + UART
    // diagnostics: image_description, core_description, boot_vectors, pc, sp,
    // steps, now_cycles, time_us, budget, halt_reason, usb, irqs,
    // take_uart_output, take_notices, recent_steps, register, dump_memory
    // debugger: read_register, write_register, xpsr, write_xpsr, cpu_state
}

pub fn register_snapshot(machine: &Machine) -> String;  // R0..R12/SP/LR/PC line
pub fn fault_tail_lines(machine: &mut Machine) -> Vec<String>; // [regs]+[mem]+[tail]
```

### Bus assembly / 总线组装

`Machine::new` builds, in order: the synthetic boot ROM (`Rp2350Bootrom::new`
with the flash handle, mounted at `0x0000_0000`), the flash XIP window at
`0x1000_0000` plus its non-cached alias at `0x1400_0000` (one shared backing
store), the write-absorbing XIP maintenance window at `0x1800_0000`,
520 KB SRAM at `0x2000_0000`, `BootPeripherals` (GPIO, UART0, timer, etc.),
and `rp2350_usb::mount` for the USB controller (DPSRAM + register windows).
`Machine::new` 依次构建：合成 boot ROM（`Rp2350Bootrom::new` 持有 flash
句柄，挂载于 `0x0000_0000`）、`0x1000_0000` 的 Flash XIP 窗口与
`0x1400_0000` 的非缓存别名（共享同一份底层存储）、`0x1800_0000` 吸收写入的
XIP 维护窗口、`0x2000_0000` 的 520 KB SRAM、`BootPeripherals`（GPIO、UART0、
定时器等），以及 USB 控制器的 `rp2350_usb::mount`（DPSRAM + 寄存器窗口）。

### Firmware formats / 固件格式

`load_image` detects the container by magic:
`load_image` 按魔数检测容器：

- **UF2** (`"UF2\n\x0A"` prefix): parsed and loaded through `Uf2Image`; the
  family selects the default architecture (`Arm` when unknown). Arm boots from
  the reset vector table at the flash XIP base (VTOR follows); RISC-V lets
  `Hazard3Core` locate the picobin `IMAGE_DEF` entry-point item and load the
  boot2 entry/SP. `--arch` overrides the detected architecture.
  **UF2**（`"UF2\n\x0A"` 前缀）：经 `Uf2Image` 解析并加载；family 决定默认
  架构（未知则为 `Arm`）。Arm 从 Flash XIP 基地址的复位向量表启动（VTOR
  随之指向那里）；RISC-V 由 `Hazard3Core` 定位 picobin `IMAGE_DEF` 入口点
  条目并加载 boot2 入口/SP。`--arch` 覆盖检测到的架构。
- **ELF**: parsed and loaded through `FirmwareImage`; the entry point comes
  from the image, the stack pointer is SRAM top, and
  `CortexM33Core::boot_at` is used (Arm) or `Hazard3Core` boots from its own
  picobin header (RISC-V). `--arch` overrides both paths.
  **ELF**：经 `FirmwareImage` 解析并加载；入口点来自映像，栈指针为 SRAM
  顶部，Arm 使用 `CortexM33Core::boot_at`，RISC-V 则由 `Hazard3Core` 从其
  自身的 picobin 头启动。`--arch` 可覆盖两条路径。

Boot ROM traps apply only to Arm cores (`bootrom_applicable` is false for
RISC-V); on each trap step the machine increments its step counter, pumps
interrupts once and does not advance the peripheral timebase.
Boot ROM 陷阱只对 Arm 核生效（RISC-V 时 `bootrom_applicable` 为 false）；每个
陷阱步会令机器步数加一、立即泵送一次中断，且不推进外设时基。

### Execution loop details / 执行循环细节

- Steps double as cycles: `steps == now_cycles`, and the peripheral timebase
  advances in 10 000-step batches (`time_us` includes pending steps).
  步数同时充当周期：`steps == now_cycles`；外设时基按 10 000 步批量推进
  （`time_us` 会把未推进的步数也计入）。
- System-control writes (e.g. CPACR) captured from the private peripheral
  bus are replayed into the core **before the next instruction**; IRQ lines
  are pumped every 64 steps and USB's `USBCTRL_IRQ` whenever
  `controller.irq_asserted()`.
  从私有外设总线捕获的系统控制写入（如 CPACR）会在**下一条指令前**回放进
  核心；中断线每 64 步泵送一次，`controller.irq_asserted()` 成立时投递
  `USBCTRL_IRQ`。
- A tight spin warning prints when the PC stays within one 32-byte window
  for 1 000 000 consecutive steps (up to 4 warnings per run).
  当 PC 在同一个 32 字节窗口内连续停留 1 000 000 步时打印一次自旋告警
  （每次运行最多 4 条）。

---

## Dependencies / 依赖

| Crate | Why / 用途 |
|-------|------------|
| `clap` (derive) | subcommand/flag parsing / 子命令与标志解析 |
| `anyhow` | error context chain / 错误上下文链 |
| `log` + `env_logger` | logging / 日志 |
| `rp2350-core`, `rp2350-cpu` | bus, loaders, `CpuCore` / 总线、加载器、`CpuCore` |
| `rp2350-arm`, `rp2350-riscv` | the two cores / 两个核心 |
| `rp2350-bootrom` | synthetic ROM + trap dispatch / 合成 ROM 与陷阱分发 |
| `rp2350-periph` | `BootPeripherals`, `Peripheral` contract / 启动外设 |
| `rp2350-usb`, `rp2350-usb-host` | controller + CDC/raw-REPL front end / 控制器与前端 |
| `rp2350-virt-periph` | `DeviceManager` behind `list-devices` / `wire` / 设备管理器 |
| `rp2350-gdb` | RSP server behind `gdb` / `gdb` 子命令的 RSP 服务器 |
| `rp2350-mcp` | `McpServer` + HTTP transport behind `mcp-serve` / `mcp-serve` 的服务器与 HTTP 传输 |
| `rp2350-egui`, `rp2350-ffi`, `tokio` | linked for the future egui integration; `tokio` also waits for Ctrl-C / 为后续 egui 集成预留；`tokio` 另用于等待 Ctrl-C |

---

## Examples / 示例

```bash
# Run a UF2 firmware with a 5 million instruction budget
rp2350 run --file firmware.uf2 --max-cycles 5000000

# Force the RISC-V core for an ELF image
rp2350 run --file firmware.elf --arch riscv

# Execute one snippet over the MicroPython raw REPL
rp2350 repl --file firmware.uf2 --exec "print(6 * 7)"

# Transparent stdin/stdout bridge for pyboard.py / run-perfbench.py
# (stdout carries guest bytes only; use --max-cycles 0 for long sessions)
rp2350 repl --file firmware.uf2 --stdio --max-cycles 0 < input.bin > output.bin

# Benchmark, 4x64 repetitions
rp2350 perfbench --file firmware.uf2 --bench bm_float.py \
                 --benchrun benchrun.py --n 4 --m 64

# Serve an ELF to GDB on port 3333
rp2350 gdb --file firmware.elf --port 3333

# Inspect the device plugins built into target/release
rp2350 list-devices --plugin-dir target/release

# Wire the LED plugin's port 0 to GPIO25 and print the wiring table
rp2350 wire --plugin-dir target/release --device com.rp2350-rs.led --port 0 --gpio 25

# Serve the MCP JSON-RPC server over HTTP on port 18080
rp2350 mcp-serve --port 18080

# Same, requiring `Authorization: Bearer s3cret` on POST /mcp (/health stays open)
$env:RP2350_MCP_TOKEN = 's3cret'; rp2350 mcp-serve --port 18080

# Explicit flag wins over the environment; an empty flag disables auth
rp2350 mcp-serve --port 18080 --auth-token s3cret
```
