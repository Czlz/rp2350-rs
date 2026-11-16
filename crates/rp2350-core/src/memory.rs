//! RAM regions and the flat address bus used by tests and the boot path
//! 供测试与启动路径使用的 RAM 区域和扁平地址总线
//!
//! This module owns the single-threaded storage and routing types. The region
//! implementation lives in a private `ram` submodule and the router in a private
//! `simple` submodule; both are re-exported here.
//! 本模块提供单线程的存储与路由类型。区域实现位于私有 `ram` 子模块，路由器位于
//! 私有 `simple` 子模块，两者都在此处重新导出。
//!
//! * [`RamRegion`] — ten word-interleaved bank buffers behind per-bank locks.
//! * [`SimpleBus`] — the single-threaded router with a 256-bucket route cache;
//!   the thread-safe [`crate::shared::SharedBus`] mirrors it.
//! * [`RamRegion`] — 十个按字交错、每 bank 一把锁的缓冲区。
//! * [`SimpleBus`] — 带 256 桶路由缓存的单线程路由器；线程安全的
//!   [`crate::shared::SharedBus`] 与之对应。

mod ram;
mod ram_shared;
mod simple;

pub use ram::{RAM_BANKS, RamRegion};
pub use simple::{REGION_BUCKETS, SimpleBus};

pub(crate) use simple::{PPB_WINDOW_BASE, PPB_WINDOW_SIZE};

#[cfg(test)]
mod tests;
