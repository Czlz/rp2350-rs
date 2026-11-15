//! PIO register block and state-machine execution / PIO 寄存器块与状态机执行
//!
//! RP2350 has three PIO blocks (`PIO0`/`PIO1`/`PIO2`) at `0x5020_0000`,
//! `0x5030_0000` and `0x5040_0000`. This module models the register file, the
//! 32-word instruction memory, the four state-machine register windows, the
//! 4-deep TX/RX FIFOs (joinable into 8-deep FIFOs), the GPIO pin mapping with
//! side-set and delay cycles, the 16.8 fixed-point clock dividers and the PIO
//! instruction set (JMP/WAIT/IN/OUT/PUSH/PULL/MOV/IRQ/SET). The FIFOs expose
//! live empty/full queries that the host publishes onto the block's DMA DREQ
//! selectors ([`crate::BootPeripherals::sync_pio_dreqs`]); no FIFO state is
//! copied. Processor interrupt routing is provided by [`SharedPio`], which maps
//! the eight `IRQ` flags onto the block's two shared `PIOx_IRQ_n` lines.
//! TX FIFO pushes can optionally be captured into an [`crate::AudioTap`] that a
//! host installs on one state machine with [`Pio::set_audio_tap`]; without a tap
//! the push path only performs one `Option` check.
//! RP2350 有三个 PIO 块（`PIO0`/`PIO1`/`PIO2`），分别位于 `0x5020_0000`、
//! `0x5030_0000` 与 `0x5040_0000`。本模块建模寄存器文件、32 字指令存储、四个
//! 状态机寄存器窗口、4 深度 TX/RX FIFO（可合并为 8 深度）、GPIO 引脚映射与
//! side-set/延迟周期、16.8 定点分频器以及 PIO 指令集（JMP/WAIT/IN/OUT/PUSH/
//! PULL/MOV/IRQ/SET）。FIFO 暴露实时的空/满查询，由宿主发布到本块的 DMA DREQ
//! 选择器（[`crate::BootPeripherals::sync_pio_dreqs`]），不复制任何 FIFO 状态。
//! 处理器中断路由由 [`SharedPio`] 提供，它把八个 `IRQ` 标志映射到本块的两条共享
//! `PIOx_IRQ_n` 线。
//! TX FIFO 压入可选择被宿主通过 [`Pio::set_audio_tap`] 安装到某个状态机的
//! [`crate::AudioTap`] 捕获；未安装 tap 时压入路径只做一次 `Option` 判断。
//!
//! Reset values are taken from the RP2350 register headers (`PIO_*_RESET`):
//! `FSTAT = 0x0F00_0F00`, `SMx_CLKDIV = 0x0001_0000`,
//! `SMx_EXECCTRL = 0x0001_F000`, `SMx_SHIFTCTRL = 0x000C_0000`,
//! `SMx_PINCTRL = 0x1400_0000` and `DBG_CFGINFO = 0x1020_0404`.
//! 复位值取自 RP2350 寄存器头文件（`PIO_*_RESET`）：
//! `FSTAT = 0x0F00_0F00`、`SMx_CLKDIV = 0x0001_0000`、
//! `SMx_EXECCTRL = 0x0001_F000`、`SMx_SHIFTCTRL = 0x000C_0000`、
//! `SMx_PINCTRL = 0x1400_0000`、`DBG_CFGINFO = 0x1020_0404`。

#[cfg(test)]
mod dreq_tests;
#[cfg(test)]
mod pin_tests;
mod regs;
mod shared;
#[cfg(test)]
mod shared_tests;
mod shift;
mod sm;
#[cfg(test)]
mod sm_tests;
#[cfg(test)]
mod tests;

use rp2350_core::mmio::{MmioDevice, MmioWindow};

use super::gpio_pads::GpioPads;
use super::io_bank0::PinFunction;
use super::map;
use crate::alias::{apply_alias, split_alias};
use crate::audio::AudioTap;
// PIO 寄存器常量沿用数据手册命名并集中平铺，通配导入让寄存器映射保持直观。
// PIO register constants follow datasheet names and are declared in one place, so
// a glob import keeps the register map readable.
#[allow(clippy::wildcard_imports)]
use regs::*;

