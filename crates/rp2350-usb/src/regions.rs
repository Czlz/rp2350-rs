//! `MemoryRegion` views for the USB DPSRAM and control registers
//! USB DPSRAM 与控制寄存器的 `MemoryRegion` 视图

use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_core::bus::{BusResult, MemoryRegion};
use rp2350_core::error::{BusErrorKind, CoreError};
use rp2350_core::types::Address;
use rp2350_periph::Peripheral;

use crate::device::UsbCore;
use crate::regs::Alias;
use crate::{DPRAM_BASE, DPRAM_SIZE, REGS_BASE, REGS_REGION_SIZE, USBCTRL_IRQ};

/// Byte offset from the DPSRAM base to the register block (0x10000)
/// 从 DPSRAM 基址到寄存器块的字节偏移（0x10000）
const REGS_WINDOW_OFFSET: usize = 0x1_0000;

/// Size of the combined DPSRAM + gap + register window
/// DPSRAM、间隙与寄存器窗口合并后的字节大小
const COMBINED_SIZE: usize = REGS_WINDOW_OFFSET + REGS_REGION_SIZE;

fn read_reg_u8(core: &Arc<Mutex<UsbCore>>, offset: usize) -> u8 {
    let aligned = offset & !3;
    let shift = (offset & 3) * 8;
    let word = core.lock().read_reg(aligned);
    (word >> shift) as u8
}

fn write_reg_u8(core: &Arc<Mutex<UsbCore>>, offset: usize, value: u8) {
    let aligned = offset & !3;
    let shift = (offset & 3) * 8;
    let mask = 0xFFu32 << shift;
    core.lock()
        .write_reg_masked(aligned, Alias::Rw, u32::from(value) << shift, mask);
}

fn read_reg_u16(core: &Arc<Mutex<UsbCore>>, offset: usize) -> u16 {
    let aligned = offset & !3;
    let shift = (offset & 2) * 8;
    let word = core.lock().read_reg(aligned);
    (word >> shift) as u16
}

fn write_reg_u16(core: &Arc<Mutex<UsbCore>>, offset: usize, value: u16) {
    let aligned = offset & !3;
    let shift = (offset & 2) * 8;
    let mask = 0xFFFFu32 << shift;
    core.lock()
        .write_reg_masked(aligned, Alias::Rw, u32::from(value) << shift, mask);
}

fn read_reg_u32(core: &Arc<Mutex<UsbCore>>, offset: usize) -> u32 {
    core.lock().read_reg(offset)
}

/// Decode a register-window address into a register offset and alias
/// 将寄存器窗口地址解码为寄存器偏移与别名
#[must_use]
pub fn decode_regs(addr: Address) -> Option<(usize, Alias)> {
    let window = addr.checked_sub(REGS_BASE)? as usize;
    if window >= REGS_REGION_SIZE {
        return None;
    }
    let alias = Alias::from_window((window >> 12) as u32);
    Some((window & 0xFFF, alias))
}

/// DPSRAM window at `0x5010_0000` (4 KB)
/// 位于 `0x5010_0000` 的 DPSRAM 窗口（4 KB）
pub struct DpramRegion {
    core: Arc<Mutex<UsbCore>>,
}

impl DpramRegion {
    /// Create a DPSRAM window over a shared controller core
    /// 在共享控制器核心上创建 DPSRAM 窗口
    #[must_use]
    pub fn new(core: Arc<Mutex<UsbCore>>) -> Self {
        Self { core }
    }

    #[allow(clippy::unused_self)]
    fn offset(&self, addr: Address) -> BusResult<usize> {
        let offset = addr
            .checked_sub(DPRAM_BASE)
            .map_or(usize::MAX, |v| v as usize);
        if offset < DPRAM_SIZE {
            Ok(offset)
        } else {
            Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })
        }
    }
}

