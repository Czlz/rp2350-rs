//! Clock tree unit tests / 时钟树单元测试

use super::*;

/// `XOSC_STATUS.STABLE` / `XOSC_STATUS.STABLE`
const XOSC_STABLE: u32 = 1 << 31;

/// `XOSC_STATUS.BADWRITE` / `XOSC_STATUS.BADWRITE`
const XOSC_BADWRITE: u32 = 1 << 24;

/// `XOSC_STATUS.ENABLED` / `XOSC_STATUS.ENABLED`
const XOSC_ENABLED: u32 = 1 << 12;

/// `XOSC_CTRL.ENABLE` = DISABLE / `XOSC_CTRL.ENABLE` = DISABLE
const XOSC_DISABLE: u32 = 0xD1E;

/// `ROSC_CTRL.ENABLE` = DISABLE / `ROSC_CTRL.ENABLE` = DISABLE
const ROSC_DISABLE: u32 = 0xD1E;

/// `ROSC_STATUS.STABLE` / `ROSC_STATUS.STABLE`
const ROSC_STABLE: u32 = 1 << 31;

/// Write a whole channel control value / 写入完整的通道控制值
fn set_ctrl(tree: &ClockTree, index: usize, value: u32) {
    tree.write_channel_ctrl(index, value);
}

#[test]
fn bootrom_state_is_150_mhz_from_the_pll_chain() {
    // 12 MHz / REFDIV=1 × FBDIV=125 = 1500 MHz VCO；/POSTDIV1=5
    // = 300 MHz；/POSTDIV2=2 = 150 MHz。
    // 12 MHz / REFDIV=1 × FBDIV=125 = 1500 MHz VCO; /5 = 300 MHz;
    // /2 = 150 MHz.
    let tree = ClockTree::new();
    assert_eq!(tree.xosc_hz(), 12_000_000);
    assert_eq!(tree.pll_sys_hz(), 150_000_000);
    assert_eq!(tree.refclk_hz(), 12_000_000);
    assert_eq!(tree.sysclk_hz(), 150_000_000);
    assert_eq!(tree.periclk_hz(), 150_000_000);
    assert_eq!(tree.hstxclk_hz(), 150_000_000);
    assert_eq!(tree.usbclk_hz(), 48_000_000);
    assert_eq!(tree.adcclk_hz(), 48_000_000);
    assert_eq!(tree.rtcclk_hz(), LPOSC_HZ);
    assert!(tree.pll_locked(PllId::Sys));
    assert!(tree.pll_locked(PllId::Usb));
}

#[test]
fn overclocking_to_252_mhz_follows_the_sdk_search_order() {
    // `set_sys_clock_khz(252000)` 从 fbdiv=320 向下搜索：fbdiv=126 给出
    // 1512 MHz VCO（600..1600 内），postdiv1=6/postdiv2=1 → 252 MHz。
    // `set_sys_clock_khz(252000)` searches from fbdiv=320 downwards:
    // fbdiv=126 yields a 1512 MHz VCO and 6/1 gives 252 MHz.
    let tree = ClockTree::new();
    tree.write_pll(PllId::Sys, 0x08, 126);
    tree.write_pll(PllId::Sys, 0x0C, (6 << 16) | (1 << 12));
    assert_eq!(tree.pll_sys_hz(), 252_000_000);
    assert_eq!(tree.sysclk_hz(), 252_000_000);
    // 降频到 50 MHz：fbdiv=100 → 1200 MHz，/4/6 = 50 MHz。
    // Underclock to 50 MHz: fbdiv=100 → 1200 MHz, /4/6 = 50 MHz.
    tree.write_pll(PllId::Sys, 0x08, 100);
    tree.write_pll(PllId::Sys, 0x0C, (4 << 16) | (6 << 12));
    assert_eq!(tree.sysclk_hz(), 50_000_000);
}

