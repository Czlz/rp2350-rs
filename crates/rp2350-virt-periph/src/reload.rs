//! Hot reload support — change detection for directory-scanned plugins
//! 热重载支持 — 目录扫描插件的变更检测
//!
//! Provides the [`Fingerprint`] used to detect library changes, the
//! [`ReloadReport`] returned by [`crate::DeviceManager::reload`], and the
//! directory scanner shared by `scan_and_load` and `reload`.
//! 提供用于检测库变化的 [`Fingerprint`]、[`crate::DeviceManager::reload`] 返回的
//! [`ReloadReport`]，以及 `scan_and_load` 与 `reload` 共用的目录扫描函数。

// ReloadReport / Fingerprint 等类型名来自项目命名约定，文档引用沿用内部名称。
// ReloadReport / Fingerprint and similar type names come from project
// conventions; doc references keep the internal names.
#![allow(clippy::doc_markdown, clippy::module_name_repetitions)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::DeviceManager;
use crate::error::DeviceError;
use crate::loader::{LoadedPlugin, PluginLoader};
use crate::manager::{DeviceId, ManagedDevice, device_id_for};

/// Change report returned by [`crate::DeviceManager::reload`]
/// [`crate::DeviceManager::reload`] 返回的变更报告
///
/// Every field holds library paths rendered with `Path::display`, so the report
/// is directly printable and comparable in tests.
/// 每个字段都以 `Path::display` 形式保存库路径，因此报告可直接打印并在测试中比较。
///
/// # Safety / 安全约束
///
/// Reloading is a **synchronous** operation: the host must guarantee that no
/// device call stack (`tick`, `on_gpio_change`, `state_snapshot`, …) is active
/// while [`crate::DeviceManager::reload`] runs. The manager unloads libraries, so any
/// outstanding reference into a removed plugin would be a dangling vtable.
/// 热重载是**同步**操作：宿主必须保证执行 [`crate::DeviceManager::reload`] 时没有
/// 设备调用栈（`tick`、`on_gpio_change`、`state_snapshot` 等）处于活动状态。
/// 管理器会卸载库，此时任何仍指向已被移除插件的引用都会成为悬空 vtable。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReloadReport {
    /// Newly loaded libraries / 新加载的库
    pub added: Vec<String>,
    /// Libraries whose devices were removed / 已移除设备的库
    pub removed: Vec<String>,
    /// Libraries unloaded and loaded again after a change / 变化后卸载并重新加载的库
    pub reloaded: Vec<String>,
    /// Libraries that failed to load, with the error message / 加载失败的库及错误信息
    pub failed: Vec<(String, String)>,
}

impl ReloadReport {
    /// Does the report contain no changes at all? / 报告是否完全没有变更？
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.reloaded.is_empty()
            && self.failed.is_empty()
    }

    /// Total number of reported entries / 报告条目总数
    #[must_use]
    pub fn change_count(&self) -> usize {
        self.added.len() + self.removed.len() + self.reloaded.len() + self.failed.len()
    }

    /// Render one human-readable line per entry plus a summary line
    /// 每个条目渲染一行可读文本，并附加一行汇总
    #[must_use]
    pub fn render_lines(&self) -> Vec<String> {
        let mut lines = Vec::with_capacity(self.change_count() + 1);
        lines.extend(self.added.iter().map(|path| format!("[reload] + {path}")));
        lines.extend(self.removed.iter().map(|path| format!("[reload] - {path}")));
        lines.extend(
            self.reloaded
                .iter()
                .map(|path| format!("[reload] ~ {path}")),
        );
        lines.extend(
            self.failed
                .iter()
                .map(|(path, error)| format!("[reload] ! {path}: {error}")),
        );
        lines.push(format!(
            "[reload] summary: added={} removed={} reloaded={} failed={}",
            self.added.len(),
            self.removed.len(),
            self.reloaded.len(),
            self.failed.len()
        ));
        lines
    }
}

/// File fingerprint used to detect plugin library changes
/// 用于检测插件库变化的文件指纹
///
/// Length and modification time are compared together; a `None` modification
/// time means "cannot be read" and is treated as **changed**, so a library is
/// reloaded rather than silently ignored.
/// 同时比较长度与修改时间；`None` 表示"无法读取"，按**已变化**处理，
/// 从而优先重载而不是静默忽略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint {
    /// File length in bytes / 文件字节长度
    pub len: u64,
    /// Last modification time, `None` when it could not be read
    /// 最后修改时间；读取失败时为 `None`
    pub modified: Option<SystemTime>,
}

