//! FPU data-movement tests: VLDR/VSTR, VLDM/VSTM, VMOV, VMRS/VMSR, NOCP
//! 浮点数据搬移测试：VLDR/VSTR、VLDM/VSTM、VMOV、VMRS/VMSR、NOCP

use crate::execute_thumb16::StepResult;
use crate::execute_thumb32::test_support::{TestBus, run};
use crate::fpu::{cpacr, fpexc, fpscr};
use crate::registers::CortexM33Registers;

/// Build a register set with the FPU enabled through CPACR
/// 构建一个通过 CPACR 使能 FPU 的寄存器集
fn regs_with_fpu() -> CortexM33Registers {
    let mut regs = CortexM33Registers::new();
    regs.cpacr = cpacr::CP10_CP11_MASK;
    regs
}

// === VMRS / VMSR ===

#[test]
fn vmrs_reads_fpscr() {
    let mut r = regs_with_fpu();
    r.fpu.fpscr = 0x0000_001F;
    assert_eq!(
        run(0xEEF1_0A10, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.read(0), 0x0000_001F);
}

#[test]
fn vmrs_to_apsr_nzcv_copies_condition_flags() {
    let mut r = regs_with_fpu();
    r.fpu.fpscr = fpscr::N | fpscr::C;
    assert_eq!(
        run(0xEEF1_FA10, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert!(r.apsr_n());
    assert!(!r.apsr_z());
    assert!(r.apsr_c());
    assert!(!r.apsr_v());
    // APSR_nzcv form must not write a general-purpose register.
    // APSR_nzcv 形式不得写入通用寄存器。
    assert_eq!(r.read(15), r.pc);
}

#[test]
fn vmsr_writes_fpscr() {
    let mut r = regs_with_fpu();
    r.write(1, 0x0300_0000);
    assert_eq!(
        run(0xEEE1_1A10, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.fpscr, 0x0300_0000);
}

#[test]
fn vmsr_rejects_pc_as_source() {
    let mut r = regs_with_fpu();
    assert_eq!(
        run(0xEEE1_FA10, &mut r, &mut TestBus::new()),
        StepResult::Undefined
    );
}

// === VMOV core <-> FP registers ===

#[test]
fn vmov_core_to_single_enables_fpu_and_stores() {
    let mut r = regs_with_fpu();
    r.write(1, 0xDEAD_BEEF);
    assert_eq!(
        run(0xEE00_1A10, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.read_s(0), 0xDEAD_BEEF);
    assert_ne!(r.fpu.fpexc & fpexc::EN, 0);
}

#[test]
fn vmov_single_to_core() {
    let mut r = regs_with_fpu();
    r.fpu.write_s(3, 0x7FC0_0000);
    assert_eq!(
        run(0xEE11_2A90, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.read(2), 0x7FC0_0000);
}

#[test]
fn vmov_two_core_to_double() {
    let mut r = regs_with_fpu();
    r.write(1, 0x89AB_CDEF);
    r.write(2, 0x0123_4567);
    assert_eq!(
        run(0xEC42_1B18, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.read_d(8), 0x0123_4567_89AB_CDEF);
}

#[test]
fn vmov_double_to_two_core() {
    let mut r = regs_with_fpu();
    r.fpu.write_d(8, 0x0123_4567_89AB_CDEF);
    assert_eq!(
        run(0xEC54_3B18, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.read(3), 0x89AB_CDEF);
    assert_eq!(r.read(4), 0x0123_4567);
}

#[test]
fn vmov_immediate_expands_vfp_encoding() {
    let mut r = regs_with_fpu();
    assert_eq!(
        run(0xEEB6_0A00, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.read_s(0), 0x3F00_0000); // 0.5
    assert_eq!(
        run(0xEEB0_4A0A, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.read_s(8), 0x4050_0000); // 3.25
}

// === VLDR / VSTR ===

#[test]
fn vldr_immediate_offset() {
    let mut r = regs_with_fpu();
    let mut bus = TestBus::new();
    bus.poke_u32(0x104, 0x4142_4300);
    r.write(0, 0x100);
    assert_eq!(run(0xED90_0A01, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.fpu.read_s(0), 0x4142_4300);
}

#[test]
fn vstr_negative_offset() {
    let mut r = regs_with_fpu();
    let mut bus = TestBus::new();
    r.fpu.write_s(16, 0x3F80_0000);
    r.write(1, 0x100);
    assert_eq!(run(0xED01_8A01, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u32(0x0FC), 0x3F80_0000);
}

#[test]
fn vldr_pc_relative_literal() {
    let mut r = regs_with_fpu();
    let mut bus = TestBus::new();
    bus.poke_u32(0x108, 0x4049_0FDB);
    // Inside the core step the PC already points past the instruction.
    // 核心单步执行时 PC 已指向该指令之后。
    r.pc = 0x100;
    assert_eq!(run(0xED9F_0A02, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.fpu.read_s(0), 0x4049_0FDB);
}

#[test]
fn vstr_with_pc_base_is_undefined() {
    let mut r = regs_with_fpu();
    assert_eq!(
        run(0xED0F_8A00, &mut r, &mut TestBus::new()),
        StepResult::Undefined
    );
}

#[test]
fn vldr_double_reads_s_pair() {
    let mut r = regs_with_fpu();
    let mut bus = TestBus::new();
    bus.poke_u32(0x108, 0x89AB_CDEF);
    bus.poke_u32(0x10C, 0x0123_4567);
    r.write(2, 0x100);
    assert_eq!(run(0xED92_1B02, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.fpu.read_d(1), 0x0123_4567_89AB_CDEF);
}

#[test]
fn vstr_double_writes_s_pair() {
    let mut r = regs_with_fpu();
    let mut bus = TestBus::new();
    r.fpu.write_d(2, 0x0123_4567_89AB_CDEF);
    r.write(3, 0x100);
    assert_eq!(run(0xED03_2B04, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u32(0x0F0), 0x89AB_CDEF);
    assert_eq!(bus.peek_u32(0x0F4), 0x0123_4567);
}

#[test]
fn vldr_double_with_absent_registers_is_undefined() {
    let mut r = regs_with_fpu();
    // U=1, D=1 selects D16-D31, which FPv5-SP-D16 does not provide.
    // U=1、D=1 选择 D16-D31，FPv5-SP-D16 不提供这些寄存器。
    assert_eq!(
        run(0xEDD2_1B02, &mut r, &mut TestBus::new()),
        StepResult::Undefined
    );
}

// === VLDM / VSTM ===

#[test]
fn vldm_ia_without_writeback() {
    let mut r = regs_with_fpu();
    let mut bus = TestBus::new();
    for index in 0..4u32 {
        bus.poke_u32(0x100 + index * 4, 0x3F80_0000 + index);
    }
    r.write(0, 0x100);
    assert_eq!(run(0xEC90_0A04, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.fpu.read_s(0), 0x3F80_0000);
    assert_eq!(r.fpu.read_s(3), 0x3F80_0003);
    assert_eq!(r.read(0), 0x100);
}

#[test]
fn vldm_ia_with_writeback_advances_base() {
    let mut r = regs_with_fpu();
    let mut bus = TestBus::new();
    for index in 0..4u32 {
        bus.poke_u32(0x100 + index * 4, 0x4000_0000 + index);
    }
    r.write(1, 0x100);
    assert_eq!(run(0xECB1_2A04, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.fpu.read_s(4), 0x4000_0000);
    assert_eq!(r.fpu.read_s(7), 0x4000_0003);
    assert_eq!(r.read(1), 0x110);
}

#[test]
fn vstm_db_with_writeback_stores_descending_block() {
    let mut r = regs_with_fpu();
    let mut bus = TestBus::new();
    for index in 0..8u8 {
        r.fpu.write_s(8 + index, 0xC000_0000 + u32::from(index));
    }
    r.write(2, 0x140);
    assert_eq!(run(0xED22_4A08, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(2), 0x120);
    assert_eq!(bus.peek_u32(0x120), 0xC000_0000);
    assert_eq!(bus.peek_u32(0x13C), 0xC000_0007);
}

#[test]
fn vpush_and_vpop_use_stack_pointer() {
    let mut r = regs_with_fpu();
    let mut bus = TestBus::new();
    r.sp = 0x800;
    for index in 0..4u8 {
        r.fpu.write_s(16 + index, 0xDEAD_0000 + u32::from(index));
    }
    assert_eq!(run(0xED2D_8A04, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.sp, 0x7F0);
    assert_eq!(bus.peek_u32(0x7F0), 0xDEAD_0000);

    // Clear S20-S23, then pop them back from the same frame.
    // 清空 S20-S23，然后从同一栈帧弹回。
    for index in 0..4u8 {
        r.fpu.write_s(20 + index, 0);
    }
    assert_eq!(run(0xECBD_AA04, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.sp, 0x800);
    assert_eq!(r.fpu.read_s(20), 0xDEAD_0000);
}

#[test]
fn vldm_and_vstm_double_use_d_view() {
    let mut r = regs_with_fpu();
    let mut bus = TestBus::new();
    bus.poke_u32(0x100, 0x1111_1111);
    bus.poke_u32(0x104, 0x2222_2222);
    bus.poke_u32(0x108, 0x3333_3333);
    bus.poke_u32(0x10C, 0x4444_4444);
    r.write(3, 0x100);
    assert_eq!(run(0xEC93_0B08, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.fpu.read_d(0), 0x2222_2222_1111_1111);
    assert_eq!(r.fpu.read_d(1), 0x4444_4444_3333_3333);

    r.write(4, 0x140);
    assert_eq!(run(0xECA4_0B08, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(4), 0x160);
    assert_eq!(bus.peek_u32(0x140), 0x1111_1111);
    assert_eq!(bus.peek_u32(0x14C), 0x4444_4444);

    r.write(5, 0x200);
    assert_eq!(run(0xED25_8B10, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(5), 0x1C0);
}

#[test]
fn vldm_with_zero_count_is_undefined() {
    let mut r = regs_with_fpu();
    assert_eq!(
        run(0xEC90_0A00, &mut r, &mut TestBus::new()),
        StepResult::Undefined
    );
}

#[test]
fn vldm_single_overflowing_register_file_is_undefined() {
    let mut r = regs_with_fpu();
    // s31 plus one more would require s32. / s31 再加载一个就需要 s32。
    assert_eq!(
        run(0xEC9F_8A02, &mut r, &mut TestBus::new()),
        StepResult::Undefined
    );
}

// === NOCP ===

#[test]
fn fpu_access_without_cpacr_rewinds_pc_and_raises_usage_fault() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x100, 0x4142_4300);
    r.write(0, 0x100);
    r.pc = 0x104;
    assert_eq!(run(0xED90_0A01, &mut r, &mut bus), StepResult::Ok);
    // The stacked PC must point at the faulting instruction.
    // 压栈的 PC 必须指向引发故障的指令。
    assert_eq!(r.pc, 0x100);
    assert!(r.pending_usage_fault);
    assert_eq!(r.fpu.fpexc & fpexc::EN, 0, "FPEXC.EN must stay clear");
    assert_eq!(r.fpu.read_s(0), 0, "no data may be transferred");
}
