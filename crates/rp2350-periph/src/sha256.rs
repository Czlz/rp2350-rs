//! SHA-256 accelerator stub / SHA-256 加速器桩
//!
//! Implements the RP2350 SHA-256 register block: `CSR` exposes the datasheet
//! reset value `0x1206` (`BSWAP`, `DMA_SIZE = 32-bit`, `SUM_VLD`,
//! `WDATA_RDY`), `START` reloads the FIPS-180-4 initial hash values, sixteen
//! `WDATA` writes compress one 512-bit block, and `SUM0..SUM7` return the
//! running digest. `WDATA_RDY` is always high because the model compresses
//! synchronously.
//! 实现 RP2350 SHA-256 寄存器块：`CSR` 暴露数据手册复位值 `0x1206`
//! （`BSWAP`、`DMA_SIZE = 32-bit`、`SUM_VLD`、`WDATA_RDY`），`START` 重装
//! FIPS-180-4 初始散列值，十六次 `WDATA` 写入压缩一个 512 位块，
//! `SUM0..SUM7` 返回当前摘要。模型同步完成压缩，因此 `WDATA_RDY` 始终为高。

// SHA-256 压缩函数使用 FIPS-180-4 的单字母工作变量 a..h，与规范一致。
// The SHA-256 compression uses the FIPS-180-4 single-letter working variables a..h.
#![allow(clippy::many_single_char_names)]

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// `SHA256_CSR` offset / `SHA256_CSR` 偏移
const CSR: u32 = 0x00;

/// `SHA256_WDATA` offset / `SHA256_WDATA` 偏移
const WDATA: u32 = 0x04;

/// `SHA256_SUM0` offset / `SHA256_SUM0` 偏移
const SUM0: u32 = 0x08;

/// `SHA256_SUM7` offset / `SHA256_SUM7` 偏移
const SUM7: u32 = 0x24;

/// `CSR.DMA_SIZE` mask / `CSR.DMA_SIZE` 掩码
const CSR_DMA_SIZE_MASK: u32 = 0x300;

/// `CSR.BSWAP` / `CSR.BSWAP`
const CSR_BSWAP: u32 = 1 << 12;

/// `CSR.ERR_WDATA_NOT_RDY` / `CSR.ERR_WDATA_NOT_RDY`
const CSR_ERR_WDATA_NOT_RDY: u32 = 1 << 4;

/// `CSR.SUM_VLD` / `CSR.SUM_VLD`
const CSR_SUM_VLD: u32 = 1 << 2;

/// `CSR.WDATA_RDY` / `CSR.WDATA_RDY`
const CSR_WDATA_RDY: u32 = 1 << 1;

/// `CSR.START` / `CSR.START`
const CSR_START: u32 = 1 << 0;

/// `CSR` reset value (`BSWAP`, `DMA_SIZE = 32-bit` and `SUM_VLD`)
/// `CSR` 复位值（`BSWAP`、`DMA_SIZE = 32-bit` 与 `SUM_VLD`）
const CSR_RESET: u32 = 0x0000_1206;

/// Number of message words in one SHA-256 block / 一个 SHA-256 块的消息字数
const BLOCK_WORDS: usize = 16;

/// FIPS-180-4 §5.3.3 initial hash values / FIPS-180-4 §5.3.3 初始散列值
const INITIAL_STATE: [u32; 8] = [
    0x6A09_E667,
    0xBB67_AE85,
    0x3C6E_F372,
    0xA54F_F53A,
    0x510E_527F,
    0x9B05_688C,
    0x1F83_D9AB,
    0x5BE0_CD19,
];

