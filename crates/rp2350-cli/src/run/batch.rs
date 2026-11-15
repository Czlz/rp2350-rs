//! Batched stepping for the host frontends / 宿主前端的批量步进
//!
//! `run`、`play` 的 `repl`/`perfbench` 路径都按「一批 N 步」推进客户机。
//! [`Machine::run_steps`] 把与指令无关的宿主工作（预算与停机检查、外设泵送、
//! 中断投递、时基推进）从每条指令摊薄到固定的步数边界上，同时保持这些事件发生
//! 在**完全相同**的步号：中断投递仍是每 [`PUMP_INTERVAL`] 步一次、时基仍是每
//! [`ADVANCE_INTERVAL`] 步一次、DMA/PIO 批次节奏不变，因此客户机可观察行为
//! （含 `[irq]` 日志、HSTX/DMA 计数、`[tail]` 与故障回溯）逐字节不变。
//! The `run`, `play` and `repl`/`perfbench` paths advance the guest in batches.
//! [`Machine::run_steps`] amortizes host-only work (budget/halt checks, peripheral
//! pumps, interrupt delivery, timebase) over fixed step boundaries while keeping
//! every event on exactly the same step number: interrupt delivery stays once per
//! [`PUMP_INTERVAL`] steps, the timebase once per [`ADVANCE_INTERVAL`] steps and
//! the DMA/PIO batch cadence is unchanged, so guest-visible behaviour (`[irq]`
//! log, HSTX/DMA counters, `[tail]` and fault traces) is byte-identical.

use anyhow::Result;
use rp2350_cpu::{CpuCore, CpuState};

use super::{
    ADVANCE_INTERVAL, CoreHandle, Machine, PUMP_INTERVAL, SPIN_REPORT_LIMIT, SPIN_REPORT_STEPS,
    SPIN_WINDOW_BYTES, deliver_irqs, pump, replay_system_writes,
};

