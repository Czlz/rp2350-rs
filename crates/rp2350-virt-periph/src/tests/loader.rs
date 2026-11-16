//! Loader error-path tests — corrupt, incomplete and incompatible libraries
//! 加载器错误路径测试 — 损坏、不完整与不兼容的插件库
//!
//! Every case asserts three properties: the call returns `Err` (never panics),
//! the message names the offending path, and the message is bilingual.
//! 每个用例断言三点：返回 `Err`（绝不 panic）、错误信息包含出错路径、信息为双语。

use std::path::Path;

use crate::DeviceManager;
use crate::error::DeviceError;
use crate::loader::PluginLoader;

use super::{build_fixture, copy_led_plugin, locate_led_plugin, platform_ext, temp_dir};

/// Load `path` expecting failure and return the rendered message
/// 加载 `path` 并期望失败，返回渲染后的错误信息
///
/// Also asserts the two cross-cutting guarantees (path + bilingual text).
/// 同时断言两条通用保证（包含路径 + 双语文本）。
fn load_error_message(path: &Path) -> String {
    let error = PluginLoader::new()
        .load_device(path)
        .err()
        .expect("load must fail / 加载必须失败");
    let message = error.to_string();
    assert!(
        message.contains(&path.display().to_string()),
        "error must name the path: {message}"
    );
    assert!(
        message.contains("插件"),
        "error must be bilingual: {message}"
    );
    message
}

/// Write `bytes` to a fresh temp file with the platform library extension
/// 将 `bytes` 写入带平台库扩展名的全新临时文件
fn temp_library(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.join(format!("{name}.{}", platform_ext()));
    std::fs::write(&path, bytes).expect("write temp library");
    path
}

