//! Data-processing tests: modified immediate, shifted register, register
//! shifts, extend, reverse
//! 数据处理测试：改良立即数、移位寄存器、寄存器移位、扩展、反转

use crate::execute_thumb16::StepResult;
use crate::execute_thumb32::test_support::execute;
use crate::registers::CortexM33Registers;

// === Data-processing (modified immediate) ===

#[test]
fn mov_w_zero_immediate() {
    let mut r = CortexM33Registers::new();
    r.write(0, 0xFFFF_FFFF);
    assert_eq!(execute(0xF04F_0000, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0);
}

#[test]
fn mov_w_rotated_immediate() {
    let mut r = CortexM33Registers::new();
    assert_eq!(execute(0xF04F_4050, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xD000_0000);
}

#[test]
fn movs_w_sets_flags_and_preserves_carry() {
    let mut r = CortexM33Registers::new();
    r.set_apsr_nzcv(false, false, true, false);
    assert_eq!(execute(0xF05F_4050, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xD000_0000);
    assert!(r.apsr_n());
    assert!(!r.apsr_z());
    assert!(r.apsr_c());
}

#[test]
fn mov_w_replicated_byte() {
    let mut r = CortexM33Registers::new();
    assert_eq!(execute(0xF04F_105A, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x005A_005A);
}

#[test]
fn mov_w_unpredictable_replication_is_undefined() {
    let mut r = CortexM33Registers::new();
    assert_eq!(execute(0xF04F_1000, &mut r), StepResult::Undefined);
}

#[test]
fn mvn_w_inverts_immediate() {
    let mut r = CortexM33Registers::new();
    assert_eq!(execute(0xF06F_0001, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_FFFE);
}

#[test]
fn and_w_immediate() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0xF0F0_FFFF);
    assert_eq!(execute(0xF001_000F, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x0000_000F);
}

#[test]
fn bic_w_immediate() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0xFFFF_FFFF);
    assert_eq!(execute(0xF021_000F, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_FFF0);
}

#[test]
fn orr_w_immediate() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x8000_0000);
    assert_eq!(execute(0xF041_000F, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x8000_000F);
}

#[test]
fn tst_w_immediate_sets_zero_flag() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0xF0);
    assert_eq!(execute(0xF011_0F0F, &mut r), StepResult::Ok);
    assert!(r.apsr_z());
}

#[test]
fn teq_w_immediate_sets_zero_flag() {
    let mut r = CortexM33Registers::new();
    r.write(1, 1);
    assert_eq!(execute(0xF091_0F01, &mut r), StepResult::Ok);
    assert!(r.apsr_z());
}

#[test]
fn cmp_w_immediate_sets_flags() {
    let mut r = CortexM33Registers::new();
    r.write(1, 5);
    assert_eq!(execute(0xF1B1_0F01, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0);
    assert!(r.apsr_c());
    assert!(!r.apsr_z());
}

#[test]
fn cmn_w_immediate_sets_flags() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0xFFFF_FFFF);
    assert_eq!(execute(0xF111_0F01, &mut r), StepResult::Ok);
    assert!(r.apsr_z());
    assert!(r.apsr_c());
}

#[test]
fn adc_w_immediate_uses_carry_in() {
    let mut r = CortexM33Registers::new();
    r.write(1, 1);
    r.set_apsr_nzcv(false, false, true, false);
    assert_eq!(execute(0xF141_0002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 4);
}

#[test]
fn sbc_w_immediate_uses_carry_in() {
    let mut r = CortexM33Registers::new();
    r.write(1, 10);
    // C == 1 means "no borrow", so SBC subtracts only the immediate.
    // C == 1 表示“无借位”，因此 SBC 只减去立即数。
    r.set_apsr_nzcv(false, false, true, false);
    assert_eq!(execute(0xF161_0002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 8);
}

#[test]
fn rsb_w_immediate_reverses_operands() {
    let mut r = CortexM33Registers::new();
    r.write(1, 5);
    assert_eq!(execute(0xF1C1_0002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_FFFD);
}

#[test]
fn sub_w_sp_immediate_256() {
    let mut r = CortexM33Registers::new();
    r.sp = 0x2008_2000;
    assert_eq!(execute(0xF5AD_7D80, &mut r), StepResult::Ok);
    assert_eq!(r.sp, 0x2008_1F00);
}

#[test]
fn add_w_sp_immediate_128() {
    let mut r = CortexM33Registers::new();
    r.sp = 0x2008_0000;
    assert_eq!(execute(0xF10D_0D80, &mut r), StepResult::Ok);
    assert_eq!(r.sp, 0x2008_0080);
}

// === Data-processing (shifted register) ===

#[test]
fn and_w_basic() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0xF0F0_FFFF);
    r.write(2, 0x0F0F_0F0F);
    assert_eq!(execute(0xEA01_0002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x0000_0F0F);
}

#[test]
fn ands_w_sets_flags() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0xFFFF_FFFF);
    r.write(2, 0x8000_0000);
    assert_eq!(execute(0xEA11_0002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x8000_0000);
    assert!(r.apsr_n());
    assert!(!r.apsr_z());
}

#[test]
fn orr_w_with_pc_operand_is_mov() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x1234_5678);
    assert_eq!(execute(0xEA4F_0001, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x1234_5678);
}

