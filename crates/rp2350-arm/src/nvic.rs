//! NVIC — Nested Vectored Interrupt Controller (ARMv8-M)
//! NVIC — 嵌套向量中断控制器（ARMv8-M）
//!
//! Implements ISER, ICER, ISPR, ICPR, IPR, and STIR registers.
//! Supports 256 interrupts with configurable priority levels.
//! 实现 ISER、ICER、ISPR、ICPR、IPR 和 STIR 寄存器。
//! 支持 256 个中断，具有可配置的优先级级别。

// NVIC 使用 ARM 定义的首字母缩写词命名寄存器，文档标记反映了硬件参考手册中的命名约定。
// NVIC uses ARM-defined acronyms for register names; doc markup reflects naming
// conventions from the hardware reference manual.
#![allow(
    clippy::doc_markdown,
    clippy::unusual_byte_groupings,
    clippy::unreadable_literal,
    clippy::module_name_repetitions
)]

use rp2350_core::types::IrqNumber;

/// NVIC ISER alias offset stride (ARM uses 32 per register)
/// NVIC ISER 别名偏移步长（ARM 每个寄存器 32 个）
const NVIC_REGISTER_STRIDE: usize = 32;

/// Number of NVIC priority bits implemented
/// 实现的 NVIC 优先级位数
///
/// Also used by [`crate::scb::Scb`] for the `SHPR2`/`SHPR3` system-exception
/// priority bytes, so every configurable exception decodes its priority with
/// the same width. / 也被 [`crate::scb::Scb`] 用于 `SHPR2`/`SHPR3` 系统异常
/// 优先级字节，使所有可配置异常用同一宽度解码优先级。
pub(crate) const PRIORITY_BITS: u8 = 3;

/// Nested Vectored Interrupt Controller for Cortex-M33
/// Cortex-M33 的嵌套向量中断控制器
#[derive(Debug, Clone)]
pub struct Nvic {
    /// Interrupt Set-Enable Registers (ISER0..ISER7 for 256 IRQs)
    /// 中断置位使能寄存器
    iser: [u32; 8],
    /// Interrupt Clear-Enable Registers (ICER0..ICER7 for 256 IRQs)
    /// 中断清除使能寄存器
    icer: [u32; 8],
    /// Interrupt Set-Pending Registers (ISPR0..ISPR7 for 256 IRQs)
    /// 中断置位挂起寄存器
    ispr: [u32; 8],
    /// Interrupt Clear-Pending Registers (ICPR0..ICPR7 for 256 IRQs)
    /// 中断清除挂起寄存器
    icpr: [u32; 8],
    /// Interrupt Active Bit Registers (IABR0..IABR7 for 256 IRQs)
    /// 中断活跃位寄存器
    iabr: [u32; 8],
    /// Interrupt Priority Registers (IPR0..IPR63, 8-bit each)
    /// 中断优先级寄存器
    ipr: [u8; 256],
    /// Priority buckets: `priority_mask[p][g]` marks the IRQs of group `g`
    /// whose priority equals `p`
    /// 优先级分桶：`priority_mask[p][g]` 标记第 `g` 组中优先级等于 `p` 的中断
    ///
    /// Selection walks eight buckets instead of 256 interrupts, and the bitmaps
    /// are rebuilt only when a priority register changes.
    /// 选择时只遍历 8 个优先级桶而不是 256 个中断，位图仅在优先级寄存器变化时重建。
    priority_mask: [[u32; 8]; 8],
    /// Conservative hint: "some enabled IRQ is pending"
    /// 保守提示位：「存在已使能且挂起的中断」
    ///
    /// Kept up to date by every register that can change `ISER` or `ISPR`, so a
    /// caller can skip [`Self::find_highest_priority`] entirely when it is
    /// `false`. Active state is not folded in, so a `true` value may still
    /// select nothing; a `false` value is always exact.
    /// 每个可能改变 `ISER` 或 `ISPR` 的寄存器都会维护它，因此它为 `false` 时调用方
    /// 可以完全跳过 [`Self::find_highest_priority`]；活跃状态不参与，因此 `true`
    /// 仍可能选不出中断，而 `false` 永远精确。
    runnable: bool,
}

