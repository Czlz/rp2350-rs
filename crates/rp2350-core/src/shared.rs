//! Thread-safe bus layer for concurrent (multi-core) execution
//! 面向并发（多核）执行的线程安全总线层
//!
//! The single-threaded path keeps using [`AddressBus`](crate::bus::AddressBus)
//! and [`MemoryRegion`]: those traits take `&mut self`,
//! so the interpreter pays nothing for sharing it never uses. This module adds
//! the parallel layer the multi-core work needs:
//! 单线程路径继续使用 [`AddressBus`](crate::bus::AddressBus) 与
//! [`MemoryRegion`]：这两个 trait 取 `&mut self`，因此
//! 解释器不会为「它从未用到的共享」付出任何代价。本模块提供多核工作所需的并发层：
//!
//! * [`SharedRegion`] — the same region contract with `&self` accessors. Every
//!   implementation owns its own locking granularity (SRAM banks, per-MMIO-device
//!   mutexes, lock-free flash words), so two cores touching different banks or
//!   different devices never serialise.
//! * [`SharedAddressBus`] — routing with `&self`, so one bus can be shared by an
//!   `Arc` across core threads.
//! * [`SharedBus`] — the concrete thread-safe bus: the same 256-bucket
//!   direct-mapped route cache as `SimpleBus`, but the tags are atomics and stale
//!   entries are revalidated against the region window, so a racing fill can only
//!   cost a scan, never a wrong route.
//! * [`LockedRegion`] — a bridge for regions that still implement only the
//!   exclusive trait: it wraps them in one mutex (one lock per region, never a
//!   global bus lock).
//! * [`SharedRegion`]: 相同的区域契约，但访问器取 `&self`。每个实现自带锁粒度
//!   （SRAM bank、每 MMIO 设备一把锁、Flash 无锁字），因此两核访问不同 bank 或
//!   不同设备时永不串行。
//! * [`SharedAddressBus`]: 取 `&self` 的路由，因此一条总线可以经 `Arc` 跨核共享。
//! * [`SharedBus`]: 具体的线程安全总线，与 `SimpleBus` 相同的 256 桶直接映射路由
//!   缓存，但标签是原子量，过期条目会用区域窗口重新校验，因此竞争中的填充最多
//!   多一次扫描，绝不会路由到错误的区域。
//! * [`LockedRegion`]: 仍只实现独占 trait 的区域桥接器，用一把锁包住该区域
//!   （每区域一把锁，绝不是全局总线锁）。
//!
//! Lock order / 锁顺序: a bus access may take at most one region lock (SRAM may
//! take two bank locks, always in ascending bank index). No region lock is ever
//! held while another region is entered, so the lock graph is acyclic.
//! 一次总线访问最多获取一个区域锁（SRAM 可能取两把 bank 锁，始终按 bank 下标
//! 升序）。任何区域锁都不会在进入另一个区域时继续持有，因此锁图无环。

// 并发总线层涉及地址、桶下标与宽度的定宽换算，这是内存映射层的固有特性。
// The concurrent bus layer converts fixed-width addresses, bucket indices and
// access widths, which is inherent to the memory-mapped layer.
#![allow(
    clippy::cast_possible_truncation,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};

use parking_lot::Mutex;

use crate::bus::BusResult;
use crate::bus::MemoryRegion;
use crate::error::{BusErrorKind, CoreError};
use crate::memory::{PPB_WINDOW_BASE, PPB_WINDOW_SIZE, REGION_BUCKETS};
use crate::types::Address;

