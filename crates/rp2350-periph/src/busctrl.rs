//! BUSCTRL register block / BUSCTRL 寄存器块
//!
//! Models the parts of `BUSCTRL` (`0x4006_8000`) that firmware programs during
//! bring-up: the DMA/core bus priority hints and the performance-counter
//! enables. The priority write is reported as acknowledged immediately, which
//! matches silicon once the arbiter has latched the new levels; counters
//! themselves are not accumulated because the model has no bus stall events.
//! 建模固件在启动时写入的 `BUSCTRL`（`0x4006_8000`）部分：DMA/核心总线优先级
//! 提示与性能计数器使能。优先级写入立即报告已生效，与仲裁器锁存新级别后的芯片
//! 行为一致；模型没有总线停顿事件，因此不累加计数器本身。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use crate::alias::{apply_alias, split_alias};
use crate::map;

/// `BUS_PRIORITY` offset / `BUS_PRIORITY` 偏移
pub const BUS_PRIORITY: u32 = 0x00;

/// `BUS_PRIORITY_ACK` offset / `BUS_PRIORITY_ACK` 偏移
pub const BUS_PRIORITY_ACK: u32 = 0x04;

/// `PERFCTR_EN` offset / `PERFCTR_EN` 偏移
pub const PERFCTR_EN: u32 = 0x08;

/// `DMA_W_PRIORITY` field mask / `DMA_W_PRIORITY` 字段掩码
pub const DMA_W_PRIORITY_MASK: u32 = 0x1;

/// `DMA_R_PRIORITY` field mask / `DMA_R_PRIORITY` 字段掩码
pub const DMA_R_PRIORITY_MASK: u32 = 0x2;

/// `BUSCTRL` register window size (4 KB) / `BUSCTRL` 寄存器窗口大小（4 KB）
pub const BUSCTRL_SIZE: usize = 0x1000;

/// BUSCTRL block / BUSCTRL 寄存器块
#[derive(Debug, Clone)]
pub struct BusCtrl {
    regs: RegisterFile,
}

impl BusCtrl {
    /// Create the block with the reset register values / 创建使用复位值的寄存器块
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: RegisterFile::new(BUSCTRL_SIZE),
        }
    }

    /// Current programmed bus priority / 当前配置的总线优先级
    #[must_use]
    pub fn priority(&self) -> u32 {
        self.regs.read(BUS_PRIORITY).unwrap_or(0)
    }
}

impl Default for BusCtrl {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for BusCtrl {
    fn name(&self) -> &str {
        "BUSCTRL"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::BUSCTRL_BASE, BUSCTRL_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        let (offset, _alias) = split_alias(offset);
        match offset {
            // The arbiter latches a priority write immediately in this model.
            // 本模型中仲裁器立即锁存优先级写入。
            BUS_PRIORITY_ACK => Some(1),
            BUS_PRIORITY => self.regs.read(BUS_PRIORITY),
            offset => self.regs.read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        let (offset, alias) = split_alias(offset);
        if offset == BUS_PRIORITY_ACK {
            return true;
        }
        if offset == BUS_PRIORITY {
            let merged = apply_alias(self.regs.read(BUS_PRIORITY).unwrap_or(0), value, alias);
            let _ = self.regs.write(BUS_PRIORITY, merged & 0x3);
            true
        } else {
            let merged = apply_alias(self.regs.read(offset).unwrap_or(0), value, alias);
            self.regs.write(offset, merged)
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_write_is_acknowledged() {
        let mut block = BusCtrl::new();
        assert_eq!(block.read32(BUS_PRIORITY), Some(0));
        assert_eq!(block.read32(BUS_PRIORITY_ACK), Some(1));
        assert!(block.write32(BUS_PRIORITY, DMA_W_PRIORITY_MASK | DMA_R_PRIORITY_MASK));
        assert_eq!(block.priority(), 0x3);
        assert_eq!(block.read32(BUS_PRIORITY), Some(0x3));
        assert_eq!(block.read32(BUS_PRIORITY_ACK), Some(1));
    }

    #[test]
    fn alias_windows_update_the_priority_register() {
        let mut block = BusCtrl::new();
        assert!(block.write32(BUS_PRIORITY + 0x2000, DMA_W_PRIORITY_MASK));
        assert_eq!(block.priority(), DMA_W_PRIORITY_MASK);
        assert!(block.write32(BUS_PRIORITY + 0x3000, DMA_W_PRIORITY_MASK));
        assert_eq!(block.priority(), 0);
    }

    #[test]
    fn perfctr_enable_is_stored() {
        let mut block = BusCtrl::new();
        assert!(block.write32(PERFCTR_EN, 0xF));
        assert_eq!(block.read32(PERFCTR_EN), Some(0xF));
        block.reset();
        assert_eq!(block.read32(PERFCTR_EN), Some(0));
    }
}
