# rp2350-cli — Command Line Interface / 命令行界面

> **Crate**: `rp2350-cli` | **Path**: `crates/rp2350-cli` | **Status**: ✅ Skeleton (Phase 1)

---

## Overview / 概述

The main binary entry point for the RP2350 simulator. Provides multiple subcommands via `clap` for different execution modes: standalone simulation, GDB debug server, GPIO wiring management, device listing, and MCP server for AgentFuse integration.

RP2350 模拟器的主入口二进制程序。通过 `clap` 提供多种执行模式的子命令：独立模拟、GDB 调试服务器、GPIO 接线管理、设备列表、和 AgentFuse 集成的 MCP 服务器。

---

## Subcommands / 子命令

```bash
rp2350 run <firmware>        # Run firmware in simulator mode
    [--headless]             #   No GUI, headless mode
    [--mcp-port <PORT>]      #   Enable MCP JSON-RPC on this port
    [--plugin-dir <DIR>]     #   Plugin search directory
    [--max-cycles <N>]       #   Stop simulation after N cycles

rp2350 gdb <firmware>         # Start GDB RSP server
    [--port <PORT>]           #   GDB listen port (default: 3333)
    [--headless]              #   No GUI in GDB mode

rp2350 wire                   # Manage GPIO wiring
    list                      #   List all wired connections
    add <PIN> <DEVICE>/<PORT> #   Wire a GPIO pin to a device port
    remove <PIN>              #   Remove wiring for a pin

rp2350 list-devices [--json]  # List available device plugins

rp2350 mcp-serve              # Start MCP server (AgentFuse)
    [--port <PORT>]           #   MCP listen port (default: 9000)
    [--plugin-dir <DIR>]      #   Plugin search directory
```

### Headless Mode / 无头模式

No GUI window is created. All output goes to stdout / log / MCP. The `--mcp-port` flag enables the MCP JSON-RPC server for remote state queries even in headless mode.

不创建 GUI 窗口。所有输出到 stdout / log / MCP。`--mcp-port` 标志启用 MCP JSON-RPC 服务器，即使在无头模式下也可以远程查询状态。

### MCP Integration / MCP 集成

```bash
# Terminal 1: Start emulator with MCP
rp2350 run firmware.elf --headless --mcp-port 9000 --plugin-dir ./plugins

# Terminal 2: Query device states via MCP
curl -X POST http://localhost:9000/mcp -d '{"jsonrpc":"2.0","method":"query_all","id":1}'

# Or via AgentFuse IDE integration (auto-discovery on port 9000)
```

---

## Dependencies / 依赖

| Crate | Why |
|-------|-----|
| `clap` 4.x (derive) | CLI argument parsing with subcommands |
| `anyhow` | Error handling with context chain |
| `log` + `env_logger` | Logging output |
| `rp2350-core` | Simulator core |
| `rp2350-virt-periph` | DeviceManager, WiringTable, PluginLoader |
| `rp2350-plugin` | RpPlugin trait (for listing) |
| `rp2350-gdb` | GDB server (gdb subcommand) |
| `rp2350-egui` | GUI app (non-headless mode) |
| `rp2350-mcp` | MCP server (mcp-serve / --mcp-port) |
| `tokio` | Async runtime for GDB/MCP servers |

---

## Future Work / 后续计划

- [ ] `rp2350 wire config` — import/export wiring config from JSON
- [ ] `rp2350 flash <firmware>` — flash firmware to virtual XIP
- [ ] `rp2350 screenshot <file.png>` — HSTX frame capture
- [ ] `rp2350 serve` — combined GDB + MCP + WebSocket (WASM bridge)
- [ ] `rp2350 benchmark` — cycle-counted performance measurement