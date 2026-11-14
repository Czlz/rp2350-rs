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

/// PSM (power state machine) base / PSM（电源状态机）基地址
///
/// Firmware programs `FRCE_ON`/`FRCE_OFF` here to control core and peripheral
/// power domains; the block is part of the APB span and uses the standard
/// XOR/SET/CLR alias windows.
/// 固件在这里写 `FRCE_ON`/`FRCE_OFF` 控制核心与外设电源域；该块属于 APB
/// 跨度，并使用标准的 XOR/SET/CLR 别名窗口。
pub const PSM_BASE: Address = 0x4001_8000;

/// RESETS block base / RESETS 块基地址
pub const RESETS_BASE: Address = 0x4002_0000;

/// IO_BANK0 base / IO_BANK0 基地址
pub const IO_BANK0_BASE: Address = 0x4002_8000;

/// IO_QSPI base / IO_QSPI 基地址
pub const IO_QSPI_BASE: Address = 0x4003_0000;

/// PADS_BANK0 base / PADS_BANK0 基地址
pub const PADS_BANK0_BASE: Address = 0x4003_8000;

/// PADS_QSPI base / PADS_QSPI 基地址
pub const PADS_QSPI_BASE: Address = 0x4004_0000;

/// XOSC block base / XOSC 块基地址
pub const XOSC_BASE: Address = 0x4004_8000;

/// PLL_SYS block base / PLL_SYS 块基地址
pub const PLL_SYS_BASE: Address = 0x4005_0000;

/// PLL_USB block base / PLL_USB 块基地址
pub const PLL_USB_BASE: Address = 0x4005_8000;

/// UART0 (PL011) base / UART0（PL011）基地址
pub const UART0_BASE: Address = 0x4007_0000;

/// ADC base / ADC 基地址
///
/// The ADC block resets to all-zero registers; `CS.READY` is modelled as
/// always set. ADC 块寄存器复位为零；`CS.READY` 建模为始终置位。
pub const ADC_BASE: Address = 0x400A_0000;

/// PWM register block base / PWM 寄存器块基地址
///
/// RP2350 has one 12-channel PWM register block, not separate PWM0/PWM1
/// instances. RP2350 只有一个 12 通道 PWM 寄存器块，没有独立的 PWM0/PWM1 实例。
pub const PWM0_BASE: Address = 0x400A_8000;

/// Address reported as "PWM1" in firmware logs / 固件日志中报告的“PWM1”地址
///
/// This is the XOR alias window of [`PWM0_BASE`] (`+0x1000`), not a second
/// register block. The APB router folds it onto the PWM device.
/// 这是 [`PWM0_BASE`] 的 XOR 别名窗口（`+0x1000`），并非第二个寄存器块；
/// APB 路由器会把它折叠到 PWM 设备上。
pub const PWM1_BASE: Address = 0x400A_9000;

/// TIMER0 base / TIMER0 基地址
pub const TIMER0_BASE: Address = 0x400B_0000;

/// HSTX control register block base (APB) / HSTX 控制寄存器块基地址（APB）
///
/// The control window holds `CSR`, `BIT0..BIT7`, `EXPAND_SHIFT` and
/// `EXPAND_TMDS`; the command/data FIFO is a separate AHB block at
/// [`HSTX_FIFO_BASE`]. 控制窗口包含 `CSR`、`BIT0..BIT7`、`EXPAND_SHIFT` 与
/// `EXPAND_TMDS`；命令/数据 FIFO 是 [`HSTX_FIFO_BASE`] 处的独立 AHB 块。
pub const HSTX_CTRL_BASE: Address = 0x400C_0000;

/// XIP_QMI base / XIP_QMI 基地址
pub const XIP_QMI_BASE: Address = 0x400D_0000;

/// WATCHDOG base / WATCHDOG 基地址
pub const WATCHDOG_BASE: Address = 0x400D_8000;

/// POWMAN always-on power manager base / POWMAN 常开电源管理器基地址
///
/// POWMAN is the first device above the APB span and is mounted standalone.
/// POWMAN 是 APB 跨度之上的第一个设备，独立挂载。
pub const POWMAN_BASE: Address = 0x4010_0000;

/// TICKS tick generator base / TICKS 滴答发生器基地址
///
/// TICKS sits above the 1 MB APB span modelled by the router, so it is mounted
/// as a standalone device window.
/// TICKS 位于路由器建模的 1 MB APB 跨度之上，因此作为独立设备窗口挂载。
pub const TICKS_BASE: Address = 0x4010_8000;

