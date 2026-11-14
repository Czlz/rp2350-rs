# perfbench Report — Official MicroPython Suite on rp2350-rs
# perfbench 报告——官方 MicroPython 基准套件在 rp2350-rs 上的结果

> **Date / 日期**: 2026-11-14
> **Emulator / 模拟器**: `rp2350-rs` (Arm Cortex-M33 + RISC-V Hazard3)
> **Harness / 测试脚本**: [micropython/micropython `tests/run-perfbench.py`](https://github.com/micropython/micropython/blob/master/tests/run-perfbench.py)
> **Firmware / 固件**: stock MicroPython `v1.29.0` (2026-08-24), Arm and RISC-V builds

---

## 1. Scope / 范围

This report records the official MicroPython performance suite running **inside
the emulator**, driven end-to-end by the upstream `run-perfbench.py` script. The
two target images are the stock RP2350 MicroPython builds:

| Target / 目标 | File / 文件 | SHA-256 | Size |
|---|---|---|---:|
| Arm (Cortex-M33) | `RPI_PICO2-20260824-v1.29.0.uf2` | `6BFC5D110F293870F60150468DFEEC8AEC5323B57415A0B9A86E6C334E876E93` | 661,504 B |
| RISC-V (Hazard3) | `RPI_PICO2-RISCV-20260824-v1.29.0.uf2` | `358075AFE8ED165223D35E0252EDE629808A5AB0F04844ECAF89205E7CBE158B` | 835,584 B |

CircuitPython is **not** part of this run (it is covered by gate FV-3).
The suite is `tests/perf_bench/*.py` minus `benchrun.py`: **24 benchmarks**,
each executed with `N=150`, `M=512`, one average (`-a 1`).

本报告记录在模拟器内、由上游 `run-perfbench.py` 端到端驱动的官方
MicroPython 性能套件结果。目标为两个原生 RP2350 MicroPython 固件；CircuitPython
不参与本次评测（由 FV-3 门禁覆盖）。套件为 `tests/perf_bench/*.py`（不含
`benchrun.py`）共 24 个基准，参数 `N=150`、`M=512`、平均次数 `-a 1`。

---

## 2. Environment / 环境

| Item / 项目 | Value / 值 |
|---|---|
| Host / 宿主 | AMD Ryzen 7 8840U (8C/16T), Windows 11 |
| Rust | `rustc 1.99.0 (b940084d7 2026-09-28)`, `cargo 1.99.0` |
| Python (driver + truth) | CPython 3.14.4 via the Windows `py` launcher |
| Emulator build | `cargo build --release -p rp2350-cli` |
| Emulator revision | `0a599a0` (FPv5-SP arithmetic fix) |
| Harness revision | `d2d095d0fe6e83c53fd0ef90643f7d2b8b1c1645` |
| Harness changes | **none** to `run-perfbench.py`, `test_utils.py` or `perf_bench/*` |

`PYTHON_TRUTH` is `MICROPY_CPYTHON3`, set to `py`, so every benchmark result is
checked against CPython rather than only being timed. This is why a numerically
wrong result is reported as `FAIL truth` instead of passing silently.
`PYTHON_TRUTH` 取 `MICROPY_CPYTHON3`（设为 `py`），因此每个基准结果都会与
CPython 真值比对；数值错误会报告为 `FAIL truth`，不会静默通过。

---

## 3. Harness setup / 测试装置

### 3.1 Emulator transport / 模拟器传输

Stock MicroPython on RP2350 exposes the REPL only over USB CDC (UART0 REPL is
disabled in `mpconfigboard.h`). The emulator therefore runs the real USB device
model, enumerates the CDC function, and the CLI `repl --stdio` bridges stdin to
the emulated bulk OUT endpoint and the bulk IN endpoint to stdout:

```text
pyboard.py  ←stdin/stdout→  rp2350-cli repl --stdio  ←USB CDC→  MicroPython guest
```

`--stdio` is protocol-transparent: the CLI never sends Ctrl-A/Ctrl-B/Ctrl-C/Ctrl-D
on its own, so `pyboard.py` owns raw-REPL framing. All `[repl]` diagnostics go to
stderr; stdout contains guest bytes only. `--max-cycles 0` removes the instruction
budget for the long-lived session. See [crates/rp2350-cli.md](crates/rp2350-cli.md)
for the bridge design.

`--stdio` 完全透传协议：CLI 不自行发送任何 raw REPL 控制字符，由 `pyboard.py`
负责帧格式；`[repl]` 诊断全部走 stderr，stdout 只有客户机字节。
`--max-cycles 0` 取消长会话的指令预算。

### 3.2 Local `pyboard.py` shim / 本地 `pyboard.py` 兼容补丁

Upstream `ProcessToSerial` is POSIX-only (`preexec_fn=os.setsid`,
`os.killpg`, `select.poll`). The sparse checkout's `tools/pyboard.py` carries a
local Windows shim, outside the repository:

- `ProcessToSerial` uses a reader thread plus a locked buffer for
  `read`/`write`/`inWaiting`; `shell=True` still runs `exec:` targets.
- `enter_raw_repl` and `exec_` wall-clock timeouts are configurable through
  `MICROPY_ENTER_RAW_TIMEOUT` (300 s here) and `MICROPY_EXEC_TIMEOUT` (600 s
  here), because the guest is slower than silicon.
- `close()` kills the whole process tree with `taskkill /F /T`, so a failed
  benchmark cannot leave an emulator running.

The official harness files themselves are unmodified. / 官方脚本本体未被修改。

### 3.3 Driver / 驱动方式

Each benchmark is one `run-perfbench.py` invocation in its own emulator process,
with its own log and result directory (`drive_perfbench.py`, outside the repo).
Isolation matters because a guest fault or a stalled benchmark must not poison
the following ones; a 900 s supervisor timeout kills the tree if a guest stops
making progress. Up to three benchmarks run concurrently per firmware.

每个基准使用独立的 `run-perfbench.py` 调用与独立模拟器进程，互不影响；驱动在
仓库外，超时会杀掉整个进程树。每个固件最多 3 个基准并发。

### 3.4 Command / 命令

```powershell
$env:MICROPY_CPYTHON3 = "py"
$env:MICROPY_ENTER_RAW_TIMEOUT = "300"
$env:MICROPY_EXEC_TIMEOUT = "600"

py -u tests/run-perfbench.py `
  -t 'exec:"E:\...\rp2350-cli.exe" repl --file "E:\...\RPI_PICO2-20260824-v1.29.0.uf2" --stdio --max-cycles 0' `
  -a 1 -r results 150 512 perf_bench/bm_float.py
```

---

## 4. Results / 结果

Table columns / 列说明:

- **ticks (µs)**: guest-reported `ticks_us` for the timed region.
- **score**: the harness score `1e6 * norm / ticks` (higher is better).
- **Arm/RISC-V ticks**: ratio of guest ticks; `< 1` means Arm needed fewer
  simulated cycles for the same work.
- **Status**: `pass` means the result matched the CPython truth value; failures
  are listed with the harness reason.

| Benchmark | Arm ticks (µs) | Arm score | RISC-V ticks (µs) | RISC-V score | Arm/RISC-V ticks | Status |
|---|---:|---:|---:|---:|---:|---|
| `bm_chaos` | 68,267 | 732 | 87,200 | 573 | 0.78x | pass |
| `bm_fannkuch` | 40,533 | 148 | 48,400 | 124 | 0.84x | pass |
| `bm_fft` | 145,733 | 5,270 | 237,600 | 3,232 | 0.61x | pass |
| `bm_float` | 21,666 | 11,539 | 49,534 | 5,047 | 0.44x | pass |
| `bm_hexiom` | 94,334 | 106 | 111,867 | 89 | 0.84x | pass |
| `bm_nqueens` | 118,666 | 8,427 | 142,467 | 7,019 | 0.83x | pass |
| `bm_pidigits` | 48,200 | 1,349 | 58,934 | 1,103 | 0.82x | pass |
| `bm_wordcount` | 70,400 | 114 | 86,933 | 92 | 0.81x | pass |
| `core_import_mpy_multi` | 38,133 | 1,311 | 43,400 | 1,152 | 0.88x | pass |
| `core_import_mpy_single` | 4,600 | 217 | 5,133 | 195 | 0.90x | pass |
| `core_locals` | 84,067 | 95 | 91,800 | 87 | 0.92x | pass |
| `core_qstr` | 12,133 | 330 | 12,800 | 312 | 0.95x | pass |
| `core_str` | 127,534 | 47 | 128,467 | 47 | 0.99x | pass |
| `core_yield_from` | 33,534 | 596 | 43,467 | 460 | 0.77x | pass |
| `misc_aes` | 36,200 | 884 | 46,533 | 688 | 0.78x | pass |
| `misc_mandel` | 59,866 | 6,682 | 108,867 | 3,674 | 0.55x | pass |
| `misc_pystone` | 60,467 | 4,961 | 72,067 | 4,163 | 0.84x | pass |
| `misc_raytrace` | 66,867 | 748 | 83,600 | 598 | 0.80x | pass |
| `viper_call0` | 33,600 | 893 | 36,000 | 833 | 0.93x | pass |
| `viper_call1a` | 34,867 | 860 | 37,200 | 806 | 0.94x | pass |
| `viper_call1b` | 42,066 | 713 | 44,800 | 670 | 0.94x | pass |
| `viper_call1c` | 42,000 | 714 | 44,867 | 669 | 0.94x | pass |
| `viper_call2a` | 35,600 | 843 | 38,000 | 789 | 0.94x | pass |
| `viper_call2b` | 47,867 | 627 | 51,000 | 588 | 0.94x | pass |

**Totals / 汇总**: Arm **24/24 pass**, RISC-V **24/24 pass**, no `SKIP` on either
firmware. Every result matched the CPython truth value.
Arm **24/24 通过**，RISC-V **24/24 通过**，两固件均无 `SKIP`，全部结果与
CPython 真值一致。

No benchmark was skipped by the feature checks on either firmware: both builds
report `complex` and `native` support, so `bm_fft`, `misc_mandel` and all
`viper_*` benchmarks ran.
两个固件的特性检查均支持 `complex` 与 `native`，因此 24 个基准全部实际执行，
没有 SKIP。

---

## 5. Findings / 发现

### 5.1 Arm floating point gap (fixed) / Arm 浮点缺口（已修复）

The first Arm run failed three benchmarks:

| Benchmark | Harness result | Root cause |
|---|---|---|
| `bm_float` | `FAIL truth` (`Point(inf, inf, inf)` instead of `Point(0.8939, 1.0000, 0.4472)`) | VFP arithmetic skipped |
| `bm_fft` | `ValueError: math domain error` | wrong float inputs derived from skipped VFP ops |
| `bm_chaos` | `ValueError: Function value not in domain` | same |

The emulator logged skipped `0xEE...` instructions: **106 distinct encodings**
across the three benchmarks (33 in `bm_float` alone, e.g. `0xEE266A45`,
`0xEEB07AE7`, `0xEEF87AE7`). The FPU module implemented data movement only
("`VLDR`/`VSTR`/`VLDM`/`VSTM`/`VMOV`/`VMRS`/`VMSR`) and explicitly deferred
"all floating-point arithmetic and comparisons". `rp2350-arm` now implements the
missing FPv5-SP arithmetic, comparison and conversion families
(VADD/VSUB/VMUL/VNMUL/VMLA/VMLS/VNMLA/VNMLS/VDIV/VSQRT/VABS/VNEG, VCMP/VCMPE,
VCVT integer↔float, `VMOV.F32 Sd,Sm`).
After the fix the same harness runs produce the CPython truth values, all 24 Arm
benchmarks pass (including the three that previously failed), and no `0xEE...`
undefined-instruction warnings remain. See
[crates/rp2350-arm.md](crates/rp2350-arm.md) for the supported subset and the
documented limits (double-precision arithmetic and lazy FP stacking are still
out of scope).

首次 Arm 运行有 3 个基准失败（`bm_float` 真值不符，`bm_fft`/`bm_chaos` 数学域
错误），根因是 FPU 只实现了数据搬移、算术/比较/转换全部按未定义指令跳过。
`rp2350-arm` 已补齐这些指令族；修复后真值一致，且不再出现 `0xEE...` 告警。

### 5.2 RISC-V / RISC-V 侧

All 24 benchmarks passed the CPython truth check. No undefined-instruction
warnings were logged. The only recurring diagnostic is the benign one-shot
unmodelled APB read at `0x40078018` (UART1 flag register), which returns zero.

RISC-V 侧 24 个基准全部通过 CPython 真值校验，无未定义指令告警；唯一重复出现
的是 `0x40078018`（UART1 标志寄存器）的一次性未建模读，返回 0，无副作用。

### 5.3 Cross-check with the CLI `perfbench` / 与 CLI `perfbench` 交叉验证

The CLI has an independent `perfbench` subcommand that assembles the same
benchmark + `benchrun.py` and parses the same `ticks norm result` line. Its
numbers agree with the official harness within 0.3 %, which confirms that the
stdio bridge adds no measurable perturbation:

| Benchmark | CLI `perfbench` ticks | Harness ticks |
|---|---:|---:|
| Arm `bm_nqueens` | 118,666 | 118,666 |
| Arm `bm_float` | 21,733 | 21,666 |
| RISC-V `bm_nqueens` | 142,466 | 142,467 |
| RISC-V `bm_float` | 49,533 | 49,534 |

### 5.4 Determinism / 确定性

Guest ticks depend on the executed instruction stream and on when USB
interrupts are delivered, not on host speed or parallel load. Repeat runs of the
same benchmark on the same firmware agree to within a few timer ticks
(<0.2%; at most 67 ticks in the runs above), so the `-a 1` choice reports
`0.0000` standard deviation. Use `-a 8` when variance is the goal, at 8× the
wall-clock cost.

客户机 ticks 取决于执行的指令流与 USB 中断的送达相位，不受宿主速度或并发负载
影响。同一固件的重复运行结果相差不超过几个计时 tick（<0.2%；上述运行中最大
67 tick）；`-a 1` 因而标准差为 0。如需方差信息可用 `-a 8`，但墙钟开销为 8 倍。

---

## 6. Caveats / 说明

- **Not silicon MCPS / 不是硅片 MCPS**: guest `ticks_us` is derived from the
  emulator's 150 MHz simulated timebase (`cycles / 150`), with one instruction
  counted as one cycle. There is no CPI, cache, flash wait-state or bus-latency
  model, so the absolute numbers describe the emulator's simulated time, not a
  real Pico 2. The useful signal is the **relative** Arm vs RISC-V comparison
  and the per-benchmark pass/fail status.
- **Wall-clock / 墙钟**: the host executes on the order of a few million
  instructions per second; the suite takes minutes, not the seconds it takes on
  hardware. Wall-clock time is not reported as a performance metric.
- **Coverage / 覆盖**: these are the 24 upstream `perf_bench` files at the
  harness revision above. They exercise the bytecode VM, object model, floats,
  complex numbers, Viper and core string/import paths; they do not exercise
  PIO, DMA, HSTX or other peripherals.
- **Lazy FP stacking / 惰性浮点压栈**: still unmodelled; the benchmarks do not
  take exceptions with live FP state, so this does not affect the results.

---

## 7. Reproduction / 复现

1. Build the CLI: `cargo build --release -p rp2350-cli`.
2. Sparse-checkout `tests/` and `tools/` from the harness revision above.
3. Apply the local `tools/pyboard.py` Windows shim (Section 3.2).
4. Run `run-perfbench.py` with the `exec:` target shown in Section 3.4 for each
   firmware and each `perf_bench/*.py` file, or use the per-benchmark driver.
5. Compare the emitted `ticks score` lines; failures print `FAIL truth`,
   `CRASH: ...` or `SKIP: ...` from the official harness.
