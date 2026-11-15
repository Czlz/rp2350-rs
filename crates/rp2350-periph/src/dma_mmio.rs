//! DMA register window / DMA 寄存器窗口
//!
//! Implements the APB register block side effects of [`Dma`]: per-channel
//! registers and their trigger aliases, the global interrupt registers,
//! `MULTI_CHAN_TRIGGER`, `CHAN_ABORT`, `SNIFF_CTRL`/`SNIFF_DATA` and the pacing
//! timers. The transfer engine lives in [`super`].
//! 实现 [`Dma`] 的 APB 寄存器块副作用：每通道寄存器与触发别名、全局中断寄存器、
//! `MULTI_CHAN_TRIGGER`、`CHAN_ABORT`、`SNIFF_CTRL`/`SNIFF_DATA` 与节拍定时器。
//! 传输引擎位于 [`super`]。

use rp2350_core::mmio::{MmioDevice, MmioWindow};

use super::pacing;
use super::sniff;
use super::{
    CH_AL1_CTRL, CH_AL1_READ_ADDR, CH_AL1_TRANS_COUNT_TRIG, CH_AL1_WRITE_ADDR, CH_AL2_CTRL,
    CH_AL2_READ_ADDR, CH_AL2_TRANS_COUNT, CH_AL2_WRITE_ADDR_TRIG, CH_AL3_CTRL,
    CH_AL3_READ_ADDR_TRIG, CH_AL3_TRANS_COUNT, CH_AL3_WRITE_ADDR, CH_CTRL_TRIG, CH_READ_ADDR,
    CH_TRANS_COUNT, CH_WRITE_ADDR, CHANNEL_REGION_END, CHANNEL_STRIDE, CHANNELS, CTRL_BUSY,
    CTRL_EN, CTRL_ERROR_MASK, CTRL_IRQ_QUIET, CTRL_WRITE_MASK, Channel, Dma, DmaState,
    GLOBAL_CHAN_ABORT, GLOBAL_FIFO_LEVELS, GLOBAL_INTE0, GLOBAL_INTR, GLOBAL_INTS0,
    GLOBAL_MULTI_CHAN_TRIGGER, GLOBAL_N_CHANNELS, GLOBAL_SNIFF_CTRL, GLOBAL_SNIFF_DATA,
    GLOBAL_TIMER0, INT_LINE_STRIDE, SNIFF_WRITE_MASK, TRANS_COUNT_MASK, TRANS_COUNT_MODE_MASK,
    TRANS_COUNT_MODE_SHIFT, trigger_channel,
};

use crate::alias::{apply_alias, split_alias};
use crate::map;

/// Side effect of a channel register write / 通道寄存器写入的副作用
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteEffect {
    /// The register is only stored / 仅存储寄存器值
    Plain,
    /// A non-zero value starts the channel; a zero value raises the
    /// "null trigger" interrupt when `IRQ_QUIET` is set
    /// 非零值启动通道；零值在置位 `IRQ_QUIET` 时产生“空触发”中断
    Trigger,
}

/// Latch the rewrite count and the transfer mode / 锁存重载计数与传输模式
///
/// `TRANS_COUNT[31:28]` selects the transfer mode and is latched together with
/// the reload count, exactly like the hardware reload register.
/// `TRANS_COUNT[31:28]` 选择传输模式，与重载计数一起锁存，与硬件重载寄存器一致。
fn latch_count(channel: &mut Channel, value: u32) {
    channel.trans_count_reload = value & TRANS_COUNT_MASK;
    channel.trans_count_mode = ((value & TRANS_COUNT_MODE_MASK) >> TRANS_COUNT_MODE_SHIFT) as u8;
}

