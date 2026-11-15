//! Fault delivery tests: NMI (2), HardFault (3), UsageFault (6), SVCall (11),
//! fault escalation and `SHCSR`/`CFSR`/`HFSR`/`AIRCR` views
//! 故障投递测试：NMI（2）、HardFault（3）、UsageFault（6）、SVCall（11）、
//! 故障升级与 `SHCSR`/`CFSR`/`HFSR`/`AIRCR` 视图
//!
//! ARMv8-M numbers NMI as 2, HardFault as 3, UsageFault as 6 and SVCall as 11.
//! The tests pin the vectors (`VTOR + 0x08` / `+0x0C` / `+0x18` / `+0x2C`), the
//! fixed-priority and mask rules, the escalation from a disabled or unstackable
//! fault to HardFault, and the `AIRCR.VECTKEY` write gate.
//! ARMv8-M 的 NMI 为 2、HardFault 为 3、UsageFault 为 6、SVCall 为 11。本模块锁定
//! 向量（`VTOR + 0x08` / `+0x0C` / `+0x18` / `+0x2C`）、固定优先级与屏蔽规则、
//! 未使能或无法压栈的故障升级为 HardFault，以及 `AIRCR.VECTKEY` 写入门控。

use super::*;

use crate::faults::{aircr, cfsr, hfsr, shcsr};
use crate::scb::icsr;
use rp2350_core::memory::{RamRegion, SimpleBus};

/// Vector table base programmed into `VTOR` (128-byte aligned)
/// 写入 `VTOR` 的向量表基址（128 字节对齐）
const VECTORS: u32 = 0x0000_0400;
/// Thread code base / 线程代码基址
const THREAD: u32 = 0x0000_0100;
/// NMI handler (Thumb bit set) / NMI 处理程序（含 Thumb 位）
const NMI_HANDLER: u32 = 0x0000_0181;
/// HardFault handler (Thumb bit set) / HardFault 处理程序（含 Thumb 位）
const HARDFAULT_HANDLER: u32 = 0x0000_0201;
/// UsageFault handler (Thumb bit set) / UsageFault 处理程序（含 Thumb 位）
const USAGEFAULT_HANDLER: u32 = 0x0000_0281;
/// SVCall handler (Thumb bit set) / SVCall 处理程序（含 Thumb 位）
const SVCALL_HANDLER: u32 = 0x0000_0301;
/// PendSV handler (Thumb bit set) / PendSV 处理程序（含 Thumb 位）
const PENDSV_HANDLER: u32 = 0x0000_0381;
/// SysTick handler (Thumb bit set) / SysTick 处理程序（含 Thumb 位）
const SYSTICK_HANDLER: u32 = 0x0000_0401;
/// Initial MSP / 初始 MSP
const STACK_TOP: u32 = 0x2000_1000;

/// `SCB_ICSR` / `SCB_ICSR`
const SCB_ICSR: u32 = 0xE000_ED04;
/// `SCB_AIRCR` / `SCB_AIRCR`
const SCB_AIRCR: u32 = 0xE000_ED0C;
/// `SCB_SHPR2` / `SCB_SHPR2`
const SCB_SHPR2: u32 = 0xE000_ED1C;
/// `SCB_SHPR3` / `SCB_SHPR3`
const SCB_SHPR3: u32 = 0xE000_ED20;
/// `SCB_SHCSR` / `SCB_SHCSR`
const SCB_SHCSR: u32 = 0xE000_ED24;
/// `SCB_CFSR` / `SCB_CFSR`
const SCB_CFSR: u32 = 0xE000_ED28;
/// `SCB_HFSR` / `SCB_HFSR`
const SCB_HFSR: u32 = 0xE000_ED2C;

/// `NOP` (`0xBF00`) / `NOP`
const NOP: u16 = 0xBF00;
/// `B .` (`0xE7FE`) / `B .`
const SPIN: u16 = 0xE7FE;
/// `BX LR` (`0x4770`) / `BX LR`
const BX_LR: u16 = 0x4770;
/// `SVC #0` (`0xDF00`) / `SVC #0`
const SVC_0: u16 = 0xDF00;
/// `SVC #0x42` (`0xDF42`) / `SVC #0x42`
const SVC_42: u16 = 0xDF42;
/// `UDF #0`-style undefined Thumb encoding / 未定义 Thumb 编码
const UNDEFINED: u16 = 0xB800;
/// `VLDR S0, [R0, #4]` first halfword / `VLDR S0, [R0, #4]` 第一半字
const VLDR_HW1: u16 = 0xED90;
/// `VLDR S0, [R0, #4]` second halfword / `VLDR S0, [R0, #4]` 第二半字
const VLDR_HW2: u16 = 0x0A01;

