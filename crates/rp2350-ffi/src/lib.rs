//! rp2350-ffi — RAII-safe dynamic library loading wrapper
//!
//! Provides `SafeLibrary`, an RAII wrapper around `libloading::Library`
//! that ensures proper resource cleanup. All FFI interactions with
//! `.dll` / `.so` / `.dylib` plugin files go through this crate.
//!
//! 提供 `SafeLibrary`，一个封装 `libloading::Library` 的 RAII 包装器，
//! 确保资源正确释放。所有与 `.dll` / `.so` / `.dylib` 插件文件的 FFI 交互都通过此 crate。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

use std::path::Path;

/// A RAII wrapper around a dynamically loaded library
/// 动态加载库的 RAII 包装器
///
/// Automatically calls `Library::close()` on drop.
/// 在 drop 时自动调用 `Library::close()`。
pub struct SafeLibrary {
    /// The underlying libloading library / 底层 libloading 库
    inner: libloading::Library,
    /// Path this library was loaded from / 库的加载路径
    path: std::path::PathBuf,
}

impl SafeLibrary {
    /// Load a dynamic library from the given path
    /// 从指定路径加载动态库
    ///
    /// # Errors
    ///
    /// Returns `Err` if `libloading::Library::new` fails (file not found,
    /// invalid format, missing dependencies, etc.)
    /// 如果 `libloading::Library::new` 失败（文件未找到、格式无效、缺少依赖等）则返回 `Err`。
    pub fn load(path: impl AsRef<Path>) -> Result<Self, libloading::Error> {
        let path = path.as_ref().to_path_buf();
        // SAFETY: libloading::Library::new is safe — it only loads the lib, does not call symbols
        let inner = unsafe { libloading::Library::new(&path)? };
        Ok(Self { inner, path })
    }

    /// Get a reference to the underlying library
    /// 获取底层库的引用
    #[must_use]
    pub fn inner(&self) -> &libloading::Library {
        &self.inner
    }

    /// Get the path this library was loaded from
    /// 获取库的加载路径
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Look up a symbol by name
    /// 按名称查找符号
    ///
    /// # Safety / 安全性
    ///
    /// The caller must ensure that the symbol type matches the actual
    /// exported symbol. `libloading` performs no type checking.
    /// 调用者必须确保符号类型与实际导出的符号匹配。`libloading` 不执行类型检查。
    ///
    /// # Errors
    ///
    /// Returns `Err` if the symbol is not found.
    /// 如果符号未找到则返回 `Err`。
    pub unsafe fn find_symbol<T>(&self, name: &[u8]) -> Result<libloading::Symbol<'_, T>, libloading::Error> {
        // SAFETY: delegated to libloading; caller must ensure T matches the symbol type
        unsafe { self.inner.get::<T>(name) }
    }
}

impl Drop for SafeLibrary {
    fn drop(&mut self) {
        log::debug!("Unloading library: {}", self.path.display());
    }
}

// SafeLibrary is Send + Sync because libloading::Library is Send + Sync
// SAFETY: libloading::Library is documented as Send + Sync
unsafe impl Send for SafeLibrary {}
unsafe impl Sync for SafeLibrary {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_nonexistent_fails() {
        let result = SafeLibrary::load("nonexistent_file_12345.dll");
        assert!(result.is_err());
    }
}