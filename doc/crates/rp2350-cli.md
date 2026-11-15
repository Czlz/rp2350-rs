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
| `--dump` | none | `ADDR:LEN:PATH` memory dump written when the run stops (repeatable, decimal or `0x` hex) / 停机时写入的内存导出，可重复，支持十进制或 `0x` 十六进制 |

`run` prints the image summary, core description and boot vectors, executes
the first `trace_steps` steps as `[trace]` lines, then runs to halt or budget
exhaustion. The report ends with the halt reason, a 32-step `[tail]` PC
history, the simulated time, up to 8 logged IRQ deliveries, and any buffered
UART0 output. Arm machines also print one `[run] core1 state=… pc=… sp=…
launches=…` line (plus `[run] core1 halt: …` if core 1 stopped) after the
DMA/HSTX counters.
`run` 打印映像摘要、核心描述与启动向量，先按 `[trace]` 行执行前
`trace_steps` 步，然后运行至停机或预算耗尽。报告以停机原因、32 步 `[tail]`
PC 轨迹、模拟时间、最多 8 条已记录的中断投递以及缓冲的 UART0 输出收尾。
Arm 机器还会在 DMA/HSTX 计数之后打印一行
`[run] core1 state=… pc=… sp=… launches=…`（core1 停机时另有
`[run] core1 halt: …`）。

The diagnostic block is `[run] dma=<units> busy=<mask>`, then
`[run] hstx words=<expanded> underruns=<episodes> fifo=<level> enabled=<bool>
wof=<bool> unknown=<count>`, then three `[run] dma chN rd=… wr=… cnt=… ctrl=…
busy=… chain=… treq=…` channel snapshots, then the core-1 line. `Machine` pumps
HSTX on the DMA batch cadence (every 64 steps) with the periph pixel-clock
pacing, so a healthy 60 Hz scanout advances `words` by ~134 400 per frame with
`underruns=0`, `wof=false` and `unknown=0`.
诊断块依次为 `[run] dma=<单元数> busy=<掩码>`、
`[run] hstx words=<展开字数> underruns=<断流次数> fifo=<电平> enabled=<布尔>
wof=<布尔> unknown=<计数>`、三条 `[run] dma chN rd=… wr=… cnt=… ctrl=…
busy=… chain=… treq=…` 通道快照，最后是 core1 行。`Machine` 按 DMA 批次节拍
（每 64 步）以 periph 的像素时钟节拍泵送 HSTX，因此健康的 60 Hz 扫描每帧让
`words` 增加约 134 400，且 `underruns=0`、`wof=false`、`unknown=0`。

Each `--dump` request then writes its bytes and prints
`[run] dump 0x<addr>+0x<len> -> <path> (<n> bytes)`. This is the fastest way to
capture a firmware framebuffer without attaching GDB; the concrete address of a
target's framebuffer belongs to that example's README (see
`example/gba/README.md` for `emerald.uf2`).
每条 `--dump` 请求随后写入字节并打印
`[run] dump 0x<addr>+0x<len> -> <path> (<n> bytes)`。这是不挂 GDB 也能抓取固件
帧缓冲的最快方式；具体目标帧缓冲地址属于该示例的 README（`emerald.uf2` 见
`example/gba/README.md`）。

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
- **Mask-ROM view / 掩膜 ROM 视图**：before enumeration `repl` calls
  `rp2350_usb_host::mask_rom::install()` to replace the synthetic `bootrom`
  region with a read-through, write-dropping view. TinyUSB's EP0 has
  `endpoint_control == NULL` and `hw_endpoint_start_next_buffer()` stores through
  it on every control transfer; real mask ROM ignores that store, while a
  write-rejecting region would halt the guest with
  `Bus error at address 0x00000000` at the first SETUP. `run` does not enumerate
  USB and keeps the original region.
  枚举前 `repl` 调用 `rp2350_usb_host::mask_rom::install()`，把合成 `bootrom`
  区域替换为读取透传、写入丢弃的视图。TinyUSB 的 EP0 满足
  `endpoint_control == NULL`，而 `hw_endpoint_start_next_buffer()` 会在每次控制
  传输中透过它写入；真实掩膜 ROM 会忽略该写入，拒绝写入的区域则会让客户机在
  首个 SETUP 处以 `Bus error at address 0x00000000` 停机。`run` 不枚举 USB，
  保留原区域。
- **SOF cadence / SOF 节拍**：the host bridge issues one start-of-frame token per
  1 ms of simulated time (`SofScheduler`, `sysclk_hz/1000` cycles), first during
  enumeration and then through the CDC link's `advance()`. SOF always precedes
  that frame's transactions, so TinyUSB's errata-15 workaround
  (`e15_last_sof`) sees a fresh `SOF_RD` instead of a frozen value; no SOF is
  sent while the device is detached.
  主机桥按 **1 ms 模拟时间**下发一个帧起始令牌（`SofScheduler`，
  `sysclk_hz/1000` 周期）：枚举期间由 `poll()` 下发，之后由 CDC 链路的
  `advance()` 继续。SOF 总是先于本帧事务，因此 TinyUSB 的 errata-15 变通
  （`e15_last_sof`）能看到新的 `SOF_RD` 而不是冻结值；设备未附着时不下发 SOF。
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

