//! Unit tests for the CLI GDB target adapter
//! CLI GDB 目标适配器的单元测试
//!
//! Split out of `gdb_target.rs` to keep that file within the per-file
//! code-line budget; `super` is still the adapter module.
//! 从 `gdb_target.rs` 拆出，使该文件保持单文件代码行预算；`super` 仍指向
//! 适配器模块。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use rp2350_core::FLASH_XIP_BASE;
use rp2350_gdb::{RspSession, decode_packet, encode_hex, encode_packet};

use super::*;
use crate::run::ArchArg;
use crate::test_support::cpacr_write_elf;

/// SRAM base address used by the memory tests / 内存测试使用的 SRAM 基地址
const SRAM_BASE: u32 = 0x2000_0000;

/// Build a target around the four-instruction `CPACR` fixture
/// 围绕四条指令的 `CPACR` 夹具构造调试目标
fn target() -> MachineTarget {
    let elf = cpacr_write_elf();
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles: u64::MAX,
        strict_decode: false,
        psram: None,
    };
    MachineTarget::new(Machine::new(&elf, &options).expect("machine"))
}

#[test]
fn registers_round_trip_through_the_machine() {
    let mut target = target();

    assert_eq!(target.read_register(regs::PC).expect("pc"), FLASH_XIP_BASE);
    target
        .write_register(regs::R0, 0xDEAD_BEEF)
        .expect("write r0");
    assert_eq!(target.read_register(regs::R0).expect("r0"), 0xDEAD_BEEF);

    target
        .write_register(regs::XPSR, 0x1234_5678)
        .expect("write xpsr");
    assert_eq!(target.read_register(regs::XPSR).expect("xpsr"), 0x1234_5678);

    assert!(matches!(
        target.read_register(regs::COUNT),
        Err(GdbError::InvalidRegister(regs::COUNT))
    ));
    assert!(target.write_register(regs::COUNT, 1).is_err());
}

#[test]
fn memory_round_trip_and_unmapped_error() {
    let mut target = target();

    assert_eq!(
        target.read_memory(FLASH_XIP_BASE, 4).expect("flash read"),
        vec![0x03, 0x48, 0x04, 0x49]
    );

    target
        .write_memory(SRAM_BASE, &[0xAA, 0x55, 0x12])
        .expect("sram write");
    assert_eq!(
        target.read_memory(SRAM_BASE, 3).expect("sram read"),
        vec![0xAA, 0x55, 0x12]
    );

    assert!(matches!(
        target.read_memory(0x3000_0000, 1),
        Err(GdbError::Bus(_))
    ));
}

#[test]
fn breakpoints_are_tracked_by_address() {
    let mut target = target();

    target.set_breakpoint(FLASH_XIP_BASE + 2).expect("set");
    assert!(target.has_breakpoint(FLASH_XIP_BASE + 2));
    target.clear_breakpoint(FLASH_XIP_BASE + 2).expect("clear");
    assert!(!target.has_breakpoint(FLASH_XIP_BASE + 2));
}

#[test]
fn single_step_advances_and_reports_sigtrap() {
    let mut target = target();

    let reply = target.resume(ResumeMode::Step).expect("step");
    assert_eq!(reply.reason, StopReason::SingleStep);
    assert_eq!(reply.pc, FLASH_XIP_BASE + 2);
    assert_eq!(reply.signal(), 5, "single step must map to S05");

    let status = target.status();
    assert_eq!(status.stop_reason, Some(StopReason::SingleStep));
    assert_eq!(status.pc, FLASH_XIP_BASE + 2);
    assert_eq!(status.total_cycles, 1);
}

#[test]
fn continue_stops_at_breakpoint_with_s05() {
    let mut target = target();
    target
        .set_breakpoint(FLASH_XIP_BASE + 2)
        .expect("set breakpoint");

    let reply = target.resume(ResumeMode::Continue).expect("continue");
    assert_eq!(reply.reason, StopReason::Breakpoint);
    assert_eq!(reply.pc, FLASH_XIP_BASE + 2);
    assert_eq!(reply.signal(), 5, "breakpoint must map to S05");
    assert_eq!(target.status().stop_reason, Some(StopReason::Breakpoint));
}

#[test]
fn continue_with_zero_budget_reports_instruction_limit() {
    let mut target = target();
    // Zoom to the budget limit: GDB may send `c` many times, but a single
    // zero-budget resume must stop immediately.
    // 直接设定断点预算上限：GDB 可多次发送 `c`，但单次零预算恢复必须立即停止。
    target.instruction_budget = 0;

    let reply = target.resume(ResumeMode::Continue).expect("continue");
    assert_eq!(reply.reason, StopReason::InstructionLimit);
    assert_eq!(reply.signal(), 5);
}

/// SRAM address where the watchpoint test program is installed
/// 监视点测试程序所在的 SRAM 地址
const CODE_BASE: u32 = 0x2000_1000;

