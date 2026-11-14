//! GDB target adapter backed by the CLI [`Machine`]
//! 由 CLI [`Machine`] 支撑的 GDB 目标适配器
//!
//! `rp2350-gdb` exposes the transport and protocol layers only; the machine
//! that owns the bus, boot ROM, peripherals and CPU lives in [`crate::run`].
//! [`MachineTarget`] bridges the two: registers and memory go through the
//! machine's public accessors, while `resume` uses [`Machine::step`] so boot
//! ROM traps, interrupt pumping and the peripheral timebase stay active.
//! `rp2350-gdb` 只提供传输与协议层；持有总线、boot ROM、外设与 CPU 的机器位于
//! [`crate::run`]。[`MachineTarget`] 负责桥接：寄存器与内存走机器的公共访问器，
//! `resume` 使用 [`Machine::step`]，使 boot ROM 陷阱、中断泵送与外设时基保持
//! 有效。

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use rp2350_core::SimpleBus;
use rp2350_core::bus::AddressBus;
use rp2350_cpu::CpuState;
use rp2350_gdb::target::regs;
use rp2350_gdb::{GdbError, GdbServer, GdbTarget, ResumeMode, StopReason, StopReply, TargetStatus};

use crate::run::{Machine, MachineOptions};

/// Default instruction budget for one GDB `Continue`
/// 单次 GDB `Continue` 的默认指令预算
///
/// The limit keeps an accidental infinite loop from blocking the RSP server
/// forever; a client can simply issue `c` again to keep running.
/// 该上限防止意外无限循环永久阻塞 RSP 服务器；客户端再次发送 `c` 即可继续运行。
pub const DEFAULT_CONTINUE_BUDGET: u64 = 1_000_000;

/// Start of the SRAM window (`0x2000_0000`) / SRAM 窗口起始地址（`0x2000_0000`）
const SRAM_START: u32 = 0x2000_0000;
/// End of the 520 KiB SRAM window (`0x2008_2000`) / 520 KiB SRAM 窗口结束地址（`0x2008_2000`）
const SRAM_END: u32 = 0x2008_2000;

/// Whether reads of `length` bytes at `address` are free of side effects
/// `address` 起 `length` 字节的读取是否无副作用
///
/// Watchpoints sample memory on every executed instruction, so only SRAM is
/// accepted. Peripheral and USB windows are rejected: reading them can pop
/// FIFOs or advance device state, and the machine step counter must stay
/// untouched. Flash is not accepted either because guests cannot write it.
/// 监视点会在每条指令后采样内存，因此只接受 SRAM。外设与 USB 窗口被拒绝：读取
/// 它们可能弹出 FIFO 或推进设备状态，而机器步数计数必须保持不变。Flash 也不
/// 接受，因为客户机无法写入 Flash。
fn watchable(address: u32, length: usize) -> bool {
    let Ok(length) = u32::try_from(length) else {
        return false;
    };
    let Some(last) = address.checked_add(length.saturating_sub(1)) else {
        return false;
    };
    address >= SRAM_START && last < SRAM_END
}

/// Read `length` bytes without advancing the machine timeline
/// 读取 `length` 字节且不推进机器时间线
///
/// The local `cycles` counter absorbs the bus latency accounting, so snapshot
/// reads never change [`Machine::steps`] or the peripheral timebase.
/// 局部 `cycles` 计数器吸收总线延迟记账，因此快照读取不会改变
/// [`Machine::steps`] 或外设时基。
fn read_watch_bytes(bus: &mut SimpleBus, address: u32, length: usize) -> Result<Vec<u8>, GdbError> {
    let mut cycles = 0u64;
    let mut bytes = Vec::with_capacity(length);
    for offset in 0..length {
        let offset = u32::try_from(offset).map_err(|_| GdbError::AddressOutOfRange(address))?;
        let addr = address
            .checked_add(offset)
            .ok_or(GdbError::AddressOutOfRange(address))?;
        bytes.push(bus.read_u8(addr, &mut cycles)?);
    }
    Ok(bytes)
}

/// Options for the `gdb` subcommand / `gdb` 子命令选项
#[derive(Debug)]
pub struct GdbOptions {
    /// Firmware file (ELF) / 固件文件（ELF）
    pub file: PathBuf,
    /// GDB listen port / GDB 监听端口
    pub port: u16,
}

