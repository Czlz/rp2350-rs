//! PIO shift-register instructions: IN/OUT/PUSH/PULL/MOV
//! PIO 移位寄存器指令：IN/OUT/PUSH/PULL/MOV
//!
//! Autopush/autopull thresholds follow `SHIFTCTRL.PUSH_THRESH` (bits shifted
//! into the ISR) and `SHIFTCTRL.PULL_THRESH` (bits left in the OSR). The ISR
//! and OSR start empty, matching the datasheet reset state.
//! 自动压入/拉取阈值遵循 `SHIFTCTRL.PUSH_THRESH`（ISR 已移入位数）与
//! `SHIFTCTRL.PULL_THRESH`（OSR 剩余位数）。ISR/OSR 初始为空，与数据手册复位
//! 状态一致。

use super::regs::{FDEBUG_RXSTALL, FDEBUG_TXSTALL, StateMachine};
use super::sm::{Exec, SmBus};

/// Instruction bit count: 0 encodes 32 / 指令位计数：0 表示 32
fn bit_count(arg: u8) -> u32 {
    let raw = u32::from(arg & 0x1F);
    if raw == 0 { 32 } else { raw }
}

/// Low `bits` ones / 低 `bits` 位全 1
fn low_mask(bits: u32) -> u32 {
    if bits >= 32 {
        u32::MAX
    } else {
        (1u32 << bits) - 1
    }
}

impl StateMachine {
    /// `IN source, bit count` / `IN source, bit count`
    pub(super) fn exec_in(&mut self, arg: u8, bus: &mut SmBus<'_>) -> Exec {
        let bits = bit_count(arg);
        let value = match (arg >> 5) & 0x07 {
            1 => self.x,
            2 => self.y,
            5 => self.status(),
            6 => self.isr,
            7 => self.osr,
            // 0 = PINS（未接引脚恒为 0）、3 = NULL、4 = 保留。
            // 0 = PINS (always 0 without pins), 3 = NULL, 4 = reserved.
            _ => 0,
        } & self.in_count_mask();
        let shifted = (self.isr_bits + bits as u8).min(32);
        // 自动压入会写满 RX FIFO 时先阻塞，避免重试时重复移位。
        // Stall before shifting when autopush would need a full RX FIFO, so a
        // retry cannot shift the same instruction twice.
        if self.autopush() && shifted >= self.push_threshold() && self.rx.is_full() {
            bus.set_flag(FDEBUG_RXSTALL, self.index);
            return Exec::Stall;
        }
        self.isr = self.shifted_in(value, bits);
        self.isr_bits = shifted;
        if self.autopush() && self.isr_bits >= self.push_threshold() {
            self.rx.push(self.isr);
            self.isr = 0;
            self.isr_bits = 0;
        }
        Exec::Advance
    }

    /// `OUT destination, bit count` / `OUT destination, bit count`
    pub(super) fn exec_out(&mut self, arg: u8, bus: &mut SmBus<'_>, depth: u8) -> Exec {
        let bits = bit_count(arg);
        if self.autopull() && self.osr_bits <= self.empty_level() {
            if let Some(word) = self.tx.pop() {
                self.osr = word;
                self.osr_bits = 32;
            } else {
                bus.set_flag(FDEBUG_TXSTALL, self.index);
                return Exec::Stall;
            }
        }
        let value = self.shifted_out(bits);
        self.osr_bits = self.osr_bits.saturating_sub(bits as u8);
        match (arg >> 5) & 0x07 {
            1 => self.x = value,
            2 => self.y = value,
            // 3 = NULL、4 = PINDIRS（未接引脚，写入丢弃）。
            // 3 = NULL, 4 = PINDIRS (no pins attached, write discarded).
            5 => return Exec::Branch((value & 0x1F) as u8),
            6 => {
                self.isr = value;
                self.isr_bits = bits as u8;
            }
            7 => return self.exec_nested(value, bus, depth),
            _ => {}
        }
        Exec::Advance
    }

