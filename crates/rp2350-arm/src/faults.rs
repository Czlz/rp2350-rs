//! Fault status, escalation, and `AIRCR` control (ARMv8-M)
//! 故障状态、故障升级与 `AIRCR` 控制（ARMv8-M）
//!
//! ARMv8-M splits fault reporting across three registers: `SHCSR` holds the
//! fault enable bits plus active/pending views, `CFSR` holds the synchronous
//! fault reason (MMFSR/BFSR/UFSR), and `HFSR` records a forced escalation to
//! HardFault. This module owns the state that is not already carried by the
//! register file, implements the escalation rule — a synchronous fault whose
//! handler is disabled, or that cannot be stacked, becomes HardFault — and
//! models `AIRCR` (`VECTKEY`, `PRIS`, `SYSRESETREQ`, `VECTRESET`).
//! ARMv8-M 把故障上报拆成三个寄存器：`SHCSR` 保存故障使能位与活跃/挂起视图，
//! `CFSR` 保存同步故障原因（MMFSR/BFSR/UFSR），`HFSR` 记录升级为 HardFault 的
//! 强迫位。本模块保存寄存器文件之外的故障状态，实现升级规则——处理程序未使能或
//! 无法压栈的同步故障升级为 HardFault——并建模 `AIRCR`（`VECTKEY`、`PRIS`、
//! `SYSRESETREQ`、`VECTRESET`）。

// 故障状态直接映射 ARM 位段与异常号，位运算与截断是硬件仿真的固有写法。
// Fault state maps ARM bit fields and exception numbers directly; bit
// operations and truncation are inherent to hardware emulation.
#![allow(
    clippy::cast_possible_truncation,
    clippy::doc_markdown,
    clippy::module_name_repetitions
)]

use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;

use crate::CortexM33Core;
use crate::exceptions::{
    self, EXC_BUSFAULT, EXC_HARDFAULT, EXC_MEMMANAGE, EXC_NMI, EXC_PENDSV, EXC_SVCALL, EXC_SYSTICK,
    EXC_USAGE_FAULT, EXCEPTION_STACK_FRAME_SIZE, EXCEPTION_STACK_FRAME_WITH_FP_SIZE,
};
use crate::execute_thumb16::StepResult;

/// Fixed priority rank of NMI (highest, reset excluded)
/// NMI 的固定优先级等级（最高，不含复位）
pub(crate) const RANK_NMI: u8 = 0;

/// Fixed priority rank of HardFault / HardFault 的固定优先级等级
pub(crate) const RANK_HARDFAULT: u8 = 1;

/// Rank of configurable priority 0; higher priorities add to this base
/// 可配置优先级 0 的等级；更高优先级数值在此基线上相加
pub(crate) const RANK_CONFIGURABLE_BASE: u8 = 2;

/// `SHCSR` bit definitions (ARMv8-M B3.2.11) / `SHCSR` 位定义
pub mod shcsr {
    /// MemManage handler active (bit 0) / MemManage 处理程序活跃（位 0）
    pub const MEMFAULTACT: u32 = 1 << 0;
    /// BusFault handler active (bit 1) / BusFault 处理程序活跃（位 1）
    pub const BUSFAULTACT: u32 = 1 << 1;
    /// UsageFault handler active (bit 3) / UsageFault 处理程序活跃（位 3）
    pub const USGFAULTACT: u32 = 1 << 3;
    /// NMI handler active (bit 5) / NMI 处理程序活跃（位 5）
    pub const NMIACT: u32 = 1 << 5;
    /// SVCall handler active (bit 7) / SVCall 处理程序活跃（位 7）
    pub const SVCALLACT: u32 = 1 << 7;
    /// PendSV handler active (bit 10) / PendSV 处理程序活跃（位 10）
    pub const PENDSVACT: u32 = 1 << 10;
    /// SysTick handler active (bit 11) / SysTick 处理程序活跃（位 11）
    pub const SYSTICKACT: u32 = 1 << 11;
    /// SVCall pending (bit 15, read-only) / SVCall 挂起（位 15，只读）
    pub const SVCALLPENDED: u32 = 1 << 15;
    /// MemManage handler enabled (bit 16) / MemManage 处理程序使能（位 16）
    pub const MEMFAULTENA: u32 = 1 << 16;
    /// BusFault handler enabled (bit 17) / BusFault 处理程序使能（位 17）
    pub const BUSFAULTENA: u32 = 1 << 17;
    /// UsageFault handler enabled (bit 18) / UsageFault 处理程序使能（位 18）
    pub const USGFAULTENA: u32 = 1 << 18;
    /// HardFault pending (bit 23, read-only; always 0 in this model)
    /// HardFault 挂起（位 23，只读；本模型恒为 0）
    pub const HARDFAULTPENDED: u32 = 1 << 23;
    /// HardFault handler active (bit 25) / HardFault 处理程序活跃（位 25）
    pub const HARDFAULTACT: u32 = 1 << 25;

