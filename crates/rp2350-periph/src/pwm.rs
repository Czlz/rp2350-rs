//! PWM register block stub / PWM 寄存器块桩
//!
//! RP2350 has a single 12-channel PWM register block at `PWM0_BASE`. Each
//! channel has the datasheet reset values (`DIV = 0x10`, `TOP = 0xFFFF`); the
//! CSR phase-advance/retard strobes read back as zero, and the global `EN`
//! register mirrors the per-channel `EN` bits. Counter and waveform generation
//! are not modelled.
//! RP2350 在 `PWM0_BASE` 只有一个 12 通道 PWM 寄存器块。每个通道使用数据手册
//! 复位值（`DIV = 0x10`、`TOP = 0xFFFF`）；CSR 的相位提前/延迟脉冲位读回为零，
//! 全局 `EN` 寄存器镜像各通道的 `EN` 位。计数器与波形生成未建模。
//!
//! The address `PWM1_BASE` observed in firmware logs is the XOR alias window of
//! this same block (`PWM0_BASE + 0x1000`), not a second register block; the APB
//! router folds that window onto `Pwm`.
//! 固件日志中出现的 `PWM1_BASE` 是同一个块的 XOR 别名窗口
//! （`PWM0_BASE + 0x1000`），并非第二个寄存器块；APB 路由器会把该窗口折叠到
//! `Pwm` 上。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// Number of PWM channels in the block / 块内 PWM 通道数
const CHANNELS: u32 = 12;

/// Byte stride between channel register groups / 通道寄存器组之间的字节步长
const CH_STRIDE: u32 = 0x14;

/// Offset of `CHx_CSR` inside a channel group / 通道组内 `CHx_CSR` 的偏移
const CH_CSR: u32 = 0x00;

/// Offset of `CHx_DIV` / `CHx_DIV` 偏移
const CH_DIV: u32 = 0x04;

/// Offset of `CHx_CTR` / `CHx_CTR` 偏移
const CH_CTR: u32 = 0x08;

/// Offset of `CHx_CC` / `CHx_CC` 偏移
const CH_CC: u32 = 0x0C;

/// Offset of `CHx_TOP` / `CHx_TOP` 偏移
const CH_TOP: u32 = 0x10;

/// `CHx_DIV` reset value / `CHx_DIV` 复位值
const DIV_RESET: u32 = 0x10;

/// `CHx_TOP` reset value / `CHx_TOP` 复位值
const TOP_RESET: u32 = 0xFFFF;

/// Writable CSR bits: DIVMODE, B_INV, A_INV, PH_CORRECT and EN
/// CSR 可写位：DIVMODE、B_INV、A_INV、PH_CORRECT 与 EN
const CSR_WRITE_MASK: u32 = 0x3F;

/// Bits readable from CSR; PH_ADV/PH_RET are write-only strobes
/// CSR 可读位；PH_ADV/PH_RET 是只写脉冲位
const CSR_READ_MASK: u32 = 0x3F;

/// `CHx_DIV` field mask / `CHx_DIV` 字段掩码
const DIV_MASK: u32 = 0x0FFF;

/// 16-bit counter/top field mask / 16 位计数/顶部字段掩码
const COUNT_MASK: u32 = 0xFFFF;

/// Offset of the global `EN` register / 全局 `EN` 寄存器偏移
const EN: u32 = 0xF0;

/// Offset of `INTR` / `INTR` 偏移
const INTR: u32 = 0xF4;

/// Offset of `IRQ0_INTE` / `IRQ0_INTE` 偏移
const IRQ0_INTE: u32 = 0xF8;

/// Offset of `IRQ0_INTF` / `IRQ0_INTF` 偏移
const IRQ0_INTF: u32 = 0xFC;

/// Offset of `IRQ0_INTS` / `IRQ0_INTS` 偏移
const IRQ0_INTS: u32 = 0x100;

/// Offset of `IRQ1_INTE` / `IRQ1_INTE` 偏移
const IRQ1_INTE: u32 = 0x104;

/// Offset of `IRQ1_INTF` / `IRQ1_INTF` 偏移
const IRQ1_INTF: u32 = 0x108;

/// Offset of `IRQ1_INTS` / `IRQ1_INTS` 偏移
const IRQ1_INTS: u32 = 0x10C;

/// Interrupt bit mask covering all 12 channels / 覆盖 12 个通道的中断位掩码
const IRQ_MASK: u32 = (1 << CHANNELS) - 1;

/// PWM register block / PWM 寄存器块
#[derive(Debug)]
pub struct Pwm {
    regs: RegisterFile,
}

impl Pwm {
    /// Create the block with the datasheet reset values
    /// 创建使用数据手册复位值的寄存器块
    #[must_use]
    pub fn new() -> Self {
        let mut regs = RegisterFile::new(map::DEVICE_WINDOW_SIZE);
        for channel in 0..CHANNELS {
            let base = channel * CH_STRIDE;
            regs.fill(base + CH_DIV, DIV_RESET);
            regs.fill(base + CH_TOP, TOP_RESET);
        }
        Self { regs }
    }

    /// Channel index and register offset for a channel-group address
    /// 将通道组地址拆分为通道序号与寄存器偏移
    fn channel_register(offset: u32) -> Option<(u32, u32)> {
        if offset >= EN {
            return None;
        }
        let channel = offset / CH_STRIDE;
        let register = offset % CH_STRIDE;
        if channel >= CHANNELS {
            return None;
        }
        matches!(register, CH_CSR | CH_DIV | CH_CTR | CH_CC | CH_TOP).then_some((channel, register))
    }

