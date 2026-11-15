//! `run` frontend tests / `run` 前端测试
//!
//! Split out of `run.rs` so both files stay inside the project's per-file
//! code budget; the test bodies are unchanged.
//! 从 `run.rs` 拆出，使两个文件都落在项目的单文件代码预算内；测试内容不变。

use super::*;
use crate::test_support::{counter_elf, cpacr_write_elf, infinite_loop_elf, unmapped_branch_elf};
use rp2350_periph::dma::{
    CTRL_CHAIN_TO_SHIFT, CTRL_EN, CTRL_INCR_READ, CTRL_INCR_WRITE, CTRL_TREQ_SEL_SHIFT,
    TREQ_PERMANENT,
};
use rp2350_periph::map::DMA_BASE;

/// `SCB_CPACR` absolute address / `SCB_CPACR` 绝对地址
const SCB_CPACR: u32 = 0xE000_ED88;

/// Per-channel register stride / 每通道寄存器步长
const CHANNEL_STRIDE: u32 = 0x40;

/// `CTRL_TRIG.DATA_SIZE` value for word transfers / 字传输的 `DATA_SIZE` 值
const DATA_SIZE_WORD: u32 = 2;

/// Program one word-mode RAM-to-RAM channel through the guest bus
/// 通过客户机总线编程一个字宽 RAM→RAM 通道
fn program_channel(
    machine: &mut Machine,
    channel: u32,
    src: u32,
    dst: u32,
    count: u32,
    chain_to: u8,
) {
    let base = DMA_BASE + channel * CHANNEL_STRIDE;
    write_word(machine, base, src);
    write_word(machine, base + 0x04, dst);
    write_word(machine, base + 0x08, count);
    let ctrl = CTRL_EN
        | (DATA_SIZE_WORD << rp2350_periph::dma::CTRL_DATA_SIZE_SHIFT)
        | CTRL_INCR_READ
        | CTRL_INCR_WRITE
        | (u32::from(TREQ_PERMANENT) << CTRL_TREQ_SEL_SHIFT)
        | (u32::from(chain_to) << CTRL_CHAIN_TO_SHIFT);
    write_word(machine, base + 0x0C, ctrl);
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

/// Build a machine with a fixed Arm budget / 构建带固定 Arm 预算的机器
fn machine_with_budget(max_cycles: u64) -> Machine {
    let elf = infinite_loop_elf();
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles,
        strict_decode: false,
    };
    Machine::new(&elf, &options).expect("machine")
}

#[test]
fn dma_ram_to_ram_channel_completes_under_the_machine() {
    // 固件只写 DMA 寄存器，传输应由机器主循环自动推进。
    // Firmware only programs the DMA registers; the machine main loop must
    // advance the transfer by itself.
    let mut machine = machine_with_budget(1_000);
    let src = SRAM_BASE + 0x1000;
    let dst = SRAM_BASE + 0x2000;
    let words = [0x1111_1111u32, 0x2222_2222, 0x3333_3333, 0x4444_4444];
    for (index, word) in words.iter().enumerate() {
        write_word(&mut machine, src + index as u32 * 4, *word);
    }
    program_channel(&mut machine, 0, src, dst, 4, 0);

    machine.run_cycles(200);

    assert_eq!(machine.dma_transfers(), 4);
    for (index, word) in words.iter().enumerate() {
        assert_eq!(read_word(&mut machine, dst + index as u32 * 4), *word);
    }
    let state = machine.peripherals.dma().channel_state(0).expect("ch0");
    assert_eq!(state.trans_count, 0);
    assert!(!state.busy);
}

#[test]
fn a_busy_dma_channel_does_not_stall_the_cpu() {
    // 长传输期间 CPU 必须继续执行，DMA 只在每批预算内推进。
    // The CPU must keep executing during a long transfer; DMA only advances
    // within its per-batch budget.
    let mut machine = machine_with_budget(1_000);
    program_channel(
        &mut machine,
        0,
        SRAM_BASE + 0x1000,
        SRAM_BASE + 0x4000,
        1_000,
        0,
    );

    machine.run_cycles(300);

    assert_eq!(machine.steps(), 300);
    assert!(machine.dma_transfers() > 0);
    assert!(machine.dma_transfers() < 1_000);
    assert!(
        machine
            .peripherals
            .dma()
            .channel_state(0)
            .expect("ch0")
            .busy
    );
}

#[test]
fn the_dma_batch_budget_is_bounded() {
    // 一个批次最多 16 个单元：不会一次搬完整块而长时间不返回。
    // One batch moves at most 16 units, so a large block never blocks the
    // main loop for long.
    let mut machine = machine_with_budget(1_000);
    program_channel(
        &mut machine,
        0,
        SRAM_BASE + 0x1000,
        SRAM_BASE + 0x4000,
        1_000,
        0,
    );

    machine.run_cycles(PUMP_INTERVAL);
    assert_eq!(machine.dma_transfers(), DMA_PUMP_TRANSFERS);
    assert_eq!(
        machine
            .peripherals
            .dma()
            .channel_state(0)
            .expect("ch0")
            .trans_count,
        1_000 - DMA_PUMP_TRANSFERS as u32
    );

    machine.run_cycles(PUMP_INTERVAL);
    assert_eq!(machine.dma_transfers(), 2 * DMA_PUMP_TRANSFERS);
}

#[test]
fn idle_dma_reports_zero_transfers() {
    // 未编程任何通道时泵送必须完全跳过，步进行为保持不变。
    // With no channel programmed the pump must be a no-op and stepping must
    // keep its historical behaviour.
    let mut machine = machine_with_budget(5);
    machine.run_cycles(5);

    assert_eq!(machine.steps(), 5);
    assert!(machine.budget_exhausted());
    assert_eq!(machine.dma_transfers(), 0);
    assert_eq!(machine.peripherals.dma().busy_mask(), 0);
    assert_eq!(machine.dma_state(), "dma=0 busy=0x0000");
}

#[test]
fn chained_channels_complete_under_the_machine() {
    // 链式通道：ch1 先被配置并中止，ch0 完成后由 CHAIN_TO 启动 ch1。
    // Chained channels: ch1 is configured then aborted; completing ch0
    // starts ch1 through CHAIN_TO.
    let mut machine = machine_with_budget(1_000);
    let src0 = SRAM_BASE + 0x1000;
    let dst0 = SRAM_BASE + 0x2000;
    let src1 = SRAM_BASE + 0x3000;
    let dst1 = SRAM_BASE + 0x4000;
    for index in 0..4u32 {
        write_word(&mut machine, src0 + index * 4, 0xA000_0000 | index);
    }
    for index in 0..2u32 {
        write_word(&mut machine, src1 + index * 4, 0xB000_0000 | index);
    }
    program_channel(&mut machine, 1, src1, dst1, 2, 1);
    // CHAN_ABORT 只清 BUSY，保留 EN 与计数，链式启动即可重新开跑。
    // CHAN_ABORT only clears BUSY, keeping EN and the count, so the chain
    // can restart it.
    write_word(&mut machine, DMA_BASE + 0x464, 1 << 1);
    program_channel(&mut machine, 0, src0, dst0, 4, 1);

    machine.run_cycles(200);

    assert_eq!(machine.dma_transfers(), 6);
    for index in 0..4u32 {
        assert_eq!(
            read_word(&mut machine, dst0 + index * 4),
            0xA000_0000 | index
        );
    }
    for index in 0..2u32 {
        assert_eq!(
            read_word(&mut machine, dst1 + index * 4),
            0xB000_0000 | index
        );
    }
    assert_eq!(machine.peripherals.dma().busy_mask(), 0);
}

