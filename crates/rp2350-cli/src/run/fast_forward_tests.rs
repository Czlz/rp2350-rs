//! Sleep fast-forward tests / 睡眠快进测试
//!
//! Every test builds two machines from the same synthetic Arm UF2 and runs one
//! with `sleep_fast_forward` on and the other with it off, then compares the
//! guest-visible state: steps, simulated microseconds, PC, registers, SRAM,
//! `SysTick`, TIMER, sleep counters, DMA/HSTX counters, the `[irq]` log, the
//! post-mortem ring and the queued host notices. The fast-forward path is only
//! accepted when it is indistinguishable from the step-by-step path.
//! 每个测试用同一份合成 Arm UF2 构造两台机器，一台开启睡眠快进、一台关闭，然后
//! 比较客户机可见状态：步数、模拟微秒、PC、寄存器、SRAM、SysTick、TIMER、睡眠
//! 计数、DMA/HSTX 计数、`[irq]` 日志、停机回溯环形缓冲与排队的宿主通知。只有与
//! 逐步路径不可区分时，快进路径才算通过。

use super::*;
use rp2350_periph::dma::{
    CTRL_EN, CTRL_INCR_READ, CTRL_INCR_WRITE, CTRL_TREQ_SEL_SHIFT, TREQ_PERMANENT,
};
use rp2350_periph::map::{DMA_BASE, PIO0_BASE, TIMER0_BASE};

/// Flash XIP base used by the fixtures / 夹具使用的 Flash XIP 基址
const FLASH: u32 = FLASH_XIP_BASE;

/// Initial stack pointer of the fixtures / 夹具初始栈指针
const FIXTURE_SP: u32 = SRAM_BASE + SRAM_SIZE as u32;

/// Address of the fixture's IRQ 0 handler / 夹具 IRQ0 处理程序地址
const TIMER_HANDLER: u32 = FLASH + 0x240;

/// PC while the fixture sleeps (the instruction after `WFI`)
/// 夹具睡眠期间的 PC（`WFI` 之后的那条指令）
const TIMER_SLEEP_PC: u32 = FLASH + 0x210;

/// NVIC `ISER0` / `ICPR0` absolute addresses / NVIC `ISER0` / `ICPR0` 绝对地址
const ISER0: u32 = 0xE000_E100;
const ICPR0: u32 = 0xE000_E280;

/// TIMER0 register offsets used by the fixtures / 夹具使用的 TIMER0 寄存器偏移
const TIMERAWL: u32 = 0x28;
const TIMER_INTF: u32 = 0x44;

/// `CLK_SYS_DIV` absolute address (`0x4001_0000 + 5 * 0x0C + 4`)
/// `CLK_SYS_DIV` 绝对地址
const CLK_SYS_DIV: u32 = 0x4001_0040;

/// SIO FIFO registers / SIO FIFO 寄存器
const SIO_FIFO_WR: u32 = 0xD000_0054;
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

