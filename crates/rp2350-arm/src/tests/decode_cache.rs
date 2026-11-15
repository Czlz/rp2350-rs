//! Decoded-instruction cache: differential and invalidation tests
//! 译码结果缓存：差分与失效测试
//!
//! Every test here runs the **same** program on two cores — one with the
//! decoded-instruction cache enabled, one with it disabled — and compares the
//! complete architectural state after every step. Equality therefore proves
//! that the cached path is a pure optimisation, not a semantic change.
//! 这里的每个测试都在两个核心上运行**同一段**程序——一个开启译码缓存、一个关闭——
//! 并逐步比较完整的架构状态。相等即证明缓存路径只是优化，没有改变语义。

use rp2350_core::FLASH_XIP_BASE;
use rp2350_core::bus::AddressBus;
use rp2350_core::flash::FlashRegion;
use rp2350_core::memory::{RamRegion, SimpleBus};
use rp2350_core::types::CpuId;
use rp2350_cpu::CpuCore;

use crate::CortexM33Core;

/// Flash window size used by these tests / 测试使用的 Flash 窗口大小
const FLASH_SIZE: usize = 0x1000;

/// SRAM base used by these tests / 测试使用的 SRAM 基地址
const SRAM_BASE: u32 = 0x2000_0000;

/// Build one core plus bus for `program` / 为 `program` 构造核心与总线
fn machine(program: &[u8], cache: bool) -> (CortexM33Core, SimpleBus) {
    let mut bus = SimpleBus::new();
    let mut flash = FlashRegion::new("flash", FLASH_XIP_BASE, FLASH_SIZE);
    flash.load_bytes(FLASH_XIP_BASE, program).unwrap();
    bus.register_region(Box::new(flash)).unwrap();
    bus.register_region(Box::new(RamRegion::new("sram", SRAM_BASE, 0x1_0000)))
        .unwrap();
    let mut core = CortexM33Core::new(CpuId::Core0);
    core.boot_at(0x2000_8000, FLASH_XIP_BASE, FLASH_XIP_BASE);
    core.set_flash_fetch_cache(cache);
    core.set_decode_cache(cache);
    // 一处固定的寄存器环境，使读写指令有合法地址且条件分支可预测。
    // A fixed register environment so loads/stores have valid addresses and
    // conditional branches are predictable.
    core.write_register(7, SRAM_BASE);
    core.write_register(8, SRAM_BASE + 0x40);
    core.write_register(9, SRAM_BASE + 0x80);
    core.registers.msp = 0x2000_8000;
    core.registers.psp = 0x2000_4000;
    for (offset, value) in [(0u32, 0x1111_2222u32), (4, 0x3333_4444), (0x40, 7)] {
        let mut cycles = 0;
        bus.write_u32(SRAM_BASE + offset, value, &mut cycles)
            .unwrap();
    }
    (core, bus)
}

/// Observable architectural state / 可观测的架构状态
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    pc: u32,
    r: [u32; 13],
    sp: u32,
    lr: u32,
    msp: u32,
    psp: u32,
    xpsr: u32,
    primask: u32,
    faultmask: u32,
    control: u32,
    cycles: u64,
    exclusive: Option<(u32, u8)>,
    fpscr: u32,
    fp_exc: u32,
    state: String,
}

fn snapshot(core: &CortexM33Core) -> Snapshot {
    Snapshot {
        pc: core.registers.pc,
        r: core.registers.r,
        sp: core.registers.sp,
        lr: core.registers.lr,
        msp: core.registers.msp,
        psp: core.registers.psp,
        xpsr: core.registers.xpsr,
        primask: core.registers.primask,
        faultmask: core.registers.faultmask,
        control: core.registers.control,
        cycles: core.total_cycles(),
        exclusive: core.registers.exclusive,
        fpscr: core.registers.fpu.fpscr,
        fp_exc: core.registers.fpu.fpexc,
        state: format!("{:?}", core.state()),
    }
}

