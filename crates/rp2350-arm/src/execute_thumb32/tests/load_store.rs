//! Load/store tests: register offset, immediate offset, pre/post-index,
//! literal loads, LDM/STM, exclusive and table branch
//! 加载/存储测试：寄存器偏移、立即数偏移、前/后索引、字面量加载、
//! LDM/STM、独占访问与表分支

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

// === Pre/post-indexed 8-bit immediate ===

#[test]
fn ldr_post_index_updates_base() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x100, 0x1122_3344);
    r.write(4, 0x100);
    assert_eq!(run(0xF854_3B04, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(3), 0x1122_3344);
    assert_eq!(r.read(4), 0x104);
}

#[test]
fn str_post_index_updates_base() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    r.write(3, 0xCAFE_F00D);
    r.write(4, 0x100);
    assert_eq!(run(0xF844_3B04, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u32(0x100), 0xCAFE_F00D);
    assert_eq!(r.read(4), 0x104);
}

#[test]
fn ldr_pre_index_write_back() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x104, 0x1234_5678);
    r.write(4, 0x100);
    assert_eq!(run(0xF854_3F04, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(3), 0x1234_5678);
    assert_eq!(r.read(4), 0x104);
}

#[test]
fn ldr_negative_offset() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x0FC, 0x00FF_00FF);
    r.write(4, 0x100);
    assert_eq!(run(0xF854_3C04, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(3), 0x00FF_00FF);
    assert_eq!(r.read(4), 0x100);
}

#[test]
fn ldrb_negative_offset() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u8(0x0FC, 0xA5);
    r.write(1, 0x100);
    assert_eq!(run(0xF811_3C04, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(3), 0xA5);
}

#[test]
fn ldrsb_negative_offset_sign_extends() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u8(0x100, 0x80);
    r.write(3, 0x104);
    assert_eq!(run(0xF913_3C04, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(3), 0xFFFF_FF80);
}

#[test]
fn ldrsh_negative_offset_sign_extends() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u16(0x100, 0x8000);
    r.write(3, 0x104);
    assert_eq!(run(0xF933_3C04, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(3), 0xFFFF_8000);
}

#[test]
fn ldr_imm12_uses_all_twelve_bits() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0xC04, 0xDEAD_BEEF);
    r.write(1, 0x100);
    assert_eq!(run(0xF8D1_3B04, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(3), 0xDEAD_BEEF);
}

#[test]
fn ldr_literal_positive_offset() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x10C, 0x5555_AAAA);
    r.pc = 0x108;
    assert_eq!(run(0xF8DF_0004, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 0x5555_AAAA);
}

#[test]
fn ldr_literal_negative_offset() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x104, 0xAAAA_5555);
    r.pc = 0x108;
    assert_eq!(run(0xF85F_0004, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 0xAAAA_5555);
}

// === LDM/STM ===

#[test]
fn ldm_loads_registers_and_updates_base() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x100, 0x1111_1111);
    bus.poke_u32(0x104, 0x2222_2222);
    r.write(0, 0x100);
    assert_eq!(run(0xE8B0_0006, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(1), 0x1111_1111);
    assert_eq!(r.read(2), 0x2222_2222);
    assert_eq!(r.read(0), 0x108);
}

#[test]
fn stm_stores_registers_and_updates_base() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    r.write(0, 0x100);
    r.write(1, 0xAAAA_AAAA);
    r.write(2, 0xBBBB_BBBB);
    assert_eq!(run(0xE8A0_0006, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u32(0x100), 0xAAAA_AAAA);
    assert_eq!(bus.peek_u32(0x104), 0xBBBB_BBBB);
    assert_eq!(r.read(0), 0x108);
}

#[test]
fn ldm_without_write_back_keeps_base() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x100, 0x1234_5678);
    r.write(2, 0x100);
    assert_eq!(run(0xE892_4000, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(14), 0x1234_5678);
    assert_eq!(r.read(2), 0x100);
}

#[test]
fn ldm_with_base_in_list_keeps_the_loaded_value() {
    // Same Armv8-M rule for the Thumb-32 encoding: with W=1 and Rn in the
    // register list, the load wins over the write-back.
    // Thumb-32 编码遵循同一 Armv8-M 规则：W=1 且 Rn 在寄存器列表时，
    // 加载值覆盖写回值。
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x100, 0xAAAA_AAAA);
    bus.poke_u32(0x104, 0xBBBB_BBBB);
    r.write(1, 0x100);
    assert_eq!(run(0xE8B1_0006, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(1), 0xAAAA_AAAA, "base must hold the loaded value");
    assert_eq!(r.read(2), 0xBBBB_BBBB);
}

#[test]
fn stm_with_base_in_list_still_writes_back() {
    // For stores the write-back is always performed, so the base ends at the
    // post-increment address. The first register in the list stores the
    // initial base value.
    // 存储指令始终执行写回，基址最终为后递增地址；列表首项存入初始基址。
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    r.write(0, 0x100);
    r.write(1, 0xAAAA_AAAA);
    assert_eq!(run(0xE8A0_0003, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u32(0x100), 0x100);
    assert_eq!(bus.peek_u32(0x104), 0xAAAA_AAAA);
    assert_eq!(r.read(0), 0x108);
}

#[test]
fn push_uses_stmdb() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    r.sp = 0x110;
    r.write(11, 0xBBBB_BBBB);
    r.write(14, 0xCCCC_CCCC);
    assert_eq!(run(0xE92D_4800, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u32(0x108), 0xBBBB_BBBB);
    assert_eq!(bus.peek_u32(0x10C), 0xCCCC_CCCC);
    assert_eq!(r.sp, 0x108);
}

#[test]
fn pop_uses_ldmia_and_branches_to_loaded_pc() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    r.sp = 0x108;
    bus.poke_u32(0x108, 0x1111_1111);
    bus.poke_u32(0x10C, 0x0000_0109);
    assert_eq!(run(0xE8BD_8800, &mut r, &mut bus), StepResult::BranchTaken);
    assert_eq!(r.read(11), 0x1111_1111);
    assert_eq!(r.pc, 0x108);
    assert_eq!(r.sp, 0x110);
}

