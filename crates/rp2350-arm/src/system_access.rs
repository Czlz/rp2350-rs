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

use super::{CortexM33Core, NVIC_BASE, SCB_CFSR, SCB_CPACR, SCB_VTOR, SYSTICK_BASE, fpu};

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
                    self.systick.csr = value & 0x7;
                    if value & (1 << 16) != 0 {
                        self.systick.csr &= !(1 << 16); // Clear COUNTFLAG
                    }
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
                // 简化为直接读写；真实 CFSR 为写 1 清零。
                // Modeled as a plain read/write; the real CFSR is write-one-to-clear.
                self.cfsr = value;
            }
            Some(self.cfsr)
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
                        self.nvic.write_ispr(reg_idx, value);
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
            // IPRn: priority register, 4 bytes per IRQ, byte-accessible
            o @ 0x300..=0x3FC => {
                let irq_idx = ((o - 0x300) >> 2) as u16;
                if irq_idx < 256 {
                    let byte_idx = ((o - 0x300) & 3) as u16;
                    // Each IPR word covers 4 IRQs at byte granularity
                    // 每个 IPR 字以字节粒度覆盖 4 个 IRQ
                    let group_start = (irq_idx & !3u16) as usize;
                    let value_shift = (byte_idx * 8) as u8;
                    if write {
                        let mask = !(0xFFu32 << value_shift);
                        let existing = self.nvic.read_ipr_word(group_start / 4);
                        let new_val = (existing & mask) | ((value & 0xFF) << value_shift);
                        self.nvic.write_ipr_word(group_start / 4, new_val);
                    }
                    let word = self.nvic.read_ipr_word(group_start / 4);
                    Some((word >> value_shift) & 0xFF)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}
