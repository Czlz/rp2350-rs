//! PPB replay and read-back shadow tests / PPB 回放与读回影子测试
//!
//! The PPB owns no architectural state for `CCR`, `SHPR1`, `MMFAR` and `BFAR`:
//! a firmware store is appended to the shared [`SystemWriteLog`] and the driver
//! (`Machine::replay_system_writes` in `rp2350-cli`) hands every entry to
//! `CpuCore::system_write` before the next instruction. The tests below pin that
//! port plus the cache, which must reproduce the core's write rules exactly:
//!
//! - `CCR` keeps `STKALIGN` read-as-one and the five writable bits; the trap
//!   bits `UNALIGN_TRP`/`DIV_0_TRP` are the firmware-visible precondition for
//!   `UFSR.UNALIGNED`/`UFSR.DIVBYZERO` (the core-side trap tests live in
//!   `rp2350-arm::tests::ccr`);
//! - `SHPR1` keeps the MemManage/BusFault/UsageFault priority bytes and reads
//!   the reserved byte 3 as zero;
//! - `MMFAR`/`BFAR` are read/write address fields in ARMv8-M, so firmware writes
//!   are replayed and read back, but the `CFSR` valid bits stay core-owned and
//!   a firmware address write cannot forge a valid fault address.
//!
//! PPB 对 `CCR`、`SHPR1`、`MMFAR`、`BFAR` 不拥有架构状态：固件写入会追加到共享
//! [`SystemWriteLog`]，驱动（`rp2350-cli` 的 `Machine::replay_system_writes`）在
//! 下一条指令前把每条记录交给 `CpuCore::system_write`。下面的测试锁定该端口与
//! 缓存，缓存必须精确复现核的写入规则：`CCR` 的 `STKALIGN` 恒 1 与五个可写位
//! （`UNALIGN_TRP`/`DIV_0_TRP` 是 `UFSR.UNALIGNED`/`UFSR.DIVBYZERO` 的固件侧
//! 前置条件，核侧陷阱测试位于 `rp2350-arm::tests::ccr`）；`SHPR1` 的三个故障
//! 优先级字节且第 3 字节保留读 0；`MMFAR`/`BFAR` 是 ARMv8-M 的读写地址字段，
//! 因此固件写入会回放并读回，但 `CFSR` 的 valid 位归核所有，固件写地址不可能
//! 伪造有效故障地址。

use std::sync::Arc;

use crate::map;

use super::*;

/// `SCB_DFSR` — debug fault status, plain PPB storage (not replayed)
/// `SCB_DFSR` —— 调试故障状态，PPB 普通存储（不回放）
const SCB_DFSR: u32 = 0xE000_ED30;

/// Drain the log exactly like the simulation driver and hand each entry to
/// `apply`, which stands in for `CpuCore::system_write`
/// 与仿真驱动完全相同地排空日志，并把每条记录交给代表
/// `CpuCore::system_write` 的 `apply`
fn replay_into(log: &SystemWriteLog, mut apply: impl FnMut(u32, u32)) {
    let mut entries = log.lock();
    for (addr, value) in entries.drain(..) {
        apply(addr, value);
    }
}

#[test]
fn cpu_id_is_readable() {
    let mut ppb = Ppb::new();
    assert_eq!(ppb.read32(SCB_CPUID), Some(CPUID_CORTEX_M33));
}

#[test]
fn vtor_is_storage() {
    let mut ppb = Ppb::new();
    let offset = 0xE000_ED08 - map::PPB_BASE;
    assert!(ppb.write32(offset, 0x1000_0000));
    assert_eq!(ppb.read32(offset), Some(0x1000_0000));
}

#[test]
fn nvic_writes_are_logged_for_replay() {
    let log = new_system_write_log();
    let mut ppb = Ppb::with_log(Arc::clone(&log));
    // ISER0 at 0xE000_E100. ISER0 位于 0xE000_E100。
    assert!(ppb.write32(0xE100, 1));
    assert_eq!(log.lock().as_slice(), &[(0xE000_E100, 1)]);
}

