//! ADC peripheral model / ADC 外设模型
//!
//! Models the RP2350 ADC register block at `ADC_BASE`. A conversion takes 96
//! `clk_adc` cycles (the datasheet's 12-bit SAR timing), converted into the
//! corresponding number of `clk_sys` cycles with the live clock tree:
//!
//! ```text
//! conversion_cycles = ceil(96 * clk_sys / clk_adc)
//! ```
//!
//! `START_ONCE` schedules one conversion, `START_MANY` keeps re-arming. While a
//! conversion is in flight `CS.READY` reads zero; completion stores the result
//! and, when `FCS.EN` is set, pushes one sample into the four-entry FIFO. The
//! FIFO level/empty/full flags and interrupt bits follow the datasheet layout.
//! Channel selection and input values are not modelled (samples read zero).
//! 建模 `ADC_BASE` 处的 RP2350 ADC 寄存器块。一次转换需要 96 个 `clk_adc` 周期
//! （数据手册的 12 位 SAR 时序），并按实时时钟树换算为相应的 `clk_sys` 周期：
//!
//! ```text
//! conversion_cycles = ceil(96 * clk_sys / clk_adc)
//! ```
//!
//! `START_ONCE` 安排一次转换，`START_MANY` 持续重装。转换进行中 `CS.READY` 读为
//! 零；完成后保存结果，并在 `FCS.EN` 置位时向四深度 FIFO 压入一个样本。FIFO
//! 层级/空/满标志与中断位遵循数据手册布局。通道选择与输入值未建模（样本读零）。

use std::collections::VecDeque;
use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::clock_tree::ClockTree;
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

/// `clk_adc` cycles per 12-bit conversion / 每次 12 位转换的 `clk_adc` 周期数
pub const CONVERSION_CYCLES: u32 = 96;

/// Shared ADC register/FIFO state / 共享的 ADC 寄存器/FIFO 状态
#[derive(Debug)]
struct AdcState {
    regs: RegisterFile,
    fifo: VecDeque<u32>,
    result: u32,
    /// `clk_sys` cycles left in the current conversion / 当前转换剩余的 `clk_sys` 周期
    busy_cycles: u64,
}