# Firmware that sleeps before printing: keep stdin open until the log appears.
# `--max-cycles 0` removes the default budget cap. The example tool is at
# example/gba/tools/hold_stdin.py.
py example\gba\tools\hold_stdin.py 600 | rp2350-cli.exe repl --file emerald.uf2 --stdio --max-cycles 0
```

A stdin EOF drains only 50 ms after the last byte, so firmware that sleeps
before its first `printf` (for example `emerald.uf2`: 1500 ms) ends the bridge
before printing anything. Pipe a long-lived stdin into the process instead; the
measured step counts for that firmware are in
[example/gba/doc/gba-bringup-report.md](../../example/gba/doc/gba-bringup-report.md).
stdin EOF 后仅排空 50 毫秒，因此首次 `printf` 之前会休眠的固件（例如
`emerald.uf2` 的 1500 毫秒）会在打印前就结束桥接。请改为向进程管道一个长期
存活的 stdin；该固件的实测步数见
[example/gba/doc/gba-bringup-report.md](../../example/gba/doc/gba-bringup-report.md)。

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
register numbers `0..=15` to the core register file and `16` to XPSR, routing
`g`/`G`/`p`/`P` through the thread selected by `Hg` (thread 1 = core 0,
thread 2 = core 1); memory packets go through `Machine::bus` byte accesses and
are shared by both cores. Breakpoints are PC comparisons (no `BKPT` patching),
and `Continue` runs at most 1 000 000 instructions per
request so an accidental infinite loop cannot block the server. Write
watchpoints (`watch`, RSP `Z2`) are sampled after every executed instruction:
`MachineTarget` keeps a byte snapshot per watchpoint and stops with
`T05watch:<addr>;thread:<n>;` as soon as any covered byte changes. Read watchpoints
(`rwatch`, `Z3`) use the core's runtime read hook and stop with
`T05rwatch:<addr>;thread:<n>;`; access watchpoints (`awatch`, `Z4`) stop with
`T05awatch:<addr>;thread:<n>;` on either path. Only SRAM
(`0x2000_0000..0x2008_2000`) is accepted — peripheral/USB windows are rejected
with `E01` because sampling them would pop FIFOs or advance device state.
`monitor` commands reach the adapter through `qRcmd`: `monitor reset` rebuilds
PC/SP from the boot vectors, `monitor info` prints core 0/1 state, step count and
DMA/HSTX summaries, `monitor halt` makes the next `c`/`s` stop with SIGINT, and
unknown commands are logged and answered with `OK`.
`gdb` 读取 ELF、构建与 `run` 相同的 `Machine`，打印映像摘要、核心描述与启动
向量，然后把机器交给 `GdbServer`/`MachineTarget` 并阻塞在 `serve`。适配器把
GDB 寄存器号 `0..=15` 转发到核心寄存器组、`16` 转发到 XPSR，并按 `Hg` 选定的
线程（线程 1 = core0，线程 2 = core1）路由 `g`/`G`/`p`/`P`；内存包通过
`Machine::bus` 的字节访问完成，由两个核心共享。断点按 PC 比较（不写入 `BKPT`），
单次 `Continue` 最多执行 1 000 000 条指令，避免意外死循环阻塞服务器。写监视点
（`watch`，RSP `Z2`）在每条指令执行后采样：`MachineTarget` 为每个监视点保存
字节快照，任一被覆盖字节变化即返回 `T05watch:<addr>;thread:<n>;` 停止。读监视点
（`rwatch`，`Z3`）使用核心的运行期读钩子，命中返回
`T05rwatch:<addr>;thread:<n>;`；访问监视点（`awatch`，`Z4`）在读或写任一路径命中
时返回 `T05awatch:<addr>;thread:<n>;`。仅接受
SRAM（`0x2000_0000..0x2008_2000`）；外设/USB 窗口会以 `E01` 拒绝，因为采样它们
会弹出 FIFO 或推进设备状态。`monitor` 命令经 `qRcmd` 到达适配器：`monitor reset`
按启动向量重建 PC/SP，`monitor info` 打印 core0/core1 状态、步数与 DMA/HSTX
摘要，`monitor halt` 使随后的 `c`/`s` 立即以 SIGINT 停止，未识别命令记录日志并
回复 `OK`。

```bash
cargo run -p rp2350-cli -- gdb --file firmware.elf --port 3333
# 另一个终端 / another terminal
arm-none-eabi-gdb firmware.elf
(gdb) target remote 127.0.0.1:3333
(gdb) info threads         # 2 个线程：1 = core0，2 = core1
(gdb) thread 2             # 切换到 core1
(gdb) info registers pc    # core1 的 PC（core1 只读）
(gdb) vCont;s:2            # 仅请求 core1 单步（双核同步退化）
(gdb) thread 1
(gdb) break main
(gdb) continue
(gdb) info registers
(gdb) watch counter        # 写监视点：仅 SRAM 变量
(gdb) continue             # 命中：T05watch:20000000;thread:1;
(gdb) rwatch *(unsigned int*)0x20081FF0   # 读监视点 Z3
(gdb) continue             # 命中：T05rwatch:20081ff0;thread:1;
(gdb) awatch *(unsigned int*)0x20081FF0   # 访问监视点 Z4（读写均触发）
(gdb) continue             # 命中：T05awatch:20081ff0;thread:1;
(gdb) monitor info         # core0/core1、步数、DMA/HSTX 摘要
(gdb) monitor halt         # 之后的 c/s 立即停止（SIGINT）
(gdb) monitor reset        # PC/SP 回到启动向量
(gdb) delete               # 移除监视点后不再触发
(gdb) continue
```

**Behaviour / 行为**

- `resume` 通过 `Machine::step` 执行，因此 boot ROM 陷阱、中断泵送、PPB 系统
  写入回放与外设时基都保持有效；`StopReply` 的 `Breakpoint`/`SingleStep`/
  `Watchpoint`/`ReadWatch`/`AccessWatch`/`Fault`/`Sleeping`/`Halted`/`Reset`/
  `InstructionLimit` 映射为对应 GDB 信号（`T05thread:<n>;` /
  `T05watch:<addr>;thread:<n>;` / `T05rwatch:<addr>;thread:<n>;` /
  `T05awatch:<addr>;thread:<n>;` / `T0bthread:<n>;` / `T02thread:<n>;`）。
  `resume` uses `Machine::step`, so boot ROM traps, interrupt pumping, PPB
  replay and the peripheral timebase stay active; stop reasons map to GDB
  signals (`T05thread:<n>;` / `T05watch:<addr>;thread:<n>;` /
  `T05rwatch:<addr>;thread:<n>;` / `T05awatch:<addr>;thread:<n>;` /
  `T0bthread:<n>;` / `T02thread:<n>;`).
- 写监视点比较的是**值变化**而非“发生写入”：写入相同字节不会停止。快照读取
  使用局部周期计数器，不推进 `Machine::steps` 或外设时基。
  Watchpoints compare **values**, not the act of writing: storing identical
  bytes does not stop. Snapshot reads use a local cycle counter and never
  advance `Machine::steps` or the peripheral timebase.
- 读监视点是**访问即命中**：每步执行后先查 `watch::take_read_hit()`（读/访问
  监视点），再查写快照（写/访问监视点），因此 `awatch` 在读与写两条路径上都会
  命中。宿主自身的读取（GDB `m` 包与写监视点快照）被 `watch::without_read_watch`
  抑制，`resume` 开始时会丢弃停止期间遗留的命中。
  Read watchpoints fire **on access**: after each step the adapter checks
  `watch::take_read_hit()` (read/access) before the write snapshots
  (write/access), so `awatch` stops on both paths. Host-side reads (GDB `m`
  packets and write-watchpoint snapshots) are wrapped in
  `watch::without_read_watch`, and `resume` discards hits recorded while stopped.
- 机器自身预算固定为 `u64::MAX`，停机时机完全由 GDB 的 `c`/`s` 与断点决定；
  `--max-cycles` 未暴露。
  The machine budget is pinned to `u64::MAX`; stopping is driven only by GDB
  `c`/`s` requests and breakpoints (no `--max-cycles` flag).
- 线程模型：`MachineTarget` 在 Arm 目标上暴露两个线程（`m1,2`），线程 1 是
  core0、线程 2 是 core1；RISC-V 目标没有 core1，保持单线程（`m1`）。`qfThreadInfo`
  首包列全部 id，`qsThreadInfo` 回复 `l` 结束。`thread 2` 后 `info registers pc`
  读取 core1 的 PC，`vCont;s:2` 只请求 core1 单步。
  Thread model: `MachineTarget` exposes two threads on Arm targets (`m1,2`) —
  thread 1 is core 0 and thread 2 is core 1; RISC-V targets have no core 1 and
  keep a single-thread table (`m1`). `qfThreadInfo` lists every id in its first
  packet and `qsThreadInfo` ends the list with `l`. After `thread 2`,
  `info registers pc` reads core 1's PC and `vCont;s:2` requests a core-1 step.
- core1 寄存器只读：`Machine` 只公开 `core1_pc`/`core1_sp`/`core1_state`，因此
  线程 2 的 `p f`（PC）与 `p d`（SP）是真实值，其余通用寄存器读 0，任何写入
  回复 `E01`（`GdbError::Unsupported`）。core1 断点通过 PC 比较检测，命中时停止
  回复带 `thread:2;`。
  Core-1 registers are read-only: `Machine` only publishes
  `core1_pc`/`core1_sp`/`core1_state`, so thread 2's `p f` (PC) and `p d` (SP)
  are real, the other general-purpose registers read 0, and every write replies
  `E01` (`GdbError::Unsupported`). Core-1 breakpoints use PC comparison and their
  stop reply carries `thread:2;`.
- 单步/继续由 `Machine::step` 驱动：它每次都先执行一条 core0 指令再执行一条
  core1 指令。因此 `vCont;s:2`（或 `thread 2` 后的 `s`）**无法让 core0 停住**，
  这是已文档化的退化语义；只有上报的线程号与 PC 跟随请求。
  Single-steps and continues run through `Machine::step`, which retires one core-0
  instruction and then one core-1 instruction. `vCont;s:2` (or `s` after
  `thread 2`) therefore cannot freeze core 0 — a documented degradation; only the
  reported thread and PC follow the request.
- 停止回复统一带 `thread:<n>;`：普通停止 `T05thread:1;`、监视点停止
  `T05watch:<addr>;thread:2;`，`?` 复用最近停止线程。`monitor info` 仍打印
  core0/core1 摘要。
  Stop replies always carry `thread:<n>;`: a normal stop is `T05thread:1;`, a
  watchpoint stop is `T05watch:<addr>;thread:2;`, and `?` reuses the last stopped
  thread. `monitor info` still prints the core 0/1 summary.
- 客户端断开后 `serve` 继续接受下一个连接，目标状态（内存、寄存器、断点、
  监视点）保留，仅无应答模式协商会被重置。
  After a client disconnects `serve` keeps accepting; target state (memory,
  registers, breakpoints, watchpoints) survives and only the no-ack negotiation
  is reset.

**Limitations / 限制**

- 无 `--arch` 覆盖：ELF 默认按 Cortex-M33 启动。RISC-V 核心没有 XPSR，
  `p10`/`P10`（寄存器 16）会回复 `E01`。
  No `--arch` override: ELF boots as Cortex-M33. RISC-V has no XPSR, so
  `p10`/`P10` (register 16) replies `E01`.
- 监视点覆盖写（`watch` / `Z2`）、读（`rwatch` / `Z3`）与访问（`awatch` / `Z4`）
  三类，仅接受 SRAM（`0x2000_0000`–`0x2008_2000`）地址；外设、USB、PPB 与
  Flash 窗口返回 `E01`。不使用 DWT 数据断点寄存器：写在每条指令后比较快照，
  读由核心的 `watch::check_read` 钩子记录。核心层只支持一个合并后的读区间，且
  取指与 core1 落在此区间内的读同样算命中；未安装过读监视的进程每次 SRAM 读只多
  一次 relaxed 原子 load（门控只置位不复位，之后为一次 thread-local 判断）。
  Watchpoints cover write (`watch` / `Z2`), read (`rwatch` / `Z3`) and access
  (`awatch` / `Z4`), and only SRAM (`0x2000_0000`–`0x2008_2000`) addresses;
  peripheral, USB, PPB and flash windows reply `E01`. No DWT data-breakpoint
  registers are used: writes compare a snapshot after each instruction and reads
  are recorded by the core `watch::check_read` hook. The core supports one merged
  read span, and instruction fetches or core-1 reads inside it also count as
  hits; a process that never armed a read watch pays one extra relaxed atomic
  load per SRAM read (the gate is sticky, then it becomes a thread-local check).
- `monitor` 最小集合：`reset`（按启动向量重建 PC/SP）、`info`（core0/core1 状态、
  步数、DMA/HSTX 摘要）、`halt`（下一次 `c`/`s` 立即以 SIGINT 停止，`reset`
  解除）。未识别命令记录 `log::info!` 并回复 `OK`，不会中断会话。
  Minimal `monitor` set: `reset` (rebuild PC/SP from the boot vectors), `info`
  (core 0/1 state, step count, DMA/HSTX summaries) and `halt` (the next `c`/`s`
  stops immediately with SIGINT; `reset` clears it). Unknown commands are logged
  with `log::info!` and answered with `OK`, so the session never breaks.
- 已支持双核线程模型（core0/core1）与 `vCont` 单核选择；受 `Machine::step`
  双核同步步进限制，单核 resume 只改变上报线程。单客户端串行、无 TLS；细节见
  `doc/crates/rp2350-gdb.md`。
  The dual-core thread model (core 0/core 1) and single-thread `vCont` selection
  are supported; because `Machine::step` advances both cores, a single-core
  resume only changes the reported thread. One sequential client and no TLS; see
  `doc/crates/rp2350-gdb.md`.
- 不做 Flash 编程：Flash 窗口对客户机写保护，`M`/`load` 写 Flash 会以 `E01`
  失败；SRAM 与已映射的可写 MMIO 可正常写入。
  No flash programming: the flash window rejects guest writes, so `M`/`load`
  into flash fails with `E01`; SRAM and writable mapped MMIO work normally.
- 打印的监听地址在端口非 0 时即实际地址；端口 `0` 的实际端口由服务器日志给出。
  The printed address is the actual one for non-zero ports; for port `0` the
  assigned port is reported by the server log.
### `list-devices` — enumerate device plugins / 枚举设备插件

```text
rp2350 list-devices [--plugin-dir <DIR>] [--reload | --watch]
```

| Option / 选项 | Default / 默认 | Meaning / 含义 |
|---------------|----------------|----------------|
| `--plugin-dir` | `plugins` | Directory scanned for plugin libraries / 扫描插件库的目录 |
| `--reload` | off | Rescan once, print the hot-reload report, exit / 重新扫描一次，打印热重载报告后退出 |
| `--watch` | off | Poll the directory (1 s) and reload until Ctrl-C / 每秒轮询目录并热重载，直到 Ctrl-C |

`--reload` and `--watch` are mutually exclusive and both are off by default, so
the plain invocation behaves exactly as before.
`--reload` 与 `--watch` 互斥，默认均关闭，因此不带参数的调用行为完全不变。

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

#### Hot reload: `--reload` / `--watch` / 热重载

`--reload` scans once, prints the device list, then calls
`DeviceManager::reload(dir)` and prints the returned `ReloadReport` before
exiting:
`--reload` 扫描一次、打印设备列表，然后调用 `DeviceManager::reload(dir)` 并打印
返回的 `ReloadReport` 后退出：

```text
com.rp2350-rs.uart-terminal / UART_TERMINAL / has_ui=false / ports=[TX,RX]
[reload] + target/release/rp2350_uart_terminal.dll
[reload] summary: added=1 removed=0 reloaded=0 failed=0
```

`--watch` does the same initial scan and then sleeps `WATCH_INTERVAL` (1 s) and
reloads in a loop; a non-empty report prints the `[reload] …` lines plus the
refreshed device list, an unchanged poll stays silent. Ctrl-C terminates the
process (there is no custom signal handler).
`--watch` 同样先做一次初始扫描，然后每 `WATCH_INTERVAL`（1 秒）循环重载；有变更时
打印 `[reload] …` 行与刷新后的设备列表，无变更的轮询保持静默。Ctrl-C 直接终止
进程（未安装自定义信号处理器）。

Report line prefixes / 报告行前缀：

| Prefix | Meaning / 含义 |
|--------|----------------|
| `+` | newly loaded library / 新加载的库 |
| `-` | library file gone, device dropped / 文件消失，设备已移除 |
| `~` | same path, size/mtime changed → unloaded and loaded again / 同路径但大小或修改时间变化 → 卸载后重新加载 |
| `!` | load failed, followed by the error / 加载失败后跟错误信息 |
| `summary:` | `added=… removed=… reloaded=… failed=…` counts / 各类计数 |

Hot reload only changes files on disk; wiring is not persisted and the devices
still live only in this process. A reloaded plugin starts as a fresh instance,
and a library that keeps its `metadata().id` keeps its wiring. Windows locks
loaded DLLs, so a plugin must be rebuilt to a new file name (or removed) before
the host can pick it up; see `doc/plugins.md` → *Hot Reload*.
热重载只影响磁盘文件；接线不持久化，设备仍只存在于本进程。重载后的插件是全新
实例；保持 `metadata().id` 的库会保留接线。Windows 会锁定已加载 DLL，因此插件
必须重新构建为新文件名（或先删除）宿主才能拾取；详见 `doc/plugins.md` →
*Hot Reload*。

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
> tool. `list-devices --watch` is the only long-lived mode and it still holds
> devices only in memory. Wiring persistence and a long-lived host (GUI/MCP) are
> future work.
> `list-devices` 与 `wire` 构建管理器、执行动作后立即退出 —— 设备、动态库与
> 接线表在进程结束时全部释放。当前**不持久化**，也不会挂接到模拟器运行，
> 因此 `wire` 实际是接线/空跑检查工具。`list-devices --watch` 是唯一的常驻模式，
> 其设备同样只存在于内存。接线持久化与常驻宿主（GUI/MCP）为后续工作。

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

pub struct Machine { /* bus, bootrom, core, core1, peripherals, usb … */ }
impl Machine {
    pub fn new(bytes: &[u8], options: &MachineOptions) -> Result<Self>;
    pub fn step(&mut self) -> Result<CpuState>;
    pub fn trace_step(&mut self, index: usize) -> Result<CpuState>;  // prints [trace]
    pub fn run_cycles(&mut self, cycles: u64);   // CdcLink cycle callback
    pub fn finalize(&mut self);                  // flush timebase + UART
    // diagnostics: image_description, core_description, boot_vectors, pc, sp,
    // steps, now_cycles, time_us, budget, halt_reason, usb, irqs,
    // dma_transfers, dma_state, hstx_words, hstx_state, pio_busy_mask,
    // pio_pc (test hook), core1_state, core1_pc, core1_sp, core1_launch_count,
    // core1_halt_reason, take_uart_output, take_notices, recent_steps,
    // register, dump_memory
    // debugger: read_register, write_register, xpsr, write_xpsr, cpu_state
}

pub struct RunOptions {
    pub file: PathBuf,
    pub arch: Option<ArchArg>,
    pub max_cycles: u64,
    pub trace_steps: usize,
    pub strict_decode: bool,
    pub dumps: Vec<MemoryDump>,       // --dump ADDR:LEN:PATH (repeatable)
}

impl MemoryDump {
    pub fn parse(spec: &str) -> Result<Self, String>;
}

pub fn register_snapshot(machine: &Machine) -> String;  // R0..R12/SP/LR/PC line
pub fn fault_tail_lines(machine: &mut Machine) -> Vec<String>; // [regs]+[mem]+[tail]
```

