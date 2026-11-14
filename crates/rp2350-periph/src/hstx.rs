//! RP2350 HSTX (high-speed transmit) register, FIFO and command model
//! RP2350 HSTX（高速发送）寄存器、FIFO 与命令模型
//!
//! The block is split across two MMIO windows on silicon, exactly as in the
//! datasheet: the control registers live in APB at [`map::HSTX_CTRL_BASE`]
//! (`0x400C_0000`: `CSR`, `BIT0..BIT7`, `EXPAND_SHIFT`, `EXPAND_TMDS`) and the
//! command/data FIFO lives in AHB at [`map::HSTX_FIFO_BASE`] (`0x5060_0000`:
//! `STAT`, `FIFO`). [`Hstx::ctrl_device`] and [`Hstx::fifo_device`] return the
//! two [`MmioDevice`] adapters, which share one register/FIFO state.
//! 与数据手册一致，该块横跨两个 MMIO 窗口：控制寄存器位于 APB 的
//! [`map::HSTX_CTRL_BASE`]（`0x400C_0000`：`CSR`、`BIT0..BIT7`、
//! `EXPAND_SHIFT`、`EXPAND_TMDS`），命令/数据 FIFO 位于 AHB 的
//! [`map::HSTX_FIFO_BASE`]（`0x5060_0000`：`STAT`、`FIFO`）。
//! [`Hstx::ctrl_device`] 与 [`Hstx::fifo_device`] 返回两个共享同一寄存器/FIFO
//! 状态的 [`MmioDevice`] 适配器。
//!
//! # Modelled subset / 已建模子集
//!
//! - `CSR` reset `0x1005_0600`; `EN` (bit 0) gates FIFO popping and
//!   `EXPAND_EN` (bit 1) selects command expansion. The remaining fields
//!   (`CLKDIV`, `CLKPHASE`, `N_SHIFTS`, `SHIFT`, coupled-mode bits) are stored
//!   and read back through the datasheet write mask.
//! - `BIT0..BIT7` and `EXPAND_SHIFT`/`EXPAND_TMDS` are stored with their
//!   datasheet write masks; `EXPAND_SHIFT.ENC_SHIFT` (bits 20:16) drives the
//!   modelled TMDS word count.
//! - `HSTX_FIFO_STAT` exposes `LEVEL[7:0]`, `FULL`, `EMPTY` and the
//!   write-one-to-clear `WOF` overflow flag.
//! - `HSTX_FIFO_FIFO` is write-only (reads return zero). With `EXPAND_EN = 0`
//!   every written word is raw output data; with `EXPAND_EN = 1` words are
//!   command words followed by their data payload.
//!
//! - `CSR` 复位值 `0x1005_0600`；`EN`（位 0）控制 FIFO 弹出，`EXPAND_EN`
//!   （位 1）选择命令扩展。其余字段（`CLKDIV`、`CLKPHASE`、`N_SHIFTS`、
//!   `SHIFT`、耦合模式位）按数据手册写掩码存储并可回读。
//! - `BIT0..BIT7` 与 `EXPAND_SHIFT`/`EXPAND_TMDS` 按数据手册写掩码存储；
//!   `EXPAND_SHIFT.ENC_SHIFT`（位 20:16）驱动建模的 TMDS 字数换算。
//! - `HSTX_FIFO_STAT` 暴露 `LEVEL[7:0]`、`FULL`、`EMPTY` 与写 1 清零的
//!   `WOF` 溢出标志。
//! - `HSTX_FIFO_FIFO` 只写（读回 0）。`EXPAND_EN = 0` 时每个写入字都是原始
//!   输出数据；`EXPAND_EN = 1` 时写入的是命令字及其数据载荷。
//!
//! # Command encoding / 命令编码
//!
//! Command words use the datasheet layout: opcode in bits `[15:12]`, count in
//! bits `[11:0]`, matching the `HSTX_CMD_*` macros in `pico-examples`.
//! 命令字采用数据手册布局：操作码位于位 `[15:12]`、计数位于位 `[11:0]`，
//! 与 `pico-examples` 的 `HSTX_CMD_*` 宏一致。
//!
//! | Opcode | Constant | Payload / 载荷 |
//! |--------|----------|----------------|
//! | `0x0` | [`CMD_RAW`] | `count` raw 32-bit words / `count` 个原始 32 位字 |
//! | `0x1` | [`CMD_RAW_REPEAT`] | one word, emitted `count` times / 一个字重复 `count` 次 |
//! | `0x2` | [`CMD_TMDS`] | `ceil(count * ENC_SHIFT / 32)` words, passed through / 透传 |
//! | `0x3` | [`CMD_TMDS_REPEAT`] | one word, emitted `count` times / 一个字重复 `count` 次 |
//! | `0xE` | [`CMD_WAIT`] | model extension: wait `count` HSTX cycles / 模型扩展：等待 |
//! | `0xF` | [`CMD_NOP`] | no payload, no output / 无载荷、无输出 |
//!
//! `CMD_WAIT` is **not** a hardware command. The datasheet command set has no
//! delay command, so the model reserves opcode `0xE` (otherwise undefined) and
//! documents it as a modelling subset. A zero count means zero words/cycles for
//! every command, since the datasheet does not define a 4096 encoding here.
//! Unknown opcodes are consumed, counted, logged once per opcode and exposed
//! through [`Hstx::unknown_commands`] / [`Hstx::last_unknown_command`]; they
//! never panic and never raise the completion IRQ.
//! `CMD_WAIT` **不是**硬件命令。数据手册命令集没有延迟命令，因此模型保留
//! 未定义的操作码 `0xE`，并声明其为建模子集。数据手册未定义 4096 编码，故
//! 所有命令的计数 0 都表示 0 个字/周期。未知操作码会被消费、计数、按操作码
//! 只记录一次日志，并通过 [`Hstx::unknown_commands`] /
//! [`Hstx::last_unknown_command`] 暴露；不会 panic，也不会触发完成中断。
//!
//! # Timing model / 时序模型
//!
//! [`Hstx::advance`] pops at most one FIFO word per HSTX clock cycle when
//! `EN` is set, expands it and pushes the resulting output words onto the
//! host-side queue drained by [`Hstx::drain_output`] /
//! [`Hstx::take_scanline`]. It never synthesises pixels, so an idle HSTX only
//! updates status. While a `CMD_WAIT` is pending no FIFO word is consumed.
//! [`Hstx::advance`] 在 `EN` 置位时每个 HSTX 时钟周期至多弹出一个 FIFO 字，
//! 展开后把输出字压入宿主队列，由 [`Hstx::drain_output`] /
//! [`Hstx::take_scanline`] 取走。它从不合成像素，因此空闲 HSTX 只更新状态。
//! `CMD_WAIT` 等待期间不消费 FIFO 字。
//!
//! # Interrupts / 中断
//!
//! Silicon has **no** dedicated HSTX interrupt line, so the model adds two
//! model-only registers in the control window and borrows `SPARE_IRQ_0`
//! ([`map::HSTX_IRQ`] = 46): `IRQ_STATUS` at `+0x30` (write-one-to-clear) and
//! `IRQ_ENABLE` at `+0x34`, with bits [`IRQ_UNDERRUN`], [`IRQ_COMMAND_DONE`]
//! and [`IRQ_UNKNOWN_COMMAND`]. Any enabled status bit asserts the shared line.
//! 芯片没有专用 HSTX 中断线，因此模型在控制窗口增加两个仅模型寄存器并借用
//! `SPARE_IRQ_0`（[`map::HSTX_IRQ`] = 46）：`+0x30` 的 `IRQ_STATUS`（写 1
//! 清零）与 `+0x34` 的 `IRQ_ENABLE`，位为 [`IRQ_UNDERRUN`]、
//! [`IRQ_COMMAND_DONE`] 与 [`IRQ_UNKNOWN_COMMAND`]。任一使能状态位置位即断言
//! 该共享中断线。
//!
//! # Not modelled / 未建模项
//!
//! DVI/HDMI bit timing, the TMDS encoder and 10-bit symbol packing, the output
//! shift register and clock generator, `BITx` pin crossbar routing, GPIO
//! mapping, HSTX DMA handshakes (DREQ) and PIO coupled mode. TMDS command data
//! words are passed through unchanged; only the number of words is derived from
//! `EXPAND_SHIFT`.
//! DVI/HDMI 位时序、TMDS 编码器与 10 位符号打包、输出移位寄存器与时钟发生器、
//! `BITx` 引脚交叉开关、GPIO 映射、HSTX DMA 握手（DREQ）以及 PIO 耦合模式均
//! 未建模。TMDS 命令的数据字原样透传，只有字数由 `EXPAND_SHIFT` 推导。

