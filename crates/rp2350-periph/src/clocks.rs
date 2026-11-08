//! CLOCKS peripheral stub / CLOCKS 外设桩
//!
//! The block stores control and divider registers. `CLK_*_SELECTED` is derived
//! from the channel's control register: `CLK_REF`/`CLK_SYS` report the bit of
//! their `SRC` field, all other channels report the bit of their `AUXSRC`
//! field. SDK clock-ready polling therefore succeeds once the firmware has
//! programmed the source it waits for.
//! 该块保存控制与分频寄存器。`CLK_*_SELECTED` 由通道控制寄存器推导：
//! `CLK_REF`/`CLK_SYS` 报告其 `SRC` 字段对应的位，其余通道报告 `AUXSRC`
//! 字段对应的位。因此只要固件写入它等待的源，SDK 的时钟就绪轮询就会成功。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// Number of clock channels (GPOUT0-3, REF, SYS, PERI, USB, ADC, RTC)
/// 时钟通道数量（GPOUT0-3、REF、SYS、PERI、USB、ADC、RTC）
const CLOCK_CHANNELS: u32 = 10;

/// Stride between clock channel registers / 时钟通道寄存器之间的步长
const CHANNEL_STRIDE: u32 = 0x0C;

/// Offset of `CLK_*_SELECTED` inside a channel / 通道内 `CLK_*_SELECTED` 的偏移
const SELECTED_OFFSET: u32 = 0x08;

/// Channel index of `CLK_REF` / `CLK_REF` 的通道索引
const CLK_REF: u32 = 4;

/// Channel index of `CLK_SYS` / `CLK_SYS` 的通道索引
const CLK_SYS: u32 = 5;

/// `CLK_REF_CTRL.SRC` field mask / `CLK_REF_CTRL.SRC` 字段掩码
const REF_SRC_MASK: u32 = 0x3;

/// `CLK_SYS_CTRL.SRC` field mask / `CLK_SYS_CTRL.SRC` 字段掩码
const SYS_SRC_MASK: u32 = 0x1;

/// Shift of the `AUXSRC` field in a control register / 控制寄存器中 `AUXSRC` 的位移
const AUXSRC_SHIFT: u32 = 5;

/// `AUXSRC` field mask / `AUXSRC` 字段掩码
const AUXSRC_MASK: u32 = 0x7;

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

    /// Compute `CLK_*_SELECTED` from the channel's control register
    /// 根据通道控制寄存器计算 `CLK_*_SELECTED`
    ///
    /// `CLK_REF` and `CLK_SYS` report the bit of their `SRC` field, so a
    /// cleared control register keeps reporting the default source as
    /// selected until the next write. The remaining channels report the bit of
    /// their `AUXSRC` field.
    /// `CLK_REF` 与 `CLK_SYS` 报告 `SRC` 字段对应的位，因此控制寄存器清零后
    /// 仍会报告默认源被选中，直到下一次写入；其余通道报告 `AUXSRC` 字段对应的位。
    fn selected(&self, channel: u32) -> u32 {
        let ctrl = self.regs.read(channel * CHANNEL_STRIDE).unwrap_or(0);
        let source = match channel {
            CLK_REF => ctrl & REF_SRC_MASK,
            CLK_SYS => ctrl & SYS_SRC_MASK,
            _ => (ctrl >> AUXSRC_SHIFT) & AUXSRC_MASK,
        };
        1 << source
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
            return Some(self.selected(channel));
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
    fn ref_and_sys_selected_follow_the_src_field() {
        let mut clocks = Clocks::new();
        // SRC=1（辅助源）+ AUXSRC=1（XOSC）。SRC=1 (aux) with AUXSRC=1 (XOSC).
        assert!(clocks.write32(0x30, 0x0000_0021));
        assert_eq!(clocks.read32(0x38), Some(0x2));
        // 清零后默认源（ROSC）仍被报告为已选中。
        // After the clear the default source (ROSC) is still reported selected.
        assert!(clocks.write32(0x30, 0x0000_0000));
        assert_eq!(clocks.read32(0x38), Some(0x1));

        assert!(clocks.write32(0x3C, 0x0000_0021));
        assert_eq!(clocks.read32(0x44), Some(0x2));
        assert!(clocks.write32(0x3C, 0x0000_0000));
        assert_eq!(clocks.read32(0x44), Some(0x1));
    }

    #[test]
    fn auxiliary_only_channels_follow_auxsrc() {
        let mut clocks = Clocks::new();
        // CLK_PERI_CTRL：AUXSRC=4（XOSC）。CLK_PERI_CTRL: AUXSRC=4 (XOSC).
        assert!(clocks.write32(0x48, 4 << AUXSRC_SHIFT));
        assert_eq!(clocks.read32(0x50), Some(0x10));
    }

    #[test]
    fn control_registers_are_writable() {
        let mut clocks = Clocks::new();
        assert!(clocks.write32(0x3C, 0x0000_0001));
        assert_eq!(clocks.read32(0x3C), Some(1));
    }
}