// === Exclusive and table branch ===

#[test]
fn ldrex_loads_and_scales_offset() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x110, 0x1357_9BDF);
    r.write(0, 0x100);
    assert_eq!(run(0xE850_1F04, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(1), 0x1357_9BDF);
}

#[test]
fn strex_stores_and_reports_success() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    r.write(2, 0x100);
    // Arm the monitor with the paired LDREX before the STREX.
    // 先用配对的 LDREX 启用监视器，再执行 STREX。
    assert_eq!(run(0xE852_1F00, &mut r, &mut bus), StepResult::Ok);
    r.write(1, 0x2468_ACE0);
    assert_eq!(run(0xE842_1000, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u32(0x100), 0x2468_ACE0);
    assert_eq!(r.read(0), 0);
}

#[test]
fn strex_without_matching_ldrex_reports_failure() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u8(0x100, 0xA5);
    r.write(0, 0x100);
    // LDREX arms 0x104 as a word, so the byte STREX at 0x100 must fail.
    // LDREX 监视的是 0x104 的字，因此 0x100 的字节 STREX 必须失败。
    assert_eq!(run(0xE850_1F04, &mut r, &mut bus), StepResult::Ok);
    r.write(1, 0x5A);
    assert_eq!(run(0xE8C0_1F42, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u8(0x100), 0xA5);
    assert_eq!(r.read(2), 1);
}

#[test]
fn strexb_and_strexh_transfer_sized_values() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u8(0x100, 0xA5);
    r.write(2, 0x100);
    assert_eq!(run(0xE8D2_0F4F, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 0xA5);
    r.write(1, 0x5A);
    assert_eq!(run(0xE8C2_1F43, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u8(0x100), 0x5A);
    assert_eq!(r.read(3), 0);

    bus.poke_u16(0x100, 0x1234);
    r.write(2, 0x100);
    assert_eq!(run(0xE8D2_0F5F, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 0x1234);
    r.write(1, 0xBEEF);
    assert_eq!(run(0xE8C2_1F53, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u8(0x100), 0xEF);
    assert_eq!(bus.peek_u8(0x101), 0xBE);
    assert_eq!(r.read(3), 0);
}

#[test]
fn ldrexd_and_strexd_move_two_words() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x100, 0x1111_2222);
    bus.poke_u32(0x104, 0x3333_4444);
    r.write(2, 0x100);
    assert_eq!(run(0xE8D2_017F, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 0x1111_2222);
    assert_eq!(r.read(1), 0x3333_4444);
    assert_eq!(run(0xE8C2_0173, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u32(0x100), 0x1111_2222);
    assert_eq!(bus.peek_u32(0x104), 0x3333_4444);
    assert_eq!(r.read(3), 0);
}

#[test]
fn stlb_stlh_and_ldab_ldah_access_bytes_and_halfwords() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    r.write(1, 0x100);
    r.write(2, 0xAB);
    assert_eq!(run(0xE8C1_2F8F, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u8(0x100), 0xAB);
    r.write(3, 0);
    assert_eq!(run(0xE8D1_3F8F, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(3), 0xAB);

    r.write(2, 0x1234);
    assert_eq!(run(0xE8C1_2F9F, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u8(0x100), 0x34);
    assert_eq!(bus.peek_u8(0x101), 0x12);
    r.write(3, 0);
    assert_eq!(run(0xE8D1_3F9F, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(3), 0x1234);
}

#[test]
fn stl_and_lda_access_words() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    r.write(1, 0x100);
    r.write(2, 0xDEAD_BEEF);
    assert_eq!(run(0xE8C1_2FAF, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u32(0x100), 0xDEAD_BEEF);
    r.write(3, 0);
    assert_eq!(run(0xE8D1_3FAF, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(3), 0xDEAD_BEEF);
}

#[test]
fn ldaex_and_stlex_use_the_exclusive_monitor() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x100, 0x1234_5678);
    r.write(1, 0x100);
    assert_eq!(run(0xE8D1_2FEF, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(2), 0x1234_5678);
    assert_eq!(run(0xE8C1_2FE0, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u32(0x100), 0x1234_5678);
    assert_eq!(r.read(0), 0);
}

#[test]
fn tt_reports_secure_access_and_is_not_strex() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    // Sentinel at address 0: a STREX misdecode would overwrite it with zero.
    // 地址 0 的哨兵值：若误判为 STREX 会被写入 0。
    bus.poke_u32(0, 0x2000_1000);
    r.write(2, 0);
    assert_eq!(run(0xE842_F200, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(2), 0x004C_0000);
    assert_eq!(bus.peek_u32(0), 0x2000_1000);
}

#[test]
fn tbb_branches_through_byte_table() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    r.write(1, 0x200);
    r.write(2, 3);
    bus.poke_u8(0x203, 5);
    r.pc = 0x100;
    assert_eq!(run(0xE8D1_F002, &mut r, &mut bus), StepResult::BranchTaken);
    assert_eq!(r.pc, 0x10A);
}

#[test]
fn tbh_branches_through_halfword_table() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    r.write(1, 0x200);
    r.write(2, 3);
    bus.poke_u16(0x206, 7);
    r.pc = 0x100;
    assert_eq!(run(0xE8D1_F012, &mut r, &mut bus), StepResult::BranchTaken);
    assert_eq!(r.pc, 0x10E);
}
