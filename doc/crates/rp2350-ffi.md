# rp2350-ffi — Plugin FFI Safety / 插件 FFI 安全层

> **Crate**: `rp2350-ffi` | **Path**: `crates/rp2350-ffi` | **Status**: ✅ Implemented (Phase 1)

---

## Overview / 概述

RAII wrapper around `libloading::Library` for safe dynamic plugin loading. Implements `Send + Sync` for thread-safe use in the emulator. No `#[link]` attributes — all dynamic library loading is runtime via `dlopen`/`LoadLibrary`.

基于 `libloading::Library` 的 RAII 安全封装，用于动态插件加载。实现 `Send + Sync` 以支持模拟器中的线程安全使用。不使用 `#[link]` 属性 —— 所有动态库加载通过 `dlopen`/`LoadLibrary` 运行时完成。

---

## Key Types / 关键类型

### `SafeLibrary` — RAII 动态库句柄

```rust
/// RAII wrapper around libloading::Library.
/// Automatically closes the library on drop.
/// Safe to share across threads.
pub struct SafeLibrary {
    inner: Library,
    _path: PathBuf,
}
```

### `FfiError` — FFI 错误类型

```rust
#[derive(Debug, thiserror::Error)]
pub enum FfiError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Library loading error: {0}")]
    Library(#[from] libloading::Error),
    #[error("Symbol not found: {0}")]
    SymbolNotFound(String),
    #[error("ABI version mismatch: expected {expected}, got {got}")]
    VersionMismatch { expected: u32, got: u32 },
}
```

---

## Usage / 使用示例

```rust
use rp2350_ffi::SafeLibrary;

// Load a plugin library
let lib = SafeLibrary::new("plugins/led.dll")?;

// Look up exported symbols (unsafe: requires correct type)
// SAFETY: rp2350_plugin_create is a well-known symbol with guaranteed signature
let create_fn: Symbol<fn() -> *mut dyn RpPlugin> =
    unsafe { lib.get_symbol(b"rp2350_plugin_create")? };
let plugin = unsafe { Box::from_raw(create_fn()) };

// Library automatically closed when `lib` drops (RAII)
```

---

## ABI Contract / ABI 契约

### Required Exported Symbols / 必需导出符号

| Symbol | Signature | Purpose |
|--------|-----------|---------|
| `rp2350_plugin_create` | `fn() -> *mut dyn RpPlugin` | Create plugin instance |
| `rp2350_plugin_version` | `fn() -> u32` | ABI version check |

### ABI Version / ABI 版本

Current ABI version: `1`. Version mismatch triggers `FfiError::VersionMismatch`.

当前 ABI 版本: `1`。版本不匹配触发 `FfiError::VersionMismatch`。

---

## Platform Support / 平台支持

| Platform | Extension | Library Type |
|----------|-----------|--------------|
| Windows | `.dll` | PE/COFF |
| Linux | `.so` | ELF |
| macOS | `.dylib` | Mach-O |

---

## WASM Note / WASM 注意事项

`libloading` is **not available** in WASM builds. Plugins are compiled as static crate dependencies via cargo features. The `SafeLibrary` type is conditionally compiled out for `target_arch = "wasm32"`.

`libloading` 在 WASM 构建中 **不可用**。插件通过 cargo feature 作为静态 crate 依赖编译。`SafeLibrary` 类型在 `target_arch = "wasm32"` 时条件编译排除。

```rust
// Cargo.toml feature
[features]
native = ["dep:libloading"]
wasm = ["static-plugins"]
```