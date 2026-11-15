//! 1 ms start-of-frame scheduling for the host bridge
//! 主机桥的 1 ms 帧起始（SOF）调度
//!
//! A real full-speed host issues one SOF token per 1 ms frame. Firmware relies
//! on it: TinyUSB's errata-15 workaround caches `SOF_RD` in `e15_last_sof` and
//! only starts the next bulk-IN buffer when a *new* frame number shows up.
//! [`SofScheduler`] converts the simulated cycle count into frame boundaries so
//! the bridge never needs a wall clock.
//! 真实全速主机每 1 ms 帧发送一个 SOF 令牌。固件依赖它：TinyUSB 的 errata-15
//! 变通把 `SOF_RD` 缓存到 `e15_last_sof`，只有出现**新**帧号才启动下一个批量 IN
//! 缓冲。[`SofScheduler`] 把模拟周期数换算为帧边界，因此主机桥不需要真实时钟。

use rp2350_usb::controller::Controller;

/// Nominal USB full-speed frame interval in microseconds
/// USB 全速帧间隔标称值（微秒）
pub const FRAME_INTERVAL_MICROS: u64 = 1_000;

/// Nominal USB full-speed clock (PLL_USB) in Hz / USB 全速时钟（PLL_USB）标称频率
pub const USB_FULL_SPEED_HZ: u32 = 48_000_000;

/// Mask of the 11-bit SOF frame number / 11 位 SOF 帧号掩码
pub const SOF_FRAME_MASK: u16 = 0x07FF;

/// Upper bound on SOF tokens emitted by a single [`SofScheduler::advance`] call
/// 单次 [`SofScheduler::advance`] 最多下发的 SOF 令牌数
///
/// After a long simulated stall hundreds of frames may be due at once. Emitting
/// a token per frame would cost time without changing device-visible behaviour,
/// so the scheduler coalesces the backlog: the frame counter still tracks
/// simulated time, only the token burst is bounded.
/// 长时间停滞后可能一次到期数百帧。逐帧下发只会耗费时间而不改变设备可见行为，
/// 因此调度器合并积压：帧号仍跟随模拟时间，只限制令牌突发量。
pub const MAX_SOF_CATCH_UP: u32 = 64;

/// Schedules one SOF token per millisecond of simulated time
/// 按模拟时间每毫秒调度一个 SOF 令牌
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SofScheduler {
    period_cycles: u64,
    next_cycle: u64,
    frame: u16,
    emitted: u64,
    /// `CLK_SYS` rate used to convert `poll(now)` cycles / 用于换算 `poll(now)` 周期的 `CLK_SYS`
    sysclk_hz: u32,
    /// `CLK_USB` rate that generates the 1 ms frame / 产生 1 ms 帧的 `CLK_USB`
    usbclk_hz: u32,
}

impl SofScheduler {
    /// Create a scheduler ticking at `sysclk_hz` with no frame delivered yet
    /// 以 `sysclk_hz` 为基准创建调度器，尚未下发任何帧
    ///
    /// The USB clock defaults to full speed (48 MHz). Because one millisecond is
    /// 1000 `CLK_USB` cycles and the host passes `CLK_SYS` cycles, the frame
    /// period is `usb_period * sysclk / usbclk = sysclk / 1000`; the USB clock is
    /// still carried so diagnostics can prove the SOF generator is in the USB
    /// domain.
    /// USB 时钟默认全速（48 MHz）。一毫秒是 1000 个 `CLK_USB` 周期，而宿主传入
    /// `CLK_SYS` 周期，因此帧周期为 `usb_period * sysclk / usbclk = sysclk / 1000`；
    /// 同时保留 USB 时钟值，使诊断能证明 SOF 发生器位于 USB 时钟域。
    #[must_use]
    pub const fn new(sysclk_hz: u32) -> Self {
        Self::new_with_usb(sysclk_hz, USB_FULL_SPEED_HZ)
    }

