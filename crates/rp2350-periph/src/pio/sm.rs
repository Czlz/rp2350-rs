//! PIO state-machine execution: clocking, pin I/O, jumps, waits and IRQs
//! PIO 状态机执行：时钟驱动、引脚 I/O、跳转、等待与中断标志
//!
//! Every call models one PIO clock edge: the per-machine `CLKDIV` accumulator
//! decides whether the machine emits a cycle, a pending instruction delay is
//! burned before the next fetch, and side-set is emitted at the start of the
//! instruction that carries it. DMA handshakes and processor interrupt routing
//! belong to later tasks.
//! 每次调用建模一个 PIO 时钟边沿：每台状态机的 `CLKDIV` 累加器决定是否产生机器
//! 周期，指令退休后的延迟在下次取指前消耗，side-set 在携带它的指令起始处输出。
//! DMA 握手与处理器中断路由留待后续任务。

use super::Pio;
use super::regs::{
    EXECCTRL_EXEC_STALLED, SM_COUNT, StateMachine, low_bits_mask, read_pin_bits, write_pin_bits,
};

/// Outcome of executing one instruction / 执行一条指令的结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Exec {
    /// Instruction retired; the PC advances (with wrap) / 指令退休，PC 顺序推进（含 wrap）
    Advance,
    /// Instruction retired and redirected the PC / 指令退休并改写 PC
    Branch(u8),
    /// Instruction did not retire and must be retried / 指令未退休，需要重试
    Stall,
}

/// Mutable PIO resources an instruction can touch besides its own state
/// 指令除自身状态外可访问的可变 PIO 资源
pub(super) struct SmBus<'a> {
    /// Shared PIO IRQ flags / PIO 共享 IRQ 标志
    pub(super) irq: &'a mut u32,
    /// Sticky `FDEBUG` flags / 粘滞 `FDEBUG` 标志
    pub(super) fdebug: &'a mut u32,
    /// GPIO input levels sampled by this cycle / 本周期采样的 GPIO 输入电平
    pub(super) input: u32,
    /// GPIO output levels shared by all state machines / 所有状态机共享的 GPIO 输出电平
    pub(super) output: &'a mut u32,
    /// GPIO output enables shared by all state machines / 所有状态机共享的 GPIO 输出使能
    pub(super) output_enable: &'a mut u32,
}

impl SmBus<'_> {
    /// Raise the sticky stall flag of this state machine
    /// 置位本状态机的粘滞 stall 标志
    pub(super) fn set_flag(&mut self, base: u32, index: u8) {
        *self.fdebug |= base << index;
    }

    /// Drive `count` pins from `base` with the low bits of `value`
    /// 用 `value` 的低位驱动从 `base` 起的 `count` 个引脚
    pub(super) fn write_pins(&mut self, base: u8, count: u8, value: u32) {
        write_pin_bits(self.output, base, count, value);
    }

    /// Drive `count` pin directions from `base` with the low bits of `value`
    /// 用 `value` 的低位驱动从 `base` 起的 `count` 个引脚方向
    pub(super) fn write_pin_dirs(&mut self, base: u8, count: u8, value: u32) {
        write_pin_bits(self.output_enable, base, count, value);
    }

    /// Sample `count` pins starting at `base`, right-aligned
    /// 从 `base` 起采样 `count` 个引脚，结果按低位对齐
    pub(super) fn read_pins(&self, base: u8, count: u8) -> u32 {
        if count >= 32 {
            // 采样全部 32 个引脚等价于按 `base` 右旋。Sampling all 32 pins is a rotation.
            return self.input.rotate_right(u32::from(base & 31));
        }
        read_pin_bits(self.input, base, count)
    }

    /// Level of a single pin / 单个引脚的电平
    pub(super) fn pin(&self, pin: u8) -> bool {
        self.input & (1u32 << (pin & 31)) != 0
    }
}