impl Fingerprint {
    /// Read the fingerprint of a file; `None` when metadata is unavailable
    /// 读取文件指纹；元数据不可用时返回 `None`
    #[must_use]
    pub fn read(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }

    /// Has the file changed relative to `previous`?
    /// 相对于 `previous`，文件是否已变化？
    ///
    /// A missing modification time on either side counts as a change.
    /// 任一方的修改时间缺失都视为已变化。
    #[must_use]
    pub fn differs_from(&self, previous: &Self) -> bool {
        self.len != previous.len || self.modified != previous.modified || self.modified.is_none()
    }
}

/// Scan a directory and pair every library with its fingerprint
/// 扫描目录并为每个库附带指纹
///
/// Files are enumerated by [`PluginLoader::scan_dir`] (platform extensions,
/// path-sorted). Files whose metadata cannot be read are skipped.
/// 文件由 [`PluginLoader::scan_dir`] 枚举（按平台扩展名，路径排序）；
/// 元数据不可读的文件会被跳过。
#[must_use]
pub fn scan_with_fingerprints(dir: impl AsRef<Path>) -> Vec<(PathBuf, Fingerprint)> {
    let mut found = Vec::new();
    for path in PluginLoader::scan_dir(dir) {
        match Fingerprint::read(&path) {
            Some(fingerprint) => found.push((path, fingerprint)),
            None => log::debug!(
                "Skipping unreadable plugin candidate {} / 跳过不可读的插件候选文件",
                path.display()
            ),
        }
    }
    found
}

/// Resolve a path to its canonical form, falling back to the input path
/// 将路径解析为规范形式，失败时返回原始路径
#[must_use]
pub fn normalize_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

impl DeviceManager {
    /// Hot-reload every plugin library in `dir`
    /// 对 `dir` 中的插件库执行热重载
    ///
    /// The directory is re-scanned and compared with the libraries recorded by
    /// [`DeviceManager::scan_and_load`] / [`DeviceManager::load_plugin`]:
    /// 重新扫描目录，并与 [`DeviceManager::scan_and_load`] /
    /// [`DeviceManager::load_plugin`] 记录的库比较：
    ///
    /// - **removed** — the file disappeared: the device instance is shut down
    ///   and dropped, then its library is unloaded.
    /// - **reloaded** — same path, different size or modification time: the old
    ///   instance is dropped, the old library unloaded (mandatory on Windows,
    ///   where loaded DLLs are locked), and the library loaded again.
    /// - **added** — a new library: loaded and registered.
    /// - **failed** — a library that could not be loaded; the error is kept in
    ///   [`ReloadReport::failed`] and the rest of the manager keeps working.
    /// - **removed / 已移除** — 文件已消失：先关闭并释放设备实例，再卸载库。
    /// - **reloaded / 已重载** — 路径相同但大小或修改时间变化：先释放旧实例、
    ///   卸载旧库（Windows 上必须如此，已加载 DLL 会被锁定），再重新加载。
    /// - **added / 新增** — 新库：加载并注册。
    /// - **failed / 失败** — 加载失败：错误记入 [`ReloadReport::failed`]，
    ///   管理器其余部分继续可用。
    ///
    /// Unchanged libraries keep their existing instance and registration order.
    /// Devices registered in-process ([`DeviceManager::register`]) and devices
    /// whose source path lies outside `dir` are never touched.
    /// 未变化的库保持原有实例与注册顺序。进程内注册的设备
    /// （[`DeviceManager::register`]）以及源路径位于 `dir` 之外的设备不受影响。
    ///
    /// # Safety / 安全约束
    ///
    /// This is a **synchronous** API that unloads dynamic libraries, so the host
    /// must guarantee no device call stack is active (no concurrent `tick`,
    /// `on_gpio_change` or query into a plugin being replaced). The manager
    /// always drops the device instance before its library, and a failed load
    /// only affects that one library: every other device keeps its instance.
    /// 这是会卸载动态库的**同步** API，宿主必须保证没有设备调用栈处于活动状态
    /// （不会并发调用 `tick`、`on_gpio_change`，也不会查询正在被替换的插件）。
    /// 管理器始终先释放设备实例再卸载其库；单个库加载失败只影响该库，
    /// 其他设备保持原有实例。
    pub fn reload(&mut self, dir: &Path) -> ReloadReport {
        let mut report = ReloadReport::default();
        let dir_key = normalize_path(dir);

        // 以"规范化目录 + 文件名"为键，避免对可能已消失的文件做 canonicalize。
        // Key by "normalized dir + file name" so vanished files need no canonicalize.
        let mut current: BTreeMap<PathBuf, (PathBuf, Fingerprint)> = BTreeMap::new();
        for (path, fingerprint) in scan_with_fingerprints(dir) {
            let file_name = path.file_name().unwrap_or_default().to_os_string();
            current.insert(dir_key.join(file_name), (path, fingerprint));
        }

        let mut previous: BTreeMap<PathBuf, (DeviceId, Option<Fingerprint>)> = BTreeMap::new();
        for (id, managed) in &self.devices {
            let Some(source) = managed.source_path.as_deref() else {
                continue;
            };
            let Some(parent) = source.parent() else {
                continue;
            };
            if normalize_path(parent) != dir_key {
                continue;
            }
            let file_name = source.file_name().unwrap_or_default().to_os_string();
            previous.insert(dir_key.join(file_name), (id.clone(), managed.fingerprint));
        }

        // 1) 先卸载消失的库，确保 Windows 上的文件锁被释放。
        // First unload vanished libraries so Windows file locks are released.
        for (key, (id, _fingerprint)) in &previous {
            if !current.contains_key(key) {
                self.drop_device_instance(id);
                report.removed.push(render_path(key));
            }
        }

        // 2) 已变化的库：释放旧实例 → 卸载旧库 → 重新加载。
        // Changed libraries: drop the old instance, unload, then load again.
        for (key, (path, fingerprint)) in &current {
            let Some((old_id, old_fingerprint)) = previous.get(key) else {
                continue;
            };
            let changed = match old_fingerprint {
                Some(old) => fingerprint.differs_from(old),
                None => true,
            };
            if !changed {
                continue;
            }
            self.drop_device_instance(old_id);
            match self.insert_loaded(path, Some(*fingerprint)) {
                Ok(_id) => report.reloaded.push(render_path(key)),
                Err(error) => report.failed.push((render_path(key), error.to_string())),
            }
        }

        // 3) 新增的库：直接加载，失败只记录，不中断其余插件。
        // New libraries: load them; failures are recorded without aborting others.
        for (key, (path, fingerprint)) in &current {
            if previous.contains_key(key) {
                continue;
            }
            match self.insert_loaded(path, Some(*fingerprint)) {
                Ok(_id) => report.added.push(render_path(key)),
                Err(error) => report.failed.push((render_path(key), error.to_string())),
            }
        }

        if !report.is_empty() {
            log::info!(
                "Device plugin reload in {}: {} / 设备插件热重载完成",
                dir.display(),
                report.render_lines().join("; ")
            );
        }
        report
    }

