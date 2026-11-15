//! rp2350-cli — Command-line interface entry point
//!
//! Parses CLI arguments and dispatches to the emulator, USB raw REPL,
//! perfbench, GDB server, or MCP server subcommands.
//!
//! 解析 CLI 参数并分发到模拟器、USB raw REPL、perfbench、GDB 服务器或
//! MCP 服务器子命令。

use std::path::PathBuf;

use clap::{Parser, Subcommand};

mod devices;
mod gdb_target;
mod mcp_serve;
mod perfbench;
mod repl;
mod run;
mod sleep;
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

        /// Dump memory when the run stops: ADDR:LEN:PATH (repeatable)
        /// 停止时导出内存：ADDR:LEN:PATH（可重复）
        #[arg(long = "dump", value_name = "ADDR:LEN:PATH", value_parser = run::MemoryDump::parse)]
        dump: Vec<run::MemoryDump>,
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
        #[arg(long, conflicts_with = "stdio")]
        exec: Option<String>,

        /// Transparent stdin/stdout bridge to the USB CDC endpoints
        /// 在 stdin/stdout 与 USB CDC 端点之间透明桥接
        #[arg(long)]
        stdio: bool,

        /// Total simulated instruction budget (0 = unlimited)
        /// 总模拟指令预算（0 表示不限制）
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

        /// Plugin directory to scan / 扫描的插件目录
        #[arg(long, default_value = devices::DEFAULT_PLUGIN_DIR)]
        plugin_dir: PathBuf,
    },

    /// List all device plugins found in the plugin directory
    /// 列出插件目录中发现的全部设备插件
    ListDevices {
        /// Plugin directory to scan / 扫描的插件目录
        #[arg(long, default_value = devices::DEFAULT_PLUGIN_DIR)]
        plugin_dir: PathBuf,

        /// Rescan once, print the hot-reload report, then exit
        /// 重新扫描一次、打印热重载报告后退出
        #[arg(long, conflicts_with = "watch")]
        reload: bool,

        /// Keep watching the plugin directory until Ctrl-C
        /// 持续监视插件目录直至 Ctrl-C
        #[arg(long)]
        watch: bool,
    },

    /// Start MCP server (`AgentFuse` integration) / 启动 MCP 服务器（`AgentFuse` 集成）
    McpServe {
        /// MCP listen port / MCP 监听端口
        #[arg(short, long, default_value_t = mcp_serve::DEFAULT_PORT)]
        port: u16,

        /// Bearer token for `POST /mcp` (falls back to `RP2350_MCP_TOKEN`)
        /// 保护 `POST /mcp` 的 Bearer Token（缺省读取 `RP2350_MCP_TOKEN`）
        #[arg(long)]
        auth_token: Option<String>,
    },
}

fn main() {
    rp2350_core::watch::init();
    let cli = Cli::parse();

    init_logging(cli.verbose);

    match cli.command {
        Commands::Run {
            file,
            arch,
            max_cycles,
            trace_steps,
            strict_decode,
            dump,
        } => {
            let options = run::RunOptions {
                file: file.into(),
                arch,
                max_cycles,
                trace_steps,
                strict_decode,
                dumps: dump,
            };
            finish(run::run(&options), "run failed / 运行失败");
        }
        Commands::Repl {
            file,
            arch,
            exec,
            stdio,
            max_cycles,
            enum_cycles,
            no_soft_reset,
            strict_decode,
        } => {
            let options = repl::ReplOptions {
                file: file.into(),
                arch,
                exec,
                stdio,
                max_cycles,
                enum_cycles,
                soft_reset: !no_soft_reset,
                strict_decode,
            };
            finish(repl::run(&options), "repl failed / REPL 失败");
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
            finish(perfbench::run(&options), "perfbench failed / 基准测试失败");
        }
        Commands::Gdb { file, port } => {
            let options = gdb_target::GdbOptions {
                file: file.into(),
                port,
            };
            finish(gdb_target::run(&options), "gdb failed / GDB 失败");
        }
        Commands::Wire {
            device,
            port,
            gpio,
            plugin_dir,
        } => {
            finish(
                devices::wire(&plugin_dir, &device, port, gpio),
                "wire failed / 接线失败",
            );
        }
        Commands::ListDevices {
            plugin_dir,
            reload,
            watch,
        } => {
            let result = devices::list_devices_with(&plugin_dir, reload, watch);
            finish(result, "list-devices failed / 列出设备失败");
        }
        Commands::McpServe { port, auth_token } => {
            let options = mcp_serve::McpServeOptions { port, auth_token };
            finish(mcp_serve::run(&options), "mcp-serve failed / MCP 服务失败");
        }
    }
}