impl MemoryRegion for DpramRegion {
    fn name(&self) -> &'static str {
        "USBCTRL_DPRAM"
    }

    fn base(&self) -> Address {
        DPRAM_BASE
    }

    fn size(&self) -> usize {
        DPRAM_SIZE
    }

    fn read_u8(&mut self, addr: Address, _cycles: &mut u64) -> BusResult<u8> {
        Ok(self.core.lock().read_dpram_u8(self.offset(addr)?))
    }

    fn write_u8(&mut self, addr: Address, value: u8, _cycles: &mut u64) -> BusResult<()> {
        let offset = self.offset(addr)?;
        self.core.lock().write_dpram_u8(offset, value);
        Ok(())
    }

    fn read_u16(&mut self, addr: Address, _cycles: &mut u64) -> BusResult<u16> {
        let offset = self.offset(addr)?;
        Ok(self.core.lock().read_dpram_u16(offset))
    }

    fn write_u16(&mut self, addr: Address, value: u16, _cycles: &mut u64) -> BusResult<()> {
        let offset = self.offset(addr)?;
        self.core.lock().write_dpram_u16(offset, value);
        Ok(())
    }

    fn read_u32(&mut self, addr: Address, _cycles: &mut u64) -> BusResult<u32> {
        let offset = self.offset(addr)?;
        Ok(self.core.lock().read_dpram_u32(offset))
    }

    fn write_u32(&mut self, addr: Address, value: u32, _cycles: &mut u64) -> BusResult<()> {
        let offset = self.offset(addr)?;
        self.core.lock().write_dpram_u32(offset, value);
        Ok(())
    }
}

/// Control register window at `0x5011_0000` covering the RW/XOR/SET/CLR aliases
/// 位于 `0x5011_0000` 的控制寄存器窗口，覆盖 RW/XOR/SET/CLR 别名
pub struct RegsRegion {
    core: Arc<Mutex<UsbCore>>,
}

impl RegsRegion {
    /// Create a register window over a shared controller core
    /// 在共享控制器核心上创建寄存器窗口
    #[must_use]
    pub fn new(core: Arc<Mutex<UsbCore>>) -> Self {
        Self { core }
    }
}

impl MemoryRegion for RegsRegion {
    fn name(&self) -> &'static str {
        "USBCTRL_REGS"
    }

    fn base(&self) -> Address {
        REGS_BASE
    }

    fn size(&self) -> usize {
        REGS_REGION_SIZE
    }

    fn read_u8(&mut self, addr: Address, _cycles: &mut u64) -> BusResult<u8> {
        let (offset, _) = decode_regs(addr).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::NoDevice,
        })?;
        Ok(read_reg_u8(&self.core, offset))
    }

    fn write_u8(&mut self, addr: Address, value: u8, _cycles: &mut u64) -> BusResult<()> {
        let (offset, _) = decode_regs(addr).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::NoDevice,
        })?;
        write_reg_u8(&self.core, offset, value);
        Ok(())
    }

    fn read_u16(&mut self, addr: Address, _cycles: &mut u64) -> BusResult<u16> {
        let (offset, _) = decode_regs(addr).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::NoDevice,
        })?;
        Ok(read_reg_u16(&self.core, offset))
    }

    fn write_u16(&mut self, addr: Address, value: u16, _cycles: &mut u64) -> BusResult<()> {
        let (offset, alias) = decode_regs(addr).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::NoDevice,
        })?;
        if alias == Alias::Rw {
            write_reg_u16(&self.core, offset, value);
        } else {
            let shift = (offset & 2) * 8;
            self.core.lock().write_reg_masked(
                offset & !3,
                alias,
                u32::from(value) << shift,
                0xFFFFu32 << shift,
            );
        }
        Ok(())
    }

    fn read_u32(&mut self, addr: Address, _cycles: &mut u64) -> BusResult<u32> {
        let (offset, _) = decode_regs(addr).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::NoDevice,
        })?;
        Ok(read_reg_u32(&self.core, offset & !3))
    }

    fn write_u32(&mut self, addr: Address, value: u32, _cycles: &mut u64) -> BusResult<()> {
        let (offset, alias) = decode_regs(addr).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::NoDevice,
        })?;
        self.core.lock().write_reg(offset & !3, alias, value);
        Ok(())
    }
}

