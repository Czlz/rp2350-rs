//! PADS_QSPI peripheral stub / PADS_QSPI 外设桩
//!
//! The six QSPI pads (`GPIO_QSPI_SCLK`, `SD0..SD3`, `SS`) reset to the reset
//! values in the RP2350 SVD: `0x156` for SCLK/SD0/SD1 and `0x15A` for
//! SD2/SD3/SS. Writes are stored and read back; pad isolation and drive
//! strength have no electrical effect in this model.
//! 六个 QSPI pad（`GPIO_QSPI_SCLK`、`SD0..SD3`、`SS`）按 RP2350 SVD 的复位值
//! 初始化：SCLK/SD0/SD1 为 `0x156`，SD2/SD3/SS 为 `0x15A`。写入仅被保存并可
//! 回读；pad 隔离与驱动强度在本模型中没有电气效果。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// Offset of the `VOLTAGE_SELECT` register / `VOLTAGE_SELECT` 寄存器偏移
const VOLTAGE_SELECT: u32 = 0x00;

/// Offset of `GPIO_QSPI_SCLK` / `GPIO_QSPI_SCLK` 偏移
const SCLK: u32 = 0x04;

/// Offset of `GPIO_QSPI_SD0` / `GPIO_QSPI_SD0` 偏移
const SD0: u32 = 0x08;

/// Offset of `GPIO_QSPI_SD1` / `GPIO_QSPI_SD1` 偏移
const SD1: u32 = 0x0C;

/// Offset of `GPIO_QSPI_SD2` / `GPIO_QSPI_SD2` 偏移
const SD2: u32 = 0x10;

/// Offset of `GPIO_QSPI_SD3` / `GPIO_QSPI_SD3` 偏移
const SD3: u32 = 0x14;

/// Offset of `GPIO_QSPI_SS` / `GPIO_QSPI_SS` 偏移
const SS: u32 = 0x18;

/// Reset value shared by SCLK, SD0 and SD1 (Schmitt, pull-down, input, isolated)
/// SCLK、SD0、SD1 共用的复位值（Schmitt、下拉、输入使能、隔离）
const PAD_RESET_PULLDOWN: u32 = 0x0000_0156;

/// Reset value shared by SD2, SD3 and SS (Schmitt, pull-up, input, isolated)
/// SD2、SD3、SS 共用的复位值（Schmitt、上拉、输入使能、隔离）
const PAD_RESET_PULLUP: u32 = 0x0000_015A;

/// PADS_QSPI register block / PADS_QSPI 寄存器块
#[derive(Debug)]
pub struct PadsQspi {
    regs: RegisterFile,
}

impl PadsQspi {
    /// Create the block with the datasheet pad reset values
    /// 创建使用数据手册 pad 复位值的寄存器块
    #[must_use]
    pub fn new() -> Self {
        let mut regs = RegisterFile::new(map::DEVICE_WINDOW_SIZE);
        regs.fill(VOLTAGE_SELECT, 0);
        for offset in [SCLK, SD0, SD1] {
            regs.fill(offset, PAD_RESET_PULLDOWN);
        }
        for offset in [SD2, SD3, SS] {
            regs.fill(offset, PAD_RESET_PULLUP);
        }
        Self { regs }
    }
}

impl Default for PadsQspi {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for PadsQspi {
    fn name(&self) -> &str {
        "PADS_QSPI"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::PADS_QSPI_BASE, map::DEVICE_WINDOW_SIZE)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pads_reset_to_the_datasheet_values() {
        let mut pads = PadsQspi::new();
        assert_eq!(pads.read32(VOLTAGE_SELECT), Some(0));
        for offset in [SCLK, SD0, SD1] {
            assert_eq!(pads.read32(offset), Some(PAD_RESET_PULLDOWN));
        }
        for offset in [SD2, SD3, SS] {
            assert_eq!(pads.read32(offset), Some(PAD_RESET_PULLUP));
        }
    }

    #[test]
    fn pad_writes_are_readable_storage() {
        let mut pads = PadsQspi::new();
        assert!(pads.write32(SCLK, 0x0000_0056));
        assert_eq!(pads.read32(SCLK), Some(0x0000_0056));
        assert!(pads.write32(VOLTAGE_SELECT, 1));
        assert_eq!(pads.read32(VOLTAGE_SELECT), Some(1));
    }

    #[test]
    fn reset_restores_the_datasheet_values() {
        let mut pads = PadsQspi::new();
        assert!(pads.write32(SS, 0));
        pads.reset();
        assert_eq!(pads.read32(SS), Some(PAD_RESET_PULLUP));
    }
}
