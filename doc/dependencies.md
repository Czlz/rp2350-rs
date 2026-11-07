# Third-Party Dependencies / 第三方依赖清单

> **Version**: 3.0 | **Date**: 2026-11-08

---

## Core Dependencies / 核心依赖

| Crate | Version | Usage | Why |
|-------|---------|-------|-----|
| `thiserror` | 2.x | Library error types | `#[derive(Error)]` macro, zero-cost |
| `anyhow` | 1.x | CLI error handling | `context()` chain, fast prototyping |
| `log` | 0.4 | Facade logging | Universal log macros |
| `env_logger` | 0.11 | Log output | Environment-configurable output |
| `tracing` | 0.1 | Structured logging | Span, event, subscriber for egui |
| `tracing-subscriber` | 0.3 | Log formatting | Layered subscriber for egui integration |

## Concurrency & Data / 并发与数据结构

| Crate | Version | Usage | Why |
|-------|---------|-------|-----|
| `parking_lot` | 0.12 | Synchronization | Faster Mutex/RwLock than std |
| `tokio` | 1.x (full) | GDB server async | TCP accept, connection multiplexing |
| `serde` | 1.x | Serialization | Config, plugin metadata, state save |
| `serde_json` | 1.x | JSON ser/de | Plugin descriptor format |
| `bytemuck` | 1.x | Zero-copy casting | `Pod`/`Zeroable` for register arrays |
| `smallvec` | 1.x | Small vectors | Stack-allocated for tiny collections |
| `bitflags` | 2.x | Bitfield enums | Register flag definitions |
| `uuid` | 1.x | Unique IDs | Plugin instance IDs |

## Firmware Loading / 固件加载

| Crate | Version | Usage |
|-------|---------|-------|
| `elf` | 0.7 | ELF file parsing |
| `ihex` | 3.x | Intel HEX parsing |

## ★ egui GUI / 图形界面

| Crate | Version | Usage |
|-------|---------|-------|
| `eframe` | 0.37 | egui framework (native + wasm runner) |
| `egui` | 0.37 | Immediate mode GUI core |
| `egui-wgpu` | 0.37 | wgpu render backend |
| `egui-winit` | 0.37 | winit window integration |
| `egui_plot` | 0.37 | Plot widgets (PIO timing) |
| `winit` | 0.30 | Windowing abstraction |
| `wgpu` | 24.x | GPU rendering (cross-platform native) |
| `image` | 0.25 | HSTX screenshot → PNG |

### egui Version Policy / egui 版本策略

All egui-family crates use the latest stable release. Version is specified in `[workspace.dependencies]` for unified management.
所有 egui 系列 crate 使用最新稳定版。版本在 `[workspace.dependencies]` 中统一管理。

```toml
[workspace.dependencies]
egui = "0.37"
eframe = "0.37"
egui-wgpu = "0.37"
egui-winit = "0.37"
egui_plot = "0.37"
```

## Plugin FFI / 插件 FFI

| Crate | Version | Usage |
|-------|---------|-------|
| `libloading` | 0.8 | Dynamic library loading (dlopen/LoadLibrary) |
| `abi_stable` | 0.12 | Stable ABI for cross-version plugin compat |

## Build & CLI / 构建与命令行

| Crate | Version | Usage |
|-------|---------|-------|
| `clap` | 4.x (derive) | CLI argument parsing |
| `chrono` | 0.4 | Timestamps (changelog, trace) |

## ★ WASM / Web Assembly / Web 发布

| Crate | Version | Usage |
|-------|---------|-------|
| `wasm-bindgen` | 0.2 | Rust ↔ JS interop for wasm target |
| `wasm-bindgen-futures` | 0.4 | Async runtime bridge for wasm |
| `web-sys` | 0.3 | Browser DOM APIs (canvas, events, localStorage) |
| `console_error_panic_hook` | 0.1 | Rust panic → browser console |
| `js-sys` | 0.3 | JavaScript type bindings |
| `wee_alloc` | 0.4 | WASM-optimized allocator (~1KB) |

### WASM Build Tool / Web 构建工具

| Tool | Usage |
|------|-------|
| `trunk` | WASM bundler (index.html + wasm + JS glue) |
| `wasm-pack` | WASM npm package builder (optional, for publishing) |
| `wasm-opt` | WASM binary optimizer (binaryen) |

### Plugin ABI in WASM / Web 中的插件 ABI

In WASM builds, dynamic `.dll`/`.so` loading is NOT available (no `dlopen`). Virtual devices are compiled as **static crate dependencies** instead, selected via cargo features:
WASM 构建中无法使用动态 `.dll`/`.so` 加载（无 `dlopen`）。虚拟设备改为**静态 crate 依赖**，通过 cargo feature 选择。

```toml
# Cargo.toml
[features]
default = ["plugin-led", "plugin-tfcard", "plugin-uart"]
plugin-led = ["rp2350-plugin-led"]
plugin-tfcard = ["rp2350-plugin-tfcard"]
plugin-uart = ["rp2350-plugin-uart-terminal"]
wasm = ["eframe/wasm_bindgen", "egui-wgpu/webgl"]

---

## Dynamic Library Loading Notes / 动态库加载说明

### Platform-specific paths

| Platform | Extension | Default search |
|----------|-----------|----------------|
| Windows | `.dll` | `plugins/`, `PATH` |
| Linux | `.so` | `plugins/`, `LD_LIBRARY_PATH` |
| macOS | `.dylib` | `plugins/`, `DYLD_LIBRARY_PATH` |

### Required exported symbols

```
rp2350_plugin_create     → fn() -> *mut dyn RpPlugin
rp2350_plugin_version    → fn() -> u32  (ABI version = 1)
```

### FFI Safety

All `libloading` calls are wrapped in `SafeLibrary` (RAII wrapper in `rp2350-ffi`).
所有 `libloading` 调用均封装在 `SafeLibrary`（RAII 包装，位于 `rp2350-ffi`）。

No `#[link]` attribute is used — all dynamic loading is runtime via `libloading`.
不使用 `#[link]` 属性 —— 所有动态加载通过 `libloading` 运行时完成。

`--release + lto = "fat" + codegen-units = 1` for static linking of all internal crates.
发布构建使用 `--release + lto = "fat" + codegen-units = 1` 进行所有内部 crate 的静态链接。