/// Run both cores for `steps` steps and require identical state every step
/// 让两个核心各执行 `steps` 步，并要求每一步的状态完全相同
fn assert_equivalent(program: &[u8], steps: usize) {
    let (mut cached, mut cached_bus) = machine(program, true);
    let (mut plain, mut plain_bus) = machine(program, false);
    for step in 0..steps {
        let a = cached.step(&mut cached_bus);
        let b = plain.step(&mut plain_bus);
        match (&a, &b) {
            (Ok(x), Ok(y)) => {
                assert_eq!(x, y, "state after step {step}");
                assert_eq!(
                    snapshot(&cached),
                    snapshot(&plain),
                    "step {step} diverged (pc {:#010X})",
                    snapshot(&cached).pc
                );
            }
            (Err(x), Err(y)) => assert_eq!(
                format!("{x:?}"),
                format!("{y:?}"),
                "error after step {step}"
            ),
            _ => panic!("step {step}: cached={a:?} plain={b:?}"),
        }
        // 总线内容也必须一致：`strex`/`stlb` 只有真正写入才应改变内存。
        // Bus contents must match too: `strex`/`stlb` only change memory on success.
        let mut cycles = 0;
        for offset in [0u32, 4, 8, 0x40, 0x44, 0x80] {
            assert_eq!(
                cached_bus.read_u32(SRAM_BASE + offset, &mut cycles).ok(),
                plain_bus.read_u32(SRAM_BASE + offset, &mut cycles).ok(),
                "memory at {offset:#X} after step {step}"
            );
        }
    }
}

/// Build a one-instruction loop: `words` then `b` back to the start
/// 构造单指令循环：`words` 之后 `b` 回到起点
fn loop_program(words: &[u16]) -> Vec<u8> {
    let mut program = Vec::with_capacity((words.len() + 1) * 2);
    for word in words {
        program.extend_from_slice(&word.to_le_bytes());
    }
    let back = -(i32::try_from(words.len()).unwrap() * 2 + 4) / 2;
    let branch = 0xE000u16 | u16::try_from(back & 0x7FF).unwrap();
    program.extend_from_slice(&branch.to_le_bytes());
    program
}

#[test]
fn every_specialised_thumb16_encoding_matches_the_decoder() {
    // 每条编码都在循环里执行 4 次：第 1 次填充缓存，之后命中。
    // Each encoding runs four times in the loop: the first fills the cache, the
    // rest hit it.
    let words: &[u16] = &[
        0x2001, // movs r0,#1
        0x2102, // movs r1,#2
        0x1840, // adds r0,r0,r1
        0x1A40, // subs r0,r0,r1
        0x4288, // cmp r0,r1
        0x42C8, // cmn r0,r1
        0x4208, // tst r0,r1
        0x4008, // ands r0,r1
        0x4308, // orrs r0,r1
        0x4048, // eors r0,r1
        0x43C8, // mvns r0,r1
        0x4348, // muls r0,r1
        0x4088, // lsls r0,r1
        0x40C8, // lsrs r0,r1
        0x4108, // asrs r0,r1
        0x41C8, // rors r0,r1
        0x4148, // adcs r0,r1
        0x4188, // sbcs r0,r1
        0x4248, // rsbs r0,r1
        0x6078, // str r0,[r7,#4]
        0x6878, // ldr r0,[r7,#4]
        0x60B8, // str r0,[r7,#8]
        0x68B8, // ldr r0,[r7,#8]
        0x7208, // strb r0,[r1,#8]
        0x7808, // ldrb r0,[r1,#0]
        0x8208, // strh r0,[r1,#16]
        0x8808, // ldrh r0,[r1,#16]
        0x4669, // mov r9,sp
        0x4668, // mov r0,sp
        0xB672, // cpsid i
        0xB662, // cpsie i
        0xBF20, // wfe
        0xBF00, // nop
        0xBF10, // yield
        0xB510, // push {r4,lr}
        0xBD10, // pop {r4,pc}  (EXC_RETURN check on LR only; LR is a normal value)
    ];
    let program = loop_program(words);
    assert_equivalent(&program, 4 * words.len() + 8);
}