/// Debug target backed by a booted [`Machine`] / 由已启动 [`Machine`] 支撑的调试目标
///
/// Breakpoints are detected by PC comparison (no `BKPT` patching), matching the
/// library's `CortexM33Target` behaviour: resuming from a breakpoint first steps
/// over the instruction at the current PC.
/// 断点通过 PC 比较检测（不写入 `BKPT`），与库中的 `CortexM33Target` 一致：
/// 从断点恢复执行时会先跨过当前 PC 处的指令。
pub struct MachineTarget {
    /// Machine under debug / 被调试的机器
    machine: Machine,
    /// Instruction breakpoints / 指令断点
    breakpoints: BTreeSet<u32>,
    /// Write watchpoints with their last sampled bytes, keyed by `(address, length)`
    /// 写监视点及其最近一次采样字节，以 `(地址, 长度)` 为键
    watchpoints: BTreeMap<(u32, usize), Vec<u8>>,
    /// Watchpoint address that caused the latest stop / 最近一次停止命中的监视点地址
    last_watch: Option<u32>,
    /// Reason recorded by the most recent resume / 最近一次恢复执行记录的原因
    last_stop: Option<StopReason>,
    /// Instruction budget for a single `Continue` / 单次 `Continue` 的指令预算
    instruction_budget: u64,
}

impl MachineTarget {
    /// Wrap a booted machine into a debug target / 将已启动的机器封装为调试目标
    #[must_use]
    pub fn new(machine: Machine) -> Self {
        Self {
            machine,
            breakpoints: BTreeSet::new(),
            watchpoints: BTreeMap::new(),
            last_watch: None,
            last_stop: None,
            instruction_budget: DEFAULT_CONTINUE_BUDGET,
        }
    }

    /// Sample every watchpoint and return the first address whose bytes changed
    /// 采样所有监视点并返回首个字节发生变化的地址
    ///
    /// The stored snapshot is refreshed on a hit so a later `continue` does not
    /// retrigger on the same value. Bus reads use a local cycle counter, so the
    /// machine step counter is untouched.
    /// 命中后刷新已存快照，使后续 `continue` 不会因同一值再次触发。总线读取使用
    /// 局部周期计数器，不影响机器的步数计数。
    fn changed_watchpoint(&mut self) -> Option<u32> {
        let bus = &mut self.machine.bus;
        let mut hit = None;
        for (&(address, length), snapshot) in &mut self.watchpoints {
            let Ok(observed) = read_watch_bytes(bus, address, length) else {
                continue;
            };
            if observed != *snapshot {
                *snapshot = observed;
                if hit.is_none() {
                    hit = Some(address);
                }
            }
        }
        hit
    }

    /// Re-sample every watchpoint to establish the pre-step baseline
    /// 重新采样所有监视点以建立步前基线
    fn refresh_watchpoints(&mut self) {
        let bus = &mut self.machine.bus;
        for (&(address, length), snapshot) in &mut self.watchpoints {
            if let Ok(observed) = read_watch_bytes(bus, address, length) {
                *snapshot = observed;
            }
        }
    }

    /// Execute one instruction and map the result to a stop reason
    /// 执行一条指令并将结果映射为停止原因
    fn single_step(&mut self) -> StopReply {
        self.refresh_watchpoints();
        match self.machine.step() {
            Ok(CpuState::Running) => {
                if let Some(address) = self.changed_watchpoint() {
                    self.last_watch = Some(address);
                    return self.stop_reply(StopReason::Watchpoint);
                }
                self.stop_reply(StopReason::SingleStep)
            }
            Ok(state) => self.stop_reply(self.reason_for_state(state)),
            Err(error) => {
                log::debug!("GDB single step failed: {error:?}");
                self.stop_reply(StopReason::Fault)
            }
        }
    }

