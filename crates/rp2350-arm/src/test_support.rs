//! Shared helpers for unit tests that touch process-wide state
//! 触及进程级状态的单元测试共享辅助
//!
//! [`rp2350_core::flash::content_generation`] is process-global: creating a
//! [`rp2350_core::FlashRegion`], programming it through a host handle, erasing
//! it or loading bytes through the loader each bump the same counter. A cache
//! therefore cannot tell "this test rewrote its own flash" from "another test
//! created or programmed an unrelated region", so every test that either
//! observes that counter (via `FetchCache`/`DecodeCache`) or bumps it must not
//! overlap with another such test.
//! [`rp2350_core::flash::content_generation`] 是进程级全局量：构造
//! [`rp2350_core::FlashRegion`]、经宿主句柄编程/擦除或经加载器写入字节都会递增
//! 同一个计数器。因此缓存无法区分「本测试改写了自己的 Flash」与「另一个测试构造
//! 或编程了无关区域」。凡观察该计数器（经 `FetchCache`/`DecodeCache`）或递增它
//! 的测试都不得与同类测试重叠。
//!
//! `#[cfg(test)]` only; ordinary tests that never touch flash need no lock.
//! 仅用于 `#[cfg(test)]`；完全不接触 Flash 的测试无需加锁。

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Serializes tests that depend on the process-global flash generation
/// 串行化依赖进程级 Flash 世代号的测试
static FLASH_GENERATION_LOCK: Mutex<()> = Mutex::new(());

/// Acquire the flash-generation test lock
/// 获取 Flash 世代测试锁
///
/// Hold the returned guard for the whole test. Poisoning is ignored so that
/// one failed test cannot cascade into unrelated failures.
/// 返回的守卫需持有到测试结束。忽略中毒，避免一个失败测试级联影响其它测试。
pub(crate) fn flash_generation_lock() -> MutexGuard<'static, ()> {
    FLASH_GENERATION_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}