/// Number of 32-bit groups covering the 256 implemented interrupts
/// 覆盖 256 个已实现中断的 32 位组数
const IRQ_GROUPS: usize = 8;

/// Number of distinct priority values (`PRIORITY_BITS` bits)
/// 不同优先级值的数量（`PRIORITY_BITS` 位）
const PRIORITY_LEVELS: usize = 1 << PRIORITY_BITS;

impl Nvic {
    /// Create a new NVIC in reset state (all interrupts disabled, no pending)
    /// 创建处于复位状态的 NVIC（所有中断禁用，无挂起）
    #[must_use]
    pub fn new() -> Self {
        Self {
            iser: [0; 8],
            icer: [0; 8],
            ispr: [0; 8],
            icpr: [0; 8],
            iabr: [0; 8],
            ipr: [0; 256],
            // 复位时所有 IPR 字节为 0，即全部中断处于优先级 0。
            // All IPR bytes reset to zero, so every interrupt starts at priority 0.
            priority_mask: {
                let mut mask = [[0u32; IRQ_GROUPS]; PRIORITY_LEVELS];
                mask[0] = [u32::MAX; IRQ_GROUPS];
                mask
            },
            // 复位时没有任何中断使能，因此提示位为假。
            // Nothing is enabled at reset, so the hint starts clear.
            runnable: false,
        }
    }

    /// Report whether some enabled interrupt may be pending
    /// 报告是否可能存在已使能且挂起的中断
    ///
    /// `false` is exact: [`Self::find_highest_priority`] cannot select anything
    /// while no `ISER & ISPR` bit is set. `true` is conservative because active
    /// interrupts are not folded into the hint.
    /// `false` 是精确的：没有置位的 `ISER & ISPR` 位时 [`Self::find_highest_priority`]
    /// 必然选不出中断。`true` 是保守的，因为提示位未折算活跃状态。
    #[inline]
    #[must_use]
    pub fn has_runnable(&self) -> bool {
        self.runnable
    }

    /// Recompute the conservative pending hint from `ISER & ISPR`
    /// 依据 `ISER & ISPR` 重算保守挂起提示位
    #[inline]
    fn refresh_runnable(&mut self) {
        self.runnable = (0..IRQ_GROUPS).any(|group| self.iser[group] & self.ispr[group] != 0);
    }

    /// Check if an interrupt is enabled
    /// 检查中断是否已使能
    #[must_use]
    pub fn is_enabled(&self, irq: IrqNumber) -> bool {
        let idx = (irq as usize) / NVIC_REGISTER_STRIDE;
        let bit = (irq as usize) % NVIC_REGISTER_STRIDE;
        (self.iser[idx] >> bit) & 1 != 0
    }

    /// Check if an interrupt is pending
    /// 检查中断是否挂起
    #[must_use]
    pub fn is_pending(&self, irq: IrqNumber) -> bool {
        let idx = (irq as usize) / NVIC_REGISTER_STRIDE;
        let bit = (irq as usize) % NVIC_REGISTER_STRIDE;
        (self.ispr[idx] >> bit) & 1 != 0
    }

    /// Check if an interrupt is active
    /// 检查中断是否活跃
    #[must_use]
    pub fn is_active(&self, irq: IrqNumber) -> bool {
        let idx = (irq as usize) / NVIC_REGISTER_STRIDE;
        let bit = (irq as usize) % NVIC_REGISTER_STRIDE;
        (self.iabr[idx] >> bit) & 1 != 0
    }

    /// Get the priority of an interrupt
    /// 获取中断优先级
    #[must_use]
    pub fn get_priority(&self, irq: IrqNumber) -> u8 {
        self.ipr[irq as usize] >> (8 - PRIORITY_BITS)
    }

    /// Set the priority of an interrupt
    /// 设置中断优先级
    pub fn set_priority(&mut self, irq: IrqNumber, priority: u8) {
        self.write_ipr_byte(irq as usize, priority << (8 - PRIORITY_BITS));
    }

