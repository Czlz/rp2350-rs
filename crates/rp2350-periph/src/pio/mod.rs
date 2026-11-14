//! PIO register block and state-machine execution / PIO 寄存器块与状态机执行
//!
//! RP2350 has three PIO blocks (`PIO0`/`PIO1`/`PIO2`) at `0x5020_0000`,
//! `0x5030_0000` and `0x5040_0000`. This module models the register file, the
//! 32-word instruction memory, the four state-machine register windows, the
//! 4-deep TX/RX FIFOs and the PIO instruction set (JMP/WAIT/IN/OUT/PUSH/PULL/
//! MOV/IRQ/SET). GPIO/PINCTRL mapping, side-set output, delay cycles, FIFO
//! joining, DMA handshakes and interrupt routing are deliberately not modelled
//! yet; the corresponding offsets follow the project policy (reads return zero,
//! writes are ignored, one warning per offset).
//! RP2350 有三个 PIO 块（`PIO0`/`PIO1`/`PIO2`），分别位于 `0x5020_0000`、
//! `0x5030_0000` 与 `0x5040_0000`。本模块建模寄存器文件、32 字指令存储、四个
//! 状态机寄存器窗口、4 深度 TX/RX FIFO 以及 PIO 指令集（JMP/WAIT/IN/OUT/
//! PUSH/PULL/MOV/IRQ/SET）。GPIO/PINCTRL 映射、side-set 输出、延迟周期、FIFO
//! 合并、DMA 握手与中断路由暂不建模；对应偏移沿用项目策略（读为 0、写被忽略、
//! 每个偏移只警告一次）。
//!
//! Reset values are taken from the RP2350 register headers (`PIO_*_RESET`):
//! `FSTAT = 0x0F00_0F00`, `SMx_CLKDIV = 0x0001_0000`,
//! `SMx_EXECCTRL = 0x0001_F000`, `SMx_SHIFTCTRL = 0x000C_0000`,
//! `SMx_PINCTRL = 0x1400_0000` and `DBG_CFGINFO = 0x1020_0404`.
//! 复位值取自 RP2350 寄存器头文件（`PIO_*_RESET`）：
//! `FSTAT = 0x0F00_0F00`、`SMx_CLKDIV = 0x0001_0000`、
//! `SMx_EXECCTRL = 0x0001_F000`、`SMx_SHIFTCTRL = 0x000C_0000`、
//! `SMx_PINCTRL = 0x1400_0000`、`DBG_CFGINFO = 0x1020_0404`。

mod regs;
mod shift;
mod sm;
#[cfg(test)]
mod sm_tests;
#[cfg(test)]
mod tests;

use rp2350_core::mmio::{MmioDevice, MmioWindow};

use super::map;
// PIO 寄存器常量沿用数据手册命名并集中平铺，通配导入让寄存器映射保持直观。
// PIO register constants follow datasheet names and are declared in one place, so
// a glob import keeps the register map readable.
#[allow(clippy::wildcard_imports)]
use regs::*;

/// One RP2350 PIO register block / 一个 RP2350 PIO 寄存器块
///
/// The block exposes the generic registers, the instruction memory and the
/// four state-machine register windows through [`MmioDevice`]. Writing
/// `SMx_INSTR` executes an instruction immediately and `SMx_ADDR` reports the
/// current program counter. Call [`Pio::tick`] (or [`Pio::step_sm`]) to run the
/// state machines one instruction at a time.
/// 该块通过 [`MmioDevice`] 暴露通用寄存器、指令存储与四个状态机寄存器窗口。
/// 写 `SMx_INSTR` 会立即执行一条指令，`SMx_ADDR` 报告当前程序计数器。调用
/// [`Pio::tick`]（或 [`Pio::step_sm`]）可按“每次一条指令”的方式驱动状态机。
#[derive(Debug)]
pub struct Pio {
    index: u8,
    base: u32,
    enabled: u8,
    fdebug: u32,
    irq: u32,
    input_sync_bypass: u32,
    gpio_base: u32,
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
        let base = match index {
            0 => map::PIO0_BASE,
            1 => map::PIO1_BASE,
            2 => map::PIO2_BASE,
            _ => panic!("PIO index must be 0, 1 or 2"),
        };
        Self {
            index,
            base,
            enabled: 0,
            fdebug: 0,
            irq: 0,
            input_sync_bypass: 0,
            gpio_base: 0,
            imem: [0; IMEM_WORDS],
            sm: core::array::from_fn(|slot| StateMachine::new(slot as u8)),
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
            SM_CLKDIV => self.sm[index].clkdiv = value & CLKDIV_MASK,
            SM_EXECCTRL => {
                // EXEC_STALLED 为只读状态位，不受写入影响。
                // EXEC_STALLED is a read-only status bit and survives writes.
                let stalled = self.sm[index].execctrl & EXECCTRL_EXEC_STALLED;
                self.sm[index].execctrl = stalled | (value & EXECCTRL_WRITE_MASK);
            }
            SM_SHIFTCTRL => self.sm[index].shiftctrl = value & SHIFTCTRL_MASK,
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
}

impl MmioDevice for Pio {
    fn name(&self) -> &str {
        match self.index {
            0 => "PIO0",
            1 => "PIO1",
            _ => "PIO2",
        }
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(self.base, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
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
            // TXF/IRQ_FORCE/DBG_PADOUT/DBG_PADOE 均为只写或只读采样寄存器，读回 0。
            // TXF/IRQ_FORCE/DBG_PADOUT/DBG_PADOE are write-only or read-only sample
            // registers; they read back as zero.
            TXF0..=TXF3 | IRQ_FORCE | DBG_PADOUT | DBG_PADOE => Some(0),
            INPUT_SYNC_BYPASS => Some(self.input_sync_bypass),
            DBG_CFGINFO => Some(DBG_CFGINFO_VALUE),
            GPIOBASE => Some(self.gpio_base),
            _ => None,
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
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
            if self.sm[index].tx.push(value) {
                return true;
            }
            self.fdebug |= FDEBUG_TXOVER << index;
            return true;
        }
        match offset {
            CTRL => {
                // 复位/时钟重启位为自清零脉冲；本阶段只保留 SM_ENABLE 并处理 SM_RESTART。
                // Restart/clock-restart bits are self-clearing strokes; only SM_ENABLE
                // is retained and SM_RESTART is applied.
                self.enabled = (value & SM_ENABLE_MASK) as u8;
                for index in 0..SM_COUNT {
                    if value & (SM_RESTART << index) != 0 {
                        self.sm[index].restart();
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
        *self = Self::new(self.index);
    }
}