/// Resolve an IRQ field to a flag index / 将 IRQ 字段解析为标志序号
///
/// RP2350 encodes a two-bit PIO-block selector in bits 4:3: mode 2 is the
/// state-machine-relative mode (`irq rel`), modes 1/3 address the previous/next
/// PIO block. Cross-block routing is not modelled yet, so those two modes fall
/// back to this block's flags.
/// RP2350 用位 4:3 选择 PIO 块：模式 2 是状态机相对模式（`irq rel`），模式 1/3
/// 指向相邻 PIO 块。跨块路由尚未建模，因此这两种模式回落到本块的标志。
fn resolve_irq(field: u8, index: u8) -> u8 {
    let number = field & 0x07;
    match (field >> 3) & 0x03 {
        2 => (number & 0x04) | ((number + index) & 0x03),
        _ => number,
    }
}

impl Pio {
    /// Step one state machine by one PIO clock cycle / 让一个状态机执行一个 PIO 时钟周期
    ///
    /// Returns `true` when an instruction retired, or `false` when the machine
    /// stalled, was gated by `CLKDIV`, or is still burning instruction delay.
    /// `ADDR` and `EXEC_STALLED` are left describing a stall. This entry point
    /// ignores `CTRL.SM_ENABLE` so tests can drive a single machine;
    /// [`Pio::tick`] respects it.
    /// 指令退休时返回 `true`；阻塞（stall）、被 `CLKDIV` 分频拦住或仍在消耗指令
    /// 延迟时返回 `false`。阻塞时 `ADDR` 与 `EXEC_STALLED` 保持阻塞现场。该入口
    /// 忽略 `CTRL.SM_ENABLE` 以便测试单个状态机；[`Pio::tick`] 则会遵循它。
    pub fn step_sm(&mut self, index: usize) -> bool {
        if index >= SM_COUNT {
            return false;
        }
        let Pio {
            sm,
            imem,
            irq,
            fdebug,
            input_levels,
            output_levels,
            output_enables,
            ..
        } = self;
        let machine = &mut sm[index];
        let instr = imem[usize::from(machine.pc)];
        let mut bus = SmBus {
            irq,
            fdebug,
            input: *input_levels,
            output: output_levels,
            output_enable: output_enables,
        };
        machine.step(instr, &mut bus)
    }

    /// Advance every state machine by `clk_sys_cycles` system-clock cycles
    /// 让所有状态机推进 `clk_sys_cycles` 个系统时钟周期
    ///
    /// The argument is measured in `clk_sys` cycles, exactly like the retired
    /// step count the CLI hands in (one retired instruction is one `clk_sys`
    /// cycle in the CPU model). It is **not** a PIO clock count: each machine's
    /// `CLKDIV` accumulator turns `INT + FRAC/256` of these cycles into one PIO
    /// instruction, so the instruction rate is `clk_sys / CLKDIV`. Every
    /// machine's divider free-runs (also while the machine is disabled); only
    /// machines with `CTRL.SM_ENABLE` set execute. The phase accumulator keeps
    /// its fractional remainder across batches, so no clock is lost or invented
    /// at a batch boundary.
    /// 参数以 `clk_sys` 周期计，与 CLI 交入的退休步数相同（CPU 模型中一条退休
    /// 指令 = 一个 `clk_sys` 周期）。它**不是** PIO 时钟数：每台状态机的 `CLKDIV`
    /// 累加器把这些周期中的 `INT + FRAC/256` 个换成一个 PIO 指令周期，因此指令速率
    /// 为 `clk_sys / CLKDIV`。所有机器的分频器都自由运行（禁用期间也推进），只有
    /// `CTRL.SM_ENABLE` 置位的机器执行指令。相位累加器跨批次保留小数余数，批次
    /// 边界不会丢失或凭空产生时钟。
    pub fn tick(&mut self, clk_sys_cycles: u64) {
        self.clk_sys_cycles = self.clk_sys_cycles.saturating_add(clk_sys_cycles);
        let Pio {
            sm,
            imem,
            enabled,
            irq,
            fdebug,
            input_levels,
            output_levels,
            output_enables,
            ..
        } = self;
        for _ in 0..clk_sys_cycles {
            for (index, machine) in sm.iter_mut().enumerate() {
                // 分频器自由运行，禁用期间也推进；只有使能位真正执行指令。
                // The divider free-runs even while disabled; only an enabled
                // machine executes.
                if !machine.advance_divider() || *enabled & (1 << index) == 0 {
                    continue;
                }
                let instr = imem[usize::from(machine.pc)];
                let mut bus = SmBus {
                    irq: &mut *irq,
                    fdebug: &mut *fdebug,
                    input: *input_levels,
                    output: &mut *output_levels,
                    output_enable: &mut *output_enables,
                };
                machine.exec_clocked(instr, &mut bus);
            }
        }
    }