### Bus assembly / 总线组装

`Machine::new` builds, in order: the synthetic boot ROM (`Rp2350Bootrom::new`
with the flash handle, mounted at `0x0000_0000`), the flash XIP window at
`0x1000_0000` plus its non-cached alias at `0x1400_0000` (one shared backing
store), the write-absorbing XIP maintenance window at `0x1800_0000`,
520 KB SRAM at `0x2000_0000`, `BootPeripherals` (GPIO, UART0, timer, three
shared PIO block handles, etc.), and `rp2350_usb::mount` for the USB controller
(DPSRAM + register windows).
`Machine::new` 依次构建：合成 boot ROM（`Rp2350Bootrom::new` 持有 flash
句柄，挂载于 `0x0000_0000`）、`0x1000_0000` 的 Flash XIP 窗口与
`0x1400_0000` 的非缓存别名（共享同一份底层存储）、`0x1800_0000` 吸收写入的
XIP 维护窗口、`0x2000_0000` 的 520 KB SRAM、`BootPeripherals`（GPIO、UART0、
定时器、三个共享 PIO 块句柄等），以及 USB 控制器的 `rp2350_usb::mount`
（DPSRAM + 寄存器窗口）。

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

### Dual-core execution / 双核执行

`Machine` owns a second Cortex-M33 (`CpuId::Core1`) for Arm images; RISC-V
machines keep `core1: None`, so every core-1 diagnostic returns `None` there.
Core 1 is created in `CpuState::Reset` and never fetches an instruction until
the SIO FIFO launch protocol hands it vectors.
Arm 映像的 `Machine` 还持有一个 Cortex-M33（`CpuId::Core1`）；RISC-V 机器保持
`core1: None`，因此其所有 core1 诊断都返回 `None`。core1 以 `CpuState::Reset`
创建，在 SIO FIFO 启动协议交出向量前绝不取指。