impl AdcState {
    /// Create the power-on state / 创建上电状态
    fn new() -> Self {
        Self {
            regs: RegisterFile::new(map::DEVICE_WINDOW_SIZE),
            fifo: VecDeque::with_capacity(FIFO_DEPTH),
            result: 0,
            busy_cycles: 0,
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

    /// Finish the in-flight conversion / 完成进行中的转换
    fn complete(&mut self) {
        self.busy_cycles = 0;
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

/// ADC register block handle / ADC 寄存器块句柄
///
/// Clones share one register/FIFO state and one clock tree, so the bus device
/// and [`crate::BootPeripherals`] observe the same pending conversion.
/// 克隆共享同一寄存器/FIFO 状态与时钟树，因此总线设备与
/// [`crate::BootPeripherals`] 看到同一个进行中的转换。
#[derive(Debug, Clone)]
pub struct Adc {
    state: Arc<Mutex<AdcState>>,
    tree: ClockTree,
}

impl Adc {
    /// Create the block with the bootrom clock tree / 用 bootrom 时钟树创建寄存器块
    #[must_use]
    pub fn new() -> Self {
        Self::with_clock(ClockTree::new())
    }

    /// Create the block following `tree` / 创建跟随 `tree` 的寄存器块
    #[must_use]
    pub fn with_clock(tree: ClockTree) -> Self {
        Self {
            state: Arc::new(Mutex::new(AdcState::new())),
            tree,
        }
    }

    /// `clk_sys` cycles one conversion occupies at the current clocks
    /// 当前时钟下一次转换占用的 `clk_sys` 周期
    #[must_use]
    pub fn conversion_cycles(&self) -> u64 {
        let sysclk = u64::from(self.tree.sysclk_hz());
        let adcclk = u64::from(self.tree.adcclk_hz());
        if adcclk == 0 || sysclk == 0 {
            return 0;
        }
        (u64::from(CONVERSION_CYCLES) * sysclk).div_ceil(adcclk)
    }

    /// Conversion latency in nanoseconds / 转换延迟（纳秒）
    ///
    /// Derived from `clk_adc` alone: `96 / clk_adc`. `None` when `clk_adc` is
    /// stopped. 仅由 `clk_adc` 推导：`96 / clk_adc`；`clk_adc` 停止时返回 `None`。
    #[must_use]
    pub fn conversion_time_ns(&self) -> Option<u64> {
        let adcclk = u64::from(self.tree.adcclk_hz());
        (adcclk != 0).then(|| u64::from(CONVERSION_CYCLES) * 1_000_000_000 / adcclk)
    }

    /// Advance the conversion timer by executed `clk_sys` cycles
    /// 按已执行的 `clk_sys` 周期推进转换计时器
    pub fn advance(&self, cycles: u64, sysclk_hz: u32) {
        let mut state = self.state.lock();
        if state.busy_cycles == 0 {
            return;
        }
        // 频率可能在转换期间改变：按当前时钟重新计算总周期，再扣掉已等待部分。
        // The clock may change mid-conversion: recompute the total at the current
        // rates and subtract what was already waiting.
        let total = if sysclk_hz == 0 || self.tree.adcclk_hz() == 0 {
            0
        } else {
            (u64::from(CONVERSION_CYCLES) * u64::from(sysclk_hz))
                .div_ceil(u64::from(self.tree.adcclk_hz()))
        };
        if cycles >= state.busy_cycles {
            state.complete();
        } else {
            state.busy_cycles -= cycles;
        }
        // START_MANY 在完成后立即重装下一次转换。
        // START_MANY re-arms the next conversion as soon as one completes.
        let cs = state.regs.read(CS).unwrap_or(0);
        if state.busy_cycles == 0 && cs & CS_EN != 0 && cs & CS_START_MANY != 0 {
            state.busy_cycles = total.max(1);
        }
    }

    /// Whether a conversion is in flight / 是否有转换正在进行
    #[must_use]
    pub fn busy(&self) -> bool {
        self.state.lock().busy_cycles != 0
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
        let mut state = self.state.lock();
        match offset {
            CS => {
                let stored = state.regs.read(CS).unwrap_or(0);
                let error = if stored & CS_ERR_STICKY != 0 {
                    CS_ERR
                } else {
                    0
                };
                let ready = if state.busy_cycles == 0 { CS_READY } else { 0 };
                Some((stored & (CS_WRITE_MASK | CS_ERR_STICKY)) | ready | error)
            }
            RESULT => Some(state.result),
            FCS => {
                let mut value = state.regs.read(FCS).unwrap_or(0) & (FCS_WRITE_MASK | FCS_W1C_MASK);
                value |= (state.fifo_level() << FCS_LEVEL_SHIFT) & FCS_LEVEL_MASK;
                if state.fifo.is_empty() {
                    value |= FCS_EMPTY;
                }
                if state.fifo.len() >= FIFO_DEPTH {
                    value |= FCS_FULL;
                }
                Some(value)
            }
            FIFO => {
                if let Some(value) = state.fifo.pop_front() {
                    Some(value)
                } else {
                    let fcs = state.regs.read(FCS).unwrap_or(0);
                    state.regs.write(FCS, fcs | FCS_UNDER);
                    Some(0)
                }
            }
            INTR => Some(state.raw_interrupt()),
            INTS => {
                let raw = state.raw_interrupt();
                let enable = state.regs.read(INTE).unwrap_or(0);
                let force = state.regs.read(INTF).unwrap_or(0);
                Some((raw & enable) | force)
            }
            _ => state.regs.read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        let mut state = self.state.lock();
        match offset {
            CS => {
                let stored = state.regs.read(CS).unwrap_or(0);
                let sticky = stored & CS_ERR_STICKY & !value;
                let writable = value & CS_WRITE_MASK;
                state.regs.write(CS, sticky | writable);
                let enabled = writable & CS_EN != 0;
                if enabled && value & CS_START_ONCE != 0 && state.busy_cycles == 0 {
                    let cycles = self.conversion_cycles();
                    if cycles == 0 {
                        state.busy_cycles = u64::MAX;
                    } else {
                        state.busy_cycles = cycles;
                    }
                }
                if enabled && value & CS_START_MANY != 0 && state.busy_cycles == 0 {
                    let cycles = self.conversion_cycles();
                    if cycles == 0 {
                        state.busy_cycles = u64::MAX;
                    } else {
                        state.busy_cycles = cycles;
                    }
                }
                if !enabled {
                    state.busy_cycles = 0;
                }
                true
            }
            FCS => {
                let stored = state.regs.read(FCS).unwrap_or(0);
                let sticky = stored & FCS_W1C_MASK & !value;
                state.regs.write(FCS, sticky | (value & FCS_WRITE_MASK));
                true
            }
            INTR | INTS => true,
            INTE | INTF => {
                state.regs.write(offset, value & INTR_FIFO);
                true
            }
            _ => state.regs.write(offset, value),
        }
    }

    fn reset(&mut self) {
        *self.state.lock() = AdcState::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock_tree::{CLK_ADC, PllId};

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
    fn a_start_once_conversion_completes_after_96_adc_cycles() {
        let mut adc = Adc::new();
        assert!(adc.write32(CS, CS_EN));
        assert!(adc.write32(FCS, FCS_EN));
        assert!(adc.write32(CS, CS_EN | CS_START_ONCE));
        assert_eq!(adc.read32(CS).unwrap() & CS_READY, 0, "conversion is busy");
        assert!(adc.busy());
        // bootrom clk_sys/clk_adc = 150/48：96 * 150/48 = 300 个 clk_sys 周期。
        // bootrom clk_sys/clk_adc = 150/48, so 96 * 150/48 = 300 clk_sys cycles.
        assert_eq!(adc.conversion_cycles(), 300);
        adc.advance(299, 150_000_000);
        assert!(adc.busy());
        adc.advance(1, 150_000_000);
        assert!(!adc.busy());
        let fcs = adc.read32(FCS).unwrap();
        assert_eq!(fcs & FCS_LEVEL_MASK, 1 << FCS_LEVEL_SHIFT);
        assert_eq!(fcs & FCS_EMPTY, 0);
        assert_eq!(adc.read32(FIFO), Some(0));
        assert_eq!(adc.read32(CS).unwrap() & CS_READY, CS_READY);
    }

    #[test]
    fn conversion_time_follows_clk_adc() {
        let tree = ClockTree::new();
        let adc = Adc::with_clock(tree.clone());
        assert_eq!(adc.conversion_time_ns(), Some(2_000));
        // CLK_ADC 四分频 → 12 MHz：96 周期 = 8 µs。
        // Divide CLK_ADC by four to 12 MHz: 96 cycles = 8 µs.
        tree.write_channel_div(CLK_ADC, 4 << 16);
        assert_eq!(adc.conversion_time_ns(), Some(8_000));
        // 超频到 252 MHz 只改变 clk_sys 侧的周期数，不改变 ADC 转换时间。
        // Overclocking to 252 MHz changes only the clk_sys cycle count.
        tree.write_pll(PllId::Sys, 0x08, 126);
        tree.write_pll(PllId::Sys, 0x0C, (6 << 16) | (1 << 12));
        assert_eq!(adc.conversion_time_ns(), Some(8_000));
        assert_eq!(
            adc.conversion_cycles(),
            (96 * 252_000_000u64).div_ceil(12_000_000)
        );
    }

    #[test]
    fn fifo_interrupt_follows_the_threshold_and_enable_mask() {
        let mut adc = Adc::new();
        assert_eq!(adc.read32(INTR), Some(0));
        assert!(adc.write32(INTE, INTR_FIFO));
        assert!(adc.write32(CS, CS_EN));
        assert!(adc.write32(FCS, FCS_EN));
        assert!(adc.write32(CS, CS_EN | CS_START_ONCE));
        adc.advance(300, 150_000_000);
        // 一份样本达到阈值 0，屏蔽后状态为 1。
        // One sample meets threshold 0, so the masked status is 1.
        assert_eq!(adc.read32(INTR), Some(INTR_FIFO));
        assert_eq!(adc.read32(INTS), Some(INTR_FIFO));
        assert!(adc.write32(INTE, 0));
        assert_eq!(adc.read32(INTS), Some(0));
    }
}