/// Arm TIMER0 alarm 0, enable its NVIC line and then `WFI` forever
/// 武装 TIMER0 alarm 0、使能其 NVIC 线，然后一直 `WFI`
///
/// `alarm_us` is an absolute microsecond deadline, so the fixture is
/// deterministic regardless of how many cycles the prologue takes.
/// `alarm_us` 是绝对微秒截止时间，因此无论前导代码执行多少周期，夹具都是确定的。
fn timer_sleep_uf2(alarm_us: u32) -> Vec<u8> {
    let mut vectors = vec![0u8; 256];
    put_word(&mut vectors, 0x00, FIXTURE_SP);
    put_word(&mut vectors, 0x04, FLASH + 0x201);
    // IRQ 0 → exception 16 → vector at 16 * 4 = 0x40.
    put_word(&mut vectors, 0x40, TIMER_HANDLER | 1);

    let mut code = vec![0u8; 0x60];
    // 0x200: ldr r0,[pc,#16] → 0x214 (TIMER0 base)
    put_half(&mut code, 0x00, 0x4804);
    // 0x202: ldr r1,[pc,#20] → 0x218 (1)
    put_half(&mut code, 0x02, 0x4905);
    // 0x204: str r1,[r0,#0x40] — INTE bit 0
    put_half(&mut code, 0x04, 0x6401);
    // 0x206: ldr r2,[pc,#20] → 0x21C (ISER0)
    put_half(&mut code, 0x06, 0x4A05);
    // 0x208: str r1,[r2] — enable IRQ 0
    put_half(&mut code, 0x08, 0x6011);
    // 0x20A: ldr r3,[pc,#20] → 0x220 (alarm microseconds)
    put_half(&mut code, 0x0A, 0x4B05);
    // 0x20C: str r3,[r0,#0x10] — ALARM0
    put_half(&mut code, 0x0C, 0x6103);
    // 0x20E: wfi
    put_half(&mut code, 0x0E, 0xBF30);
    // 0x210: adds r5,#1 — wake counter
    put_half(&mut code, 0x10, 0x3501);
    // 0x212: b .
    put_half(&mut code, 0x12, 0xE7FE);
    put_word(&mut code, 0x14, TIMER0_BASE);
    put_word(&mut code, 0x18, 1);
    put_word(&mut code, 0x1C, ISER0);
    put_word(&mut code, 0x20, alarm_us);
    // 0x240: ldr r0,[pc,#16] → 0x254 (TIMER0 base)
    put_half(&mut code, 0x40, 0x4804);
    // 0x242: movs r1,#1
    put_half(&mut code, 0x42, 0x2101);
    // 0x244: str r1,[r0,#0x3C] — clear INTR bit 0
    put_half(&mut code, 0x44, 0x63C1);
    // 0x246: movs r3,#0
    put_half(&mut code, 0x46, 0x2300);
    // 0x248: str r3,[r0,#0x44] — clear INTF bit 0
    put_half(&mut code, 0x48, 0x6443);
    // 0x24A: adds r6,#1 — handler counter
    put_half(&mut code, 0x4A, 0x3601);
    // 0x24C: ldr r2,[pc,#8] → 0x258 (ICPR0)
    put_half(&mut code, 0x4C, 0x4A02);
    // 0x24E: str r1,[r2] — clear the NVIC pending bit
    put_half(&mut code, 0x4E, 0x6011);
    // 0x250: bx lr
    put_half(&mut code, 0x50, 0x4770);
    put_word(&mut code, 0x54, TIMER0_BASE);
    put_word(&mut code, 0x58, ICPR0);

    let mut image = uf2_block(0xE48B_FF59, 0, 2, FLASH, &vectors);
    image.extend(uf2_block(0xE48B_FF59, 1, 2, FLASH + 0x200, &code));
    image
}

/// A plain `WFI` loop with no interrupt source / 没有中断源的空 `WFI` 循环
fn plain_wfi_uf2() -> Vec<u8> {
    let mut vectors = vec![0u8; 256];
    put_word(&mut vectors, 0x00, FIXTURE_SP);
    put_word(&mut vectors, 0x04, FLASH + 0x201);
    // SysTick exception 15 → vector at 15 * 4 = 0x3C; returns immediately.
    put_word(&mut vectors, 0x3C, (FLASH + 0x240) | 1);
    let mut code = vec![0u8; 0x48];
    // 0x200: wfi
    put_half(&mut code, 0x00, 0xBF30);
    // 0x202: adds r2,#1
    put_half(&mut code, 0x02, 0x3201);
    // 0x204: b 0x200
    put_half(&mut code, 0x04, 0xE7FC);
    // 0x240: bx lr
    put_half(&mut code, 0x40, 0x4770);
    let mut image = uf2_block(0xE48B_FF59, 0, 2, FLASH, &vectors);
    image.extend(uf2_block(0xE48B_FF59, 1, 2, FLASH + 0x200, &code));
    image
}

/// A load from the empty core-0 SIO inbox, then a spin
/// 从空的 core0 SIO 收件箱载入，然后自旋
fn blocked_fifo_uf2() -> Vec<u8> {
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
    image
}

/// Build one Arm machine from `image` / 用 `image` 构造一台 Arm 机器
fn machine_from(image: &[u8], max_cycles: u64) -> Machine {
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles,
        strict_decode: false,
        psram: None,
    };
    Machine::new(image, &options).expect("machine")
}