/// Apply one per-channel register write, returning its side effect
/// 应用一次通道寄存器写入，并返回其副作用
///
/// The destination register is addressed by its offset inside the channel
/// block (`0x00..=0x3C`). Trigger policy lives in the caller so that a single
/// `WriteEffect` decides between a plain store and a channel start.
/// 目标寄存器由通道块内偏移（`0x00..=0x3C`）指定。触发策略留在调用方，用一个
/// `WriteEffect` 区分普通存储与启动通道。
fn write_channel_register(
    state: &mut DmaState,
    index: usize,
    register: u32,
    value: u32,
) -> Option<WriteEffect> {
    let channel = state.channels.get_mut(index)?;
    let effect = match register {
        CH_READ_ADDR | CH_AL1_READ_ADDR | CH_AL2_READ_ADDR => {
            channel.read_addr = value;
            WriteEffect::Plain
        }
        CH_AL3_READ_ADDR_TRIG => {
            channel.read_addr = value;
            WriteEffect::Trigger
        }
        CH_WRITE_ADDR | CH_AL1_WRITE_ADDR | CH_AL3_WRITE_ADDR => {
            channel.write_addr = value;
            WriteEffect::Plain
        }
        CH_AL2_WRITE_ADDR_TRIG => {
            channel.write_addr = value;
            WriteEffect::Trigger
        }
        // 计数别名只锁存重载值与模式，由触发器装载到工作计数。
        // The count aliases only latch the rewrite value and mode; a trigger
        // copies the count into the working count.
        CH_TRANS_COUNT | CH_AL2_TRANS_COUNT | CH_AL3_TRANS_COUNT => {
            latch_count(channel, value);
            WriteEffect::Plain
        }
        CH_AL1_TRANS_COUNT_TRIG => {
            latch_count(channel, value);
            WriteEffect::Trigger
        }
        CH_CTRL_TRIG => {
            write_ctrl(channel, value);
            WriteEffect::Trigger
        }
        // AL1/AL2/AL3_CTRL 是**非触发**控制别名：Pico SDK 的
        // dma_channel_configure(..., false) 正是写 al1_ctrl 来配置通道而不启动。
        // The AL1/AL2/AL3_CTRL aliases are non-triggering control writes;
        // dma_channel_configure(..., false) relies on that.
        CH_AL1_CTRL | CH_AL2_CTRL | CH_AL3_CTRL => {
            write_ctrl(channel, value);
            WriteEffect::Plain
        }
        _ => return None,
    };
    Some(effect)
}

/// Apply a `CTRL_TRIG` write to a channel / 将 `CTRL_TRIG` 写入应用到通道
fn write_ctrl(channel: &mut Channel, value: u32) {
    // 错误位为写 1 清零，其余可写位按写入值覆盖。
    // Error flags are write-one-to-clear; every other writable bit is replaced.
    let errors = channel.ctrl & CTRL_ERROR_MASK & !(value & CTRL_ERROR_MASK);
    channel.ctrl = (value & CTRL_WRITE_MASK) | errors;
    if channel.ctrl & CTRL_EN == 0 {
        channel.busy = false;
    }
}

/// Compute `INTSn` for one interrupt line / 计算某条中断线的 `INTSn`
fn line_status(state: &DmaState, line: usize) -> u32 {
    u32::from(state.intr & state.inte[line]) | u32::from(state.intf[line])
}

