//! PIO register layout, FIFOs and state-machine state
//! PIO 寄存器布局、FIFO 与状态机状态

/// Generic register offsets / 通用寄存器偏移
pub(super) const CTRL: u32 = 0x000;
pub(super) const FSTAT: u32 = 0x004;
pub(super) const FDEBUG: u32 = 0x008;
pub(super) const FLEVEL: u32 = 0x00C;
pub(super) const TXF0: u32 = 0x010;
pub(super) const TXF3: u32 = 0x01C;
pub(super) const RXF0: u32 = 0x020;
pub(super) const RXF3: u32 = 0x02C;
pub(super) const IRQ: u32 = 0x030;
pub(super) const IRQ_FORCE: u32 = 0x034;
pub(super) const INPUT_SYNC_BYPASS: u32 = 0x038;
pub(super) const DBG_PADOUT: u32 = 0x03C;
pub(super) const DBG_PADOE: u32 = 0x040;
pub(super) const DBG_CFGINFO: u32 = 0x044;
pub(super) const INSTR_MEM0: u32 = 0x048;
pub(super) const INSTR_MEM31: u32 = 0x0C4;
pub(super) const SM0_CLKDIV: u32 = 0x0C8;
pub(super) const GPIOBASE: u32 = 0x168;

/// Byte stride between consecutive state-machine windows
/// 相邻状态机窗口之间的字节步长
pub(super) const SM_STRIDE: u32 = 0x18;

/// Register offsets inside one state-machine window / 单个状态机窗口内的寄存器偏移
pub(super) const SM_CLKDIV: u32 = 0x00;
pub(super) const SM_EXECCTRL: u32 = 0x04;
pub(super) const SM_SHIFTCTRL: u32 = 0x08;
pub(super) const SM_ADDR: u32 = 0x0C;
pub(super) const SM_INSTR: u32 = 0x10;
pub(super) const SM_PINCTRL: u32 = 0x14;

/// Number of state machines per PIO block / 每个 PIO 块的状态机数量
pub(super) const SM_COUNT: usize = 4;

/// Number of instruction-memory slots / 指令存储槽数量
pub(super) const IMEM_WORDS: usize = 32;

/// Depth of each TX/RX FIFO in words / 每个 TX/RX FIFO 的深度（字）
pub(super) const FIFO_DEPTH: usize = 4;

/// `FSTAT`/`FDEBUG` per-state-machine field mask / `FSTAT`/`FDEBUG` 每状态机字段掩码
pub(super) const FIFO_STATUS_MASK: u32 = 0x0F0F_0F0F;

/// `IRQ` flag mask (eight PIO IRQ flags) / `IRQ` 标志掩码（8 个 PIO IRQ 标志）
pub(super) const IRQ_MASK: u32 = 0x0000_00FF;

/// `CTRL.SM_ENABLE` mask / `CTRL.SM_ENABLE` 掩码
pub(super) const SM_ENABLE_MASK: u32 = 0x0000_000F;

/// `CTRL.SM_RESTART` base bit / `CTRL.SM_RESTART` 基位
pub(super) const SM_RESTART: u32 = 0x0000_0010;

/// `SMx_CLKDIV` writable mask (`INT` + `FRAC`) / `SMx_CLKDIV` 可写掩码（`INT` + `FRAC`）
pub(super) const CLKDIV_MASK: u32 = 0xFFFF_FF00;

/// `SMx_CLKDIV` reset value / `SMx_CLKDIV` 复位值
pub(super) const CLKDIV_RESET: u32 = 0x0001_0000;

/// `SMx_EXECCTRL` writable mask; bit 31 (`EXEC_STALLED`) is read-only
/// `SMx_EXECCTRL` 可写掩码；位 31（`EXEC_STALLED`）为只读
pub(super) const EXECCTRL_WRITE_MASK: u32 = 0x7FFF_FFFF;

/// `SMx_EXECCTRL` reset value (`WRAP_TOP = 0x1F`) / `SMx_EXECCTRL` 复位值
pub(super) const EXECCTRL_RESET: u32 = 0x0001_F000;

/// `SMx_EXECCTRL.EXEC_STALLED` / `SMx_EXECCTRL.EXEC_STALLED`
pub(super) const EXECCTRL_EXEC_STALLED: u32 = 1 << 31;

