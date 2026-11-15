//! CLOCKS peripheral / CLOCKS 外设
//!
//! The block owns the ten `CLK_*_CTRL`/`CLK_*_DIV` slices of RP2350 and
//! delegates every frequency decision to a shared [`ClockTree`]. Writes take
//! effect immediately: `CLK_SYS_CTRL`/`CLK_SYS_DIV` changes are visible in
//! [`ClockTree::sysclk_hz`] on the next read, which is what runtime
//! overclocking needs.
//! 该块拥有 RP2350 的十个 `CLK_*_CTRL`/`CLK_*_DIV` 通道，并把所有频率换算交给
//! 共享的 [`ClockTree`]。写入立即生效：`CLK_SYS_CTRL`/`CLK_SYS_DIV` 的改变在下
//! 一次读取 [`ClockTree::sysclk_hz`] 时即可见，满足运行时超频/降频需求。
//!
//! `CLK_*_SELECTED` is derived from the channel state: `CLK_REF`/`CLK_SYS`
//! report the one-hot bit of their glitchless `SRC` field, every other slice is
//! hardwired to `0x1` (they have no glitchless mux, see
//! `hardware/regs/clocks.h`). Switching is instant in this model; the
//! datasheet's "may briefly show all-0s" window is not modelled.
//! `CLK_*_SELECTED` 由通道状态推导：`CLK_REF`/`CLK_SYS` 报告其无毛刺 `SRC` 字段
//! 对应的单热位，其余通道按 `hardware/regs/clocks.h` 恒为 `0x1`（没有无毛刺
//! 选择器）。本模型切换瞬时完成，不建模数据手册所述「切换期间可能短暂全 0」。
//!
//! The remaining registers (`FC0_*`, `CLK_SYS_RESUS_*`, `WAKE_EN*`,
//! `SLEEP_EN*`, `DFTCLK_*`, interrupt registers) are plain storage; the
//! frequency counter is not modelled (P4-2 consumer work).
//! 其余寄存器（`FC0_*`、`CLK_SYS_RESUS_*`、`WAKE_EN*`、`SLEEP_EN*`、
//! `DFTCLK_*`、中断寄存器）按普通存储处理；频率计数器未建模（属于 P4-2 的消费
//! 者工作）。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::clock_tree::{CLOCK_COUNT, ClockTree};
use super::map;

/// Stride between clock channel registers / 时钟通道寄存器之间的步长
const CHANNEL_STRIDE: u32 = 0x0C;

/// Offset of `CLK_*_CTRL` inside a channel / 通道内 `CLK_*_CTRL` 的偏移
const CTRL_OFFSET: u32 = 0x00;

/// Offset of `CLK_*_DIV` inside a channel / 通道内 `CLK_*_DIV` 的偏移
const DIV_OFFSET: u32 = 0x04;

/// Offset of `CLK_*_SELECTED` inside a channel / 通道内 `CLK_*_SELECTED` 的偏移
const SELECTED_OFFSET: u32 = 0x08;

/// CLOCKS register block / CLOCKS 寄存器块
#[derive(Debug)]
pub struct Clocks {
    regs: RegisterFile,
    tree: ClockTree,
}

impl Clocks {
    /// Create the block in the bootrom-equivalent clock state
    /// 创建处于 bootrom 等价时钟状态的寄存器块
    #[must_use]
    pub fn new() -> Self {
        Self::with_tree(ClockTree::new())
    }

    /// Create the block on a shared clock tree / 在共享时钟树上创建寄存器块
    ///
    /// `BootPeripherals` uses this to mount `CLOCKS`, `XOSC`, both PLLs and
    /// `ROSC` on one tree.
    /// `BootPeripherals` 用它把 `CLOCKS`、`XOSC`、两个 PLL 与 `ROSC` 挂到同一
    /// 棵树。
    #[must_use]
    pub fn with_tree(tree: ClockTree) -> Self {
        Self {
            regs: RegisterFile::new(map::DEVICE_WINDOW_SIZE),
            tree,
        }
    }

