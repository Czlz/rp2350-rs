//! `perfbench` subcommand: run a `MicroPython` benchmark through USB raw REPL
//! `perfbench` 子命令：通过 USB raw REPL 运行 `MicroPython` 基准测试
//!
//! This mirrors `tests/run-perfbench.py`: concatenate the benchmark file with
//! `tests/perf_bench/benchrun.py`, append `bm_run(N, M)`, execute the script in
//! raw REPL mode and parse the device's `ticks norm result` line. / 本子命令
//! 复现 `tests/run-perfbench.py`：将基准文件与 `tests/perf_bench/benchrun.py`
//! 拼接、追加 `bm_run(N, M)`，在 raw REPL 模式执行脚本并解析设备输出的
//! `ticks norm result` 行。

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use rp2350_usb_host::perfbench::run_script;
use rp2350_usb_host::{BenchError, BenchOutcome, BenchResult, assemble_bench_script};

use crate::repl::open_session;
use crate::run::{Machine, MachineOptions, fault_tail_lines};

/// Default first repetition count (`N` in `bm_run(N, M)`)
/// 默认的第一个重复次数（`bm_run(N, M)` 中的 `N`）
pub const DEFAULT_N: u32 = 1;

/// Default second repetition count (`M` in `bm_run(N, M)`)
/// 默认的第二个重复次数（`bm_run(N, M)` 中的 `M`）
pub const DEFAULT_M: u32 = 1;

/// Options for a `perfbench` invocation / `perfbench` 调用选项
#[derive(Debug)]
pub struct PerfbenchOptions {
    /// Firmware file (UF2 or ELF) / 固件文件（UF2 或 ELF）
    pub file: PathBuf,
    /// Benchmark file (`bm_*.py`) / 基准文件（`bm_*.py`）
    pub bench: PathBuf,
    /// Shared `benchrun.py` helper / 共享的 `benchrun.py` 辅助文件
    pub benchrun: PathBuf,
    /// First repetition count / 第一个重复次数
    pub n: u32,
    /// Second repetition count / 第二个重复次数
    pub m: u32,
    /// Total instruction budget / 总指令预算
    pub max_cycles: u64,
    /// Budget allowed before USB enumeration completes / USB 枚举完成前的预算
    pub enum_cycles: u64,
    /// Perform the Ctrl-D soft reset when entering raw REPL
    /// 进入 raw REPL 时执行 Ctrl-D 软复位
    pub soft_reset: bool,
    /// Halt on undefined instructions instead of skipping them
    /// 遇到未定义指令时停机而不是跳过
    pub strict_decode: bool,
}

/// Run one benchmark file through the USB CDC REPL
/// 通过 USB CDC REPL 运行一个基准文件
///
/// # Errors
/// Returns an error when a file cannot be read, firmware cannot be loaded, USB
/// enumeration or raw REPL times out, the device raises an exception, or its
/// output does not match the `ticks norm result` contract. / 文件无法读取、固件
/// 无法加载、USB 枚举或 raw REPL 超时、设备抛出异常，或输出不符合
/// `ticks norm result` 约定时返回错误。
pub fn run(options: &PerfbenchOptions) -> Result<()> {
    let firmware = std::fs::read(&options.file).with_context(|| {
        format!(
            "cannot read firmware / 无法读取固件: {}",
            options.file.display()
        )
    })?;
    let bench = std::fs::read(&options.bench).with_context(|| {
        format!(
            "cannot read benchmark / 无法读取基准文件: {}",
            options.bench.display()
        )
    })?;
    let benchrun = std::fs::read(&options.benchrun).with_context(|| {
        format!(
            "cannot read benchrun / 无法读取 benchrun: {}",
            options.benchrun.display()
        )
    })?;
    let script = assemble_bench_script(&bench, &benchrun, options.n, options.m);
    println!(
        "[perfbench] script: bench={} benchrun={} n={} m={} bytes={}",
        options.bench.display(),
        options.benchrun.display(),
        options.n,
        options.m,
        script.len()
    );

    let machine_options = MachineOptions {
        arch: None,
        max_cycles: options.max_cycles,
        strict_decode: options.strict_decode,
        psram: None,
    };
    let mut machine = Machine::new(&firmware, &machine_options)?;
    println!("[perfbench] {}", machine.image_description());
    println!("[perfbench] core: {}", machine.core_description());

    let result = (|| -> Result<()> {
        let mut open =
            open_session(&mut machine, options.enum_cycles).map_err(|error| anyhow!(error))?;
        println!(
            "[perfbench] USB CDC configured: IN=0x{:02X}/{} OUT=0x{:02X}/{} interface={}",
            open.endpoints.in_address,
            open.endpoints.in_max_packet,
            open.endpoints.out_address,
            open.endpoints.out_max_packet,
            open.endpoints.interface
        );
        open.session
            .enter_raw_repl(options.soft_reset)
            .map_err(|error| anyhow!("enter raw REPL failed / 进入 raw REPL 失败: {error:?}"))?;
        let outcome = run_script(&mut open.session, &script);
        let exit = open
            .session
            .exit_raw_repl()
            .map_err(|error| anyhow!("exit raw REPL failed / 退出 raw REPL 失败: {error:?}"));
        match outcome {
            Ok(outcome) => {
                report_outcome(&outcome);
                exit?;
                Ok(())
            }
            Err(error) => {
                if let Err(exit_error) = exit {
                    eprintln!("[perfbench] additionally, exit failed: {exit_error:#}");
                }
                Err(anyhow!(describe_bench_error(&error)))
            }
        }
    })();
    machine.finalize();

    let state = machine_state(&machine);
    match result {
        Ok(()) => {
            println!("[perfbench] {state}");
            Ok(())
        }
        Err(error) => {
            for line in fault_tail_lines(&mut machine) {
                eprintln!("[perfbench] {line}");
            }
            Err(error.context(state))
        }
    }
}

