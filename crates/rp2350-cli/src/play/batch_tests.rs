//! Batched frame stepping tests / 批量帧步进测试
//!
//! These tests pin the two invariants of the batched `play` path: one frame is
//! exactly `steps_per_frame` machine steps, and a single `run_steps` batch is
//! guest-state identical to the old 4096-step slicing (and to plain
//! `Machine::step`). They also cover the in-batch host poll: buttons and the
//! stop flag are still sampled every 4096 steps even though the frame is one
//! threaded batch.
//! 这些测试锁定批量 `play` 路径的两个不变量：一帧恰好是 `steps_per_frame` 个机器步；
//! 单个 `run_steps` 批次与旧的 4096 步切片（以及逐条 `Machine::step`）在客户机状态上
//! 完全一致。测试还覆盖批内宿主轮询：整帧虽然是一个线程化批次，按键与停止标志仍每
//! 4096 步采样一次。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use rp2350_core::FLASH_XIP_BASE;
use rp2350_cpu::CpuState;

use super::*;
use crate::run::CoreThreadOptions;
use crate::test_support::{build_elf, counter_elf, systick_elf};

/// SRAM base address used by the fixtures / 夹具使用的 SRAM 基地址
const SRAM_BASE: u32 = 0x2000_0000;

/// Machine options for a synthetic Arm image / 合成 Arm 映像的机器选项
fn arm_options(max_cycles: u64) -> MachineOptions {
    MachineOptions {
        arch: Some(crate::run::ArchArg::Arm),
        max_cycles,
        strict_decode: false,
        psram: None,
    }
}

/// Counter machine that never halts / 永不停机的计数机器
fn counter_machine() -> Machine {
    Machine::new(&counter_elf(), &arm_options(u64::MAX)).expect("counter machine")
}

/// `SysTick` machine that keeps delivering interrupts / 持续投递中断的 `SysTick` 机器
fn irq_machine() -> Machine {
    Machine::new(&systick_elf(1_000, 0x7, None), &arm_options(u64::MAX)).expect("SysTick machine")
}

/// Frame spec inside SRAM used by the headless fixtures / 无头夹具使用的 SRAM 帧规格
fn frame_spec() -> FrameSpec {
    FrameSpec::new(SRAM_BASE + 0x1000, 4, 4)
}

/// Old slice-by-slice frame driver kept as the A/B reference
/// 旧的逐切片帧驱动，保留作为 A/B 参照
fn sliced_step_frame(
    machine: &mut Machine,
    config: &PlayConfig,
    stop: &Arc<AtomicBool>,
    buttons: &Arc<AtomicU16>,
    trace_left: &mut usize,
) -> Option<String> {
    let mut stepped = 0u64;
    while stepped < config.steps_per_frame {
        if stepped % STOP_POLL_STEPS == 0 {
            apply_buttons(machine, buttons.load(Ordering::Relaxed), config.button_pins);
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            if machine.budget_exhausted() {
                return Some(BUDGET_REASON.to_owned());
            }
            if let Some(reason) = machine.halt_reason() {
                return Some(reason.to_owned());
            }
            if machine.take_system_reset_request() {
                return Some(SYSTEM_RESET_MESSAGE.to_owned());
            }
        }
        let slice = (config.steps_per_frame - stepped).min(STOP_POLL_STEPS);
        if *trace_left > 0 {
            let mut done = 0u64;
            while done < slice && *trace_left > 0 {
                *trace_left -= 1;
                let index = config.trace_steps - *trace_left - 1;
                let outcome = machine.trace_step(index);
                for line in machine.take_notices() {
                    println!("{line}");
                }
                if let Err(error) = outcome {
                    return Some(error.to_string());
                }
                done += 1;
            }
            stepped += done;
            let rest = slice - done;
            if rest > 0 {
                if let Err(error) = machine.run_steps(rest) {
                    return Some(error.to_string());
                }
                stepped += rest;
            }
        } else {
            if let Err(error) = machine.run_steps(slice) {
                return Some(error.to_string());
            }
            stepped += slice;
        }
        if machine.take_system_reset_request() {
            return Some(SYSTEM_RESET_MESSAGE.to_owned());
        }
    }
    None
}

