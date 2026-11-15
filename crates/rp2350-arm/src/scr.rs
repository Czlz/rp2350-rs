//! System Control Register (`SCR`) sleep-control bits
//! 系统控制寄存器（`SCR`）的睡眠控制位
//!
//! `SCR` lives in the SCB at `0xE000_ED10`. Only the three bits that change
//! ARMv8-M sleep behaviour are modeled; every reserved bit reads as zero.
//! `SCR` 位于 SCB 的 `0xE000_ED10`。本模型只保存会改变 ARMv8-M 睡眠行为的三个位，
//! 其余保留位读为 0。
//!
//! Reference: ARM ARM B3.2.7 (`SCR`) and B1.4.2 (sleep/deep sleep events).
//! 参考：ARM ARM B3.2.7（`SCR`）与 B1.4.2（sleep/deep sleep 事件）。

// `SCR` 的位定义沿用 ARM 参考手册的缩写命名。
// The `SCR` bit definitions follow the ARM reference manual acronyms.
#![allow(clippy::doc_markdown)]

/// Absolute address of `SCB_SCR` / `SCB_SCR` 的绝对地址
pub const SCR: u32 = 0xE000_ED10;

/// `SLEEPONEXIT`: enter sleep on exception return to Thread mode
/// `SLEEPONEXIT`：异常返回到 Thread 模式时进入睡眠
///
/// The sleep starts after the exception-return sequence has restored `PC` and
/// the stack pointer, so the instruction at the restored `PC` is never executed
/// before the core suspends. Handler-to-Handler returns do not trigger it.
/// 睡眠发生在异常返回序列恢复 `PC` 与栈指针之后，因此恢复出的 `PC` 处指令绝不会在
/// 挂起前执行。Handler→Handler 返回不会触发。
pub const SLEEPONEXIT: u32 = 1 << 1;

/// `SLEEPDEEP`: select deep sleep instead of sleep
/// `SLEEPDEEP`：选择深度睡眠而非普通睡眠
///
/// This model has no separately powered domain, so the bit only changes the
/// reported [`crate::SleepReason`]; wake-up behaviour is unchanged.
/// 本模型没有独立掉电域，因此该位只改变上报的 [`crate::SleepReason`]，唤醒行为不变。
pub const SLEEPDEEP: u32 = 1 << 2;

/// `SEVONPEND`: an exception becoming pending sends an event
/// `SEVONPEND`：异常变为挂起时发送事件
///
/// The event is generated on the 0-to-1 transition of any exception pending
/// bit, whether the exception is enabled or masked. It is the ARMv8-M event
/// source that lets `WFE` wake from an interrupt that `PRIMASK` keeps masked.
/// 任一异常的挂起位由 0 变 1 时产生事件，无论该异常是否使能或被屏蔽。这正是
/// ARMv8-M 中让 `WFE` 被 `PRIMASK` 屏蔽的中断唤醒的事件源。
pub const SEVONPEND: u32 = 1 << 4;

/// Bits stored by the core; every other bit reads as zero
/// 核心保存的位；其余位读为 0
pub const MODELED_MASK: u32 = SLEEPONEXIT | SLEEPDEEP | SEVONPEND;
