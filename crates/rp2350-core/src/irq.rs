//! Interrupt routing matrix for RP2350
//! RP2350 中断路由矩阵
//!
//! RP2350 supports up to 256 IRQs routed to two Cortex-M33 cores.
//! Each IRQ can be enabled, pending, and optionally routed to a specific core.
//! RP2350 支持最多 256 个中断，可路由到两个 Cortex-M33 核心。
//! 每个中断可被使能、挂起，并可选择性路由到指定核心。

use super::types::{IrqNumber, IRQ_COUNT};

/// Interrupt routing matrix
/// 中断路由矩阵
///
/// Manages pending status and enable state for all NVIC interrupts.
/// 管理所有 NVIC 中断的挂起和使能状态。
pub struct IrqMatrix {
    /// Pending status for each IRQ / 各中断的挂起状态
    pending: [bool; IRQ_COUNT],
    /// Enable status for each IRQ / 各中断的使能状态
    enabled: [bool; IRQ_COUNT],
    /// Priority for each IRQ (0-255, lower = higher priority) / 各中断优先级（0-255，越小优先级越高）
    priority: [u8; IRQ_COUNT],
}

impl IrqMatrix {
    /// Create a new IRQ matrix (all interrupts disabled and not pending)
    /// 创建新的中断矩阵（所有中断初始禁用且无挂起）
    #[must_use]
    pub fn new() -> Self {
        Self {
            pending: [false; IRQ_COUNT],
            enabled: [false; IRQ_COUNT],
            priority: [0; IRQ_COUNT],
        }
    }

    /// Check if IRQ `n` is pending (latched)
    /// 检查中断 n 是否已挂起（已锁存）
    #[must_use]
    pub fn is_pending(&self, irq: IrqNumber) -> bool {
        self.pending[irq as usize]
    }

    /// Check if IRQ `n` is enabled
    /// 检查中断 n 是否已使能
    #[must_use]
    pub fn is_enabled(&self, irq: IrqNumber) -> bool {
        self.enabled[irq as usize]
    }

    /// Check if IRQ `n` is both pending and enabled (active)
    /// 检查中断 n 是否同时挂起且使能（活跃）
    #[must_use]
    pub fn is_active(&self, irq: IrqNumber) -> bool {
        self.is_enabled(irq) && self.is_pending(irq)
    }

    /// Set IRQ `n` as pending / 将中断 n 设为挂起
    pub fn set_pending(&mut self, irq: IrqNumber) {
        self.pending[irq as usize] = true;
    }

    /// Clear pending status for IRQ `n` / 清除中断 n 的挂起状态
    pub fn clear_pending(&mut self, irq: IrqNumber) {
        self.pending[irq as usize] = false;
    }

    /// Enable IRQ `n` / 使能中断 n
    pub fn enable(&mut self, irq: IrqNumber) {
        self.enabled[irq as usize] = true;
    }

    /// Disable IRQ `n` / 禁用中断 n
    pub fn disable(&mut self, irq: IrqNumber) {
        self.enabled[irq as usize] = false;
    }

    /// Set priority for IRQ `n` / 设置中断 n 的优先级
    pub fn set_priority(&mut self, irq: IrqNumber, pri: u8) {
        self.priority[irq as usize] = pri;
    }

    /// Get priority for IRQ `n` / 获取中断 n 的优先级
    #[must_use]
    pub fn priority(&self, irq: IrqNumber) -> u8 {
        self.priority[irq as usize]
    }

    /// Find the highest-priority active IRQ for a given core
    /// 为指定核心查找最高优先级活跃中断
    #[must_use]
    pub fn find_highest_active(&self) -> Option<IrqNumber> {
        (0..IRQ_COUNT as u16)
            .filter(|&irq| self.is_active(irq))
            .min_by_key(|&irq| self.priority[irq as usize])
    }
}

impl Default for IrqMatrix {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_matrix_all_clear() {
        let matrix = IrqMatrix::new();
        assert!(!matrix.is_pending(5));
        assert!(!matrix.is_enabled(5));
    }

    #[test]
    fn set_and_clear_pending() {
        let mut matrix = IrqMatrix::new();
        matrix.set_pending(42);
        assert!(matrix.is_pending(42));
        matrix.clear_pending(42);
        assert!(!matrix.is_pending(42));
    }

    #[test]
    fn active_only_when_enabled_and_pending() {
        let mut matrix = IrqMatrix::new();
        matrix.set_pending(10);
        assert!(!matrix.is_active(10)); // not enabled
        matrix.enable(10);
        assert!(matrix.is_active(10)); // now active
    }

    #[test]
    fn highest_active_by_priority() {
        let mut matrix = IrqMatrix::new();
        // IRQ 5: priority 200, enabled, pending
        matrix.set_priority(5, 200);
        matrix.enable(5);
        matrix.set_pending(5);
        // IRQ 7: priority 100 (higher), enabled, pending
        matrix.set_priority(7, 100);
        matrix.enable(7);
        matrix.set_pending(7);

        assert_eq!(matrix.find_highest_active(), Some(7));
    }
}