    /// Bits software may change: only the three fault enables
    /// 软件可写的位：仅三个故障使能
    pub const WRITABLE_MASK: u32 = MEMFAULTENA | BUSFAULTENA | USGFAULTENA;
}

/// `CFSR` bit definitions (ARMv8-M B3.2.12) / `CFSR` 位定义
pub mod cfsr {
    /// MemManage: instruction access violation (bit 0)
    /// MemManage：取指访问违例（位 0）
    pub const IACCVIOL: u32 = 1 << 0;
    /// MemManage: data access violation (bit 1)
    /// MemManage：数据访问违例（位 1）
    pub const DACCVIOL: u32 = 1 << 1;
    /// BusFault: instruction bus error (bit 8)
    /// BusFault：取指总线错误（位 8）
    pub const IBUSERR: u32 = 1 << 8;
    /// BusFault: precise data bus error (bit 9)
    /// BusFault：精确数据总线错误（位 9）
    pub const PRECISERR: u32 = 1 << 9;
    /// BusFault: imprecise data bus error (bit 10)
    /// BusFault：非精确数据总线错误（位 10）
    pub const IMPRECISERR: u32 = 1 << 10;
    /// BusFault: unstacking error (bit 11) / BusFault：出栈错误（位 11）
    pub const UNSTKERR: u32 = 1 << 11;
    /// BusFault: stacking error (bit 12) / BusFault：压栈错误（位 12）
    pub const STKERR: u32 = 1 << 12;
    /// UsageFault: undefined instruction (bit 16)
    /// UsageFault：未定义指令（位 16）
    pub const UNDEFINSTR: u32 = 1 << 16;
    /// UsageFault: invalid state (bit 17) / UsageFault：非法状态（位 17）
    pub const INVSTATE: u32 = 1 << 17;
    /// UsageFault: invalid PC load (bit 18) / UsageFault：非法 PC 载入（位 18）
    pub const INVPC: u32 = 1 << 18;
    /// UsageFault: coprocessor access denied (bit 19)
    /// UsageFault：协处理器访问被拒绝（位 19）
    pub const NOCP: u32 = 1 << 19;
    /// UsageFault: stack overflow (bit 20) / UsageFault：栈溢出（位 20）
    pub const STKOF: u32 = 1 << 20;
    /// UsageFault: unaligned access when `CCR.UNALIGN_TRP` is set (bit 21)
    /// UsageFault：`CCR.UNALIGN_TRP` 置位时的非对齐访问（位 21）
    pub const UNALIGNED: u32 = 1 << 21;
    /// UsageFault: divide by zero when `CCR.DIV_0_TRP` is set (bit 22)
    /// UsageFault：`CCR.DIV_0_TRP` 置位时的除零（位 22）
    pub const DIVBYZERO: u32 = 1 << 22;
}

