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
