//! Cortex-M33 adapter for the GDB target abstraction
//! GDB 目标抽象的 Cortex-M33 适配器
//!
//! `CortexM33Target` owns a [`CortexM33Core`] and its [`AddressBus`], so the
//! GDB server can read/write registers and memory, resume or single-step, and
//! stop at instruction breakpoints without touching core internals.
//! `CortexM33Target` 持有 [`CortexM33Core`] 及其 [`AddressBus`]，使 GDB 服务器
//! 无需接触核心内部即可读写寄存器与内存、继续/单步执行，并在指令断点处停止。

use std::collections::BTreeSet;

use rp2350_arm::CortexM33Core;
use rp2350_core::bus::AddressBus;
use rp2350_cpu::{CpuCore, CpuState};

use crate::target::{GdbError, GdbTarget, ResumeMode, StopReason, StopReply, TargetStatus, regs};

/// Default number of instructions a single `Continue` may execute
/// 单次 `Continue` 默认允许执行的最大指令数
///
/// The limit keeps an accidental infinite loop (for example `B .`) from
/// blocking the RSP server forever; callers can override it per target.
/// 该上限防止意外的无限循环（例如 `B .`）永久阻塞 RSP 服务器；调用方可在
/// 每个目标上覆盖此值。
pub const DEFAULT_INSTRUCTION_BUDGET: u64 = 1_000_000;

/// Cortex-M33 debug target backed by a core and an address bus
/// 由核心与地址总线支撑的 Cortex-M33 调试目标
pub struct CortexM33Target<B: AddressBus> {
    /// CPU core under debug / 被调试的 CPU 核心
    core: CortexM33Core,
    /// Address bus used for instruction fetches and memory packets
    /// 用于取指与内存包的地址总线
    bus: B,
    /// Instruction breakpoints detected by PC comparison
    /// 通过 PC 比较检测的指令断点
    breakpoints: BTreeSet<u32>,
    /// Reason recorded by the most recent resume / 最近一次恢复执行记录的原因
    last_stop: Option<StopReason>,
    /// Instruction budget for a single `Continue` / 单次 `Continue` 的指令预算
    instruction_budget: u64,
}

impl<B: AddressBus> CortexM33Target<B> {
    /// Wrap a booted core and its bus into a debug target
    /// 将已启动的核心及其总线封装为调试目标
    ///
    /// The core keeps its current execution state; callers are expected to have
    /// booted it (`CortexM33Core::boot`) before attaching a debugger.
    /// 核心保留当前执行状态；调用方应在连接调试器前完成启动
    /// （`CortexM33Core::boot`）。
    #[must_use]
    pub fn new(core: CortexM33Core, bus: B) -> Self {
        Self {
            core,
            bus,
            breakpoints: BTreeSet::new(),
            last_stop: None,
            instruction_budget: DEFAULT_INSTRUCTION_BUDGET,
        }
    }

    /// Override the instruction budget for a single `Continue`
    /// 覆盖单次 `Continue` 的指令预算
    ///
    /// A budget of `0` makes `Continue` report
    /// [`StopReason::InstructionLimit`] immediately.
    /// 预算为 `0` 时 `Continue` 会立即上报
    /// [`StopReason::InstructionLimit`]。
    #[must_use]
    pub fn with_instruction_budget(mut self, budget: u64) -> Self {
        self.instruction_budget = budget;
        self
    }

    /// Execute one instruction and map the resulting state to a stop reason
    /// 执行一条指令并将结果状态映射为停止原因
    fn single_step(&mut self) -> StopReply {
        match self.core.step(&mut self.bus) {
            Ok(state) => self.stop_reply(stop_reason_for_step(state)),
            Err(error) => {
                log::debug!("single step failed: {error:?}");
                self.stop_reply(StopReason::Fault)
            }
        }
    }

