//! System-exception arbitration and delivery (NMI / SVCall / PendSV / SysTick)
//! 系统异常仲裁与投递（NMI / SVCall / PendSV / SysTick）
//!
//! ARMv8-M exception numbers are not a flat NVIC index: `NMI` is 2, `HardFault`
//! is 3, `SVCall` is 11, `PendSV` is 14 and `SysTick` is 15, and only external
//! interrupts use `16 + n`. This module implements the arbitration between
//! those system exceptions and the external interrupt lines: fixed-priority
//! NMI/HardFault first, then numeric-priority ordering with the lowest
//! exception number as tie-break, the `PRIMASK`/`FAULTMASK`/`BASEPRI` gates and
//! the "only a strictly higher priority may preempt" rule. It also owns the
//! `SCB_ICSR` read/write view (`NMIPENDSET`/`PENDSTSET`/`PENDSTCLR`/
//! `PENDSVSET`/`PENDSVCLR`/`VECTPENDING`).
//! ARMv8-M 的异常号并不是扁平的 NVIC 索引：`NMI` 为 2、`HardFault` 为 3、
//! `SVCall` 为 11、`PendSV` 为 14、`SysTick` 为 15，只有外部中断才用 `16 + n`。
//! 本模块实现这些系统异常与外部中断线之间的仲裁：固定优先级的 NMI/HardFault
//! 优先，其后按数值优先级排序、同优先级取最小异常号，并应用
//! `PRIMASK`/`FAULTMASK`/`BASEPRI` 门控与「只有严格更高优先级才能抢占」的规则；
//! 同时负责 `SCB_ICSR` 的读写视图（`NMIPENDSET`/`PENDSTSET`/`PENDSTCLR`/
//! `PENDSVSET`/`PENDSVCLR`/`VECTPENDING`）。

// 仲裁代码直接操作异常号与优先级掩码，属于异常控制器仿真的固有写法。
// Arbitration manipulates exception numbers and priority masks directly, which
// is inherent to exception-controller emulation.
#![allow(clippy::doc_markdown, clippy::module_name_repetitions)]

use std::fmt;

use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;

use crate::exceptions::{EXC_HARDFAULT, EXC_NMI, EXC_PENDSV, EXC_SVCALL, EXC_SYSTICK};
use crate::faults::{RANK_CONFIGURABLE_BASE, RANK_HARDFAULT, RANK_NMI};
use crate::scb::icsr;
use crate::{CortexM33Core, FIRST_EXTERNAL_EXCEPTION, scr};

/// Snapshot of the modelled PendSV / SysTick exception state
/// 已建模的 PendSV / SysTick 异常状态快照
///
/// `summary` prints a stable, documented string so diagnostics can be compared
/// between runs: `systick=<state> pendsv=<state>` where `<state>` is one of
/// `idle`, `pending`, `active` or `active+pending`.
/// `summary` 输出稳定且成文的字符串，便于跨运行比较诊断信息：
/// `systick=<state> pendsv=<state>`，其中 `<state>` 为 `idle`、`pending`、
/// `active` 或 `active+pending` 之一。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemExceptionState {
    /// SysTick pending latch / SysTick 挂起锁存
    pub systick_pending: bool,
    /// SysTick handler is on the active exception stack
    /// SysTick 处理程序位于活跃异常栈
    pub systick_active: bool,
    /// PendSV pending latch / PendSV 挂起锁存
    pub pendsv_pending: bool,
    /// PendSV handler is on the active exception stack
    /// PendSV 处理程序位于活跃异常栈
    pub pendsv_active: bool,
    /// SysTick priority decoded from `SHPR3[31:24]`
    /// 由 `SHPR3[31:24]` 解码的 SysTick 优先级
    pub systick_priority: u8,
    /// PendSV priority decoded from `SHPR3[23:16]`
    /// 由 `SHPR3[23:16]` 解码的 PendSV 优先级
    pub pendsv_priority: u8,
}