/// `HFSR` bit definitions (ARMv8-M B3.2.13) / `HFSR` 位定义
pub mod hfsr {
    /// Vector table read returned a bus error (bit 1)
    /// 向量表读取产生总线错误（位 1）
    pub const VECTTBL: u32 = 1 << 1;
    /// A configurable fault was escalated to HardFault (bit 30)
    /// 可配置故障被升级为 HardFault（位 30）
    pub const FORCED: u32 = 1 << 30;
    /// Debug event (BKPT) caused the HardFault (bit 31)
    /// 调试事件（BKPT）导致 HardFault（位 31）
    pub const DEBUGEVT: u32 = 1 << 31;
}

/// `AIRCR` bit definitions (ARMv8-M B3.2.7) / `AIRCR` 位定义
pub mod aircr {
    /// Value that must be written to `VECTKEY` for a write to be accepted
    /// 写入 `VECTKEY` 才能被接受的值
    pub const VECTKEY: u32 = 0x05FA_0000;

    /// Value read back from `VECTKEYSTAT` / 从 `VECTKEYSTAT` 读回的值
    pub const VECTKEYSTAT: u32 = 0xFA05_0000;

    /// Prioritize Secure exceptions (bit 14, storage only in this model)
    /// 优先安全异常（位 14，本模型仅存储）
    pub const PRIS: u32 = 1 << 14;

    /// Request a system reset (bit 2, write-only)
    /// 请求系统复位（位 2，只写）
    pub const SYSRESETREQ: u32 = 1 << 2;

    /// Request a processor reset (bit 0, write-only)
    /// 请求处理器复位（位 0，只写）
    pub const VECTRESET: u32 = 1 << 0;

    /// Bits stored by [`super::FaultState`] (the write-only requests are latched
    /// separately) / [`super::FaultState`] 保存的位（只写请求单独锁存）
    pub const WRITABLE_MASK: u32 = PRIS;
}

/// Fault system registers that are not part of the register file
/// 不属于寄存器文件的故障系统寄存器
///
/// `shcsr` stores only the writable enable bits; the active and pending views
/// are composed by [`CortexM33Core::shcsr`] from the live exception stack.
/// `shcsr` 只保存可写使能位；活跃与挂起视图由 [`CortexM33Core::shcsr`] 从实时
/// 异常栈组合。
#[derive(Debug, Clone)]
pub struct FaultState {
    /// Writable `SHCSR` enable bits / 可写的 `SHCSR` 使能位
    shcsr: u32,
    /// `HFSR` status bits (write-one-to-clear) / `HFSR` 状态位（写 1 清零）
    hfsr: u32,
    /// Stored `AIRCR` bits (`PRIS` only) / 保存的 `AIRCR` 位（仅 `PRIS`）
    aircr: u32,
    /// Latched `AIRCR.SYSRESETREQ` / 锁存的 `AIRCR.SYSRESETREQ`
    system_reset_requested: bool,
    /// Latched `AIRCR.VECTRESET` (diagnostic) / 锁存的 `AIRCR.VECTRESET`（诊断）
    vect_reset_requested: bool,
}

impl FaultState {
    /// Create the block in reset state (all fault handlers disabled)
    /// 创建复位状态的故障块（所有故障处理程序关闭）
    #[must_use]
    pub fn new() -> Self {
        Self {
            shcsr: 0,
            hfsr: 0,
            aircr: 0,
            system_reset_requested: false,
            vect_reset_requested: false,
        }
    }

    /// Stored `SHCSR` enable bits / 保存的 `SHCSR` 使能位
    #[must_use]
    pub(crate) fn shcsr_bits(&self) -> u32 {
        self.shcsr
    }

    /// Whether any of `mask` is enabled in `SHCSR` / `SHCSR` 中是否使能了 `mask`
    #[must_use]
    pub fn enabled(&self, mask: u32) -> bool {
        self.shcsr & mask != 0
    }

    /// Whether the handler for `exception_number` is enabled
    /// `exception_number` 的处理程序是否已使能
    #[must_use]
    pub fn fault_handler_enabled(&self, exception_number: u16) -> bool {
        match exception_number {
            EXC_MEMMANAGE => self.enabled(shcsr::MEMFAULTENA),
            EXC_BUSFAULT => self.enabled(shcsr::BUSFAULTENA),
            EXC_USAGE_FAULT => self.enabled(shcsr::USGFAULTENA),
            _ => true,
        }
    }

