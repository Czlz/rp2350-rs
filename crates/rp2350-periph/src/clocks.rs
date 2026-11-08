//! CLOCKS peripheral stub / CLOCKS 外设桩
//!
//! The block stores control and divider registers. Every `CLK_*_SELECTED`
//! register reads back `1` (source 0 selected), so SDK clock-ready polling loops
//! that wait for a selected source succeed immediately.
//! 该块保存控制与分频寄存器。所有 `CLK_*_SELECTED` 寄存器读回 `1`（已选择
//! 源 0），因此 SDK 中等待选源完成的时钟就绪轮询会立即成功。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// Number of clock channels (GPOUT0-3, REF, SYS, PERI, USB, ADC, RTC)
/// 时钟通道数量（GPOUT0-3、REF、SYS、PERI、USB、ADC、RTC）
const CLOCK_CHANNELS: u32 = 10;

/// Stride between clock channel registers / 时钟通道寄存器之间的步长
const CHANNEL_STRIDE: u32 = 0x0C;

/// Offset of `CLK_*_SELECTED` inside a channel / 通道内 `CLK_*_SELECTED` 的偏移
const SELECTED_OFFSET: u32 = 0x08;

/// CLOCKS register block stub / CLOCKS 寄存器块桩
#[derive(Debug)]
pub struct Clocks {
    regs: RegisterFile,
}

impl Clocks {
    /// Create the block with default clock routing / 创建使用默认时钟路由的寄存器块
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: RegisterFile::new(map::DEVICE_WINDOW_SIZE),
        }
    }
}

impl Default for Clocks {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for Clocks {
    fn name(&self) -> &str {
        "CLOCKS"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::CLOCKS_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        let channel = offset / CHANNEL_STRIDE;
        if channel < CLOCK_CHANNELS && offset % CHANNEL_STRIDE == SELECTED_OFFSET {
            // 报告已选择源 0；真实芯片会按 CTRL 中的 AUXSRC 置位。
            // Report source 0 as selected; silicon sets the bit from AUXSRC.
            return Some(1);
        }
        self.regs.read(offset)
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        let channel = offset / CHANNEL_STRIDE;
        if channel < CLOCK_CHANNELS && offset % CHANNEL_STRIDE == SELECTED_OFFSET {
            // SELECTED 为只读寄存器。SELECTED is read-only.
            return true;
        }
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
    fn selected_registers_report_source_zero() {
        let mut clocks = Clocks::new();
        assert_eq!(clocks.read32(0x38), Some(1)); // CLK_REF_SELECTED
        assert_eq!(clocks.read32(0x44), Some(1)); // CLK_SYS_SELECTED
        assert_eq!(clocks.read32(0x50), Some(1)); // CLK_PERI_SELECTED
    }

    #[test]
    fn control_registers_are_writable() {
        let mut clocks = Clocks::new();
        assert!(clocks.write32(0x3C, 0x0000_0001));
        assert_eq!(clocks.read32(0x3C), Some(1));
    }
}
