//! ADC peripheral stub / ADC 外设桩
//!
//! Models the RP2350 ADC register block at `ADC_BASE`. `CS.READY` always reads
//! one, so firmware polling for conversion completion continues immediately.
//! A `START_ONCE` write with `EN` set completes in the same access and pushes
//! one zero sample into the four-entry result FIFO when `FCS.EN` is set; the
//! FIFO level/empty/full flags and the FIFO interrupt bits follow the datasheet
//! layout. Channel selection and clock division are stored but not interpreted.
//! 建模 `ADC_BASE` 处的 RP2350 ADC 寄存器块。`CS.READY` 始终读一，等待转换
//! 完成的固件会立即继续。`EN` 置位时写 `START_ONCE` 会在同一次访问内完成转换，
//! 若 `FCS.EN` 置位则向四深度结果 FIFO 压入一个零样本；FIFO 的层级/空/满标志
//! 与 FIFO 中断位遵循数据手册布局。通道选择与时钟分频仅被保存，不解释。

use std::collections::VecDeque;

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// `ADC_CS` offset / `ADC_CS` 偏移
const CS: u32 = 0x00;

/// `ADC_RESULT` offset / `ADC_RESULT` 偏移
const RESULT: u32 = 0x04;

/// `ADC_FCS` offset / `ADC_FCS` 偏移
const FCS: u32 = 0x08;

/// `ADC_FIFO` offset / `ADC_FIFO` 偏移
const FIFO: u32 = 0x0C;

/// `ADC_INTR` offset / `ADC_INTR` 偏移
const INTR: u32 = 0x14;

/// `ADC_INTE` offset / `ADC_INTE` 偏移
const INTE: u32 = 0x18;

/// `ADC_INTF` offset / `ADC_INTF` 偏移
const INTF: u32 = 0x1C;

/// `ADC_INTS` offset / `ADC_INTS` 偏移
const INTS: u32 = 0x20;

/// `CS.ERR_STICKY` / `CS.ERR_STICKY`
const CS_ERR_STICKY: u32 = 1 << 10;

/// `CS.ERR` / `CS.ERR`
const CS_ERR: u32 = 1 << 9;

/// `CS.READY` / `CS.READY`
const CS_READY: u32 = 1 << 8;

/// `CS.START_MANY` / `CS.START_MANY`
const CS_START_MANY: u32 = 1 << 3;

/// `CS.START_ONCE` / `CS.START_ONCE`
const CS_START_ONCE: u32 = 1 << 2;

/// `CS.EN` / `CS.EN`
const CS_EN: u32 = 1 << 0;

/// `CS` writable field mask (RROBIN, AINSEL, START_MANY, TS_EN, EN)
/// `CS` 可写字段掩码（RROBIN、AINSEL、START_MANY、TS_EN、EN）
const CS_WRITE_MASK: u32 = 0x01FF_F00B;

/// `FCS.THRESH` shift / `FCS.THRESH` 移位
const FCS_THRESH_SHIFT: u32 = 24;

/// `FCS.THRESH` mask / `FCS.THRESH` 掩码
const FCS_THRESH_MASK: u32 = 0xF << FCS_THRESH_SHIFT;

/// `FCS.LEVEL` shift / `FCS.LEVEL` 移位
const FCS_LEVEL_SHIFT: u32 = 16;

/// `FCS.LEVEL` mask / `FCS.LEVEL` 掩码
const FCS_LEVEL_MASK: u32 = 0xF << FCS_LEVEL_SHIFT;

/// `FCS.OVER` / `FCS.OVER`
const FCS_OVER: u32 = 1 << 11;

/// `FCS.UNDER` / `FCS.UNDER`
const FCS_UNDER: u32 = 1 << 10;

/// `FCS.FULL` / `FCS.FULL`
const FCS_FULL: u32 = 1 << 9;

/// `FCS.EMPTY` / `FCS.EMPTY`
const FCS_EMPTY: u32 = 1 << 8;

/// `FCS.DREQ_EN` / `FCS.DREQ_EN`
const FCS_DREQ_EN: u32 = 1 << 3;

