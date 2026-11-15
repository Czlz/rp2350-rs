//! PLL_SYS and PLL_USB / PLL_SYS 与 PLL_USB
//!
//! Both register blocks drive a shared [`ClockTree`]: `PLL_CS.REFDIV`,
//! `PLL_FBDIV_INT` and `PLL_PRIM.POSTDIV1/POSTDIV2` feed the live output
//! frequency through `vco = (ref / refdiv) * fbdiv`,
//! `out = vco / postdiv1 / postdiv2`. `PLL_CS.LOCK` is read-only and reads 1
//! while the PLL is powered (`PLL_PWR.PD == 0`) with a legal feedback divider.
//! 两个寄存器块共享同一 [`ClockTree`]：`PLL_CS.REFDIV`、
//! `PLL_FBDIV_INT`、`PLL_PRIM.POSTDIV1/POSTDIV2` 按
//! `vco = (ref / refdiv) * fbdiv`、`out = vco / postdiv1 / postdiv2` 实时决定
//! 输出频率。`PLL_CS.LOCK` 只读，PLL 已上电（`PLL_PWR.PD == 0`）且反馈分频
//! 合法时读 1。
//!
//! Invalid settings (`REFDIV = 0`, `POSTDIV = 0`, `FBDIV_INT` outside
//! `16..=320`) make the output read `0 Hz` and log one warning instead of
//! panicking; see [`crate::clock_tree`].
//! 非法配置（`REFDIV = 0`、`POSTDIV = 0`、`FBDIV_INT` 超出 `16..=320`）使输出
//! 读为 `0 Hz` 并只警告一次，不会 panic；见 [`crate::clock_tree`]。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::clock_tree::{ClockTree, PllId};
use super::map;

/// `PLL_CS` offset / `PLL_CS` 偏移
const CS: u32 = 0x00;

/// `PLL_PWR` offset / `PLL_PWR` 偏移
const PWR: u32 = 0x04;

/// `PLL_FBDIV_INT` offset / `PLL_FBDIV_INT` 偏移
const FBDIV: u32 = 0x08;

/// `PLL_PRIM` offset / `PLL_PRIM` 偏移
const PRIM: u32 = 0x0C;

/// One PLL register block / 一个 PLL 寄存器块
#[derive(Debug)]
pub struct Pll {
    name: &'static str,
    window: MmioWindow,
    id: PllId,
    tree: ClockTree,
    /// Interrupt registers (`INTR..INTS`) are plain storage.
    /// 中断寄存器（`INTR..INTS`）按普通存储处理。
    regs: RegisterFile,
}

impl Pll {
    /// Create `PLL_SYS` with a private bootrom-state tree
    /// 用私有 bootrom 状态时钟树创建 `PLL_SYS`
    #[must_use]
    pub fn sys() -> Self {
        Self::sys_with(ClockTree::new())
    }

    /// Create `PLL_USB` with a private bootrom-state tree
    /// 用私有 bootrom 状态时钟树创建 `PLL_USB`
    #[must_use]
    pub fn usb() -> Self {
        Self::usb_with(ClockTree::new())
    }

    /// Create `PLL_SYS` on a shared clock tree / 在共享时钟树上创建 `PLL_SYS`
    #[must_use]
    pub fn sys_with(tree: ClockTree) -> Self {
        Self::new(
            "PLL_SYS",
            MmioWindow::new(map::PLL_SYS_BASE, map::DEVICE_WINDOW_SIZE),
            PllId::Sys,
            tree,
        )
    }

    /// Create `PLL_USB` on a shared clock tree / 在共享时钟树上创建 `PLL_USB`
    #[must_use]
    pub fn usb_with(tree: ClockTree) -> Self {
        Self::new(
            "PLL_USB",
            MmioWindow::new(map::PLL_USB_BASE, map::DEVICE_WINDOW_SIZE),
            PllId::Usb,
            tree,
        )
    }

    /// Create one block / 创建一个寄存器块
    fn new(name: &'static str, window: MmioWindow, id: PllId, tree: ClockTree) -> Self {
        Self {
            name,
            window,
            id,
            tree,
            regs: RegisterFile::new(map::DEVICE_WINDOW_SIZE),
        }
    }

    /// Shared clock tree / 共享时钟树
    #[must_use]
    pub fn tree(&self) -> &ClockTree {
        &self.tree
    }

    /// Whether the PLL is locked / PLL 是否已锁定
    #[must_use]
    pub fn locked(&self) -> bool {
        self.tree.pll_locked(self.id)
    }

    /// Current output frequency / 当前输出频率
    #[must_use]
    pub fn output_hz(&self) -> u32 {
        self.tree.pll_hz(self.id)
    }
}

