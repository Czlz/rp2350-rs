//! POWMAN peripheral model / POWMAN 外设模型
//!
//! Minimal model of the always-on power manager window at `0x4010_0000`.
//! Registers are plain storage except for these behaviours:
//!
//! * Writes carrying the `0x5AFE` password in bits 31:16 are accepted and only
//!   the 16-bit data field is stored, matching the register protection in the
//!   datasheet.
//! * `VREG_STS.VOUT_OK` reads as 1 because the regulator already finished its
//!   startup sequence by the time firmware runs.
//! * The AON timer (`TIMER.RUN`) counts `clk_rtc` (LPOSC, nominally 32.768 kHz)
//!   and drives `READ_TIME_UPPER`/`READ_TIME_LOWER` in milliseconds.
//!   `SET_TIME_63TO48..15TO0` program its base value and `TIMER.CLEAR` restarts
//!   it. The microsecond-to-RTC conversion uses the live `clk_sys`/`clk_rtc`
//!   ratios, so overclocking the CPU does not change the RTC rate.
//!
//! `0x4010_0000` 处常开电源管理器窗口的最小模型。除以下行为外寄存器均为普通存储：
//!
//! * 高 16 位携带 `0x5AFE` 口令的写入被接受，仅保存 16 位数据字段，与数据手册
//!   的寄存器保护一致。
//! * `VREG_STS.VOUT_OK` 读为 1，因为固件执行时稳压器已完成启动序列。
//! * 常开定时器（`TIMER.RUN`）对 `clk_rtc`（LPOSC，标称 32.768 kHz）计数，并以
//!   毫秒驱动 `READ_TIME_UPPER`/`READ_TIME_LOWER`。`SET_TIME_63TO48..15TO0`
//!   编程其基准值，`TIMER.CLEAR` 重新开始。微秒到 RTC 的换算使用实时
//!   `clk_sys`/`clk_rtc` 比值，因此 CPU 超频不改变 RTC 速率。

use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::clock_tree::ClockTree;
use super::map;

/// `VREG_STS` offset / `VREG_STS` 偏移
const REG_VREG_STS: u32 = 0x08;

/// `VREG_STS.VOUT_OK`: output regulation finished / 输出稳压完成
const VREG_VOUT_OK: u32 = 1 << 4;

/// `SET_TIME_63TO48` offset / `SET_TIME_63TO48` 偏移
const SET_TIME_63TO48: u32 = 0x60;

/// `SET_TIME_15TO0` offset (last word of the little-endian set) / 设置时间最低字偏移
const SET_TIME_15TO0: u32 = 0x6C;

/// `READ_TIME_UPPER` offset / `READ_TIME_UPPER` 偏移
const READ_TIME_UPPER: u32 = 0x70;

/// `READ_TIME_LOWER` offset / `READ_TIME_LOWER` 偏移
const READ_TIME_LOWER: u32 = 0x74;

/// `TIMER` offset / `TIMER` 偏移
const TIMER: u32 = 0x88;

/// `TIMER.RUN` / `TIMER.RUN`
const TIMER_RUN: u32 = 1 << 1;

/// `TIMER.CLEAR` (self-clearing) / `TIMER.CLEAR`（自清零）
const TIMER_CLEAR: u32 = 1 << 2;

/// `TIMER.USING_LPOSC` read-only status / `TIMER.USING_LPOSC` 只读状态
const TIMER_USING_LPOSC: u32 = 1 << 17;

/// Write password in bits 31:16 / 位于第 31:16 位的写入口令
const PASSWORD: u32 = 0x5AFE;

/// Mask of the protected 16-bit data field / 受保护 16 位数据字段的掩码
const DATA_MASK: u32 = 0xFFFF;

