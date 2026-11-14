//! Saturating and packed-SIMD DSP tests: `QADD`/`QSUB`/`QDADD`/`QDSUB`,
//! `Q*`/`UQ*` parallel lanes, exchange lanes, `SEL`, `SSAT`/`USAT`
//! 饱和与打包 SIMD DSP 测试：`QADD`/`QSUB`/`QDADD`/`QDSUB`、`Q*`/`UQ*`
//! 并行通道、交换通道、`SEL`、`SSAT`/`USAT`
//!
//! All encodings were cross-checked with `arm-none-eabi-as -mcpu=cortex-m33`
//! and `arm-none-eabi-objdump -d`.
//! 所有编码均已用 `arm-none-eabi-as -mcpu=cortex-m33` 与
//! `arm-none-eabi-objdump -d` 交叉核对。

use crate::execute_thumb16::StepResult;
use crate::execute_thumb32::test_support::execute;
use crate::registers::CortexM33Registers;

/// Read `APSR.Q` (`xpsr[27]`); there is no dedicated accessor for it
/// 读取 `APSR.Q`（`xpsr[27]`）；没有专用的访问器
fn q(regs: &CortexM33Registers) -> bool {
    regs.xpsr & (1 << 27) != 0
}

// === QADD / QSUB / QDADD / QDSUB ===

#[test]
fn qadd_saturates_positive_and_sets_q() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x7FFF_FFFF);
    r.write(2, 0x0000_0001);
    r.set_apsr_nzcv(false, true, false, true);
    r.set_apsr_ge(0b0101);
    // QADD r0, r1, r2: Rd = SignedSat(Rn + Rm).
    assert_eq!(execute(0xFA82_F081, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x7FFF_FFFF);
    assert!(q(&r));
    // Q must never disturb GE or NZCV.
    // Q 绝不能影响 GE 与 NZCV。
    assert_eq!(r.apsr_ge(), 0b0101);
    assert!(!r.apsr_n() && r.apsr_z() && !r.apsr_c() && r.apsr_v());
}