    /// Execute `instr` immediately, as a `SMx_INSTR` write does
    /// 立即执行 `instr`，与写入 `SMx_INSTR` 的行为一致
    ///
    /// The PC only changes when the instruction itself redirects it. Side-set
    /// and delay still apply, but the injected instruction bypasses `CLKDIV`
    /// and any pending delay because the write happens synchronously.
    /// 仅当指令自身改写 PC 时 PC 才会变化。side-set 与延迟仍然生效，但注入指令
    /// 绕过 `CLKDIV` 与已有延迟，因为写入是同步发生的。
    pub(super) fn execute_injected(&mut self, index: usize, instr: u16) {
        let Pio {
            sm,
            irq,
            fdebug,
            input_levels,
            output_levels,
            output_enables,
            ..
        } = self;
        let machine = &mut sm[index];
        let mut bus = SmBus {
            irq,
            fdebug,
            input: *input_levels,
            output: output_levels,
            output_enable: output_enables,
        };
        let delay = machine.apply_delay_sideset(instr, &mut bus);
        match machine.execute(instr, &mut bus, 0) {
            Exec::Stall => machine.execctrl |= EXECCTRL_EXEC_STALLED,
            other => {
                if let Exec::Branch(target) = other {
                    machine.pc = target & 0x1F;
                }
                machine.delay_cycles = delay;
            }
        }
    }
}

impl StateMachine {
    /// Execute one PIO clock edge and update `PC` / 执行一个 PIO 时钟边沿并更新 `PC`
    ///
    /// Returns `false` when the edge produced no retired instruction: either the
    /// clock divider gated this machine or a delay cycle was burned.
    /// 边沿未退休指令时返回 `false`：要么被分频器拦住，要么消耗了一个延迟周期。
    pub(super) fn step(&mut self, instr: u16, bus: &mut SmBus<'_>) -> bool {
        if !self.advance_divider() {
            return false;
        }
        self.exec_clocked(instr, bus)
    }

    /// Execute one machine cycle that the clock divider has already granted
    /// 执行一个分频器已经放行的机器周期
    ///
    /// The caller advances the divider first, so no `clk_sys` cycle is counted
    /// twice. Pending instruction delay is burned before the next fetch; a
    /// stalled instruction is retried on the next machine cycle.
    /// 调用方先推进分频器，因此同一个 `clk_sys` 周期不会被重复计数。指令延迟在下次
    /// 取指前消耗；阻塞的指令在下一个机器周期重试。
    pub(super) fn exec_clocked(&mut self, instr: u16, bus: &mut SmBus<'_>) -> bool {
        self.machine_cycles = self.machine_cycles.saturating_add(1);
        if self.delay_cycles > 0 {
            self.delay_cycles -= 1;
            return false;
        }
        let delay = self.apply_delay_sideset(instr, bus);
        match self.execute(instr, bus, 0) {
            Exec::Stall => {
                self.execctrl |= EXECCTRL_EXEC_STALLED;
                false
            }
            Exec::Advance => {
                self.execctrl &= !EXECCTRL_EXEC_STALLED;
                self.pc = self.next_pc();
                self.delay_cycles = delay;
                self.instructions_retired = self.instructions_retired.saturating_add(1);
                true
            }
            Exec::Branch(target) => {
                self.execctrl &= !EXECCTRL_EXEC_STALLED;
                self.pc = target & 0x1F;
                self.delay_cycles = delay;
                self.instructions_retired = self.instructions_retired.saturating_add(1);
                true
            }
        }
    }