use std::collections::VecDeque;
use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_core::mmio::{MmioDevice, MmioWindow};

use super::irq::IrqAssert;
use super::map;

/// Depth of the HSTX command/data FIFO in 32-bit words
/// HSTX 命令/数据 FIFO 深度（32 位字）
///
/// The RP2350 HSTX FIFO is 8 entries deep, which is the depth modelled here.
/// RP2350 HSTX FIFO 为 8 项深度，本模型采用同一深度。
pub const FIFO_DEPTH: usize = 8;

/// `HSTX_FIFO_STAT` offset inside the FIFO window / FIFO 窗口中的 `HSTX_FIFO_STAT` 偏移
const FIFO_STAT: u32 = 0x00;

/// `HSTX_FIFO_FIFO` offset inside the FIFO window / FIFO 窗口中的 `HSTX_FIFO_FIFO` 偏移
const FIFO_DATA: u32 = 0x04;

/// `HSTX_FIFO_STAT.LEVEL` field mask / `HSTX_FIFO_STAT.LEVEL` 字段掩码
pub const FIFO_STAT_LEVEL_MASK: u32 = 0x0000_00ff;

/// `HSTX_FIFO_STAT.FULL` — FIFO is full / FIFO 已满
pub const FIFO_STAT_FULL: u32 = 1 << 8;