    /// Write the writable `SHCSR` bits / 写入 `SHCSR` 的可写位
    ///
    /// Active and pending bits are read-only views, so a write cannot change
    /// them; only the three fault enables are stored.
    /// 活跃与挂起位是只读视图，写入无法改变它们；只有三个故障使能会被保存。
    pub fn write_shcsr(&mut self, value: u32) {
        self.shcsr = value & shcsr::WRITABLE_MASK;
    }

    /// Raise `HFSR` bits / 置位 `HFSR` 位
    pub fn set_hfsr(&mut self, bits: u32) {
        self.hfsr |= bits;
    }

    /// Read `HFSR` / 读 `HFSR`
    #[must_use]
    pub fn read_hfsr(&self) -> u32 {
        self.hfsr
    }

    /// Clear `HFSR` bits (write-one-to-clear) / 清除 `HFSR` 位（写 1 清零）
    pub fn write_hfsr(&mut self, value: u32) {
        self.hfsr &= !value;
    }

    /// Compose the `AIRCR` read value / 组合 `AIRCR` 读值
    ///
    /// `VECTKEYSTAT` always reads as `0xFA05`, `PRIS` reads back the stored bit,
    /// and the write-only reset requests read as zero.
    /// `VECTKEYSTAT` 恒读 `0xFA05`，`PRIS` 读回保存值，只写的复位请求读 0。
    #[must_use]
    pub fn read_aircr(&self) -> u32 {
        aircr::VECTKEYSTAT | self.aircr
    }

    /// Apply an `AIRCR` write, returning `false` when `VECTKEY` was missing
    /// 应用一次 `AIRCR` 写入；缺少 `VECTKEY` 时返回 `false`
    ///
    /// ARMv8-M requires `bits[31:16]` to be `0x05FA`; every other value is
    /// ignored except for the `VECTKEYSTAT` read side. `SYSRESETREQ` and
    /// `VECTRESET` are write-only latches consumed by the host through
    /// [`CortexM33Core::take_system_reset_request`].
    /// ARMv8-M 要求 `bits[31:16]` 为 `0x05FA`，其它值除读侧的 `VECTKEYSTAT` 外
    /// 整次忽略。`SYSRESETREQ` 与 `VECTRESET` 是只写锁存，由宿主通过
    /// [`CortexM33Core::take_system_reset_request`] 取走。
    pub fn write_aircr(&mut self, value: u32) -> bool {
        if value & 0xFFFF_0000 != aircr::VECTKEY {
            return false;
        }
        self.aircr = value & aircr::WRITABLE_MASK;
        if value & aircr::SYSRESETREQ != 0 {
            self.system_reset_requested = true;
        }
        if value & aircr::VECTRESET != 0 {
            self.vect_reset_requested = true;
            // 本模型不实现处理器复位；记录请求并报告宿主，避免静默忽略。
            // This model does not implement a processor reset; latch the
            // request and report it to the host instead of ignoring it.
            log::warn!(
                "AIRCR.VECTRESET requested; latched as a diagnostic only / \
                 收到 AIRCR.VECTRESET 请求；仅作为诊断锁存"
            );
        }
        true
    }

    /// Take the latched `SYSRESETREQ` flag / 取走锁存的 `SYSRESETREQ` 标志
    pub fn take_system_reset_request(&mut self) -> bool {
        std::mem::take(&mut self.system_reset_requested)
    }

    /// Whether `AIRCR.VECTRESET` was requested / 是否请求过 `AIRCR.VECTRESET`
    #[must_use]
    pub fn vect_reset_requested(&self) -> bool {
        self.vect_reset_requested
    }
}

impl Default for FaultState {
    fn default() -> Self {
        Self::new()
    }
}

