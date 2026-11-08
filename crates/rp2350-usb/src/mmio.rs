//! `MmioDevice` adapters for the mainline MMIO framework
//! 面向主线 MMIO 框架的 `MmioDevice` 适配器
//!
//! The bus-facing [`crate::regions`] types implement `MemoryRegion` directly.
//! The mainline `rp2350_core::mmio` framework uses a smaller `MmioDevice`
//! contract; these adapters expose the same [`UsbCore`] through it, so a chip
//! build can mount USBCTRL with `mount_device` without depending on this crate
//! beyond the two windows.
//! 面向总线的 [`crate::regions`] 类型直接实现 `MemoryRegion`。主线
//! `rp2350_core::mmio` 框架使用更小的 `MmioDevice` 契约；这些适配器通过该契约
//! 暴露同一个 [`UsbCore`]，因此芯片构建可用 `mount_device` 挂载 USBCTRL，
//! 而无需了解本 crate 的窗口细节。

use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_core::BusResult;
use rp2350_core::bus::AddressBus;
use rp2350_core::mmio::{MmioDevice, MmioWindow, mount_device};

use crate::device::UsbCore;
use crate::regs::Alias;
use crate::{DPRAM_BASE, DPRAM_SIZE, REGS_BASE, REGS_REGION_SIZE, USBCTRL_IRQ};

/// Decode a register-window offset into a register offset and write alias
/// 将寄存器窗口偏移解码为寄存器偏移与写别名
///
/// Offsets `0x0000..0x0FFF` are plain RW, `0x1000..0x1FFF` XOR,
/// `0x2000..0x2FFF` SET and `0x3000..0x3FFF` CLR.
/// 偏移 `0x0000..0x0FFF` 为普通读写，`0x1000..0x1FFF` 为异或，
/// `0x2000..0x2FFF` 为置位，`0x3000..0x3FFF` 为清零。
#[must_use]
pub fn decode_alias(offset: u32) -> Option<(usize, Alias)> {
    let offset = offset as usize;
    if offset >= REGS_REGION_SIZE {
        return None;
    }
    Some((offset & 0xFFF, Alias::from_window((offset >> 12) as u32)))
}

/// DPSRAM window (`0x5010_0000`, 4 KB) as an [`MmioDevice`]
/// 作为 [`MmioDevice`] 的 DPSRAM 窗口（`0x5010_0000`，4 KB）
pub struct DpramMmio {
    core: Arc<Mutex<UsbCore>>,
}

impl DpramMmio {
    /// Wrap a shared controller core / 包装共享控制器核心
    #[must_use]
    pub fn new(core: Arc<Mutex<UsbCore>>) -> Self {
        Self { core }
    }
}

impl MmioDevice for DpramMmio {
    fn name(&self) -> &'static str {
        "USBCTRL_DPRAM"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(DPRAM_BASE, DPRAM_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        let offset = offset as usize;
        if offset + 4 > DPRAM_SIZE {
            return None;
        }
        Some(self.core.lock().read_dpram_u32(offset))
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        let offset = offset as usize;
        if offset + 4 > DPRAM_SIZE {
            return false;
        }
        self.core.lock().write_dpram_u32(offset, value);
        true
    }

    fn reset(&mut self) {
        self.core.lock().reset();
    }

    fn irq_number(&self) -> Option<u16> {
        Some(USBCTRL_IRQ)
    }
}

/// Register window (`0x5011_0000`, 16 KB with RW/XOR/SET/CLR aliases)
/// 寄存器窗口（`0x5011_0000`，含 RW/XOR/SET/CLR 别名的 16 KB）
pub struct RegsMmio {
    core: Arc<Mutex<UsbCore>>,
}

impl RegsMmio {
    /// Wrap a shared controller core / 包装共享控制器核心
    #[must_use]
    pub fn new(core: Arc<Mutex<UsbCore>>) -> Self {
        Self { core }
    }
}

impl MmioDevice for RegsMmio {
    fn name(&self) -> &'static str {
        "USBCTRL_REGS"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(REGS_BASE, REGS_REGION_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        let (register, _) = decode_alias(offset)?;
        Some(self.core.lock().read_reg(register & !3))
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        let Some((register, alias)) = decode_alias(offset) else {
            return false;
        };
        self.core.lock().write_reg(register & !3, alias, value);
        true
    }