/// `HSTX_FIFO_STAT.EMPTY` — FIFO is empty / FIFO 为空
pub const FIFO_STAT_EMPTY: u32 = 1 << 9;

/// `HSTX_FIFO_STAT.WOF` — written while full, write-one-to-clear
/// `HSTX_FIFO_STAT.WOF` — 满时写入，写 1 清零
pub const FIFO_STAT_WOF: u32 = 1 << 10;

/// `HSTX_CTRL_CSR` offset / `HSTX_CTRL_CSR` 偏移
const CSR: u32 = 0x00;

/// `HSTX_CTRL_BIT0` offset; `BIT0..BIT7` are contiguous / `HSTX_CTRL_BIT0` 偏移，`BIT0..BIT7` 连续
const BIT0: u32 = 0x04;

/// `HSTX_CTRL_EXPAND_SHIFT` offset / `HSTX_CTRL_EXPAND_SHIFT` 偏移
const EXPAND_SHIFT: u32 = 0x24;

/// `HSTX_CTRL_EXPAND_TMDS` offset / `HSTX_CTRL_EXPAND_TMDS` 偏移
const EXPAND_TMDS: u32 = 0x28;

/// Model-only IRQ status register (write-one-to-clear)
/// 仅模型的 IRQ 状态寄存器（写 1 清零）
const IRQ_STATUS: u32 = 0x30;

/// Model-only IRQ enable register / 仅模型的 IRQ 使能寄存器
const IRQ_ENABLE: u32 = 0x34;

/// `CSR` reset value from the RP2350 register header / RP2350 寄存器头文件中的 `CSR` 复位值
const CSR_RESET: u32 = 0x1005_0600;

/// Writable `CSR` bits from the RP2350 register header / RP2350 寄存器头文件中的 `CSR` 可写位
const CSR_WRITE_MASK: u32 = 0xff1f_1f73;

/// `CSR.EN` — shift data as it appears in the FIFO / 有数据时移位输出
pub const CSR_EN: u32 = 1 << 0;

/// `CSR.EXPAND_EN` — route FIFO words through the command expander
/// `CSR.EXPAND_EN` — FIFO 字经命令扩展器处理
pub const CSR_EXPAND_EN: u32 = 1 << 1;

/// `BIT0..BIT7` writable mask / `BIT0..BIT7` 可写掩码
const BIT_WRITE_MASK: u32 = 0x0003_1f1f;

