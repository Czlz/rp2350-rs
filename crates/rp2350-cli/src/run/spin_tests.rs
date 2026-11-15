//! Tight-spin detector tests / 紧凑自旋检测测试
//!
//! The detector must distinguish a firmware that polls a value which never
//! changes from a wait loop that sleeps, wakes on a timer or interrupt and
//! returns to the same window. These tests drive synthetic Arm images for both
//! cases and assert that only the pure spin produces
//! `[run] tight spin` lines.
//! 检测器必须区分「轮询一个永不变化的值」与「睡眠、被定时器/中断唤醒后回到同一
//! 窗口」的等待循环。这些测试为两种情况各驱动一个合成 Arm 映像，并断言只有纯自旋
//! 会产生 `[run] tight spin` 行。

// SysTick 是 ARM 参考手册中的驼峰寄存器名，测试文档保持硬件写法以便检索。
// SysTick is the CamelCase name used by the ARM reference manuals; the test
// documentation keeps the hardware spelling so it stays searchable.
#![allow(clippy::doc_markdown)]

use super::*;
use crate::test_support::{build_elf, infinite_loop_elf, systick_elf};

/// Flash XIP base used by the fixtures / 夹具使用的 Flash XIP 基址
const FLASH: u32 = FLASH_XIP_BASE;

/// Initial stack pointer of the fixtures / 夹具的初始栈指针
const FIXTURE_SP: u32 = SRAM_BASE + SRAM_SIZE as u32;

/// NVIC `ISER0` absolute address / NVIC `ISER0` 绝对地址
const ISER0: u32 = 0xE000_E100;

/// NVIC `ISPR0` absolute address / NVIC `ISPR0` 绝对地址
const ISPR0: u32 = 0xE000_E200;

/// NVIC `ICPR0` absolute address / NVIC `ICPR0` 绝对地址
const ICPR0: u32 = 0xE000_E280;

/// `SYST_CSR` absolute address / `SYST_CSR` 绝对地址
const SYST_CSR: u32 = 0xE000_E010;

/// IRQ line the wait fixture enables / 等待夹具使能的中断线
const TEST_IRQ: u32 = 20;

/// Shared-SRAM word the wait fixture's handler increments
/// 等待夹具处理程序自增的共享 SRAM 字
const HANDLER_COUNTER: u32 = SRAM_BASE + 0x9300;

