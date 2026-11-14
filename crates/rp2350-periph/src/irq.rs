//! Peripheral interrupt assertions / 外设中断断言
//!
//! Peripherals share an [`IrqAssert`] with [`crate::BootPeripherals`] so the
//! simulation driver can poll which interrupt lines are currently asserted and
//! forward them to the CPU. The set covers IRQ numbers `0..=63`, which is wider
//! than every Stage-5 line (TIMER0 `0..=3`, DMA `10..=13`, USBCTRL `14`,
//! PIO0..2 `15..=20`, SIO `25/26`, UART0 `33`).
//! 外设与 [`crate::BootPeripherals`] 共享 [`IrqAssert`]，仿真驱动据此轮询当前
//! 断言的中断线并转发给 CPU。该集合覆盖 IRQ `0..=63`，宽于 Stage 5 的所有
//! 中断线（TIMER0 `0..=3`、DMA `10..=13`、USBCTRL `14`、PIO0..2 `15..=20`、
//! SIO `25/26`、UART0 `33`）。

use std::sync::Arc;

use parking_lot::Mutex;

/// Highest IRQ number representable in the shared mask / 共享掩码可表示的最大 IRQ 号
pub const MAX_IRQ: u16 = 63;

/// PIO0 interrupt line 0 (`IRQ` flags `0..=3`) / PIO0 中断线 0（`IRQ` 标志 `0..=3`）
pub const PIO0_IRQ_0: u16 = 15;

/// PIO0 interrupt line 1 (`IRQ` flags `4..=7`) / PIO0 中断线 1（`IRQ` 标志 `4..=7`）
pub const PIO0_IRQ_1: u16 = 16;

/// PIO1 interrupt line 0 (`IRQ` flags `0..=3`) / PIO1 中断线 0（`IRQ` 标志 `0..=3`）
pub const PIO1_IRQ_0: u16 = 17;

/// PIO1 interrupt line 1 (`IRQ` flags `4..=7`) / PIO1 中断线 1（`IRQ` 标志 `4..=7`）
pub const PIO1_IRQ_1: u16 = 18;

/// PIO2 interrupt line 0 (`IRQ` flags `0..=3`) / PIO2 中断线 0（`IRQ` 标志 `0..=3`）
pub const PIO2_IRQ_0: u16 = 19;

/// PIO2 interrupt line 1 (`IRQ` flags `4..=7`) / PIO2 中断线 1（`IRQ` 标志 `4..=7`）
pub const PIO2_IRQ_1: u16 = 20;

/// PIO interrupt lines indexed by block and line / 按块与线索引的 PIO 中断线
///
/// RP2350 numbers the six lines `15..=20`, immediately after `USBCTRL_IRQ`
/// (14) and before `IO_IRQ_BANK0` (21). In each block the datasheet groups
/// `IRQ` flags `0..=3` behind `IRQ0_INTE` (line 0) and flags `4..=7` behind
/// `IRQ1_INTE` (line 1).
/// RP2350 把这六条线编号为 `15..=20`，正好位于 `USBCTRL_IRQ`（14）之后、
/// `IO_IRQ_BANK0`（21）之前。每个块中数据手册把 `IRQ` 标志 `0..=3` 放在
/// `IRQ0_INTE`（0 号线）之后、标志 `4..=7` 放在 `IRQ1_INTE`（1 号线）之后。
pub const PIO_IRQ_LINES: [[u16; 2]; 3] = [
    [PIO0_IRQ_0, PIO0_IRQ_1],
    [PIO1_IRQ_0, PIO1_IRQ_1],
    [PIO2_IRQ_0, PIO2_IRQ_1],
];

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

    #[test]
    fn pio_lines_sit_between_usb_and_gpio() {
        // RP2350 的六条 PIO 线紧接 USBCTRL(14) 之后、IO_IRQ_BANK0(21) 之前。
        // The six PIO lines follow USBCTRL (14) and precede IO_IRQ_BANK0 (21).
        assert_eq!(PIO_IRQ_LINES[0], [15, 16]);
        assert_eq!(PIO_IRQ_LINES[1], [17, 18]);
        assert_eq!(PIO_IRQ_LINES[2], [19, 20]);
        assert_eq!(PIO0_IRQ_0, crate::map::USBCTRL_IRQ + 1);
        assert_eq!(PIO2_IRQ_1 + 1, crate::map::IO_IRQ_BANK0);
    }
}
