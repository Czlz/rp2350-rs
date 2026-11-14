# rp2350-cli — Public API / 公共 API

> Command-line front end for the RP2350 simulator. Builds the machine bus,
> boots Arm Cortex-M33 or Hazard3 RISC-V firmware, and drives it either as a
> traced `run`, an interactive USB CDC raw REPL (`repl`), or a benchmark
> harness (`perfbench`).
> RP2350 模拟器的命令行前端。构建机器总线，启动 Arm Cortex-M33 或 Hazard3
> RISC-V 固件，并按三种模式驱动：带轨迹的 `run`、交互式 USB CDC raw REPL
> （`repl`）、基准测试（`perfbench`）。
>
> **Status**: implemented — `run` / `repl` / `perfbench`. Stubs — `gdb`,
> `wire`, `list-devices`, `mcp-serve` log their arguments and return.
> **状态**: 已实现 — `run` / `repl` / `perfbench`。桩 — `gdb`、`wire`、
> `list-devices`、`mcp-serve` 仅记录参数后返回。

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

### Stubs / 桩

```text
rp2350 gdb --file <ELF> [--port 3333]      # logs "not yet implemented"
rp2350 wire --device <ID> --port <P> --gpio <PIN>   # logs and returns
rp2350 list-devices                         # logs "no devices registered yet"
rp2350 mcp-serve [--port 8080]              # logs "not yet implemented"
```

`gdb` listens on port `3333` by default and `mcp-serve` on `8080`; neither
starts a server. `wire` and `list-devices` do not touch the
`rp2350-virt-periph` manager yet.
`gdb` 默认监听端口 `3333`，`mcp-serve` 监听 `8080`；两者都不会真正启动服务
器。`wire` 与 `list-devices` 尚未接入 `rp2350-virt-periph` 的管理器。

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
| `rp2350-virt-periph` | device manager (linked; not yet driven by stubs) / 设备管理器 |
| `rp2350-gdb`, `rp2350-egui`, `rp2350-ffi`, `tokio` | linked for the future gdb/egui integration / 为后续集成预留 |

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
```
