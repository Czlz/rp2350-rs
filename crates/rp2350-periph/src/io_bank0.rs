//! IO_BANK0 peripheral stub / IO_BANK0 外设桩
//!
//! `GPIOx_STATUS` and `GPIOx_CTRL` registers are plain storage for all 48 GPIOs.
//! Function selection and interrupt routing are not interpreted in Stage 4.
//! 48 个 GPIO 的 `GPIOx_STATUS` 与 `GPIOx_CTRL` 寄存器均为普通存储。Stage 4
//! 不解释功能选择与中断路由。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// IO_BANK0 register block / IO_BANK0 寄存器块
#[derive(Debug)]
pub struct IoBank0 {
    regs: RegisterFile,
}

impl IoBank0 {
    /// Create the block / 创建寄存器块
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: RegisterFile::new(map::DEVICE_WINDOW_SIZE),
        }
    }
}

impl Default for IoBank0 {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for IoBank0 {
    fn name(&self) -> &str {
        "IO_BANK0"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::IO_BANK0_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        self.regs.read(offset)
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        self.regs.write(offset, value)
    }

    fn reset(&mut self) {
        self.regs.reset();
    }

    fn irq_number(&self) -> Option<u16> {
        Some(map::IO_IRQ_BANK0)
    }
}
