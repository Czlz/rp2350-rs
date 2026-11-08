//! IO_QSPI peripheral stub / IO_QSPI 外设桩
//!
//! The six QSPI pins (SCLK, SS, SD0..SD3) each expose a `STATUS`/`CTRL` register
//! pair with the same layout as IO_BANK0. `CTRL` is plain storage, while
//! `STATUS` reports the `OEFROMPERI` bit (bit 9) when `FUNCSEL` selects the
//! XIP/QMI function, because the QMI block then drives the pin's output enable.
//! This is the bit the firmware's flash-exit helper polls before reprogramming
//! the QMI timing.
//! 六个 QSPI 引脚（SCLK、SS、SD0..SD3）各自暴露一对与 IO_BANK0 布局相同的
//! `STATUS`/`CTRL` 寄存器。`CTRL` 为普通存储；当 `FUNCSEL` 选择 XIP/QMI
//! 功能时，`STATUS` 的 `OEFROMPERI` 位（第 9 位）置一，因为此时 QMI 块驱动
//! 引脚输出使能。固件的 flash-exit 辅助函数在重配 QMI 时序前轮询的正是该位。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// Number of QSPI pins / QSPI 引脚数量
const PIN_COUNT: u32 = 6;

/// Offset stride between per-pin `STATUS` and `CTRL` registers
/// 每个引脚 `STATUS` 与 `CTRL` 之间的偏移步长
const PIN_STRIDE: u32 = 8;

/// Offset of the `CTRL` register inside one pin pair / 引脚对中 `CTRL` 的偏移
const CTRL_OFFSET: u32 = 4;

/// Mask of the `FUNCSEL` field in `CTRL` / `CTRL` 中 `FUNCSEL` 字段的掩码
const FUNCSEL_MASK: u32 = 0x1F;

/// `FUNCSEL` value selecting the XIP/QMI function / 选择 XIP/QMI 功能的 `FUNCSEL` 值
const FUNC_XIP: u32 = 0;

/// `STATUS.OEFROMPERI`: output enable driven by the peripheral
/// `STATUS.OEFROMPERI`：由外设驱动的输出使能
const STATUS_OEFROMPERI: u32 = 1 << 9;

/// IO_QSPI register block / IO_QSPI 寄存器块
#[derive(Debug)]
pub struct IoQspi {
    regs: RegisterFile,
}

impl IoQspi {
    /// Create the block / 创建寄存器块
    ///
    /// `CTRL` resets to zero so every pin starts on `FUNCSEL = 0` (XIP). The
    /// synthetic boot ROM does not reconfigure the QSPI pads, so the model
    /// starts from the post-boot state where QMI already owns them.
    /// `CTRL` 复位为零，所有引脚初始为 `FUNCSEL = 0`（XIP）。合成 boot ROM 不会
    /// 重新配置 QSPI pad，因此模型从 QMI 已接管这些引脚的启动后状态开始。
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: RegisterFile::new(map::DEVICE_WINDOW_SIZE),
        }
    }

    /// Report whether `offset` addresses a pin `STATUS` register
    /// 判断 `offset` 是否指向某个引脚的 `STATUS` 寄存器
    fn is_status_offset(offset: u32) -> bool {
        offset % PIN_STRIDE == 0 && offset / PIN_STRIDE < PIN_COUNT
    }
}

impl Default for IoQspi {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for IoQspi {
    fn name(&self) -> &str {
        "IO_QSPI"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::IO_QSPI_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        if Self::is_status_offset(offset) {
            let ctrl = self.regs.read(offset + CTRL_OFFSET)?;
            let status = if ctrl & FUNCSEL_MASK == FUNC_XIP {
                STATUS_OEFROMPERI
            } else {
                0
            };
            return Some(status);
        }
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

    /// Offset of `GPIO_QSPI_SD1_STATUS` polled by the flash-exit helper
    /// flash-exit 辅助函数轮询的 `GPIO_QSPI_SD1_STATUS` 偏移
    const SD1_STATUS: u32 = 0x18;

    /// Offset of `GPIO_QSPI_SD1_CTRL` / `GPIO_QSPI_SD1_CTRL` 的偏移
    const SD1_CTRL: u32 = 0x1C;

    #[test]
    fn status_reports_output_enable_on_the_xip_function() {
        let mut io = IoQspi::new();
        assert_eq!(io.read32(SD1_STATUS), Some(STATUS_OEFROMPERI));
    }

    #[test]
    fn status_clears_when_the_pin_moves_off_the_xip_function() {
        let mut io = IoQspi::new();
        assert!(io.write32(SD1_CTRL, 5)); // FUNCSEL = SIO
        assert_eq!(io.read32(SD1_STATUS), Some(0));
        assert!(io.write32(SD1_CTRL, FUNC_XIP));
        assert_eq!(io.read32(SD1_STATUS), Some(STATUS_OEFROMPERI));
    }

    #[test]
    fn control_registers_are_readable_storage() {
        let mut io = IoQspi::new();
        assert!(io.write32(SD1_CTRL, 0x0000_0010));
        assert_eq!(io.read32(SD1_CTRL), Some(0x0000_0010));
    }
}