#[test]
fn qadd_in_range_leaves_q_clear() {
    let mut r = CortexM33Registers::new();
    r.write(1, 5);
    r.write(2, 7);
    r.set_apsr_nzcv(false, false, false, false);
    r.set_apsr_q(false);
    r.set_apsr_ge(0b0011);
    assert_eq!(execute(0xFA82_F081, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 12);
    assert!(!q(&r));
    assert_eq!(r.apsr_ge(), 0b0011);
}

#[test]
fn qsub_saturates_both_directions() {
    // QSUB r0, r1, r2: Rd = SignedSat(Rn - Rm), so the field values are
    // Rn = r2 and Rm = r1.
    // QSUB r0, r1, r2：Rd = SignedSat(Rn - Rm)，其中字段值 Rn = r2、Rm = r1。
    let mut r = CortexM33Registers::new();
    r.write(1, 0xFFFF_FFFF); // Rm = -1
    r.write(2, 0x7FFF_FFFF); // Rn
    assert_eq!(execute(0xFA82_F0A1, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x7FFF_FFFF);
    assert!(q(&r));

    let mut r = CortexM33Registers::new();
    r.write(1, 0x0000_0001);
    r.write(2, 0x8000_0000);
    assert_eq!(execute(0xFA82_F0A1, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x8000_0000);
    assert!(q(&r));
}

#[test]
fn qdadd_saturates_the_doubled_operand() {
    // QDADD r0, r1, r2: Rd = SignedSat(Rn + SignedSat(2 * Rm)).
    // 2 * 0x4000_0000 overflows, so the doubling alone sets Q.
    // QDADD r0, r1, r2：Rd = SignedSat(Rn + SignedSat(2 * Rm))。
    // 2 * 0x4000_0000 上溢，仅加倍这一步就会置 Q。
    let mut r = CortexM33Registers::new();
    r.write(1, 0x4000_0000);
    r.write(2, 0);
    assert_eq!(execute(0xFA82_F091, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x7FFF_FFFF);
    assert!(q(&r));

    let mut r = CortexM33Registers::new();
    r.write(1, 5);
    r.write(2, 100);
    r.set_apsr_q(false);
    assert_eq!(execute(0xFA82_F091, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 110);
    assert!(!q(&r));
}

#[test]
fn qdsub_subtracts_the_saturated_doubled_operand() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0000_0001);
    r.write(2, 0x8000_0000);
    // 0x8000_0000 - 2 underflows the signed range.
    // 0x8000_0000 - 2 低于有符号范围。
    assert_eq!(execute(0xFA82_F0B1, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x8000_0000);
    assert!(q(&r));
}

// === Saturating parallel byte/halfword lanes ===

#[test]
fn qadd8_saturates_each_byte_and_preserves_ge() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x7F7F_7F7F);
    r.write(2, 0x0101_0101);
    r.set_apsr_ge(0b1001);
    assert_eq!(execute(0xFA81_F012, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x7F7F_7F7F);
    assert!(q(&r));
    assert_eq!(r.apsr_ge(), 0b1001);

    // Negative saturation on every lane.
    // 每个通道都向下饱和。
    let mut r = CortexM33Registers::new();
    r.write(1, 0x8080_8080);
    r.write(2, 0xFFFF_FFFF);
    assert_eq!(execute(0xFA81_F012, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x8080_8080);
    assert!(q(&r));
}

#[test]
fn qsub8_saturates_mixed_lanes() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x7F80_0100);
    r.write(2, 0xFF01_8001);
    assert_eq!(execute(0xFAC1_F012, &mut r), StepResult::Ok);
    // lane0: 0x00 - 0x01 = -1; lane1: 1 - (-128) = 129 -> 127 (saturated);
    // lane2: -128 - 1 -> -128 (saturated); lane3: 127 - (-1) -> 127 (saturated).
    // 通道0：0x00 - 0x01 = -1；通道1：1 - (-128) = 129 -> 127（饱和）；
    // 通道2：-128 - 1 -> -128（饱和）；通道3：127 - (-1) -> 127（饱和）。
    assert_eq!(r.read(0), 0x7F80_7FFF);
    assert!(q(&r));
}

#[test]
fn qadd16_and_qsub16_saturate_halfwords() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x7FFF_8000);
    r.write(2, 0x0001_FFFF);
    assert_eq!(execute(0xFA91_F012, &mut r), StepResult::Ok);
    // Low: -32768 + -1 -> -32768; high: 32767 + 1 -> 32767.
    // 低：-32768 + -1 -> -32768；高：32767 + 1 -> 32767。
    assert_eq!(r.read(0), 0x7FFF_8000);
    assert!(q(&r));

    let mut r = CortexM33Registers::new();
    r.write(1, 0x8000_7FFF);
    r.write(2, 0x0001_FFFF);
    assert_eq!(execute(0xFAD1_F012, &mut r), StepResult::Ok);
    // Low: 32767 - (-1) -> 32767; high: -32768 - 1 -> -32768.
    // 低：32767 - (-1) -> 32767；高：-32768 - 1 -> -32768。
    assert_eq!(r.read(0), 0x8000_7FFF);
    assert!(q(&r));
}

#[test]
fn uqadd8_and_uqsub8_saturate_the_unsigned_range() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0xF0F0_FF10);
    r.write(2, 0x2020_2010);
    assert_eq!(execute(0xFA81_F052, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_FF20);
    assert!(q(&r));

    let mut r = CortexM33Registers::new();
    r.write(1, 0x0010_FF00);
    r.write(2, 0x10FF_0001);
    assert_eq!(execute(0xFAC1_F052, &mut r), StepResult::Ok);
    // Negative lanes clamp to zero, 0xFF - 0 keeps its value.
    // 结果为负的通道钳制到零，0xFF - 0 保持原值。
    assert_eq!(r.read(0), 0x0000_FF00);
    assert!(q(&r));
}

