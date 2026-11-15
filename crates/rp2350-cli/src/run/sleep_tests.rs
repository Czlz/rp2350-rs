//! Sleep scheduling tests / 睡眠调度测试
//!
//! These drive a synthetic UF2 whose core 0 configures one NVIC line, executes
//! `WFI`, and only counts wake-ups and handler entries. They are the
//! instruction-level evidence for the machine scheduler: a sleeping core retires
//! nothing while time advances, and an interrupt wakes it with no extra step.
//! 这些测试驱动一个合成 UF2：core0 使能一条 NVIC 线后执行 `WFI`，只统计唤醒次数与
//! 处理程序进入次数。它们是机器调度器的指令级证据：睡眠核不退役指令但时间推进，
//! 中断唤醒不额外消耗一步。

use super::*;

/// Flash XIP base used by the fixture / 夹具使用的 Flash XIP 基址
const FLASH: u32 = FLASH_XIP_BASE;

/// Initial stack pointer of the fixture / 夹具的初始栈指针
const FIXTURE_SP: u32 = SRAM_BASE + SRAM_SIZE as u32;

/// Address of the fixture's IRQ 20 handler / 夹具 IRQ20 处理程序地址
const HANDLER: u32 = FLASH + 0x240;

/// Address of the fixture's `WFI` instruction / 夹具 `WFI` 指令地址
///
/// Kept next to [`SLEEP_PC`] so the "instruction retired, PC advanced" property
/// is explicit in the tests.
/// 与 [`SLEEP_PC`] 并列，使「指令已退役、PC 已前进」的性质在测试中显式可见。
#[allow(dead_code)]
const WFI_INSN: u32 = FLASH + 0x206;

/// PC while the fixture sleeps (the instruction after `WFI`)
/// 夹具睡眠期间的 PC（`WFI` 之后的那条指令）
const SLEEP_PC: u32 = FLASH + 0x208;

/// Shared-SRAM word the handler increments / 处理程序自增的共享 SRAM 字
///
/// A register written inside the handler is restored by exception return, so the
/// handler count must live in memory to be observable afterwards.
/// 处理程序内写入的寄存器会在异常返回时被恢复，因此处理程序计数必须落在内存里才能被
/// 观察到。
const HANDLER_COUNTER: u32 = SRAM_BASE + 0x9000;

/// NVIC ISER0 absolute address / NVIC ISER0 绝对地址
const ISER0: u32 = 0xE000_E100;

/// NVIC ISPR0 absolute address / NVIC ISPR0 绝对地址
const ISPR0: u32 = 0xE000_E200;

/// IRQ line the fixture enables / 夹具使能的中断线
const TEST_IRQ: u32 = 20;

/// SIO `FIFO_WR` / SIO `FIFO_WR`
const SIO_FIFO_WR: u32 = 0xD000_0054;

/// SIO `FIFO_RD` / SIO `FIFO_RD`
const SIO_FIFO_RD: u32 = 0xD000_0058;

/// Build one 512-byte UF2 block / 构造一个 512 字节 UF2 块
fn uf2_block(family: u32, index: u32, total: u32, address: u32, payload: &[u8]) -> Vec<u8> {
    let mut block = vec![0u8; 512];
    block[0..4].copy_from_slice(&0x0A32_4655u32.to_le_bytes());
    block[4..8].copy_from_slice(&0x9E5D_5157u32.to_le_bytes());
    block[8..12].copy_from_slice(&0x0000_2000u32.to_le_bytes());
    block[12..16].copy_from_slice(&address.to_le_bytes());
    block[16..20].copy_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
    block[20..24].copy_from_slice(&index.to_le_bytes());
    block[24..28].copy_from_slice(&total.to_le_bytes());
    block[28..32].copy_from_slice(&family.to_le_bytes());
    block[32..32 + payload.len()].copy_from_slice(payload);
    block[508..512].copy_from_slice(&0x0AB1_6F30u32.to_le_bytes());
    block
}

