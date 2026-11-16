# rp2350-cli — Public API / 公共 API

> Command-line front end for the RP2350 simulator. Builds the machine bus,
> boots Arm Cortex-M33 or Hazard3 RISC-V firmware, and drives it either as a
> traced `run`, an interactive USB CDC raw REPL (`repl`), or a benchmark
> harness (`perfbench`).
> RP2350 模拟器的命令行前端。构建机器总线，启动 Arm Cortex-M33 或 Hazard3
> RISC-V 固件，并按三种模式驱动：带轨迹的 `run`、交互式 USB CDC raw REPL
> （`repl`）、基准测试（`perfbench`）。
>
> **Status**: implemented — `run` / `repl` / `perfbench` / `play` / `gdb` /
> `list-devices` / `wire` / `mcp-serve`.
> **状态**: 已实现 — `run` / `repl` / `perfbench` / `play` / `gdb` /
> `list-devices` / `wire` / `mcp-serve`。

---

## Subcommands / 子命令

Global flags: `--verbose` (log level `debug` instead of `info`) and
`--headless` (accepted on every subcommand; the CLI has no GUI today).
全局标志：`--verbose`（日志级别从 `info` 变为 `debug`）与 `--headless`
（所有子命令都接受；目前 CLI 本身没有 GUI）。

#### PSRAM window and size syntax / PSRAM 窗口与大小语法

`run`, `repl`, `play` and `gdb` accept `--psram <SIZE>`, which mounts the
built-in PSRAM model on the XIP chip-select-1 window at `0x1100_0000`
(`XIP_MAIN1`; it is **not** an alias of the CS0 flash window at `0x1000_0000`).
The window is guest-writable RAM, while flash stays read-only and untouched.
`run`、`repl`、`play`、`gdb` 都接受 `--psram <SIZE>`，在 XIP 片选 1 的
`0x1100_0000`（`XIP_MAIN1`，**不是** `0x1000_0000` 处 CS0 Flash 窗口的别名）
挂载内置 PSRAM 模型。窗口对客户机可写，Flash 保持只读且不受影响。

- **Syntax / 语法**：plain bytes (`8388608`, `0x800000`) or a `K`/`M` suffix
  (`8192K`, `8M`), with optional `B`/`iB` (`16MB`, `16MiB`). Suffixes are
  case-insensitive and always powers of two: 1 K = 1024 B, 1 M = 1024 K.
  纯字节数（`8388608`、`0x800000`）或带 `K`/`M` 后缀（`8192K`、`8M`），后缀可
  加 `B`/`iB`（`16MB`、`16MiB`）。后缀不区分大小写，且一律按 2 的幂解释：
  1 K = 1024 B，1 M = 1024 K。
- **Validation / 校验**：`0`, negative numbers, unknown suffixes, empty values
  and values that overflow `u64`/`usize` are rejected at parse time (clap exits
  with status 2 before any firmware is read). A value above the 16 MiB
  architectural ceiling, or one that is not 4 KiB aligned, is **clamped to
  16 MiB** by the model with a single `psram:` warning — the run still starts,
  nothing is truncated to zero.
  `0`、负数、未知后缀、空值以及溢出 `u64`/`usize` 的值在解析阶段即被拒绝（clap 在
  读取固件前就以退出码 2 结束）。超过 16 MiB 架构上限或未按 4 KiB 对齐的值由模型
  **钳制到 16 MiB** 并只打印一次 `psram:` 警告：运行照常开始，窗口绝不会被截断
  成 0。
- **Not enabled / 未启用**：without `--psram` (and without `psram` in a profile)
  the window is **not** registered, so guest accesses to `0x1100_0000` report
  `BusErrorKind::NoDevice` exactly as before the option existed.
  不给 `--psram`（profile 中也没有 `psram`）时该窗口**不注册**，客户机访问
  `0x1100_0000` 仍报 `BusErrorKind::NoDevice`，与该选项存在前完全一致。
- **Not modelled / 未建模**：QMI CS1 timing/format registers (`M1_TIMING`,
  `M1_RFMT`, `M1_RCMD`) stay plain storage; the non-cached PSRAM alias at
  `0x1500_0000` is not mapped. See `doc/crates/rp2350-periph.md`.
  QMI 的 CS1 时序/格式寄存器（`M1_TIMING`、`M1_RFMT`、`M1_RCMD`）仍是普通存储；
  `0x1500_0000` 处的非缓存 PSRAM 别名未映射。详见 `doc/crates/rp2350-periph.md`。

```bash
# 8 MB PSRAM behind CS1 / 在 CS1 后挂载 8 MB PSRAM
rp2350 run --file firmware.uf2 --psram 8M
# same size in bytes / 以字节书写同一大小
rp2350 run --file firmware.uf2 --psram 8388608
```

### `run` — traced execution / 带轨迹运行

```text
rp2350 run --file <FIRMWARE> [--arch arm|riscv] [--max-cycles N]
           [--trace-steps N] [--strict-decode] [--sleep-fast-forward on|off]
           [--core-threads on|off] [--core-window N]
           [--psram SIZE]
```

| Option / 选项 | Default / 默认 | Meaning / 含义 |
|---------------|----------------|----------------|
| `--file` | required | Firmware, UF2 or ELF / 固件，UF2 或 ELF |
| `--arch` | auto | Override the detected architecture / 覆盖检测到的架构 |
| `--max-cycles` | `20_000_000` | Instruction budget / 指令预算 |
| `--trace-steps` | `16` | Boot-trace steps printed / 打印的启动轨迹步数 |
| `--strict-decode` | off | Halt on undefined instructions instead of delivering HardFault / 未定义指令停机，而不是投递 HardFault |
| `--sleep-fast-forward` | `on` | Batch sleeping steps between events; `off` restores the pre-fast-forward path for A/B runs / 在事件之间批量推进睡眠步；`off` 恢复快进前的路径供 A/B 对照 |
| `--core-threads` | `off` | Run core0/core1 on two host threads; see “Optional two-thread cores” below / 在两条宿主线程上运行 core0/core1，见下文「可选双核线程」 |
| `--core-window` | `64` | Machine steps between cross-core synchronisations; accepted range `1..=4096` / 跨核同步之间的机器步数，可接受范围 `1..=4096` |
| `--psram` | off (unmapped) | Mount an external PSRAM window at `0x1100_0000`; see “PSRAM window and size syntax” below / 在 `0x1100_0000` 挂载外部 PSRAM 窗口，见下文「PSRAM 窗口与大小语法」 |
| `--dump` | none | `ADDR:LEN:PATH` memory dump written when the run stops (repeatable, decimal or `0x` hex) / 停机时写入的内存导出，可重复，支持十进制或 `0x` 十六进制 |

Without `--strict-decode` an undefined Arm instruction follows the architecture:
`CFSR.UFSR.UNDEFINSTR` is set and the core enters HardFault (or UsageFault when
firmware enabled `SHCSR.USGFAULTENA`) at `VTOR + 0x0C`; the handler's return
address is the undefined instruction itself, so firmware can fix it up. The
flag is a host diagnostic that turns the same event into an immediate halt with
`undefined instruction at PC ...` in the error, which is the recommended mode
when bringing up a new image.
不带 `--strict-decode` 时，未定义 ARM 指令遵循架构：置位
`CFSR.UFSR.UNDEFINSTR`，核心从 `VTOR + 0x0C` 进入 HardFault（固件使能
`SHCSR.USGFAULTENA` 时为 UsageFault）；处理程序的返回地址就是未定义指令本身，
固件可自行修正。该开关是宿主诊断选项，把同一事件变为立即停机，并在错误中打印
`undefined instruction at PC ...`，适合新映像的启动调试。

`run` prints the image summary, core description and boot vectors, executes
the first `trace_steps` steps as `[trace]` lines, then runs to halt or budget
exhaustion. The report ends with the halt reason, a 32-step `[tail]` PC
history, the simulated time, up to 8 logged IRQ deliveries, and any buffered
UART0 output. `simulated=<µs>` comes from the live time base, so the same
`--max-cycles` covers less simulated time once the guest raises `CLK_SYS`
(252 MHz: 252 M steps per second; 150 MHz: 150 M). Arm machines also print one
`[run] core1 state=… pc=… sp=… launches=…` line (plus `[run] core1 halt: …` if
core 1 stopped) after the DMA/HSTX counters.
`run` 打印映像摘要、核心描述与启动向量，先按 `[trace]` 行执行前
`trace_steps` 步，然后运行至停机或预算耗尽。报告以停机原因、32 步 `[tail]`
PC 轨迹、模拟时间、最多 8 条已记录的中断投递以及缓冲的 UART0 输出收尾。
`simulated=<µs>` 来自实时时基，因此客户机升频后同样 `--max-cycles` 覆盖的模拟时间
会变短（252 MHz 每秒 252 M 步，150 MHz 每秒 150 M 步）。Arm 机器还会在 DMA/HSTX
计数之后打印一行 `[run] core1 state=… pc=… sp=… launches=…`（core1 停机时另有
`[run] core1 halt: …`）。

The diagnostic block is `[run] dma=<units> busy=<mask>`, then
`[run] hstx words=<expanded> underruns=<episodes> fifo=<level> enabled=<bool>
wof=<bool> unknown=<count>`, then the sleep counters
`[run] sleep core0 steps=<n> wakes=<n> core1 steps=<n> wakes=<n>`, then three
`[run] dma chN rd=… wr=… cnt=… ctrl=…
busy=… chain=… treq=…` channel snapshots, then the core-1 line. `Machine` pumps
HSTX on the DMA batch cadence (every 64 steps) with the periph pixel-clock
pacing, so a healthy 60 Hz scanout advances `words` by ~134 400 per frame with
`underruns=0`, `wof=false` and `unknown=0`.
诊断块依次为 `[run] dma=<单元数> busy=<掩码>`、
`[run] hstx words=<展开字数> underruns=<断流次数> fifo=<电平> enabled=<布尔>
wof=<布尔> unknown=<计数>`、睡眠计数
`[run] sleep core0 steps=<n> wakes=<n> core1 steps=<n> wakes=<n>`、三条
`[run] dma chN rd=… wr=… cnt=… ctrl=…
busy=… chain=… treq=…` 通道快照，最后是 core1 行。`Machine` 按 DMA 批次节拍
（每 64 步）以 periph 的像素时钟节拍泵送 HSTX，因此健康的 60 Hz 扫描每帧让
`words` 增加约 134 400，且 `underruns=0`、`wof=false`、`unknown=0`。

#### System reset requests / 系统复位请求

A firmware store of `VECTKEY | SYSRESETREQ` into `SCB_AIRCR` is replayed into
the core, where it latches a one-shot reset request. `run` prints
`[run] system reset requested (AIRCR.SYSRESETREQ)` and then rebuilds the whole
`Machine` from the same image (`[run] reboot 1/3: PC=… SP=…`): flash, SRAM,
clocks, peripherals, both cores and every step/IRQ/trap/diagnostic counter start
over. Up to **3** restarts are honoured; the next request stops the command with
a `reset storm: guest requested AIRCR.SYSRESETREQ 4 times, more than the 3
restart limit` error. `--dump` files describe the final boot.
固件向 `SCB_AIRCR` 写入 `VECTKEY | SYSRESETREQ` 后，写入会回放到核并锁存为一次性
复位请求。`run` 打印 `[run] system reset requested (AIRCR.SYSRESETREQ)`，随后用
同一映像重建整个 `Machine`（`[run] reboot 1/3: PC=… SP=…`）：Flash、SRAM、时钟、
外设、两个核心以及全部步数/中断/陷阱/诊断计数都从头开始。最多响应 **3** 次重启；
下一次请求以 `reset storm: guest requested AIRCR.SYSRESETREQ 4 times, more than
the 3 restart limit` 错误停止命令。`--dump` 写的是最后一次启动的内存。
A write without `VECTKEY = 0x05FA` is ignored by the core (and by the PPB
read-back cache), so it never reaches the latch.
无 `VECTKEY = 0x05FA` 的写入被核（以及 PPB 读回缓存）忽略，永远不会进入锁存。

