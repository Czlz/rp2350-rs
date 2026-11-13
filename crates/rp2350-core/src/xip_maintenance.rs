//! XIP maintenance window: a write-only cache-maintenance sink
//! XIP 维护窗口：只写的缓存维护汇聚点
//!
//! pico-sdk `addressmap.h` places `XIP_MAINTENANCE_BASE` at `0x1800_0000`, spanning
//! up to `XIP_NOCACHE_NOALLOC_NOTRANSLATE_BASE` (`0x1C00_0000`). A write into this
//! window asks the XIP cache to clean or invalidate the line covering the
//! corresponding XIP address; the written data is discarded. Firmware uses it for
//! cache maintenance, for example the `xip_cache_clean_all()` loop that
//! `flash_save_hardware_state()` runs before touching QMI.
//! pico-sdk `addressmap.h` 将 `XIP_MAINTENANCE_BASE` 定在 `0x1800_0000`，窗口延伸
//! 到 `XIP_NOCACHE_NOALLOC_NOTRANSLATE_BASE`（`0x1C00_0000`）。写入该窗口表示请求
//! XIP 缓存清理或失效对应 XIP 地址所在的缓存行，写入数据本身被丢弃。固件用它做
//! 缓存维护，例如 `flash_save_hardware_state()` 在操作 QMI 前执行的
//! `xip_cache_clean_all()` 循环。
//!
//! This emulator does not model the XIP cache, so accepted writes are no-ops and
//! reads return zero. Refusing these accesses would fault firmware that performs
//! legitimate maintenance, while storing the bytes would invent state the hardware
//! does not keep.
//! 本模拟器不建模 XIP 缓存，因此接受写入但视为空操作，读取返回 0。拒绝这些访问
//! 会让执行合法维护的固件故障；而把字节存下来又会凭空造出硬件并不保留的状态。

use super::bus::{BusResult, MemoryRegion};
use super::error::{BusErrorKind, CoreError};
use super::types::Address;

/// Base address of the XIP maintenance window / XIP 维护窗口基地址
pub const XIP_MAINTENANCE_BASE: Address = 0x1800_0000;

/// Size of the XIP maintenance window (64 MB) / XIP 维护窗口大小（64 MB）
pub const XIP_MAINTENANCE_SIZE: usize = 0x0400_0000;

/// A write-only cache-maintenance window / 只写的缓存维护窗口
///
/// Writes are accepted and discarded; reads return zero. The region keeps no
/// backing store because the hardware keeps none either.
/// 写入被接受并丢弃；读取返回 0。该区域不保留任何存储，因为硬件同样不保留。
#[derive(Debug, Clone)]
pub struct XipMaintenanceRegion {
    name: String,
}

impl XipMaintenanceRegion {
    /// Create a maintenance window / 创建维护窗口
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl MemoryRegion for XipMaintenanceRegion {
    fn name(&self) -> &str {
        &self.name
    }

    fn base(&self) -> Address {
        XIP_MAINTENANCE_BASE
    }

    fn size(&self) -> usize {
        XIP_MAINTENANCE_SIZE
    }

    fn read_u8(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        if !self.contains(addr) {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            });
        }
        *cycles += 1;
        Ok(0)
    }

    fn write_u8(&mut self, addr: Address, _value: u8, cycles: &mut u64) -> BusResult<()> {
        if !self.contains(addr) {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            });
        }
        *cycles += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::AddressBus;
    use crate::memory::SimpleBus;

    #[test]
    fn maintenance_write_is_accepted_and_reads_zero() {
        let mut window = XipMaintenanceRegion::new("xip-maintenance");
        let mut cycles = 0;
        window
            .write_u8(XIP_MAINTENANCE_BASE + 0x03FF_C000, 0, &mut cycles)
            .unwrap();
        assert_eq!(
            window
                .read_u8(XIP_MAINTENANCE_BASE + 0x03FF_C000, &mut cycles)
                .unwrap(),
            0
        );
        assert_eq!(cycles, 2);
    }

    #[test]
    fn route_is_mounted_between_the_aliases() {
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(XipMaintenanceRegion::new("xip-maintenance")))
            .unwrap();
        let mut cycles = 0;
        // 维护窗口顶端最后一个字（0x1BFF_C000..0x1C00_0000）必须可写。
        // The last word of the maintenance window (0x1BFF_C000..0x1C00_0000)
        // must accept writes.
        bus.write_u8(0x1BFF_C000, 0, &mut cycles).unwrap();
        assert!(bus.write_u8(0x1C00_0000, 0, &mut cycles).is_err());
    }
}
