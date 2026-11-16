//! Tests for the optional two-thread core executor
//! 可选双核线程执行器的测试
//!
//! The synthetic programs are deliberately small: they prove step accounting,
//! halt/reset propagation, cross-core FIFO handshakes, spinlock mutual
//! exclusion and thread lifecycle without depending on a real firmware. FV-2 /
//! FV-3 are the real-firmware gate and are run from the CLI.
//! 合成程序刻意保持极小：它们证明步数记账、停机/复位传播、跨核 FIFO 握手、自旋锁
//! 互斥与线程生命周期，不依赖真实固件。FV-2 / FV-3 是真实固件门禁，由 CLI 运行。

use rp2350_core::AddressBus;
use rp2350_core::FLASH_XIP_BASE;

use super::*;
use crate::test_support::{counter_elf, unmapped_branch_elf};

/// Build an Arm machine with a fixed budget / 构建固定预算的 Arm 机器
fn machine(image: &[u8], max_cycles: u64) -> Machine {
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles,
        strict_decode: false,
        psram: None,
    };
    Machine::new(image, &options).expect("machine")
}

/// Build an Arm machine with the threaded executor / 构建线程化 Arm 机器
fn threaded(image: &[u8], max_cycles: u64, window: u32) -> Machine {
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles,
        strict_decode: false,
        psram: None,
    };
    Machine::new_with_core_threads(
        image,
        &options,
        CoreThreadOptions {
            enabled: true,
            window,
        },
    )
    .expect("threaded machine")
}

#[test]
fn off_is_the_default_and_byte_identical_between_runs() {
    let elf = counter_elf();
    let mut first = machine(&elf, 4_000);
    let mut second = machine(&elf, 4_000);
    assert!(!first.core_threads_enabled());
    first.run_steps(4_000).expect("first");
    second.run_steps(4_000).expect("second");
    assert_eq!(first.steps(), 4_000);
    assert_eq!(first.steps(), second.steps());
    assert_eq!(first.register(0), second.register(0));
    assert_eq!(first.registers(), second.registers());
}

#[test]
fn a_single_core_counter_matches_between_off_and_on() {
    let elf = counter_elf();
    let mut off = machine(&elf, 2_000);
    let mut on = threaded(&elf, 2_000, 64);
    assert!(on.core_threads_enabled());
    off.run_steps(2_000).expect("off");
    on.run_steps(2_000).expect("on");
    assert_eq!(off.steps(), on.steps());
    assert_eq!(off.register(0), on.register(0));
    assert_eq!(off.pc(), on.pc());
    assert_eq!(off.cpu_state(), on.cpu_state());
}

#[test]
fn window_sizes_32_64_128_agree_on_a_single_core_program() {
    let elf = counter_elf();
    let mut reference = machine(&elf, 1_000);
    reference.run_steps(1_000).expect("reference");
    for window in [32u32, 64, 128] {
        let mut machine = threaded(&elf, 1_000, window);
        machine.run_steps(1_000).expect("threaded");
        assert_eq!(machine.steps(), reference.steps(), "window={window}");
        assert_eq!(
            machine.register(0),
            reference.register(0),
            "window={window}"
        );
        assert_eq!(machine.pc(), reference.pc(), "window={window}");
    }
}

#[test]
fn worker_threads_are_joined_after_every_run() {
    let elf = counter_elf();
    for _ in 0..10 {
        let mut machine = threaded(&elf, 512, 64);
        machine.run_steps(512).expect("run");
        assert_eq!(machine.active_core_threads(), 0);
    }
}

#[test]
fn reboot_preserves_the_threaded_mode_and_restarts_the_clock() {
    let elf = counter_elf();
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles: 500,
        strict_decode: false,
        psram: None,
    };
    let mut machine = threaded(&elf, 500, 32);
    machine.run_steps(500).expect("run");
    assert_eq!(machine.steps(), 500);
    machine.reboot(&elf, &options).expect("reboot");
    assert!(machine.core_threads_enabled());
    assert_eq!(machine.core_window(), 32);
    assert_eq!(machine.steps(), 0);
    assert_eq!(machine.pc(), FLASH_XIP_BASE);
    assert_eq!(machine.cpu_state(), CpuState::Running);
    machine.run_steps(500).expect("run after reboot");
    assert_eq!(machine.steps(), 500);
}

