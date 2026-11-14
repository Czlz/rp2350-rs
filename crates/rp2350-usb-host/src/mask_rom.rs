//! Mask-ROM write-drop view needed by the USB EP0 compatibility path
//! USB EP0 兼容路径所需的掩膜 ROM 写入丢弃视图
//!
//! The RP2350 boot ROM window at `0x0000_0000` is mask ROM: real hardware
//! silently ignores writes to it. TinyUSB's RP2040 port relies on that: EP0 has
//! no endpoint-control register, so `hw_endpoint_init()` stores `NULL` in
//! `hw_endpoint.endpoint_control` and `hw_endpoint_start_next_buffer()` still
//! writes `*ep->endpoint_control` on every control transfer. Against a region
//! that rejects writes, that store becomes a bus fault at address 0 and the
//! guest halts during the first SETUP.
//! RP2350 位于 `0x0000_0000` 的 boot ROM 窗口是掩膜 ROM：真实硬件会静默忽略写入。
//! TinyUSB 的 RP2040 端口依赖这一行为：EP0 没有端点控制寄存器，因此
//! `hw_endpoint_init()` 把 `hw_endpoint.endpoint_control` 设为 `NULL`，而
//! `hw_endpoint_start_next_buffer()` 仍会在每次控制传输中写 `*ep->endpoint_control`。
//! 如果该窗口拒绝写入，这次存储就会在地址 0 产生总线错误，客户机在首个 SETUP
//! 阶段直接停机。
//!
//! [`install`] replaces the boot ROM region with a read-through view whose write
//! path is dropped, so guest reads still see the ROM image while the EP0 store
//! behaves like mask ROM. The `[repl]` command installs it before enumerating.
//! [`install`] 用只读透传、写入丢弃的视图替换 boot ROM 区域：客户机读取仍得到
//! ROM 镜像内容，而 EP0 的存储表现与掩膜 ROM 一致。`[repl]` 在枚举前安装它。

use std::sync::Arc;

use rp2350_core::bus::{AddressBus, BusResult, MemoryRegion};
use rp2350_core::error::{BusErrorKind, CoreError};
use rp2350_core::types::Address;

/// Name of the boot ROM region mounted by `rp2350-bootrom`
/// `rp2350-bootrom` 挂载的 boot ROM 区域名称
pub const BOOTROM_REGION_NAME: &str = "bootrom";

/// Base address of the mask ROM window / 掩膜 ROM 窗口基地址
pub const MASK_ROM_BASE: Address = 0x0000_0000;

/// Size of the RP2350 mask ROM window / RP2350 掩膜 ROM 窗口大小
pub const MASK_ROM_SIZE: usize = 0x8000;

/// Read-through, write-dropping view over a copy of the boot ROM image
/// 对 boot ROM 镜像副本的只读透传、写入丢弃视图
///
/// Reads behave exactly like the original ROM; guest writes are accepted and
/// discarded, matching RP2350 mask ROM. Host-side [`MemoryRegion::load_u8`]
/// also forwards to the dropping path because the ROM image never changes.
/// 读取与原 ROM 完全一致；客户机写入被接受并丢弃，符合 RP2350 掩膜 ROM 行为。
/// 由于 ROM 镜像运行时不变，宿主侧 [`MemoryRegion::load_u8`] 也走丢弃路径。
#[derive(Debug, Clone)]
pub struct MaskRom {
    name: String,
    base: Address,
    data: Arc<Vec<u8>>,
}

impl MaskRom {
    /// Create a view over `data` at `base` / 在 `base` 上创建 `data` 的视图
    #[must_use]
    pub fn new(name: impl Into<String>, base: Address, data: Arc<Vec<u8>>) -> Self {
        Self {
            name: name.into(),
            base,
            data,
        }
    }

    /// Borrow the captured ROM bytes / 借用捕获的 ROM 字节
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

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

impl MemoryRegion for MaskRom {
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