Each `--dump` request then writes its bytes and prints
`[run] dump 0x<addr>+0x<len> -> <path> (<n> bytes)`. This is the fastest way to
capture a firmware framebuffer without attaching GDB; the concrete address of a
target's framebuffer belongs to that example's README (see
`example/gba/README.md` for `emerald.uf2`).
每条 `--dump` 请求随后写入字节并打印
`[run] dump 0x<addr>+0x<len> -> <path> (<n> bytes)`。这是不挂 GDB 也能抓取固件
帧缓冲的最快方式；具体目标帧缓冲地址属于该示例的 README（`emerald.uf2` 见
`example/gba/README.md`）。

### Optional two-thread cores / 可选双核线程

`--core-threads on` runs core0 and core1 on two host threads. The default
`off` path is unchanged and remains the deterministic acceptance baseline.
`--core-threads on` 让 core0/core1 运行在两条宿主线程上；默认 `off` 路径不变，
仍是确定性验收基准。

```text
# emerald: threaded cores with the default 64-step window
target\release\rp2350-cli.exe run --file "<emerald.uf2>" \
    --max-cycles 300000000 --trace-steps 0 --core-threads on
```

**Window model / 窗口模型.** A window is `--core-window N` machine steps
(default 64, range `1..=4096`). Inside a window both cores execute freely; each
instruction is really executed, nothing is skipped. At the boundary the caller
waits for both workers, settles the completed window (step/ sleep accounting,
halt/reset latches) and hands the next window to the workers. The boundary work
of a completed window — DMA/PIO/HSTX pumps, core-1 mailbox handshakes, IRQ
delivery, SIO/SEV forwarding and the 10 000-step timebase advance — then runs
**while the next window executes**, so it overlaps core execution. The device
handles and the shared bus are thread-safe; IRQ/event commands produced there
take effect two windows later. Cross-core events therefore have a latency of at
most two windows. The 64-step pump cadence is preserved for `--core-window`
values up to 64; larger windows coalesce two or more pump periods into one
boundary (see the measured table below).
**窗口模型.** 窗口为 `--core-window N` 个机器步（默认 64，范围 `1..=4096`）。
窗口内两核自由执行，每条指令都真实执行、绝不跳过。边界处调用方等待两个工作线程，
结算刚完成的窗口（步数/睡眠记账、停机/复位锁存），并把下一个窗口交给工作线程。
已完成窗口的边界工作——DMA/PIO/HSTX 泵送、core1 信箱握手、中断投递、SIO/SEV 转发
与 10 000 步时基推进——随后在**下一个窗口执行期间**运行，与核执行重叠。设备句柄与
共享总线是线程安全的；这里产生的中断/事件命令在两个窗口后生效。因此跨核事件延迟
最多两个窗口。窗口 ≤64 时保持 64 步泵送节奏；更大窗口会把两个或更多泵送周期合并到
一个边界（见下表实测）。

**Determinism boundary / 确定性边界.** Window boundaries are fixed by the
machine step counter, but the order of memory accesses **inside** a window is
decided by the host scheduler. `on` mode is therefore **not bit-reproducible**;
this is the explicit trade-off for interactive speed. The `off` path remains
deterministic and byte-identical to the historical baseline. For the same
reason the cache-line-level watchpoint state used by GDB is thread-local: use
`--core-threads off` when debugging watchpoints.
**确定性边界.** 窗口边界由机器步数固定，但窗口**内部**的内存访问顺序取决于宿主
调度。因此 `on` 模式**不是位可复现的**；这是为交互速度做的显式取舍。`off` 路径
保持确定性，并与历史基线逐字节一致。同样，GDB 使用的监视点状态是线程局部的：
调试监视点时请使用 `--core-threads off`。

**Execution and lifecycle / 执行与生命周期.** The two worker threads belong to
one `run_steps`/`run_cycles` batch: they are started when at least one core can
make progress in parallel, and joined before the call returns. Boot, a
not-yet-launched core1, and double sleep stay on the serial batch path, which
also keeps the sleep fast-forward. `reboot` preserves the mode and window.
`Machine::active_core_threads()` reports `0` outside a batch (per machine).
RISC-V images always use the serial path even when the flag is on.
**执行与生命周期.** 两个工作线程属于一次 `run_steps`/`run_cycles` 批次：当至少
一个核可并行推进时启动，并在调用返回前 join。启动阶段、core1 尚未启动以及双核睡眠
保持在串行批量路径（睡眠快进也继续生效）。`reboot` 保留模式与窗口。
`Machine::active_core_threads()` 在批次之外返回 `0`（按机器隔离）。RISC-V 映像
即使开关为 on 也走串行路径。

**Sharing / 共享.** The bus is built on `SharedBus`: 10-bank SRAM (`(addr>>2)%10`),
lock-free Flash reads and one mutex per other region (never a global bus lock).
PPB writes are captured per core and replayed into the writing core, so core0's
and core1's NVIC/VTOR/CPACR stay independent; SIO register accesses take a
dedicated view lock around `Sio::select_core`, so `FIFO_WR`/`FIFO_RD`/spinlocks
follow the correct core. The host-side peripheral pumps run only while both
workers are parked at a boundary.
**共享.** 总线基于 `SharedBus`：10 bank SRAM（`(addr>>2)%10`）、Flash 免锁读、
其余区域每区域一把互斥锁（绝不使用全局总线锁）。PPB 写入按核捕获并回放到写入核，
因此 core0/core1 的 NVIC/VTOR/CPACR 相互独立；SIO 寄存器访问在
`Sio::select_core` 外持有一把专用视图锁，使 `FIFO_WR`/`FIFO_RD`/自旋锁跟随正确
的核。宿主侧外设泵送只在两个工作线程都停在边界时执行。

Set `RP2350_THREAD_STATS=1` to print one host-side line per threaded batch
(`windows=… total=… wait=… boundary=…`) on stderr. It is diagnostic only and
never changes guest state.
设置 `RP2350_THREAD_STATS=1` 可在 stderr 上每个线程化批次打印一行宿主诊断
（`windows=… total=… wait=… boundary=…`），只影响诊断，不改变客户机状态。

Measured on the acceptance host (see `doc/performance.md` §12 for the full
table). “Counters” compares the emerald 300 M `time_us`/DMA/HSTX/sleep fields
with the serial run; `=` means identical.
验收机实测（完整表见 `doc/performance.md` §12）。“计数器”把 emerald 300 M 的
`time_us`/DMA/HSTX/睡眠字段与串行运行比较，`=` 表示完全一致。

| Window | Wall / 墙钟 | vs `off` | Counters / 计数器 |
|---|---|---|---|
| `off` (baseline) | 9.35 s | 1.00× | — |
| 32 | 5.56 s | 1.68× | `=` |
| 64 (default) | 5.15 s | 1.82× | `=` |
| 128 | 4.51 s | 2.07× | `=` |
| 256 | 4.16 s | 2.25× | DMA/HSTX under-fed |

- 窗口 64 的客户机计数器与 `off` **逐字段一致**（`time_us`、`dma`、`hstx`、
  睡眠步数；最终 PC 与 core1 PC 因窗口相位不同而不同，允许）。
- 窗口 32 的屏障/协调次数翻倍，仍比 `off` 快；窗口 128 更快，且 emerald 的
  DMA/HSTX 计数器仍一致；窗口 256 开始让 HSTX 欠载，因此不作为推荐值。
- phase 1（225–300 M）单独对比：`off` 约 7.4 s，`on`(64) 约 4.2 s，**≈1.8×**；
  整跑 1.82×。启动阶段（0–225 M）由 MT-3a 快进覆盖，`on` 与 `off` 都在 0.1 s 量级。
- Window 64 is the default because it keeps every 64-step pump boundary and the
  measured counters identical to the serial path. Window 128 is faster but
  doubles the event/pump latency; choose it for interactive play when
  throughput matters more than event latency.
- 窗口 64 作为默认值，因为它保留每个 64 步泵送边界、计数器与串行路径完全一致。
  窗口 128 更快，但事件/泵送延迟翻倍；交互游玩更看重吞吐时可选 128。
- Cross-core event latency is at most two windows: ~0.25 µs at window 32,
  ~0.5 µs at 64 and ~1 µs at 128 (252 MHz). All are far below host-visible
  interaction latency.
- 跨核事件延迟最多两个窗口：窗口 32 约 0.25 µs、64 约 0.5 µs、128 约 1 µs
  （252 MHz），都远低于宿主可感知的交互延迟。

### `repl` — USB CDC raw REPL / USB CDC raw REPL

```text
rp2350 repl --file <FIRMWARE> [--exec "CODE" | --stdio] [--max-cycles N]
            [--enum-cycles N] [--no-soft-reset] [--strict-decode]
            [--core-threads on|off] [--core-window N]
            [--arch arm|riscv] [--psram SIZE]
```

| Option / 选项 | Default / 默认 | Meaning / 含义 |
|---------------|----------------|----------------|
| `--file` | required | Firmware (MicroPython/CircuitPython UF2 or ELF) / 固件 |
| `--exec` | interactive | Python source to run once; conflicts with `--stdio` / 只执行一次的 Python 源码；与 `--stdio` 冲突 |
| `--stdio` | off | Transparent stdin/stdout bridge to the CDC bulk endpoints; conflicts with `--exec` / 在 stdin/stdout 与 CDC 批量端点之间透明桥接；与 `--exec` 冲突 |
| `--max-cycles` | `300_000_000` | Total simulated instruction budget; `0` means unlimited for `repl` / 总指令预算；`repl` 中 `0` 表示不限制 |
| `--enum-cycles` | `100_000_000` | Budget before USB enumeration must finish / USB 枚举需在此预算内完成 |
| `--no-soft-reset` | off | Skip the Ctrl-D soft reset on entry (ignored by `--stdio`) / 进入时跳过 Ctrl-D 软复位（`--stdio` 下忽略） |
| `--strict-decode` | off | Halt on undefined instructions instead of delivering HardFault / 未定义指令停机，而不是投递 HardFault |
| `--core-threads` / `--core-window` | `off` / `64` | Optional two-thread cores and their window; see “Optional two-thread cores” under `run` / 可选双核线程与其窗口，见 `run` 下的「可选双核线程」 |
| `--arch` | from UF2 family | Override the core architecture / 覆盖核心架构（`riscv` 为别名） |
| `--psram` | off (unmapped) | Mount the PSRAM window at `0x1100_0000` (same syntax as `run`) / 在 `0x1100_0000` 挂载 PSRAM 窗口（语法同 `run`） |

The subcommand boots the machine, runs the guest under the USB host bridge
until `HostProgress::Configured` (within `--enum-cycles`), enters the raw
REPL and then either executes `--exec` and prints stdout/stderr, or runs an
interactive line-based session. Enumeration failures print the host bridge's
`progress_line` snapshot plus the fault-tail diagnostics.
该子命令启动机器，在 USB 主机桥下运行客户机直至
`HostProgress::Configured`（须在 `--enum-cycles` 内），进入 raw REPL，随后
要么执行 `--exec` 并打印 stdout/stderr，要么进入交互式按行会话。枚举失败
会打印主机桥的 `progress_line` 快照与故障尾部诊断。