/// `SMx_EXECCTRL.WRAP_BOTTOM` bit position / `SMx_EXECCTRL.WRAP_BOTTOM` 位位置
pub(super) const EXECCTRL_WRAP_BOTTOM_SHIFT: u32 = 7;

/// `SMx_EXECCTRL.WRAP_TOP` bit position / `SMx_EXECCTRL.WRAP_TOP` 位位置
pub(super) const EXECCTRL_WRAP_TOP_SHIFT: u32 = 12;

/// `SMx_EXECCTRL.STATUS_SEL` (0 = TX FIFO, 1 = RX FIFO)
/// `SMx_EXECCTRL.STATUS_SEL`（0 = TX FIFO，1 = RX FIFO）
pub(super) const EXECCTRL_STATUS_SEL: u32 = 1 << 4;

/// `SMx_EXECCTRL.STATUS_N` mask (FIFO level threshold)
/// `SMx_EXECCTRL.STATUS_N` 掩码（FIFO 占用阈值）
pub(super) const EXECCTRL_STATUS_N_MASK: u32 = 0x0000_000F;

/// `SMx_SHIFTCTRL` writable mask / `SMx_SHIFTCTRL` 可写掩码
pub(super) const SHIFTCTRL_MASK: u32 = 0xFFFF_C01F;

/// `SMx_SHIFTCTRL` reset value (`IN`/`OUT_SHIFTDIR = 1`) / `SMx_SHIFTCTRL` 复位值
pub(super) const SHIFTCTRL_RESET: u32 = 0x000C_0000;

/// `SMx_SHIFTCTRL.AUTOPUSH` / `SMx_SHIFTCTRL.AUTOPUSH`
pub(super) const SHIFTCTRL_AUTOPUSH: u32 = 1 << 16;

/// `SMx_SHIFTCTRL.AUTOPULL` / `SMx_SHIFTCTRL.AUTOPULL`
pub(super) const SHIFTCTRL_AUTOPULL: u32 = 1 << 17;

/// `SMx_SHIFTCTRL.IN_SHIFTDIR` (1 = shift right) / `SMx_SHIFTCTRL.IN_SHIFTDIR`（1 = 右移）
pub(super) const SHIFTCTRL_IN_SHIFTDIR: u32 = 1 << 18;

/// `SMx_SHIFTCTRL.OUT_SHIFTDIR` (1 = shift right) / `SMx_SHIFTCTRL.OUT_SHIFTDIR`（1 = 右移）
pub(super) const SHIFTCTRL_OUT_SHIFTDIR: u32 = 1 << 19;

/// `SMx_SHIFTCTRL.PUSH_THRESH` bit position / `SMx_SHIFTCTRL.PUSH_THRESH` 位位置
pub(super) const SHIFTCTRL_PUSH_THRESH_SHIFT: u32 = 20;

/// `SMx_SHIFTCTRL.PULL_THRESH` bit position / `SMx_SHIFTCTRL.PULL_THRESH` 位位置
pub(super) const SHIFTCTRL_PULL_THRESH_SHIFT: u32 = 25;

/// `SMx_SHIFTCTRL.IN_COUNT` mask (RP2350 LSB pins kept in `IN PINS`)
/// `SMx_SHIFTCTRL.IN_COUNT` 掩码（RP2350 `IN PINS` 保留的引脚位数）
pub(super) const SHIFTCTRL_IN_COUNT_MASK: u32 = 0x0000_001F;

/// `SMx_PINCTRL` reset value (`SET_COUNT = 5`) / `SMx_PINCTRL` 复位值
pub(super) const PINCTRL_RESET: u32 = 0x1400_0000;

/// `INSTR_MEMx` data mask / `INSTR_MEMx` 数据掩码
pub(super) const INSTR_MASK: u32 = 0x0000_FFFF;

/// `GPIOBASE` writable bit / `GPIOBASE` 可写位
pub(super) const GPIOBASE_MASK: u32 = 0x10;

/// `DBG_CFGINFO`: version 1, 32 instruction slots, 4 SMs, FIFO depth 4
/// `DBG_CFGINFO`：版本 1、32 个指令槽、4 个状态机、FIFO 深度 4
pub(super) const DBG_CFGINFO_VALUE: u32 = 0x1020_0404;

/// `FDEBUG` RXSTALL base bit, plus the SM index / `FDEBUG` RXSTALL 基位，加上状态机序号
pub(super) const FDEBUG_RXSTALL: u32 = 1 << 0;

