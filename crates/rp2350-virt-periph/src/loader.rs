//! Plugin loader — scans directories and loads plugin libraries
//! 插件加载器 — 扫描目录并加载插件库

// 跨动态库边界传递 trait 对象指针依赖同一编译器与 ABI 版本，这是插件 ABI 的固有设计。
// Passing trait-object pointers across a dynamic library boundary relies on the
// same compiler and ABI version, which is inherent to the plugin ABI design.
#![allow(improper_ctypes_definitions)]

use std::path::{Path, PathBuf};

use rp2350_ffi::SafeLibrary;
use rp2350_plugin::abi::{ABI_VERSION, SYMBOL_DEVICE_CREATE, SYMBOL_IS_DEVICE, SYMBOL_VERSION};

use super::device::DevicePlugin;
use super::error::DeviceError;

/// Function pointer type for the plugin ABI version export
/// 插件 ABI 版本导出函数的指针类型
type VersionFn = unsafe extern "C" fn() -> u32;

/// Function pointer type for the "is device plugin" export
/// “是否为设备插件”导出函数的指针类型
type IsDeviceFn = unsafe extern "C" fn() -> u32;

/// Function pointer type for the device plugin factory export
/// 设备插件工厂导出函数的指针类型
type DeviceCreateFn = unsafe extern "C" fn() -> *mut dyn DevicePlugin;

/// Build a load error that always names the offending path
/// 构造始终携带出错路径的加载错误
///
/// The rendered message is `<path>: <english> / <chinese>` so CLI/GUI users see
/// which file failed before the reason. 渲染格式为 `<路径>: <英文> / <中文>`，
/// 让 CLI/GUI 用户先看到**哪个文件**出错，再看原因。
fn path_error(path: &Path, message: impl std::fmt::Display) -> DeviceError {
    DeviceError::PluginLoadFailed(format!("{}: {message}", path.display()))
}

/// Build a "missing ABI symbol" error carrying path and symbol name
/// 构造携带路径与符号名的“缺少 ABI 符号”错误
fn missing_symbol(path: &Path, symbol: &str, error: impl std::fmt::Display) -> DeviceError {
    path_error(
        path,
        format!("missing ABI symbol '{symbol}': {error} / 缺少 ABI 符号 '{symbol}'：{error}"),
    )
}

/// Reject paths that cannot possibly be a loadable plugin library
/// 拒绝不可能成为可加载插件的路径
///
/// Directories, special files and empty files are rejected **before** the OS
/// dynamic loader is invoked so the error names the real problem instead of a
/// platform-specific `LoadLibrary`/`dlopen` message. 目录、特殊文件与空文件在调用
/// 操作系统动态加载器**之前**被拒绝，使错误描述真实原因而不是平台相关的
/// `LoadLibrary`/`dlopen` 报文。
fn ensure_loadable_file(path: &Path) -> Result<(), DeviceError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        path_error(
            path,
            format!("cannot access plugin file: {error} / 无法访问插件文件：{error}"),
        )
    })?;
    if !metadata.is_file() {
        return Err(path_error(
            path,
            "not a regular file; directories and special files cannot be plugins / \
             不是普通文件；目录与特殊文件不能作为插件加载",
        ));
    }
    if metadata.len() == 0 {
        return Err(path_error(
            path,
            "empty library file (0 bytes) / 空库文件（0 字节）",
        ));
    }
    Ok(())
}

/// A loaded plugin library kept alive together with its device instance
/// 与其设备实例一同保持存活的已加载插件库
///
/// `plugin` is declared **before** `library` because Rust drops struct fields in
/// declaration order: the trait object (whose vtable lives inside the library)
/// must be destroyed while the library is still mapped. 字段按声明顺序析构，
/// 因此 `plugin` 必须声明在 `library` 之前：trait 对象的 vtable 位于库内，
/// 必须在库仍被映射时销毁。
pub struct LoadedPlugin {
    /// Instantiated device plugin created by the library / 由库实例化的设备插件
    pub plugin: Box<dyn DevicePlugin>,
    /// Loaded dynamic library backing `plugin` / 支撑 `plugin` 的动态库
    pub library: SafeLibrary,
}

/// Scans a directory for plugin files and loads them as dynamic libraries.
/// 扫描目录中的插件文件并将其加载为动态库。
pub struct PluginLoader {
    /// Search paths for plugins / 插件搜索路径
    search_paths: Vec<PathBuf>,
}