#### System reset requests / 系统复位请求

`repl` never reboots the machine behind the user's back. A guest store of
`VECTKEY | SYSRESETREQ` into `SCB_AIRCR` is replayed into the core, the machine
stops advancing, and the subcommand reports
`system reset requested (AIRCR.SYSRESETREQ)` on the diagnostic stream and exits
with an error. During enumeration this is a `SystemReset` enumeration error;
during the raw REPL phase the same diagnostic is printed before the fault-tail
dump. A write without `VECTKEY = 0x05FA` is ignored by the core and never reaches
the latch. Use `run` when the image is expected to reset itself.
`repl` 绝不在用户不知情时重启机器。客户机向 `SCB_AIRCR` 写入
`VECTKEY | SYSRESETREQ` 后，写入回放到核，机器停止推进，子命令在诊断流上报告
`system reset requested (AIRCR.SYSRESETREQ)` 并带错误退出。枚举期间这是
`SystemReset` 枚举错误；raw REPL 阶段会在故障尾部转储前打印同一条诊断。无
`VECTKEY = 0x05FA` 的写入被核忽略，永不进入锁存。若映像预期会自我复位，请使用
`run`。

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
  150 MHz or 126 000 at 252 MHz; the live `CLK_SYS` is injected into
  `CdcLink`). Smaller steps lower latency but add host loop overhead;
  100–1000 µs is the useful range.
  后台线程阻塞读取 stdin，并送入有界队列（16 槽 × 4 KiB = 64 KiB 在途）；
  主循环发送队列字节、把 CDC IN 排空到 stdout 并 flush，然后按
  `STDIO_CHUNK_MICROS`（500 微秒；150 MHz 下 75 000 个客户机周期，252 MHz 下
  126 000 个，实时 `CLK_SYS` 注入 `CdcLink`）推进模拟。更小的步长降低延迟但增加
  宿主循环开销；100–1000 微秒是有效范围。
- **EOF and budget / EOF 与预算**：after stdin EOF the bridge sends every
  queued byte and keeps simulating for a 50 ms (simulated) drain window so a
  late banner/prompt is not truncated, then exits with a stdin-byte count on
  stderr. The default `--max-cycles 300_000_000` is about two simulated seconds
  at 150 MHz but only ≈1.19 s at 252 MHz; use `time_us` or the live frequency
  when a wall-clock budget matters. Pass `--max-cycles 0` for an unlimited
  budget (`u64::MAX`). This `0` interpretation applies to `repl` (both `--exec`
  and `--stdio`); `repl --exec` keeps its historical default of `300_000_000`.
  stdin EOF 后桥接会把队列字节全部送出，并继续模拟 50 毫秒（模拟时间）的
  排空窗口，避免截断迟到的 banner/提示符，然后在 stderr 打印已处理的 stdin
  字节数并退出。默认 `--max-cycles 300_000_000` 在 150 MHz 下约两秒模拟时间，
  但在 252 MHz 下只有约 1.19 秒；需要墙钟预算时应按 `time_us` 或实时频率换算。
  传 `--max-cycles 0` 表示不限制预算（`u64::MAX`）。该 `0`
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

### `play` — background frame and input worker / 后台帧与输入 worker

```text
rp2350 play [--profile <FILE>] [--file <FIRMWARE>] [--fb <ADDR:WxH[:STRIDE]>]
            [--steps-per-frame N] [--frames N] [--buttons MASK]
            [--audio --audio-block N --audio-sm N] [--audio-rate HZ]
            [--audio-channels N] [--audio-capacity N] [--trace-steps N]
            [--max-cycles N] [--arch arm|riscv] [--strict-decode]
            [--core-threads on|off] [--core-window N] [--psram SIZE]
```

There are **no example-specific defaults** in the crate. The firmware path, the
frame buffer, the steps per frame and the I2S wiring must come from a profile
file or from the command line; without either the command fails with a message
naming the missing option. This follows the project rule that example constants
(frame buffer, button pins, I2S block/state machine/rate, firmware paths) live
in `example/`, CLI arguments or profile files only.
crate 内**没有示例专用默认值**。固件路径、帧缓冲、每帧步数与 I2S 接线必须来自
profile 文件或命令行；两者都没有时命令直接失败并指出缺失的选项。这遵循项目规则：
示例常量（帧缓冲、按键引脚、I2S 块/状态机/采样率、固件路径）只能出现在
`example/`、CLI 参数或 profile 文件中。

| Option / 选项 | Default / 默认 | Meaning / 含义 |
|---------------|----------------|----------------|
| `--profile` | none | `key = value` profile file; see below / profile 文件，格式见下 |
| `--file` | required without `--profile` | Firmware (UF2 or ELF) / 固件 |
| `--fb` | required without `--profile` | Guest frame buffer `ADDR:WxH[:STRIDE]`; `STRIDE` defaults to `W*2` / 客户机帧缓冲；跨度默认为 `W*2` |
| `--steps-per-frame` | required without `--profile` | Guest steps executed per published frame / 每发布一帧执行的步数 |
| `--frames` | unlimited | Stop after N frames (headless test mode) / 跑满 N 帧后停止（无头测试模式） |
| `--buttons` | `0` (drives no pin) | Bitmap of pins that act as buttons / 作为按键的引脚位图，`0` 表示不驱动任何引脚 |
| `--audio` | off | Install an `AudioTap`; needs `--audio-block` and `--audio-sm` / 安装 `AudioTap`，需要 `--audio-block`/`--audio-sm` |
| `--audio-block` / `--audio-sm` | required with `--audio` | PIO block and state machine that push the I2S words / 推送 I2S 字的 PIO 块与状态机 |
| `--audio-rate` / `--audio-channels` / `--audio-capacity` | `48000` / `2` / `48000` | Generic sink metadata (samples retained) / 通用 sink 元数据（保留样本数） |
| `--trace-steps` | `0` | Leading steps of the first frame printed as `[trace]` lines / 第一帧开头打印的 `[trace]` 步数 |
| `--max-cycles` | `u64::MAX` | Total instruction budget; `u64::MAX` = unlimited / 总指令预算，`u64::MAX` 表示不限制 |
| `--arch` / `--strict-decode` | as `run` | Architecture override / halt on undefined instructions instead of HardFault / 架构覆盖 / 未定义指令停机，而不是投递 HardFault |
| `--core-threads` / `--core-window` | `off` / profile or `64` | Optional two-thread cores and their window (profile keys `core_threads` / `core_window`); see “Optional two-thread cores” under `run` and “Batched frame stepping” below / 可选双核线程与其窗口（profile 键 `core_threads` / `core_window`），见 `run` 下的「可选双核线程」与下面的「批量帧步进」 |
| `--psram` | off (unmapped) | Mount the PSRAM window at `0x1100_0000` (same syntax as `run`; a profile may set `psram = 8M` instead) / 在 `0x1100_0000` 挂载 PSRAM 窗口（语法同 `run`；profile 也可用 `psram = 8M`） |

#### Profile format / profile 格式

One `key = value` per line; `#` starts a whole-line comment and blank lines are
ignored. Keys are trimmed, values keep their internal `=` (only the first `=`
separates), LF and CRLF both parse, and unknown keys, duplicate keys, missing
`=` and invalid values are errors that name the line number.
每行一个 `key = value`；`#` 开头为整行注释，空行忽略。键两侧空白被裁剪，值内部
可以包含 `=`（只有第一个 `=` 分隔键值），LF 与 CRLF 都可解析；未知键、重复键、
缺 `=` 与非法值都返回带行号的错误。

| Key / 键 | Value / 值 |
|----------|-----------|
| `firmware` | Firmware path / 固件路径 |
| `frame` | `ADDR:WxH[:STRIDE]` (decimal or `0x` hex address/stride) |
| `steps_per_frame` | Steps per published frame / 每帧步数 |
| `max_frames` | Frame limit (`--frames`) |
| `buttons` | `u16` bitmap, `0x` hex or decimal / 按键引脚位图 |
| `audio` | `true` / `false` |
| `audio_block` / `audio_sm` | PIO block and state machine / PIO 块与状态机 |
| `audio_rate` / `audio_channels` / `audio_capacity` | Sink metadata / sink 元数据 |
| `arch` | `arm` / `riscv` |
| `strict_decode` | `true` / `false` |
| `max_cycles` | Total instruction budget / 总指令预算 |
| `psram` | PSRAM window size at `0x1100_0000` (`8M`, `1024K`, `8388608`); absent = unmapped / `0x1100_0000` 处 PSRAM 窗口大小；缺省表示不挂载 |
| `core_threads` | `true` / `false`; two-thread core executor (default `false` = deterministic) / 双核线程执行器（默认 `false`，确定性） |
| `core_window` | `1`..=`4096`; machine steps per cross-core window (default `64`) / 每个跨核窗口的机器步数（默认 `64`） |

Precedence is **command line > profile > built-in generic default**. A profile
may set `audio = true` and still take `--audio-rate` from the command line; an
`audio = true` without any block/state machine (in either place) is an error at
resolution time.
优先级为**命令行 > profile > 内置通用默认值**。profile 可以设置 `audio = true`
而由命令行提供 `--audio-rate`；若两处都没有块/状态机，解析阶段直接报错。

#### Batched frame stepping / 批量帧步进

Each frame is **one** `Machine::run_steps(steps_per_frame)` call (the `[trace]`
prefix of the first frame still runs one `trace_step` at a time so the lines
match `run --trace-steps`). Splitting a frame into 4096-step calls used to pay
two host-thread spawns per slice, which is why the threaded executor could not
help `play`; one batch per frame lets it amortise the worker handshake over the
whole frame.
每帧是**一个** `Machine::run_steps(steps_per_frame)` 调用（第一帧的 `[trace]` 前缀
仍逐条 `trace_step`，使输出与 `run --trace-steps` 相同）。过去一帧被拆成 4096 步的
调用，每个切片都要启动两条宿主线程，线程化执行器因此帮不到 `play`；现在每帧一个
批次，工作线程握手成本摊薄到整帧。

Frame semantics are unchanged: the worker still advances exactly
`steps_per_frame` machine steps, publishes one frame, samples the buttons every
4096 steps and stops within ≤4096 steps of a host request. The sampling moved
into the machine: `Machine::set_host_input(pins, buttons, stop, interval)` makes
`run_steps` poll the mask and the stop flag on absolute step numbers — a hard
chunk/window boundary in both the serial and the threaded path — so a poll lands
on the same step as the old slice start. A host stop returns `CpuState::Halted`
**without** latching a halt reason (`play` reports it as a normal stop, never as
`halted`). Only the first frame with `--trace-steps > 0` shifts its in-batch
sampling points by the trace length.
帧语义不变：worker 仍精确推进 `steps_per_frame` 步、发布一帧、每 4096 步采样按键，
并在宿主请求后 ≤4096 步内停止。采样点移入机器：`Machine::set_host_input(pins,
buttons, stop, interval)` 让 `run_steps` 在绝对步号上轮询掩码与停止标志——串行与
线程化路径都把该步号当作硬切片/窗口边界——因此轮询点与旧的切片起点完全相同。宿主
停止返回 `CpuState::Halted` 但**不**锁存停机原因（`play` 把它当作正常停止，绝不记为
`halted`）。只有设置了 `--trace-steps > 0` 的第一帧，其批内采样点会因 trace 长度而
平移。

The default `--core-threads off` path stays deterministic and byte-identical to
the old slicing; `on` runs core0/core1 on two host threads and is explicitly not
bit-reproducible inside a window (see “Optional two-thread cores” under `run`).
默认 `--core-threads off` 路径保持确定性，并与旧切片逐字节一致；`on` 让 core0/core1
运行在两条宿主线程上，窗口内显式不可位复现（见 `run` 下的「可选双核线程」）。