#[test]
fn traced_steps_advance_the_dma_pump() {
    // `run` 的轨迹前缀使用 trace_step；该路径也必须泵送 DMA。
    // The `run` trace prefix uses trace_step; that path must pump DMA too.
    let mut machine = machine_with_budget(1_000);
    program_channel(
        &mut machine,
        0,
        SRAM_BASE + 0x1000,
        SRAM_BASE + 0x2000,
        4,
        0,
    );

    for index in 0..PUMP_INTERVAL as usize {
        machine.trace_step(index).expect("trace step");
    }

    assert_eq!(machine.dma_transfers(), 4);
    let _ = machine.take_notices();
}

#[test]
fn gdb_single_steps_also_pump_dma() {
    // GDB `resume` 走 `step`；单步不能绕过 DMA 推进。
    // GDB `resume` uses `step`; single stepping must not bypass DMA.
    let mut machine = machine_with_budget(1_000);
    program_channel(
        &mut machine,
        0,
        SRAM_BASE + 0x1000,
        SRAM_BASE + 0x2000,
        4,
        0,
    );

    for _ in 0..PUMP_INTERVAL {
        machine.step().expect("step");
    }

    assert_eq!(machine.dma_transfers(), 4);
}

/// PIO `CTRL` offset / PIO `CTRL` 偏移
const PIO_CTRL: u32 = 0x000;

/// PIO `IRQ` offset (write-one-to-clear) / PIO `IRQ` 偏移（写 1 清零）
const PIO_IRQ: u32 = 0x030;

/// PIO `INSTR_MEM0` offset / PIO `INSTR_MEM0` 偏移
const PIO_INSTR_MEM0: u32 = 0x048;

/// PIO `TXF0` offset / PIO `TXF0` 偏移
const PIO_TXF0: u32 = 0x010;

/// `SM0_EXECCTRL` offset / `SM0_EXECCTRL` 偏移
const PIO_SM0_EXECCTRL: u32 = 0x0CC;

/// `MOV Y, Y` no-op instruction / `MOV Y, Y` 空操作指令
const PIO_NOP: u32 = 0xA042;

/// `SET pins, data` instruction / `SET pins, data` 指令
fn pio_set_pins(data: u32) -> u32 {
    0xE000 | (data & 0x1F)
}

/// `IRQ set, field` instruction / `IRQ set, field` 指令
fn pio_irq_set(field: u32) -> u32 {
    0xC000 | (field & 0x1F)
}

#[test]
fn pio_state_machines_tick_once_per_pump_batch() {
    // 10 条 NOP + WRAP_TOP = 9：PC 等于已执行拍数对 10 取模，因此 64 拍后
    // 为 4、128 拍后为 8，可区分“每 64 步 tick 一次”与“未 tick”。
    // Ten NOPs with WRAP_TOP = 9 make the PC the executed-cycle count mod
    // 10, so 64 cycles land on 4 and 128 on 8: the value distinguishes the
    // 64-step pump from no pump at all.
    let mut machine = machine_with_budget(1_000);
    let base = rp2350_periph::map::PIO0_BASE;
    write_word(&mut machine, base + PIO_INSTR_MEM0, pio_set_pins(1));
    for slot in 1..10 {
        write_word(&mut machine, base + PIO_INSTR_MEM0 + slot * 4, PIO_NOP);
    }
    write_word(&mut machine, base + PIO_SM0_EXECCTRL, 9 << 12);
    write_word(&mut machine, base + PIO_CTRL, 1);
    assert_eq!(machine.pio_busy_mask(), 0b0001);
    assert_eq!(machine.peripherals.pio(0).expect("pio0").output_levels(), 0);

    machine.run_cycles(PUMP_INTERVAL - 1);
    assert_eq!(machine.pio_pc(0, 0), Some(0), "no tick before the batch");
    assert_eq!(
        machine.peripherals.pio(0).expect("pio0").output_levels(),
        0,
        "no pin activity before the batch"
    );

    machine.run_cycles(1);
    assert_eq!(machine.pio_pc(0, 0), Some(4), "one batch of 64 cycles");
    assert_eq!(
        machine.peripherals.pio(0).expect("pio0").output_levels() & 1,
        1,
        "SET pins ran inside the batch"
    );

    machine.run_cycles(1);
    assert_eq!(machine.pio_pc(0, 0), Some(4), "one batch of 64 cycles");

    machine.run_cycles(PUMP_INTERVAL);
    assert_eq!(machine.pio_pc(0, 0), Some(8), "second batch");

    // 未使能的块不推进：SM1 始终保持 PC 0。
    // A disabled machine never advances: SM1 stays at PC 0.
    assert_eq!(machine.pio_pc(0, 1), Some(0));
}

#[test]
fn pio_irq_flags_reach_the_shared_irq_mask() {
    // `IRQ set` 在泵送批次内执行，标志必须出现在共享中断掩码中；客户机
    // 写 1 清零后立即释放。
    // `IRQ set` runs inside a pump batch, so the flag must appear in the
    // shared IRQ mask and the guest's write-one-to-clear must release it.
    let mut machine = machine_with_budget(1_000);
    let base = rp2350_periph::map::PIO0_BASE;
    write_word(&mut machine, base + PIO_INSTR_MEM0, pio_irq_set(2));
    write_word(&mut machine, base + PIO_CTRL, 1);
    let line = 1u64 << rp2350_periph::irq::PIO0_IRQ_0;
    assert_eq!(machine.peripherals.irq_mask() & line, 0);

    machine.run_cycles(PUMP_INTERVAL);
    assert_ne!(machine.peripherals.irq_mask() & line, 0, "flag 2 asserted");

    write_word(&mut machine, base + PIO_IRQ, 1 << 2);
    assert_eq!(machine.peripherals.irq_mask() & line, 0, "W1C released");
    assert_eq!(machine.peripherals.pio(0).expect("pio0").irq_flags(), 0);
}

