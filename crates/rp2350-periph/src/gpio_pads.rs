//! Shared GPIO pad electrical state / 共享 GPIO pad 电气状态
//!
//! RP2350 pads have pull-up (`PUE`), pull-down (`PDE`) and input-enable bits.
//! The SIO input register reflects the resulting pin level, not just the SIO
//! output latch: a pin configured as an input floats to its pull, and an output
//! drives its latch. This module is the shared source of truth between
//! `PADS_BANK0` (which programs the pulls) and `SIO` (which samples the pins).
//! RP2350 的 pad 有上拉（`PUE`）、下拉（`PDE`）与输入使能位。SIO 输入寄存器反映
//! 引脚电平，而不只是 SIO 输出锁存：输入引脚被拉到其上下拉电平，输出引脚则由
//! 输出锁存驱动。本模块是 `PADS_BANK0`（配置上下拉）与 `SIO`（采样引脚）之间
//! 共享的唯一事实来源。

use std::sync::Arc;

use parking_lot::Mutex;

/// Number of GPIO pads modelled / 建模的 GPIO pad 数量
pub const PAD_COUNT: u32 = 48;

/// `PADS_BANK0` pull-up enable bit / `PADS_BANK0` 上拉使能位
pub const PAD_PUE: u32 = 1 << 3;

/// `PADS_BANK0` pull-down enable bit / `PADS_BANK0` 下拉使能位
pub const PAD_PDE: u32 = 1 << 2;

/// Electrical state shared by PADS_BANK0 and SIO / PADS_BANK0 与 SIO 共享的电气状态
#[derive(Debug, Clone)]
pub struct GpioPads {
    inner: Arc<Mutex<PadState>>,
}

#[derive(Debug, Default)]
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
}

impl GpioPads {
    /// Create pads with the reset pulls (pull-down on every pin)
    /// 创建使用复位上下拉的 pad（所有引脚下拉）
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(PadState {
                external: 0,
                drive: 0,
                pull_up: 0,
                pull_down: (1u64 << PAD_COUNT) - 1,
            })),
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

    /// Resulting input level for every modelled pin / 每个建模引脚的输入电平
    #[must_use]
    pub fn input_levels(&self) -> u64 {
        let state = self.inner.lock();
        let pulled = state.pull_up & !state.pull_down & !state.drive;
        let driven = state.external & state.drive;
        pulled | driven
    }

    /// Restore reset pulls (pull-down on every pin) / 恢复复位上下拉（所有引脚下拉）
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
}