- **Launch protocol / 启动协议**：pico-sdk `multicore_launch_core1_raw` writes
  `{0, 0, 1, vtor, sp, entry}` to `SIO_FIFO_WR` and waits for every word to be
  echoed. `BootPeripherals::service_core1_peer` (called on every 64-step pump
  and on traced/trap steps) plays the resident core-1 boot ROM: it echoes each
  word, records `vtor`/`sp`/`entry` when the six-word window matches, and then
  retires so real core-1 ↔ core-0 traffic is no longer stolen. The machine
  consumes the vectors through `take_core1_launch`, probes the vector table and
  calls `boot_at(sp, entry, vtor)` — `SP = sp`, `PC = entry & !1`,
  `VTOR = vtor`; the final state is reported as `[run] core1 state=… pc=… sp=…
  launches=…`.
  **启动协议**：pico-sdk `multicore_launch_core1_raw` 向 `SIO_FIFO_WR` 写入
  `{0, 0, 1, vtor, sp, entry}` 并等待逐字回显。
  `BootPeripherals::service_core1_peer`（每 64 步泵送与轨迹/陷阱步调用）扮演
  常驻 core1 boot ROM：回显每个字，六字窗口匹配时记录 `vtor`/`sp`/`entry`，
  随后退役，使真实的 core1 ↔ core0 流量不再被窃取。机器通过
  `take_core1_launch` 取走向量，探测向量表后调用 `boot_at(sp, entry, vtor)`，
  即 `SP = sp`、`PC = entry & !1`、`VTOR = vtor`；结果由
  `[run] core1 state=… pc=… sp=… launches=…` 报告。
