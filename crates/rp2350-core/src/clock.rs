//! Simulation clock with configurable frequency and divider chain
//! 仿真时钟，支持可配置频率和分频器链

use super::types::CycleCount;

/// System clock frequencies (hardware defaults for RP2350)
/// 系统时钟频率（RP2350 硬件默认值）
#[derive(Debug, Clone, Copy)]
pub struct ClockConfig {
    /// Reference clock (typically 12 MHz from XOSC) / 参考时钟（通常为 XOSC 12 MHz）
    pub refclk_hz: u32,
    /// System clock after PLL / PLL 后的系统时钟
    pub sysclk_hz: u32,
    /// Peripheral clock (sysclk / divider) / 外设时钟（sysclk / 分频）
    pub periphclk_hz: u32,
}

impl Default for ClockConfig {
    fn default() -> Self {
        Self {
            refclk_hz: 12_000_000,
            sysclk_hz: 150_000_000,
            periphclk_hz: 150_000_000,
        }
    }
}

/// Simulation clock tracker
/// 仿真时钟追踪器
///
/// Tracks the total cycle count and provides tick advancement with
/// configurable clock frequency.
/// 追踪总周期计数，支持可配置时钟频率的 tick 推进。
pub struct SimulationClock {
    /// Total cycles elapsed since simulation start
    /// 自仿真启动以来经过的总周期数
    total_cycles: CycleCount,
    /// Clock configuration / 时钟配置
    config: ClockConfig,
}

impl SimulationClock {
    /// Create a new simulation clock with default RP2350 configuration
    /// 使用默认 RP2350 配置创建仿真时钟
    #[must_use]
    pub fn new() -> Self {
        Self {
            total_cycles: 0,
            config: ClockConfig::default(),
        }
    }

    /// Create with custom clock config / 使用自定义时钟配置创建
    #[must_use]
    pub fn with_config(config: ClockConfig) -> Self {
        Self {
            total_cycles: 0,
            config,
        }
    }

    /// Get total cycles elapsed / 获取经过的总周期数
    #[must_use]
    pub fn total_cycles(&self) -> CycleCount {
        self.total_cycles
    }

    /// Get current clock config / 获取当前时钟配置
    #[must_use]
    pub fn config(&self) -> ClockConfig {
        self.config
    }

    /// Update the system clock frequency / 更新系统时钟频率
    ///
    /// This only changes the value the tracker reports; the timing consumers
    /// (`SysTick`, `TIMER`, PIO, HSTX, ...) are wired to the live clock tree in a
    /// follow-up task.
    /// 只改变追踪器报告的值；各计时消费者（SysTick、TIMER、PIO、HSTX 等）将在
    /// 后续任务中接入实时时钟树。
    pub fn set_sysclk_hz(&mut self, hz: u32) {
        self.config.sysclk_hz = hz;
    }

    /// Update the peripheral clock frequency / 更新外设时钟频率
    pub fn set_periphclk_hz(&mut self, hz: u32) {
        self.config.periphclk_hz = hz;
    }

    /// Update the reference clock frequency / 更新参考时钟频率
    pub fn set_refclk_hz(&mut self, hz: u32) {
        self.config.refclk_hz = hz;
    }

    /// Advance the simulation by `n` system clock cycles
    /// 将仿真推进 n 个系统时钟周期
    pub fn tick(&mut self, cycles: CycleCount) {
        self.total_cycles += cycles;
    }

    /// Read the `SysTick` counter value (24-bit down-counter)
    /// 读取 `SysTick` 计数器值（24 位递减计数器）
    #[must_use]
    pub fn systick_value(&self, reload: u32, enable: bool) -> u32 {
        if !enable || reload == 0 {
            return 0;
        }
        #[allow(clippy::cast_possible_truncation)]
        let elapsed = (self.total_cycles % u64::from(reload + 1)) as u32;
        reload - elapsed
    }

    /// Calculate peripheral clock divider cycles for a given peripheral frequency
    /// 计算外设时钟分频器周期数
    #[must_use]
    pub fn peripheral_cycles(&self, target_hz: u32, ticks: CycleCount) -> CycleCount {
        if target_hz == 0 || self.config.periphclk_hz == 0 {
            return 0;
        }
        ticks * u64::from(target_hz) / u64::from(self.config.periphclk_hz)
    }
}

impl Default for SimulationClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_increments() {
        let mut clock = SimulationClock::new();
        assert_eq!(clock.total_cycles(), 0);
        clock.tick(100);
        assert_eq!(clock.total_cycles(), 100);
        clock.tick(50);
        assert_eq!(clock.total_cycles(), 150);
    }

    #[test]
    fn peripheral_cycles_scaling() {
        let clock = SimulationClock::new();
        // periphclk = 150MHz, target = 75MHz → 1/2 rate
        let periph = clock.peripheral_cycles(75_000_000, 100);
        assert_eq!(periph, 50);
    }

    #[test]
    fn zero_frequency_returns_zero() {
        let clock = SimulationClock::new();
        assert_eq!(clock.peripheral_cycles(0, 100), 0);
    }

    #[test]
    fn set_sysclk_hz_updates_the_reported_config() {
        // 运行时超频：150 MHz → 252 MHz 只改报告值，周期计数不变。
        // Runtime overclock: 150 MHz → 252 MHz only changes the reported
        // config; the cycle count is untouched.
        let mut clock = SimulationClock::new();
        clock.tick(42);
        clock.set_sysclk_hz(252_000_000);
        assert_eq!(clock.config().sysclk_hz, 252_000_000);
        assert_eq!(clock.config().refclk_hz, 12_000_000);
        assert_eq!(clock.total_cycles(), 42);
    }

    #[test]
    fn set_periphclk_hz_drives_peripheral_scaling() {
        // 外设时钟 252 MHz、目标 126 MHz → 每 2 个周期 1 个外设周期。
        // periphclk 252 MHz with a 126 MHz target → one peripheral cycle
        // every two ticks.
        let mut clock = SimulationClock::new();
        clock.set_periphclk_hz(252_000_000);
        assert_eq!(clock.peripheral_cycles(126_000_000, 100), 50);
        clock.set_refclk_hz(24_000_000);
        assert_eq!(clock.config().refclk_hz, 24_000_000);
    }
}
