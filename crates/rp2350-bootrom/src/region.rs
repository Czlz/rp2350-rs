//! Read-only ROM memory region / 只读 ROM 内存区域

// ROM 区域实现涉及 u32/usize 地址换算，这是硬件仿真层的固有特性。
// The ROM region converts between u32 and usize for address arithmetic, which is
// inherent to the hardware emulation layer.
#![allow(clippy::cast_possible_truncation)]

use std::sync::Arc;

use rp2350_core::bus::{BusResult, MemoryRegion};
use rp2350_core::error::{BusErrorKind, CoreError};
use rp2350_core::types::Address;

/// A read-only region presenting prebuilt ROM bytes to the guest
/// 向客户机呈现预构建 ROM 字节的只读区域
///
/// Guest and host writes are both rejected; a boot ROM never changes at runtime.
/// 客户机与宿主写入都会被拒绝；boot ROM 在运行时不会改变。
#[derive(Debug, Clone)]
pub struct RomRegion {
    name: String,
    base: Address,
    data: Arc<Vec<u8>>,
}

impl RomRegion {
    /// Create a region over shared image bytes / 在共享镜像字节上创建区域
    #[must_use]
    pub fn new(name: impl Into<String>, base: Address, data: Arc<Vec<u8>>) -> Self {
        Self {
            name: name.into(),
            base,
            data,
        }
    }

    /// Borrow the image bytes / 借用镜像字节
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.data
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

impl MemoryRegion for RomRegion {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_bytes_and_rejects_writes() {
        let data = Arc::new(vec![0xAA, 0xBB, 0xCC, 0xDD]);
        let mut rom = RomRegion::new("rom", 0, data);
        let mut cycles = 0;

        assert_eq!(rom.read_u16(0, &mut cycles).unwrap(), 0xBBAA);
        assert!(rom.write_u8(0, 0, &mut cycles).is_err());
        assert!(rom.load_u8(0, 0, &mut cycles).is_err());
    }
}
