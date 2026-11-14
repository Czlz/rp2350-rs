//! PIO register-block model / PIO 寄存器块模型
//!
//! RP2350 has three PIO blocks (`PIO0`/`PIO1`/`PIO2`) at `0x5020_0000`,
//! `0x5030_0000` and `0x5040_0000`. This stage models the register file only:
//! the generic control/status/debug registers, the 32-word instruction memory,
//! the four state-machine register windows and the 4-deep TX/RX FIFOs. State
//! machine execution, FIFO joining, DMA handshakes and interrupt routing are
//! deliberately not modelled yet; the corresponding offsets follow the project
//! policy (reads return zero, writes are ignored, one warning per offset).
//! RP2350 有三个 PIO 块（`PIO0`/`PIO1`/`PIO2`），分别位于 `0x5020_0000`、
//! `0x5030_0000` 与 `0x5040_0000`。本阶段只建模寄存器文件：通用控制/状态/调试
//! 寄存器、32 字指令存储、四个状态机寄存器窗口以及 4 深度 TX/RX FIFO。状态机
//! 执行、FIFO 合并、DMA 握手与中断路由暂不建模；对应偏移沿用项目策略（读为
//! 0、写被忽略、每个偏移只警告一次）。
//!
//! Reset values are taken from the RP2350 register headers (`PIO_*_RESET`):
//! `FSTAT = 0x0F00_0F00`, `SMx_CLKDIV = 0x0001_0000`,
//! `SMx_EXECCTRL = 0x0001_F000`, `SMx_SHIFTCTRL = 0x000C_0000`,
//! `SMx_PINCTRL = 0x1400_0000` and `DBG_CFGINFO = 0x1020_0404`.
//! 复位值取自 RP2350 寄存器头文件（`PIO_*_RESET`）：
//! `FSTAT = 0x0F00_0F00`、`SMx_CLKDIV = 0x0001_0000`、
//! `SMx_EXECCTRL = 0x0001_F000`、`SMx_SHIFTCTRL = 0x000C_0000`、
//! `SMx_PINCTRL = 0x1400_0000`、`DBG_CFGINFO = 0x1020_0404`。

use rp2350_core::mmio::{MmioDevice, MmioWindow};

use super::map;

/// Generic register offsets / 通用寄存器偏移
const CTRL: u32 = 0x000;
const FSTAT: u32 = 0x004;
const FDEBUG: u32 = 0x008;
const FLEVEL: u32 = 0x00C;
const TXF0: u32 = 0x010;
const TXF3: u32 = 0x01C;
const RXF0: u32 = 0x020;
const RXF3: u32 = 0x02C;
const IRQ: u32 = 0x030;
const IRQ_FORCE: u32 = 0x034;
const INPUT_SYNC_BYPASS: u32 = 0x038;
const DBG_PADOUT: u32 = 0x03C;
const DBG_PADOE: u32 = 0x040;
const DBG_CFGINFO: u32 = 0x044;
const INSTR_MEM0: u32 = 0x048;
const INSTR_MEM31: u32 = 0x0C4;
const SM0_CLKDIV: u32 = 0x0C8;
const GPIOBASE: u32 = 0x168;

/// Byte stride between consecutive state-machine windows
/// 相邻状态机窗口之间的字节步长
const SM_STRIDE: u32 = 0x18;

/// Register offsets inside one state-machine window / 单个状态机窗口内的寄存器偏移
const SM_CLKDIV: u32 = 0x00;
const SM_EXECCTRL: u32 = 0x04;
const SM_SHIFTCTRL: u32 = 0x08;
const SM_ADDR: u32 = 0x0C;
const SM_INSTR: u32 = 0x10;
const SM_PINCTRL: u32 = 0x14;

/// Number of state machines per PIO block / 每个 PIO 块的状态机数量
const SM_COUNT: usize = 4;

/// Number of instruction-memory slots / 指令存储槽数量
const IMEM_WORDS: usize = 32;

/// Depth of each TX/RX FIFO in words / 每个 TX/RX FIFO 的深度（字）
const FIFO_DEPTH: usize = 4;

/// `FSTAT`/`FDEBUG` per-state-machine field mask / `FSTAT`/`FDEBUG` 每状态机字段掩码
const FIFO_STATUS_MASK: u32 = 0x0F0F_0F0F;

/// `IRQ` flag mask (eight PIO IRQ flags) / `IRQ` 标志掩码（8 个 PIO IRQ 标志）
const IRQ_MASK: u32 = 0x0000_00FF;