/// `EXPAND_SHIFT` reset value / `EXPAND_SHIFT` 复位值
const EXPAND_SHIFT_RESET: u32 = 0x0100_0100;

/// `EXPAND_SHIFT` writable mask / `EXPAND_SHIFT` 可写掩码
const EXPAND_SHIFT_WRITE_MASK: u32 = 0x1f1f_1f1f;

/// `EXPAND_TMDS` writable mask / `EXPAND_TMDS` 可写掩码
const EXPAND_TMDS_WRITE_MASK: u32 = 0x00ff_ffff;

/// Command opcode shift (`[15:12]`) / 命令操作码移位（`[15:12]`）
pub const CMD_OPCODE_SHIFT: u32 = 12;

/// Command count field mask (`[11:0]`) / 命令计数字段掩码（`[11:0]`）
pub const CMD_COUNT_MASK: u32 = 0x0000_0fff;

/// Command opcode mask / 命令操作码掩码
const CMD_OPCODE_MASK: u32 = 0x0000_000f;

/// Raw data command (opcode `0x0`) / 原始数据命令（操作码 `0x0`）
pub const CMD_RAW: u32 = 0;

/// Raw data repeat command (opcode `0x1`) / 原始数据重复命令（操作码 `0x1`）
pub const CMD_RAW_REPEAT: u32 = 0x1 << CMD_OPCODE_SHIFT;

/// TMDS data command (opcode `0x2`), data passed through / TMDS 数据命令（操作码 `0x2`），数据透传
pub const CMD_TMDS: u32 = 0x2 << CMD_OPCODE_SHIFT;

/// TMDS repeat command (opcode `0x3`), data passed through / TMDS 重复命令（操作码 `0x3`），数据透传
pub const CMD_TMDS_REPEAT: u32 = 0x3 << CMD_OPCODE_SHIFT;

/// Model-only wait command (opcode `0xE`), count is HSTX clock cycles
/// 仅模型的等待命令（操作码 `0xE`），计数为 HSTX 时钟周期
pub const CMD_WAIT: u32 = 0xe << CMD_OPCODE_SHIFT;

/// No-operation command (opcode `0xF`) / 空操作命令（操作码 `0xF`）
pub const CMD_NOP: u32 = 0xf << CMD_OPCODE_SHIFT;

/// `IRQ_STATUS`/`IRQ_ENABLE` bit: FIFO ran dry mid-command
/// `IRQ_STATUS`/`IRQ_ENABLE` 位：命令执行中 FIFO 断流
pub const IRQ_UNDERRUN: u32 = 1 << 0;

/// `IRQ_STATUS`/`IRQ_ENABLE` bit: a command retired / 位：命令已完成
pub const IRQ_COMMAND_DONE: u32 = 1 << 1;

/// `IRQ_STATUS`/`IRQ_ENABLE` bit: an unknown opcode was consumed
/// 位：消费了未知操作码
pub const IRQ_UNKNOWN_COMMAND: u32 = 1 << 2;

/// Bits implemented by the model-only IRQ registers / 仅模型 IRQ 寄存器实现的位
const IRQ_MASK: u32 = IRQ_UNDERRUN | IRQ_COMMAND_DONE | IRQ_UNKNOWN_COMMAND;

/// State of a command that still needs FIFO data words
/// 仍需 FIFO 数据字的命令状态
#[derive(Debug, Clone, Copy)]
struct Pending {
    /// Data words still to consume / 仍需消费的数据字数
    words: u32,
    /// When `Some(n)`, the single consumed word is emitted `n` times
    /// 为 `Some(n)` 时，单个已消费字输出 `n` 次
    repeat: Option<u32>,
}

/// Shared HSTX register, FIFO and command-expander state
/// 共享的 HSTX 寄存器、FIFO 与命令扩展器状态
#[derive(Debug)]
struct HstxState {
    csr: u32,
    bit: [u32; 8],
    expand_shift: u32,
    expand_tmds: u32,
    fifo: VecDeque<u32>,
    output: VecDeque<u32>,
    pending: Option<Pending>,
    wait_cycles: u64,
    starving: bool,
    wof: bool,
    underruns: u64,
    irq_status: u32,
    irq_enable: u32,
    unknown_commands: u64,
    last_unknown: Option<u32>,
    unknown_opcodes: u16,
}