#[test]
fn pio_tx_dreq_paces_the_dma_under_the_machine() {
    // PIO0_TX0 节拍的 DMA 只能填满 4 深度 FIFO，PIO 取走后继续推进。
    // A DMA channel paced by PIO0_TX0 can only fill the 4-deep FIFO; it
    // makes progress again after the PIO drains it.
    let mut machine = machine_with_budget(4_000);
    let src = SRAM_BASE + 0x1000;
    let pio_base = rp2350_periph::map::PIO0_BASE;
    for index in 0..6u32 {
        write_word(&mut machine, src + index * 4, 0x7000_0000 | index);
    }
    let ctrl = CTRL_EN
        | (DATA_SIZE_WORD << rp2350_periph::dma::CTRL_DATA_SIZE_SHIFT)
        | CTRL_INCR_READ
        | (u32::from(rp2350_periph::dreq::PIO0_TX0) << CTRL_TREQ_SEL_SHIFT);
    write_word(&mut machine, DMA_BASE, src);
    write_word(&mut machine, DMA_BASE + 0x04, pio_base + PIO_TXF0);
    write_word(&mut machine, DMA_BASE + 0x08, 6);
    write_word(&mut machine, DMA_BASE + 0x0C, ctrl);

    // 第一批：SM 关闭，DMA 填满 FIFO 后因 DREQ 变低而停止。
    // First batch: with the SM disabled the DMA fills the FIFO and stops
    // when the DREQ drops.
    machine.run_cycles(PUMP_INTERVAL);
    assert_eq!(machine.dma_transfers(), 4);
    assert_eq!(
        machine.peripherals.pio(0).expect("pio0").tx_fifo_full(0),
        Some(true)
    );
    assert_eq!(
        machine
            .peripherals
            .dma()
            .channel_state(0)
            .expect("ch0")
            .trans_count,
        2
    );
    machine.run_cycles(PUMP_INTERVAL);
    assert_eq!(machine.dma_transfers(), 4, "full FIFO gates the channel");

    // 启用 `pull block` 循环：PIO 取走两批数据后阻塞在空 FIFO 上。
    // Enable a `pull block` loop: the PIO drains both batches and finally
    // stalls on the empty FIFO.
    write_word(&mut machine, pio_base + PIO_INSTR_MEM0, 0x80A0);
    write_word(&mut machine, pio_base + PIO_INSTR_MEM0 + 4, 0x0000);
    write_word(&mut machine, pio_base + PIO_CTRL, 1);
    machine.run_cycles(PUMP_INTERVAL);
    machine.run_cycles(PUMP_INTERVAL);

    assert_eq!(machine.dma_transfers(), 6);
    let state = machine.peripherals.dma().channel_state(0).expect("ch0");
    assert_eq!(state.trans_count, 0);
    assert!(!state.busy);
    let pio0 = machine.peripherals.pio(0).expect("pio0");
    assert_eq!(pio0.tx_fifo_empty(0), Some(true));
    assert_ne!(pio0.stalled_mask() & 1, 0, "PULL waits on the empty FIFO");
    machine.peripherals.sync_pio_dreqs();
    assert_eq!(
        machine
            .peripherals
            .dma()
            .dreq(rp2350_periph::dreq::PIO0_TX0),
        Some(true)
    );
}

#[test]
fn unused_pio_blocks_do_not_change_stepping() {
    // 没有任何 PIO 使能时，泵送只做一次状态探测，步数与 PC 行为不变。
    // With no PIO enabled the pump only probes state: step count and PC
    // behaviour stay unchanged.
    let mut machine = machine_with_budget(200);
    machine.run_cycles(200);
    assert_eq!(machine.steps(), 200);
    assert_eq!(machine.pio_busy_mask(), 0);
    assert_eq!(machine.pio_pc(0, 0), Some(0));
    assert_eq!(machine.peripherals.irq_mask(), 0);
}

#[test]
fn memory_dump_specs_parse_decimal_and_hex() {
    let dump = MemoryDump::parse("0x20000000:0x12C00:fb.bin").expect("spec");
    assert_eq!(dump.address, 0x2000_0000);
    assert_eq!(dump.length, 0x1_2C00);
    assert_eq!(dump.path, PathBuf::from("fb.bin"));

    let plain = MemoryDump::parse("537000000:1024:out.bin").expect("decimal");
    assert_eq!(plain.address, 537_000_000);
    assert_eq!(plain.length, 1024);

    assert!(MemoryDump::parse("0x2000").is_err(), "missing fields");
    assert!(MemoryDump::parse("zz:1:x").is_err(), "bad address");
    assert!(MemoryDump::parse("1:zz:x").is_err(), "bad length");
    assert!(MemoryDump::parse("1:2:").is_err(), "empty path");
}

#[test]
fn machine_boots_and_stops_at_the_budget() {
    let elf = infinite_loop_elf();
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles: 5,
        strict_decode: false,
    };
    let mut machine = Machine::new(&elf, &options).expect("machine");
    assert_eq!(
        machine.boot_vectors(),
        Some((SRAM_BASE + SRAM_SIZE as u32, 0x1000_0000))
    );
    assert_eq!(machine.pc(), 0x1000_0000);
    // The PPB window must expose the same VTOR that boot_at programmed in
    // the core; Pico SDK's runtime_init copies the RAM vector table from it.
    // PPB 窗口必须暴露与 core 相同的 VTOR；Pico SDK runtime_init 依赖它
    // 复制 RAM 向量表。
    let mut cycles = 0u64;
    assert_eq!(
        machine
            .bus
            .read_u32(0xE000_ED08, &mut cycles)
            .expect("VTOR readable"),
        0x1000_0000
    );

    machine.run_cycles(3);
    assert_eq!(machine.steps(), 3);
    assert!(!machine.budget_exhausted());

    machine.run_cycles(100);
    assert_eq!(machine.steps(), 5);
    assert!(machine.budget_exhausted());
    assert_eq!(machine.halt_reason(), None);

    machine.run_cycles(100);
    assert_eq!(machine.steps(), 5);
    assert_eq!(machine.pc(), 0x1000_0000);
    assert_eq!(machine.time_us(), 0);
}

#[test]
fn trace_step_reports_the_branch_opcode() {
    let elf = infinite_loop_elf();
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles: 10,
        strict_decode: false,
    };
    let mut machine = Machine::new(&elf, &options).expect("machine");
    let state = machine.trace_step(0).expect("trace step");
    assert_eq!(state, CpuState::Running);
    let notices = machine.take_notices();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].starts_with("[trace] 000 PC=0x10000000 op=0xE7FE"));
}

#[test]
fn recent_steps_keep_a_bounded_pc_history() {
    let elf = infinite_loop_elf();
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles: 40,
        strict_decode: false,
    };
    let mut machine = Machine::new(&elf, &options).expect("machine");
    machine.run_cycles(40);
    let tail = machine.recent_steps();
    // 预算小于回溯窗口时，缓冲只保留实际执行过的步数。
    // When the budget is smaller than the trace window, the ring buffer
    // only holds the steps that actually ran.
    let expected = usize::try_from(machine.steps())
        .unwrap()
        .min(TAIL_TRACE_STEPS);
    assert_eq!(tail.len(), expected);
    assert_eq!(tail[0].0, machine.steps() - expected as u64);
    assert!(
        tail.iter()
            .all(|&(_, pc, opcode)| pc == 0x1000_0000 && opcode == 0xE7FE)
    );
}

#[test]
fn ppb_writes_reach_the_core_before_the_next_instruction() {
    // 固件在写 CPACR 后紧接协处理器指令；若系统写入按 64 步批量回放，
    // 这里第 3 步的写入要到第 64 步才生效，核内 CPACR 仍是上电默认值。
    // Firmware writes CPACR right before a coprocessor instruction. With a
    // 64-step batch replay, the step-3 write would not land until step 64
    // and the in-core CPACR would still hold its power-on default.
    let elf = cpacr_write_elf();
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles: 8,
        strict_decode: false,
    };
    let mut machine = Machine::new(&elf, &options).expect("machine");
    assert_eq!(
        machine.system_register(SCB_CPACR),
        Some(0x00F0_0000),
        "power-on CPACR must model the boot ROM's FPU enable / 上电 CPACR 应复现 boot ROM 的 FPU 使能"
    );

    machine.run_cycles(4);
    assert_eq!(machine.steps(), 4);
    assert_eq!(machine.halt_reason(), None);
    assert_eq!(
        machine.system_register(SCB_CPACR),
        Some(0x0030_0303),
        "CPACR write must be replayed on the very next step / CPACR 写入必须在下一步前回放"
    );
}

