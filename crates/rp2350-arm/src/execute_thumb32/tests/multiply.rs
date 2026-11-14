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
