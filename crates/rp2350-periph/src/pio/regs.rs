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

/// Depth of a joined TX/RX FIFO in words / 合并后 TX/RX FIFO 的深度（字）
pub(super) const FIFO_JOIN_DEPTH: usize = 2 * FIFO_DEPTH;

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

/// `SMx_EXECCTRL.SIDE_EN`: the MSB of the delay/side-set field enables side-set
/// `SMx_EXECCTRL.SIDE_EN`：延迟/side-set 字段的最高位作为 side-set 使能
pub(super) const EXECCTRL_SIDE_EN: u32 = 1 << 30;

/// `SMx_EXECCTRL.SIDE_PINDIR`: side-set drives pin directions instead of levels
/// `SMx_EXECCTRL.SIDE_PINDIR`：side-set 驱动引脚方向而非电平
pub(super) const EXECCTRL_SIDE_PINDIR: u32 = 1 << 29;

/// `SMx_EXECCTRL.JMP_PIN` bit position / `SMx_EXECCTRL.JMP_PIN` 位位置
pub(super) const EXECCTRL_JMP_PIN_SHIFT: u32 = 24;

/// `SMx_EXECCTRL.JMP_PIN` mask / `SMx_EXECCTRL.JMP_PIN` 掩码
pub(super) const EXECCTRL_JMP_PIN_MASK: u32 = 0x1F << EXECCTRL_JMP_PIN_SHIFT;

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

/// `SMx_SHIFTCTRL.FJOIN_RX`: the RX FIFO takes the TX storage and is 8 words deep
/// `SMx_SHIFTCTRL.FJOIN_RX`：RX FIFO 占用 TX 存储，深度变为 8 字
pub(super) const SHIFTCTRL_FJOIN_RX: u32 = 1 << 31;

/// `SMx_SHIFTCTRL.FJOIN_TX`: the TX FIFO takes the RX storage and is 8 words deep
/// `SMx_SHIFTCTRL.FJOIN_TX`：TX FIFO 占用 RX 存储，深度变为 8 字
pub(super) const SHIFTCTRL_FJOIN_TX: u32 = 1 << 30;

/// `SMx_SHIFTCTRL` FIFO-join field mask / `SMx_SHIFTCTRL` FIFO 合并字段掩码
pub(super) const SHIFTCTRL_FJOIN_MASK: u32 = SHIFTCTRL_FJOIN_RX | SHIFTCTRL_FJOIN_TX;

/// `SMx_SHIFTCTRL.PUSH_THRESH` bit position / `SMx_SHIFTCTRL.PUSH_THRESH` 位位置
pub(super) const SHIFTCTRL_PUSH_THRESH_SHIFT: u32 = 20;

/// `SMx_SHIFTCTRL.PULL_THRESH` bit position / `SMx_SHIFTCTRL.PULL_THRESH` 位位置
pub(super) const SHIFTCTRL_PULL_THRESH_SHIFT: u32 = 25;

/// `SMx_SHIFTCTRL.IN_COUNT` mask (RP2350 LSB pins kept in `IN PINS`)
/// `SMx_SHIFTCTRL.IN_COUNT` 掩码（RP2350 `IN PINS` 保留的引脚位数）
pub(super) const SHIFTCTRL_IN_COUNT_MASK: u32 = 0x0000_001F;

/// `SMx_PINCTRL` reset value (`SET_COUNT = 5`) / `SMx_PINCTRL` 复位值
pub(super) const PINCTRL_RESET: u32 = 0x1400_0000;

/// `SMx_PINCTRL.OUT_BASE` bit position / `SMx_PINCTRL.OUT_BASE` 位位置
pub(super) const PINCTRL_OUT_BASE_SHIFT: u32 = 0;

/// `SMx_PINCTRL.SET_BASE` bit position / `SMx_PINCTRL.SET_BASE` 位位置
pub(super) const PINCTRL_SET_BASE_SHIFT: u32 = 5;

/// `SMx_PINCTRL.SIDESET_BASE` bit position / `SMx_PINCTRL.SIDESET_BASE` 位位置
pub(super) const PINCTRL_SIDESET_BASE_SHIFT: u32 = 10;