/// A memory or MMIO region that can be read and written through `&self`
/// 可经 `&self` 读写的内存或 MMIO 区域
///
/// The concurrent counterpart of [`MemoryRegion`].
/// Implementations must make each individual access atomic with respect to other
/// threads: an access must never expose a half-written value **inside the
/// storage it locks**. Accesses wider than one lockable unit are split; see each
/// implementation for its documented granularity.
/// [`MemoryRegion`] 的并发版本。实现必须保证**单次**访问
/// 相对其他线程原子：绝不在其加锁的存储内暴露写了一半的值。宽于一个可锁单元的
/// 访问会被拆分；粒度见各实现的文档。
pub trait SharedRegion: Send + Sync {
    /// Region name for debugging / 区域名称（调试用）
    fn name(&self) -> &str;

    /// Start address (inclusive) / 起始地址（含）
    fn base(&self) -> Address;

    /// Region size in bytes / 区域大小（字节）
    fn size(&self) -> usize;

    /// End address (exclusive) / 结束地址（不含）
    fn end(&self) -> Address {
        self.base().wrapping_add(self.size() as u32)
    }

    /// Check if `addr` falls within this region / 检查地址是否在此区域内
    fn contains(&self, addr: Address) -> bool {
        addr >= self.base() && addr < self.end()
    }

    /// Whether a read of `addr` would currently stall
    /// 读取 `addr` 目前是否会 stall
    ///
    /// Same contract as [`MemoryRegion::read_would_stall`]: the bus rejects the
    /// read with [`BusErrorKind::Stall`] before the region is touched.
    /// 与 [`MemoryRegion::read_would_stall`] 相同：总线在触碰区域之前以
    /// [`BusErrorKind::Stall`] 拒绝读取。
    fn read_would_stall(&self, _addr: Address) -> bool {
        false
    }

    /// Read a single byte / 读取一个字节
    ///
    /// # Errors
    /// Returns `BusError` if the address is invalid or the access fails.
    /// 地址无效或访问失败时返回 `BusError`。
    fn read_u8(&self, addr: Address, cycles: &mut u64) -> BusResult<u8>;

    /// Write a single byte / 写入一个字节
    ///
    /// # Errors
    /// Returns `BusError` if the address is invalid or the access is denied.
    /// 地址无效或访问被拒绝时返回 `BusError`。
    fn write_u8(&self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()>;

    /// Read a 16-bit halfword / 读取 16 位半字
    ///
    /// # Errors
    /// Returns `BusError` if the address is invalid or the access fails.
    /// 地址无效或访问失败时返回 `BusError`。
    fn read_u16(&self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        let lo = u16::from(self.read_u8(addr, cycles)?);
        let hi = u16::from(self.read_u8(addr.wrapping_add(1), cycles)?);
        Ok(lo | (hi << 8))
    }

    /// Write a 16-bit halfword / 写入 16 位半字
    ///
    /// # Errors
    /// Returns `BusError` if the address is invalid or the access is denied.
    /// 地址无效或访问被拒绝时返回 `BusError`。
    fn write_u16(&self, addr: Address, value: u16, cycles: &mut u64) -> BusResult<()> {
        self.write_u8(addr, value as u8, cycles)?;
        self.write_u8(addr.wrapping_add(1), (value >> 8) as u8, cycles)
    }

    /// Read a 32-bit word / 读取 32 位字
    ///
    /// # Errors
    /// Returns `BusError` if the address is invalid or the access fails.
    /// 地址无效或访问失败时返回 `BusError`。
    fn read_u32(&self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        let lo = u32::from(self.read_u16(addr, cycles)?);
        let hi = u32::from(self.read_u16(addr.wrapping_add(2), cycles)?);
        Ok(lo | (hi << 16))
    }

    /// Write a 32-bit word / 写入 32 位字
    ///
    /// # Errors
    /// Returns `BusError` if the address is invalid or the access is denied.
    /// 地址无效或访问被拒绝时返回 `BusError`。
    fn write_u32(&self, addr: Address, value: u32, cycles: &mut u64) -> BusResult<()> {
        self.write_u16(addr, value as u16, cycles)?;
        self.write_u16(addr.wrapping_add(2), (value >> 16) as u16, cycles)
    }

    /// Host-side byte write that bypasses guest write protection
    /// 宿主侧字节写入，绕过客户机写保护
    ///
    /// # Errors
    /// Returns `BusError` if the address is invalid or the access is denied.
    /// 地址无效或访问被拒绝时返回 `BusError`。
    fn load_u8(&self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        self.write_u8(addr, value, cycles)
    }

    /// Host-side bulk write into a region / 宿主侧批量写入区域
    ///
    /// # Errors
    /// Returns `BusError` if the address is invalid or the access is denied.
    /// 地址无效或访问被拒绝时返回 `BusError`。
    fn load_bytes(&self, addr: Address, bytes: &[u8], cycles: &mut u64) -> BusResult<()> {
        for (offset, byte) in bytes.iter().enumerate() {
            self.load_u8(addr.wrapping_add(offset as u32), *byte, cycles)?;
        }
        Ok(())
    }
}

/// A bus that routes accesses to [`SharedRegion`]s through `&self`
/// 经 `&self` 把访问路由到 [`SharedRegion`] 的总线
///
/// The concurrent counterpart of [`AddressBus`](crate::bus::AddressBus). Only
/// routing and the PPB epoch are required; the accessors default to the same
/// stall gate and error semantics as the exclusive bus.
/// [`AddressBus`](crate::bus::AddressBus) 的并发版本。只需实现路由与 PPB 世代，
/// 访问器默认提供与独占总线相同的 stall 门控与错误语义。
pub trait SharedAddressBus: Send + Sync {
    /// Find the region that serves `addr` / 查找处理 `addr` 的区域
    fn find_region(&self, addr: Address) -> Option<&dyn SharedRegion>;

