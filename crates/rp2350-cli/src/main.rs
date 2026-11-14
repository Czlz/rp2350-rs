//! rp2350-cli — Command-line interface entry point
//!
//! Parses CLI arguments and dispatches to the emulator, USB raw REPL,
//! perfbench, GDB server, or MCP server subcommands.
//!
//! 解析 CLI 参数并分发到模拟器、USB raw REPL、perfbench、GDB 服务器或
//! MCP 服务器子命令。

use clap::{Parser, Subcommand};

mod gdb_target;
mod perfbench;
mod repl;
mod run;
#[cfg(test)]
mod test_support;

/// RP2350 Emulator — cycle-accurate ARM/RISC-V simulator
/// RP2350 模拟器 — 精确周期 ARM/RISC-V 仿真器
#[derive(Parser, Debug)]
#[command(name = "rp2350", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose logging / 启用详细日志
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Disable GUI (headless mode) / 禁用 GUI（无头模式）
    #[arg(long, global = true)]
    headless: bool,
}

/// CLI subcommands / CLI 子命令
#[derive(Subcommand, Debug)]
enum Commands {
    /// Run the emulator with the given firmware / 使用给定固件运行模拟器
    Run {
        /// Firmware file (UF2 or ELF) / 固件文件（UF2 或 ELF）
        #[arg(short, long)]
        file: String,

        /// CPU architecture override / CPU 架构覆盖
        #[arg(long, value_enum)]
        arch: Option<run::ArchArg>,

        /// Instruction budget before stopping / 停止前的指令预算
        #[arg(long, default_value_t = run::DEFAULT_RUN_MAX_CYCLES)]
        max_cycles: u64,

        /// Number of boot-trace steps to print / 打印的启动轨迹步数
        #[arg(long, default_value_t = 16)]
        trace_steps: usize,

        /// Halt on undefined instructions instead of skipping them
        /// 遇到未定义指令时停机而不是跳过
        #[arg(long)]
        strict_decode: bool,
    },

    /// Open the USB CDC raw REPL of a booted firmware
    /// 打开已启动固件的 USB CDC raw REPL
    Repl {
        /// Firmware file (UF2 or ELF) / 固件文件（UF2 或 ELF）
        #[arg(short, long)]
        file: String,

        /// CPU architecture override (UF2 family is auto-detected by default)
        /// CPU 架构覆盖（默认由 UF2 family 自动识别）
        #[arg(long, value_enum)]
        arch: Option<run::ArchArg>,

        /// Python source to execute after entering raw REPL
        /// 进入 raw REPL 后执行的 Python 源码
        #[arg(long)]
        exec: Option<String>,

        /// Total simulated instruction budget / 总模拟指令预算
        #[arg(long, default_value_t = repl::DEFAULT_MAX_CYCLES)]
        max_cycles: u64,

        /// Instruction budget allowed for USB enumeration / USB 枚举指令预算
        #[arg(long, default_value_t = repl::DEFAULT_ENUM_CYCLES)]
        enum_cycles: u64,

        /// Skip the Ctrl-D soft reset when entering raw REPL
        /// 进入 raw REPL 时跳过 Ctrl-D 软复位
        #[arg(long)]
        no_soft_reset: bool,

        /// Halt on undefined instructions instead of skipping them
        /// 遇到未定义指令时停机而不是跳过
        #[arg(long)]
        strict_decode: bool,
    },

    /// Run a `MicroPython` benchmark through the USB CDC raw REPL
    /// 通过 USB CDC raw REPL 运行 `MicroPython` 基准测试
    Perfbench {
        /// Firmware file (UF2 or ELF) / 固件文件（UF2 或 ELF）
        #[arg(short, long)]
        file: String,

        /// Benchmark file (`bm_*.py`) / 基准文件（`bm_*.py`）
        #[arg(long)]
        bench: String,

        /// Shared `benchrun.py` helper / 共享的 `benchrun.py` 辅助文件
        #[arg(long)]
        benchrun: String,

        /// Repetition count `N` passed to `bm_run(N, M)`
        /// 传给 `bm_run(N, M)` 的重复次数 `N`
        #[arg(long, default_value_t = perfbench::DEFAULT_N)]
        n: u32,

        /// Repetition count `M` passed to `bm_run(N, M)`
        /// 传给 `bm_run(N, M)` 的重复次数 `M`
        #[arg(long, default_value_t = perfbench::DEFAULT_M)]
        m: u32,

        /// Total simulated instruction budget / 总模拟指令预算
        #[arg(long, default_value_t = repl::DEFAULT_MAX_CYCLES)]
        max_cycles: u64,

        /// Instruction budget allowed for USB enumeration / USB 枚举指令预算
        #[arg(long, default_value_t = repl::DEFAULT_ENUM_CYCLES)]
        enum_cycles: u64,

        /// Skip the Ctrl-D soft reset when entering raw REPL
        /// 进入 raw REPL 时跳过 Ctrl-D 软复位
        #[arg(long)]
        no_soft_reset: bool,

        /// Halt on undefined instructions instead of skipping them
        /// 遇到未定义指令时停机而不是跳过
        #[arg(long)]
        strict_decode: bool,
    },

