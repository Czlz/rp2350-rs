//! TRNG peripheral stub / TRNG 外设桩
//!
//! Models the Synopsys true random number generator registers used during
//! boot. Enabling the source (`RND_SOURCE_ENABLE.RND_SRC_EN`) immediately
//! collects a deterministic xorshift32 192-bit sample, `TRNG_VALID`/`RNG_ISR`
//! report `EHR_VALID`, and `TRNG_BUSY` always reads zero so the SDK
//! `while (trng_hw->trng_busy)` wait passes at once. Reading `EHR_DATA5`
//! consumes the sample and starts the next one while the source is enabled.
//! 建模启动期间使用的 Synopsys 真随机数发生器寄存器。使能源
//! （`RND_SOURCE_ENABLE.RND_SRC_EN`）后立即用确定性的 xorshift32 采集 192 位
//! 样本，`TRNG_VALID`/`RNG_ISR` 报告 `EHR_VALID`，`TRNG_BUSY` 始终读零，
//! 因此 SDK 的 `while (trng_hw->trng_busy)` 等待立即通过。读 `EHR_DATA5`
//! 会消费当前样本，并在使能源时立即采集下一份。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// `RNG_IMR` offset / `RNG_IMR` 偏移
const RNG_IMR: u32 = 0x100;

/// `RNG_ISR` offset / `RNG_ISR` 偏移
const RNG_ISR: u32 = 0x104;

/// `RNG_ICR` offset / `RNG_ICR` 偏移
const RNG_ICR: u32 = 0x108;

/// `TRNG_CONFIG` offset / `TRNG_CONFIG` 偏移
const TRNG_CONFIG: u32 = 0x10C;

/// `TRNG_VALID` offset / `TRNG_VALID` 偏移
const TRNG_VALID: u32 = 0x110;

/// `EHR_DATA0` offset / `EHR_DATA0` 偏移
const EHR_DATA0: u32 = 0x114;

/// `EHR_DATA5` offset / `EHR_DATA5` 偏移
const EHR_DATA5: u32 = 0x128;

/// `RND_SOURCE_ENABLE` offset / `RND_SOURCE_ENABLE` 偏移
const RND_SOURCE_ENABLE: u32 = 0x12C;

/// `SAMPLE_CNT1` offset / `SAMPLE_CNT1` 偏移
const SAMPLE_CNT1: u32 = 0x130;

/// `TRNG_DEBUG_CONTROL` offset / `TRNG_DEBUG_CONTROL` 偏移
const TRNG_DEBUG_CONTROL: u32 = 0x138;

/// `TRNG_SW_RESET` offset / `TRNG_SW_RESET` 偏移
const TRNG_SW_RESET: u32 = 0x140;

/// `TRNG_BUSY` offset / `TRNG_BUSY` 偏移
const TRNG_BUSY: u32 = 0x1B8;

/// `RNG_VERSION` offset / `RNG_VERSION` 偏移
const RNG_VERSION: u32 = 0x1C0;

/// Number of EHR words in one sample / 一份样本的 EHR 字数
const EHR_WORDS: usize = 6;

/// `RNG_IMR` reset value: every interrupt masked / `RNG_IMR` 复位值：全部中断屏蔽
const RNG_IMR_RESET: u32 = 0x0F;

/// `SAMPLE_CNT1` reset value / `SAMPLE_CNT1` 复位值
const SAMPLE_CNT1_RESET: u32 = 0xFFFF;

/// `RNG_ISR.EHR_VALID` / `RNG_ISR.EHR_VALID`
const ISR_EHR_VALID: u32 = 1;

/// Deterministic PRNG seed / 确定性 PRNG 种子
const SEED: u32 = 0x2350_C0DE;

/// TRNG register block / TRNG 寄存器块
#[derive(Debug)]
pub struct Trng {
    regs: RegisterFile,
    ehr: [u32; EHR_WORDS],
    state: u32,
    enabled: bool,
    valid: bool,
}

impl Trng {
    /// Create the block with the datasheet reset values
    /// 创建使用数据手册复位值的寄存器块
    #[must_use]
    pub fn new() -> Self {
        let mut regs = RegisterFile::new(map::DEVICE_WINDOW_SIZE);
        regs.fill(RNG_IMR, RNG_IMR_RESET);
        regs.fill(SAMPLE_CNT1, SAMPLE_CNT1_RESET);
        Self {
            regs,
            ehr: [0; EHR_WORDS],
            state: SEED,
            enabled: false,
            valid: false,
        }
    }

    /// Next xorshift32 word / 下一个 xorshift32 字
    fn next_word(&mut self) -> u32 {
        let mut value = self.state;
        value ^= value << 13;
        value ^= value >> 17;
        value ^= value << 5;
        self.state = value;
        value
    }

    /// Fill the 192-bit EHR and mark it valid / 填充 192 位 EHR 并标记有效
    fn collect(&mut self) {
        let mut words = [0; EHR_WORDS];
        for word in &mut words {
            *word = self.next_word();
        }
        self.ehr = words;
        self.valid = true;
    }

