//! Hot-reload semantics tests — classification, wiring preservation, consistency
//! 热重载语义测试 — 分类、接线保留与一致性
//!
//! The reference LED plugin is used wherever a real `cdylib` is required;
//! in-process stubs model devices whose library file is replaced or deleted.
//! 需要真实 `cdylib` 时使用参考 LED 插件；库文件被替换/删除的设备用进程内桩模拟。

use std::path::PathBuf;
use std::time::SystemTime;

use rp2350_core::gpio::{GpioPin, PinDirection};
use rp2350_plugin::RpPlugin;
use rp2350_plugin::error::PluginError;
use rp2350_plugin::metadata::PluginMetadata;

use crate::DeviceManager;
use crate::device::{DevicePlugin, DevicePort};
use crate::reload::{Fingerprint, ReloadReport, scan_with_fingerprints};

use super::{assert_wiring_consistent, copy_led_plugin, locate_led_plugin, platform_ext, temp_dir};

/// Minimal in-process device proving reload leaves it untouched
/// 最小进程内设备，用于验证热重载不会影响它
struct StubLed {
    metadata: PluginMetadata,
    ports: Vec<DevicePort>,
    level: bool,
}

impl RpPlugin for StubLed {
    fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }
    fn init(&mut self, _ctx: &mut dyn rp2350_plugin::PluginContext) -> Result<(), PluginError> {
        Ok(())
    }
    fn tick(&mut self, _ctx: &mut dyn rp2350_plugin::PluginContext, _cycles: u64) {}
    fn shutdown(&mut self) {}
    fn state_snapshot(&self) -> serde_json::Value {
        serde_json::json!({ "level": self.level })
    }
}

impl DevicePlugin for StubLed {
    fn ports(&self) -> &[DevicePort] {
        &self.ports
    }
    fn ports_mut(&mut self) -> &mut [DevicePort] {
        &mut self.ports
    }
    fn on_gpio_change(&mut self, _port_index: usize, level: bool) {
        self.level = level;
    }
}

/// Build a stub device with the given ID and port table
/// 构造具有指定 ID 与端口表的桩设备
fn stub_with(id: &str, ports: Vec<DevicePort>) -> StubLed {
    StubLed {
        metadata: PluginMetadata::builder()
            .id(id)
            .name("Reload Stub")
            .version("0.1.0")
            .description("stub")
            .author("test")
            .abi_version(1)
            .min_host_version("0.1.0")
            .has_ui(false)
            .tags(vec!["test".into()])
            .build(),
        ports,
        level: false,
    }
}

fn stub_led() -> StubLed {
    stub_with("reload.stub", vec![DevicePort::input("LED", "LED input")])
}

/// A fingerprint that can never match a real file (forces a reload)
/// 永远无法与真实文件匹配的指纹（强制触发重载）
fn stale_fingerprint() -> Fingerprint {
    Fingerprint {
        len: 0,
        modified: None,
    }
}