    /// Shared clock tree / 共享时钟树
    #[must_use]
    pub fn tree(&self) -> &ClockTree {
        &self.tree
    }

    /// Split an offset into `(channel, sub-offset)` / 把偏移拆成 `(通道, 子偏移)`
    fn channel_of(offset: u32) -> Option<(usize, u32)> {
        let channel = offset / CHANNEL_STRIDE;
        let sub = offset % CHANNEL_STRIDE;
        // 0x78（DFTCLK_XOSC_CTRL）会整除步长，必须按通道数排除。
        // 0x78 (DFTCLK_XOSC_CTRL) divides evenly, so bound by the channel count.
        (channel < CLOCK_COUNT as u32).then_some((channel as usize, sub))
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
        if let Some((channel, sub)) = Self::channel_of(offset) {
            return match sub {
                CTRL_OFFSET => Some(self.tree.channel_ctrl(channel)),
                DIV_OFFSET => Some(self.tree.channel_div(channel)),
                SELECTED_OFFSET => Some(self.tree.channel_selected(channel)),
                _ => None,
            };
        }
        self.regs.read(offset)
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        if let Some((channel, sub)) = Self::channel_of(offset) {
            match sub {
                CTRL_OFFSET => {
                    self.tree.write_channel_ctrl(channel, value);
                    return true;
                }
                DIV_OFFSET => {
                    self.tree.write_channel_div(channel, value);
                    return true;
                }
                // SELECTED 为只读寄存器。SELECTED is read-only.
                SELECTED_OFFSET => return true,
                _ => return false,
            }
        }
        self.regs.write(offset, value)
    }

