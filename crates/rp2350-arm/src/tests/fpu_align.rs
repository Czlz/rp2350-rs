//! FPU load/store alignment faults / 浮点加载/存储对齐故障
//!
//! ARMv8-M B2.3.6 lists non-word-aligned `VLDR`/`VSTR`/`VLDM`/`VSTM` among the
//! data accesses that **always** generate an alignment fault, independently of
//! `CCR.UNALIGN_TRP`; only plain `LDR`/`LDRH`/`STR`/`STRH` are gated by the
//! trap bit. This module pins that rule at the core level: `UFSR.UNALIGNED`
//! routing, the untouched register/memory/base state, the 4-byte (not 8-byte)
//! requirement of the 64-bit transfers, and the interaction with a committed
//! lazy FP context.
//!
//! ARMv8-M B2.3.6 把未按字对齐的 `VLDR`/`VSTR`/`VLDM`/`VSTM` 列为**始终**
//! 产生对齐故障的数据访问，与 `CCR.UNALIGN_TRP` 无关；只有普通
//! `LDR`/`LDRH`/`STR`/`STRH` 受该陷阱位控制。本模块在核心层锁定该规则：
//! `UFSR.UNALIGNED` 路由、寄存器/内存/基址保持不变、64 位传输只要求 4 字节
//! （而非 8 字节）对齐，以及与已提交惰性浮点上下文的交互。

use super::*;

use crate::faults::{ccr, cfsr, hfsr, shcsr};
use crate::registers::CONTROL_FPCA;
use rp2350_core::memory::SimpleBus;

/// Thread program base / 线程程序基址
const THREAD: u32 = 0x0000_0100;
/// IRQ 0 handler / IRQ 0 处理程序
const IRQ_HANDLER: u32 = 0x0000_0140;
/// HardFault handler / HardFault 处理程序
const HARDFAULT_HANDLER: u32 = 0x0000_0170;
/// UsageFault handler / UsageFault 处理程序
const USAGEFAULT_HANDLER: u32 = 0x0000_0190;
/// Initial MSP / 初始 MSP
const STACK_TOP: u32 = 0x2000_1000;

/// `SCB_CCR` / `SCB_CCR`
const SCB_CCR: u32 = 0xE000_ED14;
/// `SCB_SHCSR` / `SCB_SHCSR`
const SCB_SHCSR: u32 = 0xE000_ED24;

/// `BX LR` (`0x4770`) / `BX LR`
const BX_LR: u16 = 0x4770;

/// `VLDR S0, [R1]` / `VLDR S0, [R1]`
const VLDR_S0_R1: u32 = 0xED91_0A00;
/// `VSTR S0, [R1]` / `VSTR S0, [R1]`
const VSTR_S0_R1: u32 = 0xED81_0A00;
/// `VLDR S1, [R1]` / `VLDR S1, [R1]`
const VLDR_S1_R1: u32 = 0xEDD1_0A00;
/// `VLDR D0, [R1]` / `VLDR D0, [R1]`
const VLDR_D0_R1: u32 = 0xED91_0B00;
/// `VLDMIA R0!, {S0, S1}` / `VLDMIA R0!, {S0, S1}`
const VLDMIA_R0_S0_S1: u32 = 0xECB0_0A02;
/// `VSTMIA R0!, {S0, S1}` / `VSTMIA R0!, {S0, S1}`
const VSTMIA_R0_S0_S1: u32 = 0xECA0_0A02;
/// `VMOV S0, R0` / `VMOV S0, R0`
const VMOV_S0_R0: u32 = 0xEE00_0A10;

/// Write one Thumb-32 instruction in ARM word order
/// 按 ARM 指令字顺序写入一条 Thumb-32 指令
fn write_instr(bus: &mut SimpleBus, addr: u32, instr: u32) {
    let mut cycles = 0u64;
    bus.write_u16(addr, (instr >> 16) as u16, &mut cycles)
        .unwrap();
    bus.write_u16(addr + 2, instr as u16, &mut cycles).unwrap();
}