    /// Number of guest writes into the PPB window since construction
    /// 自构造以来客户机对 PPB 窗口的写入次数
    ///
    /// Monotonic and only advanced for PPB addresses, exactly like
    /// [`SimpleBus::ppb_write_epoch`](crate::memory::SimpleBus::ppb_write_epoch),
    /// so a frontend can skip the system-register drain while it is unchanged.
    /// 单调递增且只在 PPB 地址上推进，语义与
    /// [`SimpleBus::ppb_write_epoch`](crate::memory::SimpleBus::ppb_write_epoch)
    /// 相同，前端可据此在计数未变时跳过系统寄存器回放。
    fn ppb_write_epoch(&self) -> u64;

    /// Resolve `addr` to a region that can serve a read right now
    /// 把 `addr` 解析到当前可服务读取的区域
    ///
    /// # Errors
    /// Returns `NoDevice` when nothing is mapped and `Stall` for a blocked read.
    /// 未映射返回 `NoDevice`，阻塞读返回 `Stall`。
    fn resolve_readable(&self, addr: Address) -> BusResult<&dyn SharedRegion> {
        let region = self.find_region(addr).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::NoDevice,
        })?;
        if region.read_would_stall(addr) {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::Stall,
            });
        }
        Ok(region)
    }

    /// Read a single byte / 读取一个字节
    ///
    /// # Errors
    /// Returns `BusError` when nothing is mapped or the read stalls.
    /// 未映射或读取 stall 时返回 `BusError`。
    fn read_u8(&self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        self.resolve_readable(addr)?.read_u8(addr, cycles)
    }

    /// Read a 16-bit halfword / 读取 16 位半字
    ///
    /// # Errors
    /// Returns `BusError` when nothing is mapped or the read stalls.
    /// 未映射或读取 stall 时返回 `BusError`。
    fn read_u16(&self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        self.resolve_readable(addr)?.read_u16(addr, cycles)
    }

    /// Read a 32-bit word / 读取 32 位字
    ///
    /// # Errors
    /// Returns `BusError` when nothing is mapped or the read stalls.
    /// 未映射或读取 stall 时返回 `BusError`。
    fn read_u32(&self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        self.resolve_readable(addr)?.read_u32(addr, cycles)
    }

    /// Write a single byte / 写入一个字节
    ///
    /// # Errors
    /// Returns `BusError` when nothing is mapped.
    /// 未映射时返回 `BusError`。
    fn write_u8(&self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        self.find_region(addr)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .write_u8(addr, value, cycles)
    }

    /// Write a 16-bit halfword / 写入 16 位半字
    ///
    /// # Errors
    /// Returns `BusError` when nothing is mapped.
    /// 未映射时返回 `BusError`。
    fn write_u16(&self, addr: Address, value: u16, cycles: &mut u64) -> BusResult<()> {
        self.find_region(addr)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .write_u16(addr, value, cycles)
    }

    /// Write a 32-bit word / 写入 32 位字
    ///
    /// # Errors
    /// Returns `BusError` when nothing is mapped.
    /// 未映射时返回 `BusError`。
    fn write_u32(&self, addr: Address, value: u32, cycles: &mut u64) -> BusResult<()> {
        crate::watch::check_write(addr, value);
        self.find_region(addr)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .write_u32(addr, value, cycles)
    }

    /// Host-side byte write / 宿主侧字节写入
    ///
    /// # Errors
    /// Returns `BusError` when nothing is mapped.
    /// 未映射时返回 `BusError`。
    fn load_u8(&self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        self.find_region(addr)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .load_u8(addr, value, cycles)
    }

    /// Host-side bulk write / 宿主侧批量写入
    ///
    /// # Errors
    /// Returns `BusError` when nothing is mapped or the region rejects a byte.
    /// 未映射或区域拒绝某个字节时返回 `BusError`。
    fn load_bytes(&self, addr: Address, bytes: &[u8], cycles: &mut u64) -> BusResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.find_region(addr)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .load_bytes(addr, bytes, cycles)
    }
}

