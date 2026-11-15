//! System Control Block exception state (ARMv8-M)
//! 系统控制块异常状态（ARMv8-M）
//!
//! ARMv8-M numbers the software-triggered system exceptions separately from
//! the external interrupt lines: `NMI` is exception 2, `PendSV` is exception
//! 14, `SVCall` is exception 11 and `SysTick` is exception 15, while external
//! interrupt `n` is exception `16 + n`. Their pending bits live in `SCB_ICSR`
//! (`NMIPENDSET` / `PENDSVSET` / `PENDSVCLR` / `PENDSTSET` / `PENDSTCLR`) or in
//! `SHCSR.SVCALLPENDED` (modelled by [`crate::faults`]), and their priorities
//! live in `SCB_SHPR2` (SVCall in byte 3) and `SCB_SHPR3` (PendSV in byte 2,
//! SysTick in byte 3). This module keeps that state apart from [`crate::Nvic`],
//! whose 256 pending bits are the external IRQ lines only, so a SysTick
//! exception can never be mistaken for external IRQ 15.
//! ARMv8-M 把软件触发的系统异常与外部中断线分开编号：`NMI` 为异常 2、`PendSV`
//! 为异常 14、`SVCall` 为异常 11、`SysTick` 为异常 15，而外部中断 `n` 为异常
//! `16 + n`。它们的挂起位位于 `SCB_ICSR`（`NMIPENDSET`/`PENDSVSET`/`PENDSVCLR`/
//! `PENDSTSET`/`PENDSTCLR`）或 `SHCSR.SVCALLPENDED`（由 [`crate::faults`] 建模），
//! 优先级位于 `SCB_SHPR2`（第 3 字节 SVCall）与 `SCB_SHPR3`（第 2 字节 PendSV、
//! 第 3 字节 SysTick）。本模块把这些状态与 [`crate::Nvic`] 的 256 个挂起位分开
//! 保存，后者只表示外部中断线，因此 SysTick 异常绝不会被误当作外部 IRQ 15。

// SCB 寄存器使用 ARM 定义的首字母缩写词命名，文档标记反映硬件参考手册的命名约定。
// SCB registers use ARM-defined acronyms; doc markup reflects the reference manual.
#![allow(clippy::doc_markdown, clippy::module_name_repetitions)]

use crate::nvic::PRIORITY_BITS;

/// `SCB_ICSR` bit definitions (ARMv8-M B3.2.6) / `SCB_ICSR` 位定义
pub mod icsr {
    /// NMI set-pending (write 1 to pend; reads back the pending state)
    /// NMI 置挂起（写 1 挂起；读回挂起状态）
    pub const NMIPENDSET: u32 = 1 << 31;

    /// PendSV set-pending (write 1 to pend; reads back the pending state)
    /// PendSV 置挂起（写 1 挂起；读回挂起状态）
    pub const PENDSVSET: u32 = 1 << 28;

    /// PendSV clear-pending (write 1 to clear; reads as zero)
    /// PendSV 清挂起（写 1 清除；读为 0）
    pub const PENDSVCLR: u32 = 1 << 27;

    /// SysTick set-pending (write 1 to pend; reads back the pending state)
    /// SysTick 置挂起（写 1 挂起；读回挂起状态）
    pub const PENDSTSET: u32 = 1 << 26;

    /// SysTick clear-pending (write 1 to clear; reads as zero)
    /// SysTick 清挂起（写 1 清除；读为 0）
    pub const PENDSTCLR: u32 = 1 << 25;

    /// At least one external interrupt line is pending
    /// 至少有一条外部中断线挂起
    pub const ISRPENDING: u32 = 1 << 22;

    /// Shift of the `VECTPENDING` exception-number field
    /// `VECTPENDING` 异常号位段的移位
    pub const VECTPENDING_SHIFT: u32 = 12;

    /// Mask of an exception-number field (`VECTACTIVE` and `VECTPENDING`)
    /// 异常号位段的掩码（`VECTACTIVE` 与 `VECTPENDING`）
    pub const EXCEPTION_NUMBER_MASK: u32 = 0x1FF;