#[test]
fn vtor_writes_are_logged_for_replay() {
    let log = new_system_write_log();
    let mut ppb = Ppb::with_log(Arc::clone(&log));
    // SCB_VTOR at 0xE000_ED08. SCB_VTOR 位于 0xE000_ED08。
    assert!(ppb.write32(0xED08, 0x1000_0000));
    assert_eq!(log.lock().as_slice(), &[(0xE000_ED08, 0x1000_0000)]);
}

#[test]
fn cpacr_writes_are_logged_for_replay() {
    let log = new_system_write_log();
    let mut ppb = Ppb::with_log(Arc::clone(&log));
    // SCB_CPACR at 0xE000_ED88. SCB_CPACR 位于 0xE000_ED88。
    assert!(ppb.write32(0xED88, 0x00F0_0000));
    assert_eq!(log.lock().as_slice(), &[(0xE000_ED88, 0x00F0_0000)]);
}

#[test]
fn scr_and_stir_writes_are_logged_for_replay() {
    // 真实固件的 `STR SCR`/`STR STIR` 必须到达核心：SCR 控制 SLEEPONEXIT，
    // STIR 挂起软件中断。
    // Real firmware's `STR SCR`/`STR STIR` must reach the core: SCR controls
    // SLEEPONEXIT and STIR pends a software interrupt.
    let log = new_system_write_log();
    let mut ppb = Ppb::with_log(Arc::clone(&log));
    assert!(ppb.write32(SCB_SCR - map::PPB_BASE, 2));
    assert!(ppb.write32(SCB_STIR - map::PPB_BASE, 9));
    assert_eq!(
        log.lock().as_slice(),
        &[(SCB_SCR, 2), (SCB_STIR, 9)],
        "SCR and STIR replay"
    );
}

#[test]
fn plain_ppb_writes_are_not_logged() {
    let log = new_system_write_log();
    let mut ppb = Ppb::with_log(Arc::clone(&log));
    // DFSR 与 MPI/MPU 字段在 PPB 里只是普通存储，不属于回放清单。
    // DFSR and the MPU/MPI fields are plain PPB storage, not on the replay list.
    let offset = SCB_DFSR - map::PPB_BASE;
    assert!(ppb.write32(offset, 0x0000_0200));
    assert!(log.lock().is_empty());
    assert_eq!(ppb.read32(offset), Some(0x0000_0200));
}

#[test]
fn replay_list_covers_ccr_shpr1_and_the_fault_addresses() {
    // 固件对这四个寄存器的写入必须到达核，否则陷阱与故障优先级永远不生效。
    // Firmware writes to these four registers must reach the core, otherwise
    // the traps and fault priorities never take effect.
    for addr in [SCB_CCR, SCB_SHPR1, SCB_MMFAR, SCB_BFAR] {
        assert!(is_replayed(addr), "{addr:#010X} must be replayed");
    }
    // DFSR 仍是 PPB 本地存储，不应被误加进回放清单。
    // DFSR stays PPB-local storage and must not join the replay list.
    assert!(!is_replayed(SCB_DFSR));
}

#[test]
fn fault_and_reset_registers_are_logged_for_replay() {
    // 固件写故障使能、故障状态与复位请求必须回放到核。
    // Firmware writes to the fault enables, fault status and reset requests
    // must be replayed into the core.
    let log = new_system_write_log();
    let mut ppb = Ppb::with_log(Arc::clone(&log));
    let shcsr = SCB_SHCSR - map::PPB_BASE;
    let cfsr = SCB_CFSR - map::PPB_BASE;
    let hfsr = SCB_HFSR - map::PPB_BASE;
    let aircr = SCB_AIRCR - map::PPB_BASE;
    assert!(ppb.write32(shcsr, 0x0005_0000));
    assert!(ppb.write32(cfsr, 0x0008_0000));
    assert!(ppb.write32(hfsr, 0x4000_0000));
    assert!(ppb.write32(aircr, 0x05FA_0004));
    assert_eq!(
        log.lock().as_slice(),
        &[
            (SCB_SHCSR, 0x0005_0000),
            (SCB_CFSR, 0x0008_0000),
            (SCB_HFSR, 0x4000_0000),
            (SCB_AIRCR, 0x05FA_0004),
        ]
    );
}