    /// Create a scheduler with explicit `CLK_SYS` and `CLK_USB` rates
    /// 用显式的 `CLK_SYS` 与 `CLK_USB` 频率创建调度器
    #[must_use]
    pub const fn new_with_usb(sysclk_hz: u32, usbclk_hz: u32) -> Self {
        // 第一个帧边界在起点之后一个周期，因此 new() 等价于 reset(0)。
        // The first frame boundary sits one period after the start, so `new()`
        // is equivalent to `reset(0)`.
        let period = Self::cycles_for(sysclk_hz);
        Self {
            period_cycles: period,
            next_cycle: period,
            frame: 0,
            emitted: 0,
            sysclk_hz,
            usbclk_hz,
        }
    }

    /// Cycles in one 1 ms frame at `sysclk_hz` (never zero)
    /// `sysclk_hz` 下 1 ms 帧对应的周期数（永不为零）
    #[must_use]
    pub const fn cycles_for(sysclk_hz: u32) -> u64 {
        let cycles = sysclk_hz as u64 / FRAME_INTERVAL_MICROS;
        if cycles == 0 { 1 } else { cycles }
    }

    /// One frame in the USB clock domain (never zero) / USB 时钟域的一帧（永不为零）
    ///
    /// This is `CLK_USB / 1 MHz`, nominally 48000 cycles. `/ 即 `CLK_USB / 1 MHz`，
    /// 标称 48000 个周期。`
    #[must_use]
    pub const fn usb_period_cycles(&self) -> u64 {
        let cycles = self.usbclk_hz as u64 / FRAME_INTERVAL_MICROS;
        if cycles == 0 { 1 } else { cycles }
    }

    /// Override the system clock used for the frame period
    /// 覆盖用于帧周期换算的系统时钟
    pub const fn set_sysclk_hz(&mut self, sysclk_hz: u32) {
        self.sysclk_hz = sysclk_hz;
        self.period_cycles = Self::cycles_for(sysclk_hz);
    }

    /// Override both clocks / 覆盖两个时钟
    pub const fn set_clocks(&mut self, sysclk_hz: u32, usbclk_hz: u32) {
        self.usbclk_hz = usbclk_hz;
        self.set_sysclk_hz(sysclk_hz);
    }

    /// `CLK_SYS` rate used for the CPU-cycle argument / 用于 CPU 周期参数的 `CLK_SYS`
    #[must_use]
    pub const fn sysclk_hz(&self) -> u32 {
        self.sysclk_hz
    }

    /// `CLK_USB` rate that generates the frame / 产生帧的 `CLK_USB` 频率
    #[must_use]
    pub const fn usbclk_hz(&self) -> u32 {
        self.usbclk_hz
    }

    /// Frame period in simulated cycles / 模拟周期表示的帧周期
    #[must_use]
    pub const fn period_cycles(&self) -> u64 {
        self.period_cycles
    }

    /// Frame number delivered last (0 before the first token)
    /// 最近下发的帧号（首个令牌之前为 0）
    #[must_use]
    pub const fn frame(&self) -> u16 {
        self.frame
    }

    /// Cycle at which the next frame is due / 下一帧到期的周期
    #[must_use]
    pub const fn next_cycle(&self) -> u64 {
        self.next_cycle
    }

    /// Total SOF tokens handed to the controller / 已交给控制器的 SOF 令牌总数
    #[must_use]
    pub const fn emitted(&self) -> u64 {
        self.emitted
    }

    /// Restart the schedule so the next frame is due one period after `now`
    /// 重新开始调度：下一帧在 `now` 之后一个帧周期到期
    pub const fn reset(&mut self, now: u64) {
        self.reset_at(now, 0);
    }

    /// Restart the schedule at `now`, continuing from `frame`
    /// 在 `now` 从 `frame` 继续重新开始调度
    ///
    /// Used when the host bridge hands the device over to the CDC data phase:
    /// the frame counter keeps following the device's `SOF_RD` instead of
    /// jumping back to zero.
    /// 主机桥把设备交给 CDC 数据阶段时使用：帧号延续设备 `SOF_RD`，而不是
    /// 回退到零。
    pub const fn reset_at(&mut self, now: u64, frame: u16) {
        self.frame = frame & SOF_FRAME_MASK;
        self.next_cycle = now.saturating_add(self.period_cycles);
        self.emitted = 0;
    }

