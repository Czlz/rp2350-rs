//! Shared RP2350 clock tree / 共享的 RP2350 时钟树
//!
//! One [`ClockTree`] is owned by `CLOCKS`, `PLL_SYS`, `PLL_USB`, `XOSC` and
//! `ROSC`, so a guest write to any of those blocks is visible in every exported
//! frequency immediately. The tree is the single place that decodes register
//! fields into frequencies; the MMIO devices only store raw register images.
//! [`ClockTree`] 由 `CLOCKS`、`PLL_SYS`、`PLL_USB`、`XOSC` 与 `ROSC` 共享，
//! 因此客户机对其中任一块的写入会立即反映到所有导出的频率上。频率换算只在
//! 本模块进行，MMIO 设备只保存原始寄存器镜像。
//!
//! # Formulas / 公式
//!
//! ```text
//! vco      = (xosc_hz / refdiv) * fbdiv                 (PLL_CS.FBDIV_INT)
//! pll_out  = vco / postdiv1 / postdiv2                  (PLL_PRIM)
//! clk      = source / (div_int + div_frac / 65536)      (CLK_*_CTRL/DIV)
//! ```
//!
//! `div_int == 0` means `max + 1` for the field width (datasheet: "0 -> max+1"):
//! 65536 for the 16-bit fields (`CLK_SYS`/`CLK_GPOUTn`), 256 for the 8-bit
//! `CLK_REF` field, 16 for the 4-bit `CLK_USB`/`CLK_ADC` fields and 4 for the
//! 2-bit `CLK_PERI`/`CLK_HSTX` fields. Only `CLK_SYS` and `CLK_GPOUTn` carry a
//! fractional part in `DIV.FRAC`; the remaining slices are integer only.
//! `div_int == 0` 表示该字段宽度的 `max + 1`（数据手册 "0 -> max+1"）：16 位字段
//! （`CLK_SYS`/`CLK_GPOUTn`）为 65536，8 位 `CLK_REF` 为 256，4 位
//! `CLK_USB`/`CLK_ADC` 为 16，2 位 `CLK_PERI`/`CLK_HSTX` 为 4。只有 `CLK_SYS`
//! 与 `CLK_GPOUTn` 的 `DIV.FRAC` 参与运算，其余通道只支持整数分频。
//!
//! # Invalid configurations / 非法配置
//!
//! `refdiv == 0`, `postdiv1/2 == 0`, `fbdiv` outside `16..=320` and undefined
//! clock-source encodings are **not** fatal: the affected clock reports `0 Hz`
//! and one `log::warn!` is emitted for that class of error. The guest sees the
//! same "broken clock" the hardware would produce instead of a host panic.
//! `refdiv == 0`、`postdiv1/2 == 0`、`fbdiv` 超出 `16..=320` 以及未定义的时钟源
//! 编码**不会** panic：受影响的时钟读为 `0 Hz`，每类错误只输出一次
//! `log::warn!`。客户机看到的是与硬件一致的「时钟损坏」，而不是宿主崩溃。
//!
//! # Generation counter / 世代计数器
//!
//! Any accepted write that can change a frequency increments
//! [`ClockTree::generation`]. Consumers (P4-2) cache derived timing and
//! invalidate it when the generation changes. Writes that cannot change a clock
//! (read-only `CLK_*_SELECTED`, PLL interrupt enables, `XOSC_STARTUP`, ...) do
//! not increment it.
//! 任何可能改变频率且被接受的写入都会递增 [`ClockTree::generation`]。消费者
//! （P4-2）据此失效缓存的时序。不会改变时钟的写入（只读的 `CLK_*_SELECTED`、
//! PLL 中断使能、`XOSC_STARTUP` 等）不会递增。

mod formula;
mod state;
#[cfg(test)]
mod tests;
mod tree;

pub use tree::ClockTree;

/// Crystal frequency of the reference oscillator / 参考晶振频率
pub const XOSC_HZ: u32 = 12_000_000;

/// Nominal ring frequency of the ROSC before its divider
/// ROSC 分频前的标称环形频率
///
/// The ring is not calibrated on real silicon; this nominal value matches the
/// RP2350 datasheet's "approximately 6.5 MHz" figure and is only used when the
/// guest explicitly selects `rosc_clksrc`.
/// 真实芯片上的环形振荡器未校准；该标称值对应 RP2350 数据手册的「约 6.5 MHz」，
/// 仅在客户机显式选择 `rosc_clksrc` 时使用。
pub const ROSC_RING_HZ: u32 = 6_500_000;

/// Low-power oscillator frequency used by the RTC / RTC 使用的低功耗振荡器频率
///
/// RP2350 has no `CLK_RTC` slice: the always-on timer runs from `lposc_clksrc`,
/// nominally 32.768 kHz. The value is not calibrated.
/// RP2350 没有 `CLK_RTC` 分频器：常开定时器直接使用 `lposc_clksrc`，标称
/// 32.768 kHz，未校准。
pub const LPOSC_HZ: u32 = 32_768;

/// `CLK_GPOUT0` channel index / `CLK_GPOUT0` 通道序号
pub const CLK_GPOUT0: usize = 0;

/// `CLK_GPOUT1` channel index / `CLK_GPOUT1` 通道序号
pub const CLK_GPOUT1: usize = 1;

/// `CLK_GPOUT2` channel index / `CLK_GPOUT2` 通道序号
pub const CLK_GPOUT2: usize = 2;

/// `CLK_GPOUT3` channel index / `CLK_GPOUT3` 通道序号
pub const CLK_GPOUT3: usize = 3;

/// `CLK_REF` channel index / `CLK_REF` 通道序号
pub const CLK_REF: usize = 4;

/// `CLK_SYS` channel index / `CLK_SYS` 通道序号
pub const CLK_SYS: usize = 5;

/// `CLK_PERI` channel index / `CLK_PERI` 通道序号
pub const CLK_PERI: usize = 6;

/// `CLK_HSTX` channel index / `CLK_HSTX` 通道序号
pub const CLK_HSTX: usize = 7;

/// `CLK_USB` channel index / `CLK_USB` 通道序号
pub const CLK_USB: usize = 8;

/// `CLK_ADC` channel index / `CLK_ADC` 通道序号
pub const CLK_ADC: usize = 9;

/// Number of `CLOCKS` channels on RP2350 / RP2350 的 `CLOCKS` 通道数量
pub const CLOCK_COUNT: usize = 10;

/// `CLK_*_CTRL.ENABLE` (all slices except `REF`/`SYS` use the same bit)
/// `CLK_*_CTRL.ENABLE`（除 `REF`/`SYS` 外所有通道使用同一位）
pub const CHANNEL_ENABLE: u32 = 1 << 11;

/// `CLK_*_CTRL.ENABLED` read-only status bit / `CLK_*_CTRL.ENABLED` 只读状态位
pub const CHANNEL_ENABLED: u32 = 1 << 28;

/// Which PLL a register write targets / 寄存器写入目标 PLL
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PllId {
    /// `PLL_SYS` at `0x4005_0000` / `0x4005_0000` 处的 `PLL_SYS`
    Sys,
    /// `PLL_USB` at `0x4005_8000` / `0x4005_8000` 处的 `PLL_USB`
    Usb,
}