/// Build a core with HardFault/UsageFault/IRQ0 vectors and a thread program
/// 构建带 HardFault/UsageFault/IRQ0 向量与线程程序的核心
fn fixture_with_irq(program: &[u32], irq_program: &[u32]) -> (CortexM33Core, SimpleBus) {
    let mut bus = fpu_test_bus(THREAD + 1);
    let mut cycles = 0u64;
    bus.write_u32(0x0000_000C, HARDFAULT_HANDLER + 1, &mut cycles)
        .unwrap();
    bus.write_u32(0x0000_0018, USAGEFAULT_HANDLER + 1, &mut cycles)
        .unwrap();
    bus.write_u32(0x0000_0040, IRQ_HANDLER + 1, &mut cycles)
        .unwrap();
    for (index, instr) in program.iter().enumerate() {
        write_instr(&mut bus, THREAD + 4 * index as u32, *instr);
    }
    for (index, instr) in irq_program.iter().enumerate() {
        write_instr(&mut bus, IRQ_HANDLER + 4 * index as u32, *instr);
    }
    bus.write_u16(
        IRQ_HANDLER + 4 * irq_program.len() as u32,
        BX_LR,
        &mut cycles,
    )
    .unwrap();
    bus.write_u16(HARDFAULT_HANDLER, BX_LR, &mut cycles)
        .unwrap();
    bus.write_u16(USAGEFAULT_HANDLER, BX_LR, &mut cycles)
        .unwrap();

    let mut core = CortexM33Core::new(CpuId::Core0);
    core.boot(&mut bus).unwrap();
    (core, bus)
}

/// Build a fixture whose IRQ handler is just `BX LR`
/// 构建 IRQ 处理程序仅为 `BX LR` 的夹具
fn fixture(program: &[u32]) -> (CortexM33Core, SimpleBus) {
    fixture_with_irq(program, &[])
}

/// Current exception number from `IPSR` / 从 `IPSR` 取当前异常号
fn ipsr(core: &CortexM33Core) -> u32 {
    core.xpsr() & 0x1FF
}

/// Stacked return address of the frame the core is currently running
/// 核心当前运行帧的压栈返回地址
///
/// The 8-word basic frame sits at the bottom of both frame layouts; the
/// extended frame's 0x48-byte FP context is placed above it, so the stacked PC
/// is always `SP + 24` for the active frame.
/// 8 字基础帧位于两种帧布局的底部；扩展帧的 0x48 字节浮点上下文放在其上方，
/// 因此活动帧的压栈 PC 始终是 `SP + 24`。
fn stacked_pc(core: &CortexM33Core, bus: &mut SimpleBus) -> u32 {
    let mut cycles = 0u64;
    bus.read_u32(core.registers.current_sp() + 24, &mut cycles)
        .unwrap()
}

#[test]
fn unaligned_vldr_faults_even_without_unalign_trap() {
    let (mut core, mut bus) = fixture(&[VLDR_S0_R1]);
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    // 复位后 UNALIGN_TRP = 0；FPU 传输仍必须始终故障。
    // UNALIGN_TRP is clear after reset; FPU transfers must still always fault.
    assert_eq!(core.ccr() & ccr::UNALIGN_TRP, 0);
    let mut cycles = 0u64;
    bus.write_u32(0x2000_0000, 0x4142_4300, &mut cycles)
        .unwrap();
    core.registers.r[1] = 0x2000_0002;
    core.registers.fpu.write_s(0, 0xDEAD_BEEF);

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), USAGEFAULT_HANDLER & !1);
    assert_eq!(ipsr(&core), 6);
    assert_ne!(core.cfsr & cfsr::UNALIGNED, 0);
    assert_eq!(core.hfsr() & hfsr::FORCED, 0);
    // 目标寄存器未被写入，压栈返回地址是故障指令本身。
    // The destination stays unchanged and the stacked PC is the faulting
    // instruction itself.
    assert_eq!(core.registers.fpu.read_s(0), 0xDEAD_BEEF);
    assert_eq!(stacked_pc(&core, &mut bus), THREAD);
    // 地址记入 BFAR 供诊断，但架构不定义该故障的地址寄存器。
    // The address goes to BFAR for diagnostics, without claiming validity.
    assert_eq!(core.bfar(), 0x2000_0002);
    assert_eq!(core.cfsr & cfsr::BFARVALID, 0);
    assert_eq!(core.cfsr & cfsr::MMARVALID, 0);
}