/// Shared POWMAN state / 共享的 POWMAN 状态
#[derive(Debug)]
struct PowmanState {
    regs: RegisterFile,
    /// Base time in milliseconds programmed through `SET_TIME_*`
    /// 通过 `SET_TIME_*` 编程的基准时间（毫秒）
    base_ms: u64,
    /// RTC ticks accumulated since the last clear / 自上次清零以来累计的 RTC 节拍
    rtc_ticks: u128,
    /// Fractional numerator for `clk_sys` -> RTC tick conversion
    /// `clk_sys` -> RTC 节拍换算的小数分子
    phase: u128,
    /// `TIMER` storage without the self-clearing bits / 不含自清零位的 `TIMER` 存储
    timer: u32,
}

impl PowmanState {
    /// Create the reset state / 创建复位状态
    fn new() -> Self {
        Self {
            regs: RegisterFile::new(map::DEVICE_WINDOW_SIZE),
            base_ms: 0,
            rtc_ticks: 0,
            phase: 0,
            timer: 0,
        }
    }

    /// Current time in milliseconds / 当前时间（毫秒）
    fn time_ms(&self, rtcclk_hz: u32) -> u64 {
        let elapsed = if rtcclk_hz == 0 {
            0
        } else {
            u64::try_from(self.rtc_ticks * 1_000 / u128::from(rtcclk_hz)).unwrap_or(u64::MAX)
        };
        self.base_ms.saturating_add(elapsed)
    }
}

/// POWMAN register block handle / POWMAN 寄存器块句柄
///
/// Clones share one register/RTC state and one clock tree, so the bus device and
/// [`crate::BootPeripherals`] observe the same time.
/// 克隆共享同一寄存器/RTC 状态与时钟树，因此总线设备与
/// [`crate::BootPeripherals`] 看到同一时间。
#[derive(Debug, Clone)]
pub struct Powman {
    state: Arc<Mutex<PowmanState>>,
    tree: ClockTree,
}

impl Powman {
    /// Create the block with the bootrom clock tree / 用 bootrom 时钟树创建寄存器块
    #[must_use]
    pub fn new() -> Self {
        Self::with_clock(ClockTree::new())
    }

    /// Create the block following `tree` / 创建跟随 `tree` 的寄存器块
    #[must_use]
    pub fn with_clock(tree: ClockTree) -> Self {
        Self {
            state: Arc::new(Mutex::new(PowmanState::new())),
            tree,
        }
    }

    /// RTC time in milliseconds / RTC 时间（毫秒）
    #[must_use]
    pub fn time_ms(&self) -> u64 {
        self.state.lock().time_ms(self.tree.rtcclk_hz())
    }

    /// RTC time in seconds / RTC 时间（秒）
    #[must_use]
    pub fn time_s(&self) -> u64 {
        self.time_ms() / 1_000
    }

    /// Advance the RTC by executed `clk_sys` cycles / 按已执行的 `clk_sys` 周期推进 RTC
    ///
    /// Only the `clk_rtc` domain moves: the CPU cycles are converted with the
    /// live `clk_sys`/`clk_rtc` ratio and the fractional remainder is kept.
    /// 只有 `clk_rtc` 域前进：CPU 周期按实时 `clk_sys`/`clk_rtc` 比值换算，小数
    /// 余数保留。
    pub fn advance(&self, cycles: u64, sysclk_hz: u32, rtcclk_hz: u32) {
        let mut state = self.state.lock();
        if state.timer & TIMER_RUN == 0 || sysclk_hz == 0 || rtcclk_hz == 0 {
            return;
        }
        let total = state.phase + u128::from(cycles) * u128::from(rtcclk_hz);
        let rate = u128::from(sysclk_hz);
        state.rtc_ticks += total / rate;
        state.phase = total % rate;
    }

