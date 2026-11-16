//! Shared fixtures for plugin loader and hot-reload tests
//! 插件加载器与热重载测试的共享夹具
//!
//! Fixtures are generated at test time with `rustc` so every ABI-mismatch and
//! missing-symbol case is a **real** dynamic library, not a mocked symbol table.
//! 夹具在测试时用 `rustc` 生成，因此每个 ABI 不匹配/缺符号场景都是**真实**的
//! 动态库，而不是伪造的符号表。

use std::path::{Path, PathBuf};
use std::process::Command;

/// Fresh temporary directory unique to one test
/// 单个测试专用的全新临时目录
pub(crate) fn temp_dir(tag: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("rp2350-virt-periph-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("create temp dir");
    path
}

/// Platform plugin library extension used to name fake libraries
/// 用于命名伪造库文件的平台插件扩展名
pub(crate) fn platform_ext() -> &'static str {
    crate::loader::PluginLoader::platform_extensions()[0]
}

/// Locate the built reference LED plugin library in the workspace target dir
/// 在工作区 target 目录中定位已构建的参考 LED 插件库
pub(crate) fn locate_led_plugin() -> Option<PathBuf> {
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let name = if cfg!(target_os = "windows") {
        "rp2350_led_plugin.dll"
    } else if cfg!(target_os = "macos") {
        "librp2350_led_plugin.dylib"
    } else {
        "librp2350_led_plugin.so"
    };
    ["debug", "release", "debug/deps"]
        .into_iter()
        .map(|profile| target.join(profile).join(name))
        .find(|path| path.is_file())
}

/// Copy the reference LED plugin into `dir` and return the copy's path
/// 将参考 LED 插件复制到 `dir` 并返回副本路径
pub(crate) fn copy_led_plugin(dir: &Path) -> Option<PathBuf> {
    let source = locate_led_plugin()?;
    let destination = dir.join(source.file_name()?);
    std::fs::copy(&source, &destination).expect("copy LED plugin");
    Some(destination)
}

/// Compile a `cdylib` fixture exporting exactly `body`
/// 编译一个只导出 `body` 中符号的 `cdylib` 夹具
///
/// Returns `None` when `rustc` is unavailable or compilation fails; the caller
/// reports a skip instead of failing on a toolchain-less host. 当 `rustc` 不可用
/// 或编译失败时返回 `None`；调用方报告跳过，而不是在无工具链的宿主机上失败。
pub(crate) fn build_fixture(tag: &str, body: &str) -> Option<PathBuf> {
    let dir = temp_dir(&format!("fixture-{tag}"));
    let source = dir.join(format!("{tag}.rs"));
    let library = dir.join(format!("{tag}.{}", platform_ext()));
    let text = format!("#![crate_type = \"cdylib\"]\n{body}\n");
    std::fs::write(&source, text).expect("write fixture source");
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let output = Command::new(rustc)
        .args(["--edition", "2024", "--crate-type", "cdylib", "-o"])
        .arg(&library)
        .arg(&source)
        .output()
        .ok()?;
    if !output.status.success() {
        eprintln!(
            "SKIP: fixture '{tag}' failed to compile / 夹具编译失败: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return None;
    }
    library.is_file().then_some(library)
}

/// Assert that every wiring entry points at a live device port of the same name
/// 断言每个接线条目都指向真实存在且同名的设备端口
pub(crate) fn assert_wiring_consistent(manager: &crate::DeviceManager) {
    for entry in manager.wiring().all() {
        let Some(ports) = manager.ports_of(&entry.device_id) else {
            panic!(
                "wiring references missing device {} / 接线引用了不存在的设备",
                entry.device_id
            );
        };
        let Some(port) = ports.get(entry.port_index) else {
            panic!(
                "device {} has no port {} / 设备 {} 没有端口 {}",
                entry.device_id, entry.port_index, entry.device_id, entry.port_index
            );
        };
        assert_eq!(
            port.name, entry.port_name,
            "wiring port name mismatch / 接线端口名不匹配"
        );
    }
}

mod executor;
mod executor_faults;
mod fixtures;
mod loader;
mod reload;