/// Write a halfword program / 写入半字程序
fn write_code(bus: &mut SimpleBus, base: u32, code: &[u16]) {
    let mut cycles = 0u64;
    for (index, halfword) in code.iter().enumerate() {
        bus.write_u16(base + 2 * index as u32, *halfword, &mut cycles)
            .unwrap();
    }
}

/// Build a core whose vector table has a handler for every modelled fault
/// 构建向量表含全部已建模故障处理程序的核心
fn fixture(program: &[u16]) -> (CortexM33Core, SimpleBus) {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("code", 0x0000_0000, 0x800)))
        .unwrap();
    bus.register_region(Box::new(RamRegion::new("stack", 0x2000_0000, 0x1000)))
        .unwrap();
    let mut cycles = 0u64;
    bus.write_u32(0x0000_0000, STACK_TOP, &mut cycles).unwrap();
    bus.write_u32(0x0000_0004, THREAD + 1, &mut cycles).unwrap();
    for (number, handler) in [
        (2, NMI_HANDLER),
        (3, HARDFAULT_HANDLER),
        (6, USAGEFAULT_HANDLER),
        (11, SVCALL_HANDLER),
        (14, PENDSV_HANDLER),
        (15, SYSTICK_HANDLER),
    ] {
        bus.write_u32(VECTORS + number * 4, handler, &mut cycles)
            .unwrap();
    }
    write_code(&mut bus, THREAD, program);
    for handler in [
        NMI_HANDLER,
        HARDFAULT_HANDLER,
        USAGEFAULT_HANDLER,
        SVCALL_HANDLER,
        PENDSV_HANDLER,
        SYSTICK_HANDLER,
    ] {
        write_code(&mut bus, handler & !1, &[BX_LR]);
    }

    let mut core = CortexM33Core::new(CpuId::Core0);
    core.boot(&mut bus).unwrap();
    core.system_write(SCB_VTOR, VECTORS);
    (core, bus)
}

/// Current exception number from `IPSR` / 从 `IPSR` 取当前异常号
fn ipsr(core: &CortexM33Core) -> u32 {
    core.xpsr() & 0x1FF
}

/// Read the stacked return address of the innermost frame
/// 读取最内层栈帧的返回地址
fn stacked_pc(bus: &mut SimpleBus, sp: u32) -> u32 {
    let mut cycles = 0u64;
    bus.read_u32(sp - 32 + 24, &mut cycles).unwrap()
}

#[test]
fn svc_enters_exception_11_at_vtor_plus_0x2c() {
    let (mut core, mut bus) = fixture(&[SVC_42, NOP, SPIN]);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);

    assert_eq!(core.pc(), SVCALL_HANDLER & !1);
    assert_eq!(ipsr(&core), 11);
    assert_eq!(core.read_register(14), 0xFFFF_FFF9);
    assert_eq!(core.active_exceptions, vec![11]);
    // SVC 的压栈返回地址是其后继指令，imm8 从该地址 -2 处的指令取回。
    // The SVC stacked return address is the successor; retrieve imm8 from the
    // instruction at that address - 2.
    let stacked = stacked_pc(&mut bus, STACK_TOP);
    assert_eq!(stacked, THREAD + 2);
    let mut cycles = 0u64;
    let instruction = bus.read_u16(stacked - 2, &mut cycles).unwrap();
    assert_eq!(instruction, SVC_42);
    assert_eq!(u32::from((instruction & 0xFF) as u8), 0x42);
    // 进入后 `SVCALLACT` 置位、`SVCALLPENDED` 清除。
    // On entry SVCALLACT is set and SVCALLPENDED is clear.
    assert_ne!(core.shcsr() & shcsr::SVCALLACT, 0);
    assert_eq!(core.shcsr() & shcsr::SVCALLPENDED, 0);
}