    /// Emit the side-set field of `instr` and return its delay-cycle count
    /// 输出 `instr` 的 side-set 字段并返回其延迟周期数
    ///
    /// Bits 12:8 are shared: the top `SIDESET_COUNT` bits carry side-set and the
    /// remaining low bits are the delay. With `EXECCTRL.SIDE_EN` the top bit of
    /// that field is the side-set enable strobe and one fewer bit is data.
    /// 位 12:8 为共享字段：高 `SIDESET_COUNT` 位是 side-set，剩余低位是延迟。置位
    /// `EXECCTRL.SIDE_EN` 时该字段最高位是 side-set 使能脉冲，数据位少一位。
    fn apply_delay_sideset(&self, instr: u16, bus: &mut SmBus<'_>) -> u8 {
        let field = (instr >> 8) & 0x1F;
        let count = self.sideset_count();
        if count == 0 {
            // 无 side-set 时整个字段都是延迟。Without side-set the whole field is delay.
            return field as u8;
        }
        let delay_bits = 5 - u32::from(count);
        let delay = (u32::from(field) & low_bits_mask(delay_bits)) as u8;
        let value = u32::from(field >> delay_bits);
        if !self.side_en() {
            self.drive_sideset(bus, value, count);
        } else if field & 0x10 != 0 {
            // 最高位为 1 才执行 side-set；其余 count-1 位为数据。
            // The MSB enables side-set this cycle; the other count-1 bits are data.
            self.drive_sideset(bus, value & low_bits_mask(u32::from(count) - 1), count - 1);
        }
        delay
    }

    /// Drive the side-set pins or pin directions / 驱动 side-set 引脚电平或方向
    fn drive_sideset(&self, bus: &mut SmBus<'_>, value: u32, count: u8) {
        if self.side_pindir() {
            bus.write_pin_dirs(self.sideset_base(), count, value);
        } else {
            bus.write_pins(self.sideset_base(), count, value);
        }
    }

    /// Sequential next PC, honouring `WRAP_TOP`/`WRAP_BOTTOM`
    /// 顺序推进的下一 PC，遵循 `WRAP_TOP`/`WRAP_BOTTOM`
    fn next_pc(&self) -> u8 {
        if self.pc == self.wrap_top() {
            self.wrap_bottom()
        } else {
            (self.pc + 1) & 0x1F
        }
    }

    /// Decode and execute one 16-bit instruction / 解码并执行一条 16 位指令
    ///
    /// `depth` guards against recursive `EXEC`: hardware executes at most one
    /// nested instruction.
    /// `depth` 用于阻止 `EXEC` 递归：硬件最多执行一层嵌套指令。
    pub(super) fn execute(&mut self, instr: u16, bus: &mut SmBus<'_>, depth: u8) -> Exec {
        let arg = (instr & 0xFF) as u8;
        match instr >> 13 {
            0 => self.exec_jmp(arg, bus),
            1 => self.exec_wait(arg, bus),
            2 => self.exec_in(arg, bus),
            3 => self.exec_out(arg, bus, depth),
            4 => self.exec_push_pull(arg, bus),
            5 => self.exec_mov(arg, bus, depth),
            6 => self.exec_irq(arg, bus),
            _ => self.exec_set(arg, bus),
        }
    }