    /// Load one library and register it under its reported device ID
    /// 加载单个库并按插件报告的设备 ID 注册
    pub(crate) fn insert_loaded(
        &mut self,
        path: &Path,
        fingerprint: Option<Fingerprint>,
    ) -> Result<DeviceId, DeviceError> {
        let LoadedPlugin { library, plugin } = self.loader.load_device(path)?;
        let id = device_id_for(plugin.metadata().id.as_str(), path);
        if self.devices.contains_key(&id) {
            // 显式按"实例先、库后"的顺序释放，避免卸载仍在使用的 vtable。
            // Drop the instance before the library so no vtable is unmapped in use.
            drop(plugin);
            drop(library);
            return Err(DeviceError::AlreadyExists(id));
        }
        self.devices.insert(
            id.clone(),
            ManagedDevice {
                plugin,
                source_path: Some(path.to_path_buf()),
                library: Some(library),
                fingerprint,
            },
        );
        Ok(id)
    }

    /// Shut down and drop one device instance, then release its library
    /// 关闭并释放单个设备实例，随后释放其库
    ///
    /// Returns `false` when the ID is unknown.
    /// ID 不存在时返回 `false`。
    pub(crate) fn drop_device_instance(&mut self, id: &DeviceId) -> bool {
        let Some(mut managed) = self.devices.remove(id) else {
            return false;
        };
        managed.plugin.shutdown();
        self.wiring.remove_device(id);
        // ManagedDevice 字段按声明顺序析构：plugin 先于 library 释放。
        // ManagedDevice fields drop in declaration order: plugin before library.
        drop(managed);
        true
    }

    /// Test-only: overwrite the recorded fingerprint of one device
    /// 仅测试用：覆盖单个设备已记录的指纹
    ///
    /// Simulates "the library file changed on disk" without writing to a loaded
    /// DLL (impossible on Windows) and returns `false` for unknown IDs.
    /// 在不写入已加载 DLL（Windows 上不可行）的前提下模拟“磁盘上的库已变化”；
    /// ID 不存在时返回 `false`。
    #[cfg(test)]
    pub(crate) fn set_fingerprint_for_test(&mut self, id: &str, fingerprint: Fingerprint) -> bool {
        match self.devices.get_mut(id) {
            Some(managed) => {
                managed.fingerprint = Some(fingerprint);
                true
            }
            None => false,
        }
    }
}

