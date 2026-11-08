//! Peripheral interrupt assertions / 外设中断断言
//!
//! Peripherals share an [`IrqAssert`] with [`crate::BootPeripherals`] so the
//! simulation driver can poll which interrupt lines are currently asserted and
//! forward them to the CPU. The set covers IRQ numbers `0..=63`, which is wider
//! than every Stage-5 line (TIMER0 `0..=3`, USBCTRL `14`, SIO `25/26`, UART0 `33`).
//! 外设与 [`crate::BootPeripherals`] 共享 [`IrqAssert`]，仿真驱动据此轮询当前
//! 断言的中断线并转发给 CPU。该集合覆盖 IRQ `0..=63`，宽于 Stage 5 的所有
//! 中断线（TIMER0 `0..=3`、USBCTRL `14`、SIO `25/26`、UART0 `33`）。

use std::sync::Arc;

use parking_lot::Mutex;

/// Highest IRQ number representable in the shared mask / 共享掩码可表示的最大 IRQ 号
pub const MAX_IRQ: u16 = 63;

/// Shared set of asserted interrupt lines / 共享的中断线断言集合
#[derive(Debug, Clone, Default)]
pub struct IrqAssert {
    /// Bit `n` is set when IRQ `n` is asserted. 第 `n` 位置位表示 IRQ `n` 已断言。
    mask: Arc<Mutex<u64>>,
}

impl IrqAssert {
    /// Create an empty assertion set / 创建空的断言集合
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Assert or release one interrupt line / 断言或释放一条中断线
    ///
    /// IRQ numbers above [`MAX_IRQ`] are ignored; they cannot be represented in
    /// the shared mask.
    /// 超过 [`MAX_IRQ`] 的中断号会被忽略，因为共享掩码无法表示。
    pub fn set(&self, irq: u16, asserted: bool) {
        if irq > MAX_IRQ {
            return;
        }
        let bit = 1u64 << irq;
        if asserted {
            *self.mask.lock() |= bit;
        } else {
            *self.mask.lock() &= !bit;
        }
    }

    /// Current assertion mask / 当前的断言掩码
    #[must_use]
    pub fn mask(&self) -> u64 {
        *self.mask.lock()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_clear_lines() {
        let irq = IrqAssert::new();
        irq.set(0, true);
        irq.set(33, true);
        assert_eq!(irq.mask(), (1 << 0) | (1 << 33));
        irq.set(0, false);
        assert_eq!(irq.mask(), 1 << 33);
    }

    #[test]
    fn out_of_range_numbers_are_ignored() {
        let irq = IrqAssert::new();
        irq.set(64, true);
        assert_eq!(irq.mask(), 0);
    }
}