#[test]
fn orn_w_with_pc_operand_is_mvn() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0000_00FF);
    assert_eq!(execute(0xEA6F_0001, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_FF00);
}

#[test]
fn bic_w_basic() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0xFFFF_0000);
    r.write(2, 0x0000_FFFF);
    assert_eq!(execute(0xEA21_0002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_0000);
}

#[test]
fn eor_w_basic() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0xFFFF_0000);
    r.write(2, 0x00FF_00FF);
    assert_eq!(execute(0xEA81_0002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFF00_00FF);
}

#[test]
fn add_w_does_not_set_flags() {
    let mut r = CortexM33Registers::new();
    r.write(1, 1);
    r.write(2, 2);
    assert_eq!(execute(0xEB01_0002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 3);
    assert!(!r.apsr_n() && !r.apsr_z() && !r.apsr_c() && !r.apsr_v());
}

#[test]
fn adds_w_sets_carry() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0xFFFF_FFFF);
    r.write(2, 2);
    assert_eq!(execute(0xEB11_0002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 1);
    assert!(r.apsr_c());
    assert!(!r.apsr_v());
}

#[test]
fn subs_w_sets_carry() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x1000);
    r.write(2, 1);
    assert_eq!(execute(0xEBB1_0002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x0FFF);
    assert!(r.apsr_c());
}

#[test]
fn rsb_w_reverses_operands() {
    let mut r = CortexM33Registers::new();
    r.write(1, 5);
    r.write(2, 10);
    assert_eq!(execute(0xEBC1_0002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 5);
}

#[test]
fn adc_w_uses_carry_in() {
    let mut r = CortexM33Registers::new();
    r.write(1, 1);
    r.write(2, 2);
    r.set_apsr_nzcv(false, false, true, false);
    assert_eq!(execute(0xEB41_0002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 4);
}

#[test]
fn sbc_w_uses_carry_in() {
    let mut r = CortexM33Registers::new();
    r.write(1, 10);
    r.write(2, 3);
    r.set_apsr_nzcv(false, false, true, false);
    assert_eq!(execute(0xEB61_0002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 7);
}

#[test]
fn cmp_w_only_sets_flags() {
    let mut r = CortexM33Registers::new();
    r.write(1, 5);
    r.write(2, 5);
    assert_eq!(execute(0xEBB1_0F02, &mut r), StepResult::Ok);
    assert!(r.apsr_z());
    assert!(r.apsr_c());
    assert_eq!(r.read(0), 0);
}

#[test]
fn cmn_w_adds_and_sets_flags() {
    let mut r = CortexM33Registers::new();
    r.write(1, 1);
    r.write(2, 1);
    assert_eq!(execute(0xEB11_0F02, &mut r), StepResult::Ok);
    assert!(!r.apsr_z());
    assert!(!r.apsr_c());
}

#[test]
fn tst_w_sets_zero_flag() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0F);
    r.write(2, 0xF0);
    assert_eq!(execute(0xEA11_0F02, &mut r), StepResult::Ok);
    assert!(r.apsr_z());
}

#[test]
fn teq_w_sets_zero_flag() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0xF0);
    r.write(2, 0xF0);
    assert_eq!(execute(0xEA91_0F02, &mut r), StepResult::Ok);
    assert!(r.apsr_z());
}

#[test]
fn rrx_through_carry() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0000_0002);
    r.set_apsr_nzcv(false, false, true, false);
    assert_eq!(execute(0xEA4F_0031, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x8000_0001);
}

// === Data-processing (register): shifts, extend, reverse, count ===

#[test]
fn lsl_w_register_amount() {
    let mut r = CortexM33Registers::new();
    r.write(1, 1);
    r.write(2, 4);
    assert_eq!(execute(0xFA01_F002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 16);
}

#[test]
fn lsls_w_sets_carry_and_zero() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x8000_0000);
    r.write(2, 1);
    assert_eq!(execute(0xFA11_F002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0);
    assert!(r.apsr_c());
    assert!(r.apsr_z());
}

#[test]
fn lsr_w_register_amount() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x10);
    r.write(2, 2);
    assert_eq!(execute(0xFA21_F002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 4);
}

#[test]
fn asr_w_sign_fills() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x8000_0000);
    r.write(2, 1);
    assert_eq!(execute(0xFA41_F002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xC000_0000);
}