Measured on `emerald.uf2` + `example/gba/profile.conf`, `--frames 130`, same
machine, quiet host, back-to-back: **off 41.7 s vs on 20.9 s (2.00×)**; the last
ten frames report **off 1.74–3.30 fps / 7.3–13.8 M steps/s** and **on 3.55–6.53
fps / 14.9–27.4 M steps/s**. Both runs retire exactly 546 M steps
(130 × 4.2 M). `on` is not bit-reproducible, so its per-frame spread varies
between runs; across repeated off/on pairs the wall-clock ratio ranged from
1.5× to 2.2× depending on where the host scheduler placed the two per-batch
worker threads (2.00× and 2.03× on quiet back-to-back pairs).
在 `emerald.uf2` + `example/gba/profile.conf`、`--frames 130`、同一台机器、宿主空闲
且背靠背运行：**off 41.7 s，on 20.9 s（2.00×）**；最后十帧为 **off 1.74–3.30 fps /
7.3–13.8 M steps/s**、**on 3.55–6.53 fps / 14.9–27.4 M steps/s**。两次运行都恰好执行
546 M 步（130 × 4.2 M）。`on` 不可位复现，单帧区间随运行变化；重复的 off/on 成对测量
墙钟比为 1.5–2.2×，取决于宿主调度把每批两条工作线程放到哪里（空闲背靠背时为
2.00× 与 2.03×）。

The subcommand boots the firmware on a worker thread, advances it by
`--steps-per-frame` steps per frame, copies the guest frame buffer and prints
one line per published frame:

```text
[play] frame=1 steps=2500000 fps=6.37 steps/s=15931096
[play] frame=2 steps=5000000 fps=6.33 steps/s=15820247
...
[play] summary frames=20 steps=50000000 fps=7.00 steps/s=17501742 halted=none
```

`--frames N` exits with status 0 after N frames. Without it the worker runs
until the machine halts or the process is killed. A frame buffer that cannot be
read completely (unmapped window) prints `[play] warn: frame buffer unreadable`
once and the affected pixels are black; nothing panics.

`play` also **never reboots**: a guest `AIRCR.SYSRESETREQ` makes the current
frame stop with `system reset requested (AIRCR.SYSRESETREQ)`, the worker
publishes the frame reached so far and the session ends with `halted=system
reset requested (AIRCR.SYSRESETREQ)` in `[play] summary`. This keeps the window,
input state and audio tap owned by the host instead of silently resetting the
image under the player.

Measured on `emerald.uf2` with `example/gba/profile.conf` (4.2 M steps per
frame): `--frames 130` reports **off 1.74–3.30 fps / 7.3–13.8 M steps/s** and
`--core-threads on` **3.55–6.53 fps / 14.9–27.4 M steps/s** on the last ten
frames (**41.7 s vs 20.9 s** for the whole run, 2.00×). The wide per-frame range
is the 1-second rolling window catching one or two frames, not a step-rate
change. See “Batched frame stepping” above.
在 `emerald.uf2` + `example/gba/profile.conf`（每帧 4.2 M 步）实测：`--frames 130`
的最后十帧为 **off 1.74–3.30 fps / 7.3–13.8 M steps/s**、`--core-threads on`
**3.55–6.53 fps / 14.9–27.4 M steps/s**（整跑 **41.7 s vs 20.9 s**，2.00×）。单帧
区间较宽是 1 秒滚动窗口只覆盖 1–2 帧造成的，不是步进速率变化。详见上文「批量帧步进」。
该子命令在 worker 线程上启动固件，每帧推进 `--steps-per-frame` 步，拷贝客户机
帧缓冲并为每帧打印一行。`--frames N` 跑满 N 帧后以状态码 0 退出；不带该参数时
运行到机器停机或进程被杀。帧缓冲读不全（窗口未映射）时只打印一次
`[play] warn: frame buffer unreadable`，缺失像素为黑色，不会 panic。
`play` 同样**绝不自动重启**：客户机发出 `AIRCR.SYSRESETREQ` 时当前帧以
`system reset requested (AIRCR.SYSRESETREQ)` 停止，worker 发布已到达的帧，会话以
`[play] summary` 中的 `halted=system reset requested (AIRCR.SYSRESETREQ)` 结束。
窗口、输入状态与音频 tap 始终由宿主掌控，不会在玩家不知情时重置映像。

#### Frame format and button semantics / 帧格式与按键语义

- **Frame format / 帧格式**：row-major little-endian RGB565, one `u16` per
  pixel, exactly as the guest stores it (`bytes[0]` is the low byte). Rows are
  `stride_bytes` apart, so a firmware that pads rows can be described with
  `:STRIDE`; the padding is skipped. `240x160` is `0x12C00` bytes at stride
  `480`.
  按行优先的小端 RGB565，每像素一个 `u16`，与客户机存储形式一致（`bytes[0]`
  为低字节）。行间距离为 `stride_bytes`，因此固件的行填充可用 `:STRIDE` 描述并
  被跳过。`240x160` 在跨度 `480` 下为 `0x12C00` 字节。
- **Buttons / 按键**：the mask is a `u16` bitmap (bit `n` = GP`n`). A set bit
  means *pressed*, which drives the pin **low** through
  `GpioPads::set_external_pin`; a clear bit means released and drives it
  **high** (the firmware enables internal pull-ups). Only pins present in
  `button_pins`/`--buttons` are driven, so other injected inputs are untouched;
  `--buttons` defaults to `0`, which drives nothing at all.
  The mask is re-read every 4096 guest steps (not once per frame), so a press
  reaches the firmware within one slice of guest time even when a frame takes
  85 ms; a fast tap inside one frame can no longer be swallowed.
  掩码为 `u16` 位图（位 `n` = GP`n`）。置位表示**按下**，通过
  `GpioPads::set_external_pin` 把引脚拉**低**；清零表示松开并把引脚拉**高**
  （固件启用内部上拉）。只驱动 `button_pins`/`--buttons` 中置位的引脚，其它注入
  输入不受影响；`--buttons` 默认为 `0`，即完全不驱动引脚。
  掩码**每 4096 客户机步**重新采样一次（不是每帧一次），因此即使
  一帧需要 85 ms，按下也能在一个切片内到达固件；帧内快速点按不再被吞掉。
- **Triple buffer / 三缓冲**：the worker publishes `Arc<Frame>` into a
  single-slot "latest frame" mailbox. The producer never blocks; a frame the
  consumer has not taken is replaced and counted (`replaced_frames`).
  worker 把 `Arc<Frame>` 发布到单槽「最新帧」信箱。生产者永不阻塞；消费者未取走
  的帧被替换并计数（`replaced_frames`）。
- **Pacing / 节拍**：if a frame finishes in less than 1/60 s the worker sleeps
  until the 60 fps boundary; a slower frame runs at full speed (no sleep).
  一帧耗时小于 1/60 秒时 worker 睡到 60 fps 边界；更慢的帧全速运行（不 sleep）。

#### Why only ~3 fps / 为什么只有约 3 fps

On the real game segment emerald needs 4.2 M machine steps per 16.67 ms frame
(252 MHz overclock), and the deterministic single-threaded path retires roughly
7–14 M steps/s there, so a frame takes 300–700 ms. `--core-threads on` roughly
doubles that (15–27 M steps/s, ~2×), which is the biggest single lever available
without a JIT. Playability still needs 30 fps (≥75 M steps/s) or 60 fps
(≥150 M steps/s); closing that gap stays on the P0 performance track in
`doc/playability-plan.md` (interpreter hot paths, JIT or frame-parallel
evaluation). The play worker deliberately adds almost no per-step work: a frame
costs one `read_block` (76.8 KB, copied with 32-bit bus reads), one RGB565
conversion and the in-batch input polls, all far below one frame of emulation.
真实游戏段中 emerald 每 16.67 ms 帧需要 4.2 M 机器步（252 MHz 超频），而确定性单线程
路径在该段约 7–14 M 步/秒，因此一帧需要 300–700 ms。`--core-threads on` 大约翻倍
（15–27 M 步/秒，≈2×），是不做 JIT 时最大的单项杠杆。可玩仍需 30 fps
（≥75 M 步/秒）或 60 fps（≥150 M 步/秒），差距由 `doc/playability-plan.md` 的 P0
性能线负责（解释器热路径、JIT 或帧级并行）。play worker 刻意几乎不增加每步开销：
每帧一次 `read_block`（76.8 KB，按 32 位总线读拷贝）、一次 RGB565 转换与批内输入
轮询，都远低于一帧仿真成本。

### `gdb` — GDB remote debug server / GDB 远程调试服务器

```text
rp2350 gdb --file <ELF> [--port 3333] [--psram SIZE]
```

| Option / 选项 | Default / 默认 | Meaning / 含义 |
|---------------|----------------|----------------|
| `--file` | required | Firmware (ELF; `--arch` is not exposed here) / 固件（ELF；此处不提供 `--arch`） |
| `--port` | `3333` | TCP listen port (`0.0.0.0`) / TCP 监听端口（绑定 `0.0.0.0`） |
| `--psram` | off (unmapped) | Mount the PSRAM window at `0x1100_0000` so a debugger can inspect CS1/PSRAM contents / 在 `0x1100_0000` 挂载 PSRAM 窗口，使调试器可检查 CS1/PSRAM 内容 |

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
(gdb) stepi                # 裸 stepi 可用：GDB 发 vCont;s:2;c，只推进 core1
(gdb) vCont;s:2            # 仅推进 core1 一条指令（core0 冻结）
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

- `resume` 通过 `Machine::step`（双核）或 `Machine::step_cores`（单核）执行，因此
  boot ROM 陷阱、中断泵送、PPB 系统写入回放与外设时基都保持有效；`StopReply` 的
  `Breakpoint`/`SingleStep`/
  `Watchpoint`/`ReadWatch`/`AccessWatch`/`Fault`/`Sleeping`/`Halted`/`Reset`/
  `InstructionLimit` 映射为对应 GDB 信号（`T05thread:<n>;` /
  `T05watch:<addr>;thread:<n>;` / `T05rwatch:<addr>;thread:<n>;` /
  `T05awatch:<addr>;thread:<n>;` / `T0bthread:<n>;` / `T02thread:<n>;`）。
  `resume` uses `Machine::step` (both cores) or `Machine::step_cores` (one core),
  so boot ROM traps, interrupt pumping, PPB
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
- 单步与继续按核执行：`vCont;s:2`、`vCont;c:2` 或 `thread 2` 后的 `s`/`c` 只让
  core1 逐条退役指令，core0 的 PC、寄存器与中断状态完全冻结（`vCont;s:1`/
  `c:1` 反之）。只有裸 `c`/`s`、`vCont;c`/`s` 与 `c:1;c:2` 这类「整个目标」请求
  才双核同步推进。单核继续用逐条 `Machine::step_cores` 循环实现（而非
  `run_cycles`），语义正确但吞吐低于双核路径。
  Single-steps and continues are per-core: `vCont;s:2`, `vCont;c:2` or `s`/`c`
  after `thread 2` retire only core-1 instructions, leaving core 0's PC,
  registers and interrupt state frozen (`vCont;s:1`/`c:1` is the mirror image).
  Only whole-target requests — bare `c`/`s`, `vCont;c`/`s` and `c:1;c:2` —
  advance both cores in lock-step. A single-core continue is implemented as a
  loop of `Machine::step_cores` calls (not `run_cycles`): correct semantics, but
  lower throughput than the dual-core path.