/// Render a path for a reload report / 将路径渲染为热重载报告文本
fn render_path(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DeviceManager;
    use crate::device::{DevicePlugin, DevicePort};
    use rp2350_plugin::RpPlugin;
    use rp2350_plugin::error::PluginError;
    use rp2350_plugin::metadata::PluginMetadata;

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

    fn stub_led() -> StubLed {
        StubLed {
            metadata: PluginMetadata::builder()
                .id("reload.stub")
                .name("Reload Stub")
                .version("0.1.0")
                .description("stub")
                .author("test")
                .abi_version(1)
                .min_host_version("0.1.0")
                .has_ui(false)
                .tags(vec!["test".into()])
                .build(),
            ports: vec![DevicePort::input("LED", "LED input")],
            level: false,
        }
    }

    /// Fresh temporary directory for reload tests / 热重载测试的临时目录
    fn temp_dir(tag: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("rp2350-reload-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    /// Platform library extension used to name fake libraries
    /// 用于伪造库文件的平台库扩展名
    fn platform_ext() -> &'static str {
        crate::loader::PluginLoader::platform_extensions()[0]
    }

    /// Locate the built LED plugin library in the workspace target directory
    /// 在工作区 target 目录中定位已构建的 LED 插件库
    fn locate_led_plugin() -> Option<PathBuf> {
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

    #[test]
    fn reload_empty_directory_reports_nothing() {
        let dir = temp_dir("empty");
        let mut dm = DeviceManager::new();
        let report = dm.reload(&dir);
        assert!(report.is_empty(), "{report:?}");
        assert_eq!(report.change_count(), 0);
        assert_eq!(dm.device_count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_missing_directory_reports_nothing() {
        let dir = temp_dir("missing");
        let mut dm = DeviceManager::new();
        let report = dm.reload(&dir.join("absent"));
        assert!(report.is_empty(), "{report:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_ignores_non_library_files() {
        let dir = temp_dir("nonlib");
        std::fs::write(dir.join("notes.txt"), b"hello").expect("write notes");
        std::fs::write(dir.join("readme.md"), b"# doc").expect("write readme");
        let mut dm = DeviceManager::new();
        let report = dm.reload(&dir);
        assert!(report.is_empty(), "{report:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_records_corrupt_library_as_failed() {
        let dir = temp_dir("corrupt");
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
        let dir = temp_dir("failure");
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
        let dir = temp_dir("retry");
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
            failed: vec![("d.dll".into(), "boom".into())],
        };
        let lines = report.render_lines();
        assert_eq!(lines[0], "[reload] + a.dll");
        assert_eq!(lines[1], "[reload] - b.dll");
        assert_eq!(lines[2], "[reload] ~ c.dll");
        assert_eq!(lines[3], "[reload] ! d.dll: boom");
        assert_eq!(
            lines[4],
            "[reload] summary: added=1 removed=1 reloaded=1 failed=1"
        );
        assert_eq!(report.change_count(), 4);
        assert!(!report.is_empty());
    }

    #[test]
    fn reload_reloads_changed_library_in_place() {
        let Some(source) = locate_led_plugin() else {
            eprintln!("SKIP: LED plugin not built / 未构建 LED 插件，跳过");
            return;
        };
        let dir = temp_dir("changed");
        std::fs::copy(&source, dir.join(source.file_name().expect("file name")))
            .expect("copy plugin");
        let mut dm = DeviceManager::new();
        assert_eq!(dm.scan_and_load(&dir).len(), 1);

        // 注入"已过期"的指纹来模拟文件变化：Windows 上无法写已加载的 DLL。
        // Inject a stale fingerprint to simulate a change: loaded DLLs are locked
        // on Windows, so the file itself cannot be rewritten by the test.
        let stale = Fingerprint {
            len: 0,
            modified: None,
        };
        assert!(dm.set_fingerprint_for_test("com.rp2350-rs.led", stale));

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
        let dir = temp_dir("added");
        let mut dm = DeviceManager::new();
        assert_eq!(dm.scan_and_load(&dir), Vec::<(String, PathBuf)>::new());
        std::fs::copy(&source, dir.join(source.file_name().expect("file name")))
            .expect("copy plugin");
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
        let dir = temp_dir("removed");
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
}