impl MmioDevice for Pll {
    fn name(&self) -> &str {
        self.name
    }

    fn window(&self) -> MmioWindow {
        self.window
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        match offset {
            CS => Some(self.tree.pll_cs(self.id)),
            PWR => Some(self.tree.pll_pwr(self.id)),
            FBDIV => Some(self.tree.pll_fbdiv(self.id)),
            PRIM => Some(self.tree.pll_prim(self.id)),
            _ => self.regs.read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        match offset {
            CS | PWR | FBDIV | PRIM => {
                self.tree.write_pll(self.id, offset, value);
                true
            }
            _ => self.regs.write(offset, value),
        }
    }

    fn reset(&mut self) {
        self.regs.reset();
        // 恢复 bootrom 的 PLL 配置（PLL_SYS 150 MHz / PLL_USB 48 MHz）。
        // Restore the bootrom PLL configuration (150 MHz / 48 MHz).
        self.tree.reset_pll(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_plls_report_lock_at_boot() {
        let mut sys = Pll::sys();
        let mut usb = Pll::usb();
        assert_ne!(sys.read32(CS).unwrap() & (1 << 31), 0);
        assert_ne!(usb.read32(CS).unwrap() & (1 << 31), 0);
        assert_eq!(sys.output_hz(), 150_000_000);
        assert_eq!(usb.output_hz(), 48_000_000);
    }

    #[test]
    fn register_writes_change_the_output_immediately() {
        let tree = ClockTree::new();
        let mut sys = Pll::sys_with(tree.clone());
        // FBDIV=126、POSTDIV1=6、POSTDIV2=1 → 1512 MHz / 6 = 252 MHz。
        // FBDIV=126 with 6/1 gives 1512 MHz / 6 = 252 MHz.
        assert!(sys.write32(FBDIV, 126));
        assert!(sys.write32(PRIM, (6 << 16) | (1 << 12)));
        assert_eq!(sys.output_hz(), 252_000_000);
        assert_eq!(tree.sysclk_hz(), 252_000_000);
        assert_eq!(sys.read32(FBDIV), Some(126));
        assert_eq!(sys.read32(PRIM), Some((6 << 16) | (1 << 12)));
    }

    #[test]
    fn lock_follows_power_and_fbdiv() {
        let tree = ClockTree::new();
        let mut sys = Pll::sys_with(tree.clone());
        assert!(sys.locked());
        // PWR.PD=1（复位值 0x2D）→ 掉电且不锁定。
        // PWR.PD=1 (reset value 0x2D) powers down and drops lock.
        assert!(sys.write32(PWR, 0x2D));
        assert!(!sys.locked());
        assert_eq!(sys.read32(CS).unwrap() & (1 << 31), 0);
        assert_eq!(sys.output_hz(), 0);
        assert!(sys.write32(PWR, 0));
        assert!(sys.locked());
        // FBDIV=0 不合法 → 不锁定。FBDIV=0 is invalid → no lock.
        assert!(sys.write32(FBDIV, 0));
        assert!(!sys.locked());
    }

    #[test]
    fn lock_bit_is_read_only_and_pwr_masks_reserved_bits() {
        let mut sys = Pll::sys();
        let before = sys.read32(CS).unwrap();
        // 写 LOCK=1 不改变 REFDIV/BYPASS，也不会被存进可写镜像。
        // Writing LOCK=1 leaves REFDIV/BYPASS alone.
        assert!(sys.write32(CS, before | (1 << 31) | 0xFFFF_0000));
        assert_eq!(sys.read32(CS).unwrap(), before);
        assert!(sys.write32(PWR, 0xFFFF_FFFF));
        assert_eq!(sys.read32(PWR), Some(0x2D));
    }

    #[test]
    fn interrupt_registers_are_plain_storage() {
        let mut sys = Pll::sys();
        assert_eq!(sys.read32(0x14), Some(0));
        assert!(sys.write32(0x14, 1));
        assert_eq!(sys.read32(0x14), Some(1));
    }

    #[test]
    fn reset_restores_the_bootrom_configuration() {
        let mut sys = Pll::sys();
        assert!(sys.write32(FBDIV, 126));
        // 只改 FBDIV：12 MHz × 126 / (5 × 2) = 151.2 MHz。
        // FBDIV alone: 12 MHz × 126 / (5 × 2) = 151.2 MHz.
        assert_eq!(sys.output_hz(), 151_200_000);
        sys.reset();
        assert_eq!(sys.output_hz(), 150_000_000);
        assert_eq!(sys.read32(FBDIV), Some(125));
        assert!(sys.locked());
    }
}