/// `FCS.ERR` / `FCS.ERR`
const FCS_ERR: u32 = 1 << 2;

/// `FCS.SHIFT` / `FCS.SHIFT`
const FCS_SHIFT: u32 = 1 << 1;

/// `FCS.EN` / `FCS.EN`
const FCS_EN: u32 = 1 << 0;

/// `FCS` writable storage mask (THRESH, DREQ_EN, ERR, SHIFT, EN)
/// `FCS` 可写存储掩码（THRESH、DREQ_EN、ERR、SHIFT、EN）
const FCS_WRITE_MASK: u32 = FCS_THRESH_MASK | FCS_DREQ_EN | FCS_ERR | FCS_SHIFT | FCS_EN;

/// `FCS` W1C mask (OVER, UNDER) / `FCS` 写 1 清零掩码（OVER、UNDER）
const FCS_W1C_MASK: u32 = FCS_OVER | FCS_UNDER;

/// `INTR.FIFO` / `INTR.FIFO`
const INTR_FIFO: u32 = 1;

/// Result FIFO depth / 结果 FIFO 深度
const FIFO_DEPTH: usize = 4;

/// ADC register block / ADC 寄存器块
#[derive(Debug)]
pub struct Adc {
    regs: RegisterFile,
    fifo: VecDeque<u32>,
    result: u32,
}

impl Adc {
    /// Create the block with the datasheet reset values
    /// 创建使用数据手册复位值的寄存器块
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: RegisterFile::new(map::DEVICE_WINDOW_SIZE),
            fifo: VecDeque::with_capacity(FIFO_DEPTH),
            result: 0,
        }
    }

    /// FIFO level as a register field value / 作为寄存器字段值的 FIFO 层级
    fn fifo_level(&self) -> u32 {
        u32::try_from(self.fifo.len()).unwrap_or(0)
    }

    /// Raw FIFO interrupt before masking / 屏蔽前的 FIFO 原始中断
    fn raw_interrupt(&self) -> u32 {
        let fcs = self.regs.read(FCS).unwrap_or(0);
        let threshold = (fcs & FCS_THRESH_MASK) >> FCS_THRESH_SHIFT;
        // 空 FIFO 不产生中断，保证复位后 INTR 为数据手册的 0。
        // An empty FIFO raises no interrupt, keeping INTR at its datasheet reset 0.
        if !self.fifo.is_empty() && self.fifo_level() >= threshold {
            INTR_FIFO
        } else {
            0
        }
    }

    /// Complete one conversion and push a sample when the FIFO is enabled
    /// 完成一次转换，并在 FIFO 使能时压入一个样本
    fn convert_once(&mut self) {
        self.result = 0;
        if self.regs.read(FCS).unwrap_or(0) & FCS_EN == 0 {
            return;
        }
        if self.fifo.len() >= FIFO_DEPTH {
            let fcs = self.regs.read(FCS).unwrap_or(0);
            self.regs.write(FCS, fcs | FCS_OVER);
        } else {
            self.fifo.push_back(self.result);
        }
    }
}