    /// `PUSH`/`PULL` with `block`, `iffull`/`ifempty` and `noerr`
    /// 带 `block`、`iffull`/`ifempty` 与 `noerr` 的 `PUSH`/`PULL`
    pub(super) fn exec_push_pull(&mut self, arg: u8, bus: &mut SmBus<'_>) -> Exec {
        let block = arg & 0x20 != 0;
        let conditional = arg & 0x40 != 0;
        // noerr：FIFO 空/满时不置位粘滞调试标志。noerr: skip the sticky debug flag.
        let noerr = arg & 0x10 != 0;
        if arg & 0x80 != 0 {
            if conditional && self.osr_bits > self.empty_level() {
                return Exec::Advance;
            }
            if let Some(word) = self.tx.pop() {
                self.osr = word;
                self.osr_bits = 32;
                Exec::Advance
            } else {
                if !noerr {
                    bus.set_flag(FDEBUG_TXSTALL, self.index);
                }
                if block {
                    return Exec::Stall;
                }
                // 非阻塞 PULL 在空 FIFO 时把 X 装入 OSR。
                // A non-blocking PULL loads X into the OSR when the FIFO is empty.
                self.osr = self.x;
                self.osr_bits = 32;
                Exec::Advance
            }
        } else {
            if conditional && self.isr_bits < self.push_threshold() {
                return Exec::Advance;
            }
            if self.rx.is_full() {
                if !noerr {
                    bus.set_flag(FDEBUG_RXSTALL, self.index);
                }
                if block {
                    return Exec::Stall;
                }
            } else {
                self.rx.push(self.isr);
            }
            self.isr = 0;
            self.isr_bits = 0;
            Exec::Advance
        }
    }

    /// `MOV destination, (op) source` / `MOV destination, (op) source`
    pub(super) fn exec_mov(&mut self, arg: u8, bus: &mut SmBus<'_>, depth: u8) -> Exec {
        let mut value = match arg & 0x07 {
            1 => self.x,
            2 => self.y,
            // 5 = STATUS，其余为 PINS/NULL/保留（读 0）。
            // 5 = STATUS; PINS/NULL/reserved read 0.
            5 => self.status(),
            6 => self.isr,
            7 => self.osr,
            _ => 0,
        };
        match (arg >> 3) & 0x03 {
            1 => value = !value,
            2 => value = value.reverse_bits(),
            _ => {}
        }
        match (arg >> 5) & 0x07 {
            1 => self.x = value,
            2 => self.y = value,
            4 => return self.exec_nested(value, bus, depth),
            5 => return Exec::Branch((value & 0x1F) as u8),
            6 => {
                self.isr = value;
                // MOV ISR 复位输入移位计数，即视为空。MOV ISR clears the ISR counter (empty).
                self.isr_bits = 0;
            }
            7 => {
                self.osr = value;
                // MOV OSR 复位输出移位计数，即视为满。MOV OSR reloads the OSR counter (full).
                self.osr_bits = 32;
            }
            _ => {}
        }
        Exec::Advance
    }

    /// Execute an instruction supplied by `EXEC`/`MOV EXEC`
    /// 执行由 `EXEC`/`MOV EXEC` 提供的指令
    fn exec_nested(&mut self, value: u32, bus: &mut SmBus<'_>, depth: u8) -> Exec {
        if depth > 0 {
            // 只允许一层嵌套，内层 EXEC 忽略。Only one nesting level is allowed.
            return Exec::Advance;
        }
        // 嵌套指令阻塞时按已退休处理，避免外层指令重复执行产生二次副作用。
        // A stalling nested instruction retires here so the outer instruction
        // cannot run its side effects twice.
        match self.execute((value & 0xFFFF) as u16, bus, 1) {
            Exec::Stall => Exec::Advance,
            other => other,
        }
    }

    /// Shift `value` into the ISR and return the new ISR
    /// 将 `value` 移入 ISR 并返回新的 ISR
    fn shifted_in(&self, value: u32, bits: u32) -> u32 {
        if bits >= 32 {
            return value;
        }
        let mask = low_mask(bits);
        if self.in_shift_right() {
            (self.isr >> bits) | ((value & mask) << (32 - bits))
        } else {
            (self.isr << bits) | (value & mask)
        }
    }

    /// Shift `bits` out of the OSR and return them right-aligned
    /// 从 OSR 移出 `bits` 位并按低位对齐返回
    fn shifted_out(&mut self, bits: u32) -> u32 {
        if bits >= 32 {
            let value = self.osr;
            self.osr = 0;
            return value;
        }
        if self.out_shift_right() {
            let value = self.osr & low_mask(bits);
            self.osr >>= bits;
            value
        } else {
            let value = self.osr >> (32 - bits);
            self.osr <<= bits;
            value
        }
    }
}
