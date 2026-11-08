//! Private peripheral bus stub / 私有外设总线桩
//!
//! The PPB window (SCB, NVIC, SysTick, MPU) is plain storage so `ldr`/`str`
//! accesses from firmware do not halt on an unmapped address. `CPUID` reports a
//! Cortex-M33 part number. Writes to the NVIC and SysTick register ranges are
//! also appended to a shared log; the simulation driver replays them into the
//! core through `CpuCore::system_write`, which is how interrupt enables,
//! priorities and SysTick configuration reach the emulated core.
//! PPB 窗口（SCB、NVIC、SysTick、MPU）为普通存储，固件发出的 `ldr`/`str`
//! 不会因地址未映射而停机。`CPUID` 报告 Cortex-M33 部件号。对 NVIC 与 SysTick
//! 寄存器范围的写入还会追加到共享日志；仿真驱动通过 `CpuCore::system_write`
//! 把它们回放到核中，中断使能、优先级与 SysTick 配置由此到达被模拟的核。

use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// `SCB_CPUID` offset / `SCB_CPUID` 偏移
const SCB_CPUID: u32 = 0xE000_ED00 - map::PPB_BASE;

/// Cortex-M33 `CPUID` value (implementer ARM, part 0xD21)
/// Cortex-M33 `CPUID` 值（实现者 ARM，部件号 0xD21）
const CPUID_CORTEX_M33: u32 = 0x410F_D210;

/// Maximum logged writes kept for replay / 为回放保留的最大日志条数
///
/// Boot-time configuration fits well within this bound; steady-state `ICPR`
/// writes beyond it are dropped, which only delays cleared pending bits.
/// 启动期配置远小于该上限；超出的稳态 `ICPR` 写会被丢弃，仅会延迟已清除
/// 的挂起位生效。
const LOG_CAPACITY: usize = 256;

/// Shared write log `(absolute address, value)` / 共享写入日志（绝对地址、值）
pub type SystemWriteLog = Arc<Mutex<Vec<(u32, u32)>>>;

/// Create an empty system write log / 创建空的系统写入日志
#[must_use]
pub fn new_system_write_log() -> SystemWriteLog {
    Arc::new(Mutex::new(Vec::new()))
}

/// Is `addr` a system control space register replayed into the core?
/// `addr` 是否为需要回放到核的系统控制空间寄存器？
fn is_replayed(addr: u32) -> bool {
    addr == 0xE000_ED08 // SCB_VTOR
        || (0xE000_E010..=0xE000_E01F).contains(&addr) // SysTick
        || (0xE000_E100..=0xE000_E4FF).contains(&addr) // NVIC
}

/// PPB window stub / PPB 窗口桩
#[derive(Debug)]
pub struct Ppb {
    regs: RegisterFile,
    system_writes: SystemWriteLog,
}

impl Ppb {
    /// Create the window with a private log / 创建窗口并使用私有日志
    #[must_use]
    pub fn new() -> Self {
        Self::with_log(new_system_write_log())
    }

    /// Create the window sharing `system_writes` / 创建窗口并共享 `system_writes`
    #[must_use]
    pub fn with_log(system_writes: SystemWriteLog) -> Self {
        Self {
            regs: RegisterFile::new(map::PPB_WINDOW_SIZE),
            system_writes,
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
        if offset == SCB_CPUID {
            return true;
        }
        let addr = map::PPB_BASE.saturating_add(offset);
        if is_replayed(addr) {
            let mut log = self.system_writes.lock();
            if log.len() < LOG_CAPACITY {
                log.push((addr, value));
            }
        }
        let _ = self.regs.write(offset, value);
        true
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

    #[test]
    fn nvic_writes_are_logged_for_replay() {
        let log = new_system_write_log();
        let mut ppb = Ppb::with_log(Arc::clone(&log));
        // ISER0 at 0xE000_E100. ISER0 位于 0xE000_E100。
        assert!(ppb.write32(0xE100, 1));
        assert_eq!(log.lock().as_slice(), &[(0xE000_E100, 1)]);
    }

    #[test]
    fn vtor_writes_are_logged_for_replay() {
        let log = new_system_write_log();
        let mut ppb = Ppb::with_log(Arc::clone(&log));
        // SCB_VTOR at 0xE000_ED08. SCB_VTOR 位于 0xE000_ED08。
        assert!(ppb.write32(0xED08, 0x1000_0000));
        assert_eq!(log.lock().as_slice(), &[(0xE000_ED08, 0x1000_0000)]);
    }

    #[test]
    fn plain_ppb_writes_are_not_logged() {
        let log = new_system_write_log();
        let mut ppb = Ppb::with_log(Arc::clone(&log));
        // AIRCR at 0xE000_ED0C is plain storage in this stub.
        // 本桩中 0xE000_ED0C 的 AIRCR 只是普通存储。
        let offset = 0xE000_ED0C - map::PPB_BASE;
        assert!(ppb.write32(offset, 0x05FA_0000));
        assert!(log.lock().is_empty());
    }
}