/// Install a Thumb program that stores one value into SRAM, then spins
/// 安装一段向 SRAM 写入一个值后自旋的 Thumb 程序
///
/// `ldr r0, [pc, #8]` / `ldr r1, [pc, #12]` / `str[b] r1, [r0]` / `b .`,
/// with the address and value literals at `+0xC` and `+0x10`.
/// 程序为 `ldr r0, [pc, #8]` / `ldr r1, [pc, #12]` / `str[b] r1, [r0]` /
/// `b .`，地址与值常量位于 `+0xC` 与 `+0x10`。
fn install_store(target: &mut MachineTarget, byte: bool, address: u32, value: u32) {
    let store = if byte { 0x7001u16 } else { 0x6001u16 };
    let [low, high] = store.to_le_bytes();
    let mut image = vec![0x02, 0x48, 0x03, 0x49, low, high, 0xFE, 0xE7];
    image.resize(0x0C, 0);
    image.extend_from_slice(&address.to_le_bytes());
    image.extend_from_slice(&value.to_le_bytes());
    target.write_memory(CODE_BASE, &image).expect("install");
    target.write_register(regs::PC, CODE_BASE).expect("pc");
}

/// Install a Thumb program that loads one word from SRAM, then spins
/// 安装一段从 SRAM 读取一个字的 Thumb 程序
///
/// `ldr r0, [pc, #8]` / `ldr r2, [r0]` / `b .`, with the address literal at
/// `+0xC`. / 程序为 `ldr r0, [pc, #8]` / `ldr r2, [r0]` / `b .`，地址常量
/// 位于 `+0xC`。
fn install_load(target: &mut MachineTarget, address: u32) {
    let mut image = vec![0x02, 0x48, 0x02, 0x68, 0xFE, 0xE7];
    image.resize(0x0C, 0);
    image.extend_from_slice(&address.to_le_bytes());
    target.write_memory(CODE_BASE, &image).expect("install");
    target.write_register(regs::PC, CODE_BASE).expect("pc");
}

#[test]
fn write_watchpoint_stops_after_the_store() {
    let mut target = target();
    let steps = target.status().total_cycles;
    target.set_watchpoint(SRAM_BASE, 4).expect("watch");
    assert_eq!(
        target.status().total_cycles,
        steps,
        "snapshot reads must not advance steps / 快照读取不得推进步数"
    );
    install_store(&mut target, false, SRAM_BASE, 0xDEAD_BEEF);

    let reply = target.resume(ResumeMode::Continue).expect("continue");
    assert_eq!(reply.reason, StopReason::Watchpoint);
    assert_eq!(reply.pc, CODE_BASE + 6);
    assert_eq!(reply.signal(), 5);
    assert_eq!(target.watchpoint_hit(), Some(SRAM_BASE));
    assert_eq!(
        target.read_memory(SRAM_BASE, 4).expect("read sram"),
        0xDEAD_BEEFu32.to_le_bytes()
    );
}

#[test]
fn writing_the_same_value_does_not_stop() {
    let mut target = target();
    let value = 0x1122_3344u32;
    target
        .write_memory(SRAM_BASE, &value.to_le_bytes())
        .expect("prime sram");
    target.set_watchpoint(SRAM_BASE, 4).expect("watch");
    install_store(&mut target, false, SRAM_BASE, value);
    target.instruction_budget = 16;

    let reply = target.resume(ResumeMode::Continue).expect("continue");
    assert_eq!(reply.reason, StopReason::InstructionLimit);
    assert_eq!(target.watchpoint_hit(), None);
}

#[test]
fn byte_store_inside_a_four_byte_watchpoint_stops() {
    let mut target = target();
    target.set_watchpoint(SRAM_BASE, 4).expect("watch");
    install_store(&mut target, true, SRAM_BASE + 2, 0x7F);

    let reply = target.resume(ResumeMode::Continue).expect("continue");
    assert_eq!(reply.reason, StopReason::Watchpoint);
    assert_eq!(reply.pc, CODE_BASE + 6);
    assert_eq!(
        target.read_memory(SRAM_BASE + 2, 1).expect("read byte"),
        vec![0x7F]
    );
}

#[test]
fn watchpoints_reject_peripheral_windows() {
    let mut target = target();
    for address in [0x4000_0000u32, 0x5000_0000, 0xE000_ED88] {
        assert!(matches!(
            target.set_watchpoint(address, 4),
            Err(GdbError::Unsupported(_))
        ));
    }
    assert!(matches!(
        target.set_watchpoint(SRAM_END - 2, 4),
        Err(GdbError::Unsupported(_))
    ));
}

#[test]
fn clearing_a_watchpoint_disables_detection() {
    let mut target = target();
    target.set_watchpoint(SRAM_BASE, 4).expect("watch");
    target.clear_watchpoint(SRAM_BASE, 4).expect("clear");
    install_store(&mut target, false, SRAM_BASE, 0xA5A5_5A5A);
    target.instruction_budget = 16;

    let reply = target.resume(ResumeMode::Continue).expect("continue");
    assert_eq!(reply.reason, StopReason::InstructionLimit);
    assert_eq!(
        target.read_memory(SRAM_BASE, 4).expect("read sram"),
        0xA5A5_5A5Au32.to_le_bytes()
    );
}

#[test]
fn read_watchpoint_stops_on_the_load_with_rwatch() {
    let mut target = target();
    install_load(&mut target, SRAM_BASE);
    let mut session = RspSession::new(target);

    assert_eq!(
        session.handle_packet("Z3,20000000,4").as_deref(),
        Some("OK")
    );
    assert_eq!(
        session.handle_packet("c").as_deref(),
        Some("T05rwatch:20000000;thread:1;")
    );
    assert_eq!(session.target().status().pc, CODE_BASE + 4);
}