#[test]
fn sram_top_is_mapped_for_exception_stack_frames() {
    // MicroPython 把 MSP 放在 520 KB SRAM 顶端，软复位/异常压栈会访问
    // 顶端下方几百字节。若某个窗口抢先覆盖或 SRAM 尺寸算错，固件会在
    // raw REPL 握手阶段莫名报 NoDevice。
    // MicroPython parks MSP at the top of the 520 KB SRAM, so soft resets
    // and exception stacking touch the few hundred bytes below the top.
    // A clobbering window or a wrong SRAM size would surface as a spurious
    // NoDevice during the raw REPL handshake.
    use rp2350_core::bus::AddressBus;

    let elf = infinite_loop_elf();
    let machine = Machine::new(&elf, &MachineOptions::default()).expect("machine");
    let mut bus = machine.bus;
    let top = SRAM_BASE + SRAM_SIZE as u32;
    for addr in [top - 0x108, top - 0x20, top - 4] {
        let region = bus
            .find_region_mut(addr)
            .unwrap_or_else(|| panic!("no region for {addr:#010X}"));
        assert_eq!(region.name(), "sram", "unexpected owner of {addr:#010X}");
        let mut cycles = 0;
        bus.write_u32(addr, 0xA5A5_A5A5, &mut cycles)
            .unwrap_or_else(|e| panic!("write {addr:#010X}: {e:?}"));
        assert_eq!(bus.read_u32(addr, &mut cycles).expect("read"), 0xA5A5_A5A5);
    }
}

#[test]
fn image_summary_keeps_the_run_format() {
    let elf = infinite_loop_elf();
    let machine = Machine::new(&elf, &MachineOptions::default()).expect("machine");
    assert!(
        machine
            .image_description()
            .starts_with("elf: entry=0x10000001")
    );
    assert_eq!(machine.boot_vectors().map(|(_, pc)| pc), Some(0x1000_0000));
}

/// Build one 512-byte UF2 block with a family ID / 构造一个带 family ID 的 UF2 块
fn uf2_block(family: u32, address: u32, payload: &[u8]) -> Vec<u8> {
    let mut block = vec![0u8; 512];
    block[0..4].copy_from_slice(&0x0A32_4655u32.to_le_bytes());
    block[4..8].copy_from_slice(&0x9E5D_5157u32.to_le_bytes());
    // Family-ID-present flag. / 置位 family ID 存在标志。
    block[8..12].copy_from_slice(&0x0000_2000u32.to_le_bytes());
    block[12..16].copy_from_slice(&address.to_le_bytes());
    block[16..20].copy_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
    block[20..24].copy_from_slice(&0u32.to_le_bytes());
    block[24..28].copy_from_slice(&1u32.to_le_bytes());
    block[28..32].copy_from_slice(&family.to_le_bytes());
    block[32..32 + payload.len()].copy_from_slice(payload);
    block[508..512].copy_from_slice(&0x0AB1_6F30u32.to_le_bytes());
    block
}

#[test]
fn uf2_family_selects_the_cpu_architecture() {
    // RISC-V family must auto-select Hazard3; the explicit override wins.
    // RISC-V family 必须自动选择 Hazard3；显式覆盖优先。
    let riscv = uf2_block(0xE48B_FF5A, FLASH_XIP_BASE, &[0u8; 4]);
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("flash", FLASH_XIP_BASE, 0x1000)))
        .expect("flash region");
    let (arch, entry, description) = load_image(&riscv, &mut bus, None).expect("load");
    assert_eq!(arch, ArchArg::RiscV);
    assert_eq!(entry, None);
    assert!(description.contains("RISC-V"));
    let (arch, _, _) = load_image(&riscv, &mut bus, Some(ArchArg::Arm)).expect("load override");
    assert_eq!(arch, ArchArg::Arm);

    let arm = uf2_block(0xE48B_FF59, FLASH_XIP_BASE, &[0u8; 4]);
    let (arch, _, _) = load_image(&arm, &mut bus, None).expect("load arm");
    assert_eq!(arch, ArchArg::Arm);
}

// ---- core 1: launch protocol, stepping, IRQ routing ---------------------

/// SIO `FIFO_ST` as seen from core 0 / core0 视角的 SIO `FIFO_ST`
const SIO_FIFO_ST: u32 = 0xD000_0050;

/// SIO `FIFO_WR` / SIO `FIFO_WR`
const SIO_FIFO_WR: u32 = 0xD000_0054;

/// SIO `FIFO_RD` / SIO `FIFO_RD`
const SIO_FIFO_RD: u32 = 0xD000_0058;

/// `PSM_FRCE_OFF` SET alias / `PSM_FRCE_OFF` 的 SET 别名
const PSM_FRCE_OFF_SET: u32 = 0x4001_A004;

/// `PSM_FRCE_OFF` CLR alias / `PSM_FRCE_OFF` 的 CLR 别名
const PSM_FRCE_OFF_CLR: u32 = 0x4001_B004;

/// `PSM_FRCE_OFF.PROC1` / `PSM_FRCE_OFF.PROC1`
const PSM_PROC1: u32 = 0x0100_0000;

/// Core-1 code region inside SRAM / SRAM 中的 core1 代码区
const CORE1_CODE: u32 = SRAM_BASE + 0x8000;

/// Core-1 vector table inside SRAM / SRAM 中的 core1 向量表
const CORE1_VTOR: u32 = SRAM_BASE + 0x1000;

/// Core-1 initial stack pointer / core1 初始栈指针
const CORE1_STACK: u32 = SRAM_BASE + 0xA000;

/// Shared sentinel word / 共享哨兵字
const SENTINEL: u32 = SRAM_BASE + 0x9000;

/// `LDR r0,[pc,#4]; LDR r1,[pc,#8]; STR r0,[r1]; B .`
const STORE_PROGRAM: [u16; 4] = [0x4801, 0x4902, 0x6008, 0xE7FE];

/// `LDR r1,[pc,#8]; LDR r0,[r1]; STR r0,[r1,#4]; B .`
const COPY_PROGRAM: [u16; 4] = [0x4902, 0x6808, 0x6048, 0xE7FE];

/// `LDR r1,[pc,#8]; LDR r0,[r1]; ADDS r0,#1; STR r0,[r1]; B`
const COUNTER_LOOP: [u16; 5] = [0x4902, 0x6808, 0x3001, 0x6008, 0xE7FB];

/// Thumb `b .` / Thumb `b .`
const SPIN_PROGRAM: [u16; 1] = [0xE7FE];

/// Write a halfword program / 写入半字程序
fn bind_halfwords(machine: &mut Machine, base: u32, halfwords: &[u16]) {
    let mut cycles = 0u64;
    for (index, halfword) in halfwords.iter().enumerate() {
        machine
            .bus
            .write_u16(base + index as u32 * 2, *halfword, &mut cycles)
            .expect("code write");
    }
}

/// Write `value` to `target` from the store program / 用存储程序把值写入目标
fn bind_store_program(machine: &mut Machine, base: u32, target: u32, value: u32) {
    bind_halfwords(machine, base, &STORE_PROGRAM);
    write_word(machine, base + 0x08, value);
    write_word(machine, base + 0x0C, target);
}

/// Write a self-contained increment loop over `counter` / 写入自包含自增循环
fn bind_counter_program(machine: &mut Machine, base: u32, counter: u32) {
    bind_halfwords(machine, base, &COUNTER_LOOP);
    write_word(machine, base + 0x0C, counter);
}