    /// Store one IPR byte and keep the priority buckets in sync
    /// 写入一个 IPR 字节并同步优先级分桶
    ///
    /// The buckets only change when the *decoded* priority changes, so repeated
    /// writes of the same value cost nothing beyond one comparison.
    /// 只有*解码后*的优先级变化时才移动桶内位，因此重复写同一值的开销只有一次比较。
    fn write_ipr_byte(&mut self, irq: usize, value: u8) {
        if irq >= self.ipr.len() {
            return;
        }
        let old = (self.ipr[irq] >> (8 - PRIORITY_BITS)) as usize;
        let new = (value >> (8 - PRIORITY_BITS)) as usize;
        self.ipr[irq] = value;
        if old != new {
            let group = irq / NVIC_REGISTER_STRIDE;
            let bit = 1u32 << (irq % NVIC_REGISTER_STRIDE);
            self.priority_mask[old][group] &= !bit;
            self.priority_mask[new][group] |= bit;
        }
    }

    /// Read ISER value (word at 4-byte offset)
    /// 读取 ISER 值（4 字节偏移处的字）
    #[must_use]
    pub fn read_iser(&self, index: usize) -> u32 {
        self.iser.get(index).copied().unwrap_or(0)
    }

    /// Read an IPR word (4 IRQs packed into one u32)
    /// 读取 IPR 字（4 个 IRQ 打包到一个 u32 中）
    #[must_use]
    pub fn read_ipr_word(&self, index: usize) -> u32 {
        let base = index * 4;
        let mut word: u32 = 0;
        for i in 0..4 {
            if base + i < 256 {
                word |= (self.ipr[base + i] as u32) << (i * 8);
            }
        }
        word
    }

    /// Write an IPR word (4 IRQs packed into one u32)
    /// 写入 IPR 字（4 个 IRQ 打包到一个 u32 中）
    pub fn write_ipr_word(&mut self, index: usize, value: u32) {
        let base = index * 4;
        for i in 0..4 {
            if base + i < 256 {
                self.write_ipr_byte(base + i, ((value >> (i * 8)) & 0xFF) as u8);
            }
        }
    }

    /// Write ISER value — SETs bits (writing 1 sets, writing 0 has no effect)
    /// 写入 ISER 值 — SET 位（写 1 置位，写 0 无效果）
    pub fn write_iser(&mut self, index: usize, value: u32) {
        if let Some(reg) = self.iser.get_mut(index) {
            // ISER is write-1-to-set (reads return current enable state)
            // ISER 是写 1 置位（读返回当前使能状态）
            *reg |= value;
            self.refresh_runnable();
        }
    }

    /// Read ICER value (word at 4-byte offset)
    /// 读取 ICER 值（4 字节偏移处的字）
    #[must_use]
    pub fn read_icer(&self, index: usize) -> u32 {
        self.icer.get(index).copied().unwrap_or(0)
    }

    /// Write ICER value — CLEARs bits (writing 1 clears, writing 0 has no effect)
    /// 写入 ICER 值 — CLEAR 位（写 1 清除，写 0 无效果）
    pub fn write_icer(&mut self, index: usize, value: u32) {
        if let Some(reg) = self.icer.get_mut(index) {
            *reg = value;
        }
        if let Some(en) = self.iser.get_mut(index) {
            // ICER is write-1-to-clear
            // ICER 是写 1 清除
            *en &= !value;
        }
        self.refresh_runnable();
    }

    /// Read ISPR value (word at 4-byte offset)
    /// 读取 ISPR 值（4 字节偏移处的字）
    #[must_use]
    pub fn read_ispr(&self, index: usize) -> u32 {
        self.ispr.get(index).copied().unwrap_or(0)
    }

    /// Write ISPR value — SETs pending bits (write-1-to-set)
    /// 写入 ISPR 值 — SET 挂起位（写 1 置位）
    pub fn write_ispr(&mut self, index: usize, value: u32) {
        if let Some(reg) = self.ispr.get_mut(index) {
            *reg |= value;
        }
        self.refresh_runnable();
    }

    /// Read ICPR value (word at 4-byte offset)
    /// 读取 ICPR 值（4 字节偏移处的字）
    #[must_use]
    pub fn read_icpr(&self, index: usize) -> u32 {
        self.icpr.get(index).copied().unwrap_or(0)
    }