- **Stepping order / 步进顺序**：`Machine::step`, `trace_step`, `run_cycles`
  and GDB `resume` all retire one core-0 instruction and then one core-1
  instruction (when core 1 is `Running`). `steps`/`now_cycles`/`budget` only
  count core-0 steps; a halted core 1 contributes its state and nothing else.
  `Machine::step`、`trace_step`、`run_cycles` 与 GDB `resume` 都先退役一条
  core0 指令，再退役一条 core1 指令（core1 处于 `Running` 时）。
  `steps`/`now_cycles`/`budget` 只统计 core0 步数；停机的 core1 只贡献状态。
- **Fault isolation / 故障隔离**：a core-1 bus error or undefined instruction
  halts **core 1 only** and is reported by `core1_halt_reason()` and
  `[run] core1 halt: …`; it never sets the machine's `halt_reason` and never
  stops core 0.
  **故障隔离**：core1 的总线错误或未定义指令**只停机 core1**，由
  `core1_halt_reason()` 与 `[run] core1 halt: …` 报告；它绝不设置机器的
  `halt_reason`，也不停止 core0。
- **Per-core register state / 每核寄存器状态**：the SIO view is switched to
  `Core1` around core 1's instruction, and PPB system writes (VTOR, CPACR,
  NVIC, SysTick) are replayed into the core that executed them, so the two
  cores keep independent vector tables, FPU enable state and NVIC enable masks.
  core1 指令前后把 SIO 视图切到 `Core1`，PPB 系统写入（VTOR、CPACR、NVIC、
  SysTick）回放到实际执行它的核心，因此两核各自保有独立的向量表、FPU 使能与
  NVIC 使能掩码。