/// Combined peripheral covering DPSRAM, the reserved gap and the register window
/// 覆盖 DPSRAM、保留间隙与寄存器窗口的合并外设
pub struct UsbPeripheral {
    core: Arc<Mutex<UsbCore>>,
}

impl UsbPeripheral {
    /// Create the combined peripheral over a shared controller core
    /// 在共享控制器核心上创建合并外设
    #[must_use]
    pub fn new(core: Arc<Mutex<UsbCore>>) -> Self {
        Self { core }
    }

    #[allow(clippy::unused_self)]
    fn offset(&self, addr: Address) -> BusResult<usize> {
        let offset = addr
            .checked_sub(DPRAM_BASE)
            .map_or(usize::MAX, |v| v as usize);
        if offset < COMBINED_SIZE {
            Ok(offset)
        } else {
            Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })
        }
    }
}

impl MemoryRegion for UsbPeripheral {
    fn name(&self) -> &'static str {
        "USBCTRL"
    }

    fn base(&self) -> Address {
        DPRAM_BASE
    }

    fn size(&self) -> usize {
        COMBINED_SIZE
    }

    fn read_u8(&mut self, addr: Address, _cycles: &mut u64) -> BusResult<u8> {
        let offset = self.offset(addr)?;
        if offset < DPRAM_SIZE {
            Ok(self.core.lock().read_dpram_u8(offset))
        } else if offset >= REGS_WINDOW_OFFSET {
            Ok(read_reg_u8(&self.core, offset - REGS_WINDOW_OFFSET))
        } else {
            Ok(0)
        }
    }

    fn write_u8(&mut self, addr: Address, value: u8, _cycles: &mut u64) -> BusResult<()> {
        let offset = self.offset(addr)?;
        if offset < DPRAM_SIZE {
            self.core.lock().write_dpram_u8(offset, value);
        } else if offset >= REGS_WINDOW_OFFSET {
            write_reg_u8(&self.core, offset - REGS_WINDOW_OFFSET, value);
        }
        Ok(())
    }

    fn read_u16(&mut self, addr: Address, _cycles: &mut u64) -> BusResult<u16> {
        let offset = self.offset(addr)?;
        if offset < DPRAM_SIZE {
            Ok(self.core.lock().read_dpram_u16(offset))
        } else if offset >= REGS_WINDOW_OFFSET {
            Ok(read_reg_u16(&self.core, offset - REGS_WINDOW_OFFSET))
        } else {
            Ok(0)
        }
    }

    fn write_u16(&mut self, addr: Address, value: u16, _cycles: &mut u64) -> BusResult<()> {
        let offset = self.offset(addr)?;
        if offset < DPRAM_SIZE {
            self.core.lock().write_dpram_u16(offset, value);
        } else if offset >= REGS_WINDOW_OFFSET {
            let reg = offset - REGS_WINDOW_OFFSET;
            let shift = (reg & 2) * 8;
            self.core.lock().write_reg_masked(
                reg & !3,
                Alias::Rw,
                u32::from(value) << shift,
                0xFFFFu32 << shift,
            );
        }
        Ok(())
    }

    fn read_u32(&mut self, addr: Address, _cycles: &mut u64) -> BusResult<u32> {
        let offset = self.offset(addr)?;
        if offset < DPRAM_SIZE {
            Ok(self.core.lock().read_dpram_u32(offset))
        } else if offset >= REGS_WINDOW_OFFSET {
            Ok(read_reg_u32(&self.core, (offset - REGS_WINDOW_OFFSET) & !3))
        } else {
            Ok(0)
        }
    }

    fn write_u32(&mut self, addr: Address, value: u32, _cycles: &mut u64) -> BusResult<()> {
        let offset = self.offset(addr)?;
        if offset < DPRAM_SIZE {
            self.core.lock().write_dpram_u32(offset, value);
        } else if offset >= REGS_WINDOW_OFFSET {
            let reg = (offset - REGS_WINDOW_OFFSET) & !3;
            self.core.lock().write_reg(reg, Alias::Rw, value);
        }
        Ok(())
    }
}