    /// Clear the volatile TRNG state (software reset) / 清除 TRNG 易变状态（软复位）
    fn reset_state(&mut self) {
        self.ehr = [0; EHR_WORDS];
        self.state = SEED;
        self.enabled = false;
        self.valid = false;
    }
}

impl Default for Trng {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for Trng {
    fn name(&self) -> &str {
        "TRNG"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::TRNG_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        match offset {
            RNG_ISR => {
                let errors = self.regs.read(RNG_ISR).unwrap_or(0) & !ISR_EHR_VALID;
                Some(errors | u32::from(self.valid))
            }
            TRNG_VALID => Some(u32::from(self.valid)),
            RND_SOURCE_ENABLE => Some(u32::from(self.enabled)),
            // 忙状态始终为零，避免 SDK 的等待循环卡死；软复位寄存器读为零。
            // Busy always reads zero so the SDK wait loop never spins; SW reset reads zero.
            TRNG_BUSY | TRNG_SW_RESET => Some(0),
            EHR_DATA0..=EHR_DATA5 => {
                let index = ((offset - EHR_DATA0) / 4) as usize;
                let value = self.ehr[index];
                if offset == EHR_DATA5 && self.enabled {
                    // 最后一份被读走，立即采集下一份。
                    // The last word was consumed; collect the next sample.
                    self.collect();
                }
                Some(value)
            }
            _ => self.regs.read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        match offset {
            RND_SOURCE_ENABLE => {
                self.enabled = value & 1 != 0;
                self.regs.write(offset, u32::from(self.enabled));
                if self.enabled && !self.valid {
                    self.collect();
                }
                true
            }
            RNG_ICR => {
                if value & ISR_EHR_VALID != 0 {
                    // 真实硬件会继续采样；这里立即完成下一份。
                    // Real hardware keeps sampling; complete the next sample now.
                    self.valid = false;
                    if self.enabled {
                        self.collect();
                    }
                }
                true
            }
            TRNG_SW_RESET => {
                if value & 1 != 0 {
                    self.reset_state();
                }
                true
            }
            TRNG_CONFIG | TRNG_DEBUG_CONTROL | RNG_VERSION => {
                self.regs.write(offset, value);
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
    fn reset_values_match_the_datasheet() {
        let mut trng = Trng::new();
        assert_eq!(trng.read32(RNG_IMR), Some(RNG_IMR_RESET));
        assert_eq!(trng.read32(SAMPLE_CNT1), Some(SAMPLE_CNT1_RESET));
        assert_eq!(trng.read32(TRNG_VALID), Some(0));
        assert_eq!(trng.read32(TRNG_BUSY), Some(0));
        assert_eq!(trng.read32(TRNG_CONFIG), Some(0));
    }

    #[test]
    fn enabling_the_source_produces_a_valid_sample() {
        let mut trng = Trng::new();
        assert!(trng.write32(RND_SOURCE_ENABLE, 1));
        assert_eq!(trng.read32(TRNG_VALID), Some(1));
        assert_eq!(trng.read32(RNG_ISR), Some(ISR_EHR_VALID));
        assert_eq!(trng.read32(RND_SOURCE_ENABLE), Some(1));
        assert_eq!(trng.read32(TRNG_BUSY), Some(0));

        let first = trng.read32(EHR_DATA0).unwrap();
        assert_ne!(first, 0);
        for offset in (EHR_DATA0 + 4..=EHR_DATA5).step_by(4) {
            assert!(trng.read32(offset).is_some());
        }
        // 读走最后一份后使能源仍有效。The source stays valid after consumption.
        assert_eq!(trng.read32(TRNG_VALID), Some(1));
        let next = trng.read32(EHR_DATA0).unwrap();
        assert_ne!(first, next);
    }

    #[test]
    fn config_registers_are_readable_storage() {
        let mut trng = Trng::new();
        assert!(trng.write32(TRNG_CONFIG, 0x1234_5678));
        assert_eq!(trng.read32(TRNG_CONFIG), Some(0x1234_5678));
        assert!(trng.write32(TRNG_DEBUG_CONTROL, 0xFFFF_FFFF));
        assert_eq!(trng.read32(TRNG_DEBUG_CONTROL), Some(0xFFFF_FFFF));
    }

    #[test]
    fn software_reset_clears_the_valid_flag_and_source() {
        let mut trng = Trng::new();
        assert!(trng.write32(RND_SOURCE_ENABLE, 1));
        assert!(trng.write32(TRNG_SW_RESET, 1));
        assert_eq!(trng.read32(TRNG_VALID), Some(0));
        assert_eq!(trng.read32(RND_SOURCE_ENABLE), Some(0));
        assert_eq!(trng.read32(RNG_ISR), Some(0));
    }

    #[test]
    fn interrupt_clear_keeps_a_collected_sample_valid() {
        let mut trng = Trng::new();
        assert!(trng.write32(RND_SOURCE_ENABLE, 1));
        assert!(trng.write32(RNG_ICR, ISR_EHR_VALID));
        // 使能源时采样继续，因此有效位立即恢复。
        // Sampling continues while enabled, so the valid bit returns immediately.
        assert_eq!(trng.read32(TRNG_VALID), Some(1));
    }
}
