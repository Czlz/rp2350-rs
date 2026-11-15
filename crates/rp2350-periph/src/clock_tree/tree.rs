//! Shared clock tree facade / 共享时钟树外观层

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use parking_lot::Mutex;

use super::state::ClockState;
use super::{CLK_ADC, CLK_HSTX, CLK_PERI, CLK_REF, CLK_SYS, CLK_USB, CLOCK_COUNT, LPOSC_HZ, PllId};

/// Shared, cloneable clock tree / 可克隆的共享时钟树
///
/// Clones share one state and one generation counter, so mounting `CLOCKS`,
/// `XOSC`, `PLL_SYS`, `PLL_USB` and `ROSC` on the same handle makes every write
/// immediately visible to [`ClockTree::sysclk_hz`] and friends.
/// 克隆共享同一状态与同一世代号，因此把 `CLOCKS`、`XOSC`、`PLL_SYS`、
/// `PLL_USB`、`ROSC` 挂到同一句柄后，任何写入都会立即反映到
/// [`ClockTree::sysclk_hz`] 等接口。
#[derive(Debug, Clone)]
pub struct ClockTree {
    /// Register images / 寄存器镜像
    state: Arc<Mutex<ClockState>>,
    /// Monotonic write generation / 单调写入世代号
    generation: Arc<AtomicU32>,
}

impl ClockTree {
    /// Create a tree in the bootrom-equivalent state / 创建 bootrom 等价状态的时钟树
    ///
    /// This is intentionally **not** the power-on reset state: firmware starts
    /// execution after the mask ROM has already enabled XOSC, locked PLL_SYS at
    /// 150 MHz and selected it for `CLK_SYS`.
    /// 这有意**不是**上电复位值：固件开始执行时掩膜 ROM 已使能 XOSC、把
    /// PLL_SYS 锁到 150 MHz 并为 `CLK_SYS` 选中它。
    #[must_use]
    pub fn new() -> Self {
        Self::post_bootrom()
    }

