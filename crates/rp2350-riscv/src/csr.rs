//! Machine-mode CSRs and trap entry for Hazard3 / Hazard3 机器模式 CSR 与陷阱入口

/// User interrupt enable in `mstatus` (N extension, storage only)
/// `mstatus` 用户中断使能位（N 扩展，仅存储）
pub(crate) const MSTATUS_UIE: u32 = 1 << 0;

/// Supervisor interrupt enable in `mstatus` (storage only)
/// `mstatus` 主管中断使能位（仅存储）
pub(crate) const MSTATUS_SIE: u32 = 1 << 1;

/// Global interrupt enable in `mstatus` / `mstatus` 全局中断使能位
pub(crate) const MSTATUS_MIE: u32 = 1 << 3;

/// User previous interrupt enable in `mstatus` (N extension, storage only)
/// `mstatus` 用户前一中断使能位（N 扩展，仅存储）
pub(crate) const MSTATUS_UPIE: u32 = 1 << 4;

/// Supervisor previous interrupt enable in `mstatus` (storage only)
/// `mstatus` 主管前一中断使能位（仅存储）
pub(crate) const MSTATUS_SPIE: u32 = 1 << 5;

/// Previous interrupt enable in `mstatus` / `mstatus` 前一中断使能位
pub(crate) const MSTATUS_MPIE: u32 = 1 << 7;

/// Supervisor previous privilege in `mstatus` (storage only)
/// `mstatus` 主管前一特权位（仅存储）
pub(crate) const MSTATUS_SPP: u32 = 1 << 8;

/// Previous privilege mask in `mstatus` (bits 12:11) / `mstatus` 特权级掩码
pub(crate) const MSTATUS_MPP: u32 = 0b11 << 11;

/// Floating-point status mask in `mstatus` (bits 14:13, storage only)
/// `mstatus` 浮点状态掩码（位 14:13，仅存储）
pub(crate) const MSTATUS_FS: u32 = 0b11 << 13;

/// Custom-extension status mask in `mstatus` (bits 16:15, storage only)
/// `mstatus` 自定义扩展状态掩码（位 16:15，仅存储）
pub(crate) const MSTATUS_XS: u32 = 0b11 << 15;

/// Modify privilege for data accesses / 数据访问特权级修饰位
pub(crate) const MSTATUS_MPRV: u32 = 1 << 17;

/// Permit supervisor user-memory access (storage only)
/// 允许主管访问用户内存（仅存储）
pub(crate) const MSTATUS_SUM: u32 = 1 << 18;

/// Make executable readable (storage only) / 可执行页可读（仅存储）
pub(crate) const MSTATUS_MXR: u32 = 1 << 19;

/// Trap on supervisor virtual-memory operations (storage only)
/// 主管虚拟内存操作陷阱（仅存储）
pub(crate) const MSTATUS_TVM: u32 = 1 << 20;

/// Trap on `wfi` in lower privilege modes (storage only)
/// 低特权级 `wfi` 陷阱（仅存储）
pub(crate) const MSTATUS_TW: u32 = 1 << 21;

/// Trap on `sret` in supervisor mode (storage only)
/// 主管模式 `sret` 陷阱（仅存储）
pub(crate) const MSTATUS_TSR: u32 = 1 << 22;

/// Dirty state summary, read-only on RV32 / RV32 上的只读脏状态汇总位
pub(crate) const MSTATUS_SD: u32 = 1 << 31;

/// Writable `mstatus` bits of this model / 本模型可写的 `mstatus` 位
///
/// `SD` is read-only and derived from `FS`/`XS`; every other field listed in
/// the privileged specification is storage. `MIE`/`MPIE`/`MPP` drive trap
/// entry/return and `MPRV` selects the PMP privilege for data accesses.
/// `SD` 只读，由 `FS`/`XS` 推导；特权规范列出的其余字段均为存储位。
/// `MIE`/`MPIE`/`MPP` 驱动陷阱入口/返回，`MPRV` 选择数据访问的 PMP 特权级。
const MSTATUS_WRITABLE: u32 = MSTATUS_UIE
    | MSTATUS_SIE
    | MSTATUS_MIE
    | MSTATUS_UPIE
    | MSTATUS_SPIE
    | MSTATUS_MPIE
    | MSTATUS_SPP
    | MSTATUS_MPP
    | MSTATUS_FS
    | MSTATUS_XS
    | MSTATUS_MPRV
    | MSTATUS_SUM
    | MSTATUS_MXR
    | MSTATUS_TVM
    | MSTATUS_TW
    | MSTATUS_TSR;

/// `mcountinhibit.CY`: stop `mcycle` / `mcountinhibit.CY`：停止 `mcycle`
pub(crate) const MCOUNTINHIBIT_CY: u32 = 1 << 0;

/// `mcountinhibit.IR`: stop `minstret` / `mcountinhibit.IR`：停止 `minstret`
pub(crate) const MCOUNTINHIBIT_IR: u32 = 1 << 2;