    /// Write ICPR value — CLEARs pending bits (write-1-to-clear)
    /// 写入 ICPR 值 — CLEAR 挂起位（写 1 清除）
    pub fn write_icpr(&mut self, index: usize, value: u32) {
        if let Some(reg) = self.icpr.get_mut(index) {
            *reg = value;
        }
        if let Some(pend) = self.ispr.get_mut(index) {
            *pend &= !value;
        }
        self.refresh_runnable();
    }

    /// Read IABR value (word at 4-byte offset)
    /// 读取 IABR 值（4 字节偏移处的字）
    #[must_use]
    pub fn read_iabr(&self, index: usize) -> u32 {
        self.iabr.get(index).copied().unwrap_or(0)
    }

    /// Set an interrupt active
    /// 设置中断为活跃
    pub fn set_active(&mut self, irq: IrqNumber) {
        let idx = (irq as usize) / NVIC_REGISTER_STRIDE;
        let bit = (irq as usize) % NVIC_REGISTER_STRIDE;
        if let Some(reg) = self.iabr.get_mut(idx) {
            *reg |= 1 << bit;
        }
    }

    /// Clear an interrupt active
    /// 清除中断活跃状态
    pub fn clear_active(&mut self, irq: IrqNumber) {
        let idx = (irq as usize) / NVIC_REGISTER_STRIDE;
        let bit = (irq as usize) % NVIC_REGISTER_STRIDE;
        if let Some(reg) = self.iabr.get_mut(idx) {
            *reg &= !(1 << bit);
        }
    }

    /// Set interrupt pending via hardware (external event)
    /// 通过硬件设置中断挂起（外部事件）
    pub fn set_pending(&mut self, irq: IrqNumber) {
        let idx = (irq as usize) / NVIC_REGISTER_STRIDE;
        let bit = (irq as usize) % NVIC_REGISTER_STRIDE;
        if let Some(reg) = self.ispr.get_mut(idx) {
            *reg |= 1 << bit;
        }
        // 只在该中断已使能时点亮提示位；无需扫描全部 256 项。
        // Raise the hint only when this interrupt is enabled; no 256-entry scan.
        if self.iser.get(idx).is_some_and(|en| en & (1 << bit) != 0) {
            self.runnable = true;
        }
    }

    /// Clear interrupt pending
    /// 清除中断挂起
    pub fn clear_pending(&mut self, irq: IrqNumber) {
        let idx = (irq as usize) / NVIC_REGISTER_STRIDE;
        let bit = (irq as usize) % NVIC_REGISTER_STRIDE;
        if let Some(reg) = self.ispr.get_mut(idx) {
            *reg &= !(1 << bit);
        }
        self.refresh_runnable();
    }

    /// Software Trigger Interrupt Register — trigger an IRQ via software
    /// 软件触发中断寄存器 — 通过软件触发 IRQ
    pub fn write_stir(&mut self, irq: IrqNumber) {
        if irq < 256 {
            self.set_pending(irq);
        }
    }

    /// Whether `irq` passes a `BASEPRI` mask / `irq` 是否通过 `BASEPRI` 屏蔽
    ///
    /// `BASEPRI == 0` disables the mask. Otherwise an interrupt is masked when its
    /// priority is numerically at or above the `BASEPRI` value, which is the
    /// ARMv8-M rule; the stored `BASEPRI` value only uses its top priority bits,
    /// so the comparison happens on the same width as [`Self::get_priority`].
    /// `BASEPRI == 0` 表示关闭屏蔽。否则优先级数值大于等于 `BASEPRI` 值的中断被屏蔽，
    /// 这是 ARMv8-M 的规则；`BASEPRI` 只使用其高位优先级字段，因此比较与
    /// [`Self::get_priority`] 使用相同宽度。
    #[must_use]
    pub fn basepri_allows(&self, irq: IrqNumber, basepri: u32) -> bool {
        self.basepri_allows_priority(self.get_priority(irq), basepri)
    }

