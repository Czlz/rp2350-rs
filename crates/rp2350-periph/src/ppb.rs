//! Private peripheral bus stub / 私有外设总线桩
//!
//! The PPB window (SCB, NVIC, SysTick, MPU) is plain storage so `ldr`/`str`
//! accesses from firmware do not halt on an unmapped address. `CPUID` reports a
//! Cortex-M33 part number. The emulated core keeps its own NVIC and SysTick
//! state; writes to this window do not reconfigure the core.
//! PPB 窗口（SCB、NVIC、SysTick、MPU）为普通存储，固件发出的 `ldr`/`str`
//! 不会因地址未映射而停机。`CPUID` 报告 Cortex-M33 部件号。被模拟的核维护
//! 自己的 NVIC 与 SysTick 状态；写入该窗口不会重新配置核。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// `SCB_CPUID` offset / `SCB_CPUID` 偏移
const SCB_CPUID: u32 = 0xE000_ED00 - map::PPB_BASE;

/// Cortex-M33 `CPUID` value (implementer ARM, part 0xD21)
/// Cortex-M33 `CPUID` 值（实现者 ARM，部件号 0xD21）
const CPUID_CORTEX_M33: u32 = 0x410F_D210;

/// PPB window stub / PPB 窗口桩
#[derive(Debug)]
pub struct Ppb {
    regs: RegisterFile,
}

impl Ppb {
    /// Create the window / 创建窗口
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: RegisterFile::new(map::PPB_WINDOW_SIZE),
        }
    }
}

impl Default for Ppb {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for Ppb {
    fn name(&self) -> &str {
        "PPB"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::PPB_BASE, map::PPB_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        match offset {
            SCB_CPUID => Some(CPUID_CORTEX_M33),
            _ => self.regs.read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        match offset {
            SCB_CPUID => true,
            _ => self.regs.write(offset, value),
        }
    }

    fn reset(&mut self) {
        self.regs.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_id_is_readable() {
        let mut ppb = Ppb::new();
        assert_eq!(ppb.read32(SCB_CPUID), Some(CPUID_CORTEX_M33));
    }

    #[test]
    fn vtor_is_storage() {
        let mut ppb = Ppb::new();
        let offset = 0xE000_ED08 - map::PPB_BASE;
        assert!(ppb.write32(offset, 0x1000_0000));
        assert_eq!(ppb.read32(offset), Some(0x1000_0000));
    }
}