#[test]
fn sys_switches_between_ref_and_aux() {
    let tree = ClockTree::new();
    // CLK_SYS_CTRL.SRC=0（clk_ref），DIV=3 → 12 MHz / 3 = 4 MHz。
    // SRC=0 (clk_ref) with DIV=3 gives 12 MHz / 3 = 4 MHz.
    set_ctrl(&tree, CLK_SYS, 0);
    tree.write_channel_div(CLK_SYS, 3 << 16);
    assert_eq!(tree.sysclk_hz(), 4_000_000);
    // 切回 AUX/PLL_SYS 并恢复 DIV=1。Back to AUX/PLL_SYS with DIV=1.
    set_ctrl(&tree, CLK_SYS, 0x01);
    tree.write_channel_div(CLK_SYS, 1 << 16);
    assert_eq!(tree.sysclk_hz(), 150_000_000);
}

#[test]
fn peri_and_usb_known_vectors() {
    let tree = ClockTree::new();
    // CLK_PERI AUXSRC=1 (PLL_SYS) + DIV=2 → 75 MHz。
    // CLK_PERI AUXSRC=1 (PLL_SYS) with DIV=2 gives 75 MHz.
    set_ctrl(&tree, CLK_PERI, CHANNEL_ENABLE | (1 << 5));
    tree.write_channel_div(CLK_PERI, 2 << 16);
    assert_eq!(tree.periclk_hz(), 75_000_000);
    // CLK_USB 源 PLL_USB（48 MHz），DIV=2 → 24 MHz。
    // CLK_USB from PLL_USB (48 MHz) with DIV=2 gives 24 MHz.
    tree.write_channel_div(CLK_USB, 2 << 16);
    assert_eq!(tree.usbclk_hz(), 24_000_000);
    tree.write_channel_div(CLK_USB, 1 << 16);
    assert_eq!(tree.usbclk_hz(), 48_000_000);
    // 4 位 ADC 分频字段：0 表示 /16。
    // The 4-bit ADC divider treats 0 as /16.
    tree.write_channel_div(CLK_ADC, 0);
    assert_eq!(tree.adcclk_hz(), 48_000_000 / 16);
}

#[test]
fn fractional_divider_is_honoured_for_sys() {
    let tree = ClockTree::new();
    // 150 MHz / 1.5 = 100 MHz：INT=1, FRAC=0x8000。
    // 150 MHz / 1.5 = 100 MHz with INT=1 and FRAC=0x8000.
    tree.write_channel_div(CLK_SYS, (1 << 16) | 0x8000);
    assert_eq!(tree.sysclk_hz(), 100_000_000);
}

#[test]
fn invalid_pll_settings_report_zero_without_panicking() {
    let tree = ClockTree::new();
    // REFDIV=0 是未定义行为 → 0 Hz。REFDIV=0 is undefined → 0 Hz.
    tree.write_pll(PllId::Sys, 0x00, 0);
    assert_eq!(tree.pll_sys_hz(), 0);
    assert_eq!(tree.sysclk_hz(), 0);
    // 恢复并测试 POSTDIV=0。Restore and test POSTDIV=0.
    tree.write_pll(PllId::Sys, 0x00, 1);
    tree.write_pll(PllId::Sys, 0x0C, 0);
    assert_eq!(tree.pll_sys_hz(), 0);
    // FBDIV 超范围 → 0 Hz。Out-of-range FBDIV → 0 Hz.
    tree.write_pll(PllId::Sys, 0x0C, (5 << 16) | (2 << 12));
    tree.write_pll(PllId::Sys, 0x08, 0);
    assert_eq!(tree.pll_sys_hz(), 0);
    tree.write_pll(PllId::Sys, 0x08, 4095);
    assert_eq!(tree.pll_sys_hz(), 0);
}

#[test]
fn generation_tracks_frequency_writes() {
    let tree = ClockTree::new();
    let start = tree.generation();
    tree.write_channel_div(CLK_SYS, 2 << 16);
    let after_div = tree.generation();
    assert!(after_div > start, "divider write bumps generation");
    tree.write_pll(PllId::Usb, 0x08, 101);
    assert!(tree.generation() > after_div, "PLL write bumps generation");
    let after_pll = tree.generation();
    // 只读 SELECTED 与中断使能寄存器不影响频率，也不递增世代号。
    // Read-only SELECTED and interrupt-enable registers do not bump.
    assert_eq!(tree.channel_selected(CLK_SYS), 0x2);
    tree.write_xosc_status(0);
    assert_eq!(tree.generation(), after_pll);
    tree.set_xosc_hz(12_000_000);
    assert_eq!(tree.generation(), after_pll, "same value is not a change");
    tree.set_xosc_hz(25_000_000);
    assert!(tree.generation() > after_pll, "crystal change bumps");
}