#[test]
fn svc_return_resumes_the_instruction_after_svc() {
    let (mut core, mut bus) = fixture(&[SVC_0, NOP, SPIN]);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), SVCALL_HANDLER & !1);

    // 处理程序 BX LR：回到 SVC 后继指令，活跃/挂起位全部清除。
    // Handler BX LR: return to the instruction after SVC, with all active and
    // pending bits cleared.
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), THREAD + 2);
    assert_eq!(ipsr(&core), 0);
    assert_eq!(core.active_exceptions, Vec::<u16>::new());
    assert_eq!(core.shcsr() & (shcsr::SVCALLACT | shcsr::SVCALLPENDED), 0);

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), THREAD + 4);
}

#[test]
fn svc_priority_from_shpr2_is_masked_by_basepri_until_unmasked() {
    let (mut core, mut bus) = fixture(&[SVC_0, SPIN]);
    // SHPR2 第 3 字节 = 0x40 → SVCall 优先级 2。
    // SHPR2 byte 3 = 0x40 → SVCall priority 2.
    core.system_write(SCB_SHPR2, 0x4000_0000);
    assert_eq!(core.system_read(SCB_SHPR2), Some(0x4000_0000));
    assert_eq!(core.system_exception_state().pendsv_priority, 0);

    // BASEPRI=2 屏蔽优先级 >= 2：SVC 退役后保持挂起。
    // BASEPRI=2 masks priorities >= 2: SVC stays pending after retirement.
    core.registers.basepri = 0x40;
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), THREAD + 2);
    assert_eq!(core.active_exceptions, Vec::<u16>::new());
    assert_ne!(core.shcsr() & shcsr::SVCALLPENDED, 0);

    // 解除屏蔽后，下一条指令末尾的统一仲裁投递异常 11。
    // Once unmasked, the shared arbitration at the end of the next instruction
    // delivers exception 11.
    core.registers.basepri = 0;
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), SVCALL_HANDLER & !1);
    assert_eq!(ipsr(&core), 11);
}

#[test]
fn svc_is_not_blocked_by_primask() {
    let (mut core, mut bus) = fixture(&[SVC_0, SPIN]);
    core.registers.primask = 1;
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), SVCALL_HANDLER & !1);
    assert_eq!(ipsr(&core), 11);
}

#[test]
fn undefined_instruction_escalates_to_hardfault() {
    let (mut core, mut bus) = fixture(&[UNDEFINED, SPIN]);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);

    // UsageFault 未使能 → 升级为 HardFault（异常 3），向量 VTOR + 0x0C。
    // UsageFault is disabled → escalate to HardFault (3), vector VTOR + 0x0C.
    assert_eq!(core.pc(), HARDFAULT_HANDLER & !1);
    assert_eq!(ipsr(&core), 3);
    assert_ne!(core.cfsr & cfsr::UNDEFINSTR, 0);
    assert_ne!(core.hfsr() & hfsr::FORCED, 0);
    assert_ne!(core.shcsr() & shcsr::HARDFAULTACT, 0);
    // 压栈返回地址是故障指令本身。 / The stacked PC is the faulting instruction.
    assert_eq!(stacked_pc(&mut bus, STACK_TOP), THREAD);
}

#[test]
fn usagefault_enabled_routes_nocp_to_exception_6() {
    let (mut core, mut bus) = fixture(&[VLDR_HW1, VLDR_HW2, SPIN]);
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    core.system_write(SCB_CPACR, 0);

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), USAGEFAULT_HANDLER & !1);
    assert_eq!(ipsr(&core), 6);
    assert_ne!(core.cfsr & cfsr::NOCP, 0);
    assert_eq!(core.hfsr() & hfsr::FORCED, 0);
    assert_ne!(core.shcsr() & shcsr::USGFAULTACT, 0);
    assert_eq!(stacked_pc(&mut bus, STACK_TOP), THREAD);
}

#[test]
fn usagefault_disabled_escalates_nocp_to_hardfault() {
    let (mut core, mut bus) = fixture(&[VLDR_HW1, VLDR_HW2, SPIN]);
    core.system_write(SCB_CPACR, 0);

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), HARDFAULT_HANDLER & !1);
    assert_eq!(ipsr(&core), 3);
    assert_ne!(core.cfsr & cfsr::NOCP, 0);
    assert_ne!(core.hfsr() & hfsr::FORCED, 0);
}

