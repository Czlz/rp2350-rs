//! XOSC — crystal oscillator / XOSC — 晶振
//!
//! The block decodes `XOSC_CTRL.ENABLE`/`FREQ_RANGE` into a shared
//! [`ClockTree`], which feeds both PLLs and every clock source that selects
//! `xosc_clksrc`. `STATUS` reports `STABLE | ENABLED` as soon as the crystal is
//! enabled: startup delay and the startup counter are not modelled.
//! 该块把 `XOSC_CTRL.ENABLE`/`FREQ_RANGE` 解码进共享 [`ClockTree`]，后者驱动
//! 两个 PLL 以及所有选择 `xosc_clksrc` 的时钟源。晶振使能后 `STATUS` 立即报告
//! `STABLE | ENABLED`：不建模启动延时与启动计数器。
//!
//! Invalid `ENABLE`/`FREQ_RANGE` codes retain the previous value and latch
//! `STATUS.BADWRITE`, exactly as the datasheet requires.
//! 非法的 `ENABLE`/`FREQ_RANGE` 编码保留原值并锁存 `STATUS.BADWRITE`，与数据
//! 手册一致。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::clock_tree::ClockTree;
use super::map;

/// `XOSC_CTRL` offset (write-only on the real block; reads return the stored
/// image here) / `XOSC_CTRL` 偏移（真实芯片上只写；本模型读回保存的镜像）
const CTRL: u32 = 0x00;

/// `XOSC_STATUS` offset / `XOSC_STATUS` 偏移
const STATUS: u32 = 0x04;

/// XOSC register block / XOSC 寄存器块
#[derive(Debug)]
pub struct Xosc {
    tree: ClockTree,
    /// `DORMANT`, `STARTUP` and `COUNT` are plain storage.
    /// `DORMANT`、`STARTUP` 与 `COUNT` 按普通存储处理。
    regs: RegisterFile,
}

impl Xosc {
    /// Create the block with a private bootrom-state tree
    /// 用私有 bootrom 状态时钟树创建寄存器块
    #[must_use]
    pub fn new() -> Self {
        Self::with_tree(ClockTree::new())
    }

    /// Create the block on a shared clock tree / 在共享时钟树上创建寄存器块
    #[must_use]
    pub fn with_tree(tree: ClockTree) -> Self {
        Self {
            tree,
            regs: RegisterFile::new(map::DEVICE_WINDOW_SIZE),
        }
    }

    /// Shared clock tree / 共享时钟树
    #[must_use]
    pub fn tree(&self) -> &ClockTree {
        &self.tree
    }

    /// Configure the crystal frequency (host setting) / 配置晶振频率（宿主设置）
    pub fn set_frequency_hz(&self, hz: u32) {
        self.tree.set_xosc_hz(hz);
    }

    /// Current crystal frequency / 当前晶振频率
    #[must_use]
    pub fn frequency_hz(&self) -> u32 {
        self.tree.xosc_hz()
    }
}

impl Default for Xosc {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for Xosc {
    fn name(&self) -> &str {
        "XOSC"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::XOSC_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        match offset {
            CTRL => Some(self.tree.xosc_ctrl()),
            STATUS => Some(self.tree.xosc_status()),
            _ => self.regs.read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        match offset {
            CTRL => {
                self.tree.write_xosc_ctrl(value);
                true
            }
            // 只有 BADWRITE 可写（写 1 清除）。Only BADWRITE is writable (W1C).
            STATUS => {
                self.tree.write_xosc_status(value);
                true
            }
            _ => self.regs.write(offset, value),
        }
    }

    fn reset(&mut self) {
        self.regs.reset();
        self.tree.reset_xosc();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reports_stable_after_enable() {
        let mut xosc = Xosc::new();
        assert_eq!(xosc.read32(STATUS), Some(0x8000_1000));
        assert_eq!(xosc.read32(CTRL), Some(0x00FA_BAA0));
        assert_eq!(xosc.frequency_hz(), 12_000_000);
    }

    #[test]
    fn ctrl_writes_do_not_leak_into_status() {
        let mut xosc = Xosc::new();
        // 0x00FA_AA00 的 ENABLE=0xFAA 与 FREQ_RANGE=0xA00 都非法：保留原值并锁存
        // BADWRITE。Both ENABLE=0xFAA and FREQ_RANGE=0xA00 are invalid: the
        // previous values stay and BADWRITE latches.
        assert!(xosc.write32(CTRL, 0x00FA_AA00));
        assert_eq!(xosc.read32(CTRL), Some(0x00FA_BAA0));
        assert_ne!(xosc.read32(STATUS).unwrap() & (1 << 24), 0);
        assert_eq!(xosc.read32(STATUS).unwrap() & (1 << 12), 1 << 12);
    }

    #[test]
    fn valid_ctrl_codes_update_the_tree() {
        let tree = ClockTree::new();
        let mut xosc = Xosc::with_tree(tree.clone());
        // DISABLE 编码 + 1..15 MHz 范围。DISABLE code with the 1..15 MHz range.
        assert!(xosc.write32(CTRL, (0xD1E << 12) | 0xAA0));
        assert!(!tree.xosc_enabled());
        assert_eq!(tree.pll_sys_hz(), 0);
        assert!(xosc.write32(CTRL, (0xFAB << 12) | 0xAA1));
        assert!(tree.xosc_enabled());
        assert_eq!(xosc.read32(STATUS).unwrap() & 0x3, 1);
        assert_eq!(tree.pll_sys_hz(), 150_000_000);
    }

    #[test]
    fn host_frequency_override_feeds_the_pll() {
        let xosc = Xosc::new();
        // 24 MHz 晶振 + 同样的 125/5/2 → 300 MHz。
        // A 24 MHz crystal with the same 125/5/2 gives 300 MHz.
        xosc.set_frequency_hz(24_000_000);
        assert_eq!(xosc.frequency_hz(), 24_000_000);
        assert_eq!(xosc.tree().pll_sys_hz(), 300_000_000);
    }

    #[test]
    fn reset_restores_the_bootrom_state() {
        let mut xosc = Xosc::new();
        assert!(xosc.write32(CTRL, (0xD1E << 12) | 0xAA0));
        xosc.reset();
        assert_eq!(xosc.read32(CTRL), Some(0x00FA_BAA0));
        assert_eq!(xosc.tree().pll_sys_hz(), 150_000_000);
    }
}
