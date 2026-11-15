//! `CCR` traps, `SHPR1` priorities and stack alignment tests
//! `CCR` 陷阱、`SHPR1` 优先级与栈对齐测试
//!
//! ARMv8-M puts the unaligned-access and divide-by-zero traps in `CCR`
//! (`UNALIGN_TRP` bit 3, `DIV_0_TRP` bit 4) and the MemManage/BusFault/
//! UsageFault priorities in `SHPR1` bytes 0/1/2. This module pins:
//!
//! - `UNALIGN_TRP = 0` allows unaligned `LDR`/`STR`/`LDM` exactly as before,
//!   and `UNALIGN_TRP = 1` turns the same accesses into `UFSR.UNALIGNED`
//!   UsageFaults with the faulting instruction as the stacked return address;
//! - `DIV_0_TRP = 1` turns `SDIV`/`UDIV` by zero into `UFSR.DIVBYZERO`;
//! - the `SHPR1` byte layout and the resulting preemption (a higher-priority
//!   fault preempts an IRQ, an equal/lower one escalates to HardFault, and a
//!   higher-priority IRQ preempts an active fault handler);
//! - `CCR.STKALIGN` reads as one and unaligned MSPs are restored exactly.
//!
//! ARMv8-M 把未对齐与除零陷阱放在 `CCR`（`UNALIGN_TRP` 位 3、`DIV_0_TRP`
//! 位 4），把 MemManage/BusFault/UsageFault 优先级放在 `SHPR1` 第 0/1/2 字节。
//! 本模块锁定：`UNALIGN_TRP = 0` 保持原有未对齐访问行为，置位后同一批访问产生
//! `UFSR.UNALIGNED` 并以故障指令本身为压栈返回地址；`DIV_0_TRP = 1` 使
//! `SDIV`/`UDIV` 除零产生 `UFSR.DIVBYZERO`；`SHPR1` 字节布局与抢占规则；
//! `CCR.STKALIGN` 恒读 1 且未对齐 MSP 能精确恢复。

use super::*;

use crate::faults::{ccr, cfsr, hfsr, shcsr};
use rp2350_core::memory::{RamRegion, SimpleBus};

/// Vector table base programmed into `VTOR` / 写入 `VTOR` 的向量表基址
const VECTORS: u32 = 0x0000_0400;
/// Thread program base / 线程程序基址
const THREAD: u32 = 0x0000_0100;
/// HardFault handler (Thumb bit set) / HardFault 处理程序（含 Thumb 位）
const HARDFAULT_HANDLER: u32 = 0x0000_0601;
/// UsageFault handler: `NOP` then spin, so a nested IRQ can preempt it
/// UsageFault 处理程序：`NOP` 后自旋，使嵌套 IRQ 能抢占它
const USAGEFAULT_HANDLER: u32 = 0x0000_0621;
/// IRQ0 handler: unaligned `LDR r0, [r1]` then `BX LR`
/// IRQ0 处理程序：未对齐 `LDR r0, [r1]` 后 `BX LR`
const IRQ0_HANDLER: u32 = 0x0000_0631;
/// PendSV handler / PendSV 处理程序
const PENDSV_HANDLER: u32 = 0x0000_0641;
/// IRQ1 handler / IRQ1 处理程序
const IRQ1_HANDLER: u32 = 0x0000_0671;
/// MemManage handler / MemManage 处理程序
const MEMMANAGE_HANDLER: u32 = 0x0000_0681;
/// BusFault handler / BusFault 处理程序
const BUSFAULT_HANDLER: u32 = 0x0000_0691;
/// SVCall handler / SVCall 处理程序
const SVCALL_HANDLER: u32 = 0x0000_06B1;
/// SysTick handler / SysTick 处理程序
const SYSTICK_HANDLER: u32 = 0x0000_06C1;
/// Initial MSP / 初始 MSP
const STACK_TOP: u32 = 0x2000_1000;