/// Push the SDK's `{0,0,1,vtor,sp,entry}` stream and check every echo
/// 写入 SDK 的 `{0,0,1,vtor,sp,entry}` 命令流并校验每个回显
fn push_launch(machine: &mut Machine, vtor: u32, sp: u32, entry: u32) {
    for word in [0, 0, 1, vtor, sp, entry] {
        write_word(machine, SIO_FIFO_WR, word);
        machine.peripherals.service_core1_peer();
        assert_eq!(
            read_word(machine, SIO_FIFO_RD),
            word,
            "the boot-ROM peer must echo {word:#010X}"
        );
    }
    assert!(machine.peripherals.core1_launch_pending());
}

#[test]
fn the_launch_stream_boots_core1_from_entry_sp_and_vtor() {
    // 命令流被识别后 core1 从 entry 启动，SP 与 VTOR 取自同一命令流。
    // Once the stream is recognized core 1 boots at entry, with SP and VTOR
    // taken from the same command stream.
    let mut machine = machine_with_budget(1_000);
    bind_halfwords(&mut machine, CORE1_CODE, &SPIN_PROGRAM);
    assert_eq!(machine.core1_state(), Some(CpuState::Reset));

    push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);
    assert_eq!(machine.core1_state(), Some(CpuState::Reset));
    machine.step().expect("launch step");

    assert_eq!(machine.core1_state(), Some(CpuState::Running));
    assert_eq!(machine.core1_pc(), Some(CORE1_CODE));
    assert_eq!(machine.core1_sp(), Some(CORE1_STACK));
    assert_eq!(
        machine.core1_system_register(0xE000_ED08),
        Some(CORE1_VTOR),
        "core1 VTOR comes from the launch stream / core1 VTOR 来自启动流"
    );
    assert_eq!(machine.core1_launch_count(), 1);
    assert_eq!(machine.core1_halt_reason(), None);
}

#[test]
fn core1_never_touches_shared_sram_before_release() {
    // 未释放的 core1 处于 Reset：共享哨兵必须保持原值。
    // An unreleased core 1 stays in Reset, so the shared sentinel keeps its
    // original value.
    let mut machine = machine_with_budget(1_000);
    bind_store_program(&mut machine, CORE1_CODE, SENTINEL, 0x1234_5678);
    write_word(&mut machine, SENTINEL, 0xDEAD_BEEF);

    machine.run_cycles(200);

    assert_eq!(machine.core1_state(), Some(CpuState::Reset));
    assert_eq!(machine.core1_pc(), Some(0));
    assert_eq!(read_word(&mut machine, SENTINEL), 0xDEAD_BEEF);
    assert_eq!(machine.core1_launch_count(), 0);
}

#[test]
fn core1_writes_shared_sram_and_core0_reads_it() {
    // core1 运行手写 Thumb 程序写共享 SRAM，core0 随后用同一总线读出。
    // Core 1 runs a hand-written Thumb program that fills shared SRAM, then
    // core 0 reads it back through the same bus.
    let mut machine = machine_with_budget(1_000);
    bind_store_program(&mut machine, CORE1_CODE, SENTINEL, 0x5EED_1234);
    push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);

    machine.run_cycles(4);
    assert_eq!(machine.core1_state(), Some(CpuState::Running));
    assert_eq!(read_word(&mut machine, SENTINEL), 0x5EED_1234);

    bind_halfwords(&mut machine, SRAM_BASE + 0x8400, &COPY_PROGRAM);
    write_word(&mut machine, SRAM_BASE + 0x840C, SENTINEL);
    machine.write_register(15, SRAM_BASE + 0x8400);
    for _ in 0..3 {
        machine.step().expect("core0 copy");
    }
    assert_eq!(
        read_word(&mut machine, SENTINEL + 4),
        0x5EED_1234,
        "core0 copied core1's store / core0 复制了 core1 的写入"
    );
}

#[test]
fn core0_and_core1_keep_independent_register_files() {
    // 两核寄存器文件独立：core1 的栈与寄存器改变不影响 core0。
    // The register files are independent: core 1's stack and registers do
    // not disturb core 0's.
    let mut machine = machine_with_budget(1_000);
    bind_halfwords(&mut machine, CORE1_CODE, &SPIN_PROGRAM);
    machine.write_register(0, 0xAAAA_AAAA);
    let core0_sp = machine.sp();

    push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);
    machine.step().expect("launch step");

    assert_eq!(machine.core1_register(0), Some(0));
    assert_eq!(machine.core1_sp(), Some(CORE1_STACK));
    assert_eq!(machine.register(0), 0xAAAA_AAAA);
    assert_eq!(machine.sp(), core0_sp);
}

#[test]
fn both_cores_advance_their_own_counters() {
    // 双核交替步进：每次 step 两个核心各执行一条指令，计数器同步增长。
    // Alternating steps: each `step` retires one instruction per core, so the
    // two counters grow together.
    let mut machine = machine_with_budget(1_000);
    bind_counter_program(&mut machine, CORE1_CODE, SENTINEL);
    bind_counter_program(&mut machine, SRAM_BASE + 0x8400, SENTINEL + 4);
    push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);
    machine.write_register(15, SRAM_BASE + 0x8400);

    machine.run_cycles(40);

    assert_eq!(read_word(&mut machine, SENTINEL), 10);
    assert_eq!(read_word(&mut machine, SENTINEL + 4), 10);
}

#[test]
fn a_halted_core1_does_not_stop_core0_or_consume_budget() {
    // core1 的总线错误只停机 core1：core0 继续跑，预算按 core0 步数计。
    // A core-1 bus error only stops core 1: core 0 keeps running and the
    // budget still counts core-0 steps.
    let mut machine = machine_with_budget(20);
    push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, 0x3000_0000 | 1);

    machine.run_cycles(5);
    assert_eq!(machine.core1_state(), Some(CpuState::Halted));
    assert!(machine.core1_halt_reason().is_some());
    assert_eq!(machine.halt_reason(), None, "core0 must survive");
    assert_eq!(machine.steps(), 5);

    machine.run_cycles(5);
    assert_eq!(machine.steps(), 10);

    machine.run_cycles(100);
    assert_eq!(machine.steps(), 20);
    assert!(machine.budget_exhausted());
}

#[test]
fn core1_pushes_a_fifo_frame_that_core0_pops() {
    // core1 写自己的 FIFO_WR 后，core0 的 FIFO_RD / SIO_IRQ_FIFO 必须看到它。
    // After core 1 writes its own FIFO_WR, core 0's FIFO_RD and SIO_IRQ_FIFO
    // must observe the frame.
    let mut machine = machine_with_budget(1_000);
    bind_store_program(&mut machine, CORE1_CODE, SIO_FIFO_WR, 0);
    push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);
    let fifo_line = 1u64 << rp2350_periph::map::SIO_IRQ_FIFO;

    machine.run_cycles(4);

    assert_ne!(
        read_word(&mut machine, SIO_FIFO_ST) & 1,
        0,
        "core0's receive FIFO has the frame / core0 收件箱有数据"
    );
    assert_ne!(machine.peripherals.irq_mask() & fifo_line, 0);
    assert_eq!(read_word(&mut machine, SIO_FIFO_RD), 0);
    assert_eq!(
        machine.peripherals.irq_mask() & fifo_line,
        0,
        "popping releases the line / 弹出后释放中断线"
    );
}

