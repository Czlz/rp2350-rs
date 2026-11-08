//! POWMAN peripheral stub / POWMAN 外设桩
//!
//! Minimal model of the always-on power manager window at `0x4010_0000`.
//! Registers are plain storage except for two behaviours that matter to the
//! boot path:
//!
//! * Writes carrying the `0x5AFE` password in bits 31:16 are accepted and only
//!   the 16-bit data field is stored, matching the register protection in the
//!   datasheet.
//! * `VREG_STS.VOUT_OK` reads as 1 because the regulator already finished its
//!   startup sequence by the time firmware runs.
//!
//! `SET_TIME_*`/`READ_TIME_*` do not advance a real clock: a static timer is
//! enough for the boot code, which only needs stable reads and accepted writes.
//!
//! `0x4010_0000` 处常开电源管理器窗口的最小模型。除两处影响启动路径的行为外，
//! 寄存器均为普通存储：
//!
//! * 高 16 位携带 `0x5AFE` 口令的写入被接受，仅保存 16 位数据字段，与数据手册
//!   的寄存器保护一致。
//! * `VREG_STS.VOUT_OK` 读为 1，因为固件执行时稳压器已完成启动序列。
//!
//! `SET_TIME_*`/`READ_TIME_*` 不会推进真实时钟：启动代码只需要稳定的读值与
//! 被接受的写入，静态定时器即可满足。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// `VREG_STS` offset / `VREG_STS` 偏移
const REG_VREG_STS: u32 = 0x08;

/// `VREG_STS.VOUT_OK`: output regulation finished / 输出稳压完成
const VREG_VOUT_OK: u32 = 1 << 4;

/// Write password in bits 31:16 / 位于第 31:16 位的写入口令
const PASSWORD: u32 = 0x5AFE;

/// Mask of the protected 16-bit data field / 受保护 16 位数据字段的掩码
const DATA_MASK: u32 = 0xFFFF;

/// POWMAN register block / POWMAN 寄存器块
#[derive(Debug)]
pub struct Powman {
    regs: RegisterFile,
}

impl Powman {
    /// Create the block / 创建寄存器块
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: RegisterFile::new(map::DEVICE_WINDOW_SIZE),
        }
    }
}

impl Default for Powman {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for Powman {
    fn name(&self) -> &str {
        "POWMAN"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::POWMAN_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        let value = self.regs.read(offset)?;
        if offset == REG_VREG_STS {
            return Some(value | VREG_VOUT_OK);
        }
        Some(value)
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        // 口令写入只保留低 16 位数据；普通写入原样保存。
        // Password writes keep only the 16-bit data field; plain writes are
        // stored unchanged.
        let stored = if value >> 16 == PASSWORD {
            value & DATA_MASK
        } else {
            value
        };
        self.regs.write(offset, stored)
    }

    fn reset(&mut self) {
        self.regs.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `TIMER` offset: `RUN` is bit 1 / `TIMER` 偏移：`RUN` 为第 1 位
    const REG_TIMER: u32 = 0x88;

    #[test]
    fn password_write_is_accepted_and_reads_back_as_data() {
        let mut powman = Powman::new();
        assert!(powman.write32(REG_TIMER, 0x5AFE_0002));
        assert_eq!(powman.read32(REG_TIMER), Some(0x0000_0002));
    }

    #[test]
    fn set_time_fields_keep_only_the_low_half() {
        let mut powman = Powman::new();
        assert!(powman.write32(0x60, 0x5AFE_1234));
        assert_eq!(powman.read32(0x60), Some(0x1234));
    }

    #[test]
    fn vreg_status_reports_output_ok() {
        let mut powman = Powman::new();
        assert_eq!(powman.read32(REG_VREG_STS), Some(VREG_VOUT_OK));
    }

    #[test]
    fn reset_clears_the_password_protected_state() {
        let mut powman = Powman::new();
        assert!(powman.write32(REG_TIMER, 0x5AFE_0002));
        powman.reset();
        assert_eq!(powman.read32(REG_TIMER), Some(0));
    }
}