pub use shared::SharedPio;

/// Number of RP2350 PIO blocks / RP2350 的 PIO 块数量
pub const PIO_BLOCKS: usize = 3;

/// PIO register window including the SET/CLR/XOR atomic aliases (16 KB)
/// PIO 寄存器窗口（含 SET/CLR/XOR 原子别名，共 16 KB）
pub(super) const PIO_WINDOW_SIZE: usize = 0x4000;

/// `SMx_CLKDIV` clock-restart stroke / `SMx_CLKDIV` 时钟重启脉冲位
const CLKDIV_RESTART_BIT: u32 = 1 << 31;

/// Number of state machines in one PIO block / 一个 PIO 块中的状态机数量
pub const PIO_SM_COUNT: usize = regs::SM_COUNT;

/// One RP2350 PIO register block / 一个 RP2350 PIO 寄存器块
///
/// The block exposes the generic registers, the instruction memory and the
/// four state-machine register windows through [`MmioDevice`]. Writing
/// `SMx_INSTR` executes an instruction immediately and `SMx_ADDR` reports the
/// current program counter. Call [`Pio::tick`] (or [`Pio::step_sm`]) to run the
/// state machines one instruction at a time. Hosts that need the block mounted
/// on a bus while still driving it use [`SharedPio`], which owns one of these
/// register files behind a shared handle.
/// 该块通过 [`MmioDevice`] 暴露通用寄存器、指令存储与四个状态机寄存器窗口。
/// 写 `SMx_INSTR` 会立即执行一条指令，`SMx_ADDR` 报告当前程序计数器。调用
/// [`Pio::tick`]（或 [`Pio::step_sm`]）可按“每次一条指令”的方式驱动状态机。
/// 需要在挂载到总线的同时继续驱动它的宿主应使用 [`SharedPio`]，它用共享句柄
/// 持有其中一个寄存器文件。
#[derive(Debug)]
pub struct Pio {
    index: u8,
    base: u32,
    enabled: u8,
    fdebug: u32,
    irq: u32,
    input_sync_bypass: u32,
    gpio_base: u32,
    /// GPIO pin levels sampled by `IN PINS`, `WAIT` and `JMP PIN`
    /// `IN PINS`、`WAIT` 与 `JMP PIN` 采样的 GPIO 引脚电平
    ///
    /// After [`Pio::sample_inputs`] this is the routed pad sample OR-ed with the
    /// host override; without a pads handle it is the host vector verbatim.
    /// 调用 [`Pio::sample_inputs`] 后，该值是路由后的 pad 采样与宿主覆盖的按位或；
    /// 未接 pad 矩阵时就是宿主向量本身。
    input_levels: u32,
    /// Host-injected override vector / 宿主注入的覆盖向量
    host_inputs: u32,
    /// Shared pad matrix; `None` for a standalone block / 共享 pad 矩阵；独立块为 `None`
    pads: Option<GpioPads>,
    /// Optional I2S capture tap and the state machine whose TX FIFO it observes
    /// 可选的 I2S 捕获 tap，以及它观察的状态机
    ///
    /// Only `push_tx` for the selected state machine feeds the tap; every other
    /// state machine and every block without a tap pays one `Option` check.
    /// 只有被选中状态机的 `push_tx` 会写入该 tap；其它状态机以及未安装 tap 的块
    /// 只承担一次 `Option` 判断。
    audio: Option<(AudioTap, usize)>,
    /// Output levels driven by `OUT`/`SET`/side-set / `OUT`/`SET`/side-set 驱动的输出电平
    output_levels: u32,
    /// Output enables driven by `OUT PINDIRS`/`SET PINDIRS`/side-set
    /// `OUT PINDIRS`/`SET PINDIRS`/side-set 驱动的输出使能
    output_enables: u32,
    imem: [u16; IMEM_WORDS],
    sm: [StateMachine; SM_COUNT],
}