#[test]
fn uqadd16_and_uqsub16_saturate_halfwords() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0xFFFF_0000);
    r.write(2, 0x0001_1000);
    assert_eq!(execute(0xFA91_F052, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_1000);
    assert!(q(&r));

    let mut r = CortexM33Registers::new();
    r.write(1, 0x0000_0010);
    r.write(2, 0x0005_0005);
    assert_eq!(execute(0xFAD1_F052, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x0000_000B);
    assert!(q(&r));
}

// === Exchange lanes ===

#[test]
fn qasx_and_qsax_cross_the_halfword_lanes() {
    // QASX: Rd[15:0] = Rn[15:0] + Rm[31:16]; Rd[31:16] = Rn[31:16] - Rm[15:0].
    let mut r = CortexM33Registers::new();
    r.write(1, 0x8000_7FFF);
    r.write(2, 0x7FFF_0001);
    assert_eq!(execute(0xFAA1_F012, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x8000_7FFF);
    assert!(q(&r));

    // QSAX: Rd[15:0] = Rn[15:0] - Rm[31:16]; Rd[31:16] = Rn[31:16] + Rm[15:0].
    let mut r = CortexM33Registers::new();
    r.write(1, 0x7FFF_0000);
    r.write(2, 0x8001_0001);
    assert_eq!(execute(0xFAE1_F012, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x7FFF_7FFF);
    assert!(q(&r));
}

#[test]
fn sasx_and_ssax_set_ge_per_halfword() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0005_8000);
    r.write(2, 0x8000_0003);
    assert_eq!(execute(0xFAA1_F002, &mut r), StepResult::Ok);
    // Low: -32768 + -32768 = -65536 (< 0) -> GE[1:0] = 00;
    // high: 5 - 3 = 2 -> GE[3:2] = 11.
    // 低：-32768 + -32768 = -65536（< 0）-> GE[1:0] = 00；
    // 高：5 - 3 = 2 -> GE[3:2] = 11。
    assert_eq!(r.read(0), 0x0002_0000);
    assert_eq!(r.apsr_ge(), 0b1100);
    assert!(!q(&r));

    let mut r = CortexM33Registers::new();
    r.write(1, 0x7FFF_0000);
    r.write(2, 0x0001_0000);
    assert_eq!(execute(0xFAE1_F002, &mut r), StepResult::Ok);
    // Low: 0 - 1 = -1 -> GE[1:0] = 00; high: 32767 + 0 -> GE[3:2] = 11.
    // 低：0 - 1 = -1 -> GE[1:0] = 00；高：32767 + 0 -> GE[3:2] = 11。
    assert_eq!(r.read(0), 0x7FFF_FFFF);
    assert_eq!(r.apsr_ge(), 0b1100);
}

#[test]
fn uasx_and_usax_use_carry_and_borrow_for_ge() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0001_FFFF);
    r.write(2, 0x0001_0002);
    assert_eq!(execute(0xFAA1_F042, &mut r), StepResult::Ok);
    // Low: 0xFFFF + 0x0001 carries -> GE[1:0] = 11;
    // high: 0x0001 - 0x0002 borrows -> GE[3:2] = 00.
    // 低：0xFFFF + 0x0001 产生进位 -> GE[1:0] = 11；
    // 高：0x0001 - 0x0002 产生借位 -> GE[3:2] = 00。
    assert_eq!(r.read(0), 0xFFFF_0000);
    assert_eq!(r.apsr_ge(), 0b0011);

    let mut r = CortexM33Registers::new();
    r.write(1, 0xFFFF_0000);
    r.write(2, 0x0001_0001);
    assert_eq!(execute(0xFAE1_F042, &mut r), StepResult::Ok);
    // Low: 0 - 1 borrows -> GE[1:0] = 00; high: 0xFFFF + 1 carries -> 11.
    // 低：0 - 1 产生借位 -> GE[1:0] = 00；高：0xFFFF + 1 产生进位 -> 11。
    assert_eq!(r.read(0), 0x0000_FFFF);
    assert_eq!(r.apsr_ge(), 0b1100);
}

