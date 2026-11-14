//! Lazy floating-point stacking and extended exception frame tests
//! 惰性浮点压栈与扩展异常帧测试
//!
//! Covers `FPCCR`/`FPCAR`/`FPDSCR`, standard vs extended frames, the deferred
//! save triggered by the handler's first VFP instruction and the two exception
//! return paths (reclaim only / restore context).
//! 覆盖 `FPCCR`/`FPCAR`/`FPDSCR`、标准帧与扩展帧、处理程序第一条 VFP 指令触发
//! 的延迟保存，以及两种异常返回路径（仅回收 / 恢复上下文）。

use super::*;

use crate::conditions::ITBlockState;
use crate::fpu::{fpccr, fpexc, fpscr, sys_reg};
use crate::nvic::Nvic;
use crate::registers::CortexM33Registers;

/// Initial MSP used by the fixture / 测试夹具使用的初始 MSP
const MSP_TOP: u32 = 0x2000_1000;

/// Exception number of IRQ 0 / IRQ 0 的异常号
const EXC_IRQ0: u32 = 16;

/// Direct-exception fixture operating on registers and a bus
/// 直接在寄存器与总线上操作异常的测试夹具
struct FpFixture {
    bus: rp2350_core::memory::SimpleBus,
    regs: CortexM33Registers,
    nvic: Nvic,
    it: ITBlockState,
    cycles: u64,
}

impl FpFixture {
    /// Fixture with a model-reset FPU (no live FP context)
    /// 浮点单元处于模型复位状态（无活跃浮点上下文）的夹具
    fn new() -> Self {
        let mut bus = fpu_test_bus(0x0000_0101);
        let mut cycles = 0u64;
        // Vector for exception 16 (IRQ 0) points at 0x0000_0180.
        // 异常 16（IRQ 0）的向量指向 0x0000_0180。
        bus.write_u32(0x40, 0x0000_0181, &mut cycles).unwrap();
        let mut regs = CortexM33Registers::new();
        regs.msp = MSP_TOP;
        regs.sp = MSP_TOP;
        regs.pc = 0x0000_0100;
        Self {
            bus,
            regs,
            nvic: Nvic::new(),
            it: ITBlockState::new(),
            cycles,
        }
    }

    /// Fixture with a live FP context (`FPEXC.EN` set, S0 = 1.0)
    /// 带活跃浮点上下文的夹具（`FPEXC.EN` 置位，S0 = 1.0）
    fn with_live_fp() -> Self {
        let mut fixture = Self::new();
        fixture.regs.fpu.fpexc |= fpexc::EN;
        fixture.regs.fpu.write_s(0, 0x3F80_0000);
        fixture
    }

    /// Enter exception 16 with MSP as the interrupted stack
    /// 以 MSP 作为被打断的栈进入异常 16
    fn enter(&mut self) {
        exceptions::enter_exception(
            &mut self.bus,
            &mut self.regs,
            &mut self.nvic,
            EXC_IRQ0,
            0,
            &mut self.it,
            &mut self.cycles,
        )
        .unwrap();
    }

    /// Return from exception 16 using the current LR
    /// 用当前 LR 从异常 16 返回
    fn exit(&mut self) {
        let info = exceptions::decode_exc_return(self.regs.lr).unwrap();
        exceptions::exit_exception(
            &mut self.bus,
            &mut self.regs,
            &mut self.nvic,
            &info,
            EXC_IRQ0,
            &mut self.it,
            &mut self.cycles,
        )
        .unwrap();
    }

    /// Read a word from the fixture bus / 从夹具总线读取一个字
    fn read(&mut self, addr: u32) -> u32 {
        self.bus.read_u32(addr, &mut self.cycles).unwrap()
    }
}

#[test]
fn fpccr_reset_value_and_writable_bits() {
    let mut core = CortexM33Core::new(CpuId::Core0);
    // Cortex-M33 reset value: ASPEN=1, LSPEN=1, LSPACT=0.
    // Cortex-M33 复位值：ASPEN=1、LSPEN=1、LSPACT=0。
    assert_eq!(core.system_read(sys_reg::FPCCR), Some(0xC000_0000));

    // Only ASPEN and LSPEN are writable; LSPACT and reserved bits read 0.
    // 只有 ASPEN 与 LSPEN 可写；LSPACT 与保留位读 0。
    core.system_write(sys_reg::FPCCR, 0xFFFF_FFFF);
    assert_eq!(core.system_read(sys_reg::FPCCR), Some(0xC000_0000));
    core.system_write(sys_reg::FPCCR, 0);
    assert_eq!(core.system_read(sys_reg::FPCCR), Some(0));
    core.system_write(sys_reg::FPCCR, 1 << 31);
    assert_eq!(core.system_read(sys_reg::FPCCR), Some(0x8000_0000));
}