    /// Start GDB server / 启动 GDB 服务器
    Gdb {
        /// Firmware file (ELF) / 固件文件（ELF）
        #[arg(short, long)]
        file: String,

        /// GDB listen port / GDB 监听端口
        #[arg(short, long, default_value = "3333")]
        port: u16,
    },

    /// Wire a device port to a GPIO pin / 将设备端口连接到 GPIO 引脚
    Wire {
        /// Device ID / 设备 ID
        #[arg(short, long)]
        device: String,

        /// Port index / 端口索引
        #[arg(short = 'p', long)]
        port: usize,

        /// GPIO pin number / GPIO 引脚号
        #[arg(short = 'g', long)]
        gpio: u8,
    },

    /// List all registered devices / 列出所有已注册设备
    ListDevices,

    /// Start MCP server (`AgentFuse` integration) / 启动 MCP 服务器（`AgentFuse` 集成）
    McpServe {
        /// MCP listen port / MCP 监听端口
        #[arg(short, long, default_value = "8080")]
        port: u16,
    },
}

fn main() {
    rp2350_core::watch::init();
    let cli = Cli::parse();

    if cli.verbose {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();
    } else {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    }

    match cli.command {
        Commands::Run {
            file,
            arch,
            max_cycles,
            trace_steps,
            strict_decode,
        } => {
            let options = run::RunOptions {
                file: file.into(),
                arch,
                max_cycles,
                trace_steps,
                strict_decode,
            };
            if let Err(error) = run::run(&options) {
                log::error!("run failed / 运行失败: {error:#}");
                std::process::exit(1);
            }
        }
        Commands::Repl {
            file,
            arch,
            exec,
            max_cycles,
            enum_cycles,
            no_soft_reset,
            strict_decode,
        } => {
            let options = repl::ReplOptions {
                file: file.into(),
                arch,
                exec,
                max_cycles,
                enum_cycles,
                soft_reset: !no_soft_reset,
                strict_decode,
            };
            if let Err(error) = repl::run(&options) {
                log::error!("repl failed / REPL 失败: {error:#}");
                std::process::exit(1);
            }
        }
        Commands::Perfbench {
            file,
            bench,
            benchrun,
            n,
            m,
            max_cycles,
            enum_cycles,
            no_soft_reset,
            strict_decode,
        } => {
            let options = perfbench::PerfbenchOptions {
                file: file.into(),
                bench: bench.into(),
                benchrun: benchrun.into(),
                n,
                m,
                max_cycles,
                enum_cycles,
                soft_reset: !no_soft_reset,
                strict_decode,
            };
            if let Err(error) = perfbench::run(&options) {
                log::error!("perfbench failed / 基准测试失败: {error:#}");
                std::process::exit(1);
            }
        }
        Commands::Gdb { file, port } => {
            let options = gdb_target::GdbOptions {
                file: file.into(),
                port,
            };
            if let Err(error) = gdb_target::run(&options) {
                log::error!("gdb failed / GDB 失败: {error:#}");
                std::process::exit(1);
            }
        }
        Commands::Wire { device, port, gpio } => {
            log::info!("Wiring device '{device}' port {port} to GPIO {gpio} (stub)");
        }
        Commands::ListDevices => {
            log::info!("No devices registered yet (stub)");
        }
        Commands::McpServe { port } => {
            log::info!("Starting MCP server on port {port} (stub)");
            log::info!("MCP server not yet implemented (Phase 3)");
        }
    }
}
