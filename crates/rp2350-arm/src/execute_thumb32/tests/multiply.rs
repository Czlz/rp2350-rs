//! Multiply/divide tests
//! 乘法/除法测试

use crate::execute_thumb16::StepResult;
use crate::execute_thumb32::test_support::execute;
use crate::registers::CortexM33Registers;

#[test]
fn mul_w_basic() {
    let mut r = CortexM33Registers::new();
    r.write(1, 3);
    r.write(2, 4);
    assert_eq!(execute(0xFB01_F002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 12);
}

#[test]
fn mla_accumulates() {
    let mut r = CortexM33Registers::new();
    r.write(1, 3);
    r.write(2, 4);
    r.write(3, 5);
    assert_eq!(execute(0xFB01_3002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 17);
}

#[test]
fn mls_subtracts_product() {
    let mut r = CortexM33Registers::new();
    r.write(1, 3);
    r.write(2, 4);
    r.write(3, 5);
    assert_eq!(execute(0xFB01_3012, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_FFF9);
}

#[test]
fn smull_signed_product() {
    let mut r = CortexM33Registers::new();
    r.write(2, 0xFFFF_FFFF);
    r.write(3, 2);
    assert_eq!(execute(0xFB82_0103, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_FFFE);
    assert_eq!(r.read(1), 0xFFFF_FFFF);
}

#[test]
fn umull_unsigned_product() {
    let mut r = CortexM33Registers::new();
    r.write(2, 0xFFFF_FFFF);
    r.write(3, 2);
    assert_eq!(execute(0xFBA2_0103, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_FFFE);
    assert_eq!(r.read(1), 1);
}

#[test]
fn smlal_signed_accumulate() {
    let mut r = CortexM33Registers::new();
    r.write(0, 5);
    r.write(1, 0);
    r.write(2, 0xFFFF_FFFF);
    r.write(3, 2);
    assert_eq!(execute(0xFBC2_0103, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 3);
    assert_eq!(r.read(1), 0);
}

#[test]
fn umlal_unsigned_accumulate() {
    let mut r = CortexM33Registers::new();
    r.write(0, 5);
    r.write(1, 0);
    r.write(2, 0xFFFF_FFFF);
    r.write(3, 2);
    assert_eq!(execute(0xFBE2_0103, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 3);
    assert_eq!(r.read(1), 2);
}

#[test]
fn smlabb_accumulates_bottom_halfwords() {
    // SMLABB r2, ip, r2, lr — the exact instruction CircuitPython executes
    // while assembling USB descriptors during boot.
    // SMLABB r2, ip, r2, lr —— CircuitPython 在启动期间组装 USB 描述符时执行
    // 的指令。
    let mut r = CortexM33Registers::new();
    r.write(12, 3);
    r.write(2, 4);
    r.write(14, 5);
    assert_eq!(execute(0xFB1C_E202, &mut r), StepResult::Ok);
    assert_eq!(r.read(2), 17);
}

#[test]
fn smulbb_uses_selected_halfwords() {
    let mut r = CortexM33Registers::new();
    r.write(12, 0x0003_0007);
    r.write(2, 0x0002_0004);
    assert_eq!(execute(0xFB1C_F202, &mut r), StepResult::Ok);
    // bits[7:4] = 0 selects the bottom half of both registers: 7 * 4.
    // bits[7:4] = 0 选择两个寄存器的低半字：7 * 4。
    assert_eq!(r.read(2), 28);
    // bits[7:4] = 3 selects the top half of both registers: 3 * 2.
    // bits[7:4] = 3 选择两个寄存器的高半字：3 * 2。
    r.write(2, 0x0002_0004);
    assert_eq!(execute(0xFB1C_F232, &mut r), StepResult::Ok);
    assert_eq!(r.read(2), 6);
}

#[test]
fn smmul_and_smmla_take_the_high_product_word() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x4000_0000);
    r.write(2, 0x4000_0000);
    assert_eq!(execute(0xFB51_F002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x1000_0000);
    r.write(3, 1);
    assert_eq!(execute(0xFB51_3002, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x1000_0001);
}

#[test]
fn sdiv_signed() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0xFFFF_FFF9);
    r.write(2, 2);
    assert_eq!(execute(0xFB91_F0F2, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_FFFD);
}

#[test]
fn udiv_unsigned() {
    let mut r = CortexM33Registers::new();
    r.write(1, 7);
    r.write(2, 2);
    assert_eq!(execute(0xFBB1_F0F2, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 3);
}

#[test]
fn divide_by_zero_returns_zero() {
    let mut r = CortexM33Registers::new();
    r.write(1, 7);
    r.write(2, 0);
    assert_eq!(execute(0xFB91_F0F2, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0);
    assert_eq!(execute(0xFBB1_F0F2, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0);
}

// `SMLALxy` encodings verified with
// `arm-none-eabi-as -mthumb -mcpu=cortex-m33` + `arm-none-eabi-objdump -d`:
//   fbc2 0183  smlalbb  r0, r1, r2, r3   (bytes 83 01 c2 fb)
//   fbc2 0193  smlalbt  r0, r1, r2, r3   (bytes 93 01 c2 fb)
//   fbc2 01a3  smlaltb  r0, r1, r2, r3   (bytes a3 01 c2 fb)
//   fbc2 01b3  smlaltt  r0, r1, r2, r3   (bytes b3 01 c2 fb)
// For comparison the neighbouring encodings are
//   fbc2 0103  smlal   r0, r1, r2, r3   (bits[7:4] = 0000)
//   fbc2 01c3  smlald  r0, r1, r2, r3   (bits[7:4] = 1100)
// bits[7:6] = 0b10 is the SMLALxy discriminator; bit 5 selects the top
// halfword of Rn, bit 4 the top halfword of Rm.
// SMLALxy 编码用 arm-none-eabi-as/objdump 核对（见上）；bits[7:6] = 0b10 为
// 判据，位 5 选 Rn 高半字、位 4 选 Rm 高半字。

/// Encoding of `SMLALBB r0, r1, r2, r3` / `SMLALBB r0, r1, r2, r3` 的编码
const SMLALBB_R0_R1_R2_R3: u32 = 0xFBC2_0183;
/// Encoding of `SMLALBT r0, r1, r2, r3` / `SMLALBT r0, r1, r2, r3` 的编码
const SMLALBT_R0_R1_R2_R3: u32 = 0xFBC2_0193;
/// Encoding of `SMLALTB r0, r1, r2, r3` / `SMLALTB r0, r1, r2, r3` 的编码
const SMLALTB_R0_R1_R2_R3: u32 = 0xFBC2_01A3;
/// Encoding of `SMLALTT r0, r1, r2, r3` / `SMLALTT r0, r1, r2, r3` 的编码
const SMLALTT_R0_R1_R2_R3: u32 = 0xFBC2_01B3;

#[test]
fn smlalbb_accumulates_low_halfword_product() {
    let mut r = CortexM33Registers::new();
    r.write(0, 5); // RdLo
    r.write(1, 0); // RdHi
    r.write(2, 0x0003_0007);
    r.write(3, 0x0002_0004);
    assert_eq!(execute(SMLALBB_R0_R1_R2_R3, &mut r), StepResult::Ok);
    // 7 * 4 = 28 added to the 64-bit accumulator 0:00000005.
    // 7 * 4 = 28，累加到 64 位累加器 0:00000005。
    assert_eq!(r.read(0), 33);
    assert_eq!(r.read(1), 0);
}

#[test]
fn smlalbt_multiplies_by_the_top_halfword_of_rm() {
    let mut r = CortexM33Registers::new();
    // Rm top half 0x8000 is -32768, so 2 * -32768 = -65536.
    // Rm 高半字 0x8000 即 -32768，因此 2 * -32768 = -65536。
    r.write(2, 0x0000_0002);
    r.write(3, 0x8000_0003);
    assert_eq!(execute(SMLALBT_R0_R1_R2_R3, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_0000);
    assert_eq!(r.read(1), 0xFFFF_FFFF);
}

#[test]
fn smlaltb_multiplies_by_the_top_halfword_of_rn() {
    let mut r = CortexM33Registers::new();
    r.write(0, 10);
    // Rn top half 0xFFFF is -1, so -1 * 3 = -3 and 10 - 3 = 7.
    // Rn 高半字 0xFFFF 即 -1，因此 -1 * 3 = -3，10 - 3 = 7。
    r.write(2, 0xFFFF_0002);
    r.write(3, 0x0000_0003);
    assert_eq!(execute(SMLALTB_R0_R1_R2_R3, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 7);
    assert_eq!(r.read(1), 0);
}

#[test]
fn smlaltt_multiplies_both_top_halfwords() {
    let mut r = CortexM33Registers::new();
    // (-32768) * 0x7FFF = -1073709056 = 0xFFFF_FFFF_C000_8000.
    // (-32768) * 0x7FFF = -1073709056，即 0xFFFF_FFFF_C000_8000。
    r.write(2, 0x8000_FFFF);
    r.write(3, 0x7FFF_0000);
    assert_eq!(execute(SMLALTT_R0_R1_R2_R3, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xC000_8000);
    assert_eq!(r.read(1), 0xFFFF_FFFF);
}

#[test]
fn smlalxy_carries_into_the_high_word() {
    let mut r = CortexM33Registers::new();
    // 0xFFFF_FFFF + 0x7FFF * 0x7FFF = 0x0000_0001_3FFF_0000.
    // 0xFFFF_FFFF + 0x7FFF * 0x7FFF = 0x0000_0001_3FFF_0000。
    r.write(0, 0xFFFF_FFFF);
    r.write(1, 0);
    r.write(2, 0x0000_7FFF);
    r.write(3, 0x0000_7FFF);
    assert_eq!(execute(SMLALBB_R0_R1_R2_R3, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x3FFF_0000);
    assert_eq!(r.read(1), 1);
}

#[test]
fn smlalxy_wraps_the_64_bit_accumulator() {
    // 0xFFFF_FFFF_FFFF_FFFF + 0x3FFF_0001 wraps modulo 2^64 to 0x3FFF_0000.
    // 0xFFFF_FFFF_FFFF_FFFF + 0x3FFF_0001 按模 2^64 回绕为 0x3FFF_0000。
    let mut r = CortexM33Registers::new();
    r.write(0, 0xFFFF_FFFF);
    r.write(1, 0xFFFF_FFFF);
    r.write(2, 0x0000_7FFF);
    r.write(3, 0x0000_7FFF);
    assert_eq!(execute(SMLALBB_R0_R1_R2_R3, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x3FFF_0000);
    assert_eq!(r.read(1), 0);

    // Borrowing below zero wraps the same way: 0 + (0xFFFF * 1) = -1.
    // 下溢同样回绕：0 + (0xFFFF * 1) = -1。
    let mut r = CortexM33Registers::new();
    r.write(2, 0x0000_FFFF);
    r.write(3, 0x0000_0001);
    assert_eq!(execute(SMLALBB_R0_R1_R2_R3, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_FFFF);
    assert_eq!(r.read(1), 0xFFFF_FFFF);
}

#[test]
fn smlalxy_leaves_every_flag_unchanged() {
    let mut r = CortexM33Registers::new();
    // N, Z, C, V, Q plus T and the GE nibble must survive the instruction.
    // N、Z、C、V、Q 以及 T 与 GE 半字节必须保持不变。
    let flags = 0xF90F_0000;
    r.xpsr = flags;
    r.write(0, 0xFFFF_FFFF);
    r.write(1, 0x7FFF_FFFF);
    r.write(2, 0xFFFF_8000);
    r.write(3, 0x8000_FFFF);
    assert_eq!(execute(SMLALTT_R0_R1_R2_R3, &mut r), StepResult::Ok);
    assert_eq!(r.xpsr, flags, "SMLALxy must not touch APSR.Q / NZCV / GE");
}

#[test]
fn smlalxy_rejects_reserved_control_bits() {
    // bits[7:6] other than 0b00/0b10/0b11 are unallocated, and 0b111x is
    // reserved for SMLSLD. PC operands are unpredictable as well.
    // bits[7:6] 除 0b00/0b10/0b11 外未分配，0b111x 属于 SMLSLD；PC 操作数
    // 同样为不可预测编码。
    let mut r = CortexM33Registers::new();
    for instr in [
        0xFBC2_0133, // bits[7:4] = 0b0011
        0xFBC2_0143, // bits[7:4] = 0b0100
        0xFBC2_0173, // bits[7:4] = 0b0111
        0xFBC2_01E3, // bits[7:4] = 0b1110
        0xFBC2_01F3, // bits[7:4] = 0b1111
        0xFBC2_0F83, // RdHi = 15
        0xFBC2_F083, // RdLo = 15
        0xFBCF_0183, // Rn = 15
        0xFBC2_018F, // Rm = 15
    ] {
        assert_eq!(
            execute(instr, &mut r),
            StepResult::Undefined,
            "{instr:#010X} must stay undefined / 必须保持未定义"
        );
    }
}

#[test]
fn smlalbb_with_high_register_numbers_matches_assembler() {
    // fbc6 4587  smlalbb r4, r5, r6, r7
    let mut r = CortexM33Registers::new();
    r.write(4, 1);
    r.write(5, 0);
    r.write(6, 3);
    r.write(7, 5);
    assert_eq!(execute(0xFBC6_4587, &mut r), StepResult::Ok);
    assert_eq!(r.read(4), 16);
    assert_eq!(r.read(5), 0);
}
