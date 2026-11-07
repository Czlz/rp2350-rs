//! Cortex-M33 register file (R0-R15, MSP/PSP, XPSR, CONTROL, PRIMASK, BASEPRI, FAULTMASK)
//! Cortex-M33 寄存器文件（R0-R15、MSP/PSP、XPSR、CONTROL、PRIMASK、BASEPRI、FAULTMASK）

/// ARMv8-M program status register (XPSR) flags
/// ARMv8-M 程序状态寄存器标志
pub mod psr {
    use bitflags::bitflags;

    bitflags! {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct ApsrFlags: u32 {
            /// Negative flag / 负数标志
            const N = 1 << 31;
            /// Zero flag / 零标志
            const Z = 1 << 30;
            /// Carry flag / 进位标志
            const C = 1 << 29;
            /// Overflow flag / 溢出标志
            const V = 1 << 28;
            /// Saturating cumulative flag (Q) / 饱和累积标志
            const Q = 1 << 27;
        }
    }
}

/// Complete Cortex-M33 register set
/// 完整的 Cortex-M33 寄存器集
#[derive(Debug, Clone)]
pub struct CortexM33Registers {
    /// General-purpose registers R0-R12 / 通用寄存器 R0-R12
    pub r: [u32; 13],
    /// Stack Pointer (current: MSP or PSP) / 栈指针（当前：MSP 或 PSP）
    pub sp: u32,
    /// Link Register (R14) / 链接寄存器
    pub lr: u32,
    /// Program Counter (R15) / 程序计数器
    pub pc: u32,
    /// Main Stack Pointer / 主栈指针
    pub msp: u32,
    /// Process Stack Pointer / 进程栈指针
    pub psp: u32,
    /// Program Status Register (APSR + IPSR + EPSR) / 程序状态寄存器
    pub xpsr: u32,
    /// CONTROL register / 控制寄存器
    pub control: u32,
    /// Priority Mask Register / 优先级屏蔽寄存器
    pub primask: u32,
    /// Base Priority Mask Register / 基础优先级屏蔽寄存器
    pub basepri: u32,
    /// Fault Mask Register / 故障屏蔽寄存器
    pub faultmask: u32,
}

impl CortexM33Registers {
    /// Create a new register set in power-on reset state
    /// 创建上电复位状态的寄存器集
    #[must_use]
    pub fn new() -> Self {
        Self {
            r: [0; 13],
            sp: 0,
            lr: 0xFFFF_FFFF,
            pc: 0,
            msp: 0,
            psp: 0,
            xpsr: 0x0100_0000, // Thumb state bit set
            control: 0,
            primask: 0,
            basepri: 0,
            faultmask: 0,
        }
    }

    /// Read a general-purpose register by index (0-15)
    /// 按索引读取通用寄存器（0-15）
    #[must_use]
    pub fn read(&self, index: u8) -> u32 {
        match index {
            0..=12 => self.r[index as usize],
            13 => self.sp,
            14 => self.lr,
            15 => self.pc,
            _ => 0,
        }
    }

    /// Write a general-purpose register by index (0-15)
    /// 按索引写入通用寄存器（0-15）
    pub fn write(&mut self, index: u8, value: u32) {
        match index {
            0..=12 => self.r[index as usize] = value,
            13 => self.sp = value,
            14 => self.lr = value,
            15 => self.pc = value,
            _ => {}
        }
    }

    /// Get the current stack pointer (MSP or PSP based on CONTROL.SPSEL)
    /// 获取当前栈指针（根据 CONTROL.SPSEL 选择 MSP 或 PSP）
    #[must_use]
    pub fn current_sp(&self) -> u32 {
        if self.control & 0x2 != 0 {
            self.psp
        } else {
            self.msp
        }
    }
}

impl Default for CortexM33Registers {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_write_gpr() {
        let mut regs = CortexM33Registers::new();
        regs.write(0, 0xDEAD_BEEF);
        assert_eq!(regs.read(0), 0xDEAD_BEEF);
    }

    #[test]
    fn sp_is_r13() {
        let mut regs = CortexM33Registers::new();
        regs.write(13, 0x2000_1000);
        assert_eq!(regs.sp, 0x2000_1000);
    }

    #[test]
    fn default_stack_is_msp() {
        let mut regs = CortexM33Registers::new();
        regs.msp = 0x2000_4000;
        regs.psp = 0x2000_8000;
        assert_eq!(regs.current_sp(), 0x2000_4000);
    }
}