/// Print a measured result or SKIP / 打印测量结果或 SKIP
fn report_outcome(outcome: &BenchOutcome) {
    match outcome {
        BenchOutcome::Measured(BenchResult {
            ticks_us,
            norm,
            result,
        }) => {
            println!("[perfbench] measured: ticks_us={ticks_us} norm={norm} result={result}");
        }
        BenchOutcome::Skip => {
            println!("[perfbench] skip: device reported SKIP");
        }
    }
}

/// Render benchmark failures with enough evidence to diagnose them
/// 渲染带足够诊断证据的基准失败信息
fn describe_bench_error(error: &BenchError) -> String {
    match error {
        BenchError::Repl(error) => {
            format!("raw REPL failed / raw REPL 失败: {error:?}")
        }
        BenchError::Fatal(text) => {
            format!("device raised an exception / 设备抛出异常: {text}")
        }
        BenchError::BadOutput(text) => format!(
            "unexpected benchmark output / 基准输出不符合 '<ticks_us> <norm> <result>': {text:?}"
        ),
    }
}

/// One-line machine state used in success and failure reports
/// 成功与失败报告中使用的单行机器状态
fn machine_state(machine: &Machine) -> String {
    let stop = machine.halt_reason().map_or_else(
        || {
            if machine.budget_exhausted() {
                String::from("budget exhausted")
            } else {
                String::from("running")
            }
        },
        |reason| format!("halt: {reason}"),
    );
    format!(
        "last PC={:#010X}, steps={}/{}, simulated={} us, {stop}",
        machine.pc(),
        machine.steps(),
        machine.budget(),
        machine.time_us()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const BENCH: &[u8] = b"def bm(n, m):\n    return n\n";
    const BENCHRUN: &[u8] = b"def bm_run(n, m):\n    pass\n";

    #[test]
    fn default_repetitions_follow_run_perfbench() {
        assert_eq!((DEFAULT_N, DEFAULT_M), (1, 1));
        let script = assemble_bench_script(BENCH, BENCHRUN, DEFAULT_N, DEFAULT_M);
        assert!(script.ends_with(b"bm_run(1, 1)\n"));
        assert!(script.starts_with(b"import sys\nsys.path.remove('')\n\n"));
    }

    #[test]
    fn explicit_repetitions_reach_the_script() {
        let script = assemble_bench_script(BENCH, BENCHRUN, 8, 64);
        assert!(script.ends_with(b"bm_run(8, 64)\n"));
        assert!(script.windows(7).any(|window| window == b"def bm("));
    }

    #[test]
    fn bench_errors_keep_explicit_evidence() {
        let repl = describe_bench_error(&BenchError::Repl(rp2350_usb_host::ReplError::Timeout {
            marker: "stdout",
        }));
        assert!(repl.contains("raw REPL failed"));
        assert!(repl.contains("stdout"));

        let fatal = describe_bench_error(&BenchError::Fatal(String::from(
            "Traceback: ZeroDivisionError",
        )));
        assert!(fatal.contains("ZeroDivisionError"));

        let bad = describe_bench_error(&BenchError::BadOutput(String::from("12")));
        assert!(bad.contains("unexpected benchmark output"));
        assert!(bad.contains("\"12\""));
    }
}