/// `SMx_CLKDIV` writable mask (`INT` + `FRAC`) / `SMx_CLKDIV` 可写掩码（`INT` + `FRAC`）
const CLKDIV_MASK: u32 = 0xFFFF_FF00;

/// `SMx_CLKDIV` reset value / `SMx_CLKDIV` 复位值
const CLKDIV_RESET: u32 = 0x0001_0000;

/// `SMx_EXECCTRL` writable mask; bit 31 (`EXEC_STALLED`) is read-only
/// `SMx_EXECCTRL` 可写掩码；位 31（`EXEC_STALLED`）为只读
const EXECCTRL_WRITE_MASK: u32 = 0x7FFF_FFFF;

/// `SMx_EXECCTRL` reset value (`WRAP_TOP = 0x1F`) / `SMx_EXECCTRL` 复位值
const EXECCTRL_RESET: u32 = 0x0001_F000;

/// `SMx_SHIFTCTRL` writable mask / `SMx_SHIFTCTRL` 可写掩码
const SHIFTCTRL_MASK: u32 = 0xFFFF_C01F;

/// `SMx_SHIFTCTRL` reset value (`IN`/`OUT_SHIFTDIR = 1`) / `SMx_SHIFTCTRL` 复位值
const SHIFTCTRL_RESET: u32 = 0x000C_0000;

/// `SMx_PINCTRL` reset value (`SET_COUNT = 5`) / `SMx_PINCTRL` 复位值
const PINCTRL_RESET: u32 = 0x1400_0000;

/// `INSTR_MEMx` data mask / `INSTR_MEMx` 数据掩码
const INSTR_MASK: u32 = 0x0000_FFFF;

/// `GPIOBASE` writable bit / `GPIOBASE` 可写位
const GPIOBASE_MASK: u32 = 0x10;

/// `DBG_CFGINFO`: version 1, 32 instruction slots, 4 SMs, FIFO depth 4
/// `DBG_CFGINFO`：版本 1、32 个指令槽、4 个状态机、FIFO 深度 4
const DBG_CFGINFO_VALUE: u32 = 0x1020_0404;

/// `FDEBUG` RXSTALL base bit, plus the SM index / `FDEBUG` RXSTALL 基位，加上状态机序号
const FDEBUG_RXSTALL: u32 = 1 << 0;

/// `FDEBUG` RXUNDER base bit, plus the SM index / `FDEBUG` RXUNDER 基位，加上状态机序号
const FDEBUG_RXUNDER: u32 = 1 << 8;

/// `FDEBUG` TXOVER base bit, plus the SM index / `FDEBUG` TXOVER 基位，加上状态机序号
const FDEBUG_TXOVER: u32 = 1 << 16;

/// `FDEBUG` TXSTALL base bit, plus the SM index / `FDEBUG` TXSTALL 基位，加上状态机序号
const FDEBUG_TXSTALL: u32 = 1 << 24;

/// A 4-deep word FIFO / 4 深度字 FIFO
#[derive(Debug, Clone, Copy)]
struct Fifo {
    words: [u32; FIFO_DEPTH],
    head: usize,
    len: usize,
}

impl Fifo {
    /// Create an empty FIFO / 创建空 FIFO
    const fn new() -> Self {
        Self {
            words: [0; FIFO_DEPTH],
            head: 0,
            len: 0,
        }
    }

    /// Push one word, returning `false` when the FIFO is full
    /// 压入一个字；FIFO 已满时返回 `false`
    fn push(&mut self, value: u32) -> bool {
        if self.is_full() {
            return false;
        }
        let tail = (self.head + self.len) % FIFO_DEPTH;
        self.words[tail] = value;
        self.len += 1;
        true
    }

    /// Pop the oldest word, or `None` when empty / 取出最旧的字；为空时返回 `None`
    fn pop(&mut self) -> Option<u32> {
        if self.is_empty() {
            return None;
        }
        let value = self.words[self.head];
        self.head = (self.head + 1) % FIFO_DEPTH;
        self.len -= 1;
        Some(value)
    }

    /// Current occupancy in words / 当前占用字数
    fn level(&self) -> usize {
        self.len
    }

    /// Whether the FIFO is empty / FIFO 是否为空
    fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether the FIFO is full / FIFO 是否已满
    fn is_full(&self) -> bool {
        self.len == FIFO_DEPTH
    }
}

