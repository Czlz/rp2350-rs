//! XOSC peripheral stub / XOSC 外设桩
//!
//! `STATUS` always reports `STABLE` and `ENABLED`, so firmware waiting for the
//! crystal to settle continues immediately. Frequency and startup delay are not
//! modelled.
//! `STATUS` 始终报告 `STABLE` 与 `ENABLED`，等待晶振稳定的固件会立即继续。
//! 不模拟频率与启动延时。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// `XOSC_STATUS.STABLE` / `XOSC_STATUS.STABLE`
const STATUS_STABLE: u32 = 1 << 31;

/// `XOSC_STATUS.ENABLED` / `XOSC_STATUS.ENABLED`
const STATUS_ENABLED: u32 = 1 << 12;

/// XOSC register block stub / XOSC 寄存器块桩
#[derive(Debug)]
pub struct Xosc {
    regs: RegisterFile,
}

impl Xosc {
    /// Create the block with the crystal reported as running / 创建晶振已运行的寄存器块
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: RegisterFile::new(map::DEVICE_WINDOW_SIZE),
        }
    }
}

impl Default for Xosc {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for Xosc {
    fn name(&self) -> &str {
        "XOSC"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::XOSC_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        match offset {
            0x00 => Some(STATUS_STABLE | STATUS_ENABLED),
            _ => self.regs.read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        match offset {
            0x00 => true,
            _ => self.regs.write(offset, value),
        }
    }

    fn reset(&mut self) {
        self.regs.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reports_stable() {
        let mut xosc = Xosc::new();
        assert_eq!(xosc.read32(0x00), Some(0x8000_1000));
    }
}