    /// Deliver every frame due at `now`; returns the number of SOF tokens issued
    /// 在 `now` 下发所有到期帧，返回实际下发的 SOF 令牌数
    ///
    /// Frames are issued *before* the caller handles any transfer, which is the
    /// order a real host produces on the bus (SOF first, then the frame's
    /// transactions).
    /// 帧在调用方处理任何传输**之前**下发，与真实主机在总线上的顺序一致
    /// （先 SOF，再本帧事务）。
    pub fn advance(&mut self, controller: &Controller, now: u64) -> u32 {
        if now < self.next_cycle {
            return 0;
        }
        let due = (now - self.next_cycle) / self.period_cycles + 1;
        let emit = due.min(u64::from(MAX_SOF_CATCH_UP));
        for _ in 0..emit {
            self.frame = (self.frame + 1) & SOF_FRAME_MASK;
            controller.host_sof(self.frame);
            self.emitted += 1;
        }
        if due > emit {
            // 合并的帧只推进帧号：帧号必须继续跟随模拟时间，errata-15 依赖帧号
            // 至少发生变化，而不依赖每个 SOF 令牌。
            // Coalesced frames only advance the counter: the frame number must
            // keep tracking simulated time; errata-15 only needs it to change.
            let skipped = due - emit;
            let wrapped = (u64::from(self.frame) + skipped) & u64::from(SOF_FRAME_MASK);
            self.frame = wrapped as u16;
        }
        self.next_cycle = self
            .next_cycle
            .saturating_add(due.saturating_mul(self.period_cycles));
        emit as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp2350_usb::controller::Controller;
    use rp2350_usb::regs::{self, Alias, MAIN_CTRL, MAIN_CTRL_CONTROLLER_EN, SIE_CTRL_PULLUP_EN};

    const SYSCLK: u32 = 150_000_000;
    const MS: u64 = 150_000;

    /// Controller with the device visible to the host / 设备对主机可见的控制器
    fn visible_controller() -> Controller {
        let controller = Controller::new();
        controller.with_core(|core| {
            core.write_reg(MAIN_CTRL, Alias::Rw, MAIN_CTRL_CONTROLLER_EN);
            core.write_reg(regs::SIE_CTRL, Alias::Rw, SIE_CTRL_PULLUP_EN);
            core.host_attach();
        });
        controller
    }

    fn sof_rd(controller: &Controller) -> u16 {
        controller.with_core(|core| core.read_reg(regs::SOF_RD) as u16)
    }

    #[test]
    fn one_millisecond_issues_exactly_one_sof() {
        let controller = visible_controller();
        let mut sof = SofScheduler::new(SYSCLK);
        sof.reset(0);
        // 0.5 ms: no frame boundary yet. / 0.5 毫秒：尚未到帧边界。
        assert_eq!(sof.advance(&controller, MS / 2), 0);
        assert_eq!(sof_rd(&controller), 0);
        // 1 ms: exactly one frame. / 1 毫秒：恰好一帧。
        assert_eq!(sof.advance(&controller, MS), 1);
        assert_eq!(sof_rd(&controller), 1);
        assert_eq!(sof.frame(), 1);
        // 1.5 ms: still the same frame. / 1.5 毫秒：仍是同一帧。
        assert_eq!(sof.advance(&controller, MS + MS / 2), 0);
        assert_eq!(sof_rd(&controller), 1);
        // 2 ms: next frame. / 2 毫秒：下一帧。
        assert_eq!(sof.advance(&controller, 2 * MS), 1);
        assert_eq!(sof_rd(&controller), 2);
        assert_eq!(sof.emitted(), 2);
    }

    #[test]
    fn the_frame_comes_from_the_usb_clock_domain() {
        // 252 MHz CLK_SYS + 48 MHz CLK_USB：USB 域一帧是 48 000 个周期，
        // 换算到 CPU 周期是 252 000，仍然是 1 ms。
        // 252 MHz CLK_SYS with 48 MHz CLK_USB: one USB-domain frame is 48_000
        // cycles, which converts to 252_000 CPU cycles — still 1 ms.
        let controller = visible_controller();
        let mut sof = SofScheduler::new_with_usb(252_000_000, 48_000_000);
        assert_eq!(sof.usb_period_cycles(), 48_000);
        assert_eq!(sof.period_cycles(), 252_000);
        assert_eq!(sof.usbclk_hz(), 48_000_000);
        sof.reset(0);
        assert_eq!(sof.advance(&controller, 251_999), 0);
        assert_eq!(sof.advance(&controller, 252_000), 1);
        // 系统时钟改频只改变 CPU 周期数，USB 域仍是 48 000 周期。
        // Retuning the CPU clock only changes the CPU count; the USB domain
        // still runs 48_000 cycles per frame.
        sof.set_clocks(150_000_000, 48_000_000);
        assert_eq!(sof.usb_period_cycles(), 48_000);
        assert_eq!(sof.period_cycles(), 150_000);
    }

    #[test]
    fn frame_period_follows_the_system_clock() {
        let controller = visible_controller();
        let mut sof = SofScheduler::new(120_000_000);
        assert_eq!(sof.period_cycles(), 120_000);
        sof.reset(0);
        // 120 MHz: 1 ms is 120_000 cycles, not 150_000.
        assert_eq!(sof.advance(&controller, 120_000), 1);
        assert_eq!(sof.advance(&controller, 150_000), 0);
        // Re-clocking retunes the period of the next frame boundary.
        sof.set_sysclk_hz(SYSCLK);
        assert_eq!(sof.period_cycles(), MS);
    }

    #[test]
    fn a_long_gap_catches_up_without_losing_phase() {
        let controller = visible_controller();
        let mut sof = SofScheduler::new(SYSCLK);
        sof.reset(0);
        // 5 ms jump: 5 frames are due at once. / 跳 5 毫秒：一次到期 5 帧。
        assert_eq!(sof.advance(&controller, 5 * MS), 5);
        assert_eq!(sof.frame(), 5);
        assert_eq!(sof.next_cycle(), 6 * MS);
        // The next boundary is still aligned to the 1 ms grid.
        assert_eq!(sof.advance(&controller, 6 * MS - 1), 0);
        assert_eq!(sof.advance(&controller, 6 * MS), 1);
        assert_eq!(sof_rd(&controller), 6);
    }

    #[test]
    fn frame_number_wraps_at_eleven_bits() {
        let controller = visible_controller();
        let mut sof = SofScheduler::new(SYSCLK);
        sof.reset_at(0, 0x07FE);
        assert_eq!(sof.advance(&controller, MS), 1);
        assert_eq!(sof_rd(&controller), 0x07FF);
        assert_eq!(sof.advance(&controller, 2 * MS), 1);
        // SOF_RD keeps only the 11-bit frame number. / SOF_RD 只保留 11 位帧号。
        assert_eq!(sof_rd(&controller), 0);
    }

    #[test]
    fn backlog_tokens_are_bounded_but_the_frame_counter_tracks_time() {
        let controller = visible_controller();
        let mut sof = SofScheduler::new(SYSCLK);
        sof.reset(0);
        // 10 simulated seconds = 10_000 frames due in one call.
        let issued = sof.advance(&controller, 10_000 * MS);
        assert_eq!(issued, MAX_SOF_CATCH_UP);
        assert_eq!(sof.emitted(), u64::from(MAX_SOF_CATCH_UP));
        // 10_000 frames wrap to 10_000 mod 2048 = 1808 = 0x0710.
        assert_eq!(sof.frame(), 0x0710);
        assert_eq!(sof.next_cycle(), 10_001 * MS);
    }

    #[test]
    fn reset_realigns_and_restarts_the_frame_number() {
        let controller = visible_controller();
        let mut sof = SofScheduler::new(SYSCLK);
        sof.advance(&controller, 3 * MS);
        assert_eq!(sof.frame(), 3);
        sof.reset(10 * MS);
        assert_eq!(sof.frame(), 0);
        assert_eq!(sof.emitted(), 0);
        assert_eq!(sof.next_cycle(), 11 * MS);
        assert_eq!(sof.advance(&controller, 10 * MS), 0);
        assert_eq!(sof.advance(&controller, 11 * MS), 1);
        assert_eq!(sof_rd(&controller), 1);
    }
}
