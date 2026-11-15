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
use rp2350_gdb::{RspSession, decode_packet, encode_packet};

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
        Some("T05rwatch:20000000;")
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
        Some("T05awatch:20000000;")
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
        Some("T05awatch:20000000;")
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
    assert_eq!(transact(&mut stream, "?"), "S05");
    assert_eq!(transact(&mut stream, "m10000000,4"), "03480449");
    assert_eq!(transact(&mut stream, "Z0,10000002,2"), "OK");
    assert_eq!(transact(&mut stream, "c"), "S05");
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