impl Default for HstxState {
    fn default() -> Self {
        Self {
            csr: CSR_RESET,
            bit: [0; 8],
            expand_shift: EXPAND_SHIFT_RESET,
            expand_tmds: 0,
            fifo: VecDeque::with_capacity(FIFO_DEPTH),
            output: VecDeque::new(),
            pending: None,
            wait_cycles: 0,
            starving: false,
            wof: false,
            underruns: 0,
            irq_status: 0,
            irq_enable: 0,
            unknown_commands: 0,
            last_unknown: None,
            unknown_opcodes: 0,
        }
    }
}

/// Resolve a `BIT0..BIT7` window offset / 解析 `BIT0..BIT7` 窗口偏移
fn bit_index(offset: u32) -> Option<usize> {
    let relative = offset.checked_sub(BIT0)?;
    if relative % 4 != 0 {
        return None;
    }
    let index = (relative / 4) as usize;
    (index < 8).then_some(index)
}

impl HstxState {
    /// FIFO status word derived from the queue / 由队列推导的 FIFO 状态字
    fn fifo_stat(&self) -> u32 {
        let mut stat = self.fifo.len() as u32 & FIFO_STAT_LEVEL_MASK;
        if self.fifo.len() >= FIFO_DEPTH {
            stat |= FIFO_STAT_FULL;
        }
        if self.fifo.is_empty() {
            stat |= FIFO_STAT_EMPTY;
        }
        if self.wof {
            stat |= FIFO_STAT_WOF;
        }
        stat
    }

    /// Whether the enabled IRQ status bits assert the shared line
    /// 已使能的 IRQ 状态位是否断言共享中断线
    fn irq_asserted(&self) -> bool {
        self.irq_status & self.irq_enable & IRQ_MASK != 0
    }

    /// Read a control-window register / 读取控制窗口寄存器
    fn read_ctrl(&self, offset: u32) -> Option<u32> {
        match offset {
            CSR => Some(self.csr),
            EXPAND_SHIFT => Some(self.expand_shift),
            EXPAND_TMDS => Some(self.expand_tmds),
            IRQ_STATUS => Some(self.irq_status),
            IRQ_ENABLE => Some(self.irq_enable),
            _ => bit_index(offset).map(|index| self.bit[index]),
        }
    }

    /// Write a control-window register / 写入控制窗口寄存器
    fn write_ctrl(&mut self, offset: u32, value: u32) -> bool {
        match offset {
            CSR => self.csr = value & CSR_WRITE_MASK,
            EXPAND_SHIFT => self.expand_shift = value & EXPAND_SHIFT_WRITE_MASK,
            EXPAND_TMDS => self.expand_tmds = value & EXPAND_TMDS_WRITE_MASK,
            IRQ_STATUS => self.irq_status &= !(value & IRQ_MASK),
            IRQ_ENABLE => self.irq_enable = value & IRQ_MASK,
            _ => {
                let Some(index) = bit_index(offset) else {
                    return false;
                };
                self.bit[index] = value & BIT_WRITE_MASK;
            }
        }
        true
    }

    /// Read a FIFO-window register / 读取 FIFO 窗口寄存器
    fn read_fifo(&self, offset: u32) -> Option<u32> {
        match offset {
            FIFO_STAT => Some(self.fifo_stat()),
            // `HSTX_FIFO_FIFO` is write-only on silicon.
            // `HSTX_FIFO_FIFO` 在芯片上只写。
            FIFO_DATA => Some(0),
            _ => None,
        }
    }

    /// Write a FIFO-window register / 写入 FIFO 窗口寄存器
    fn write_fifo(&mut self, offset: u32, value: u32) -> bool {
        match offset {
            FIFO_STAT => {
                if value & FIFO_STAT_WOF != 0 {
                    self.wof = false;
                }
            }
            // 满时写仍然被接受，数据丢弃并置位 WOF。
            // A write while full is still accepted; the word is dropped and WOF is set.
            FIFO_DATA => {
                let _ = self.push(value);
            }
            _ => return false,
        }
        true
    }