- **IRQ routing / 中断路由**：every shared `IrqAssert` line is delivered to both
  NVICs (each core's enables decide whether it is taken), except
  `SIO_IRQ_FIFO`/`SIO_IRQ_BELL`, which are per-core endpoints fed by
  `BootPeripherals::core1_irq_mask`. Core 1 therefore runs HSTX's `DMA_IRQ_0`
  handler while core 0 keeps its own. Core-1 deliveries are logged as
  `[irq] core1 signal <n> at PC=…`.
  **中断路由**：共享 `IrqAssert` 的每条线都投递到两个 NVIC（由各核心的使能位
  决定是否响应），但 `SIO_IRQ_FIFO`/`SIO_IRQ_BELL` 是按核端点，由
  `BootPeripherals::core1_irq_mask` 提供。因此 core1 能运行 HSTX 的
  `DMA_IRQ_0` 处理程序，core0 保有自己的中断。core1 的投递记录为
  `[irq] core1 signal <n> at PC=…`。
- **PSM reset semantics / PSM 复位语义**：setting `PSM_FRCE_OFF.PROC1`
  hard-resets core 1 — the machine discards its context, re-arms the boot-ROM
  echo stub and clears the launch state; clearing the bit releases it again and
  the peer acknowledges with a `0` frame (what `multicore_reset_core1()`
  waits for) unless a new launch was recorded.
  **PSM 复位语义**：置位 `PSM_FRCE_OFF.PROC1` 会硬复位 core1 —— 机器丢弃其
  上下文、重新启用 boot ROM 回显桩并清除启动状态；清除该位会再次释放，对端回送
  一帧 `0`（`multicore_reset_core1()` 等待的确认），除非已记录新的启动信息。
- **Modelled / 已建模**：SIO hardware spinlocks (`SPINLOCK0..31` at `0x100+4n`,
  claim-on-read / release-on-write, real `SPINLOCK_ST`) and the per-core
  `LDREX`/`STREX` exclusive monitor with `CLREX`.
  **已建模**：SIO 硬件自旋锁（`SPINLOCK0..31` 位于 `0x100+4n`，读即取/写即放，
  `SPINLOCK_ST` 为真实位图）与每核 `LDREX`/`STREX` 独占监视器 + `CLREX`。
- **Not modelled / 未建模项**：DMA arbitration between cores, `WFI`/`WFE`/`SEV`
  as scheduling points (still no-ops in the current tree; the sleep work landed as
  `9a6b088` and was reverted pending blocking `SIO_FIFO_RD`/FIFO-IRQ semantics),
  core-1 writable registers and true per-core stepping: GDB now exposes core 1 as
  thread 2 with read-only PC/SP, and `Machine::step` still advances both cores in
  lock-step. Core 1 has no
  modelled mask ROM: a launch jumps straight to `entry`, reproducing the post-ROM
  state, and only the trap stubs (`rom_table_lookup` at `0x74`, the ROM function
  table) are dispatched.
  **未建模项**：双核之间的 DMA 仲裁、作为调度点的 `WFI`/`WFE`/`SEV`（当前仍为空
  操作；睡眠语义曾以 `9a6b088` 落地但因缺少阻塞式 `SIO_FIFO_RD`/FIFO IRQ 而回退）、
  core1 可写寄存器与真正的按核步进：GDB 现已把 core1 暴露为线程 2（PC/SP 只读），
  而 `Machine::step` 仍双核同步推进。core1 不建模掩膜 ROM：启动直接跳到
  `entry` 并复现 post-ROM 状态，仅分发陷阱桩（`0x74` 的 `rom_table_lookup` 与
  ROM 函数表）。

### DMA pump / DMA 泵送

Firmware only programs the 16-channel DMA register block at `0x5000_0000`;
transfers happen because the machine pumps them, not because the guest writes
`CTRL_TRIG`.
固件只写 `0x5000_0000` 的 16 通道 DMA 寄存器块；传输之所以发生，是机器在泵送
它们，而不是客户机写了 `CTRL_TRIG`。

- **Pump point / 泵送点**：`Machine::pump_dma_batch` is called from
  `execute_step` immediately after a step retires and **before** interrupt
  delivery. Plain `step`, `trace_step`, boot-ROM trap steps and GDB `resume`
  single steps all funnel through this one call, so a debugger cannot starve
  the DMA engine.
  `Machine::pump_dma_batch` 在一步退役后、**中断投递之前**由 `execute_step`
  调用。普通 `step`、`trace_step`、boot ROM 陷阱步与 GDB `resume` 单步都汇入
  这一处调用，调试器因此无法让 DMA 引擎停摆。
- **PIO DREQ / PIO DREQ 门控**：the batch runs through
  `BootPeripherals::pump_dma`, which republishes every PIO FIFO level
  (`sync_pio_dreqs`) before touching the DMA controller. Channels whose
  `TREQ_SEL` selects a PIO FIFO (`0..=23`, `PIO0_TX0` … `PIO2_RX3`) advance one
  unit at a time with a fresh level sample, so `PIOx_TXy` only writes while that
  TX FIFO is not full and `PIOx_RXy` only reads while the RX FIFO is not empty;
  joined FIFOs gate at their 8-word depth. Channels using other selectors still
  consume the whole remaining budget in one `Dma::run` call.
  该批次经 `BootPeripherals::pump_dma` 执行：访问 DMA 控制器之前先重新发布全部
  PIO FIFO 电平（`sync_pio_dreqs`）。`TREQ_SEL` 指向 PIO FIFO（`0..=23`，
  `PIO0_TX0` … `PIO2_RX3`）的通道逐单元推进并重新采样电平，因此 `PIOx_TXy`
  只在该 TX FIFO 未满时写入、`PIOx_RXy` 只在 RX FIFO 非空时读出；合并 FIFO 按
  8 深度门控。使用其它选择器的通道仍可在一次 `Dma::run` 中用完整剩余预算。
- **Budget / 预算**：throttled to one call per `PUMP_INTERVAL` (64) retired
  steps and capped at `DMA_PUMP_TRANSFERS` (16) data units per call. The DMA
  controller visits channels round-robin, so 16 units is at most one unit per
  channel and never more than 16 units for a single busy channel: the CPU
  always executes at least 64 instructions between batches and at least four
  instructions per DMA unit.
  按退役步数每 `PUMP_INTERVAL`（64）步节流一次，每次最多
  `DMA_PUMP_TRANSFERS`（16）个数据单元。DMA 控制器按轮转顺序访问通道，因此
  16 个单元最多是每通道一个单元、单一忙碌通道也不超过 16 个单元：CPU 在批次
  之间至少执行 64 条指令，平均每个 DMA 单元至少 4 条指令。
- **Idle path / 空闲路径**：the pump records its throttle point, probes
  `Dma::busy_mask()` once and returns when no channel is busy — no bus access
  and no register parsing on the hot path.
  泵送先记录节流点，再探测一次 `Dma::busy_mask()`，无忙碌通道即返回：热路径
  不访问总线、不解析寄存器。
- **Diagnostics / 诊断**：`Machine::dma_transfers()` returns the cumulative
  number of data units moved (one per performed transfer, not per byte) and
  `Machine::dma_state()` renders `dma=<units> busy=<mask>`. `run` prints that
  line as `[run] dma=… busy=…` next to the register snapshot. It is a host
  diagnostic only: `repl --exec` stdout still carries guest bytes only.
  `Machine::dma_transfers()` 返回累计搬运的数据单元数（按传输计，不按字节），
  `Machine::dma_state()` 渲染为 `dma=<单元数> busy=<掩码>`。`run` 在寄存器快照
  旁打印 `[run] dma=… busy=…`。它只是宿主诊断：`repl --exec` 的 stdout 仍只
  承载客户机字节。
- **Limitations / 限制**：no cycle-accurate arbitration or `HIGH_PRIORITY`
  scheduling, no DREQ pacing-timer (`TREQ_SEL` `0x3B..=0x3E`) tick driving
  and no HSTX DMA handshake. PIO DREQ gating is unit-granular inside a batch
  but the batch itself is throttled to 64 steps, and PIO pin FUNCSEL routing is
  not modelled, so a PIO+DMA stream is not cycle-exact. Every `Dma::run` unit
  costs one bus read plus one bus write, but the instruction budget and
  peripheral timebase only count CPU steps, so DMA traffic is not added to
  `steps` or `time_us`.
  不做周期精确仲裁或 `HIGH_PRIORITY` 调度，不驱动 DREQ 节拍定时器
  （`TREQ_SEL` `0x3B..=0x3E`），也不建模 HSTX DMA 握手。PIO DREQ 门控在批次内
  按单元生效，但批次本身按 64 步节流，且未建模 PIO 引脚的 FUNCSEL 路由，因此
  PIO+DMA 数据流不是周期精确的。每个 `Dma::run` 单元消耗一次总线读加一次总线
  写，但指令预算与外设时基只统计 CPU 步数，DMA 流量不计入 `steps` 或 `time_us`。

### PIO pump / PIO 泵送

Firmware only writes the PIO register blocks at `0x5020_0000` / `0x5030_0000` /
`0x5040_0000`; the state machines run because the machine ticks them, not
because the guest writes `CTRL.SM_ENABLE`.
固件只写 `0x5020_0000` / `0x5030_0000` / `0x5040_0000` 的 PIO 寄存器块；状态机
之所以运行，是因为机器在 tick 它们，而不是因为客户机写了 `CTRL.SM_ENABLE`。

- **Pump point / 泵送点**：`Machine::pump_pio_batch` is called from
  `execute_step` right next to `pump_dma_batch`, after the step retires and
  **before** interrupt delivery. Plain `step`, `trace_step`, boot-ROM trap steps
  and GDB `resume` all take this path, so a debugger cannot starve the PIO
  state machines either.
  `Machine::pump_pio_batch` 在一步退役后、**中断投递之前**、紧邻
  `pump_dma_batch` 由 `execute_step` 调用。普通 `step`、`trace_step`、boot ROM
  陷阱步与 GDB `resume` 都经过这里，因此调试器同样无法让 PIO 状态机停摆。
- **Budget / 预算**：throttled to one call per `PUMP_INTERVAL` (64) retired
  steps. Each call ticks every block that has `CTRL.SM_ENABLE` set by the number
  of steps retired since the previous call, so one guest step is modelled as one
  PIO clock cycle. Only blocks with at least one enabled machine are locked and
  ticked; `IRQ`-driven interrupt lines are republished only when the flags
  changed.
  按退役步数每 `PUMP_INTERVAL`（64）步节流一次。每次把自上次调用以来退役的步数
  作为周期数，tick 每个置位了 `CTRL.SM_ENABLE` 的块，因此一个客户机步长被视为
  一个 PIO 时钟周期。只有含已使能状态机的块才会被加锁与 tick；由 `IRQ` 标志
  驱动的中断线仅在标志变化时重新发布。
- **Idle path / 空闲路径**：`Machine::pio_busy_mask()` probes
  `CTRL.SM_ENABLE` of the three blocks; when it is zero the batch returns after
  that single probe, without per-block ticks. An unused PIO therefore costs one
  mask probe per 64 steps and never touches the bus.
  `Machine::pio_busy_mask()` 探测三个块的 `CTRL.SM_ENABLE`；为零时本批在该次
  探测后直接返回，不再逐块 tick。因此未使用的 PIO 每 64 步只花一次掩码探测，
  且完全不访问总线。
- **Diagnostics / 诊断**：`Machine::pio_busy_mask()` packs
  `CTRL.SM_ENABLE` as bit `block * 4 + sm`, and `Machine::pio_pc(block, sm)` (a
  test/diagnostic hook) reports a machine's program counter. Both are host-only
  and never print on `repl --exec` stdout, so guest output stays byte-exact.
  `Machine::pio_busy_mask()` 以位 `block * 4 + sm` 打包 `CTRL.SM_ENABLE`，
  `Machine::pio_pc(block, sm)`（测试/诊断接口）报告状态机 PC。两者都只属于宿主，
  绝不出现在 `repl --exec` 的 stdout 上，客户机输出保持逐字节一致。
- **DMA handshake / DMA 握手**：after ticking, the FIFO levels are not pushed
  by the PIO pump itself; the following DMA batch republishes them via
  `BootPeripherals::sync_pio_dreqs()`. A PIO TX FIFO drained in this batch is
  therefore visible to a `PIOx_TXy`-paced DMA channel at the next pump, one
  `PUMP_INTERVAL` later. This keeps the idle PIO path free of extra locks.
  推进状态机后，PIO 泵送本身不发布 FIFO 电平；随后的 DMA 批次通过
  `BootPeripherals::sync_pio_dreqs()` 发布。因此本批次排空的 TX FIFO 会在下一次
  泵送（一个 `PUMP_INTERVAL` 之后）才对 `PIOx_TXy` 节拍的 DMA 通道可见，从而让
  空闲 PIO 路径不增加额外加锁。
- **Limitations / 限制**：the PIO clock is assumed to equal the CPU clock (one
  step = one PIO cycle), so a `CLKDIV` value scales instructions per PIO cycle
  but not the PIO/CPU clock ratio. PIO DREQ levels reach the DMA controller at
  pump-batch granularity (see the DMA pump section), there is no PINCTRL
  FUNCSEL crossbar between PIO pins and GPIO pads, and no external GPIO device
  model. The PIO tick is not added to `steps` or `time_us`, exactly like DMA
  traffic.
  假定 PIO 时钟等于 CPU 时钟（一步 = 一个 PIO 周期），因此 `CLKDIV` 只缩放每个
  PIO 周期的指令数，不改变 PIO/CPU 时钟比。PIO DREQ 电平以泵送批次为粒度到达
  DMA 控制器（见 DMA 泵送一节）；不做 PIO 引脚与 GPIO 焊盘之间的 PINCTRL
  FUNCSEL 交叉开关，也不模拟外部 GPIO 设备。与 DMA 流量一样，PIO tick 不计入
  `steps` 或 `time_us`。

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

# One hot-reload pass over the plugin directory, then exit
rp2350 list-devices --plugin-dir target/release --reload

# Keep polling for rebuilt plugins (Ctrl-C to stop)
rp2350 list-devices --plugin-dir target/release --watch

# Wire the LED plugin's port 0 to GPIO25 and print the wiring table
rp2350 wire --plugin-dir target/release --device com.rp2350-rs.led --port 0 --gpio 25

# Serve the MCP JSON-RPC server over HTTP on port 18080
rp2350 mcp-serve --port 18080

# Same, requiring `Authorization: Bearer s3cret` on POST /mcp (/health stays open)
$env:RP2350_MCP_TOKEN = 's3cret'; rp2350 mcp-serve --port 18080

# Explicit flag wins over the environment; an empty flag disables auth
rp2350 mcp-serve --port 18080 --auth-token s3cret
```