    /// Read the per-channel `EN` bits into the global `EN` register
    /// 把各通道 `EN` 位汇总到全局 `EN` 寄存器
    fn enabled_mask(&self) -> u32 {
        let mut mask = 0;
        for channel in 0..CHANNELS {
            let csr = self.regs.read(channel * CH_STRIDE).unwrap_or(0);
            if csr & 1 != 0 {
                mask |= 1 << channel;
            }
        }
        mask
    }

    /// Apply a write to the global `EN` register to every channel CSR
    /// 把全局 `EN` 写入分发到每个通道的 CSR
    fn set_enabled_mask(&mut self, value: u32) {
        for channel in 0..CHANNELS {
            let offset = channel * CH_STRIDE;
            let csr = self.regs.read(offset).unwrap_or(0) & !1;
            let enable = (value >> channel) & 1;
            self.regs.write(offset, csr | enable);
        }
    }
}

impl Default for Pwm {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for Pwm {
    fn name(&self) -> &str {
        "PWM"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::PWM0_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        if let Some((channel, register)) = Self::channel_register(offset) {
            let base = channel * CH_STRIDE;
            return match register {
                CH_CSR => self.regs.read(base).map(|value| value & CSR_READ_MASK),
                CH_DIV => self.regs.read(base + CH_DIV).map(|value| value & DIV_MASK),
                CH_CTR | CH_TOP => self
                    .regs
                    .read(base + register)
                    .map(|value| value & COUNT_MASK),
                _ => self.regs.read(base + register),
            };
        }
        match offset {
            EN => Some(self.enabled_mask()),
            INTR => self.regs.read(INTR).map(|value| value & IRQ_MASK),
            IRQ0_INTE | IRQ1_INTE | IRQ0_INTF | IRQ1_INTF => {
                self.regs.read(offset).map(|value| value & IRQ_MASK)
            }
            IRQ0_INTS => Some(
                (self.regs.read(INTR).unwrap_or(0) & self.regs.read(IRQ0_INTE).unwrap_or(0)
                    | self.regs.read(IRQ0_INTF).unwrap_or(0))
                    & IRQ_MASK,
            ),
            IRQ1_INTS => Some(
                (self.regs.read(INTR).unwrap_or(0) & self.regs.read(IRQ1_INTE).unwrap_or(0)
                    | self.regs.read(IRQ1_INTF).unwrap_or(0))
                    & IRQ_MASK,
            ),
            _ => self.regs.read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        if let Some((channel, register)) = Self::channel_register(offset) {
            let base = channel * CH_STRIDE;
            let value = match register {
                CH_CSR => value & CSR_WRITE_MASK,
                CH_DIV => value & DIV_MASK,
                CH_CTR | CH_TOP => value & COUNT_MASK,
                _ => value,
            };
            return self.regs.write(base + register, value);
        }
        match offset {
            EN => {
                self.set_enabled_mask(value & IRQ_MASK);
                true
            }
            // 中断原始状态为写 1 清零。Raw interrupt status is W1C.
            INTR => {
                let current = self.regs.read(INTR).unwrap_or(0);
                self.regs.write(INTR, current & !(value & IRQ_MASK))
            }
            IRQ0_INTE | IRQ1_INTE | IRQ0_INTF | IRQ1_INTF => {
                self.regs.write(offset, value & IRQ_MASK)
            }
            // 只读状态寄存器忽略写入。Read-only status registers ignore writes.
            IRQ0_INTS | IRQ1_INTS => true,
            _ => self.regs.write(offset, value),
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_reset_to_the_datasheet_values() {
        let mut pwm = Pwm::new();
        for channel in 0..CHANNELS {
            let base = channel * CH_STRIDE;
            assert_eq!(pwm.read32(base + CH_CSR), Some(0));
            assert_eq!(pwm.read32(base + CH_DIV), Some(DIV_RESET));
            assert_eq!(pwm.read32(base + CH_TOP), Some(TOP_RESET));
        }
        assert_eq!(pwm.read32(EN), Some(0));
    }

    #[test]
    fn csr_is_writable_and_strobes_read_back_as_zero() {
        let mut pwm = Pwm::new();
        assert!(pwm.write32(CH_CSR, 0xFFFF));
        assert_eq!(pwm.read32(CH_CSR), Some(CSR_READ_MASK));
        assert!(pwm.write32(CH_CSR + CH_STRIDE * 2, 0x0000_0035));
        assert_eq!(pwm.read32(CH_CSR + CH_STRIDE * 2), Some(0x35));
    }

    #[test]
    fn global_enable_mirrors_the_channel_enable_bits() {
        let mut pwm = Pwm::new();
        assert!(pwm.write32(CH_STRIDE * 3 + CH_CSR, 1));
        assert_eq!(pwm.read32(EN), Some(1 << 3));
        assert!(pwm.write32(EN, 0x0000_0005));
        assert_eq!(pwm.read32(CH_CSR), Some(1));
        assert_eq!(pwm.read32(CH_STRIDE * 3 + CH_CSR), Some(0));
        assert_eq!(pwm.read32(EN), Some(0x5));
    }

    #[test]
    fn interrupt_status_masks_raw_and_forced_bits() {
        let mut pwm = Pwm::new();
        // PWM 波形未建模，测试直接注入原始中断位。
        // Waveforms are not modelled, so the test injects the raw interrupt bits.
        pwm.regs.write(INTR, IRQ_MASK);
        assert_eq!(pwm.read32(INTR), Some(IRQ_MASK));
        assert!(pwm.write32(IRQ0_INTE, 0x00F));
        assert!(pwm.write32(IRQ0_INTF, 0x010));
        assert_eq!(pwm.read32(IRQ0_INTS), Some(0x01F));
        assert!(pwm.write32(INTR, 0x001));
        assert_eq!(pwm.read32(IRQ0_INTS), Some(0x01E));
    }
}