/// Build the fast-forward / step-by-step pair / 构造快进与逐步对照机对
fn ff_pair(image: &[u8], max_cycles: u64) -> (Machine, Machine) {
    let mut fast = machine_from(image, max_cycles);
    let mut slow = machine_from(image, max_cycles);
    fast.set_sleep_fast_forward(true);
    slow.set_sleep_fast_forward(false);
    (fast, slow)
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

/// Step until core 0 sleeps, returning the step count / 推进到 core0 入睡
fn run_until_sleep(machine: &mut Machine) -> u64 {
    for _ in 0..64 {
        machine.step().expect("step");
        if machine.cpu_state() == CpuState::Sleeping {
            return machine.steps();
        }
    }
    panic!("core 0 never slept, PC={:#010X}", machine.pc());
}

/// Program one word-mode RAM-to-RAM DMA channel / 编程一个字宽 RAM→RAM DMA 通道
fn program_channel(machine: &mut Machine, src: u32, dst: u32, count: u32) {
    write_word(machine, DMA_BASE, src);
    write_word(machine, DMA_BASE + 0x04, dst);
    write_word(machine, DMA_BASE + 0x08, count);
    let ctrl = CTRL_EN
        | (2 << rp2350_periph::dma::CTRL_DATA_SIZE_SHIFT)
        | CTRL_INCR_READ
        | CTRL_INCR_WRITE
        | (u32::from(TREQ_PERMANENT) << CTRL_TREQ_SEL_SHIFT);
    write_word(machine, DMA_BASE + 0x0C, ctrl);
}

/// Launch core 1 on a counter loop stored in SRAM / 用 SRAM 中的计数循环启动 core1
fn launch_core1_counter(machine: &mut Machine) {
    let counter = SRAM_BASE + 0x8100;
    let code = [0x4902u16, 0x6808, 0x3001, 0x6008, 0xE7FB];
    for (index, halfword) in code.iter().enumerate() {
        let mut cycles = 0u64;
        machine
            .bus
            .write_u16(
                SRAM_BASE + 0x8000 + u32::try_from(index).unwrap() * 2,
                *halfword,
                &mut cycles,
            )
            .expect("core1 code");
    }
    write_word(machine, SRAM_BASE + 0x800C, counter);
    for word in [
        0u32,
        0,
        1,
        SRAM_BASE + 0x1000,
        SRAM_BASE + 0xA000,
        SRAM_BASE + 0x8001,
    ] {
        write_word(machine, SIO_FIFO_WR, word);
        machine.peripherals.service_core1_peer();
        assert_eq!(read_word(machine, SIO_FIFO_RD), word);
    }
}

/// Launch core 1 on a `WFI` instruction stored in SRAM / 用 SRAM 中的 `WFI` 启动 core1
fn launch_core1_sleep(machine: &mut Machine) {
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
        write_word(machine, SIO_FIFO_WR, word);
        machine.peripherals.service_core1_peer();
        assert_eq!(read_word(machine, SIO_FIFO_RD), word);
    }
}

/// Everything the fast-forward must keep identical to step-by-step execution
/// 快进必须与逐步执行完全一致的全部状态
#[derive(Debug, PartialEq, Eq)]
struct FfState {
    /// Executed steps / 已执行步数
    steps: u64,
    /// Simulated microseconds / 模拟微秒
    time_us: u64,
    /// Core 0 PC / core0 PC
    pc: u32,
    /// Core 0 SP / core0 SP
    sp: u32,
    /// Core 0 XPSR / core0 XPSR
    xpsr: u32,
    /// All 16 general-purpose registers / 全部 16 个通用寄存器
    regs: [u32; 16],
    /// Core 1 state and PC / core1 状态与 PC
    core1: Option<(CpuState, u32)>,
    /// Sleep steps and wakes per core / 每核睡眠步数与唤醒次数
    sleep: ([u64; 2], [u64; 2]),
    /// TIMER0 `INTR`, `ARMED`, `TIMERAWL`, `INTS` / TIMER0 的四个诊断值
    timer: (u32, u32, u32, u32),
    /// `SysTick` `CSR` and `CVR` / `SysTick` 的 `CSR` 与 `CVR`
    systick: (u32, u32),
    /// DMA units moved / DMA 搬运单元数
    dma: u64,
    /// HSTX words drained / HSTX 输出字数
    hstx: u64,
    /// Interrupt delivery log / 中断投递日志
    irqs: Vec<String>,
    /// Trap history / 陷阱历史
    traps: Vec<String>,
    /// Recent-step ring including opcodes / 最近步数环形缓冲（含操作码）
    tail: Vec<(u64, u32, u16)>,
    /// Queued host notices / 排队的宿主通知
    notices: Vec<String>,
    /// First 64 KiB of SRAM / SRAM 前 64 KiB
    sram: Vec<u8>,
}