#[test]
fn ror_w_rotates() {
    let mut r = CortexM33Registers::new();
    r.write(1, 1);
    r.write(2, 1);
    assert_eq!(execute(0xFA61_F002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x8000_0000);
}

#[test]
fn register_shift_by_zero_keeps_value() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0xFF);
    r.write(2, 0);
    assert_eq!(execute(0xFA21_F002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFF);
}

#[test]
fn clz_counts_leading_zeros() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0000_0100);
    assert_eq!(execute(0xFAB1_F081, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 23);
}

#[test]
fn rbit_reverses_bits() {
    let mut r = CortexM33Registers::new();
    r.write(1, 1);
    assert_eq!(execute(0xFA91_F0A1, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x8000_0000);
}

#[test]
fn rev_w_swaps_bytes() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x1234_5678);
    assert_eq!(execute(0xFA91_F081, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x7856_3412);
}

#[test]
fn rev16_w_swaps_bytes_in_halfwords() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x1234_5678);
    assert_eq!(execute(0xFA91_F091, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x3412_7856);
}

#[test]
fn revsh_w_sign_extends_swapped_byte() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0000_0080);
    assert_eq!(execute(0xFA91_F0B1, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_8000);
}

#[test]
fn uxtab_adds_the_zero_extended_byte() {
    // UXTAB r6, r3, r6 — the exact instruction CircuitPython executes while
    // assembling its USB descriptors.
    // UXTAB r6, r3, r6 —— CircuitPython 在组装 USB 描述符时执行的指令。
    let mut r = CortexM33Registers::new();
    r.write(3, 0x1000);
    r.write(6, 0xFFFF_FF00);
    assert_eq!(execute(0xFA53_F686, &mut r), StepResult::Ok);
    assert_eq!(r.read(6), 0x1000);
}

#[test]
fn uxtab_honours_the_rotate_field() {
    let mut r = CortexM33Registers::new();
    r.write(3, 0x1000);
    r.write(6, 0xFFFF_FF00);
    // bits[5:4] = 1 rotates Rm right by 8 before zero-extending the low byte.
    // bits[5:4] = 1 先把 Rm 循环右移 8 位，再零扩展低字节。
    assert_eq!(execute(0xFA53_F696, &mut r), StepResult::Ok);
    assert_eq!(r.read(6), 0x10FF);
}

#[test]
fn sxtb_w_sign_extends() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x80);
    assert_eq!(execute(0xFA4F_F081, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_FF80);
}

#[test]
fn uxtb_w_zero_extends() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x80);
    assert_eq!(execute(0xFA5F_F081, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x80);
}

#[test]
fn sxtb_w_plain_form_updates_nz_flags() {
    // Plain SXTB (Rn = 15) must update N/Z but leave C and V untouched.
    // 普通 SXTB（Rn = 15）必须更新 N/Z，但保持 C 和 V 不变。
    let mut r = CortexM33Registers::new();
    r.write(1, 0x80);
    r.set_apsr_nzcv(false, false, false, true);
    assert_eq!(execute(0xFA4F_F081, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_FF80);
    assert!(r.apsr_n());
    assert!(!r.apsr_z());
    assert!(!r.apsr_c());
    assert!(r.apsr_v());
}

#[test]
fn uxth_w_plain_form_sets_zero_flag() {
    // Plain UXTH (Rn = 15) with a zero result must set Z and clear N.
    // 普通 UXTH（Rn = 15）在结果为零时必须置 Z 并清 N。
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0001_0000);
    r.set_apsr_nzcv(true, false, true, true);
    assert_eq!(execute(0xFA1F_F081, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0);
    assert!(!r.apsr_n());
    assert!(r.apsr_z());
    assert!(r.apsr_c());
    assert!(r.apsr_v());
}

#[test]
fn uxtab_accumulating_form_leaves_flags_unchanged() {
    // Accumulating UXTAB (Rn != 15) must not update any condition flag.
    // 累加形式 UXTAB（Rn != 15）不得更新任何条件标志。
    let mut r = CortexM33Registers::new();
    r.write(3, 0x1000);
    r.write(6, 0xFFFF_FF00);
    r.set_apsr_nzcv(true, true, true, true);
    assert_eq!(execute(0xFA53_F686, &mut r), StepResult::Ok);
    assert_eq!(r.read(6), 0x1000);
    assert!(r.apsr_n() && r.apsr_z() && r.apsr_c() && r.apsr_v());
}

#[test]
fn sxth_w_sign_extends() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x8000);
    assert_eq!(execute(0xFA0F_F081, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_8000);
}

#[test]
fn uxth_w_zero_extends() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x8000);
    assert_eq!(execute(0xFA1F_F081, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x8000);
}
