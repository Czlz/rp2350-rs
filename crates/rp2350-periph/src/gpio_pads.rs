//! Shared GPIO pin matrix and pad electrical state / 共享 GPIO 引脚矩阵与 pad 电气状态
//!
//! RP2350 pads have pull-up (`PUE`), pull-down (`PDE`) and input-enable bits.
//! Which *peripheral* owns a pad is decided by `IO_BANK0.GPIOx_CTRL.FUNCSEL`;
//! only the selected peripheral drives the pad and sees its level. This module
//! is the shared fabric behind that crossbar: `PADS_BANK0` programs the pulls,
//! `IO_BANK0` publishes each pad's `FUNCSEL`, `SIO` and the three `PIO` blocks
//! publish their output latch and output enables, and every reader resolves the
//! resulting pin level from one place.
//! RP2350 的 pad 有上拉（`PUE`）、下拉（`PDE`）与输入使能位。哪个**外设**拥有 pad
//! 由 `IO_BANK0.GPIOx_CTRL.FUNCSEL` 决定；只有被选中的外设驱动 pad 并读取其电平。
//! 本模块就是该交叉开关背后的共享矩阵：`PADS_BANK0` 配置上下拉、`IO_BANK0` 发布
//! 每个 pad 的 `FUNCSEL`、`SIO` 与三个 `PIO` 块发布各自的输出锁存与输出使能，
//! 所有读者都从同一处解析出最终引脚电平。
//!
//! Resolution order, highest priority first / 解析优先级（从高到低）：
//!
//! 1. the peripheral selected by `FUNCSEL` while its output enable is set,
//! 2. the host-driven external level (`set_external`/`set_external_pin`),
//! 3. the programmed pull-up/pull-down.
//!
//! 1. `FUNCSEL` 选中且输出使能有效的外设；2. 宿主驱动的外部电平；
//! 3. 配置的上拉/下拉。

use std::sync::Arc;

use parking_lot::Mutex;

/// Number of GPIO pads modelled / 建模的 GPIO pad 数量
pub const PAD_COUNT: u32 = 48;

/// `PADS_BANK0` pull-up enable bit / `PADS_BANK0` 上拉使能位
pub const PAD_PUE: u32 = 1 << 3;

/// `PADS_BANK0` pull-down enable bit / `PADS_BANK0` 下拉使能位
pub const PAD_PDE: u32 = 1 << 2;

/// Mask of the `FUNCSEL` field / `FUNCSEL` 字段掩码
pub const FUNCSEL_MASK: u32 = 0x1F;

/// `FUNCSEL` reset value (`GPIO_FUNC_NULL`) / `FUNCSEL` 复位值（`GPIO_FUNC_NULL`）
pub const FUNCSEL_NULL: u32 = 0x1F;

/// Number of `FUNCSEL` selectors with a modelled pad driver
/// 建模了 pad 驱动器的 `FUNCSEL` 选择器数量
///
/// Selectors `0..=11` are HSTX..UART1; only `SIO`/`PIO0`/`PIO1`/`PIO2` ever
/// publish a driver, but the table keeps every documented slot so a future
/// peripheral needs no layout change. `NULL` (`0x1F`) and undocumented values
/// never drive a pad.
/// 选择器 `0..=11` 对应 HSTX..UART1；只有 `SIO`/`PIO0`/`PIO1`/`PIO2` 会发布驱动器，
/// 但表中保留了全部文档化槽位，后续外设接入无需改动布局。`NULL`（`0x1F`）与
/// 未文档化值永不驱动 pad。
const SELECTOR_COUNT: usize = 12;

/// Electrical and routing state shared by PADS_BANK0, IO_BANK0, SIO and PIO
/// PADS_BANK0、IO_BANK0、SIO 与 PIO 共享的电气与路由状态
#[derive(Debug, Clone)]
pub struct GpioPads {
    inner: Arc<Mutex<PadState>>,
}