/// `SMx_PINCTRL.IN_BASE` bit position / `SMx_PINCTRL.IN_BASE` 位位置
pub(super) const PINCTRL_IN_BASE_SHIFT: u32 = 15;

/// `SMx_PINCTRL.OUT_COUNT` bit position / `SMx_PINCTRL.OUT_COUNT` 位位置
pub(super) const PINCTRL_OUT_COUNT_SHIFT: u32 = 20;

/// `SMx_PINCTRL.SET_COUNT` bit position / `SMx_PINCTRL.SET_COUNT` 位位置
pub(super) const PINCTRL_SET_COUNT_SHIFT: u32 = 26;

/// `SMx_PINCTRL.SIDESET_COUNT` bit position / `SMx_PINCTRL.SIDESET_COUNT` 位位置
pub(super) const PINCTRL_SIDESET_COUNT_SHIFT: u32 = 29;

/// Highest legal `SIDESET_COUNT` (values above 5 are reserved)
/// `SIDESET_COUNT` 的合法上限（大于 5 为保留值）
pub(super) const PINCTRL_SIDESET_COUNT_MAX: u8 = 5;

/// `CTRL.CLKDIV_RESTART` base bit / `CTRL.CLKDIV_RESTART` 基位
pub(super) const CLKDIV_RESTART: u32 = 0x0000_0100;

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

/// A word FIFO whose depth is set by the current FIFO-join mode
/// 深度由当前 FIFO 合并模式决定的字 FIFO
///
/// The storage covers the joined depth (8 words) so both directions can share
/// one backing array. A capacity of zero models a disabled FIFO, which the
/// datasheet says reads as both full and empty.
/// 存储按合并深度（8 字）分配，使两个方向共用一份后备数组。容量为 0 表示该
/// FIFO 被禁用，按数据手册其“满”和“空”标志同时为真。
#[derive(Debug, Clone, Copy)]
pub(super) struct Fifo {
    words: [u32; FIFO_JOIN_DEPTH],
    head: usize,
    len: usize,
    capacity: usize,
}

impl Fifo {
    /// Create an empty FIFO with `capacity` words / 创建容量为 `capacity` 的空 FIFO
    pub(super) const fn with_capacity(capacity: usize) -> Self {
        Self {
            words: [0; FIFO_JOIN_DEPTH],
            head: 0,
            len: 0,
            capacity,
        }
    }

    /// Resize and flush the FIFO, as a FIFO-join change does
    /// 调整容量并清空，与 FIFO 合并模式切换的硬件行为一致
    pub(super) fn resize(&mut self, capacity: usize) {
        self.capacity = capacity.min(FIFO_JOIN_DEPTH);
        self.head = 0;
        self.len = 0;
    }

    /// Push one word, returning `false` when the FIFO is full
    /// 压入一个字；FIFO 已满时返回 `false`
    pub(super) fn push(&mut self, value: u32) -> bool {
        if self.is_full() {
            return false;
        }
        let tail = (self.head + self.len) % self.capacity;
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
        self.head = (self.head + 1) % self.capacity;
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
    ///
    /// A disabled FIFO (capacity zero) is always full.
    /// 被禁用的 FIFO（容量为 0）始终为满。
    pub(super) fn is_full(&self) -> bool {
        self.len >= self.capacity
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
    /// `CLKDIV` phase accumulator in 1/256 clock units / `CLKDIV` 相位累加器（1/256 时钟单位）
    pub(super) clock_phase: u32,
    /// Delay cycles still to burn before the next instruction / 下条指令前的剩余延迟周期
    pub(super) delay_cycles: u8,
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
            clock_phase: 0,
            delay_cycles: 0,
            tx: Fifo::with_capacity(FIFO_DEPTH),
            rx: Fifo::with_capacity(FIFO_DEPTH),
        }
    }

