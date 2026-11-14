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
> `list-devices` / `wire`. Stub — `mcp-serve` logs its arguments and returns.
> **状态**: 已实现 — `run` / `repl` / `perfbench` / `gdb` / `list-devices` /
> `wire`。桩 — `mcp-serve` 仅记录参数后返回。

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
rp2350 repl --file <FIRMWARE> [--exec "CODE"] [--max-cycles N]
            [--enum-cycles N] [--no-soft-reset] [--strict-decode]
            [--arch arm|riscv]
```

| Option / 选项 | Default / 默认 | Meaning / 含义 |
|---------------|----------------|----------------|
| `--file` | required | Firmware (MicroPython/CircuitPython UF2 or ELF) / 固件 |
| `--exec` | interactive | Python source to run once / 只执行一次的 Python 源码 |
| `--max-cycles` | `300_000_000` | Total simulated instruction budget / 总指令预算 |
| `--enum-cycles` | `100_000_000` | Budget before USB enumeration must finish / USB 枚举需在此预算内完成 |
| `--no-soft-reset` | off | Skip the Ctrl-D soft reset on entry / 进入时跳过 Ctrl-D 软复位 |
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
| `--max-cycles` / `--enum-cycles` / `--no-soft-reset` / `--strict-decode` | as `repl` | same semantics / 语义同 `repl` |

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

### Stub / 桩

```text
rp2350 mcp-serve [--port 8080]              # logs "not yet implemented"
```

`mcp-serve` listens on `8080` by default but does not start a server yet; the
`DeviceManager` MCP query API (`mcp_query_all` / `mcp_query_device` /
`mcp_list_devices`) is already available for it.
`mcp-serve` 默认监听 `8080`，但尚未启动服务器；`DeviceManager` 的 MCP 查询
API（`mcp_query_all` / `mcp_query_device` / `mcp_list_devices`）已可用。

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
| `rp2350-egui`, `rp2350-ffi`, `tokio` | linked for the future egui integration / 为后续 egui 集成预留 |

---

## Examples / 示例

```bash
# Run a UF2 firmware with a 5 million instruction budget
rp2350 run --file firmware.uf2 --max-cycles 5000000

# Force the RISC-V core for an ELF image
rp2350 run --file firmware.elf --arch riscv

# Execute one snippet over the MicroPython raw REPL
rp2350 repl --file firmware.uf2 --exec "print(6 * 7)"

# Benchmark, 4x64 repetitions
rp2350 perfbench --file firmware.uf2 --bench bm_float.py \
                 --benchrun benchrun.py --n 4 --m 64

# Serve an ELF to GDB on port 3333
rp2350 gdb --file firmware.elf --port 3333

# Inspect the device plugins built into target/release
rp2350 list-devices --plugin-dir target/release

# Wire the LED plugin's port 0 to GPIO25 and print the wiring table
rp2350 wire --plugin-dir target/release --device com.rp2350-rs.led --port 0 --gpio 25
```