#[test]
fn illegal_exc_return_raises_invstate_hardfault() {
    let (mut core, mut bus) = fixture(&[BX_LR, SPIN]);
    // 0xFFFF_FFF2 带 EXC_RETURN 前缀但置位了保留位 bit1 → 非法返回。
    // 0xFFFF_FFF2 has the EXC_RETURN prefix but sets reserved bit 1 → illegal.
    core.registers.lr = 0xFFFF_FFF2;
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);

    assert_eq!(core.pc(), HARDFAULT_HANDLER & !1);
    assert_eq!(ipsr(&core), 3);
    assert_ne!(core.cfsr & cfsr::INVSTATE, 0);
    assert_ne!(core.hfsr() & hfsr::FORCED, 0);
    assert_eq!(stacked_pc(&mut bus, STACK_TOP), THREAD);
}

#[test]
fn nmi_pending_bit_enters_exception_2_even_with_primask_and_faultmask() {
    let (mut core, mut bus) = fixture(&[NOP, SPIN]);
    core.registers.primask = 1;
    core.registers.faultmask = 1;
    core.system_write(SCB_ICSR, icsr::NMIPENDSET);
    assert_ne!(core.read_icsr() & icsr::NMIPENDSET, 0);
    assert_eq!(
        (core.read_icsr() >> icsr::VECTPENDING_SHIFT) & 0x1FF,
        2,
        "VECTPENDING reports NMI / VECTPENDING 报告 NMI"
    );

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), NMI_HANDLER & !1);
    assert_eq!(ipsr(&core), 2);
    assert_eq!(core.read_icsr() & icsr::NMIPENDSET, 0);
    assert_ne!(core.shcsr() & shcsr::NMIACT, 0);
    assert_eq!(stacked_pc(&mut bus, STACK_TOP), THREAD + 2);
}

#[test]
fn icsr_nmipendset_write_of_zero_is_ignored() {
    let (mut core, _bus) = fixture(&[SPIN]);
    core.system_write(SCB_ICSR, 0);
    assert_eq!(core.read_icsr() & icsr::NMIPENDSET, 0);

    core.system_write(SCB_ICSR, icsr::NMIPENDSET);
    assert_ne!(core.read_icsr() & icsr::NMIPENDSET, 0);
    // 写 0 不会清除已挂起的 NMI；只有取走异常 2 才能清除。
    // Writing 0 does not clear a pending NMI; only taking exception 2 clears it.
    core.system_write(SCB_ICSR, 0);
    assert_ne!(core.read_icsr() & icsr::NMIPENDSET, 0);
    assert_eq!(core.shcsr() & shcsr::NMIACT, 0);
}

#[test]
fn aircr_write_requires_the_vectkey() {
    let (mut core, _bus) = fixture(&[SPIN]);
    // 无 VECTKEY：整次写入忽略，PRIS 不存储，复位请求不锁存。
    // Without VECTKEY the whole write is ignored: PRIS is not stored and no
    // reset request is latched.
    core.system_write(SCB_AIRCR, aircr::PRIS | aircr::SYSRESETREQ);
    assert_eq!(core.system_read(SCB_AIRCR).unwrap() & aircr::PRIS, 0);
    assert!(!core.take_system_reset_request());

    // 带 key：PRIS 存储，SYSRESETREQ 产生一次性请求事件。
    // With the key: PRIS is stored and SYSRESETREQ raises a one-shot request.
    core.system_write(SCB_AIRCR, aircr::VECTKEY | aircr::PRIS | aircr::SYSRESETREQ);
    let value = core.system_read(SCB_AIRCR).unwrap();
    assert_eq!(value & 0xFFFF_0000, aircr::VECTKEYSTAT);
    assert_eq!(value & aircr::PRIS, aircr::PRIS);
    assert!(core.take_system_reset_request());
    assert!(!core.take_system_reset_request());
}

#[test]
fn aircr_vectreset_is_latched_as_a_diagnostic() {
    let (mut core, _bus) = fixture(&[SPIN]);
    core.system_write(SCB_AIRCR, aircr::VECTKEY | aircr::VECTRESET);
    assert!(core.vect_reset_requested());
    assert!(!core.take_system_reset_request());
}