/// Capture the state the fast-forward must preserve / 捕获快进必须保持的状态
fn snapshot(machine: &mut Machine) -> FfState {
    let mut regs = [0u32; 16];
    for (index, slot) in regs.iter_mut().enumerate() {
        *slot = machine.register(u8::try_from(index).unwrap());
    }
    FfState {
        steps: machine.steps(),
        time_us: machine.time_us(),
        pc: machine.pc(),
        sp: machine.sp(),
        xpsr: machine.xpsr(),
        regs,
        core1: machine
            .core1_pc()
            .map(|pc| (machine.core1_state().unwrap_or(CpuState::Reset), pc)),
        sleep: machine.sleep_counters(),
        timer: (
            machine.peripherals.timer().raw_status(),
            machine.peripherals.timer().armed(),
            read_word(machine, TIMER0_BASE + TIMERAWL),
            read_word(machine, TIMER0_BASE + 0x48),
        ),
        systick: (
            read_word(machine, 0xE000_E010),
            read_word(machine, 0xE000_E018),
        ),
        dma: machine.dma_transfers(),
        hstx: machine.hstx_words(),
        irqs: machine.irqs().to_vec(),
        traps: machine.traps().to_vec(),
        tail: machine.recent_steps(),
        notices: machine.take_notices(),
        sram: machine.read_block(SRAM_BASE, 0x1_0000),
    }
}

#[test]
fn timer_alarm_sleep_is_fast_forwarded_and_matches_step_by_step() {
    // 100 µs 的 TIMER alarm：核在约 20 000 步被唤醒，快进必须跳过绝大部分睡眠步，
    // 且唤醒后的全部可见状态与逐步路径一致。
    // A 100 µs TIMER alarm wakes the core around step 20 000: the fast-forward
    // must skip most sleeping steps and leave every visible value identical.
    let image = timer_sleep_uf2(100);
    let (mut fast, mut slow) = ff_pair(&image, 200_000);
    assert_eq!(fast.run_steps(100_000).expect("fast"), CpuState::Running);
    assert_eq!(slow.run_steps(100_000).expect("slow"), CpuState::Running);
    assert!(
        fast.sleep_fast_forward_steps() > 10_000,
        "the sleeping phase was skipped, got {}",
        fast.sleep_fast_forward_steps()
    );
    assert_eq!(fast.read_register(5), 1, "one wake / 一次唤醒");
    assert_eq!(fast.read_register(6), 1, "one handler entry / 一次处理");
    assert_eq!(snapshot(&mut fast), snapshot(&mut slow));
}

#[test]
fn repeated_segments_never_drift_from_the_step_by_step_path() {
    // 分成多个 run_steps 段推进：每一段结束时状态都必须完全一致，证明没有累积漂移。
    // Advance in several run_steps segments: every segment must end in an
    // identical state, proving there is no accumulated drift.
    let image = timer_sleep_uf2(300);
    let (mut fast, mut slow) = ff_pair(&image, 400_000);
    for segment in 0..12 {
        fast.run_steps(20_000).expect("fast segment");
        slow.run_steps(20_000).expect("slow segment");
        assert_eq!(
            snapshot(&mut fast),
            snapshot(&mut slow),
            "segment {segment} diverged"
        );
    }
    assert!(fast.sleep_fast_forward_steps() > 40_000);
}