/// Writable `mcountinhibit` bits / 可写的 `mcountinhibit` 位
///
/// Bit 1 is reserved and the HPM bits (3..31) name counters this model does not
/// implement, so both read as zero.
/// 位 1 保留；HPM 位（3..31）对应该模型未实现的计数器，因此两者读出为 0。
const MCOUNTINHIBIT_WRITABLE: u32 = MCOUNTINHIBIT_CY | MCOUNTINHIBIT_IR;

/// Machine external interrupt enable in `mie` / `mie` 机器外部中断使能位
pub(crate) const MIE_MEIE: u32 = 1 << 11;

/// Machine external interrupt pending in `mip` / `mip` 机器外部中断挂起位
pub(crate) const MIP_MEIP: u32 = 1 << 11;

/// Machine ISA register value: RV32IMAC plus the `B` bit manipulation extension
/// 机器 ISA 寄存器值：RV32IMAC 加 `B` 位操作扩展
///
/// Bitmap: `A` = bit 0, `B` = bit 1, `C` = bit 2, `I` = bit 8, `M` = bit 12,
/// `MXL` = bits 31:30 = `0b01` (32-bit). The Zba/Zbb/Zbs subsets implemented by
/// the core are advertised through the single `B` bit, as the unified bitmanip
/// specification requires.
/// 位图：`A` = 位 0、`B` = 位 1、`C` = 位 2、`I` = 位 8、`M` = 位 12，
/// `MXL` = 位 31:30 = `0b01`（32 位）。核心实现的 Zba/Zbb/Zbs 子集统一通过
/// 单个 `B` 位宣传，这是统一位操作规范的要求。
const MISA_RV32IMACB: u32 = 0x4000_1107;

/// `mhartid` value: hart 0 / `mhartid` 值：hart 0
///
/// Both RP2350 Hazard3 cores report hart 0 to the guest: firmware uses the value
/// only as a boot-time sanity check, and the SIO mailbox identifies the core.
/// 两个 RP2350 Hazard3 核心都向客户机报告 hart 0：固件只把它当作启动期的一致性
/// 检查，核心身份由 SIO 邮箱区分。
const MHARTID: u32 = 0;

/// Number of 16-bit Xh3irq windows / Xh3irq 16 位窗口数量
const XH3IRQ_WINDOWS: usize = 4;

/// Shift of `meicontext.preempt` (5 bits) / `meicontext.preempt` 的位偏移
const MEICONTEXT_PREEMPT_SHIFT: u32 = 16;

/// Shift of `meicontext.ppreempt` (4 bits) / `meicontext.ppreempt` 的位偏移
const MEICONTEXT_PPREEMPT_SHIFT: u32 = 24;

/// Shift of `meicontext.pppreempt` (4 bits) / `meicontext.pppreempt` 的位偏移
const MEICONTEXT_PPPREEMPT_SHIFT: u32 = 28;

/// `meicontext.MRETEIRQ` bit / `meicontext.MRETEIRQ` 位
const MEICONTEXT_MRETEIRQ: u32 = 1;

/// `meicontext.NOIRQ` bit / `meicontext.NOIRQ` 位
const MEICONTEXT_NOIRQ: u32 = 1 << 15;

/// Xh3irq external interrupt controller state / Xh3irq 外部中断控制器状态
///
/// The CSRs at `0xBE0..=0xBE3` are 16-bit arrays addressed by the low five bits
/// of the CSR operand; the upper 16 bits carry the data. `meinext` and
/// `meicontext` provide the current-IRQ window used by the SDK's dispatch loop.
/// `0xBE0..=0xBE3` 的 CSR 是由操作数低 5 位索引的 16 位数组，高 16 位承载数据。
/// `meinext` 与 `meicontext` 提供 SDK 分发循环使用的当前 IRQ 窗口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Xh3Irq {
    /// `meiea`: per-IRQ enable bits / `meiea`：每 IRQ 使能位
    pub enable: [u16; XH3IRQ_WINDOWS],
    /// `meipa`: per-IRQ pending bits / `meipa`：每 IRQ 挂起位
    pub pending: [u16; XH3IRQ_WINDOWS],
    /// `meifa`: per-IRQ force bits / `meifa`：每 IRQ 强制位
    pub force: [u16; XH3IRQ_WINDOWS],
    /// `meipra`: four 4-bit priority fields per row / `meipra`：每行四个 4 位优先级
    pub priority: [u16; 16],
    /// `meinext`: next/current IRQ encoded as `irq << 2`, bit 31 = NOIRQ
    /// `meinext`：下一个/当前 IRQ，编码为 `irq << 2`，位 31 为 NOIRQ
    pub next: u32,
    /// `meicontext`: current IRQ context / `meicontext`：当前 IRQ 上下文
    pub context: u32,
}

impl Xh3Irq {
    /// Create the reset state: no pending IRQ, NOIRQ set
    /// 创建复位状态：无挂起 IRQ，NOIRQ 置位
    pub(crate) fn new() -> Self {
        Self {
            enable: [0; XH3IRQ_WINDOWS],
            pending: [0; XH3IRQ_WINDOWS],
            force: [0; XH3IRQ_WINDOWS],
            priority: [0; 16],
            next: 1 << 31,
            context: 1 << 15,
        }
    }

    /// Window index encoded in a CSR operand / CSR 操作数中编码的窗口索引
    fn window(raw: u32) -> usize {
        usize::try_from(raw & 0x1F).unwrap_or(0) % XH3IRQ_WINDOWS
    }

