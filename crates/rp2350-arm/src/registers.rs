//! Cortex-M33 register file (R0-R15, MSP/PSP, XPSR, CONTROL, PRIMASK, BASEPRI, FAULTMASK)
//! Cortex-M33 寄存器文件（R0-R15、MSP/PSP、XPSR、CONTROL、PRIMASK、BASEPRI、FAULTMASK）

use crate::fpu::{FpuRegisters, cpacr, fpexc};

/// CONTROL.FPCA — floating-point context active / CONTROL.FPCA —— 浮点上下文活跃
///
/// The model keeps this bit in [`CortexM33Registers::control`]. Hardware raises
/// it whenever a floating-point instruction executes with CPACR access granted
/// (see `CortexM33Core::prepare_fpu_instruction`), software can read and write
/// it through `MRS`/`MSR CONTROL`, and an exception return restores it from the
/// returned frame type: `EXC_RETURN` bit 4 = 1 (basic frame) clears it, bit 4 =
/// 0 (extended frame) sets it. `FPCCR.CLRONRET`/`CLRONRETS` additionally clear
/// it when a basic frame is returned. See
/// [`CortexM33Registers::fp_context_active`] for the frame-selection predicate.
/// 本模型把该位保存在 [`CortexM33Registers::control`] 中。只要有浮点指令通过
/// CPACR 检查，硬件就置位它（见 `CortexM33Core::prepare_fpu_instruction`）；
/// 软件可通过 `MRS`/`MSR CONTROL` 读写；异常返回按返回帧类型恢复：`EXC_RETURN`
/// bit4 = 1（基础帧）清除，bit4 = 0（扩展帧）置位。返回基础帧时
/// `FPCCR.CLRONRET`/`CLRONRETS` 还会再次清除它。帧选择判据见
/// [`CortexM33Registers::fp_context_active`]。
pub const CONTROL_FPCA: u32 = 1 << 2;

/// ARMv8-M program status register (XPSR) flags
/// ARMv8-M 程序状态寄存器标志
pub mod psr {
    // bitflags! generates structs without doc comments -- suppress missing_docs lint
    // bitflags! 生成的 struct 不含文档注释 -- 抑制 missing_docs lint
    #![allow(missing_docs)]
    use bitflags::bitflags;

    bitflags! {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct ApsrFlags: u32 {
            /// Negative flag / 负数标志
            const N = 1 << 31;
            /// Zero flag / 零标志
            const Z = 1 << 30;
            /// Carry flag / 进位标志
            const C = 1 << 29;
            /// Overflow flag / 溢出标志
            const V = 1 << 28;
            /// Saturating cumulative flag (Q) / 饱和累积标志
            const Q = 1 << 27;
        }
    }
}

/// Complete Cortex-M33 register set
/// 完整的 Cortex-M33 寄存器集
#[derive(Debug, Clone)]
pub struct CortexM33Registers {
    /// General-purpose registers R0-R12 / 通用寄存器 R0-R12
    pub r: [u32; 13],
    /// Stack Pointer (current: MSP or PSP) / 栈指针（当前：MSP 或 PSP）
    pub sp: u32,
    /// Link Register (R14) / 链接寄存器
    pub lr: u32,
    /// Program Counter (R15) / 程序计数器
    pub pc: u32,
    /// Main Stack Pointer / 主栈指针
    pub msp: u32,
    /// Process Stack Pointer / 进程栈指针
    pub psp: u32,
    /// Program Status Register (APSR + IPSR + EPSR) / 程序状态寄存器
    pub xpsr: u32,
    /// CONTROL register / 控制寄存器
    pub control: u32,
    /// Priority Mask Register / 优先级屏蔽寄存器
    pub primask: u32,
    /// Base Priority Mask Register / 基础优先级屏蔽寄存器
    pub basepri: u32,
    /// Fault Mask Register / 故障屏蔽寄存器
    pub faultmask: u32,
    /// Main Stack Pointer limit / 主栈指针下限
    pub msplim: u32,
    /// Process Stack Pointer limit / 进程栈指针下限
    pub psplim: u32,
    /// Floating-point registers S0-S31, FPSCR and FPEXC
    /// 浮点寄存器 S0-S31、FPSCR 与 FPEXC
    pub fpu: FpuRegisters,
    /// Coprocessor Access Control Register (CPACR) / 协处理器访问控制寄存器
    pub cpacr: u32,
    /// Exclusive monitor armed by this core's last exclusive load, as
    /// `(address, size in bytes)`
    /// 本核最近一次独占加载武装的独占监视器，形式为 `(地址, 字节尺寸)`
    ///
    /// The monitor belongs to the core, not to the host thread: both RP2350
    /// cores are stepped on one thread, so shared state would let core0's
    /// `LDREX` satisfy core1's `STREX`. See [`Self::arm_exclusive`].
    /// 监视器属于核心而非宿主线程：RP2350 的两个核心在同一线程中推进，若共享
    /// 状态，core0 的 `LDREX` 就会让 core1 的 `STREX` 误判成功。参见
    /// [`Self::arm_exclusive`]。
    pub(crate) exclusive: Option<(u32, u8)>,
    /// A synchronous UsageFault is pending from the instruction just executed
    /// 刚执行的指令已引发待处理的同步用法故障
    pub(crate) pending_usage_fault: bool,
}