    /// No preempted active exception is waiting (`RETTOBASE`)
    /// 没有等待返回的被抢占活跃异常（`RETTOBASE`）
    pub const RETTOBASE: u32 = 1 << 11;
}

/// Bytes of `SHPR3` that this model stores (PendSV byte 2, SysTick byte 3)
/// 本模型保存的 `SHPR3` 字节（PendSV 第 2 字节、SysTick 第 3 字节）
///
/// Only the implemented priority bits are writable; the reserved low bits of
/// each byte read as zero. / 只有实现的优先级位可写，每个字节的保留低位读 0。
const SHPR3_MODELED: u32 = 0xE0E0_0000;

/// Bytes of `SHPR2` that this model stores (SVCall byte 3)
/// 本模型保存的 `SHPR2` 字节（SVCall 第 3 字节）
const SHPR2_MODELED: u32 = 0xE000_0000;

/// Implemented priority bits inside one priority byte
/// 单个优先级字节中实现的优先级位
const PRIORITY_BYTE_MASK: u32 = 0xE0;

/// System Control Block exception state for NMI, SVCall, PendSV and SysTick
/// NMI、SVCall、PendSV 与 SysTick 的系统控制块异常状态
///
/// Holds exactly the state that belongs to the system exceptions: their pending
/// latches and the `SHPR2`/`SHPR3` priority bytes. Active state stays in the
/// core's exception stack because it is shared with every other exception.
/// 只保存系统异常自己的状态：挂起锁存与 `SHPR2`/`SHPR3` 优先级字节。活跃状态
/// 由核心的异常栈保存，因为它与所有其它异常共享。
#[derive(Debug, Clone)]
pub struct Scb {
    /// `ICSR.NMIPENDSET` latch / `ICSR.NMIPENDSET` 锁存
    nmi_pending: bool,
    /// `SHCSR.SVCALLPENDED` latch / `SHCSR.SVCALLPENDED` 锁存
    svcall_pending: bool,
    /// `ICSR.PENDSVSET` latch / `ICSR.PENDSVSET` 锁存
    pendsv_pending: bool,
    /// `ICSR.PENDSTSET` latch / `ICSR.PENDSTSET` 锁存
    systick_pending: bool,
    /// `SHPR2` (SVCall priority in byte 3) / `SHPR2`（第 3 字节为 SVCall 优先级）
    shpr2: u32,
    /// `SHPR3` (PendSV in byte 2, SysTick in byte 3)
    /// `SHPR3`（第 2 字节 PendSV、第 3 字节 SysTick）
    shpr3: u32,
}

impl Scb {
    /// Create the block in reset state (nothing pending, priorities zero)
    /// 创建复位状态的系统控制块（无挂起，优先级为 0）
    #[must_use]
    pub fn new() -> Self {
        Self {
            nmi_pending: false,
            svcall_pending: false,
            pendsv_pending: false,
            systick_pending: false,
            shpr2: 0,
            shpr3: 0,
        }
    }

    /// Whether NMI is pending / NMI 是否挂起
    #[must_use]
    pub fn nmi_pending(&self) -> bool {
        self.nmi_pending
    }

    /// Whether SVCall is pending / SVCall 是否挂起
    #[must_use]
    pub fn svcall_pending(&self) -> bool {
        self.svcall_pending
    }

    /// Set the SVCall pending latch, returning `true` on a 0-to-1 transition
    /// 置位 SVCall 挂起锁存，0→1 跳变时返回 `true`
    pub fn set_svcall_pending(&mut self) -> bool {
        !std::mem::replace(&mut self.svcall_pending, true)
    }

    /// Whether PendSV is pending / PendSV 是否挂起
    #[must_use]
    pub fn pendsv_pending(&self) -> bool {
        self.pendsv_pending
    }

    /// Whether SysTick is pending / SysTick 是否挂起
    #[must_use]
    pub fn systick_pending(&self) -> bool {
        self.systick_pending
    }

    /// Whether any modelled system exception is pending
    /// 是否有任何已建模的系统异常挂起
    #[must_use]
    pub fn has_pending(&self) -> bool {
        self.nmi_pending || self.svcall_pending || self.pendsv_pending || self.systick_pending
    }