#[test]
fn a_running_peer_blocks_fast_forward() {
    // core0 睡在 WFI 上，core1 在跑计数循环：另一核可运行，因此绝不快进。
    // Core 0 sleeps in WFI while core 1 runs a counter: a runnable peer means
    // no fast-forward at all.
    let image = timer_sleep_uf2(10_000_000);
    let (mut fast, mut slow) = ff_pair(&image, 1_000_000);
    launch_core1_counter(&mut fast);
    launch_core1_counter(&mut slow);
    run_until_sleep(&mut fast);
    run_until_sleep(&mut slow);
    assert_eq!(fast.pc(), TIMER_SLEEP_PC);
    fast.run_steps(5_000).expect("fast");
    slow.run_steps(5_000).expect("slow");
    assert_eq!(fast.core1_state(), Some(CpuState::Running));
    assert_eq!(fast.sleep_fast_forward_steps(), 0);
    assert_eq!(snapshot(&mut fast), snapshot(&mut slow));
}

#[test]
fn both_sleeping_cores_are_fast_forwarded_with_identical_state() {
    // 双核都睡眠时快进生效；超过 1M 步时「双核睡眠」通知必须与逐步路径逐字节相同。
    // With both cores asleep the fast-forward runs; past 1M steps the
    // "both cores slept" notice must be byte-identical to the slow path.
    let image = plain_wfi_uf2();
    let (mut fast, mut slow) = ff_pair(&image, 3_000_000);
    launch_core1_sleep(&mut fast);
    launch_core1_sleep(&mut slow);
    run_until_sleep(&mut fast);
    run_until_sleep(&mut slow);
    fast.run_steps(2_500_000).expect("fast");
    slow.run_steps(2_500_000).expect("slow");
    assert_eq!(fast.core1_state(), Some(CpuState::Sleeping));
    assert!(
        fast.sleep_fast_forward_steps() > 2_000_000,
        "both sleeping cores were skipped"
    );
    let state = snapshot(&mut fast);
    assert!(
        state
            .notices
            .iter()
            .any(|line| line.starts_with("[run] sleep stall:")),
        "the stall notice survives the batched path"
    );
    assert_eq!(state, snapshot(&mut slow));
}

#[test]
fn a_systick_expiry_wakes_at_the_same_step() {
    // SysTick 以 clk_sys 计数：到期必须在同一个泵送边界被转发为异常 15。
    // SysTick counts clk_sys: its expiry must be forwarded as exception 15 on
    // the same pump boundary in both paths.
    let image = plain_wfi_uf2();
    let (mut fast, mut slow) = ff_pair(&image, 300_000);
    for machine in [&mut fast, &mut slow] {
        write_word(machine, 0xE000_E010, 0x7); // ENABLE | TICKINT | CLKSOURCE
        write_word(machine, 0xE000_E014, 14_999);
        write_word(machine, 0xE000_E018, 15_000);
    }
    fast.run_steps(100_000).expect("fast");
    slow.run_steps(100_000).expect("slow");
    assert!(fast.sleep_fast_forward_steps() > 10_000);
    assert!(
        fast.irqs().iter().any(|line| line.contains("systick")),
        "the expiry was forwarded"
    );
    assert_eq!(snapshot(&mut fast), snapshot(&mut slow));
}