impl CortexM33Registers {
    /// Create a new register set in power-on reset state
    /// 创建上电复位状态的寄存器集
    #[must_use]
    pub fn new() -> Self {
        Self {
            r: [0; 13],
            sp: 0,
            lr: 0xFFFF_FFFF,
            pc: 0,
            msp: 0,
            psp: 0,
            xpsr: 0x0100_0000, // Thumb state bit set
            control: 0,
            primask: 0,
            basepri: 0,
            faultmask: 0,
            msplim: 0,
            psplim: 0,
            fpu: FpuRegisters::new(),
            cpacr: 0,
            exclusive: None,
            pending_usage_fault: false,
        }
    }

    /// Read a general-purpose register by index (0-15)
    /// 按索引读取通用寄存器（0-15）
    #[must_use]
    pub fn read(&self, index: u8) -> u32 {
        match index {
            0..=12 => self.r[index as usize],
            13 => self.sp,
            14 => self.lr,
            15 => self.pc,
            _ => 0,
        }
    }

    /// Write a general-purpose register by index (0-15)
    /// 按索引写入通用寄存器（0-15）
    pub fn write(&mut self, index: u8, value: u32) {
        match index {
            0..=12 => self.r[index as usize] = value,
            13 => {
                self.sp = value;
                self.sync_stack_bank();
            }
            14 => self.lr = value,
            15 => self.pc = value,
            _ => {}
        }
    }

    /// Mirror `SP` into the active bank (`MSP` or `PSP`)
    /// 把 `SP` 同步回当前栈寄存器组（`MSP` 或 `PSP`）
    ///
    /// On real Cortex-M hardware `R13` *is* the selected bank register, not a
    /// separate value. Instructions such as `push`/`pop`/`add sp` used to update
    /// only `sp`, so exception return unstacked from a stale `msp` and restored
    /// a bogus PC/SP (the real `MicroPython` firmware ended up reading at the
    /// SRAM top, `0x2008_2000`).
    /// 真实 Cortex-M 中 `R13` 就是被选中的栈寄存器，并非独立数值。此前
    /// `push`/`pop`/`add sp` 只更新 `sp`，异常返回便从过时的 `msp` 出栈，
    /// 恢复出错误的 PC/SP（真实 MicroPython 固件最终读到 SRAM 顶端
    /// `0x2008_2000` 而越界）。
    pub fn sync_stack_bank(&mut self) {
        if self.control & 0x2 != 0 {
            self.psp = self.sp;
        } else {
            self.msp = self.sp;
        }
    }

    /// Get the current stack pointer (MSP or PSP based on CONTROL.SPSEL)
    /// 获取当前栈指针（根据 CONTROL.SPSEL 选择 MSP 或 PSP）
    #[must_use]
    pub fn current_sp(&self) -> u32 {
        if self.control & 0x2 != 0 {
            self.psp
        } else {
            self.msp
        }
    }

    /// Report whether CPACR grants access to the FPv5-SP unit (CP10)
    /// 报告 CPACR 是否授予 FPv5-SP 单元（CP10）访问权限
    ///
    /// FPv5-SP access is governed by `CPACR.CP10`. `0b01` grants privileged-only
    /// access and `0b11` grants full access; the emulator always executes in
    /// privileged mode, so both encodings allow FPU instructions. `0b00` and
    /// the reserved `0b10` encoding raise NOCP.
    ///
    /// `CPACR.CP11` is not consulted: it gates double-precision arithmetic,
    /// which FPv5-SP does not implement. Real RP2350 firmware (MicroPython)
    /// leaves CP11 at `0b00` while running VFP instructions, so requiring it
    /// here would reject every FPU access on the target.
    /// FPv5-SP 的访问权限由 `CPACR.CP10` 决定。`0b01` 为仅特权访问、`0b11`
    /// 为完全访问；模拟器始终以特权模式执行，因此两种编码都允许 FPU 指令。
    /// `0b00` 与保留值 `0b10` 产生 NOCP。
    ///
    /// 此处不检查 `CPACR.CP11`：它控制双精度算术，而 FPv5-SP 不实现双精度。
    /// 真实 RP2350 固件（MicroPython）在运行 VFP 指令时把 CP11 保持为
    /// `0b00`，若这里要求 CP11，目标上的所有浮点访问都会被拒绝。
    #[must_use]
    pub fn fp_access_enabled(&self) -> bool {
        self.cp_access_enabled(10)
    }