    fn reset(&mut self) {
        self.regs.reset();
        // 恢复 bootrom 等价状态（150 MHz），不是上电复位值。
        // Restore the bootrom-equivalent state (150 MHz), not power-on reset.
        self.tree.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock_tree::{CLK_PERI, CLK_REF, CLK_SYS, CLK_USB};

    /// Write one `CLK_*_CTRL` register / 写入一个 `CLK_*_CTRL` 寄存器
    fn write_ctrl(clocks: &mut Clocks, channel: u32, value: u32) {
        let offset = channel * CHANNEL_STRIDE + CTRL_OFFSET;
        assert!(clocks.write32(offset, value));
    }

    /// Write one `CLK_*_DIV` register / 写入一个 `CLK_*_DIV` 寄存器
    fn write_div(clocks: &mut Clocks, channel: u32, value: u32) {
        let offset = channel * CHANNEL_STRIDE + DIV_OFFSET;
        assert!(clocks.write32(offset, value));
    }

    #[test]
    fn selected_registers_report_the_bootrom_sources() {
        // bootrom 之后 CLK_REF 选择 XOSC（bit2），CLK_SYS 选择 AUX（bit1）。
        // After the bootrom CLK_REF selects XOSC (bit 2) and CLK_SYS AUX (bit 1).
        let mut clocks = Clocks::new();
        assert_eq!(clocks.read32(0x38), Some(0x4)); // CLK_REF_SELECTED
        assert_eq!(clocks.read32(0x44), Some(0x2)); // CLK_SYS_SELECTED
        assert_eq!(clocks.read32(0x50), Some(0x1)); // CLK_PERI_SELECTED
        assert_eq!(clocks.read32(0x68), Some(0x1)); // CLK_USB_SELECTED
        assert_eq!(clocks.read32(0x74), Some(0x1)); // CLK_ADC_SELECTED
    }

    #[test]
    fn ref_and_sys_selected_follow_the_src_field() {
        let mut clocks = Clocks::new();
        // SRC=1（辅助源）+ AUXSRC=1（GPIN0）。SRC=1 (aux) with AUXSRC=1.
        write_ctrl(&mut clocks, CLK_REF as u32, 0x0000_0021);
        assert_eq!(clocks.read32(0x38), Some(0x2));
        write_ctrl(&mut clocks, CLK_REF as u32, 2);
        assert_eq!(clocks.read32(0x38), Some(0x4));

        write_ctrl(&mut clocks, CLK_SYS as u32, 0);
        assert_eq!(clocks.read32(0x44), Some(0x1));
        write_ctrl(&mut clocks, CLK_SYS as u32, 0x01);
        assert_eq!(clocks.read32(0x44), Some(0x2));
    }

    #[test]
    fn auxiliary_only_slices_are_hardwired_to_one() {
        // 数据手册：没有无毛刺选择器的通道 SELECTED 恒为 0x1。
        // Datasheet: slices without a glitchless mux report SELECTED = 0x1.
        let mut clocks = Clocks::new();
        write_ctrl(&mut clocks, CLK_PERI as u32, 4 << 5);
        assert_eq!(clocks.read32(0x50), Some(0x1));
    }

    #[test]
    fn control_and_divider_registers_mask_reserved_bits() {
        let mut clocks = Clocks::new();
        // CLK_SYS_CTRL 只有 bit0 与 bit7:5 可写。
        // Only bits 0 and 7:5 of CLK_SYS_CTRL are writable.
        write_ctrl(&mut clocks, CLK_SYS as u32, 0xFFFF_FFFF);
        assert_eq!(clocks.read32(0x3C), Some(0xE1));
        // CLK_USB_DIV 是 4 位整数分频，低 16 位 FRAC 不存在。
        // CLK_USB_DIV is a 4-bit integer divider with no FRAC field.
        write_div(&mut clocks, CLK_USB as u32, 0xFFFF_FFFF);
        assert_eq!(clocks.read32(0x64), Some(0x000F_0000));
    }

    #[test]
    fn divider_writes_change_the_live_system_clock() {
        let mut clocks = Clocks::new();
        assert_eq!(clocks.tree().sysclk_hz(), 150_000_000);
        write_div(&mut clocks, CLK_SYS as u32, 2 << 16);
        assert_eq!(clocks.tree().sysclk_hz(), 75_000_000);
        // 切到 CLK_REF 并分频 3：12 MHz / 3 = 4 MHz。
        // Switch to CLK_REF with DIV=3: 12 MHz / 3 = 4 MHz.
        write_ctrl(&mut clocks, CLK_SYS as u32, 0);
        write_div(&mut clocks, CLK_SYS as u32, 3 << 16);
        assert_eq!(clocks.tree().sysclk_hz(), 4_000_000);
        // 切回 AUX/PLL_SYS 并恢复 DIV=1。
        // Back to AUX/PLL_SYS with DIV=1.
        write_ctrl(&mut clocks, CLK_SYS as u32, 0x01);
        write_div(&mut clocks, CLK_SYS as u32, 1 << 16);
        assert_eq!(clocks.tree().sysclk_hz(), 150_000_000);
    }

    #[test]
    fn selected_is_read_only_and_does_not_bump_generation() {
        let mut clocks = Clocks::new();
        let before = clocks.tree().generation();
        assert!(clocks.write32(0x44, 0xFFFF_FFFF));
        assert_eq!(clocks.read32(0x44), Some(0x2));
        assert_eq!(clocks.tree().generation(), before);
        // FC0_REF_KHZ 等非频率寄存器也不影响世代号。
        // Non-frequency registers such as FC0_REF_KHZ do not bump either.
        assert!(clocks.write32(0x8C, 12_000));
        assert_eq!(clocks.tree().generation(), before);
    }

    #[test]
    fn reset_restores_the_post_bootrom_clock() {
        let mut clocks = Clocks::new();
        write_ctrl(&mut clocks, CLK_SYS as u32, 0);
        write_div(&mut clocks, CLK_SYS as u32, 4 << 16);
        assert_eq!(clocks.tree().sysclk_hz(), 3_000_000);
        clocks.reset();
        assert_eq!(clocks.tree().sysclk_hz(), 150_000_000);
        assert_eq!(clocks.read32(0x44), Some(0x2));
        assert_eq!(clocks.read32(0x38), Some(0x4));
        assert_eq!(clocks.read32(0x3C), Some(0x01));
    }
}
