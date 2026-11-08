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

/// Base address of the RP2350 flash XIP window / RP2350 Flash XIP 窗口基地址
pub const FLASH_XIP_BASE: Address = 0x1000_0000;

/// Size of the RP2350 flash XIP window (16 MB) / RP2350 Flash XIP 窗口大小（16 MB）
pub const FLASH_XIP_SIZE: usize = 16 * 1024 * 1024;

/// A flash XIP region that is read-only to the guest and host-programmable
/// 对客户机只读、对宿主可编程的 Flash XIP 区域
///
/// Guest writes return [`BusErrorKind::AccessDenied`]. Flash programming through
/// the QMI/XIP controller is not modelled in Stage 4; the loader is the only
/// writer.
/// 客户机写入返回 [`BusErrorKind::AccessDenied`]。Stage 4 不模拟经由 QMI/XIP
/// 控制器的 Flash 编程，只有加载器可以写入。
#[derive(Debug, Clone)]
pub struct FlashRegion {
    name: String,
    base: Address,
    data: Vec<u8>,
}

impl FlashRegion {
    /// Create a zeroed flash region / 创建一个清零的 Flash 区域
    ///
    /// The caller must ensure `base + size` stays within the 32-bit address space.
    /// 调用者需确保 `base + size` 不超出 32 位地址空间。
    #[must_use]
    pub fn new(name: impl Into<String>, base: Address, size: usize) -> Self {
        Self {
            name: name.into(),
            base,
            data: vec![0; size],
        }
    }

    /// Create the standard 16 MB RP2350 XIP window / 创建标准的 16 MB RP2350 XIP 窗口
    #[must_use]
    pub fn rp2350_xip() -> Self {
        Self::new("flash-xip", FLASH_XIP_BASE, FLASH_XIP_SIZE)
    }

    /// Borrow the backing bytes / 借用底层字节
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    /// Read a slice of flash bytes without going through the bus
    /// 不经过总线读取一段 Flash 字节
    ///
    /// Returns `None` when the requested range leaves the region.
    /// 请求范围超出区域时返回 `None`。
    #[must_use]
    pub fn slice(&self, addr: Address, len: usize) -> Option<&[u8]> {
        let offset = usize::try_from(addr.checked_sub(self.base)?).ok()?;
        self.data.get(offset..offset.checked_add(len)?)
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
        let slot = self.data.get_mut(offset..end).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })?;
        slot.copy_from_slice(bytes);
        Ok(())
    }

    /// Translate an in-region address into a buffer offset / 将区域内地址转换为缓冲区偏移
    fn offset(&self, addr: Address) -> BusResult<usize> {
        if !self.contains(addr) {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            });
        }
        Ok((addr - self.base) as usize)
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
        self.data.len()
    }

    fn read_u8(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        *cycles += 1;
        let offset = self.offset(addr)?;
        self.data.get(offset).copied().ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })
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
        let slot = self.data.get_mut(offset).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })?;
        *slot = value;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::AddressBus;
    use crate::memory::SimpleBus;

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
        assert_eq!(flash.slice(0x1000_0010, 3), Some(&[1u8, 2, 3][..]));
        assert!(flash.load_bytes(0x1000_003F, &[1, 2]).is_err());
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
