//! Integration test — load the LED plugin `cdylib` through `DeviceManager`
//! 集成测试 — 通过 `DeviceManager` 加载 LED 插件 `cdylib`

use std::path::{Path, PathBuf};

use rp2350_core::gpio::{GpioPin, PinDirection};
use rp2350_virt_periph::DeviceManager;

/// Platform-specific file name of the plugin library
/// 插件库的平台相关文件名
fn library_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "rp2350_led_plugin.dll"
    }
    #[cfg(target_os = "macos")]
    {
        "librp2350_led_plugin.dylib"
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        "librp2350_led_plugin.so"
    }
}

/// Locate the built plugin library in the workspace target directory
/// 在工作区 target 目录中定位已构建的插件库
fn locate_library() -> Option<PathBuf> {
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let name = library_name();
    [
        target.join("debug").join(name),
        target.join("debug/deps").join(name),
        target.join("release").join(name),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

#[test]
fn device_manager_loads_and_drives_plugin() {
    let Some(path) = locate_library() else {
        eprintln!(
            "SKIP: {} not built yet; run `cargo build -p rp2350-led-plugin` first / \
             尚未构建，请先运行 cargo build -p rp2350-led-plugin",
            library_name()
        );
        return;
    };

    let mut manager = DeviceManager::new();
    manager
        .load_plugin("led0".to_string(), &path)
        .expect("plugin should load through the ABI");
    assert_eq!(manager.device_count(), 1);

    manager.wire("led0", 0, 25).unwrap();
    let mut pins =
        std::array::from_fn::<GpioPin, 48, _>(|i| GpioPin::new(u8::try_from(i).unwrap()));
    pins[25].set_direction(PinDirection::Output);
    pins[25].set_output(true);
    manager.sync_gpio(&mut pins);

    let query = manager.mcp_query_all();
    assert_eq!(query["led0"]["on"], true);
    assert_eq!(query["led0"]["transitions"], 1);

    let id = "led0".to_string();
    manager.unregister(&id).unwrap();
    assert_eq!(manager.device_count(), 0);
}

#[test]
fn missing_library_reports_error() {
    let mut manager = DeviceManager::new();
    let path = Path::new("definitely-not-a-plugin.dll");
    assert!(manager.load_plugin("bad".to_string(), path).is_err());
    assert_eq!(manager.device_count(), 0);
}