impl SystemExceptionState {
    /// Render one exception state / 渲染单个异常状态
    fn state_name(pending: bool, active: bool) -> &'static str {
        match (pending, active) {
            (true, true) => "active+pending",
            (true, false) => "pending",
            (false, true) => "active",
            (false, false) => "idle",
        }
    }

    /// Stable diagnostic string: `systick=<state> pendsv=<state>`
    /// 稳定诊断字符串：`systick=<state> pendsv=<state>`
    ///
    /// Takes `self` by value because the snapshot is a six-byte `Copy` value.
    /// 以值传递 `self`，因为该快照是 6 字节的 `Copy` 值。
    #[must_use]
    pub fn summary(self) -> String {
        format!(
            "systick={} pendsv={}",
            Self::state_name(self.systick_pending, self.systick_active),
            Self::state_name(self.pendsv_pending, self.pendsv_active)
        )
    }
}

impl fmt::Display for SystemExceptionState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.summary())
    }
}

impl CortexM33Core {
    /// Highest-priority pending exception as `(exception number, rank)`
    /// 最高优先级的挂起异常，形式为 `(异常号, 优先级等级)`
    ///
    /// NMI is checked first because it has the highest fixed priority in the
    /// architecture (ARMv8-M B1.5.4: Reset > NMI > HardFault). External
    /// interrupts come from [`crate::Nvic::find_highest_priority`] (enabled,
    /// pending and not active); PendSV, SysTick and SVCall come from
    /// [`crate::scb::Scb`]. Ties compare the exception number, which is exactly
    /// the ARMv8-M rule: a numerically smaller exception number wins, so
    /// SVCall (11) beats PendSV (14), which beats SysTick (15), and all three
    /// beat every external interrupt.
    /// 首先检查 NMI，因为它在架构中具有最高固定优先级（ARMv8-M B1.5.4：
    /// Reset > NMI > HardFault）。外部中断来自 [`crate::Nvic::find_highest_priority`]
    /// （已使能、挂起且非活跃）；PendSV、SysTick 与 SVCall 来自
    /// [`crate::scb::Scb`]。并列时比较异常号，这正是 ARMv8-M 的规则：异常号更小者胜，
    /// 因此 SVCall（11）优先于 PendSV（14），后者优先于 SysTick（15），三者都优先于
    /// 任何外部中断。
    #[must_use]
    pub(crate) fn pending_exception(&self) -> Option<(u16, u8)> {
        // NMI 不可屏蔽且高于所有可配置异常，直接短路。
        // NMI is non-maskable and above every configurable exception.
        if self.scb.nmi_pending() {
            return Some((EXC_NMI, RANK_NMI));
        }
        let mut best = self.nvic.find_highest_priority().map(|irq| {
            let number = FIRST_EXTERNAL_EXCEPTION + irq;
            (number, self.exception_priority(number))
        });
        for number in [EXC_SVCALL, EXC_PENDSV, EXC_SYSTICK] {
            if !self.scb.is_pending(number) {
                continue;
            }
            let candidate = (number, self.exception_priority(number));
            match best {
                Some((best_number, best_priority))
                    if (best_priority, best_number) <= (candidate.1, candidate.0) => {}
                _ => best = Some(candidate),
            }
        }
        best
    }

    /// Priority rank of an exception number, active or pending
    /// 异常号的优先级等级（活跃或挂起）
    ///
    /// NMI and HardFault use the architecture's fixed ranks (0 and 1);
    /// configurable exceptions map to `2 + priority`, so a smaller rank always
    /// preempts a larger one. PendSV, SysTick and SVCall decode their
    /// `SHPR2`/`SHPR3` bytes, external interrupts their NVIC `IPR` byte, and
    /// the fault exceptions report configurable priority 0 until `SHPR1` is
    /// modelled.
    /// NMI 与 HardFault 使用架构固定等级（0 与 1）；可配置异常映射为
    /// `2 + 优先级`，因此等级更小者总能抢占。PendSV、SysTick 与 SVCall 解码各自的
    /// `SHPR2`/`SHPR3` 字节，外部中断解码 NVIC `IPR` 字节；`SHPR1` 尚未建模前，
    /// 故障异常报告可配置优先级 0。
    #[must_use]
    pub(crate) fn exception_priority(&self, exception_number: u16) -> u8 {
        match exception_number {
            EXC_NMI => RANK_NMI,
            EXC_HARDFAULT => RANK_HARDFAULT,
            _ => RANK_CONFIGURABLE_BASE + self.configurable_priority(exception_number),
        }
    }