/// BOOTRAM base / BOOTRAM 基地址
///
/// The boot ROM scratch RAM and the boot locks share one 4 KB window.
/// boot ROM 暂存 RAM 与 boot 锁共用同一个 4 KB 窗口。
pub const BOOTRAM_BASE: Address = 0x400E_0000;

/// ROSC base / ROSC 基地址
///
/// `STATUS` reports `STABLE`, `DIV_RUNNING` and `ENABLED` in this model.
/// 本模型中 `STATUS` 报告 `STABLE`、`DIV_RUNNING` 与 `ENABLED`。
pub const ROSC_BASE: Address = 0x400E_8000;

/// TRNG base / TRNG 基地址
///
/// The block keeps its Synopsys register layout starting at `+0x100`.
/// 该块从 `+0x100` 开始保留 Synopsys 寄存器布局。
pub const TRNG_BASE: Address = 0x400F_0000;

/// SHA-256 accelerator base / SHA-256 加速器基地址
pub const SHA256_BASE: Address = 0x400F_8000;

/// DMA controller base / DMA 控制器基地址
///
/// The 16-channel DMA register block occupies one 4 KB window at
/// `0x5000_0000`: channels at `+0x000..=0x3FF`, global registers at
/// `+0x400..=0x7FF` and per-channel debug registers (not modelled) at
/// `+0x800..=0xBFF`.
/// 16 通道 DMA 寄存器块占据 `0x5000_0000` 的一个 4 KB 窗口：通道寄存器位于
/// `+0x000..=0x3FF`，全局寄存器位于 `+0x400..=0x7FF`，通道调试寄存器
/// （未建模）位于 `+0x800..=0xBFF`。
pub const DMA_BASE: Address = 0x5000_0000;

/// DMA register window including SET/CLR/XOR aliases (16 KB)
/// DMA 寄存器窗口（含 SET/CLR/XOR 别名，共 16 KB）
pub const DMA_SIZE: usize = 0x4000;

/// DMA interrupt line 0 / DMA 中断线 0
///
/// RP2350 routes four DMA interrupt lines (`DMA_IRQ_0..3`); each one can be
/// enabled per channel through `INTE0..3`.
/// RP2350 提供四条 DMA 中断线（`DMA_IRQ_0..3`），每条都可按通道通过
/// `INTE0..3` 使能。
pub const DMA_IRQ_0: u16 = 10;

/// DMA interrupt line 1 / DMA 中断线 1
pub const DMA_IRQ_1: u16 = 11;

/// DMA interrupt line 2 / DMA 中断线 2
pub const DMA_IRQ_2: u16 = 12;

/// DMA interrupt line 3 / DMA 中断线 3
pub const DMA_IRQ_3: u16 = 13;

/// PIO0 register block base / PIO0 寄存器块基地址
///
/// RP2350 exposes three identical PIO blocks in the AHB peripheral space.
/// RP2350 在 AHB 外设空间提供三个相同的 PIO 块。
pub const PIO0_BASE: Address = 0x5020_0000;

/// PIO1 register block base / PIO1 寄存器块基地址
pub const PIO1_BASE: Address = 0x5030_0000;

/// PIO2 register block base / PIO2 寄存器块基地址
pub const PIO2_BASE: Address = 0x5040_0000;

/// HSTX FIFO register block base (AHB) / HSTX FIFO 寄存器块基地址（AHB）
///
/// The FIFO window holds `HSTX_FIFO_STAT` and the write-only `HSTX_FIFO_FIFO`.
/// FIFO 窗口包含 `HSTX_FIFO_STAT` 与只写的 `HSTX_FIFO_FIFO`。
pub const HSTX_FIFO_BASE: Address = 0x5060_0000;

/// Size of each HSTX register window / 每个 HSTX 寄存器窗口大小
pub const HSTX_SIZE: usize = 0x1000;

/// Interrupt line asserted by the HSTX model / HSTX 模型断言的中断线
///
/// The RP2350 datasheet assigns no dedicated HSTX interrupt line, so the model
/// borrows `SPARE_IRQ_0` (46) for the model-only underrun, command-done and
/// unknown-command status bits.
/// RP2350 数据手册没有专用 HSTX 中断线，因此模型借用 `SPARE_IRQ_0`（46）承载
/// 仅模型的下溢、命令完成与未知命令状态位。
pub const HSTX_IRQ: u16 = 46;

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