    /// Whether `exception_number` is pending / `exception_number` 是否挂起
    ///
    /// Only NMI (2), SVCall (11), PendSV (14) and SysTick (15) have state here;
    /// every other number reads as not pending. External IRQs live in
    /// [`crate::Nvic`].
    /// 这里只保存 NMI（2）、SVCall（11）、PendSV（14）与 SysTick（15）的状态；
    /// 其它编号一律读为未挂起。外部中断位于 [`crate::Nvic`]。
    #[must_use]
    pub fn is_pending(&self, exception_number: u16) -> bool {
        match exception_number {
            crate::exceptions::EXC_NMI => self.nmi_pending,
            crate::exceptions::EXC_SVCALL => self.svcall_pending,
            crate::exceptions::EXC_PENDSV => self.pendsv_pending,
            crate::exceptions::EXC_SYSTICK => self.systick_pending,
            _ => false,
        }
    }

    /// Set `exception_number` pending, returning `true` on a 0-to-1 transition
    /// 置位 `exception_number` 挂起，0→1 跳变时返回 `true`
    ///
    /// Other exception numbers are ignored, so a caller that mixes external and
    /// system numbers must check [`Self::is_pending`] first.
    /// 其它异常号会被忽略，因此混合外部号与系统号的调用方必须先检查
    /// [`Self::is_pending`]。
    pub fn set_pending(&mut self, exception_number: u16) -> bool {
        match exception_number {
            crate::exceptions::EXC_NMI => !std::mem::replace(&mut self.nmi_pending, true),
            crate::exceptions::EXC_SVCALL => self.set_svcall_pending(),
            crate::exceptions::EXC_PENDSV => !std::mem::replace(&mut self.pendsv_pending, true),
            crate::exceptions::EXC_SYSTICK => !std::mem::replace(&mut self.systick_pending, true),
            _ => false,
        }
    }

    /// Clear `exception_number` pending / 清除 `exception_number` 挂起
    pub fn clear_pending(&mut self, exception_number: u16) {
        match exception_number {
            crate::exceptions::EXC_NMI => self.nmi_pending = false,
            crate::exceptions::EXC_SVCALL => self.svcall_pending = false,
            crate::exceptions::EXC_PENDSV => self.pendsv_pending = false,
            crate::exceptions::EXC_SYSTICK => self.systick_pending = false,
            _ => {}
        }
    }

    /// Decode one priority byte into the implemented priority width
    /// 把一个优先级字节解码为实现宽度的优先级值
    fn decode_priority(byte: u32) -> u8 {
        ((byte & PRIORITY_BYTE_MASK) >> (8 - PRIORITY_BITS)) as u8
    }

    /// Priority of SVCall (`SHPR2[31:24]`) / SVCall 优先级（`SHPR2[31:24]`）
    #[must_use]
    pub fn svcall_priority(&self) -> u8 {
        Self::decode_priority((self.shpr2 >> 24) & 0xFF)
    }

    /// Priority of PendSV (`SHPR3[23:16]`) / PendSV 优先级（`SHPR3[23:16]`）
    #[must_use]
    pub fn pendsv_priority(&self) -> u8 {
        Self::decode_priority((self.shpr3 >> 16) & 0xFF)
    }

    /// Priority of SysTick (`SHPR3[31:24]`) / SysTick 优先级（`SHPR3[31:24]`）
    #[must_use]
    pub fn systick_priority(&self) -> u8 {
        Self::decode_priority((self.shpr3 >> 24) & 0xFF)
    }

    /// Read `SHPR2` / 读 `SHPR2`
    ///
    /// Byte 3 is the modelled SVCall priority; the reserved bytes read zero.
    /// 第 3 字节为已建模的 SVCall 优先级，保留字节读 0。
    #[must_use]
    pub fn read_shpr2(&self) -> u32 {
        self.shpr2
    }

    /// Write `SHPR2` / 写 `SHPR2`
    pub fn write_shpr2(&mut self, value: u32) {
        self.shpr2 = value & SHPR2_MODELED;
    }

