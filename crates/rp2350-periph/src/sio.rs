//! SIO peripheral stub / SIO 外设桩
//!
//! Models the single-cycle GPIO output, output-enable, input, high-bank and
//! spinlock registers. GPIO values are looped back as inputs; the pin matrix
//! between SIO, IO_BANK0 and PADS_BANK0 is not connected in Stage 4.
//! 建模单周期 GPIO 输出、输出使能、输入、高半区与自旋锁寄存器。GPIO 值回环为
//! 输入；Stage 4 尚未连接 SIO、IO_BANK0 与 PADS_BANK0 之间的引脚矩阵。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// `GPIO_OUT` / `GPIO_OUT`
const GPIO_OUT: u32 = 0x10;

/// `GPIO_OUT_SET` / `GPIO_OUT_SET`
const GPIO_OUT_SET: u32 = 0x14;

/// `GPIO_OUT_CLR` / `GPIO_OUT_CLR`
const GPIO_OUT_CLR: u32 = 0x18;

/// `GPIO_OUT_XOR` / `GPIO_OUT_XOR`
const GPIO_OUT_XOR: u32 = 0x1C;

/// `GPIO_OE` / `GPIO_OE`
const GPIO_OE: u32 = 0x20;

/// `GPIO_OE_SET` / `GPIO_OE_SET`
const GPIO_OE_SET: u32 = 0x24;

/// `GPIO_OE_CLR` / `GPIO_OE_CLR`
const GPIO_OE_CLR: u32 = 0x28;

/// `GPIO_OE_XOR` / `GPIO_OE_XOR`
const GPIO_OE_XOR: u32 = 0x2C;

/// `GPIO_IN` / `GPIO_IN`
const GPIO_IN: u32 = 0x30;

/// `GPIO_HI_OUT` / `GPIO_HI_OUT`
const GPIO_HI_OUT: u32 = 0x34;

/// SIO register block / SIO 寄存器块
#[derive(Debug)]
pub struct Sio {
    regs: RegisterFile,
}

impl Sio {
    /// Create the block with all pins low and inputs / 创建所有引脚为低电平输入的寄存器块
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: RegisterFile::new(map::SIO_WINDOW_SIZE),
        }
    }

    /// Update a value register with set/clear/xor semantics
    /// 按置位/清零/异或语义更新值寄存器
    fn update(&mut self, value_offset: u32, patch: u32, mode: Patch) -> bool {
        let current = self.regs.read(value_offset).unwrap_or(0);
        let updated = match mode {
            Patch::Set => current | patch,
            Patch::Clear => current & !patch,
            Patch::Xor => current ^ patch,
        };
        self.regs.write(value_offset, updated)
    }
}

impl Default for Sio {
    fn default() -> Self {
        Self::new()
    }
}

/// Atomic register update mode / 原子寄存器更新模式
#[derive(Debug, Clone, Copy)]
enum Patch {
    /// Bitwise OR / 按位或
    Set,
    /// Bitwise AND-NOT / 按位与非
    Clear,
    /// Bitwise XOR / 按位异或
    Xor,
}

impl MmioDevice for Sio {
    fn name(&self) -> &str {
        "SIO"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::SIO_BASE, map::SIO_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        match offset {
            // GPIO_IN loops the output value back. GPIO_IN 将输出值回环。
            GPIO_IN => self.regs.read(GPIO_OUT),
            // FIFO_ST: ready, valid, not full. FIFO_ST：就绪、有效、未满。
            0x50 => Some(0b0000_0010),
            // FIFO_RD: no data. FIFO_RD：无数据。
            0x58 => Some(0),
            // SPINLOCK_ST: all 32 spinlocks free. SPINLOCK_ST：全部 32 个自旋锁空闲。
            0x5C => Some(0xFFFF_FFFF),
            _ => self.regs.read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        match offset {
            GPIO_OUT_SET => self.update(GPIO_OUT, value, Patch::Set),
            GPIO_OUT_CLR => self.update(GPIO_OUT, value, Patch::Clear),
            GPIO_OUT_XOR => self.update(GPIO_OUT, value, Patch::Xor),
            GPIO_OE_SET => self.update(GPIO_OE, value, Patch::Set),
            GPIO_OE_CLR => self.update(GPIO_OE, value, Patch::Clear),
            GPIO_OE_XOR => self.update(GPIO_OE, value, Patch::Xor),
            // HI aliases share the 32-bit value in this stub. HI 别名在桩中共用 32 位值。
            GPIO_HI_OUT => self.regs.write(GPIO_OUT, value),
            // FIFO writes are discarded. FIFO 写入被丢弃。
            0x54 | 0x58 => true,
            _ => self.regs.write(offset, value),
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
    fn set_clear_xor_updates_output() {
        let mut sio = Sio::new();
        assert!(sio.write32(GPIO_OUT_SET, 0b1010));
        assert_eq!(sio.read32(GPIO_OUT), Some(0b1010));
        assert!(sio.write32(GPIO_OUT_CLR, 0b0010));
        assert_eq!(sio.read32(GPIO_OUT), Some(0b1000));
        assert!(sio.write32(GPIO_OUT_XOR, 0b1111));
        assert_eq!(sio.read32(GPIO_OUT), Some(0b0111));
        assert_eq!(sio.read32(GPIO_IN), Some(0b0111));
    }
}