    /// Push one FIFO word, setting `WOF` when full / 压入一个 FIFO 字，满时置位 `WOF`
    fn push(&mut self, word: u32) -> bool {
        if self.fifo.len() >= FIFO_DEPTH {
            self.wof = true;
            return false;
        }
        self.fifo.push_back(word);
        true
    }

    /// Number of TMDS data words for `count` pixels / `count` 个像素对应的 TMDS 数据字数
    fn tmds_words(&self, count: u32) -> u32 {
        let shift = (self.expand_shift >> 16) & 0x1f;
        if shift == 0 || count == 0 {
            count
        } else {
            (count * shift).div_ceil(32)
        }
    }

    /// Start a non-repeating data command / 启动非重复数据命令
    fn start_data(&mut self, words: u32) -> bool {
        if words == 0 {
            return true;
        }
        self.pending = Some(Pending {
            words,
            repeat: None,
        });
        false
    }

    /// Start a repeating data command / 启动重复数据命令
    fn start_repeat(&mut self, copies: u32) -> bool {
        if copies == 0 {
            return true;
        }
        self.pending = Some(Pending {
            words: 1,
            repeat: Some(copies),
        });
        false
    }

    /// Decode one command word; returns `true` when it retires immediately
    /// 解码一个命令字；立即退休时返回 `true`
    fn decode_command(&mut self, word: u32) -> bool {
        let opcode = (word >> CMD_OPCODE_SHIFT) & CMD_OPCODE_MASK;
        let count = word & CMD_COUNT_MASK;
        match opcode {
            0x0 => self.start_data(count),
            0x1 => self.start_repeat(count),
            0x2 => {
                let words = self.tmds_words(count);
                self.start_data(words)
            }
            0x3 => {
                let copies = self.tmds_words(count);
                self.start_repeat(copies)
            }
            0xe => {
                self.wait_cycles = u64::from(count);
                true
            }
            0xf => true,
            _ => {
                self.note_unknown(word, opcode);
                true
            }
        }
    }

    /// Record an unknown command without failing / 记录未知命令而不失败
    fn note_unknown(&mut self, word: u32, opcode: u32) {
        self.unknown_commands += 1;
        self.last_unknown = Some(word);
        self.irq_status |= IRQ_UNKNOWN_COMMAND;
        let bit = 1u16 << opcode;
        if self.unknown_opcodes & bit == 0 {
            self.unknown_opcodes |= bit;
            log::warn!(
                "HSTX: unknown command opcode {opcode:#X} in word {word:#010X}, ignored / \
                 HSTX：未知命令操作码 {opcode:#X}（命令字 {word:#010X}），已忽略"
            );
        }
    }

    /// Count one FIFO starvation episode / 统计一次 FIFO 断流事件
    fn note_underrun(&mut self) {
        if self.starving {
            return;
        }
        self.starving = true;
        self.underruns += 1;
        self.irq_status |= IRQ_UNDERRUN;
    }

    /// Pop and expand at most one FIFO word / 至多弹出一个 FIFO 字并展开
    fn pump(&mut self) {
        let Some(word) = self.fifo.pop_front() else {
            return;
        };
        if self.csr & CSR_EXPAND_EN == 0 {
            // 扩展器关闭时每个 FIFO 字都是原始输出数据。
            // With the expander off every FIFO word is raw output data.
            self.starving = false;
            self.output.push_back(word);
            return;
        }
        let retired = match self.pending.take() {
            Some(Pending { words, repeat }) => {
                self.starving = false;
                if let Some(copies) = repeat {
                    for _ in 0..copies {
                        self.output.push_back(word);
                    }
                    true
                } else {
                    self.output.push_back(word);
                    if words <= 1 {
                        true
                    } else {
                        self.pending = Some(Pending {
                            words: words - 1,
                            repeat: None,
                        });
                        false
                    }
                }
            }
            None => self.decode_command(word),
        };
        if retired {
            self.irq_status |= IRQ_COMMAND_DONE;
        }
        if self.pending.is_some() && self.fifo.is_empty() {
            self.note_underrun();
        }
    }

