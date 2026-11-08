//! Foundational types for the RP2350 emulator
//! RP2350 模拟器的基础类型定义

/// Physical memory address (32-bit)
/// 物理内存地址（32 位）
pub type Address = u32;

/// IRQ number (0-255, NVIC supports 256 interrupts)
/// 中断号（0-255，NVIC 支持 256 个中断）
pub type IrqNumber = u16;

/// GPIO pin number (0-47 on RP2350 QFN-60 package, up to 62 total)
/// GPIO 引脚号（RP2350 QFN-60 封装为 0-47，总计可达 62）
pub type PinNumber = u8;

/// Logical GPIO bank + index (0-5 banks)
/// 逻辑 GPIO 分组号（0-5 组）
pub type GpioNumber = u8;

/// Simulation cycle counter (64-bit to avoid overflow during long runs)
/// 仿真周期计数器（64 位，避免长时间运行溢出）
pub type CycleCount = u64;

/// Number of GPIO pins on RP2350
/// RP2350 的 GPIO 引脚数量
pub const GPIO_PIN_COUNT: usize = 48;

/// Total IRQ lines (NVIC: 240 external + 16 system reserved)
/// 总中断线数（NVIC：240 外部 + 16 系统保留）
pub const IRQ_COUNT: usize = 256;

/// Number of DMA channels
/// DMA 通道数
pub const DMA_CHANNEL_COUNT: usize = 16;

/// RP2350 CPU index (0 = Core0, 1 = Core1)
/// RP2350 CPU 索引（0 = Core0，1 = Core1）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum CpuId {
    /// Core 0 (ARM Cortex-M33 primary)
    Core0 = 0,
    /// Core 1 (ARM Cortex-M33 secondary)
    Core1 = 1,
}

impl CpuId {
    /// Return the other core / 返回另一个核心
    #[must_use]
    pub fn other(self) -> Self {
        match self {
            Self::Core0 => Self::Core1,
            Self::Core1 => Self::Core0,
        }
    }
}