    /// Read `SHPR3` / 读 `SHPR3`
    ///
    /// Byte 2 is PendSV and byte 3 is SysTick; unmodeled low bits read zero.
    /// 第 2 字节为 PendSV、第 3 字节为 SysTick；未建模的低位读 0。
    #[must_use]
    pub fn read_shpr3(&self) -> u32 {
        self.shpr3
    }

    /// Write `SHPR3` / 写 `SHPR3`
    pub fn write_shpr3(&mut self, value: u32) {
        self.shpr3 = value & SHPR3_MODELED;
    }
}

impl Default for Scb {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_state_has_no_pending_and_zero_priorities() {
        let scb = Scb::new();
        assert!(!scb.has_pending());
        assert!(!scb.pendsv_pending());
        assert!(!scb.systick_pending());
        assert_eq!(scb.pendsv_priority(), 0);
        assert_eq!(scb.systick_priority(), 0);
        assert_eq!(scb.read_shpr3(), 0);
    }

    #[test]
    fn pending_latches_are_separate_and_report_transitions() {
        let mut scb = Scb::new();
        assert!(scb.set_pending(crate::exceptions::EXC_PENDSV));
        assert!(!scb.set_pending(crate::exceptions::EXC_PENDSV));
        assert!(scb.pendsv_pending());
        assert!(!scb.systick_pending());

        assert!(scb.set_pending(crate::exceptions::EXC_SYSTICK));
        scb.clear_pending(crate::exceptions::EXC_PENDSV);
        assert!(!scb.pendsv_pending());
        assert!(scb.systick_pending());
        assert!(scb.has_pending());

        // 外部异常号（含外部 IRQ 15 对应的异常 31）不属于本块。
        // External exception numbers (including exception 31 for IRQ 15) are
        // not part of this block.
        assert!(!scb.is_pending(31));
        assert!(!scb.set_pending(31));
        assert!(!scb.is_pending(31));
    }

    #[test]
    fn shpr3_bytes_decode_pendsv_and_systick_priorities() {
        let mut scb = Scb::new();
        // PendSV byte = 0xC0 (priority 6), SysTick byte = 0x40 (priority 2).
        // PendSV 字节 0xC0（优先级 6），SysTick 字节 0x40（优先级 2）。
        scb.write_shpr3(0x40C0_0000);
        assert_eq!(scb.read_shpr3(), 0x40C0_0000);
        assert_eq!(scb.pendsv_priority(), 6);
        assert_eq!(scb.systick_priority(), 2);

        // 未建模的低位必须读 0（RAZ/WI）。
        // Unmodeled low bits read as zero (RAZ/WI).
        scb.write_shpr3(0xFFFF_FFFF);
        assert_eq!(scb.read_shpr3(), 0xE0E0_0000);
        assert_eq!(scb.pendsv_priority(), 7);
        assert_eq!(scb.systick_priority(), 7);
    }

    #[test]
    fn shpr2_models_the_svcall_priority_byte() {
        let mut scb = Scb::new();
        scb.write_shpr2(0xFFFF_FFFF);
        assert_eq!(scb.read_shpr2(), 0xE000_0000);
        assert_eq!(scb.svcall_priority(), 7);
        scb.write_shpr2(0x4000_0000);
        assert_eq!(scb.svcall_priority(), 2);
    }

    #[test]
    fn nmi_and_svcall_latches_are_independent_system_exceptions() {
        let mut scb = Scb::new();
        assert!(!scb.nmi_pending());
        assert!(!scb.svcall_pending());

        assert!(scb.set_pending(crate::exceptions::EXC_NMI));
        assert!(!scb.set_pending(crate::exceptions::EXC_NMI));
        assert!(scb.nmi_pending());
        assert!(scb.has_pending());

        assert!(scb.set_svcall_pending());
        assert!(!scb.set_svcall_pending());
        assert!(scb.svcall_pending());
        assert!(!scb.pendsv_pending());
        assert!(!scb.systick_pending());

        // 取走异常清除自己的锁存，绝不影响其它异常。
        // Taking one exception clears its own latch only.
        scb.clear_pending(crate::exceptions::EXC_NMI);
        assert!(!scb.nmi_pending());
        assert!(scb.svcall_pending());
        scb.clear_pending(crate::exceptions::EXC_SVCALL);
        assert!(!scb.has_pending());
    }
}
