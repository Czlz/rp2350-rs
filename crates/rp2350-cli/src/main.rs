//! rp2350-cli — Command-line interface entry point
//!
//! Parses CLI arguments and dispatches to the emulator, GDB server,
//! or MCP server subcommands.
//!
//! 解析 CLI 参数并分发到模拟器、GDB 服务器或 MCP 服务器子命令。

use clap::{Parser, Subcommand};

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
        /// Firmware file (ELF or HEX) / 固件文件（ELF 或 HEX）
        #[arg(short, long)]
        file: String,
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

    /// Start MCP server (AgentFuse integration) / 启动 MCP 服务器（AgentFuse 集成）
    McpServe {
        /// MCP listen port / MCP 监听端口
        #[arg(short, long, default_value = "8080")]
        port: u16,
    },
}

fn main() {
    let cli = Cli::parse();

    if cli.verbose {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();
    } else {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    }

    match cli.command {
        Commands::Run { file } => {
            log::info!("Running firmware: {} (stub)", file);
            log::info!("Run mode not yet implemented (Phase 3+)");
        }
        Commands::Gdb { file, port } => {
            log::info!("Starting GDB server on port {} for firmware: {} (stub)", port, file);
            log::info!("GDB server not yet implemented (Phase 7)");
        }
        Commands::Wire {
            device,
            port,
            gpio,
        } => {
            log::info!(
                "Wiring device '{}' port {} to GPIO {} (stub)",
                device,
                port,
                gpio
            );
        }
        Commands::ListDevices => {
            log::info!("No devices registered yet (stub)");
        }
        Commands::McpServe { port } => {
            log::info!("Starting MCP server on port {} (stub)", port);
            log::info!("MCP server not yet implemented (Phase 3)");
        }
    }
}