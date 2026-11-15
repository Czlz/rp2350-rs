//! Exclusive monitor tests: per-core isolation, `STREX` pairing and `CLREX`
//! 独占监视器测试：每核隔离、`STREX` 配对与 `CLREX`

use crate::execute_thumb16::StepResult;
use crate::execute_thumb32::test_support::{TestBus, run};
use crate::registers::CortexM33Registers;

/// `LDREX r1, [r2]` / `LDREX r1, [r2]`
const LDREX_R1_R2: u32 = 0xE852_1F00;
/// `STREX r0, r1, [r2]` / `STREX r0, r1, [r2]`
const STREX_R0_R1_R2: u32 = 0xE842_1000;
/// `STREX r3, r1, [r2]` — keeps the status away from R0
/// `STREX r3, r1, [r2]` —— 状态写入 R3，不干扰 R0
const STREX_R3_R1_R2: u32 = 0xE842_1300;
/// `LDREXB r0, [r2]` / `LDREXB r0, [r2]`
const LDREXB_R0_R2: u32 = 0xE8D2_0F4F;
/// `STREXB r0, r1, [r2]` / `STREXB r0, r1, [r2]`
const STREXB_R0_R1_R2: u32 = 0xE8C2_1F40;
/// `CLREX` (`1111 0011 1011 1111 1000 1111 0010 1111`)
/// `CLREX`（`1111 0011 1011 1111 1000 1111 0010 1111`）
const CLREX: u32 = 0xF3BF_8F2F;

#[test]
fn new_registers_start_without_an_exclusive_monitor() {
    let regs = CortexM33Registers::new();
    assert_eq!(regs.exclusive, None);
    // Default delegates to `new`, so it must start disarmed as well.
    // `Default` 委托给 `new`，因此同样必须从未武装状态开始。
    assert_eq!(CortexM33Registers::default().exclusive, None);
}

#[test]
fn ldrex_then_strex_on_the_same_core_succeeds() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x100, 0xAAAA_BBBB);
    r.write(2, 0x100);

    assert_eq!(run(LDREX_R1_R2, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(1), 0xAAAA_BBBB);
    assert_eq!(r.exclusive, Some((0x100, 4)));

    r.write(1, 0x1234_5678);
    assert_eq!(run(STREX_R0_R1_R2, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 0, "STREX must report success as 0");
    assert_eq!(bus.peek_u32(0x100), 0x1234_5678);
    assert_eq!(r.exclusive, None, "STREX consumes the monitor");
}

#[test]
fn exclusive_monitor_is_not_shared_between_cores() {
    let mut core0 = CortexM33Registers::new();
    let mut core1 = CortexM33Registers::new();
    let mut bus0 = TestBus::new();
    let mut bus1 = TestBus::new();
    bus0.poke_u32(0x100, 0x1111_1111);
    bus1.poke_u32(0x100, 0x1111_1111);
    core0.write(2, 0x100);
    core1.write(2, 0x100);
    core1.write(1, 0x2222_2222);

    // core0 arms its own monitor; the two register files stay independent.
    // core0 武装自己的监视器；两个寄存器文件保持独立。
    assert_eq!(run(LDREX_R1_R2, &mut core0, &mut bus0), StepResult::Ok);
    assert_eq!(core1.exclusive, None);

    assert_eq!(run(STREX_R0_R1_R2, &mut core1, &mut bus1), StepResult::Ok);
    assert_eq!(core1.read(0), 1, "cross-core STREX must fail");
    assert_eq!(bus1.peek_u32(0x100), 0x1111_1111, "no store on failure");

    // core1's failure must not disturb core0's still-armed monitor.
    // core1 的失败不得影响 core0 仍然武装的监视器。
    assert_eq!(core0.exclusive, Some((0x100, 4)));
    core0.write(1, 0x3333_3333);
    assert_eq!(run(STREX_R0_R1_R2, &mut core0, &mut bus0), StepResult::Ok);
    assert_eq!(core0.read(0), 0);
    assert_eq!(bus0.peek_u32(0x100), 0x3333_3333);
}

#[test]
fn strex_without_ldrex_fails_and_writes_nothing() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x100, 0x2000_0000);
    r.write(2, 0x100);
    r.write(1, 0xDEAD_BEEF);

    assert_eq!(run(STREX_R3_R1_R2, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(3), 1, "unarmed monitor must report failure");
    assert_eq!(bus.peek_u32(0x100), 0x2000_0000);
}