    /// Run until a breakpoint, a non-running state, or the budget runs out
    /// 运行直到断点、非运行状态或预算耗尽
    fn continue_until_stop(&mut self) -> StopReply {
        let mut budget = self.instruction_budget;
        // The first iteration skips the PC check so that resuming from a
        // breakpoint steps over the current instruction instead of stopping
        // again immediately.
        // 首次迭代跳过 PC 检查，使从断点恢复执行时跨过当前指令，而不是立即
        // 再次停下。
        let mut resume_from_breakpoint = true;

        loop {
            let pc = self.core.pc();
            if !resume_from_breakpoint && self.breakpoints.contains(&pc) {
                return self.stop_reply(StopReason::Breakpoint);
            }
            resume_from_breakpoint = false;

            if budget == 0 {
                return self.stop_reply(StopReason::InstructionLimit);
            }
            budget -= 1;

            match self.core.step(&mut self.bus) {
                Ok(CpuState::Running) => {}
                Ok(state) => return self.stop_reply(stop_reason_for_step(state)),
                Err(error) => {
                    log::debug!("continue stopped by error: {error:?}");
                    return self.stop_reply(StopReason::Fault);
                }
            }
        }
    }

    /// Build a stop reply from the current PC / 根据当前 PC 构造停止回复
    fn stop_reply(&self, reason: StopReason) -> StopReply {
        StopReply {
            reason,
            pc: self.core.pc(),
        }
    }
}

/// Map a core state returned by `step` to a stop reason
/// 将 `step` 返回的核心状态映射为停止原因
fn stop_reason_for_step(state: CpuState) -> StopReason {
    match state {
        CpuState::Running => StopReason::SingleStep,
        CpuState::Breakpoint => StopReason::Breakpoint,
        CpuState::Sleeping => StopReason::Sleeping,
        CpuState::Faulted => StopReason::Fault,
        CpuState::Halted => StopReason::Halted,
        CpuState::Reset => StopReason::Reset,
    }
}

impl<B: AddressBus> GdbTarget for CortexM33Target<B> {
    fn read_register(&mut self, register: u32) -> Result<u32, GdbError> {
        match register {
            regs::R0..=regs::PC => Ok(self
                .core
                .read_register(u8::try_from(register).unwrap_or_default())),
            regs::XPSR => Ok(self.core.xpsr()),
            _ => Err(GdbError::InvalidRegister(register)),
        }
    }