    /// Raw configurable priority of an exception number
    /// 异常号的可配置优先级原始值
    #[must_use]
    fn configurable_priority(&self, exception_number: u16) -> u8 {
        match exception_number {
            EXC_SVCALL => self.scb.svcall_priority(),
            EXC_PENDSV => self.scb.pendsv_priority(),
            EXC_SYSTICK => self.scb.systick_priority(),
            number if number >= FIRST_EXTERNAL_EXCEPTION => {
                self.nvic.get_priority(number - FIRST_EXTERNAL_EXCEPTION)
            }
            _ => 0,
        }
    }

    /// Whether an exception may be taken right now
    /// 当前是否允许取该异常
    ///
    /// NMI bypasses `PRIMASK`, `FAULTMASK`, `BASEPRI` and every active-exception
    /// comparison except an active NMI. HardFault bypasses the same masks and is
    /// blocked only by an active NMI or HardFault. For configurable exceptions,
    /// `FAULTMASK` blocks everything, `PRIMASK` blocks everything except SVCall
    /// (a synchronous supervisor call latched by `SVCALLPENDED`; keeping it
    /// pending only because `PRIMASK` is set would deadlock a supervisor call
    /// made from a critical section), `BASEPRI` blocks priorities numerically at
    /// or above its value, and an active exception is only preempted by a
    /// strictly higher priority. Tail-chaining an equal priority is decided by
    /// the exception return path, not here.
    /// NMI 绕过 `PRIMASK`、`FAULTMASK`、`BASEPRI` 与除「NMI 已活跃」外的所有
    /// 活跃比较。HardFault 同样绕过这些屏蔽，只被活跃的 NMI 或 HardFault 阻止。
    /// 对可配置异常：`FAULTMASK` 屏蔽全部；`PRIMASK` 屏蔽除 SVCall 外的全部
    /// （同步监管调用由 `SVCALLPENDED` 锁存，仅因 `PRIMASK` 置位就让它永久挂起
    /// 会让临界区中的监管调用死锁）；`BASEPRI` 屏蔽数值大于等于其值的优先级；
    /// 活跃异常只能被严格更高的优先级抢占。同优先级的尾链由异常返回路径决定，
    /// 不在此处判断。
    #[must_use]
    pub(crate) fn exception_entry_allowed(&self, exception_number: u16, rank: u8) -> bool {
        match exception_number {
            EXC_NMI => return !self.active_exceptions.contains(&EXC_NMI),
            EXC_HARDFAULT => {
                return !self.active_exceptions.contains(&EXC_NMI)
                    && !self.active_exceptions.contains(&EXC_HARDFAULT);
            }
            _ => {}
        }
        if self.registers.faultmask & 1 != 0 {
            return false;
        }
        if self.registers.primask & 1 != 0 && exception_number != EXC_SVCALL {
            return false;
        }
        if !self.nvic.basepri_allows_priority(
            self.configurable_priority(exception_number),
            self.registers.basepri,
        ) {
            return false;
        }
        match self.active_exceptions.last() {
            Some(&active) => rank < self.exception_priority(active),
            None => true,
        }
    }

    /// Whether some enabled external interrupt or system exception is pending
    /// 是否有已使能的外部中断或系统异常挂起
    ///
    /// Used by the `WFI`/`WFE`/`SLEEPONEXIT` wake conditions: SysTick, PendSV,
    /// SVCall and NMI pending latches wake the core exactly like an enabled NVIC
    /// line.
    /// 用于 `WFI`/`WFE`/`SLEEPONEXIT` 的唤醒条件：SysTick、PendSV、SVCall 与
    /// NMI 挂起锁存与已使能的 NVIC 线一样能唤醒核心。
    #[must_use]
    pub(crate) fn has_pending_exception(&self) -> bool {
        self.nvic.has_runnable() || self.scb.has_pending()
    }