#[test]
fn a_clock_change_while_sleeping_does_not_cross_the_boundary() {
    // 睡眠期间把 CLK_SYS 分频改成 2：新频率必须与逐步路径一样在下一个时基边界生效。
    // Retune CLK_SYS to /2 while asleep: the new rate must take effect on the
    // next timebase boundary exactly as in the step-by-step path.
    let image = timer_sleep_uf2(5_000_000);
    let (mut fast, mut slow) = ff_pair(&image, 2_000_000);
    run_until_sleep(&mut fast);
    run_until_sleep(&mut slow);
    fast.run_steps(40_000).expect("fast before retune");
    slow.run_steps(40_000).expect("slow before retune");
    assert!(fast.sleep_fast_forward_steps() > 0);
    for machine in [&mut fast, &mut slow] {
        // 16.16 fixed point: 2.0 / 16.16 定点：2.0
        write_word(machine, CLK_SYS_DIV, 2 << 16);
    }
    assert_eq!(fast.peripherals.sysclk_hz(), 75_000_000);
    // 改频世代变化后，快进必须退避到下一个时基边界，由 `advance` 结算新频率。
    // After the retune bumps the generation, the fast-forward must back off until
    // the next timebase boundary settles the new rate.
    let skipped_before = fast.sleep_fast_forward_steps();
    fast.run_steps(1_000).expect("fast during back-off");
    assert_eq!(
        fast.sleep_fast_forward_steps(),
        skipped_before,
        "no skipping before the generation boundary"
    );
    slow.run_steps(1_000).expect("slow during back-off");
    fast.run_steps(100_000).expect("fast after retune");
    slow.run_steps(100_000).expect("slow after retune");
    assert!(fast.sleep_fast_forward_steps() > skipped_before);
    assert_eq!(snapshot(&mut fast), snapshot(&mut slow));
}

#[test]
fn dma_activity_blocks_fast_forward() {
    // 忙 DMA 通道每 64 步搬运一批：任何批量跳过都会改变传输节拍，因此守卫必须拒绝。
    // A busy DMA channel moves one batch per 64 steps: skipping would change its
    // pacing, so the guard must refuse.
    let image = plain_wfi_uf2();
    let (mut fast, mut slow) = ff_pair(&image, 500_000);
    launch_core1_sleep(&mut fast);
    launch_core1_sleep(&mut slow);
    for machine in [&mut fast, &mut slow] {
        program_channel(machine, SRAM_BASE, SRAM_BASE + 0x1000, 1_000_000);
    }
    run_until_sleep(&mut fast);
    run_until_sleep(&mut slow);
    fast.run_steps(20_000).expect("fast");
    slow.run_steps(20_000).expect("slow");
    assert!(fast.dma_transfers() > 0, "the channel is moving");
    assert_eq!(fast.sleep_fast_forward_steps(), 0);
    assert_eq!(snapshot(&mut fast), snapshot(&mut slow));
}

#[test]
fn pio_activity_blocks_fast_forward() {
    // 使能了一个 PIO 状态机：每个泵送批次都有副作用，因此不允许快进。
    // One PIO state machine is enabled: every pump batch has side effects, so
    // the fast-forward must stay off.
    let image = plain_wfi_uf2();
    let (mut fast, mut slow) = ff_pair(&image, 200_000);
    launch_core1_sleep(&mut fast);
    launch_core1_sleep(&mut slow);
    for machine in [&mut fast, &mut slow] {
        write_word(machine, PIO0_BASE + 0x048, 0x80A0); // pull block
        write_word(machine, PIO0_BASE + 0x04C, 0x0000); // jmp 0
        write_word(machine, PIO0_BASE, 1); // enable SM0
    }
    run_until_sleep(&mut fast);
    run_until_sleep(&mut slow);
    fast.run_steps(10_000).expect("fast");
    slow.run_steps(10_000).expect("slow");
    assert_eq!(fast.pio_busy_mask(), 1);
    assert_eq!(fast.sleep_fast_forward_steps(), 0);
    assert_eq!(snapshot(&mut fast), snapshot(&mut slow));
}

#[test]
fn an_irq_asserted_between_segments_wakes_on_the_same_boundary() {
    // 睡眠中由宿主置位 TIMER0 的强制中断位：下一次 64 步泵送必须唤醒两台机器的
    // 同一处理程序，快进不得把它提前或推后。
    // Raising TIMER0's force bit from the host while asleep: the next 64-step
    // pump must wake both machines into the same handler.
    let image = timer_sleep_uf2(10_000_000);
    let (mut fast, mut slow) = ff_pair(&image, 400_000);
    run_until_sleep(&mut fast);
    run_until_sleep(&mut slow);
    fast.run_steps(50_000).expect("fast");
    slow.run_steps(50_000).expect("slow");
    assert!(fast.sleep_fast_forward_steps() > 0);
    for machine in [&mut fast, &mut slow] {
        write_word(machine, TIMER0_BASE + TIMER_INTF, 1);
    }
    fast.run_steps(1_000).expect("fast wake");
    slow.run_steps(1_000).expect("slow wake");
    assert_eq!(fast.read_register(6), 1, "one handler entry");
    assert_eq!(fast.read_register(5), 1, "execution resumed after the WFI");
    assert_eq!(snapshot(&mut fast), snapshot(&mut slow));
}