#[test]
fn read_watchpoint_ignores_writes() {
    let mut target = target();
    install_store(&mut target, false, SRAM_BASE, 0x1234_5678);
    target.set_read_watchpoint(SRAM_BASE, 4).expect("rwatch");
    target.instruction_budget = 16;

    let reply = target.resume(ResumeMode::Continue).expect("continue");
    assert_eq!(reply.reason, StopReason::InstructionLimit);
    assert_eq!(target.watchpoint_hit(), None);
}

#[test]
fn access_watchpoint_stops_on_a_read() {
    let mut target = target();
    install_load(&mut target, SRAM_BASE);
    let mut session = RspSession::new(target);

    assert_eq!(
        session.handle_packet("Z4,20000000,4").as_deref(),
        Some("OK")
    );
    assert_eq!(
        session.handle_packet("c").as_deref(),
        Some("T05awatch:20000000;thread:1;")
    );
}

#[test]
fn access_watchpoint_stops_on_a_write() {
    let mut target = target();
    install_store(&mut target, false, SRAM_BASE, 0xCAFE_F00D);
    let mut session = RspSession::new(target);

    assert_eq!(
        session.handle_packet("Z4,20000000,4").as_deref(),
        Some("OK")
    );
    assert_eq!(
        session.handle_packet("c").as_deref(),
        Some("T05awatch:20000000;thread:1;")
    );
}

#[test]
fn clearing_a_read_watchpoint_disarms_the_core_hook() {
    let mut target = target();
    install_load(&mut target, SRAM_BASE);
    target.set_read_watchpoint(SRAM_BASE, 4).expect("rwatch");
    target.clear_read_watchpoint(SRAM_BASE, 4).expect("clear");
    target.instruction_budget = 16;

    let reply = target.resume(ResumeMode::Continue).expect("continue");
    assert_eq!(reply.reason, StopReason::InstructionLimit);
    assert_eq!(target.watchpoint_hit(), None);
}

#[test]
fn monitor_info_reports_core_and_peripheral_summary() {
    let mut target = target();
    let info = target.monitor_command("info");
    for needle in [
        "core0:",
        "state=",
        "steps=",
        "core1:",
        "dma=",
        "hstx words=",
        "read_watch: off",
    ] {
        assert!(info.contains(needle), "missing {needle:?} in {info:?}");
    }
    target.set_read_watchpoint(SRAM_BASE, 4).expect("rwatch");
    assert!(target.monitor_command("info").contains("read_watch: armed"));
}

#[test]
fn monitor_halt_stops_resumes_and_reset_restores_boot_vectors() {
    let mut target = target();
    let (sp, pc) = target.machine.boot_vectors().expect("boot vectors");
    target.write_register(regs::PC, CODE_BASE).expect("pc");

    assert!(target.monitor_command("halt").contains("halted"));
    assert_eq!(
        target.resume(ResumeMode::Continue).expect("halted").reason,
        StopReason::Halted
    );
    assert_eq!(target.read_register(regs::PC).expect("pc"), CODE_BASE);

    let reset = target.monitor_command("reset");
    assert!(reset.starts_with("reset:"), "{reset:?}");
    assert_eq!(target.read_register(regs::PC).expect("pc"), pc);
    assert_eq!(target.read_register(regs::SP).expect("sp"), sp);
    assert_eq!(
        target.resume(ResumeMode::Step).expect("step").reason,
        StopReason::SingleStep
    );
}

#[test]
fn monitor_unknown_command_is_silently_accepted() {
    let mut target = target();
    assert_eq!(target.monitor_command("bogus"), "");
    let mut session = RspSession::new(target);
    // "bogus" in hex / "bogus" 的十六进制
    assert_eq!(
        session.handle_packet("qRcmd,626f677573").as_deref(),
        Some("OK")
    );
}

#[test]
fn threads_expose_core0_and_core1() {
    let target = target();
    assert_eq!(target.thread_count(), 2);
    assert_eq!(target.thread_id(0), Some(ThreadId::CORE0));
    assert_eq!(target.thread_id(1), Some(ThreadId::CORE1));
    assert_eq!(target.thread_id(2), None);
    assert_eq!(target.thread_name(ThreadId::CORE0), "core0");
    assert_eq!(target.thread_name(ThreadId::CORE1), "core1");
    assert_eq!(
        target.thread_state(ThreadId::CORE0),
        Some(target.machine.cpu_state())
    );
    // core1 在 SIO FIFO 启动前保持 Reset。 / core1 stays Reset until launched.
    assert_eq!(target.thread_state(ThreadId::CORE1), Some(CpuState::Reset));
    assert_eq!(target.thread_state(ThreadId(3)), None);
}

#[test]
fn thread_state_mapping_covers_every_cpu_state() {
    assert_eq!(stop_reason_for_state(CpuState::Reset), StopReason::Reset);
    assert_eq!(
        stop_reason_for_state(CpuState::Running),
        StopReason::SingleStep
    );
    assert_eq!(
        stop_reason_for_state(CpuState::Sleeping),
        StopReason::Sleeping
    );
    assert_eq!(stop_reason_for_state(CpuState::Halted), StopReason::Halted);
    assert_eq!(
        stop_reason_for_state(CpuState::Breakpoint),
        StopReason::Breakpoint
    );
    assert_eq!(stop_reason_for_state(CpuState::Faulted), StopReason::Fault);
}

