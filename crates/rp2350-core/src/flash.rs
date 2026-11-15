//! Flash XIP memory region
//! Flash XIP 内存区域
//!
//! The RP2350 executes code directly from a 16 MB XIP window at `0x1000_0000`.
//! This region models that window as a byte array: the guest may read and
//! execute from it, but writes are rejected. Firmware loaders program it
//! through [`MemoryRegion::load_u8`], which bypasses the guest write policy.
//! RP2350 通过 `0x1000_0000` 处的 16 MB XIP 窗口直接执行代码。本区域用字节
//! 数组模拟该窗口：客户机可读取与执行，但写入会被拒绝。固件加载器通过
//! [`MemoryRegion::load_u8`] 编程该区域，从而绕过客户机写策略。

// Flash 区域实现涉及 u32/usize 之间的地址换算，这是硬件仿真层的固有特性。
// The flash region converts between u32 and usize for address arithmetic, which
// is inherent to the hardware emulation layer.
#![allow(clippy::cast_possible_truncation)]

use super::bus::{BusResult, MemoryRegion};
use super::error::{BusErrorKind, CoreError};
use super::types::Address;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Monotonic generation of host-side flash content changes
/// 宿主侧 Flash 内容变更的单调世代号
///
/// Guest writes to flash are rejected, so the only way the programmed bytes can
/// change is a host-side loader or debugger write through [`FlashRegion`] or
/// [`FlashHandle`]. Every such mutation bumps this counter, which lets a reader
/// cache flash bytes and revalidate them with one relaxed load instead of
/// taking the shared-store lock on every access.
/// 客户机对 Flash 的写会被拒绝，因此已编程字节只可能经由 [`FlashRegion`] 或
/// [`FlashHandle`] 的宿主侧加载器/调试器写入而改变。每次变更都会递增本计数器，
/// 读取方据此缓存 Flash 字节，并用一次 relaxed 读取重新验证，而不必每次访问都
/// 获取共享存储锁。
static CONTENT_GENERATION: AtomicU32 = AtomicU32::new(0);

/// Current flash content generation / 当前 Flash 内容世代号
#[must_use]
pub fn content_generation() -> u32 {
    CONTENT_GENERATION.load(Ordering::Relaxed)
}