    /// Program the `SET_TIME_*` base value / 编程 `SET_TIME_*` 基准值
    fn write_set_time(state: &mut PowmanState, offset: u32, value: u32) {
        let shift = 48 - (offset - SET_TIME_63TO48) / 4 * 16;
        let word = u64::from(value & DATA_MASK) << shift;
        let mask = u64::from(DATA_MASK) << shift;
        state.base_ms = (state.base_ms & !mask) | word;
        // 寄存器镜像保留最近写入的数据字段，供回读。
        // The register image keeps the last data field for read-back.
        let _ = state.regs.write(offset, value & DATA_MASK);
        // 写最低字表示一次完整的时间设置：此后 RTC 从该值继续。
        // Writing the lowest word completes the set: the RTC continues from it.
        if offset == SET_TIME_15TO0 {
            state.rtc_ticks = 0;
            state.phase = 0;
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
        let rtcclk = self.tree.rtcclk_hz();
        let state = self.state.lock();
        match offset {
            READ_TIME_UPPER => Some((state.time_ms(rtcclk) >> 32) as u32),
            READ_TIME_LOWER => Some(state.time_ms(rtcclk) as u32),
            TIMER => {
                let running = state.timer & TIMER_RUN != 0;
                let status = if running { TIMER_USING_LPOSC } else { 0 };
                Some(state.timer | status)
            }
            _ => {
                let value = state.regs.read(offset)?;
                if offset == REG_VREG_STS {
                    return Some(value | VREG_VOUT_OK);
                }
                Some(value)
            }
        }
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
        let mut state = self.state.lock();
        match offset {
            SET_TIME_63TO48..=SET_TIME_15TO0 if offset.trailing_zeros() >= 2 => {
                Self::write_set_time(&mut state, offset, stored);
                true
            }
            TIMER => {
                if stored & TIMER_CLEAR != 0 {
                    state.rtc_ticks = 0;
                    state.phase = 0;
                }
                state.timer = stored & !(TIMER_CLEAR | TIMER_USING_LPOSC);
                true
            }
            _ => state.regs.write(offset, stored),
        }
    }

    fn reset(&mut self) {
        *self.state.lock() = PowmanState::new();
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
        assert_eq!(powman.read32(REG_TIMER), Some(0x0002_0002));
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

    #[test]
    fn the_rtc_advances_at_32768_hz_regardless_of_clk_sys() {
        // 1 秒 RTC 时间 = 32768 个 LPOSC 节拍。150 MHz 下需要 150M 周期；
        // 252 MHz 下需要 252M 周期，得到同样的毫秒数。
        // One RTC second is 32768 LPOSC ticks: 150M CPU cycles at 150 MHz and
        // 252M at 252 MHz both yield the same millisecond count.
        let tree = ClockTree::new();
        let mut powman = Powman::with_clock(tree.clone());
        assert!(powman.write32(REG_TIMER, 0x5AFE_0002));
        powman.advance(150_000_000, 150_000_000, 32_768);
        assert_eq!(powman.time_ms(), 1_000);
        powman.advance(252_000_000, 252_000_000, 32_768);
        assert_eq!(powman.time_ms(), 2_000);
        assert_eq!(powman.time_s(), 2);
    }

    #[test]
    fn read_time_registers_report_the_programmed_base() {
        let mut powman = Powman::new();
        // 编程 0x0000_0000_0000_03E8 = 1000 ms。
        // Program 0x3E8 = 1000 ms into the 64-bit set registers.
        assert!(powman.write32(SET_TIME_63TO48, 0x5AFE_0000));
        assert!(powman.write32(0x64, 0x5AFE_0000));
        assert!(powman.write32(0x68, 0x5AFE_0000));
        assert!(powman.write32(SET_TIME_15TO0, 0x5AFE_03E8));
        assert_eq!(powman.read32(READ_TIME_LOWER), Some(1_000));
        assert_eq!(powman.read32(READ_TIME_UPPER), Some(0));
    }

    #[test]
    fn a_stopped_timer_does_not_advance() {
        let powman = Powman::new();
        powman.advance(150_000_000, 150_000_000, 32_768);
        assert_eq!(powman.time_ms(), 0);
    }
}