#[test]
fn core1_registers_are_read_only_and_limited_to_pc_and_sp() {
    let mut target = target();
    let pc = target.machine.core1_pc().expect("core1 pc");
    let sp = target.machine.core1_sp().expect("core1 sp");

    assert_eq!(
        target
            .read_thread_register(ThreadId::CORE1, regs::PC)
            .expect("core1 pc"),
        pc
    );
    assert_eq!(
        target
            .read_thread_register(ThreadId::CORE1, regs::SP)
            .expect("core1 sp"),
        sp
    );
    // 其余 core1 通用寄存器未建模，读为 0。 / Other core1 GPRs read as zero.
    assert_eq!(
        target
            .read_thread_register(ThreadId::CORE1, regs::R0)
            .expect("core1 r0"),
        0
    );
    assert!(matches!(
        target.write_thread_register(ThreadId::CORE1, regs::R0, 1),
        Err(GdbError::Unsupported(_))
    ));
    assert!(matches!(
        target.read_thread_register(ThreadId(3), regs::R0),
        Err(GdbError::InvalidRegister(3))
    ));

    // core0 仍可正常读写。 / core0 stays readable and writable.
    target
        .write_thread_register(ThreadId::CORE0, regs::R0, 0xA5A5_5A5A)
        .expect("core0 write");
    assert_eq!(
        target
            .read_thread_register(ThreadId::CORE0, regs::R0)
            .expect("core0 r0"),
        0xA5A5_5A5A
    );
}

/// SRAM code and counter addresses of the per-core stepping fixture
/// 按核步进夹具使用的 SRAM 代码与计数器地址
const CORE0_CODE: u32 = SRAM_BASE + 0x8400;
const CORE1_CODE: u32 = SRAM_BASE + 0x8800;
const CORE0_COUNTER: u32 = SRAM_BASE + 0x9000;
const CORE1_COUNTER: u32 = SRAM_BASE + 0x9004;
const CORE1_VTOR: u32 = SRAM_BASE + 0x1000;
const CORE1_STACK: u32 = SRAM_BASE + 0xA000;
/// `SIO.FIFO_WR` used by the core-1 launch protocol
/// core1 启动协议使用的 `SIO.FIFO_WR`
const SIO_FIFO_WR: u32 = 0xD000_0054;

/// Thumb `LDR r1,[pc,#8]; LDR r0,[r1]; ADDS r0,#1; STR r0,[r1]; B` + literal
/// Thumb `LDR r1,[pc,#8]; LDR r0,[r1]; ADDS r0,#1; STR r0,[r1]; B` 与常量
///
/// The branch targets the second halfword, so one loop iteration is four
/// instructions and every counter increment is observable in SRAM.
/// 分支目标是第二个半字，因此一次循环为四条指令，且每次自增都能在 SRAM 中看到。
fn counter_image(counter: u32) -> Vec<u8> {
    let mut image = vec![0u8; 0x10];
    for (index, halfword) in [0x4902u16, 0x6808, 0x3001, 0x6008, 0xE7FB]
        .into_iter()
        .enumerate()
    {
        image[index * 2..index * 2 + 2].copy_from_slice(&halfword.to_le_bytes());
    }
    image[0x0C..0x10].copy_from_slice(&counter.to_le_bytes());
    image
}

/// Write the increment loop and its literal through the machine bus
/// 通过机器总线写入自增循环及其常量
///
/// Words go through [`AddressBus::write_u32`] instead of the byte-wise
/// `write_memory`: `SIO.FIFO_WR` only pushes a frame on a 32-bit write, and
/// SRAM code is naturally word-aligned here.
/// 写入使用 [`AddressBus::write_u32`] 而非逐字节的 `write_memory`：
/// `SIO.FIFO_WR` 只在 32 位写入时压入一帧，SRAM 代码本身也按字对齐。
fn install_counter(machine: &mut crate::run::Machine, base: u32, counter: u32) {
    let image = counter_image(counter);
    let mut cycles = 0u64;
    for offset in (0..image.len()).step_by(4) {
        let word = u32::from_le_bytes([
            image[offset],
            image[offset + 1],
            image[offset + 2],
            image[offset + 3],
        ]);
        machine
            .bus
            .write_u32(
                base + u32::try_from(offset).expect("offset"),
                word,
                &mut cycles,
            )
            .expect("code write");
    }
}