#[test]
fn fpcar_forces_low_three_bits_to_zero() {
    let mut core = CortexM33Core::new(CpuId::Core0);
    assert_eq!(core.system_read(sys_reg::FPCAR), Some(0));
    core.system_write(sys_reg::FPCAR, 0x2000_0FFF);
    assert_eq!(core.system_read(sys_reg::FPCAR), Some(0x2000_0FF8));
}

#[test]
fn fpdscr_keeps_only_rounding_fields() {
    let mut core = CortexM33Core::new(CpuId::Core0);
    assert_eq!(core.system_read(sys_reg::FPDSCR), Some(0));
    core.system_write(sys_reg::FPDSCR, 0xFFFF_FFFF);
    // AHP/DN/FZ (bits 26:24) and RMode (bits 23:22) only.
    // 仅 AHP/DN/FZ（bits 26:24）与 RMode（bits 23:22）。
    assert_eq!(core.system_read(sys_reg::FPDSCR), Some(0x07C0_0000));
}

#[test]
fn inactive_fpu_keeps_standard_frame() {
    let mut fixture = FpFixture::new();
    fixture.enter();

    // No FP context: 8-word frame, no reservation, standard EXC_RETURN.
    // 无浮点上下文：8 字帧、无预留、标准 EXC_RETURN。
    assert_eq!(fixture.regs.msp, MSP_TOP - 0x20);
    assert_eq!(fixture.regs.lr, 0xFFFF_FFF9);
    assert!(!fixture.regs.fpu.lspact());
    assert_eq!(fixture.regs.fpu.fpcar, 0);
    assert_eq!(fixture.regs.fpu.fpccr, 0xC000_0000);
}

#[test]
fn lazy_entry_reserves_context_without_writing_registers() {
    let mut fixture = FpFixture::with_live_fp();
    fixture.regs.fpu.fpscr = 0x0000_0001;
    fixture.enter();

    let fpcar = MSP_TOP - 0x48;
    assert!(fixture.regs.fpu.lspact());
    // Extended frame: state frame plus 17-word context plus padding word.
    // 扩展帧：状态帧 + 17 字上下文 + 1 字填充。
    assert_eq!(fixture.regs.msp, MSP_TOP - 0x68);
    assert_eq!(fixture.regs.fpu.fpcar, fpcar);
    assert_eq!(fixture.regs.fpu.fpcar, fixture.regs.msp + 0x20);
    // EXC_RETURN bit 4 = 0 marks the extended frame.
    // EXC_RETURN bit 4 = 0 表示扩展帧。
    assert_eq!(fixture.regs.lr, 0xFFFF_FFE9);

    // The registers stay live and the reserved area is untouched.
    // 寄存器保持活跃值，预留区域未被写入。
    assert_eq!(fixture.regs.fpu.s[0], 0x3F80_0000);
    assert_eq!(fixture.read(fpcar), 0);
    assert_eq!(fixture.read(fpcar + 0x40), 0);
}

#[test]
fn first_fp_instruction_commits_lazy_context_once() {
    let mut bus = fpu_test_bus(0x0000_0101);
    let mut cycles = 0u64;
    bus.write_u32(0x40, 0x0000_0181, &mut cycles).unwrap();
    // Thread: VMOV S0, R0 ; NOP
    bus.write_u16(0x0100, 0xEE00, &mut cycles).unwrap();
    bus.write_u16(0x0102, 0x0A10, &mut cycles).unwrap();
    bus.write_u16(0x0104, 0xBF00, &mut cycles).unwrap();
    // Handler: VMOV S1, R2 ; BX LR
    bus.write_u16(0x0180, 0xEE00, &mut cycles).unwrap();
    bus.write_u16(0x0182, 0x2A90, &mut cycles).unwrap();
    bus.write_u16(0x0184, 0x4770, &mut cycles).unwrap();

    let mut core = CortexM33Core::new(CpuId::Core0);
    core.boot(&mut bus).unwrap();
    core.nvic.write_iser(0, 1);
    core.write_register(0, 0x3F80_0000); // R0 = 1.0
    core.write_register(2, 0x4000_0000); // R2 = 2.0
    core.signal_interrupt(0);

    // Thread VMOV executes, then the pending IRQ is taken.
    // 线程 VMOV 执行后，挂起的 IRQ 被响应。
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.registers.pc, 0x0000_0180);
    assert!(core.registers.fpu.lspact());
    let fpcar = core.registers.fpu.fpcar;
    assert_eq!(fpcar, MSP_TOP - 0x48);
    assert_eq!(core.registers.msp, MSP_TOP - 0x68);
    assert_eq!(core.registers.lr, 0xFFFF_FFE9);
    assert_eq!(
        bus.read_u32(fpcar, &mut cycles).unwrap(),
        0,
        "entry must not write S0-S15 / 进入时不得写入 S0-S15"
    );

    // The handler's first VFP instruction commits the deferred context.
    // 处理程序的第一条 VFP 指令提交延迟的上下文。
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert!(!core.registers.fpu.lspact());
    assert_eq!(bus.read_u32(fpcar, &mut cycles).unwrap(), 0x3F80_0000);
    assert_eq!(bus.read_u32(fpcar + 0x40, &mut cycles).unwrap(), 0);
    assert_eq!(core.registers.fpu.s[1], 0x4000_0000);

    // Exception return restores the caller-saved FP registers.
    // 异常返回恢复调用者保存的浮点寄存器。
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.registers.msp, MSP_TOP);
    assert_eq!(core.registers.pc, 0x0000_0104);
    assert_eq!(core.registers.fpu.s[0], 0x3F80_0000);
    assert_eq!(core.registers.fpu.s[1], 0);
    assert!(!core.registers.fpu.lspact());
}

