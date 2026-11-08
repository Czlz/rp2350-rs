//! Verified RP2350 Stage-4 peripheral address map
//! 已验证的 RP2350 Stage 4 外设地址表
//!
//! Bases are taken from the RP2350 datasheet tables used by the reference
//! emulators (`picoem` `silicon_scenarios.rs`, `rp2350js` APB offsets).
//! 基地址取自参考模拟器使用的 RP2350 数据手册表（`picoem` 的
//! `silicon_scenarios.rs`、`rp2350js` 的 APB 偏移）。

use rp2350_core::types::Address;

/// Start of the APB peripheral span / APB 外设跨度起始地址
pub const APB_BASE: Address = 0x4000_0000;

/// Size of the APB span modelled by the Stage-4 router (1 MB)
/// Stage 4 路由器建模的 APB 跨度大小（1 MB）
pub const APB_SIZE: usize = 0x0010_0000;

/// CLOCKS block base / CLOCKS 块基地址
pub const CLOCKS_BASE: Address = 0x4001_0000;

/// RESETS block base / RESETS 块基地址
pub const RESETS_BASE: Address = 0x4002_0000;

/// IO_BANK0 base / IO_BANK0 基地址
pub const IO_BANK0_BASE: Address = 0x4002_8000;

/// PADS_BANK0 base / PADS_BANK0 基地址
pub const PADS_BANK0_BASE: Address = 0x4003_8000;

/// XOSC block base / XOSC 块基地址
pub const XOSC_BASE: Address = 0x4004_8000;

/// PLL_SYS block base / PLL_SYS 块基地址
pub const PLL_SYS_BASE: Address = 0x4005_0000;

/// PLL_USB block base / PLL_USB 块基地址
pub const PLL_USB_BASE: Address = 0x4005_8000;

/// UART0 (PL011) base / UART0（PL011）基地址
pub const UART0_BASE: Address = 0x4007_0000;

/// TIMER0 base / TIMER0 基地址
pub const TIMER0_BASE: Address = 0x400B_0000;

/// WATCHDOG base / WATCHDOG 基地址
pub const WATCHDOG_BASE: Address = 0x400D_8000;

/// SIO base / SIO 基地址
pub const SIO_BASE: Address = 0xD000_0000;

/// Private peripheral bus base (Core 0) / 私有外设总线基地址（Core 0）
pub const PPB_BASE: Address = 0xE000_0000;

/// Window size used by the small APB peripheral stubs / 小型 APB 外设桩使用的窗口大小
pub const DEVICE_WINDOW_SIZE: usize = 0x1000;

/// SIO window size / SIO 窗口大小
pub const SIO_WINDOW_SIZE: usize = 0x1000;

/// PPB window size (1 MB) / PPB 窗口大小（1 MB）
pub const PPB_WINDOW_SIZE: usize = 0x0010_0000;

/// TIMER0 alarm 0 interrupt line / TIMER0 闹钟 0 中断线
pub const TIMER0_IRQ_0: u16 = 0;

/// USBCTRL interrupt line (RP2350 `intctrl.h`) / USBCTRL 中断线（RP2350 `intctrl.h`）
pub const USBCTRL_IRQ: u16 = 14;

/// SIO inter-core FIFO interrupt line / SIO 核间 FIFO 中断线
pub const SIO_IRQ_FIFO: u16 = 25;

/// SIO doorbell interrupt line / SIO 门铃中断线
pub const SIO_IRQ_BELL: u16 = 26;

/// UART0 interrupt line / UART0 中断线
pub const UART0_IRQ: u16 = 33;

/// GPIO bank 0 interrupt line / GPIO bank 0 中断线
pub const IO_IRQ_BANK0: u16 = 21;