/// A thread-safe region built from an exclusive ([`MemoryRegion`]) one
/// 由独占（[`MemoryRegion`]）区域构建的线程安全区域
///
/// Migration bridge: regions that have not been converted to [`SharedRegion`]
/// yet are wrapped in exactly one mutex, so a [`SharedBus`] can host them
/// without a global bus lock. The window and name are captured at wrap time
/// (a region's window never changes after construction). Width accessors forward
/// to the region's own wide implementations, so per-region cycle, watchpoint and
/// error semantics are unchanged.
/// 迁移桥：尚未转换为 [`SharedRegion`] 的区域被包进恰好一把互斥锁，因此
/// [`SharedBus`] 可以承载它们而不引入全局总线锁。窗口与名称在包装时捕获（区域
/// 构造后窗口不再变化）。宽访问器转发到区域自身的宽实现，因此每区域的周期、
/// 监视点与错误语义保持不变。
pub struct LockedRegion {
    name: String,
    base: Address,
    size: usize,
    inner: Mutex<Box<dyn MemoryRegion>>,
}

impl LockedRegion {
    /// Wrap an exclusive region in one mutex / 用一把互斥锁包装独占区域
    #[must_use]
    pub fn new(inner: Box<dyn MemoryRegion>) -> Self {
        let name = inner.name().to_owned();
        let base = inner.base();
        let size = inner.size();
        Self {
            name,
            base,
            size,
            inner: Mutex::new(inner),
        }
    }

    /// Borrow the wrapped region exclusively / 独占借用被包装的区域
    ///
    /// Useful for host-side setup before the bus is shared; taking the mutex
    /// directly from a running simulation would stall a core thread.
    /// 适用于总线被共享之前的宿主侧配置；在运行中的模拟里直接取锁会让核线程等待。
    pub fn get_mut(&mut self) -> &mut dyn MemoryRegion {
        self.inner.get_mut().as_mut()
    }
}

impl SharedRegion for LockedRegion {
    fn name(&self) -> &str {
        &self.name
    }

    fn base(&self) -> Address {
        self.base
    }