impl Peripheral for UsbPeripheral {
    fn peripheral_name(&self) -> &'static str {
        "USBCTRL"
    }

    fn reset(&mut self) {
        self.core.lock().reset();
    }

    fn irq_number(&self) -> Option<u16> {
        Some(USBCTRL_IRQ)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::REGS_BLOCK_SIZE;
    use crate::regs::{Direction, MAIN_CTRL, MAIN_CTRL_CONTROLLER_EN};

    const MAIN_CTRL_ADDR: Address = REGS_BASE + MAIN_CTRL as Address;
    const MAIN_CTRL_SET_ADDR: Address = REGS_BASE + 0x2000 + MAIN_CTRL as Address;

    fn shared() -> Arc<Mutex<UsbCore>> {
        Arc::new(Mutex::new(UsbCore::new()))
    }

    #[test]
    fn dpram_region_bounds() {
        let mut region = DpramRegion::new(shared());
        let mut cycles = 0;
        region
            .write_u8(DPRAM_BASE + 0x180, 0x5A, &mut cycles)
            .unwrap();
        assert_eq!(
            region.read_u8(DPRAM_BASE + 0x180, &mut cycles).unwrap(),
            0x5A
        );
        assert!(region.read_u8(DPRAM_BASE + 0x1000, &mut cycles).is_err());
        assert_eq!(region.size(), DPRAM_SIZE);
    }

    #[test]
    fn regs_region_covers_alias_windows() {
        let core = shared();
        let mut region = RegsRegion::new(Arc::clone(&core));
        let mut cycles = 0;
        region
            .write_u32(MAIN_CTRL_ADDR, MAIN_CTRL_CONTROLLER_EN, &mut cycles)
            .unwrap();
        region
            .write_u32(MAIN_CTRL_SET_ADDR, 1, &mut cycles)
            .unwrap();
        assert_eq!(
            region.read_u32(MAIN_CTRL_ADDR, &mut cycles).unwrap(),
            MAIN_CTRL_CONTROLLER_EN | 1
        );
        assert_eq!(region.size(), 16 * 1024);
        assert_eq!(REGS_BLOCK_SIZE, 4096);
    }

    #[test]
    fn combined_peripheral_routes_both_windows() {
        let mut peripheral = UsbPeripheral::new(shared());
        let mut cycles = 0;
        peripheral
            .write_u32(DPRAM_BASE + 0x180, 0xDEAD_BEEF, &mut cycles)
            .unwrap();
        peripheral
            .write_u32(MAIN_CTRL_ADDR, MAIN_CTRL_CONTROLLER_EN, &mut cycles)
            .unwrap();
        assert_eq!(
            peripheral
                .read_u32(DPRAM_BASE + 0x180, &mut cycles)
                .unwrap(),
            0xDEAD_BEEF
        );
        assert_eq!(
            peripheral.read_u32(MAIN_CTRL_ADDR, &mut cycles).unwrap(),
            MAIN_CTRL_CONTROLLER_EN
        );
        assert_eq!(Peripheral::irq_number(&peripheral), Some(USBCTRL_IRQ));
        assert_eq!(Peripheral::peripheral_name(&peripheral), "USBCTRL");
        assert_eq!(
            peripheral
                .read_u32(DPRAM_BASE + 0x1000, &mut cycles)
                .unwrap(),
            0
        );
    }

    #[test]
    fn register_byte_lanes_are_independent() {
        let core = shared();
        let mut region = RegsRegion::new(Arc::clone(&core));
        let mut cycles = 0;
        region.write_u8(MAIN_CTRL_ADDR, 1, &mut cycles).unwrap();
        region
            .write_u8(MAIN_CTRL_ADDR + 3, 0x80, &mut cycles)
            .unwrap();
        assert_eq!(core.lock().read_reg(MAIN_CTRL), 0x8000_0001);
    }

    #[test]
    fn direction_import_is_used() {
        // Keeps the public `Direction` re-export exercised through this module.
        assert_eq!(Direction::In.index(), 0);
    }
}