    /// Whether a raw priority value passes a `BASEPRI` mask
    /// 原始优先级数值是否通过 `BASEPRI` 屏蔽
    ///
    /// System exceptions (PendSV, SysTick) have no NVIC IRQ number, so their
    /// `SHPR2`/`SHPR3` priority must be tested with the same rule without going
    /// through [`Self::get_priority`].
    /// 系统异常（PendSV、SysTick）没有 NVIC 中断号，其 `SHPR2`/`SHPR3` 优先级必须
    /// 用同一规则判定，而不能经过 [`Self::get_priority`]。
    #[must_use]
    pub fn basepri_allows_priority(&self, priority: u8, basepri: u32) -> bool {
        let basepri = (basepri & 0xFF) as u8;
        basepri == 0 || priority < (basepri >> (8 - PRIORITY_BITS))
    }

    /// Whether any external interrupt line is pending, enabled or not
    /// 是否有任何外部中断线挂起（无论是否使能）
    ///
    /// This is the `ICSR.ISRPENDING` view: the NVIC pending lines with active
    /// exceptions excluded (an active line that is still asserted has already
    /// been taken and is not reported as pending again).
    /// 这是 `ICSR.ISRPENDING` 的视图：NVIC 挂起线，排除活跃异常（仍然保持电平的
    /// 活跃线已被取走，不再报告为挂起）。
    #[must_use]
    pub fn has_any_pending(&self) -> bool {
        (0..IRQ_GROUPS).any(|group| self.ispr[group] & !self.iabr[group] != 0)
    }

    /// Find the highest-priority pending enabled interrupt
    /// 查找最高优先级的挂起已使能中断    ///
    /// Active interrupts are skipped: an interrupt cannot preempt itself, so a
    /// level still asserted by its peripheral must wait for exception return.
    /// 活跃中断会被跳过：中断不能抢占自身，因此外设仍保持的电平必须等到
    /// 异常返回后再处理。
    ///
    /// Returns `None` if no interrupt is pending and enabled.
    /// 如果没有挂起且已使能的中断，返回 `None`。
    ///
    /// The candidate set is built as eight `u32` bitmaps (`enabled & pending &
    /// !active`) and then walked bucket by bucket from priority 0 upwards, so a
    /// cycle costs a handful of word operations instead of 256 interrupt tests.
    /// Ties inside one priority bucket pick the lowest IRQ number because
    /// `trailing_zeros` returns the least significant set bit.
    /// 候选集先表示为 8 个 `u32` 位图（`使能 & 挂起 & !活跃`），再按优先级从 0
    /// 逐桶遍历；一次选择只需若干字操作，而不是 256 次中断判定。同一优先级桶内
    /// 由 `trailing_zeros` 取最低置位，因此并列时总是 IRQ 号最小者。
    #[must_use]
    pub fn find_highest_priority(&self) -> Option<IrqNumber> {
        let mut candidates = [0u32; IRQ_GROUPS];
        for (group, bits) in candidates.iter_mut().enumerate() {
            // 活跃中断不能抢占自身，因此从候选集中排除。
            // An active interrupt cannot preempt itself, so drop it.
            *bits = self.iser[group] & self.ispr[group] & !self.iabr[group];
        }
        if candidates.iter().all(|bits| *bits == 0) {
            return None;
        }
        // 数字越小优先级越高。/
        // A numerically smaller priority wins.
        for priority in 0..PRIORITY_LEVELS {
            let bucket = &self.priority_mask[priority];
            for (group, bits) in candidates.iter().enumerate() {
                let hit = *bits & bucket[group];
                if hit != 0 {
                    #[allow(clippy::cast_possible_truncation)]
                    let irq = (group as u16) * 32 + hit.trailing_zeros() as u16;
                    return Some(irq);
                }
            }
        }
        None
    }
}

impl Default for Nvic {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_nvic_all_disabled() {
        let nvic = Nvic::new();
        assert!(!nvic.is_enabled(5));
        assert!(!nvic.is_pending(5));
    }

    #[test]
    fn set_pending_and_enable() {
        let mut nvic = Nvic::new();
        nvic.write_iser(0, 1 << 5); // Enable IRQ5
        assert!(nvic.is_enabled(5));
        assert!(!nvic.is_pending(5));

        nvic.set_pending(5);
        assert!(nvic.is_pending(5));
        assert_eq!(nvic.find_highest_priority(), Some(5));
    }

