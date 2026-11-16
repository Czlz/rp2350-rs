//! Second wave of DSP tests: halving lanes, dual multiply-accumulate, wide
//! multiply, `SMMLS` and the accumulating packed byte extends
//! 第二批 DSP 测试：折半通道、双乘累加、宽乘法、`SMMLS` 与累加式打包字节扩展
//!
//! Every encoding was cross-checked with `arm-none-eabi-as -mcpu=cortex-m33`
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

// === Halving lanes: SHADD8 / SHSUB8 / UHADD8 / UHSUB8 ===

#[test]
fn shadd8_halves_each_byte_lane() {
    // SHADD8 r0, r1, r2 - 0xFA81F022.
    let mut r = CortexM33Registers::new();
    r.write(1, 0x7F7F_8080);
    r.write(2, 0x0101_0101);
    assert_eq!(execute(0xFA81_F022, &mut r), StepResult::Ok);
    // lane0/1: (-128 + 1) >> 1 = -64 (0xC0); lane2/3: (127 + 1) >> 1 = 64.
    // 通道 0/1：(-128 + 1) >> 1 = -64（0xC0）；通道 2/3：(127 + 1) >> 1 = 64。
    assert_eq!(r.read(0), 0x4040_C0C0);
}

#[test]
fn shsub8_halves_and_keeps_the_sign_extended_shift() {
    // SHSUB8 r0, r1, r2 - 0xFAC1F022.
    let mut r = CortexM33Registers::new();
    r.write(1, 0x8000_0001);
    r.write(2, 0x0000_0003);
    assert_eq!(execute(0xFAC1_F022, &mut r), StepResult::Ok);
    // lane0: (1 - 3) >> 1 = -1; lane3: (-128 - 0) >> 1 = -64.
    // 通道 0：(1 - 3) >> 1 = -1；通道 3：(-128 - 0) >> 1 = -64。
    assert_eq!(r.read(0), 0xC000_00FF);
}

