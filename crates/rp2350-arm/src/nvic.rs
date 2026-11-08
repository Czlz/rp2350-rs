//! NVIC — Nested Vectored Interrupt Controller (ARMv8-M)
//! NVIC — 嵌套向量中断控制器（ARMv8-M）
//!
//! Implements ISER, ICER, ISPR, ICPR, IPR, and STIR registers.
//! Supports 256 interrupts with configurable priority levels.
//! 实现 ISER、ICER、ISPR、ICPR、IPR 和 STIR 寄存器。
//! 支持 256 个中断，具有可配置的优先级级别。

// NVIC 使用 ARM 定义的首字母缩写词命名寄存器，文档标记反映了硬件参考手册中的命名约定。
// NVIC uses ARM-defined acronyms for register names; doc markup reflects naming
// conventions from the hardware reference manual.
#![allow(
    clippy::doc_markdown,
    clippy::unusual_byte_groupings,
    clippy::unreadable_literal,
    clippy::module_name_repetitions
)]

use rp2350_core::types::IrqNumber;

/// NVIC ISER alias offset stride (ARM uses 32 per register)
/// NVIC ISER 别名偏移步长（ARM 每个寄存器 32 个）
const NVIC_REGISTER_STRIDE: usize = 32;

/// Number of NVIC priority bits implemented
/// 实现的 NVIC 优先级位数
const PRIORITY_BITS: u8 = 3;

/// Nested Vectored Interrupt Controller for Cortex-M33
/// Cortex-M33 的嵌套向量中断控制器
#[derive(Debug, Clone)]
pub struct Nvic {
    /// Interrupt Set-Enable Registers (ISER0..ISER7 for 256 IRQs)
    /// 中断置位使能寄存器
    iser: [u32; 8],
    /// Interrupt Clear-Enable Registers (ICER0..ICER7 for 256 IRQs)
    /// 中断清除使能寄存器
    icer: [u32; 8],
    /// Interrupt Set-Pending Registers (ISPR0..ISPR7 for 256 IRQs)
    /// 中断置位挂起寄存器
    ispr: [u32; 8],
    /// Interrupt Clear-Pending Registers (ICPR0..ICPR7 for 256 IRQs)
    /// 中断清除挂起寄存器
    icpr: [u32; 8],
    /// Interrupt Active Bit Registers (IABR0..IABR7 for 256 IRQs)
    /// 中断活跃位寄存器
    iabr: [u32; 8],
    /// Interrupt Priority Registers (IPR0..IPR63, 8-bit each)
    /// 中断优先级寄存器
    ipr: [u8; 256],
}

impl Nvic {
    /// Create a new NVIC in reset state (all interrupts disabled, no pending)
    /// 创建处于复位状态的 NVIC（所有中断禁用，无挂起）
    #[must_use]
    pub fn new() -> Self {
        Self {
            iser: [0; 8],
            icer: [0; 8],
            ispr: [0; 8],
            icpr: [0; 8],
            iabr: [0; 8],
            ipr: [0; 256],
        }
    }

    /// Check if an interrupt is enabled
    /// 检查中断是否已使能
    #[must_use]
    pub fn is_enabled(&self, irq: IrqNumber) -> bool {
        let idx = (irq as usize) / NVIC_REGISTER_STRIDE;
        let bit = (irq as usize) % NVIC_REGISTER_STRIDE;
        (self.iser[idx] >> bit) & 1 != 0
    }

    /// Check if an interrupt is pending
    /// 检查中断是否挂起
    #[must_use]
    pub fn is_pending(&self, irq: IrqNumber) -> bool {
        let idx = (irq as usize) / NVIC_REGISTER_STRIDE;
        let bit = (irq as usize) % NVIC_REGISTER_STRIDE;
        (self.ispr[idx] >> bit) & 1 != 0
    }

    /// Check if an interrupt is active
    /// 检查中断是否活跃
    #[must_use]
    pub fn is_active(&self, irq: IrqNumber) -> bool {
        let idx = (irq as usize) / NVIC_REGISTER_STRIDE;
        let bit = (irq as usize) % NVIC_REGISTER_STRIDE;
        (self.iabr[idx] >> bit) & 1 != 0
    }

    /// Get the priority of an interrupt
    /// 获取中断优先级
    #[must_use]
    pub fn get_priority(&self, irq: IrqNumber) -> u8 {
        self.ipr[irq as usize] >> (8 - PRIORITY_BITS)
    }