#[derive(Debug)]
struct PadState {
    /// Host-driven external levels (buttons, external devices)
    /// 宿主驱动的外部电平（按键、外部设备）
    external: u64,
    /// Pins actively driven by the host / 由宿主主动驱动的引脚
    drive: u64,
    /// Pins with pull-up enabled / 启用上拉的引脚
    pull_up: u64,
    /// Pins with pull-down enabled / 启用下拉的引脚
    pull_down: u64,
    /// `IO_BANK0.GPIOx_CTRL.FUNCSEL` per pad / 每个 pad 的 `FUNCSEL`
    funcsel: [u8; PAD_COUNT as usize],
    /// Pads whose `FUNCSEL` equals the slot index / `FUNCSEL` 等于槽位下标的 pad 位图
    selector_mask: [u64; SELECTOR_COUNT],
    /// Output levels published by a selector / 某选择器发布的输出电平
    driver_out: [u64; SELECTOR_COUNT],
    /// Output enables published by a selector / 某选择器发布的输出使能
    driver_oe: [u64; SELECTOR_COUNT],
}

impl Default for PadState {
    fn default() -> Self {
        Self {
            external: 0,
            drive: 0,
            pull_up: 0,
            pull_down: (1u64 << PAD_COUNT) - 1,
            funcsel: [FUNCSEL_NULL as u8; PAD_COUNT as usize],
            selector_mask: [0; SELECTOR_COUNT],
            driver_out: [0; SELECTOR_COUNT],
            driver_oe: [0; SELECTOR_COUNT],
        }
    }
}

/// Resolve every pad level from the current routing state / 由当前路由状态解析全部 pad 电平
///
/// A peripheral driver wins over the host's external level, which in turn wins
/// over the pull, mirroring the pad's drive arbitration.
/// 外设驱动器优先于宿主外部电平，后者又优先于上下拉，对应 pad 的驱动仲裁。
fn resolve(state: &PadState) -> u64 {
    let mut driven = 0u64;
    let mut driven_mask = 0u64;
    for slot in 0..SELECTOR_COUNT {
        // 只有被 FUNCSEL 选中且输出使能有效的 pad 才受该驱动器影响。
        // Only pads selected by FUNCSEL and enabled by the driver are affected.
        let enables = state.driver_oe[slot] & state.selector_mask[slot];
        driven_mask |= enables;
        driven |= state.driver_out[slot] & enables;
    }
    let host = state.external & state.drive;
    let pulled = state.pull_up & !state.pull_down;
    driven | (host & !driven_mask) | (pulled & !state.drive & !driven_mask)
}