#[test]
fn a_guest_bus_fault_halts_the_threaded_machine() {
    let elf = unmapped_branch_elf();
    let mut machine = threaded(&elf, 64, 64);
    machine.run_steps(64).expect("run");
    let reason = machine.halt_reason().expect("halt reason");
    assert!(
        reason.contains("30000000") || reason.contains("Bus"),
        "unexpected halt: {reason}"
    );
}

#[test]
fn sram_snapshots_match_between_off_and_on_for_a_single_core_program() {
    let elf = counter_elf();
    let mut off = machine(&elf, 2_000);
    let mut on = threaded(&elf, 2_000, 64);
    off.run_steps(2_000).expect("off");
    on.run_steps(2_000).expect("on");
    let off_sram = off.read_block(TEST_SRAM, 0x1000);
    let on_sram = on.read_block(TEST_SRAM, 0x1000);
    assert_eq!(off_sram, on_sram, "SRAM diverged between off and on");
}

#[test]
fn window_changes_are_clamped_to_the_supported_range() {
    let elf = counter_elf();
    let mut machine = threaded(&elf, 32, 64);
    machine.set_core_window(0);
    assert_eq!(machine.core_window(), 1);
    machine.set_core_window(99_999);
    assert_eq!(machine.core_window(), 4096);
}

/// A `WFI` loop with no interrupt source / 没有中断源的空 `WFI` 循环
const WFI_LOOP: [u16; 3] = [0xBF30, 0x3201, 0xE7FC];

/// Build an ELF whose entry is a `WFI` loop at the flash base
/// 构建入口位于 Flash 基址、内容为 `WFI` 循环的 ELF
fn wfi_loop_elf() -> Vec<u8> {
    let mut data = vec![0u8; 0x20];
    let mut halfwords = WFI_LOOP;
    for (index, halfword) in halfwords.iter_mut().enumerate() {
        data[index * 2..index * 2 + 2].copy_from_slice(&halfword.to_le_bytes());
    }
    crate::test_support::build_elf(
        FLASH_XIP_BASE | 1,
        &[(FLASH_XIP_BASE, &data, data.len() as u32)],
    )
}

#[test]
fn a_wfi_loop_is_fast_forwarded_on_the_threaded_path() {
    let elf = wfi_loop_elf();
    let mut machine = threaded(&elf, 100_000, 64);
    machine.set_sleep_fast_forward(true);
    machine.run_steps(100_000).expect("run");
    assert_eq!(machine.steps(), 100_000);
    assert!(
        machine.sleep_fast_forward_steps() > 90_000,
        "fast-forward skipped {} steps",
        machine.sleep_fast_forward_steps()
    );
    let (steps, _wakes) = machine.sleep_counters();
    assert!(steps[0] > 90_000);
    assert_eq!(machine.active_core_threads(), 0);
}

// ---- synthetic dual-core programs ----------------------------------------

/// SRAM base used by the synthetic programs / 合成程序使用的 SRAM 基址
const TEST_SRAM: u32 = 0x2000_0000;
/// Initial stack pointer shared by both synthetic cores / 两个合成核共享的初始栈指针
const TEST_SP: u32 = 0x2008_2000;
/// Word the FIFO handshake stores its result in / FIFO 握手写入结果的字
const FIFO_RESULT: u32 = TEST_SRAM + 0x800;
/// Word the spinlock counter lives in / 自旋锁计数器所在的字
const SPIN_COUNTER: u32 = TEST_SRAM + 0x900;
/// SIO `FIFO_WR` / SIO `FIFO_WR`
const SIO_FIFO_WR: u32 = 0xD000_0054;
/// SIO `FIFO_RD` / SIO `FIFO_RD`
const SIO_FIFO_RD: u32 = 0xD000_0058;
/// SIO `SPINLOCK0` / SIO `SPINLOCK0`
const SIO_SPINLOCK0: u32 = 0xD000_0100;
/// `SCB_AIRCR` / `SCB_AIRCR`
const SCB_AIRCR: u32 = 0xE000_ED0C;
/// `AIRCR` value with `VECTKEY` and `SYSRESETREQ` / 带 `VECTKEY` 与 `SYSRESETREQ` 的值
const AIRCR_SYSRESETREQ: u32 = 0x05FA_0004;