impl Pio {
    /// Create a PIO block by index (`0`..=`2`) / 按序号（`0`..=`2`）创建 PIO 块
    ///
    /// # Panics
    /// Panics when `index` is not a valid RP2350 PIO block index.
    /// 当 `index` 不是合法的 RP2350 PIO 块序号时 panic。
    #[must_use]
    pub fn new(index: u8) -> Self {
        let base = Self::block_base(index);
        Self {
            index,
            base,
            enabled: 0,
            fdebug: 0,
            irq: 0,
            input_sync_bypass: 0,
            gpio_base: 0,
            input_levels: 0,
            host_inputs: 0,
            pads: None,
            audio: None,
            output_levels: 0,
            output_enables: 0,
            imem: [0; IMEM_WORDS],
            sm: core::array::from_fn(|slot| StateMachine::new(slot as u8)),
        }
    }

    /// Create a block wired to the shared pin matrix / 创建接到共享引脚矩阵的块
    ///
    /// The block then samples only pads whose `IO_BANK0.FUNCSEL` selects it
    /// (`PIO0`/`PIO1`/`PIO2`) and publishes its driven levels back into the same
    /// matrix, so PIO ↔ pad traffic follows the `FUNCSEL` crossbar.
    /// 该块此后只采样 `IO_BANK0.FUNCSEL` 选中它（`PIO0`/`PIO1`/`PIO2`）的 pad，并
    /// 把驱动电平发布回同一矩阵，因此 PIO ↔ pad 的流量遵循 `FUNCSEL` 交叉开关。
    #[must_use]
    pub fn with_pads(index: u8, pads: GpioPads) -> Self {
        let mut block = Self::new(index);
        block.pads = Some(pads);
        block
    }

    /// `FUNCSEL` value that routes a pad to this block / 把 pad 路由到本块的 `FUNCSEL` 值
    fn selector(&self) -> u32 {
        PinFunction::Pio0.to_bits() + u32::from(self.index)
    }

    /// Register base address of a block index / 块序号的寄存器基地址
    ///
    /// # Panics
    /// Panics when `index` is not a valid RP2350 PIO block index.
    /// 当 `index` 不是合法的 RP2350 PIO 块序号时 panic。
    pub(super) fn block_base(index: u8) -> u32 {
        match index {
            0 => map::PIO0_BASE,
            1 => map::PIO1_BASE,
            2 => map::PIO2_BASE,
            _ => panic!("PIO index must be 0, 1 or 2"),
        }
    }