- 停止回复统一带 `thread:<n>;`：普通停止 `T05thread:1;`、监视点停止
  `T05watch:<addr>;thread:2;`，`?` 复用最近停止线程。`monitor info` 仍打印
  core0/core1 摘要。
  Stop replies always carry `thread:<n>;`: a normal stop is `T05thread:1;`, a
  watchpoint stop is `T05watch:<addr>;thread:2;`, and `?` reuses the last stopped
  thread. `monitor info` still prints the core 0/1 summary.
- 混合动作列表已支持：all-stop 模式下线程 2 的裸 `stepi` 会发送 `vCont;s:2;c`，
  RSP 层按“先单步、继续集合保持停止”执行，因此 core1 前进一条指令后回复
  `T05thread:2;`，core0 完全不动。无需再 `set scheduler-locking step`；`s:1;s:2`
  这类多线程单步、`c:1;s:2` 与 `c;c`（等价裸 `c`）同样可用，重复线程号 / 未知
  动作 / 空列表 / 越界线程号一律 `E01`。
  Mixed action lists are supported: in all-stop mode a bare `stepi` on thread 2
  sends `vCont;s:2;c`, and the RSP layer steps first while keeping the continue
  set stopped, so core 1 advances one instruction and answers `T05thread:2;`
  while core 0 stays frozen. `set scheduler-locking step` is no longer needed;
  `s:1;s:2`, `c:1;s:2` and `c;c` (identical to a bare `c`) work too, and
  duplicate ids, unknown actions, empty lists and out-of-range ids reply `E01`.
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
- 已支持双核线程模型（core0/core1）、真正的按核 `vCont`/`Hc` 选择与 `vCont` 混合
  动作列表（`vCont;s:2;c` 只单步 core1、core0 冻结）：单核 `s`/`c` 只推进选中核，
  停止回复的线程号与该核一致。单核继续以逐条 `step_cores` 循环实现，吞吐低于整目标
  `c`；单客户端串行、无 TLS；细节见 `doc/crates/rp2350-gdb.md`。
  The dual-core thread model (core 0/core 1), true per-core `vCont`/`Hc` selection
  and mixed `vCont` action lists (`vCont;s:2;c` steps only core 1 and freezes
  core 0) are supported: a single-core `s`/`c` advances only the selected core,
  and the stop reply names that core. A single-core continue loops over
  `step_cores`, so it is slower than a whole-target `c`. One sequential client
  and no TLS; see `doc/crates/rp2350-gdb.md`.
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
    pub psram: Option<PsramOptions>,         // PSRAM window at 0x1100_0000
}

pub struct PsramOptions {
    pub size_bytes: usize,
}
impl PsramOptions {
    pub fn parse(spec: &str) -> Result<Self, String>;  // `8M`, `1024K`, `8388608`
}

pub struct Machine { /* bus, bootrom, core, core1, peripherals, usb … */ }
impl Machine {
    pub fn new(bytes: &[u8], options: &MachineOptions) -> Result<Self>;
    pub fn new_with_core_threads(
        bytes: &[u8],
        options: &MachineOptions,
        threads: CoreThreadOptions,
    ) -> Result<Self>;
    pub fn step(&mut self) -> Result<CpuState>;   // both runnable cores
    pub fn step_cores(&mut self, core0: bool, core1: bool) -> Result<CpuState>;
    pub fn trace_step(&mut self, index: usize) -> Result<CpuState>;  // prints [trace]
    pub fn run_steps(&mut self, max_steps: u64) -> Result<CpuState>; // batched stepping
    pub fn set_host_input(                 // in-batch input polling for play
        &mut self,
        pins: u16,
        buttons: Arc<AtomicU16>,
        stop: Arc<AtomicBool>,
        interval: u64,
    );
    pub fn clear_host_input(&mut self);
    pub fn next_host_poll(&self) -> Option<u64>;  // absolute step of the next poll
    pub fn run_cycles(&mut self, cycles: u64);   // CdcLink cycle callback, both cores
    pub fn read_block(&mut self, addr: u32, len: usize) -> Vec<u8>;  // frame copies
    pub fn finalize(&mut self);                  // flush timebase + UART
    // threading: core_threads_enabled, core_window, set_core_window,
    // active_core_threads (0 outside a threaded batch)
    // diagnostics: image_description, core_description, boot_vectors, pc, sp,
    // steps, now_cycles, time_us, budget, halt_reason, usb, irqs,
    // dma_transfers, dma_state, hstx_words, hstx_state, pio_busy_mask,
    // sleep_counters, sleep_state, sleep_fast_forward, set_sleep_fast_forward,
    // sleep_fast_forward_steps,
    // pio_pc (test hook), core1_state, core1_pc, core1_sp, core1_launch_count,
    // core1_halt_reason, take_uart_output, take_notices, notices_pending,
    // recent_steps, register, registers, dump_memory
    // debugger: read_register, write_register, xpsr, write_xpsr, cpu_state
}

pub struct CoreThreadOptions { pub enabled: bool, pub window: u32 } // default off / 64

pub struct RunOptions {
    pub file: PathBuf,
    pub arch: Option<ArchArg>,
    pub max_cycles: u64,
    pub trace_steps: usize,
    pub strict_decode: bool,
    pub sleep_fast_forward: Option<bool>, // None = env/default (`on`)
    pub core_threads: CoreThreadOptions,  // `--core-threads` / `--core-window`
    pub psram: Option<PsramOptions>,
    pub dumps: Vec<MemoryDump>,       // --dump ADDR:LEN:PATH (repeatable)
}

impl MemoryDump {
    pub fn parse(spec: &str) -> Result<Self, String>;
}

pub fn register_snapshot(machine: &Machine) -> String;  // R0..R12/SP/LR/PC line
pub fn fault_tail_lines(machine: &mut Machine) -> Vec<String>; // [regs]+[mem]+[tail]
```

`Machine` lives in the library target (`rp2350_cli::run::Machine`), so a GUI or
another host links it directly instead of shelling out to the binary. The
frame/input worker below is the supported way to drive it from a frontend that
must not block on emulation.
`Machine` 位于库目标（`rp2350_cli::run::Machine`），因此 GUI 或其它宿主可以直接
链接，而不必调用可执行文件。下面的帧/输入 worker 是「前端不能被仿真阻塞」时受支持
的驱动方式。

`Machine::read_block` is a prefix copy meant for frame buffers: it stops at the
first unmapped address, reads the aligned body with `AddressBus::read_u32`
(one bus round trip per four bytes) and falls back to byte reads for an
unaligned head, a sub-word tail and any wide read that fails, so the returned
prefix is byte-for-byte the same as a byte-wise walk. A 240x160 RGB565 frame
(76 800 bytes) therefore costs ~19 200 bus reads instead of 76 800. It is not a
side-effect-free guarantee for MMIO ranges: like `dump_memory`, callers must
only point it at memory.
`Machine::read_block` 是面向帧缓冲的前缀拷贝：遇到首个未映射地址即停止，对齐主体
用 `AddressBus::read_u32` 读取（每 4 字节一次总线往返），不对齐的头、不足一字的尾
以及失败的宽读都回退到字节读，因此返回的前缀与逐字节遍历逐字节相同。240x160 的
RGB565 帧（76 800 字节）因此只需约 19 200 次总线读，而不是 76 800 次。它不保证
MMIO 区间无副作用：与 `dump_memory` 一样，调用方只能指向内存。

## `PlaySession` — background frame worker / 后台帧 worker

```rust
pub struct FrameSpec { pub base: u32, pub width: u16, pub height: u16, pub stride_bytes: u32 }
impl FrameSpec {
    pub fn new(base: u32, width: u16, height: u16) -> Self;      // stride = width * 2
    pub fn parse(spec: &str) -> Result<Self, String>;            // "ADDR:WxH[:STRIDE]"
    pub fn buffer_len(&self) -> usize;                           // height * stride
    pub fn decode(&self, bytes: &[u8]) -> Vec<u16>;              // little-endian RGB565
    pub fn frame(&self, index: u64, steps: u64, bytes: &[u8]) -> Frame;
}

pub struct Frame {
    pub index: u64,              // 1-based
    pub steps: u64,              // guest steps at capture
    pub width: u16,
    pub height: u16,
    pub rgb565: Vec<u16>,        // row-major, width * height
    pub readable: bool,          // false when the window was partly unmapped
}
impl Frame { pub fn is_blank(&self) -> bool; }

pub struct PlayAudioConfig {
    pub block: usize,              // PIO block that pushes I2S words
    pub sm: usize,                 // state machine inside that block
    pub rate_hz: u32,              // sink metadata
    pub channels: u8,
    pub capacity: usize,           // samples retained by the sink
}

pub struct PlayConfig {
    pub frame: FrameSpec,
    pub steps_per_frame: u64,
    pub max_frames: Option<u64>,   // None = run until shutdown
    pub button_pins: u16,          // 0 = drive no pin (no example default)
    pub audio: Option<PlayAudioConfig>, // None = no capture
    pub trace_steps: usize,        // [trace] lines at the first frame
}
impl PlayConfig {
    pub fn new(frame: FrameSpec, steps_per_frame: u64) -> Self; // buttons=0, audio=None
}

pub struct PlayStats {
    pub frames: u64,
    pub steps: u64,
    pub steps_per_second: f64,     // rolling 1 s window (cumulative until it closes)
    pub fps: f64,
    pub halted: Option<String>,    // budget exhausted or machine halt
}

pub struct PlaySession { /* worker handle */ }
impl PlaySession {
    pub fn start(image: Vec<u8>, options: MachineOptions, config: PlayConfig)
        -> Result<Self, String>;   // deterministic single-threaded cores
    pub fn start_with_core_threads(
        image: Vec<u8>,
        options: MachineOptions,
        threads: CoreThreadOptions,
        config: PlayConfig,
    ) -> Result<Self, String>;     // Err on out-of-range audio block/SM
    pub fn latest_frame(&self) -> Option<Arc<Frame>>;
    pub fn set_buttons(&self, mask: u16);
    pub fn buttons(&self) -> u16;
    pub fn button_pins(&self) -> u16;
    pub fn core_threads_enabled(&self) -> bool;  // read back from the machine
    pub fn core_window(&self) -> u32;            // read back (clamped) window
    pub fn stats(&self) -> PlayStats;
    pub fn replaced_frames(&self) -> u64;
    pub fn is_running(&self) -> bool;
    pub fn audio_tap(&self) -> Option<rp2350_periph::AudioTap>;
    pub fn shutdown(self) -> PlayStats;   // stop + join + final stats
}
pub fn parse_button_mask(spec: &str) -> Result<u16, String>;