impl CortexM33Core {
    /// Compose the `SHCSR` read value / 组合 `SHCSR` 读值
    ///
    /// The enable bits come from [`FaultState`]; every `*ACT` bit is derived
    /// from the live exception stack and `SVCALLPENDED` from the SCB latch, so
    /// software always sees the same state that arbitration uses.
    /// 使能位来自 [`FaultState`]；每个 `*ACT` 位由实时异常栈推出，`SVCALLPENDED`
    /// 来自 SCB 锁存，因此软件看到的状态与仲裁使用的状态始终一致。
    #[must_use]
    pub fn shcsr(&self) -> u32 {
        let mut value = self.faults.shcsr_bits();
        let active = |number: u16| self.active_exceptions.contains(&number);
        if active(EXC_MEMMANAGE) {
            value |= shcsr::MEMFAULTACT;
        }
        if active(EXC_BUSFAULT) {
            value |= shcsr::BUSFAULTACT;
        }
        if active(EXC_USAGE_FAULT) {
            value |= shcsr::USGFAULTACT;
        }
        if active(EXC_NMI) {
            value |= shcsr::NMIACT;
        }
        if active(EXC_SVCALL) {
            value |= shcsr::SVCALLACT;
        }
        if active(EXC_PENDSV) {
            value |= shcsr::PENDSVACT;
        }
        if active(EXC_SYSTICK) {
            value |= shcsr::SYSTICKACT;
        }
        if active(EXC_HARDFAULT) {
            value |= shcsr::HARDFAULTACT;
        }
        if self.scb.svcall_pending() {
            value |= shcsr::SVCALLPENDED;
        }
        value
    }

    /// Write the writable `SHCSR` bits / 写入 `SHCSR` 的可写位
    pub(crate) fn write_shcsr(&mut self, value: u32) {
        self.faults.write_shcsr(value);
    }

    /// Read `HFSR` / 读 `HFSR`
    #[must_use]
    pub fn hfsr(&self) -> u32 {
        self.faults.read_hfsr()
    }

    /// Clear `HFSR` bits (write-one-to-clear) / 清除 `HFSR` 位（写 1 清零）
    pub(crate) fn write_hfsr(&mut self, value: u32) {
        self.faults.write_hfsr(value);
    }

    /// Read `AIRCR` / 读 `AIRCR`
    #[must_use]
    pub fn aircr(&self) -> u32 {
        self.faults.read_aircr()
    }

    /// Apply an `AIRCR` write / 应用一次 `AIRCR` 写入
    ///
    /// Returns whether the write carried `VECTKEY = 0x05FA`. A write without
    /// the key changes nothing, exactly as the architecture requires.
    /// 返回写入是否携带 `VECTKEY = 0x05FA`。没有 key 的写入不改变任何状态，
    /// 与架构要求一致。
    pub fn write_aircr(&mut self, value: u32) -> bool {
        let accepted = self.faults.write_aircr(value);
        if !accepted {
            log::warn!(
                "AIRCR write {value:#010X} ignored: VECTKEY != 0x05FA / \
                 AIRCR 写入 {value:#010X} 被忽略：VECTKEY != 0x05FA"
            );
        }
        accepted
    }

    /// Take a latched `AIRCR.SYSRESETREQ` system-reset request
    /// 取走锁存的 `AIRCR.SYSRESETREQ` 系统复位请求
    ///
    /// The core never resets itself: a frontend decides how to honour the
    /// request (re-boot the machine, report it, or ignore it). The latch is
    /// one-shot, so repeated polling returns `false` until firmware requests
    /// another reset.
    /// 核心不会自行复位：如何响应该请求由前端决定（重启机器、上报或忽略）。锁存
    /// 为一次性，因此重复查询会返回 `false`，直到固件再次请求复位。
    pub fn take_system_reset_request(&mut self) -> bool {
        self.faults.take_system_reset_request()
    }

    /// Whether `AIRCR.VECTRESET` was ever requested (diagnostic)
    /// 是否曾经请求过 `AIRCR.VECTRESET`（诊断）
    #[must_use]
    pub fn vect_reset_requested(&self) -> bool {
        self.faults.vect_reset_requested()
    }

