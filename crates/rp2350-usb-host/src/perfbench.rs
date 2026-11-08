//! Benchmark driver equivalent to `tests/run-perfbench.py`
//! 等价于 `tests/run-perfbench.py` 的基准驱动
//!
//! `run-perfbench.py` concatenates the benchmark file with
//! `tests/perf_bench/benchrun.py`, appends `bm_run(N, M)` and expects the
//! device to print one line of `time norm result` where `time` is measured in
//! `time.ticks_us()` units and `norm` is time divided by `N * M`.
//! `run-perfbench.py` 将基准文件与 `tests/perf_bench/benchrun.py` 拼接，追加
//! `bm_run(N, M)`，并期望设备打印一行 `time norm result`；其中 `time` 以
//! `time.ticks_us()` 为单位，`norm` 为时间除以 `N * M`。

use crate::repl::{ByteLink, ExecOutput, RawReplSession, ReplError};

/// One measured benchmark result / 一次基准测量结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchResult {
    /// `time.ticks_us()` difference, in microseconds / `time.ticks_us()` 差值（微秒）
    pub ticks_us: i64,
    /// Normalised time per iteration / 每轮迭代的归一化时间
    pub norm: i64,
    /// Benchmark return value rendered by the device / 设备渲染的基准返回值
    pub result: String,
}

/// Outcome of one benchmark script / 单次基准脚本的结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenchOutcome {
    /// The device printed `SKIP` / 设备打印了 `SKIP`
    Skip,
    /// A measurement was produced / 产生了测量结果
    Measured(BenchResult),
}

/// Benchmark driver failures / 基准驱动失败
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenchError {
    /// The raw REPL transaction failed / raw REPL 事务失败
    Repl(ReplError),
    /// The device raised an exception / 设备抛出异常
    Fatal(String),
    /// The output did not follow the `time norm result` contract
    /// 输出不符合 `time norm result` 约定
    BadOutput(String),
}

/// Run one benchmark script and parse its output / 运行一个基准脚本并解析输出
///
/// # Errors
/// Returns [`BenchError::Repl`] for protocol failures,
/// [`BenchError::Fatal`] when stderr is non-empty, and
/// [`BenchError::BadOutput`] when stdout does not match the contract.
/// 协议失败返回 [`BenchError::Repl`]，标准错误非空返回 [`BenchError::Fatal`]，
/// 标准输出不符合约定时返回 [`BenchError::BadOutput`]。
pub fn run_script<L: ByteLink>(
    session: &mut RawReplSession<L>,
    script: &[u8],
) -> Result<BenchOutcome, BenchError> {
    let output = session.exec_raw(script).map_err(BenchError::Repl)?;
    if !output.stderr.is_empty() {
        return Err(BenchError::Fatal(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    parse_output(&output)
}

/// Parse the device's `time norm result` line / 解析设备的 `time norm result` 行
fn parse_output(output: &ExecOutput) -> Result<BenchOutcome, BenchError> {
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| BenchError::BadOutput("non-UTF-8 benchmark output".into()))?
        .trim();
    if text == "SKIP" {
        return Ok(BenchOutcome::Skip);
    }
    let mut fields = text.split_whitespace();
    let ticks_us = parse_i64(fields.next(), text)?;
    let norm = parse_i64(fields.next(), text)?;
    let result = fields.collect::<Vec<_>>().join(" ");
    Ok(BenchOutcome::Measured(BenchResult {
        ticks_us,
        norm,
        result,
    }))
}

fn parse_i64(field: Option<&str>, text: &str) -> Result<i64, BenchError> {
    field
        .ok_or_else(|| BenchError::BadOutput(format!("missing field in {text:?}")))?
        .parse::<i64>()
        .map_err(|_| BenchError::BadOutput(format!("non-numeric field in {text:?}")))
}

/// Build the same script `run-perfbench.py` sends to the target
/// 构建与 `run-perfbench.py` 发送给目标的相同脚本
///
/// The result is `import sys; sys.path.remove('')` followed by the benchmark,
/// the shared `benchrun.py` helper and a final `bm_run(N, M)` call.
/// 结果为 `import sys; sys.path.remove('')`、基准文件、共享的 `benchrun.py`
/// 辅助代码，以及末尾的 `bm_run(N, M)` 调用。
#[must_use]
pub fn assemble_bench_script(benchmark: &[u8], benchrun: &[u8], n: u32, m: u32) -> Vec<u8> {
    let mut script = b"import sys\nsys.path.remove('')\n\n".to_vec();
    script.extend_from_slice(benchmark);
    script.extend_from_slice(benchrun);
    script.extend_from_slice(format!("bm_run({n}, {m})\n").as_bytes());
    script
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(stdout: &[u8]) -> ExecOutput {
        ExecOutput {
            stdout: stdout.to_vec(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn parses_a_measurement() {
        let outcome = parse_output(&output(b"1234 56 7\r\n")).unwrap();
        let BenchOutcome::Measured(result) = outcome else {
            panic!("expected a measurement");
        };
        assert_eq!(result.ticks_us, 1234);
        assert_eq!(result.norm, 56);
        assert_eq!(result.result, "7");
    }

    #[test]
    fn preserves_text_results() {
        let BenchOutcome::Measured(result) = parse_output(&output(b"10 1 None\n")).unwrap() else {
            panic!("expected a measurement");
        };
        assert_eq!(result.result, "None");
    }

    #[test]
    fn skips_and_rejects_malformed_output() {
        assert_eq!(
            parse_output(&output(b"SKIP\r\n")).unwrap(),
            BenchOutcome::Skip
        );
        assert!(matches!(
            parse_output(&output(b"not-a-number 1 2\n")),
            Err(BenchError::BadOutput(_))
        ));
        assert!(matches!(
            parse_output(&output(b"12\n")),
            Err(BenchError::BadOutput(_))
        ));
    }

    #[test]
    fn assembles_the_run_perfbench_script() {
        let script = assemble_bench_script(
            b"def bm(n, m):\n    pass\n",
            b"def bm_run(n, m):\n    pass\n",
            8,
            64,
        );
        assert!(script.starts_with(b"import sys\nsys.path.remove('')\n\n"));
        assert!(script.ends_with(b"bm_run(8, 64)\n"));
        assert!(
            script
                .windows(16)
                .any(|window| window == b"def bm_run(n, m)")
        );
    }
}