    /// Read one 16-bit array window into the upper halfword
    /// 将一个 16 位数组窗口读到高半字
    fn read_window(array: [u16; XH3IRQ_WINDOWS], raw: u32) -> u32 {
        u32::from(array[Self::window(raw)]) << 16
    }

    /// Read the effective pending window (`meipa = irq_r | meifa`)
    /// 读取有效挂起窗口（`meipa = irq_r | meifa`）
    fn read_pending_window(&self, raw: u32) -> u32 {
        let window = Self::window(raw);
        u32::from(self.pending[window] | self.force[window]) << 16
    }

    /// Write the upper halfword into the window selected by the raw operand
    /// 将高半字写入由原始操作数选择的窗口
    fn write_window(array: &mut [u16; XH3IRQ_WINDOWS], raw: u32, value: u32) {
        array[Self::window(raw)] = (value >> 16) as u16;
    }

    /// Read a four-bit priority field / 读取 4 位优先级字段
    pub(crate) fn priority_of(&self, irq: u16) -> u8 {
        let row = usize::from(irq) / 4;
        let shift = 4 * (usize::from(irq) % 4);
        ((self.priority[row] >> shift) & 0xF) as u8
    }

    /// Return the current preemption level (5 bits; 16 disables preemption)
    /// 返回当前抢占级别（5 位；16 表示禁止抢占）
    pub(crate) fn preempt(&self) -> u8 {
        ((self.context >> MEICONTEXT_PREEMPT_SHIFT) & 0x1F) as u8
    }

    /// Return the previous preemption level used for `meinext` visibility
    /// 返回用于 `meinext` 可见性的前一级抢占级别
    pub(crate) fn ppreempt(&self) -> u8 {
        ((self.context >> MEICONTEXT_PPREEMPT_SHIFT) & 0xF) as u8
    }

    /// Return the highest-priority enabled pending IRQ at or above `min_priority`
    /// 返回优先级不低于 `min_priority` 的最高优先级已使能挂起 IRQ
    ///
    /// Ties are broken by the lowest IRQ number, matching Hazard3's candidate
    /// ordering. / 同优先级时取编号最小者，与 Hazard3 的候选排序一致。
    pub(crate) fn best_visible(&self, min_priority: u8) -> Option<u16> {
        let mut best: Option<(u8, u16)> = None;
        for window in 0..XH3IRQ_WINDOWS {
            let ready = self.enable[window] & (self.pending[window] | self.force[window]);
            let mut bits = ready;
            while bits != 0 {
                let bit = bits.trailing_zeros() as u16;
                bits &= !(1u16 << bit);
                let irq = (window as u16) * 16 + bit;
                let priority = self.priority_of(irq);
                if priority < min_priority {
                    continue;
                }
                let better = match best {
                    None => true,
                    Some((best_priority, best_irq)) => {
                        priority > best_priority || (priority == best_priority && irq < best_irq)
                    }
                };
                if better {
                    best = Some((priority, irq));
                }
            }
        }
        best.map(|(_, irq)| irq)
    }

    /// Recompute `meinext` from enable/pending/force/priority and `ppreempt`
    /// 根据使能/挂起/强制/优先级与 `ppreempt` 重算 `meinext`
    pub(crate) fn update_next(&mut self) {
        self.next = self
            .best_visible(self.ppreempt())
            .map_or(1 << 31, |irq| u32::from(irq) << 2);
    }

    /// Update `meicontext` when software writes `meinext.update`
    /// 软件写 `meinext.update` 时更新 `meicontext`
    ///
    /// The write claims the IRQ returned by the preceding `meinext` read; the
    /// emulator consumes its pending/force bit here so the pico-sdk dispatch
    /// loop terminates. A level source that is still asserted is re-raised by
    /// the platform on its next poll. / 该写入认领前一次 `meinext` 读返回的
    /// IRQ；模拟器在此消费其挂起/强制位，使 pico-sdk 分发循环能够结束。仍然
    /// 有效的电平源会在平台下一次轮询时重新置位。
    fn update_context(&mut self) {
        if self.next >> 31 == 0 {
            let irq = ((self.next >> 2) & 0x1FF) as u16;
            let window = usize::from(irq) / 16;
            let bit = 1u16 << (irq % 16);
            self.pending[window] &= !bit;
            self.force[window] &= !bit;
        }
        let noirq = self.next >> 31;
        self.context &= !(MEICONTEXT_NOIRQ | (0x1FF << 4) | (0x1F << MEICONTEXT_PREEMPT_SHIFT));
        if noirq != 0 {
            self.context |= MEICONTEXT_NOIRQ | (16 << MEICONTEXT_PREEMPT_SHIFT);
        } else {
            let irq = ((self.next >> 2) & 0x1FF) as u16;
            self.context |= u32::from(irq) << 4;
            self.context |= (u32::from(self.priority_of(irq) + 1)) << MEICONTEXT_PREEMPT_SHIFT;
        }
        self.update_next();
    }