    /// Report whether a live floating-point context exists
    /// 报告是否存在活跃的浮点上下文
    ///
    /// ARMv8-M selects the extended exception frame from `CONTROL.FPCA`. This
    /// model tracks `FPCA` (raised by every executed FPU instruction after the
    /// CPACR check, software-writable through `MSR CONTROL`, restored from the
    /// frame type on exception return, cleared by `FPCCR.CLRONRET`) **and**
    /// keeps `FPEXC.EN` as a fallback: the bit is set by the same instructions
    /// but is never cleared by a software `MSR CONTROL`, so a context can never
    /// be silently lost. The disjunction is therefore a superset of the
    /// architectural predicate and leaves the pre-existing frame-selection
    /// behaviour untouched while making `FPCA` observable through `MRS CONTROL`.
    /// ARMv8-M 用 `CONTROL.FPCA` 决定是否使用扩展异常帧。本模型跟踪 `FPCA`
    /// （每条通过 CPACR 检查的浮点指令置位，软件可经 `MSR CONTROL` 写入，异常返回
    /// 按帧类型恢复，`FPCCR.CLRONRET` 清除），同时保留 `FPEXC.EN` 作为回退：
    /// 同一条指令会置该位，且软件 `MSR CONTROL` 不会清除它，因此上下文绝不会被
    /// 静默丢弃。该逻辑或是架构判据的超集，既保持既有帧选择行为不变，又让 `FPCA`
    /// 可通过 `MRS CONTROL` 观察。
    #[must_use]
    pub fn fp_context_active(&self) -> bool {
        self.control & CONTROL_FPCA != 0 || self.fpu.fpexc & fpexc::EN != 0
    }

    /// Report whether CPACR grants access to coprocessor `cp`
    /// 报告 CPACR 是否授予协处理器 `cp` 访问权限
    ///
    /// Each coprocessor has a two-bit CPACR field at bits `2*cp+1:2*cp`.
    /// As with [`Self::fp_access_enabled`], `0b01` (privileged-only) and
    /// `0b11` (full) are accepted because the emulator always executes in
    /// privileged mode; `0b00` and the reserved `0b10` raise NOCP.
    /// 每个协处理器在 CPACR 的 `2*cp+1:2*cp` 位段有一个两位字段。与
    /// [`Self::fp_access_enabled`] 一致，`0b01`（仅特权）与 `0b11`（完全）
    /// 都视为允许，因为模拟器始终以特权模式执行；`0b00` 与保留值 `0b10`
    /// 会产生 NOCP。
    #[must_use]
    pub fn cp_access_enabled(&self, cp: u8) -> bool {
        if cp >= 16 {
            return false;
        }
        let shift = u32::from(cp) * 2;
        let field = (self.cpacr >> shift) & 0x3;
        matches!(field, cpacr::PRIVILEGED | cpacr::FULL)
    }

    /// Mark a synchronous UsageFault as pending for the current instruction
    /// 将当前指令标记为引发待处理的同步用法故障
    pub(crate) fn raise_usage_fault(&mut self) {
        self.pending_usage_fault = true;
    }

    /// Take and clear the pending synchronous UsageFault flag
    /// 取出并清除待处理的同步用法故障标志
    pub(crate) fn take_usage_fault(&mut self) -> bool {
        core::mem::take(&mut self.pending_usage_fault)
    }

    /// Arm this core's exclusive monitor for `addr` with byte size `size`
    /// 以字节尺寸 `size` 为本核武装地址 `addr` 的独占监视器
    ///
    /// Every core owns its monitor, so an `LDREX` executed by core0 can never
    /// be consumed by a `STREX` on core1: a cross-core pair must fail, as the
    /// architecture requires. Arming overwrites any monitor still pending,
    /// matching the single-monitor-per-core model (`LDREX` is not additive).
    /// 每个核心拥有自己的监视器，因此 core0 的 `LDREX` 绝不可能被 core1 的
    /// `STREX` 消费：跨核配对必须失败，与架构要求一致。武装会覆盖仍然待处理的
    /// 监视器，符合「每核单一监视器」模型（`LDREX` 不会累加）。
    pub(crate) fn arm_exclusive(&mut self, addr: u32, size: u8) {
        self.exclusive = Some((addr, size));
    }