#[test]
fn unaligned_vstr_faults_and_leaves_memory_untouched() {
    let (mut core, mut bus) = fixture(&[VSTR_S0_R1]);
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    let mut cycles = 0u64;
    bus.write_u32(0x2000_0000, 0x1122_3344, &mut cycles)
        .unwrap();
    bus.write_u32(0x2000_0004, 0x5566_7788, &mut cycles)
        .unwrap();
    core.registers.r[1] = 0x2000_0001;
    core.registers.fpu.write_s(0, 0xAABB_CCDD);

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), USAGEFAULT_HANDLER & !1);
    assert_ne!(core.cfsr & cfsr::UNALIGNED, 0);
    // 故障存储不得写任何字节，源寄存器也保持不变。
    // A trapped store must not write any byte; the source stays unchanged.
    assert_eq!(bus.read_u32(0x2000_0000, &mut cycles).unwrap(), 0x1122_3344);
    assert_eq!(bus.read_u32(0x2000_0004, &mut cycles).unwrap(), 0x5566_7788);
    assert_eq!(core.registers.fpu.read_s(0), 0xAABB_CCDD);
    assert_eq!(stacked_pc(&core, &mut bus), THREAD);
}

#[test]
fn unaligned_vldr_escalates_to_hardfault_when_usagefault_is_disabled() {
    let (mut core, mut bus) = fixture(&[VLDR_S0_R1]);
    // 故意不使能 SHCSR.USGFAULTENA：同步故障升级为 HardFault。
    // SHCSR.USGFAULTENA is deliberately left clear: the synchronous fault
    // escalates to HardFault.
    core.registers.r[1] = 0x2000_0003;
    core.registers.fpu.write_s(0, 0xDEAD_BEEF);

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), HARDFAULT_HANDLER & !1);
    assert_eq!(ipsr(&core), 3);
    assert_ne!(core.cfsr & cfsr::UNALIGNED, 0);
    assert_ne!(core.hfsr() & hfsr::FORCED, 0);
    assert_eq!(core.registers.fpu.read_s(0), 0xDEAD_BEEF);
    assert_eq!(stacked_pc(&core, &mut bus), THREAD);
}

#[test]
fn aligned_vldr_and_vstr_are_unaffected_by_the_trap() {
    let (mut core, mut bus) = fixture(&[VLDR_S0_R1, VSTR_S0_R1]);
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    core.system_write(SCB_CCR, ccr::UNALIGN_TRP);
    let mut cycles = 0u64;
    bus.write_u32(0x2000_0000, 0x4142_4300, &mut cycles)
        .unwrap();
    core.registers.r[1] = 0x2000_0000;

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.registers.fpu.read_s(0), 0x4142_4300);
    assert_eq!(core.cfsr & cfsr::UNALIGNED, 0);

    core.registers.r[1] = 0x2000_0004;
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(bus.read_u32(0x2000_0004, &mut cycles).unwrap(), 0x4142_4300);
    assert_eq!(core.cfsr & cfsr::UNALIGNED, 0);
    assert_eq!(core.pc(), THREAD + 8);
}

#[test]
fn unaligned_vldm_faults_and_preserves_registers_and_base() {
    let (mut core, mut bus) = fixture(&[VLDMIA_R0_S0_S1]);
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    let mut cycles = 0u64;
    bus.write_u32(0x2000_0000, 0x3F80_0000, &mut cycles)
        .unwrap();
    bus.write_u32(0x2000_0004, 0x4000_0000, &mut cycles)
        .unwrap();
    core.registers.r[0] = 0x2000_0002;
    core.registers.fpu.write_s(0, 0xAAAA_AAAA);
    core.registers.fpu.write_s(1, 0xBBBB_BBBB);

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(ipsr(&core), 6);
    assert_ne!(core.cfsr & cfsr::UNALIGNED, 0);
    // 未对齐 VLDM 不加载任何寄存器，也不写回基址。
    // An unaligned VLDM loads no register and skips the base write-back.
    assert_eq!(core.registers.fpu.read_s(0), 0xAAAA_AAAA);
    assert_eq!(core.registers.fpu.read_s(1), 0xBBBB_BBBB);
    assert_eq!(core.registers.r[0], 0x2000_0002);
    assert_eq!(stacked_pc(&core, &mut bus), THREAD);
}