    fn size(&self) -> usize {
        self.size
    }

    fn read_would_stall(&self, addr: Address) -> bool {
        self.inner.lock().read_would_stall(addr)
    }

    fn read_u8(&self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        self.inner.lock().read_u8(addr, cycles)
    }

    fn write_u8(&self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        self.inner.lock().write_u8(addr, value, cycles)
    }

    fn read_u16(&self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        self.inner.lock().read_u16(addr, cycles)
    }

    fn write_u16(&self, addr: Address, value: u16, cycles: &mut u64) -> BusResult<()> {
        self.inner.lock().write_u16(addr, value, cycles)
    }

    fn read_u32(&self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        self.inner.lock().read_u32(addr, cycles)
    }

    fn write_u32(&self, addr: Address, value: u32, cycles: &mut u64) -> BusResult<()> {
        self.inner.lock().write_u32(addr, value, cycles)
    }

    fn load_u8(&self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        self.inner.lock().load_u8(addr, value, cycles)
    }

    fn load_bytes(&self, addr: Address, bytes: &[u8], cycles: &mut u64) -> BusResult<()> {
        self.inner.lock().load_bytes(addr, bytes, cycles)
    }
}

/// A bus that can be shared by several core threads through [`Arc`]
/// 可经 [`Arc`] 被多个核线程共享的总线
///
/// Routing is the same direct-mapped table as
/// [`SimpleBus`](crate::memory::SimpleBus): one bucket per 16 MiB of address
/// space holding the index of the last region that served that bucket. The tags
/// are atomics and every hit is validated with [`SharedRegion::contains`], so a
/// racing fill (or a stale tag after a remount) can only force a scan; it can
/// never send an access to the wrong region. Mounting and unmounting take
/// `&mut self`, i.e. they happen before the bus is put behind an `Arc`.
/// 路由表与 [`SimpleBus`](crate::memory::SimpleBus) 相同的直接映射：每 16 MiB
/// 地址空间一个桶，保存最近服务该桶的区域下标。标签是原子量，每次命中都用
/// [`SharedRegion::contains`] 校验，因此竞争中的填充（或重挂载后的过期标签）最多
/// 迫使一次扫描，绝不会把访问送到错误的区域。挂载/卸载取 `&mut self`，即发生在
/// 总线被放进 `Arc` 之前。
pub struct SharedBus {
    regions: Vec<Arc<dyn SharedRegion>>,
    buckets: [AtomicU16; REGION_BUCKETS],
    ppb_write_epoch: AtomicU64,
}

impl SharedBus {
    /// Create an empty bus / 创建一个空总线
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of registered regions / 已注册区域数量
    #[must_use]
    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    /// Register a shared region, rejecting overlaps
    /// 注册一个共享区域，重叠会被拒绝
    ///
    /// # Errors
    /// Returns `Overlap` when the window intersects an existing region.
    /// 窗口与已有区域相交时返回 `Overlap`。
    pub fn register_region(&mut self, region: Arc<dyn SharedRegion>) -> BusResult<()> {
        let overlaps = self
            .regions
            .iter()
            .any(|existing| region.base() < existing.end() && existing.base() < region.end());
        if overlaps {
            return Err(CoreError::BusError {
                addr: region.base(),
                kind: BusErrorKind::Overlap,
            });
        }
        self.regions.push(region);
        self.clear_buckets();
        Ok(())
    }

    /// Register an exclusive region behind one mutex
    /// 用一把互斥锁注册独占区域
    ///
    /// # Errors
    /// Returns `Overlap` when the window intersects an existing region.
    /// 窗口与已有区域相交时返回 `Overlap`。
    pub fn register_locked(&mut self, region: Box<dyn MemoryRegion>) -> BusResult<()> {
        let wrapped: Arc<dyn SharedRegion> = Arc::new(LockedRegion::new(region));
        self.register_region(wrapped)
    }