#[test]
fn exclusive_monitor_rejects_a_different_address_and_clears() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x100, 0x1111_1111);
    bus.poke_u32(0x104, 0x2222_2222);
    r.write(2, 0x100);
    assert_eq!(run(LDREX_R1_R2, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.exclusive, Some((0x100, 4)));

    // Same core, same width, different address.
    // 同核、同宽度、不同地址。
    r.write(2, 0x104);
    r.write(1, 0x3333_3333);
    assert_eq!(run(STREX_R0_R1_R2, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 1);
    assert_eq!(bus.peek_u32(0x104), 0x2222_2222);
    assert_eq!(r.exclusive, None, "a failed STREX clears the monitor");

    // The monitor is gone, so returning to the watched address still fails.
    // 监视器已清除，因此回到被监视地址仍然失败。
    r.write(2, 0x100);
    assert_eq!(run(STREX_R0_R1_R2, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 1);
    assert_eq!(bus.peek_u32(0x100), 0x1111_1111);
}

#[test]
fn exclusive_monitor_rejects_a_size_mismatch_and_clears() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u8(0x100, 0xA5);
    r.write(2, 0x100);

    // A word LDREX arms (0x100, 4); the byte STREXB on the same address must
    // not match it.
    // 字 LDREX 武装 (0x100, 4)；同地址的字节 STREXB 不得匹配。
    assert_eq!(run(LDREX_R1_R2, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.exclusive, Some((0x100, 4)));
    r.write(1, 0x5A);
    assert_eq!(run(STREXB_R0_R1_R2, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 1, "STREXB must fail against a word monitor");
    assert_eq!(bus.peek_u8(0x100), 0xA5);
    assert_eq!(r.exclusive, None);

    // The matching word store fails too: the mismatch retired the monitor.
    // 匹配的字存储同样失败：尺寸不匹配已废弃监视器。
    assert_eq!(run(STREX_R0_R1_R2, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 1);

    // A byte LDREX pairs with a byte STREX at the same address.
    // 字节 LDREX 与同地址的字节 STREX 正常配对。
    assert_eq!(run(LDREXB_R0_R2, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.exclusive, Some((0x100, 1)));
    r.write(1, 0x5A);
    assert_eq!(run(STREXB_R0_R1_R2, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 0);
    assert_eq!(bus.peek_u8(0x100), 0x5A);
}

#[test]
fn a_second_ldrex_replaces_the_first_monitor() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x100, 1);
    bus.poke_u32(0x104, 2);

    r.write(2, 0x100);
    assert_eq!(run(LDREX_R1_R2, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(1), 1);
    r.write(2, 0x104);
    assert_eq!(run(LDREX_R1_R2, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(1), 2);
    assert_eq!(r.exclusive, Some((0x104, 4)));

    // Only the second address is still watched.
    // 只有第二个地址仍在被监视。
    r.write(1, 1);
    r.write(2, 0x100);
    assert_eq!(run(STREX_R0_R1_R2, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 1);
    assert_eq!(bus.peek_u32(0x100), 1, "the first monitor was overwritten");

    // Re-arming both keeps the second one valid.
    // 重新依次武装后，第二次加载的地址有效。
    r.write(2, 0x100);
    assert_eq!(run(LDREX_R1_R2, &mut r, &mut bus), StepResult::Ok);
    r.write(2, 0x104);
    assert_eq!(run(LDREX_R1_R2, &mut r, &mut bus), StepResult::Ok);
    r.write(1, 0x2222_2222);
    r.write(2, 0x104);
    assert_eq!(run(STREX_R0_R1_R2, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 0);
    assert_eq!(bus.peek_u32(0x104), 0x2222_2222);
}

#[test]
fn clrex_clears_the_monitor_and_is_defined() {
    let mut r = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x100, 0x1111_1111);
    r.write(2, 0x100);
    assert_eq!(run(LDREX_R1_R2, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.exclusive, Some((0x100, 4)));

    // CLREX used to fall through to the barrier group as an undefined
    // instruction, which startup firmware tripped over.
    // CLREX 此前落入屏障分组被判为未定义指令，启动固件会因此告警。
    assert_eq!(run(CLREX, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.exclusive, None);

    r.write(1, 0x4444_4444);
    assert_eq!(run(STREX_R0_R1_R2, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.read(0), 1, "STREX after CLREX must fail");
    assert_eq!(bus.peek_u32(0x100), 0x1111_1111);

    // CLREX on an already-clear monitor stays a no-op success.
    // 监视器已清除时 CLREX 仍是无操作的成功指令。
    assert_eq!(run(CLREX, &mut r, &mut bus), StepResult::Ok);
    assert_eq!(r.exclusive, None);
}

#[test]
fn clrex_only_clears_the_executing_core() {
    let mut core0 = CortexM33Registers::new();
    let mut core1 = CortexM33Registers::new();
    let mut bus = TestBus::new();
    bus.poke_u32(0x100, 0x7777_7777);
    core0.write(2, 0x100);
    core1.write(2, 0x100);
    assert_eq!(run(LDREX_R1_R2, &mut core0, &mut bus), StepResult::Ok);
    assert_eq!(run(LDREX_R1_R2, &mut core1, &mut bus), StepResult::Ok);

    assert_eq!(run(CLREX, &mut core0, &mut bus), StepResult::Ok);
    assert_eq!(core0.exclusive, None);
    assert_eq!(core1.exclusive, Some((0x100, 4)));

    core1.write(1, 0x8888_8888);
    assert_eq!(run(STREX_R0_R1_R2, &mut core1, &mut bus), StepResult::Ok);
    assert_eq!(core1.read(0), 0);
    assert_eq!(bus.peek_u32(0x100), 0x8888_8888);
}