#[test]
fn aircr_requires_vectkey_and_reads_vectkeystat() {
    let mut ppb = Ppb::new();
    let offset = SCB_AIRCR - map::PPB_BASE;
    // 无 key：PRIS 与复位请求都不得进入读视图。
    // Without the key neither PRIS nor the reset requests reach the view.
    assert!(ppb.write32(offset, 0x0000_4004));
    assert_eq!(
        ppb.read32(offset),
        Some(0xFA05_0000),
        "VECTKEYSTAT only without a key / 无 key 时只读 VECTKEYSTAT"
    );
    // 有 key：只保存 PRIS，只写位读 0。
    // With the key only PRIS is stored; the write-only bits read zero.
    assert!(ppb.write32(offset, 0x05FA_4004));
    assert_eq!(ppb.read32(offset), Some(0xFA05_4000));
    assert!(ppb.write32(offset, 0x05FA_0004));
    assert_eq!(ppb.read32(offset), Some(0xFA05_0000));
}

#[test]
fn shcsr_shadow_keeps_only_the_fault_enables() {
    let mut ppb = Ppb::new();
    let offset = SCB_SHCSR - map::PPB_BASE;
    assert!(ppb.write32(offset, 0xFFFF_FFFF));
    assert_eq!(
        ppb.read32(offset),
        Some(0x0007_0000),
        "only MEMFAULTENA/BUSFAULTENA/USGFAULTENA are writable / 只有三个故障使能可写"
    );
    assert!(ppb.write32(offset, 0));
    assert_eq!(ppb.read32(offset), Some(0));
}

#[test]
fn cfsr_and_hfsr_shadows_are_write_one_to_clear() {
    let mut ppb = Ppb::new();
    let cfsr = SCB_CFSR - map::PPB_BASE;
    let hfsr = SCB_HFSR - map::PPB_BASE;
    // 模拟核内故障已经写入缓存的状态位。
    // Simulate fault bits already cached from the core-side fault paths.
    assert!(ppb.regs.write(cfsr, 0x0009_0001));
    assert!(ppb.regs.write(hfsr, 0x4000_0002));
    assert_eq!(ppb.read32(cfsr), Some(0x0009_0001));
    assert_eq!(ppb.read32(hfsr), Some(0x4000_0002));

    // 写 0 不改变状态；写 1 只清除对应位。
    // Writing zero changes nothing; writing one clears only those bits.
    assert!(ppb.write32(cfsr, 0));
    assert_eq!(ppb.read32(cfsr), Some(0x0009_0001));
    assert!(ppb.write32(cfsr, 0x0008_0001));
    assert_eq!(ppb.read32(cfsr), Some(0x0001_0000));
    assert!(ppb.write32(hfsr, 0x4000_0000));
    assert_eq!(ppb.read32(hfsr), Some(0x0002));
}

#[test]
fn ccr_reset_shadow_reports_stkalign() {
    // ARMv8-M 的 CCR.STKALIGN 恒读 1；寄存器文件的全零复位值不能泄漏给固件。
    // ARMv8-M CCR.STKALIGN always reads one; the all-zero register-file reset
    // value must not leak to firmware.
    let mut ppb = Ppb::new();
    let offset = SCB_CCR - map::PPB_BASE;
    assert_eq!(ppb.read32(offset), Some(0x0000_0200));
    assert!(ppb.write32(offset, 0));
    assert_eq!(ppb.read32(offset), Some(0x0000_0200));
    ppb.reset();
    assert_eq!(ppb.read32(offset), Some(0x0000_0200));
}