/// Register state of one state machine / 单个状态机的寄存器状态
#[derive(Debug, Clone, Copy)]
struct StateMachine {
    clkdiv: u32,
    execctrl: u32,
    shiftctrl: u32,
    pinctrl: u32,
    tx: Fifo,
    rx: Fifo,
}

impl StateMachine {
    /// Create a state machine with the datasheet reset values
    /// 创建使用数据手册复位值的状态机
    const fn new() -> Self {
        Self {
            clkdiv: CLKDIV_RESET,
            execctrl: EXECCTRL_RESET,
            shiftctrl: SHIFTCTRL_RESET,
            pinctrl: PINCTRL_RESET,
            tx: Fifo::new(),
            rx: Fifo::new(),
        }
    }
}

/// One RP2350 PIO register block / 一个 RP2350 PIO 寄存器块
///
/// The block exposes the generic registers, the instruction memory and the
/// four state-machine register windows through [`MmioDevice`]. Program counter
/// and instruction execution are not modelled, so `SMx_ADDR` reads zero and
/// `SMx_INSTR` returns the instruction currently stored at address 0.
/// 该块通过 [`MmioDevice`] 暴露通用寄存器、指令存储与四个状态机寄存器窗口。
/// 程序计数器与指令执行未建模，因此 `SMx_ADDR` 读为 0，`SMx_INSTR` 返回地址 0
/// 当前存放的指令。
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
            sm: [StateMachine::new(); SM_COUNT],
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
        for (index, sm) in self.sm.iter().enumerate() {
            if sm.rx.is_full() {
                status |= 1 << index;
            }
            if sm.rx.is_empty() {
                status |= 1 << (8 + index);
            }
            if sm.tx.is_full() {
                status |= 1 << (16 + index);
            }
            if sm.tx.is_empty() {
                status |= 1 << (24 + index);
            }
        }
        status
    }

    /// Compute `FLEVEL` from the current FIFO occupancy / 由当前 FIFO 占用计算 `FLEVEL`
    fn flevel(&self) -> u32 {
        let mut level = 0;
        for (index, sm) in self.sm.iter().enumerate() {
            level |= (sm.tx.level() as u32) << (index * 8);
            level |= (sm.rx.level() as u32) << (index * 8 + 4);
        }
        level
    }

    /// Read one state-machine window register / 读取一个状态机窗口寄存器
    fn read_sm(&self, index: usize, register: u32) -> u32 {
        let sm = &self.sm[index];
        match register {
            SM_CLKDIV => sm.clkdiv,
            SM_EXECCTRL => sm.execctrl,
            SM_SHIFTCTRL => sm.shiftctrl,
            SM_INSTR => u32::from(self.imem[0]),
            SM_PINCTRL => sm.pinctrl,
            // 未建模指令执行，PC 恒为 0；未知窗口寄存器同样读 0。
            // Instruction execution is unmodelled, so PC is 0; unknown window registers also read 0.
            _ => 0,
        }
    }

    /// Write one state-machine window register / 写入一个状态机窗口寄存器
    fn write_sm(&mut self, index: usize, register: u32, value: u32) -> bool {
        let sm = &mut self.sm[index];
        match register {
            SM_CLKDIV => sm.clkdiv = value & CLKDIV_MASK,
            SM_EXECCTRL => sm.execctrl = value & EXECCTRL_WRITE_MASK,
            SM_SHIFTCTRL => sm.shiftctrl = value & SHIFTCTRL_MASK,
            // `SMx_ADDR` 只读；`SMx_INSTR` 写入会触发执行，本阶段不建模。
            // `SMx_ADDR` is read-only; `SMx_INSTR` writes trigger execution, unmodelled.
            SM_ADDR | SM_INSTR => {}
            SM_PINCTRL => sm.pinctrl = value,
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
        let Some(sm) = self.sm.get_mut(index) else {
            return false;
        };
        let pushed = sm.rx.push(value);
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
        let sm = self.sm.get_mut(index)?;
        let word = sm.tx.pop();
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
                // 复位/时钟重启位为自清零脉冲；本阶段只保留 SM_ENABLE。
                // Restart/clock-restart bits are self-clearing strokes; only SM_ENABLE is kept.
                self.enabled = (value & 0x0F) as u8;
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

#[cfg(test)]
mod tests {
    use rp2350_core::bus::AddressBus;
    use rp2350_core::memory::SimpleBus;
    use rp2350_core::mmio::mount_device;

    use super::*;

    #[test]
    fn reset_values_match_the_datasheet() {
        let mut pio = Pio::new(0);
        assert_eq!(pio.read32(CTRL), Some(0));
        assert_eq!(pio.read32(FSTAT), Some(0x0F00_0F00));
        assert_eq!(pio.read32(FDEBUG), Some(0));
        assert_eq!(pio.read32(FLEVEL), Some(0));
        assert_eq!(pio.read32(IRQ), Some(0));
        assert_eq!(pio.read32(DBG_CFGINFO), Some(DBG_CFGINFO_VALUE));
        assert_eq!(pio.read32(GPIOBASE), Some(0));
        for index in 0..SM_COUNT {
            let base = SM0_CLKDIV + index as u32 * SM_STRIDE;
            assert_eq!(pio.read32(base + SM_CLKDIV), Some(CLKDIV_RESET));
            assert_eq!(pio.read32(base + SM_EXECCTRL), Some(EXECCTRL_RESET));
            assert_eq!(pio.read32(base + SM_SHIFTCTRL), Some(SHIFTCTRL_RESET));
            assert_eq!(pio.read32(base + SM_ADDR), Some(0));
            assert_eq!(pio.read32(base + SM_INSTR), Some(0));
            assert_eq!(pio.read32(base + SM_PINCTRL), Some(PINCTRL_RESET));
        }
    }

    #[test]
    fn ctrl_reads_back_only_the_enable_bits() {
        let mut pio = Pio::new(0);
        assert!(pio.write32(CTRL, 0xFFFF_FFFF));
        assert_eq!(pio.read32(CTRL), Some(0x0000_000F));
        assert!(pio.write32(CTRL, 0x0000_0005));
        assert_eq!(pio.read32(CTRL), Some(5));
    }

    #[test]
    fn sm_register_writes_apply_the_datasheet_masks() {
        let mut pio = Pio::new(1);
        assert!(pio.write32(SM0_CLKDIV, 0x1234_5678));
        assert_eq!(pio.read32(SM0_CLKDIV), Some(0x1234_5600));
        assert!(pio.write32(SM0_CLKDIV + SM_EXECCTRL, 0xFFFF_FFFF));
        assert_eq!(
            pio.read32(SM0_CLKDIV + SM_EXECCTRL),
            Some(EXECCTRL_WRITE_MASK)
        );
        assert!(pio.write32(SM0_CLKDIV + SM_SHIFTCTRL, 0xFFFF_FFFF));
        assert_eq!(pio.read32(SM0_CLKDIV + SM_SHIFTCTRL), Some(SHIFTCTRL_MASK));
        assert!(pio.write32(SM0_CLKDIV + SM_PINCTRL, 0xDEAD_BEEF));
        assert_eq!(pio.read32(SM0_CLKDIV + SM_PINCTRL), Some(0xDEAD_BEEF));
        // `SMx_ADDR` 只读，写入不改变 PC。`SMx_ADDR` is read-only.
        assert!(pio.write32(SM0_CLKDIV + SM_ADDR, 17));
        assert_eq!(pio.read32(SM0_CLKDIV + SM_ADDR), Some(0));
    }

    #[test]
    fn instruction_memory_holds_32_sixteen_bit_words() {
        let mut pio = Pio::new(2);
        for slot in 0..IMEM_WORDS {
            let offset = INSTR_MEM0 + slot as u32 * 4;
            assert!(pio.write32(offset, 0xFFFF_0000 | slot as u32));
            assert_eq!(pio.read32(offset), Some(slot as u32));
        }
        assert_eq!(pio.read32(INSTR_MEM31), Some(31));
        // SMx_INSTR 读取 PC=0 处的指令。SMx_INSTR reads the instruction at PC 0.
        assert_eq!(pio.read32(SM0_CLKDIV), Some(CLKDIV_RESET));
        assert!(pio.write32(INSTR_MEM0, 0x1234));
        assert_eq!(pio.read32(SM0_CLKDIV + SM_INSTR), Some(0x1234));
    }

    #[test]
    fn tx_fifo_pushes_four_words_then_reports_full_and_overrun() {
        let mut pio = Pio::new(0);
        for word in 1..=4u32 {
            assert!(pio.write32(TXF0, word));
        }
        assert_eq!(
            pio.read32(FSTAT).expect("fstat") & (1 << 16),
            1 << 16,
            "SM0 TXFULL"
        );
        assert_eq!(pio.read32(FLEVEL).expect("flevel") & 0xF, 4);
        assert!(pio.write32(TXF0, 5));
        assert_eq!(
            pio.read32(FLEVEL).expect("flevel") & 0xF,
            4,
            "overflow word is dropped"
        );
        assert_eq!(
            pio.read32(FDEBUG).expect("fdebug") & FDEBUG_TXOVER,
            FDEBUG_TXOVER
        );
    }

    #[test]
    fn tx_fifo_pull_returns_words_in_order_and_tracks_empty() {
        let mut pio = Pio::new(0);
        for word in 0x11..=0x14 {
            pio.write32(TXF0, word);
        }
        for word in 0x11..=0x14 {
            assert_eq!(pio.pop_tx(0), Some(word));
        }
        assert_eq!(pio.pop_tx(0), None);
        assert_eq!(pio.read32(FLEVEL), Some(0));
        assert_eq!(
            pio.read32(FSTAT).expect("fstat") & (1 << 24),
            1 << 24,
            "SM0 TXEMPTY"
        );
    }

    #[test]
    fn rx_fifo_pops_in_order_and_underflow_sets_fdebug() {
        let mut pio = Pio::new(0);
        for word in 0x21..=0x24 {
            assert!(pio.push_rx(2, word));
        }
        assert_eq!(
            pio.read32(FSTAT).expect("fstat") & (1 << 2),
            1 << 2,
            "SM2 RXFULL"
        );
        for word in 0x21..=0x24 {
            assert_eq!(pio.read32(RXF0 + 8), Some(word));
        }
        assert_eq!(pio.read32(RXF0 + 8), Some(0));
        assert_eq!(
            pio.read32(FDEBUG).expect("fdebug") & (FDEBUG_RXUNDER << 2),
            FDEBUG_RXUNDER << 2,
            "SM2 RXUNDER"
        );
    }

    #[test]
    fn fdebug_stall_bits_are_write_one_to_clear() {
        let mut pio = Pio::new(0);
        // 指令执行未建模，直接注入粘滞 stall 位以验证 W1C 语义。
        // Execution is unmodelled, so inject the sticky stall bits to check W1C.
        pio.fdebug = FDEBUG_TXSTALL | FDEBUG_RXSTALL;
        assert!(pio.write32(FDEBUG, 0));
        assert_eq!(pio.read32(FDEBUG), Some(FDEBUG_TXSTALL | FDEBUG_RXSTALL));
        assert!(pio.write32(FDEBUG, FDEBUG_RXSTALL));
        assert_eq!(pio.read32(FDEBUG), Some(FDEBUG_TXSTALL));
        assert!(pio.write32(FDEBUG, FDEBUG_TXSTALL));
        assert_eq!(pio.read32(FDEBUG), Some(0));
    }

    #[test]
    fn irq_flags_are_forced_and_write_one_cleared() {
        let mut pio = Pio::new(0);
        assert!(pio.write32(IRQ_FORCE, 0x5A));
        assert_eq!(pio.read32(IRQ), Some(0x5A));
        assert!(pio.write32(IRQ, 0x08));
        assert_eq!(pio.read32(IRQ), Some(0x52));
    }

    #[test]
    fn reset_restores_power_on_defaults() {
        let mut pio = Pio::new(2);
        pio.write32(CTRL, 0x0F);
        pio.write32(TXF3, 0xAA);
        pio.write32(INSTR_MEM0, 0x1234);
        pio.fdebug = FDEBUG_TXSTALL;
        MmioDevice::reset(&mut pio);
        assert_eq!(pio.read32(CTRL), Some(0));
        assert_eq!(pio.read32(FSTAT), Some(0x0F00_0F00));
        assert_eq!(pio.read32(INSTR_MEM0), Some(0));
        assert_eq!(pio.read32(FDEBUG), Some(0));
    }

    #[test]
    fn bus_mount_exposes_the_register_block() {
        let mut bus = SimpleBus::new();
        mount_device(&mut bus, Box::new(Pio::new(1))).expect("mount");
        let mut cycles = 0;
        assert_eq!(
            bus.read_u32(map::PIO1_BASE + DBG_CFGINFO, &mut cycles)
                .expect("cfg"),
            DBG_CFGINFO_VALUE
        );
        bus.write_u32(map::PIO1_BASE + INSTR_MEM0 + 8, 0x1234, &mut cycles)
            .expect("imem write");
        assert_eq!(
            bus.read_u32(map::PIO1_BASE + INSTR_MEM0 + 8, &mut cycles)
                .expect("imem read"),
            0x1234
        );
    }

    #[test]
    #[should_panic(expected = "PIO index")]
    fn invalid_pio_index_is_rejected() {
        let _ = Pio::new(3);
    }
}