impl MmioDevice for Dma {
    fn name(&self) -> &str {
        "DMA"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::DMA_BASE, map::DMA_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        let (offset, _alias) = split_alias(offset);
        let state = self.state.lock();
        match offset {
            0x000..CHANNEL_REGION_END => {
                let index = (offset / CHANNEL_STRIDE) as usize;
                let register = offset % CHANNEL_STRIDE;
                let channel = state.channels.get(index)?;
                Some(match register {
                    CH_READ_ADDR | CH_AL1_READ_ADDR | CH_AL2_READ_ADDR | CH_AL3_READ_ADDR_TRIG => {
                        channel.read_addr
                    }
                    CH_WRITE_ADDR
                    | CH_AL1_WRITE_ADDR
                    | CH_AL2_WRITE_ADDR_TRIG
                    | CH_AL3_WRITE_ADDR => channel.write_addr,
                    CH_TRANS_COUNT
                    | CH_AL1_TRANS_COUNT_TRIG
                    | CH_AL2_TRANS_COUNT
                    | CH_AL3_TRANS_COUNT => channel.trans_count,
                    CH_CTRL_TRIG | CH_AL1_CTRL | CH_AL2_CTRL | CH_AL3_CTRL => {
                        channel.ctrl | (u32::from(channel.busy) * CTRL_BUSY)
                    }
                    _ => return None,
                })
            }
            GLOBAL_INTR => Some(u32::from(state.intr)),
            0x404..=0x43C => {
                let line = ((offset - GLOBAL_INTE0) / INT_LINE_STRIDE) as usize;
                match (offset - GLOBAL_INTE0) % INT_LINE_STRIDE {
                    0 => Some(u32::from(state.inte[line])),
                    4 => Some(u32::from(state.intf[line])),
                    8 => Some(line_status(&state, line)),
                    _ => None,
                }
            }
            GLOBAL_TIMER0..=0x44C => Some(state.timer[((offset - GLOBAL_TIMER0) / 4) as usize]),
            GLOBAL_MULTI_CHAN_TRIGGER | GLOBAL_FIFO_LEVELS | GLOBAL_CHAN_ABORT => Some(0),
            GLOBAL_SNIFF_CTRL => Some(state.sniff_ctrl),
            // 嗅探结果是累加器的展示视图（仅应用 OUT_INV/OUT_REV）。
            // The sniff value is a presentation view of the accumulator, with
            // only OUT_INV/OUT_REV applied.
            GLOBAL_SNIFF_DATA => Some(sniff::read(state.sniff_ctrl, state.sniff_data)),
            GLOBAL_N_CHANNELS => Some(CHANNELS as u32),
            _ => None,
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        // The Pico SDK uses hw_set_bits/hw_clear_bits/hw_xor_bits on DMA
        // registers (e.g. INTE1 via 0x5000_2414), so fold the alias first.
        // Pico SDK 会对 DMA 寄存器使用 hw_set_bits/hw_clear_bits/hw_xor_bits
        // （例如经 0x5000_2414 写 INTE1），因此先合并别名。
        let (offset, alias) = split_alias(offset);
        let value = apply_alias(self.read32(offset).unwrap_or(0), value, alias);
        let handled;
        {
            let mut state = self.state.lock();
            handled = match offset {
                0x000..CHANNEL_REGION_END => {
                    let index = (offset / CHANNEL_STRIDE) as usize;
                    let register = offset % CHANNEL_STRIDE;
                    let Some(effect) = write_channel_register(&mut state, index, register, value)
                    else {
                        return false;
                    };
                    match effect {
                        WriteEffect::Plain => {}
                        WriteEffect::Trigger if value != 0 => {
                            trigger_channel(&mut state, index);
                        }
                        WriteEffect::Trigger => {
                            // 零值写入是“空触发”：置位 IRQ_QUIET 时仍会产生完成中断。
                            // A zero write is a null trigger: it still raises the
                            // completion interrupt when IRQ_QUIET is set.
                            if state.channels[index].ctrl & CTRL_IRQ_QUIET != 0 {
                                state.intr |= 1u16 << index;
                            }
                        }
                    }
                    true
                }
                GLOBAL_INTR | GLOBAL_INTS0 | 0x41C | 0x42C | 0x43C => {
                    // INTR 与 INTSn 均为写 1 清零。INTR and INTSn are W1C.
                    state.intr &= !(value as u16);
                    true
                }
                0x404..=0x43C => {
                    let line = ((offset - GLOBAL_INTE0) / INT_LINE_STRIDE) as usize;
                    match (offset - GLOBAL_INTE0) % INT_LINE_STRIDE {
                        0 => state.inte[line] = value as u16,
                        4 => state.intf[line] = value as u16,
                        _ => return false,
                    }
                    true
                }
                GLOBAL_TIMER0..=0x44C => {
                    let index = ((offset - GLOBAL_TIMER0) / 4) as usize;
                    state.timer[index] = value;
                    pacing::program(&mut state, index);
                    true
                }
                GLOBAL_MULTI_CHAN_TRIGGER => {
                    for index in 0..CHANNELS {
                        if value & (1u32 << index) != 0 {
                            trigger_channel(&mut state, index);
                        }
                    }
                    true
                }
                GLOBAL_SNIFF_CTRL => {
                    state.sniff_ctrl = value & SNIFF_WRITE_MASK;
                    true
                }
                GLOBAL_SNIFF_DATA => {
                    // 数据手册：SNIFF_DATA 是种子兼累加器，写入即原样设置；
                    // 读出的 OUT_INV/OUT_REV 只作用于展示值。
                    // Datasheet: SNIFF_DATA is the seed and accumulator, so a
                    // write stores the value verbatim; OUT_INV/OUT_REV only
                    // affect the presented read value.
                    state.sniff_data = value;
                    true
                }
                GLOBAL_CHAN_ABORT => {
                    for index in 0..CHANNELS {
                        if value & (1u32 << index) != 0 {
                            state.channels[index].busy = false;
                        }
                    }
                    true
                }
                GLOBAL_FIFO_LEVELS | GLOBAL_N_CHANNELS => true,
                _ => false,
            };
        }
        self.refresh_irq();
        handled
    }

    fn reset(&mut self) {
        self.state.lock().reset();
        self.refresh_irq();
    }

    fn irq_number(&self) -> Option<u16> {
        Some(map::DMA_IRQ_0)
    }
}