#[test]
fn ccr_replay_and_shadow_mirror_the_core_rules() {
    let log = new_system_write_log();
    let mut ppb = Ppb::with_log(Arc::clone(&log));
    let offset = SCB_CCR - map::PPB_BASE;

    // 全 1 写入：回放原始字，影子只保留 STKALIGN 与五个可写位。
    // All-ones write: the raw word is replayed while the shadow keeps only
    // STKALIGN and the five writable bits.
    assert!(ppb.write32(offset, 0xFFFF_FFFF));
    assert_eq!(ppb.read32(offset), Some(0x0000_031B));
    assert_eq!(log.lock().as_slice(), &[(SCB_CCR, 0xFFFF_FFFF)]);

    // 保留位（bit 5/6/10）读 0；随后五个可写位整体写入后读回。
    // Reserved bits (5/6/10) read zero; the five writable bits read back.
    assert!(ppb.write32(offset, 0x0000_0460));
    assert_eq!(ppb.read32(offset), Some(0x0000_0200));
    assert!(ppb.write32(offset, 0x0000_011B));
    assert_eq!(ppb.read32(offset), Some(0x0000_031B));
}

#[test]
fn ccr_div0_trap_bit_is_delivered_on_the_system_write_port() {
    let log = new_system_write_log();
    let mut ppb = Ppb::with_log(Arc::clone(&log));
    let offset = SCB_CCR - map::PPB_BASE;
    // 固件 `SCB->CCR |= SCB_CCR_DIV_0_TRP_Msk` 等价于对 0xE000_ED14 写 bit 4。
    // Firmware `SCB->CCR |= SCB_CCR_DIV_0_TRP_Msk` writes bit 4 of 0xE000_ED14.
    assert!(ppb.write32(offset, 1 << 4));
    assert_eq!(ppb.read32(offset), Some(0x0000_0210));

    // 驱动把日志原样交给 `CpuCore::system_write`；核内 `write_ccr` 保存 bit 4，
    // 之后 `SDIV`/`UDIV` 除零产生 `UFSR.DIVBYZERO`（核侧测试
    // `sdiv_by_zero_raises_divbyzero_and_skips_the_write`）。
    // The driver forwards the log to `CpuCore::system_write`; the core keeps
    // bit 4 and a later `SDIV`/`UDIV` by zero raises `UFSR.DIVBYZERO` (core
    // test `sdiv_by_zero_raises_divbyzero_and_skips_the_write`).
    let mut delivered = None;
    replay_into(&log, |addr, value| delivered = Some((addr, value)));
    assert_eq!(delivered, Some((SCB_CCR, 1 << 4)));
}

#[test]
fn ccr_unalign_trap_bit_is_delivered_on_the_system_write_port() {
    let log = new_system_write_log();
    let mut ppb = Ppb::with_log(Arc::clone(&log));
    let offset = SCB_CCR - map::PPB_BASE;
    // 固件 `SCB->CCR |= SCB_CCR_UNALIGN_TRP_Msk` 等价于对 0xE000_ED14 写 bit 3。
    // Firmware `SCB->CCR |= SCB_CCR_UNALIGN_TRP_Msk` writes bit 3 of
    // 0xE000_ED14.
    assert!(ppb.write32(offset, 1 << 3));
    assert_eq!(ppb.read32(offset), Some(0x0000_0208));

    // 之后未对齐 `LDR` 以 `UFSR.UNALIGNED` 进入 UsageFault（核侧测试
    // `unaligned_word_load_raises_ufsr_unaligned_when_trap_is_set`）。
    // A later unaligned `LDR` then enters UsageFault with `UFSR.UNALIGNED`
    // (core test `unaligned_word_load_raises_ufsr_unaligned_when_trap_is_set`).
    let mut delivered = None;
    replay_into(&log, |addr, value| delivered = Some((addr, value)));
    assert_eq!(delivered, Some((SCB_CCR, 1 << 3)));
}

