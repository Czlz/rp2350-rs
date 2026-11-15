//! System control space (SCB / NVIC / SysTick) register routing
//! 系统控制空间（SCB / NVIC / SysTick）寄存器路由
//!
//! The PPB window reaches the core through the bus: `system_write` / `system_read`
//! hand an absolute address to [`CortexM33Core::route_system_access`], which
//! demultiplexes the SCB registers, the FPU system registers, the NVIC alias
//! windows and SysTick. Keeping this dispatch in its own module leaves the
//! instruction path in `lib.rs` readable and keeps both files under the
//! project's per-file code budget.
//! PPB 窗口经总线抵达核心：`system_write` / `system_read` 把绝对地址交给
//! [`CortexM33Core::route_system_access`]，由它解复用 SCB 寄存器、FPU 系统寄存器、
//! NVIC 别名窗口与 SysTick。把这套分发放在独立模块里，可以让 `lib.rs` 的指令路径
//! 保持可读，并使两个文件都不超出项目的单文件代码预算。

// 系统控制空间路由使用 PPB 偏移常量与位掩码，属于寄存器仿真固有写法。
// System-control routing uses PPB offsets and bit masks, inherent to register
// emulation.
#![allow(
    clippy::cast_possible_truncation,
    clippy::doc_markdown,
    clippy::unreadable_literal
)]

use super::{CortexM33Core, NVIC_BASE, SCB_CFSR, SCB_CPACR, SCB_VTOR, SYSTICK_BASE, fpu, scr};

/// `SCB_ICSR` — pending/active exception control and status
/// `SCB_ICSR` —— 挂起/活跃异常控制与状态
const SCB_ICSR: u32 = 0xE000_ED04;

/// `SCB_AIRCR` — VECTKEY / PRIS / SYSRESETREQ / VECTRESET
/// `SCB_AIRCR` —— VECTKEY / PRIS / SYSRESETREQ / VECTRESET
const SCB_AIRCR: u32 = 0xE000_ED0C;

/// `SCB_SHCSR` — fault enables plus active/pending views
/// `SCB_SHCSR` —— 故障使能与活跃/挂起视图
const SCB_SHCSR: u32 = 0xE000_ED24;

/// `SCB_HFSR` — HardFault status / `SCB_HFSR` —— HardFault 状态
const SCB_HFSR: u32 = 0xE000_ED2C;

/// `SCB_SHPR2` — SVCall priority in byte 3 / `SCB_SHPR2` —— 第 3 字节为 SVCall 优先级
const SCB_SHPR2: u32 = 0xE000_ED1C;

/// `SCB_SHPR3` — PendSV priority in byte 2, SysTick priority in byte 3
/// `SCB_SHPR3` —— 第 2 字节 PendSV 优先级、第 3 字节 SysTick 优先级
const SCB_SHPR3: u32 = 0xE000_ED20;

/// Software Trigger Interrupt Register (`SCB_STIR`) / 软件触发中断寄存器
const SCB_STIR: u32 = 0xE000_EF00;

impl CortexM33Core {
    /// Handle SysTick register accesses via bus
    /// 通过总线处理 SysTick 寄存器访问
    pub(crate) fn handle_systick_access(
        &mut self,
        offset: u32,
        write: bool,
        value: u32,
        _cycles: &mut u64,
    ) -> Option<u32> {
        match offset {
            0x00 => {
                // SYST_CSR
                if write {
                    // `write_csr` keeps `ENABLE`/`TICKINT`/`CLKSOURCE`, clears
                    // `COUNTFLAG` when software writes bit 16, and drops a pending
                    // latch when TICKINT is disabled.
                    // `write_csr` 保留 `ENABLE`/`TICKINT`/`CLKSOURCE`，软件写 bit16
                    // 时清除 `COUNTFLAG`，并在关闭 TICKINT 时丢弃挂起锁存。
                    self.systick.write_csr(value);
                }
                Some(self.systick.csr)
            }
            0x04 => {
                // SYST_RVR
                if write {
                    self.systick.write_rvr(value);
                }
                Some(self.systick.rvr)
            }
            0x08 => {
                // SYST_CVR
                if write {
                    self.systick.write_cvr(value);
                }
                Some(self.systick.read_cvr())
            }
            0x0C => {
                // SYST_CALIB (read-only)
                Some(self.systick.calib)
            }
            _ => None,
        }
    }

