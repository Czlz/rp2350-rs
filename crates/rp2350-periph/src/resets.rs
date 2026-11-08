//! RESETS peripheral stub / RESETS 外设桩
//!
//! `RESET_DONE` always reports every block as released, so SDK `reset_block` /
//! `unreset_block` polling loops complete immediately. Granular reset timing is
//! not modelled.
//! `RESET_DONE` 始终报告所有模块已释放，因此 SDK 的 `reset_block` /
//! `unreset_block` 轮询循环立即完成。不模拟细粒度复位时序。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// RESETS register block stub / RESETS 寄存器块桩
#[derive(Debug)]
pub struct Resets {
    regs: RegisterFile,
}

impl Resets {
    /// Create the block with all resets released / 创建所有复位均已释放的寄存器块
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: RegisterFile::new(map::DEVICE_WINDOW_SIZE),
        }
    }
}

impl Default for Resets {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for Resets {
    fn name(&self) -> &str {
        "RESETS"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::RESETS_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        match offset {
            // RESET_DONE: every block is out of reset. 所有模块均已完成复位。
            0x08 => Some(u32::MAX),
            _ => self.regs.read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        match offset {
            // RESET_DONE is read-only. RESET_DONE 为只读。
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
    fn reset_done_is_always_set() {
        let mut resets = Resets::new();
        resets.write32(0x00, 0xFFFF_FFFF);
        assert_eq!(resets.read32(0x08), Some(u32::MAX));
    }
}