impl Default for Adc {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for Adc {
    fn name(&self) -> &str {
        "ADC"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::ADC_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        match offset {
            CS => {
                let stored = self.regs.read(CS).unwrap_or(0);
                let error = if stored & CS_ERR_STICKY != 0 {
                    CS_ERR
                } else {
                    0
                };
                Some((stored & (CS_WRITE_MASK | CS_ERR_STICKY)) | CS_READY | error)
            }
            RESULT => Some(self.result),
            FCS => {
                let mut value = self.regs.read(FCS).unwrap_or(0) & (FCS_WRITE_MASK | FCS_W1C_MASK);
                value |= (self.fifo_level() << FCS_LEVEL_SHIFT) & FCS_LEVEL_MASK;
                if self.fifo.is_empty() {
                    value |= FCS_EMPTY;
                }
                if self.fifo.len() >= FIFO_DEPTH {
                    value |= FCS_FULL;
                }
                Some(value)
            }
            FIFO => {
                if let Some(value) = self.fifo.pop_front() {
                    Some(value)
                } else {
                    let fcs = self.regs.read(FCS).unwrap_or(0);
                    self.regs.write(FCS, fcs | FCS_UNDER);
                    Some(0)
                }
            }
            INTR => Some(self.raw_interrupt()),
            INTS => {
                let raw = self.raw_interrupt();
                let enable = self.regs.read(INTE).unwrap_or(0);
                let force = self.regs.read(INTF).unwrap_or(0);
                Some((raw & enable) | force)
            }
            _ => self.regs.read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        match offset {
            CS => {
                let stored = self.regs.read(CS).unwrap_or(0);
                let sticky = stored & CS_ERR_STICKY & !value;
                let writable = value & CS_WRITE_MASK;
                self.regs.write(CS, sticky | writable);
                if value & CS_EN != 0 && value & (CS_START_ONCE | CS_START_MANY) != 0 {
                    self.convert_once();
                }
                true
            }
            FCS => {
                let stored = self.regs.read(FCS).unwrap_or(0);
                let sticky = stored & FCS_W1C_MASK & !value;
                self.regs.write(FCS, sticky | (value & FCS_WRITE_MASK));
                true
            }
            INTR | INTS => true,
            INTE | INTF => {
                self.regs.write(offset, value & INTR_FIFO);
                true
            }
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
    fn reset_reports_a_ready_converter_and_an_empty_fifo() {
        let mut adc = Adc::new();
        assert_eq!(adc.read32(CS), Some(CS_READY));
        assert_eq!(adc.read32(FCS), Some(FCS_EMPTY));
        assert_eq!(adc.read32(0x10), Some(0));
        assert_eq!(adc.read32(INTR), Some(0));
    }

    #[test]
    fn control_and_divider_writes_are_readable_storage() {
        let mut adc = Adc::new();
        assert!(adc.write32(CS, CS_EN | (1 << 1) | (3 << 12)));
        let cs = adc.read32(CS).unwrap();
        assert_eq!(cs & CS_EN, CS_EN);
        assert_eq!(cs & CS_READY, CS_READY);
        assert_eq!(cs & (1 << 1), 1 << 1);
        assert_eq!(cs & (0xF << 12), 3 << 12);
        assert!(adc.write32(0x10, 0x0000_1234));
        assert_eq!(adc.read32(0x10), Some(0x0000_1234));
    }

    #[test]
    fn a_start_once_conversion_pushes_a_sample_when_fifo_enabled() {
        let mut adc = Adc::new();
        assert!(adc.write32(CS, CS_EN));
        assert!(adc.write32(FCS, FCS_EN));
        assert!(adc.write32(CS, CS_EN | CS_START_ONCE));
        let fcs = adc.read32(FCS).unwrap();
        assert_eq!(fcs & FCS_LEVEL_MASK, 1 << FCS_LEVEL_SHIFT);
        assert_eq!(fcs & FCS_EMPTY, 0);
        assert_eq!(adc.read32(FIFO), Some(0));
        assert_eq!(adc.read32(FCS).unwrap() & FCS_EMPTY, FCS_EMPTY);
        assert_eq!(adc.read32(CS).unwrap() & CS_READY, CS_READY);
    }

    #[test]
    fn fifo_interrupt_follows_the_threshold_and_enable_mask() {
        let mut adc = Adc::new();
        assert_eq!(adc.read32(INTR), Some(0));
        assert!(adc.write32(INTE, INTR_FIFO));
        assert!(adc.write32(CS, CS_EN));
        assert!(adc.write32(FCS, FCS_EN));
        assert!(adc.write32(CS, CS_EN | CS_START_ONCE));
        // 一份样本达到阈值 0，屏蔽后状态为 1。
        // One sample meets threshold 0, so the masked status is 1.
        assert_eq!(adc.read32(INTR), Some(INTR_FIFO));
        assert_eq!(adc.read32(INTS), Some(INTR_FIFO));
        assert!(adc.write32(INTE, 0));
        assert_eq!(adc.read32(INTS), Some(0));
    }
}