/// `SCB_CCR` / `SCB_CCR`
const SCB_CCR: u32 = 0xE000_ED14;
/// `SCB_SHPR1` / `SCB_SHPR1`
const SCB_SHPR1: u32 = 0xE000_ED18;
/// `SCB_SHCSR` / `SCB_SHCSR`
const SCB_SHCSR: u32 = 0xE000_ED24;
/// `SCB_CFSR` / `SCB_CFSR`
const SCB_CFSR: u32 = 0xE000_ED28;
/// `SCB_MMFAR` / `SCB_MMFAR`
const SCB_MMFAR: u32 = 0xE000_ED34;
/// `SCB_BFAR` / `SCB_BFAR`
const SCB_BFAR: u32 = 0xE000_ED38;

/// `NOP` (`0xBF00`) / `NOP`
const NOP: u16 = 0xBF00;
/// `B .` (`0xE7FE`) / `B .`
const SPIN: u16 = 0xE7FE;
/// `BX LR` (`0x4770`) / `BX LR`
const BX_LR: u16 = 0x4770;
/// `LDR r0, [r1, #0]` / `LDR r0, [r1, #0]`
const LDR_WORD: u16 = 0x6808;
/// `STR r2, [r1, #0]` / `STR r2, [r1, #0]`
const STR_WORD: u16 = 0x600A;
/// `LDRH r0, [r1, #0]` / `LDRH r0, [r1, #0]`
const LDRH: u16 = 0x8808;
/// `LDMIA r0!, {r1, r2}` / `LDMIA r0!, {r1, r2}`
const LDMIA: u16 = 0xC806;
/// `SDIV r0, r1, r2` (first halfword) / `SDIV r0, r1, r2`（第一半字）
const SDIV_HW1: u16 = 0xFB91;
/// `SDIV r0, r1, r2` (second halfword) / `SDIV r0, r1, r2`（第二半字）
const SDIV_HW2: u16 = 0xF0F2;
/// `UDIV r0, r1, r2` (first halfword) / `UDIV r0, r1, r2`（第一半字）
const UDIV_HW1: u16 = 0xFBB1;
/// `UDIV r0, r1, r2` (second halfword) / `UDIV r0, r1, r2`（第二半字）
const UDIV_HW2: u16 = 0xF0F2;

/// Write a halfword program / 写入半字程序
fn write_code(bus: &mut SimpleBus, base: u32, code: &[u16]) {
    let mut cycles = 0u64;
    for (index, halfword) in code.iter().enumerate() {
        bus.write_u16(base + 2 * index as u32, *halfword, &mut cycles)
            .unwrap();
    }
}

/// Build a core with vectors for every modelled fault, IRQ0 and IRQ1
/// 构建含全部已建模故障、IRQ0 与 IRQ1 向量的核心
fn fixture(program: &[u16]) -> (CortexM33Core, SimpleBus) {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("code", 0x0000_0000, 0x1000)))
        .unwrap();
    bus.register_region(Box::new(RamRegion::new("stack", 0x2000_0000, 0x1000)))
        .unwrap();
    let mut cycles = 0u64;
    bus.write_u32(0x0000_0000, STACK_TOP, &mut cycles).unwrap();
    bus.write_u32(0x0000_0004, THREAD + 1, &mut cycles).unwrap();
    for (number, handler) in [
        (3u16, HARDFAULT_HANDLER),
        (4, MEMMANAGE_HANDLER),
        (5, BUSFAULT_HANDLER),
        (6, USAGEFAULT_HANDLER),
        (11, SVCALL_HANDLER),
        (14, PENDSV_HANDLER),
        (15, SYSTICK_HANDLER),
        (16, IRQ0_HANDLER),
        (17, IRQ1_HANDLER),
    ] {
        bus.write_u32(VECTORS + u32::from(number) * 4, handler, &mut cycles)
            .unwrap();
    }
    write_code(&mut bus, THREAD, program);
    for handler in [
        HARDFAULT_HANDLER,
        PENDSV_HANDLER,
        IRQ1_HANDLER,
        MEMMANAGE_HANDLER,
        BUSFAULT_HANDLER,
        SVCALL_HANDLER,
        SYSTICK_HANDLER,
    ] {
        write_code(&mut bus, handler & !1, &[BX_LR]);
    }
    // UsageFault 处理程序先执行一条 NOP，使统一仲裁能在故障仍活跃时投递 IRQ。
    // The UsageFault handler executes one NOP so the shared arbitration can
    // deliver an IRQ while the fault is still active.
    write_code(&mut bus, USAGEFAULT_HANDLER & !1, &[NOP, SPIN]);
    write_code(&mut bus, IRQ0_HANDLER & !1, &[LDR_WORD, BX_LR]);

    let mut core = CortexM33Core::new(CpuId::Core0);
    core.boot(&mut bus).unwrap();
    core.system_write(SCB_VTOR, VECTORS);
    (core, bus)
}

