//! PSM (power state machine) register block / PSM（电源状态机）寄存器块
//!
//! The RP2350 PSM controls which peripherals and processor cores are forced on
//! or off. Firmware uses the `FRCE_ON` / `FRCE_OFF` registers to power core 1
//! down during a soft reset and then polls the register until the block reports
//! the requested state. This model keeps the force bits as ordinary storage and
//! derives `DONE` from them, so polling loops make progress instead of spinning
//! on an unimplemented address.
//! RP2350 PSM 控制哪些外设与处理器核心被强制开启或关闭。固件在软复位期间用
//! `FRCE_ON` / `FRCE_OFF` 关闭 core1，然后轮询该寄存器直到目标块报告所需状态。
//! 本模型把强制位实现为普通存储，并由它们推导 `DONE`，使轮询循环能够推进，
//! 而不是在未实现地址上空转。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// `PSM_FRCE_OFF` bit for core 1 / `PSM_FRCE_OFF` 的 core1 位
const PROC1: u32 = 0x0100_0000;

/// Offset of `PSM_FRCE_ON` / `PSM_FRCE_ON` 偏移
const FRCE_ON: u32 = 0x00;
/// Offset of `PSM_FRCE_OFF` / `PSM_FRCE_OFF` 偏移
const FRCE_OFF: u32 = 0x04;
/// Offset of `PSM_WDSEL` / `PSM_WDSEL` 偏移
const WDSEL: u32 = 0x08;
/// Offset of `PSM_DONE` / `PSM_DONE` 偏移
const DONE: u32 = 0x0C;

/// Bits covering the modelled power domains / 已建模电源域的位掩码
const DOMAIN_MASK: u32 = 0x01FF_FFFF;

/// State reported by `DONE` when nothing is forced off
/// 没有电源域被强制关闭时 `DONE` 报告的状态
const ALL_POWERED: u32 = DOMAIN_MASK;

/// A modelled block inside the PSM register file
/// PSM 寄存器文件中的一个已建模块
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Block {
    /// Force a domain on / 强制开启某个域
    ForceOn,
    /// Force a domain off / 强制关闭某个域
    ForceOff,
    /// Watchdog selection / 看门狗选择
    Watchdog,
}

/// PSM register block / PSM 寄存器块
#[derive(Debug)]
pub struct Psm {
    regs: RegisterFile,
    core1_release: Arc<AtomicBool>,
}

impl Psm {
    /// Create the block with reset values / 用复位值创建寄存器块
    #[must_use]
    pub fn new() -> Self {
        Self::with_core1_release(Arc::new(AtomicBool::new(false)))
    }

    /// Create the block with a shared core-1 release flag
    /// 创建寄存器块并共享 core1 释放标志
    ///
    /// The simulation driver observes this flag to emulate the core-1 boot
    /// ROM's reset handshake (drain the mailbox and push `0` back to core 0).
    /// 仿真驱动器观察该标志，模拟 core1 boot ROM 的复位握手（清空信箱并向
    /// core0 回送 `0`）。
    #[must_use]
    pub fn with_core1_release(flag: Arc<AtomicBool>) -> Self {
        Self {
            regs: RegisterFile::new(map::DEVICE_WINDOW_SIZE),
            core1_release: flag,
        }
    }

    /// Decode a register offset into a modelled block
    /// 将寄存器偏移译码为已建模块
    fn block(offset: u32) -> Option<Block> {
        match offset {
            FRCE_ON => Some(Block::ForceOn),
            FRCE_OFF => Some(Block::ForceOff),
            WDSEL => Some(Block::Watchdog),
            _ => None,
        }
    }

    /// Current force-on bits / 当前强制开启位
    fn force_on(&self) -> u32 {
        self.regs.read(FRCE_ON).unwrap_or(0) & DOMAIN_MASK
    }

    /// Current force-off bits / 当前强制关闭位
    fn force_off(&self) -> u32 {
        self.regs.read(FRCE_OFF).unwrap_or(0) & DOMAIN_MASK
    }

