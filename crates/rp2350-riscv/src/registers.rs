//! RV32 integer register file / RV32 整数寄存器组

/// RV32 integer register file with a hard-wired zero register
/// RV32 整数寄存器组，零号寄存器硬连线为 0
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rv32Registers {
    /// x0..x31 / 通用寄存器 x0..x31
    pub x: [u32; 32],
    /// Program counter / 程序计数器
    pub pc: u32,
}

impl Rv32Registers {
    /// Create a zeroed register file / 创建清零的寄存器组
    #[must_use]
    pub fn new() -> Self {
        Self { x: [0; 32], pc: 0 }
    }

    /// Read a register; x0 always reads as zero / 读取寄存器；x0 恒为 0
    #[must_use]
    pub fn read(&self, index: u8) -> u32 {
        if index == 0 {
            0
        } else {
            self.x[usize::from(index) & 0x1F]
        }
    }

    /// Write a register; writes to x0 are discarded / 写入寄存器；对 x0 的写入被丢弃
    pub fn write(&mut self, index: u8, value: u32) {
        if index != 0 {
            self.x[usize::from(index) & 0x1F] = value;
        }
    }
}

impl Default for Rv32Registers {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_register_is_hard_wired() {
        let mut registers = Rv32Registers::new();
        registers.write(0, 0xDEAD_BEEF);
        assert_eq!(registers.read(0), 0);
        registers.write(5, 0x1234_5678);
        assert_eq!(registers.read(5), 0x1234_5678);
    }

    #[test]
    fn out_of_range_index_wraps() {
        let mut registers = Rv32Registers::new();
        registers.write(33, 7);
        assert_eq!(registers.read(1), 7);
    }
}
