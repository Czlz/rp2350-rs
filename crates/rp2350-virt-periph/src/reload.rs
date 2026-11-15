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
use crate::wiring::WiringEntry;

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
    /// Reloaded libraries whose wiring was re-applied to the new instance
    /// 接线已成功重新应用到新实例的重载库
    pub rewired: Vec<String>,
    /// Reloaded libraries whose wiring was dropped, with the reason
    /// 接线被清除的重载库及原因
    ///
    /// The reason is bilingual and names the port that changed or disappeared.
    /// The wiring table never keeps entries that point at a stale port.
    /// 原因为双语文本，指出变化或消失的端口；接线表不会保留指向过期端口的条目。
    pub unwired: Vec<(String, String)>,
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
            && self.rewired.is_empty()
            && self.unwired.is_empty()
            && self.failed.is_empty()
    }

    /// Total number of reported entries / 报告条目总数
    #[must_use]
    pub fn change_count(&self) -> usize {
        self.added.len()
            + self.removed.len()
            + self.reloaded.len()
            + self.rewired.len()
            + self.unwired.len()
            + self.failed.len()
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
        lines.extend(self.rewired.iter().map(|path| format!("[reload] = {path}")));
        lines.extend(
            self.unwired
                .iter()
                .map(|(path, reason)| format!("[reload] x {path}: {reason}")),
        );
        lines.extend(
            self.failed
                .iter()
                .map(|(path, error)| format!("[reload] ! {path}: {error}")),
        );
        lines.push(format!(
            "[reload] summary: added={} removed={} reloaded={} rewired={} unwired={} failed={}",
            self.added.len(),
            self.removed.len(),
            self.reloaded.len(),
            self.rewired.len(),
            self.unwired.len(),
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
    ///   and dropped, its wiring entries are removed, then its library is
    ///   unloaded.
    /// - **reloaded** — same path, different size or modification time: the old
    ///   instance is dropped, the old library unloaded (mandatory on Windows,
    ///   where loaded DLLs are locked), and the library loaded again. Wiring is
    ///   captured before the drop and re-applied when the new instance exposes
    ///   the same port indices and names (`rewired`).
    /// - **rewired** — the replacement kept the port table, so every existing
    ///   wiring entry (device, port index → GPIO pin) stays valid.
    /// - **unwired** — the replacement changed a port index/name (or its device
    ///   ID): all wiring of that device is dropped and the reason is recorded;
    ///   the wiring table never references a port that no longer exists.
    /// - **added** — a new library: loaded and registered.
    /// - **failed** — a library that could not be loaded; the error is kept in
    ///   [`ReloadReport::failed`] and the rest of the manager keeps working. A
    ///   failed *replacement* leaves no device behind (the old library must be
    ///   unloaded before the file can be reopened on Windows).
    /// - **removed / 已移除** — 文件已消失：先关闭并释放设备实例、清除其接线，
    ///   再卸载库。
    /// - **reloaded / 已重载** — 路径相同但大小或修改时间变化：先释放旧实例、
    ///   卸载旧库（Windows 上必须如此，已加载 DLL 会被锁定），再重新加载。释放
    ///   前会快照接线，新实例端口索引与名称一致时重新应用（`rewired`）。
    /// - **rewired / 接线保留** — 替换实例保持相同端口表，原有接线条目
    ///   （设备、端口索引 → GPIO 引脚）继续有效。
    /// - **unwired / 接线清除** — 替换实例的端口索引/名称（或设备 ID）变化：
    ///   该设备的全部接线被清除并记录原因；接线表不会引用已不存在的端口。
    /// - **added / 新增** — 新库：加载并注册。
    /// - **failed / 失败** — 加载失败：错误记入 [`ReloadReport::failed`]，管理器
    ///   其余部分继续可用。替换失败时不会保留旧设备（Windows 上必须先卸载旧库
    ///   才能重新打开同一文件）。
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
            let key = dir_key.join(file_name);
            // 同一文件可能被多个宿主 ID 加载；热重载按文件跟踪，固定选择字典序
            // 最小的 ID，避免 HashMap 迭代顺序导致行为不确定。
            // One file may back several host IDs; reload tracks files and picks the
            // smallest ID so the outcome never depends on HashMap order.
            match previous.get_mut(&key) {
                Some(slot) => {
                    if id < &slot.0 {
                        *slot = (id.clone(), managed.fingerprint);
                    }
                }
                None => {
                    previous.insert(key, (id.clone(), managed.fingerprint));
                }
            }
        }

        // 1) 先卸载消失的库，确保 Windows 上的文件锁被释放。
        // First unload vanished libraries so Windows file locks are released.
        for (key, (id, _fingerprint)) in &previous {
            if !current.contains_key(key) {
                self.drop_device_instance(id);
                report.removed.push(render_path(key));
            }
        }

        // 2) 已变化的库：快照接线 → 释放旧实例 → 卸载旧库 → 重新加载。
        // Changed libraries: snapshot wiring, drop the old instance, unload, load again.
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
            let wiring = self.wiring.entries_for_device(old_id);
            self.drop_device_instance(old_id);
            match self.insert_loaded(path, Some(*fingerprint)) {
                Ok(new_id) => {
                    report.reloaded.push(render_path(key));
                    if wiring.is_empty() {
                        continue;
                    }
                    let restored = if new_id == *old_id {
                        self.restore_wiring(&new_id, &wiring)
                    } else {
                        Err(format!(
                            "device id changed from '{old_id}' to '{new_id}' / \
                             设备 ID 由 '{old_id}' 变为 '{new_id}'"
                        ))
                    };
                    match restored {
                        Ok(()) => report.rewired.push(render_path(key)),
                        Err(reason) => {
                            // 端口表不兼容时清空该设备的全部接线，保证 devices/wiring 一致。
                            // Drop every entry so devices and wiring stay consistent.
                            self.wiring.remove_device(&new_id);
                            report.unwired.push((render_path(key), reason));
                        }
                    }
                }
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

    /// Re-apply captured wiring entries to a freshly loaded device
    /// 将快照中的接线条目重新应用到刚加载的设备
    ///
    /// An error is returned when the replacement no longer exposes the same
    /// `(port_index, port_name)` pair; the caller then removes every entry of
    /// that device so the wiring table never points at a stale port.
    /// 替换实例不再提供相同的 `(端口索引, 端口名)` 时返回错误；调用方随后删除该
    /// 设备的全部条目，确保接线表不会指向过期端口。
    fn restore_wiring(&mut self, id: &DeviceId, entries: &[WiringEntry]) -> Result<(), String> {
        for entry in entries {
            {
                let Some(managed) = self.devices.get(id) else {
                    return Err("device vanished after reload / 重载后设备消失".to_string());
                };
                let ports = managed.plugin.ports();
                match ports.get(entry.port_index) {
                    Some(port) if port.name == entry.port_name => {}
                    Some(port) => {
                        return Err(format!(
                            "port {} changed from '{}' to '{}' / 端口 {} 由 '{}' 变为 '{}'",
                            entry.port_index,
                            entry.port_name,
                            port.name,
                            entry.port_index,
                            entry.port_name,
                            port.name
                        ));
                    }
                    None => {
                        return Err(format!(
                            "port {} ('{}') no longer exists in the reloaded plugin / \
                             重载后的插件不再提供端口 {}（'{}'）",
                            entry.port_index, entry.port_name, entry.port_index, entry.port_name
                        ));
                    }
                }
            }
            self.wiring
                .wire(id, &entry.port_name, entry.port_index, entry.gpio_pin);
        }
        Ok(())
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

    /// Test-only: point an existing device at a library path and fingerprint
    /// 仅测试用：将已有设备指向某个库路径与指纹
    ///
    /// Lets tests model "the file behind this device was replaced / deleted"
    /// without writing to a loaded DLL, which Windows locks. Returns `false` for
    /// unknown IDs. 让测试无需写入已加载 DLL（Windows 会锁定）即可模拟“设备背后的
    /// 文件被替换/删除”；ID 不存在时返回 `false`。
    #[cfg(test)]
    pub(crate) fn set_source_for_test(
        &mut self,
        id: &str,
        path: PathBuf,
        fingerprint: Option<Fingerprint>,
    ) -> bool {
        match self.devices.get_mut(id) {
            Some(managed) => {
                managed.source_path = Some(path);
                managed.fingerprint = fingerprint;
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