#[test]
fn unaligned_vstm_faults_and_leaves_memory_untouched() {
    let (mut core, mut bus) = fixture(&[VSTMIA_R0_S0_S1]);
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    let mut cycles = 0u64;
    bus.write_u32(0x2000_0004, 0x1122_3344, &mut cycles)
        .unwrap();
    bus.write_u32(0x2000_0008, 0x5566_7788, &mut cycles)
        .unwrap();
    core.registers.r[0] = 0x2000_0006;
    core.registers.fpu.write_s(0, 0xCCCC_CCCC);
    core.registers.fpu.write_s(1, 0xDDDD_DDDD);

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(ipsr(&core), 6);
    assert_ne!(core.cfsr & cfsr::UNALIGNED, 0);
    assert_eq!(bus.read_u32(0x2000_0004, &mut cycles).unwrap(), 0x1122_3344);
    assert_eq!(bus.read_u32(0x2000_0008, &mut cycles).unwrap(), 0x5566_7788);
    assert_eq!(core.registers.r[0], 0x2000_0006);
    assert_eq!(stacked_pc(&core, &mut bus), THREAD);
}

#[test]
fn aligned_vldm_vstm_with_writeback_work_when_unalign_trap_is_set() {
    let (mut core, mut bus) = fixture(&[VLDMIA_R0_S0_S1, VSTMIA_R0_S0_S1]);
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    core.system_write(SCB_CCR, ccr::UNALIGN_TRP);
    let mut cycles = 0u64;
    bus.write_u32(0x2000_0000, 0x3F80_0000, &mut cycles)
        .unwrap();
    bus.write_u32(0x2000_0004, 0x4000_0000, &mut cycles)
        .unwrap();
    core.registers.r[0] = 0x2000_0000;

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.registers.fpu.read_s(0), 0x3F80_0000);
    assert_eq!(core.registers.fpu.read_s(1), 0x4000_0000);
    assert_eq!(core.registers.r[0], 0x2000_0008);

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(bus.read_u32(0x2000_0008, &mut cycles).unwrap(), 0x3F80_0000);
    assert_eq!(bus.read_u32(0x2000_000C, &mut cycles).unwrap(), 0x4000_0000);
    assert_eq!(core.registers.r[0], 0x2000_0010);
    assert_eq!(core.cfsr & cfsr::UNALIGNED, 0);
    assert_eq!(core.pc(), THREAD + 8);
}

#[test]
fn unaligned_double_vldr_faults_on_word_misalignment() {
    let (mut core, mut bus) = fixture(&[VLDR_D0_R1]);
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    let mut cycles = 0u64;
    bus.write_u32(0x2000_0000, 0x1111_1111, &mut cycles)
        .unwrap();
    bus.write_u32(0x2000_0004, 0x2222_2222, &mut cycles)
        .unwrap();
    core.registers.r[1] = 0x2000_0002;
    core.registers.fpu.write_s(0, 0xAAAA_AAAA);
    core.registers.fpu.write_s(1, 0xBBBB_BBBB);

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(ipsr(&core), 6);
    assert_ne!(core.cfsr & cfsr::UNALIGNED, 0);
    assert_eq!(core.registers.fpu.read_s(0), 0xAAAA_AAAA);
    assert_eq!(core.registers.fpu.read_s(1), 0xBBBB_BBBB);
    assert_eq!(core.bfar(), 0x2000_0002);
}