#[test]
fn shpr1_replay_and_shadow_keep_three_priority_bytes() {
    let log = new_system_write_log();
    let mut ppb = Ppb::with_log(Arc::clone(&log));
    let offset = SCB_SHPR1 - map::PPB_BASE;

    // 只有第 0/1/2 字节的已实现优先级位可写，第 3 字节保留读 0。
    // Only the implemented priority bits of bytes 0/1/2 are writable; the
    // reserved byte 3 reads zero.
    assert!(ppb.write32(offset, 0xFFFF_FFFF));
    assert_eq!(ppb.read32(offset), Some(0x00E0_E0E0));
    assert_eq!(log.lock().as_slice(), &[(SCB_SHPR1, 0xFFFF_FFFF)]);

    assert!(ppb.write32(offset, 0x0020_4060));
    assert_eq!(ppb.read32(offset), Some(0x0020_4060));
    assert!(ppb.write32(offset, 0xFF00_0000));
    assert_eq!(ppb.read32(offset), Some(0));
}

#[test]
fn shpr1_usagefault_priority_is_delivered_on_the_system_write_port() {
    let log = new_system_write_log();
    let mut ppb = Ppb::with_log(Arc::clone(&log));
    let offset = SCB_SHPR1 - map::PPB_BASE;
    // UsageFault 优先级字节 0x40（优先级 2）；核侧
    // `shpr1_usagefault_priority_preempts_a_lower_priority_irq` 覆盖抢占语义。
    // UsageFault priority byte 0x40 (priority 2); the core test
    // `shpr1_usagefault_priority_preempts_a_lower_priority_irq` covers the
    // preemption semantics.
    assert!(ppb.write32(offset, 0x0040_0000));
    assert_eq!(ppb.read32(offset), Some(0x0040_0000));

    let mut delivered = None;
    replay_into(&log, |addr, value| delivered = Some((addr, value)));
    assert_eq!(delivered, Some((SCB_SHPR1, 0x0040_0000)));
}

#[test]
fn mmfar_and_bfar_writes_are_replayed_and_read_back() {
    let log = new_system_write_log();
    let mut ppb = Ppb::with_log(Arc::clone(&log));
    let mmfar = SCB_MMFAR - map::PPB_BASE;
    let bfar = SCB_BFAR - map::PPB_BASE;

    // 复位值 0，与核一致。 / Reset value 0, matching the core.
    assert_eq!(ppb.read32(mmfar), Some(0));
    assert_eq!(ppb.read32(bfar), Some(0));

    // ARMv8-M 的 MMFAR/BFAR 是读写地址字段：写入回放并原样读回。
    // ARMv8-M MMFAR/BFAR are read/write address fields: writes are replayed
    // and read back unchanged.
    assert!(ppb.write32(mmfar, 0x2000_0010));
    assert!(ppb.write32(bfar, 0x6000_0020));
    assert_eq!(ppb.read32(mmfar), Some(0x2000_0010));
    assert_eq!(ppb.read32(bfar), Some(0x6000_0020));
    assert_eq!(
        log.lock().as_slice(),
        &[(SCB_MMFAR, 0x2000_0010), (SCB_BFAR, 0x6000_0020)]
    );
}