    /// Device name of a block index / 块序号的设备名
    pub(super) fn block_name(index: u8) -> &'static str {
        match index {
            0 => "PIO0",
            1 => "PIO1",
            _ => "PIO2",
        }
    }

    /// Decode an offset into a state-machine index and window register
    /// 将偏移解析为状态机序号与窗口内寄存器
    fn sm_register(offset: u32) -> Option<(usize, u32)> {
        let relative = offset.checked_sub(SM0_CLKDIV)?;
        let index = (relative / SM_STRIDE) as usize;
        let register = relative % SM_STRIDE;
        if index >= SM_COUNT
            || !matches!(
                register,
                SM_CLKDIV | SM_EXECCTRL | SM_SHIFTCTRL | SM_ADDR | SM_INSTR | SM_PINCTRL
            )
        {
            return None;
        }
        Some((index, register))
    }

    /// Compute `FSTAT` from the current FIFO occupancy / 由当前 FIFO 占用计算 `FSTAT`
    fn fstat(&self) -> u32 {
        let mut status = 0;
        for (index, machine) in self.sm.iter().enumerate() {
            if machine.rx.is_full() {
                status |= 1 << index;
            }
            if machine.rx.is_empty() {
                status |= 1 << (8 + index);
            }
            if machine.tx.is_full() {
                status |= 1 << (16 + index);
            }
            if machine.tx.is_empty() {
                status |= 1 << (24 + index);
            }
        }
        status
    }

    /// Compute `FLEVEL` from the current FIFO occupancy / 由当前 FIFO 占用计算 `FLEVEL`
    fn flevel(&self) -> u32 {
        let mut level = 0;
        for (index, machine) in self.sm.iter().enumerate() {
            level |= (machine.tx.level() as u32) << (index * 8);
            level |= (machine.rx.level() as u32) << (index * 8 + 4);
        }
        level
    }

    /// Read one state-machine window register / 读取一个状态机窗口寄存器
    fn read_sm(&self, index: usize, register: u32) -> u32 {
        let machine = &self.sm[index];
        match register {
            SM_CLKDIV => machine.clkdiv,
            SM_EXECCTRL => machine.execctrl,
            SM_SHIFTCTRL => machine.shiftctrl,
            SM_ADDR => u32::from(machine.pc),
            SM_INSTR => u32::from(self.imem[usize::from(machine.pc)]),
            SM_PINCTRL => machine.pinctrl,
            _ => 0,
        }
    }

    /// Write one state-machine window register / 写入一个状态机窗口寄存器
    fn write_sm(&mut self, index: usize, register: u32, value: u32) -> bool {
        match register {
            SM_CLKDIV => {
                // Bit 31 is a self-clearing clock-restart stroke, not storage.
                // 位 31 是自清零的时钟重启脉冲，不属于存储值。
                if value & CLKDIV_RESTART_BIT != 0 {
                    self.sm[index].clock_phase = 0;
                }
                self.sm[index].clkdiv = value & CLKDIV_MASK & !CLKDIV_RESTART_BIT;
            }
            SM_EXECCTRL => {
                // EXEC_STALLED 为只读状态位，不受写入影响。
                // EXEC_STALLED is a read-only status bit and survives writes.
                let stalled = self.sm[index].execctrl & EXECCTRL_EXEC_STALLED;
                self.sm[index].execctrl = stalled | (value & EXECCTRL_WRITE_MASK);
            }
            SM_SHIFTCTRL => self.sm[index].write_shiftctrl(value),
            // `SMx_ADDR` 只读；`SMx_INSTR` 写入立即执行一条指令。
            // `SMx_ADDR` is read-only; writing `SMx_INSTR` executes immediately.
            SM_ADDR => {}
            SM_INSTR => self.execute_injected(index, (value & INSTR_MASK) as u16),
            SM_PINCTRL => self.sm[index].pinctrl = value,
            _ => return false,
        }
        true
    }

    /// Push a word into the state machine RX FIFO from the device side
    /// 从设备侧向状态机 RX FIFO 压入一个字
    ///
    /// Returns `false` when the FIFO is full or `index` is out of range.
    /// FIFO 已满或 `index` 越界时返回 `false`。
    pub fn push_rx(&mut self, index: usize, value: u32) -> bool {
        let Some(machine) = self.sm.get_mut(index) else {
            return false;
        };
        let pushed = machine.rx.push(value);
        if !pushed {
            self.fdebug |= FDEBUG_RXSTALL << index;
        }
        pushed
    }

    /// Push a word into the state machine TX FIFO from the device side
    /// 从设备侧向状态机 TX FIFO 压入一个字
    ///
    /// This is the single entry point for both the CPU (`TXF0..TXF3` writes) and
    /// the DMA controller (bus writes to the same offsets), so an installed
    /// [`AudioTap`] observes every accepted word without touching either path.
    /// Returns `false` when the FIFO is full or `index` is out of range; a
    /// rejected word sets the sticky `FDEBUG.TXOVER` flag and is not captured.
    /// 这是 CPU（写 `TXF0..TXF3`）与 DMA 控制器（总线写同一组偏移）共同的入口，
    /// 因此已安装的 [`AudioTap`] 无需改动任何一条路径即可观察到每个被接受的字。
    /// FIFO 已满或 `index` 越界时返回 `false`；被拒绝的字会置位粘滞的
    /// `FDEBUG.TXOVER` 标志且不会被捕获。
    pub fn push_tx(&mut self, index: usize, value: u32) -> bool {
        let Some(machine) = self.sm.get_mut(index) else {
            return false;
        };
        if !machine.tx.push(value) {
            self.fdebug |= FDEBUG_TXOVER << index;
            return false;
        }
        // 只有被 tap 选中的状态机才写入样本流；未安装 tap 时这里只是空判断。
        // Only the state machine selected by the tap feeds the sample stream;
        // with no tap installed this is an empty check.
        if let Some((tap, sm)) = &self.audio {
            if *sm == index {
                tap.push_word(value);
            }
        }
        true
    }

    /// Install an I2S capture tap on one state machine / 在一个状态机上安装 I2S 捕获 tap
    ///
    /// Every word accepted into state machine `sm`'s TX FIFO is then appended to
    /// `tap`, whether it arrives from the CPU or from DMA. Only one state machine
    /// per block is captured; installing a new tap replaces the previous one.
    /// A block with no tap installed captures nothing and performs no extra work
    /// on the push path.
    /// 此后状态机 `sm` 的 TX FIFO 接受的每个字都会追加到 `tap`，无论它来自 CPU
    /// 还是 DMA。每个块只捕获一个状态机；安装新 tap 会替换旧的。未安装 tap 的块
    /// 不捕获任何数据，压入路径也没有额外工作。
    pub fn set_audio_tap(&mut self, tap: AudioTap, sm: usize) {
        self.audio = Some((tap, sm));
    }

    /// Remove the installed capture tap / 移除已安装的捕获 tap
    ///
    /// TX FIFO pushes are then not captured at all; the sink itself is left
    /// untouched so the host can still drain what was captured before.
    /// 此后 TX FIFO 压入完全不被捕获；缓冲本身保持不变，宿主仍可取走此前捕获的内容。
    pub fn clear_audio_tap(&mut self) {
        self.audio = None;
    }

    /// Capture tap currently installed on this block / 本块当前安装的捕获 tap
    #[must_use]
    pub fn audio_tap(&self) -> Option<&AudioTap> {
        self.audio.as_ref().map(|(tap, _)| tap)
    }

    /// Pop a word from the state machine TX FIFO as the state machine would
    /// 按状态机的行为从 TX FIFO 取出一个字
    ///
    /// Returns `None` when the FIFO is empty or `index` is out of range.
    /// FIFO 为空或 `index` 越界时返回 `None`。
    pub fn pop_tx(&mut self, index: usize) -> Option<u32> {
        let machine = self.sm.get_mut(index)?;
        let word = machine.tx.pop();
        if word.is_none() {
            self.fdebug |= FDEBUG_TXSTALL << index;
        }
        word
    }

    /// Whether state machine `sm`'s TX FIFO is empty / 状态机 `sm` 的 TX FIFO 是否为空
    ///
    /// Read-only view of the live FIFO, so no state is copied. A direction
    /// disabled by FIFO joining reports both empty and full, exactly as
    /// `FSTAT` does. Returns `None` for `sm >= 4`.
    /// 实时 FIFO 的只读视图，不复制状态。被 FIFO 合并禁用的一侧同时报告为空与满，
    /// 与 `FSTAT` 一致。`sm >= 4` 时返回 `None`。
    #[must_use]
    pub fn tx_fifo_empty(&self, sm: usize) -> Option<bool> {
        self.sm.get(sm).map(|machine| machine.tx.is_empty())
    }

    /// Whether state machine `sm`'s TX FIFO is full / 状态机 `sm` 的 TX FIFO 是否已满
    ///
    /// A joined TX FIFO is 8 words deep; a TX FIFO disabled by `FJOIN_RX` always
    /// reads full. Returns `None` for `sm >= 4`.
    /// 合并后的 TX FIFO 深度为 8 字；被 `FJOIN_RX` 禁用的 TX FIFO 始终读为满。
    /// `sm >= 4` 时返回 `None`。
    #[must_use]
    pub fn tx_fifo_full(&self, sm: usize) -> Option<bool> {
        self.sm.get(sm).map(|machine| machine.tx.is_full())
    }

    /// Whether state machine `sm`'s RX FIFO is empty / 状态机 `sm` 的 RX FIFO 是否为空
    ///
    /// A joined RX FIFO is 8 words deep; an RX FIFO disabled by `FJOIN_TX` always
    /// reads empty. Returns `None` for `sm >= 4`.
    /// 合并后的 RX FIFO 深度为 8 字；被 `FJOIN_TX` 禁用的 RX FIFO 始终读为空。
    /// `sm >= 4` 时返回 `None`。
    #[must_use]
    pub fn rx_fifo_empty(&self, sm: usize) -> Option<bool> {
        self.sm.get(sm).map(|machine| machine.rx.is_empty())
    }

    /// Whether state machine `sm`'s RX FIFO is full / 状态机 `sm` 的 RX FIFO 是否已满
    ///
    /// A direction disabled by FIFO joining reports both full and empty.
    /// Returns `None` for `sm >= 4`.
    /// 被 FIFO 合并禁用的一侧同时报告为满与空。`sm >= 4` 时返回 `None`。
    #[must_use]
    pub fn rx_fifo_full(&self, sm: usize) -> Option<bool> {
        self.sm.get(sm).map(|machine| machine.rx.is_full())
    }

    /// Set all 32 host-side pin levels at once / 一次设置全部 32 个宿主侧引脚电平
    ///
    /// Bit `n` is OR-ed into the sampled vector for PIO pin `n` as a host-side
    /// external override; pin levels that the `FUNCSEL` crossbar routes from the
    /// shared pads keep participating. A block created without a pads handle
    /// samples this vector verbatim, which preserves the earlier "no pins
    /// attached" host injection.
    /// 位 `n` 作为宿主侧外部覆盖，按位或进 PIO 引脚 `n` 的采样向量；由 `FUNCSEL`
    /// 交叉开关从共享 pad 路由来的电平仍然参与。未接 pad 矩阵的块直接使用该向量，
    /// 保持早期「未接引脚」的宿主注入语义。
    pub fn set_input_levels(&mut self, levels: u32) {
        self.host_inputs = levels;
        self.sample_inputs();
    }

    /// Levels the state machines currently sample / 状态机当前采样的电平
    ///
    /// Without a pads handle this is exactly the vector set by
    /// [`Pio::set_input_levels`]; with pads it is the routed pad sample OR-ed
    /// with that host override (see [`Pio::sample_inputs`]).
    /// 未接 pad 矩阵时就是 [`Pio::set_input_levels`] 设置的向量；接了 pad 时是
    /// 路由后的 pad 采样与该宿主覆盖的按位或（见 [`Pio::sample_inputs`]）。
    #[must_use]
    pub fn input_levels(&self) -> u32 {
        self.input_levels
    }

    /// Set one host-side pin level; pins `>= 32` are ignored
    /// 设置单个宿主侧引脚电平；`>= 32` 的引脚被忽略
    pub fn set_input_level(&mut self, pin: u8, high: bool) {
        if pin >= 32 {
            return;
        }
        let bit = 1u32 << pin;
        if high {
            self.host_inputs |= bit;
        } else {
            self.host_inputs &= !bit;
        }
        self.sample_inputs();
    }

    /// Resample the pin vector from the shared pads / 从共享 pad 重新采样引脚向量
    ///
    /// Only pads whose `IO_BANK0.FUNCSEL` selects this block contribute, offset
    /// by `GPIOBASE`; every other pin reads `0` unless the host injected it.
    /// [`crate::pio::SharedPio::tick`] calls this before every batch of state
    /// machine cycles, which is the host tick boundary.
    /// 只有 `IO_BANK0.FUNCSEL` 选中本块的 pad 参与采样（按 `GPIOBASE` 偏移），其它
    /// 引脚读 `0`（除非宿主注入）。[`crate::pio::SharedPio::tick`] 在每个状态机批次
    /// 之前调用，也就是宿主 tick 边界。
    pub fn sample_inputs(&mut self) {
        self.input_levels = match &self.pads {
            Some(pads) => pads.sample_for(self.selector(), self.gpio_base) | self.host_inputs,
            None => self.host_inputs,
        };
    }

    /// Levels the state machines currently sample after routing
    /// 状态机当前采样的（路由后）电平
    ///
    /// Same value as [`Pio::input_levels`]; the explicit name documents that the
    /// `FUNCSEL` crossbar has already been applied.
    /// 与 [`Pio::input_levels`] 相同；该名字强调 `FUNCSEL` 交叉开关已生效。
    #[must_use]
    pub fn sampled_inputs(&self) -> u32 {
        self.input_levels
    }

    /// Publish the driven levels and enables into the shared pin matrix
    /// 把驱动电平与输出使能发布到共享引脚矩阵
    ///
    /// `GPIOBASE` shifts the local 0..31 pin window onto the real GPIO
    /// numbers; the matrix only applies the contribution to pads whose
    /// `FUNCSEL` selects this block. A standalone block has nothing to publish.
    /// `GPIOBASE` 把局部 0..31 引脚窗口平移到真实 GPIO 编号；矩阵只在
    /// `FUNCSEL` 选中本块的 pad 上应用该贡献。独立块无发布目标。
    pub fn publish_outputs(&self) {
        let Some(pads) = &self.pads else {
            return;
        };
        pads.set_driver(
            self.selector(),
            u64::from(self.output_levels) << self.gpio_base,
            u64::from(self.output_enables) << self.gpio_base,
        );
    }

    /// Output levels currently driven by the state machines / 状态机当前驱动的输出电平
    ///
    /// Updated by `OUT PINS`, `SET PINS`, `MOV PINS` and side-set, and mirrored
    /// in `DBG_PADOUT`.
    /// 由 `OUT PINS`、`SET PINS`、`MOV PINS` 与 side-set 更新，并反映在 `DBG_PADOUT`。
    #[must_use]
    pub fn output_levels(&self) -> u32 {
        self.output_levels
    }

    /// Output enables currently driven by the state machines / 状态机当前驱动的输出使能
    ///
    /// Updated by `OUT PINDIRS`, `SET PINDIRS` and side-set with
    /// `EXECCTRL.SIDE_PINDIR`, and mirrored in `DBG_PADOE`.
    /// 由 `OUT PINDIRS`、`SET PINDIRS` 以及带 `EXECCTRL.SIDE_PINDIR` 的 side-set
    /// 更新，并反映在 `DBG_PADOE`。
    #[must_use]
    pub fn output_enables(&self) -> u32 {
        self.output_enables
    }
}