    /// Enter an external interrupt: set the preemption stack and select the IRQ
    /// 进入外部中断：压入抢占栈并选定当前 IRQ
    ///
    /// The pending bit is consumed (edge-style delivery); a level source that
    /// remains asserted re-raises it on the next platform poll. `preempt` is
    /// set to `1 + priority` so the same or a lower-priority IRQ cannot nest
    /// into its own handler. `NOIRQ` and the current-IRQ field are deliberately
    /// left untouched: the trap handler saves `meicontext` on entry and restores
    /// it before `mret`, so clearing NOIRQ here would make task-context code
    /// believe it is still inside an interrupt. / 挂起位在此消费（边沿式投递）；
    /// 仍保持有效的电平源会在下一次平台轮询时重新置位。`preempt` 取 `1 + 优先级`，
    /// 使同优先级或更低优先级 IRQ 无法在自身处理程序中嵌套。`NOIRQ` 与当前 IRQ
    /// 字段有意保持不变：陷阱处理程序在入口保存 `meicontext`、在 `mret` 前恢复，
    /// 若在此清除 NOIRQ，任务上下文代码会误以为自己仍在中断内。
    pub(crate) fn enter(&mut self, irq: u16) {
        let window = usize::from(irq) / 16;
        let bit = 1u16 << (irq % 16);
        self.pending[window] &= !bit;
        let old = self.context;
        self.context &= !((0x1F << MEICONTEXT_PREEMPT_SHIFT)
            | (0xF << MEICONTEXT_PPREEMPT_SHIFT)
            | (0xF << MEICONTEXT_PPPREEMPT_SHIFT)
            | MEICONTEXT_MRETEIRQ);
        self.context |= ((old >> MEICONTEXT_PREEMPT_SHIFT) & 0xF) << MEICONTEXT_PPREEMPT_SHIFT;
        self.context |= ((old >> MEICONTEXT_PPREEMPT_SHIFT) & 0xF) << MEICONTEXT_PPPREEMPT_SHIFT;
        self.context |= u32::from(self.priority_of(irq) + 1) << MEICONTEXT_PREEMPT_SHIFT;
        self.context |= MEICONTEXT_MRETEIRQ;
        self.next = u32::from(irq) << 2;
    }

    /// Pop the preemption stack on `mret` when the trap was an external IRQ
    /// `mret` 时对由外部中断进入的陷阱弹出抢占栈
    // `pppreempt` 与 `ppreempt` 是 Hazard3 规范中的三级栈字段名，名字相近但
    // 语义不同，保持规范命名。
    // `pppreempt` and `ppreempt` are Hazard3's three-level stack field names;
    // they look similar but mean different levels, so the spec names are kept.
    #[allow(clippy::similar_names)]
    pub(crate) fn restore_on_mret(&mut self) {
        if self.context & MEICONTEXT_MRETEIRQ == 0 {
            return;
        }
        let old = self.context;
        let pppreempt = (old >> MEICONTEXT_PPPREEMPT_SHIFT) & 0xF;
        let ppreempt = (old >> MEICONTEXT_PPREEMPT_SHIFT) & 0xF;
        self.context &= !((0x1F << MEICONTEXT_PREEMPT_SHIFT)
            | (0xF << MEICONTEXT_PPREEMPT_SHIFT)
            | (0xF << MEICONTEXT_PPPREEMPT_SHIFT)
            | MEICONTEXT_MRETEIRQ);
        self.context |= pppreempt << MEICONTEXT_PPREEMPT_SHIFT;
        self.context |= ppreempt << MEICONTEXT_PREEMPT_SHIFT;
        self.update_next();
    }
}

/// Machine-mode CSR file / 机器模式 CSR 文件
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CsrFile {
    /// `mstatus` register / `mstatus` 寄存器
    pub mstatus: u32,
    /// `mie` register / `mie` 寄存器
    pub mie: u32,
    /// `mtvec` trap vector / `mtvec` 陷阱向量
    pub mtvec: u32,
    /// `mscratch` scratch register / `mscratch` 暂存寄存器
    pub mscratch: u32,
    /// `mepc` exception program counter / `mepc` 异常程序计数器
    pub mepc: u32,
    /// `mcause` exception cause / `mcause` 异常原因
    pub mcause: u32,
    /// `mtval` trap value / `mtval` 陷阱值
    pub mtval: u32,
    /// `mip` pending interrupt bits / `mip` 挂起中断位
    pub mip: u32,
    /// `mstatush` high half of `mstatus` (storage only)
    /// `mstatush`：`mstatus` 的高半部分（仅存储）
    pub mstatush: u32,
    /// `mcounteren` counter enable / `mcounteren` 计数器使能
    pub mcounteren: u32,
    /// `mcountinhibit` counter inhibit / `mcountinhibit` 计数器抑制
    pub mcountinhibit: u32,
    /// `pmpcfg0`/`pmpcfg2`/`pmpcfg4`/`pmpcfg6` PMP configuration
    /// `pmpcfg0`/`pmpcfg2`/`pmpcfg4`/`pmpcfg6` PMP 配置
    ///
    /// RV32 only defines the even-numbered configuration registers, each holding
    /// four 8-bit entry descriptors. / RV32 只定义偶数编号的配置寄存器，每个持有
    /// 四个 8 位条目描述符。
    pub pmpcfg: [u32; 4],
    /// `pmpaddr0..15` PMP address registers / `pmpaddr0..15` PMP 地址寄存器
    pub pmpaddr: [u32; 16],
    /// Xh3irq external interrupt controller / Xh3irq 外部中断控制器
    pub xh3irq: Xh3Irq,
    /// Retired instruction counter / 已退休指令计数
    pub minstret: u64,
    /// Cycle counter / 周期计数
    pub mcycle: u64,
}

