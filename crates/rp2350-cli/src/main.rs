//! rp2350-cli — Command-line interface entry point
//!
//! Parses CLI arguments and dispatches to the emulator, USB raw REPL,
//! perfbench, GDB server, or MCP server subcommands.
//!
//! 解析 CLI 参数并分发到模拟器、USB raw REPL、perfbench、GDB 服务器或
//! MCP 服务器子命令。

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use rp2350_cli::{play, run};

mod devices;
mod gdb_target;
mod mcp_serve;
mod perfbench;
mod repl;
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

        /// PSRAM window behind XIP chip select 1: bytes or K/M (`8M`, `8192K`)
        /// XIP 片选 1 后的 PSRAM 窗口：字节数或带 K/M 后缀（`8M`、`8192K`）
        #[arg(long, value_name = "SIZE", value_parser = run::PsramOptions::parse)]
        psram: Option<run::PsramOptions>,
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

        /// PSRAM window behind XIP chip select 1: bytes or K/M (`8M`, `8192K`)
        /// XIP 片选 1 后的 PSRAM 窗口：字节数或带 K/M 后缀（`8M`、`8192K`）
        #[arg(long, value_name = "SIZE", value_parser = run::PsramOptions::parse)]
        psram: Option<run::PsramOptions>,
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

    /// Play a guest frame buffer in a background worker
    /// 在后台 worker 中播放客户机帧缓冲
    Play {
        /// Profile file with `key = value` lines (example/gba/profile.conf)
        /// `key = value` 形式的 profile 文件（如 example/gba/profile.conf）
        #[arg(long)]
        profile: Option<PathBuf>,

        /// Firmware file (UF2 or ELF); required without --profile
        /// 固件文件（UF2 或 ELF）；没有 --profile 时必填
        #[arg(short, long)]
        file: Option<String>,

        /// Frame buffer spec `ADDR:WxH[:STRIDE]`; required without --profile
        /// 帧缓冲规格 `ADDR:WxH[:STRIDE]`；没有 --profile 时必填
        #[arg(long, value_name = "ADDR:WxH[:STRIDE]", value_parser = play::FrameSpec::parse)]
        fb: Option<play::FrameSpec>,

        /// Guest steps executed per published frame / 每发布一帧执行的客户机步数
        #[arg(long)]
        steps_per_frame: Option<u64>,

        /// Stop after this many frames (headless test mode)
        /// 跑满该帧数后停止（无 GUI 测试模式）
        #[arg(long)]
        frames: Option<u64>,

        /// Button pin bitmap (bit set = pressed = pin low); default drives no pin
        /// 按键引脚位图（置位 = 按下 = 引脚低电平）；默认不驱动任何引脚
        #[arg(long, value_name = "MASK", value_parser = play::parse_button_mask)]
        buttons: Option<u16>,

        /// Capture I2S audio words through an `AudioTap` (needs --audio-block/--audio-sm)
        /// 通过 `AudioTap` 捕获 I2S 音频字（需要 --audio-block/--audio-sm）
        #[arg(long)]
        audio: bool,

        /// PIO block that pushes the I2S words / 推送 I2S 字的 PIO 块
        #[arg(long)]
        audio_block: Option<usize>,

        /// State machine inside that PIO block / 该 PIO 块内的状态机
        #[arg(long)]
        audio_sm: Option<usize>,

        /// Sample rate stored in the audio sink / 音频 sink 的采样率
        #[arg(long)]
        audio_rate: Option<u32>,

        /// Channel count stored in the audio sink / 音频 sink 的声道数
        #[arg(long)]
        audio_channels: Option<u8>,

        /// Samples retained by the audio sink / 音频 sink 保留的样本数
        #[arg(long)]
        audio_capacity: Option<usize>,

        /// Leading steps of the first frame printed as `[trace]` lines
        /// 第一帧开头以 `[trace]` 行打印的步数
        #[arg(long)]
        trace_steps: Option<usize>,

        /// Total instruction budget (`u64::MAX` = unlimited)
        /// 总指令预算（`u64::MAX` 表示不限制）
        #[arg(long)]
        max_cycles: Option<u64>,

        /// CPU architecture override / CPU 架构覆盖
        #[arg(long, value_enum)]
        arch: Option<run::ArchArg>,

        /// Halt on undefined instructions instead of skipping them
        /// 遇到未定义指令时停机而不是跳过
        #[arg(long)]
        strict_decode: bool,

        /// PSRAM window behind XIP chip select 1: bytes or K/M (`8M`, `8192K`)
        /// XIP 片选 1 后的 PSRAM 窗口：字节数或带 K/M 后缀（`8M`、`8192K`）
        #[arg(long, value_name = "SIZE", value_parser = run::PsramOptions::parse)]
        psram: Option<run::PsramOptions>,
    },

    /// Start GDB server / 启动 GDB 服务器
    Gdb {
        /// Firmware file (ELF) / 固件文件（ELF）
        #[arg(short, long)]
        file: String,

        /// GDB listen port / GDB 监听端口
        #[arg(short, long, default_value = "3333")]
        port: u16,

        /// PSRAM window behind XIP chip select 1: bytes or K/M (`8M`, `8192K`)
        /// XIP 片选 1 后的 PSRAM 窗口：字节数或带 K/M 后缀（`8M`、`8192K`）
        #[arg(long, value_name = "SIZE", value_parser = run::PsramOptions::parse)]
        psram: Option<run::PsramOptions>,
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

// 子命令分发是纯样板代码，行数由 CLI 表面决定，拆分反而降低可读性。
// Subcommand dispatch is boilerplate whose length follows the CLI surface;
// splitting it would only hurt readability.
#[allow(clippy::too_many_lines)]
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
            psram,
        } => {
            let options = run::RunOptions {
                file: file.into(),
                arch,
                max_cycles,
                trace_steps,
                strict_decode,
                psram,
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
            psram,
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
                psram,
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
        Commands::Play {
            profile,
            file,
            fb,
            steps_per_frame,
            frames,
            buttons,
            audio,
            audio_block,
            audio_sm,
            audio_rate,
            audio_channels,
            audio_capacity,
            trace_steps,
            max_cycles,
            arch,
            strict_decode,
            psram,
        } => {
            let args = play::PlayArgs {
                profile,
                file: file.map(Into::into),
                frame: fb,
                steps_per_frame,
                max_frames: frames,
                buttons,
                audio,
                audio_block,
                audio_sm,
                audio_rate,
                audio_channels,
                audio_capacity,
                arch,
                strict_decode,
                max_cycles,
                trace_steps,
                psram,
            };
            finish(play::run(&args), "play failed / 播放失败");
        }
        Commands::Gdb { file, port, psram } => {
            let options = gdb_target::GdbOptions {
                file: file.into(),
                port,
                psram,
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
    use std::path::Path;

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

    #[test]
    fn play_parses_frame_spec_and_headless_frame_limit() {
        let cli = parse(&[
            "play",
            "--file",
            "fw.uf2",
            "--fb",
            "0x20000000:240x160",
            "--steps-per-frame",
            "2500000",
            "--frames",
            "2",
            "--buttons",
            "0x3FF",
        ])
        .expect("parse play");
        let Commands::Play {
            fb,
            frames,
            buttons,
            steps_per_frame,
            ..
        } = cli.command
        else {
            panic!("expected the play subcommand");
        };
        let fb = fb.expect("explicit frame buffer");
        assert_eq!(fb.base, 0x2000_0000);
        assert_eq!(fb.width, 240);
        assert_eq!(fb.height, 160);
        assert_eq!(fb.stride_bytes, 480);
        assert_eq!(frames, Some(2));
        assert_eq!(buttons, Some(0x3FF));
        assert_eq!(steps_per_frame, Some(2_500_000));
    }

    #[test]
    fn play_defaults_are_left_unset_for_the_profile_merge() {
        // 无 `--profile` 时所有示例相关项都没有隐含默认值，由运行期解析报错。
        // Without `--profile` nothing example-specific has an implicit default;
        // the runtime resolution reports what is missing.
        let cli = parse(&["play", "--frames", "2"]).expect("parse play");
        let Commands::Play {
            profile,
            file,
            fb,
            steps_per_frame,
            buttons,
            audio,
            audio_block,
            audio_rate,
            max_cycles,
            trace_steps,
            psram,
            ..
        } = cli.command
        else {
            panic!("expected the play subcommand");
        };
        assert!(profile.is_none());
        assert!(file.is_none());
        assert!(fb.is_none());
        assert!(steps_per_frame.is_none());
        assert!(buttons.is_none());
        assert!(!audio);
        assert!(audio_block.is_none());
        assert!(audio_rate.is_none());
        assert!(max_cycles.is_none());
        assert!(trace_steps.is_none());
        assert!(psram.is_none());
    }

    #[test]
    fn run_and_repl_parse_the_psram_size() {
        let cli = parse(&["run", "--file", "fw.uf2", "--psram", "8M"]).expect("parse run");
        let Commands::Run { psram, .. } = cli.command else {
            panic!("expected the run subcommand");
        };
        assert_eq!(
            psram,
            Some(run::PsramOptions {
                size_bytes: 8 * 1024 * 1024
            })
        );

        let cli = parse(&["repl", "--file", "fw.uf2", "--psram", "1024K"]).expect("parse repl");
        let Commands::Repl { psram, .. } = cli.command else {
            panic!("expected the repl subcommand");
        };
        assert_eq!(
            psram,
            Some(run::PsramOptions {
                size_bytes: 1024 * 1024
            })
        );
    }

    #[test]
    fn play_and_gdb_parse_the_psram_size() {
        let cli = parse(&["play", "--psram", "8388608"]).expect("parse play");
        let Commands::Play { psram, .. } = cli.command else {
            panic!("expected the play subcommand");
        };
        assert_eq!(psram.map(|psram| psram.size_bytes), Some(8_388_608));

        let cli = parse(&["gdb", "--file", "fw.elf", "--psram", "16MiB"]).expect("parse gdb");
        let Commands::Gdb { psram, .. } = cli.command else {
            panic!("expected the gdb subcommand");
        };
        assert_eq!(psram.map(|psram| psram.size_bytes), Some(16 * 1024 * 1024));
    }

    #[test]
    fn psram_rejects_zero_and_misspelled_sizes() {
        let cases: [&[&str]; 4] = [
            &["run", "--file", "fw.uf2", "--psram", "0"],
            &["repl", "--file", "fw.uf2", "--psram=-8M"],
            &["play", "--psram", "8G"],
            &["gdb", "--file", "fw.elf", "--psram", "eight"],
        ];
        for args in cases {
            let error = parse(args).expect_err("PSRAM size must be validated");
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::ValueValidation,
                "args={args:?}"
            );
        }
    }

    #[test]
    fn play_parses_profile_audio_wiring() {
        let cli = parse(&[
            "play",
            "--profile",
            "example/gba/profile.conf",
            "--audio",
            "--audio-block",
            "2",
            "--audio-sm",
            "0",
            "--audio-rate",
            "13440",
        ])
        .expect("parse play");
        let Commands::Play {
            profile,
            audio,
            audio_block,
            audio_sm,
            audio_rate,
            ..
        } = cli.command
        else {
            panic!("expected the play subcommand");
        };
        assert_eq!(
            profile.as_deref(),
            Some(Path::new("example/gba/profile.conf"))
        );
        assert!(audio);
        assert_eq!(audio_block, Some(2));
        assert_eq!(audio_sm, Some(0));
        assert_eq!(audio_rate, Some(13_440));
    }
}
