//! PIO state-machine execution: clocking, jumps, waits and IRQs
//! PIO 状态机执行：时钟驱动、跳转、等待与中断标志
//!
//! Only the instruction semantics are modelled here. GPIO/PINCTRL, side-set,
//! delay cycles, FIFO joining, DMA handshakes and processor interrupt routing
//! belong to later tasks.
//! 此处只建模指令语义。GPIO/PINCTRL、side-set、延迟周期、FIFO 合并、DMA 握手
//! 与处理器中断路由留待后续任务。

use super::Pio;
use super::regs::{EXECCTRL_EXEC_STALLED, SM_COUNT, StateMachine};

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
}

impl SmBus<'_> {
    /// Raise the sticky stall flag of this state machine
    /// 置位本状态机的粘滞 stall 标志
    pub(super) fn set_flag(&mut self, base: u32, index: u8) {
        *self.fdebug |= base << index;
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
    /// stalled (`ADDR` and `EXEC_STALLED` are left describing the stall).
    /// This entry point ignores `CTRL.SM_ENABLE` so tests can drive a single
    /// machine; [`Pio::tick`] respects it.
    /// 指令退休时返回 `true`；阻塞（stall）时返回 `false`，此时 `ADDR` 与
    /// `EXEC_STALLED` 保持阻塞现场。该入口忽略 `CTRL.SM_ENABLE` 以便测试单个
    /// 状态机；[`Pio::tick`] 则会遵循它。
    pub fn step_sm(&mut self, index: usize) -> bool {
        if index >= SM_COUNT {
            return false;
        }
        let Pio {
            sm,
            imem,
            irq,
            fdebug,
            ..
        } = self;
        let machine = &mut sm[index];
        let instr = imem[usize::from(machine.pc)];
        let mut bus = SmBus { irq, fdebug };
        machine.step(instr, &mut bus)
    }

    /// Advance every enabled state machine by `cycles` PIO clock cycles
    /// 让所有已启用的状态机推进 `cycles` 个 PIO 时钟周期
    ///
    /// Each cycle runs at most one instruction per machine. `CLKDIV` is treated
    /// as 1:1 in this stage.
    /// 每个周期每台状态机最多执行一条指令。本阶段 `CLKDIV` 按 1:1 处理。
    pub fn tick(&mut self, cycles: u64) {
        for _ in 0..cycles {
            for index in 0..SM_COUNT {
                if self.enabled & (1 << index) != 0 {
                    self.step_sm(index);
                }
            }
        }
    }

    /// Execute `instr` immediately, as a `SMx_INSTR` write does
    /// 立即执行 `instr`，与写入 `SMx_INSTR` 的行为一致
    ///
    /// The PC only changes when the instruction itself redirects it.
    /// 仅当指令自身改写 PC 时 PC 才会变化。
    pub(super) fn execute_injected(&mut self, index: usize, instr: u16) {
        let Pio {
            sm, irq, fdebug, ..
        } = self;
        let machine = &mut sm[index];
        let mut bus = SmBus { irq, fdebug };
        match machine.execute(instr, &mut bus, 0) {
            Exec::Advance => {}
            Exec::Branch(target) => machine.pc = target & 0x1F,
            Exec::Stall => machine.execctrl |= EXECCTRL_EXEC_STALLED,
        }
    }
}

impl StateMachine {
    /// Execute one instruction and update `PC` / 执行一条指令并更新 `PC`
    pub(super) fn step(&mut self, instr: u16, bus: &mut SmBus<'_>) -> bool {
        match self.execute(instr, bus, 0) {
            Exec::Stall => {
                self.execctrl |= EXECCTRL_EXEC_STALLED;
                false
            }
            Exec::Advance => {
                self.execctrl &= !EXECCTRL_EXEC_STALLED;
                self.pc = self.next_pc();
                true
            }
            Exec::Branch(target) => {
                self.execctrl &= !EXECCTRL_EXEC_STALLED;
                self.pc = target & 0x1F;
                true
            }
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
            0 => self.exec_jmp(arg),
            1 => self.exec_wait(arg, bus),
            2 => self.exec_in(arg, bus),
            3 => self.exec_out(arg, bus, depth),
            4 => self.exec_push_pull(arg, bus),
            5 => self.exec_mov(arg, bus, depth),
            6 => self.exec_irq(arg, bus),
            _ => self.exec_set(arg),
        }
    }

    /// `JMP condition, address` / `JMP condition, address`
    fn exec_jmp(&mut self, arg: u8) -> Exec {
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
            // JMP PIN：引脚模型未接入，输入恒读 0，条件不成立。
            // JMP PIN: no pin model yet, the sampled pin reads 0 so the branch is not taken.
            6 => false,
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
    fn exec_wait(&mut self, arg: u8, bus: &mut SmBus<'_>) -> Exec {
        let polarity = arg & 0x80 != 0;
        let satisfied = match (arg >> 5) & 0x03 {
            // GPIO/PIN：未接引脚时输入恒为 0，因此 `WAIT 0` 立即通过、
            // `WAIT 1` 永久阻塞（引脚模型接入后此语义会改变）。
            // GPIO/PIN: with no pins attached the sampled level is always 0, so
            // `WAIT 0` passes immediately and `WAIT 1` blocks forever.
            0 | 1 => !polarity,
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
    fn exec_set(&mut self, arg: u8) -> Exec {
        let data = u32::from(arg & 0x1F);
        match (arg >> 5) & 0x07 {
            1 => self.x = data,
            2 => self.y = data,
            // 0 = PINS、4 = PINDIRS：引脚模型未接入，写入被丢弃。
            // 0 = PINS, 4 = PINDIRS: no pin model yet, the write is discarded.
            _ => {}
        }
        Exec::Advance
    }
}