pub mod profile {                       // key = value profile files
    pub struct Profile { /* every key is Option<_> */ }
    pub struct ProfileError { pub line: usize, pub message: String }
    pub fn parse(text: &str) -> Result<Profile, ProfileError>;
}
pub struct PlayArgs { /* raw CLI arguments, every field optional */ }
pub struct PlayOptions { /* resolved: file, frame, steps, audio wiring, ... */ }
pub fn run(args: &PlayArgs) -> anyhow::Result<()>;   // CLI implementation
```

### Threading model / 线程模型

`Machine` is **not** `Send`, so `PlaySession::start` moves the image bytes and
`MachineOptions` into a dedicated worker thread (`rp2350-play`) that builds the
machine itself; `start` waits for a one-shot ready message so a boot failure is
reported as `Err` instead of disappearing on the worker. Everything crossing the
thread boundary is plain data behind `Arc`:

| Shared state | Type | Direction |
|---|---|---|
| button mask | `Arc<AtomicU16>` | host → worker |
| latest frame | `Arc<Mutex<Option<Arc<Frame>>>>` | worker → host |
| statistics | `Arc<Mutex<PlayStats>>` | worker → host |
| stop flag / running flag | `Arc<AtomicBool>` | both |
| audio tap | `Option<AudioTap>` (`Arc<Mutex<AudioSink>>`) | worker + host |

The producer never waits for the consumer: publishing overwrites the single
"latest frame" slot and increments `replaced_frames`, so a slow renderer drops
frames instead of stalling the emulator. `shutdown` (or `Drop`) sets the stop
flag, joins the thread and returns the final statistics; a poisoned lock is
recovered instead of panicking, so a worker panic cannot bring the host down
during teardown.
`Machine` **不是** `Send`，因此 `PlaySession::start` 把映像字节与
`MachineOptions` 移入专用 worker 线程（`rp2350-play`），由线程自行构造机器；
`start` 等待一次就绪消息，因此启动失败会以 `Err` 返回而不是在 worker 上消失。
跨线程共享的只有 `Arc` 包着的纯数据：按键掩码（host → worker）、最新帧与统计
（worker → host）、停止/运行标志（双向）与音频 tap。生产者从不等待消费者：发布
直接覆盖单槽「最新帧」并递增 `replaced_frames`，慢渲染只会丢帧而不会卡住仿真。
`shutdown`（或 `Drop`）置停止标志、join 线程并返回最终统计；锁中毒会被恢复而不
panic，因此 worker panic 不会在收尾阶段拖垮宿主。

### egui usage / egui 用法示例

GUI 侧不必自己写适配器：`rp2350-egui` 的 `play_view/cli_source.rs` 已提供
`CliPlaySource` / `CliPlayStarter`，依赖方向为 `rp2350-egui → rp2350-cli`（CLI 不再
依赖 GUI），`main.rs` 默认挂载。下面是最小用法，供其它前端参考。

The GUI does not need its own adapter: `rp2350-egui` ships `CliPlaySource` /
`CliPlayStarter` in `play_view/cli_source.rs` with the dependency pointing
`rp2350-egui → rp2350-cli` (the CLI no longer depends on the GUI), attached by
`main.rs`. The snippet below is the minimal usage for any other frontend.

```rust
use std::sync::Arc;
use rp2350_cli::play::{
    Frame, FrameSpec, PlayAudioConfig, PlayConfig, PlaySession, PlayStats,
};
use rp2350_cli::run::MachineOptions;

let image = std::fs::read("firmware.uf2")?;
let frame = FrameSpec::parse("0x20000000:320x240")?;  // caller-supplied geometry
let mut config = PlayConfig::new(frame, 1_000_000);    // caller-supplied cadence
config.button_pins = 0x00FF;                            // caller-supplied pin map
config.audio = Some(PlayAudioConfig {                   // None = no capture
    block: 0,
    sm: 0,
    rate_hz: 48_000,
    channels: 2,
    capacity: 48_000,
});
let session = PlaySession::start(image, MachineOptions::default(), config)?;