    /// Take the highest-priority pending exception, if one may be entered
    /// 取最高优先级的挂起异常（若允许进入）
    ///
    /// Returns `true` when an exception was entered; the caller must then skip
    /// the instruction at the restored `PC` for this step. Entry goes through
    /// [`CortexM33Core::enter_exception_checked`], so a stack-limit overflow or
    /// a stacking bus error escalates to HardFault instead of halting the core.
    /// 进入异常时返回 `true`；调用方随后必须在本步跳过恢复出的 `PC` 处指令。
    /// 进入经由 [`CortexM33Core::enter_exception_checked`]，因此栈上限溢出或压栈
    /// 总线错误会升级为 HardFault，而不是让核心停机。
    ///
    /// # Errors
    /// Propagates a bus error from vector fetch or stack push.
    /// 传播向量读取或压栈产生的总线错误。
    pub(crate) fn deliver_pending_exception(
        &mut self,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
    ) -> Result<bool, CoreError> {
        let Some((number, priority)) = self.pending_exception() else {
            return Ok(false);
        };
        if !self.exception_entry_allowed(number, priority) {
            return Ok(false);
        }
        let entered = self.enter_exception_checked(bus, cycles, number)?;
        self.active_exceptions.push(entered);
        // 异常进入是 ARMv8-M 的事件源。 / Exception entry is an ARMv8-M event.
        self.event_register = true;
        Ok(true)
    }

    /// Set the SVCall exception pending (`SHCSR.SVCALLPENDED`)
    /// 置位 SVCall 异常挂起（`SHCSR.SVCALLPENDED`）
    ///
    /// Called when an `SVC` instruction retires. A 0-to-1 transition raises a
    /// `SEVONPEND` event, exactly like the other system-exception latches.
    /// 在 `SVC` 指令退役时调用。0→1 跳变会产生 `SEVONPEND` 事件，与其它系统异常
    /// 锁存一致。
    pub fn pend_svcall(&mut self) {
        if self.scb.set_svcall_pending() && self.scr & scr::SEVONPEND != 0 {
            self.event_register = true;
        }
    }

    /// Set the NMI exception pending (`ICSR.NMIPENDSET`)
    /// 置位 NMI 异常挂起（`ICSR.NMIPENDSET`）
    ///
    /// NMI is non-maskable; the pending latch can only be cleared by taking the
    /// exception, and a 0-to-1 transition raises the `SEVONPEND` event.
    /// NMI 不可屏蔽；挂起锁存只能由取走异常清除，0→1 跳变会产生 `SEVONPEND` 事件。
    pub fn pend_nmi(&mut self) {
        self.pend_system_exception(EXC_NMI);
    }

    /// Set the PendSV exception pending (`ICSR.PENDSVSET` write semantics)
    /// 置位 PendSV 异常挂起（`ICSR.PENDSVSET` 写入语义）
    ///
    /// A 0-to-1 transition raises an event when `SCR.SEVONPEND` is set; writes
    /// to an already-pending latch do not, otherwise `WFE` would stop waiting.
    /// 置位发生 0→1 跳变且 `SCR.SEVONPEND` 置位时产生事件；对已挂起锁存的重复写入
    /// 不产生事件，否则 `WFE` 将不再等待。
    pub fn pend_pendsv(&mut self) {
        self.pend_system_exception(EXC_PENDSV);
    }

    /// Set the SysTick exception pending and latch `SYST_CSR.COUNTFLAG`
    /// 置位 SysTick 异常挂起并锁存 `SYST_CSR.COUNTFLAG`
    ///
    /// `ICSR.PENDSTSET` also sets `COUNTFLAG` (ARMv8-M B3.2.6), which is why the
    /// counter flag is raised here as well.
    /// `ICSR.PENDSTSET` 同时置位 `COUNTFLAG`（ARMv8-M B3.2.6），因此这里也置位计数器
    /// 标志。
    pub fn pend_systick(&mut self) {
        self.systick.set_countflag();
        self.pend_system_exception(EXC_SYSTICK);
    }

    /// Set one system exception pending, raising `SEVONPEND` on a transition
    /// 置位一个系统异常挂起，跳变时产生 `SEVONPEND` 事件
    pub(crate) fn pend_system_exception(&mut self, exception_number: u16) {
        if self.scb.set_pending(exception_number) && self.scr & scr::SEVONPEND != 0 {
            self.event_register = true;
        }
    }