#[test]
fn the_disabled_switch_never_skips_and_is_deterministic() {
    // 关闭开关时路径与旧实现完全相同：不跳过任何步，且两次运行逐字节一致。
    // With the switch off the path is the old one: no step is skipped and two
    // runs are byte-identical.
    let image = timer_sleep_uf2(100);
    let mut first = machine_from(&image, 200_000);
    let mut second = machine_from(&image, 200_000);
    first.set_sleep_fast_forward(false);
    second.set_sleep_fast_forward(false);
    first.run_steps(100_000).expect("first");
    second.run_steps(100_000).expect("second");
    assert_eq!(first.sleep_fast_forward_steps(), 0);
    assert_eq!(snapshot(&mut first), snapshot(&mut second));

    let mut fast = machine_from(&image, 200_000);
    fast.set_sleep_fast_forward(true);
    fast.run_steps(100_000).expect("fast");
    assert!(fast.sleep_fast_forward_steps() > 0);
    assert_eq!(snapshot(&mut fast), snapshot(&mut first));
}

#[test]
fn a_blocking_fifo_read_is_never_fast_forwarded() {
    // 阻塞 FIFO 读每步重放同一条载入：帧到达的时刻必须保持不变，因此不得快进。
    // A blocking FIFO load is replayed every step: the frame arrival must keep
    // its timing, so those steps must stay step-by-step.
    let image = blocked_fifo_uf2();
    let (mut fast, mut slow) = ff_pair(&image, 10_000);
    fast.run_steps(1_000).expect("fast");
    slow.run_steps(1_000).expect("slow");
    assert_eq!(fast.cpu_state(), CpuState::Sleeping);
    assert_eq!(fast.sleep_fast_forward_steps(), 0);
    assert_eq!(snapshot(&mut fast), snapshot(&mut slow));
    // 对端送帧后重放同一条载入并只消费一次。
    // A peer frame makes the retried load complete exactly once.
    fast.peripherals().sio().post_to_core(0x1234_5678);
    slow.peripherals().sio().post_to_core(0x1234_5678);
    fast.run_steps(10).expect("fast frame");
    slow.run_steps(10).expect("slow frame");
    assert_eq!(fast.read_register(0), 0x1234_5678);
    assert_eq!(snapshot(&mut fast), snapshot(&mut slow));
}

#[test]
fn the_wake_step_never_crosses_the_next_event() {
    // 以 32 步为粒度扫描唤醒点：两台机器必须在同一段醒来，且睡眠计数完全相同，
    // 证明快进没有跨过 TIMER alarm 的观察边界。
    // Scan the wake point in 32-step chunks: both machines must wake in the same
    // chunk with identical sleep counters, proving no event boundary was crossed.
    let image = timer_sleep_uf2(100);
    let (mut fast, mut slow) = ff_pair(&image, 100_000);
    let mut fast_chunk = None;
    let mut slow_chunk = None;
    for chunk in 0..2_000 {
        fast.run_steps(32).expect("fast chunk");
        slow.run_steps(32).expect("slow chunk");
        assert_eq!(fast.sleep_counters().0[0], slow.sleep_counters().0[0]);
        if fast_chunk.is_none() && fast.cpu_state() != CpuState::Sleeping {
            fast_chunk = Some(chunk);
        }
        if slow_chunk.is_none() && slow.cpu_state() != CpuState::Sleeping {
            slow_chunk = Some(chunk);
        }
        if fast_chunk.is_some() && slow_chunk.is_some() {
            break;
        }
    }
    assert_eq!(fast_chunk, slow_chunk, "both wake in the same chunk");
    assert!(fast_chunk.is_some(), "the alarm eventually wakes the core");
    assert!(fast.sleep_fast_forward_steps() > 0);
    assert_eq!(snapshot(&mut fast), snapshot(&mut slow));
}