/// Initialize the logger: `debug` when verbose, `info` otherwise
/// 初始化日志：详细模式为 `debug`，否则为 `info`
fn init_logging(verbose: bool) {
    let default_filter = if verbose { "debug" } else { "info" };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_filter))
        .init();
}

/// Report a subcommand failure and exit with status 1 / 记录子命令失败并以状态码 1 退出
fn finish(result: anyhow::Result<()>, label: &str) {
    if let Err(error) = result {
        log::error!("{label}: {error:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("rp2350").chain(args.iter().copied()))
    }

    #[test]
    fn repl_stdio_conflicts_with_exec() {
        let error = parse(&["repl", "--file", "fw.uf2", "--stdio", "--exec", "print(1)"])
            .expect_err("--stdio must conflict with --exec");
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn repl_stdio_parses_into_options() {
        let cli = parse(&["repl", "--file", "fw.uf2", "--stdio"]).expect("parse repl");
        let Commands::Repl { stdio, exec, .. } = cli.command else {
            panic!("expected the repl subcommand");
        };
        assert!(stdio);
        assert!(exec.is_none());
    }

    #[test]
    fn mcp_serve_auth_token_parses_into_options() {
        let cli = parse(&["mcp-serve", "--port", "0", "--auth-token", "secret"])
            .expect("parse mcp-serve");
        let Commands::McpServe { port, auth_token } = cli.command else {
            panic!("expected the mcp-serve subcommand");
        };
        let options = mcp_serve::McpServeOptions { port, auth_token };
        assert_eq!(options.port, 0);
        assert_eq!(options.auth_token.as_deref(), Some("secret"));
    }

    #[test]
    fn mcp_serve_without_auth_token_leaves_it_unset() {
        let cli = parse(&["mcp-serve"]).expect("parse mcp-serve");
        let Commands::McpServe { port, auth_token } = cli.command else {
            panic!("expected the mcp-serve subcommand");
        };
        assert_eq!(port, mcp_serve::DEFAULT_PORT);
        assert!(auth_token.is_none());
    }

    #[test]
    fn list_devices_defaults_to_no_reload() {
        let cli = parse(&["list-devices"]).expect("parse list-devices");
        let Commands::ListDevices {
            plugin_dir,
            reload,
            watch,
        } = cli.command
        else {
            panic!("expected the list-devices subcommand");
        };
        assert_eq!(plugin_dir, PathBuf::from(devices::DEFAULT_PLUGIN_DIR));
        assert!(!reload);
        assert!(!watch);
    }

    #[test]
    fn list_devices_parses_reload_flag() {
        let cli = parse(&["list-devices", "--reload"]).expect("parse list-devices --reload");
        let Commands::ListDevices { reload, watch, .. } = cli.command else {
            panic!("expected the list-devices subcommand");
        };
        assert!(reload);
        assert!(!watch);
    }

    #[test]
    fn list_devices_parses_watch_flag() {
        let cli = parse(&["list-devices", "--watch", "--plugin-dir", "target/release"])
            .expect("parse list-devices --watch");
        let Commands::ListDevices {
            plugin_dir,
            reload,
            watch,
        } = cli.command
        else {
            panic!("expected the list-devices subcommand");
        };
        assert_eq!(plugin_dir, PathBuf::from("target/release"));
        assert!(watch);
        assert!(!reload);
    }

    #[test]
    fn list_devices_reload_conflicts_with_watch() {
        let error = parse(&["list-devices", "--reload", "--watch"])
            .expect_err("--reload must conflict with --watch");
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }
}