#[test]
fn lazy_return_without_handler_fp_use_reclaims_only() {
    let mut fixture = FpFixture::with_live_fp();
    fixture.regs.fpu.fpscr = 0x0000_0001;
    fixture.enter();
    let fpcar = fixture.regs.fpu.fpcar;

    // The handler does not touch the FPU; the live registers must survive.
    // 处理程序不碰浮点单元；活跃寄存器必须原样保留。
    fixture.exit();

    assert_eq!(fixture.regs.msp, MSP_TOP);
    assert_eq!(fixture.regs.sp, MSP_TOP);
    assert!(!fixture.regs.fpu.lspact());
    assert_eq!(fixture.regs.fpu.s[0], 0x3F80_0000);
    assert_eq!(fixture.regs.fpu.fpscr, 0x0000_0001);
    // Nothing was written to the discarded reservation.
    // 被丢弃的预留区未被写入。
    assert_eq!(fixture.read(fpcar), 0);
}

#[test]
fn lazy_return_restores_stacked_context() {
    let mut fixture = FpFixture::with_live_fp();
    fixture.regs.fpu.fpscr = 0x0000_0004;
    fixture.regs.fpu.write_s(15, 0x4120_0000);
    fixture.enter();
    let fpcar = fixture.regs.fpu.fpcar;

    // Simulate the handler using the FPU: commit, then clobber the registers.
    // 模拟处理程序使用浮点单元：提交后破坏寄存器内容。
    assert!(
        exceptions::commit_lazy_fp_context(
            &mut fixture.bus,
            &mut fixture.regs,
            &mut fixture.cycles
        )
        .unwrap()
    );
    assert_eq!(fixture.read(fpcar), 0x3F80_0000);
    assert_eq!(fixture.read(fpcar + 0x3C), 0x4120_0000);
    assert_eq!(fixture.read(fpcar + 0x40), 0x0000_0004);
    fixture.regs.fpu.write_s(0, 0xDEAD_BEEF);
    fixture.regs.fpu.write_s(15, 0xDEAD_BEEF);
    fixture.regs.fpu.fpscr = 0xFFFF_FFFF;

    fixture.exit();

    assert_eq!(fixture.regs.msp, MSP_TOP);
    assert_eq!(fixture.regs.fpu.s[0], 0x3F80_0000);
    assert_eq!(fixture.regs.fpu.s[15], 0x4120_0000);
    assert_eq!(fixture.regs.fpu.fpscr, 0x0000_0004);
    assert!(!fixture.regs.fpu.lspact());
}

#[test]
fn eager_entry_stacks_context_when_lspen_is_clear() {
    let mut fixture = FpFixture::with_live_fp();
    // ASPEN only: automatic (eager) state preservation.
    // 仅 ASPEN：自动（立即）状态保存。
    fixture.regs.fpu.write_fpccr(fpccr::ASPEN);
    fixture.regs.fpu.fpscr = 0x0000_0001;
    fixture.regs.fpu.write_s(15, 0x4120_0000);
    fixture.enter();

    let fpcar = MSP_TOP - 0x48;
    assert!(!fixture.regs.fpu.lspact(), "eager save clears LSPACT");
    assert_eq!(fixture.regs.fpu.fpcar, fpcar);
    assert_eq!(fixture.read(fpcar), 0x3F80_0000);
    assert_eq!(fixture.read(fpcar + 0x3C), 0x4120_0000);
    assert_eq!(fixture.read(fpcar + 0x40), 0x0000_0001);
    assert_eq!(fixture.regs.msp, MSP_TOP - 0x68);
    assert_eq!(fixture.regs.lr, 0xFFFF_FFE9);
}