/// Target whose core 0 and core 1 both run their own increment loop
/// core0 与 core1 各自运行自增循环的目标
///
/// Core 1 is released through the real SIO FIFO launch stream, which the
/// machine's 64-step peripheral pump consumes, so both cores are `Running`
/// when this returns. The instruction budget is left at 14 — not a multiple of
/// the four-instruction loop — so a `vCont;c` ends with both PCs on a different
/// offset.
/// core1 通过真实的 SIO FIFO 启动流释放，由机器的 64 步外设泵送消费，因此返回时
/// 双核都处于 `Running`。指令预算留在 14（不是四条指令循环的整数倍），使
/// `vCont;c` 结束时两个 PC 都落在不同偏移。
fn dual_core_target() -> MachineTarget {
    let mut target = target();
    install_counter(&mut target.machine, CORE0_CODE, CORE0_COUNTER);
    install_counter(&mut target.machine, CORE1_CODE, CORE1_COUNTER);
    target
        .write_register(regs::PC, CORE0_CODE)
        .expect("core0 pc");
    let mut cycles = 0u64;
    for word in [0u32, 0, 1, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1] {
        target
            .machine
            .bus
            .write_u32(SIO_FIFO_WR, word, &mut cycles)
            .expect("launch word");
    }
    target.instruction_budget = 96;
    let reply = target.resume(ResumeMode::Continue).expect("boot core1");
    assert_eq!(reply.reason, StopReason::InstructionLimit);
    assert_eq!(target.machine.core1_state(), Some(CpuState::Running));
    target.instruction_budget = 14;
    target
}

#[test]
fn stepping_a_reset_core1_reports_thread_two_without_advancing_core0() {
    let mut target = target();
    let steps = target.machine.steps();
    let core0_pc = target.machine.pc();
    let reply = target.step_thread(ThreadId::CORE1).expect("step core1");

    // core1 仍为 Reset，因此上报其真实状态与 PC。
    // core1 is still in Reset, so its real state and PC are reported.
    assert_eq!(reply.reason, StopReason::Reset);
    assert_eq!(reply.pc, target.machine.core1_pc().expect("core1 pc"));
    assert_eq!(target.stopped_thread(), ThreadId::CORE1);
    // 未启动的 core1 不退役指令：core0 的 PC 与步数保持不变。
    // A core 1 that was never released retires nothing: core 0's PC and the
    // step counter stay unchanged.
    assert_eq!(target.machine.pc(), core0_pc);
    assert_eq!(target.machine.steps(), steps);
}

#[test]
fn stepping_core1_advances_only_core1() {
    let mut target = dual_core_target();
    let core0_pc = target.machine.pc();
    let core1_pc = target.machine.core1_pc().expect("core1 pc");
    let steps = target.machine.steps();

    let reply = target.step_thread(ThreadId::CORE1).expect("step core1");

    assert_eq!(reply.reason, StopReason::SingleStep);
    assert_eq!(reply.pc, target.machine.core1_pc().expect("core1 pc"));
    assert_eq!(target.stopped_thread(), ThreadId::CORE1);
    assert_ne!(target.machine.core1_pc(), Some(core1_pc));
    assert_eq!(
        target.machine.pc(),
        core0_pc,
        "core0 must stay frozen / core0 必须冻结"
    );
    assert_eq!(target.machine.steps(), steps + 1);
}

#[test]
fn stepping_core0_advances_only_core0() {
    let mut target = dual_core_target();
    let core0_pc = target.machine.pc();
    let core1_pc = target.machine.core1_pc().expect("core1 pc");

    let reply = target.step_thread(ThreadId::CORE0).expect("step core0");

    assert_eq!(reply.reason, StopReason::SingleStep);
    assert_eq!(reply.pc, target.machine.pc());
    assert_eq!(target.stopped_thread(), ThreadId::CORE0);
    assert_ne!(target.machine.pc(), core0_pc);
    assert_eq!(
        target.machine.core1_pc(),
        Some(core1_pc),
        "core1 must stay frozen / core1 必须冻结"
    );
}

#[test]
fn continuing_only_core1_leaves_core0_frozen() {
    let mut target = dual_core_target();
    target.instruction_budget = 10;
    let core0_pc = target.machine.pc();
    let core1_pc = target.machine.core1_pc().expect("core1 pc");

    let reply = target
        .continue_thread(ThreadId::CORE1)
        .expect("continue core1");

    assert_eq!(reply.reason, StopReason::InstructionLimit);
    assert_eq!(reply.pc, target.machine.core1_pc().expect("core1 pc"));
    assert_ne!(target.machine.core1_pc(), Some(core1_pc));
    assert_eq!(
        target.machine.pc(),
        core0_pc,
        "core0 must stay frozen / core0 必须冻结"
    );
}

#[test]
fn continuing_core0_reports_thread_one() {
    let mut target = target();
    target.instruction_budget = 1;
    let reply = target.continue_thread(ThreadId::CORE0).expect("continue");

    assert_eq!(reply.reason, StopReason::InstructionLimit);
    assert_eq!(reply.pc, target.machine.pc());
    assert_eq!(target.stopped_thread(), ThreadId::CORE0);
}

#[test]
fn breakpoint_thread_identifies_each_core() {
    let mut target = target();
    let all = (true, true);
    let core1_pc = target.machine.core1_pc().expect("core1 pc");
    target.set_breakpoint(core1_pc).expect("core1 breakpoint");
    assert_eq!(target.breakpoint_thread_in(all), Some(ThreadId::CORE1));

    let core0_pc = target.machine.pc();
    target.set_breakpoint(core0_pc).expect("core0 breakpoint");
    // core0 优先检查。 / core0 is checked first.
    assert_eq!(target.breakpoint_thread_in(all), Some(ThreadId::CORE0));

    target.clear_breakpoint(core0_pc).expect("clear core0");
    target.clear_breakpoint(core1_pc).expect("clear core1");
    assert_eq!(target.breakpoint_thread_in(all), None);
    // 未选中的核心不参与断点检查。 / Unselected cores are not inspected.
    target.set_breakpoint(core0_pc).expect("core0 breakpoint");
    assert_eq!(target.breakpoint_thread_in((false, true)), None);
}