    /// Set the priority of an interrupt
    /// 设置中断优先级
    pub fn set_priority(&mut self, irq: IrqNumber, priority: u8) {
        self.ipr[irq as usize] = priority << (8 - PRIORITY_BITS);
    }

    /// Read ISER value (word at 4-byte offset)
    /// 读取 ISER 值（4 字节偏移处的字）
    #[must_use]
    pub fn read_iser(&self, index: usize) -> u32 {
        self.iser.get(index).copied().unwrap_or(0)
    }

    /// Read an IPR word (4 IRQs packed into one u32)
    /// 读取 IPR 字（4 个 IRQ 打包到一个 u32 中）
    #[must_use]
    pub fn read_ipr_word(&self, index: usize) -> u32 {
        let base = index * 4;
        let mut word: u32 = 0;
        for i in 0..4 {
            if base + i < 256 {
                word |= (self.ipr[base + i] as u32) << (i * 8);
            }
        }
        word
    }

    /// Write an IPR word (4 IRQs packed into one u32)
    /// 写入 IPR 字（4 个 IRQ 打包到一个 u32 中）
    pub fn write_ipr_word(&mut self, index: usize, value: u32) {
        let base = index * 4;
        for i in 0..4 {
            if base + i < 256 {
                self.ipr[base + i] = ((value >> (i * 8)) & 0xFF) as u8;
            }
        }
    }

    /// Write ISER value — SETs bits (writing 1 sets, writing 0 has no effect)
    /// 写入 ISER 值 — SET 位（写 1 置位，写 0 无效果）
    pub fn write_iser(&mut self, index: usize, value: u32) {
        if let Some(reg) = self.iser.get_mut(index) {
            // ISER is write-1-to-set (reads return current enable state)
            // ISER 是写 1 置位（读返回当前使能状态）
            *reg |= value;
        }
    }

    /// Read ICER value (word at 4-byte offset)
    /// 读取 ICER 值（4 字节偏移处的字）
    #[must_use]
    pub fn read_icer(&self, index: usize) -> u32 {
        self.icer.get(index).copied().unwrap_or(0)
    }

    /// Write ICER value — CLEARs bits (writing 1 clears, writing 0 has no effect)
    /// 写入 ICER 值 — CLEAR 位（写 1 清除，写 0 无效果）
    pub fn write_icer(&mut self, index: usize, value: u32) {
        if let Some(reg) = self.icer.get_mut(index) {
            *reg = value;
        }
        if let Some(en) = self.iser.get_mut(index) {
            // ICER is write-1-to-clear
            // ICER 是写 1 清除
            *en &= !value;
        }
    }

    /// Read ISPR value (word at 4-byte offset)
    /// 读取 ISPR 值（4 字节偏移处的字）
    #[must_use]
    pub fn read_ispr(&self, index: usize) -> u32 {
        self.ispr.get(index).copied().unwrap_or(0)
    }

    /// Write ISPR value — SETs pending bits (write-1-to-set)
    /// 写入 ISPR 值 — SET 挂起位（写 1 置位）
    pub fn write_ispr(&mut self, index: usize, value: u32) {
        if let Some(reg) = self.ispr.get_mut(index) {
            *reg |= value;
        }
    }

    /// Read ICPR value (word at 4-byte offset)
    /// 读取 ICPR 值（4 字节偏移处的字）
    #[must_use]
    pub fn read_icpr(&self, index: usize) -> u32 {
        self.icpr.get(index).copied().unwrap_or(0)
    }

    /// Write ICPR value — CLEARs pending bits (write-1-to-clear)
    /// 写入 ICPR 值 — CLEAR 挂起位（写 1 清除）
    pub fn write_icpr(&mut self, index: usize, value: u32) {
        if let Some(reg) = self.icpr.get_mut(index) {
            *reg = value;
        }
        if let Some(pend) = self.ispr.get_mut(index) {
            *pend &= !value;
        }
    }

    /// Read IABR value (word at 4-byte offset)
    /// 读取 IABR 值（4 字节偏移处的字）
    #[must_use]
    pub fn read_iabr(&self, index: usize) -> u32 {
        self.iabr.get(index).copied().unwrap_or(0)
    }