    /// Run until a breakpoint, watchpoint, non-running state, or budget limit
    /// 运行直到断点、监视点、非运行状态或预算耗尽
    fn continue_until_stop(&mut self) -> StopReply {
        let mut budget = self.instruction_budget;
        // 首次迭代跳过 PC 检查，使从断点恢复执行时跨过当前指令，而不是立即
        // 再次停下。
        // The first iteration skips the PC check so resuming from a breakpoint
        // steps over the current instruction instead of stopping again.
        let mut resume_from_breakpoint = true;

        loop {
            let pc = self.machine.pc();
            if !resume_from_breakpoint && self.breakpoints.contains(&pc) {
                return self.stop_reply(StopReason::Breakpoint);
            }
            resume_from_breakpoint = false;

            if budget == 0 || self.machine.budget_exhausted() {
                return self.stop_reply(StopReason::InstructionLimit);
            }
            budget -= 1;

            // Establish the pre-step baseline, then compare after the step so a
            // store anywhere inside the watched range is detected.
            // 先建立步前基线，步后比较，从而检测监视范围内的任意写入。
            self.refresh_watchpoints();
            match self.machine.step() {
                Ok(CpuState::Running) => {
                    if let Some(address) = self.changed_watchpoint() {
                        self.last_watch = Some(address);
                        return self.stop_reply(StopReason::Watchpoint);
                    }
                }
                Ok(state) => return self.stop_reply(self.reason_for_state(state)),
                Err(error) => {
                    log::debug!("GDB continue stopped by error: {error:?}");
                    return self.stop_reply(StopReason::Fault);
                }
            }
        }
    }

    /// Map a core state to a stop reason, preferring the machine budget
    /// 将核心状态映射为停止原因，优先报告机器预算耗尽
    fn reason_for_state(&self, state: CpuState) -> StopReason {
        if self.machine.budget_exhausted() {
            StopReason::InstructionLimit
        } else {
            stop_reason_for_state(state)
        }
    }

    /// Build a stop reply from the current PC / 根据当前 PC 构造停止回复
    fn stop_reply(&self, reason: StopReason) -> StopReply {
        StopReply {
            reason,
            pc: self.machine.pc(),
        }
    }
}

/// Map a core state returned by `step` to a stop reason
/// 将 `step` 返回的核心状态映射为停止原因
fn stop_reason_for_state(state: CpuState) -> StopReason {
    match state {
        CpuState::Running => StopReason::SingleStep,
        CpuState::Breakpoint => StopReason::Breakpoint,
        CpuState::Sleeping => StopReason::Sleeping,
        CpuState::Faulted => StopReason::Fault,
        CpuState::Halted => StopReason::Halted,
        CpuState::Reset => StopReason::Reset,
    }
}

impl GdbTarget for MachineTarget {
    fn read_register(&mut self, register: u32) -> Result<u32, GdbError> {
        match register {
            regs::R0..=regs::PC => Ok(self
                .machine
                .read_register(u8::try_from(register).unwrap_or_default())),
            regs::XPSR => Ok(self.machine.xpsr()),
            _ => Err(GdbError::InvalidRegister(register)),
        }
    }

    fn write_register(&mut self, register: u32, value: u32) -> Result<(), GdbError> {
        match register {
            regs::R0..=regs::PC => {
                self.machine
                    .write_register(u8::try_from(register).unwrap_or_default(), value);
                Ok(())
            }
            regs::XPSR => {
                // RISC-V cores have no XPSR: report the write as unsupported
                // instead of silently dropping it.
                // RISC-V 核心没有 XPSR：把写入报告为不支持，而不是静默丢弃。
                if self.machine.write_xpsr(value) {
                    Ok(())
                } else {
                    Err(GdbError::InvalidRegister(register))
                }
            }
            _ => Err(GdbError::InvalidRegister(register)),
        }
    }

    fn read_memory(&mut self, address: u32, length: usize) -> Result<Vec<u8>, GdbError> {
        let mut bytes = Vec::with_capacity(length);
        let mut cycles = 0u64;

        for offset in 0..length {
            let offset = u32::try_from(offset).map_err(|_| GdbError::AddressOutOfRange(address))?;
            let addr = address
                .checked_add(offset)
                .ok_or(GdbError::AddressOutOfRange(address))?;
            bytes.push(self.machine.bus.read_u8(addr, &mut cycles)?);
        }

        Ok(bytes)
    }