#[test]
fn rsp_thread_enumeration_and_selection_drive_core1_registers() {
    let target = target();
    let pc = target.machine.core1_pc().expect("core1 pc");
    let mut session = RspSession::new(target);

    assert_eq!(
        session.handle_packet("qfThreadInfo").as_deref(),
        Some("m1,2")
    );
    assert_eq!(session.handle_packet("qsThreadInfo").as_deref(), Some("l"));
    assert_eq!(session.handle_packet("qC").as_deref(), Some("QC1"));
    assert_eq!(session.handle_packet("Hg2").as_deref(), Some("OK"));
    assert_eq!(session.handle_packet("qC").as_deref(), Some("QC2"));
    assert_eq!(session.handle_packet("T2").as_deref(), Some("OK"));
    assert_eq!(session.handle_packet("T3").as_deref(), Some("E01"));
    let expected = encode_hex(&pc.to_le_bytes());
    assert_eq!(
        session.handle_packet("pf").as_deref(),
        Some(expected.as_str())
    );
}

#[test]
fn rsp_vcont_step_core1_advances_only_core1() {
    let target = dual_core_target();
    let core0_pc = target.machine.pc();
    let core1_pc = target.machine.core1_pc().expect("core1 pc");
    let mut session = RspSession::new(target);

    assert_eq!(
        session.handle_packet("vCont;s:2").as_deref(),
        Some("T05thread:2;")
    );
    assert_eq!(session.target().stopped_thread(), ThreadId::CORE1);
    assert_ne!(session.target().machine.core1_pc(), Some(core1_pc));
    assert_eq!(session.target().machine.pc(), core0_pc);
    // `Hg` 未选择时 `pf` 跟随最近停止线程，即 core1 的 PC。
    // Without an `Hg` selection, `pf` follows the last stopped thread (core 1).
    let expected = encode_hex(
        &session
            .target()
            .machine
            .core1_pc()
            .expect("core1 pc")
            .to_le_bytes(),
    );
    assert_eq!(
        session.handle_packet("pf").as_deref(),
        Some(expected.as_str())
    );
}

#[test]
fn rsp_vcont_step_core0_advances_only_core0() {
    let target = dual_core_target();
    let core0_pc = target.machine.pc();
    let core1_pc = target.machine.core1_pc().expect("core1 pc");
    let mut session = RspSession::new(target);

    assert_eq!(
        session.handle_packet("vCont;s:1").as_deref(),
        Some("T05thread:1;")
    );
    assert_eq!(session.target().stopped_thread(), ThreadId::CORE0);
    assert_ne!(session.target().machine.pc(), core0_pc);
    assert_eq!(session.target().machine.core1_pc(), Some(core1_pc));
}

#[test]
fn rsp_vcont_continue_advances_both_cores() {
    let target = dual_core_target();
    let core0_pc = target.machine.pc();
    let core1_pc = target.machine.core1_pc().expect("core1 pc");
    let steps = target.machine.steps();
    let mut session = RspSession::new(target);

    assert_eq!(
        session.handle_packet("vCont;c").as_deref(),
        Some("T05thread:1;")
    );
    assert_eq!(session.target().stopped_thread(), ThreadId::CORE0);
    assert!(session.target().machine.steps() > steps);
    assert_ne!(session.target().machine.pc(), core0_pc);
    assert_ne!(
        session.target().machine.core1_pc(),
        Some(core1_pc),
        "core1 must advance too / core1 也必须推进"
    );
}

#[test]
fn rsp_vcont_continue_all_is_equivalent_to_plain_continue() {
    let mut target = target();
    target
        .set_breakpoint(FLASH_XIP_BASE + 2)
        .expect("breakpoint");
    let mut session = RspSession::new(target);

    assert_eq!(
        session.handle_packet("vCont;c:1;c:2").as_deref(),
        Some("T05thread:1;")
    );
    assert_eq!(session.target().stopped_thread(), ThreadId::CORE0);
}

#[test]
fn rsp_vcont_mixed_step_and_continue_advances_only_core1() {
    let target = dual_core_target();
    let core0_pc = target.machine.pc();
    let core1_pc = target.machine.core1_pc().expect("core1 pc");
    let steps = target.machine.steps();
    let mut session = RspSession::new(target);

    // 真实 GDB 在 all-stop 模式下裸 `stepi` 就发送 `vCont;s:2;c`。
    // Real GDB sends `vCont;s:2;c` for a bare `stepi` in all-stop mode.
    assert_eq!(
        session.handle_packet("vCont;s:2;c").as_deref(),
        Some("T05thread:2;")
    );
    assert_eq!(session.target().stopped_thread(), ThreadId::CORE1);
    assert_ne!(session.target().machine.core1_pc(), Some(core1_pc));
    assert_eq!(
        session.target().machine.pc(),
        core0_pc,
        "the continued core must stay stopped / 被继续的 core0 必须保持停止"
    );
    assert_eq!(session.target().machine.steps(), steps + 1);
}

