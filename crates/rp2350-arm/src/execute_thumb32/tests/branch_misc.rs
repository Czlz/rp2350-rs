//! Branch, miscellaneous, plain-immediate, and bitfield tests
//! 分支、杂项、纯二进制立即数与位域测试

use crate::execute_thumb16::StepResult;
use crate::execute_thumb32::test_support::execute;
use crate::registers::CortexM33Registers;

// === Branches and miscellaneous ===

#[test]
fn beq_w_taken() {
    let mut r = CortexM33Registers::new();
    r.pc = 0x100;
    r.set_apsr_nzcv(false, true, false, false);
    assert_eq!(execute(0xF000_8032, &mut r), StepResult::BranchTaken);
    assert_eq!(r.pc, 0x164);
}

#[test]
fn beq_w_not_taken() {
    let mut r = CortexM33Registers::new();
    r.pc = 0x100;
    r.set_apsr_nzcv(false, false, false, false);
    assert_eq!(execute(0xF000_8032, &mut r), StepResult::Ok);
    assert_eq!(r.pc, 0x100);
}

#[test]
fn b_w_t4_branch() {
    let mut r = CortexM33Registers::new();
    r.pc = 0x100;
    assert_eq!(execute(0xF000_B82E, &mut r), StepResult::BranchTaken);
    assert_eq!(r.pc, 0x15C);
}

#[test]
fn bl_sets_link_register() {
    let mut r = CortexM33Registers::new();
    r.pc = 0x100;
    assert_eq!(execute(0xF000_F82C, &mut r), StepResult::BranchTaken);
    assert_eq!(r.pc, 0x158);
    assert_eq!(r.lr, 0x101);
}

#[test]
fn blx_switches_to_arm_state() {
    let mut r = CortexM33Registers::new();
    r.pc = 0x100;
    assert_eq!(execute(0xF000_E800, &mut r), StepResult::BranchTaken);
    assert_eq!(r.pc, 0x100);
    assert_eq!(r.lr, 0x101);
}

#[test]
fn blx_rejects_misaligned_target() {
    let mut r = CortexM33Registers::new();
    r.pc = 0x100;
    assert_eq!(execute(0xF000_E801, &mut r), StepResult::Undefined);
    assert_eq!(r.pc, 0x100);
}