    #[test]
    fn priority_routing() {
        let mut nvic = Nvic::new();
        nvic.write_iser(0, (1 << 3) | (1 << 7));
        nvic.set_pending(3);
        nvic.set_pending(7);
        nvic.set_priority(3, 2);
        nvic.set_priority(7, 1); // Lower number = higher priority

        assert_eq!(nvic.find_highest_priority(), Some(7));
    }

    #[test]
    fn clear_pending() {
        let mut nvic = Nvic::new();
        nvic.write_iser(0, 1 << 10);
        nvic.set_pending(10);
        assert!(nvic.is_pending(10));
        nvic.write_icpr(0, 1 << 10);
        assert!(!nvic.is_pending(10));
        assert_eq!(nvic.find_highest_priority(), None);
    }

    #[test]
    fn stir_triggers_pending() {
        let mut nvic = Nvic::new();
        nvic.write_iser(0, 1 << 15);
        nvic.write_stir(15);
        assert!(nvic.is_pending(15));
    }

    #[test]
    fn active_interrupt_is_not_selected_again() {
        let mut nvic = Nvic::new();
        nvic.write_iser(0, 1 << 3);
        nvic.set_pending(3);
        assert_eq!(nvic.find_highest_priority(), Some(3));

        nvic.set_active(3);
        assert_eq!(nvic.find_highest_priority(), None);

        nvic.clear_active(3);
        assert_eq!(nvic.find_highest_priority(), Some(3));
    }

    /// Linear scan used as the reference for the bitmap selector
    /// 位图选择器的参考实现（线性扫描）
    fn reference(nvic: &Nvic) -> Option<IrqNumber> {
        let mut best_irq = None;
        let mut best_prio = u8::MAX;
        for irq in 0u16..256 {
            if !nvic.is_enabled(irq) || !nvic.is_pending(irq) || nvic.is_active(irq) {
                continue;
            }
            let prio = nvic.get_priority(irq);
            if prio < best_prio {
                best_prio = prio;
                best_irq = Some(irq);
            }
        }
        best_irq
    }