/// Minimal ELF64 header for a relocatable object (`ET_REL`, not a shared object)
/// 可重定位目标文件（`ET_REL`，非共享库）的最小 ELF64 头
fn elf_relocatable_header() -> Vec<u8> {
    let mut bytes = vec![0_u8; 64];
    bytes[..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 2; // ELFCLASS64 / 64 位
    bytes[5] = 1; // ELFDATA2LSB / 小端
    bytes[6] = 1; // EV_CURRENT
    bytes[16] = 1; // ET_REL — not loadable by dlopen / dlopen 不可加载
    bytes[18] = 0x3e; // EM_X86_64
    bytes[20] = 1; // e_version
    bytes[52] = 64; // e_ehsize
    bytes
}

#[test]
fn load_device_rejects_missing_path() {
    let dir = temp_dir("err-missing");
    let path = dir.join(format!("absent.{}", platform_ext()));
    let message = load_error_message(&path);
    assert!(message.contains("无法访问插件文件"), "{message}");
    assert_eq!(DeviceManager::new().device_count(), 0);
}

#[test]
fn load_device_rejects_directory_path() {
    let dir = temp_dir("err-directory");
    let path = dir.join(format!("a-directory.{}", platform_ext()));
    std::fs::create_dir(&path).expect("create directory");
    let message = load_error_message(&path);
    assert!(message.contains("不是普通文件"), "{message}");
}

#[test]
fn load_device_rejects_empty_file() {
    let dir = temp_dir("err-empty");
    let path = temp_library(&dir, "empty", b"");
    let message = load_error_message(&path);
    assert!(message.contains("空库文件"), "{message}");
}

#[test]
fn load_device_rejects_random_bytes() {
    let dir = temp_dir("err-random");
    let path = temp_library(&dir, "random", &[0xde, 0xad, 0xbe, 0xef, 0x00, 0x01, 0x02]);
    let message = load_error_message(&path);
    assert!(message.contains("打开动态库失败"), "{message}");
}

#[test]
fn load_device_rejects_truncated_elf() {
    let dir = temp_dir("err-truncated-elf");
    let path = temp_library(
        &dir,
        "truncated",
        &[
            0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0,
        ],
    );
    let message = load_error_message(&path);
    assert!(message.contains("打开动态库失败"), "{message}");
}

#[test]
fn load_device_rejects_non_cdylib_elf_object() {
    let dir = temp_dir("err-elf-rel");
    let path = temp_library(&dir, "object", &elf_relocatable_header());
    let message = load_error_message(&path);
    assert!(message.contains("打开动态库失败"), "{message}");
}

#[test]
fn load_device_rejects_library_without_version_symbol() {
    let Some(path) = build_fixture(
        "no-version",
        r#"
#[unsafe(no_mangle)]
pub extern "C" fn unrelated_export() -> u32 { 7 }
"#,
    ) else {
        eprintln!("SKIP: rustc fixture unavailable / 无法生成 rustc 夹具");
        return;
    };
    let message = load_error_message(&path);
    assert!(message.contains("rp2350_plugin_version"), "{message}");
    assert!(message.contains("缺少 ABI 符号"), "{message}");
}

#[test]
fn load_device_rejects_library_without_is_device_symbol() {
    let Some(path) = build_fixture(
        "no-is-device",
        r#"
#[unsafe(no_mangle)]
pub extern "C" fn rp2350_plugin_version() -> u32 { 1 }
"#,
    ) else {
        eprintln!("SKIP: rustc fixture unavailable / 无法生成 rustc 夹具");
        return;
    };
    let message = load_error_message(&path);
    assert!(message.contains("rp2350_plugin_is_device"), "{message}");
    assert!(message.contains("缺少 ABI 符号"), "{message}");
}

#[test]
fn load_device_rejects_library_without_create_symbol() {
    let Some(path) = build_fixture(
        "no-create",
        r#"
#[unsafe(no_mangle)]
pub extern "C" fn rp2350_plugin_version() -> u32 { 1 }
#[unsafe(no_mangle)]
pub extern "C" fn rp2350_plugin_is_device() -> u32 { 1 }
"#,
    ) else {
        eprintln!("SKIP: rustc fixture unavailable / 无法生成 rustc 夹具");
        return;
    };
    let message = load_error_message(&path);
    assert!(message.contains("rp2350_device_plugin_create"), "{message}");
    assert!(message.contains("缺少 ABI 符号"), "{message}");
}

#[test]
fn load_device_rejects_abi_mismatch_with_full_message() {
    let expected_abi = rp2350_plugin::abi::ABI_VERSION;
    let reported_abi = expected_abi + 1;
    let body = format!(
        "#[unsafe(no_mangle)]\npub extern \"C\" fn rp2350_plugin_version() -> u32 {{ {reported_abi} }}"
    );
    let Some(path) = build_fixture("abi-mismatch", &body) else {
        eprintln!("SKIP: rustc fixture unavailable / 无法生成 rustc 夹具");
        return;
    };
    let message = load_error_message(&path);
    // 完整错误消息：路径 + 英文 + 中文 + 期望/实际版本号。
    // Full message: path + English + Chinese + expected/actual versions.
    assert!(message.contains("incompatible plugin ABI"), "{message}");
    assert!(message.contains("插件 ABI 不兼容"), "{message}");
    assert!(
        message.contains(&format!("expected {expected_abi}, got {reported_abi}")),
        "{message}"
    );
    assert!(
        message.contains(&format!("期望 {expected_abi}，实际 {reported_abi}")),
        "{message}"
    );
}

#[test]
fn load_device_rejects_null_factory() {
    let Some(path) = build_fixture(
        "null-factory",
        r#"
// 工厂返回 fat-pointer 的空值：与 `DeviceCreateFn` 的 `*mut dyn Trait` 返回
// 约定一致，避免调用约定不匹配导致的未定义行为。
// The factory returns a null fat pointer so the calling convention matches
// `DeviceCreateFn` (`*mut dyn Trait`) instead of a thin `*mut c_void`.
#[unsafe(no_mangle)]
pub extern "C" fn rp2350_plugin_version() -> u32 { 1 }
#[unsafe(no_mangle)]
pub extern "C" fn rp2350_plugin_is_device() -> u32 { 1 }
#[unsafe(no_mangle)]
pub extern "C" fn rp2350_device_plugin_create() -> *mut dyn core::any::Any {
    core::ptr::null_mut::<()>() as *mut dyn core::any::Any
}
"#,
    ) else {
        eprintln!("SKIP: rustc fixture unavailable / 无法生成 rustc 夹具");
        return;
    };
    let message = load_error_message(&path);
    assert!(message.contains("null pointer"), "{message}");
    assert!(message.contains("空指针"), "{message}");
}

#[test]
fn load_device_rejects_non_device_library() {
    let Some(path) = build_fixture(
        "not-a-device",
        r#"
#[unsafe(no_mangle)]
pub extern "C" fn rp2350_plugin_version() -> u32 { 1 }
#[unsafe(no_mangle)]
pub extern "C" fn rp2350_plugin_is_device() -> u32 { 0 }
"#,
    ) else {
        eprintln!("SKIP: rustc fixture unavailable / 无法生成 rustc 夹具");
        return;
    };
    let message = load_error_message(&path);
    assert!(message.contains("not a device plugin"), "{message}");
    assert!(message.contains("插件不是设备插件"), "{message}");
}

#[test]
fn failed_load_does_not_poison_registry_id() {
    let dir = temp_dir("err-reuse-id");
    let Some(plugin) = copy_led_plugin(&dir) else {
        eprintln!("SKIP: LED plugin not built / 未构建 LED 插件，跳过");
        return;
    };
    let mut dm = DeviceManager::new();
    let Err(error) = dm.load_plugin("led".into(), dir.join("missing.dll")) else {
        panic!("missing library must fail / 缺失的库必须加载失败");
    };
    assert!(
        matches!(error, DeviceError::PluginLoadFailed(_)),
        "{error:?}"
    );
    assert_eq!(dm.device_count(), 0);

    // 加载失败后同一 ID 仍可成功注册。
    // The same ID stays usable after a failed load.
    dm.load_plugin("led".into(), &plugin).expect("load LED");
    assert_eq!(dm.device_count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn same_path_loads_as_independent_instances() {
    let dir = temp_dir("same-path");
    let Some(plugin) = copy_led_plugin(&dir) else {
        eprintln!("SKIP: LED plugin not built / 未构建 LED 插件，跳过");
        return;
    };
    let mut dm = DeviceManager::new();
    dm.load_plugin("led-a".into(), &plugin).expect("load a");
    dm.load_plugin("led-b".into(), &plugin).expect("load b");
    assert_eq!(dm.device_count(), 2);
    dm.wire("led-a", 0, 25).expect("wire a");
    dm.wire("led-b", 0, 26).expect("wire b");
    assert_eq!(dm.wiring().find_by_device_port("led-a", 0), Some(25));
    assert_eq!(dm.wiring().find_by_device_port("led-b", 0), Some(26));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn duplicate_id_is_rejected_before_loading() {
    let dir = temp_dir("duplicate-id");
    let Some(plugin) = copy_led_plugin(&dir) else {
        eprintln!("SKIP: LED plugin not built / 未构建 LED 插件，跳过");
        return;
    };
    let mut dm = DeviceManager::new();
    dm.load_plugin("led".into(), &plugin).expect("first load");
    let error = dm
        .load_plugin("led".into(), &plugin)
        .expect_err("duplicate id must fail / 重复 ID 必须失败");
    assert!(
        matches!(error, DeviceError::AlreadyExists(ref id) if id == "led"),
        "{error:?}"
    );
    assert_eq!(dm.device_count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unregister_then_load_same_path_succeeds() {
    let dir = temp_dir("unload-reload");
    let Some(plugin) = copy_led_plugin(&dir) else {
        eprintln!("SKIP: LED plugin not built / 未构建 LED 插件，跳过");
        return;
    };
    let mut dm = DeviceManager::new();
    dm.load_plugin("led".into(), &plugin).expect("load");
    dm.unregister(&"led".to_string()).expect("unregister");
    assert_eq!(dm.device_count(), 0);
    dm.load_plugin("led".into(), &plugin).expect("load again");
    assert_eq!(dm.device_count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scan_and_load_skips_directory_named_like_library() {
    let dir = temp_dir("scan-directory-entry");
    std::fs::create_dir(dir.join(format!("fake.{}", platform_ext())))
        .expect("create directory with library extension");
    let mut dm = DeviceManager::new();
    assert_eq!(
        dm.scan_and_load(&dir),
        Vec::<(String, std::path::PathBuf)>::new()
    );
    assert_eq!(dm.device_count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scan_and_load_two_copies_with_same_id_registers_one() {
    let Some(source) = locate_led_plugin() else {
        eprintln!("SKIP: LED plugin not built / 未构建 LED 插件，跳过");
        return;
    };
    let dir = temp_dir("scan-same-id");
    let ext = platform_ext();
    std::fs::copy(&source, dir.join(format!("a.{ext}"))).expect("copy a");
    std::fs::copy(&source, dir.join(format!("b.{ext}"))).expect("copy b");
    let mut dm = DeviceManager::new();
    let loaded = dm.scan_and_load(&dir);
    assert_eq!(loaded.len(), 1, "{loaded:?}");
    assert_eq!(loaded[0].0, "com.rp2350-rs.led");
    assert_eq!(dm.device_count(), 1);

    // 重复扫描必须幂等：不重复注册、不改变已有设备。
    // A repeated scan is idempotent: nothing new, existing device untouched.
    assert_eq!(
        dm.scan_and_load(&dir),
        Vec::<(String, std::path::PathBuf)>::new()
    );
    assert_eq!(dm.device_count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