/// Current exception number from `IPSR` / 从 `IPSR` 取当前异常号
fn ipsr(core: &CortexM33Core) -> u32 {
    core.xpsr() & 0x1FF
}

/// Fault status bits that must stay clear for a fault-free run
/// 无故障运行时必须保持清零的故障状态位
const ALL_USAGE_BITS: u32 = cfsr::UNALIGNED | cfsr::DIVBYZERO;

#[test]
fn ccr_reset_reads_stkalign_only() {
    let (mut core, _bus) = fixture(&[SPIN]);
    assert_eq!(core.ccr(), ccr::STKALIGN);
    assert_eq!(core.system_read(SCB_CCR), Some(ccr::RESET));
    // 复位时两个陷阱都关闭：默认行为与历史实现一致。
    // Both traps are off at reset: the default matches the historical behaviour.
    assert_eq!(core.ccr() & (ccr::UNALIGN_TRP | ccr::DIV_0_TRP), 0);
}

#[test]
fn ccr_round_trips_writable_bits_and_masks_reserved_ones() {
    let (mut core, _bus) = fixture(&[SPIN]);
    // 全 1 写入：只保存五个可写位，STKALIGN 恒 1，保留位读 0。
    // All-ones write: only the five writable bits are stored, STKALIGN reads
    // one and reserved bits read zero.
    core.system_write(SCB_CCR, 0xFFFF_FFFF);
    assert_eq!(
        core.system_read(SCB_CCR),
        Some(ccr::STKALIGN | ccr::WRITABLE_MASK)
    );
    // 三个「仅存储」位可读回，但本模型没有可用它们的行为。
    // The three storage-only bits read back even though no behaviour uses them.
    assert_eq!(
        core.ccr() & (ccr::NONBASETHRDENA | ccr::USERSETMPEND | ccr::BFHFNMIGN),
        ccr::NONBASETHRDENA | ccr::USERSETMPEND | ccr::BFHFNMIGN
    );
    core.system_write(SCB_CCR, 0);
    assert_eq!(core.system_read(SCB_CCR), Some(ccr::STKALIGN));
}

#[test]
fn unaligned_word_loads_are_allowed_when_unalign_trap_is_clear() {
    let (mut core, mut bus) = fixture(&[LDR_WORD, SPIN]);
    let mut cycles = 0u64;
    bus.write_u32(0x2000_0000, 0x1122_3344, &mut cycles)
        .unwrap();
    core.registers.r[1] = 0x2000_0001;

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    // 逐字节回退读出的值：0x2000_0001..0x2000_0004 = 22 33 44 ??。
    // Byte-wise fallback reads 0x2000_0001..0x2000_0004 = 22 33 44 ??.
    assert_eq!(core.read_register(0), 0x0011_2233);
    assert_eq!(core.pc(), THREAD + 2);
    assert_eq!(core.cfsr & ALL_USAGE_BITS, 0);
    assert_eq!(ipsr(&core), 0);
}