#[test]
fn mrs_apsr() {
    let mut r = CortexM33Registers::new();
    r.set_apsr_nzcv(true, true, false, false);
    assert_eq!(execute(0xF3EF_8000, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xC000_0000);
}

#[test]
fn mrs_ipsr() {
    let mut r = CortexM33Registers::new();
    r.xpsr |= 5;
    assert_eq!(execute(0xF3EF_8005, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 5);
}

#[test]
fn mrs_control() {
    let mut r = CortexM33Registers::new();
    r.control = 3;
    assert_eq!(execute(0xF3EF_8014, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 3);
}

#[test]
fn msr_basepri_max_only_raises_priority() {
    // CircuitPython uses `msr BASEPRI_MAX, r2` around its critical sections.
    // A numerically lower priority value raises the mask; zero or a larger
    // value must leave BASEPRI untouched.
    // CircuitPython 在其临界区使用 `msr BASEPRI_MAX, r2`。数值更小的优先级
    // 会提高掩码；零或更大的值必须保持 BASEPRI 不变。
    let mut r = CortexM33Registers::new();
    r.write(2, 0x60);
    assert_eq!(execute(0xF382_8812, &mut r), StepResult::Ok);
    assert_eq!(r.basepri, 0x60);
    r.write(2, 0x80);
    assert_eq!(execute(0xF382_8812, &mut r), StepResult::Ok);
    assert_eq!(r.basepri, 0x60);
    r.write(2, 0);
    assert_eq!(execute(0xF382_8812, &mut r), StepResult::Ok);
    assert_eq!(r.basepri, 0x60);
    r.write(2, 0x20);
    assert_eq!(execute(0xF382_8812, &mut r), StepResult::Ok);
    assert_eq!(r.basepri, 0x20);
}

#[test]
fn msr_apsr() {
    let mut r = CortexM33Registers::new();
    r.write(0, 0x8000_0000);
    assert_eq!(execute(0xF380_8800, &mut r), StepResult::Ok);
    assert!(r.apsr_n());
    assert!(!r.apsr_z());
    assert!(!r.apsr_c());
    assert!(!r.apsr_v());
}

#[test]
fn msr_primask() {
    let mut r = CortexM33Registers::new();
    r.write(0, 1);
    assert_eq!(execute(0xF380_8810, &mut r), StepResult::Ok);
    assert_eq!(r.primask, 1);
}

#[test]
fn msr_and_mrs_stack_limits() {
    // `msr msplim, r0` is what Pico SDK crt0 clears on every core.
    // Pico SDK 的 crt0 在每个核上用 `msr msplim, r0` 清零。
    let mut r = CortexM33Registers::new();
    r.write(0, 0x2008_1FFF);
    assert_eq!(execute(0xF380_880A, &mut r), StepResult::Ok);
    assert_eq!(r.msplim, 0x2008_1FF8, "bits[2:0] are reserved");
    assert_eq!(execute(0xF3EF_800A, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x2008_1FF8);

    r.write(1, 0x2000_4007);
    assert_eq!(execute(0xF381_880B, &mut r), StepResult::Ok);
    assert_eq!(r.psplim, 0x2000_4000);
    assert_eq!(execute(0xF3EF_810B, &mut r), StepResult::Ok);
    assert_eq!(r.read(1), 0x2000_4000);
}

#[test]
fn barriers_execute_as_no_ops() {
    let mut r = CortexM33Registers::new();
    assert_eq!(execute(0xF3BF_8F4F, &mut r), StepResult::Ok); // DSB
    assert_eq!(execute(0xF3BF_8F5F, &mut r), StepResult::Ok); // DMB
    assert_eq!(execute(0xF3BF_8F6F, &mut r), StepResult::Ok); // ISB
}

#[test]
fn hint_instructions() {
    let mut r = CortexM33Registers::new();
    assert_eq!(execute(0xF3AF_8000, &mut r), StepResult::Ok); // NOP
    assert_eq!(execute(0xF3AF_8001, &mut r), StepResult::Ok); // YIELD
    assert_eq!(execute(0xF3AF_8002, &mut r), StepResult::Sleep); // WFE
    assert_eq!(execute(0xF3AF_8003, &mut r), StepResult::Sleep); // WFI
    assert_eq!(execute(0xF3AF_8004, &mut r), StepResult::Ok); // SEV
}

// === Plain-binary immediate ===

#[test]
fn movw_loads_16_bits() {
    let mut r = CortexM33Registers::new();
    assert_eq!(execute(0xF241_2034, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x1234);
}

#[test]
fn movt_loads_top_half() {
    let mut r = CortexM33Registers::new();
    r.write(0, 0x5678);
    assert_eq!(execute(0xF2C1_2034, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x1234_5678);
}

#[test]
fn addw_adds_12_bit_immediate() {
    let mut r = CortexM33Registers::new();
    r.write(1, 10);
    assert_eq!(execute(0xF201_0064, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 110);
}

#[test]
fn subw_subtracts_12_bit_immediate() {
    let mut r = CortexM33Registers::new();
    r.write(1, 110);
    assert_eq!(execute(0xF2A1_0064, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 10);
}

#[test]
fn movw_loads_high_bit_immediate() {
    let mut r = CortexM33Registers::new();
    assert_eq!(execute(0xF640_0000, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x800);
}

#[test]
fn addw_adds_high_bit_immediate() {
    let mut r = CortexM33Registers::new();
    r.write(0, 1);
    assert_eq!(execute(0xF600_0000, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x801);
}

#[test]
fn subw_subtracts_high_bit_immediate() {
    let mut r = CortexM33Registers::new();
    r.write(0, 0x1000);
    assert_eq!(execute(0xF6A0_0000, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0x800);
}

// === Bitfield ===

#[test]
fn sbfx_sign_extends_field() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0x0000_0F80);
    assert_eq!(execute(0xF341_1007, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_FFF8);
}

#[test]
fn ubfx_zero_extends_field() {
    let mut r = CortexM33Registers::new();
    r.write(1, 0xABCD);
    assert_eq!(execute(0xF3C1_1007, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xBC);
}

#[test]
fn bfi_inserts_field() {
    let mut r = CortexM33Registers::new();
    r.write(0, 0xFFFF_FFFF);
    r.write(1, 0x0ABC);
    assert_eq!(execute(0xF361_100F, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_ABCF);
}

#[test]
fn bfc_clears_field() {
    let mut r = CortexM33Registers::new();
    r.write(0, 0xFFFF_FFFF);
    assert_eq!(execute(0xF36F_100F, &mut r), StepResult::Ok);
    assert_eq!(r.read(0), 0xFFFF_000F);
}