#[test]
fn aligned_double_vldr_only_requires_word_alignment() {
    let (mut core, mut bus) = fixture(&[VLDR_D0_R1]);
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    core.system_write(SCB_CCR, ccr::UNALIGN_TRP);
    let mut cycles = 0u64;
    // 4 字节对齐但非 8 字节对齐：64 位 VLDR 只要求字对齐。
    // Word-aligned but not doubleword-aligned: a 64-bit VLDR only requires
    // word alignment.
    bus.write_u32(0x2000_0004, 0x1111_1111, &mut cycles)
        .unwrap();
    bus.write_u32(0x2000_0008, 0x2222_2222, &mut cycles)
        .unwrap();
    core.registers.r[1] = 0x2000_0004;

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.registers.fpu.read_s(0), 0x1111_1111);
    assert_eq!(core.registers.fpu.read_s(1), 0x2222_2222);
    assert_eq!(core.cfsr & cfsr::UNALIGNED, 0);
}

#[test]
fn unaligned_vldr_in_lazy_extended_frame_returns_to_faulting_instruction() {
    let (mut core, mut bus) = fixture_with_irq(&[VMOV_S0_R0], &[VLDR_S1_R1]);
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    // IRQ0 优先级 6，UsageFault 保持 0：故障可抢占 IRQ 处理程序。
    // IRQ0 priority 6, UsageFault stays 0: the fault preempts the IRQ handler.
    core.nvic.write_iser(0, 1);
    core.nvic.set_priority(0, 6);
    core.registers.r[0] = 0x3F80_0000;
    core.registers.r[1] = 0x2000_0002;
    core.registers.fpu.write_s(1, 0xDEAD_BEEF);
    core.signal_interrupt(0);

    // 线程 VMOV 建立活跃浮点上下文，IRQ 进入时预留惰性扩展帧。
    // The thread VMOV creates a live FP context; IRQ entry reserves a lazy
    // extended frame.
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), IRQ_HANDLER);
    assert!(core.registers.fpu.lspact());
    assert_eq!(core.registers.lr & 0x10, 0, "extended frame / 扩展帧");
    let outer_fpcar = core.registers.fpu.fpcar;
    assert_eq!(outer_fpcar, STACK_TOP - 0x48);
    assert_eq!(core.registers.msp, STACK_TOP - 0x68);

    // 处理程序首条 VFP 指令先提交外层上下文，然后未对齐 VLDR 故障。
    // The handler's first VFP instruction commits the outer context, then the
    // unaligned VLDR faults.
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_ne!(core.cfsr & cfsr::UNALIGNED, 0);
    assert_eq!(core.pc(), USAGEFAULT_HANDLER & !1);
    assert_eq!(core.active_exceptions, vec![16, 6]);
    let mut cycles = 0u64;
    assert_eq!(
        bus.read_u32(outer_fpcar, &mut cycles).unwrap(),
        0x3F80_0000,
        "lazy context committed before the fault / 故障前已提交惰性上下文"
    );
    // 故障指令的目标寄存器保持不变；内层帧同样为扩展帧。
    // The faulting instruction leaves its destination unchanged; the inner
    // frame is extended as well.
    assert_eq!(core.registers.fpu.read_s(1), 0xDEAD_BEEF);
    assert_eq!(core.registers.lr & 0x10, 0);
    assert_eq!(core.registers.msp, STACK_TOP - 0xD0);
    assert_eq!(stacked_pc(&core, &mut bus), IRQ_HANDLER);
    assert!(
        core.registers.fp_context_active(),
        "FP context stays active / 浮点上下文保持活跃"
    );

    // 从 UsageFault 返回：内层惰性帧只回收，PC 回到故障的 VLDR。
    // Returning from the UsageFault reclaims the inner lazy frame and resumes
    // at the faulting VLDR.
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), IRQ_HANDLER);
    assert_eq!(core.registers.msp, STACK_TOP - 0x68);
    assert_eq!(core.active_exceptions, vec![16]);
    assert_eq!(core.registers.fpu.read_s(0), 0x3F80_0000);
    assert_eq!(core.registers.fpu.read_s(1), 0xDEAD_BEEF);
    assert!(core.registers.control & CONTROL_FPCA != 0);
}
