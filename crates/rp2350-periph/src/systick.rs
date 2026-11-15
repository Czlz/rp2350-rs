//! ARMv8-M SysTick counter with selectable clock source
//! 带可选时钟源的 ARMv8-M SysTick 计数器
//!
//! The counter lives in the PPB at `0xE000_E010` and is modelled here rather
//! than in the core so that it can follow the live clock tree:
//!
//! - `CSR.CLKSOURCE = 1` counts `clk_sys` cycles;
//! - `CSR.CLKSOURCE = 0` counts `clk_ref / 2` cycles (the ARMv8-M reference
//!   clock input on RP2350 is `clk_ref`).
//!
//! The reload value is expressed in **source** cycles, so the wall-clock period
//! is `(RVR + 1) / source_hz` and follows runtime overclocking automatically.
//! 计数器位于 PPB 的 `0xE000_E010`，放在这里而不是核内，才能跟随实时时钟树：
//!
//! - `CSR.CLKSOURCE = 1` 计 `clk_sys` 周期；
//! - `CSR.CLKSOURCE = 0` 计 `clk_ref / 2` 周期（RP2350 上 ARMv8-M 的参考时钟输入
//!   为 `clk_ref`）。
//!
//! 重载值以**源**周期表示，因此实际周期为 `(RVR + 1) / source_hz`，运行期超频
//! 会自动生效。

/// `SYST_CSR.ENABLE` / `SYST_CSR.ENABLE`
pub const CSR_ENABLE: u32 = 1 << 0;

/// `SYST_CSR.TICKINT` / `SYST_CSR.TICKINT`
pub const CSR_TICKINT: u32 = 1 << 1;

/// `SYST_CSR.CLKSOURCE` (1 = processor clock) / `SYST_CSR.CLKSOURCE`（1 = 处理器时钟）
pub const CSR_CLKSOURCE: u32 = 1 << 2;

/// `SYST_CSR.COUNTFLAG` (read-only, writing one clears it)
/// `SYST_CSR.COUNTFLAG`（只读，写 1 清零）
pub const CSR_COUNTFLAG: u32 = 1 << 16;

/// Writable `SYST_CSR` bits / `SYST_CSR` 可写位
const CSR_WRITE_MASK: u32 = CSR_ENABLE | CSR_TICKINT | CSR_CLKSOURCE;

/// 24-bit counter mask / 24 位计数器掩码
const COUNTER_MASK: u32 = 0x00FF_FFFF;

/// `SYST_CALIB` reported to firmware (NOREF=1, SKEW=1, TENMS=250000)
/// 报告给固件的 `SYST_CALIB`（NOREF=1、SKEW=1、TENMS=250000）
pub const CALIB: u32 = 0xC000_03E8;

/// Shared SysTick counter state / 共享的 SysTick 计数器状态
///
/// Three status bools mirror the ARMv8-M COUNTFLAG, reload phase and pending
/// latch; they are independent hardware bits, not a mode.
/// 三个状态布尔量对应 ARMv8-M 的 COUNTFLAG、重载相位与挂起锁存；它们是相互独立的
/// 硬件位，不是模式。
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct SysTickCounter {
    /// `SYST_CSR` without `COUNTFLAG` / 不含 `COUNTFLAG` 的 `SYST_CSR`
    csr: u32,
    /// `SYST_RVR` / `SYST_RVR`
    rvr: u32,
    /// `SYST_CVR` / `SYST_CVR`
    cvr: u32,
    /// `COUNTFLAG` latch / `COUNTFLAG` 锁存
    countflag: bool,
    /// Reload on the next source tick (counter just reached zero)
    /// 下一个源周期重载（计数器刚到达零）
    needs_reload: bool,
    /// One-shot interrupt pending latch / 一次性中断挂起锁存
    pending: bool,
    /// Source-clock phase accumulator numerator / 源时钟相位累加器分子
    phase: u128,
    /// Last observed source frequency / 上次观察到的源频率
    source_hz: u32,
    /// Zero-source warning latch / 零源频率告警锁存
    zero_warned: bool,
}

impl SysTickCounter {
    /// Create a counter in the reset state / 创建复位状态的计数器
    #[must_use]
    pub fn new() -> Self {
        Self {
            csr: 0,
            rvr: 0,
            cvr: 0,
            countflag: false,
            needs_reload: false,
            pending: false,
            phase: 0,
            source_hz: 0,
            zero_warned: false,
        }
    }

    /// Reset the counter / 复位计数器
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// `SYST_CSR` including the live `COUNTFLAG` / 含实时 `COUNTFLAG` 的 `SYST_CSR`
    #[must_use]
    pub fn csr(&self) -> u32 {
        self.csr | if self.countflag { CSR_COUNTFLAG } else { 0 }
    }