/// `FDEBUG` RXUNDER base bit, plus the SM index / `FDEBUG` RXUNDER 基位，加上状态机序号
pub(super) const FDEBUG_RXUNDER: u32 = 1 << 8;

/// `FDEBUG` TXOVER base bit, plus the SM index / `FDEBUG` TXOVER 基位，加上状态机序号
pub(super) const FDEBUG_TXOVER: u32 = 1 << 16;

/// `FDEBUG` TXSTALL base bit, plus the SM index / `FDEBUG` TXSTALL 基位，加上状态机序号
pub(super) const FDEBUG_TXSTALL: u32 = 1 << 24;

/// A 4-deep word FIFO / 4 深度字 FIFO
#[derive(Debug, Clone, Copy)]
pub(super) struct Fifo {
    words: [u32; FIFO_DEPTH],
    head: usize,
    len: usize,
}

impl Fifo {
    /// Create an empty FIFO / 创建空 FIFO
    pub(super) const fn new() -> Self {
        Self {
            words: [0; FIFO_DEPTH],
            head: 0,
            len: 0,
        }
    }

    /// Push one word, returning `false` when the FIFO is full
    /// 压入一个字；FIFO 已满时返回 `false`
    pub(super) fn push(&mut self, value: u32) -> bool {
        if self.is_full() {
            return false;
        }
        let tail = (self.head + self.len) % FIFO_DEPTH;
        self.words[tail] = value;
        self.len += 1;
        true
    }

    /// Pop the oldest word, or `None` when empty / 取出最旧的字；为空时返回 `None`
    pub(super) fn pop(&mut self) -> Option<u32> {
        if self.is_empty() {
            return None;
        }
        let value = self.words[self.head];
        self.head = (self.head + 1) % FIFO_DEPTH;
        self.len -= 1;
        Some(value)
    }

    /// Current occupancy in words / 当前占用字数
    pub(super) fn level(&self) -> usize {
        self.len
    }

    /// Whether the FIFO is empty / FIFO 是否为空
    pub(super) fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether the FIFO is full / FIFO 是否已满
    pub(super) fn is_full(&self) -> bool {
        self.len == FIFO_DEPTH
    }
}

/// Register and execution state of one state machine / 单个状态机的寄存器与执行状态
#[derive(Debug, Clone, Copy)]
pub(super) struct StateMachine {
    /// Position inside its PIO block, used by relative IRQ / 块内序号，供相对 IRQ 使用
    pub(super) index: u8,
    /// `SMx_CLKDIV` / `SMx_CLKDIV`
    pub(super) clkdiv: u32,
    /// `SMx_EXECCTRL` / `SMx_EXECCTRL`
    pub(super) execctrl: u32,
    /// `SMx_SHIFTCTRL` / `SMx_SHIFTCTRL`
    pub(super) shiftctrl: u32,
    /// `SMx_PINCTRL` / `SMx_PINCTRL`
    pub(super) pinctrl: u32,
    /// Program counter / 程序计数器
    pub(super) pc: u8,
    /// Scratch register X / 暂存寄存器 X
    pub(super) x: u32,
    /// Scratch register Y / 暂存寄存器 Y
    pub(super) y: u32,
    /// Input shift register / 输入移位寄存器
    pub(super) isr: u32,
    /// Output shift register / 输出移位寄存器
    pub(super) osr: u32,
    /// Bits shifted into the ISR since it was last emptied / 本次 ISR 已移入的位数
    pub(super) isr_bits: u8,
    /// Bits remaining in the OSR before it is empty / OSR 中剩余的位数
    pub(super) osr_bits: u8,
    /// IRQ flag awaited by an in-flight `IRQ WAIT` / `IRQ WAIT` 正在等待的标志
    pub(super) pending_irq_wait: Option<u8>,
    /// TX FIFO / TX FIFO
    pub(super) tx: Fifo,
    /// RX FIFO / RX FIFO
    pub(super) rx: Fifo,
}

impl StateMachine {
    /// Create a state machine with the datasheet reset values
    /// 创建使用数据手册复位值的状态机
    pub(super) const fn new(index: u8) -> Self {
        Self {
            index,
            clkdiv: CLKDIV_RESET,
            execctrl: EXECCTRL_RESET,
            shiftctrl: SHIFTCTRL_RESET,
            pinctrl: PINCTRL_RESET,
            pc: 0,
            x: 0,
            y: 0,
            isr: 0,
            osr: 0,
            isr_bits: 0,
            osr_bits: 0,
            pending_irq_wait: None,
            tx: Fifo::new(),
            rx: Fifo::new(),
        }
    }

