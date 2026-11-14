//! RESETS peripheral stub / RESETS 外设桩
//!
//! `RESET_DONE` reports the implemented reset bits that are currently released,
//! so SDK `reset_block` / `unreset_block` polling loops complete. Granular reset
//! timing is not modelled.
//! `RESET_DONE` 报告当前已释放的已实现复位位，因此 SDK 的 `reset_block` /
//! `unreset_block` 轮询循环能够完成。不模拟细粒度复位时序。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// Implemented `RESET_DONE` bits / 已实现的 `RESET_DONE` 位
///
/// RP2350 implements resets 0..28. Boot2 waits for `mask & ~RESET_DONE == 0`
/// after releasing a block, so unimplemented bits must read zero.
/// RP2350 实现复位位 0..28。boot2 释放模块后会等待
/// `mask & ~RESET_DONE == 0`，因此未实现的位必须读为零。
const RESET_DONE_IMPLEMENTED: u32 = 0x1FFF_FFFF;

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
            // RESET_DONE: a set bit means the block is out of reset. Bits that
            // no peripheral implements always read zero.
            // RESET_DONE：置位表示模块已脱离复位；没有外设实现的位始终读零。
            0x08 => {
                let held = self.regs.read(0x00).unwrap_or(0);
                Some(!held & RESET_DONE_IMPLEMENTED)
            }
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
    fn reset_done_tracks_the_implemented_reset_bits() {
        let mut resets = Resets::new();
        // Power-on: nothing is held in reset, so every implemented bit reads 1.
        // 上电时没有模块处于复位，因此所有已实现位读为 1。
        assert_eq!(resets.read32(0x08), Some(RESET_DONE_IMPLEMENTED));
        // Holding every bit in reset clears the implemented done bits.
        // 将所有位保持复位会清除已实现的 done 位。
        resets.write32(0x00, u32::MAX);
        assert_eq!(resets.read32(0x08), Some(0));
        // Releasing the vendor mask restores exactly those implemented bits.
        // 释放厂商掩码后，恰好恢复这些已实现位。
        resets.write32(0x00, !RESET_DONE_IMPLEMENTED);
        assert_eq!(resets.read32(0x08), Some(RESET_DONE_IMPLEMENTED));
    }
}