impl GpioPads {
    /// Create pads with the reset pulls (pull-down on every pin)
    /// 创建使用复位上下拉的 pad（所有引脚下拉）
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(PadState::default())),
        }
    }

    /// Program the pull bits for one pad / 配置一个 pad 的上下拉位
    pub fn set_pad(&self, pin: u32, value: u32) {
        if pin >= PAD_COUNT {
            return;
        }
        let bit = 1u64 << pin;
        let mut state = self.inner.lock();
        if value & PAD_PUE != 0 && value & PAD_PDE == 0 {
            state.pull_up |= bit;
            state.pull_down &= !bit;
        } else if value & PAD_PDE != 0 && value & PAD_PUE == 0 {
            state.pull_down |= bit;
            state.pull_up &= !bit;
        } else {
            // Both or neither: no deliberate pull.
            // 两者都置位或都不置位：无确定上下拉。
            state.pull_up &= !bit;
            state.pull_down &= !bit;
        }
    }

    /// Set all external pin levels at once / 一次设置所有外部引脚电平
    pub fn set_external(&self, levels: u64) {
        let mut state = self.inner.lock();
        state.external = levels;
        state.drive = (1u64 << PAD_COUNT) - 1;
    }

    /// Set one external pin level / 设置一个外部引脚电平
    pub fn set_external_pin(&self, pin: u32, high: bool) {
        if pin >= PAD_COUNT {
            return;
        }
        let bit = 1u64 << pin;
        let mut state = self.inner.lock();
        state.drive |= bit;
        if high {
            state.external |= bit;
        } else {
            state.external &= !bit;
        }
    }

    /// Record the `FUNCSEL` of one pad / 记录一个 pad 的 `FUNCSEL`
    ///
    /// Called by `IO_BANK0` whenever `GPIOx_CTRL` is written. Values outside
    /// `0..=11` (including `NULL`) select no driver.
    /// `IO_BANK0` 在 `GPIOx_CTRL` 被写入时调用；`0..=11` 之外的值（含 `NULL`）
    /// 不选中任何驱动器。
    pub fn set_funcsel(&self, pin: u32, bits: u32) {
        if pin >= PAD_COUNT {
            return;
        }
        let bits = (bits & FUNCSEL_MASK) as u8;
        let bit = 1u64 << pin;
        let mut state = self.inner.lock();
        let previous = usize::from(state.funcsel[pin as usize]);
        if previous < SELECTOR_COUNT {
            state.selector_mask[previous] &= !bit;
        }
        state.funcsel[pin as usize] = bits;
        let selected = usize::from(bits);
        if selected < SELECTOR_COUNT {
            state.selector_mask[selected] |= bit;
        }
    }

    /// `FUNCSEL` of one pad, or `None` outside bank 0 / 一个 pad 的 `FUNCSEL`；越界返回 `None`
    #[must_use]
    pub fn funcsel(&self, pin: u32) -> Option<u32> {
        if pin >= PAD_COUNT {
            return None;
        }
        Some(u32::from(self.inner.lock().funcsel[pin as usize]))
    }

    /// Bitmap of pads currently routed to selector `bits` / 当前路由到选择器 `bits` 的 pad 位图
    ///
    /// Values outside the modelled selector table return `0`.
    /// 超出已建模选择器表的数值返回 `0`。
    #[must_use]
    pub fn funcsel_mask(&self, bits: u32) -> u64 {
        let slot = bits as usize;
        if slot >= SELECTOR_COUNT {
            return 0;
        }
        self.inner.lock().selector_mask[slot]
    }

    /// Publish the output levels and enables of one peripheral selector
    /// 发布某个外设选择器的输出电平与输出使能
    ///
    /// The contribution only reaches pads whose `FUNCSEL` equals `selector`, so
    /// two peripherals may publish freely without fighting over a pad. Levels
    /// above [`PAD_COUNT`] are dropped.
    /// 该贡献只作用于 `FUNCSEL` 等于 `selector` 的 pad，因此多个外设可自由发布而
    /// 不会争用同一个 pad；超出 [`PAD_COUNT`] 的位被丢弃。
    pub fn set_driver(&self, selector: u32, levels: u64, enables: u64) {
        let slot = selector as usize;
        if slot >= SELECTOR_COUNT {
            return;
        }
        let mask = (1u64 << PAD_COUNT) - 1;
        let mut state = self.inner.lock();
        state.driver_out[slot] = levels & mask;
        state.driver_oe[slot] = enables & mask;
    }

    /// Sample the 32-pin window of selector `selector` starting at GPIO `base`
    /// 采样选择器 `selector` 从 GPIO `base` 起的 32 引脚窗口
    ///
    /// Bit `n` is the resolved level of pad `base + n`, or `0` when that pad is
    /// not routed to `selector` or does not exist. This is what a PIO block sees
    /// through its `IN PINS`/`WAIT`/`JMP PIN` sources.
    /// 位 `n` 是 pad `base + n` 的解析电平；该 pad 未路由到 `selector` 或不存在时
    /// 为 `0`。这就是 PIO 块通过 `IN PINS`/`WAIT`/`JMP PIN` 看到的输入。
    #[must_use]
    pub fn sample_for(&self, selector: u32, base: u32) -> u32 {
        let slot = selector as usize;
        if slot >= SELECTOR_COUNT || base >= PAD_COUNT {
            return 0;
        }
        let state = self.inner.lock();
        let levels = resolve(&state);
        let window = (levels >> base) & (state.selector_mask[slot] >> base);
        window as u32
    }

    /// Resulting input level for every modelled pin / 每个建模引脚的输入电平
    #[must_use]
    pub fn input_levels(&self) -> u64 {
        resolve(&self.inner.lock())
    }

    /// Restore reset pulls (pull-down on every pin) / 恢复复位上下拉（所有引脚下拉）
    ///
    /// `FUNCSEL` routing belongs to `IO_BANK0` and is therefore left untouched;
    /// `IoBank0::reset` clears it through [`GpioPads::set_funcsel`].
    /// `FUNCSEL` 路由属于 `IO_BANK0`，因此不受影响；`IoBank0::reset` 会通过
    /// [`GpioPads::set_funcsel`] 清除它。
    pub fn reset(&self) {
        let mut state = self.inner.lock();
        state.external = 0;
        state.drive = 0;
        state.pull_up = 0;
        state.pull_down = (1u64 << PAD_COUNT) - 1;
    }
}

