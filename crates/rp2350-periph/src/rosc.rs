//! ROSC — ring oscillator / ROSC — 环形振荡器
//!
//! The block decodes `ROSC_CTRL.ENABLE`, `ROSC_CTRL.FREQ_RANGE` and `ROSC_DIV`
//! into a shared [`ClockTree`]. The divider follows the datasheet encoding
//! `0xAA00 + div`: `div = 0` divides by 128, `1..=127` by `div`, and any other
//! value also by 128. `STATUS` reports `STABLE | DIV_RUNNING | ENABLED` while
//! the oscillator is on.
//! 该块把 `ROSC_CTRL.ENABLE`、`ROSC_CTRL.FREQ_RANGE` 与 `ROSC_DIV` 解码进共享
//! [`ClockTree`]。分频遵循数据手册编码 `0xAA00 + div`：`div = 0` 为 128 分频，
//! `1..=127` 按 `div`，其他值同样按 128。振荡器开启时 `STATUS` 报告
//! `STABLE | DIV_RUNNING | ENABLED`。
//!
//! The ring frequency itself is a nominal constant ([`ROSC_RING_HZ`]); the chip
//! has no way to measure it, so `FREQ_RANGE` and the drive-strength registers
//! are stored but do not rescale that constant.
//! 环形频率本身是标称常量（[`ROSC_RING_HZ`]）；芯片无法测量它，因此
//! `FREQ_RANGE` 与驱动强度寄存器只保存，不改变该常量。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::clock_tree::{ClockTree, ROSC_RING_HZ};
use super::map;

/// `ROSC_CTRL` offset / `ROSC_CTRL` 偏移
const CTRL: u32 = 0x00;

/// `ROSC_RANDOM` offset / `ROSC_RANDOM` 偏移
const RANDOM: u32 = 0x0C;

/// `ROSC_DIV` offset / `ROSC_DIV` 偏移
const DIV: u32 = 0x14;

/// `ROSC_PHASE` offset / `ROSC_PHASE` 偏移
const PHASE: u32 = 0x18;

/// `ROSC_STATUS` offset / `ROSC_STATUS` 偏移
const STATUS: u32 = 0x1C;

/// `ROSC_RANDOMBIT` offset / `ROSC_RANDOMBIT` 偏移
const RANDOMBIT: u32 = 0x20;

/// `ROSC_RANDOM` reset value / `ROSC_RANDOM` 复位值
const RANDOM_RESET: u32 = 0x3F04_B16D;

/// `ROSC_PHASE` reset value / `ROSC_PHASE` 复位值
const PHASE_RESET: u32 = 0x0000_0008;

/// ROSC register block / ROSC 寄存器块
#[derive(Debug)]
pub struct Rosc {
    tree: ClockTree,
    /// `FREQA`, `FREQB`, `RANDOM`, `DORMANT`, `PHASE` and `COUNT` storage.
    /// `FREQA`、`FREQB`、`RANDOM`、`DORMANT`、`PHASE` 与 `COUNT` 的存储。
    regs: RegisterFile,
}

impl Rosc {
    /// Create the block with a private bootrom-state tree
    /// 用私有 bootrom 状态时钟树创建寄存器块
    #[must_use]
    pub fn new() -> Self {
        Self::with_tree(ClockTree::new())
    }

    /// Create the block on a shared clock tree / 在共享时钟树上创建寄存器块
    #[must_use]
    pub fn with_tree(tree: ClockTree) -> Self {
        let mut regs = RegisterFile::new(map::DEVICE_WINDOW_SIZE);
        regs.fill(RANDOM, RANDOM_RESET);
        regs.fill(PHASE, PHASE_RESET);
        Self { tree, regs }
    }

    /// Shared clock tree / 共享时钟树
    #[must_use]
    pub fn tree(&self) -> &ClockTree {
        &self.tree
    }

    /// Nominal ring frequency before the divider / 分频前的标称环形频率
    #[must_use]
    pub fn ring_hz(&self) -> u32 {
        ROSC_RING_HZ
    }

    /// Output frequency after `ROSC_DIV` / `ROSC_DIV` 之后的输出频率
    #[must_use]
    pub fn output_hz(&self) -> u32 {
        self.tree.roscclk_hz()
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
            CTRL => Some(self.tree.rosc_ctrl()),
            DIV => Some(self.tree.rosc_div()),
            STATUS => Some(self.tree.rosc_status()),
            // RANDOMBIT 直接反映振荡器输出，本模型恒为 1。
            // RANDOMBIT reflects the oscillator output; the model reports 1.
            RANDOMBIT => Some(1),
            _ => self.regs.read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        match offset {
            CTRL => {
                self.tree.write_rosc_ctrl(value);
                true
            }
            DIV => {
                self.tree.write_rosc_div(value);
                true
            }
            STATUS => {
                self.tree.write_rosc_status(value);
                true
            }
            // RANDOMBIT 只读。RANDOMBIT is read-only.
            RANDOMBIT => true,
            _ => self.regs.write(offset, value),
        }
    }

    fn reset(&mut self) {
        self.regs.reset();
        self.regs.fill(RANDOM, RANDOM_RESET);
        self.regs.fill(PHASE, PHASE_RESET);
        self.tree.reset_rosc();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reports_the_oscillator_ready() {
        let mut rosc = Rosc::new();
        let status = rosc.read32(STATUS).unwrap();
        assert_eq!(status & (1 << 31), 1 << 31);
        assert_eq!(status & (1 << 16), 1 << 16);
        assert_eq!(status & (1 << 12), 1 << 12);
    }

    #[test]
    fn registers_reset_to_the_datasheet_values() {
        let mut rosc = Rosc::new();
        assert_eq!(rosc.read32(CTRL), Some(0x0000_0AA0));
        assert_eq!(rosc.read32(RANDOM), Some(RANDOM_RESET));
        assert_eq!(rosc.read32(PHASE), Some(PHASE_RESET));
        assert_eq!(rosc.read32(RANDOMBIT), Some(1));
    }

    #[test]
    fn control_and_divider_writes_are_readable_storage() {
        let mut rosc = Rosc::new();
        // 0xFAB ENABLE + MEDIUM 范围编码 0xFA5。
        // 0xFAB ENABLE with the MEDIUM range code 0xFA5.
        assert!(rosc.write32(CTRL, 0x00FA_BFA5));
        assert_eq!(rosc.read32(CTRL), Some(0x00FA_BFA5));
        assert!(rosc.write32(DIV, 0x0000_AA10));
        assert_eq!(rosc.read32(DIV), Some(0x0000_AA10));
        assert_eq!(rosc.read32(STATUS).unwrap() & (1 << 31), 1 << 31);
    }

    #[test]
    fn divider_changes_the_reported_frequency() {
        let mut rosc = Rosc::new();
        assert_eq!(rosc.output_hz(), ROSC_RING_HZ / 32);
        assert!(rosc.write32(DIV, 0xAA00 | 4));
        assert_eq!(rosc.output_hz(), ROSC_RING_HZ / 4);
        assert!(rosc.write32(DIV, 0xAA00));
        assert_eq!(rosc.output_hz(), ROSC_RING_HZ / 128);
    }

    #[test]
    fn disable_stops_the_output_and_reset_restores_it() {
        let mut rosc = Rosc::new();
        assert!(rosc.write32(CTRL, 0xD1E << 12));
        assert_eq!(rosc.output_hz(), 0);
        rosc.reset();
        assert_eq!(rosc.output_hz(), ROSC_RING_HZ / 32);
        assert_eq!(rosc.read32(CTRL), Some(0x0000_0AA0));
    }
}