/// Put one 32-bit word into a byte payload / 在字节载荷中写入一个 32 位字
fn put_word(payload: &mut [u8], offset: usize, value: u32) {
    payload[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// Put one Thumb halfword into a byte payload / 在字节载荷中写入一个 Thumb 半字
fn put_half(payload: &mut [u8], offset: usize, value: u16) {
    payload[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

/// Write one word through the machine bus / 通过机器总线写入一个字
fn write_word(machine: &mut Machine, addr: u32, value: u32) {
    let mut cycles = 0u64;
    machine
        .bus
        .write_u32(addr, value, &mut cycles)
        .expect("bus write");
}

/// Read one word through the machine bus / 通过机器总线读取一个字
fn read_word(machine: &mut Machine, addr: u32) -> u32 {
    let mut cycles = 0u64;
    machine.bus.read_u32(addr, &mut cycles).expect("bus read")
}

/// Drain the queued tight-spin notices / 取走排队的自旋告警
fn spin_notices(machine: &mut Machine) -> Vec<String> {
    machine
        .take_notices()
        .into_iter()
        .filter(|notice| notice.contains("tight spin"))
        .collect()
}

/// Synthetic Arm image: enable IRQ 20, `wfe`, then spin in the same window
/// 合成 Arm 映像：使能 IRQ20 后 `wfe`，随后在同一窗口内自旋
///
/// The vector table places the IRQ 20 handler at `FLASH + 0x210`, and the
/// program at `+0x200` enables the line, executes `wfe` at `+0x206` and branches
/// to itself at `+0x208`. The handler body (`+0x210..+0x21C`) and the spin both
/// fit inside the detector's 32-byte window starting at `+0x200`, so the PC
/// never leaves the window while the firmware sleeps, wakes and handles the
/// interrupt. That is exactly the shape of a real `sleep_ms` wait loop.
/// 向量表把 IRQ20 处理程序放在 `FLASH + 0x210`；`+0x200` 的程序使能该中断线，
/// `+0x206` 执行 `wfe`，`+0x208` 原地自旋。处理程序主体（`+0x210..+0x21C`）与
/// 自旋都落在检测器从 `+0x200` 开始的 32 字节窗口内，因此固件睡眠、唤醒并处理中断
/// 期间 PC 从不离开该窗口——这正是真实 `sleep_ms` 等待循环的形状。
fn wfe_wait_elf() -> Vec<u8> {
    let mut data = vec![0u8; 0x230];
    put_word(&mut data, 0x00, FIXTURE_SP);
    put_word(&mut data, 0x04, FLASH + 0x201);
    // IRQ 20 → exception 36 → vector at 36 * 4 = 0x90.
    put_word(&mut data, 0x90, (FLASH + 0x210) | 1);
    // 0x200: ldr r4,[pc,#36] → 0x228 (ISER0)
    put_half(&mut data, 0x200, 0x4C09);
    // 0x202: ldr r5,[pc,#40] → 0x22C (1 << 20)
    put_half(&mut data, 0x202, 0x4D0A);
    // 0x204: str r5,[r4] — enable the IRQ line
    put_half(&mut data, 0x204, 0x6025);
    // 0x206: wfe — sleep until the pending interrupt wakes the core
    put_half(&mut data, 0x206, 0xBF20);
    // 0x208: b . — the post-wake spin, still inside the same window
    put_half(&mut data, 0x208, 0xE7FE);
    // 0x210: ldr r0,[pc,#12] → 0x220 (ICPR0)
    put_half(&mut data, 0x210, 0x4803);
    // 0x212: str r5,[r0] — clear the pending bit
    put_half(&mut data, 0x212, 0x6005);
    // 0x214: ldr r0,[pc,#12] → 0x224 (HANDLER_COUNTER)
    put_half(&mut data, 0x214, 0x4803);
    // 0x216: ldr r1,[r0]
    put_half(&mut data, 0x216, 0x6801);
    // 0x218: adds r1,#1
    put_half(&mut data, 0x218, 0x3101);
    // 0x21A: str r1,[r0]
    put_half(&mut data, 0x21A, 0x6001);
    // 0x21C: bx lr
    put_half(&mut data, 0x21C, 0x4770);
    put_word(&mut data, 0x220, ICPR0);
    put_word(&mut data, 0x224, HANDLER_COUNTER);
    put_word(&mut data, 0x228, ISER0);
    put_word(&mut data, 0x22C, 1 << TEST_IRQ);

    let memsz = data.len() as u32;
    build_elf(FLASH + 0x201, &[(FLASH, &data, memsz)])
}

/// Build the WFE wait fixture / 构建 WFE 等待夹具
fn wait_machine(max_cycles: u64) -> Machine {
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles,
        strict_decode: false,
        psram: None,
    };
    Machine::new(&wfe_wait_elf(), &options).expect("machine")
}

/// Build a plain `b .` machine / 构建普通 `b .` 机器
fn pure_spin_machine(max_cycles: u64) -> Machine {
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles,
        strict_decode: false,
        psram: None,
    };
    Machine::new(&infinite_loop_elf(), &options).expect("machine")
}

/// Build a `cpsid i` spin whose SysTick expiries stay pending
/// 构建 `cpsid i` 自旋：SysTick 到期只保持挂起
///
/// The delay loop at `+0x214` never enables interrupts, so every forwarded
/// SysTick expiry is logged as an `[irq]` delivery while the PC stays in one
/// two-instruction window. That isolates the interrupt-delivery criterion from
/// both sleep and wake counters.
/// `+0x214` 的延时循环从不解除中断屏蔽，因此每次转发的 SysTick 到期都会记录为一次
/// `[irq]` 投递，而 PC 始终停留在同一个两指令窗口内。这样就把「中断投递」判据与
/// 睡眠、唤醒计数器完全隔离开。
fn masked_spin_machine(delay: u32, max_cycles: u64) -> Machine {
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles,
        strict_decode: false,
        psram: None,
    };
    Machine::new(&systick_elf(16, 0x7, Some(delay)), &options).expect("machine")
}

#[test]
fn a_wfe_wait_loop_with_a_timer_wake_does_not_warn() {
    // WFE 睡眠 → 中断唤醒 → 处理程序 → 回到同一窗口。窗口内睡眠步数与唤醒
    // 次数都增长，因此 100 万步后不得告警。
    // WFE sleep -> interrupt wake -> handler -> back to the same window.
    // Sleep steps and wakes grow inside the window, so no warning may appear.
    let mut machine = wait_machine(4_000_000);
    for _ in 0..16 {
        machine.step().expect("step");
        if machine.cpu_state() == CpuState::Sleeping {
            break;
        }
    }
    assert_eq!(
        machine.cpu_state(),
        CpuState::Sleeping,
        "the fixture must sleep on WFI"
    );
    let (sleep_before, wakes_before) = machine.sleep_counters();
    write_word(&mut machine, ISPR0, 1 << TEST_IRQ);
    // 批量路径在睡眠时不回放 PPB 写入，因此先用逐指令路径把挂起位交给 NVIC 并
    // 完成唤醒；之后的长时间自旋仍走 `run_steps`（即 `run` 的批量路径）。
    // The batched path does not replay PPB writes while sleeping, so the
    // per-instruction path hands the pending bit to the NVIC and completes the
    // wake first; the long spin afterwards still uses `run_steps` (the `run`
    // batched path).
    machine.step().expect("replay the pending bit");
    machine.step().expect("wake into the handler");
    machine.run_steps(1_400_000).expect("wait then spin");

    let (sleep_after, wakes_after) = machine.sleep_counters();
    assert!(
        sleep_after[0] > sleep_before[0],
        "the wait window must contain sleep steps"
    );
    assert!(
        wakes_after[0] > wakes_before[0],
        "the wait window must contain a wake"
    );
    assert_eq!(
        read_word(&mut machine, HANDLER_COUNTER),
        1,
        "the handler ran exactly once / 处理程序只运行一次"
    );
    assert!(
        spin_notices(&mut machine).is_empty(),
        "a WFE wait loop must not produce a tight-spin warning"
    );
}

#[test]
fn a_pure_spin_still_warns_with_the_stable_diagnostic() {
    // 无睡眠、无中断的 `b .` 仍必须在 100 万步后告警，且文本格式不变。
    // A `b .` with no sleep and no interrupts must still warn after one million
    // steps, with the unchanged diagnostic format.
    let mut machine = pure_spin_machine(1_200_000);
    machine.run_steps(1_100_000).expect("spin");

    let notices = machine.take_notices();
    assert_eq!(notices.len(), 1, "{notices:?}");
    let line = &notices[0];
    assert!(
        line.starts_with(
            "[run] tight spin: PC stayed in 0x10000000..0x10000020 for 1000000 steps, \
             likely polling a value that never changes / 疑似轮询等待; r0="
        ),
        "{line}"
    );
    assert!(line.contains(" sp=0x"), "{line}");
}

#[test]
fn pure_spin_warnings_stop_at_the_report_limit() {
    // 每个新窗口在第一个阈值处报一次；第 5 个页面不再报告，证明上限有效。
    // Each new window reports once at its first threshold; the fifth window stays
    // silent, proving the cap is effective.
    let mut machine = pure_spin_machine(7_000_000);
    for page in 0..=SPIN_REPORT_LIMIT {
        if page > 0 {
            let addr = SRAM_BASE + 0x1000 + (page as u32) * 0x100;
            let mut cycles = 0u64;
            machine
                .bus
                .write_u16(addr, 0xE7FE, &mut cycles)
                .expect("spin code");
            machine.core.set_pc(addr);
        }
        machine.run_steps(1_050_000).expect("spin window");
    }

    let notices = spin_notices(&mut machine);
    assert_eq!(
        notices.len(),
        SPIN_REPORT_LIMIT,
        "one warning per window, capped at the limit: {notices:?}"
    );
}

#[test]
fn masked_irq_deliveries_suppress_the_spin_warning() {
    // PC 两指令窗口内长时间停留，但每 10 000 步都有 SysTick 投递：属于等待，
    // 不是卡死轮询。睡眠与唤醒计数保持为零，证明是中断判据在起作用。
    // The PC stays in a two-instruction window while SysTick deliveries keep
    // arriving every 10_000 steps: a wait, not a stuck poll. Sleep and wake
    // counters stay zero, proving the interrupt criterion is what suppresses.
    let mut machine = masked_spin_machine(4_000_000, 3_000_000);
    machine.run_steps(2_300_000).expect("irq wait");

    assert!(
        machine.irq_delivery_count() > 0,
        "the window must contain interrupt deliveries"
    );
    let (sleep, wakes) = machine.sleep_counters();
    assert_eq!(sleep, [0, 0], "this fixture never sleeps");
    assert_eq!(wakes, [0, 0], "this fixture never wakes");
    assert!(
        spin_notices(&mut machine).is_empty(),
        "interrupt deliveries must suppress the tight-spin warning"
    );
}

#[test]
fn masked_irq_deliveries_suppress_the_spin_warning_on_single_steps() {
    // 逐指令路径（`after_running_step`）必须与批量路径使用同一判据。
    // The per-instruction path (`after_running_step`) must use the same rule as
    // the batched path.
    let mut machine = masked_spin_machine(2_000_000, 1_400_000);
    for _ in 0..1_300_000 {
        machine.step().expect("step");
    }

    assert!(machine.irq_delivery_count() > 0);
    assert!(
        spin_notices(&mut machine).is_empty(),
        "the per-instruction path must suppress the same wait loop"
    );
}

#[test]
fn spin_activity_is_rearmed_after_the_window_changes() {
    // 第一个窗口由 SysTick 投递抑制；关闭 SysTick 后，活动快照必须重新武装，
    // 使之后的纯自旋恰好告警一次，而不是被旧窗口的投递永久掩盖。
    // The first window is suppressed by SysTick deliveries; after SysTick is
    // disabled the activity snapshot must be re-armed so the later pure spin
    // warns exactly once instead of being masked forever.
    let mut machine = masked_spin_machine(4_000_000, 4_000_000);
    machine.run_steps(1_100_000).expect("suppressed window");
    assert_eq!(
        spin_notices(&mut machine),
        Vec::<String>::new(),
        "the wait window must not warn"
    );

    write_word(&mut machine, SYST_CSR, 0);
    machine.run_steps(2_200_000).expect("pure window");
    let notices = spin_notices(&mut machine);
    assert_eq!(
        notices.len(),
        1,
        "a re-armed pure window warns exactly once: {notices:?}"
    );
    assert!(
        notices[0].starts_with("[run] tight spin: PC stayed in 0x"),
        "{}",
        notices[0]
    );
}

#[test]
fn sleep_and_wake_activity_is_not_recounted_in_a_later_window() {
    // 等待窗口被睡眠/唤醒抑制后，同一窗口继续纯自旋：下一百万步必须告警一次，
    // 证明抑制时的快照刷新没有把旧的睡眠计数带到新窗口。
    // After the wait window is suppressed by sleep/wake activity, the same window
    // continues as a pure spin: the next million steps must warn once, proving the
    // refreshed snapshot does not carry the old sleep counters forward.
    let mut machine = wait_machine(4_000_000);
    for _ in 0..16 {
        machine.step().expect("step");
        if machine.cpu_state() == CpuState::Sleeping {
            break;
        }
    }
    write_word(&mut machine, ISPR0, 1 << TEST_IRQ);
    machine.step().expect("replay the pending bit");
    machine.step().expect("wake into the handler");
    machine
        .run_steps(1_400_000)
        .expect("suppressed wait window");
    assert_eq!(
        spin_notices(&mut machine),
        Vec::<String>::new(),
        "the wait window must not warn"
    );

    machine.run_steps(1_200_000).expect("pure spin window");
    let notices = spin_notices(&mut machine);
    assert_eq!(
        notices.len(),
        1,
        "sleep activity must not suppress the later window: {notices:?}"
    );
}