/// Put one 32-bit word into a byte payload / 在字节载荷中写入一个 32 位字
fn put_word(payload: &mut [u8], offset: usize, value: u32) {
    payload[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// Put one Thumb halfword into a byte payload / 在字节载荷中写入一个 Thumb 半字
fn put_half(payload: &mut [u8], offset: usize, value: u16) {
    payload[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

/// Synthetic Arm UF2: enable IRQ 20, then `WFI` in a counting loop
/// 合成 Arm UF2：使能 IRQ20 后在计数循环里 `WFI`
///
/// Vector table at the flash base (`SP`, reset, IRQ 20), code at `+0x200`:
/// `ldr r4,[pc]` → `ISER0`; `ldr r5,[pc]` → `1 << 20`; `str r5,[r4]`;
/// `wfi`; `adds r2,#1`; `b wfi`. The handler at `+0x240` clears `ICPR0` and
/// increments `r3`, so `r2` counts wake-ups and `r3` counts handled interrupts.
/// 向量表位于 Flash 基址（`SP`、复位、IRQ20），代码位于 `+0x200`：使能 ISER、
/// `wfi`、`adds r2,#1`、跳回 `wfi`。`+0x240` 的处理程序清 `ICPR0` 并自增 `r3`，
/// 因此 `r2` 统计唤醒次数、`r3` 统计已处理中断。
fn wfi_uf2() -> Vec<u8> {
    let mut vectors = vec![0u8; 256];
    put_word(&mut vectors, 0x00, FIXTURE_SP);
    put_word(&mut vectors, 0x04, FLASH + 0x201);
    // IRQ 20 → exception 36 → vector at 36 * 4 = 0x90.
    put_word(&mut vectors, 0x90, HANDLER | 1);

    let mut code = vec![0u8; 0x60];
    // 0x200: ldr r4,[pc,#12] → 0x210 (ISER0)
    put_half(&mut code, 0x00, 0x4C03);
    // 0x202: ldr r5,[pc,#16] → 0x214 (1 << 20)
    put_half(&mut code, 0x02, 0x4D04);
    // 0x204: str r5,[r4]
    put_half(&mut code, 0x04, 0x6025);
    // 0x206: wfi
    put_half(&mut code, 0x06, 0xBF30);
    // 0x208: adds r2,#1
    put_half(&mut code, 0x08, 0x3201);
    // 0x20A: b 0x206
    put_half(&mut code, 0x0A, 0xE7FC);
    // 0x210: ISER0, 0x214: IRQ mask
    put_word(&mut code, 0x10, ISER0);
    put_word(&mut code, 0x14, 1 << TEST_IRQ);
    // 0x240: ldr r0,[pc,#12] → 0x250 (handler counter address)
    put_half(&mut code, 0x40, 0x4803);
    // 0x242: ldr r1,[r0]
    put_half(&mut code, 0x42, 0x6801);
    // 0x244: adds r1,#1
    put_half(&mut code, 0x44, 0x3101);
    // 0x246: str r1,[r0]
    put_half(&mut code, 0x46, 0x6001);
    // 0x248: ldr r0,[pc,#8] → 0x254 (ICPR0)
    put_half(&mut code, 0x48, 0x4802);
    // 0x24A: ldr r1,[pc,#12] → 0x258 (1 << 20)
    put_half(&mut code, 0x4A, 0x4903);
    // 0x24C: str r1,[r0] — clear the pending bit
    put_half(&mut code, 0x4C, 0x6001);
    // 0x24E: bx lr
    put_half(&mut code, 0x4E, 0x4770);
    // 0x250: handler counter (shared SRAM), 0x254: ICPR0, 0x258: IRQ mask
    put_word(&mut code, 0x50, HANDLER_COUNTER);
    put_word(&mut code, 0x54, 0xE000_E280);
    put_word(&mut code, 0x58, 1 << TEST_IRQ);

    let mut image = uf2_block(0xE48B_FF59, 0, 2, FLASH, &vectors);
    image.extend(uf2_block(0xE48B_FF59, 1, 2, FLASH + 0x200, &code));
    image
}

/// Build the synthetic machine / 构建合成机器
fn wfi_machine(max_cycles: u64) -> Machine {
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles,
        strict_decode: false,
    };
    Machine::new(&wfi_uf2(), &options).expect("machine")
}

/// Step until core 0 is asleep, returning the number of steps taken
/// 推进到 core0 入睡，返回所用步数
fn run_until_sleep(machine: &mut Machine) -> u64 {
    for _ in 0..16 {
        machine.step().expect("step");
        if machine.cpu_state() == CpuState::Sleeping {
            return machine.steps();
        }
    }
    panic!("core 0 never slept, PC={:#010X}", machine.pc());
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

#[test]
fn wfi_firmware_sleeps_and_wakes_on_an_irq() {
    let mut machine = wfi_machine(10_000);
    run_until_sleep(&mut machine);
    assert_eq!(
        machine.pc(),
        SLEEP_PC,
        "WFI retires (PC advances) before the sleep begins"
    );
    assert_eq!(
        machine.system_register(ISER0),
        Some(1 << TEST_IRQ),
        "the firmware enabled IRQ {TEST_IRQ} / 固件已使能该中断线"
    );

    // 睡眠期间 PC 冻结、步数推进：时间照常流逝。
    // The PC freezes while the step counter advances: time keeps flowing.
    let sleep_steps = machine.sleep_counters().0[0];
    let pc = machine.pc();
    machine.run_cycles(200);
    assert_eq!(machine.pc(), pc, "a sleeping core retires nothing");
    assert!(
        machine.sleep_counters().0[0] > sleep_steps,
        "sleep steps are counted / 睡眠步数被统计"
    );
    assert!(machine.steps() > sleep_steps, "the clock advanced");

    // 挂起中断后：一步让 NVIC 看到挂起位，下一步唤醒并进入处理程序。
    // After the pending bit is raised: one step makes it visible to the NVIC and
    // the following step wakes the core and enters the handler.
    write_word(&mut machine, ISPR0, 1 << TEST_IRQ);
    machine.step().expect("replay step");
    machine.step().expect("wake step");
    assert_eq!(
        machine.pc(),
        HANDLER,
        "the wake enters the handler, not the next thread instruction"
    );
    assert_eq!(machine.cpu_state(), CpuState::Running);

    // 处理程序清挂起位、把计数写到共享 SRAM 并返回；r2 统计唤醒。
    // The handler clears pending, bumps the shared-SRAM counter and returns;
    // r2 counts wake-ups.
    for _ in 0..16 {
        machine.step().expect("handler step");
    }
    assert_eq!(
        read_word(&mut machine, HANDLER_COUNTER),
        1,
        "the handler ran exactly once / 处理程序只执行一次"
    );
    assert_eq!(machine.read_register(2), 1, "one wake-up observed");
    assert_eq!(
        machine.pc(),
        SLEEP_PC,
        "execution continued after WFI and slept again"
    );
    assert_eq!(machine.cpu_state(), CpuState::Sleeping);
    assert!(
        machine.halt_reason().is_none(),
        "the fixture never halted / 夹具从未停机"
    );
}

#[test]
fn wake_up_has_zero_step_latency() {
    let mut machine = wfi_machine(10_000);
    run_until_sleep(&mut machine);
    write_word(&mut machine, ISPR0, 1 << TEST_IRQ);
    // 一步把挂起位回放到 NVIC（核仍在睡），下一步必须直接唤醒，不能再多睡一步。
    // One step replays the pending bit into the NVIC (the core still sleeps); the
    // following step must wake immediately instead of sleeping once more.
    machine.step().expect("replay");
    assert_eq!(machine.cpu_state(), CpuState::Sleeping);
    let slept = machine.sleep_counters().0[0];
    machine.step().expect("wake");
    assert_eq!(
        machine.sleep_counters().0[0],
        slept,
        "the wake step must not be a sleep step / 唤醒步不得再计为睡眠步"
    );
    assert_ne!(machine.cpu_state(), CpuState::Sleeping);
    assert_eq!(machine.pc(), HANDLER);
}

#[test]
fn sleep_counters_and_diagnostic_line_are_stable() {
    let mut machine = wfi_machine(10_000);
    assert_eq!(
        machine.sleep_state(),
        "sleep core0 steps=0 wakes=0 core1 steps=0 wakes=0",
        "the diagnostic format is stable / 诊断格式稳定"
    );
    run_until_sleep(&mut machine);
    machine.run_cycles(50);
    let (steps, wakes) = machine.sleep_counters();
    assert!(steps[0] >= 50, "core 0 sleep steps are counted");
    let line = machine.sleep_state();
    assert!(line.starts_with("sleep core0 steps="));
    assert!(line.contains("wakes="));
    assert!(line.contains("core1 steps=0"), "core 1 never slept yet");
    // 唤醒后 wakes 必须增加，睡眠步数不再增长。
    // After a wake the wake counter grows and the sleep counter stops.
    write_word(&mut machine, ISPR0, 1 << TEST_IRQ);
    machine.step().expect("replay");
    machine.step().expect("wake");
    let (_, wakes_after) = machine.sleep_counters();
    assert!(wakes_after[0] > wakes[0], "the wake is counted");
}

#[test]
fn a_sleeping_core0_leaves_core1_running() {
    // core0 睡在 WFI 上，core1 忙等自增：两核状态互不影响。
    // Core 0 sleeps in WFI while core 1 increments a counter: the two states are
    // independent.
    let mut machine = wfi_machine(1_000_000);
    let counter = SRAM_BASE + 0x8100;
    let code = [0x4902u16, 0x6808, 0x3001, 0x6008, 0xE7FB];
    for (index, halfword) in code.iter().enumerate() {
        let mut cycles = 0u64;
        machine
            .bus
            .write_u16(
                SRAM_BASE + 0x8000 + index as u32 * 2,
                *halfword,
                &mut cycles,
            )
            .expect("core1 code");
    }
    write_word(&mut machine, SRAM_BASE + 0x800C, counter);
    for word in [
        0u32,
        0,
        1,
        SRAM_BASE + 0x1000,
        SRAM_BASE + 0xA000,
        SRAM_BASE + 0x8001,
    ] {
        write_word(&mut machine, SIO_FIFO_WR, word);
        machine.peripherals.service_core1_peer();
        assert_eq!(read_word(&mut machine, SIO_FIFO_RD), word);
    }

    run_until_sleep(&mut machine);
    let core0_pc = machine.pc();
    let before = read_word(&mut machine, counter);
    machine.run_cycles(203);
    assert_eq!(machine.core1_state(), Some(CpuState::Running));
    assert_eq!(machine.pc(), core0_pc, "core 0 stays asleep");
    assert!(
        read_word(&mut machine, counter) > before,
        "core 1 made progress while core 0 slept / core1 在 core0 睡眠时继续运行"
    );
    let (steps, _) = machine.sleep_counters();
    assert_eq!(steps[1], 0, "core 1 never slept");
}

#[test]
fn both_sleeping_cores_advance_time_only() {
    let mut machine = wfi_machine(2_000_000);
    // core1 的入口就是一条 `wfi`：使能中断后它也睡下。
    let mut cycles = 0u64;
    machine
        .bus
        .write_u16(SRAM_BASE + 0x8000, 0xBF30, &mut cycles)
        .expect("core1 wfi");
    for word in [
        0u32,
        0,
        1,
        SRAM_BASE + 0x1000,
        SRAM_BASE + 0xA000,
        SRAM_BASE + 0x8001,
    ] {
        write_word(&mut machine, SIO_FIFO_WR, word);
        machine.peripherals.service_core1_peer();
        assert_eq!(read_word(&mut machine, SIO_FIFO_RD), word);
    }
    run_until_sleep(&mut machine);
    machine.run_cycles(64);
    assert_eq!(machine.core1_state(), Some(CpuState::Sleeping));

    let core0_pc = machine.pc();
    let core1_pc = machine.core1_pc();
    let before = machine.steps();
    machine.run_cycles(1_000);
    assert_eq!(machine.steps(), before + 1_000, "the clock keeps running");
    assert_eq!(machine.pc(), core0_pc, "core 0 retired nothing");
    assert_eq!(machine.core1_pc(), core1_pc, "core 1 retired nothing");
    let (steps, _) = machine.sleep_counters();
    assert!(steps[0] >= 1_000 && steps[1] >= 1_000);
}

#[test]
fn sleep_steps_do_not_grow_the_retired_step_history() {
    // 睡眠步只是时钟，不是退役指令：回溯环形缓冲不得因睡眠而增长。
    // Sleep steps are clock ticks, not retired instructions: the recent-step ring
    // must not grow while a core sleeps.
    let mut machine = wfi_machine(10_000);
    run_until_sleep(&mut machine);
    let last = machine.recent_steps().last().map(|(step, _, _)| *step);
    machine.run_cycles(500);
    assert_eq!(
        machine.recent_steps().last().map(|(step, _, _)| *step),
        last,
        "no instruction retired while sleeping / 睡眠期间没有指令退役"
    );
}

#[test]
fn a_blocked_fifo_load_wakes_on_a_peer_frame() {
    // 固件从 `SIO_FIFO_RD` 读取一帧：空队列时载入必须阻塞而不是读回丢弃值。
    // The firmware loads a frame from `SIO_FIFO_RD`: with an empty queue the load
    // must block instead of returning a discarded value.
    let mut vectors = vec![0u8; 256];
    put_word(&mut vectors, 0x00, FIXTURE_SP);
    put_word(&mut vectors, 0x04, FLASH + 0x201);
    let mut code = vec![0u8; 0x14];
    // 0x200: ldr r1,[pc,#8] → 0x20C (FIFO_RD)
    put_half(&mut code, 0x00, 0x4902);
    // 0x202: ldr r0,[r1]
    put_half(&mut code, 0x02, 0x6808);
    // 0x204: adds r2,#1
    put_half(&mut code, 0x04, 0x3201);
    // 0x206: b .
    put_half(&mut code, 0x06, 0xE7FE);
    put_word(&mut code, 0x0C, SIO_FIFO_RD);
    let mut image = uf2_block(0xE48B_FF59, 0, 2, FLASH, &vectors);
    image.extend(uf2_block(0xE48B_FF59, 1, 2, FLASH + 0x200, &code));
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles: 1_000,
        strict_decode: false,
    };
    let mut machine = Machine::new(&image, &options).expect("machine");

    machine.step().expect("ldr r1");
    machine.step().expect("blocked load");
    assert_eq!(machine.cpu_state(), CpuState::Sleeping);
    assert_eq!(machine.pc(), FLASH + 0x202, "the load must be retried");
    assert_eq!(machine.read_register(0), 0, "r0 untouched");

    // 对端写入一帧后，重放同一条载入并只消费一次。
    // A peer frame makes the retried load complete, consuming exactly one frame.
    machine.peripherals().sio().post_to_core(0x1234_5678);
    machine.step().expect("retry");
    assert_eq!(machine.read_register(0), 0x1234_5678);
    assert_eq!(machine.pc(), FLASH + 0x204, "the retry retired once");
    assert_eq!(machine.cpu_state(), CpuState::Running);
    machine.peripherals().sio().post_to_core(0x9ABC_DEF0);
    machine.step().expect("adds");
    assert_eq!(machine.read_register(2), 1);
    assert!(
        machine
            .peripherals()
            .sio()
            .fifo_rx_ready(rp2350_core::types::CpuId::Core0),
        "the second frame is still queued / 第二帧仍留在队列中"
    );
}