#[test]
fn rsp_vcont_continue_then_step_keeps_the_continued_core_frozen() {
    let target = dual_core_target();
    let core0_pc = target.machine.pc();
    let core1_pc = target.machine.core1_pc().expect("core1 pc");
    let mut session = RspSession::new(target);

    assert_eq!(
        session.handle_packet("vCont;c:1;s:2").as_deref(),
        Some("T05thread:2;")
    );
    assert_ne!(session.target().machine.core1_pc(), Some(core1_pc));
    assert_eq!(session.target().machine.pc(), core0_pc);
}

#[test]
fn rsp_vcont_two_step_actions_advance_both_cores_once() {
    let target = dual_core_target();
    let core0_pc = target.machine.pc();
    let core1_pc = target.machine.core1_pc().expect("core1 pc");
    let steps = target.machine.steps();
    let mut session = RspSession::new(target);

    assert_eq!(
        session.handle_packet("vCont;s:1;s:2").as_deref(),
        Some("T05thread:2;")
    );
    assert_eq!(session.target().stopped_thread(), ThreadId::CORE1);
    assert_ne!(session.target().machine.pc(), core0_pc);
    assert_ne!(session.target().machine.core1_pc(), Some(core1_pc));
    assert_eq!(
        session.target().machine.steps(),
        steps + 2,
        "each step action retires exactly one instruction per core"
    );
}

#[test]
fn rsp_vcont_stop_actions_never_advance_a_core() {
    let target = dual_core_target();
    let core0_pc = target.machine.pc();
    let core1_pc = target.machine.core1_pc().expect("core1 pc");
    let steps = target.machine.steps();
    let mut session = RspSession::new(target);

    assert_eq!(
        session.handle_packet("vCont;t").as_deref(),
        Some("T02thread:1;")
    );
    assert_eq!(
        session.handle_packet("vCont;t:2").as_deref(),
        Some("T02thread:2;")
    );
    assert_eq!(session.target().machine.steps(), steps);
    assert_eq!(session.target().machine.pc(), core0_pc);
    assert_eq!(session.target().machine.core1_pc(), Some(core1_pc));
}

#[test]
fn rsp_vcont_rejects_duplicate_and_out_of_range_actions() {
    let target = dual_core_target();
    let steps = target.machine.steps();
    let mut session = RspSession::new(target);

    for packet in [
        "vCont;c:1;c:1",
        "vCont;s:2;c:2",
        "vCont;s:3",
        "vCont;x",
        "vCont;c;s",
        "vCont;",
        "vCont;t;c",
    ] {
        assert_eq!(
            session.handle_packet(packet).as_deref(),
            Some("E01"),
            "{packet}"
        );
    }
    // 非法列表不得推进任何一个核心。 / Malformed lists never advance a core.
    assert_eq!(session.target().machine.steps(), steps);
}

/// Single-core target built from a RISC-V UF2 image / 由 RISC-V UF2 映像构造的单核目标
///
/// RISC-V builds have no core 1, so the thread table stays `m1` and any `:2`
/// action must be rejected. The image carries the legacy boot2 header (magic,
/// entry point and initial SP) plus one `c.nop`, so the core boots without any
/// core-1 launch stream.
/// RISC-V 构建没有 core1，线程表保持 `m1`，任何 `:2` 动作都必须被拒绝。映像带旧式
/// boot2 头（魔数、入口点与初始 SP）以及一条 `c.nop`，因此核心启动不依赖 core1
/// 启动流。
fn riscv_target() -> MachineTarget {
    /// Size of the boot2 header plus one `c.nop`
    /// boot2 头加一条 `c.nop` 的长度
    const IMAGE_LEN: usize = 0x30;
    let mut image = vec![0u8; IMAGE_LEN];
    image[0x00..0x04].copy_from_slice(&0x7188_EBF2u32.to_le_bytes());
    image[0x04..0x08].copy_from_slice(&(FLASH_XIP_BASE + 0x28).to_le_bytes());
    image[0x24..0x28].copy_from_slice(&(SRAM_BASE + 0x1000).to_le_bytes());
    image[0x28..0x2C].copy_from_slice(&1u32.to_le_bytes());

    let mut block = vec![0u8; 512];
    block[0..4].copy_from_slice(&0x0A32_4655u32.to_le_bytes());
    block[4..8].copy_from_slice(&0x9E5D_5157u32.to_le_bytes());
    // Family-ID-present flag. / 置位 family ID 存在标志。
    block[8..12].copy_from_slice(&0x0000_2000u32.to_le_bytes());
    block[12..16].copy_from_slice(&FLASH_XIP_BASE.to_le_bytes());
    block[16..20].copy_from_slice(&u32::try_from(IMAGE_LEN).expect("length").to_le_bytes());
    block[24..28].copy_from_slice(&1u32.to_le_bytes());
    block[28..32].copy_from_slice(&0xE48B_FF5Au32.to_le_bytes());
    block[32..32 + IMAGE_LEN].copy_from_slice(&image);
    block[508..512].copy_from_slice(&0x0AB1_6F30u32.to_le_bytes());

    let options = MachineOptions {
        arch: None,
        max_cycles: u64::MAX,
        strict_decode: false,
        psram: None,
    };
    MachineTarget::new(Machine::new(&block, &options).expect("riscv machine"))
}