    /// Power-state snapshot reported by `DONE` / `DONE` 报告的电源状态快照
    fn done(&self) -> u32 {
        // Always-on domains stay powered unless explicitly forced off. Force-on
        // bits can bring a domain back after force-off is cleared.
        // 常开域默认保持上电，除非被显式强制关闭。强制开启位可在强制关闭解除后
        // 重新给域上电。
        (ALL_POWERED | self.force_on()) & !self.force_off()
    }
}

impl Default for Psm {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for Psm {
    fn name(&self) -> &str {
        "PSM"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::PSM_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        if offset == DONE {
            return Some(self.done());
        }
        if Self::block(offset).is_some() {
            return self.regs.read(offset);
        }
        None
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        match Self::block(offset) {
            // `DONE` is a status register; writes have no effect but are
            // accepted so firmware polling cannot stall on a rejected store.
            // `DONE` 是状态寄存器；写入无效果，但接受写请求，避免固件轮询因
            // 存储被拒而停滞。
            Some(Block::ForceOn | Block::ForceOff | Block::Watchdog) => {
                let was_off = self.force_off();
                let accepted = self.regs.write(offset, value & DOMAIN_MASK);
                if offset == FRCE_OFF && was_off & PROC1 != 0 && self.force_off() & PROC1 == 0 {
                    // Clearing `FRCE_OFF.PROC1` releases core 1 from reset. Its
                    // boot ROM then drains the mailbox and sends `0` to core 0.
                    // 清除 `FRCE_OFF.PROC1` 会让 core1 退出复位，其 boot ROM
                    // 随后清空信箱并向 core0 发送 `0`。
                    self.core1_release.store(true, Ordering::Release);
                }
                accepted
            }
            _ if offset == DONE => true,
            _ => false,
        }
    }

    fn reset(&mut self) {
        self.regs.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_off_reports_the_requested_domain() {
        // `multicore_reset_core1` sets `PSM_PROC1` through the SET alias and
        // waits for `FRCE_OFF` to read back the bit.
        // `multicore_reset_core1` 通过 SET 别名设置 `PSM_PROC1`，并等待
        // `FRCE_OFF` 回读该位。
        let mut psm = Psm::new();
        assert!(psm.write32(FRCE_OFF, 0x0100_0000));
        assert_eq!(psm.read32(FRCE_OFF), Some(0x0100_0000));
        assert_eq!(psm.done() & 0x0100_0000, 0);
    }

    #[test]
    fn clearing_force_off_powers_the_domain_again() {
        let mut psm = Psm::new();
        psm.write32(FRCE_OFF, DOMAIN_MASK);
        assert_eq!(psm.done(), 0, "every domain is forced off");
        psm.write32(FRCE_OFF, 0);
        assert_eq!(psm.done(), ALL_POWERED);
    }

    #[test]
    fn done_write_is_ignored_but_accepted() {
        let mut psm = Psm::new();
        assert!(psm.write32(DONE, 0));
        assert_eq!(psm.read32(DONE), Some(ALL_POWERED));
    }

    #[test]
    fn core1_release_flag_tracks_force_off_transition() {
        let flag = Arc::new(AtomicBool::new(false));
        let mut psm = Psm::with_core1_release(Arc::clone(&flag));
        psm.write32(FRCE_OFF, PROC1);
        assert!(!flag.load(Ordering::Acquire));
        psm.write32(FRCE_OFF, 0);
        assert!(flag.load(Ordering::Acquire));
        psm.write32(FRCE_OFF, 0);
        assert!(
            flag.load(Ordering::Acquire),
            "flag stays latched until taken"
        );
    }

    #[test]
    fn unmodelled_offsets_are_rejected() {
        let mut psm = Psm::new();
        assert!(!psm.write32(0x10, 1));
        assert_eq!(psm.read32(0x10), None);
    }
}
