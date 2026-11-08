//! Machine-mode CSRs and trap entry for Hazard3 / Hazard3 机器模式 CSR 与陷阱入口

/// Global interrupt enable in `mstatus` / `mstatus` 全局中断使能位
pub(crate) const MSTATUS_MIE: u32 = 1 << 3;

/// Previous interrupt enable in `mstatus` / `mstatus` 前一中断使能位
pub(crate) const MSTATUS_MPIE: u32 = 1 << 7;

/// Previous privilege mask in `mstatus` (bits 12:11) / `mstatus` 特权级掩码
pub(crate) const MSTATUS_MPP: u32 = 0b11 << 11;

/// Machine external interrupt enable in `mie` / `mie` 机器外部中断使能位
pub(crate) const MIE_MEIE: u32 = 1 << 11;

/// Machine external interrupt pending in `mip` / `mip` 机器外部中断挂起位
pub(crate) const MIP_MEIP: u32 = 1 << 11;

/// Machine identifier / 机器标识
const MISA_RV32IMAC: u32 = 0x4000_1105;

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
    /// `mcounteren` counter enable / `mcounteren` 计数器使能
    pub mcounteren: u32,
    /// `mcountinhibit` counter inhibit / `mcountinhibit` 计数器抑制
    pub mcountinhibit: u32,
    /// `pmpcfg0` PMP configuration / `pmpcfg0` PMP 配置
    pub pmpcfg0: u32,
    /// `pmpaddr0..15` PMP address registers / `pmpaddr0..15` PMP 地址寄存器
    pub pmpaddr: [u32; 16],
    /// `meiea` Xh3irq enable / `meiea` Xh3irq 使能
    pub meiea: u32,
    /// `meipa` Xh3irq pending / `meipa` Xh3irq 挂起
    pub meipa: u32,
    /// `meifa` Xh3irq force / `meifa` Xh3irq 强制
    pub meifa: u32,
    /// `meipra` Xh3irq priority / `meipra` Xh3irq 优先级
    pub meipra: u32,
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
            mcounteren: 0,
            mcountinhibit: 0,
            pmpcfg0: 0,
            pmpaddr: [0; 16],
            meiea: 0,
            meipa: 0,
            meifa: 0,
            meipra: 0,
            minstret: 0,
            mcycle: 0,
        }
    }

    /// Advance the cycle and instruction counters by one / 周期与指令计数器加一
    pub(crate) fn tick(&mut self) {
        self.mcycle = self.mcycle.wrapping_add(1);
        self.minstret = self.minstret.wrapping_add(1);
    }

    /// Read a CSR by address / 按地址读取 CSR
    ///
    /// Unmodelled CSRs read as zero, which keeps vendor firmware polling loops
    /// running without pretending to implement unimplemented features.
    /// 未建模的 CSR 读为 0，使厂商固件的轮询循环可继续运行，同时不假装实现了
    /// 尚未实现的功能。
    pub(crate) fn read(&self, addr: u32) -> u32 {
        match addr {
            0x300 => self.mstatus,
            0x301 => MISA_RV32IMAC,
            0x304 => self.mie,
            0x305 => self.mtvec,
            0x306 => self.mcounteren,
            0x310 => 0, // mstatush
            0x320 => self.mcountinhibit,
            0x340 => self.mscratch,
            0x341 => self.mepc,
            0x342 => self.mcause,
            0x343 => self.mtval,
            0x344 => self.mip,
            0x3A0 => self.pmpcfg0,
            addr if (0x3B0..=0x3EF).contains(&addr) => self.pmpaddr[((addr - 0x3B0) >> 2) as usize],
            0xB00 | 0xC00 => self.mcycle as u32,
            0xB02 | 0xC02 => self.minstret as u32,
            0xB80 | 0xC80 => (self.mcycle >> 32) as u32,
            0xB82 | 0xC82 => (self.minstret >> 32) as u32,
            0xBE0 => self.meiea,
            0xBE1 => self.meipa,
            0xBE2 => self.meifa,
            0xBE3 => self.meipra,
            _ => 0,
        }
    }

    /// Write a CSR by address / 按地址写入 CSR
    ///
    /// Read-only and unmodelled CSRs ignore writes.
    /// 只读与未建模的 CSR 忽略写入。
    pub(crate) fn write(&mut self, addr: u32, value: u32) {
        match addr {
            0x300 => {
                self.mstatus = (self.mstatus & !(MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP))
                    | (value & (MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP));
            }
            0x304 => self.mie = value,
            0x305 => self.mtvec = value,
            0x306 => self.mcounteren = value,
            0x320 => self.mcountinhibit = value & 0x7,
            0x340 => self.mscratch = value,
            0x341 => self.mepc = value & !3,
            0x342 => self.mcause = value,
            0x343 => self.mtval = value,
            0x344 => self.mip = value & MIP_MEIP,
            0x3A0 => self.pmpcfg0 = value,
            addr if (0x3B0..=0x3EF).contains(&addr) => {
                self.pmpaddr[((addr - 0x3B0) >> 2) as usize] = value;
            }
            0xB00 => self.mcycle = (self.mcycle & 0xFFFF_FFFF_0000_0000) | u64::from(value),
            0xB02 => self.minstret = (self.minstret & 0xFFFF_FFFF_0000_0000) | u64::from(value),
            0xB80 => self.mcycle = (self.mcycle & 0xFFFF_FFFF) | (u64::from(value) << 32),
            0xB82 => self.minstret = (self.minstret & 0xFFFF_FFFF) | (u64::from(value) << 32),
            0xBE0 => self.meiea = value,
            0xBE1 => self.meipa = value,
            0xBE2 => self.meifa = value,
            0xBE3 => self.meipra = value,
            _ => {}
        }
    }

    /// Enter a machine-mode trap and return the trap handler PC
    /// 进入机器模式陷阱并返回陷阱处理程序 PC
    pub(crate) fn take_trap(&mut self, cause: u32, tval: u32, epc: u32) -> u32 {
        self.mepc = epc & !3;
        self.mcause = cause;
        self.mtval = tval;

        let previous_enable = self.mstatus & MSTATUS_MIE;
        self.mstatus &= !(MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP);
        if previous_enable != 0 {
            self.mstatus |= MSTATUS_MPIE;
        }
        self.mstatus |= MSTATUS_MPP;

        self.mip &= !MIP_MEIP;
        self.trap_pc(cause)
    }

    /// Resolve the PC for a trap / 计算陷阱目标 PC
    pub(crate) fn trap_pc(&self, cause: u32) -> u32 {
        let base = self.mtvec & !3;
        if self.mtvec & 3 == 1 {
            base.wrapping_add(4 * (cause & 0x7FFF_FFFF))
        } else {
            base
        }
    }

    /// Execute `mret` and return the restored PC / 执行 `mret` 并返回恢复的 PC
    pub(crate) fn mret(&mut self) -> u32 {
        let mpie = self.mstatus & MSTATUS_MPIE;
        self.mstatus &= !(MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP);
        if mpie != 0 {
            self.mstatus |= MSTATUS_MIE | MSTATUS_MPIE;
        }
        self.mepc & !3
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
        assert_eq!(csr.mepc, 0x2000_0040);
        assert_eq!(csr.mcause, 11);
        assert_eq!(csr.mstatus & MSTATUS_MIE, 0);
        assert_ne!(csr.mstatus & MSTATUS_MPIE, 0);
        assert_eq!(csr.mstatus & MSTATUS_MPP, MSTATUS_MPP);
        assert_eq!(csr.trap_pc(11), 0x2000_0100);
    }

    #[test]
    fn vectored_trap_offsets_by_cause() {
        let mut csr = CsrFile::new();
        csr.mtvec = 0x2000_0101;
        assert_eq!(csr.trap_pc(11), 0x2000_0100 + 44);
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
}