#[test]
fn gdb_options_forward_the_psram_window_to_the_machine() {
    // `gdb` 路径把 `--psram` 原样转给机器，同时保持「GDB 自己决定停机」的预算。
    // The `gdb` path forwards `--psram` unchanged while keeping GDB's `u64::MAX`
    // stop budget.
    let without = machine_options(&GdbOptions {
        file: std::path::PathBuf::from("fw.elf"),
        port: 3333,
        psram: None,
    });
    assert_eq!(without.max_cycles, u64::MAX);
    assert!(without.psram.is_none());

    let sized = MachineOptions {
        psram: Some(PsramOptions {
            size_bytes: 8 * 1024 * 1024,
        }),
        ..without
    };
    let forwarded = machine_options(&GdbOptions {
        file: std::path::PathBuf::from("fw.elf"),
        port: 3333,
        psram: sized.psram,
    });
    assert_eq!(forwarded.psram, sized.psram);

    // 真实机器：带该选项时 `0x1100_0000` 可读写，不带时未映射。
    // Real machine: the window serves accesses with the option, unmapped without.
    let mut machine = Machine::new(&cpacr_write_elf(), &sized).expect("machine");
    let mut cycles = 0u64;
    machine
        .bus
        .write_u8(0x1100_0000, 0x42, &mut cycles)
        .expect("psram write");
    assert_eq!(
        machine.bus.read_u8(0x1100_0000, &mut cycles).expect("read"),
        0x42
    );
}

#[test]
fn rsp_vcont_rejects_thread_two_on_a_single_core_target() {
    let target = riscv_target();
    assert_eq!(target.thread_count(), 1);
    let mut session = RspSession::new(target);
    assert_eq!(session.handle_packet("qfThreadInfo").as_deref(), Some("m1"));
    for packet in ["vCont;s:2", "vCont;c:2", "vCont;t:2", "vCont;c:1;c:2"] {
        assert_eq!(
            session.handle_packet(packet).as_deref(),
            Some("E01"),
            "{packet}"
        );
    }
    assert_eq!(
        session.handle_packet("vCont;s").as_deref(),
        Some("T05thread:1;")
    );
}

#[test]
fn cont_dispatches_a_thread_action_list_through_the_machine() {
    let mut target = dual_core_target();
    let core0_pc = target.machine.pc();
    let core1_pc = target.machine.core1_pc().expect("core1 pc");
    let reply = target
        .cont(&[
            rp2350_gdb::ThreadAction {
                thread: Some(ThreadId::CORE1),
                mode: ResumeMode::Step,
            },
            rp2350_gdb::ThreadAction {
                thread: None,
                mode: ResumeMode::Continue,
            },
        ])
        .expect("cont");

    assert_eq!(reply.reason, StopReason::SingleStep);
    assert_eq!(reply.pc, target.machine.core1_pc().expect("core1 pc"));
    assert_eq!(target.stopped_thread(), ThreadId::CORE1);
    assert_ne!(target.machine.core1_pc(), Some(core1_pc));
    assert_eq!(target.machine.pc(), core0_pc);
}

/// One TCP round trip against the real server and machine target
/// 针对真实服务器与机器目标的一次 TCP 往返
#[test]
fn tcp_session_drives_the_machine_over_rsp() {
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).expect("bind");
    let addr = listener.local_addr().expect("listener address");
    let handle = thread::spawn(move || {
        let mut server = GdbServer::new(addr);
        server.serve_with(&listener, Box::new(target()))
    });

    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .expect("write timeout");

    let supported = transact(&mut stream, "qSupported:multiprocess+");
    assert!(supported.contains("PacketSize="));
    assert_eq!(transact(&mut stream, "?"), "T05thread:1;");
    assert_eq!(transact(&mut stream, "m10000000,4"), "03480449");
    assert_eq!(transact(&mut stream, "Z0,10000002,2"), "OK");
    assert_eq!(transact(&mut stream, "c"), "T05thread:1;");
    assert_eq!(transact(&mut stream, "pf"), "02000010");

    drop(stream);
    handle
        .join()
        .expect("server thread panicked")
        .expect("server failed");
}

/// Send one payload and return the decoded reply packet
/// 发送一个载荷并返回解码后的回复包
fn transact(stream: &mut TcpStream, payload: &str) -> String {
    stream
        .write_all(encode_packet(payload).as_bytes())
        .expect("write frame");
    assert_eq!(read_byte(stream), b'+', "expected positive acknowledgement");

    // 应答后先定位帧起始 `$`，再读到 `#` 与两位校验和。
    // After the acknowledgement, locate the `$` frame start before reading
    // through `#` and the two checksum digits.
    while read_byte(stream) != b'$' {}
    let mut frame = vec![b'$'];
    loop {
        let byte = read_byte(stream);
        frame.push(byte);
        if byte == b'#' {
            frame.push(read_byte(stream));
            frame.push(read_byte(stream));
            break;
        }
    }
    decode_packet(&frame).expect("valid reply packet")
}

/// Read one byte from the client socket / 从客户端 socket 读取一个字节
fn read_byte(stream: &mut TcpStream) -> u8 {
    let mut byte = [0u8; 1];
    stream.read_exact(&mut byte).expect("read byte");
    byte[0]
}