    /// Apply a `CTRL.SM_RESTART` strobe: PC, shift counters and stalls
    /// 应用 `CTRL.SM_RESTART` 脉冲：复位 PC、移位计数与阻塞状态
    ///
    /// FIFO contents are preserved, as required by the datasheet. A restart also
    /// resets the clock-divider phase and the remaining delay, as the hardware
    /// restarts the machine's clock.
    /// 按数据手册要求保留 FIFO 内容。重启同时复位分频器相位与剩余延迟，对应硬件
    /// 重启状态机时钟的行为。
    pub(super) fn restart(&mut self) {
        self.pc = 0;
        self.isr = 0;
        self.osr = 0;
        self.isr_bits = 0;
        self.osr_bits = 0;
        self.pending_irq_wait = None;
        self.clock_phase = 0;
        self.delay_cycles = 0;
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

    /// Read one `PINCTRL` bit field / 读取一个 `PINCTRL` 位域
    fn pinctrl_field(&self, shift: u32, bits: u32) -> u8 {
        ((self.pinctrl >> shift) & ((1u32 << bits) - 1)) as u8
    }

    /// `PINCTRL.OUT_BASE` / `PINCTRL.OUT_BASE`
    pub(super) fn out_base(&self) -> u8 {
        self.pinctrl_field(PINCTRL_OUT_BASE_SHIFT, 5)
    }

    /// `PINCTRL.SET_BASE` / `PINCTRL.SET_BASE`
    pub(super) fn set_base(&self) -> u8 {
        self.pinctrl_field(PINCTRL_SET_BASE_SHIFT, 5)
    }

    /// `PINCTRL.SIDESET_BASE` / `PINCTRL.SIDESET_BASE`
    pub(super) fn sideset_base(&self) -> u8 {
        self.pinctrl_field(PINCTRL_SIDESET_BASE_SHIFT, 5)
    }

    /// `PINCTRL.IN_BASE` / `PINCTRL.IN_BASE`
    pub(super) fn in_base(&self) -> u8 {
        self.pinctrl_field(PINCTRL_IN_BASE_SHIFT, 5)
    }

    /// `PINCTRL.OUT_COUNT` (capped at the 32 pin model) / `PINCTRL.OUT_COUNT`（上限 32 引脚）
    pub(super) fn out_count(&self) -> u8 {
        self.pinctrl_field(PINCTRL_OUT_COUNT_SHIFT, 6).min(32)
    }

    /// `PINCTRL.SET_COUNT` / `PINCTRL.SET_COUNT`
    pub(super) fn set_count(&self) -> u8 {
        self.pinctrl_field(PINCTRL_SET_COUNT_SHIFT, 3)
    }

    /// `PINCTRL.SIDESET_COUNT`, clamped to the legal 0..=5 range
    /// `PINCTRL.SIDESET_COUNT`，限制在合法范围 0..=5
    pub(super) fn sideset_count(&self) -> u8 {
        self.pinctrl_field(PINCTRL_SIDESET_COUNT_SHIFT, 3)
            .min(PINCTRL_SIDESET_COUNT_MAX)
    }

    /// `EXECCTRL.JMP_PIN`, the GPIO sampled by `JMP PIN`
    /// `EXECCTRL.JMP_PIN`，`JMP PIN` 采样的 GPIO
    pub(super) fn jmp_pin(&self) -> u8 {
        ((self.execctrl & EXECCTRL_JMP_PIN_MASK) >> EXECCTRL_JMP_PIN_SHIFT) as u8
    }

    /// `EXECCTRL.SIDE_EN` / `EXECCTRL.SIDE_EN`
    pub(super) fn side_en(&self) -> bool {
        self.execctrl & EXECCTRL_SIDE_EN != 0
    }

    /// `EXECCTRL.SIDE_PINDIR` / `EXECCTRL.SIDE_PINDIR`
    pub(super) fn side_pindir(&self) -> bool {
        self.execctrl & EXECCTRL_SIDE_PINDIR != 0
    }

    /// FIFO mode decoded from `SHIFTCTRL` / 由 `SHIFTCTRL` 解析的 FIFO 模式
    pub(super) fn fifo_join(&self) -> FifoJoin {
        match self.shiftctrl & SHIFTCTRL_FJOIN_MASK {
            0 => FifoJoin::None,
            SHIFTCTRL_FJOIN_TX => FifoJoin::Tx,
            SHIFTCTRL_FJOIN_RX => FifoJoin::Rx,
            _ => FifoJoin::Both,
        }
    }

    /// Store `SHIFTCTRL`, resizing and flushing the FIFOs when the join mode changes
    /// 写入 `SHIFTCTRL`；合并模式变化时调整 FIFO 容量并清空
    pub(super) fn write_shiftctrl(&mut self, value: u32) {
        let value = value & SHIFTCTRL_MASK;
        let changed = (self.shiftctrl ^ value) & SHIFTCTRL_FJOIN_MASK != 0;
        self.shiftctrl = value;
        if changed {
            let (tx, rx) = match self.fifo_join() {
                FifoJoin::None => (FIFO_DEPTH, FIFO_DEPTH),
                FifoJoin::Tx => (FIFO_JOIN_DEPTH, 0),
                FifoJoin::Rx => (0, FIFO_JOIN_DEPTH),
                FifoJoin::Both => (0, 0),
            };
            self.tx.resize(tx);
            self.rx.resize(rx);
        }
    }

    /// `CLKDIV` in 1/256 clock units (`INT * 256 + FRAC`) / `CLKDIV` 的 1/256 时钟单位值
    pub(super) fn clkdiv_units(&self) -> u32 {
        (self.clkdiv >> 16) * 256 + ((self.clkdiv >> 8) & 0xFF)
    }

    /// Advance the clock-divider phase, returning `true` when a machine cycle is due
    /// 推进分频相位；本周期应产生一个机器周期时返回 `true`
    ///
    /// The datasheet encodes a divisor of `INT + FRAC/256` in 16.8 fixed point.
    /// Divisors of 1.0 or below (including the invalid `INT = 0` case) tick on
    /// every PIO clock so the model never deadlocks on a zero divisor.
    /// 数据手册用 16.8 定点数表示 `INT + FRAC/256` 的分频比。分频比 ≤ 1.0（含非法的
    /// `INT = 0`）时每个 PIO 时钟都推进，避免零分频让模型死锁。
    pub(super) fn clock_tick(&mut self) -> bool {
        let divisor = self.clkdiv_units();
        if divisor <= 256 {
            return true;
        }
        self.clock_phase += 256;
        if self.clock_phase < divisor {
            return false;
        }
        self.clock_phase -= divisor;
        true
    }
}

/// FIFO joining mode decoded from `SHIFTCTRL.FJOIN_RX`/`FJOIN_TX`
/// 由 `SHIFTCTRL.FJOIN_RX`/`FJOIN_TX` 解析的 FIFO 合并模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FifoJoin {
    /// Two independent 4-deep FIFOs / 两个独立的 4 深度 FIFO
    None,
    /// 8-deep TX FIFO; the RX FIFO is disabled / 8 深度 TX FIFO，RX FIFO 被禁用
    Tx,
    /// 8-deep RX FIFO; the TX FIFO is disabled / 8 深度 RX FIFO，TX FIFO 被禁用
    Rx,
    /// Both bits set: both directions read as full and empty
    /// 两位置位：两个方向同时读为满和空
    Both,
}