    /// `JMP condition, address` / `JMP condition, address`
    fn exec_jmp(&mut self, arg: u8, bus: &SmBus<'_>) -> Exec {
        let take = match (arg >> 5) & 0x07 {
            0 => true,
            1 => self.x == 0,
            2 => {
                let nonzero = self.x != 0;
                self.x = self.x.wrapping_sub(1);
                nonzero
            }
            3 => self.y == 0,
            4 => {
                let nonzero = self.y != 0;
                self.y = self.y.wrapping_sub(1);
                nonzero
            }
            5 => self.x != self.y,
            // JMP PIN：采样 `EXECCTRL.JMP_PIN` 指定的引脚（不是 `IN_BASE`）。
            // JMP PIN samples the pin named by `EXECCTRL.JMP_PIN`, not `IN_BASE`.
            6 => bus.pin(self.jmp_pin()),
            // !OSRE：OSR 仍有数据时为真。!OSRE is true while the OSR still holds data.
            _ => self.osr_bits > self.empty_level(),
        };
        if take {
            Exec::Branch(arg & 0x1F)
        } else {
            Exec::Advance
        }
    }

    /// `WAIT polarity source, index` / `WAIT polarity source, index`
    fn exec_wait(&mut self, arg: u8, bus: &SmBus<'_>) -> Exec {
        let polarity = arg & 0x80 != 0;
        let satisfied = match (arg >> 5) & 0x03 {
            // GPIO：索引就是 PIO 引脚号。GPIO: the index is the PIO pin number.
            0 => bus.pin(arg & 0x1F) == polarity,
            // PIN：引脚号为 `IN_BASE + index`（回绕）。PIN: `IN_BASE + index`, wrapping.
            1 => bus.pin((self.in_base() + (arg & 0x1F)) & 31) == polarity,
            // IRQ：等待标志等于极性。IRQ: wait until the flag equals the polarity.
            2 => {
                let bit = resolve_irq(arg & 0x1F, self.index);
                (*bus.irq & (1 << bit) != 0) == polarity
            }
            // 保留源按立即通过处理。Reserved source passes immediately.
            _ => true,
        };
        if satisfied {
            Exec::Advance
        } else {
            Exec::Stall
        }
    }

    /// `IRQ set/wait/clear/rel, index` / `IRQ set/wait/clear/rel, index`
    fn exec_irq(&mut self, arg: u8, bus: &mut SmBus<'_>) -> Exec {
        if arg & 0x80 != 0 {
            // 保留编码：按空操作退休。Reserved encoding retires as a no-op.
            return Exec::Advance;
        }
        let bit = resolve_irq(arg & 0x1F, self.index);
        let clear = arg & 0x40 != 0;
        let wait = arg & 0x20 != 0;
        if clear {
            *bus.irq &= !(1 << bit);
            self.pending_irq_wait = None;
            return Exec::Advance;
        }
        if self.pending_irq_wait == Some(bit) {
            // 已置位并等待其他代理清零；清零后本次指令退休。
            // The flag was raised by this instruction; retire once another agent clears it.
            if *bus.irq & (1 << bit) != 0 {
                return Exec::Stall;
            }
            self.pending_irq_wait = None;
            return Exec::Advance;
        }
        *bus.irq |= 1 << bit;
        if wait {
            self.pending_irq_wait = Some(bit);
            Exec::Stall
        } else {
            Exec::Advance
        }
    }

    /// `SET destination, data` / `SET destination, data`
    fn exec_set(&mut self, arg: u8, bus: &mut SmBus<'_>) -> Exec {
        let data = u32::from(arg & 0x1F);
        match (arg >> 5) & 0x07 {
            // 0 = PINS：写 `SET_COUNT` 个引脚。0 = PINS writes `SET_COUNT` pins.
            0 => bus.write_pins(self.set_base(), self.set_count(), data),
            1 => self.x = data,
            2 => self.y = data,
            // 4 = PINDIRS：写 `SET_COUNT` 个输出使能。4 = PINDIRS writes enables.
            4 => bus.write_pin_dirs(self.set_base(), self.set_count(), data),
            _ => {}
        }
        Exec::Advance
    }
}