/// Publish a host-side content change / 发布一次宿主侧内容变更
fn bump_content_generation() {
    CONTENT_GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Base address of the RP2350 flash XIP window / RP2350 Flash XIP 窗口基地址
pub const FLASH_XIP_BASE: Address = 0x1000_0000;

/// Base address of the non-cached flash XIP alias (pico-sdk `XIP_NOCACHE_NOALLOC_BASE`)
/// 非缓存 Flash XIP 别名基地址（pico-sdk `XIP_NOCACHE_NOALLOC_BASE`）
pub const FLASH_XIP_NOCACHE_BASE: Address = 0x1400_0000;

/// Size of the RP2350 flash XIP window (16 MB) / RP2350 Flash XIP 窗口大小（16 MB）
pub const FLASH_XIP_SIZE: usize = 16 * 1024 * 1024;

/// A flash XIP region that is read-only to the guest and host-programmable
/// 对客户机只读、对宿主可编程的 Flash XIP 区域
///
/// Guest writes return [`BusErrorKind::AccessDenied`]. Flash programming through
/// the QMI/XIP controller is not modelled in Stage 4; the loader is the only
/// writer. Clones and [`FlashRegion::alias`] windows share one backing store.
/// 客户机写入返回 [`BusErrorKind::AccessDenied`]。Stage 4 不模拟经由 QMI/XIP
/// 控制器的 Flash 编程，只有加载器可以写入。克隆与 [`FlashRegion::alias`]
/// 窗口共享同一份底层存储。
#[derive(Debug, Clone)]
pub struct FlashRegion {
    name: String,
    base: Address,
    size: usize,
    data: Arc<RwLock<Vec<u8>>>,
}

impl FlashRegion {
    /// Create a zeroed flash region / 创建一个清零的 Flash 区域
    ///
    /// The caller must ensure `base + size` stays within the 32-bit address space.
    /// 调用者需确保 `base + size` 不超出 32 位地址空间。
    #[must_use]
    pub fn new(name: impl Into<String>, base: Address, size: usize) -> Self {
        // 新建底层存储等同于内容变更：任何按地址缓存的 Flash 字节都必须失效。
        // A fresh backing store counts as a content change so any address-keyed
        // cache of flash bytes is dropped.
        bump_content_generation();
        Self {
            name: name.into(),
            base,
            size,
            data: Arc::new(RwLock::new(vec![0; size])),
        }
    }

    /// Create the standard 16 MB RP2350 XIP window / 创建标准的 16 MB RP2350 XIP 窗口
    #[must_use]
    pub fn rp2350_xip() -> Self {
        Self::new("flash-xip", FLASH_XIP_BASE, FLASH_XIP_SIZE)
    }

    /// Create another read-only window over the same flash bytes
    /// 在同一份 Flash 字节上创建另一个只读窗口
    ///
    /// RP2350 exposes the flash at `0x1000_0000` (cached) and at the
    /// `XIP_NOCACHE_NOALLOC_BASE` alias `0x1400_0000` (non-cached). Both windows
    /// must observe the same programmed content, so the alias shares the backing
    /// store rather than copying it.
    /// RP2350 在 `0x1000_0000`（缓存）与 `XIP_NOCACHE_NOALLOC_BASE` 别名
    /// `0x1400_0000`（非缓存）同时暴露 Flash。两个窗口必须看到相同的已编程
    /// 内容，因此别名共享底层存储而非复制。
    #[must_use]
    pub fn alias(&self, name: impl Into<String>, base: Address) -> Self {
        Self {
            name: name.into(),
            base,
            size: self.size,
            data: Arc::clone(&self.data),
        }
    }

    /// Borrow the backing bytes (copied out of the shared store)
    /// 借用底层字节（从共享存储复制一份）
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        self.read_store().clone()
    }

    /// Read a slice of flash bytes without going through the bus
    /// 不经过总线读取一段 Flash 字节
    ///
    /// Returns `None` when the requested range leaves the region.
    /// 请求范围超出区域时返回 `None`。
    #[must_use]
    pub fn slice(&self, addr: Address, len: usize) -> Option<Vec<u8>> {
        let offset = usize::try_from(addr.checked_sub(self.base)?).ok()?;
        let end = offset.checked_add(len)?;
        self.read_store().get(offset..end).map(<[u8]>::to_vec)
    }

    /// Create a host-only handle over the same backing store
    /// 在同一份底层存储上创建宿主专用句柄
    ///
    /// Bootrom emulation runs on the host but must erase and program exactly the
    /// bytes the guest reads through the XIP windows. The handle exposes that
    /// path without weakening [`MemoryRegion`]'s guest write rejection.
    /// bootrom 仿真运行在宿主侧，但必须擦除和编程客户机经 XIP 窗口读取的同一份
    /// 字节。该句柄提供这条路径，同时不削弱 [`MemoryRegion`] 对客户机写入的拒绝。
    #[must_use]
    pub fn handle(&self) -> FlashHandle {
        FlashHandle {
            base: self.base,
            data: Arc::clone(&self.data),
        }
    }

    /// Program bytes through the host-side loader path
    /// 通过宿主侧加载器路径编程字节
    ///
    /// # Errors
    /// Returns a bus error if the range leaves the region.
    /// 范围超出区域时返回总线错误。
    pub fn load_bytes(&mut self, addr: Address, bytes: &[u8]) -> BusResult<()> {
        let offset = self.offset(addr)?;
        let end = offset.checked_add(bytes.len()).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })?;
        let mut data = self.write_store();
        let slot = data.get_mut(offset..end).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })?;
        slot.copy_from_slice(bytes);
        drop(data);
        bump_content_generation();
        Ok(())
    }

    /// Translate an in-region address into a buffer offset / 将区域内地址转换为缓冲区偏移
    fn offset(&self, addr: Address) -> BusResult<usize> {
        if addr < self.base || addr.wrapping_sub(self.base) as usize >= self.size {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            });
        }
        Ok((addr - self.base) as usize)
    }

    /// Copy `N` bytes at `addr` into an array, or `None` when the span leaves
    /// the region
    /// 将 `addr` 处 `N` 字节复制进数组；跨度超出区域时返回 `None`
    ///
    /// One shared-store acquisition serves the whole access.
    /// 整次访问只获取一次共享存储。
    fn window<const N: usize>(&self, addr: Address) -> Option<[u8; N]> {
        let offset = self.offset(addr).ok()?;
        let end = offset.checked_add(N)?;
        if end > self.size {
            return None;
        }
        let data = self.read_store();
        let slot = data.get(offset..end)?;
        let mut out = [0u8; N];
        out.copy_from_slice(slot);
        Some(out)
    }

    /// Acquire the shared store for reading / 获取共享存储的读锁
    fn read_store(&self) -> RwLockReadGuard<'_, Vec<u8>> {
        self.data.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Acquire the shared store for writing / 获取共享存储的写锁
    fn write_store(&mut self) -> RwLockWriteGuard<'_, Vec<u8>> {
        self.data.write().unwrap_or_else(PoisonError::into_inner)
    }
}