impl Machine {
    /// Advance both cores by up to `max_steps` instructions
    /// 在两个核心上最多推进 `max_steps` 条指令
    ///
    /// Equivalent to calling [`Machine::step`] `max_steps` times, but host-only
    /// work is hoisted to the step boundaries where it actually takes effect.
    /// Each iteration still retires exactly one instruction on core 0 and, when
    /// it can make progress, one on core 1; the machine budget, the recent-step
    /// ring, the PPB replay guard and the spin detector keep per-instruction
    /// granularity because they are observable by diagnostics.
    /// 等价于调用 [`Machine::step`] `max_steps` 次，但把与指令无关的宿主工作提升到
    /// 真正生效的步边界上。每次迭代仍在 core0 退役一条指令，并在 core1 可推进时
    /// 也退役一条；预算、最近步数环形缓冲、PPB 回放门控与自旋检测保留逐指令粒度，
    /// 因为它们会被诊断读取。
    ///
    /// Returns [`CpuState::Running`] when the requested steps were retired and
    /// the halt reason observed by [`Machine::step`] otherwise.
    /// 完成请求的步数时返回 [`CpuState::Running`]，否则返回 [`Machine::step`]
    /// 观察到的停机状态。
    ///
    /// # Errors
    /// Propagates bus errors raised while dispatching a core-0 boot ROM trap,
    /// exactly like [`Machine::step`]. Core-1 faults are recorded in
    /// [`Machine::core1_halt_reason`] and never abort core 0.
    /// 与 [`Machine::step`] 一样传播 core0 boot ROM 陷阱分发期间的总线错误。
    /// core1 故障记录在 [`Machine::core1_halt_reason`]，绝不影响 core0。
    // 热循环必须保持为单个函数：拆分引入的调用边界正是本项优化要消除的开销。
    // The hot loop must stay one function: the call boundaries a split would add
    // are exactly the overhead this optimization removes.
    #[allow(clippy::too_many_lines)]
    pub fn run_steps(&mut self, max_steps: u64) -> Result<CpuState> {
        if self.halt.is_some() {
            return Ok(CpuState::Halted);
        }
        // 复位请求已锁存：宿主必须先重建机器，继续步进只会执行复位前的旧状态。
        // A reset request is latched: the host must rebuild the machine first;
        // stepping further would only execute the stale pre-reset state.
        if self.system_reset_pending {
            return Ok(CpuState::Halted);
        }
        let end = self.steps.saturating_add(max_steps).min(self.budget);
        if end <= self.steps {
            return Ok(CpuState::Halted);
        }
        // ARM 没有 CSR 窗口：`csr_read` 恒为 0，因此同步陷阱记录永远只会命中
        // 「原因未变化」的提前返回；RISC-V 必须逐指令探测 `mcause`。
        // ARM has no CSR window: `csr_read` is always 0, so the synchronous-trap
        // record always takes its "cause unchanged" early return; RISC-V must
        // probe `mcause` for every instruction.
        let tracks_traps = matches!(self.core, CoreHandle::RiscV(_));
        let mut steps = self.steps;
        let mut spin_base = self.spin_base;
        let mut spin_steps = self.spin_steps;
        let mut core1_live = core1_running(self);

        while steps < end {
            // 下一个共享边界（DMA/PIO 泵、中断投递、时基）的步号。这些工作都按
            // 绝对步数节流，因此内层循环可以直接跑到该边界，再一次性执行。
            // Step number of the next shared boundary (DMA/PIO pump, IRQ delivery,
            // timebase). All of them throttle on absolute step counts, so the inner
            // loop runs straight to that boundary and performs the work once.
            let until_pump = (self.pumped + PUMP_INTERVAL).saturating_sub(steps);
            let until_dma = (self.dma_pumped + PUMP_INTERVAL).saturating_sub(steps);
            let until_pio = (self.pio_pumped + PUMP_INTERVAL).saturating_sub(steps);
            let until_time = (self.advanced + ADVANCE_INTERVAL).saturating_sub(steps);
            let chunk = (end - steps)
                .min(until_pump.max(1))
                .min(until_dma.max(1))
                .min(until_pio.max(1))
                .min(until_time.max(1));

            let mut retired = 0u64;
            while retired < chunk {
                // 核心之间的 SIO 事件与 SEV 转发：事件到达后同一步即可唤醒睡眠核。
                // SIO events and SEV forwarding between cores, so a core woken by an
                // event resumes within the same step.
                self.sync_core_events();
                // ---- core 0: one instruction -------------------------------
                let pc = self.core.pc();
                // 睡眠核不执行指令，也不进入「已执行 PC」回溯缓冲。
                // A sleeping core executes nothing and stays out of the
                // executed-PC post-mortem ring.
                let core0_running = self.core.state() == CpuState::Running;
                if core0_running {
                    self.recent.push(steps, pc);
                }
                // 合成 boot ROM 的全部陷阱桩都位于 ROM 窗口内，窗口外的 PC 无需
                // 线性扫描桩表；与逐指令路径使用同一个门控。
                // Every synthetic boot ROM trap stub lives inside the ROM window,
                // so a PC outside it needs no stub-table scan; same guard as the
                // per-instruction path.
                let in_rom_window = pc.wrapping_sub(rp2350_bootrom::ROM_BASE)
                    < u32::try_from(rp2350_bootrom::ROM_SIZE).unwrap_or(u32::MAX);
                let mut trapped = false;
                if self.bootrom_applicable && in_rom_window && self.bootrom.handles(pc) {
                    self.steps = steps;
                    if let Some(outcome) = self.bootrom.dispatch(&mut self.core, &mut self.bus)? {
                        steps += 1;
                        self.steps = steps;
                        self.record_sync_trap();
                        // DMA 先于中断投递泵送，完成中断可在同一批次送达客户机。
                        // DMA is pumped before IRQ delivery so a completion
                        // interrupt is visible to the guest in the same batch.
                        self.pump_dma_batch();
                        self.pump_pio_batch();
                        if let Some(halt) = outcome.halt {
                            self.halt = Some(format!("bootrom halt: {halt}"));
                            self.finalize();
                            self.spin_base = spin_base;
                            self.spin_steps = spin_steps;
                            return Ok(CpuState::Halted);
                        }
                        // 陷阱步与原路径一致：立即泵送中断，不推进时基。
                        // Trap steps mirror the original path: pump interrupts now
                        // and do not advance the timebase.
                        if pump(
                            &mut self.core,
                            self.core1.as_mut(),
                            &self.peripherals,
                            &self.usb,
                            &mut self.irqs,
                        ) {
                            self.system_reset_pending = true;
                        }
                        // 陷阱步不执行 `after_running_step`：跳过 PPB 回放、自旋
                        // 统计与批次边界工作，与原路径一致。
                        // Trap steps skip `after_running_step`: no PPB replay, spin
                        // accounting or boundary work, matching the original path.
                        trapped = true;
                    }
                }
                if !trapped {
                    // 睡眠步：不退役指令，但时间照常推进（`steps` 仍 +1），且
                    // 继续执行本步的边界工作，使中断投递/泵送仍能唤醒它。
                    // A sleep step retires nothing while time advances (`steps`
                    // still increments) and keeps this step's boundary work, so
                    // interrupt delivery and peripheral pumps can still wake it.
                    let mut sleeping = false;
                    match self.core.step(&mut self.bus) {
                        Ok(CpuState::Running) => {}
                        Ok(CpuState::Sleeping) => sleeping = true,
                        Ok(state) => {
                            self.steps = steps;
                            self.halt = Some(format!("core entered state {state:?}"));
                            self.finalize();
                            self.spin_base = spin_base;
                            self.spin_steps = spin_steps;
                            return Ok(state);
                        }
                        Err(error) => {
                            self.steps = steps;
                            self.halt = Some(error.to_string());
                            self.finalize();
                            self.spin_base = spin_base;
                            self.spin_steps = spin_steps;
                            return Ok(CpuState::Faulted);
                        }
                    }
                    steps += 1;
                    if sleeping {
                        self.note_sleep_step(0);
                    } else {
                        if tracks_traps {
                            self.steps = steps;
                            self.record_sync_trap();
                        }
                        // 紧凑自旋检测：PC 连续停留在同一小窗口内达到阈值时告警一次。
                        // Tight-spin detection: warn once when the PC stays inside one
                        // small window for the threshold number of steps.
                        let pc = self.core.pc();
                        if pc.wrapping_sub(spin_base) < SPIN_WINDOW_BYTES {
                            spin_steps += 1;
                            if spin_steps == SPIN_REPORT_STEPS
                                && self.spin_reports < SPIN_REPORT_LIMIT
                            {
                                self.spin_reports += 1;
                                self.emit_spin_notice(spin_base, spin_steps);
                            }
                        } else {
                            spin_base = pc;
                            spin_steps = 0;
                        }
                        // PPB 系统写入必须在下一条指令前生效。世代未推进时回放队列
                        // 必然为空，直接跳过（省去一次互斥锁）。
                        // System-control writes must land before the next instruction.
                        // An unchanged epoch proves the replay queue is empty, so the
                        // drain (and its mutex) is skipped entirely.
                        let epoch = self.bus.ppb_write_epoch();
                        if epoch != self.ppb_epoch_seen {
                            self.ppb_epoch_seen = epoch;
                            if replay_system_writes(&mut self.core, &self.peripherals) {
                                self.system_reset_pending = true;
                            }
                        }
                    }
                }

                // ---- shared boundary work, then core 1 ----------------------
                // 边界步的顺序必须与原 `step` 完全一致：DMA/PIO 泵送 → 对端握手
                // 与中断投递 → 时基 → core1。`service_core1_peer` 可能在同一步内
                // 挂起 core1 启动，原路径就是在同一步的 `step_core1` 里消费它。
                // On a boundary step the order must match the original `step`
                // exactly: DMA/PIO pumps -> peer handshake and IRQ delivery ->
                // timebase -> core 1. `service_core1_peer` can make a core-1 launch
                // pending within this step, and the original path consumes it in
                // the same step's `step_core1`.
                if !trapped && retired + 1 == chunk {
                    self.steps = steps;
                    self.pump_dma_batch();
                    self.pump_pio_batch();
                    if steps - self.pumped >= PUMP_INTERVAL {
                        self.peripherals.service_core1_peer();
                        deliver_irqs(
                            &mut self.core,
                            self.core1.as_mut(),
                            &self.peripherals,
                            &self.usb,
                            &mut self.irqs,
                        );
                        self.pumped = steps;
                    }
                    if steps - self.advanced >= ADVANCE_INTERVAL {
                        self.advance_timebase();
                    }
                }
                // 只有 core1 已在运行，或 PSM 复位 / SIO FIFO 启动挂起时才需要调用：
                // 其余情况下 `step_core1` 只会读取状态后立即返回 false。
                // Core 1 only needs a call while it is running or a PSM reset /
                // SIO FIFO launch is pending; otherwise `step_core1` just reads
                // its state and returns false.
                if core1_live
                    || self.peripherals.core1_reset_pending()
                    || self.peripherals.core1_launch_pending()
                {
                    self.steps = steps;
                    self.step_core1();
                    core1_live = core1_running(self);
                }

                // 睡眠/唤醒统计与「双核都在睡」诊断必须覆盖批量路径，否则
                // `run`/`repl`/`perfbench` 的睡眠统计会漏记。
                // Sleep/wake accounting and the "both cores asleep" diagnostic
                // must cover the batched path too, otherwise `run`, `repl` and
                // `perfbench` would under-report sleep steps.
                let core0_sleeping = self.core.is_sleeping();
                let core1_sleeping = self
                    .core1
                    .as_ref()
                    .is_some_and(|core| core.state() == CpuState::Sleeping);
                self.note_core_sleep_state(0, core0_sleeping);
                self.note_core_sleep_state(1, core1_sleeping);
                self.note_both_sleeping(core0_sleeping && core1_sleeping, !core0_sleeping);

                retired += 1;
                // 复位请求一旦锁存就停止推进：前端随后重建机器（`run`）或报告
                // 并停止（`repl`/`play`）。
                // Stop advancing as soon as a reset request is latched: the
                // frontend then either rebuilds the machine (`run`) or reports
                // and stops (`repl`/`play`).
                if self.system_reset_pending {
                    self.steps = steps;
                    self.spin_base = spin_base;
                    self.spin_steps = spin_steps;
                    return Ok(CpuState::Halted);
                }
                if trapped {
                    break;
                }
            }

            self.steps = steps;
        }

        self.steps = steps;
        self.spin_base = spin_base;
        self.spin_steps = spin_steps;
        Ok(CpuState::Running)
    }
}

/// Whether core 1 can be stepped right now / core1 当前是否可推进
fn core1_running(machine: &Machine) -> bool {
    matches!(
        machine.core1.as_ref().map(CpuCore::state),
        Some(CpuState::Running | CpuState::Sleeping)
    )
}