#[test]
fn core1_takes_a_dma_completion_interrupt() {
    // DMA_IRQ_0 属于共享线；core1 使能后必须进入自己的 hstx 处理程序。
    // DMA_IRQ_0 is a shared line; once core 1 enables it, its own handler
    // must run.
    let mut machine = machine_with_budget(1_000);
    let handler = SRAM_BASE + 0x8800;
    let irq = rp2350_periph::map::DMA_IRQ_0;
    bind_halfwords(&mut machine, handler, &SPIN_PROGRAM);
    bind_halfwords(&mut machine, CORE1_CODE, &SPIN_PROGRAM);
    write_word(&mut machine, CORE1_VTOR, CORE1_STACK);
    write_word(&mut machine, CORE1_VTOR + 4, CORE1_CODE | 1);
    write_word(
        &mut machine,
        CORE1_VTOR + (16 + u32::from(irq)) * 4,
        handler | 1,
    );
    push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);
    machine.step().expect("launch step");
    machine
        .core1
        .as_mut()
        .expect("core1")
        .system_write(0xE000_E100, 1u32 << u32::from(irq));

    program_channel(
        &mut machine,
        0,
        SRAM_BASE + 0x1000,
        SRAM_BASE + 0x2000,
        1,
        0,
    );
    write_word(&mut machine, DMA_BASE + 0x404, 1);
    machine.run_cycles(2 * PUMP_INTERVAL);

    assert_eq!(
        machine.core1_pc(),
        Some(handler),
        "core1 entered its DMA_IRQ_0 handler / core1 已进入 DMA_IRQ_0 处理程序"
    );
}

#[test]
fn psm_reset_then_relaunch_restarts_core1() {
    // 置位 PROC1 硬复位 core1 并清空启动状态；清除后重新走一遍启动协议。
    // Setting PROC1 hard-resets core 1 and drops the launch state; clearing
    // it lets the launch protocol run again.
    let mut machine = machine_with_budget(1_000);
    bind_halfwords(&mut machine, CORE1_CODE, &SPIN_PROGRAM);
    push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);
    machine.step().expect("launch step");
    assert_eq!(machine.core1_state(), Some(CpuState::Running));
    assert_eq!(machine.core1_launch_count(), 1);

    write_word(&mut machine, PSM_FRCE_OFF_SET, PSM_PROC1);
    machine.step().expect("reset step");
    assert_eq!(machine.core1_state(), Some(CpuState::Reset));
    assert_eq!(machine.core1_pc(), Some(0));

    write_word(&mut machine, PSM_FRCE_OFF_CLR, PSM_PROC1);
    machine.peripherals.service_core1_peer();
    assert_eq!(
        read_word(&mut machine, SIO_FIFO_RD),
        0,
        "boot ROM acknowledges the release / boot ROM 确认释放"
    );

    push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);
    machine.step().expect("relaunch step");
    assert_eq!(machine.core1_state(), Some(CpuState::Running));
    assert_eq!(machine.core1_launch_count(), 2);
}

#[test]
fn gdb_single_steps_advance_core1() {
    // GDB `resume` 走 `Machine::step`，因此单步必须同时推进 core1。
    // GDB `resume` goes through `Machine::step`, so a single step must also
    // advance core 1.
    let mut machine = machine_with_budget(1_000);
    bind_counter_program(&mut machine, CORE1_CODE, SENTINEL);
    push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);
    machine.step().expect("launch step");
    let first_pc = machine.core1_pc().expect("core1 pc");
    assert_eq!(read_word(&mut machine, SENTINEL), 0);

    machine.step().expect("single step");
    assert_ne!(machine.core1_pc(), Some(first_pc));
    machine.step().expect("single step");
    machine.step().expect("single step");
    assert_eq!(read_word(&mut machine, SENTINEL), 1);
}

/// Build a machine whose two cores each run their own increment loop
/// 构建两个核心各自运行自增循环的机器
///
/// Core 1 is released through the real SIO FIFO launch protocol, so both
/// cores are `Running` when this returns: core 0 counts into [`SENTINEL`]
/// and core 1 into `SENTINEL + 4`.
/// core1 通过真实的 SIO FIFO 启动协议释放，因此返回时双核都处于 `Running`：
/// core0 自增 [`SENTINEL`]，core1 自增 `SENTINEL + 4`。
fn dual_core_machine(max_cycles: u64) -> Machine {
    let mut machine = machine_with_budget(max_cycles);
    bind_counter_program(&mut machine, CORE1_CODE, SENTINEL);
    bind_counter_program(&mut machine, SRAM_BASE + 0x8400, SENTINEL + 4);
    push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);
    machine.write_register(15, SRAM_BASE + 0x8400);
    machine.step().expect("launch step");
    assert_eq!(machine.core1_state(), Some(CpuState::Running));
    machine
}

#[test]
fn step_cores_core0_only_freezes_core1() {
    // 只选 core0：core1 的 PC 与计数器保持不变，步数仍推进一次。
    // Core 0 only: core 1's PC and counter stay frozen while steps advance.
    let mut machine = dual_core_machine(1_000);
    let core0_pc = machine.pc();
    let core1_pc = machine.core1_pc().expect("core1 pc");
    let core0_counter = read_word(&mut machine, SENTINEL + 4);
    let core1_counter = read_word(&mut machine, SENTINEL);
    let steps = machine.steps();

    // 循环体 4 条指令，7 步后 PC 一定落在不同偏移。
    // The loop body is four instructions, so seven steps land on a
    // different PC offset.
    for _ in 0..7 {
        machine.step_cores(true, false).expect("core0 step");
    }

    assert_ne!(
        machine.pc(),
        core0_pc,
        "core0 must advance / core0 必须推进"
    );
    assert!(read_word(&mut machine, SENTINEL + 4) > core0_counter);
    assert_eq!(
        machine.core1_pc(),
        Some(core1_pc),
        "core1 must stay frozen / core1 必须冻结"
    );
    assert_eq!(read_word(&mut machine, SENTINEL), core1_counter);
    assert_eq!(machine.steps(), steps + 7);
}

#[test]
fn step_cores_core1_only_freezes_core0() {
    // 只选 core1：core0 的 PC 与计数器保持不变。
    // Core 1 only: core 0's PC and counter stay frozen.
    let mut machine = dual_core_machine(1_000);
    let core0_pc = machine.pc();
    let core1_pc = machine.core1_pc().expect("core1 pc");
    let core0_counter = read_word(&mut machine, SENTINEL + 4);
    let steps = machine.steps();

    for _ in 0..7 {
        machine.step_cores(false, true).expect("core1 step");
    }

    assert_eq!(
        machine.pc(),
        core0_pc,
        "core0 must stay frozen / core0 必须冻结"
    );
    assert_ne!(machine.core1_pc(), Some(core1_pc));
    assert_eq!(read_word(&mut machine, SENTINEL + 4), core0_counter);
    assert!(read_word(&mut machine, SENTINEL) > 0, "core1 must advance");
    assert_eq!(machine.steps(), steps + 7);
}

#[test]
fn step_cores_with_both_flags_matches_step() {
    // `step_cores(true, true)` 与 `step()` 在 PC、步数、计数器与时基上等价。
    // `step_cores(true, true)` equals `step()` on PC, steps, counters and time.
    let mut plain = dual_core_machine(1_000);
    let mut selected = dual_core_machine(1_000);

    for _ in 0..20 {
        plain.step().expect("step");
        selected.step_cores(true, true).expect("step cores");
    }

    assert_eq!(selected.steps(), plain.steps());
    assert_eq!(selected.pc(), plain.pc());
    assert_eq!(selected.core1_pc(), plain.core1_pc());
    assert_eq!(selected.time_us(), plain.time_us());
    assert_eq!(
        read_word(&mut selected, SENTINEL),
        read_word(&mut plain, SENTINEL)
    );
    assert_eq!(
        read_word(&mut selected, SENTINEL + 4),
        read_word(&mut plain, SENTINEL + 4)
    );
}