#[test]
fn unaligned_word_load_raises_ufsr_unaligned_when_trap_is_set() {
    let (mut core, mut bus) = fixture(&[LDR_WORD, SPIN]);
    let mut cycles = 0u64;
    bus.write_u32(0x2000_0000, 0x1122_3344, &mut cycles)
        .unwrap();
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    core.system_write(SCB_CCR, ccr::UNALIGN_TRP);
    core.registers.r[0] = 0xDEAD_BEEF;
    core.registers.r[1] = 0x2000_0001;

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), USAGEFAULT_HANDLER & !1);
    assert_eq!(ipsr(&core), 6);
    assert_ne!(core.cfsr & cfsr::UNALIGNED, 0);
    assert_eq!(core.hfsr() & hfsr::FORCED, 0);
    assert_ne!(core.shcsr() & shcsr::USGFAULTACT, 0);
    // 故障指令未写目标寄存器，压栈返回地址是故障指令本身。
    // The faulting instruction leaves the destination unwritten and stacks
    // its own address as the return address.
    assert_eq!(core.read_register(0), 0xDEAD_BEEF);
    assert_eq!(
        bus.read_u32(STACK_TOP - 32 + 24, &mut cycles).unwrap(),
        THREAD
    );
}

#[test]
fn unaligned_word_store_raises_and_leaves_memory_untouched() {
    let (mut core, mut bus) = fixture(&[STR_WORD, SPIN]);
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    core.system_write(SCB_CCR, ccr::UNALIGN_TRP);
    core.registers.r[1] = 0x2000_0002;
    core.registers.r[2] = 0x5566_7788;

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), USAGEFAULT_HANDLER & !1);
    assert_ne!(core.cfsr & cfsr::UNALIGNED, 0);
    // 捕获的访问不得写内存（0x2000_0002 处仍为 0）。
    // A trapped access must not write memory (0x2000_0002 is still zero).
    let mut cycles = 0u64;
    assert_eq!(bus.read_u32(0x2000_0000, &mut cycles).unwrap(), 0);
}

#[test]
fn unaligned_ldm_raises_and_skips_base_write_back() {
    let (mut core, mut bus) = fixture(&[LDMIA, SPIN]);
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    core.system_write(SCB_CCR, ccr::UNALIGN_TRP);
    core.registers.r[0] = 0x2000_0002;
    core.registers.r[1] = 0xAAAA_AAAA;
    core.registers.r[2] = 0xBBBB_BBBB;

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(ipsr(&core), 6);
    assert_ne!(core.cfsr & cfsr::UNALIGNED, 0);
    // 捕获时基址与目标寄存器都保持原值。
    // On a trap both the base and the destination registers stay unchanged.
    assert_eq!(core.read_register(0), 0x2000_0002);
    assert_eq!(core.read_register(1), 0xAAAA_AAAA);
    assert_eq!(core.read_register(2), 0xBBBB_BBBB);
}

#[test]
fn unaligned_ldm_is_allowed_when_unalign_trap_is_clear() {
    let (mut core, mut bus) = fixture(&[LDMIA, SPIN]);
    let mut cycles = 0u64;
    bus.write_u32(0x2000_0000, 0x0102_0304, &mut cycles)
        .unwrap();
    bus.write_u32(0x2000_0004, 0x0506_0708, &mut cycles)
        .unwrap();
    core.registers.r[0] = 0x2000_0002;

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    // 现有「允许未对齐」行为：LDM 从基址逐字读取并写回。
    // The pre-existing permissive behaviour: LDM reads each word from the base
    // and writes the base back.
    assert_eq!(core.cfsr & ALL_USAGE_BITS, 0);
    assert_eq!(core.pc(), THREAD + 2);
    assert_eq!(core.read_register(0), 0x2000_000A);
    assert_eq!(core.read_register(1), 0x0708_0102);
    assert_eq!(core.read_register(2), 0x0000_0506);
}

#[test]
fn aligned_word_and_halfword_accesses_are_unaffected_by_the_trap() {
    let (mut core, mut bus) = fixture(&[LDR_WORD, STR_WORD, LDRH, SPIN]);
    let mut cycles = 0u64;
    bus.write_u32(0x2000_0000, 0x1122_3344, &mut cycles)
        .unwrap();
    bus.write_u16(0x2000_0008, 0x7788, &mut cycles).unwrap();
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    core.system_write(SCB_CCR, ccr::UNALIGN_TRP);
    core.registers.r[1] = 0x2000_0000;
    core.registers.r[2] = 0x5566_7788;

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.read_register(0), 0x7788);
    assert_eq!(core.cfsr & ALL_USAGE_BITS, 0);
    assert_eq!(bus.read_u32(0x2000_0000, &mut cycles).unwrap(), 0x5566_7788);
}

