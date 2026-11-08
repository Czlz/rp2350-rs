//! Load/store tests: register offset, immediate offset, LDRD/STRD
//! 加载/存储测试：寄存器偏移、立即数偏移、LDRD/STRD

use crate::execute_thumb16::StepResult;
use crate::execute_thumb32::test_support::{TestBus, run};
use crate::registers::CortexM33Registers;

// === Load/store single ===

#[test]
fn str_register_offset() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    r.write(0, 0xDEAD_BEEF);
    r.write(1, 0x100);
    r.write(2, 4);
    assert_eq!(run(0xF841_0002, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u32(0x104), 0xDEAD_BEEF);
}

#[test]
fn ldr_register_offset() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x104, 0x1122_3344);
    r.write(1, 0x100);
    r.write(2, 4);
    assert_eq!(run(0xF851_0002, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 0x1122_3344);
}

#[test]
fn strb_and_ldrb_register_offset() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    r.write(0, 0xAB);
    r.write(1, 0x100);
    r.write(2, 5);
    assert_eq!(run(0xF801_0002, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u8(0x105), 0xAB);
    r.write(0, 0);
    assert_eq!(run(0xF811_0002, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 0xAB);
}

#[test]
fn strh_and_ldrh_register_offset() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    r.write(0, 0x1234);
    r.write(1, 0x100);
    r.write(2, 6);
    assert_eq!(run(0xF821_0002, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 0x1234);
    r.write(0, 0);
    assert_eq!(run(0xF831_0002, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 0x1234);
}

#[test]
fn ldrsb_sign_extends() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u8(0x105, 0x80);
    r.write(1, 0x100);
    r.write(2, 5);
    assert_eq!(run(0xF911_0002, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_FF80);
}

#[test]
fn ldrsh_sign_extends() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u16(0x104, 0x8000);
    r.write(1, 0x100);
    r.write(2, 4);
    assert_eq!(run(0xF931_0002, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_8000);
}

#[test]
fn str_and_ldr_imm12() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    r.write(0, 0xCAFE_BABE);
    r.write(1, 0x100);
    assert_eq!(run(0xF8C1_0004, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u32(0x104), 0xCAFE_BABE);
    r.write(0, 0);
    assert_eq!(run(0xF8D1_0004, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 0xCAFE_BABE);
}

// === LDRD/STRD ===

#[test]
fn strd_and_ldrd_positive_offset() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    r.write(0, 0x1111_2222);
    r.write(1, 0x3333_4444);
    r.write(2, 0x100);
    assert_eq!(run(0xE9C2_0102, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u32(0x108), 0x1111_2222);
    assert_eq!(bus.peek_u32(0x10C), 0x3333_4444);

    r.write(0, 0);
    r.write(1, 0);
    assert_eq!(run(0xE9D2_0102, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 0x1111_2222);
    assert_eq!(r.read(1), 0x3333_4444);
}

#[test]
fn strd_negative_offset() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    r.write(0, 0xAAAA_BBBB);
    r.write(1, 0xCCCC_DDDD);
    r.write(2, 0x110);
    assert_eq!(run(0xE942_0102, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u32(0x108), 0xAAAA_BBBB);
    assert_eq!(bus.peek_u32(0x10C), 0xCCCC_DDDD);
}

#[test]
fn strd_post_index_updates_base() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    r.write(0, 0x5555_6666);
    r.write(1, 0x7777_8888);
    r.write(2, 0x100);
    assert_eq!(run(0xE8E2_0102, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u32(0x100), 0x5555_6666);
    assert_eq!(bus.peek_u32(0x104), 0x7777_8888);
    assert_eq!(r.read(2), 0x108);
}

#[test]
fn ldrd_pre_index_write_back() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x108, 0x9999_AAAA);
    bus.poke_u32(0x10C, 0xBBBB_CCCC);
    r.write(2, 0x100);
    assert_eq!(run(0xE9F2_0102, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 0x9999_AAAA);
    assert_eq!(r.read(1), 0xBBBB_CCCC);
    assert_eq!(r.read(2), 0x108);
}