#[test]
fn step_cores_skips_a_reset_core1_without_side_effects() {
    // core1 未启动时只选 core1 不退役指令：状态、PC 与步数都不变。
    // Arm 目前没有 `Sleeping` 状态（WFI 已回退），`Reset`/`Halted` 覆盖非运行态。
    // Selecting a never-released core 1 retires nothing: state, PC and the
    // step counter stay unchanged. The Arm core has no `Sleeping` state in
    // this tree (WFI was reverted), so `Reset` and `Halted` cover non-running.
    let mut machine = machine_with_budget(1_000);
    bind_counter_program(&mut machine, CORE1_CODE, SENTINEL);
    let steps = machine.steps();

    for _ in 0..4 {
        machine.step_cores(false, true).expect("core1 step");
    }

    assert_eq!(machine.core1_state(), Some(CpuState::Reset));
    assert_eq!(machine.core1_pc(), Some(0));
    assert_eq!(machine.core1_launch_count(), 0);
    assert_eq!(machine.steps(), steps, "nothing retired / 未退役指令");
    assert_eq!(read_word(&mut machine, SENTINEL), 0);
}

#[test]
fn step_cores_keeps_an_unselected_halted_core1_halted() {
    // 未选中的 core1 已经停机：其停机原因、PC 与机器停机状态都不受影响。
    // An unselected, already halted core 1 keeps its reason, PC and the
    // machine halt state untouched.
    let mut machine = machine_with_budget(20);
    push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, 0x3000_0000 | 1);
    machine.run_cycles(5);
    assert_eq!(machine.core1_state(), Some(CpuState::Halted));
    let reason = machine.core1_halt_reason().map(str::to_owned);
    let core1_pc = machine.core1_pc();

    for _ in 0..4 {
        machine.step_cores(true, false).expect("core0 step");
    }

    assert_eq!(machine.core1_state(), Some(CpuState::Halted));
    assert_eq!(machine.core1_halt_reason().map(str::to_owned), reason);
    assert_eq!(machine.core1_pc(), core1_pc);
    assert_eq!(machine.halt_reason(), None);
    assert_eq!(machine.steps(), 9);
}

#[test]
fn core1_only_steps_still_pump_dma_and_the_timebase() {
    // 仅 core1 步进仍走公共外设路径：步数/预算、DMA 泵送与时基都要推进。
    // Core-1-only stepping still runs the shared peripheral path: the step
    // counter/budget, the DMA pump and the timebase keep advancing.
    let mut machine = machine_with_budget(20_000);
    bind_counter_program(&mut machine, CORE1_CODE, SENTINEL);
    push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);
    program_channel(
        &mut machine,
        0,
        SRAM_BASE + 0x1000,
        SRAM_BASE + 0x4000,
        1_000,
        0,
    );

    for _ in 0..10_000 {
        machine.step_cores(false, true).expect("core1 step");
    }

    assert_eq!(machine.core1_state(), Some(CpuState::Running));
    assert_eq!(machine.steps(), 10_000);
    assert!(
        machine.dma_transfers() > 0,
        "DMA must advance / DMA 必须推进"
    );
    assert!(
        machine.time_us() > 0,
        "timebase must advance / 时基必须推进"
    );
}

#[test]
fn step_cores_respects_the_budget() {
    // 预算耗尽后按核步进立即返回 `Halted`，不再推进任何核心。
    // Once the budget is gone, a per-core step returns `Halted` without
    // advancing any core.
    let mut machine = machine_with_budget(3);
    bind_counter_program(&mut machine, CORE1_CODE, SENTINEL);
    push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);

    for _ in 0..3 {
        machine.step_cores(false, true).expect("core1 step");
    }
    let core1_pc = machine.core1_pc();
    let state = machine.step_cores(false, true).expect("budget stop");

    assert_eq!(state, CpuState::Halted);
    assert!(machine.budget_exhausted());
    assert_eq!(machine.core1_pc(), core1_pc);
    assert_eq!(machine.steps(), 3);
}

/// Environment variable naming the firmware for the ignore-gated smoke test
/// 冒烟测试所用的固件路径环境变量
///
/// Example: `$env:RP2350_EMERALD_UF2 = "…\emerald.uf2"`. The test is skipped
/// when the variable is unset or the file is unreadable, because the image is
/// not part of this repository.
/// 例如 `$env:RP2350_EMERALD_UF2 = "…\emerald.uf2"`。映像不属于本仓库，因此变量
/// 未设置或文件不可读时测试直接跳过。
const EMERALD_UF2_ENV: &str = "RP2350_EMERALD_UF2";

#[test]
#[ignore = "long firmware smoke test / 长固件冒烟测试"]
fn emerald_runs_core1_hstx_without_bus_errors() {
    // 真实固件冒烟：`main` 用 `multicore_launch_core1_with_stack` 启动
    // core1_render，其第一件事是 hstx_display_start，因此 HSTX 输出字数必须
    // 变为非零，且不得出现总线错误。
    // Real-firmware smoke test: `main` launches `core1_render` through
    // `multicore_launch_core1_with_stack`; its first action is
    // `hstx_display_start`, so HSTX output words must become non-zero and no
    // bus error may occur.
    let Some(path) = std::env::var_os(EMERALD_UF2_ENV) else {
        return;
    };
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    let options = MachineOptions {
        arch: None,
        max_cycles: 500_000_000,
        strict_decode: false,
    };
    let mut machine = Machine::new(&bytes, &options).expect("machine");
    while machine.halt_reason().is_none() && !machine.budget_exhausted() {
        machine.step().expect("step");
    }
    machine.finalize();

    assert_eq!(machine.halt_reason(), None);
    assert_eq!(machine.core1_state(), Some(CpuState::Running));
    assert!(machine.core1_launch_count() >= 1);
    assert!(
        machine.hstx_words() > 0,
        "hstx words={} dma={} / HSTX 输出字数为 0",
        machine.hstx_words(),
        machine.dma_transfers()
    );
    // Pixel-clock pacing keeps the expander in step with the DMA ring, so a
    // healthy scanout shows no starvation episodes and (at most) a single
    // start-up command/data hiccup.
    // 像素时钟节拍使扩展器与 DMA 环同步，因此健康扫描不应有断流，命令/数据
    // 相位错位最多出现一次（启动期）。
    let hstx = machine.peripherals.hstx();
    assert_eq!(
        hstx.output_underrun(),
        0,
        "hstx underruns must stay at zero / HSTX 不应断流"
    );
    assert!(
        hstx.unknown_commands() <= 1,
        "unknown commands: {} / 未知命令过多",
        hstx.unknown_commands()
    );
    assert!(machine.dma_transfers() > 0);
}

// ---- 批量步进 / batched stepping ------------------------------------------