    /// Apply a `CTRL.SM_RESTART` strobe: PC, shift counters and stalls
    /// 应用 `CTRL.SM_RESTART` 脉冲：复位 PC、移位计数与阻塞状态
    ///
    /// FIFO contents are preserved, as required by the datasheet.
    /// 按数据手册要求保留 FIFO 内容。
    pub(super) fn restart(&mut self) {
        self.pc = 0;
        self.isr = 0;
        self.osr = 0;
        self.isr_bits = 0;
        self.osr_bits = 0;
        self.pending_irq_wait = None;
        self.execctrl &= !EXECCTRL_EXEC_STALLED;
    }

    /// `SHIFTCTRL.PUSH_THRESH`, where 0 encodes 32 / `SHIFTCTRL.PUSH_THRESH`，0 表示 32
    pub(super) fn push_threshold(&self) -> u8 {
        let value = ((self.shiftctrl >> SHIFTCTRL_PUSH_THRESH_SHIFT) & 0x1F) as u8;
        if value == 0 { 32 } else { value }
    }

    /// `SHIFTCTRL.PULL_THRESH`, where 0 encodes 32 / `SHIFTCTRL.PULL_THRESH`，0 表示 32
    pub(super) fn pull_threshold(&self) -> u8 {
        let value = ((self.shiftctrl >> SHIFTCTRL_PULL_THRESH_SHIFT) & 0x1F) as u8;
        if value == 0 { 32 } else { value }
    }

    /// Remaining OSR bit count at which the OSR counts as empty
    /// OSR 视为“已空”时的剩余位数
    pub(super) fn empty_level(&self) -> u8 {
        32 - self.pull_threshold()
    }

    /// `EXECCTRL.WRAP_TOP` / `EXECCTRL.WRAP_TOP`
    pub(super) fn wrap_top(&self) -> u8 {
        ((self.execctrl >> EXECCTRL_WRAP_TOP_SHIFT) & 0x1F) as u8
    }

    /// `EXECCTRL.WRAP_BOTTOM` / `EXECCTRL.WRAP_BOTTOM`
    pub(super) fn wrap_bottom(&self) -> u8 {
        ((self.execctrl >> EXECCTRL_WRAP_BOTTOM_SHIFT) & 0x1F) as u8
    }

    /// Whether automatic push is enabled / 是否启用自动压入
    pub(super) fn autopush(&self) -> bool {
        self.shiftctrl & SHIFTCTRL_AUTOPUSH != 0
    }

    /// Whether automatic pull is enabled / 是否启用自动拉取
    pub(super) fn autopull(&self) -> bool {
        self.shiftctrl & SHIFTCTRL_AUTOPULL != 0
    }

    /// Whether the ISR shifts right / ISR 是否右移
    pub(super) fn in_shift_right(&self) -> bool {
        self.shiftctrl & SHIFTCTRL_IN_SHIFTDIR != 0
    }

    /// Whether the OSR shifts right / OSR 是否右移
    pub(super) fn out_shift_right(&self) -> bool {
        self.shiftctrl & SHIFTCTRL_OUT_SHIFTDIR != 0
    }

    /// `SHIFTCTRL.IN_COUNT` pin mask; 0 keeps all 32 pins
    /// `SHIFTCTRL.IN_COUNT` 引脚掩码；0 表示保留全部 32 位
    pub(super) fn in_count_mask(&self) -> u32 {
        let count = self.shiftctrl & SHIFTCTRL_IN_COUNT_MASK;
        if count == 0 {
            u32::MAX
        } else {
            (1u32 << count) - 1
        }
    }

    /// Value reported by `MOV dst, STATUS` / `MOV dst, STATUS` 读到的值
    ///
    /// All-ones when the selected FIFO holds fewer than `STATUS_N` words.
    /// 当选中的 FIFO 占用少于 `STATUS_N` 个字时返回全 1。
    pub(super) fn status(&self) -> u32 {
        let threshold = self.execctrl & EXECCTRL_STATUS_N_MASK;
        let level = if self.execctrl & EXECCTRL_STATUS_SEL != 0 {
            self.rx.level()
        } else {
            self.tx.level()
        };
        if (level as u32) < threshold {
            u32::MAX
        } else {
            0
        }
    }
}