impl MmioDevice for Pio {
    fn name(&self) -> &str {
        Self::block_name(self.index)
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(self.base, PIO_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        // Atomic alias reads observe the underlying register unchanged.
        // 原子别名读观察底层寄存器，不做修改。
        let (offset, _alias) = split_alias(offset);
        if let Some((index, register)) = Self::sm_register(offset) {
            return Some(self.read_sm(index, register));
        }
        if (INSTR_MEM0..=INSTR_MEM31).contains(&offset) {
            let slot = ((offset - INSTR_MEM0) / 4) as usize;
            return Some(u32::from(self.imem[slot]));
        }
        if (RXF0..=RXF3).contains(&offset) {
            let index = ((offset - RXF0) / 4) as usize;
            let word = self.sm[index].rx.pop();
            if word.is_none() {
                self.fdebug |= FDEBUG_RXUNDER << index;
            }
            return Some(word.unwrap_or(0));
        }
        match offset {
            CTRL => Some(u32::from(self.enabled)),
            FSTAT => Some(self.fstat()),
            FDEBUG => Some(self.fdebug),
            FLEVEL => Some(self.flevel()),
            IRQ => Some(self.irq),
            // TXF/IRQ_FORCE 为只写寄存器，读回 0；DBG_PADOUT/DBG_PADOE 回读当前
            // 引脚输出电平与输出使能。
            // TXF/IRQ_FORCE are write-only and read as zero; DBG_PADOUT/DBG_PADOE
            // sample the current output levels and enables.
            TXF0..=TXF3 | IRQ_FORCE => Some(0),
            DBG_PADOUT => Some(self.output_levels),
            DBG_PADOE => Some(self.output_enables),
            INPUT_SYNC_BYPASS => Some(self.input_sync_bypass),
            DBG_CFGINFO => Some(DBG_CFGINFO_VALUE),
            GPIOBASE => Some(self.gpio_base),
            _ => None,
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        // SET/CLR/XOR windows read-modify-write the plain register; the PIO
        // firmware uses them for `hw_xor_bits` on `SMx_CLKDIV`.
        // SET/CLR/XOR 窗口对普通寄存器做读-改-写；PIO 固件用它实现
        // `SMx_CLKDIV` 上的 `hw_xor_bits`。
        let (offset, alias) = split_alias(offset);
        let value = apply_alias(self.read32(offset).unwrap_or(0), value, alias);
        if let Some((index, register)) = Self::sm_register(offset) {
            return self.write_sm(index, register, value);
        }
        if (INSTR_MEM0..=INSTR_MEM31).contains(&offset) {
            let slot = ((offset - INSTR_MEM0) / 4) as usize;
            self.imem[slot] = (value & INSTR_MASK) as u16;
            return true;
        }
        if (TXF0..=TXF3).contains(&offset) {
            let index = ((offset - TXF0) / 4) as usize;
            // 已满时 push_tx 置位 FDEBUG.TXOVER；与之前的语义一致，写入仍被接受。
            // On a full FIFO push_tx raises FDEBUG.TXOVER; the write is still
            // accepted, matching the previous semantics.
            let _ = self.push_tx(index, value);
            return true;
        }
        match offset {
            CTRL => {
                // 复位/时钟重启位为自清零脉冲；本阶段保留 SM_ENABLE，并处理
                // SM_RESTART 与 CLKDIV_RESTART。
                // Restart/clock-restart bits are self-clearing strokes; SM_ENABLE is
                // retained, and SM_RESTART/CLKDIV_RESTART are applied below.
                self.enabled = (value & SM_ENABLE_MASK) as u8;
                for index in 0..SM_COUNT {
                    if value & (SM_RESTART << index) != 0 {
                        self.sm[index].restart();
                    }
                    if value & (CLKDIV_RESTART << index) != 0 {
                        self.sm[index].clock_phase = 0;
                    }
                }
                true
            }
            // FDEBUG 为写 1 清零的粘滞标志。FDEBUG is write-one-to-clear.
            FDEBUG => {
                self.fdebug &= !(value & FIFO_STATUS_MASK);
                true
            }
            // IRQ 标志写 1 清零。IRQ flags are write-one-to-clear.
            IRQ => {
                self.irq &= !(value & IRQ_MASK);
                true
            }
            // IRQ_FORCE 直接置位 PIO 内部 IRQ 标志。IRQ_FORCE sets the internal flags.
            IRQ_FORCE => {
                self.irq |= value & IRQ_MASK;
                true
            }
            INPUT_SYNC_BYPASS => {
                self.input_sync_bypass = value;
                true
            }
            GPIOBASE => {
                self.gpio_base = value & GPIOBASE_MASK;
                true
            }
            // 只读/只写寄存器忽略相反方向的访问。Read-only/WF registers ignore writes.
            FSTAT | FLEVEL | DBG_CFGINFO | DBG_PADOUT | DBG_PADOE | RXF0..=RXF3 => true,
            _ => false,
        }
    }

    fn reset(&mut self) {
        // 寄存器与引脚向量回到复位值，但共享 pad 矩阵句柄与音频 tap 属于宿主接线，
        // 必须保留；复位同时清空已捕获的样本，避免旧音频跨复位残留。
        // Registers and pin vectors return to their reset values; the shared pad
        // handle and the audio tap belong to the host wiring and must survive the
        // reset. Resetting also clears captured samples so stale audio cannot
        // survive across a reset.
        let pads = self.pads.clone();
        let audio = self.audio.clone();
        *self = Self::new(self.index);
        self.pads = pads;
        self.audio = audio;
        if let Some((tap, _)) = &self.audio {
            tap.clear();
        }
        self.publish_outputs();
    }
}
