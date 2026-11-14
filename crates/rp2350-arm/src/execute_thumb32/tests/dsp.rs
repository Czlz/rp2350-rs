//! DSP packed SIMD tests: parallel add/sub, SXTB16/UXTB16, USAD8/USADA8
//! DSP 打包 SIMD 测试：并行加/减、SXTB16/UXTB16、USAD8/USADA8

use crate::execute_thumb16::StepResult;
use crate::execute_thumb32::test_support::execute;
use crate::registers::CortexM33Registers;

// === Parallel byte add/subtract ===

#[test]
fn sadd8_adds_bytes_and_sets_all_ge_bits() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0102_0304);
    r.write(2, 0x0101_0101);
    assert_eq!(execute(0xFA81_F002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x0203_0405);
    assert_eq!(r.apsr_ge(), 0xF);
}

#[test]
fn sadd8_uses_full_width_sign_for_ge() {
    // 0x7F + 0x01 = 0x80: the wrapped byte is negative but the full-width sum
    // is positive, so GE[0] must stay set.
    // 0x7F + 0x01 = 0x80：回绕后的字节为负，但完整宽度和为正，GE[0] 必须保持置位。
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0000_007F);
    r.write(2, 0x0000_0001);
    assert_eq!(execute(0xFA81_F002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x0000_0080);
    assert_eq!(r.apsr_ge(), 0xF);
}

#[test]
fn ssub8_sets_ge_per_byte() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0080_0102);
    r.write(2, 0x0001_0201);
    assert_eq!(execute(0xFAC1_F002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x007F_FF01);
    // Lane 0: 1 >= 0, lane 1: -1 < 0, lane 2: -129 < 0, lane 3: 0 >= 0.
    // 通道 0：1 >= 0，通道 1：-1 < 0，通道 2：-129 < 0，通道 3：0 >= 0。
    assert_eq!(r.apsr_ge(), 0b1001);
}

#[test]
fn uadd8_sets_ge_on_carry_per_byte() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x00FE_01F0);
    r.write(2, 0x0001_0220);
    assert_eq!(execute(0xFA81_F042, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x00FF_0310);
    // Only lane 0 produced a carry (0xF0 + 0x20 = 0x110).
    // 仅通道 0 产生进位（0xF0 + 0x20 = 0x110）。
    assert_eq!(r.apsr_ge(), 0b0001);
}

#[test]
fn usub8_sets_ge_when_no_borrow() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x3020_1000);
    r.write(2, 0x2F21_1001);
    assert_eq!(execute(0xFAC1_F042, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x01FF_00FF);
    // Lanes 1 and 3 have no borrow.
    // 通道 1 与 3 无借位。
    assert_eq!(r.apsr_ge(), 0b1010);
}

// === Parallel halfword add/subtract ===

#[test]
fn sadd16_sets_ge_pairs() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x8000_0001);
    r.write(2, 0x0000_0002);
    assert_eq!(execute(0xFA91_F002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x8000_0003);
    // Low halfword sum 3 >= 0 -> GE[1:0] = 11; high sum -32768 -> GE[3:2] = 00.
    // 低半字和 3 >= 0 -> GE[1:0] = 11；高半字和 -32768 -> GE[3:2] = 00。
    assert_eq!(r.apsr_ge(), 0b0011);
}

#[test]
fn ssub16_sets_ge_pairs() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0005_8000);
    r.write(2, 0x0003_0001);
    assert_eq!(execute(0xFAD1_F002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x0002_7FFF);
    // Low difference -32769 < 0 -> GE[1:0] = 00; high difference 2 -> GE[3:2] = 11.
    // 低半字差 -32769 < 0 -> GE[1:0] = 00；高半字差 2 -> GE[3:2] = 11。
    assert_eq!(r.apsr_ge(), 0b1100);
}

#[test]
fn uadd16_sets_ge_on_carry_per_halfword() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0001_FFFF);
    r.write(2, 0x0001_0001);
    assert_eq!(execute(0xFA91_F042, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x0002_0000);
    assert_eq!(r.apsr_ge(), 0b0011);
}

#[test]
fn usub16_sets_ge_when_no_borrow() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0005_0001);
    r.write(2, 0x0003_0002);
    assert_eq!(execute(0xFAD1_F042, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x0002_FFFF);
    assert_eq!(r.apsr_ge(), 0b1100);
}

#[test]
fn parallel_add_sub_leaves_nzcvq_untouched() {
    let mut r = CortexM33Registers::new();
    r.set_apsr_nzcv(true, false, true, false);
    r.set_apsr_q(true);
    let saved = r.xpsr & 0xF800_0000;
    r.write(1, 0x0102_0304);
    r.write(2, 0x0101_0101);
    assert_eq!(execute(0xFA81_F002, &mut r), StepResult::Ok);
    assert_eq!(r.xpsr & 0xF800_0000, saved);
    assert!(r.apsr_n());
    assert!(!r.apsr_z());
    assert!(r.apsr_c());
    assert!(!r.apsr_v());
    assert_eq!(r.xpsr & (1 << 27), 1 << 27);
    assert_eq!(r.apsr_ge(), 0xF);
}

