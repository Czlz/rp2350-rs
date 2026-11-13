//! Simple RAM region and flat address bus used by tests and the boot path
//! 供测试与启动路径使用的简易 RAM 区域和扁平地址总线

// 内存区域实现涉及 u32/usize 之间的地址换算，这是硬件仿真层的固有特性。
// The memory region implementation converts between u32 and usize for address
// arithmetic, which is inherent to the hardware emulation layer.
#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap
)]

use super::bus::{AddressBus, BusResult, MemoryRegion};
use super::error::{BusErrorKind, CoreError};
use super::types::Address;

/// A flat, byte-addressable RAM region backed by a heap buffer
/// 由堆缓冲区支持的扁平、字节可寻址 RAM 区域
#[derive(Debug, Clone)]
pub struct RamRegion {
    name: String,
    base: Address,
    writable: bool,
    data: Vec<u8>,
}

impl RamRegion {
    /// Create a zeroed, writable RAM region
    /// 创建一个清零的可写 RAM 区域
    ///
    /// The caller must ensure `base + size` stays within the 32-bit address space.
    /// 调用者需确保 `base + size` 不超出 32 位地址空间。
    #[must_use]
    pub fn new(name: impl Into<String>, base: Address, size: usize) -> Self {
        Self {
            name: name.into(),
            base,
            writable: true,
            data: vec![0; size],
        }
    }

    /// Create a zeroed, read-only region (used to model ROM)
    /// 创建一个清零的只读区域（用于模拟 ROM）
    ///
    /// The caller must ensure `base + size` stays within the 32-bit address space.
    /// 调用者需确保 `base + size` 不超出 32 位地址空间。
    #[must_use]
    pub fn new_rom(name: impl Into<String>, base: Address, size: usize) -> Self {
        Self {
            name: name.into(),
            base,
            writable: false,
            data: vec![0; size],
        }
    }

    /// Borrow the backing bytes / 借用底层字节
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    /// Readable/writable flag / 是否可写
    #[must_use]
    pub fn is_writable(&self) -> bool {
        self.writable
    }

    /// Translate an in-region address into a buffer offset
    /// 将区域内地址转换为缓冲区偏移
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

impl MemoryRegion for RamRegion {
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

    fn write_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        *cycles += 1;
        crate::watch::check_write(addr, u32::from(value));
        if !self.writable {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::AccessDenied,
            });
        }
        let offset = self.offset(addr)?;
        let slot = self.data.get_mut(offset).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })?;
        *slot = value;
        Ok(())
    }
}

/// A flat address bus that routes accesses to registered regions in order
/// 按注册顺序将访问路由到各区域的扁平地址总线
#[derive(Default)]
pub struct SimpleBus {
    regions: Vec<Box<dyn MemoryRegion>>,
}

impl SimpleBus {
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
}

impl AddressBus for SimpleBus {
    fn register_region(&mut self, region: Box<dyn MemoryRegion>) -> BusResult<()> {
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
        Ok(())
    }

    fn unregister_region(&mut self, name: &str) {
        self.regions.retain(|region| region.name() != name);
    }

    fn find_region_mut(&mut self, addr: Address) -> Option<&mut dyn MemoryRegion> {
        for region in &mut self.regions {
            if region.contains(addr) {
                return Some(region.as_mut());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_round_trip_byte() {
        let mut ram = RamRegion::new("ram", 0x2000_0000, 16);
        let mut cycles = 0;
        ram.write_u8(0x2000_0003, 0xA5, &mut cycles).unwrap();
        assert_eq!(ram.read_u8(0x2000_0003, &mut cycles).unwrap(), 0xA5);
        assert_eq!(ram.bytes()[3], 0xA5);
    }

    #[test]
    fn ram_word_round_trip_uses_byte_accessors() {
        let mut ram = RamRegion::new("ram", 0x2000_0000, 16);
        let mut cycles = 0;
        ram.write_u32(0x2000_0004, 0xDEAD_BEEF, &mut cycles)
            .unwrap();
        assert_eq!(ram.read_u32(0x2000_0004, &mut cycles).unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn ram_rejects_out_of_range() {
        let mut ram = RamRegion::new("ram", 0x2000_0000, 16);
        let mut cycles = 0;
        assert!(ram.read_u8(0x2000_0010, &mut cycles).is_err());
    }

    #[test]
    fn rom_rejects_writes() {
        let mut rom = RamRegion::new_rom("rom", 0x0000_0000, 16);
        let mut cycles = 0;
        let err = rom.write_u8(0x0000_0000, 1, &mut cycles).unwrap_err();
        match err {
            CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::AccessDenied),
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(!rom.is_writable());
    }

    #[test]
    fn bus_routes_to_region() {
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(RamRegion::new("ram", 0x2000_0000, 64)))
            .unwrap();
        let mut cycles = 0;
        bus.write_u32(0x2000_0000, 0x1234_5678, &mut cycles)
            .unwrap();
        assert_eq!(bus.read_u32(0x2000_0000, &mut cycles).unwrap(), 0x1234_5678);
    }

    #[test]
    fn bus_reports_unmapped_address() {
        let mut bus = SimpleBus::new();
        let mut cycles = 0;
        let err = bus.read_u8(0x4000_0000, &mut cycles).unwrap_err();
        match err {
            CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::NoDevice),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn bus_rejects_overlapping_regions() {
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(RamRegion::new("a", 0x2000_0000, 64)))
            .unwrap();
        let err = bus
            .register_region(Box::new(RamRegion::new("b", 0x2000_0010, 64)))
            .unwrap_err();
        match err {
            CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::Overlap),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn bus_unregister_by_name() {
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(RamRegion::new("ram", 0x2000_0000, 64)))
            .unwrap();
        assert_eq!(bus.region_count(), 1);
        bus.unregister_region("ram");
        assert_eq!(bus.region_count(), 0);
    }
}