    /// Enter one exception through the stack-limit and stacking-error wrapper
    /// 通过栈上限与压栈错误包装进入一个异常
    ///
    /// A frame that would cross `MSPLIM`/`PSPLIM` sets `CFSR.STKOF`, a bus error
    /// while stacking sets `CFSR.STKERR`, and both escalate to HardFault with
    /// `HFSR.FORCED`. The escalated entry deliberately skips the limit check so
    /// the handler is still reachable; real hardware would lock up when the
    /// second frame cannot be written either, which this model reports as a
    /// propagated bus error instead.
    /// 若栈帧将越过 `MSPLIM`/`PSPLIM` 则置位 `CFSR.STKOF`；压栈期间的总线错误置位
    /// `CFSR.STKERR`；两者都以 `HFSR.FORCED` 升级为 HardFault。升级后的进入刻意跳过
    /// 上限检查，使处理程序仍可到达；真实硬件在第二个栈帧也无法写入时会进入 lockup，
    /// 本模型改为传播总线错误。
    ///
    /// Returns the exception number that was actually entered (3 on escalation).
    /// 返回实际进入的异常号（升级时为 3）。
    ///
    /// # Errors
    /// Propagates a bus error when even the HardFault frame cannot be written.
    /// 连 HardFault 栈帧都无法写入时传播总线错误。
    pub(crate) fn enter_exception_checked(
        &mut self,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
        exception_number: u16,
    ) -> Result<u16, CoreError> {
        if exception_number == EXC_HARDFAULT {
            self.enter_exception_raw(bus, cycles, exception_number)?;
            return Ok(exception_number);
        }
        if self.stack_frame_overflows_limit() {
            self.cfsr |= cfsr::STKOF;
            self.faults.set_hfsr(hfsr::FORCED);
            log::warn!(
                "exception {exception_number} escalated to HardFault: stack limit \
                 overflow (CFSR.STKOF) / 异常 {exception_number} 升级为 HardFault：\
                 栈上限溢出（CFSR.STKOF）"
            );
            self.enter_exception_raw(bus, cycles, EXC_HARDFAULT)?;
            return Ok(EXC_HARDFAULT);
        }
        match self.enter_exception_raw(bus, cycles, exception_number) {
            Ok(()) => Ok(exception_number),
            Err(error) => {
                self.cfsr |= cfsr::STKERR;
                self.faults.set_hfsr(hfsr::FORCED);
                log::warn!(
                    "exception {exception_number} escalated to HardFault: stacking \
                     error (CFSR.STKERR) / 异常 {exception_number} 升级为 HardFault：\
                     压栈错误（CFSR.STKERR）"
                );
                self.enter_exception_raw(bus, cycles, EXC_HARDFAULT)
                    .map_err(|_| error)?;
                Ok(EXC_HARDFAULT)
            }
        }
    }