// === SXTB16 / UXTB16 ===

#[test]
fn sxtb16_sign_extends_bytes_with_rotate() {
    let cases = [
        (0xFA2F_F081, 0xFF80_0040),
        (0xFA2F_F091, 0x0000_FF80),
        (0xFA2F_F0A1, 0x0040_FF80),
        (0xFA2F_F0B1, 0xFF80_0000),
    ];
    for (instr, expected) in cases {
        let mut r = CortexM33Registers::new();
        r.write(1, 0x0080_8040);
        assert_eq!(execute(instr, &mut r), StepResult::Ok);
        assert_eq!(r.read(0), expected, "instr = {instr:#010X}");
    }
}

#[test]
fn uxtb16_zero_extends_bytes_with_rotate() {
    let cases = [
        (0xFA3F_F081, 0x0034_00CD),
        (0xFA3F_F091, 0x0012_00AB),
        (0xFA3F_F0A1, 0x00CD_0034),
        (0xFA3F_F0B1, 0x00AB_0012),
    ];
    for (instr, expected) in cases {
        let mut r = CortexM33Registers::new();
        r.write(1, 0x1234_ABCD);
        assert_eq!(execute(instr, &mut r), StepResult::Ok);
        assert_eq!(r.read(0), expected, "instr = {instr:#010X}");
    }
}

#[test]
fn sxtb16_and_uxtb16_do_not_update_flags() {
    let mut r = CortexM33Registers::new();
    r.set_apsr_nzcv(true, false, true, false);
    r.set_apsr_q(true);
    r.set_apsr_ge(0b0101);
    let saved = r.xpsr;
    r.write(1, 0x0000_8040);
    assert_eq!(execute(0xFA2F_F081, &mut r), StepResult::Ok);
    assert_eq!(r.xpsr, saved);
    assert_eq!(execute(0xFA3F_F081, &mut r), StepResult::Ok);
    assert_eq!(r.xpsr, saved);
}

// === USAD8 / USADA8 ===

#[test]
fn usad8_sums_absolute_byte_differences() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0102_0304);
    r.write(2, 0x0403_0201);
    assert_eq!(execute(0xFB71_F002, &mut r), StepResult::Ok);
    // |4-1| + |3-2| + |2-3| + |1-4| = 8.
    assert_eq!(r.read(0), 8);
}

#[test]
fn usada8_adds_the_accumulator() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0102_0304);
    r.write(2, 0x0403_0201);
    r.write(3, 100);
    assert_eq!(execute(0xFB71_3002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 108);
}

#[test]
fn usad8_and_usada8_do_not_update_flags() {
    let mut r = CortexM33Registers::new();
    r.set_apsr_nzcv(false, true, false, true);
    r.set_apsr_q(true);
    r.set_apsr_ge(0b0110);
    let saved = r.xpsr;
    r.write(1, 0x0000_00FF);
    r.write(2, 0x0000_0000);
    r.write(3, 7);
    assert_eq!(execute(0xFB71_F002, &mut r), StepResult::Ok);
    assert_eq!(r.xpsr, saved);
    assert_eq!(execute(0xFB71_3002, &mut r), StepResult::Ok);
    assert_eq!(r.xpsr, saved);
}

// === APSR.GE read/write path ===

#[test]
fn mrs_apsr_reports_ge_flags() {
    let mut r = CortexM33Registers::new();
    r.set_apsr_nzcv(true, false, false, false);
    r.set_apsr_ge(0b0101);
    assert_eq!(execute(0xF3EF_8000, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x8005_0000);
}

#[test]
fn msr_apsr_g_writes_only_ge() {
    let mut r = CortexM33Registers::new();
    r.set_apsr_nzcv(true, false, true, false);
    r.write(0, 0x000A_0000);
    assert_eq!(execute(0xF380_8400, &mut r), StepResult::Ok);
    assert_eq!(r.apsr_ge(), 0b1010);
    assert!(r.apsr_n());
    assert!(!r.apsr_z());
    assert!(r.apsr_c());
    assert!(!r.apsr_v());
}

#[test]
fn msr_apsr_nzcvqg_writes_both_groups() {
    let mut r = CortexM33Registers::new();
    r.write(0, 0xA80A_0000);
    assert_eq!(execute(0xF380_8C00, &mut r), StepResult::Ok);
    assert!(r.apsr_n());
    assert!(!r.apsr_z());
    assert!(r.apsr_c());
    assert!(!r.apsr_v());
    assert_eq!(r.xpsr & (1 << 27), 1 << 27);
    assert_eq!(r.apsr_ge(), 0b1010);
}