impl CsrFile {
    /// Create a CSR file with architectural reset values / 创建带架构复位值的 CSR 文件
    pub(crate) fn new() -> Self {
        Self {
            mstatus: 0,
            mie: 0,
            mtvec: 0,
            mscratch: 0,
            mepc: 0,
            mcause: 0,
            mtval: 0,
            mip: 0,
            mstatush: 0,
            mcounteren: 0,
            mcountinhibit: 0,
            pmpcfg: [0; 4],
            pmpaddr: [0; 16],
            xh3irq: Xh3Irq::new(),
            minstret: 0,
            mcycle: 0,
        }
    }

    /// Advance the cycle and instruction counters by one / 周期与指令计数器加一
    ///
    /// `mcountinhibit.CY` freezes `mcycle`/`mcycleh` and `mcountinhibit.IR`
    /// freezes `minstret`/`minstreth`; the counters keep their value while
    /// frozen and resume from it when the inhibit bit is cleared.
    /// `mcountinhibit.CY` 冻结 `mcycle`/`mcycleh`，`mcountinhibit.IR` 冻结
    /// `minstret`/`minstreth`；冻结期间计数器保持原值，清除抑制位后从原值继续。
    pub(crate) fn tick(&mut self) {
        if self.mcountinhibit & MCOUNTINHIBIT_CY == 0 {
            self.mcycle = self.mcycle.wrapping_add(1);
        }
        if self.mcountinhibit & MCOUNTINHIBIT_IR == 0 {
            self.minstret = self.minstret.wrapping_add(1);
        }
    }

    /// Read `mstatus` with the read-only `SD` summary bit applied
    /// 读取 `mstatus`，并应用只读 `SD` 汇总位
    ///
    /// The privileged specification defines `SD = (FS == 11) || (XS == 11)`;
    /// software cannot write it, so it is derived on every read.
    /// 特权规范定义 `SD = (FS == 11) || (XS == 11)`；软件不能写该位，因此每次读取
    /// 时重新推导。
    fn status_read(&self) -> u32 {
        let dirty =
            self.mstatus & MSTATUS_FS == MSTATUS_FS || self.mstatus & MSTATUS_XS == MSTATUS_XS;
        if dirty {
            self.mstatus | MSTATUS_SD
        } else {
            self.mstatus
        }
    }

    /// Report whether data accesses run with machine privilege
    /// 报告数据访问是否以机器特权级运行
    ///
    /// `mstatus.MPRV` redirects data accesses through `MPP`; instruction
    /// fetches always use the current privilege mode.
    /// `mstatus.MPRV` 让数据访问按 `MPP` 指定的特权级检查；取指始终使用当前特权级。
    pub(crate) fn data_access_is_machine(&self) -> bool {
        if self.mstatus & MSTATUS_MPRV == 0 {
            return true;
        }
        self.mstatus & MSTATUS_MPP == MSTATUS_MPP
    }

    /// Read a CSR by address, or report that it is not implemented
    /// 按地址读取 CSR，或报告该 CSR 未实现
    ///
    /// `None` means the address names no CSR of this core; the caller must raise
    /// an illegal-instruction trap (cause 2) rather than fabricate a value.
    /// `raw` is the instruction's raw source operand, used by the Xh3irq 16-bit
    /// arrays to select a window.
    /// `None` 表示该地址不对应本核心的任何 CSR，调用方必须触发非法指令陷阱
    /// （cause 2）而不是伪造数值。`raw` 是指令的原始源操作数，供 Xh3irq 的
    /// 16 位数组选择窗口。
    pub(crate) fn try_read(&self, addr: u32, raw: u32) -> Option<u32> {
        let value = match addr {
            0x300 => self.status_read(),
            0x301 => MISA_RV32IMACB,
            0x304 => self.mie,
            0x305 => self.mtvec,
            0x306 => self.mcounteren,
            0x310 => self.mstatush,
            0x320 => self.mcountinhibit,
            0x340 => self.mscratch,
            0x341 => self.mepc,
            0x342 => self.mcause,
            0x343 => self.mtval,
            0x344 => self.mip,
            // RV32 defines pmpcfg0/2/4/6 only; the odd-numbered registers do not
            // exist and stay unimplemented. / RV32 只定义 pmpcfg0/2/4/6，奇数
            // 编号的寄存器不存在，保持未实现。
            0x3A0 | 0x3A2 | 0x3A4 | 0x3A6 => self.pmpcfg[((addr - 0x3A0) >> 1) as usize],
            addr if (0x3B0..=0x3EF).contains(&addr) => self.pmpaddr[((addr - 0x3B0) >> 2) as usize],
            0xB00 | 0xC00 => self.mcycle as u32,
            0xB02 | 0xC02 => self.minstret as u32,
            0xB80 | 0xC80 => (self.mcycle >> 32) as u32,
            0xB82 | 0xC82 => (self.minstret >> 32) as u32,
            // mvendorid/marchid/mimpid/mhartid are read-only identification CSRs
            // with constant values in the Hazard3 configuration.
            // mvendorid/marchid/mimpid/mhartid 是只读识别 CSR，在 Hazard3 配置中
            // 取常量值。
            0xF11..=0xF13 => 0,
            0xF14 => MHARTID,
            0xBE0 => Xh3Irq::read_window(self.xh3irq.enable, raw),
            // MEIPA is the OR of the hardware pending array and MEIFA, as
            // required by the Xh3irq specification.
            // 按 Xh3irq 规范，MEIPA 是硬件挂起数组与 MEIFA 的按位或。
            0xBE1 => self.xh3irq.read_pending_window(raw),
            0xBE2 => Xh3Irq::read_window(self.xh3irq.force, raw),
            0xBE3 => {
                let index = usize::try_from(raw & 0xF).unwrap_or(0) % 16;
                u32::from(self.xh3irq.priority[index]) << 16
            }
            0xBE4 => self.xh3irq.next,
            0xBE5 => self.xh3irq.context,
            _ => return None,
        };
        Some(value)
    }