// === SEL ===

#[test]
fn sel_picks_bytes_with_ge_and_keeps_flags() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x1122_3344);
    r.write(2, 0xAABB_CCDD);
    r.set_apsr_nzcv(true, false, true, false);
    r.set_apsr_q(true);
    r.set_apsr_ge(0b1010);
    let saved = r.xpsr;
    assert_eq!(execute(0xFAA1_F082, &mut r), StepResult::Ok);
    // GE[3] and GE[1] are set: bytes 3 and 1 come from Rn.
    // GE[3] 与 GE[1] 置位：字节 3 与字节 1 取自 Rn。
    assert_eq!(r.read(0), 0x11BB_33DD);
    assert_eq!(r.xpsr, saved);
}

// === SSAT / USAT ===

#[test]
fn ssat_saturates_to_the_requested_width() {
    // SSAT r0, #8, r1 -> signed range [-128, 127].
    // SSAT r0, #8, r1 -> 有符号范围 [-128, 127]。
    let cases = [
        (0x0000_007F, 0x0000_007F, false),
        (0x0000_0080, 0x0000_007F, true),
        (0xFFFF_FF80, 0xFFFF_FF80, false),
        (0xFFFF_FF7F, 0xFFFF_FF80, true),
    ];
    for (input, expected, saturated) in cases {
        let mut r = CortexM33Registers::new();
        r.write(1, input);
        r.set_apsr_q(false);
        assert_eq!(execute(0xF301_0007, &mut r), StepResult::Ok);
        assert_eq!(r.read(0), expected, "input = {input:#010X}");
        assert_eq!(q(&r), saturated, "input = {input:#010X}");
    }

    // SSAT r0, #1, r1 -> signed range [-1, 0].
    // SSAT r0, #1, r1 -> 有符号范围 [-1, 0]。
    for (input, expected) in [(5u32, 0u32), (0xFFFF_FFFB, 0xFFFF_FFFF)] {
        let mut r = CortexM33Registers::new();
        r.write(1, input);
        assert_eq!(execute(0xF301_0000, &mut r), StepResult::Ok);
        assert_eq!(r.read(0), expected);
        assert!(q(&r));
    }

    // SSAT r0, #32, r1 -> the full signed range, so never saturates.
    // SSAT r0, #32, r1 -> 完整有符号范围，永不饱和。
    let mut r = CortexM33Registers::new();
    r.write(1, 0xDEAD_BEEF);
    r.set_apsr_q(false);
    assert_eq!(execute(0xF301_001F, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xDEAD_BEEF);
    assert!(!q(&r));
}

#[test]
fn ssat_honours_the_shift_immediate() {
    // SSAT r0, #8, r1, lsl #2: 40 << 2 = 160 saturates to 127.
    // SSAT r0, #8, r1, lsl #2：40 << 2 = 160 饱和为 127。
    let mut r = CortexM33Registers::new();
    r.write(1, 40);
    assert_eq!(execute(0xF301_0087, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 127);
    assert!(q(&r));

    let mut r = CortexM33Registers::new();
    r.write(1, 10);
    assert_eq!(execute(0xF301_0087, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 40);

    // SSAT r0, #16, r1, asr #3: 0x0008_0000 >> 3 = 65536 saturates to 32767.
    // SSAT r0, #16, r1, asr #3：0x0008_0000 >> 3 = 65536 饱和为 32767。
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0008_0000);
    assert_eq!(execute(0xF321_00CF, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x0000_7FFF);
    assert!(q(&r));

    // The same shift downwards saturates to -32768.
    // 同样的向下移位饱和到 -32768。
    let mut r = CortexM33Registers::new();
    r.write(1, 0xFFF8_0000);
    assert_eq!(execute(0xF321_00CF, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_8000);
    assert!(q(&r));
}

#[test]
fn usat_clamps_negative_input_to_zero() {
    // USAT r0, #8, r1 -> unsigned range [0, 255] of a signed input.
    // USAT r0, #8, r1 -> 有符号输入钳制到无符号范围 [0, 255]。
    let cases = [
        (200u32, 200u32, false),
        (255, 255, false),
        (300, 255, true),
        (0xFFFF_FFFF, 0, true),
    ];
    for (input, expected, saturated) in cases {
        let mut r = CortexM33Registers::new();
        r.write(1, input);
        r.set_apsr_q(false);
        assert_eq!(execute(0xF381_0008, &mut r), StepResult::Ok);
        assert_eq!(r.read(0), expected, "input = {input:#010X}");
        assert_eq!(q(&r), saturated, "input = {input:#010X}");
    }

    // USAT r0, #8, r1, lsl #5: 10 << 5 = 320 saturates to 255.
    // USAT r0, #8, r1, lsl #5：10 << 5 = 320 饱和为 255。
    let mut r = CortexM33Registers::new();
    r.write(1, 10);
    assert_eq!(execute(0xF381_1048, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 255);
    assert!(q(&r));

    // USAT r0, #16, r1, asr #3: 65536 saturates to 65535.
    // USAT r0, #16, r1, asr #3：65536 饱和为 65535。
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0008_0000);
    assert_eq!(execute(0xF3A1_00D0, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x0000_FFFF);
    assert!(q(&r));
}

#[test]
fn ssat16_saturates_each_halfword() {
    // SSAT16 r0, #8, r1 -> both halfwords are clamped to [-128, 127].
    // SSAT16 r0, #8, r1 -> 两个半字都钳制到 [-128, 127]。
    let mut r = CortexM33Registers::new();
    r.write(1, 0xFF80_007F);
    r.set_apsr_q(false);
    assert_eq!(execute(0xF321_0007, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFF80_007F);
    assert!(!q(&r));

    let mut r = CortexM33Registers::new();
    r.write(1, 0xFF80_0128);
    assert_eq!(execute(0xF321_0007, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFF80_007F);
    assert!(q(&r));

    // SSAT16 r0, #16, r1 is the identity over the halfword range.
    // SSAT16 r0, #16, r1 在半字范围内为恒等操作。
    let mut r = CortexM33Registers::new();
    r.write(1, 0x8000_7FFF);
    r.set_apsr_q(false);
    assert_eq!(execute(0xF321_000F, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x8000_7FFF);
    assert!(!q(&r));
}

#[test]
fn usat16_saturates_each_halfword() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0xF000_0200);
    assert_eq!(execute(0xF3A1_0008, &mut r), StepResult::Ok);
    // High halfword is negative -> 0; low halfword 512 -> 255.
    // 高半字为负 -> 0；低半字 512 -> 255。
    assert_eq!(r.read(0), 0x0000_00FF);
    assert!(q(&r));

    let mut r = CortexM33Registers::new();
    r.write(1, 0x0002_0001);
    assert_eq!(execute(0xF3A1_0001, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x0001_0001);
    assert!(q(&r));
}

#[test]
fn ssat_preserves_ge_and_nzcv_and_q_is_sticky() {
    let mut r = CortexM33Registers::new();
    r.write(1, 5);
    r.set_apsr_nzcv(true, true, false, true);
    r.set_apsr_q(true);
    r.set_apsr_ge(0b0110);
    let saved = r.xpsr & !(1 << 27);
    assert_eq!(execute(0xF301_0007, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 5);
    // A non-saturating SSAT keeps the sticky Q and all other flags.
    // 未饱和的 SSAT 保持 Q（保持型）与其余标志。
    assert!(q(&r));
    assert_eq!(r.xpsr & !(1 << 27), saved);
}

#[test]
fn q_flag_survives_non_saturating_dsp_instructions() {
    let mut r = CortexM33Registers::new();
    r.set_apsr_q(true);
    r.write(1, 0x0102_0304);
    r.write(2, 0x0101_0101);
    // SADD8, QADD8 (in range) and SEL must not clear Q.
    // SADD8、QADD8（未饱和）与 SEL 不得清除 Q。
    assert_eq!(execute(0xFA81_F002, &mut r), StepResult::Ok);
    assert!(q(&r));
    assert_eq!(execute(0xFA81_F012, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x0203_0405);
    assert!(q(&r));
    r.set_apsr_ge(0b1111);
    assert_eq!(execute(0xFAA1_F082, &mut r), StepResult::Ok);
    assert!(q(&r));
}

// === Reserved encodings ===

#[test]
fn reserved_dsp_and_saturation_encodings_stay_undefined() {
    // Reserved lane variants 3/7/0xC, the QDADD/QSUB sub-opcodes above 0xB,
    // SEL outside op 0xA, and SSAT encodings with bit 5 set or Rd = 15.
    // The halving variants 2/6 are real instructions and are covered by
    // `dsp_extra`.
    // 保留的通道变体 3/7/0xC、0xB 以上的 QDADD/QSUB 子操作码、
    // op 0xA 之外的 SEL，以及位 5 置位或 Rd = 15 的 SSAT 编码。
    // 折半变体 2/6 是真实指令，由 `dsp_extra` 覆盖。
    let cases = [
        0xFA81_F032, // reserved variant 3
        0xFA81_F072, // reserved variant 7
        0xFA81_F0C2, // reserved variant 0xC
        0xFA82_F0C1, // QADD with sub-opcode 0xC
        0xFAE1_F082, // "SEL" outside its op field
        0xFA82_FF81, // QADD writing PC
        0xF301_0027, // SSAT with reserved bit 5
        0xF301_0F07, // SSAT writing PC
    ];
    for instr in cases {
        let mut r = CortexM33Registers::new();
        r.write(1, 0x1234_5678);
        r.write(2, 0x9ABC_DEF0);
        assert_eq!(
            execute(instr, &mut r),
            StepResult::Undefined,
            "instr = {instr:#010X}"
        );
    }
}

// === Mixed execution ===

#[test]
fn mixed_dsp_sequences_do_not_regress() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0102_0304);
    r.write(2, 0x0101_0101);

    // SADD8: packed add with per-byte GE.
    // SADD8：带逐字节 GE 的打包加法。
    assert_eq!(execute(0xFA81_F002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x0203_0405);
    assert_eq!(r.apsr_ge(), 0xF);

    // SEL with GE = 1111 selects Rn entirely.
    // GE = 1111 的 SEL 完全选择 Rn。
    r.write(1, 0xAAAA_AAAA);
    assert_eq!(execute(0xFAA1_F082, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xAAAA_AAAA);

    // QADD8 with a single saturating lane sets Q but leaves GE alone.
    // 单个通道饱和的 QADD8 置 Q，但保持 GE 不变。
    r.write(1, 0x7F00_0000);
    r.write(2, 0x0100_0000);
    assert_eq!(execute(0xFA81_F012, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x7F00_0000);
    assert!(q(&r));
    assert_eq!(r.apsr_ge(), 0xF);

    // USAD8 (multiply unit) must not disturb Q or GE.
    // USAD8（乘法单元）不得影响 Q 与 GE。
    r.write(1, 0x0102_0304);
    r.write(2, 0x0403_0201);
    assert_eq!(execute(0xFB71_F002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 8);
    assert!(q(&r));
    assert_eq!(r.apsr_ge(), 0xF);

    // SSAT clamps a value to 8 signed bits and keeps GE.
    // SSAT 把值钳制到 8 位有符号范围，并保持 GE。
    r.write(1, 200);
    assert_eq!(execute(0xF301_0007, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 127);
    assert!(q(&r));
    assert_eq!(r.apsr_ge(), 0xF);
}