/// FIPS-180-4 §4.2.2 round constants / FIPS-180-4 §4.2.2 轮常量
const ROUND_CONSTANTS: [u32; 64] = [
    0x428A_2F98,
    0x7137_4491,
    0xB5C0_FBCF,
    0xE9B5_DBA5,
    0x3956_C25B,
    0x59F1_11F1,
    0x923F_82A4,
    0xAB1C_5ED5,
    0xD807_AA98,
    0x1283_5B01,
    0x2431_85BE,
    0x550C_7DC3,
    0x72BE_5D74,
    0x80DE_B1FE,
    0x9BDC_06A7,
    0xC19B_F174,
    0xE49B_69C1,
    0xEFBE_4786,
    0x0FC1_9DC6,
    0x240C_A1CC,
    0x2DE9_2C6F,
    0x4A74_84AA,
    0x5CB0_A9DC,
    0x76F9_88DA,
    0x983E_5152,
    0xA831_C66D,
    0xB003_27C8,
    0xBF59_7FC7,
    0xC6E0_0BF3,
    0xD5A7_9147,
    0x06CA_6351,
    0x1429_2967,
    0x27B7_0A85,
    0x2E1B_2138,
    0x4D2C_6DFC,
    0x5338_0D13,
    0x650A_7354,
    0x766A_0ABB,
    0x81C2_C92E,
    0x9272_2C85,
    0xA2BF_E8A1,
    0xA81A_664B,
    0xC24B_8B70,
    0xC76C_51A3,
    0xD192_E819,
    0xD699_0624,
    0xF40E_3585,
    0x106A_A070,
    0x19A4_C116,
    0x1E37_6C08,
    0x2748_774C,
    0x34B0_BCB5,
    0x391C_0CB3,
    0x4ED8_AA4A,
    0x5B9C_CA4F,
    0x682E_6FF3,
    0x748F_82EE,
    0x78A5_636F,
    0x84C8_7814,
    0x8CC7_0208,
    0x90BE_FFFA,
    0xA450_6CEB,
    0xBEF9_A3F7,
    0xC671_78F2,
];

/// Rotate a 32-bit word right / 32 位字循环右移
fn rotr(value: u32, bits: u32) -> u32 {
    value.rotate_right(bits)
}

/// Byte-swap one 32-bit word / 交换一个 32 位字的字节序
fn byteswap(value: u32) -> u32 {
    value.swap_bytes()
}