    /// Read a CSR for host diagnostics / 供宿主诊断读取 CSR
    ///
    /// Unimplemented addresses read as zero here so host-side reports never
    /// fault; the instruction path uses [`Self::try_read`] and raises cause 2.
    /// 此处未实现的地址读为 0，使宿主侧报告不会出错；指令路径改用
    /// [`Self::try_read`] 并触发 cause 2。
    pub(crate) fn read(&self, addr: u32, raw: u32) -> u32 {
        self.try_read(addr, raw).unwrap_or(0)
    }

    /// Report whether a CSR address is architecturally read-only
    /// 报告某个 CSR 地址是否为架构只读
    ///
    /// CSR address bits `[11:10] == 0b11` select the read-only space (`cycle` and
    /// friends); `misa` sits in the read/write encoding space but is hard-wired
    /// in Hazard3.
    /// CSR 地址位 `[11:10] == 0b11` 表示只读空间（`cycle` 等）；`misa` 位于读写
    /// 编码空间，但在 Hazard3 中是硬连线只读。
    pub(crate) fn is_read_only(addr: u32) -> bool {
        (addr >> 10) & 0b11 == 0b11 || addr == 0x301
    }

    /// Report whether a write to `addr` is legal / 报告对 `addr` 的写是否合法
    ///
    /// A write is illegal when the CSR is unimplemented or read-only; both raise
    /// illegal-instruction, as the privileged specification requires.
    /// CSR 未实现或只读时写操作非法；两者都按特权规范触发非法指令陷阱。
    pub(crate) fn is_writable(&self, addr: u32) -> bool {
        !Self::is_read_only(addr) && self.try_read(addr, 0).is_some()
    }

    /// Recompute `meinext` and `mip.MEIP` after an Xh3irq state change
    /// Xh3irq 状态变化后重算 `meinext` 与 `mip.MEIP`
    pub(crate) fn refresh_xh3irq(&mut self) {
        self.xh3irq.update_next();
        if self.xh3irq.next >> 31 == 0 {
            self.mip |= MIP_MEIP;
        } else {
            self.mip &= !MIP_MEIP;
        }
    }

    /// Write a CSR by address / 按地址写入 CSR
    ///
    /// Read-only and unmodelled CSRs ignore writes. Instruction execution never
    /// reaches those cases: [`Self::is_writable`] gates every guest write and
    /// raises illegal-instruction instead.
    /// 只读与未建模的 CSR 忽略写入。指令执行不会走到这些分支：
    /// [`Self::is_writable`] 会拦截所有客户机写操作并改为触发非法指令陷阱。
    #[cfg(test)]
    pub(crate) fn write(&mut self, addr: u32, value: u32) {
        self.write_raw(addr, value, value);
    }