    /// Route a system control space access to the matching inner device
    /// 将系统控制空间访问路由到对应的内部设备
    pub(crate) fn route_system_access(
        &mut self,
        addr: u32,
        write: bool,
        value: u32,
    ) -> Option<u32> {
        if addr == SCB_VTOR {
            if write {
                // ARMv8-M：VTOR 低 7 位保留，向量表按 128 字节对齐。
                // ARMv8-M: VTOR bits[6:0] are reserved; the table is 128-byte aligned.
                self.vtor = value & !0x7F;
            }
            Some(self.vtor)
        } else if addr == SCB_CPACR {
            if write {
                // 建模 CP0-CP7（GPIOC/DCP）与 CP10-CP11（FPU）字段，
                // 其余协处理器键保持为零。
                // Model CP0-CP7 (GPIOC/DCP) and CP10-CP11 (FPU) fields;
                // the remaining coprocessor keys read as zero.
                self.registers.cpacr = value & fpu::cpacr::MODELED_MASK;
            }
            Some(self.registers.cpacr)
        } else if addr == SCB_CFSR {
            if write {
                // CFSR 为写 1 清零；写入 0 不改变任何状态位。
                // The real CFSR is write-one-to-clear; writing 0 changes nothing.
                self.cfsr &= !value;
            }
            Some(self.cfsr)
        } else if addr == SCB_HFSR {
            // HFSR 为写 1 清零：FORCED/DEBUGEVT/VECTTBL 由故障路径置位。
            // HFSR is write-one-to-clear; the fault paths set
            // FORCED/DEBUGEVT/VECTTBL.
            if write {
                self.write_hfsr(value);
            }
            Some(self.hfsr())
        } else if addr == SCB_SHCSR {
            // SHCSR 只保存三个故障使能位；ACT/PENDED 为实时只读视图。
            // SHCSR stores only the three fault enables; ACT/PENDED are live
            // read-only views.
            if write {
                self.write_shcsr(value);
            }
            Some(self.shcsr())
        } else if addr == SCB_AIRCR {
            // AIRCR 写入必须携带 VECTKEY = 0x05FA；SYSRESETREQ/VECTRESET 锁存为
            // 宿主事件，读侧 VECTKEYSTAT 恒为 0xFA05，只写位读 0。
            // An AIRCR write must carry VECTKEY = 0x05FA; SYSRESETREQ/VECTRESET
            // are latched as host events, VECTKEYSTAT reads 0xFA05 and the
            // write-only bits read as zero.
            if write {
                self.write_aircr(value);
            }
            Some(self.aircr())
        } else if addr == scr::SCR {
            // SLEEPONEXIT/SLEEPDEEP/SEVONPEND 是唯一建模的位；其余保留位读 0。
            // SLEEPONEXIT/SLEEPDEEP/SEVONPEND are the only modeled bits; every
            // reserved bit reads as zero.
            if write {
                self.scr = value & scr::MODELED_MASK;
            }
            Some(self.scr)
        } else if addr == SCB_ICSR {
            // ICSR 承载 PendSV/SysTick 挂起位与 VECTPENDING/VECTACTIVE 只读视图；
            // 写入只解释四个置位/清除位。
            // ICSR carries the PendSV/SysTick pending bits and the read-only
            // VECTPENDING/VECTACTIVE view; a write only interprets the four
            // set/clear bits.
            if write {
                self.write_icsr(value);
            }
            Some(self.read_icsr())
        } else if addr == SCB_SHPR2 {
            // SHPR2 第 3 字节为 SVCall 优先级，参与异常 11 的仲裁。
            // Byte 3 of SHPR2 is the SVCall priority and takes part in
            // exception 11 arbitration.
            if write {
                self.scb.write_shpr2(value);
            }
            Some(self.scb.read_shpr2())
        } else if addr == SCB_SHPR3 {
            // SHPR3：第 2 字节 PendSV、第 3 字节 SysTick，参与抢占仲裁。
            // SHPR3: byte 2 PendSV, byte 3 SysTick; both take part in preemption.
            if write {
                self.scb.write_shpr3(value);
            }
            Some(self.scb.read_shpr3())
        } else if addr == SCB_STIR {
            // STIR 写入挂起一个外部中断；SEVONPEND 事件由打开挂起位的跳变产生。
            // A STIR write pends an external interrupt; the SEVONPEND event comes
            // from the 0-to-1 pending transition.
            if write {
                let irq = (value & 0x1FF) as u16;
                self.set_pending_irq(irq);
            }
            Some(0)
        } else if let Some(value) = self.registers.fpu.system_access(addr, write, value) {
            // FPCCR / FPCAR / FPDSCR：LSPACT 只读，FPCAR 低 3 位保留，
            // FPDSCR 只保存 AHP/DN/FZ/RMode 字段。
            // FPCCR / FPCAR / FPDSCR: LSPACT is read-only, FPCAR keeps its low
            // three bits clear and FPDSCR stores only AHP/DN/FZ/RMode.
            Some(value)
        } else if (NVIC_BASE..NVIC_BASE + 0x400).contains(&addr) {
            self.handle_nvic_access(addr - NVIC_BASE, write, value)
        } else if (SYSTICK_BASE..SYSTICK_BASE + 0x10).contains(&addr) {
            let mut cycles = 0u64;
            self.handle_systick_access(addr - SYSTICK_BASE, write, value, &mut cycles)
        } else {
            None
        }
    }