/// Everything a batched run must keep identical to single stepping
/// 批量推进必须与逐指令推进完全一致的全部状态
#[derive(Debug, PartialEq, Eq)]
struct SteppedState {
    /// Executed steps / 已执行步数
    steps: u64,
    /// Core 0 program counter / core0 程序计数器
    pc: u32,
    /// Core 0 stack pointer / core0 栈指针
    sp: u32,
    /// Core 0 program status register / core0 程序状态寄存器
    xpsr: u32,
    /// Core 0 accumulated cycles / core0 累计周期
    cycles: u64,
    /// DMA data units moved by the host pump / 宿主泵送搬运的 DMA 数据单元
    dma: u64,
    /// HSTX words drained by the host pump / 宿主泵送排空的 HSTX 字数
    hstx: u64,
    /// Logged interrupt deliveries / 已记录的中断投递
    irqs: Vec<String>,
    /// Core 1 program counter / core1 程序计数器
    core1_pc: Option<u32>,
    /// Core 1 execution state / core1 执行状态
    core1_state: Option<CpuState>,
    /// Recent-step ring, including opcodes / 最近步数环形缓冲（含操作码）
    tail: Vec<(u64, u32, u16)>,
}

/// Capture the state `run_steps` must preserve / 捕获 `run_steps` 必须保持的状态
fn stepped_state(machine: &mut Machine) -> SteppedState {
    SteppedState {
        steps: machine.steps(),
        pc: machine.pc(),
        sp: machine.sp(),
        xpsr: machine.xpsr(),
        cycles: machine.core.total_cycles(),
        dma: machine.dma_transfers(),
        hstx: machine.hstx_words(),
        irqs: machine.irqs().to_vec(),
        core1_pc: machine.core1_pc(),
        core1_state: machine.core1_state(),
        tail: machine.recent_steps(),
    }
}

#[test]
fn batched_steps_match_single_steps_over_a_counter_loop() {
    // 计数器循环使 PC 在三个地址间变化，环形缓冲因此包含不同 PC。
    // The counter loop moves the PC between three addresses, so the ring holds
    // distinct PCs.
    let elf = counter_elf();
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles: 100_000,
        strict_decode: false,
    };
    let mut single = Machine::new(&elf, &options).expect("single machine");
    let mut batched = Machine::new(&elf, &options).expect("batched machine");
    for _ in 0..3_000 {
        single.step().expect("single step");
    }
    assert_eq!(
        batched.run_steps(3_000).expect("batched steps"),
        CpuState::Running
    );
    assert_eq!(stepped_state(&mut single), stepped_state(&mut batched));
}

#[test]
fn batched_steps_advance_both_cores_like_step() {
    // 双核都在运行时，core1 必须与逐指令路径同样推进。
    // With both cores running, core 1 must advance exactly like the per-step path.
    let mut single = dual_core_machine(100_000);
    let mut batched = dual_core_machine(100_000);
    for _ in 0..500 {
        single.step().expect("single step");
    }
    assert_eq!(
        batched.run_steps(500).expect("batched steps"),
        CpuState::Running
    );
    assert_eq!(stepped_state(&mut single), stepped_state(&mut batched));
    assert_ne!(
        batched.core1_pc(),
        Some(0),
        "core1 retired instructions / core1 必须推进"
    );
}

#[test]
fn batched_steps_keep_interrupt_delivery_on_the_same_step() {
    // PIO 在泵送批次内执行 `IRQ set`，中断线随后由共享投递路径送达；两条路径的
    // 投递步号必须一致，因此 `[irq]` 日志必须逐条相同。
    // `IRQ set` runs inside a pump batch and the line is then delivered by the
    // shared path; both paths must deliver on the same steps, so the `[irq]`
    // log must match entry by entry.
    let elf = counter_elf();
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles: 100_000,
        strict_decode: false,
    };
    let mut single = Machine::new(&elf, &options).expect("single machine");
    let mut batched = Machine::new(&elf, &options).expect("batched machine");
    let base = rp2350_periph::map::PIO0_BASE;
    for machine in [&mut single, &mut batched] {
        write_word(machine, base + PIO_INSTR_MEM0, pio_irq_set(2));
        write_word(machine, base + PIO_CTRL, 1);
    }
    for _ in 0..1_000 {
        single.step().expect("single step");
    }
    assert_eq!(
        batched.run_steps(1_000).expect("batched steps"),
        CpuState::Running
    );
    let single_state = stepped_state(&mut single);
    let batched_state = stepped_state(&mut batched);
    assert!(!single_state.irqs.is_empty(), "PIO IRQ must be delivered");
    assert_eq!(single_state, batched_state);
}

#[test]
fn batched_steps_wake_the_dma_pump_after_it_goes_idle() {
    // 空闲门控不能让泵送永久休眠：控制器空闲后再次使能通道仍必须推进。
    // The idle gate must not put the pump to sleep forever: re-enabling a
    // channel after the controller went idle must still make progress.
    let mut machine = machine_with_budget(100_000);
    let src = SRAM_BASE + 0x1000;
    let dst = SRAM_BASE + 0x2000;
    program_channel(&mut machine, 0, src, dst, 4, 0);
    machine.run_steps(200).expect("batched steps");
    assert_eq!(machine.dma_transfers(), 4);
    assert!(
        !machine
            .peripherals
            .dma()
            .channel_state(0)
            .expect("ch0")
            .busy
    );

    program_channel(&mut machine, 0, src, dst + 0x100, 4, 0);
    machine.run_steps(200).expect("batched steps");
    assert_eq!(machine.dma_transfers(), 8, "pump must wake up again");
}

#[test]
fn run_steps_stops_exactly_at_the_budget() {
    let mut machine = machine_with_budget(100);
    assert_eq!(
        machine.run_steps(u64::MAX).expect("batched steps"),
        CpuState::Running
    );
    assert_eq!(machine.steps(), 100);
    assert!(machine.budget_exhausted());
    // 预算耗尽后不再推进，返回 Halted。 / No progress after the budget: Halted.
    assert_eq!(
        machine.run_steps(10).expect("batched steps"),
        CpuState::Halted
    );
    assert_eq!(machine.steps(), 100);
}

#[test]
fn fault_tail_survives_batched_stepping() {
    // 故障回溯依赖逐指令的最近步数环形缓冲，批量推进不得让它变短或改变内容。
    // The fault tail depends on the per-instruction recent-step ring; batched
    // stepping must neither shorten it nor change its contents.
    let elf = unmapped_branch_elf();
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles: 1_000,
        strict_decode: false,
    };
    let mut single = Machine::new(&elf, &options).expect("single machine");
    let mut batched = Machine::new(&elf, &options).expect("batched machine");
    while single.halt_reason().is_none() && !single.budget_exhausted() {
        single.step().expect("single step");
    }
    machine_run_to_halt(&mut batched);
    assert!(single.halt_reason().is_some(), "the image must fault");
    assert_eq!(single.halt_reason(), batched.halt_reason());
    assert_eq!(single.steps(), batched.steps());

    let single_lines = fault_tail_lines(&mut single);
    let batched_lines = fault_tail_lines(&mut batched);
    assert_eq!(single_lines, batched_lines);
    let tails = batched_lines
        .iter()
        .filter(|line| line.starts_with("[tail]"))
        .count();
    assert_eq!(
        tails, 3,
        "both retired instructions plus the faulting fetch are recorded"
    );
}

/// Drive a machine to its halt with batched steps / 用批量步进跑到停机
fn machine_run_to_halt(machine: &mut Machine) {
    while machine.halt_reason().is_none() && !machine.budget_exhausted() {
        machine.run_steps(1_000).expect("batched steps");
    }
}