#[test]
fn divide_by_zero_returns_arm_results_when_div_0_trap_is_clear() {
    let (mut core, mut bus) = fixture(&[SDIV_HW1, SDIV_HW2, UDIV_HW1, UDIV_HW2, SPIN]);
    core.registers.r[1] = 1234;
    core.registers.r[2] = 0;

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    // SDIV 除零按 ARM 返回 0。 / SDIV by zero returns 0 per the ARM ARM.
    assert_eq!(core.read_register(0), 0);
    assert_eq!(core.cfsr & ALL_USAGE_BITS, 0);

    core.registers.r[1] = 4321;
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    // UDIV 除零返回 0。 / UDIV by zero returns 0.
    assert_eq!(core.read_register(0), 0);
    assert_eq!(core.cfsr & ALL_USAGE_BITS, 0);
    assert_eq!(ipsr(&core), 0);
}

#[test]
fn sdiv_by_zero_raises_divbyzero_and_skips_the_write() {
    let (mut core, mut bus) = fixture(&[SDIV_HW1, SDIV_HW2, SPIN]);
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    core.system_write(SCB_CCR, ccr::DIV_0_TRP);
    core.registers.r[0] = 0xDEAD_BEEF;
    core.registers.r[1] = 0x1234_5678;
    core.registers.r[2] = 0;

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), USAGEFAULT_HANDLER & !1);
    assert_eq!(ipsr(&core), 6);
    assert_ne!(core.cfsr & cfsr::DIVBYZERO, 0);
    assert_eq!(core.cfsr & cfsr::UNALIGNED, 0);
    assert_eq!(core.read_register(0), 0xDEAD_BEEF);
    let mut cycles = 0u64;
    assert_eq!(
        bus.read_u32(STACK_TOP - 32 + 24, &mut cycles).unwrap(),
        THREAD
    );
}

#[test]
fn udiv_by_zero_escalates_to_hardfault_when_usagefault_is_disabled() {
    let (mut core, mut bus) = fixture(&[UDIV_HW1, UDIV_HW2, SPIN]);
    core.system_write(SCB_CCR, ccr::DIV_0_TRP);
    core.registers.r[1] = 0xFFFF_FFFF;
    core.registers.r[2] = 0;

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), HARDFAULT_HANDLER & !1);
    assert_eq!(ipsr(&core), 3);
    assert_ne!(core.cfsr & cfsr::DIVBYZERO, 0);
    assert_ne!(core.hfsr() & hfsr::FORCED, 0);
}

#[test]
fn shpr1_byte_layout_and_priority_decoding() {
    let (mut core, _bus) = fixture(&[SPIN]);
    // 第 0/1/2 字节 = MemManage/BusFault/UsageFault；第 3 字节保留读 0。
    // Bytes 0/1/2 = MemManage/BusFault/UsageFault; byte 3 is reserved and
    // reads zero.
    core.system_write(SCB_SHPR1, 0xFFFF_FFFF);
    assert_eq!(core.system_read(SCB_SHPR1), Some(0x00E0_E0E0));
    core.system_write(SCB_SHPR1, 0x0020_4060);
    assert_eq!(core.system_read(SCB_SHPR1), Some(0x0020_4060));
    let state = &core.scb;
    assert_eq!(state.memmanage_priority(), 3);
    assert_eq!(state.busfault_priority(), 2);
    assert_eq!(state.usagefault_priority(), 1);
}

