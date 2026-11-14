//! Single-precision arithmetic, comparison and conversion tests
//! 单精度算术、比较与转换测试

use super::*;

// === Arithmetic (Stage 3) ===

#[test]
fn vadd_adds_signed_values_including_high_registers() {
    let mut r = regs_with_fpu();
    write_f32(&mut r, 1, 1.5);
    write_f32(&mut r, 2, -2.25);
    assert_eq!(
        run(0xEE30_0A81, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), -0.75);

    // s31 = s30 + s29 exercises the D/N/M encoding bits.
    // s31 = s30 + s29 覆盖 D/N/M 编码位。
    write_f32(&mut r, 30, 1.0);
    write_f32(&mut r, 29, 2.0);
    assert_eq!(
        run(0xEE7F_FA2E, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 31), 3.0);
}

#[test]
fn vsub_produces_positive_zero_and_negative_results() {
    let mut r = regs_with_fpu();
    write_f32(&mut r, 1, 5.0);
    write_f32(&mut r, 2, 5.0);
    assert_eq!(
        run(0xEE30_0AC1, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.read_s(0), 0.0f32.to_bits(), "5.0 - 5.0 is +0.0");

    write_f32(&mut r, 1, -1.25);
    write_f32(&mut r, 2, 2.5);
    assert_eq!(
        run(0xEE30_0AC1, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), -3.75);
}

#[test]
fn vmul_handles_infinity_and_invalid_zero_times_infinity() {
    let mut r = regs_with_fpu();
    write_f32(&mut r, 1, f32::INFINITY);
    write_f32(&mut r, 2, 2.0);
    assert_eq!(
        run(0xEE20_0A81, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), f32::INFINITY);
    assert_eq!(r.fpu.fpscr & fpscr::IOC, 0, "inf * 2 is exact");

    write_f32(&mut r, 1, 0.0);
    write_f32(&mut r, 2, f32::NEG_INFINITY);
    assert_eq!(
        run(0xEE20_0A81, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert!(read_f32(&r, 0).is_nan(), "0 * inf is a NaN");
    assert_ne!(r.fpu.fpscr & fpscr::IOC, 0, "invalid operation sets IOC");
}

#[test]
fn vnmul_negates_the_product_and_propagates_nan() {
    let mut r = regs_with_fpu();
    write_f32(&mut r, 1, 3.0);
    write_f32(&mut r, 2, 4.0);
    assert_eq!(
        run(0xEE20_0AC1, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), -12.0);

    write_f32(&mut r, 1, f32::NAN);
    write_f32(&mut r, 2, 2.0);
    assert_eq!(
        run(0xEE20_0AC1, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert!(read_f32(&r, 0).is_nan(), "NaN operands propagate");
}

#[test]
fn vdiv_quotient_divide_by_zero_and_invalid_operation() {
    let mut r = regs_with_fpu();
    write_f32(&mut r, 1, 7.0);
    write_f32(&mut r, 2, 2.0);
    assert_eq!(
        run(0xEE80_0A81, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), 3.5);
    assert_eq!(r.fpu.fpscr & fpscr::EXCEPTION_FLAGS, 0);

    write_f32(&mut r, 1, 1.0);
    write_f32(&mut r, 2, 0.0);
    assert_eq!(
        run(0xEE80_0A81, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), f32::INFINITY);
    assert_ne!(r.fpu.fpscr & fpscr::DZC, 0, "finite / 0 sets DZC");

    write_f32(&mut r, 1, 0.0);
    assert_eq!(
        run(0xEE80_0A81, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert!(read_f32(&r, 0).is_nan(), "0 / 0 is a NaN");
    assert_ne!(r.fpu.fpscr & fpscr::IOC, 0, "0 / 0 sets IOC");
}

#[test]
fn vsqrt_of_two_and_negative_radicand() {
    let mut r = regs_with_fpu();
    write_f32(&mut r, 1, 2.0);
    assert_eq!(
        run(0xEEB1_0AE0, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.read_s(0), 0x3FB5_04F3, "sqrt(2) in binary32");

    write_f32(&mut r, 1, -4.0);
    assert_eq!(
        run(0xEEB1_0AE0, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert!(read_f32(&r, 0).is_nan(), "sqrt(-4) is a NaN");
    assert_ne!(r.fpu.fpscr & fpscr::IOC, 0, "sqrt(-4) sets IOC");
}

#[test]
fn vabs_and_vneg_operate_on_the_sign_bit() {
    let mut r = regs_with_fpu();
    write_f32(&mut r, 1, -3.5);
    assert_eq!(
        run(0xEEB0_0AE0, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), 3.5);

    // s16 = |s31| exercises the D and M register bits. / s16 = |s31| 覆盖 D 与 M 位。
    write_f32(&mut r, 31, -0.0);
    assert_eq!(
        run(0xEEB0_8AEF, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.read_s(16), 0.0f32.to_bits(), "|−0.0| is +0.0");

    write_f32(&mut r, 1, 3.5);
    assert_eq!(
        run(0xEEB1_0A60, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), -3.5);
}

#[test]
fn vmov_register_to_register_copies_bits_verbatim() {
    let mut r = regs_with_fpu();
    r.fpu.write_s(1, 0x7FC0_1234); // quiet NaN payload / 静默 NaN 载荷
    assert_eq!(
        run(0xEEB0_0A60, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.read_s(0), 0x7FC0_1234);
}

#[test]
fn vmla_and_vmls_accumulate_into_the_destination() {
    let mut r = regs_with_fpu();
    write_f32(&mut r, 0, 0.5);
    write_f32(&mut r, 1, 2.0);
    write_f32(&mut r, 2, 3.0);
    assert_eq!(
        run(0xEE00_0A81, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), 6.5);

    write_f32(&mut r, 0, 0.5);
    assert_eq!(
        run(0xEE00_0AC1, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), -5.5);
}

#[test]
fn vnmla_and_vnmls_negate_the_accumulator() {
    let mut r = regs_with_fpu();
    write_f32(&mut r, 0, 1.0);
    write_f32(&mut r, 1, 2.0);
    write_f32(&mut r, 2, 3.0);
    assert_eq!(
        run(0xEE10_0AC1, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), -7.0, "VNMLA: -(1 + 2*3)");

    write_f32(&mut r, 0, 1.0);
    assert_eq!(
        run(0xEE10_0A81, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), 5.0, "VNMLS: -(1 - 2*3)");
}

#[test]
fn vfma_uses_a_single_rounding() {
    let mut r = regs_with_fpu();
    write_f32(&mut r, 0, -1.0);
    // -1 + (1 + 2^-23) * (1 - 2^-24) == 2^-24 - 2^-47 exactly; an unfused
    // multiply-add would round the product to 1.0 first and yield +0.0.
    // -1 + (1 + 2^-23) * (1 - 2^-24) 精确等于 2^-24 - 2^-47；非融合乘加会
    // 先把乘积舍入为 1.0，结果为 +0.0。
    r.fpu.write_s(1, 0x3F80_0001);
    r.fpu.write_s(2, 0x3F7F_FFFF);
    assert_eq!(
        run(0xEEA0_0A81, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.read_s(0), 0x337F_FFFE);

    write_f32(&mut r, 0, 1.0);
    assert_eq!(
        run(0xEEA0_0AC1, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.read_s(0), 0xB37F_FFFE, "VFMS negates the product");
}

#[test]
fn vfnma_and_vfnms_negate_the_accumulator() {
    let mut r = regs_with_fpu();
    write_f32(&mut r, 0, 1.0);
    write_f32(&mut r, 1, 2.0);
    write_f32(&mut r, 2, 3.0);
    assert_eq!(
        run(0xEE90_0AC1, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), -7.0, "VFNMA: -(1 + 2*3)");

    write_f32(&mut r, 0, 1.0);
    assert_eq!(
        run(0xEE90_0A81, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), 5.0, "VFNMS: -1 + 2*3");
}

// === Comparison (Stage 3) ===

/// Run `VCMP.F32 S0, S1` followed by `VMRS APSR_nzcv, FPSCR`
/// 执行 `VCMP.F32 S0, S1` 后接 `VMRS APSR_nzcv, FPSCR`
fn compare_to_apsr(
    regs: &mut CortexM33Registers,
    left: f32,
    right: f32,
) -> (bool, bool, bool, bool) {
    write_f32(regs, 0, left);
    write_f32(regs, 1, right);
    assert_eq!(run(0xEEB4_0A60, regs, &mut TestBus::new()), StepResult::Ok);
    assert_eq!(run(0xEEF1_FA10, regs, &mut TestBus::new()), StepResult::Ok);
    (regs.apsr_n(), regs.apsr_z(), regs.apsr_c(), regs.apsr_v())
}

#[test]
fn vcmp_reports_less_equal_greater_and_unordered() {
    let mut r = regs_with_fpu();
    assert_eq!(
        compare_to_apsr(&mut r, 1.0, 2.0),
        (true, false, false, false)
    );
    assert_eq!(
        compare_to_apsr(&mut r, 1.0, 1.0),
        (false, true, true, false)
    );
    assert_eq!(
        compare_to_apsr(&mut r, 2.0, 1.0),
        (false, false, true, false)
    );
    assert_eq!(
        compare_to_apsr(&mut r, f32::NAN, 1.0),
        (false, false, true, true)
    );
    assert_ne!(r.fpu.fpscr & fpscr::IOC, 0, "unordered compare sets IOC");
}

#[test]
fn vcmp_with_zero_sets_fpscr_flags_for_signed_zero() {
    let mut r = regs_with_fpu();
    write_f32(&mut r, 0, -2.0);
    assert_eq!(
        run(0xEEB5_0A40, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.fpscr & fpscr::N, fpscr::N);
    assert_eq!(r.fpu.fpscr & (fpscr::Z | fpscr::C | fpscr::V), 0);

    write_f32(&mut r, 0, 0.0);
    assert_eq!(
        run(0xEEB5_0AC0, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.fpscr & fpscr::CONDITION_FLAGS, fpscr::Z | fpscr::C);
}

// === Conversion (Stage 3) ===

#[test]
fn vcvt_signed_float_to_integer_truncates_toward_zero() {
    let mut r = regs_with_fpu();
    write_f32(&mut r, 1, -3.75);
    assert_eq!(
        run(0xEEBD_0AE0, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.read_s(0), (-3i32) as u32);
    assert_ne!(r.fpu.fpscr & fpscr::IXC, 0, "lost fraction sets IXC");

    write_f32(&mut r, 1, 2.9);
    assert_eq!(
        run(0xEEBD_0AE0, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.read_s(0), 2);
}

#[test]
fn vcvt_unsigned_float_to_integer_handles_out_of_range() {
    let mut r = regs_with_fpu();
    write_f32(&mut r, 1, 3.75);
    assert_eq!(
        run(0xEEBC_0AE0, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.read_s(0), 3);

    r.fpu.fpscr = 0;
    write_f32(&mut r, 1, -1.0);
    assert_eq!(
        run(0xEEBC_0AE0, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(
        r.fpu.read_s(0),
        0,
        "negative input yields the unsigned indefinite"
    );
    assert_ne!(r.fpu.fpscr & fpscr::IOC, 0);

    r.fpu.fpscr = 0;
    write_f32(&mut r, 1, f32::NAN);
    assert_eq!(
        run(0xEEBD_0AE0, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.read_s(0), 0, "NaN yields zero");
    assert_ne!(r.fpu.fpscr & fpscr::IOC, 0);

    r.fpu.fpscr = 0;
    write_f32(&mut r, 1, 1.0e20);
    assert_eq!(
        run(0xEEBD_0AE0, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(
        r.fpu.read_s(0),
        i32::MIN as u32,
        "out-of-range signed indefinite"
    );
    assert_ne!(r.fpu.fpscr & fpscr::IOC, 0);
}

#[test]
fn vcvtr_rounds_ties_to_even() {
    let mut r = regs_with_fpu();
    write_f32(&mut r, 1, 2.5);
    assert_eq!(
        run(0xEEBD_0A60, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.read_s(0), 2);

    write_f32(&mut r, 1, 3.5);
    assert_eq!(
        run(0xEEBD_0A60, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.read_s(0), 4);
}

#[test]
fn vcvt_integer_to_float_handles_signed_and_unsigned_sources() {
    let mut r = regs_with_fpu();
    r.fpu.write_s(1, (-7i32) as u32);
    assert_eq!(
        run(0xEEB8_0AE0, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), -7.0);
    assert_eq!(r.fpu.fpscr & fpscr::IXC, 0, "-7 is exact in binary32");

    r.fpu.write_s(1, 3_000_000_001);
    assert_eq!(
        run(0xEEB8_0A60, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.fpu.read_s(0), 3_000_000_001.0_f32.to_bits());
    assert_eq!(read_f32(&r, 0), 3_000_000_001.0_f32);
    assert_ne!(
        r.fpu.fpscr & fpscr::IXC,
        0,
        "3e9 + 1 is inexact in binary32"
    );
}

// === Stage 3 conditional select ===

#[test]
fn vsel_selects_by_the_fpscr_condition_flags() {
    let mut r = regs_with_fpu();
    write_f32(&mut r, 1, 1.5);
    write_f32(&mut r, 2, -2.5);

    // VSELEQ: Z == 1 selects Vn.
    // VSELEQ：Z == 1 时选择 Vn。
    r.fpu.fpscr = fpscr::Z;
    assert_eq!(
        run(0xFE00_0A81, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), 1.5);

    // VSELEQ: Z == 0 selects Vm.
    // VSELEQ：Z == 0 时选择 Vm。
    r.fpu.fpscr = 0;
    assert_eq!(
        run(0xFE00_0A81, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), -2.5);

    // VSELGT: Z == 0 && N == V selects Vn.
    // VSELGT：Z == 0 且 N == V 时选择 Vn。
    assert_eq!(
        run(0xFE30_0A81, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), 1.5);

    // VSELGE: N != V selects Vm.
    // VSELGE：N != V 时选择 Vm。
    r.fpu.fpscr = fpscr::N;
    assert_eq!(
        run(0xFE20_0A81, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), -2.5);

    // VSELVS: V == 1 selects Vn.
    // VSELVS：V == 1 时选择 Vn。
    r.fpu.fpscr = fpscr::V;
    assert_eq!(
        run(0xFE10_0A81, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), 1.5);
}

// === Stage 3 round to integral ===

#[test]
fn vrint_rounds_in_the_requested_direction() {
    let mut r = regs_with_fpu();
    // VRINTM (0xFE): toward minus infinity. / VRINTM（0xFE）：向负无穷。
    write_f32(&mut r, 1, -2.25);
    assert_eq!(
        run(0xFEBB_0A60, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), -3.0);

    // VRINTP (0xFE): toward plus infinity. / VRINTP（0xFE）：向正无穷。
    write_f32(&mut r, 1, 2.25);
    assert_eq!(
        run(0xFEBA_0A60, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), 3.0);

    // VRINTA (0xFE): ties away from zero. / VRINTA（0xFE）：就近、远离零。
    write_f32(&mut r, 1, -2.5);
    assert_eq!(
        run(0xFEB8_0A60, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), -3.0);

    // VRINTN (0xFE): ties to even. / VRINTN（0xFE）：就近、偶数优先。
    write_f32(&mut r, 1, 2.5);
    assert_eq!(
        run(0xFEB9_0A60, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), 2.0);

    // VRINTZ (0xEE): toward zero. / VRINTZ（0xEE）：向零。
    write_f32(&mut r, 1, -2.75);
    assert_eq!(
        run(0xEEB6_0AE0, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), -2.0);

    // VRINTR (0xEE): FPSCR rounding mode, modeled as ties-to-even.
    // VRINTR（0xEE）：按 FPSCR 舍入模式，此处按就近偶数建模。
    write_f32(&mut r, 1, 3.5);
    assert_eq!(
        run(0xEEB6_0A60, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(read_f32(&r, 0), 4.0);
}

// === Stage 3 faults and undefined encodings ===

#[test]
fn arithmetic_without_cpacr_raises_nocp_usage_fault() {
    let mut r = CortexM33Registers::new();
    write_f32(&mut r, 1, 1.5);
    write_f32(&mut r, 2, 2.5);
    r.pc = 0x104;
    assert_eq!(
        run(0xEE30_0A81, &mut r, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(r.pc, 0x100, "PC rewinds to the faulting instruction");
    assert!(r.pending_usage_fault);
    assert_eq!(r.fpu.fpexc & fpexc::EN, 0, "FPEXC.EN must stay clear");
    assert_eq!(r.fpu.read_s(0), 0, "no arithmetic may be performed");
}

#[test]
fn unimplemented_encodings_are_reported_undefined() {
    let mut r = regs_with_fpu();
    // VDIV with the sub-opcode bit set is not a valid encoding.
    // VDIV 置位子操作码位不是合法编码。
    assert_eq!(
        run(0xEE80_0AC1, &mut r, &mut TestBus::new()),
        StepResult::Undefined
    );
    // Half-precision conversion (VCVTB.F32.F16) is out of scope.
    // 半精度转换（VCVTB.F32.F16）不在实现范围内。
    assert_eq!(
        run(0xEEB2_0AE0, &mut r, &mut TestBus::new()),
        StepResult::Undefined
    );
    // Double-precision VCVT.F64.F32 is storage-only in this emulator.
    // 双精度 VCVT.F64.F32 在本模拟器中仅作存储。
    assert_eq!(
        run(0xEEB7_0BE0, &mut r, &mut TestBus::new()),
        StepResult::Undefined
    );
    // bit[4] set is not a data-processing encoding at all.
    // bit[4] 置位根本不是数据处理编码。
    assert_eq!(
        run(0xEE30_0A91, &mut r, &mut TestBus::new()),
        StepResult::Undefined
    );
}