    /// Write a CSR using the instruction's raw source operand as the array index
    /// 使用指令的原始源操作数作为数组索引写入 CSR
    ///
    /// The Xh3irq 16-bit arrays take their window from bits [4:0] of the
    /// instruction operand, not from the read-modify-write result. For
    /// `csrrc`/`csrrs` the result has those index bits masked away (or XORed),
    /// so the raw operand must be passed separately or a clear of window 1 can
    /// land in window 0. / Xh3irq 的 16 位数组从指令操作数的低 5 位取窗口，
    /// 而不是从读-改-写结果取。`csrrc`/`csrrs` 的结果可能把这些索引位清掉
    /// （或异或掉），因此必须单独传入原始操作数，否则窗口 1 的清位会落到
    /// 窗口 0。
    pub(crate) fn write_raw(&mut self, addr: u32, value: u32, raw: u32) {
        match addr {
            0x300 => {
                // `SD` is read-only and absent from the writable mask; all
                // other modeled fields keep the written value.
                // `SD` 只读且不在可写掩码中；其余建模字段保留写入值。
                self.mstatus = (self.mstatus & !MSTATUS_WRITABLE) | (value & MSTATUS_WRITABLE);
            }
            0x304 => self.mie = value,
            0x305 => self.mtvec = value,
            0x306 => self.mcounteren = value,
            0x310 => self.mstatush = value,
            0x320 => self.mcountinhibit = value & MCOUNTINHIBIT_WRITABLE,
            0x340 => self.mscratch = value,
            0x341 => self.mepc = value & !1,
            0x342 => self.mcause = value,
            0x343 => self.mtval = value,
            0x344 => self.mip = value & MIP_MEIP,
            0x3A0 | 0x3A2 | 0x3A4 | 0x3A6 => {
                self.pmp_write_cfg(((addr - 0x3A0) >> 1) as usize, value);
            }
            addr if (0x3B0..=0x3EF).contains(&addr) => {
                self.pmp_write_addr(((addr - 0x3B0) >> 2) as usize, value);
            }
            0xB00 => self.mcycle = (self.mcycle & 0xFFFF_FFFF_0000_0000) | u64::from(value),
            0xB02 => self.minstret = (self.minstret & 0xFFFF_FFFF_0000_0000) | u64::from(value),
            0xB80 => self.mcycle = (self.mcycle & 0xFFFF_FFFF) | (u64::from(value) << 32),
            0xB82 => self.minstret = (self.minstret & 0xFFFF_FFFF) | (u64::from(value) << 32),
            0xBE0 => {
                Xh3Irq::write_window(&mut self.xh3irq.enable, raw, value);
                self.refresh_xh3irq();
            }
            // MEIPA is hardware pending state; writes (including 0xBE1)
            // fall through to the wildcard and are ignored.
            // MEIPA 是硬件挂起状态；写入（包括 0xBE1）落到通配分支并被忽略。
            0xBE2 => {
                Xh3Irq::write_window(&mut self.xh3irq.force, raw, value);
                self.refresh_xh3irq();
            }
            0xBE3 => {
                let index = usize::try_from(raw & 0x1F).unwrap_or(0) % 16;
                self.xh3irq.priority[index] = (value >> 16) as u16;
                self.refresh_xh3irq();
            }
            0xBE4 => {
                if value & 1 != 0 {
                    self.xh3irq.update_context();
                    self.refresh_xh3irq();
                }
            }
            0xBE5 => {
                // CLEARTS (bit 1) clears and saves MIE.MTIE/MSIE; the firmware
                // only relies on the saved values being visible in meicontext.
                // CLEARTS（位 1）会清除并保存 MIE.MTIE/MSIE；固件只依赖保存值
                // 在 meicontext 中可见。
                self.xh3irq.context = value;
                self.refresh_xh3irq();
            }
            _ => {}
        }
    }

    /// Enter a machine-mode trap and return the trap handler PC
    /// 进入机器模式陷阱并返回陷阱处理程序 PC
    pub(crate) fn take_trap(&mut self, cause: u32, tval: u32, epc: u32) -> u32 {
        // RISC-V instructions are 2-byte aligned; masking bit 1 would point
        // mepc at the previous instruction after a compressed instruction fault.
        // RISC-V 指令按 2 字节对齐；屏蔽位 1 会让压缩指令出错后的 mepc 指向前一条。
        self.mepc = epc & !1;
        self.mcause = cause;
        self.mtval = tval;
        // 非外部中断的陷阱会清除 MRETEIRQ，避免之后的 mret 误弹出抢占栈。
        // Non-external traps clear MRETEIRQ so a later mret does not pop the
        // preemption stack by mistake.
        if cause & 0x8000_0000 == 0 {
            self.xh3irq.context &= !MEICONTEXT_MRETEIRQ;
        }

        let previous_enable = self.mstatus & MSTATUS_MIE;
        self.mstatus &= !(MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP);
        if previous_enable != 0 {
            self.mstatus |= MSTATUS_MPIE;
        }
        self.mstatus |= MSTATUS_MPP;

        self.trap_pc(cause)
    }

    /// Resolve the PC for a trap / 计算陷阱目标 PC
    ///
    /// Hazard3 vector mode only applies to interrupts (`mcause[31] == 1`).
    /// Synchronous exceptions always jump to `mtvec` BASE, as required by the
    /// RISC-V privileged specification and by `hazard3_csr.v`'s `vector_sel`.
    /// Hazard3 的向量模式仅适用于中断（`mcause[31] == 1`）。同步异常始终跳到
    /// `mtvec` 的 BASE，这既是 RISC-V 特权规范的要求，也符合
    /// `hazard3_csr.v` 中 `vector_sel` 的行为。
    pub(crate) fn trap_pc(&self, cause: u32) -> u32 {
        let base = self.mtvec & !3;
        if self.mtvec & 3 == 1 && cause & 0x8000_0000 != 0 {
            base.wrapping_add(4 * (cause & 0xF))
        } else {
            base
        }
    }