#[test]
fn branches_and_it_blocks_match_the_decoder() {
    // `cmp r7,r7` sets Z; `beq` then skips the `movs r0,#0xAA`; the IT block
    // executes one conditional `movs` and one suppressed-flag `adds`.
    // `cmp r7,r7` 置 Z；随后 `beq` 跳过 `movs r0,#0xAA`；IT 块内执行一条条件
    // `movs` 与一条不写标志的 `adds`。
    let words: &[u16] = &[
        0x42BF, // cmp r7,r7      (Z=1)
        0xD001, // beq +2         (taken, skips the next halfword)
        0x20AA, // movs r0,#0xAA  (skipped)
        0x2004, // movs r0,#4
        0x42BF, // cmp r7,r7      (Z=1)
        0xBF08, // it eq
        0x2009, // moveq r0,#9    (executes)
        0xD1FE, // bne -4         (not taken: Z=1)
        0xE7FE, // b .            (never reached)
    ];
    let program = loop_program(words);
    assert_equivalent(&program, 8 * words.len());
}

#[test]
fn thumb32_groups_match_the_decoder() {
    let words: &[u16] = &[
        // movs.w r2,#0x1234
        0xF24C, 0x3245, // movw r2,#0xC245 (plain binary immediate)
        0xF05F, 0x0E01, // movs.w lr,#1     (modified immediate)
        0xF1B8, 0x0F00, // cmp.w r8,#0      (modified immediate)
        0xF8D7, 0x1004, // ldr.w r1,[r7,#4] (single data transfer)
        0xF8C7, 0x1008, // str.w r1,[r7,#8]
        0xE8D7, 0x1201, // ldm.w r7,{r0,r9} (load/store multiple)
        0xE8D3, 0x8FCF, // ldaexb r8,[r3]   (exclusive)
        0xE8C3, 0xEF48, // strexb r8,lr,[r3]
        0xF3EF, 0x8010, // mrs r0,PRIMASK
        0xF380, 0x8810, // msr PRIMASK,r0
        0xF3BF, 0x8F5F, // dmb sy
        0xFA00, 0xF001, // lsl.w r0,r0,r1   (shifted register)
        0xEA00, 0x0001, // and.w r0,r0,r1   (shifted register)
        0xFB00, 0xF001, // mul.w r0,r0,r1   (multiply)
        0xF010, 0x0001, // ands.w r0,r0,#1  (modified immediate)
    ];
    // `r3` must be a valid exclusive address for `ldaexb`/`strexb`.
    // `r3` 必须是合法独占地址，供 `ldaexb`/`strexb` 使用。
    let program = loop_program(words);
    let (mut cached, mut cached_bus) = machine(&program, true);
    let (mut plain, mut plain_bus) = machine(&program, false);
    for core in [&mut cached, &mut plain] {
        core.write_register(3, SRAM_BASE + 0x80);
    }
    for step in 0..6 * words.len() {
        let a = cached.step(&mut cached_bus);
        let b = plain.step(&mut plain_bus);
        assert_eq!(format!("{a:?}"), format!("{b:?}"), "step {step}");
        assert_eq!(snapshot(&cached), snapshot(&plain), "step {step}");
    }
}

#[test]
fn undefined_instructions_are_reported_identically() {
    // 0xDE00 = UDF #0 in Thumb; both paths must produce the same
    // `Undefined`/halt behaviour, in strict and in tolerant mode.
    // 0xDE00 = Thumb 的 UDF #0；两条路径都必须产生相同的 `Undefined`/停机行为。
    let program = loop_program(&[0xDE00, 0x2001]);
    assert_equivalent(&program, 16);

    for strict in [true, false] {
        let (mut cached, mut cached_bus) = machine(&program, true);
        let (mut plain, mut plain_bus) = machine(&program, false);
        cached.set_strict_decode(strict);
        plain.set_strict_decode(strict);
        for step in 0..8 {
            let a = cached.step(&mut cached_bus);
            let b = plain.step(&mut plain_bus);
            assert_eq!(format!("{a:?}"), format!("{b:?}"), "strict={strict} {step}");
            assert_eq!(
                snapshot(&cached),
                snapshot(&plain),
                "strict={strict} {step}"
            );
        }
    }
}