    /// Fill the NVIC from a deterministic pseudo-random stream
    /// 用确定性伪随机流填充 NVIC
    fn fill_pseudo_random(nvic: &mut Nvic, seed: u64) {
        let mut state = seed;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as u32
        };
        for index in 0..8 {
            nvic.write_iser(index, next());
            nvic.write_ispr(index, next());
            nvic.write_ipr_word(index * 8, next());
            nvic.write_ipr_word(index * 8 + 1, next());
            nvic.write_ipr_word(index * 8 + 2, next());
            nvic.write_ipr_word(index * 8 + 3, next());
            nvic.write_ipr_word(index * 8 + 4, next());
            nvic.write_ipr_word(index * 8 + 5, next());
            nvic.write_ipr_word(index * 8 + 6, next());
            nvic.write_ipr_word(index * 8 + 7, next());
        }
        // 半数 IRQ 置为活跃，覆盖「活跃中断被跳过」的分支。
        // Mark half the IRQs active to exercise the "active is skipped" branch.
        for irq in 0..256u16 {
            if next() & 1 != 0 {
                nvic.set_active(irq);
            }
        }
    }

    #[test]
    fn bitmap_selector_matches_the_linear_reference() {
        for seed in 1..24u64 {
            let mut nvic = Nvic::new();
            fill_pseudo_random(&mut nvic, seed);
            assert_eq!(
                nvic.find_highest_priority(),
                reference(&nvic),
                "seed {seed} must agree with the linear scan"
            );
        }
    }

    #[test]
    fn every_irq_enabled_and_pending_picks_the_lowest_number() {
        let mut nvic = Nvic::new();
        for index in 0..8 {
            nvic.write_iser(index, u32::MAX);
            nvic.write_ispr(index, u32::MAX);
        }
        // 全部默认优先级 0：必须取 IRQ0。
        // All priorities reset to 0, so IRQ0 must win.
        assert_eq!(nvic.find_highest_priority(), Some(0));
        // 屏蔽 IRQ0 后应取 IRQ1，说明同优先级按编号最小者。
        // Masking IRQ0 must select IRQ1, proving ties pick the lowest number.
        nvic.write_icpr(0, 1);
        assert_eq!(nvic.find_highest_priority(), Some(1));
        nvic.set_active(1);
        assert_eq!(nvic.find_highest_priority(), Some(2));
    }

    #[test]
    fn equal_priority_across_groups_picks_the_lowest_irq() {
        let mut nvic = Nvic::new();
        nvic.write_iser(0, 1 << 31);
        nvic.write_iser(7, 1 << 31);
        nvic.set_pending(31);
        nvic.set_pending(255);
        nvic.set_priority(31, 3);
        nvic.set_priority(255, 3);
        assert_eq!(nvic.find_highest_priority(), Some(31));
        nvic.set_active(31);
        assert_eq!(nvic.find_highest_priority(), Some(255));
    }

    #[test]
    fn lower_priority_number_wins_across_groups() {
        let mut nvic = Nvic::new();
        nvic.write_iser(7, 1);
        nvic.write_iser(0, 1);
        nvic.set_pending(7 * 32);
        nvic.set_pending(0);
        nvic.set_priority(7 * 32, 0);
        nvic.set_priority(0, 5);
        assert_eq!(nvic.find_highest_priority(), Some(7 * 32));
        nvic.set_priority(0, 0);
        assert_eq!(nvic.find_highest_priority(), Some(0));
    }

    #[test]
    fn ipr_word_writes_move_ints_between_priority_buckets() {
        let mut nvic = Nvic::new();
        nvic.write_iser(0, (1 << 1) | (1 << 2));
        nvic.set_pending(1);
        nvic.set_pending(2);
        // IPR 字按小端打包：字节 1 是 IRQ1，字节 2 是 IRQ2。
        // 把 IRQ1 设为优先级 0、IRQ2 设为 7，然后反过来。
        // The IPR word packs little-endian: byte 1 is IRQ1, byte 2 is IRQ2.
        // Program IRQ1 to 0 and IRQ2 to 7, then swap them.
        nvic.write_ipr_word(0, 0x00E0_0000);
        assert_eq!(nvic.get_priority(1), 0);
        assert_eq!(nvic.get_priority(2), 7);
        assert_eq!(nvic.find_highest_priority(), Some(1));
        // 同优先级 7 时编号更小者胜出。
        // Both at priority 7: the lower IRQ number wins.
        nvic.write_ipr_word(0, 0x00E0_E000);
        assert_eq!(nvic.find_highest_priority(), Some(1));
        // 把 IRQ2 提升到 0：它现在必须胜出。
        // Promote IRQ2 to 0: it must win now.
        nvic.write_ipr_word(0, 0x0000_E000);
        assert_eq!(nvic.find_highest_priority(), Some(2));
    }

    #[test]
    fn disabled_or_unpending_or_active_irqs_are_skipped() {
        let mut nvic = Nvic::new();
        // IRQ4/IRQ5 位于 ISER0（每 32 个 IRQ 一个寄存器）。
        // IRQ4/IRQ5 live in ISER0 (one register per 32 IRQs).
        nvic.write_iser(0, (1 << 4) | (1 << 5));
        nvic.set_pending(4);
        nvic.set_pending(5);
        assert_eq!(nvic.find_highest_priority(), Some(4));
        nvic.write_icer(0, 1 << 4);
        assert_eq!(nvic.find_highest_priority(), Some(5));
        nvic.set_active(5);
        assert_eq!(nvic.find_highest_priority(), None);
        nvic.write_icpr(0, 1 << 5);
        nvic.clear_active(5);
        assert_eq!(nvic.find_highest_priority(), None);
    }

    #[test]
    fn reset_priorities_are_zero_for_every_irq() {
        let nvic = Nvic::new();
        for irq in 0u16..256 {
            assert_eq!(nvic.get_priority(irq), 0);
        }
    }

    #[test]
    fn basepri_priority_mask_matches_the_irq_helper() {
        let mut nvic = Nvic::new();
        nvic.set_priority(5, 2);
        nvic.set_priority(6, 0);
        // BASEPRI=0 关闭屏蔽；优先级 2 在 BASEPRI 2/3 下被屏蔽。
        // BASEPRI=0 disables the mask; priority 2 is masked by BASEPRI 2 and 3.
        assert!(nvic.basepri_allows(5, 0));
        assert!(nvic.basepri_allows_priority(2, 0));
        assert!(!nvic.basepri_allows(5, 0x40));
        assert!(!nvic.basepri_allows_priority(2, 0x40));
        assert!(nvic.basepri_allows_priority(1, 0x40));
        assert!(nvic.basepri_allows(6, 0x40));
    }

    #[test]
    fn any_pending_ignores_enable_state_but_not_active_state() {
        let mut nvic = Nvic::new();
        assert!(!nvic.has_any_pending());
        // 未使能也必须报告为挂起（ICSR.ISRPENDING 不检查 ISER）。
        // Pending without enable must still report (ISRPENDING ignores ISER).
        nvic.set_pending(200);
        assert!(nvic.has_any_pending());
        nvic.set_active(200);
        assert!(!nvic.has_any_pending());
        nvic.clear_active(200);
        assert!(nvic.has_any_pending());
        nvic.clear_pending(200);
        assert!(!nvic.has_any_pending());
    }

    /// Reference predicate for the gate: some enabled IRQ is pending
    /// 门控的参考判据：存在已使能且挂起的中断
    fn reference_pending(nvic: &Nvic) -> bool {
        (0u16..256).any(|irq| nvic.is_enabled(irq) && nvic.is_pending(irq))
    }

    #[test]
    fn runnable_hint_tracks_every_writer_of_enable_and_pending() {
        let mut nvic = Nvic::new();
        assert!(
            !nvic.has_runnable(),
            "reset has nothing pending / 复位时无挂起"
        );

        // 使能但未挂起：提示位必须保持为假。
        // Enabled but not pending: the hint must stay clear.
        nvic.write_iser(0, 1 << 3);
        assert!(!nvic.has_runnable());

        nvic.write_stir(3);
        assert!(nvic.has_runnable());

        nvic.write_icpr(0, 1 << 3);
        assert!(!nvic.has_runnable());

        // ISPR 直接写入与硬件 set_pending 同样点亮提示位（需先使能）。
        // A direct ISPR write and a hardware `set_pending` both raise it, but
        // only for an enabled IRQ.
        nvic.write_iser(1, 1 << 5);
        nvic.write_ispr(1, 1 << 5);
        assert!(nvic.has_runnable());
        nvic.clear_pending(32 + 5);
        assert!(!nvic.has_runnable());
        nvic.set_pending(3);
        assert!(nvic.has_runnable());
        nvic.clear_pending(3);
        assert!(!nvic.has_runnable());

        // 挂起但未使能不会点亮；随后使能才点亮。
        // Pending without enable stays clear; enabling raises it.
        nvic.set_pending(200);
        assert!(!nvic.has_runnable());
        nvic.write_iser(6, 1 << 8);
        assert!(nvic.has_runnable());
        nvic.write_icer(6, 1 << 8);
        assert!(!nvic.has_runnable());
    }

    #[test]
    fn runnable_hint_stays_set_while_the_irq_is_active() {
        let mut nvic = Nvic::new();
        nvic.write_iser(0, 1 << 4);
        nvic.set_pending(4);
        nvic.set_active(4);
        // 活跃中断不能抢占自身，但提示位是保守的：仍为真。
        // An active IRQ cannot preempt itself, but the hint stays conservative.
        assert!(nvic.has_runnable());
        assert_eq!(nvic.find_highest_priority(), None);
        nvic.clear_active(4);
        assert_eq!(nvic.find_highest_priority(), Some(4));
    }

    #[test]
    fn closed_gate_never_hides_a_runnable_irq() {
        for seed in 1..16u64 {
            let mut nvic = Nvic::new();
            fill_pseudo_random(&mut nvic, seed);
            assert_eq!(
                nvic.has_runnable(),
                reference_pending(&nvic),
                "seed {seed} must match the linear reference / 必须与线性参考一致"
            );
            // 随机改写挂起集合后仍需与参考一致。
            // Random pending rewrites must keep matching the reference.
            let mut state = seed;
            let mut next = || {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 33) as u32
            };
            for _ in 0..32 {
                let index = (next() % 8) as usize;
                let value = next();
                if next() & 1 == 0 {
                    nvic.write_ispr(index, value);
                } else {
                    nvic.write_icpr(index, value);
                }
                assert_eq!(nvic.has_runnable(), reference_pending(&nvic));
            }
        }
    }
}