/// Low `bits` ones, saturating at 32 / 低 `bits` 位全 1，超过 32 位饱和
pub(super) fn low_bits_mask(bits: u32) -> u32 {
    if bits >= 32 {
        u32::MAX
    } else {
        (1u32 << bits) - 1
    }
}

/// Drive `count` consecutive PIO pins from the low bits of `value`
/// 把 `value` 的低 `count` 位写到从 `base` 起的连续 PIO 引脚
///
/// Pin numbers wrap from 31 back to 0, as on the hardware.
/// 引脚编号按硬件行为从 31 回绕到 0。
pub(super) fn write_pin_bits(target: &mut u32, base: u8, count: u8, value: u32) {
    for bit in 0..count.min(32) {
        let pin = (base + bit) & 31;
        let level = (value >> bit) & 1;
        *target = (*target & !(1u32 << pin)) | (level << pin);
    }
}

/// Sample `count` consecutive PIO pins into the low bits of the result
/// 从 `base` 起连续采样 `count` 个 PIO 引脚，结果按低位对齐
pub(super) fn read_pin_bits(levels: u32, base: u8, count: u8) -> u32 {
    let mut value = 0;
    for bit in 0..count.min(32) {
        let pin = (base + bit) & 31;
        value |= ((levels >> pin) & 1) << bit;
    }
    value
}
