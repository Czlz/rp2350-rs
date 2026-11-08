//! `Peripheral` trait — base trait for all APB memory-mapped peripherals
//! `Peripheral` trait — 所有 APB 内存映射外设的基础 trait

use rp2350_core::bus::MemoryRegion;

/// A memory-mapped peripheral connected to the APB bus.
/// 连接到 APB 总线的内存映射外设。
///
/// Each peripheral must provide its base address, register layout,
/// and interrupt number if applicable.
/// 每个外设必须提供其基地址、寄存器布局和中断号（如适用）。
pub trait Peripheral: MemoryRegion + Send + Sync {
    /// Peripheral name (e.g. "PIO0", "HSTX", "DMA") / 外设名称
    fn peripheral_name(&self) -> &str;

    /// Reset the peripheral to power-on defaults / 复位外设到上电默认值
    fn reset(&mut self);

    /// Interrupt number associated with this peripheral (if any)
    /// 与此外设关联的中断号（如果有）
    fn irq_number(&self) -> Option<u16> {
        None
    }

    /// Called each simulation tick for peripheral state updates
    /// 每个仿真 tick 调用，用于外设状态更新
    fn on_tick(&mut self, _cycles: u64) {}
}
