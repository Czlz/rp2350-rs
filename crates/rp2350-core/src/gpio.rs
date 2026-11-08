//! GPIO pin model for RP2350
//! RP2350 的 GPIO 引脚模型

use bitflags::bitflags;

use super::types::PinNumber;

bitflags! {
    /// GPIO interrupt flags / GPIO 中断标志
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct GpioIntFlags: u8 {
        /// Level-low triggered / 低电平触发
        const LEVEL_LOW = 0x01;
        /// Level-high triggered / 高电平触发
        const LEVEL_HIGH = 0x02;
        /// Falling edge triggered / 下降沿触发
        const EDGE_FALL = 0x04;
        /// Rising edge triggered / 上升沿触发
        const EDGE_RISE = 0x08;
    }
}

/// GPIO pin direction
/// GPIO 引脚方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinDirection {
    /// Input (high-impedance) / 输入（高阻态）
    Input,
    /// Output (driving) / 输出（驱动）
    Output,
}

/// GPIO pin function selection (F1-F5, NULL, SIO, RTS)
/// GPIO 引脚功能选择
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinFunction {
    /// Function disabled / 功能禁用
    Null,
    /// Software-controlled I/O (SIO) / 软件控制的 I/O
    Sio,
    /// Function 1 (e.g. SPI, UART) / 功能 1
    F1,
    /// Function 2 / 功能 2
    F2,
    /// Function 3 / 功能 3
    F3,
    /// Function 4 / 功能 4
    F4,
    /// Function 5 / 功能 5
    F5,
    /// RTS (pin debug access) / RTS（引脚调试访问）
    Rts,
}

/// Represents a single GPIO pin on the RP2350
/// 表示 RP2350 上的单个 GPIO 引脚
#[derive(Debug, Clone)]
#[allow(dead_code, clippy::struct_excessive_bools)]
pub struct GpioPin {
    /// Pin number (0-47) / 引脚号（0-47）
    number: PinNumber,
    /// Current direction / 当前方向
    direction: PinDirection,
    /// Current function / 当前功能
    function: PinFunction,
    /// Output level (true = high) / 输出电平（true = 高）
    output_level: bool,
    /// Input level (true = high) / 输入电平（true = 高）
    input_level: bool,
    /// Pull-up enabled / 上拉使能
    pull_up: bool,
    /// Pull-down enabled / 下拉使能
    pull_down: bool,
    /// Input buffer enabled / 输入缓冲使能
    input_enable: bool,
    /// Interrupt enable flags / 中断使能标志
    interrupt_enable: GpioIntFlags,
    /// Interrupt status flags / 中断状态标志
    interrupt_status: GpioIntFlags,
    /// Output enable override (for peripheral control) / 输出使能覆写（外设控制用）
    oe_over: bool,
    /// Output value override / 输出值覆写
    out_over: bool,
}

impl GpioPin {
    /// Create a new GPIO pin with default (input) configuration
    /// 创建一个新的 GPIO 引脚，默认配置为输入
    #[must_use]
    pub fn new(number: PinNumber) -> Self {
        Self {
            number,
            direction: PinDirection::Input,
            function: PinFunction::Null,
            output_level: false,
            input_level: false,
            pull_up: false,
            pull_down: false,
            input_enable: false,
            interrupt_enable: GpioIntFlags::empty(),
            interrupt_status: GpioIntFlags::empty(),
            oe_over: false,
            out_over: false,
        }
    }

    /// Get pin number / 获取引脚号
    #[must_use]
    pub fn number(&self) -> PinNumber {
        self.number
    }

    /// Get current direction / 获取当前方向
    #[must_use]
    pub fn direction(&self) -> PinDirection {
        self.direction
    }

    /// Set pin direction / 设置引脚方向
    pub fn set_direction(&mut self, dir: PinDirection) {
        self.direction = dir;
    }

    /// Get current function / 获取当前功能
    #[must_use]
    pub fn function(&self) -> PinFunction {
        self.function
    }

    /// Set pin function / 设置引脚功能
    pub fn set_function(&mut self, func: PinFunction) {
        self.function = func;
    }

    /// Set output level / 设置输出电平
    pub fn set_output(&mut self, level: bool) {
        self.output_level = level;
    }

    /// Get effective level (from output or external) / 获取有效电平
    #[must_use]
    pub fn level(&self) -> bool {
        match self.direction {
            PinDirection::Output => self.output_level,
            PinDirection::Input => self.input_level,
        }
    }

    /// Set input level (called by external source, e.g. `DeviceManager`)
    /// 设置输入电平（由外部源调用，如 `DeviceManager`）
    pub fn set_input(&mut self, level: bool) {
        self.input_level = level;
    }

    /// Enable pull-up resistor / 使能上拉电阻
    pub fn set_pull_up(&mut self, enable: bool) {
        self.pull_up = enable;
    }

    /// Enable pull-down resistor / 使能下拉电阻
    pub fn set_pull_down(&mut self, enable: bool) {
        self.pull_down = enable;
    }

    /// Enable input buffer / 使能输入缓冲
    pub fn set_input_enable(&mut self, enable: bool) {
        self.input_enable = enable;
    }

    /// Check if interrupt is pending / 检查是否有待处理中断
    #[must_use]
    pub fn interrupt_pending(&self) -> bool {
        !self.interrupt_status.is_empty()
    }

    /// Clear interrupt flags / 清除中断标志
    pub fn clear_interrupt(&mut self, flags: GpioIntFlags) {
        self.interrupt_status.remove(flags);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_pin_is_input() {
        let pin = GpioPin::new(0);
        assert_eq!(pin.direction(), PinDirection::Input);
        assert_eq!(pin.function(), PinFunction::Null);
    }

    #[test]
    fn output_level_reflected() {
        let mut pin = GpioPin::new(5);
        pin.set_direction(PinDirection::Output);
        pin.set_output(true);
        assert!(pin.level());
        pin.set_output(false);
        assert!(!pin.level());
    }

    #[test]
    fn input_tracks_external() {
        let mut pin = GpioPin::new(10);
        pin.set_input(true);
        assert!(pin.level());
        pin.set_input(false);
        assert!(!pin.level());
    }
}