    fn write_register(&mut self, register: u32, value: u32) -> Result<(), GdbError> {
        match register {
            regs::R0..=regs::PC => {
                self.core
                    .write_register(u8::try_from(register).unwrap_or_default(), value);
                Ok(())
            }
            regs::XPSR => {
                self.core.write_xpsr(value);
                Ok(())
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
            bytes.push(self.bus.read_u8(addr, &mut cycles)?);
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
            self.bus.write_u8(addr, byte, &mut cycles)?;
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

    fn resume(&mut self, mode: ResumeMode) -> Result<StopReply, GdbError> {
        let reply = match mode {
            ResumeMode::Step => self.single_step(),
            ResumeMode::Continue => self.continue_until_stop(),
        };
        self.last_stop = Some(reply.reason);
        Ok(reply)
    }

    fn status(&self) -> TargetStatus {
        TargetStatus {
            state: self.core.state(),
            stop_reason: self.last_stop,
            pc: self.core.pc(),
            total_cycles: self.core.total_cycles(),
        }
    }
}

#[cfg(test)]
mod tests {
    use rp2350_core::memory::{RamRegion, SimpleBus};
    use rp2350_core::types::CpuId;

    use super::*;

    const VECTOR_BASE: u32 = 0x0000_0000;
    const CODE_BASE: u32 = 0x2000_0000;
    const INITIAL_SP: u32 = 0x2000_0100;

    /// Build a booted core running NOP, NOP, B . from `CODE_BASE`
    /// 构造一个从 `CODE_BASE` 执行 NOP、NOP、B . 的已启动核心
    fn sample_target(budget: u64) -> CortexM33Target<SimpleBus> {
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(RamRegion::new("vectors", VECTOR_BASE, 64)))
            .unwrap();
        bus.register_region(Box::new(RamRegion::new("code", CODE_BASE, 64)))
            .unwrap();

        let mut cycles = 0;
        bus.write_u32(VECTOR_BASE, INITIAL_SP, &mut cycles).unwrap();
        bus.write_u32(VECTOR_BASE + 4, CODE_BASE | 1, &mut cycles)
            .unwrap();
        bus.write_u16(CODE_BASE, 0xBF00, &mut cycles).unwrap(); // NOP
        bus.write_u16(CODE_BASE + 2, 0xBF00, &mut cycles).unwrap(); // NOP
        bus.write_u16(CODE_BASE + 4, 0xE7FE, &mut cycles).unwrap(); // B .

        let mut core = CortexM33Core::new(CpuId::Core0);
        core.boot(&mut bus).unwrap();
        CortexM33Target::new(core, bus).with_instruction_budget(budget)
    }

    #[test]
    fn register_reads_match_booted_core() {
        let mut target = sample_target(10);

        assert_eq!(target.read_register(regs::PC).unwrap(), CODE_BASE);
        assert_eq!(target.read_register(regs::SP).unwrap(), INITIAL_SP);
        assert_eq!(target.read_register(regs::XPSR).unwrap(), 0x0100_0000);
        assert!(matches!(
            target.read_register(regs::COUNT),
            Err(GdbError::InvalidRegister(regs::COUNT))
        ));
    }

    #[test]
    fn register_writes_round_trip() {
        let mut target = sample_target(10);

        target.write_register(regs::R0, 0xDEAD_BEEF).unwrap();
        assert_eq!(target.read_register(regs::R0).unwrap(), 0xDEAD_BEEF);

        target.write_register(regs::XPSR, 0x1234_5678).unwrap();
        assert_eq!(target.read_register(regs::XPSR).unwrap(), 0x1234_5678);

        assert!(target.write_register(regs::COUNT, 1).is_err());
    }

    #[test]
    fn memory_round_trip_and_unmapped_error() {
        let mut target = sample_target(10);

        assert_eq!(
            target.read_memory(CODE_BASE, 4).unwrap(),
            vec![0x00, 0xBF, 0x00, 0xBF]
        );

        target.write_memory(CODE_BASE + 8, &[0xAA, 0x55]).unwrap();
        assert_eq!(
            target.read_memory(CODE_BASE + 8, 2).unwrap(),
            vec![0xAA, 0x55]
        );

        assert!(matches!(
            target.read_memory(0x5000_0000, 1),
            Err(GdbError::Bus(_))
        ));
    }

    #[test]
    fn single_step_advances_pc() {
        let mut target = sample_target(10);

        let reply = target.resume(ResumeMode::Step).unwrap();
        assert_eq!(reply.reason, StopReason::SingleStep);
        assert_eq!(reply.pc, CODE_BASE + 2);

        let status = target.status();
        assert_eq!(status.state, CpuState::Running);
        assert_eq!(status.stop_reason, Some(StopReason::SingleStep));
        assert_eq!(status.pc, CODE_BASE + 2);
        assert!(status.total_cycles > 0);
    }

    #[test]
    fn continue_stops_at_breakpoint() {
        let mut target = sample_target(10);
        target.set_breakpoint(CODE_BASE + 2).unwrap();
        assert!(target.has_breakpoint(CODE_BASE + 2));

        let reply = target.resume(ResumeMode::Continue).unwrap();
        assert_eq!(reply.reason, StopReason::Breakpoint);
        assert_eq!(reply.pc, CODE_BASE + 2);
        assert_eq!(target.status().stop_reason, Some(StopReason::Breakpoint));
    }

    #[test]
    fn continue_steps_over_current_breakpoint_then_hits_budget() {
        let mut target = sample_target(3);
        target.set_breakpoint(CODE_BASE + 2).unwrap();

        let first = target.resume(ResumeMode::Continue).unwrap();
        assert_eq!(first.reason, StopReason::Breakpoint);
        assert_eq!(first.pc, CODE_BASE + 2);

        let second = target.resume(ResumeMode::Continue).unwrap();
        assert_eq!(second.reason, StopReason::InstructionLimit);
    }

    #[test]
    fn clearing_breakpoint_disables_detection() {
        let mut target = sample_target(1);
        target.set_breakpoint(CODE_BASE + 2).unwrap();
        target.clear_breakpoint(CODE_BASE + 2).unwrap();
        assert!(!target.has_breakpoint(CODE_BASE + 2));

        let reply = target.resume(ResumeMode::Continue).unwrap();
        assert_eq!(reply.reason, StopReason::InstructionLimit);
    }

    #[test]
    fn step_on_reset_core_reports_reset() {
        let mut target = CortexM33Target::new(CortexM33Core::new(CpuId::Core0), SimpleBus::new());

        let reply = target.resume(ResumeMode::Step).unwrap();
        assert_eq!(reply.reason, StopReason::Reset);
        assert_eq!(target.status().state, CpuState::Reset);
    }
}