impl MemoryRegion for FlashRegion {
    fn name(&self) -> &str {
        &self.name
    }

    fn base(&self) -> Address {
        self.base
    }

    fn size(&self) -> usize {
        // 长度在构造时固定且底层 `Vec` 从不扩容，因此无需加锁读取。
        // The length is fixed at construction and the backing `Vec` never
        // grows, so it can be read without taking the lock.
        self.size
    }

    fn read_u8(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        *cycles += 1;
        let offset = self.offset(addr)?;
        let data = self.read_store();
        data.get(offset).copied().ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })
    }

    /// A halfword fetch is one shared-store acquisition, not one per byte
    /// 半字取指只获取一次共享存储，而不是每字节一次
    ///
    /// Instruction fetch is the hottest bus operation, and every acquisition
    /// of the shared flash lock costs an interlocked read-modify-write on a
    /// cache line that the CPU otherwise never dirties. Reaching past the end
    /// of the region still falls back to the byte-wise default so per-byte
    /// error and cycle accounting is unchanged.
    /// 取指是最热的总线操作，而每次获取共享 Flash 锁都要在 CPU 平时不会写脏的
    /// 缓存行上做一次互锁读-改-写。跨出区域末尾时仍退回逐字节默认实现，保持逐
    /// 字节的错误与周期记账不变。
    fn read_u16(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        let Some(bytes) = self.window::<2>(addr) else {
            let lo = u16::from(self.read_u8(addr, cycles)?);
            let hi = u16::from(self.read_u8(addr.wrapping_add(1), cycles)?);
            return Ok(lo | (hi << 8));
        };
        *cycles += 2;
        Ok(u16::from_le_bytes(bytes))
    }

    /// A word read is one shared-store acquisition / 整字读取只获取一次共享存储
    ///
    /// See [`Self::read_u16`] for the out-of-range fallback.
    /// 越界回退语义见 [`Self::read_u16`]。
    fn read_u32(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        let Some(bytes) = self.window::<4>(addr) else {
            let lo = u32::from(self.read_u16(addr, cycles)?);
            let hi = u32::from(self.read_u16(addr.wrapping_add(2), cycles)?);
            return Ok(lo | (hi << 16));
        };
        *cycles += 4;
        Ok(u32::from_le_bytes(bytes))
    }

    fn write_u8(&mut self, addr: Address, _value: u8, _cycles: &mut u64) -> BusResult<()> {
        Err(CoreError::BusError {
            addr,
            kind: BusErrorKind::AccessDenied,
        })
    }

    fn load_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        *cycles += 1;
        let offset = self.offset(addr)?;
        let mut data = self.write_store();
        let slot = data.get_mut(offset).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })?;
        *slot = value;
        drop(data);
        bump_content_generation();
        Ok(())
    }
}

/// Host-only handle to the flash bytes shared by every XIP window
/// 由所有 XIP 窗口共享的 Flash 字节的宿主专用句柄
///
/// The guest reaches flash only through [`MemoryRegion`], which rejects writes.
/// Bootrom emulation runs on the host and uses this handle to erase and program
/// the same backing store that the guest reads.
/// 客户机只能通过 [`MemoryRegion`] 访问 Flash，而该路径拒绝写入。bootrom 仿真
/// 运行在宿主侧，通过本句柄擦除和编程客户机读取的同一份底层存储。
#[derive(Debug, Clone)]
pub struct FlashHandle {
    base: Address,
    data: Arc<RwLock<Vec<u8>>>,
}