    /// Snapshot of the PendSV / SysTick state for diagnostics
    /// 用于诊断的 PendSV / SysTick 状态快照
    ///
    /// The `Display` implementation prints `systick=<state> pendsv=<state>`,
    /// which is the stable format exposed to frontends.
    /// `Display` 实现输出 `systick=<state> pendsv=<state>`，这是暴露给前端的稳定格式。
    #[must_use]
    pub fn system_exception_state(&self) -> SystemExceptionState {
        SystemExceptionState {
            systick_pending: self.scb.systick_pending(),
            systick_active: self.active_exceptions.contains(&EXC_SYSTICK),
            pendsv_pending: self.scb.pendsv_pending(),
            pendsv_active: self.active_exceptions.contains(&EXC_PENDSV),
            systick_priority: self.scb.systick_priority(),
            pendsv_priority: self.scb.pendsv_priority(),
        }
    }

    /// Apply an `SCB_ICSR` write / 应用一次 `SCB_ICSR` 写入
    ///
    /// `NMIPENDSET`/`PENDSVSET`/`PENDSTSET` pend, `PENDSVCLR`/`PENDSTCLR` clear;
    /// the set bits read back as the latches, the clear bits read as zero.
    /// Writing 0 to `NMIPENDSET` is ignored — NMI has no software clear, only
    /// taking exception 2 clears it. `PENDSTSET` also sets
    /// `SYST_CSR.COUNTFLAG`. All other bits are reserved or read-only and are
    /// ignored on write.
    /// `NMIPENDSET`/`PENDSVSET`/`PENDSTSET` 置挂起，`PENDSVCLR`/`PENDSTCLR` 清除；
    /// 置位位读回为锁存，清除位读为 0。向 `NMIPENDSET` 写 0 被忽略——NMI 没有软件
    /// 清除位，只有取走异常 2 才能清除。`PENDSTSET` 同时置位 `SYST_CSR.COUNTFLAG`。
    /// 其余位为保留或只读，写入时忽略。
    pub(crate) fn write_icsr(&mut self, value: u32) {
        if value & icsr::NMIPENDSET != 0 {
            self.pend_nmi();
        }
        if value & icsr::PENDSVSET != 0 {
            self.pend_pendsv();
        }
        if value & icsr::PENDSVCLR != 0 {
            self.scb.clear_pending(EXC_PENDSV);
        }
        if value & icsr::PENDSTSET != 0 {
            self.pend_systick();
        }
        if value & icsr::PENDSTCLR != 0 {
            self.scb.clear_pending(EXC_SYSTICK);
        }
    }

    /// Compose the `SCB_ICSR` read value / 组合 `SCB_ICSR` 读值
    ///
    /// `VECTACTIVE` is `IPSR`, `VECTPENDING` is the exception number reported by
    /// [`Self::pending_exception`] (including NMI = 2 and SVCall = 11),
    /// `RETTOBASE` is set while at most one exception is active, and
    /// `ISRPENDING` reports any pending NVIC line even when it is disabled.
    /// `VECTACTIVE` 为 `IPSR`，`VECTPENDING` 为 [`Self::pending_exception`] 报告的
    /// 异常号（含 NMI = 2 与 SVCall = 11），最多只有一个活跃异常时置位
    /// `RETTOBASE`，`ISRPENDING` 报告任何挂起的 NVIC 线（即使未使能）。
    pub(crate) fn read_icsr(&self) -> u32 {
        let mut value = self.registers.xpsr & icsr::EXCEPTION_NUMBER_MASK;
        if self.nvic.has_any_pending() {
            value |= icsr::ISRPENDING;
        }
        if let Some((number, _)) = self.pending_exception() {
            value |= u32::from(number) << icsr::VECTPENDING_SHIFT;
        }
        if self.active_exceptions.len() <= 1 {
            value |= icsr::RETTOBASE;
        }
        if self.scb.nmi_pending() {
            value |= icsr::NMIPENDSET;
        }
        if self.scb.pendsv_pending() {
            value |= icsr::PENDSVSET;
        }
        if self.scb.systick_pending() {
            value |= icsr::PENDSTSET;
        }
        value
    }
}