    /// Raise a synchronous fault and enter its handler
    /// 触发同步故障并进入其处理程序
    ///
    /// `cfsr_bits` is recorded before the target is chosen, so HardFault still
    /// shows the original cause next to `HFSR.FORCED`. The specific handler is
    /// used only when it is enabled **and** the current execution priority
    /// allows it; a disabled or masked MemManage/BusFault/UsageFault handler
    /// therefore escalates to HardFault (ARMv8-M B1.5.4), which no mask can
    /// block. The chosen exception is entered with `frame_pc` as the stacked
    /// return address, which is the faulting instruction for
    /// undefined/`INVSTATE` faults.
    /// 先记录 `cfsr_bits`，因此升级后 HardFault 仍能看到原始原因加上
    /// `HFSR.FORCED`。只有在处理程序已使能**且**当前执行优先级允许时才进入该处理
    /// 程序；未使能或被屏蔽的 MemManage/BusFault/UsageFault 因此升级为 HardFault
    /// （ARMv8-M B1.5.4），而任何屏蔽都无法阻止 HardFault。进入时以 `frame_pc`
    /// 作为压栈返回地址——对未定义/`INVSTATE` 故障即故障指令本身。
    ///
    /// Returns the exception number that was actually entered.
    /// 返回实际进入的异常号。
    ///
    /// # Errors
    /// Propagates a bus error from stacking or the vector fetch.
    /// 传播压栈或向量读取产生的总线错误。
    pub(crate) fn raise_sync_fault(
        &mut self,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
        fault: u16,
        cfsr_bits: u32,
        frame_pc: u32,
    ) -> Result<u16, CoreError> {
        self.cfsr |= cfsr_bits;
        let rank = self.exception_priority(fault);
        let target = if self.faults.fault_handler_enabled(fault)
            && self.exception_entry_allowed(fault, rank)
        {
            fault
        } else {
            self.faults.set_hfsr(hfsr::FORCED);
            EXC_HARDFAULT
        };
        // 同步故障的压栈返回地址是故障指令本身（SVC 除外，它使用后继地址）。
        // The stacked return address of a synchronous fault is the faulting
        // instruction itself (SVC is the exception: it stacks the successor).
        self.registers.pc = frame_pc;
        let entered = self.enter_exception_checked(bus, cycles, target)?;
        if entered != target {
            self.faults.set_hfsr(hfsr::FORCED);
        }
        Ok(entered)
    }

    /// Whether the next exception frame would cross the active stack limit
    /// 下一个异常栈帧是否会越过当前栈上限
    fn stack_frame_overflows_limit(&self) -> bool {
        let limit = if self.registers.control & 0x2 != 0 {
            self.registers.psplim
        } else {
            self.registers.msplim
        };
        let sp = self.registers.current_sp();
        let frame = if self.registers.fp_context_active() {
            EXCEPTION_STACK_FRAME_WITH_FP_SIZE
        } else {
            EXCEPTION_STACK_FRAME_SIZE
        };
        sp < limit || sp - limit < frame
    }

    /// Enter one exception without limit/stack-error escalation
    /// 进入一个异常，不做栈上限/压栈错误升级
    fn enter_exception_raw(
        &mut self,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
        exception_number: u16,
    ) -> Result<(), CoreError> {
        let vtor = self.vtor;
        exceptions::enter_exception(
            bus,
            &mut self.registers,
            &mut self.nvic,
            &mut self.scb,
            u32::from(exception_number),
            vtor,
            &mut self.it_state,
            cycles,
        )
    }
}

impl CortexM33Core {
    /// Report an undefined instruction, deduplicated by opcode
    /// 按操作码去重地报告未定义指令
    ///
    /// The warning is diagnostic only; the fault itself is raised by
    /// [`Self::finish_instruction`]. Strict mode returns before this helper.
    /// 该告警仅用于诊断；故障本身由 [`Self::finish_instruction`] 触发。严格模式
    /// 会在调用本辅助函数前返回。
    fn note_undefined(&mut self, pc: u32, opcode: u32, width: u8) {
        if self.undefined_seen.insert((width, opcode)) {
            log::warn!(
                "undefined instruction at PC {pc:#010X}: opcode {opcode:#010X} \
                 ({width} bytes), entering HardFault / 未定义指令 PC {pc:#010X}: \
                 opcode {opcode:#010X}（{width} 字节），进入 HardFault"
            );
        }
    }

