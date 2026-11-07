# Plugin ABI Specification / 插件 ABI 规范

> **Version**: 1.0 | **Date**: 2026-11-08

---

## Overview / 概述

Plugins are compiled as shared libraries (`.dll` / `.so` / `.dylib`) and loaded at runtime via `libloading`.
插件以共享库形式编译（`.dll` / `.so` / `.dylib`），运行时通过 `libloading` 加载。

---

## Required Exports / 必需导出符号

### `rp2350_plugin_create`

```c
/// Create plugin instance. Returns heap-allocated Box<dyn RpPlugin>.
/// 创建插件实例。返回堆分配的 Box<dyn RpPlugin>。
///
/// The emulator takes ownership and calls `shutdown()` + `drop()` on unload.
/// 模拟器取得所有权，在卸载时调用 `shutdown()` + `drop()`。
void* rp2350_plugin_create(void);
```

### `rp2350_plugin_version`

```c
/// Returns ABI version number. Current: 1.
/// 返回 ABI 版本号。当前版本: 1。
///
/// Emulator refuses to load plugins with incompatible versions.
/// 模拟器拒绝加载不兼容版本的插件。
uint32_t rp2350_plugin_version(void);
```

---

## Plugin Metadata Format / 插件元数据格式

```json
{
    "name": "tf_card_plugin",
    "version": "1.0.0",
    "author": "Your Name",
    "description": "Virtual TF/SD card in SPI+SDIO mode",
    "device_type": "tf_card",
    "icon": "<base64 encoded 64x64 PNG>"
}
```

---

## Version Compatibility / 版本兼容性

| Plugin ABI | Emulator Min | Emulator Max |
|------------|-------------|-------------|
| 1 | 0.1.0 | 0.x.x |

SemVer: major version changes indicate breaking ABI changes.
主版本号变更表示 ABI 不兼容。

---

## Plugin Loading / 插件加载

```
Platform    Extension    Directory Search
Windows     .dll         plugins/, PATH
Linux       .so          plugins/, LD_LIBRARY_PATH
macOS       .dylib       plugins/, DYLD_LIBRARY_PATH
```

---

## Signed Plugin Verification (Future) / 插件签名验证 (未来)

TBD — optional Ed25519 signature verification for production plugins.
待定 —— 可选的 Ed25519 签名验证，用于生产环境插件。