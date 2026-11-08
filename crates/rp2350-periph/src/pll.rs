//! PLL_SYS and PLL_USB stubs / PLL_SYS 与 PLL_USB 桩
//!
//! `PLL_CS.LOCK` always reads as set, so SDK `pll_init` waits complete at once.
//! Feedback dividers and post dividers are stored but do not drive any clock
//! frequency in Stage 4.
//! `PLL_CS.LOCK` 始终读为已锁定，SDK 的 `pll_init` 等待会立即完成。反馈分频与
//! 后分频仅被保存，Stage 4 中不驱动任何时钟频率。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// `PLL_CS.LOCK` / `PLL_CS.LOCK`
const CS_LOCK: u32 = 1 << 31;

/// One PLL register block / 一个 PLL 寄存器块
#[derive(Debug)]
pub struct Pll {
    name: &'static str,
    window: MmioWindow,
    regs: RegisterFile,
}

impl Pll {
    /// Create the system PLL block / 创建系统 PLL 寄存器块
    #[must_use]
    pub fn sys() -> Self {
        Self::new(
            "PLL_SYS",
            MmioWindow::new(map::PLL_SYS_BASE, map::DEVICE_WINDOW_SIZE),
        )
    }

    /// Create the USB PLL block / 创建 USB PLL 寄存器块
    #[must_use]
    pub fn usb() -> Self {
        Self::new(
            "PLL_USB",
            MmioWindow::new(map::PLL_USB_BASE, map::DEVICE_WINDOW_SIZE),
        )
    }

    fn new(name: &'static str, window: MmioWindow) -> Self {
        Self {
            name,
            window,
            regs: RegisterFile::new(map::DEVICE_WINDOW_SIZE),
        }
    }
}

impl MmioDevice for Pll {
    fn name(&self) -> &str {
        self.name
    }

    fn window(&self) -> MmioWindow {
        self.window
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        match offset {
            // PLL_CS: always locked. PLL_CS：始终已锁定。
            0x00 => Some(self.regs.read(0).unwrap_or(0) | CS_LOCK),
            // PLL_PWR: all blocks powered. PLL_PWR：所有模块已上电。
            0x04 => Some(0),
            _ => self.regs.read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        self.regs.write(offset, value)
    }

    fn reset(&mut self) {
        self.regs.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_plls_report_lock() {
        let mut sys = Pll::sys();
        let mut usb = Pll::usb();
        assert_eq!(sys.read32(0x00).unwrap() & CS_LOCK, CS_LOCK);
        assert_eq!(usb.read32(0x00).unwrap() & CS_LOCK, CS_LOCK);
    }
}