impl Default for GpioPads {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{GpioPads, PAD_PDE, PAD_PUE};

    #[test]
    fn reset_pulls_low() {
        let pads = GpioPads::new();
        assert_eq!(pads.input_levels(), 0);
    }

    #[test]
    fn pull_up_drives_an_input_high() {
        let pads = GpioPads::new();
        pads.set_pad(4, PAD_PUE);
        assert_eq!(pads.input_levels(), 1 << 4);
    }

    #[test]
    fn external_level_overrides_a_pull() {
        let pads = GpioPads::new();
        pads.set_pad(4, PAD_PUE);
        pads.set_external_pin(4, false);
        assert_eq!(pads.input_levels(), 0);
        pads.set_external_pin(4, true);
        assert_eq!(pads.input_levels(), 1 << 4);
    }

    #[test]
    fn pull_down_replaces_pull_up() {
        let pads = GpioPads::new();
        pads.set_pad(2, PAD_PUE);
        pads.set_pad(2, PAD_PDE);
        assert_eq!(pads.input_levels(), 0);
        pads.set_pad(2, 0);
        assert_eq!(pads.input_levels(), 0);
    }

    #[test]
    fn funcsel_routing_tracks_the_selected_selector() {
        let pads = GpioPads::new();
        assert_eq!(pads.funcsel(3), Some(super::FUNCSEL_NULL));
        pads.set_funcsel(3, 6);
        assert_eq!(pads.funcsel(3), Some(6));
        assert_eq!(pads.funcsel_mask(6), 1 << 3);
        // 切到 SIO 后位图从前一个选择器移除。Switching removes the old selector bit.
        pads.set_funcsel(3, 5);
        assert_eq!(pads.funcsel_mask(6), 0);
        assert_eq!(pads.funcsel_mask(5), 1 << 3);
        // `NULL` 与越界 pad 不选中任何驱动器。NULL and out-of-range pads select nothing.
        pads.set_funcsel(3, super::FUNCSEL_NULL);
        assert_eq!(pads.funcsel_mask(5), 0);
        assert_eq!(pads.funcsel(crate::gpio_pads::PAD_COUNT), None);
        assert_eq!(pads.funcsel_mask(0x1F), 0);
    }

    #[test]
    fn a_selected_driver_beats_the_pull_and_the_host() {
        let pads = GpioPads::new();
        pads.set_pad(1, PAD_PUE);
        pads.set_external_pin(1, false);
        pads.set_driver(5, 1 << 1, 1 << 1);

        // 未被 FUNCSEL 选中时驱动器不影响 pad。Without FUNCSEL the driver is ignored.
        assert_eq!(pads.input_levels(), 0);
        pads.set_funcsel(1, 5);
        assert_eq!(pads.input_levels(), 1 << 1, "driver beats host and pull");
        // 输出使能清零后回到宿主/上下拉。Clearing the enable returns to host/pull.
        pads.set_driver(5, 1 << 1, 0);
        assert_eq!(pads.input_levels(), 0);
    }

    #[test]
    fn sample_for_windows_the_selected_pads() {
        let pads = GpioPads::new();
        pads.set_pad(16, PAD_PUE);
        pads.set_pad(17, PAD_PUE);
        pads.set_funcsel(17, 6);
        assert_eq!(pads.sample_for(6, 16), 0b10);
        assert_eq!(pads.sample_for(6, 0), 1 << 17, "base 0 window keeps pin 17");
        assert_eq!(pads.sample_for(7, 16), 0, "another selector sees nothing");
        assert_eq!(
            pads.sample_for(6, 40),
            0,
            "window past the last pad is empty"
        );
    }
}