    /// Remove a region by name / 按名称移除区域
    pub fn unregister_region(&mut self, name: &str) {
        self.regions.retain(|region| region.name() != name);
        self.clear_buckets();
    }

    /// Drop every cached route after the region list changes
    /// 区域列表变化后丢弃全部缓存路由
    fn clear_buckets(&mut self) {
        for bucket in &self.buckets {
            bucket.store(0, Ordering::Relaxed);
        }
    }

    /// Resolve `addr` to a region index, filling the bucket on a scan hit
    /// 把 `addr` 解析为区域下标；扫描命中时填充桶
    #[inline]
    fn resolve(&self, addr: Address) -> Option<usize> {
        let bucket = (addr >> 24) as usize;
        let tag = self.buckets[bucket].load(Ordering::Relaxed);
        if tag != 0 {
            let index = usize::from(tag - 1);
            // 命中仍需校验，过期标签只会导致扫描，不会错误路由。
            // A hit is still validated: a stale tag forces a scan, never a
            // wrong route.
            if self.regions.get(index).is_some_and(|r| r.contains(addr)) {
                return Some(index);
            }
        }
        let index = self
            .regions
            .iter()
            .position(|region| region.contains(addr))?;
        if let Ok(tag) = u16::try_from(index + 1) {
            self.buckets[bucket].store(tag, Ordering::Relaxed);
        }
        Some(index)
    }

    /// Count a write that lands in the PPB window / 统计落入 PPB 窗口的写
    #[inline]
    fn note_write(&self, addr: Address) {
        if addr.wrapping_sub(PPB_WINDOW_BASE) < PPB_WINDOW_SIZE {
            self.ppb_write_epoch.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Default for SharedBus {
    fn default() -> Self {
        Self {
            regions: Vec::new(),
            buckets: [const { AtomicU16::new(0) }; REGION_BUCKETS],
            ppb_write_epoch: AtomicU64::new(0),
        }
    }
}

impl SharedAddressBus for SharedBus {
    fn find_region(&self, addr: Address) -> Option<&dyn SharedRegion> {
        self.resolve(addr).map(|index| self.regions[index].as_ref())
    }

    fn ppb_write_epoch(&self) -> u64 {
        self.ppb_write_epoch.load(Ordering::Relaxed)
    }

    fn write_u8(&self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        self.note_write(addr);
        SharedAddressBus::find_region(self, addr)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .write_u8(addr, value, cycles)
    }

    fn write_u16(&self, addr: Address, value: u16, cycles: &mut u64) -> BusResult<()> {
        self.note_write(addr);
        SharedAddressBus::find_region(self, addr)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .write_u16(addr, value, cycles)
    }

    fn write_u32(&self, addr: Address, value: u32, cycles: &mut u64) -> BusResult<()> {
        crate::watch::check_write(addr, value);
        self.note_write(addr);
        SharedAddressBus::find_region(self, addr)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .write_u32(addr, value, cycles)
    }

    fn load_u8(&self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        self.note_write(addr);
        SharedAddressBus::find_region(self, addr)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .load_u8(addr, value, cycles)
    }

    fn load_bytes(&self, addr: Address, bytes: &[u8], cycles: &mut u64) -> BusResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        // 与逐字节路径一致：只有跨度可能与 PPB 窗口相交时才逐字节记账。
        // Same as the byte path: count per byte only when the span can touch
        // the PPB window.
        let start = u64::from(addr);
        let end = start + bytes.len() as u64;
        if start < u64::from(PPB_WINDOW_BASE + PPB_WINDOW_SIZE) && end > u64::from(PPB_WINDOW_BASE)
        {
            for offset in 0..bytes.len() {
                self.note_write(addr.wrapping_add(offset as u32));
            }
        }
        self.find_region(addr)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .load_bytes(addr, bytes, cycles)
    }
}

#[cfg(test)]
mod concurrency;
#[cfg(test)]
mod tests;