    /// Set an interrupt active
    /// 设置中断为活跃
    pub fn set_active(&mut self, irq: IrqNumber) {
        let idx = (irq as usize) / NVIC_REGISTER_STRIDE;
        let bit = (irq as usize) % NVIC_REGISTER_STRIDE;
        if let Some(reg) = self.iabr.get_mut(idx) {
            *reg |= 1 << bit;
        }
    }

    /// Clear an interrupt active
    /// 清除中断活跃状态
    pub fn clear_active(&mut self, irq: IrqNumber) {
        let idx = (irq as usize) / NVIC_REGISTER_STRIDE;
        let bit = (irq as usize) % NVIC_REGISTER_STRIDE;
        if let Some(reg) = self.iabr.get_mut(idx) {
            *reg &= !(1 << bit);
        }
    }

    /// Set interrupt pending via hardware (external event)
    /// 通过硬件设置中断挂起（外部事件）
    pub fn set_pending(&mut self, irq: IrqNumber) {
        let idx = (irq as usize) / NVIC_REGISTER_STRIDE;
        let bit = (irq as usize) % NVIC_REGISTER_STRIDE;
        if let Some(reg) = self.ispr.get_mut(idx) {
            *reg |= 1 << bit;
        }
    }

    /// Clear interrupt pending
    /// 清除中断挂起
    pub fn clear_pending(&mut self, irq: IrqNumber) {
        let idx = (irq as usize) / NVIC_REGISTER_STRIDE;
        let bit = (irq as usize) % NVIC_REGISTER_STRIDE;
        if let Some(reg) = self.ispr.get_mut(idx) {
            *reg &= !(1 << bit);
        }
    }

    /// Software Trigger Interrupt Register — trigger an IRQ via software
    /// 软件触发中断寄存器 — 通过软件触发 IRQ
    pub fn write_stir(&mut self, irq: IrqNumber) {
        if irq < 256 {
            self.set_pending(irq);
        }
    }

    /// Find the highest-priority pending enabled interrupt
    /// 查找最高优先级的挂起已使能中断
    ///
    /// Returns `None` if no interrupt is pending and enabled.
    /// 如果没有挂起且已使能的中断，返回 `None`。
    #[must_use]
    pub fn find_highest_priority(&self) -> Option<IrqNumber> {
        let mut best_irq: Option<IrqNumber> = None;
        let mut best_prio: u8 = 0xFF;

        for irq in 0u16..256 {
            if self.is_enabled(irq) && self.is_pending(irq) {
                let prio = self.get_priority(irq);
                // Lower number = higher priority
                // 数字越小 = 优先级越高
                if prio < best_prio {
                    best_prio = prio;
                    best_irq = Some(irq);
                }
            }
        }

        best_irq
    }
}

impl Default for Nvic {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_nvic_all_disabled() {
        let nvic = Nvic::new();
        assert!(!nvic.is_enabled(5));
        assert!(!nvic.is_pending(5));
    }

    #[test]
    fn set_pending_and_enable() {
        let mut nvic = Nvic::new();
        nvic.write_iser(0, 1 << 5); // Enable IRQ5
        assert!(nvic.is_enabled(5));
        assert!(!nvic.is_pending(5));

        nvic.set_pending(5);
        assert!(nvic.is_pending(5));
        assert_eq!(nvic.find_highest_priority(), Some(5));
    }

    #[test]
    fn priority_routing() {
        let mut nvic = Nvic::new();
        nvic.write_iser(0, (1 << 3) | (1 << 7));
        nvic.set_pending(3);
        nvic.set_pending(7);
        nvic.set_priority(3, 2);
        nvic.set_priority(7, 1); // Lower number = higher priority

        assert_eq!(nvic.find_highest_priority(), Some(7));
    }

    #[test]
    fn clear_pending() {
        let mut nvic = Nvic::new();
        nvic.write_iser(0, 1 << 10);
        nvic.set_pending(10);
        assert!(nvic.is_pending(10));
        nvic.write_icpr(0, 1 << 10);
        assert!(!nvic.is_pending(10));
        assert_eq!(nvic.find_highest_priority(), None);
    }

    #[test]
    fn stir_triggers_pending() {
        let mut nvic = Nvic::new();
        nvic.write_iser(0, 1 << 15);
        nvic.write_stir(15);
        assert!(nvic.is_pending(15));
    }
}