    /// Create a tree in the bootrom-equivalent state / 创建 bootrom 等价状态的时钟树
    #[must_use]
    pub fn post_bootrom() -> Self {
        Self {
            state: Arc::new(Mutex::new(ClockState::bootrom())),
            generation: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Restore the bootrom-equivalent state / 恢复 bootrom 等价状态
    pub fn reset(&self) {
        *self.state.lock() = ClockState::bootrom();
        self.bump();
    }

    /// Monotonic generation of frequency-affecting writes / 影响频率写入的单调世代号
    #[must_use]
    pub fn generation(&self) -> u32 {
        self.generation.load(Ordering::Acquire)
    }

    /// Increment the generation counter / 递增世代号
    fn bump(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    /// Crystal frequency configured by the host / 宿主配置的晶振频率
    #[must_use]
    pub fn xosc_hz(&self) -> u32 {
        self.state.lock().xtal.hz
    }

    /// Set the crystal frequency (host configuration) / 设置晶振频率（宿主配置）
    pub fn set_xosc_hz(&self, hz: u32) {
        let mut state = self.state.lock();
        if state.xtal.hz == hz {
            return;
        }
        state.xtal.hz = hz;
        drop(state);
        self.bump();
    }

    /// Whether the crystal is enabled / 晶振是否已使能
    #[must_use]
    pub fn xosc_enabled(&self) -> bool {
        self.state.lock().xtal.enabled
    }

    /// Whether the crystal reports `STABLE` / 晶振是否报告 `STABLE`
    ///
    /// Startup delay is not modelled: an enabled crystal is stable at once.
    /// 不建模启动延时：使能后立即稳定。
    #[must_use]
    pub fn xosc_stable(&self) -> bool {
        self.state.lock().xtal.enabled
    }

    /// `XOSC_CTRL` register image / `XOSC_CTRL` 寄存器镜像
    #[must_use]
    pub fn xosc_ctrl(&self) -> u32 {
        self.state.lock().xtal.ctrl
    }

    /// `XOSC_STATUS` register value / `XOSC_STATUS` 寄存器值
    #[must_use]
    pub fn xosc_status(&self) -> u32 {
        self.state.lock().xosc_status()
    }

    /// Write `XOSC_CTRL` / 写 `XOSC_CTRL`
    ///
    /// Invalid `ENABLE`/`FREQ_RANGE` codes retain the previous value and latch
    /// `STATUS.BADWRITE`, matching the datasheet.
    /// 非法的 `ENABLE`/`FREQ_RANGE` 编码保留原值并锁存 `STATUS.BADWRITE`，与
    /// 数据手册一致。
    pub fn write_xosc_ctrl(&self, value: u32) {
        self.state.lock().write_xosc_ctrl(value);
        self.bump();
    }

    /// Write `XOSC_STATUS` (only `BADWRITE` is write-1-to-clear)
    /// 写 `XOSC_STATUS`（只有 `BADWRITE` 是写 1 清除）
    pub fn write_xosc_status(&self, value: u32) {
        if value & (1 << 24) != 0 {
            self.state.lock().xtal.badwrite = false;
        }
    }

    /// Restore the bootrom XOSC configuration / 恢复 bootrom 的 XOSC 配置
    pub fn reset_xosc(&self) {
        let boot = ClockState::bootrom();
        self.state.lock().xtal = boot.xtal;
        self.bump();
    }

    /// Set the ROSC ring frequency (host configuration) / 设置 ROSC 环形频率（宿主配置）
    pub fn set_rosc_hz(&self, hz: u32) {
        let mut state = self.state.lock();
        if state.ring.hz == hz {
            return;
        }
        state.ring.hz = hz;
        drop(state);
        self.bump();
    }

    /// ROSC ring frequency before `ROSC_DIV` / `ROSC_DIV` 之前的 ROSC 环形频率
    #[must_use]
    pub fn rosc_ring_hz(&self) -> u32 {
        self.state.lock().ring.hz
    }

    /// ROSC output after `ROSC_DIV` / `ROSC_DIV` 之后的 ROSC 输出
    #[must_use]
    pub fn roscclk_hz(&self) -> u32 {
        self.state.lock().ring_hz()
    }

    /// Whether the ring oscillator is enabled / 环形振荡器是否已使能
    #[must_use]
    pub fn rosc_enabled(&self) -> bool {
        self.state.lock().ring.enabled
    }

    /// `ROSC_CTRL` register image / `ROSC_CTRL` 寄存器镜像
    #[must_use]
    pub fn rosc_ctrl(&self) -> u32 {
        self.state.lock().ring.ctrl
    }

    /// `ROSC_DIV` register image / `ROSC_DIV` 寄存器镜像
    #[must_use]
    pub fn rosc_div(&self) -> u32 {
        self.state.lock().ring_div
    }

    /// `ROSC_STATUS` register value / `ROSC_STATUS` 寄存器值
    #[must_use]
    pub fn rosc_status(&self) -> u32 {
        self.state.lock().rosc_status()
    }

    /// Write `ROSC_CTRL` / 写 `ROSC_CTRL`
    pub fn write_rosc_ctrl(&self, value: u32) {
        self.state.lock().write_rosc_ctrl(value);
        self.bump();
    }

    /// Write `ROSC_DIV` (`0xAA00 + div`) / 写 `ROSC_DIV`（`0xAA00 + div`）
    pub fn write_rosc_div(&self, value: u32) {
        self.state.lock().ring_div = value & 0xFFFF;
        self.bump();
    }

    /// Write `ROSC_STATUS` (only `BADWRITE` is write-1-to-clear)
    /// 写 `ROSC_STATUS`（只有 `BADWRITE` 是写 1 清除）
    pub fn write_rosc_status(&self, value: u32) {
        if value & (1 << 24) != 0 {
            self.state.lock().ring.badwrite = false;
        }
    }

    /// Restore the bootrom ROSC configuration / 恢复 bootrom 的 ROSC 配置
    pub fn reset_rosc(&self) {
        let boot = ClockState::bootrom();
        {
            let mut state = self.state.lock();
            state.ring = boot.ring;
            state.ring_div = boot.ring_div;
        }
        self.bump();
    }

    /// `PLL_CS` register value including live `LOCK` / 含实时 `LOCK` 的 `PLL_CS` 值
    #[must_use]
    pub fn pll_cs(&self, id: PllId) -> u32 {
        self.state.lock().pll_cs(id)
    }

    /// `PLL_PWR` register value / `PLL_PWR` 寄存器值
    #[must_use]
    pub fn pll_pwr(&self, id: PllId) -> u32 {
        self.state.lock().pll_pwr(id)
    }

    /// `PLL_FBDIV_INT` register value / `PLL_FBDIV_INT` 寄存器值
    #[must_use]
    pub fn pll_fbdiv(&self, id: PllId) -> u32 {
        self.state.lock().pll_fbdiv(id)
    }

    /// `PLL_PRIM` register value / `PLL_PRIM` 寄存器值
    #[must_use]
    pub fn pll_prim(&self, id: PllId) -> u32 {
        self.state.lock().pll_prim(id)
    }

    /// Whether a PLL is locked / 某个 PLL 是否已锁定
    #[must_use]
    pub fn pll_locked(&self, id: PllId) -> bool {
        self.state.lock().pll_locked(id)
    }

    /// PLL output frequency / PLL 输出频率
    #[must_use]
    pub fn pll_hz(&self, id: PllId) -> u32 {
        self.state.lock().pll_hz(id)
    }

    /// `PLL_SYS` output frequency / `PLL_SYS` 输出频率
    #[must_use]
    pub fn pll_sys_hz(&self) -> u32 {
        self.pll_hz(PllId::Sys)
    }

    /// `PLL_USB` output frequency / `PLL_USB` 输出频率
    #[must_use]
    pub fn pll_usb_hz(&self) -> u32 {
        self.pll_hz(PllId::Usb)
    }

    /// Write one PLL register (`0x00..=0x0C`) / 写一个 PLL 寄存器（`0x00..=0x0C`）
    ///
    /// Only the register's writable fields are stored. `PLL_CS.LOCK` is
    /// read-only, so writing it has no effect.
    /// 只保存寄存器可写字段。`PLL_CS.LOCK` 只读，写入无效。
    pub fn write_pll(&self, id: PllId, offset: u32, value: u32) {
        if self.state.lock().store_pll(id, offset, value) {
            self.bump();
        }
    }

    /// Restore one PLL to its bootrom configuration / 把一个 PLL 恢复到 bootrom 配置
    pub fn reset_pll(&self, id: PllId) {
        let boot = ClockState::bootrom();
        self.state.lock().restore_pll(id, &boot);
        self.bump();
    }

    /// Raw `CLK_*_CTRL` of one channel / 某通道的原始 `CLK_*_CTRL`
    #[must_use]
    pub fn channel_ctrl(&self, index: usize) -> u32 {
        self.state.lock().channel_ctrl(index)
    }

    /// Raw `CLK_*_DIV` of one channel / 某通道的原始 `CLK_*_DIV`
    #[must_use]
    pub fn channel_div(&self, index: usize) -> u32 {
        self.state
            .lock()
            .channels
            .get(index)
            .map_or(0, |slice| slice.div)
    }

    /// `CLK_*_SELECTED` value (one-hot) / `CLK_*_SELECTED` 值（单热编码）
    ///
    /// `CLK_REF`/`CLK_SYS` decode their glitchless `SRC` field; every other
    /// slice has no glitchless mux and is hardwired to `0x1` by the datasheet.
    /// `CLK_REF`/`CLK_SYS` 解码其无毛刺 `SRC` 字段；其余通道没有无毛刺选择器，
    /// 按数据手册恒为 `0x1`。
    #[must_use]
    pub fn channel_selected(&self, index: usize) -> u32 {
        let ctrl = self.channel_ctrl(index);
        match index {
            CLK_REF => 1 << (ctrl & 0x3),
            CLK_SYS => 1 << (ctrl & 0x1),
            _ if index < CLOCK_COUNT => 1,
            _ => 0,
        }
    }

    /// Write `CLK_*_CTRL` for one channel / 写某通道的 `CLK_*_CTRL`
    ///
    /// Reserved bits and the read-only `ENABLED` bit are dropped.
    /// 保留位与只读的 `ENABLED` 位被丢弃。
    pub fn write_channel_ctrl(&self, index: usize, value: u32) {
        self.state.lock().store_ctrl(index, value);
        self.bump();
    }

    /// Write `CLK_*_DIV` for one channel / 写某通道的 `CLK_*_DIV`
    pub fn write_channel_div(&self, index: usize, value: u32) {
        self.state.lock().store_div(index, value);
        self.bump();
    }

    /// Frequency of one channel (`0` for out-of-range indices)
    /// 某个通道的频率（越界索引返回 `0`）
    #[must_use]
    pub fn channel_hz(&self, index: usize) -> u32 {
        self.state.lock().channel_hz(index, 0)
    }

    /// `CLK_REF` frequency / `CLK_REF` 频率
    #[must_use]
    pub fn refclk_hz(&self) -> u32 {
        self.channel_hz(CLK_REF)
    }

    /// `CLK_SYS` frequency / `CLK_SYS` 频率
    #[must_use]
    pub fn sysclk_hz(&self) -> u32 {
        self.channel_hz(CLK_SYS)
    }

    /// `CLK_PERI` frequency / `CLK_PERI` 频率
    #[must_use]
    pub fn periclk_hz(&self) -> u32 {
        self.channel_hz(CLK_PERI)
    }

    /// `CLK_HSTX` frequency / `CLK_HSTX` 频率
    #[must_use]
    pub fn hstxclk_hz(&self) -> u32 {
        self.channel_hz(CLK_HSTX)
    }

    /// `CLK_USB` frequency / `CLK_USB` 频率
    #[must_use]
    pub fn usbclk_hz(&self) -> u32 {
        self.channel_hz(CLK_USB)
    }

    /// `CLK_ADC` frequency / `CLK_ADC` 频率
    #[must_use]
    pub fn adcclk_hz(&self) -> u32 {
        self.channel_hz(CLK_ADC)
    }

    /// RTC clock frequency / RTC 时钟频率
    ///
    /// RP2350 has no `CLK_RTC` divider: the always-on domain runs directly from
    /// the low-power oscillator, so this is [`LPOSC_HZ`].
    /// RP2350 没有 `CLK_RTC` 分频器：常开域直接由低功耗振荡器驱动，因此返回
    /// [`LPOSC_HZ`]。
    #[must_use]
    pub fn rtcclk_hz(&self) -> u32 {
        LPOSC_HZ
    }
}

impl Default for ClockTree {
    fn default() -> Self {
        Self::new()
    }
}