#[test]
fn shpr1_usagefault_priority_preempts_a_lower_priority_irq() {
    let (mut core, mut bus) = fixture(&[NOP, SPIN]);
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    core.system_write(SCB_CCR, ccr::UNALIGN_TRP);
    // UsageFault 优先级 2，IRQ0 优先级 6：故障处理器可抢占 IRQ0。
    // UsageFault priority 2, IRQ0 priority 6: the fault handler preempts IRQ0.
    core.system_write(SCB_SHPR1, 0x0040_0000);
    core.nvic.set_priority(0, 6);
    core.nvic.write_iser(0, 1);
    core.registers.r[1] = 0x2000_0001;

    core.signal_interrupt(0);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), IRQ0_HANDLER & !1);
    assert_eq!(core.active_exceptions, vec![16]);

    // IRQ0 处理程序执行未对齐 LDR：UsageFault 以更高优先级抢占它。
    // The IRQ0 handler executes the unaligned LDR: UsageFault preempts it.
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), USAGEFAULT_HANDLER & !1);
    assert_eq!(ipsr(&core), 6);
    assert_eq!(core.active_exceptions, vec![16, 6]);
    assert_ne!(core.cfsr & cfsr::UNALIGNED, 0);
    assert_eq!(core.hfsr() & hfsr::FORCED, 0);
}

#[test]
fn shpr1_usagefault_priority_that_cannot_preempt_escalates_to_hardfault() {
    let (mut core, mut bus) = fixture(&[NOP, SPIN]);
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    core.system_write(SCB_CCR, ccr::UNALIGN_TRP);
    // UsageFault 优先级 7，IRQ0 优先级 0：同步故障无法抢占，升级为 HardFault。
    // UsageFault priority 7, IRQ0 priority 0: the synchronous fault cannot
    // preempt, so it escalates to HardFault.
    core.system_write(SCB_SHPR1, 0x00E0_0000);
    core.nvic.set_priority(0, 0);
    core.nvic.write_iser(0, 1);
    core.registers.r[1] = 0x2000_0001;

    core.signal_interrupt(0);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), IRQ0_HANDLER & !1);

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), HARDFAULT_HANDLER & !1);
    assert_eq!(ipsr(&core), 3);
    assert_eq!(core.active_exceptions, vec![16, 3]);
    // CFSR 保留原始原因，HFSR 记录强迫升级。
    // CFSR keeps the original reason and HFSR records the forced escalation.
    assert_ne!(core.cfsr & cfsr::UNALIGNED, 0);
    assert_ne!(core.hfsr() & hfsr::FORCED, 0);
}

#[test]
fn higher_priority_irq_preempts_an_active_usagefault_handler() {
    let (mut core, mut bus) = fixture(&[LDR_WORD, SPIN]);
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    core.system_write(SCB_CCR, ccr::UNALIGN_TRP);
    // UsageFault 优先级 3，IRQ1 优先级 1：IRQ1 可抢占故障处理程序。
    // UsageFault priority 3, IRQ1 priority 1: IRQ1 may preempt the fault handler.
    core.system_write(SCB_SHPR1, 0x0060_0000);
    core.nvic.set_priority(1, 1);
    core.nvic.write_iser(0, 1 << 1);
    core.registers.r[1] = 0x2000_0001;

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), USAGEFAULT_HANDLER & !1);
    assert_eq!(core.active_exceptions, vec![6]);

    core.signal_interrupt(1);
    // 处理程序的 NOP 退役后，统一仲裁按 SHPR1 优先级投递 IRQ1。
    // After the handler's NOP retires, the shared arbitration delivers IRQ1
    // using the SHPR1 priority of the active UsageFault.
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), IRQ1_HANDLER & !1);
    assert_eq!(core.active_exceptions, vec![6, 17]);
}

#[test]
fn equal_priority_irq_does_not_preempt_an_active_usagefault_handler() {
    let (mut core, mut bus) = fixture(&[LDR_WORD, SPIN]);
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    core.system_write(SCB_CCR, ccr::UNALIGN_TRP);
    // 同为优先级 1：不满足「严格更高」，IRQ1 必须保持挂起。
    // Both at priority 1: not strictly higher, so IRQ1 stays pending.
    core.system_write(SCB_SHPR1, 0x0020_0000);
    core.nvic.set_priority(1, 1);
    core.nvic.write_iser(0, 1 << 1);
    core.registers.r[1] = 0x2000_0001;

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    core.signal_interrupt(1);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), (USAGEFAULT_HANDLER & !1) | 0x2);
    assert_eq!(core.active_exceptions, vec![6]);
    assert!(core.nvic.is_pending(1));
}