    fn write_memory(&mut self, address: u32, bytes: &[u8]) -> Result<(), GdbError> {
        let mut cycles = 0u64;

        for (offset, byte) in bytes.iter().copied().enumerate() {
            let offset = u32::try_from(offset).map_err(|_| GdbError::AddressOutOfRange(address))?;
            let addr = address
                .checked_add(offset)
                .ok_or(GdbError::AddressOutOfRange(address))?;
            self.machine.bus.write_u8(addr, byte, &mut cycles)?;
        }

        Ok(())
    }

    fn set_breakpoint(&mut self, address: u32) -> Result<(), GdbError> {
        self.breakpoints.insert(address);
        Ok(())
    }

    fn clear_breakpoint(&mut self, address: u32) -> Result<(), GdbError> {
        self.breakpoints.remove(&address);
        Ok(())
    }

    fn has_breakpoint(&self, address: u32) -> bool {
        self.breakpoints.contains(&address)
    }

    fn supports_watchpoints(&self) -> bool {
        true
    }

    fn set_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        if !watchable(address, length) {
            return Err(GdbError::Unsupported("watchpoints need SRAM windows"));
        }
        let snapshot = read_watch_bytes(&mut self.machine.bus, address, length)?;
        self.watchpoints.insert((address, length), snapshot);
        Ok(())
    }

    fn clear_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        self.watchpoints.remove(&(address, length));
        Ok(())
    }

    fn watchpoint_hit(&self) -> Option<u32> {
        self.last_watch
    }

    fn resume(&mut self, mode: ResumeMode) -> Result<StopReply, GdbError> {
        self.last_watch = None;
        let reply = match mode {
            ResumeMode::Step => self.single_step(),
            ResumeMode::Continue => self.continue_until_stop(),
        };
        self.last_stop = Some(reply.reason);
        Ok(reply)
    }

    fn status(&self) -> TargetStatus {
        TargetStatus {
            state: self.machine.cpu_state(),
            stop_reason: self.last_stop,
            pc: self.machine.pc(),
            // Steps double as simulated cycles in the machine timeline.
            // 在机器时间线中步数同时充当模拟周期。
            total_cycles: self.machine.steps(),
        }
    }
}

/// Load the firmware, boot a machine and serve GDB clients until the listener fails
/// 加载固件、启动机器并提供 GDB 客户端服务，直到监听器出错
///
/// # Errors
/// Returns an error when the firmware cannot be read, parsed or booted, or when
/// the TCP listener fails.
/// 固件无法读取、解析或启动，或 TCP 监听器失败时返回错误。
pub fn run(options: &GdbOptions) -> Result<()> {
    let bytes = std::fs::read(&options.file).with_context(|| {
        format!(
            "cannot read firmware / 无法读取固件: {}",
            options.file.display()
        )
    })?;

    // `u64::MAX` keeps the machine from halting on its own budget: GDB decides
    // when to stop, and each `Continue` has its own instruction cap.
    // `u64::MAX` 避免机器因自身预算停机：由 GDB 决定何时停止，单次 `Continue`
    // 另有指令上限。
    let machine_options = MachineOptions {
        arch: None,
        max_cycles: u64::MAX,
        strict_decode: false,
    };
    let machine = Machine::new(&bytes, &machine_options)?;

    println!("[gdb] {}", machine.image_description());
    println!("[gdb] core: {}", machine.core_description());
    println!(
        "[gdb] started: PC={:#010X} SP={:#010X}",
        machine.pc(),
        machine.sp()
    );

    let addr = SocketAddr::from(([0, 0, 0, 0], options.port));
    let mut server = GdbServer::new(addr);
    // The address is exact for every non-zero port; `GdbServer` logs the actual
    // bound address once the listener exists.
    // 除端口 0 外该地址即实际地址；监听器建立后 `GdbServer` 会记录实际绑定地址。
    println!("[gdb] listening on {} (Ctrl-C to stop)", server.addr());

    let result = server.serve(Box::new(MachineTarget::new(machine)));
    println!(
        "[gdb] server stopped (last listen address {})",
        server.addr()
    );
    result.context("GDB server failed / GDB 服务器失败")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::Duration;

    use rp2350_core::FLASH_XIP_BASE;
    use rp2350_gdb::{decode_packet, encode_packet};

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
}
