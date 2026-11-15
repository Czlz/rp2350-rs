//! Frequency formulas and one-shot warnings / 频率公式与一次性警告

use std::sync::atomic::{AtomicBool, Ordering};

/// Apply a channel divider to `source` / 对 `source` 应用通道分频
///
/// `int_bits` is the width of `DIV.INT` (0 means `max + 1`), `frac_bits` the
/// width of `DIV.FRAC` (0 when the slice has no fractional divider).
/// `int_bits` 是 `DIV.INT` 的位宽（0 表示 `max + 1`），`frac_bits` 是
/// `DIV.FRAC` 的位宽（通道没有小数分频时为 0）。
pub(super) fn divide(source: u32, raw_div: u32, int_bits: u32, frac_bits: u32) -> u32 {
    if source == 0 {
        return 0;
    }
    let int_mask = (1u32 << int_bits) - 1;
    let int_field = (raw_div >> 16) & int_mask;
    let int = if int_field == 0 {
        1u32 << int_bits
    } else {
        int_field
    };
    let frac_mask = if frac_bits == 0 {
        0
    } else {
        (1u32 << frac_bits) - 1
    };
    let frac = raw_div & frac_mask;
    let divisor = (u64::from(int) << 16) | u64::from(frac);
    let scaled = (u64::from(source) << 16) / divisor;
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

/// Emit one warning for invalid PLL settings / 对非法 PLL 配置只警告一次
pub(super) fn warn_invalid_pll(reason: &str) {
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !WARNED.swap(true, Ordering::Relaxed) {
        log::warn!(
            "PLL: invalid configuration ({reason}), clock reads 0 Hz / \
             PLL 配置非法（{reason}），时钟读为 0 Hz"
        );
    }
}