#[test]
fn xosc_status_and_badwrite_latch() {
    let tree = ClockTree::new();
    assert!(tree.xosc_stable());
    let status = tree.xosc_status();
    assert_eq!(status & XOSC_STABLE, XOSC_STABLE);
    assert_eq!(status & XOSC_ENABLED, XOSC_ENABLED);
    assert_eq!(status & 0x3, 0, "1..15 MHz range");
    // 非法 ENABLE 编码保留原值并锁存 BADWRITE。
    // An invalid ENABLE code keeps the previous value and latches BADWRITE.
    tree.write_xosc_ctrl(0x00FA_AA00);
    assert!(tree.xosc_enabled());
    assert_ne!(tree.xosc_status() & XOSC_BADWRITE, 0);
    tree.write_xosc_status(XOSC_BADWRITE);
    assert_eq!(tree.xosc_status() & XOSC_BADWRITE, 0);
    tree.write_xosc_ctrl((XOSC_DISABLE << 12) | 0xAA0);
    assert!(!tree.xosc_enabled());
    assert_eq!(tree.xosc_status() & XOSC_STABLE, 0);
    assert_eq!(tree.pll_sys_hz(), 0, "PLL reference disappears");
}

#[test]
fn rosc_divider_and_enable_follow_the_datasheet() {
    let tree = ClockTree::new();
    assert!(tree.rosc_enabled());
    assert_ne!(tree.rosc_status() & ROSC_STABLE, 0);
    assert_eq!(tree.roscclk_hz(), ROSC_RING_HZ / 32);
    tree.write_rosc_div(0xAA00 | 8);
    assert_eq!(tree.roscclk_hz(), ROSC_RING_HZ / 8);
    // div=0 表示 /128。div=0 means /128.
    tree.write_rosc_div(0xAA00);
    assert_eq!(tree.roscclk_hz(), ROSC_RING_HZ / 128);
    // 非法 DIV 编码回落到 /128。An invalid DIV code falls back to /128.
    tree.write_rosc_div(0x1234);
    assert_eq!(tree.roscclk_hz(), ROSC_RING_HZ / 128);
    tree.write_rosc_ctrl(ROSC_DISABLE << 12);
    assert_eq!(tree.roscclk_hz(), 0);
}

#[test]
fn reset_restores_the_bootrom_frequencies() {
    let tree = ClockTree::new();
    tree.write_pll(PllId::Sys, 0x08, 126);
    tree.write_pll(PllId::Sys, 0x0C, (6 << 16) | (1 << 12));
    tree.write_channel_div(CLK_SYS, 2 << 16);
    assert_ne!(tree.sysclk_hz(), 150_000_000);
    tree.reset();
    assert_eq!(tree.sysclk_hz(), 150_000_000);
    assert_eq!(tree.refclk_hz(), 12_000_000);
    assert_eq!(tree.usbclk_hz(), 48_000_000);
    assert_eq!(tree.channel_selected(CLK_REF), 1 << 2);
    assert_eq!(tree.channel_selected(CLK_SYS), 0x2);
}

#[test]
fn undefined_aux_sources_read_zero() {
    let tree = ClockTree::new();
    // CLK_SYS AUXSRC=6 未定义。CLK_SYS AUXSRC=6 is undefined.
    set_ctrl(&tree, CLK_SYS, 0x01 | (6 << 5));
    assert_eq!(tree.sysclk_hz(), 0);
    // GPIN0/1 未接入时读 0。GPIN0/1 read 0 while not driven.
    set_ctrl(&tree, CLK_PERI, CHANNEL_ENABLE | (5 << 5));
    assert_eq!(tree.periclk_hz(), 0);
    // GPOUT 选择 clk_sys 时跟随 CLK_SYS。GPOUT follows CLK_SYS.
    set_ctrl(&tree, CLK_SYS, 0x01);
    set_ctrl(&tree, CLK_GPOUT0, CHANNEL_ENABLE | (0x8 << 5));
    assert_eq!(tree.channel_hz(CLK_GPOUT0), 150_000_000);
}