#[test]
fn reload_empty_directory_reports_nothing() {
    let dir = temp_dir("reload-empty");
    let mut dm = DeviceManager::new();
    let report = dm.reload(&dir);
    assert!(report.is_empty(), "{report:?}");
    assert_eq!(report.change_count(), 0);
    assert_eq!(dm.device_count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reload_missing_directory_reports_nothing() {
    let dir = temp_dir("reload-missing");
    let mut dm = DeviceManager::new();
    let report = dm.reload(&dir.join("absent"));
    assert!(report.is_empty(), "{report:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reload_ignores_non_library_files() {
    let dir = temp_dir("reload-nonlib");
    std::fs::write(dir.join("notes.txt"), b"hello").expect("write notes");
    std::fs::write(dir.join("readme.md"), b"# doc").expect("write readme");
    let mut dm = DeviceManager::new();
    let report = dm.reload(&dir);
    assert!(report.is_empty(), "{report:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reload_records_corrupt_library_as_failed() {
    let dir = temp_dir("reload-corrupt");
    let bad = dir.join(format!("broken.{}", platform_ext()));
    std::fs::write(&bad, b"this is not a dynamic library").expect("write fake library");
    let mut dm = DeviceManager::new();
    let report = dm.reload(&dir);
    assert_eq!(report.added, Vec::<String>::new(), "{report:?}");
    assert_eq!(report.failed.len(), 1, "{report:?}");
    assert!(report.failed[0].0.contains("broken."), "{report:?}");
    assert!(!report.failed[0].1.is_empty(), "{report:?}");
    assert_eq!(dm.device_count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reload_failure_keeps_other_devices_intact() {
    let dir = temp_dir("reload-failure");
    std::fs::write(dir.join(format!("broken.{}", platform_ext())), b"garbage")
        .expect("write fake library");
    let mut dm = DeviceManager::new();
    dm.register("reload.stub".into(), Box::new(stub_led()))
        .expect("register stub");
    let report = dm.reload(&dir);
    assert_eq!(report.failed.len(), 1, "{report:?}");
    assert_eq!(dm.device_count(), 1);
    let devices = dm.mcp_list_devices();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0]["id"], "reload.stub");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reload_retries_failed_library_after_fingerprint_change() {
    let dir = temp_dir("reload-retry");
    let bad = dir.join(format!("broken.{}", platform_ext()));
    std::fs::write(&bad, b"garbage").expect("write fake library");
    let mut dm = DeviceManager::new();
    assert_eq!(dm.reload(&dir).failed.len(), 1);
    // 文件内容变化后重载仍失败，但必须再次尝试并记录，而不是静默跳过。
    // After the content changes the reload still fails, but it must retry.
    std::fs::write(&bad, b"garbage with different length").expect("rewrite fake library");
    let report = dm.reload(&dir);
    assert_eq!(report.failed.len(), 1, "{report:?}");
    assert_eq!(report.reloaded, Vec::<String>::new(), "{report:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fingerprint_detects_length_change() {
    let dir = temp_dir("fp-len");
    let file = dir.join("lib.bin");
    std::fs::write(&file, b"1234").expect("write file");
    let first = Fingerprint::read(&file).expect("first fingerprint");
    std::fs::write(&file, b"12345678").expect("rewrite file");
    let second = Fingerprint::read(&file).expect("second fingerprint");
    assert!(second.differs_from(&first));
    assert!(!second.differs_from(&second));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fingerprint_without_mtime_counts_as_changed() {
    let unknown = Fingerprint {
        len: 42,
        modified: None,
    };
    assert!(unknown.differs_from(&unknown));
    let known = Fingerprint {
        len: 42,
        modified: Some(SystemTime::now()),
    };
    assert!(known.differs_from(&unknown));
}

#[test]
fn scan_with_fingerprints_reports_sorted_libraries() {
    let dir = temp_dir("scan-fp");
    let ext = platform_ext();
    let first = dir.join(format!("a.{ext}"));
    let second = dir.join(format!("b.{ext}"));
    std::fs::write(&first, b"aa").expect("write a");
    std::fs::write(&second, b"bb").expect("write b");
    std::fs::write(dir.join("ignored.txt"), b"txt").expect("write txt");
    let found = scan_with_fingerprints(&dir);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].0, first);
    assert_eq!(found[1].0, second);
    assert_eq!(found[0].1.len, 2);
    assert_eq!(
        scan_with_fingerprints(dir.join("absent")),
        Vec::<(PathBuf, Fingerprint)>::new()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reload_report_renders_entries_and_summary() {
    let report = ReloadReport {
        added: vec!["a.dll".into()],
        removed: vec!["b.dll".into()],
        reloaded: vec!["c.dll".into()],
        rewired: vec!["e.dll".into()],
        unwired: vec![("f.dll".into(), "port changed".into())],
        failed: vec![("d.dll".into(), "boom".into())],
    };
    let lines = report.render_lines();
    assert_eq!(lines[0], "[reload] + a.dll");
    assert_eq!(lines[1], "[reload] - b.dll");
    assert_eq!(lines[2], "[reload] ~ c.dll");
    assert_eq!(lines[3], "[reload] = e.dll");
    assert_eq!(lines[4], "[reload] x f.dll: port changed");
    assert_eq!(lines[5], "[reload] ! d.dll: boom");
    assert_eq!(
        lines[6],
        "[reload] summary: added=1 removed=1 reloaded=1 rewired=1 unwired=1 failed=1"
    );
    assert_eq!(report.change_count(), 6);
    assert!(!report.is_empty());
}

#[test]
fn reload_reloads_changed_library_in_place() {
    let Some(source) = locate_led_plugin() else {
        eprintln!("SKIP: LED plugin not built / 未构建 LED 插件，跳过");
        return;
    };
    let dir = temp_dir("reload-changed");
    std::fs::copy(&source, dir.join(source.file_name().expect("file name"))).expect("copy plugin");
    let mut dm = DeviceManager::new();
    assert_eq!(dm.scan_and_load(&dir).len(), 1);

    // 注入"已过期"的指纹来模拟文件变化：Windows 上无法写已加载的 DLL。
    // Inject a stale fingerprint to simulate a change: loaded DLLs are locked
    // on Windows, so the file itself cannot be rewritten by the test.
    assert!(dm.set_fingerprint_for_test("com.rp2350-rs.led", stale_fingerprint()));

    let report = dm.reload(&dir);
    assert_eq!(report.reloaded.len(), 1, "{report:?}");
    assert_eq!(report.failed, Vec::<(String, String)>::new(), "{report:?}");
    assert_eq!(dm.device_count(), 1);
    assert_eq!(dm.mcp_list_devices()[0]["id"], "com.rp2350-rs.led");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reload_adds_library_copied_after_scan() {
    let Some(source) = locate_led_plugin() else {
        eprintln!("SKIP: LED plugin not built / 未构建 LED 插件，跳过");
        return;
    };
    let dir = temp_dir("reload-added");
    let mut dm = DeviceManager::new();
    assert_eq!(dm.scan_and_load(&dir), Vec::<(String, PathBuf)>::new());
    std::fs::copy(&source, dir.join(source.file_name().expect("file name"))).expect("copy plugin");
    let report = dm.reload(&dir);
    assert_eq!(report.added.len(), 1, "{report:?}");
    assert_eq!(dm.device_count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reload_removes_deleted_library_from_mcp_list() {
    let Some(source) = locate_led_plugin() else {
        eprintln!("SKIP: LED plugin not built / 未构建 LED 插件，跳过");
        return;
    };
    let dir = temp_dir("reload-removed");
    let library = dir.join(source.file_name().expect("file name"));
    std::fs::copy(&source, &library).expect("copy plugin");
    let mut dm = DeviceManager::new();
    assert_eq!(dm.scan_and_load(&dir).len(), 1);

    // Windows 会锁定已加载的 DLL，删除失败时无法验证 removed 路径。
    // Windows locks loaded DLLs; without a successful delete this cannot run.
    if std::fs::remove_file(&library).is_err() {
        eprintln!("SKIP: loaded library cannot be deleted / 已加载库无法删除，跳过");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }

    let report = dm.reload(&dir);
    assert_eq!(report.removed.len(), 1, "{report:?}");
    assert_eq!(dm.device_count(), 0);
    assert_eq!(dm.mcp_list_devices(), Vec::<serde_json::Value>::new());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reload_deleted_source_reports_removed_without_panic() {
    let dir = temp_dir("reload-vanished");
    let mut dm = DeviceManager::new();
    dm.register(
        "reload.vanish".into(),
        Box::new(stub_with(
            "reload.vanish",
            vec![DevicePort::input("LED", "LED input")],
        )),
    )
    .expect("register stub");
    dm.wire("reload.vanish", 0, 25).expect("wire stub");
    // 记录一个从未存在的源文件：重载时文件集合中缺少该键 → removed。
    // Record a source file that never existed: the key is absent → removed.
    assert!(dm.set_source_for_test(
        "reload.vanish",
        dir.join(format!("gone.{}", platform_ext())),
        Some(Fingerprint {
            len: 1,
            modified: None,
        }),
    ));

    let report = dm.reload(&dir);
    assert_eq!(report.removed.len(), 1, "{report:?}");
    assert_eq!(dm.device_count(), 0);
    assert!(dm.wiring().is_empty(), "{:?}", dm.wiring().all());
    assert_wiring_consistent(&dm);
}

#[test]
fn reload_replacement_keeps_wiring_and_port_access() {
    let dir = temp_dir("reload-rewire");
    let Some(plugin) = copy_led_plugin(&dir) else {
        eprintln!("SKIP: LED plugin not built / 未构建 LED 插件，跳过");
        return;
    };
    let mut dm = DeviceManager::new();
    assert_eq!(dm.scan_and_load(&dir).len(), 1);
    let id = "com.rp2350-rs.led".to_string();
    dm.wire(&id, 0, 25).expect("wire LED");
    assert!(dm.set_fingerprint_for_test(&id, stale_fingerprint()));

    let report = dm.reload(&dir);
    assert_eq!(report.reloaded.len(), 1, "{report:?}");
    assert_eq!(report.rewired.len(), 1, "{report:?}");
    assert_eq!(report.unwired, Vec::<(String, String)>::new(), "{report:?}");
    assert_eq!(dm.wiring().find_by_device_port(&id, 0), Some(25));
    assert_wiring_consistent(&dm);

    // 重载后端口索引仍可访问：GPIO25 的电平驱动新设备实例。
    // Port access still works after reload: GPIO25 drives the new instance.
    let mut pins =
        std::array::from_fn::<GpioPin, 48, _>(|i| GpioPin::new(u8::try_from(i).unwrap()));
    pins[25].set_direction(PinDirection::Output);
    pins[25].set_output(true);
    dm.sync_gpio(&mut pins);
    assert_eq!(
        dm.mcp_query_device(&id).expect("device present")["on"],
        serde_json::json!(true)
    );
    assert!(plugin.is_file(), "library copy must remain on disk");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reload_port_change_drops_wiring_with_reason() {
    let dir = temp_dir("reload-port-change");
    let Some(plugin) = copy_led_plugin(&dir) else {
        eprintln!("SKIP: LED plugin not built / 未构建 LED 插件，跳过");
        return;
    };
    let mut dm = DeviceManager::new();
    // 旧实例声明两个端口（第二端口在替换后的 LED 插件中不存在）。
    // The old instance exposes two ports; the replacement LED has only one.
    dm.register(
        "com.rp2350-rs.led".into(),
        Box::new(stub_with(
            "com.rp2350-rs.led",
            vec![
                DevicePort::input("LED", "LED input"),
                DevicePort::input("EXTRA", "extra input"),
            ],
        )),
    )
    .expect("register stub");
    dm.wire("com.rp2350-rs.led", 1, 26)
        .expect("wire second port");
    assert!(dm.set_source_for_test("com.rp2350-rs.led", plugin, Some(stale_fingerprint())));

    let report = dm.reload(&dir);
    assert_eq!(report.reloaded.len(), 1, "{report:?}");
    assert_eq!(report.rewired, Vec::<String>::new(), "{report:?}");
    assert_eq!(report.unwired.len(), 1, "{report:?}");
    assert!(report.unwired[0].1.contains("EXTRA"), "{report:?}");
    assert!(dm.wiring().is_empty(), "{:?}", dm.wiring().all());
    assert_eq!(dm.device_count(), 1);
    assert_wiring_consistent(&dm);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reload_failed_replacement_removes_device_and_wiring() {
    let dir = temp_dir("reload-failed-replace");
    let bad = dir.join(format!("broken.{}", platform_ext()));
    std::fs::write(&bad, b"corrupted replacement").expect("write fake library");
    let mut dm = DeviceManager::new();
    dm.register(
        "reload.stale".into(),
        Box::new(stub_with(
            "reload.stale",
            vec![DevicePort::input("LED", "LED input")],
        )),
    )
    .expect("register stub");
    dm.wire("reload.stale", 0, 25).expect("wire stub");
    assert!(dm.set_source_for_test("reload.stale", bad, Some(stale_fingerprint())));

    let report = dm.reload(&dir);
    assert_eq!(report.reloaded, Vec::<String>::new(), "{report:?}");
    assert_eq!(report.failed.len(), 1, "{report:?}");
    assert_eq!(dm.device_count(), 0);
    assert!(dm.wiring().is_empty(), "{:?}", dm.wiring().all());
    assert_wiring_consistent(&dm);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reload_unchanged_is_idempotent_and_keeps_wiring() {
    let dir = temp_dir("reload-idempotent");
    if copy_led_plugin(&dir).is_none() {
        eprintln!("SKIP: LED plugin not built / 未构建 LED 插件，跳过");
        return;
    }
    let mut dm = DeviceManager::new();
    assert_eq!(dm.scan_and_load(&dir).len(), 1);
    dm.wire("com.rp2350-rs.led", 0, 25).expect("wire LED");

    for _ in 0..2 {
        let report = dm.reload(&dir);
        assert!(report.is_empty(), "{report:?}");
        assert_eq!(dm.device_count(), 1);
        assert_eq!(
            dm.wiring().find_by_device_port("com.rp2350-rs.led", 0),
            Some(25)
        );
        assert_wiring_consistent(&dm);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reload_keeps_device_and_wiring_tables_consistent() {
    let dir = temp_dir("reload-consistent");
    if copy_led_plugin(&dir).is_none() {
        eprintln!("SKIP: LED plugin not built / 未构建 LED 插件，跳过");
        return;
    }
    std::fs::write(
        dir.join(format!("broken.{}", platform_ext())),
        b"still not a library",
    )
    .expect("write fake library");
    let mut dm = DeviceManager::new();
    assert_eq!(dm.scan_and_load(&dir).len(), 1);
    dm.register("reload.stub".into(), Box::new(stub_led()))
        .expect("register in-process stub");
    dm.wire("com.rp2350-rs.led", 0, 25).expect("wire LED");
    assert!(dm.set_fingerprint_for_test("com.rp2350-rs.led", stale_fingerprint()));

    let report = dm.reload(&dir);
    assert_eq!(report.reloaded.len(), 1, "{report:?}");
    assert_eq!(report.rewired.len(), 1, "{report:?}");
    assert_eq!(report.failed.len(), 1, "{report:?}");

    let mut ids: Vec<String> = dm.devices().map(|(id, _)| id.clone()).collect();
    ids.sort();
    assert_eq!(ids, vec!["com.rp2350-rs.led", "reload.stub"]);
    assert_wiring_consistent(&dm);
    assert_eq!(dm.wiring().all().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