impl FlashHandle {
    /// Base address of the window this handle was created from
    /// 创建本句柄时所用窗口的基地址
    #[must_use]
    pub fn base(&self) -> Address {
        self.base
    }

    /// Capacity of the backing store in bytes / 底层存储容量（字节）
    #[must_use]
    pub fn size(&self) -> usize {
        self.read_store().len()
    }

    /// Read a range of flash bytes / 读取一段 Flash 字节
    ///
    /// # Errors
    /// Returns a bus error if the range leaves the backing store.
    /// 范围超出底层存储时返回总线错误。
    pub fn read(&self, addr: Address, len: usize) -> BusResult<Vec<u8>> {
        let offset = self.offset(addr)?;
        let end = offset.checked_add(len).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })?;
        self.read_store()
            .get(offset..end)
            .map(<[u8]>::to_vec)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::DeviceError,
            })
    }

    /// Program a byte range / 编程一段字节
    ///
    /// # Errors
    /// Returns a bus error if the range leaves the backing store.
    /// 范围超出底层存储时返回总线错误。
    pub fn write(&self, addr: Address, bytes: &[u8]) -> BusResult<()> {
        let offset = self.offset(addr)?;
        let end = offset.checked_add(bytes.len()).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })?;
        let mut data = self.write_store();
        let slot = data.get_mut(offset..end).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })?;
        slot.copy_from_slice(bytes);
        drop(data);
        bump_content_generation();
        Ok(())
    }

    /// Erase a byte range to the NOR flash erased state (`0xFF`)
    /// 将一段字节擦除为 NOR Flash 的擦除态（`0xFF`）
    ///
    /// # Errors
    /// Returns a bus error if the range leaves the backing store.
    /// 范围超出底层存储时返回总线错误。
    pub fn erase(&self, addr: Address, len: usize) -> BusResult<()> {
        let offset = self.offset(addr)?;
        let end = offset.checked_add(len).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })?;
        let mut data = self.write_store();
        let slot = data.get_mut(offset..end).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })?;
        slot.fill(0xFF);
        drop(data);
        bump_content_generation();
        Ok(())
    }

    /// Translate an in-region address into a buffer offset / 将区域内地址转换为缓冲区偏移
    fn offset(&self, addr: Address) -> BusResult<usize> {
        if addr < self.base || addr - self.base >= self.size() as u32 {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            });
        }
        Ok((addr - self.base) as usize)
    }

    /// Acquire the shared store for reading / 获取共享存储的读锁
    fn read_store(&self) -> RwLockReadGuard<'_, Vec<u8>> {
        self.data.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Acquire the shared store for writing / 获取共享存储的写锁
    fn write_store(&self) -> RwLockWriteGuard<'_, Vec<u8>> {
        self.data.write().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::AddressBus;
    use crate::memory::SimpleBus;

    #[test]
    fn wide_reads_match_the_byte_path_and_book_the_same_cycles() {
        let mut flash = FlashRegion::new("flash", 0x1000_0000, 8);
        let mut cycles = 0;
        flash
            .load_bytes(0x1000_0000, &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66])
            .unwrap();
        assert_eq!(flash.read_u16(0x1000_0000, &mut cycles).unwrap(), 0x2211);
        assert_eq!(
            flash.read_u32(0x1000_0002, &mut cycles).unwrap(),
            0x6655_4433
        );
        assert_eq!(cycles, 6, "one cycle per byte, like the default path");
        // 末尾半字仍可读；越过末尾必须报 NoDevice。
        // The last halfword is readable; one byte past the end must report
        // NoDevice.
        assert_eq!(flash.read_u16(0x1000_0006, &mut cycles).unwrap(), 0);
        let err = flash.read_u16(0x1000_0007, &mut cycles).unwrap_err();
        match err {
            CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::NoDevice),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn size_and_alias_report_the_configured_length() {
        let flash = FlashRegion::new("flash", FLASH_XIP_BASE, 0x1000);
        assert_eq!(flash.size(), 0x1000);
        assert_eq!(flash.end(), FLASH_XIP_BASE + 0x1000);
        let alias = flash.alias("alias", FLASH_XIP_NOCACHE_BASE);
        assert_eq!(alias.size(), 0x1000);
        assert_eq!(alias.end(), FLASH_XIP_NOCACHE_BASE + 0x1000);
    }

    #[test]
    fn guest_can_read_but_not_write() {
        let mut flash = FlashRegion::new("flash", 0x1000_0000, 64);
        let mut cycles = 0;
        flash
            .load_u8(0x1000_0004, 0xA5, &mut cycles)
            .expect("host load should succeed");
        assert_eq!(flash.read_u8(0x1000_0004, &mut cycles).unwrap(), 0xA5);

        // 客户机写入必须被拒绝。
        // Guest writes must be rejected.
        assert!(flash.write_u8(0x1000_0004, 0x00, &mut cycles).is_err());
    }

    #[test]
    fn load_bytes_programs_a_range() {
        let mut flash = FlashRegion::new("flash", 0x1000_0000, 64);
        flash.load_bytes(0x1000_0010, &[1, 2, 3]).unwrap();
        assert_eq!(flash.slice(0x1000_0010, 3), Some(vec![1u8, 2, 3]));
        assert!(flash.load_bytes(0x1000_003F, &[1, 2]).is_err());
    }

    #[test]
    fn alias_shares_flash_bytes_and_rejects_guest_writes() {
        let mut flash = FlashRegion::new("flash", FLASH_XIP_BASE, 64);
        let mut alias = flash.alias("flash-nocache", FLASH_XIP_NOCACHE_BASE);
        let mut cycles = 0;

        flash
            .load_u8(FLASH_XIP_BASE + 8, 0x5A, &mut cycles)
            .unwrap();
        assert_eq!(
            alias
                .read_u8(FLASH_XIP_NOCACHE_BASE + 8, &mut cycles)
                .unwrap(),
            0x5A
        );
        assert!(
            alias
                .write_u8(FLASH_XIP_NOCACHE_BASE + 8, 0, &mut cycles)
                .is_err()
        );
    }

    #[test]
    fn handle_programs_every_window_and_erases_to_ff() {
        let mut flash = FlashRegion::new("flash", FLASH_XIP_BASE, 64);
        let mut alias = flash.alias("flash-nocache", FLASH_XIP_NOCACHE_BASE);
        let handle = flash.handle();
        let mut cycles = 0;

        handle.write(FLASH_XIP_BASE + 4, &[1, 2, 3]).unwrap();
        assert_eq!(
            alias
                .read_u8(FLASH_XIP_NOCACHE_BASE + 4, &mut cycles)
                .unwrap(),
            1
        );
        handle.erase(FLASH_XIP_BASE + 4, 3).unwrap();
        assert_eq!(
            flash.read_u8(FLASH_XIP_BASE + 4, &mut cycles).unwrap(),
            0xFF
        );
        assert_eq!(handle.read(FLASH_XIP_BASE + 4, 3).unwrap(), vec![0xFF; 3]);

        // 越界按 NoDevice 报错，客户机写入仍被拒绝。
        // Out-of-range reports NoDevice and guest writes stay rejected.
        assert!(handle.write(FLASH_XIP_BASE + 63, &[1, 2]).is_err());
        assert!(flash.write_u8(FLASH_XIP_BASE + 4, 0, &mut cycles).is_err());
    }

    #[test]
    fn bus_serves_flash_through_nocache_alias() {
        let mut bus = SimpleBus::new();
        let flash = FlashRegion::new("flash", FLASH_XIP_BASE, 64);
        let alias = flash.alias("flash-nocache", FLASH_XIP_NOCACHE_BASE);
        bus.register_region(Box::new(flash)).unwrap();
        bus.register_region(Box::new(alias)).unwrap();

        let mut cycles = 0;
        bus.load_u8(FLASH_XIP_BASE, 0x42, &mut cycles).unwrap();
        assert_eq!(
            bus.read_u8(FLASH_XIP_NOCACHE_BASE, &mut cycles).unwrap(),
            0x42
        );
    }

    #[test]
    fn bus_load_bypasses_write_protection() {
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(FlashRegion::rp2350_xip()))
            .unwrap();
        let mut cycles = 0;
        bus.load_u8(FLASH_XIP_BASE, 0x42, &mut cycles).unwrap();
        assert_eq!(bus.read_u8(FLASH_XIP_BASE, &mut cycles).unwrap(), 0x42);
        assert!(bus.write_u8(FLASH_XIP_BASE, 0x00, &mut cycles).is_err());
    }
}