    /// Shared tail: synchronous fault entry, SVCall pending and diagnostics
    /// 共享尾部：同步故障进入、SVCall 挂起与诊断
    ///
    /// Every synchronous fault funnels through here so the strict-decode host
    /// option and the architectural path cannot diverge: `NOCP` and illegal
    /// `EXC_RETURN` values are raised by the instruction executors, undefined
    /// encodings arrive as [`StepResult::Undefined`], and `BKPT` raises
    /// `HFSR.DEBUGEVT`. A disabled or masked UsageFault handler escalates to
    /// HardFault inside [`Self::raise_sync_fault`].
    /// 所有同步故障都汇聚到这里，严格模式宿主选项与架构路径不会分叉：`NOCP` 与
    /// 非法 `EXC_RETURN` 由指令执行器触发，未定义编码以 [`StepResult::Undefined`]
    /// 到达，`BKPT` 置位 `HFSR.DEBUGEVT`。未使能或被屏蔽的 UsageFault 处理程序在
    /// [`Self::raise_sync_fault`] 内升级为 HardFault。
    #[inline]
    pub(crate) fn finish_instruction(
        &mut self,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
        old_pc: u32,
        raw_opcode: u32,
        width: u8,
        result: StepResult,
    ) -> Result<StepResult, CoreError> {
        // 同步用法故障（NOCP）：记录 CFSR.UFSR.NOCP，再由 SHCSR.USGFAULTENA
        // 决定进入 UsageFault（6）还是升级为 HardFault（3）。
        // Synchronous UsageFault (NOCP): record CFSR.UFSR.NOCP; SHCSR.USGFAULTENA
        // decides between UsageFault (6) and escalated HardFault (3).
        if self.registers.take_usage_fault() {
            let entered =
                self.raise_sync_fault(bus, cycles, EXC_USAGE_FAULT, cfsr::NOCP, old_pc)?;
            self.active_exceptions.push(entered);
            return Ok(StepResult::Ok);
        }

        // 非法 EXC_RETURN（写 PC 时值带 0xFFFF_FF 前缀但保留位非法）：
        // CFSR.UFSR.INVSTATE，同样按 SHCSR 使能位选择 UsageFault 或 HardFault。
        // Illegal EXC_RETURN (a PC write with the 0xFFFF_FF prefix but reserved
        // bits set): CFSR.UFSR.INVSTATE, routed by the same SHCSR enable bit.
        if self.registers.take_invstate_fault() {
            let entered =
                self.raise_sync_fault(bus, cycles, EXC_USAGE_FAULT, cfsr::INVSTATE, old_pc)?;
            self.active_exceptions.push(entered);
            return Ok(StepResult::Ok);
        }

        match result {
            StepResult::Undefined => {
                if self.strict_decode {
                    return Err(CoreError::UndefinedInstruction {
                        pc: old_pc,
                        opcode: raw_opcode,
                        width,
                    });
                }
                self.note_undefined(old_pc, raw_opcode, width);
                let entered =
                    self.raise_sync_fault(bus, cycles, EXC_USAGE_FAULT, cfsr::UNDEFINSTR, old_pc)?;
                self.active_exceptions.push(entered);
                Ok(StepResult::Ok)
            }
            // SVC 是同步异常：指令退役后立刻把异常 11 置为挂起（`SVCALLPENDED`），
            // 本步末尾的统一仲裁按 SHPR2 优先级与 BASEPRI 立即投递；优先级不允许时
            // 保持挂起，等异常返回或解除屏蔽后再取。
            // SVC is synchronous: on retirement it pends exception 11
            // (`SVCALLPENDED`) and the shared arbitration at the end of the step
            // delivers it immediately, subject to SHPR2 priority and BASEPRI.
            // When the priority gate refuses, it stays pending until a return or
            // an unmask makes it takeable.
            StepResult::SvCall(_immediate) => {
                self.pend_svcall();
                Ok(result)
            }
            // 无调试器时 BKPT 产生 HFSR.DEBUGEVT 的 HardFault（ARMv8-M
            // B1.5.4）；压栈返回地址取 BKPT 指令本身。
            // Without a debugger, BKPT raises HardFault with HFSR.DEBUGEVT
            // (ARMv8-M B1.5.4); the stacked return address is the BKPT itself.
            StepResult::Breakpoint(_immediate) => {
                self.faults.set_hfsr(hfsr::DEBUGEVT);
                self.registers.pc = old_pc;
                let entered = self.enter_exception_checked(bus, cycles, EXC_HARDFAULT)?;
                self.active_exceptions.push(entered);
                Ok(StepResult::Ok)
            }
            other => Ok(other),
        }
    }
}
