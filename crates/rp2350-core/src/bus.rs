//! Address bus trait and memory region abstraction
//! 地址总线 trait 和内存区域抽象

use super::types::Address;

use super::error::{BusErrorKind, CoreError};

/// Result type for bus operations
/// 总线操作的结果类型
pub type BusResult<T> = Result<T, CoreError>;

/// Memory access width
/// 内存访问宽度
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessWidth {
    /// 8-bit byte access / 8 位字节访问
    Byte,
    /// 16-bit halfword access / 16 位半字访问
    HalfWord,
    /// 32-bit word access / 32 位字访问
    Word,
}

/// A region of memory or MMIO that can be read and written
/// 可读写的内存或 MMIO 区域
pub trait MemoryRegion: Send + Sync {
    /// Region name for debugging / 区域名称（调试用）
    fn name(&self) -> &str;

    /// Start address (inclusive) / 起始地址（含）
    fn base(&self) -> Address;

    /// Region size in bytes / 区域大小（字节）
    fn size(&self) -> usize;

    /// End address (exclusive) / 结束地址（不含）
    fn end(&self) -> Address {
        self.base() + self.size() as u32
    }

    /// Check if `addr` falls within this region / 检查地址是否在此区域内
    fn contains(&self, addr: Address) -> bool {
        addr >= self.base() && addr < self.end()
    }

    /// Read a single byte / 读取一个字节
    fn read_u8(&mut self, addr: Address, _cycles: &mut u64) -> BusResult<u8>;

    /// Write a single byte / 写入一个字节
    fn write_u8(&mut self, addr: Address, value: u8, _cycles: &mut u64) -> BusResult<()>;

    /// Read a 16-bit halfword / 读取 16 位半字
    fn read_u16(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        let lo = self.read_u8(addr, cycles)? as u16;
        let hi = self.read_u8(addr + 1, cycles)? as u16;
        Ok(lo | (hi << 8))
    }

    /// Write a 16-bit halfword / 写入 16 位半字
    fn write_u16(&mut self, addr: Address, value: u16, cycles: &mut u64) -> BusResult<()> {
        self.write_u8(addr, (value & 0xFF) as u8, cycles)?;
        self.write_u8(addr + 1, ((value >> 8) & 0xFF) as u8, cycles)
    }

    /// Read a 32-bit word / 读取 32 位字
    fn read_u32(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        let lo = self.read_u16(addr, cycles)? as u32;
        let hi = self.read_u16(addr + 2, cycles)? as u32;
        Ok(lo | (hi << 16))
    }

    /// Write a 32-bit word / 写入 32 位字
    fn write_u32(&mut self, addr: Address, value: u32, cycles: &mut u64) -> BusResult<()> {
        self.write_u16(addr, (value & 0xFFFF) as u16, cycles)?;
        self.write_u16(addr + 2, ((value >> 16) & 0xFFFF) as u16, cycles)
    }
}

/// The central address bus that routes memory accesses to the correct region
/// 中央地址总线，将内存访问路由到正确区域
pub trait AddressBus: Send + Sync {
    /// Register a memory region / 注册一个内存区域
    fn register_region(&mut self, region: Box<dyn MemoryRegion>) -> BusResult<()>;

    /// Remove a region by name / 按名称移除区域
    fn unregister_region(&mut self, name: &str);

    /// Find which region handles this address / 查找处理此地址的区域
    fn find_region_mut(&mut self, addr: Address) -> Option<&mut dyn MemoryRegion>;

    /// Read from the bus (auto-routes to correct region) / 从总线读取（自动路由到正确区域）
    fn read_u8(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        self.find_region_mut(addr)
            .ok_or_else(|| CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .read_u8(addr, cycles)
    }

    /// Read a 16-bit halfword / 读取 16 位半字
    fn read_u16(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        self.find_region_mut(addr)
            .ok_or_else(|| CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .read_u16(addr, cycles)
    }

    /// Read a 32-bit word / 读取 32 位字
    fn read_u32(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        self.find_region_mut(addr)
            .ok_or_else(|| CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .read_u32(addr, cycles)
    }

    /// Write to the bus (auto-routes to correct region) / 写入总线（自动路由到正确区域）
    fn write_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        self.find_region_mut(addr)
            .ok_or_else(|| CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .write_u8(addr, value, cycles)
    }

    /// Write a 16-bit halfword / 写入 16 位半字
    fn write_u16(&mut self, addr: Address, value: u16, cycles: &mut u64) -> BusResult<()> {
        self.find_region_mut(addr)
            .ok_or_else(|| CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .write_u16(addr, value, cycles)
    }

    /// Write a 32-bit word / 写入 32 位字
    fn write_u32(&mut self, addr: Address, value: u32, cycles: &mut u64) -> BusResult<()> {
        self.find_region_mut(addr)
            .ok_or_else(|| CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .write_u32(addr, value, cycles)
    }
}