/// Tiny Thumb-16 assembler with a patched literal pool
/// 带字面量池回填的极简 Thumb-16 汇编器
struct Thumb {
    /// Emitted halfwords / 已输出半字
    code: Vec<u16>,
    /// `(instruction index, literal value)` / `(指令下标, 字面量值)`
    literals: Vec<(usize, u32)>,
}

impl Thumb {
    /// Create an empty program / 创建空程序
    fn new() -> Self {
        Self {
            code: Vec::new(),
            literals: Vec::new(),
        }
    }

    /// Emit one raw halfword / 输出一个原始半字
    fn raw(&mut self, halfword: u16) {
        self.code.push(halfword);
    }

    /// Emit `ldr Rd, =value` (patched at [`Self::finish`])
    /// 输出 `ldr Rd, =value`（在 [`Self::finish`] 回填）
    fn ldr(&mut self, rd: u8, value: u32) {
        self.code.push(0x4800 | (u16::from(rd) << 8));
        self.literals.push((self.code.len() - 1, value));
    }

    /// Resolve the literal pool and return the code bytes
    /// 解析字面量池并返回代码字节
    fn finish(mut self) -> Vec<u8> {
        let code_bytes = self.code.len() * 2;
        let pool_base = (code_bytes + 3) & !3;
        for (slot, (index, _value)) in self.literals.iter().enumerate() {
            let target = pool_base + slot * 4;
            let pc = (index * 2 + 4) & !3;
            let immediate = (target - pc) / 4;
            debug_assert!(immediate <= 255);
            self.code[*index] |= immediate as u16;
        }
        let mut bytes = Vec::with_capacity(pool_base + self.literals.len() * 4);
        for halfword in &self.code {
            bytes.extend_from_slice(&halfword.to_le_bytes());
        }
        while bytes.len() < pool_base {
            bytes.push(0);
        }
        for (_, value) in &self.literals {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}

/// Build a two-core ELF: core0 in flash, core1 in SRAM
/// 构建双核 ELF：core0 在 Flash，core1 在 SRAM
fn dual_core_elf(core0: &[u8], core1: &[u8]) -> Vec<u8> {
    let memsz0 = core0.len() as u32;
    let memsz1 = core1.len() as u32;
    crate::test_support::build_elf(
        FLASH_XIP_BASE | 1,
        &[(FLASH_XIP_BASE, core0, memsz0), (TEST_SRAM, core1, memsz1)],
    )
}

/// Release core 1 at `TEST_SRAM` for a synthetic dual-core run
/// 为合成双核运行在 `TEST_SRAM` 释放 core1
fn release_synthetic_core1(machine: &mut Machine, entry: u32, vtor: u32) {
    machine.release_core1_for_test(TEST_SP, entry | 1, vtor);
    assert_eq!(machine.core1_state(), Some(CpuState::Running));
}

/// SRAM flag core0 sets after draining the launch echoes / core0 清空启动回显后置位的 SRAM 标志
const FIFO_FLAG: u32 = TEST_SRAM + 0x700;

/// Build the FIFO handshake pair: core0 drains six launch echoes, sends 42 and
/// records the reply; core1 waits for the flag, then replies 43.
/// 构建 FIFO 握手程序对：core0 清空 6 个启动回显、发送 42 并记录回复；core1 等待
/// 标志后回复 43。
///
/// Core 1 is released through the real SIO FIFO launch stream, so the resident
/// echo stub retires exactly as it does for firmware. The six echoes are the
/// `{0, 0, 1, vtor, sp, entry}` stream replayed by that stub.
/// core1 通过真实的 SIO FIFO 启动流释放，因此常驻回显桩像真实固件一样退役。六个
/// 回显就是该桩重放的 `{0, 0, 1, vtor, sp, entry}` 流。
fn fifo_handshake_elf() -> Vec<u8> {
    let mut core0 = Thumb::new();
    core0.ldr(4, SIO_FIFO_RD);
    for _ in 0..6 {
        core0.raw(0x6820); // ldr r0,[r4] — drain one launch echo
    }
    core0.ldr(6, FIFO_FLAG);
    core0.raw(0x2001); // movs r0,#1
    core0.raw(0x6030); // str r0,[r6]
    core0.ldr(4, SIO_FIFO_WR);
    core0.raw(0x202A); // movs r0,#42
    core0.raw(0x6020); // str r0,[r4]
    core0.ldr(4, SIO_FIFO_RD);
    core0.raw(0x6820); // ldr r0,[r4] (waits for 43)
    core0.ldr(6, FIFO_RESULT);
    core0.raw(0x6030); // str r0,[r6]
    core0.raw(0xE7FE); // b .

    let mut core1 = Thumb::new();
    core1.ldr(6, FIFO_FLAG);
    core1.raw(0x6830); // wait: ldr r0,[r6]
    core1.raw(0x2800); // cmp r0,#0
    core1.raw(0xD0FC); // beq wait
    core1.ldr(4, SIO_FIFO_RD);
    core1.raw(0x6820); // ldr r0,[r4] (42)
    core1.raw(0x3001); // adds r0,#1
    core1.ldr(4, SIO_FIFO_WR);
    core1.raw(0x6020); // str r0,[r4] (43)
    core1.raw(0xE7FE); // b .
    dual_core_elf(&core0.finish(), &core1.finish())
}

/// Release core 1 through the real SIO FIFO launch stream
/// 通过真实的 SIO FIFO 启动流释放 core1
fn launch_synthetic_core1(machine: &mut Machine, entry: u32, vtor: u32) {
    let mut cycles = 0u64;
    for word in [0u32, 0, 1, vtor, TEST_SP, entry | 1] {
        machine
            .bus
            .write_u32(SIO_FIFO_WR, word, &mut cycles)
            .expect("launch word");
    }
    for _ in 0..200 {
        machine.run_steps(64).expect("launch step");
        if machine.core1_launch_count() == 1 {
            break;
        }
    }
    assert_eq!(machine.core1_launch_count(), 1, "core1 did not launch");
    assert_eq!(machine.core1_state(), Some(CpuState::Running));
}

/// Build the spinlock counter pair: both cores increment `SPIN_COUNTER`
/// 构建自旋锁计数器程序对：两核各自自增 `SPIN_COUNTER`
fn spinlock_counter_elf(iterations: u8) -> Vec<u8> {
    let program = || {
        let mut program = Thumb::new();
        program.ldr(4, SIO_SPINLOCK0);
        program.ldr(5, SPIN_COUNTER);
        program.raw(0x2600 | u16::from(iterations)); // movs r6,#iterations
        program.raw(0x6820); // ldr r0,[r4]          ; acquire
        program.raw(0x2800); // cmp r0,#0
        program.raw(0xD0FC); // beq acquire
        program.raw(0x6828); // ldr r0,[r5]
        program.raw(0x3001); // adds r0,#1
        program.raw(0x6028); // str r0,[r5]
        program.raw(0x2000); // movs r0,#0
        program.raw(0x6020); // str r0,[r4]          ; release
        program.raw(0x3E01); // subs r6,#1
        program.raw(0xD1F5); // bne acquire
        program.raw(0xE7FE); // b .
        program.finish()
    };
    dual_core_elf(&program(), &program())
}

/// Read one word through the machine bus / 通过机器总线读取一个字
fn read_word(machine: &mut Machine, addr: u32) -> u32 {
    let mut cycles = 0u64;
    machine.bus.read_u32(addr, &mut cycles).expect("bus read")
}

/// Build a program that writes `AIRCR.SYSRESETREQ` and spins
/// 构建写入 `AIRCR.SYSRESETREQ` 后自旋的程序
fn aircr_reset_elf() -> Vec<u8> {
    let mut program = Thumb::new();
    program.ldr(0, SCB_AIRCR);
    program.ldr(1, AIRCR_SYSRESETREQ);
    program.raw(0x6001); // str r1,[r0]
    program.raw(0xE7FE); // b .
    let code = program.finish();
    let memsz = code.len() as u32;
    crate::test_support::build_elf(FLASH_XIP_BASE | 1, &[(FLASH_XIP_BASE, &code, memsz)])
}

#[test]
fn threaded_cores_complete_a_sio_fifo_handshake() {
    let elf = fifo_handshake_elf();
    let mut machine = threaded(&elf, 100_000, 64);
    launch_synthetic_core1(&mut machine, TEST_SRAM, FLASH_XIP_BASE);
    machine.run_steps(50_000).expect("run");
    assert_eq!(machine.halt_reason(), None);
    assert_eq!(
        read_word(&mut machine, FIFO_RESULT),
        43,
        "core0 did not observe the reply"
    );
    assert_eq!(machine.active_core_threads(), 0);
}

#[test]
fn threaded_cores_serialise_a_shared_counter_with_sio_spinlocks() {
    let iterations = 200u8;
    let elf = spinlock_counter_elf(iterations);
    let mut machine = threaded(&elf, 20_000, 64);
    release_synthetic_core1(&mut machine, TEST_SRAM, FLASH_XIP_BASE);
    machine.run_steps(20_000).expect("run");
    assert_eq!(machine.halt_reason(), None);
    assert_eq!(
        read_word(&mut machine, SPIN_COUNTER),
        u32::from(iterations) * 2,
        "spinlock lost an update"
    );
}

#[test]
fn the_off_path_serialises_the_same_spinlock_counter() {
    let iterations = 200u8;
    let elf = spinlock_counter_elf(iterations);
    let mut machine = machine(&elf, 20_000);
    release_synthetic_core1(&mut machine, TEST_SRAM, FLASH_XIP_BASE);
    machine.run_steps(20_000).expect("run");
    assert_eq!(
        read_word(&mut machine, SPIN_COUNTER),
        u32::from(iterations) * 2
    );
}

#[test]
fn threaded_mode_latches_a_guest_system_reset_request() {
    let elf = aircr_reset_elf();
    let mut machine = threaded(&elf, 64, 64);
    machine.run_steps(64).expect("run");
    assert!(machine.take_system_reset_request());
    assert_eq!(machine.halt_reason(), None);
}

/// Two cores storing into disjoint SRAM words forever (no MMIO, no events)
/// 两个核不停地写入互不相交的 SRAM 字（无 MMIO、无事件）
fn disjoint_counter_elf(core0_word: u32, core1_word: u32) -> Vec<u8> {
    let program = |word: u32| {
        let mut program = Thumb::new();
        program.ldr(1, word);
        program.raw(0x2000); // movs r0,#0
        program.raw(0x3001); // adds r0,#1
        program.raw(0x6008); // str r0,[r1]
        program.raw(0xE7FC); // b adds
        program.finish()
    };
    dual_core_elf(&program(core0_word), &program(core1_word))
}

/// Throughput probe: serial vs threaded on a pure dual-core store loop
/// 吞吐探针：纯双核存储循环上的串行与线程化对比
///
/// Ignored by default: it is a performance measurement, not a gate.
/// 默认忽略：它是性能测量，不是门禁。
#[test]
#[ignore = "performance probe, not a gate / 性能探针，非门禁"]
fn dual_core_counter_throughput() {
    use std::time::Instant;

    let elf = disjoint_counter_elf(TEST_SRAM + 0x100, TEST_SRAM + 0x200);
    let steps = 40_000_000u64;
    let mut serial = machine(&elf, steps);
    release_synthetic_core1(&mut serial, TEST_SRAM, FLASH_XIP_BASE);
    let start = Instant::now();
    serial.run_steps(steps).expect("serial");
    let serial_time = start.elapsed();
    println!("serial={serial_time:?}");

    for window in [32u32, 64, 128, 256, 1024, 4096] {
        let mut parallel = threaded(&elf, steps, window);
        release_synthetic_core1(&mut parallel, TEST_SRAM, FLASH_XIP_BASE);
        let start = Instant::now();
        parallel.run_steps(steps).expect("threaded");
        let parallel_time = start.elapsed();
        println!(
            "window={window} threaded={parallel_time:?} speedup={:.2}x",
            serial_time.as_secs_f64() / parallel_time.as_secs_f64()
        );
        assert!(read_word(&mut parallel, TEST_SRAM + 0x100) > 13_000_000);
        assert!(read_word(&mut parallel, TEST_SRAM + 0x200) > 13_000_000);
    }
    assert!(read_word(&mut serial, TEST_SRAM + 0x100) > 13_000_000);
}
