//! ROSC peripheral stub / ROSC 外设桩
//!
//! Register reset values come from the RP2350 SVD (`CTRL = 0xAA0`,
//! `RANDOM = 0x3F04B16D`, `PHASE = 0x8`, `RANDOMBIT = 1`). `STATUS` always
//! reports `STABLE`, `DIV_RUNNING` and `ENABLED`, so firmware waiting for the
//! ring oscillator to settle continues immediately. The divider and phase
//! settings are stored but do not change any clock frequency.
//! 寄存器复位值取自 RP2350 SVD（`CTRL = 0xAA0`、`RANDOM = 0x3F04B16D`、
//! `PHASE = 0x8`、`RANDOMBIT = 1`）。`STATUS` 始终报告 `STABLE`、
//! `DIV_RUNNING` 与 `ENABLED`，等待环形振荡器稳定的固件会立即继续。分频与
//! 相位设置仅被保存，不改变任何时钟频率。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// `ROSC_CTRL` offset / `ROSC_CTRL` 偏移
const CTRL: u32 = 0x00;

/// `ROSC_RANDOM` offset / `ROSC_RANDOM` 偏移
const RANDOM: u32 = 0x0C;

/// `ROSC_PHASE` offset / `ROSC_PHASE` 偏移
const PHASE: u32 = 0x18;

/// `ROSC_STATUS` offset / `ROSC_STATUS` 偏移
const STATUS: u32 = 0x1C;

/// `ROSC_RANDOMBIT` offset / `ROSC_RANDOMBIT` 偏移
const RANDOMBIT: u32 = 0x20;

/// `ROSC_CTRL` reset value (`FREQ_RANGE = MEDIUM`, enable disabled)
/// `ROSC_CTRL` 复位值（`FREQ_RANGE = MEDIUM`，使能关闭）
const CTRL_RESET: u32 = 0x0000_0AA0;

/// `ROSC_RANDOM` reset value / `ROSC_RANDOM` 复位值
const RANDOM_RESET: u32 = 0x3F04_B16D;

/// `ROSC_PHASE` reset value / `ROSC_PHASE` 复位值
const PHASE_RESET: u32 = 0x0000_0008;

/// `ROSC_RANDOMBIT` reset value / `ROSC_RANDOMBIT` 复位值
const RANDOMBIT_RESET: u32 = 1;

/// `STATUS.STABLE` / `STATUS.STABLE`
const STATUS_STABLE: u32 = 1 << 31;

/// `STATUS.BADWRITE` / `STATUS.BADWRITE`
const STATUS_BADWRITE: u32 = 1 << 24;

/// `STATUS.DIV_RUNNING` / `STATUS.DIV_RUNNING`
const STATUS_DIV_RUNNING: u32 = 1 << 16;

/// `STATUS.ENABLED` / `STATUS.ENABLED`
const STATUS_ENABLED: u32 = 1 << 12;

/// ROSC register block / ROSC 寄存器块
#[derive(Debug)]
pub struct Rosc {
    regs: RegisterFile,
}

impl Rosc {
    /// Create the block with the datasheet reset values
    /// 创建使用数据手册复位值的寄存器块
    #[must_use]
    pub fn new() -> Self {
        let mut regs = RegisterFile::new(map::DEVICE_WINDOW_SIZE);
        regs.fill(CTRL, CTRL_RESET);
        regs.fill(RANDOM, RANDOM_RESET);
        regs.fill(PHASE, PHASE_RESET);
        regs.fill(RANDOMBIT, RANDOMBIT_RESET);
        Self { regs }
    }
}

impl Default for Rosc {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for Rosc {
    fn name(&self) -> &str {
        "ROSC"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::ROSC_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        match offset {
            // 固件轮询的是 STATUS。Firmware polls STATUS.
            STATUS => {
                let badwrite = self.regs.read(STATUS).unwrap_or(0) & STATUS_BADWRITE;
                Some(badwrite | STATUS_STABLE | STATUS_DIV_RUNNING | STATUS_ENABLED)
            }
            _ => self.regs.read(offset),
        }
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
    fn status_reports_the_oscillator_ready() {
        let mut rosc = Rosc::new();
        let status = rosc.read32(STATUS).unwrap();
        assert_eq!(status & STATUS_STABLE, STATUS_STABLE);
        assert_eq!(status & STATUS_DIV_RUNNING, STATUS_DIV_RUNNING);
        assert_eq!(status & STATUS_ENABLED, STATUS_ENABLED);
    }

    #[test]
    fn registers_reset_to_the_datasheet_values() {
        let mut rosc = Rosc::new();
        assert_eq!(rosc.read32(CTRL), Some(CTRL_RESET));
        assert_eq!(rosc.read32(RANDOM), Some(RANDOM_RESET));
        assert_eq!(rosc.read32(PHASE), Some(PHASE_RESET));
        assert_eq!(rosc.read32(RANDOMBIT), Some(RANDOMBIT_RESET));
    }

    #[test]
    fn control_and_divider_writes_are_readable_storage() {
        let mut rosc = Rosc::new();
        assert!(rosc.write32(CTRL, 0x0000_FAA0));
        assert_eq!(rosc.read32(CTRL), Some(0x0000_FAA0));
        assert!(rosc.write32(0x14, 0x0000_AA10));
        assert_eq!(rosc.read32(0x14), Some(0x0000_AA10));
        assert_eq!(rosc.read32(STATUS).unwrap() & STATUS_STABLE, STATUS_STABLE);
    }
}
