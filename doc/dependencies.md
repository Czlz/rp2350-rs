# Third-Party Dependencies / 第三方依赖清单

> **Version**: 4.0 | **Date**: 2026-11-16

统一在 `[workspace.dependencies]` 声明，子 crate 用 `{ workspace = true }` 引用。

## Core / 核心

| Crate | Version | Usage |
|-------|---------|-------|
| `thiserror` | 2.x | 库错误类型（`#[derive(Error)]`） |
| `anyhow` | 1.x | CLI 错误上下文 |
| `log` / `env_logger` | 0.4 / 0.11 | 日志门面与输出 |
| `tracing` / `tracing-subscriber` | 0.1 / 0.3 | 结构化日志（GUI 集成） |

## Concurrency & Data / 并发与数据

| Crate | Version | Usage |
|-------|---------|-------|
| `parking_lot` | 0.12 | 互斥锁/RwLock |
| `tokio` | 1.x | GDB / MCP 服务异步运行时 |
| `serde` / `serde_json` | 1.x | 插件配置、状态快照、MCP |
| `bytemuck` | 1.x | 寄存器数组零拷贝 |
| `smallvec` | 1.x | 小集合栈分配 |
| `bitflags` | 2.x | 寄存器位定义 |
| `uuid` | 1.x | 设备实例 ID |

## Firmware Loading / 固件加载

| Crate | Version | Usage |
|-------|---------|-------|
| `elf` | 0.7 | ELF 解析 |

> UF2 由仓库内部 `rp2350-core::uf2` 解析，不使用第三方 crate；Intel HEX 解析不在
> V1 范围（无调用方，相关 workspace 条目已删除）。

## GUI / 图形界面

| Crate | Version | Usage |
|-------|---------|-------|
| `eframe` / `egui` | 0.37 | 应用框架与即时模式 GUI |
| `egui-wgpu` / `wgpu` | 0.37 / 24.x | wgpu 渲染后端 |
| `egui-winit` / `winit` | 0.37 / 0.30 | 窗口与事件 |
| `egui_plot` | 0.37 | 曲线/时序图 |

## Plugin FFI / 插件 FFI

| Crate | Version | Usage |
|-------|---------|-------|
| `libloading` | 0.8 | 动态库加载（`LoadLibrary`/`dlopen`） |

插件 ABI 由 `rp2350-plugin` 自定义 `extern "C"` 符号 + 版本号校验实现，
**不使用** `abi_stable`（该 workspace 条目已删除）。

## Audio / 音频

| Crate | Version | Feature | Usage |
|-------|---------|---------|-------|
| `cpal` | 0.18 | `playback` | 宿主声卡输出；未启用时使用 null 后端 |

## Build & CLI / 构建与命令行

| Crate | Version | Usage |
|-------|---------|-------|
| `clap` | 4.x (derive) | CLI 参数解析 |

## WebAssembly

**不属于 V1。** GUI 依赖 `tokio full`、`libloading` 与文件系统固件加载等宿主专用
crate，仓库也没有 `index.html`/`Trunk.toml` web 入口；V1 只发布原生二进制。
原先预留的 `wasm-bindgen`/`web-sys`/`js-sys`/`wee_alloc` 等未使用依赖已从
`[workspace.dependencies]` 删除。

## Dynamic Library Loading / 动态库加载

| Platform | Extension | Default search |
|----------|-----------|----------------|
| Windows | `.dll` | `plugins/`, `PATH` |
| Linux | `.so` | `plugins/`, `LD_LIBRARY_PATH` |
| macOS | `.dylib` | `plugins/`, `DYLD_LIBRARY_PATH` |

导出的 ABI 符号（由 `rp2350-plugin` 定义）：

```
rp2350_plugin_version        → fn() -> u32  (ABI version = 1)
rp2350_plugin_is_device      → fn() -> u32  (1 = DevicePlugin)
rp2350_device_plugin_create  → fn() -> *mut dyn DevicePlugin
```

所有 `libloading` 调用封装在 `rp2350-ffi` 的 `SafeLibrary`（RAII）；不使用 `#[link]`。
发布构建使用 `--release` + `lto = "fat"` + `codegen-units = 1`。
