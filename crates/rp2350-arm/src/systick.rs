//! SysTick timer — ARMv8-M system timer (PPB)
//! SysTick 定时器 — ARMv8-M 系统定时器（PPB）
//!
//! Implements the SysTick control/status, reload value, current value,
//! and calibration registers. Generates SysTick exception (IRQ 15)
//! when the counter reaches zero.
//! 实现 SysTick 控制/状态、重载值、当前值和校准寄存器。
//! 计数器归零时生成 SysTick 异常（IRQ 15）。

// SysTick 硬件寄存器在 ARM 参考手册中以驼峰命名约定记录，因此文档标记和结构体布局反映了硬件规范。
// SysTick hardware registers are documented with CamelCase naming conventions in
// ARM reference manuals; doc markup and struct layout reflect the hardware spec.
#![allow(
    clippy::doc_markdown,
    clippy::struct_excessive_bools,
    clippy::module_name_repetitions
)]

use rp2350_core::types::CycleCount;

/// SysTick control and status register bits
/// SysTick 控制和状态寄存器的位定义
pub mod csr {
    /// Counter enabled / 计数器使能
    pub const ENABLE: u32 = 1 << 0;
    /// Generate interrupt on zero / 归零时产生中断
    pub const TICKINT: u32 = 1 << 1;
    /// Clock source: 0 = external (divided), 1 = processor clock
    /// 时钟源：0 = 外部（分频），1 = 处理器时钟
    pub const CLKSOURCE: u32 = 1 << 2;
    /// Count flag: set to 1 when counter reaches 0 (write to clear)
    /// 计数标志：计数器归零时设为 1（写操作清除）
    pub const COUNTFLAG: u32 = 1 << 16;
}

/// SysTick timer peripheral
/// SysTick 定时器外设
#[derive(Debug, Clone)]
pub struct SysTick {
    /// Control and Status Register / 控制和状态寄存器
    pub csr: u32,
    /// Reload Value Register (24-bit max) / 重载值寄存器（最大 24 位）
    pub rvr: u32,
    /// Current Value Register (24-bit, counts down) / 当前值寄存器（24 位，递减）
    pub cvr: u32,
    /// Calibration Value Register (read-only) / 校准值寄存器（只读）
    pub calib: u32,
    /// Flag: interrupt pending after countdown / 标志：递减后中断挂起
    pub interrupt_pending: bool,
}

impl SysTick {
    /// Default calibration value for Cortex-M33: TENMS = 250000 (25MHz / 100 = 250k)
    /// Cortex-M33 的默认校准值：TENMS = 250000（25MHz / 100 = 250k）
    pub const DEFAULT_CALIB: u32 = 0xC000_03E8; // NOREF=1, SKEW=1, TENMS=250000

    /// Create a new SysTick timer in reset state
    /// 创建处于复位状态的新 SysTick 定时器
    #[must_use]
    pub fn new() -> Self {
        Self {
            csr: 0,
            rvr: 0,
            cvr: 0,
            calib: Self::DEFAULT_CALIB,
            interrupt_pending: false,
        }
    }

    /// Check if SysTick is enabled
    /// 检查 SysTick 是否使能
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.csr & csr::ENABLE != 0
    }

    /// Check if interrupt generation is enabled
    /// 检查中断生成是否使能
    #[must_use]
    pub fn interrupt_enabled(&self) -> bool {
        self.csr & csr::TICKINT != 0
    }

    /// Check if using processor clock
    /// 检查是否使用处理器时钟
    #[must_use]
    pub fn uses_processor_clock(&self) -> bool {
        self.csr & csr::CLKSOURCE != 0
    }

    /// Tick the SysTick timer by `cycles` clock cycles
    /// 将 SysTick 定时器计时 `cycles` 个时钟周期
    ///
    /// Returns `true` if an interrupt was generated.
    /// 如果产生了中断，返回 `true`。
    pub fn tick(&mut self, cycles: CycleCount) -> bool {
        if !self.is_enabled() {
            return false;
        }

        let mut irq_generated = false;

        for _ in 0..cycles {
            self.cvr = self.cvr.wrapping_sub(1);
            // Counter reached zero (including wrap-around from initial 0)
            // 计数器到达零（包括从初始 0 回绕的情况）
            if self.cvr == 0 || self.cvr > self.rvr {
                self.cvr = self.rvr & 0x00FF_FFFF; // 24-bit max
                self.csr |= csr::COUNTFLAG;
                if self.interrupt_enabled() {
                    irq_generated = true;
                    self.interrupt_pending = true;
                }
            }
        }

        irq_generated
    }

    /// Read the current value register
    /// 读取当前值寄存器
    #[must_use]
    pub fn read_cvr(&self) -> u32 {
        self.cvr
    }

    /// Write to the current value register (clears COUNTFLAG)
    /// 写入当前值寄存器（清除 COUNTFLAG）
    pub fn write_cvr(&mut self, value: u32) {
        self.cvr = value & 0x00FF_FFFF;
        self.csr &= !csr::COUNTFLAG;
    }

    /// Write to the reload value register
    /// 写入重载值寄存器
    pub fn write_rvr(&mut self, value: u32) {
        self.rvr = value & 0x00FF_FFFF;
    }

    /// Clear the interrupt pending flag
    /// 清除中断挂起标志
    pub fn clear_interrupt(&mut self) {
        self.interrupt_pending = false;
    }
}

impl Default for SysTick {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systick_disabled_no_count() {
        let mut st = SysTick::new();
        st.rvr = 100;
        st.cvr = 100;
        assert!(!st.tick(100));
        assert_eq!(st.cvr, 100); // No change when disabled
    }

    #[test]
    fn systick_counts_down() {
        let mut st = SysTick::new();
        st.csr = csr::ENABLE;
        st.rvr = 100;
        st.cvr = 100;
        st.tick(50);
        assert_eq!(st.cvr, 50);
    }

    #[test]
    fn systick_reload_and_interrupt() {
        let mut st = SysTick::new();
        st.csr = csr::ENABLE | csr::TICKINT;
        st.rvr = 10;
        st.cvr = 10;
        let irq = st.tick(10);
        assert!(irq);
        assert!(st.interrupt_pending);
        assert_eq!(st.cvr, 10); // Reloaded
        assert!(st.csr & csr::COUNTFLAG != 0);
    }

    #[test]
    fn systick_no_interrupt_without_tickint() {
        let mut st = SysTick::new();
        st.csr = csr::ENABLE;
        st.rvr = 10;
        st.cvr = 10;
        let irq = st.tick(10);
        assert!(!irq);
        assert!(!st.interrupt_pending);
    }
}
