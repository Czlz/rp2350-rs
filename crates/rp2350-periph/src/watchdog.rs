//! WATCHDOG peripheral stub / WATCHDOG 外设桩
//!
//! Scratch registers are real storage so boot-ROM style magic values survive.
//! The counter never fires: `REASON` reads zero and arming the watchdog has no
//! effect in Stage 4.
//! Scratch 寄存器是真实存储，可保留 boot ROM 风格的魔法值。计数器不会触发：
//! `REASON` 读为零，Stage 4 中使能看门狗没有效果。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// WATCHDOG register block stub / WATCHDOG 寄存器块桩
#[derive(Debug)]
pub struct Watchdog {
    regs: RegisterFile,
}

impl Watchdog {
    /// Create the block / 创建寄存器块
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: RegisterFile::new(map::DEVICE_WINDOW_SIZE),
        }
    }
}

impl Default for Watchdog {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for Watchdog {
    fn name(&self) -> &str {
        "WATCHDOG"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::WATCHDOG_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        match offset {
            // REASON: no watchdog reset occurred. REASON：未发生过看门狗复位。
            0x08 => Some(0),
            _ => self.regs.read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        match offset {
            // REASON is write-one-to-clear. REASON 为写 1 清零。
            0x08 => true,
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
    fn scratch_registers_are_storage() {
        let mut wd = Watchdog::new();
        wd.write32(0x0C, 0xB007_C0DE);
        assert_eq!(wd.read32(0x0C), Some(0xB007_C0DE));
        assert_eq!(wd.read32(0x08), Some(0));
    }
}