    fn reset(&mut self) {
        self.core.lock().reset();
    }

    fn irq_number(&self) -> Option<u16> {
        Some(USBCTRL_IRQ)
    }
}

/// Mount both USBCTRL windows on a mainline MMIO bus
/// 在主线的 MMIO 总线上挂载两个 USBCTRL 窗口
///
/// # Errors
/// Returns a bus error when either window is invalid or overlaps an existing
/// region.
/// 任一窗口非法或与已有区域重叠时返回总线错误。
pub fn mount_usb(bus: &mut dyn AddressBus, core: Arc<Mutex<UsbCore>>) -> BusResult<()> {
    mount_device(bus, Box::new(DpramMmio::new(Arc::clone(&core))))?;
    mount_device(bus, Box::new(RegsMmio::new(core)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp2350_core::memory::SimpleBus;
    use rp2350_core::types::Address;

    use crate::regs::{self, Direction, MAIN_CTRL_CONTROLLER_EN};

    const MAIN_CTRL_ADDR: Address = REGS_BASE + regs::MAIN_CTRL as Address;
    const MAIN_CTRL_SET_ADDR: Address = REGS_BASE + 0x2000 + regs::MAIN_CTRL as Address;

    #[test]
    fn mounts_both_windows_and_routes_registers() {
        let core = Arc::new(Mutex::new(UsbCore::new()));
        let mut bus = SimpleBus::new();
        mount_usb(&mut bus, Arc::clone(&core)).unwrap();

        let mut cycles = 0;
        bus.write_u32(MAIN_CTRL_ADDR, MAIN_CTRL_CONTROLLER_EN, &mut cycles)
            .unwrap();
        bus.write_u32(MAIN_CTRL_SET_ADDR, 1, &mut cycles).unwrap();
        assert_eq!(
            bus.read_u32(MAIN_CTRL_ADDR, &mut cycles).unwrap(),
            MAIN_CTRL_CONTROLLER_EN | 1
        );

        bus.write_u32(DPRAM_BASE + 0x180, 0xDEAD_BEEF, &mut cycles)
            .unwrap();
        assert_eq!(
            bus.read_u32(DPRAM_BASE + 0x180, &mut cycles).unwrap(),
            0xDEAD_BEEF
        );
        assert_eq!(
            core.lock()
                .read_dpram_u32(regs::ep_ctrl_offset(1, Direction::In)),
            0
        );
    }

    #[test]
    fn cli_mount_covers_sie_ctrl_at_0x5011_004c() {
        let mut bus = SimpleBus::new();
        let controller = crate::mount(&mut bus).unwrap();
        let mut cycles = 0;
        // `0x5011_004C` is SIE_CTRL; firmware performs a read-modify-write
        // during TinyUSB `dcd_init`, which previously aborted as `NoDevice`.
        // `0x5011_004C` 是 SIE_CTRL；TinyUSB `dcd_init` 会对其读-改-写，
        // 之前会以 `NoDevice` 中止。
        let value = bus
            .read_u32(REGS_BASE + 0x4C, &mut cycles)
            .expect("SIE_CTRL must be readable after mount");
        bus.write_u32(REGS_BASE + 0x4C, value | 0x8000_0000, &mut cycles)
            .unwrap();
        assert_ne!(
            bus.read_u32(REGS_BASE + 0x4C, &mut cycles).unwrap() & 0x8000_0000,
            0
        );
        assert!(!controller.device_visible());
    }

    #[test]
    fn register_window_shadows_word_offsets() {
        let core = Arc::new(Mutex::new(UsbCore::new()));
        let mut device = RegsMmio::new(core);
        assert_eq!(device.read32(0x0FC), Some(0));
        assert!(device.write32(0x0FC, 0xA5A5_A5A5));
        assert_eq!(device.read32(0x0FC), Some(0xA5A5_A5A5));
        assert_eq!(device.read32(0x4000), None);
        assert!(!device.write32(0x4000, 0));
        assert_eq!(device.irq_number(), Some(USBCTRL_IRQ));
    }
}