// once per egui update:
session.set_buttons(keys_to_button_mask(&ctx.input(|i| i.clone())));
if let Some(frame) = session.latest_frame() {
    upload_texture(&frame);                          // frame.rgb565: &[u16], width, height
}
let stats: PlayStats = session.stats();
ctx.request_repaint_after(std::time::Duration::from_millis(16));
```

Keyboard-to-mask mapping belongs to the frontend (landed for the egui Play tab as
P1-2); the worker only consumes the mask. `PlaySession::audio_tap()` returns the
same tap the machine installed, so a playback plugin can drain `i16` samples
concurrently.
按键到掩码的映射属于前端（egui 的「游戏」标签页已落地，P1-2），worker 只消费掩码。
`PlaySession::audio_tap()` 返回机器实际安装的同一个 tap，因此播放插件可并发取走
`i16` 样本。

### Limitations / 限制

- Active frame source is the guest frame buffer, not the HSTX pixel stream (no
  scanline/frame boundary model). A firmware that never writes its frame buffer
  yields black frames.
  活动帧源是客户机帧缓冲，而不是 HSTX 像素流（无扫描线/帧边界模型）。从不写帧
  缓冲的固件只会得到黑帧。
- One session owns one machine: no save-states, no rewind, no stepping API. Stop
  and start a new session to reload firmware.
  一个会话独占一台机器：无存档、无回退、无单步接口。需要更换固件时停止并新建会话。
- Audio capture is installed but not consumed here; `rp2350-audio` provides the
  host output backend.
  这里只安装音频捕获、不消费样本；宿主输出后端由 `rp2350-audio` 提供。

### Bus assembly / 总线组装

`Machine::new` builds, in order: the synthetic boot ROM (`Rp2350Bootrom::new`
with the flash handle, mounted at `0x0000_0000`), the flash XIP window at
`0x1000_0000` plus its non-cached alias at `0x1400_0000` (one shared backing
store), the write-absorbing XIP maintenance window at `0x1800_0000`,
520 KB SRAM at `0x2000_0000`, `BootPeripherals` (GPIO, UART0, timer, three
shared PIO block handles, etc.), the optional PSRAM window at `0x1100_0000`
(only when `MachineOptions::psram` is `Some`), and `rp2350_usb::mount` for the
USB controller (DPSRAM + register windows).
`Machine::new` 依次构建：合成 boot ROM（`Rp2350Bootrom::new` 持有 flash
句柄，挂载于 `0x0000_0000`）、`0x1000_0000` 的 Flash XIP 窗口与
`0x1400_0000` 的非缓存别名（共享同一份底层存储）、`0x1800_0000` 吸收写入的
XIP 维护窗口、`0x2000_0000` 的 520 KB SRAM、`BootPeripherals`（GPIO、UART0、
定时器、三个共享 PIO 块句柄等）、可选的 `0x1100_0000` PSRAM 窗口（仅当
`MachineOptions::psram` 为 `Some`），以及 USB 控制器的 `rp2350_usb::mount`
（DPSRAM + 寄存器窗口）。

The PSRAM mount is order-independent of `BootPeripherals::mount` (neither touches
`0x1100_0000`); it runs after the peripherals and before the core boots, so the
window exists before the first fetch. A registration failure (for example a
second window overlapping the first) is reported as a startup error
(`PSRAM mount failed / PSRAM 挂载失败`) instead of a panic.
PSRAM 挂载与 `BootPeripherals::mount` 顺序无关（两者都不触碰 `0x1100_0000`）；
它在外设之后、核心启动之前执行，因此首条取指前窗口已存在。注册失败（例如第二个
窗口与第一个重叠）会作为启动错误（`PSRAM mount failed / PSRAM 挂载失败`）上报，
而不是 panic。

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

- **Batched stepping / 批量步进**：`run`, `repl` and `perfbench` drive the
  machine through `Machine::run_steps` (`crates/rp2350-cli/src/run/batch.rs`),
  which is equivalent to calling `step()` N times but hoists host-only work to
  the step boundary where it takes effect: DMA/PIO batches, IRQ delivery
  (`service_core1_peer` + `deliver_irqs`) and the timebase still run on exactly
  the same step numbers (every 64 / 64 / 10 000 retired steps), so guest-visible
  behaviour is byte-identical. Per-instruction work that diagnostics can read is
  kept per instruction: the budget/halt check, the `RecentSteps` ring, the PPB
  replay gate, the tight-spin detector and core-1 stepping. `step()`,
  `step_cores()` and `trace_step()` keep their original granularity for the GDB
  stub and `--trace-steps`.
  `run`、`repl` 与 `perfbench` 通过 `Machine::run_steps`
  （`crates/rp2350-cli/src/run/batch.rs`）驱动机器：它等价于调用 `step()` N 次，
  但把与指令无关的宿主工作提升到真正生效的步边界——DMA/PIO 批次、中断投递
  （`service_core1_peer` + `deliver_irqs`）与时基仍在与原先完全相同的步号上执行
  （每 64 / 64 / 10 000 步），因此客户机可观察行为逐字节不变。诊断会读取的
  逐指令工作仍保留逐指令粒度：预算/停机检查、`RecentSteps` 环形缓冲、PPB 回放
  门控、自旋检测与 core1 步进。`step()`、`step_cores()` 与 `trace_step()` 仍保持
  原来的粒度，供 GDB stub 与 `--trace-steps` 使用。
- **Hoisted checks / 提升的检查**：the synchronous-trap probe (`csr_read(0x342)`)
  runs per instruction only on RISC-V; the Cortex-M33 has no CSR window (`csr_read`
  is always 0), so on Arm the probe is statically impossible to observe. Core 1 is
  only stepped when it is `Running` or a PSM reset / SIO FIFO launch is pending,
  which is exactly when `step_core1` can have an effect.
  同步陷阱探测（`csr_read(0x342)`）只在 RISC-V 上逐指令执行；Cortex-M33 没有 CSR
  窗口（`csr_read` 恒为 0），因此在 Arm 上该探测在语义上不可能被观察到。core1
  仅在 `Running` 或存在 PSM 复位 / SIO FIFO 启动挂起时才步进，这正是
  `step_core1` 可能产生作用的条件。
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
  for 1 000 000 consecutive instructions (up to 4 warnings per run). A window
  that saw sleep steps, wake-ups or interrupt deliveries is a wait loop
  (`WFI`/`WFE` plus a timer/IRQ wake) rather than a stuck poll: the detector
  suppresses its warning and re-arms from the current counters, so later pure
  spins still report.
  当 PC 在同一个 32 字节窗口内连续停留 1 000 000 条指令时打印一次自旋告警
  （每次运行最多 4 条）。窗口内出现过睡眠步数、唤醒或中断投递时属于等待循环
  （`WFI`/`WFE` + 定时器/中断唤醒），而不是卡死轮询：检测器不告警，并用当前
  计数器值重新武装窗口，因此之后的纯自旋仍会报告。
- `deliver_irqs` walks only the asserted bits of the mask
  (`trailing_zeros` + `mask &= mask - 1`), in the same ascending order as the
  former `0..64` scan, so IRQ log lines and delivery order are unchanged.
  `deliver_irqs` 只遍历掩码中已断言的位（`trailing_zeros` + `mask &= mask - 1`），
  顺序与原先的 `0..64` 扫描一致，因此 IRQ 日志行与投递顺序不变。
- `record_step` writes the `(step, PC)` pair into a fixed 32-entry ring
  (`RecentSteps`) instead of a `VecDeque`; `[tail]` and fault traces iterate the
  same chronological order, so the diagnostic text is unchanged.
  `record_step` 把 `(步数, PC)` 写入定长 32 条的 `RecentSteps` 环形缓冲（替代
  `VecDeque`）；`[tail]` 与故障回溯仍按相同的时间顺序迭代，诊断文本不变。
- The boot ROM stub check is gated by the ROM window
  (`pc - ROM_BASE < ROM_SIZE`) before scanning the stub table. Every stub is
  built inside the ROM image, so the gate cannot skip a real trap.
  boot ROM 桩检查先用 ROM 窗口（`pc - ROM_BASE < ROM_SIZE`）过滤，再扫描桩表。
  所有桩都构建在 ROM 映像内，因此该门控不会漏掉真实陷阱。
- System-control replay is gated by `SimpleBus::ppb_write_epoch`: an unchanged
  epoch proves the replay queue is empty, so the drain (and its mutex) is
  skipped. A PPB write in the same step is still replayed before the next
  instruction.
  系统控制回放由 `SimpleBus::ppb_write_epoch` 门控：世代未变即证明回放队列为空，
  于是跳过回放（及其互斥锁）。同一步内发生的 PPB 写仍会在下一条指令前回放。
- The `run` loop drains notices only when `Machine::notices_pending()` reports a
  queued line; trace lines and spin warnings are emitted in the same order.
  `run` 循环仅在 `Machine::notices_pending()` 报告有排队内容时才取走通知；轨迹行
  与自旋告警的输出顺序不变。

#### Bus, MMIO and watch cost / 总线、MMIO 与监视开销

`Machine` inherits its lookup costs from `rp2350-core`:

- `SimpleBus` resolves addresses through a 256-bucket table keyed by `addr >> 24`
  (see `doc/crates/rp2350-core.md`), so a fetch or store no longer scans the
  region list;
- `FlashRegion` caches its length in the struct (no lock in `size()`/`contains`)
  and reads halfwords/words with a single shared-store acquisition;
- `MmioRouter` caches direct child hits per 4 KiB page;
- the `RP2350_WATCH` / `RP2350_WATCHV` write channel is behind a sticky
  `AtomicBool`, so an unarmed run pays one relaxed load per store and nothing
  per instruction for the PC record.

None of these change what the guest observes, what `[run]`/`[tail]`/`[irq]`
print, or when DMA/PIO batches and IRQ lines are pumped.
`Machine` 的查找开销继承自 `rp2350-core`：

- `SimpleBus` 按 `addr >> 24` 索引 256 个桶解析地址（见
  `doc/crates/rp2350-core.md`），因此取指与存储不再遍历区域列表；
- `FlashRegion` 在结构体里缓存长度（`size()`/`contains()` 不再加锁），并用一次
  共享存储获取读取半字/整字；
- `MmioRouter` 按 4 KiB 页缓存直接子设备命中；
- `RP2350_WATCH` / `RP2350_WATCHV` 写通道位于一个只置位的 `AtomicBool` 之后，
  未安装时每次存储只付一次 relaxed load，PC 记录每条指令零开销。

以上都不改变客户机观察到的行为、`[run]`/`[tail]`/`[irq]` 的输出内容，或 DMA/PIO
批次与中断线的泵送时机。

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
- **Stepping order / 步进顺序**：`Machine::step` and `Machine::trace_step` retire
  one core-0 instruction and then one core-1 instruction (when core 1 is
  `Running`); `step` is a thin wrapper over `step_cores(true, true)`.
  `Machine::step_cores(core0, core1)` advances only the selected cores: an
  unselected core keeps its state and PC, and a selected core that is not
  `Running` is not advanced. The step still performs the shared peripheral work
  once (budget/timebase, DMA/PIO batches, PPB replay, SEV/IRQ forwarding), and
  `steps`/`now_cycles`/`budget` count one instruction slot per step whatever the
  selection. `run_cycles` always advances both cores.
  `Machine::step` 与 `Machine::trace_step` 先退役一条 core0 指令，再退役一条
  core1 指令（core1 处于 `Running` 时）；`step` 是 `step_cores(true, true)` 的
  薄封装。`Machine::step_cores(core0, core1)` 只推进选中的核心：未选中的核心保持
  状态与 PC，被选中但非 `Running` 的核心也不推进。该步仍执行一次共享外设工作
  （预算/时基、DMA/PIO 批次、PPB 回放、SEV/中断转发），且无论选择如何，
  `steps`/`now_cycles`/`budget` 每步计一条指令槽。`run_cycles` 始终推进双核。
- **Fault isolation / 故障隔离**：a core-1 bus error or undefined instruction
  halts **core 1 only** and is reported by `core1_halt_reason()` and
  `[run] core1 halt: …`; it never sets the machine's `halt_reason` and never
  stops core 0.
  **故障隔离**：core1 的总线错误或未定义指令**只停机 core1**，由
  `core1_halt_reason()` 与 `[run] core1 halt: …` 报告；它绝不设置机器的
  `halt_reason`，也不停止 core0。
- **Per-core register state / 每核寄存器状态**：the SIO view is switched to
  `Core1` around core 1's instruction, and PPB system writes (VTOR, CPACR,
  NVIC, `ICSR`, `SHPR2`/`SHPR3`, `SCR`, `STIR`) are replayed into the core that
  executed them, so the two cores keep independent vector tables, FPU enable
  state and NVIC enable masks. SysTick is the exception: `SYST_CSR`/`SYST_RVR`/
  `SYST_CVR` stay in the peripheral `SysTickCounter`, which follows the live
  clock tree and is never replayed into a core.
  core1 指令前后把 SIO 视图切到 `Core1`，PPB 系统写入（VTOR、CPACR、NVIC、
  `ICSR`、`SHPR2`/`SHPR3`、`SCR`、`STIR`）回放到实际执行它的核心，因此两核各自
  保有独立的向量表、FPU 使能与 NVIC 使能掩码。SysTick 是例外：
  `SYST_CSR`/`SYST_RVR`/`SYST_CVR` 留在外设 `SysTickCounter` 中，跟随实时时钟树，
  永不回放到核。
- **SysTick forwarding / SysTick 转发**：the peripheral `SysTickCounter` is the
  single source of truth for counting, `COUNTFLAG`, the live clock source
  (`CLKSOURCE=1` → `clk_sys`, `CLKSOURCE=0` → `clk_ref/2`) and the one-shot
  expiry latch. Every IRQ pump calls `BootPeripherals::take_systick_pending()`
  at most once and forwards a `true` result to `CortexM33Core::pend_systick()`;
  the core then enters system exception 15 at `VTOR + 0x3C`. The forward is
  logged as `[irq] core0 systick at PC=…`. A machine never replays the SysTick
  registers, so the core-side counter stays disabled and nothing double-counts;
  that self-count path only serves `rp2350-arm` unit tests without a PPB.
  **SysTick 转发**：外设 `SysTickCounter` 是计数、`COUNTFLAG`、实时时钟源
  （`CLKSOURCE=1` → `clk_sys`，`CLKSOURCE=0` → `clk_ref/2`）与一次性到期锁存的
  唯一真相源。每次中断泵送最多调用一次 `BootPeripherals::take_systick_pending()`，
  结果为 `true` 时转发给 `CortexM33Core::pend_systick()`，核随后进入
  `VTOR + 0x3C` 的系统异常 15。转发记录为 `[irq] core0 systick at PC=…`。
  机器从不回放 SysTick 寄存器，因此核内计数器保持关闭、不会双计时；该自计数
  路径只服务于没有 PPB 的 `rp2350-arm` 单测。
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
- **Modelled / 已建模（2026-11-16）**：`SCR.SLEEPONEXIT` 在异常返回 Thread 后进入睡眠、
  `SLEEPDEEP` 记录为深睡原因（不建模电气掉电）、`SVCall`/`HardFault`/`NMI` 与故障升级、
  `AIRCR.SYSRESETREQ` 的宿主策略（`run` 最多重启 3 次，`repl`/`play` 停止）。
- **Not modelled / 未建模项**：DMA arbitration between cores. Core-1
  writable registers: GDB exposes core 1 as thread 2 with read-only
  PC/SP, while per-core stepping is available through `Machine::step_cores`.
  Core 1 has no
  modelled mask ROM: a launch jumps straight to `entry`, reproducing the post-ROM
  state, and only the trap stubs (`rom_table_lookup` at `0x74`, the ROM function
  table) are dispatched. SysTick is no longer in this list: the machine forwards
  the peripheral latch to exception 15 (see the forwarding entry above).
  **未建模项**：双核之间的 DMA 仲裁（`SLEEPDEEP`/`SLEEPONEXIT` 已建模，见上）。
  core1 可写寄存器：GDB 把 core1 暴露为线程 2（PC/SP 只读），而按核步进已可通过
  `Machine::step_cores` 使用。core1 不建模掩膜 ROM：启动直接跳到
  `entry` 并复现 post-ROM 状态，仅分发陷阱桩（`0x74` 的 `rom_table_lookup` 与
  ROM 函数表）。SysTick 已不在未建模列表中：机器把外设锁存转发为异常 15
  （见上面的转发条目）。

### Sleep scheduling and diagnostics / 睡眠调度与诊断

`WFI`, `WFE` and `SEV` are real scheduling points now, and a core that blocks on
`SIO_FIFO_RD` sleeps instead of reading a discarded value.

* **Sleeping cores do not retire instructions but time still advances.** A step
  in which the selected core reports `CpuState::Sleeping` increments the machine
  step counter (the timebase, DMA/PIO pumps and interrupt delivery all key off
  it), counts one sleep step for that core and pushes nothing into the
  executed-PC post-mortem ring. The machine stays `Running`: only `Halted`,
  `Faulted` and `Reset` stop it, so a sleeping core 0 never freezes core 1 or the
  display pipeline.
* **Events before stepping.** `step_cores_with_trace` (and the batched
  `run_steps`) call `Machine::sync_core_events` before stepping: a change of
  `BootPeripherals::sio_event_generation()` is broadcast to both cores as an ARM
  event, and a core's `SEV` count is forwarded to the peer. Together with
  `deliver_irqs` (NVIC pending) this is what lets the SDK's
  `while (!multicore_fifo_rvalid()) __wfe();` launch handshake finish inside a
  sleeping core-0 loop.
* **Zero-step wake latency.** When the wake condition holds, the very next step
  makes progress: a `WFE`/`WFI` sleeper executes the instruction at `PC` (or
  enters the takeable interrupt in that same step), and a blocked load replays
  and retires its instruction.
* **Diagnostics.** `Machine::sleep_counters()` returns
  `([core0_steps, core1_steps], [core0_wakes, core1_wakes])` and
  `Machine::sleep_state()` renders the stable line
  `sleep core0 steps=<n> wakes=<n> core1 steps=<n> wakes=<n>`, printed by
  `run` as `[run] sleep …` after the HSTX line. A machine whose cores all sleep
  while no event arrives can only advance its clock, so after
  `SLEEP_STALL_STEPS` (1 000 000) consecutive double-sleep steps the machine
  queues one `[run] sleep stall: …` notice instead of silently burning the
  budget.

`WFI`、`WFE` 与 `SEV` 现在是真实调度点；在 `SIO_FIFO_RD` 上阻塞的核心进入睡眠，
而不是读回被丢弃的值。

- **睡眠核不退役指令，但时间照常推进。** 被选中核心报告 `CpuState::Sleeping`
  的那一步仍递增机器步数（时基、DMA/PIO 泵送与中断投递都以它为准），为该核记一次
  睡眠步，并且不写入「已执行 PC」回溯环形缓冲。机器状态仍是 `Running`：只有
  `Halted`、`Faulted`、`Reset` 会停机，因此 core0 睡眠绝不会冻结 core1 或显示流水线。
- **先转发事件再推进核心。** `step_cores_with_trace`（以及批量 `run_steps`）在推进前
  调用 `Machine::sync_core_events`：`BootPeripherals::sio_event_generation()` 变化会
  作为 ARM 事件广播给两个核心，某核的 `SEV` 计数会转发给对端。配合
  `deliver_irqs`（NVIC 挂起），SDK 的
  `while (!multicore_fifo_rvalid()) __wfe();` 启动握手才能在睡眠的 core0 循环内完成。
- **唤醒 0 步延迟。** 唤醒条件成立后的下一步立即取得进展：`WFE`/`WFI` 睡眠者执行
  `PC` 处指令（或在那一步进入可进入的中断），阻塞载入则重放并退役其指令。
- **诊断。** `Machine::sleep_counters()` 返回
  `([core0_steps, core1_steps], [core0_wakes, core1_wakes])`，
  `Machine::sleep_state()` 生成稳定行
  `sleep core0 steps=<n> wakes=<n> core1 steps=<n> wakes=<n>`，由 `run` 在 HSTX 行
  之后打印为 `[run] sleep …`。若所有核心都睡眠且没有事件到达，机器只能推进时钟，
  因此在 `SLEEP_STALL_STEPS`（1 000 000）个连续双核睡眠步之后排队一条
  `[run] sleep stall: …` 通知，而不是静默耗尽预算。

#### Sleep fast-forward / 睡眠快进

`Machine::sleep_fast_forward_skip` (`crates/rp2350-cli/src/run/fast_forward.rs`)
batches the clock steps of a sleeping machine while keeping every boundary the
step-by-step path would have observed.

`Machine::sleep_fast_forward_skip`（`crates/rp2350-cli/src/run/fast_forward.rs`）
在保留逐步路径会观察到的每个边界的前提下，批量推进睡眠机器的时钟步。

**Switch / 开关**

* `run --sleep-fast-forward on|off` — `on` is the default; `true`/`false` and
  `1`/`0` are accepted aliases.
* `RP2350_NO_SLEEP_FF=1` (or `true`) disables it for every frontend (`run`,
  `repl`, `play`, `gdb`, `perfbench`); an explicit `--sleep-fast-forward` value
  wins over the environment.
* `Machine::set_sleep_fast_forward(bool)`, `Machine::sleep_fast_forward()` and
  `Machine::sleep_fast_forward_steps()` expose the state to hosts and tests.
  With it off, the batched path performs exactly the pre-fast-forward work and
  `sleep_fast_forward_steps()` stays `0`.

* `run --sleep-fast-forward on|off`：默认 `on`，也接受 `true`/`false`、`1`/`0`。
* `RP2350_NO_SLEEP_FF=1`（或 `true`）对全部前端（`run`、`repl`、`play`、`gdb`、
  `perfbench`）关闭快进；显式 `--sleep-fast-forward` 优先于环境变量。
* `Machine::set_sleep_fast_forward(bool)`、`Machine::sleep_fast_forward()` 与
  `Machine::sleep_fast_forward_steps()` 向宿主/测试暴露同一状态。关闭时批量路径
  执行与快进前完全相同的工作，`sleep_fast_forward_steps()` 恒为 `0`。

**When it applies / 判定规则**

The skip is attempted at the top of a `run_steps` iteration only when every
"no event can fire" proof holds:

1. core 0 is `Sleeping` and not in a `BlockedRead` retry;
2. core 1 is absent/`Reset`/`Sleeping`; a sleeping core 1 must also not be in a
   blocked load and must not have a pending event (`WFE`) or exception;
3. **neither core has a pending exception**: `NVIC.ISER & NVIC.ISPR` is empty
   for both enable/pending words and `SCB_ICSR.NMIPENDSET/PENDSVSET/PENDSTSET`
   plus `SHCSR.SVCALLPENDED` are clear. This is the `WFI` wake condition, and it
   also catches a SysTick latch that `deliver_irqs` already forwarded into the
   core (a peripheral-side check cannot see it);
4. no interrupt line is asserted (`BootPeripherals::irq_mask`,
   `core1_irq_mask`, `USBCTRL.INTS`) and no SysTick latch waits for forwarding;
5. no DMA channel is busy, no PIO state machine is enabled, HSTX is disabled and
   the ADC has no conversion in flight;
6. no core-1 handshake/mailbox work is pending: `core1_reset_pending`,
   `core1_launch_pending`, `core1_release_pending` and
   `Sio::fifo_rx_ready(Core1)` are all false.

Any failed check falls back to the normal step-by-step loop for this iteration.
A sleeping core alone is not enough: a runnable peer always blocks the skip.

每次 `run_steps` 迭代开始时，只有以下「不可能有事件触发」的证明全部成立才尝试跳过：

1. core0 处于 `Sleeping` 且不是 `BlockedRead` 重试；
2. core1 为「不存在/`Reset`/`Sleeping`」；睡眠中的 core1 也不得处于阻塞载入、
   不得有 `WFE` 待处理事件或挂起异常；
3. **两个核心都没有挂起异常**：两个字（enable/pending）的 `NVIC.ISER & NVIC.ISPR`
   均为空，且 `SCB_ICSR.NMIPENDSET/PENDSVSET/PENDSTSET` 与
   `SHCSR.SVCALLPENDED` 均清零。这正是 `WFI` 的唤醒条件，也能捕获
   `deliver_irqs` 已转发进核内的 SysTick 锁存（外设侧检查看不到它）；
4. 没有中断线断言（`BootPeripherals::irq_mask`、`core1_irq_mask`、
   `USBCTRL.INTS`），也没有等待转发的 SysTick 锁存；
5. 没有忙碌 DMA 通道、没有使能的 PIO 状态机、HSTX 关闭、ADC 没有进行中的转换；
6. 没有待处理的 core1 握手/信箱工作：`core1_reset_pending`、
   `core1_launch_pending`、`core1_release_pending` 与
   `Sio::fifo_rx_ready(Core1)` 全为假。

任何一条失败，该轮直接回退到逐步路径。仅「core0 在睡」不够：只要另一核可运行，
就绝不快进。

**Event boundary / 事件边界**

The observation boundary is the next `ADVANCE_INTERVAL` (10 000) step timebase
advance. TIMER alarms and `SysTick` are sampled only when `advance()` runs, the
ADC/POWMAN and clock-tree `generation` take effect there, and interrupt delivery
happens on the 64-step pump. The skip therefore lands on `boundary - 1` and lets
the normal loop execute the boundary step itself: the timebase sees exactly the
same cycle counts as the step-by-step path, and a newly latched alarm/SysTick is
observed by the next guard (which then refuses to skip and lets the 64-step pump
wake the core). A frequency change written while the guest sleeps is likewise
settled on that boundary with the old rate, exactly as before.

观察边界是下一次 `ADVANCE_INTERVAL`（10 000）步的时基推进。TIMER alarm 与
`SysTick` 只在 `advance()` 运行时被采样，ADC/POWMAN 与时钟树 `generation` 也在该
边界生效，中断投递则在 64 步泵送点。因此快进停在 `boundary - 1`，由正常循环亲自
执行该边界步：时基看到的周期数与逐步路径完全相同，新锁存的 alarm/SysTick 会被下一次
守卫观察到（随后拒绝快进，让 64 步泵送唤醒核心）。客户机睡眠期间写入的频率同样在该
边界按旧频率结算，与快进前一致。

**Pump consistency / 泵送一致性**

Inside the skipped interval the DMA/PIO/IRQ pumps provably have no effect, so the
loop is not run per 64 steps. Instead `pumped`, `dma_pumped` and `pio_pumped` are
folded forward by whole `PUMP_INTERVAL` periods, which preserves the phase of
every later pump (the first DMA/PIO batch and IRQ delivery after the wake happen
on exactly the same steps). Sleep steps are credited in bulk to `steps`,
`sleep_steps` and the double-sleep stall notice, while the recent-step ring, the
spin detector and the wake counters stay untouched because a sleeping step never
touches them.

跳过区间内 DMA/PIO/IRQ 泵送已被证明没有作用，因此不按 64 步逐次执行，而是把
`pumped`、`dma_pumped`、`pio_pumped` 按整 `PUMP_INTERVAL` 周期折叠，精确保留之后
每次泵送的相位（唤醒后的第一批 DMA/PIO 与中断投递步号完全相同）。睡眠步按批量记入
`steps`、`sleep_steps` 与「双核睡眠」通知；最近步数环形缓冲、自旋检测与唤醒计数
保持不变，因为睡眠步从不触碰它们。

**Relation to `steps`/`time_us` / 与 `steps`、`time_us` 的关系**

A skipped step is still one simulated clock cycle: `steps` grows by the same
amount, `time_us` is identical, `[run] sleep core0 steps=…` is unchanged, and no
instruction is skipped (a sleeping core retires none). `sleep_fast_forward_steps()`
is a host-only diagnostic counting skipped clock steps; it is not printed in the
stable `[run]` report.

跳过的每一步仍是一个模拟时钟周期：`steps` 增量相同、`time_us` 相同、
`[run] sleep core0 steps=…` 不变，也不跳过任何指令（睡眠核本来不退役指令）。
`sleep_fast_forward_steps()` 只是宿主诊断计数，不出现在稳定的 `[run]` 报告中。

**Architecture scope / 架构范围**

Arm Cortex-M33 only. Hazard3's `mcycle`/`minstret` advance once per sleep step and
are guest-visible CSRs, so RISC-V machines keep the step-by-step sleep loop until
`rp2350-riscv` exposes a batched cycle-accounting entry point. This is an
architecture capability check, not a firmware special case: any Arm image gets
the same treatment.

仅支持 Arm Cortex-M33。Hazard3 的 `mcycle`/`minstret` 每个睡眠步前进一次且是
客户机可见 CSR，因此在 `rp2350-riscv` 提供批量周期记账入口之前，RISC-V 机器保持
逐步睡眠循环。这是架构能力判断，与具体固件无关：任意 Arm 映像都得到相同处理。

One host-only caveat: `CpuCore::total_cycles()` advances only when the core retires
or executes a step, so the sleep steps skipped by the fast-forward are not folded
into that per-core diagnostic counter (`rp2350-arm` has no batched accounting entry
point). It is not a guest-visible register — no DWT `CYCCNT` is modelled — and it is
not printed in the `[run]` report; `steps`, `time_us` and every peripheral counter are
unchanged.

一个仅限宿主诊断的注意点：`CpuCore::total_cycles()` 只在核心执行（或退役）一步时
前进，因此快进跳过的睡眠步不会折叠进该每核诊断计数（`rp2350-arm` 没有批量记账
入口）。它不是客户机可见寄存器（未建模 DWT `CYCCNT`），也不出现在 `[run]` 报告
中；`steps`、`time_us` 与全部外设计数保持不变。

### DMA pump / DMA 泵送

Firmware only programs the 16-channel DMA register block at `0x5000_0000`;
transfers happen because the machine pumps them, not because the guest writes
`CTRL_TRIG`.
固件只写 `0x5000_0000` 的 16 通道 DMA 寄存器块；传输之所以发生，是机器在泵送
它们，而不是客户机写了 `CTRL_TRIG`。

- **Pump point / 泵送点**：`Machine::pump_dma_batch` is called from
  `step_cores_with_trace` (the shared body behind `step`, `step_cores` and
  `trace_step`) immediately after a step retires and **before** interrupt
  delivery. Plain `step`, `step_cores`, `trace_step`, boot-ROM trap steps and
  GDB `resume` single steps all funnel through this one call, so a debugger
  cannot starve the DMA engine.
  `Machine::pump_dma_batch` 在一步退役后、**中断投递之前**由
  `step_cores_with_trace`（`step`、`step_cores` 与 `trace_step` 的共享实现）
  调用。普通 `step`、`step_cores`、`trace_step`、boot ROM 陷阱步与 GDB `resume`
  单步都汇入这一处调用，调试器因此无法让 DMA 引擎停摆。
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
  `step_cores_with_trace` right next to `pump_dma_batch`, after the step retires
  and **before** interrupt delivery. Plain `step`, `step_cores`, `trace_step`,
  boot-ROM trap steps and GDB `resume` all take this path, so a debugger cannot
  starve the PIO state machines either.
  `Machine::pump_pio_batch` 在一步退役后、**中断投递之前**、紧邻
  `pump_dma_batch` 由 `step_cores_with_trace` 调用。普通 `step`、`step_cores`、
  `trace_step`、boot ROM 陷阱步与 GDB `resume` 都经过这里，因此调试器同样无法让
  PIO 状态机停摆。
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
| `rp2350-ffi`, `tokio` | C ABI export; `tokio` waits for Ctrl-C / C ABI 导出；`tokio` 用于等待 Ctrl-C |

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