    /// Advance the modelled HSTX clock / 推进建模的 HSTX 时钟
    fn advance(&mut self, cycles: u64) {
        if cycles == 0 || self.csr & CSR_EN == 0 {
            return;
        }
        let mut remaining = cycles;
        loop {
            if remaining == 0 {
                break;
            }
            if self.wait_cycles > 0 {
                let burned = self.wait_cycles.min(remaining);
                self.wait_cycles -= burned;
                remaining -= burned;
                continue;
            }
            if self.fifo.is_empty() {
                if self.pending.is_some() {
                    self.note_underrun();
                }
                break;
            }
            self.pump();
            remaining -= 1;
        }
    }
}

/// RP2350 HSTX device handle / RP2350 HSTX 设备句柄
///
/// Clones share the register file, FIFO and command-expander state, so the bus
/// devices installed by [`crate::BootPeripherals::mount`] and the handle
/// returned by [`crate::BootPeripherals::hstx`] always observe the same words.
/// 克隆共享寄存器文件、FIFO 与命令扩展器状态，因此
/// [`crate::BootPeripherals::mount`] 安装的总线设备与
/// [`crate::BootPeripherals::hstx`] 返回的句柄始终看到相同内容。
#[derive(Debug, Clone)]
pub struct Hstx {
    state: Arc<Mutex<HstxState>>,
    irq: IrqAssert,
}

impl Hstx {
    /// Create an HSTX block that asserts `irq` / 创建断言 `irq` 的 HSTX 块
    #[must_use]
    pub fn new(irq: IrqAssert) -> Self {
        Self {
            state: Arc::new(Mutex::new(HstxState::default())),
            irq,
        }
    }

    /// MMIO adapter for the APB control window / APB 控制窗口的 MMIO 适配器
    ///
    /// The adapter is mounted into the APB router at [`map::HSTX_CTRL_BASE`].
    /// 该适配器挂载到 APB 路由器中 [`map::HSTX_CTRL_BASE`] 处。
    #[must_use]
    pub fn ctrl_device(&self) -> Box<dyn MmioDevice> {
        Box::new(CtrlWindow {
            state: Arc::clone(&self.state),
            irq: self.irq.clone(),
        })
    }

    /// MMIO adapter for the AHB FIFO window / AHB FIFO 窗口的 MMIO 适配器
    ///
    /// The adapter is mounted standalone at [`map::HSTX_FIFO_BASE`].
    /// 该适配器独立挂载在 [`map::HSTX_FIFO_BASE`] 处。
    #[must_use]
    pub fn fifo_device(&self) -> Box<dyn MmioDevice> {
        Box::new(FifoWindow {
            state: Arc::clone(&self.state),
            irq: self.irq.clone(),
        })
    }

    /// Push one command word into the FIFO / 向 FIFO 压入一个命令字
    ///
    /// Returns `false` when the 8-entry FIFO is full; the word is dropped and
    /// `HSTX_FIFO_STAT.WOF` is set.
    /// 8 项 FIFO 已满时返回 `false`；该字被丢弃并置位 `HSTX_FIFO_STAT.WOF`。
    pub fn push_command(&mut self, command: u32) -> bool {
        self.state.lock().push(command)
    }

    /// Push one data word into the FIFO / 向 FIFO 压入一个数据字
    ///
    /// With `EXPAND_EN = 0` the word is raw output data; with `EXPAND_EN = 1`
    /// it is the payload of the preceding command.
    /// `EXPAND_EN = 0` 时为原始输出数据；`EXPAND_EN = 1` 时为前一条命令的载荷。
    pub fn push_data(&mut self, word: u32) -> bool {
        self.state.lock().push(word)
    }

    /// Advance the modelled HSTX clock by `cycles` / 按 `cycles` 推进建模的 HSTX 时钟
    ///
    /// One FIFO word is expanded per cycle while `EN` is set. The call does
    /// nothing when `EN` is clear or the FIFO is idle, so it never invents
    /// pixels, and it refreshes the shared interrupt line.
    /// `EN` 置位时每周期展开一个 FIFO 字。`EN` 清零或 FIFO 空闲时不做任何事，
    /// 因此不会凭空产生像素；调用后刷新共享中断线。
    pub fn advance(&self, cycles: u64) {
        let asserted = {
            let mut state = self.state.lock();
            state.advance(cycles);
            state.irq_asserted()
        };
        self.irq.set(map::HSTX_IRQ, asserted);
    }

