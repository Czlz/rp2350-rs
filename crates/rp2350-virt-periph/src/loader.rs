//! Plugin loader — scans directories and loads plugin libraries
//! 插件加载器 — 扫描目录并加载插件库

use std::path::{Path, PathBuf};

use rp2350_ffi::SafeLibrary;

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

    /// Scan the default plugin directory for compatible libraries
    /// 扫描默认插件目录以查找兼容的库
    #[must_use]
    pub fn scan(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();

        // Default extensions per platform
        #[cfg(target_os = "windows")]
        let exts = ["dll"];
        #[cfg(target_os = "linux")]
        let exts = ["so"];
        #[cfg(target_os = "macos")]
        let exts = ["dylib", "so"];

        for search_path in &self.search_paths {
            if let Ok(entries) = std::fs::read_dir(search_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if exts.contains(&ext) {
                            found.push(path);
                        }
                    }
                }
            }
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