#[test]
fn mmfar_and_bfar_writes_cannot_set_the_validity_bits() {
    let mut ppb = Ppb::new();
    let mmfar = SCB_MMFAR - map::PPB_BASE;
    let bfar = SCB_BFAR - map::PPB_BASE;
    let cfsr = SCB_CFSR - map::PPB_BASE;

    // 地址写入与 CFSR 的 valid 位完全解耦：MMARVALID/BFARVALID 只能由核内
    // 故障路径置位，固件写地址不能伪造有效故障地址。
    // Address writes are fully decoupled from the CFSR valid bits:
    // MMARVALID/BFARVALID are set only by core fault paths, so a firmware
    // address write cannot forge a valid fault address.
    assert!(ppb.write32(mmfar, 0x2000_0010));
    assert!(ppb.write32(bfar, 0x6000_0020));
    assert_eq!(ppb.read32(cfsr), Some(0));
    assert!(ppb.write32(cfsr, 0x0000_8080));
    assert_eq!(ppb.read32(cfsr), Some(0));
    assert_eq!(ppb.read32(mmfar), Some(0x2000_0010));
    assert_eq!(ppb.read32(bfar), Some(0x6000_0020));
}

#[test]
fn systick_registers_live_in_the_counter_not_the_replay_log() {
    // SysTick 跟随实时时钟，不能回放给核（否则核内计数器会重复计时）。
    // SysTick follows the live clock and must not be replayed, or the core
    // counter would tick twice.
    let log = new_system_write_log();
    let mut ppb = Ppb::with_log(Arc::clone(&log));
    assert!(ppb.write32(SYST_RVR, 999));
    assert!(ppb.write32(SYST_CVR, 999));
    assert!(ppb.write32(SYST_CSR, 7));
    assert_eq!(ppb.read32(SYST_RVR), Some(999));
    assert_eq!(ppb.read32(SYST_CVR), Some(999));
    assert_eq!(ppb.read32(SYST_CSR), Some(7));
    assert_eq!(ppb.read32(SYST_CALIB), Some(CALIB));
    assert!(log.lock().is_empty(), "SysTick is not replayed");
    assert!(!ppb.systick().lock().interrupt_pending());
}

#[test]
fn icsr_writes_are_replayed_and_follow_set_clear_semantics() {
    // ICSR 的 PendSV/SysTick 挂起位必须到达核；PPB 影子按写 1 置位/清除。
    // ICSR PendSV/SysTick latches must reach the core; the PPB shadow keeps
    // write-1-to-set / write-1-to-clear semantics.
    let log = new_system_write_log();
    let mut ppb = Ppb::with_log(Arc::clone(&log));
    let offset = SCB_ICSR - map::PPB_BASE;
    assert!(ppb.write32(offset, ICSR_PENDSVSET | ICSR_PENDSTSET));
    assert_eq!(ppb.read32(offset), Some(ICSR_PENDSVSET | ICSR_PENDSTSET));
    assert!(ppb.write32(offset, ICSR_PENDSTCLR));
    assert_eq!(ppb.read32(offset), Some(ICSR_PENDSVSET));
    assert!(ppb.write32(offset, ICSR_PENDSVCLR));
    assert_eq!(ppb.read32(offset), Some(0));
    assert_eq!(
        log.lock().as_slice(),
        &[
            (SCB_ICSR, ICSR_PENDSVSET | ICSR_PENDSTSET),
            (SCB_ICSR, ICSR_PENDSTCLR),
            (SCB_ICSR, ICSR_PENDSVCLR),
        ]
    );
}

#[test]
fn shpr2_and_shpr3_writes_are_replayed_for_system_priorities() {
    let log = new_system_write_log();
    let mut ppb = Ppb::with_log(Arc::clone(&log));
    // PendSV 优先级 1（SHPR3 第 2 字节 0x20）、SysTick 优先级 2（第 3 字节 0x40）。
    // PendSV priority 1 (SHPR3 byte 2 = 0x20), SysTick priority 2 (byte 3 = 0x40).
    assert!(ppb.write32(SCB_SHPR3 - map::PPB_BASE, 0x40_2000));
    assert!(ppb.write32(SCB_SHPR2 - map::PPB_BASE, 0xE000_0000));
    assert_eq!(
        log.lock().as_slice(),
        &[(SCB_SHPR3, 0x40_2000), (SCB_SHPR2, 0xE000_0000)]
    );
}