    /// Drain all output words produced so far / 取走目前产生的全部输出字
    #[must_use]
    pub fn drain_output(&mut self) -> Vec<u32> {
        self.state.lock().output.drain(..).collect()
    }

    /// Drain the host-side output queue as one scanline chunk
    /// 以一条扫描线为单位取走宿主侧输出队列
    ///
    /// This is an alias for [`Hstx::drain_output`] for scanline-oriented hosts;
    /// DVI line boundaries are not modelled, so the caller decides the chunking.
    /// 这是面向扫描线宿主的 [`Hstx::drain_output`] 别名；DVI 行边界未建模，
    /// 由调用方决定分块。
    #[must_use]
    pub fn take_scanline(&mut self) -> Vec<u32> {
        self.drain_output()
    }

    /// Number of recorded output underruns / 已记录的输出下溢次数
    #[must_use]
    pub fn output_underrun(&self) -> u64 {
        self.state.lock().underruns
    }

    /// Current FIFO occupancy in words / 当前 FIFO 占用字数
    #[must_use]
    pub fn fifo_level(&self) -> u8 {
        self.state.lock().fifo.len() as u8
    }

    /// Number of output words waiting for the host / 等待宿主的输出字数
    #[must_use]
    pub fn output_level(&self) -> usize {
        self.state.lock().output.len()
    }

    /// Whether `CSR.EN` is currently set / 当前是否置位 `CSR.EN`
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.state.lock().csr & CSR_EN != 0
    }

    /// Number of consumed unknown command words / 已消费的未知命令字数量
    #[must_use]
    pub fn unknown_commands(&self) -> u64 {
        self.state.lock().unknown_commands
    }

    /// Most recent unknown command word, if any / 最近一个未知命令字（如果有）
    #[must_use]
    pub fn last_unknown_command(&self) -> Option<u32> {
        self.state.lock().last_unknown
    }

    /// Reset registers, FIFOs, counters and the shared IRQ line
    /// 复位寄存器、FIFO、计数器与共享中断线
    pub fn reset(&mut self) {
        self.state.lock().reset();
        self.irq.set(map::HSTX_IRQ, false);
    }
}

impl HstxState {
    /// Restore power-on defaults / 恢复上电默认值
    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// MMIO adapter for the APB control registers / APB 控制寄存器的 MMIO 适配器
struct CtrlWindow {
    state: Arc<Mutex<HstxState>>,
    irq: IrqAssert,
}

impl MmioDevice for CtrlWindow {
    fn name(&self) -> &str {
        "HSTX_CTRL"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::HSTX_CTRL_BASE, map::HSTX_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        self.state.lock().read_ctrl(offset)
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        let accepted = self.state.lock().write_ctrl(offset, value);
        if accepted {
            let asserted = self.state.lock().irq_asserted();
            self.irq.set(map::HSTX_IRQ, asserted);
        }
        accepted
    }

    fn reset(&mut self) {
        self.state.lock().reset();
        self.irq.set(map::HSTX_IRQ, false);
    }

    fn irq_number(&self) -> Option<u16> {
        Some(map::HSTX_IRQ)
    }
}

/// MMIO adapter for the AHB FIFO registers / AHB FIFO 寄存器的 MMIO 适配器
struct FifoWindow {
    state: Arc<Mutex<HstxState>>,
    irq: IrqAssert,
}

impl MmioDevice for FifoWindow {
    fn name(&self) -> &str {
        "HSTX_FIFO"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::HSTX_FIFO_BASE, map::HSTX_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        self.state.lock().read_fifo(offset)
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        let accepted = self.state.lock().write_fifo(offset, value);
        if accepted {
            let asserted = self.state.lock().irq_asserted();
            self.irq.set(map::HSTX_IRQ, asserted);
        }
        accepted
    }

    fn reset(&mut self) {
        self.state.lock().reset();
        self.irq.set(map::HSTX_IRQ, false);
    }

    fn irq_number(&self) -> Option<u16> {
        Some(map::HSTX_IRQ)
    }
}