#[test]
fn nmi_svcall_pendsv_systick_priority_order() {
    let (mut core, mut bus) = fixture(&[SVC_0, SPIN]);
    core.system_write(SCB_SHPR2, 0x0000_0000); // SVCall priority 0
    core.system_write(SCB_SHPR3, 0x0000_0000); // PendSV/SysTick priority 0
    core.system_write(
        SCB_ICSR,
        icsr::NMIPENDSET | icsr::PENDSVSET | icsr::PENDSTSET,
    );

    // NMI 的固定优先级最高：SVC 退役后先进入异常 2。
    // NMI has the highest fixed priority: exception 2 is entered after SVC
    // retires.
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), NMI_HANDLER & !1);
    assert_eq!(core.active_exceptions, vec![2]);

    // NMI 返回后，SVCall（11）→ PendSV（14）→ SysTick（15）按异常号升序取走。
    // After the NMI return, SVCall (11) → PendSV (14) → SysTick (15) are taken
    // in ascending exception-number order.
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), SVCALL_HANDLER & !1);
    assert_eq!(core.active_exceptions, vec![11]);

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), PENDSV_HANDLER & !1);
    assert_eq!(core.active_exceptions, vec![14]);

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), SYSTICK_HANDLER & !1);
    assert_eq!(core.active_exceptions, vec![15]);
}

#[test]
fn stack_limit_overflow_escalates_pendsv_to_hardfault() {
    let (mut core, mut bus) = fixture(&[NOP, SPIN]);
    // MSPLIM 等于当前 MSP：下一帧必然越过下限 → CFSR.STKOF + HFSR.FORCED。
    // MSPLIM equals the current MSP: the next frame crosses the limit →
    // CFSR.STKOF + HFSR.FORCED.
    core.registers.msplim = STACK_TOP;
    core.pend_pendsv();
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);

    assert_eq!(core.pc(), HARDFAULT_HANDLER & !1);
    assert_eq!(ipsr(&core), 3);
    assert_ne!(core.cfsr & cfsr::STKOF, 0);
    assert_ne!(core.hfsr() & hfsr::FORCED, 0);
    // PendSV 未被取走，仍保持挂起。 / PendSV was not taken and stays pending.
    assert!(core.system_exception_state().pendsv_pending);
}

#[test]
fn shcsr_views_and_write_mask() {
    let (mut core, _bus) = fixture(&[SPIN]);
    // 写全 1 只保存三个故障使能；ACT/PENDED 是实时视图。
    // Writing all ones stores only the three enables; ACT/PENDED are views.
    core.system_write(SCB_SHCSR, 0xFFFF_FFFF);
    assert_ne!(core.shcsr() & shcsr::USGFAULTENA, 0);
    assert_ne!(core.shcsr() & shcsr::MEMFAULTENA, 0);
    assert_eq!(core.shcsr() & shcsr::SVCALLACT, 0);
    assert_eq!(core.shcsr() & shcsr::SVCALLPENDED, 0);

    core.pend_svcall();
    assert_ne!(core.shcsr() & shcsr::SVCALLPENDED, 0);
    core.pend_nmi();
    core.pend_pendsv();
    core.pend_systick();
    assert_eq!(core.shcsr() & (shcsr::SVCALLPENDED), shcsr::SVCALLPENDED);

    core.system_write(SCB_SHCSR, 0);
    assert_eq!(core.shcsr() & shcsr::WRITABLE_MASK, 0);
}

#[test]
fn cfsr_and_hfsr_are_write_one_to_clear() {
    let (mut core, _bus) = fixture(&[SPIN]);
    core.cfsr = cfsr::UNDEFINSTR | cfsr::STKERR;
    core.faults.set_hfsr(hfsr::FORCED | hfsr::DEBUGEVT);

    // 写 0 不作任何改变。 / Writing 0 changes nothing.
    core.system_write(SCB_CFSR, 0);
    core.system_write(SCB_HFSR, 0);
    assert_eq!(core.cfsr, cfsr::UNDEFINSTR | cfsr::STKERR);
    assert_eq!(core.hfsr(), hfsr::FORCED | hfsr::DEBUGEVT);

    // 写 1 只清除对应位。 / Writing 1 clears exactly the selected bits.
    core.system_write(SCB_CFSR, cfsr::UNDEFINSTR);
    core.system_write(SCB_HFSR, hfsr::FORCED);
    assert_eq!(core.cfsr, cfsr::STKERR);
    assert_eq!(core.hfsr(), hfsr::DEBUGEVT);
}