    /// Consume the exclusive monitor, reporting whether address and size matched
    /// 消费独占监视器，返回地址与尺寸是否匹配
    ///
    /// The monitor is retired whether or not it matched, so a failed `STREX`
    /// also discards the pending `LDREX`; the caller must still store only on
    /// `true`.
    /// 无论是否匹配都会废弃监视器，因此失败的 `STREX` 同样丢弃未完成的
    /// `LDREX`；调用方只能在返回 `true` 时真正写入内存。
    #[must_use]
    pub(crate) fn consume_exclusive(&mut self, addr: u32, size: u8) -> bool {
        let hit = self.exclusive == Some((addr, size));
        self.exclusive = None;
        hit
    }

    /// Clear this core's exclusive monitor (`CLREX` semantics)
    /// 清除本核的独占监视器（`CLREX` 语义）
    ///
    /// Clearing an unarmed monitor is a no-op, so `CLREX` succeeds either way.
    /// 清除未武装的监视器是无操作，因此 `CLREX` 在两种情况下都成功。
    pub(crate) fn clear_exclusive(&mut self) {
        self.exclusive = None;
    }
}

impl Default for CortexM33Registers {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_write_gpr() {
        let mut regs = CortexM33Registers::new();
        regs.write(0, 0xDEAD_BEEF);
        assert_eq!(regs.read(0), 0xDEAD_BEEF);
    }

    #[test]
    fn sp_is_r13() {
        let mut regs = CortexM33Registers::new();
        regs.write(13, 0x2000_1000);
        assert_eq!(regs.sp, 0x2000_1000);
    }

    #[test]
    fn default_stack_is_msp() {
        let mut regs = CortexM33Registers::new();
        regs.msp = 0x2000_4000;
        regs.psp = 0x2000_8000;
        assert_eq!(regs.current_sp(), 0x2000_4000);
    }

    #[test]
    fn fpu_access_is_disabled_at_reset() {
        let regs = CortexM33Registers::new();
        assert!(!regs.fp_access_enabled());
    }

    #[test]
    fn fpu_access_accepts_full_and_privileged_encodings() {
        let mut regs = CortexM33Registers::new();
        regs.cpacr = 0x00F0_0000;
        assert!(regs.fp_access_enabled());
        regs.cpacr = 0x0050_0000;
        assert!(regs.fp_access_enabled());
    }

    #[test]
    fn fpu_access_ignores_cp11_because_fpv5_sp_is_governed_by_cp10() {
        let mut regs = CortexM33Registers::new();
        // 实测 MicroPython RP2350 固件写入 CPACR = 0x00300303（CP10 全开、CP11 为 0），
        // 且其 VFP 指令在真实硬件上可执行。
        // Observed MicroPython RP2350 firmware writes CPACR = 0x00300303
        // (CP10 full, CP11 denied) and still executes VFP instructions.
        regs.cpacr = 0x0030_0000;
        assert!(regs.fp_access_enabled());
    }

    #[test]
    fn fpu_access_rejects_denied_and_reserved_cp10_encodings() {
        let mut regs = CortexM33Registers::new();
        // CP10 = denied / CP10 禁止
        regs.cpacr = 0x0000_0000;
        assert!(!regs.fp_access_enabled());
        // CP10 = reserved 0b10 / CP10 保留值 0b10
        regs.cpacr = 0x0020_0000;
        assert!(!regs.fp_access_enabled());
    }

    #[test]
    fn cp_access_reads_each_two_bit_field() {
        let mut regs = CortexM33Registers::new();
        regs.cpacr = 0x0030_0303;
        assert!(regs.cp_access_enabled(0));
        assert!(regs.cp_access_enabled(4));
        assert!(regs.cp_access_enabled(10));
        assert!(!regs.cp_access_enabled(1));
        assert!(!regs.cp_access_enabled(7));
        assert!(!regs.cp_access_enabled(16));
    }

    #[test]
    fn cp_access_accepts_privileged_only_and_rejects_reserved() {
        let mut regs = CortexM33Registers::new();
        regs.cpacr = 0b01 << 2; // CP1 privileged-only / CP1 仅特权
        assert!(regs.cp_access_enabled(1));
        regs.cpacr = 0b10 << 2; // CP1 reserved / CP1 保留值
        assert!(!regs.cp_access_enabled(1));
    }
}