/// Dual-core counter machine with core 1 already running
/// core1 已启动的双核计数机器
fn dual_core_counter_machine(threads: CoreThreadOptions) -> Machine {
    // movs r0,#0 / adds r0,#1 / b <adds> — the same loop on both cores.
    // movs r0,#0 / adds r0,#1 / b <adds> —— 两个核使用同一个循环。
    let program = [0x00u8, 0x20, 0x01, 0x30, 0xFD, 0xE7];
    let elf = build_elf(
        FLASH_XIP_BASE | 1,
        &[
            (FLASH_XIP_BASE, &program, program.len() as u32),
            (SRAM_BASE, &program, program.len() as u32),
        ],
    );
    let mut machine = Machine::new_with_core_threads(&elf, &arm_options(u64::MAX), threads)
        .expect("dual-core machine");
    machine.release_core1_for_test(0x2008_2000, SRAM_BASE | 1, FLASH_XIP_BASE);
    assert_eq!(machine.core1_state(), Some(CpuState::Running));
    machine
}

/// Wait until the session published at least `frames` frames
/// 等到会话至少发布 `frames` 帧
fn wait_for_frames(session: &PlaySession, frames: u64) -> PlayStats {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let stats = session.stats();
        if stats.frames >= frames || stats.halted.is_some() {
            return stats;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {stats:?}");
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn batched_steps_match_per_step_execution() {
    // `run_steps` 与逐条 `step` 必须在步数、时基、PC、寄存器、[irq]、[trap] 与
    // [tail] 环形缓冲上完全一致。SysTick 夹具保证 [irq] 非空。
    // `run_steps` and per-instruction `step` must agree on steps, timebase, PC,
    // registers, [irq], [trap] and the [tail] ring. The SysTick fixture makes
    // sure the IRQ log is not trivially empty.
    const STEPS: u64 = 30_000;
    let mut batched = irq_machine();
    let mut per_step = irq_machine();
    assert_eq!(
        batched.run_steps(STEPS).expect("batched"),
        CpuState::Running
    );
    for _ in 0..STEPS {
        assert_eq!(per_step.step().expect("step"), CpuState::Running);
    }
    assert_eq!(batched.steps(), STEPS);
    assert_eq!(per_step.steps(), STEPS);
    assert_eq!(batched.time_us(), per_step.time_us());
    assert_eq!(batched.pc(), per_step.pc());
    assert_eq!(batched.registers(), per_step.registers());
    assert!(!batched.irqs().is_empty(), "fixture must deliver IRQs");
    assert_eq!(batched.irqs(), per_step.irqs());
    assert_eq!(batched.irq_delivery_count(), per_step.irq_delivery_count());
    assert_eq!(batched.traps(), per_step.traps());
    assert_eq!(batched.recent_steps(), per_step.recent_steps());
}

#[test]
fn step_frame_matches_the_sliced_reference_and_keeps_the_budget() {
    // 一帧的步数必须精确等于 steps_per_frame，且旧的 4096 步切片路径与新的一次
    // `run_steps` 批次在帧边界上给出相同的机器状态与统计。
    // A frame must be exactly steps_per_frame steps, and the old 4096-step slicing
    // and the new single batch must leave the same machine state at frame
    // boundaries.
    let config = PlayConfig {
        button_pins: 0x3FF,
        ..PlayConfig::new(frame_spec(), 10_000)
    };
    let stop = Arc::new(AtomicBool::new(false));
    let buttons = Arc::new(AtomicU16::new(0));
    let mut batched = irq_machine();
    let mut reference = irq_machine();
    let mut batched_trace = 0;
    let mut reference_trace = 0;
    for frame in 1..=3u64 {
        assert_eq!(
            step_frame(&mut batched, &config, &stop, &buttons, &mut batched_trace),
            sliced_step_frame(
                &mut reference,
                &config,
                &stop,
                &buttons,
                &mut reference_trace
            ),
            "frame {frame}"
        );
        let expected = frame * config.steps_per_frame;
        assert_eq!(batched.steps(), expected, "frame {frame} steps");
        assert_eq!(reference.steps(), expected, "reference frame {frame} steps");
    }
    assert_eq!(batched.pc(), reference.pc());
    assert_eq!(batched.registers(), reference.registers());
    assert_eq!(batched.time_us(), reference.time_us());
    assert_eq!(batched.irqs(), reference.irqs());
    assert_eq!(batched.irq_delivery_count(), reference.irq_delivery_count());
    assert_eq!(batched.traps(), reference.traps());
    assert_eq!(batched.halt_reason(), None);
}

#[test]
fn stop_request_mid_batch_ends_at_a_poll_boundary() {
    // 停止请求在批次中途生效：下一次 4096 步轮询就返回，且宿主停止不锁存 halt。
    // A stop request takes effect inside the batch: the next 4096-step poll
    // returns, and a host stop never latches a halt reason.
    let mut machine = counter_machine();
    let stop = Arc::new(AtomicBool::new(false));
    let buttons = Arc::new(AtomicU16::new(0));
    let config = PlayConfig::new(frame_spec(), 2_000_000);
    let host_stop = Arc::clone(&stop);
    let stopper = thread::spawn(move || {
        thread::sleep(Duration::from_millis(5));
        host_stop.store(true, Ordering::Relaxed);
    });
    let mut trace_left = 0;
    let reason = step_frame(&mut machine, &config, &stop, &buttons, &mut trace_left);
    stopper.join().expect("stopper");
    assert_eq!(reason, None, "a host stop is not a halt");
    assert!(machine.steps() > 0, "the frame ran before the stop");
    assert!(
        machine.steps() < config.steps_per_frame,
        "the frame must not finish: steps={}",
        machine.steps()
    );
    assert_eq!(
        machine.steps() % STOP_POLL_STEPS,
        0,
        "the stop lands on a poll boundary: steps={}",
        machine.steps()
    );
    assert_eq!(machine.halt_reason(), None);
    assert!(!machine.budget_exhausted());
}

#[test]
fn threaded_stop_request_mid_batch_ends_at_a_poll_boundary() {
    // 线程化执行器的窗口绝不跨过轮询边界，因此批内停止同样停在 4096 的倍数上，
    // 且批次结束后没有工作线程残留。
    // Threaded windows never cross a poll boundary, so an in-batch stop also
    // lands on a multiple of 4096, and no worker thread survives the batch.
    let mut machine = dual_core_counter_machine(CoreThreadOptions {
        enabled: true,
        window: 64,
    });
    let stop = Arc::new(AtomicBool::new(false));
    let buttons = Arc::new(AtomicU16::new(0));
    machine.set_host_input(0, Arc::clone(&buttons), Arc::clone(&stop), STOP_POLL_STEPS);
    let host_stop = Arc::clone(&stop);
    let stopper = thread::spawn(move || {
        thread::sleep(Duration::from_millis(5));
        host_stop.store(true, Ordering::Relaxed);
    });
    let state = machine.run_steps(2_000_000).expect("threaded batch");
    stopper.join().expect("stopper");
    assert_eq!(state, CpuState::Halted);
    assert!(machine.steps() > 0 && machine.steps() < 2_000_000);
    assert_eq!(
        machine.steps() % STOP_POLL_STEPS,
        0,
        "the stop lands on a poll boundary: steps={}",
        machine.steps()
    );
    assert_eq!(machine.active_core_threads(), 0);
    assert_eq!(machine.halt_reason(), None);
}

#[test]
fn trace_steps_still_emit_trace_lines_on_the_first_frame() {
    // 第一帧的 trace 前缀仍逐条执行并产生 `[trace]` 行；`step_frame` 在追踪完
    // `trace_steps` 步后把剩余步数交给一个批量 `run_steps`。
    // The first frame's trace prefix still runs one by one and produces `[trace]`
    // lines; `step_frame` hands the remaining steps to one batched `run_steps`.
    let mut traced = counter_machine();
    let lines = trace_one(&mut traced, 0).expect("trace step");
    assert!(
        lines.iter().any(|line| line.starts_with("[trace]")),
        "trace lines: {lines:?}"
    );

    let config = PlayConfig {
        trace_steps: 4,
        ..PlayConfig::new(frame_spec(), 1_000)
    };
    let stop = Arc::new(AtomicBool::new(false));
    let buttons = Arc::new(AtomicU16::new(0));
    let mut machine = counter_machine();
    let mut trace_left = config.trace_steps;
    assert_eq!(
        step_frame(&mut machine, &config, &stop, &buttons, &mut trace_left),
        None
    );
    assert_eq!(trace_left, 0, "the trace prefix is fully consumed");
    assert_eq!(machine.steps(), 1_000, "the rest is one exact batch");
}

#[test]
fn host_input_is_sampled_at_the_configured_interval() {
    // 批内轮询按绝对步号采样：第一次在 4096 步后，下一次在 8192 步后；
    // 掩码变化只有在轮询点才写到 pad 上。
    // In-batch polling samples absolute step numbers: the first poll is after
    // 4096 steps and the next after 8192; a mask change only reaches the pads at
    // a poll point.
    let mut machine = counter_machine();
    let stop = Arc::new(AtomicBool::new(false));
    let buttons = Arc::new(AtomicU16::new(1 << 2));
    machine.set_host_input(
        1 << 2,
        Arc::clone(&buttons),
        Arc::clone(&stop),
        STOP_POLL_STEPS,
    );
    assert_eq!(machine.next_host_poll(), Some(STOP_POLL_STEPS));
    machine.run_steps(STOP_POLL_STEPS).expect("first interval");
    assert_eq!(machine.steps(), STOP_POLL_STEPS);
    assert_eq!(machine.next_host_poll(), Some(2 * STOP_POLL_STEPS));
    let levels = machine.peripherals().pads().input_levels();
    assert_eq!(levels & (1 << 2), 0, "pressed is low / 按下为低");

    buttons.store(0, Ordering::Relaxed);
    machine.run_steps(STOP_POLL_STEPS).expect("second interval");
    assert_eq!(machine.next_host_poll(), Some(3 * STOP_POLL_STEPS));
    let levels = machine.peripherals().pads().input_levels();
    assert_eq!(levels & (1 << 2), 1 << 2, "released is high / 松开为高");
}

#[test]
fn session_reports_the_effective_core_executor_settings() {
    // 会话回读机器实际接受的设置：默认 off/64；请求 on 时开启；窗口在机器侧被
    // 钳制到 1..=4096。
    // The session reports what the machine actually accepted: off/64 by default,
    // on when requested, and the window clamped to 1..=4096 by the machine.
    let config = PlayConfig {
        max_frames: Some(1),
        ..PlayConfig::new(frame_spec(), 200)
    };
    let off = PlaySession::start(counter_elf(), arm_options(u64::MAX), config).expect("off");
    assert!(!off.core_threads_enabled());
    assert_eq!(off.core_window(), 64);
    let _ = off.shutdown();

    let on = PlaySession::start_with_core_threads(
        counter_elf(),
        arm_options(u64::MAX),
        CoreThreadOptions {
            enabled: true,
            window: 32,
        },
        config,
    )
    .expect("on");
    assert!(on.core_threads_enabled());
    assert_eq!(on.core_window(), 32);
    let _ = on.shutdown();

    let clamped = PlaySession::start_with_core_threads(
        counter_elf(),
        arm_options(u64::MAX),
        CoreThreadOptions {
            enabled: true,
            window: 99_999,
        },
        config,
    )
    .expect("clamped");
    assert!(clamped.core_threads_enabled());
    assert_eq!(clamped.core_window(), 4096);
    let _ = clamped.shutdown();
}

#[test]
fn the_thread_switch_keeps_the_frame_step_budget() {
    // 线程开关只改变宿主执行方式，不改变每帧步数：off 与 on 两个会话跑同样帧数
    // 后 `steps` 完全相同。
    // The thread switch only changes how the host executes; it never changes the
    // per-frame budget: two sessions publish the same number of frames with the
    // same `steps` whether threads are off or on.
    let config = PlayConfig {
        max_frames: Some(2),
        ..PlayConfig::new(frame_spec(), 5_000)
    };
    let off = PlaySession::start(counter_elf(), arm_options(u64::MAX), config).expect("off");
    let off_stats = wait_for_frames(&off, 2);
    let off_final = off.shutdown();

    let on = PlaySession::start_with_core_threads(
        counter_elf(),
        arm_options(u64::MAX),
        CoreThreadOptions {
            enabled: true,
            window: 64,
        },
        config,
    )
    .expect("on");
    let on_stats = wait_for_frames(&on, 2);
    let on_final = on.shutdown();

    assert_eq!(off_stats.frames, 2);
    assert_eq!(on_stats.frames, 2);
    assert_eq!(off_final.steps, 2 * 5_000);
    assert_eq!(on_final.steps, 2 * 5_000);
    assert_eq!(off_final.halted, None);
    assert_eq!(on_final.halted, None);
}