impl PluginLoader {
    /// Create a new plugin loader / 创建新的插件加载器
    #[must_use]
    pub fn new() -> Self {
        Self {
            search_paths: Vec::new(),
        }
    }

    /// Add a search path / 添加搜索路径
    pub fn add_search_path(&mut self, path: impl AsRef<Path>) {
        self.search_paths.push(path.as_ref().to_path_buf());
    }

    /// Platform-specific loadable library extensions
    /// 平台相关的可加载库扩展名
    ///
    /// Windows uses `dll`, Linux `so`, macOS both `dylib` and `so`.
    /// Windows 使用 `dll`，Linux 使用 `so`，macOS 使用 `dylib` 与 `so`。
    #[must_use]
    pub fn platform_extensions() -> &'static [&'static str] {
        #[cfg(target_os = "windows")]
        {
            &["dll"]
        }
        #[cfg(target_os = "linux")]
        {
            &["so"]
        }
        #[cfg(target_os = "macos")]
        {
            &["dylib", "so"]
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
        {
            &["dylib", "so", "dll"]
        }
    }

    /// Scan one directory for compatible libraries, sorted by path
    /// 扫描单个目录以查找兼容的库，结果按路径排序
    ///
    /// A missing or unreadable directory yields an empty list.
    /// 目录不存在或不可读时返回空列表。
    #[must_use]
    pub fn scan_dir(dir: impl AsRef<Path>) -> Vec<PathBuf> {
        let exts = Self::platform_extensions();
        let mut found = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if exts.contains(&ext) {
                        found.push(path);
                    }
                }
            }
        }
        // 目录迭代顺序不稳定，排序保证扫描与加载顺序可复现。
        // Directory iteration order is unstable; sorting keeps scan and load order reproducible.
        found.sort();
        found
    }

    /// Scan the configured search paths for compatible libraries
    /// 扫描已配置的搜索路径以查找兼容的库
    #[must_use]
    pub fn scan(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for search_path in &self.search_paths {
            found.extend(Self::scan_dir(search_path));
        }
        found
    }

    /// Attempt to load a single plugin library
    /// 尝试加载单个插件库
    ///
    /// The path is validated first (must exist, must be a regular non-empty
    /// file) and the OS dynamic loader is only invoked afterwards.
    /// 先校验路径（必须存在、是普通文件且非空），随后才调用操作系统动态加载器。
    ///
    /// # Errors
    ///
    /// Returns [`DeviceError::PluginLoadFailed`] with a bilingual message naming
    /// the path when the file is missing, is a directory, is empty, or cannot be
    /// loaded as a dynamic library.
    /// 文件不存在、是目录、为空或无法作为动态库加载时返回带路径的双语
    /// [`DeviceError::PluginLoadFailed`]。
    pub fn load(&self, path: &Path) -> Result<SafeLibrary, DeviceError> {
        ensure_loadable_file(path)?;
        SafeLibrary::load(path).map_err(|error| {
            path_error(
                path,
                format!("failed to open dynamic library: {error} / 打开动态库失败：{error}"),
            )
        })
    }

    /// Load and instantiate a device plugin from a dynamic library
    /// 从动态库加载并实例化设备插件
    ///
    /// The returned [`LoadedPlugin`] keeps the library alive alongside its plugin
    /// and drops the plugin before the library, so the trait-object vtable is
    /// never unmapped while still in use. 返回的 [`LoadedPlugin`] 让库与插件一同
    /// 存活，并且先释放插件再卸载库，避免 vtable 仍在使用时被解除映射。
    ///
    /// Every failure path drops the library handle and creates no device, so a
    /// rejected plugin never leaks a loaded module. 所有失败路径都会释放库句柄且
    /// 不创建设备实例，被拒绝的插件不会泄漏已加载模块。
    ///
    /// # Errors
    ///
    /// Returns [`DeviceError::PluginLoadFailed`] when the library cannot be
    /// loaded, is not a regular non-empty file, lacks one of the ABI symbols
    /// (`rp2350_plugin_version`, `rp2350_plugin_is_device`,
    /// `rp2350_device_plugin_create`), reports a different ABI version, reports
    /// that it is not a device plugin, or its factory returns a null pointer.
    /// 库无法加载、不是非空普通文件、缺少 ABI 符号
    /// （`rp2350_plugin_version` / `rp2350_plugin_is_device` /
    /// `rp2350_device_plugin_create`）、ABI 版本不匹配、声明自己不是设备插件，
    /// 或工厂返回空指针时返回 [`DeviceError::PluginLoadFailed`]。
    pub fn load_device(&self, path: &Path) -> Result<LoadedPlugin, DeviceError> {
        let library = self.load(path)?;

        // SAFETY: `find_symbol` is unsafe because libloading cannot type-check the
        // export; `VersionFn` mirrors the documented plugin ABI and the library
        // stays alive inside the returned `LoadedPlugin`.
        // SAFETY: `find_symbol` 之所以不安全，是因为 libloading 无法对导出做类型检查；
        // `VersionFn` 与文档化的插件 ABI 一致，且库在返回的 `LoadedPlugin` 中保持存活。
        let version = unsafe { library.find_symbol::<VersionFn>(SYMBOL_VERSION.as_bytes()) }
            .map_err(|error| missing_symbol(path, SYMBOL_VERSION, error))?;
        // SAFETY: `version` is a valid function pointer of the declared type.
        // SAFETY: `version` 是声明类型的有效函数指针。
        let version = unsafe { version() };
        if version != ABI_VERSION {
            return Err(path_error(
                path,
                format!(
                    "incompatible plugin ABI: expected {ABI_VERSION}, got {version} / \
                     插件 ABI 不兼容：期望 {ABI_VERSION}，实际 {version}"
                ),
            ));
        }

        // SAFETY: same symbol contract as above; `IsDeviceFn` is the documented
        // `extern "C" fn() -> u32` export.
        // SAFETY: 符号契约同上；`IsDeviceFn` 是文档化的 `extern "C" fn() -> u32` 导出。
        let is_device = unsafe { library.find_symbol::<IsDeviceFn>(SYMBOL_IS_DEVICE.as_bytes()) }
            .map_err(|error| missing_symbol(path, SYMBOL_IS_DEVICE, error))?;
        // SAFETY: `is_device` is a valid function pointer of the declared type.
        // SAFETY: `is_device` 是声明类型的有效函数指针。
        if unsafe { is_device() } != 1 {
            return Err(path_error(
                path,
                "plugin is not a device plugin / 插件不是设备插件",
            ));
        }

        // SAFETY: same symbol contract as above; the factory is looked up before
        // being called, so a missing export is reported instead of dereferenced.
        // SAFETY: 符号契约同上；工厂在调用前先查找，缺少导出时返回错误而不是解引用。
        let create =
            unsafe { library.find_symbol::<DeviceCreateFn>(SYMBOL_DEVICE_CREATE.as_bytes()) }
                .map_err(|error| missing_symbol(path, SYMBOL_DEVICE_CREATE, error))?;
        // SAFETY: `create` follows the plugin ABI and returns an owned trait
        // object allocated inside the plugin library (or null on failure).
        // SAFETY: `create` 遵循插件 ABI，返回由插件库内部分配的、拥有所有权的 trait
        // 对象（失败时返回空指针）。
        let raw = unsafe { create() };
        if raw.is_null() {
            return Err(path_error(
                path,
                "plugin factory returned a null pointer / 插件工厂返回空指针",
            ));
        }
        // SAFETY: `raw` is non-null and exclusively owned by this loader, so it
        // may be reclaimed exactly once as a `Box`.
        // SAFETY: `raw` 非空且所有权由本加载器独占，可作为 `Box` 恰好回收一次。
        let plugin = unsafe { Box::from_raw(raw) };

        Ok(LoadedPlugin { plugin, library })
    }

    /// Scan and load all plugins from search paths
    /// 扫描并加载搜索路径中的所有插件
    ///
    /// Returns a list of successfully loaded libraries. No device is
    /// instantiated — use [`crate::DeviceManager::scan_and_load`] for devices.
    /// 返回成功加载的库列表；**不实例化设备** —— 设备级加载请使用
    /// [`crate::DeviceManager::scan_and_load`]。
    #[must_use]
    pub fn scan_and_load(&self) -> Vec<(PathBuf, SafeLibrary)> {
        self.scan()
            .into_iter()
            .filter_map(|path| match SafeLibrary::load(&path) {
                Ok(lib) => Some((path, lib)),
                Err(e) => {
                    log::warn!("Failed to load plugin {}: {}", path.display(), e);
                    None
                }
            })
            .collect()
    }
}

impl Default for PluginLoader {
    fn default() -> Self {
        let mut loader = Self::new();
        loader.add_search_path("plugins");
        loader
    }
}