#[test]
fn mmfar_and_bfar_round_trip_and_stay_clear_of_unaligned_faults() {
    let (mut core, mut bus) = fixture(&[LDR_WORD, SPIN]);
    core.system_write(SCB_MMFAR, 0x2000_0010);
    core.system_write(SCB_BFAR, 0x2000_0020);
    assert_eq!(core.system_read(SCB_MMFAR), Some(0x2000_0010));
    assert_eq!(core.system_read(SCB_BFAR), Some(0x2000_0020));

    // 未对齐 UsageFault 把地址记入 BFAR 供诊断，但不置 BFARVALID（ARM 规定该
    // 故障没有地址寄存器）；MMFAR 不参与。
    // An unaligned UsageFault records its address in BFAR for diagnostics but
    // leaves BFARVALID clear (ARM defines no address register for it); MMFAR
    // is not involved.
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    core.system_write(SCB_CCR, ccr::UNALIGN_TRP);
    core.registers.r[1] = 0x2000_0003;
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.bfar(), 0x2000_0003);
    assert_eq!(core.mmfar(), 0x2000_0010);
    assert_eq!(core.cfsr & cfsr::BFARVALID, 0);
    assert_eq!(core.cfsr & cfsr::MMARVALID, 0);
}

#[test]
fn unaligned_instruction_target_still_reports_invstate_not_unaligned() {
    let (mut core, mut bus) = fixture(&[BX_LR, SPIN]);
    core.system_write(SCB_SHCSR, shcsr::USGFAULTENA);
    core.system_write(SCB_CCR, ccr::UNALIGN_TRP);
    // bit0 为 0 的分支目标选择 ARM 状态：即使 UNALIGN_TRP=1 也仍是 INVSTATE。
    // A branch target with bit 0 clear selects ARM state: even with
    // UNALIGN_TRP = 1 this is INVSTATE, not UNALIGNED.
    core.registers.lr = 0x0000_0200;

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), USAGEFAULT_HANDLER & !1);
    assert_eq!(ipsr(&core), 6);
    assert_ne!(core.cfsr & cfsr::INVSTATE, 0);
    assert_eq!(core.cfsr & cfsr::UNALIGNED, 0);
}

#[test]
fn stkalign_restores_an_unaligned_msp_after_a_basic_frame() {
    let (mut core, mut bus) = fixture(&[NOP, SPIN]);
    core.registers.msp = 0x2000_0FF4;
    core.registers.sp = 0x2000_0FF4;
    core.pend_pendsv();

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    // 8 字节对齐后的帧位于 0x2000_0FD0，压栈 xPSR 的 bit9 记录 4 字节对齐字。
    // The 8-byte aligned frame starts at 0x2000_0FD0 and stacked xPSR bit 9
    // records the 4-byte aligner.
    assert_eq!(core.pc(), PENDSV_HANDLER & !1);
    assert_eq!(core.registers.msp, 0x2000_0FD0);
    let mut cycles = 0u64;
    let stacked_xpsr = bus.read_u32(0x2000_0FD0 + 28, &mut cycles).unwrap();
    assert_ne!(stacked_xpsr & (1 << 9), 0);

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.pc(), THREAD + 2);
    assert_eq!(core.registers.msp, 0x2000_0FF4);
}

#[test]
fn cfsr_write_one_to_clear_also_clears_the_fault_address_valid_bits() {
    let (mut core, _bus) = fixture(&[SPIN]);
    core.cfsr = cfsr::UNALIGNED | cfsr::BFARVALID | cfsr::MMARVALID;
    core.system_write(SCB_CFSR, cfsr::BFARVALID);
    assert_eq!(core.cfsr & cfsr::BFARVALID, 0);
    assert_ne!(core.cfsr & cfsr::MMARVALID, 0);
    assert_ne!(core.cfsr & cfsr::UNALIGNED, 0);
}
