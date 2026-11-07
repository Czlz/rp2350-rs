//! RP2350 chip topology — connects CPU cores, address bus, peripherals, GPIO
//! RP2350 芯片拓扑 — 连接 CPU 核心、地址总线、外设和 GPIO

use super::clock::SimulationClock;
use super::gpio::GpioPin;
use super::irq::IrqMatrix;
use super::types::{PinNumber, GPIO_PIN_COUNT};

/// The top-level RP2350 chip structure
/// RP2350 芯片顶层结构
///
/// Owns all simulation state: GPIO pins, IRQ matrix, clock, and provides
/// references to CPU cores and peripherals.
/// 拥有所有仿真状态：GPIO 引脚、中断矩阵、时钟，并提供 CPU 核心和外设的引用。
pub struct RP2350Chip {
    /// All GPIO pins (QFN-60: 0-47 available)
    /// 所有 GPIO 引脚（QFN-60 封装：0-47 可用）
    pub gpio_pins: [GpioPin; GPIO_PIN_COUNT],

    /// Interrupt routing matrix / 中断路由矩阵
    pub irq_matrix: IrqMatrix,

    /// Simulation clock / 仿真时钟
    pub clock: SimulationClock,
}

impl RP2350Chip {
    /// Create a new RP2350 chip with all peripherals reset
    /// 创建一个新的 RP2350 芯片，所有外设已复位
    #[must_use]
    pub fn new() -> Self {
        Self {
            gpio_pins: std::array::from_fn(|i| GpioPin::new(i as PinNumber)),
            irq_matrix: IrqMatrix::new(),
            clock: SimulationClock::new(),
        }
    }

    /// Get a reference to a GPIO pin / 获取 GPIO 引脚引用
    #[must_use]
    pub fn gpio(&self, pin: usize) -> Option<&GpioPin> {
        self.gpio_pins.get(pin)
    }

    /// Get a mutable reference to a GPIO pin / 获取 GPIO 引脚可变引用
    pub fn gpio_mut(&mut self, pin: usize) -> Option<&mut GpioPin> {
        self.gpio_pins.get_mut(pin)
    }

    /// Advance the simulation by one tick
    /// 推进仿真一个时钟周期
    pub fn tick(&mut self, cycles: u64) {
        self.clock.tick(cycles);
    }
}

impl Default for RP2350Chip {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_chip_has_all_pins() {
        let chip = RP2350Chip::new();
        assert_eq!(chip.gpio_pins.len(), GPIO_PIN_COUNT);
        for (i, pin) in chip.gpio_pins.iter().enumerate() {
            assert_eq!(pin.number(), i as PinNumber);
        }
    }

    #[test]
    fn gpio_bounds_check() {
        let mut chip = RP2350Chip::new();
        assert!(chip.gpio_mut(48).is_none());
        assert!(chip.gpio_mut(0).is_some());
    }
}