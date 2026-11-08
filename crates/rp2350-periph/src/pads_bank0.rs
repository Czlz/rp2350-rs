//! PADS_BANK0 peripheral stub / PADS_BANK0 外设桩
//!
//! Pad registers reset to `0x56` (`IE | PDE | SCHMITT`) for all 48 GPIOs, which
//! matches the RP2350 datasheet reset value. Drive strength and pulls are stored
//! but have no electrical effect.
//! 全部 48 个 GPIO 的 pad 寄存器复位为 `0x56`（`IE | PDE | SCHMITT`），与
//! RP2350 数据手册复位值一致。驱动强度与上下拉仅被保存，无电气效果。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// Number of pads in bank 0 / bank 0 的 pad 数量
const PAD_COUNT: u32 = 48;

/// Offset of the first pad register / 首个 pad 寄存器的偏移
const PAD0_OFFSET: u32 = 0x04;

/// Default pad register value / 默认 pad 寄存器值
const PAD_RESET_VALUE: u32 = 0x0000_0056;

/// PADS_BANK0 register block / PADS_BANK0 寄存器块
#[derive(Debug)]
pub struct PadsBank0 {
    regs: RegisterFile,
}

impl PadsBank0 {
    /// Create the block with datasheet pad defaults / 创建使用数据手册 pad 默认值的寄存器块
    #[must_use]
    pub fn new() -> Self {
        let mut regs = RegisterFile::new(map::DEVICE_WINDOW_SIZE);
        for pin in 0..PAD_COUNT {
            regs.fill(PAD0_OFFSET + pin * 4, PAD_RESET_VALUE);
        }
        Self { regs }
    }
}

impl Default for PadsBank0 {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for PadsBank0 {
    fn name(&self) -> &str {
        "PADS_BANK0"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::PADS_BANK0_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        self.regs.read(offset)
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        self.regs.write(offset, value)
    }

    fn reset(&mut self) {
        *self = Self::new();
    }
}