    /// Execute `mret` and return the restored PC / 执行 `mret` 并返回恢复的 PC
    pub(crate) fn mret(&mut self) -> u32 {
        let mpie = self.mstatus & MSTATUS_MPIE;
        self.mstatus &= !(MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP);
        if mpie != 0 {
            self.mstatus |= MSTATUS_MIE;
        }
        // The specification sets MPIE=1 on every xRET, clears MPP and returns
        // to the least-privileged supported mode (U, i.e. zero here).
        // 规范规定每次 xRET 都把 MPIE 置 1、清空 MPP，并返回支持的最低特权级
        // （此处为 U，即 0）。
        self.mstatus |= MSTATUS_MPIE;
        // Hazard3's mret clears MRETEIRQ, restores the preemption stack and
        // refreshes the next-IRQ window.
        // Hazard3 的 mret 清除 MRETEIRQ、恢复抢占栈并刷新下一 IRQ 窗口。
        self.xh3irq.restore_on_mret();
        self.refresh_xh3irq();
        self.mepc & !1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trap_saves_state_and_updates_mstatus() {
        let mut csr = CsrFile::new();
        csr.mstatus = MSTATUS_MIE;
        csr.mtvec = 0x2000_0100;
        csr.take_trap(11, 0, 0x2000_0042);
        // 2-byte alignment preserves the faulting compressed instruction.
        // 2 字节对齐保留出错的压缩指令地址。
        assert_eq!(csr.mepc, 0x2000_0042);
        assert_eq!(csr.mcause, 11);
        assert_eq!(csr.mstatus & MSTATUS_MIE, 0);
        assert_ne!(csr.mstatus & MSTATUS_MPIE, 0);
        assert_eq!(csr.mstatus & MSTATUS_MPP, MSTATUS_MPP);
        assert_eq!(csr.trap_pc(11), 0x2000_0100);
    }

    #[test]
    fn vectored_trap_offsets_only_for_interrupts() {
        let mut csr = CsrFile::new();
        csr.mtvec = 0x2000_0101;
        // Synchronous exceptions always use mtvec BASE.
        // 同步异常始终使用 mtvec 的 BASE。
        assert_eq!(csr.trap_pc(5), 0x2000_0100);
        assert_eq!(csr.trap_pc(11), 0x2000_0100);
        // External interrupts vector by cause (0x8000_000B -> slot 11).
        // 外部中断按 cause 向量（0x8000_000B -> 槽 11）。
        assert_eq!(csr.trap_pc(0x8000_000B), 0x2000_0100 + 44);
    }

    #[test]
    fn mret_restores_interrupt_enable() {
        let mut csr = CsrFile::new();
        csr.mstatus = MSTATUS_MPIE | MSTATUS_MPP | MSTATUS_MIE;
        csr.mepc = 0x2000_0080;
        assert_eq!(csr.mret(), 0x2000_0080);
        assert_ne!(csr.mstatus & MSTATUS_MIE, 0);
        assert_ne!(csr.mstatus & MSTATUS_MPIE, 0);
        assert_eq!(csr.mstatus & MSTATUS_MPP, 0);
    }

    #[test]
    fn meipa_or_combines_hardware_pending_and_force() {
        let mut csr = CsrFile::new();
        // MEIFA window 0 forces IRQ 0; MEIPA must expose irq_r | meifa.
        // MEIFA 窗口 0 强制 IRQ 0；MEIPA 必须暴露 irq_r | meifa。
        csr.write(0xBE2, 1 << 16);
        assert_eq!(csr.read(0xBE1, 0) >> 16, 1);
        csr.xh3irq.pending[0] |= 1 << 1;
        assert_eq!(csr.read(0xBE1, 0) >> 16, 0b11);
        // Writes to the hardware pending array are ignored.
        // 对硬件挂起数组的写入被忽略。
        csr.write(0xBE1, 0xFFFF_FFFF);
        assert_eq!(csr.read(0xBE1, 0) >> 16, 0b11);
    }

    #[test]
    fn meinext_update_consumes_the_claimed_irq() {
        // pico-sdk 的分发循环靠写 meinext.update 认领 IRQ；认领后挂起位必须消失，
        // 否则循环永不结束。 / The pico-sdk dispatch loop claims the IRQ by
        // writing meinext.update; the pending bit must disappear or the loop
        // never terminates.
        let mut csr = CsrFile::new();
        csr.xh3irq.enable[0] = 1 << 3;
        csr.xh3irq.pending[0] = 1 << 3;
        csr.refresh_xh3irq();
        csr.xh3irq.next = 3 << 2;
        csr.write(0xBE4, 1);
        assert_eq!(csr.xh3irq.pending[0] & (1 << 3), 0);
        assert_eq!(csr.xh3irq.next, 1 << 31);
    }

    #[test]
    fn mret_restores_the_preemption_stack() {
        let mut csr = CsrFile::new();
        // preempt=8, ppreempt=3, pppreempt=2, MRETEIRQ=1.
        // preempt=8、ppreempt=3、pppreempt=2、MRETEIRQ=1。
        csr.xh3irq.context = (8 << 16) | (3 << 24) | (2 << 28) | 1;
        csr.mstatus = MSTATUS_MPIE;
        csr.mepc = 0x2000_0080;
        assert_eq!(csr.mret(), 0x2000_0080);
        assert_eq!(csr.xh3irq.preempt(), 3);
        assert_eq!(csr.xh3irq.ppreempt(), 2);
        assert_eq!(csr.xh3irq.context & 1, 0);
    }
}