    /// Write `SYST_CSR` / 写 `SYST_CSR`
    pub fn write_csr(&mut self, value: u32) {
        self.csr = value & CSR_WRITE_MASK;
        if value & CSR_COUNTFLAG != 0 {
            self.countflag = false;
        }
        if self.csr & CSR_TICKINT == 0 {
            self.pending = false;
        }
    }

    /// `SYST_RVR` / `SYST_RVR`
    #[must_use]
    pub const fn rvr(&self) -> u32 {
        self.rvr
    }

    /// Write `SYST_RVR` (24-bit) / 写 `SYST_RVR`（24 位）
    pub fn write_rvr(&mut self, value: u32) {
        self.rvr = value & COUNTER_MASK;
    }

    /// Current counter value / 当前计数器值
    #[must_use]
    pub const fn cvr(&self) -> u32 {
        self.cvr
    }

    /// Write `SYST_CVR`, clearing `COUNTFLAG` / 写 `SYST_CVR` 并清除 `COUNTFLAG`
    pub fn write_cvr(&mut self, value: u32) {
        self.cvr = value & COUNTER_MASK;
        self.countflag = false;
        self.needs_reload = false;
    }

    /// Whether an expiry is waiting to be delivered / 是否有到期事件等待投递
    #[must_use]
    pub const fn interrupt_pending(&self) -> bool {
        self.pending
    }

    /// Take the one-shot expiry latch / 取走一次性到期锁存
    ///
    /// Returns `true` exactly once per expiry, so the host signals the NVIC for
    /// each deadline without re-pending an already delivered exception.
    /// 每次到期只返回一次 `true`，宿主据此为每个截止时刻向 NVIC 投递一次，而不会
    /// 重复挂起已投递的异常。
    pub fn take_pending(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }

    /// Source frequency selected by `CSR.CLKSOURCE` / `CSR.CLKSOURCE` 选择的源频率
    #[must_use]
    pub fn source_hz(&self, sysclk_hz: u32, refclk_hz: u32) -> u32 {
        if self.csr & CSR_CLKSOURCE != 0 {
            sysclk_hz
        } else {
            refclk_hz / 2
        }
    }

    /// Advance the counter by executed `clk_sys` cycles
    /// 按已执行的 `clk_sys` 周期推进计数器
    ///
    /// Returns `true` when at least one expiry happened in this call.
    /// 本次调用至少发生一次到期时返回 `true`。
    pub fn advance(&mut self, cycles: u64, sysclk_hz: u32, refclk_hz: u32) -> bool {
        if self.csr & CSR_ENABLE == 0 {
            return false;
        }
        let source = self.source_hz(sysclk_hz, refclk_hz);
        if source != self.source_hz || source == 0 {
            self.source_hz = source;
        }
        if source == 0 || sysclk_hz == 0 {
            if !self.zero_warned {
                self.zero_warned = true;
                log::warn!(
                    "SysTick: source clock is 0 Hz; the counter cannot advance / \
                     SysTick：源时钟为 0 Hz，计数器无法推进"
                );
            }
            return false;
        }
        // 把 CPU 周期换算为源周期，余数保留到下次；u128 覆盖长窗口。
        // Convert CPU cycles to source cycles keeping the remainder for the next
        // call; u128 covers long windows.
        let total = self.phase + u128::from(cycles) * u128::from(source);
        let rate = u128::from(sysclk_hz);
        let ticks = u64::try_from(total / rate).unwrap_or(u64::MAX);
        self.phase = total % rate;
        self.tick(ticks)
    }

    /// Advance the counter by `ticks` source-clock cycles / 按 `ticks` 个源时钟周期推进
    ///
    /// Also used directly by tests and by hosts that already know the source
    /// clock. `/ 也供测试与已知源时钟的宿主直接调用。`
    pub fn tick(&mut self, ticks: u64) -> bool {
        if self.csr & CSR_ENABLE == 0 {
            return false;
        }
        let mut fired = false;
        let mut remaining = ticks;
        let mut value = self.cvr;
        // 每个源周期递减 1；到达 0 时锁存 COUNTFLAG，下一个源周期重载（ARMv8-M）。
        // 事件批量合并，避免长时间停滞后逐周期循环。
        // Each source tick decrements by one; reaching zero latches COUNTFLAG and
        // the next source tick reloads (ARMv8-M). Events are batched so a long
        // stall does not loop per cycle.
        while remaining > 0 {
            if self.needs_reload {
                value = self.rvr;
                self.needs_reload = false;
                remaining -= 1;
                continue;
            }
            if value == 0 {
                // 写入 0 的计数器在下一个源周期重载并置标志。
                // A counter written to zero reloads on the next source tick and flags.
                value = self.rvr;
                self.countflag = true;
                fired = true;
                if self.csr & CSR_TICKINT != 0 {
                    self.pending = true;
                }
                remaining -= 1;
                continue;
            }
            let step = remaining.min(u64::from(value));
            value = u32::try_from(u64::from(value) - step).unwrap_or(0);
            remaining -= step;
            if value == 0 {
                self.countflag = true;
                fired = true;
                if self.csr & CSR_TICKINT != 0 {
                    self.pending = true;
                }
                self.needs_reload = true;
            }
        }
        self.cvr = value;
        fired
    }
}

