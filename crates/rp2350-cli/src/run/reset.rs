//! `AIRCR.SYSRESETREQ` restart policy for the `run` frontend
//! `run` 前端的 `AIRCR.SYSRESETREQ` 重启策略
//!
//! Firmware requests a chip reset by writing `SCB_AIRCR = VECTKEY | SYSRESETREQ`.
//! The core latches that request as a one-shot host event, and `run` honours it
//! by rebuilding the whole [`Machine`] from the same image. The number of
//! restarts is bounded so a firmware that resets in its boot path cannot loop
//! forever.
//! 固件通过写 `SCB_AIRCR = VECTKEY | SYSRESETREQ` 请求芯片复位。核把该请求锁存为
//! 一次性宿主事件，`run` 用同一映像重建整个 [`Machine`] 来响应。重启次数有上限，
//! 因此启动路径里就复位的固件不会无限循环。

use std::io::Write;

use anyhow::{Result, bail};
use rp2350_cpu::CpuState;

use super::{
    BUDGET_REASON, Machine, MachineOptions, RunOptions, SYSTEM_RESET_MESSAGE, drain_notices,
    print_post_mortem, write_dumps,
};

/// Maximum number of guest-requested system resets honoured by `run`
/// `run` 最多响应的客户机系统复位次数
///
/// The request that follows the limit is reported as a reset storm instead of
/// being restarted again. Three restarts cover the usual "flash/clock retune
/// once" patterns while still failing fast on a boot loop.
/// 超过上限的下一次请求会作为复位风暴报告，而不再重启。三次足以覆盖常见的
/// 「Flash/时钟重配一次」模式，同时对启动死循环快速失败。
pub const MAX_SYSTEM_RESETS: u32 = 3;

/// Restart budget state for one `run` invocation
/// 一次 `run` 调用的重启预算状态
#[derive(Debug, Default)]
pub struct SystemResetPolicy {
    /// Restarts already granted / 已允许的重启次数
    restarts: u32,
}

impl SystemResetPolicy {
    /// Create a policy with no restarts recorded / 创建尚未记录重启的策略
    #[must_use]
    pub const fn new() -> Self {
        Self { restarts: 0 }
    }

    /// Record one system-reset request and return the restart number
    /// 记录一次系统复位请求并返回重启序号
    ///
    /// # Errors
    /// Returns a `reset storm` error when this request is the one after
    /// [`MAX_SYSTEM_RESETS`].
    /// 当该请求超过 [`MAX_SYSTEM_RESETS`] 时返回 `reset storm` 错误。
    pub fn record(&mut self) -> Result<u32> {
        self.restarts = self.restarts.saturating_add(1);
        if self.restarts > MAX_SYSTEM_RESETS {
            bail!(
                "reset storm: guest requested AIRCR.SYSRESETREQ {} times, more than the {} \
                 restart limit / 复位风暴：固件请求 AIRCR.SYSRESETREQ {} 次，超过 {} 次重启上限",
                self.restarts,
                MAX_SYSTEM_RESETS,
                self.restarts,
                MAX_SYSTEM_RESETS
            );
        }
        Ok(self.restarts)
    }
}

/// Boot the image and execute it under the restart policy
/// 启动映像并在重启策略下执行
///
/// The boot header is printed once. Every honoured request prints
/// [`SYSTEM_RESET_MESSAGE`] followed by a `reboot n/3` line with the fresh
/// vectors, so the log itself proves the machine was rebuilt (PC/SP back to the
/// image entry point, step counter zero again). All diagnostics of the last
/// boot are printed exactly like a normal `run`.
/// 启动头部只打印一次。每次响应的请求先打印 [`SYSTEM_RESET_MESSAGE`]，再打印带
/// 新向量的 `reboot n/3` 行，因此日志本身即可证明机器已重建（PC/SP 回到映像入口、
/// 步数计数器重新归零）。最后一次启动的诊断与普通 `run` 相同。
///
/// # Errors
/// Propagates boot errors, bus errors from a step, and the reset-storm error.
/// 传播启动错误、步进期间的总线错误与复位风暴错误。
pub(crate) fn execute(bytes: &[u8], options: &RunOptions, stdout: &mut impl Write) -> Result<()> {
    let machine_options = MachineOptions {
        arch: options.arch,
        max_cycles: options.max_cycles,
        strict_decode: options.strict_decode,
        psram: options.psram,
    };
    let mut machine = Machine::new(bytes, &machine_options)?;
    if let Some(enabled) = options.sleep_fast_forward {
        machine.set_sleep_fast_forward(enabled);
    }
    let mut policy = SystemResetPolicy::new();

    writeln!(stdout, "[run] {}", machine.image_description())?;
    writeln!(stdout, "[run] core: {}", machine.core_description())?;
    if let Some((sp, pc)) = machine.boot_vectors() {
        writeln!(stdout, "[run] boot vectors: SP={sp:#010X} PC={pc:#010X}")?;
    }
    writeln!(
        stdout,
        "[run] started: PC={:#010X} SP={:#010X}",
        machine.pc(),
        machine.sp()
    )?;

    loop {
        for index in 0..options.trace_steps {
            let state = machine.trace_step(index)?;
            drain_notices(&mut machine, stdout)?;
            if state != CpuState::Running {
                break;
            }
        }
        while machine.halt_reason().is_none()
            && !machine.budget_exhausted()
            && !machine.system_reset_pending()
        {
            let state = machine.run_steps(machine.budget() - machine.steps())?;
            if machine.notices_pending() {
                drain_notices(&mut machine, stdout)?;
            }
            if state != CpuState::Running {
                break;
            }
        }
        machine.finalize();

        if !machine.take_system_reset_request() {
            break;
        }
        writeln!(stdout, "[run] {SYSTEM_RESET_MESSAGE}")?;
        let restart = policy.record()?;
        machine.reboot(bytes, &machine_options)?;
        if let Some(enabled) = options.sleep_fast_forward {
            machine.set_sleep_fast_forward(enabled);
        }
        writeln!(
            stdout,
            "[run] reboot {restart}/{MAX_SYSTEM_RESETS}: PC={:#010X} SP={:#010X}",
            machine.pc(),
            machine.sp()
        )?;
    }

    let reason = machine
        .halt_reason()
        .map_or_else(|| BUDGET_REASON.to_string(), str::to_owned);
    writeln!(stdout, "[run] halt: {reason}")?;
    for (step, pc, opcode) in machine.recent_steps() {
        writeln!(stdout, "[tail] {step:06} PC={pc:#010X} op={opcode:#06X}")?;
    }
    writeln!(
        stdout,
        "[run] executed {steps} steps, {us} us simulated, final PC={pc:#010X}",
        steps = machine.steps(),
        us = machine.time_us(),
        pc = machine.pc()
    )?;
    print_post_mortem(&mut machine, stdout)?;
    write_dumps(&mut machine, &options.dumps)?;
    for dump in &options.dumps {
        writeln!(
            stdout,
            "[run] dump {:#010X}+{:#X} -> {}",
            dump.address,
            dump.length,
            dump.path.display()
        )?;
    }
    Ok(())
}