    /// Handle NVIC register accesses via bus
    /// 通过总线处理 NVIC 寄存器访问
    fn handle_nvic_access(&mut self, offset: u32, write: bool, value: u32) -> Option<u32> {
        match offset {
            // ISERn: set-enable, write-1-to-set
            o @ 0x00..=0x1C => {
                let reg_idx = ((o - 0x00) >> 2) as usize;
                if reg_idx < 8 {
                    if write {
                        self.nvic.write_iser(reg_idx, value);
                    }
                    Some(self.nvic.read_iser(reg_idx))
                } else {
                    None
                }
            }
            // ICERn: clear-enable, write-1-to-clear
            o @ 0x80..=0x9C => {
                let reg_idx = ((o - 0x80) >> 2) as usize;
                if reg_idx < 8 {
                    if write {
                        self.nvic.write_icer(reg_idx, value);
                    }
                    Some(self.nvic.read_iser(reg_idx))
                } else {
                    None
                }
            }
            // ISPRn: set-pending, write-1-to-set
            o @ 0x100..=0x11C => {
                let reg_idx = ((o - 0x100) >> 2) as usize;
                if reg_idx < 8 {
                    if write {
                        // 只有 0→1 的位产生 SEVONPEND 事件；写已挂起位不产生。
                        // Only 0-to-1 bits raise the SEVONPEND event; writing an
                        // already-pending bit does not.
                        let newly = value & !self.nvic.read_ispr(reg_idx);
                        self.nvic.write_ispr(reg_idx, value);
                        self.note_pending_bits(newly);
                    }
                    Some(self.nvic.read_ispr(reg_idx))
                } else {
                    None
                }
            }
            // ICPRn: clear-pending, write-1-to-clear
            o @ 0x180..=0x19C => {
                let reg_idx = ((o - 0x180) >> 2) as usize;
                if reg_idx < 8 {
                    if write {
                        self.nvic.write_icpr(reg_idx, value);
                    }
                    Some(self.nvic.read_ispr(reg_idx))
                } else {
                    None
                }
            }
            // IPRn: priority register, one byte per IRQ
            o @ 0x300..=0x3FC => {
                // 每个 IRQ 占一个 IPR 字节：字节地址 = 0x300 + IRQ，字索引 = IRQ / 4。
                // 旧实现把字索引又当成 IRQ 号，导致 IRQ4 以上的字节写落到 IRQ0..3。
                // One IPR byte per IRQ: byte address = 0x300 + IRQ, word index =
                // IRQ / 4. The previous mapping treated the word index as an IRQ
                // number, so byte writes for IRQ >= 4 landed on IRQ0..3.
                let irq = (o - 0x300) as usize;
                let word_index = irq / 4;
                let value_shift = ((irq % 4) * 8) as u8;
                if write {
                    let mask = !(0xFFu32 << value_shift);
                    let existing = self.nvic.read_ipr_word(word_index);
                    let new_val = (existing & mask) | ((value & 0xFF) << value_shift);
                    self.nvic.write_ipr_word(word_index, new_val);
                }
                let word = self.nvic.read_ipr_word(word_index);
                Some((word >> value_shift) & 0xFF)
            }
            _ => None,
        }
    }
}