impl Default for SysTickCounter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processor_clock_period_tracks_12_150_and_252_mhz() {
        // CLKSOURCE=1：RVR = hz/1000 - 1 时每 1 ms 到期一次。
        // CLKSOURCE=1: RVR = hz/1000 - 1 expires once per 1 ms.
        for hz in [12_000_000u32, 150_000_000, 252_000_000] {
            let mut counter = SysTickCounter::new();
            counter.write_csr(CSR_ENABLE | CSR_TICKINT | CSR_CLKSOURCE);
            counter.write_rvr(hz / 1_000 - 1);
            counter.write_cvr(hz / 1_000);
            assert!(!counter.advance(u64::from(hz / 1_000 - 1), hz, 12_000_000));
            assert!(counter.advance(1, hz, 12_000_000), "{hz} Hz expiry");
            assert!(counter.csr() & CSR_COUNTFLAG != 0);
            assert!(counter.take_pending());
            assert!(!counter.take_pending(), "pending is one-shot");
        }
    }

    #[test]
    fn reference_clock_is_half_of_clk_ref() {
        // CLKSOURCE=0：源为 clk_ref/2 = 6 MHz，1 ms 需要 6000 个源周期。
        // 每 6 个 1 MHz 的 clk_ref 周期产生 1 个源周期。
        // CLKSOURCE=0: source is clk_ref/2 = 6 MHz; 1 ms needs 6000 source
        // cycles. One source cycle per six 12 MHz clk_ref cycles.
        let mut counter = SysTickCounter::new();
        counter.write_csr(CSR_ENABLE | CSR_TICKINT);
        counter.write_rvr(5_999);
        counter.write_cvr(6_000);
        assert_eq!(counter.source_hz(252_000_000, 12_000_000), 6_000_000);
        // 252 MHz 下 1 ms = 252 000 个 CPU 周期，应恰好到期一次。
        // 1 ms is 252_000 CPU cycles at 252 MHz and must expire exactly once.
        assert!(!counter.advance(251_999, 252_000_000, 12_000_000));
        assert!(counter.advance(1, 252_000_000, 12_000_000));
        assert_eq!(counter.cvr(), 0);
        // 再多 251 999 周期不到期，最后 1 周期才到期。
        // Another 251_999 cycles do not expire; the final cycle does.
        assert!(!counter.advance(251_999, 252_000_000, 12_000_000));
        assert!(counter.advance(1, 252_000_000, 12_000_000));
    }

    #[test]
    fn the_reference_period_is_independent_of_the_system_clock() {
        // 同一 6 MHz 源在 150 MHz 与 252 MHz 下都是每 6000 个源周期到期。
        // The 6 MHz source expires every 6000 source cycles at both 150 and
        // 252 MHz CPU rates.
        for sysclk in [150_000_000u32, 252_000_000] {
            let mut counter = SysTickCounter::new();
            counter.write_csr(CSR_ENABLE);
            counter.write_rvr(5_999);
            counter.write_cvr(6_000);
            // 6 ms 的源周期 = 6 * 6 个 clk_ref 周期；按 CPU 频率换算成 CPU 周期。
            // Six source ticks correspond to 36 clk_ref cycles; convert to CPU.
            let ticks = u64::from(sysclk) * 6 / 1_000;
            assert!(counter.advance(ticks, sysclk, 12_000_000), "{sysclk}");
        }
    }

    #[test]
    fn countflag_clears_on_csr_or_cvr_write() {
        let mut counter = SysTickCounter::new();
        counter.write_csr(CSR_ENABLE | CSR_CLKSOURCE);
        counter.write_rvr(1);
        counter.write_cvr(1);
        counter.advance(2, 150_000_000, 12_000_000);
        assert!(counter.csr() & CSR_COUNTFLAG != 0);
        counter.write_csr(CSR_ENABLE | CSR_COUNTFLAG | CSR_CLKSOURCE);
        assert_eq!(counter.csr() & CSR_COUNTFLAG, 0);
        counter.advance(2, 150_000_000, 12_000_000);
        assert!(counter.csr() & CSR_COUNTFLAG != 0);
        counter.write_cvr(1);
        assert_eq!(counter.csr() & CSR_COUNTFLAG, 0);
    }

    #[test]
    fn a_disabled_counter_does_not_move() {
        let mut counter = SysTickCounter::new();
        counter.write_rvr(10);
        counter.write_cvr(10);
        assert!(!counter.advance(1_000, 150_000_000, 12_000_000));
        assert_eq!(counter.cvr(), 10);
    }
}
