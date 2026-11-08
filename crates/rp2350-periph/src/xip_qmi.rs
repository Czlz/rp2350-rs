//! XIP_QMI peripheral stub / XIP_QMI 外设桩
//!
//! The QMI register block is plain storage. Flash timing and command formats
//! are not interpreted: writes are accepted and read back so the boot flash
//! routines can program `M0`/`M1` and observe the values they wrote.
//! QMI 寄存器块为普通存储。不解释 Flash 时序与命令格式：写入被接受并可回读，
//! 使启动阶段的 Flash 例程能够配置 `M0`/`M1` 并观察自己写入的值。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// XIP_QMI register block / XIP_QMI 寄存器块
#[derive(Debug)]
pub struct XipQmi {
    regs: RegisterFile,
}

impl XipQmi {
    /// Create the block / 创建寄存器块
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: RegisterFile::new(map::DEVICE_WINDOW_SIZE),
        }
    }
}

impl Default for XipQmi {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for XipQmi {
    fn name(&self) -> &str {
        "XIP_QMI"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::XIP_QMI_BASE, map::DEVICE_WINDOW_SIZE)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_register_round_trips() {
        // `flash_exit_xip` 计算出的时序值写入 M0 时序寄存器后应可回读。
        // The timing value computed by `flash_exit_xip` must read back.
        let mut qmi = XipQmi::new();
        assert!(qmi.write32(0x0C, 0x4000_0202));
        assert_eq!(qmi.read32(0x0C), Some(0x4000_0202));
    }

    #[test]
    fn reset_clears_the_registers() {
        let mut qmi = XipQmi::new();
        assert!(qmi.write32(0x0C, 0xDEAD_BEEF));
        qmi.reset();
        assert_eq!(qmi.read32(0x0C), Some(0));
    }
}