#[test]
fn lspen_takes_precedence_over_aspen() {
    // The reset value has both ASPEN and LSPEN set; lazy preservation wins and
    // nothing is written at entry.
    // 复位值同时置 ASPEN 与 LSPEN；惰性保存优先，进入时不写任何内容。
    let mut fixture = FpFixture::with_live_fp();
    assert_eq!(
        fixture.regs.fpu.fpccr & (fpccr::ASPEN | fpccr::LSPEN),
        0xC000_0000
    );
    fixture.enter();
    assert!(fixture.regs.fpu.lspact());
    assert_eq!(fixture.read(fixture.regs.fpu.fpcar), 0);
}

#[test]
fn nested_entry_commits_pending_lazy_frame() {
    let mut fixture = FpFixture::with_live_fp();
    fixture.enter();
    let outer_fpcar = fixture.regs.fpu.fpcar;
    let outer_sp = fixture.regs.msp;

    // Nested entry while the outer lazy frame is still pending: the outer
    // context is committed first, then a new lazy frame is reserved.
    // 外层惰性帧仍待保存时再次进入异常：先提交外层上下文，再为新帧预留。
    fixture.enter();
    let inner_fpcar = fixture.regs.fpu.fpcar;

    assert_eq!(fixture.read(outer_fpcar), 0x3F80_0000);
    assert_eq!(inner_fpcar, outer_sp - 0x48);
    assert_eq!(fixture.regs.msp, outer_sp - 0x68);
    assert!(fixture.regs.fpu.lspact());
    assert_eq!(fixture.regs.fpu.fpcar, fixture.regs.msp + 0x20);

    // Returning from the inner frame reclaims only; the outer frame then
    // restores the committed context from its own FP area.
    // 内层帧返回仅回收空间；外层帧随后从自己的浮点区域恢复已提交的上下文。
    fixture.exit();
    assert_eq!(fixture.regs.msp, outer_sp);
    assert!(!fixture.regs.fpu.lspact());
    fixture.regs.fpu.write_s(0, 0xDEAD_BEEF);
    fixture.exit();
    assert_eq!(fixture.regs.msp, MSP_TOP);
    assert_eq!(fixture.regs.fpu.s[0], 0x3F80_0000);
}

#[test]
fn unaligned_sp_extended_frame_restores_exact_sp() {
    let mut fixture = FpFixture::with_live_fp();
    // 4 mod 8 SP: the frame is aligned down and a 4-byte aligner is recorded.
    // SP 为 4 mod 8：帧向下对齐，并记录 4 字节对齐字。
    fixture.regs.msp = 0x2000_0FF4;
    fixture.regs.sp = 0x2000_0FF4;
    fixture.enter();

    assert_eq!(fixture.regs.fpu.fpcar, 0x2000_0FA8);
    assert_eq!(fixture.regs.msp, 0x2000_0F88);

    // Repeated entries must not drift the stack pointer.
    // 反复进入不得使栈指针漂移。
    fixture.exit();
    assert_eq!(fixture.regs.msp, 0x2000_0FF4);
    fixture.enter();
    fixture.exit();
    assert_eq!(fixture.regs.msp, 0x2000_0FF4);
}

#[test]
fn fpdscr_does_not_change_arithmetic_rounding() {
    let mut bus = fpu_test_bus(0x0000_0101);
    let mut cycles = 0u64;
    // VDIV.F32 S0, S1, S2
    bus.write_u16(0x0100, 0xEE80, &mut cycles).unwrap();
    bus.write_u16(0x0102, 0x0A81, &mut cycles).unwrap();

    let mut core = CortexM33Core::new(CpuId::Core0);
    core.boot(&mut bus).unwrap();
    core.registers.fpu.write_s(1, 0x3F80_0000); // 1.0
    core.registers.fpu.write_s(2, 0x4040_0000); // 3.0
    // Ask for round-toward-zero; the core always rounds to nearest-even.
    // 写入向零舍入；核心始终按就近偶数舍入。
    core.system_write(sys_reg::FPDSCR, fpscr::RMODE_MASK);

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.registers.fpu.s[0], 0x3EAA_AAAB);
    assert_eq!(core.system_read(sys_reg::FPDSCR), Some(fpscr::RMODE_MASK));
    assert_eq!(core.registers.fpu.fpscr & fpscr::RMODE_MASK, 0);
}
