//! PendSV / SysTick system-exception arbitration and delivery
//! PendSV / SysTick 系统异常仲裁与投递
//!
//! ARMv8-M exception numbers are not a flat NVIC index: `PendSV` is 14 and
//! `SysTick` is 15, and only external interrupts use `16 + n`. This module
//! implements the arbitration between those two system exceptions and the
//! external interrupt lines: numeric-priority ordering, the lowest exception
//! number as tie-break, the `PRIMASK`/`FAULTMASK`/`BASEPRI` gates and the
//! "only a strictly higher priority may preempt" rule. It also owns the
//! `SCB_ICSR` read/write view (`PENDSTSET`/`PENDSTCLR`/`PENDSVSET`/
//! `PENDSVCLR`/`VECTPENDING`).
//! ARMv8-M 的异常号并不是扁平的 NVIC 索引：`PendSV` 为 14、`SysTick` 为 15，
//! 只有外部中断才用 `16 + n`。本模块实现这两个系统异常与外部中断线之间的仲裁：
//! 数值优先级排序、同优先级取最小异常号、`PRIMASK`/`FAULTMASK`/`BASEPRI` 门控，
//! 以及「只有严格更高优先级才能抢占」的规则；同时负责 `SCB_ICSR` 的读写视图
//! （`PENDSTSET`/`PENDSTCLR`/`PENDSVSET`/`PENDSVCLR`/`VECTPENDING`）。

// 仲裁代码直接操作异常号与优先级掩码，属于异常控制器仿真的固有写法。
// Arbitration manipulates exception numbers and priority masks directly, which
// is inherent to exception-controller emulation.
#![allow(clippy::doc_markdown, clippy::module_name_repetitions)]

use std::fmt;

use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;