    /// Accept and discard guest writes inside the window, as mask ROM does
    /// 在窗口内像掩膜 ROM 一样接受并丢弃客户机写入
    fn write_u8(&mut self, addr: Address, _value: u8, cycles: &mut u64) -> BusResult<()> {
        self.offset(addr)?;
        *cycles += 1;
        Ok(())
    }
}

/// Replace the mounted boot ROM region with a write-dropping mask-ROM view
/// 把已挂载的 boot ROM 区域替换为写入丢弃的掩膜 ROM 视图
///
/// The current bytes are copied before the region is removed, so guest reads
/// and boot-ROM trap stubs keep working. Returns the captured window size.
/// 移除区域前先复制现有字节，因此客户机读取与 boot ROM 陷阱桩保持工作。返回
/// 捕获的窗口大小。
///
/// # Errors
/// Returns a bus error when a byte of the ROM window cannot be read or the
/// replacement region cannot be registered (for example, because something else
/// already covers `0x0000_0000..0x0000_8000`).
/// ROM 窗口任一字节读取失败，或替换区域无法注册（例如
/// `0x0000_0000..0x0000_8000` 已被其他区域占用）时返回总线错误。
pub fn install(bus: &mut dyn AddressBus) -> BusResult<usize> {
    let mut cycles = 0u64;
    let mut data = Vec::with_capacity(MASK_ROM_SIZE);
    for offset in 0..MASK_ROM_SIZE {
        let address = MASK_ROM_BASE + offset as Address;
        data.push(bus.read_u8(address, &mut cycles)?);
    }
    bus.unregister_region(BOOTROM_REGION_NAME);
    bus.register_region(Box::new(MaskRom::new(
        BOOTROM_REGION_NAME,
        MASK_ROM_BASE,
        Arc::new(data),
    )))?;
    Ok(MASK_ROM_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp2350_core::bus::MemoryRegion;
    use rp2350_core::memory::{RamRegion, SimpleBus};

    /// Mount a tiny ROM-like region and an SRAM block on a fresh bus
    /// 在新总线上挂载一个小型 ROM 区域与一块 SRAM
    fn bus_with_rom() -> SimpleBus {
        let mut bus = SimpleBus::new();
        let mut rom = RamRegion::new(BOOTROM_REGION_NAME, MASK_ROM_BASE, MASK_ROM_SIZE);
        let mut cycles = 0;
        for (offset, byte) in [0x10u8, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80]
            .into_iter()
            .enumerate()
        {
            rom.write_u8(offset as Address, byte, &mut cycles).unwrap();
        }
        bus.register_region(Box::new(rom)).unwrap();
        bus
    }

    #[test]
    fn reads_survive_the_remount() {
        let mut bus = bus_with_rom();
        let mut cycles = 0;
        assert_eq!(install(&mut bus).unwrap(), MASK_ROM_SIZE);
        assert_eq!(bus.read_u8(0, &mut cycles).unwrap(), 0x10);
        assert_eq!(bus.read_u8(7, &mut cycles).unwrap(), 0x80);
    }

    #[test]
    fn guest_writes_are_dropped_instead_of_faulting() {
        // Regression: TinyUSB writes `*ep->endpoint_control` (NULL for EP0) on
        // every control transfer; mask ROM ignores that store.
        // 回归：TinyUSB 每次控制传输都会写 `*ep->endpoint_control`（EP0 为
        // NULL）；掩膜 ROM 会忽略这次存储。
        let mut bus = bus_with_rom();
        install(&mut bus).unwrap();
        let mut cycles = 0;
        bus.write_u32(0, 0x2000_0000, &mut cycles).unwrap();
        assert_eq!(bus.read_u32(0, &mut cycles).unwrap(), 0x4030_2010);
    }

    #[test]
    fn install_reports_a_missing_rom_window() {
        let mut bus = SimpleBus::new();
        assert!(install(&mut bus).is_err());
    }

    #[test]
    fn view_rejects_addresses_outside_the_window() {
        let mut rom = MaskRom::new("rom", 0, Arc::new(vec![0xAA; 4]));
        let mut cycles = 0;
        assert!(rom.read_u8(4, &mut cycles).is_err());
        assert!(rom.write_u8(0x4000_0000, 0, &mut cycles).is_err());
    }
}