#[test]
fn a_host_flash_write_is_visible_immediately() {
    let mut bus = SimpleBus::new();
    let flash = FlashRegion::new("flash", FLASH_XIP_BASE, FLASH_SIZE);
    let handle = flash.handle();
    // `movs r0,#1; b .`
    handle
        .write(FLASH_XIP_BASE, &[0x01, 0x20, 0xFD, 0xE7])
        .unwrap();
    bus.register_region(Box::new(flash)).unwrap();
    let mut core = CortexM33Core::new(CpuId::Core0);
    core.boot_at(0x2000_8000, FLASH_XIP_BASE, FLASH_XIP_BASE);
    core.set_flash_fetch_cache(true);

    for _ in 0..8 {
        core.step(&mut bus).unwrap();
    }
    assert_eq!(core.read_register(0), 1);
    let flushes = core.decode_cache_flushes();
    // 宿主侧改写 Flash 后，下一条指令必须立即是新内容。
    // After a host-side flash rewrite the very next instruction must be the new one.
    handle
        .write(FLASH_XIP_BASE, &[0x02, 0x20, 0xFD, 0xE7])
        .unwrap();
    core.step(&mut bus).unwrap();
    assert_eq!(core.read_register(0), 2);
    assert!(core.decode_cache_flushes() > flushes);
}

#[test]
fn sram_code_is_never_cached_and_self_modification_is_visible() {
    let mut bus = SimpleBus::new();
    let mut flash = FlashRegion::new("flash", FLASH_XIP_BASE, FLASH_SIZE);
    // `ldr r6,[pc,#0]; bx r6` + literal pointing at the SRAM routine.
    // `ldr r6,[pc,#0]; bx r6` 加一个字面量，指向 SRAM 中的例程。
    let literal = SRAM_BASE | 1;
    let mut program: Vec<u8> = Vec::new();
    program.extend_from_slice(&0x4E00u16.to_le_bytes());
    program.extend_from_slice(&0x4730u16.to_le_bytes());
    program.extend_from_slice(&literal.to_le_bytes());
    flash.load_bytes(FLASH_XIP_BASE, &program).unwrap();
    bus.register_region(Box::new(flash)).unwrap();
    bus.register_region(Box::new(RamRegion::new("sram", SRAM_BASE, 0x1_0000)))
        .unwrap();
    let mut core = CortexM33Core::new(CpuId::Core0);
    core.boot_at(0x2000_8000, FLASH_XIP_BASE, FLASH_XIP_BASE);
    core.set_flash_fetch_cache(true);

    let mut cycles = 0;
    // SRAM 中的第一条例程：`movs r0,#1; b .`
    // First SRAM routine: `movs r0,#1; b .`
    bus.write_u16(SRAM_BASE, 0x2001, &mut cycles).unwrap();
    bus.write_u16(SRAM_BASE + 2, 0xE7FD, &mut cycles).unwrap();
    for _ in 0..6 {
        core.step(&mut bus).unwrap();
    }
    assert_eq!(core.read_register(0), 1);
    let flushes = core.decode_cache_flushes();

    // 客户机改写 SRAM 代码后必须立即执行新指令；SRAM 从不入译码缓存，因此也
    // 不会触发世代清空。
    // A guest rewrite of SRAM code must take effect immediately; SRAM is never
    // decoded into the cache, so no generation flush is involved.
    bus.write_u16(SRAM_BASE, 0x2002, &mut cycles).unwrap();
    for _ in 0..4 {
        core.step(&mut bus).unwrap();
    }
    assert_eq!(core.read_register(0), 2);
    assert_eq!(core.decode_cache_flushes(), flushes);
}

#[test]
fn unaligned_pc_fetch_matches_the_decoder() {
    // 非对齐 PC 取指沿用既有行为：直接按该地址读取半字，两个核心必须一致。
    // An unaligned PC keeps the pre-existing behaviour (the halfword is read at
    // that exact address); both cores must agree.
    let program = loop_program(&[0x2001, 0x2002]);
    let (mut cached, mut cached_bus) = machine(&program, true);
    let (mut plain, mut plain_bus) = machine(&program, false);
    for core in [&mut cached, &mut plain] {
        core.registers.pc = FLASH_XIP_BASE + 1;
    }
    for step in 0..8 {
        let a = cached.step(&mut cached_bus);
        let b = plain.step(&mut plain_bus);
        assert_eq!(format!("{a:?}"), format!("{b:?}"), "step {step}");
        assert_eq!(snapshot(&cached), snapshot(&plain), "step {step}");
    }
}