#[test]
fn uhadd8_and_uhsub8_half_the_unsigned_lanes() {
    // UHADD8 r0, r1, r2 - 0xFA81F062: 0xFF + 0xFF = 0x1FE -> 0xFF.
    let mut r = CortexM33Registers::new();
    r.write(1, 0xFFFF_FFFF);
    r.write(2, 0x0101_01FF);
    assert_eq!(execute(0xFA81_F062, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x8080_80FF);

    // UHSUB8 r0, r1, r2 - 0xFAC1F062: (0 - 16) >> 1 = -8, (16 - 0) >> 1 = 8.
    // UHSUB8 r0, r1, r2 —— 0xFAC1F062：(0 - 16) >> 1 = -8，(16 - 0) >> 1 = 8。
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0000_1000);
    r.write(2, 0x0000_0010);
    assert_eq!(execute(0xFAC1_F062, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x0000_08F8);
}

#[test]
fn shadd16_and_shsub16_half_the_halfword_lanes() {
    // SHADD16 r0, r1, r2 - 0xFA91F022.
    let mut r = CortexM33Registers::new();
    r.write(1, 0x8000_7FFF);
    r.write(2, 0x0000_0002);
    assert_eq!(execute(0xFA91_F022, &mut r), StepResult::Ok);
    // low: (0x7FFF + 2) >> 1 = 0x4000; high: (-32768) >> 1 = 0xC000.
    // 低：(0x7FFF + 2) >> 1 = 0x4000；高：(-32768) >> 1 = 0xC000。
    assert_eq!(r.read(0), 0xC000_4000);

    // SHSUB16 r0, r1, r2 - 0xFAD1F022.
    let mut r = CortexM33Registers::new();
    r.write(1, 0x8000_0000);
    r.write(2, 0x0001_0001);
    assert_eq!(execute(0xFAD1_F022, &mut r), StepResult::Ok);
    // low: (0 - 1) >> 1 = -1; high: (-32768 - 1) >> 1 = -16385 (0xBFFF).
    // 低：(0 - 1) >> 1 = -1；高：(-32768 - 1) >> 1 = -16385（0xBFFF）。
    assert_eq!(r.read(0), 0xBFFF_FFFF);
}

#[test]
fn uhadd16_and_uhsub16_half_the_unsigned_halfwords() {
    // UHADD16 r0, r1, r2 - 0xFA91F062.
    let mut r = CortexM33Registers::new();
    r.write(1, 0xFFFF_0000);
    r.write(2, 0x0001_FFFF);
    assert_eq!(execute(0xFA91_F062, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x8000_7FFF);

    // UHSUB16 r0, r1, r2 - 0xFAD1F062.
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0004_0000);
    r.write(2, 0x0001_0002);
    assert_eq!(execute(0xFAD1_F062, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x0001_FFFF);
}

#[test]
fn halving_exchange_forms_cross_the_halfword_lanes() {
    // SHASX r0, r1, r2 - 0xFAA1F022: low = a.lo + b.hi, high = a.hi - b.lo.
    let mut r = CortexM33Registers::new();
    r.write(1, 0x8000_0004);
    r.write(2, 0x0002_0002);
    assert_eq!(execute(0xFAA1_F022, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xBFFF_0003);

    // SHSAX r0, r1, r2 - 0xFAE1F022: low = a.lo - b.hi, high = a.hi + b.lo.
    let mut r = CortexM33Registers::new();
    r.write(1, 0x8000_0004);
    r.write(2, 0x0002_0002);
    assert_eq!(execute(0xFAE1_F022, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xC001_0001);

    // UHASX r0, r1, r2 - 0xFAA1F062: 1 + 0xFFFF = 0x10000 -> 0x8000,
    // 3 - 5 = -2 -> 0xFFFF.
    // UHASX r0, r1, r2 —— 0xFAA1F062：1 + 0xFFFF = 0x10000 -> 0x8000，
    // 3 - 5 = -2 -> 0xFFFF。
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0003_0001);
    r.write(2, 0xFFFF_0005);
    assert_eq!(execute(0xFAA1_F062, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_8000);

    // UHSAX r0, r1, r2 - 0xFAE1F062: 2 - 3 = -1 -> 0xFFFF, (4 + 1) >> 1 = 2.
    // UHSAX r0, r1, r2 —— 0xFAE1F062：2 - 3 = -1 -> 0xFFFF，(4 + 1) >> 1 = 2。
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0004_0002);
    r.write(2, 0x0003_0001);
    assert_eq!(execute(0xFAE1_F062, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x0002_FFFF);
}

#[test]
fn halving_forms_do_not_touch_any_flag() {
    // ARM ARM: SHADD8 ... UHSUB16 leave GE, Q and NZCV exactly as they were.
    // ARM ARM：SHADD8 … UHSUB16 完全保持 GE、Q 与 NZCV 不变。
    let encodings = [
        0xFA81_F022, // SHADD8
        0xFA91_F022, // SHADD16
        0xFAA1_F022, // SHASX
        0xFAE1_F022, // SHSAX
        0xFAC1_F022, // SHSUB8
        0xFAD1_F022, // SHSUB16
        0xFA81_F062, // UHADD8
        0xFA91_F062, // UHADD16
        0xFAA1_F062, // UHASX
        0xFAE1_F062, // UHSAX
        0xFAC1_F062, // UHSUB8
        0xFAD1_F062, // UHSUB16
    ];
    for instr in encodings {
        let mut r = CortexM33Registers::new();
        r.set_apsr_nzcv(true, false, true, false);
        r.set_apsr_q(true);
        r.set_apsr_ge(0b0101);
        let saved = r.xpsr;
        r.write(1, 0x7FFF_8000);
        r.write(2, 0x0001_8000);
        assert_eq!(execute(instr, &mut r), StepResult::Ok);
        assert_eq!(r.xpsr, saved, "instr = {instr:#010X}");
    }
}

// === SMLAD / SMLADX / SMLSD / SMLSDX / SMUAD / SMUADX / SMUSD / SMUSDX ===

#[test]
fn smlad_and_smladx_accumulate_the_dual_products() {
    // SMLAD r3, r4, r5, r6 - 0xFB246305: 3*5 + 2*4 + 10 = 33.
    let mut r = CortexM33Registers::new();
    r.write(4, 0x0002_0003);
    r.write(5, 0x0004_0005);
    r.write(6, 10);
    assert_eq!(execute(0xFB24_6305, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 33);

    // SMLADX r3, r4, r5, r6 - 0xFB246315: the second operand is rotated by 16,
    // so the products become 2*5 and 3*4: 2*5 + 3*4 + 10 = 32.
    // SMLADX r3, r4, r5, r6 —— 0xFB246315：第二操作数循环右移 16 位，乘积变为
    // 2*5 与 3*4：2*5 + 3*4 + 10 = 32。
    let mut r = CortexM33Registers::new();
    r.write(4, 0x0002_0003);
    r.write(5, 0x0004_0005);
    r.write(6, 10);
    assert_eq!(execute(0xFB24_6315, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 32);

    // Negative halfwords: (-2)*2 + (-1)*3 = -7.
    // 负半字：(-2)*2 + (-1)*3 = -7。
    let mut r = CortexM33Registers::new();
    r.write(4, 0xFFFF_FFFE);
    r.write(5, 0x0003_0002);
    r.write(6, 0);
    assert_eq!(execute(0xFB24_6305, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 0xFFFF_FFF9);
}

#[test]
fn smlad_sets_q_once_on_the_final_overflow() {
    // 0x4000_0000 + 0x4000_0000 = 0x8000_0000, then -1 brings it back in range:
    // Q must stay clear (the accumulate is checked as one infinite-precision sum).
    // 0x4000_0000 + 0x4000_0000 = 0x8000_0000，再加 -1 回到范围内：Q 必须保持
    // 清零（累加按一个无限精度和整体判断）。
    let mut r = CortexM33Registers::new();
    r.write(4, 0x8000_8000);
    r.write(5, 0x8000_8000);
    r.write(6, 0xFFFF_FFFF);
    r.set_apsr_q(false);
    assert_eq!(execute(0xFB24_6305, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 0x7FFF_FFFF);
    assert!(!q(&r));

    // Without the accumulator the sum 0x8000_0000 does not fit in 32 bits.
    // 不带累加器时和 0x8000_0000 超出 32 位范围。
    let mut r = CortexM33Registers::new();
    r.write(4, 0x8000_8000);
    r.write(5, 0x8000_8000);
    r.write(6, 0);
    assert_eq!(execute(0xFB24_6305, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 0x8000_0000);
    assert!(q(&r));
}

#[test]
fn smuad_and_smuadx_ignore_the_accumulator_field() {
    // SMUAD r3, r4, r5 - 0xFB24F305: 3*5 + 2*4 = 23.
    let mut r = CortexM33Registers::new();
    r.write(4, 0x0002_0003);
    r.write(5, 0x0004_0005);
    r.write(6, 0xDEAD_BEEF);
    assert_eq!(execute(0xFB24_F305, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 23);

    // SMUADX r3, r4, r5 - 0xFB24F315: 2*5 + 3*4 = 22.
    let mut r = CortexM33Registers::new();
    r.write(4, 0x0002_0003);
    r.write(5, 0x0004_0005);
    assert_eq!(execute(0xFB24_F315, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 22);

    // SMUAD still reports the overflow in Q.
    // SMUAD 仍会在上溢时置 Q。
    let mut r = CortexM33Registers::new();
    r.write(4, 0x8000_8000);
    r.write(5, 0x8000_8000);
    r.set_apsr_q(false);
    assert_eq!(execute(0xFB24_F305, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 0x8000_0000);
    assert!(q(&r));
}

#[test]
fn smlsd_smusd_and_the_exchanged_forms_subtract_products() {
    // SMLSD r3, r4, r5, r6 - 0xFB446305: 3*5 - 2*4 + 10 = 17.
    let mut r = CortexM33Registers::new();
    r.write(4, 0x0002_0003);
    r.write(5, 0x0004_0005);
    r.write(6, 10);
    assert_eq!(execute(0xFB44_6305, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 17);

    // SMLSDX r3, r4, r5, r6 - 0xFB446315: 2*5 - 3*4 + 10 = 8.
    let mut r = CortexM33Registers::new();
    r.write(4, 0x0002_0003);
    r.write(5, 0x0004_0005);
    r.write(6, 10);
    assert_eq!(execute(0xFB44_6315, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 8);

    // SMUSD r3, r4, r5 - 0xFB44F305: 15 - 8 = 7.
    let mut r = CortexM33Registers::new();
    r.write(4, 0x0002_0003);
    r.write(5, 0x0004_0005);
    assert_eq!(execute(0xFB44_F305, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 7);

    // SMUSDX r3, r4, r5 - 0xFB44F315: 10 - 12 = -2.
    let mut r = CortexM33Registers::new();
    r.write(4, 0x0002_0003);
    r.write(5, 0x0004_0005);
    assert_eq!(execute(0xFB44_F315, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 0xFFFF_FFFE);
}

// === SMLALD / SMLALDX / SMLSLD / SMLSLDX ===

#[test]
fn smlald_and_smlsld_accumulate_into_a_register_pair() {
    // SMLALD r3, r7, r4, r5 - 0xFBC437C5: RdLo = r3, RdHi = r7.
    let mut r = CortexM33Registers::new();
    r.write(3, 0xFFFF_FFFF);
    r.write(7, 0);
    r.write(4, 0x0002_0003);
    r.write(5, 0x0004_0005);
    r.set_apsr_q(true);
    let saved = r.xpsr;
    // 0xFFFF_FFFF + 23 carries into the high word.
    // 0xFFFF_FFFF + 23 产生向高字的进位。
    assert_eq!(execute(0xFBC4_37C5, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 0x16);
    assert_eq!(r.read(7), 1);
    // 64-bit accumulation never reports overflow and touches no flag.
    // 64 位累加从不上报溢出，也不修改任何标志。
    assert_eq!(r.xpsr, saved);

    // SMLALDX r3, r7, r4, r5 - 0xFBC437D5: exchanged products, 22.
    let mut r = CortexM33Registers::new();
    r.write(3, 0xFFFF_FFFF);
    r.write(7, 0);
    r.write(4, 0x0002_0003);
    r.write(5, 0x0004_0005);
    assert_eq!(execute(0xFBC4_37D5, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 0x15);
    assert_eq!(r.read(7), 1);

    // SMLSLD r3, r7, r4, r5 - 0xFBD437C5: 15 - 8 = 7 added to 0xFFFF_FFFF.
    // SMLSLD r3, r7, r4, r5 —— 0xFBD437C5：15 - 8 = 7 加到 0xFFFF_FFFF。
    let mut r = CortexM33Registers::new();
    r.write(3, 0xFFFF_FFFF);
    r.write(7, 0);
    r.write(4, 0x0002_0003);
    r.write(5, 0x0004_0005);
    assert_eq!(execute(0xFBD4_37C5, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 0x0000_0006);
    assert_eq!(r.read(7), 1);

    // SMLSLDX r3, r7, r4, r5 - 0xFBD437D5: 10 - 12 = -2, so the pair wraps.
    // SMLSLDX r3, r7, r4, r5 —— 0xFBD437D5：10 - 12 = -2，寄存器对回绕。
    let mut r = CortexM33Registers::new();
    r.write(3, 0);
    r.write(7, 0);
    r.write(4, 0x0002_0003);
    r.write(5, 0x0004_0005);
    assert_eq!(execute(0xFBD4_37D5, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 0xFFFF_FFFE);
    assert_eq!(r.read(7), 0xFFFF_FFFF);
}

// === SMMLS / SMMLSR ===

#[test]
fn smmls_and_smmlsr_subtract_the_high_product_word() {
    // SMMLS r3, r4, r5, r6 - 0xFB646305: (0 << 32) - 5 -> high word -1.
    let mut r = CortexM33Registers::new();
    r.write(4, 5);
    r.write(5, 1);
    r.write(6, 0);
    assert_eq!(execute(0xFB64_6305, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 0xFFFF_FFFF);

    // SMMLSR r3, r4, r5, r6 - 0xFB646315: the rounding constant lifts it to 0.
    // SMMLSR r3, r4, r5, r6 —— 0xFB646315：舍入常量把它抬到 0。
    let mut r = CortexM33Registers::new();
    r.write(4, 5);
    r.write(5, 1);
    r.write(6, 0);
    assert_eq!(execute(0xFB64_6315, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 0x0000_0000);

    // 2^61 - 2^60 = 2^60, so the high word is 0x1000_0000.
    // 2^61 - 2^60 = 2^60，因此高字为 0x1000_0000。
    let mut r = CortexM33Registers::new();
    r.write(4, 0x4000_0000);
    r.write(5, 0x4000_0000);
    r.write(6, 0x2000_0000);
    assert_eq!(execute(0xFB64_6305, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 0x1000_0000);
}

// === SMULWB / SMULWT / SMLAWB / SMLAWT ===

#[test]
fn smulwb_and_smulwt_scale_the_selected_halfword() {
    // SMULWB r3, r4, r5 - 0xFB34F305: 0x0001_0000 * 3 >> 16 = 3.
    let mut r = CortexM33Registers::new();
    r.write(4, 0x0001_0000);
    r.write(5, 0x0002_0003);
    assert_eq!(execute(0xFB34_F305, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 3);

    // SMULWT r3, r4, r5 - 0xFB34F315 uses the top halfword: 2.
    // SMULWT r3, r4, r5 —— 0xFB34F315 使用高半字：2。
    let mut r = CortexM33Registers::new();
    r.write(4, 0x0001_0000);
    r.write(5, 0x0002_0003);
    assert_eq!(execute(0xFB34_F315, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 2);

    // -1 * 128 >> 16 = -1.
    let mut r = CortexM33Registers::new();
    r.write(4, 0xFFFF_FFFF);
    r.write(5, 0x0000_0080);
    assert_eq!(execute(0xFB34_F305, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 0xFFFF_FFFF);
}

#[test]
fn smlawb_adds_the_accumulator_and_sets_q_on_overflow() {
    // SMLAWB r3, r4, r5, r6 - 0xFB346305: (0x3_0000 + 5 << 16) >> 16 = 8.
    let mut r = CortexM33Registers::new();
    r.write(4, 0x0001_0000);
    r.write(5, 3);
    r.write(6, 5);
    assert_eq!(execute(0xFB34_6305, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 8);

    // 0x7FFF_FFFF * 0x7FFF + (0x7FFF_FFFF << 16) does not fit in 48 bits.
    // 0x7FFF_FFFF * 0x7FFF + (0x7FFF_FFFF << 16) 超出 48 位。
    let mut r = CortexM33Registers::new();
    r.write(4, 0x7FFF_FFFF);
    r.write(5, 0x0000_7FFF);
    r.write(6, 0x7FFF_FFFF);
    r.set_apsr_q(false);
    assert_eq!(execute(0xFB34_6305, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 0xBFFF_7FFE);
    assert!(q(&r));
}

// === SXTAB16 / UXTAB16 ===

#[test]
fn sxtab16_adds_the_extended_bytes_per_halfword() {
    // SXTAB16 r3, r4, r5 with ror #0/#8/#16/#24.
    // SXTAB16 r3, r4, r5，rotate 依次为 #0/#8/#16/#24。
    let cases = [
        (0xFA24_F385, 0xFF81_0041),
        (0xFA24_F395, 0x0001_FF81),
        (0xFA24_F3A5, 0x0041_FF81),
        (0xFA24_F3B5, 0xFF81_0001),
    ];
    for (instr, expected) in cases {
        let mut r = CortexM33Registers::new();
        r.write(4, 0x0001_0001);
        r.write(5, 0x0080_8040);
        assert_eq!(execute(instr, &mut r), StepResult::Ok);
        assert_eq!(r.read(3), expected, "instr = {instr:#010X}");
    }

    // Each halfword wraps on its own: 0x8000 + 0xFF80 = 0x17F80 -> 0x7F80.
    // 每个半字独立回绕：0x8000 + 0xFF80 = 0x17F80 -> 0x7F80。
    let mut r = CortexM33Registers::new();
    r.write(4, 0x0000_8000);
    r.write(5, 0x0000_0080);
    assert_eq!(execute(0xFA24_F385, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 0x0000_7F80);
}

#[test]
fn uxtab16_zero_extends_and_adds_per_halfword() {
    // UXTAB16 r3, r4, r5 - 0xFA34F385 and the ror #8 form 0xFA34F395.
    let mut r = CortexM33Registers::new();
    r.write(4, 0x0001_0001);
    r.write(5, 0x1234_ABCD);
    assert_eq!(execute(0xFA34_F385, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 0x0035_00CE);

    let mut r = CortexM33Registers::new();
    r.write(4, 0x0001_0001);
    r.write(5, 0x1234_ABCD);
    assert_eq!(execute(0xFA34_F395, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 0x0013_00AC);
}

#[test]
fn sxtab16_uxtab16_keep_flags_and_reject_pc_operands() {
    let mut r = CortexM33Registers::new();
    r.set_apsr_nzcv(true, false, true, false);
    r.set_apsr_q(true);
    r.set_apsr_ge(0b0110);
    let saved = r.xpsr;
    r.write(4, 0x1111_2222);
    r.write(5, 0x0080_8040);
    assert_eq!(execute(0xFA24_F385, &mut r), StepResult::Ok);
    assert_eq!(r.xpsr, saved);
    assert_eq!(execute(0xFA34_F385, &mut r), StepResult::Ok);
    assert_eq!(r.xpsr, saved);

    // Rd = 15 and Rm = 15 are UNPREDICTABLE.
    // Rd = 15 与 Rm = 15 属于不可预测编码。
    let mut r = CortexM33Registers::new();
    assert_eq!(execute(0xFA24_FF85, &mut r), StepResult::Undefined);
    assert_eq!(execute(0xFA34_F38F, &mut r), StepResult::Undefined);
}

// === Reserved encodings and mixed execution ===

#[test]
fn reserved_dsp_multiply_encodings_stay_undefined() {
    let cases = [
        0xFB24_6325, // SMLAD with reserved bits[7:5] = 001
        0xFBC4_37E5, // SMLALD without the 110 prefix
        0xFB64_6325, // SMMLS with reserved bits[7:5] = 001
        0xFB34_F325, // SMULWB with reserved bits[7:5] = 001
        0xFB24_6F05, // SMLAD writing PC
        0xFBC4_33C5, // SMLALD with RdLo == RdHi
        0xFB34_F30F, // SMULWB with Rm = 15
        0xFB1C_F282, // SMULBB with reserved bits[7:4] = 1000
        0xFB54_F325, // SMMUL with reserved bits[7:4] = 0010
    ];
    for instr in cases {
        let mut r = CortexM33Registers::new();
        r.write(4, 0x1234_5678);
        r.write(5, 0x9ABC_DEF0);
        assert_eq!(
            execute(instr, &mut r),
            StepResult::Undefined,
            "instr = {instr:#010X}"
        );
    }
}

#[test]
fn halving_and_multiply_sequences_do_not_regress() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0102_0304);
    r.write(2, 0x0101_0101);

    // SADD8 sets GE per byte.
    // SADD8 逐字节置 GE。
    assert_eq!(execute(0xFA81_F002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x0203_0405);
    assert_eq!(r.apsr_ge(), 0xF);

    // SHADD8 halves the raw operands but must leave GE untouched.
    // SHADD8 对原始操作数折半，但必须保持 GE 不变。
    assert_eq!(execute(0xFA81_F022, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x0101_0202);
    assert_eq!(r.apsr_ge(), 0xF);

    // SMLAD accumulates and leaves GE alone.
    // SMLAD 累加并保持 GE 不变。
    r.write(4, 0x0002_0003);
    r.write(5, 0x0004_0005);
    r.write(6, 1);
    assert_eq!(execute(0xFB24_6305, &mut r), StepResult::Ok);
    assert_eq!(r.read(3), 24);
    assert_eq!(r.apsr_ge(), 0xF);
    assert!(!q(&r));

    // SEL still consumes the GE flags written by SADD8.
    // SEL 仍使用 SADD8 写入的 GE 标志。
    r.write(1, 0xAAAA_AAAA);
    r.write(2, 0x5555_5555);
    assert_eq!(execute(0xFAA1_F082, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xAAAA_AAAA);
}
