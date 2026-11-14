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

/// A loaded plugin library kept alive together with its device instance
/// 与其设备实例一同保持存活的已加载插件库
///
/// The library must outlive `plugin` because the trait object's vtable lives
/// inside the library.
/// 库必须比 `plugin` 存活更久，因为 trait 对象的 vtable 位于库内。
pub struct LoadedPlugin {
    /// Loaded dynamic library / 已加载的动态库
    pub library: SafeLibrary,
    /// Instantiated device plugin created by the library / 由库实例化的设备插件
    pub plugin: Box<dyn DevicePlugin>,
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
    /// # Errors
    ///
    /// Returns `Err` if the library cannot be loaded.
    pub fn load(&self, path: &Path) -> Result<SafeLibrary, crate::error::DeviceError> {
        SafeLibrary::load(path)
            .map_err(|e| crate::error::DeviceError::PluginLoadFailed(e.to_string()))
    }

    /// Load and instantiate a device plugin from a dynamic library
    /// 从动态库加载并实例化设备插件
    ///
    /// The returned [`LoadedPlugin`] keeps the library alive alongside its plugin.
    /// 返回的 [`LoadedPlugin`] 会让库与插件一同保持存活。
    ///
    /// # Errors
    ///
    /// Returns `DeviceError::PluginLoadFailed` if the library cannot be loaded,
    /// lacks a required ABI symbol, or reports an incompatible ABI version.
    /// 当库无法加载、缺少必需的 ABI 符号或 ABI 版本不兼容时，返回
    /// `DeviceError::PluginLoadFailed`。
    pub fn load_device(&self, path: &Path) -> Result<LoadedPlugin, DeviceError> {
        let library = self.load(path)?;

        // SAFETY: the symbol type follows the plugin ABI and the library stays
        // alive inside the returned `LoadedPlugin`.
        // SAFETY: 符号类型遵循插件 ABI，且库在返回的 `LoadedPlugin` 中保持存活。
        let version = unsafe { library.find_symbol::<VersionFn>(SYMBOL_VERSION.as_bytes()) }
            .map_err(|e| DeviceError::PluginLoadFailed(e.to_string()))?;
        // SAFETY: `version` is a valid function pointer of the declared type.
        // SAFETY: `version` 是声明类型的有效函数指针。
        let version = unsafe { version() };
        if version != ABI_VERSION {
            return Err(DeviceError::PluginLoadFailed(format!(
                "incompatible plugin ABI: expected {ABI_VERSION}, got {version} / \
                 插件 ABI 不兼容：期望 {ABI_VERSION}，实际 {version}"
            )));
        }

        // SAFETY: same symbol contract as above.
        // SAFETY: 符号契约同上。
        let is_device = unsafe { library.find_symbol::<IsDeviceFn>(SYMBOL_IS_DEVICE.as_bytes()) }
            .map_err(|e| DeviceError::PluginLoadFailed(e.to_string()))?;
        // SAFETY: `is_device` is a valid function pointer of the declared type.
        // SAFETY: `is_device` 是声明类型的有效函数指针。
        if unsafe { is_device() } != 1 {
            return Err(DeviceError::PluginLoadFailed(
                "plugin is not a device plugin / 插件不是设备插件".to_string(),
            ));
        }

        // SAFETY: same symbol contract as above.
        // SAFETY: 符号契约同上。
        let create =
            unsafe { library.find_symbol::<DeviceCreateFn>(SYMBOL_DEVICE_CREATE.as_bytes()) }
                .map_err(|e| DeviceError::PluginLoadFailed(e.to_string()))?;
        // SAFETY: `create` follows the plugin ABI and returns an owned trait
        // object allocated inside the plugin library.
        // SAFETY: `create` 遵循插件 ABI，返回由插件库内部分配的、拥有所有权的 trait 对象。
        let raw = unsafe { create() };
        if raw.is_null() {
            return Err(DeviceError::PluginLoadFailed(
                "plugin factory returned a null pointer / 插件工厂返回空指针".to_string(),
            ));
        }
        // SAFETY: `raw` is non-null and exclusively owned by this loader, so it
        // may be reclaimed exactly once as a `Box`.
        // SAFETY: `raw` 非空且所有权由本加载器独占，可作为 `Box` 恰好回收一次。
        let plugin = unsafe { Box::from_raw(raw) };

        Ok(LoadedPlugin { library, plugin })
    }

    /// Scan and load all plugins from search paths
    /// 扫描并加载搜索路径中的所有插件
    ///
    /// Returns a list of successfully loaded libraries.
    /// 返回成功加载的库列表。
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