/// Compress one 512-bit block into the running state / 将一个 512 位块压缩进状态
fn compress(state: &mut [u32; 8], block: &[u32; BLOCK_WORDS]) {
    let mut schedule = [0u32; 64];
    schedule[..BLOCK_WORDS].copy_from_slice(block);
    for index in BLOCK_WORDS..64 {
        let s0 = rotr(schedule[index - 15], 7)
            ^ rotr(schedule[index - 15], 18)
            ^ (schedule[index - 15] >> 3);
        let s1 = rotr(schedule[index - 2], 17)
            ^ rotr(schedule[index - 2], 19)
            ^ (schedule[index - 2] >> 10);
        schedule[index] = schedule[index - 16]
            .wrapping_add(s0)
            .wrapping_add(schedule[index - 7])
            .wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for index in 0..64 {
        let big_s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
        let choose = (e & f) ^ (!e & g);
        let temp1 = h
            .wrapping_add(big_s1)
            .wrapping_add(choose)
            .wrapping_add(ROUND_CONSTANTS[index])
            .wrapping_add(schedule[index]);
        let big_s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = big_s0.wrapping_add(majority);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

/// SHA-256 register block / SHA-256 寄存器块
#[derive(Debug)]
pub struct Sha256 {
    regs: RegisterFile,
    state: [u32; 8],
    block: [u32; BLOCK_WORDS],
    word_count: usize,
    sum_valid: bool,
    error: bool,
}

impl Sha256 {
    /// Create the block with the datasheet reset values
    /// 创建使用数据手册复位值的寄存器块
    #[must_use]
    pub fn new() -> Self {
        let mut regs = RegisterFile::new(map::DEVICE_WINDOW_SIZE);
        regs.fill(CSR, CSR_RESET & (CSR_BSWAP | CSR_DMA_SIZE_MASK));
        Self {
            regs,
            state: [0; 8],
            block: [0; BLOCK_WORDS],
            word_count: 0,
            sum_valid: true,
            error: false,
        }
    }

    /// Restart the digest with the FIPS-180-4 initial state
    /// 使用 FIPS-180-4 初始状态重新开始摘要
    fn start(&mut self) {
        self.state = INITIAL_STATE;
        self.block = [0; BLOCK_WORDS];
        self.word_count = 0;
        self.sum_valid = true;
        self.error = false;
    }
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for Sha256 {
    fn name(&self) -> &str {
        "SHA256"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::SHA256_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        match offset {
            CSR => {
                let mut value = self.regs.read(CSR).unwrap_or(0) | CSR_WDATA_RDY;
                if self.error {
                    value |= CSR_ERR_WDATA_NOT_RDY;
                }
                if self.sum_valid {
                    value |= CSR_SUM_VLD;
                }
                Some(value)
            }
            // WDATA 是只写寄存器。WDATA is write-only.
            WDATA => Some(0),
            SUM0..=SUM7 => {
                let index = ((offset - SUM0) / 4) as usize;
                Some(self.state[index])
            }
            _ => self.regs.read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        match offset {
            CSR => {
                self.regs
                    .write(CSR, value & (CSR_BSWAP | CSR_DMA_SIZE_MASK));
                if value & CSR_ERR_WDATA_NOT_RDY != 0 {
                    self.error = false;
                }
                if value & CSR_START != 0 {
                    self.start();
                }
                true
            }
            WDATA => {
                if self.word_count >= BLOCK_WORDS {
                    return true;
                }
                let bswap = self.regs.read(CSR).unwrap_or(0) & CSR_BSWAP != 0;
                let word = if bswap { byteswap(value) } else { value };
                self.block[self.word_count] = word;
                self.word_count += 1;
                self.sum_valid = false;
                if self.word_count == BLOCK_WORDS {
                    compress(&mut self.state, &self.block);
                    self.word_count = 0;
                    self.sum_valid = true;
                }
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
    fn csr_reset_value_matches_the_datasheet() {
        let mut sha = Sha256::new();
        assert_eq!(sha.read32(CSR), Some(CSR_RESET));
        // SUMx 复位为零，START 后装入初始散列值。
        // SUMx reset to zero; START loads the initial hash values.
        assert_eq!(sha.read32(SUM0), Some(0));
    }

    #[test]
    fn start_loads_the_initial_hash_values() {
        let mut sha = Sha256::new();
        assert!(sha.write32(CSR, CSR_START));
        assert_eq!(sha.read32(SUM0), Some(INITIAL_STATE[0]));
        assert_eq!(sha.read32(SUM7), Some(INITIAL_STATE[7]));
        assert_eq!(sha.read32(CSR).unwrap() & CSR_SUM_VLD, CSR_SUM_VLD);
    }

    #[test]
    fn first_data_word_clears_sum_valid() {
        let mut sha = Sha256::new();
        assert!(sha.write32(CSR, CSR_START));
        assert!(sha.write32(WDATA, 0x1234_5678));
        assert_eq!(sha.read32(CSR).unwrap() & CSR_SUM_VLD, 0);
        assert_eq!(sha.read32(CSR).unwrap() & CSR_WDATA_RDY, CSR_WDATA_RDY);
    }

    #[test]
    fn sixteen_words_compress_one_block() {
        // 一个 512 位全零块的压缩中间态；后续填充块由软件写入。
        // Intermediate state after compressing one all-zero 512-bit block;
        // the padding block is written by software.
        let mut sha = Sha256::new();
        assert!(sha.write32(CSR, CSR_START));
        for _ in 0..BLOCK_WORDS {
            assert!(sha.write32(WDATA, 0));
        }
        assert_eq!(sha.read32(SUM0), Some(0xDA56_98BE));
        assert_eq!(sha.read32(SUM7), Some(0x1837_A9D8));
        assert_eq!(sha.read32(CSR).unwrap() & CSR_SUM_VLD, CSR_SUM_VLD);
    }

    #[test]
    fn bswap_is_configurable() {
        let mut sha = Sha256::new();
        assert_eq!(sha.read32(CSR).unwrap() & CSR_BSWAP, CSR_BSWAP);
        assert!(sha.write32(CSR, 0));
        assert_eq!(sha.read32(CSR).unwrap() & CSR_BSWAP, 0);
        assert!(sha.write32(CSR, CSR_BSWAP));
        assert_eq!(sha.read32(CSR).unwrap() & CSR_BSWAP, CSR_BSWAP);
    }

    #[test]
    fn reset_restores_the_datasheet_values() {
        let mut sha = Sha256::new();
        assert!(sha.write32(CSR, CSR_START));
        assert!(sha.write32(WDATA, 1));
        sha.reset();
        assert_eq!(sha.read32(CSR), Some(CSR_RESET));
        assert_eq!(sha.read32(SUM0), Some(0));
    }
}