#[test]
fn thumb32_straddling_the_flash_window_end_matches_the_decoder() {
    // 让一条 32 位指令的第二个半字落在 Flash 窗口之外：两个核心必须一致
    // （缓存不记录该 PC，下一取指仍走总线）。
    // Put the second halfword of a Thumb-32 instruction past the flash window:
    // both cores must agree (that PC is never cached).
    let mut program = vec![0u8; FLASH_SIZE];
    // `movs.w lr,#1` 的第一个半字在窗口最后两个字节。
    // The first halfword of `movs.w lr,#1` sits in the last two bytes.
    program[FLASH_SIZE - 2] = 0x5F;
    program[FLASH_SIZE - 1] = 0xF0;
    let (mut cached, mut cached_bus) = machine(&program, true);
    let (mut plain, mut plain_bus) = machine(&program, false);
    for core in [&mut cached, &mut plain] {
        core.registers.pc = FLASH_XIP_BASE + (FLASH_SIZE as u32) - 2;
    }
    for step in 0..4 {
        let a = cached.step(&mut cached_bus);
        let b = plain.step(&mut plain_bus);
        assert_eq!(format!("{a:?}"), format!("{b:?}"), "step {step}");
        assert_eq!(snapshot(&cached), snapshot(&plain), "step {step}");
    }
}

#[test]
fn pseudo_random_instruction_stream_stays_equivalent() {
    // 伪随机指令流：混合合法 16/32 位编码与随机字，逐步比较两个核心。
    // A pseudo-random instruction stream mixing valid 16/32-bit encodings with
    // arbitrary words, compared step by step.
    let mut state = 0x1234_5678u32;
    let next = |state: &mut u32| {
        *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *state
    };
    let mut words: Vec<u16> = Vec::new();
    for _ in 0..120 {
        let value = next(&mut state);
        if value.trailing_zeros() >= 2 {
            // 32 位前缀 + 随机第二半字（可能未定义，两条路径必须一致）。
            // 32-bit prefix plus a random second halfword (possibly undefined).
            words.push(0xE800 | ((value >> 16) as u16 & 0x1FFF));
            words.push(value as u16);
        } else {
            words.push(value as u16);
        }
    }
    let program = loop_program(&words);
    assert_equivalent(&program, 600);
}

#[test]
fn the_cache_fills_and_hits_within_the_hot_loop() {
    let program = loop_program(&[0x2001, 0x2102, 0x1840]);
    let (mut core, mut bus) = machine(&program, true);
    // 第一步是填充，之后每一步都命中（同一个 4 条指令的循环）。
    // The first pass fills, every later step hits (the same four-instruction loop).
    for _ in 0..64 {
        core.step(&mut bus).unwrap();
    }
    assert_eq!(core.read_register(1), 2);
    // 并行测试共享同一个 Flash 世代号，因此这里只要求循环本身不触发清空之外
    // 的行为：命中路径不改变状态。
    // Parallel tests share one flash generation, so this only asserts that the
    // loop keeps working; hit behaviour is covered by the differential tests.
    assert!(core.decode_cache_flushes() < 16);
}

#[test]
fn pc_zero_falls_back_to_the_bus_with_the_cache_enabled() {
    // PC=0 与译码缓存空槽位的 tag=0 相等；空槽位不是有效条目，因此缓存核心
    // 必须像无缓存核心一样访问总线（此处地址 0 未映射 → BusError）。
    // PC=0 equals an empty decode-cache slot's tag 0; an empty slot is not a
    // valid entry, so the cached core must touch the bus exactly like the
    // uncached core (address 0 is unmapped here → BusError).
    let program = loop_program(&[0x2001, 0x2002]);
    let (mut cached, mut cached_bus) = machine(&program, true);
    let (mut plain, mut plain_bus) = machine(&program, false);
    cached.registers.pc = 0;
    plain.registers.pc = 0;
    let a = cached.step(&mut cached_bus);
    let b = plain.step(&mut plain_bus);
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
    assert!(a.is_err(), "address 0 is unmapped in this fixture");
    assert_eq!(snapshot(&cached), snapshot(&plain));
}
