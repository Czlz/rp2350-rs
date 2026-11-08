//! Chip family identification and the RP2040 stub
//! 芯片家族识别与 RP2040 桩实现
//!
//! `rp2350-rs` targets the RP2350 first. RP2040 support is intentionally a
//! stub: the chip can be constructed and identified, but every operational
//! entry point returns [`CoreError::UnsupportedChip`] instead of silently
//! pretending to work.
//!
//! `rp2350-rs` 以 RP2350 为首要目标。RP2040 目前是有意保留的桩：芯片可以构造并
//! 识别，但所有操作入口都返回 [`CoreError::UnsupportedChip`]，而不是静默地假装
//! 可用。

use super::bus::AddressBus;
use super::error::CoreError;
use super::types::Address;

/// Supported chip families / 支持的芯片家族
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChipFamily {
    /// RP2040 (dual Cortex-M0+, not implemented) / RP2040（双 Cortex-M0+，未实现）
    Rp2040,
    /// RP2350 (Cortex-M33 or Hazard3) / RP2350（Cortex-M33 或 Hazard3）
    Rp2350,
}

impl ChipFamily {
    /// Short machine-readable name / 简短的机器可读名称
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Rp2040 => "rp2040",
            Self::Rp2350 => "rp2350",
        }
    }
}

/// Static capabilities of a chip family / 芯片家族的静态能力描述
///
/// Sizes are documented in bytes; `gpio_pins` counts user-accessible GPIOs.
/// 尺寸以字节为单位；`gpio_pins` 为可供用户使用的 GPIO 数量。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChipCapabilities {
    /// Chip family / 芯片家族
    pub family: ChipFamily,
    /// Addressable external flash window in bytes / 可寻址外部 Flash 窗口字节数
    pub flash_bytes: usize,
    /// On-chip SRAM in bytes / 片上 SRAM 字节数
    pub sram_bytes: usize,
    /// User GPIO count / 用户 GPIO 数量
    pub gpio_pins: usize,
    /// Whether a Hazard3 RISC-V core is available / 是否提供 Hazard3 RISC-V 核心
    pub hazard3: bool,
    /// Whether a USB device controller is available / 是否提供 USB 设备控制器
    pub usb_device: bool,
    /// Whether the chip has two symmetric cores / 是否具有两个对等核心
    pub dual_core: bool,
}

impl ChipCapabilities {
    /// Capabilities of the RP2040 / RP2040 的能力
    #[must_use]
    pub const fn rp2040() -> Self {
        Self {
            family: ChipFamily::Rp2040,
            flash_bytes: 2 * 1024 * 1024,
            sram_bytes: 264 * 1024,
            gpio_pins: 30,
            hazard3: false,
            usb_device: true,
            dual_core: true,
        }
    }

    /// Capabilities of the RP2350 (16 MB XIP window, 520 KB SRAM)
    /// RP2350 的能力（16 MB XIP 窗口，520 KB SRAM）
    #[must_use]
    pub const fn rp2350() -> Self {
        Self {
            family: ChipFamily::Rp2350,
            flash_bytes: 16 * 1024 * 1024,
            sram_bytes: 520 * 1024,
            gpio_pins: 48,
            hazard3: true,
            usb_device: true,
            dual_core: true,
        }
    }
}

/// Common identification surface implemented by every chip model
/// 所有芯片模型共同实现的识别接口
pub trait ChipInfo {
    /// Return the chip family / 返回芯片家族
    fn family(&self) -> ChipFamily;

    /// Return the static capability description / 返回静态能力描述
    fn capabilities(&self) -> ChipCapabilities;
}

/// Constructible RP2040 stub / 可构造的 RP2040 桩
///
/// The type exists so hosts can select and identify an RP2040 target without
/// pretending the core, memory map or peripherals are available.
/// 该类型的存在使宿主可以选择并识别 RP2040 目标，同时不假装核心、内存映射或
/// 外设已经可用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RP2040Chip;

impl RP2040Chip {
    /// Create an RP2040 stub / 创建一个 RP2040 桩
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Attempt to boot the stub / 尝试启动桩
    ///
    /// # Errors
    /// Always returns [`CoreError::UnsupportedChip`]; the RP2040 core and boot
    /// flow are not implemented yet.
    /// 始终返回 [`CoreError::UnsupportedChip`]；RP2040 核心与启动流程尚未实现。
    pub fn boot(&mut self, _bus: &mut dyn AddressBus) -> Result<(), CoreError> {
        Err(unsupported("boot / 启动"))
    }

    /// Attempt to execute one step / 尝试执行单步
    ///
    /// # Errors
    /// Always returns [`CoreError::UnsupportedChip`]; there is no RP2040 core.
    /// 始终返回 [`CoreError::UnsupportedChip`]；尚不存在 RP2040 核心。
    pub fn step(&mut self, _bus: &mut dyn AddressBus) -> Result<(), CoreError> {
        Err(unsupported("step / 单步执行"))
    }

    /// Attempt to read memory / 尝试读取内存
    ///
    /// # Errors
    /// Always returns [`CoreError::UnsupportedChip`]; the RP2040 memory map is
    /// not modelled.
    /// 始终返回 [`CoreError::UnsupportedChip`]；RP2040 内存映射尚未建模。
    pub fn read_u32(&self, _address: Address) -> Result<u32, CoreError> {
        Err(unsupported("memory read / 内存读取"))
    }

    /// Attempt to write memory / 尝试写入内存
    ///
    /// # Errors
    /// Always returns [`CoreError::UnsupportedChip`]; the RP2040 memory map is
    /// not modelled.
    /// 始终返回 [`CoreError::UnsupportedChip`]；RP2040 内存映射尚未建模。
    pub fn write_u32(&mut self, _address: Address, _value: u32) -> Result<(), CoreError> {
        Err(unsupported("memory write / 内存写入"))
    }
}

impl ChipInfo for RP2040Chip {
    fn family(&self) -> ChipFamily {
        ChipFamily::Rp2040
    }

    fn capabilities(&self) -> ChipCapabilities {
        ChipCapabilities::rp2040()
    }
}

/// Build the stub error with the offending operation / 构造包含操作名的桩错误
fn unsupported(operation: &str) -> CoreError {
    CoreError::UnsupportedChip(format!(
        "RP2040 {operation} is not implemented / RP2040 {operation} 尚未实现"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rp2040_is_constructible_and_identifiable() {
        let chip = RP2040Chip::new();
        assert_eq!(chip.family(), ChipFamily::Rp2040);
        let caps = chip.capabilities();
        assert_eq!(caps.family, ChipFamily::Rp2040);
        assert!(!caps.hazard3);
        assert_eq!(caps.gpio_pins, 30);
    }

    #[test]
    fn rp2040_operations_report_unsupported() {
        let mut chip = RP2040Chip::new();
        assert!(matches!(
            chip.read_u32(0x1000_0000),
            Err(CoreError::UnsupportedChip(_))
        ));
        assert!(matches!(
            chip.write_u32(0x2000_0000, 0),
            Err(CoreError::UnsupportedChip(_))
        ));
        let message = chip
            .write_u32(0x2000_0000, 0)
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default();
        assert!(message.contains("RP2040"));
        assert!(message.contains("not implemented"));
    }

    #[test]
    fn rp2350_capabilities_match_memory_map() {
        let caps = ChipCapabilities::rp2350();
        assert_eq!(caps.family, ChipFamily::Rp2350);
        assert_eq!(caps.sram_bytes, 520 * 1024);
        assert_eq!(caps.flash_bytes, 16 * 1024 * 1024);
        assert!(caps.hazard3);
    }
}