use crate::exceptions::{self, EXC_PENDSV, EXC_SYSTICK};
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
    /// Highest-priority pending exception as `(exception number, priority)`
    /// 最高优先级的挂起异常，形式为 `(异常号, 优先级)`
    ///
    /// External interrupts come from [`crate::Nvic::find_highest_priority`]
    /// (enabled, pending and not active), PendSV and SysTick from
    /// [`crate::scb::Scb`]. Ties compare the exception number, which is exactly
    /// the ARMv8-M rule: a numerically smaller exception number wins, so
    /// PendSV (14) beats SysTick (15) and both beat every external interrupt.
    /// 外部中断来自 [`crate::Nvic::find_highest_priority`]（已使能、挂起且非活跃），
    /// PendSV 与 SysTick 来自 [`crate::scb::Scb`]。并列时比较异常号，这正是
    /// ARMv8-M 的规则：异常号更小者胜，因此 PendSV（14）优先于 SysTick（15），
    /// 两者都优先于任何外部中断。
    #[must_use]
    pub(crate) fn pending_exception(&self) -> Option<(u16, u8)> {
        let mut best = self
            .nvic
            .find_highest_priority()
            .map(|irq| (FIRST_EXTERNAL_EXCEPTION + irq, self.nvic.get_priority(irq)));
        for number in [EXC_PENDSV, EXC_SYSTICK] {
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

    /// Priority of an exception number, active or pending
    /// 异常号的优先级（活跃或挂起）
    ///
    /// PendSV and SysTick decode their `SHPR3` bytes, external interrupts their
    /// NVIC `IPR` byte. The fixed-priority system exceptions modelled here
    /// (`UsageFault`, `SVCall`) report priority 0, the highest configurable
    /// level, so no configurable exception can preempt them.
    /// PendSV 与 SysTick 解码 `SHPR3` 字节，外部中断解码 NVIC `IPR` 字节。此处建模的
    /// 固定优先级系统异常（`UsageFault`、`SVCall`）报告优先级 0，即可配置的最高
    /// 级别，因此任何可配置异常都不能抢占它们。
    #[must_use]
    pub(crate) fn exception_priority(&self, exception_number: u16) -> u8 {
        match exception_number {
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
    /// `PRIMASK` and `FAULTMASK` block every configurable exception (PendSV and
    /// SysTick included), `BASEPRI` blocks priorities numerically at or above
    /// its value, and an already active exception is only preempted by a
    /// strictly higher priority. Tail-chaining an equal priority is decided by
    /// the exception return path, not here.
    /// `PRIMASK` 与 `FAULTMASK` 屏蔽所有可配置异常（含 PendSV 与 SysTick），
    /// `BASEPRI` 屏蔽数值大于等于其值的优先级，已活跃异常只能被严格更高的优先级
    /// 抢占。同优先级的尾链由异常返回路径决定，不在此处判断。
    #[must_use]
    pub(crate) fn exception_entry_allowed(&self, priority: u8) -> bool {
        if self.registers.primask & 1 != 0 || self.registers.faultmask & 1 != 0 {
            return false;
        }
        if !self
            .nvic
            .basepri_allows_priority(priority, self.registers.basepri)
        {
            return false;
        }
        match self.active_exceptions.last() {
            Some(&active) => priority < self.exception_priority(active),
            None => true,
        }
    }

    /// Whether some enabled external interrupt or system exception is pending
    /// 是否有已使能的外部中断或系统异常挂起
    ///
    /// Used by the `WFI`/`WFE`/`SLEEPONEXIT` wake conditions: SysTick and PendSV
    /// pending latches wake the core exactly like an enabled NVIC line.
    /// 用于 `WFI`/`WFE`/`SLEEPONEXIT` 的唤醒条件：SysTick 与 PendSV 挂起锁存
    /// 与已使能的 NVIC 线一样能唤醒核心。
    #[must_use]
    pub(crate) fn has_pending_exception(&self) -> bool {
        self.nvic.has_runnable() || self.scb.has_pending()
    }

    /// Take the highest-priority pending exception, if one may be entered
    /// 取最高优先级的挂起异常（若允许进入）
    ///
    /// Returns `true` when an exception was entered; the caller must then skip
    /// the instruction at the restored `PC` for this step.
    /// 进入异常时返回 `true`；调用方随后必须在本步跳过恢复出的 `PC` 处指令。
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
        if !self.exception_entry_allowed(priority) {
            return Ok(false);
        }
        exceptions::enter_exception(
            bus,
            &mut self.registers,
            &mut self.nvic,
            &mut self.scb,
            u32::from(number),
            self.vtor,
            &mut self.it_state,
            cycles,
        )?;
        self.active_exceptions.push(number);
        // 异常进入是 ARMv8-M 的事件源。 / Exception entry is an ARMv8-M event.
        self.event_register = true;
        Ok(true)
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
    /// `PENDSVSET`/`PENDSTSET` pend, `PENDSVCLR`/`PENDSTCLR` clear; the set bits
    /// read back as the latches, the clear bits read as zero. `PENDSTSET` also
    /// sets `SYST_CSR.COUNTFLAG`. All other bits are reserved or read-only and
    /// are ignored on write.
    /// `PENDSVSET`/`PENDSTSET` 置挂起，`PENDSVCLR`/`PENDSTCLR` 清除；置位位读回为
    /// 锁存，清除位读为 0。`PENDSTSET` 同时置位 `SYST_CSR.COUNTFLAG`。其余位为保留
    /// 或只读，写入时忽略。
    pub(crate) fn write_icsr(&mut self, value: u32) {
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
    /// [`Self::pending_exception`], `RETTOBASE` is set while at most one
    /// exception is active, and `ISRPENDING` reports any pending NVIC line even
    /// when it is disabled.
    /// `VECTACTIVE` 为 `IPSR`，`VECTPENDING` 为 [`Self::pending_exception`] 报告的
    /// 异常号，最多只有一个活跃异常时置位 `RETTOBASE`，`ISRPENDING` 报告任何挂起的
    /// NVIC 线（即使未使能）。
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
        if self.scb.pendsv_pending() {
            value |= icsr::PENDSVSET;
        }
        if self.scb.systick_pending() {
            value |= icsr::PENDSTSET;
        }
        value
    }
}
