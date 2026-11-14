//! PADS_BANK0 peripheral stub / PADS_BANK0 外设桩
//!
//! Pad registers reset to `0x56` (`IE | PDE | SCHMITT`) for all 48 GPIOs, which
//! matches the RP2350 datasheet reset value. Drive strength and pulls are stored
//! but have no electrical effect.
//! 全部 48 个 GPIO 的 pad 寄存器复位为 `0x56`（`IE | PDE | SCHMITT`），与
//! RP2350 数据手册复位值一致。驱动强度与上下拉仅被保存，无电气效果。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::gpio_pads::GpioPads;
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
    pads: GpioPads,
}

impl PadsBank0 {
    /// Create the block with datasheet pad defaults / 创建使用数据手册 pad 默认值的寄存器块
    #[must_use]
    pub fn new() -> Self {
        Self::with_pads(GpioPads::new())
    }

    /// Create the block sharing electrical state with SIO / 创建与 SIO 共享电气状态的块
    #[must_use]
    pub fn with_pads(pads: GpioPads) -> Self {
        let mut regs = RegisterFile::new(map::DEVICE_WINDOW_SIZE);
        for pin in 0..PAD_COUNT {
            regs.fill(PAD0_OFFSET + pin * 4, PAD_RESET_VALUE);
        }
        Self { regs, pads }
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
        let written = self.regs.write(offset, value);
        if written && offset >= PAD0_OFFSET {
            let pin = (offset - PAD0_OFFSET) / 4;
            if pin < PAD_COUNT {
                self.pads.set_pad(pin, value);
            }
        }
        written
    }

    fn reset(&mut self) {
        *self = Self::with_pads(self.pads.clone());
        self.pads.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_writes_program_the_shared_pull_state() {
        let pads = GpioPads::new();
        let mut block = PadsBank0::with_pads(pads.clone());
        let offset = PAD0_OFFSET;
        assert!(block.write32(offset, crate::gpio_pads::PAD_PUE));
        assert_eq!(pads.input_levels() & 1, 1);
        assert!(block.write32(offset, crate::gpio_pads::PAD_PDE));
        assert_eq!(pads.input_levels() & 1, 0);
    }

    #[test]
    fn reset_restores_pull_down() {
        let pads = GpioPads::new();
        pads.set_pad(0, crate::gpio_pads::PAD_PUE);
        let mut block = PadsBank0::with_pads(pads.clone());
        block.reset();
        assert_eq!(pads.input_levels(), 0);
    }
}
