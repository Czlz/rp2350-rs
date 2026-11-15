//! DMA sniffer checksum engine / DMA 嗅探校验引擎
//!
//! Every unit read by the observed channel is fed into this engine, and the
//! running state lives in `SNIFF_DATA` (`0x458`). `SNIFF_CTRL` (`0x454`),
//! following the RP2350 register header, is laid out as:
//!
//! | Bits | Field | Meaning / 含义 |
//! |------|-------|----------------|
//! | `0` | `EN` | sniffer enable / 嗅探器总开关 |
//! | `4:1` | `DMACH` | observed channel / 被观察通道 |
//! | `8:5` | `CALC` | checksum mode / 校验模式 |
//! | `9` | `BSWAP` | byte-reverse the sniffed data / 嗅探数据按字节反转 |
//! | `10` | `OUT_REV` | bit-reverse the presented result / 读出结果位反转 |
//! | `11` | `OUT_INV` | complement the presented result / 读出结果取反 |
//!
//! `CALC` selects `0x0` CRC-32/IEEE (reflected `0xEDB8_8320`), `0x1` the same
//! CRC with bit-reversed input bytes, `0x2` CRC-16-CCITT (`0x1021`), `0x3` its
//! bit-reversed input variant (`0x8408`), `0xE` the XOR reduction (parity of
//! all data bits, result 0 or 1) and `0xF` a 32-bit addition checksum.
//! `CALC` `0x4..=0xD` are not modelled and leave the accumulator unchanged.
//! `SNIFF_DATA` is the accumulator **and** the seed: a write stores the value
//! verbatim (for example `0xFFFF_FFFF` before a CRC-32 run), each observed read
//! unit updates it, and a read applies only `OUT_REV`/`OUT_INV` to the
//! presented value, never to the stored state.
//! 通道每次读出的数据单元都会送入本引擎，运行状态保存在 `SNIFF_DATA`
//! （`0x458`）。`SNIFF_CTRL`（`0x454`）遵循 RP2350 寄存器头文件布局：
//! `EN[0]` 总开关、`DMACH[4:1]` 被观察通道、`CALC[8:5]` 校验模式、
//! `BSWAP[9]` 嗅探数据按字节反转、`OUT_REV[10]` 读出结果位反转、
//! `OUT_INV[11]` 读出结果取反。`CALC` 取值：`0x0` CRC-32/IEEE（反射
//! `0xEDB8_8320`）、`0x1` 同上但输入字节位反转、`0x2` CRC-16-CCITT
//! （`0x1021`）、`0x3` 其输入位反转变体（`0x8408`）、`0xE` 全部数据位的异或
//! 折叠（奇偶，结果为 0/1）、`0xF` 32 位加法校验和。`CALC` `0x4..=0xD` 未建模，
//! 累加器保持不变。`SNIFF_DATA` 既是累加器也是种子：写入即原样保存（例如
//! CRC-32 前写 `0xFFFF_FFFF`），每次观察到的读单元都会更新它；读出只对展示值
//! 应用 `OUT_REV`/`OUT_INV`，不改变存储状态。

use super::{
    SNIFF_BSWAP, SNIFF_CALC_CRC16, SNIFF_CALC_CRC16R, SNIFF_CALC_CRC32, SNIFF_CALC_CRC32R,
    SNIFF_CALC_MASK, SNIFF_CALC_SHIFT, SNIFF_CALC_SUM, SNIFF_CALC_XOR, SNIFF_OUT_INV,
    SNIFF_OUT_REV,
};

/// CRC-32/IEEE reflected polynomial / CRC-32/IEEE 反射多项式
const CRC32_POLY: u32 = 0xEDB8_8320;

/// CRC-16-CCITT polynomial / CRC-16-CCITT 多项式
const CRC16_POLY: u16 = 0x1021;

/// CRC-16-CCITT reflected polynomial / CRC-16-CCITT 反射多项式
const CRC16_POLY_REFLECTED: u16 = 0x8408;

/// Bits per byte / 每字节位数
const BITS_PER_BYTE: u32 = 8;

/// Extract the `CALC` field / 提取 `CALC` 字段
fn calc(ctrl: u32) -> u32 {
    (ctrl & SNIFF_CALC_MASK) >> SNIFF_CALC_SHIFT
}

/// Number of bytes in one data unit / 单个数据单元的字节数
///
/// `DATA_SIZE` `0`/`1`/`2` are 1/2/4 bytes; the reserved value `3` is treated
/// as a word, matching the transfer path.
/// `DATA_SIZE` 的 `0`/`1`/`2` 对应 1/2/4 字节；保留值 `3` 按字处理，与传输路径一致。
fn unit_bytes(data_size: u8) -> u32 {
    1u32 << u32::min(u32::from(data_size), 2)
}

/// Byte mask of one data unit / 单个数据单元的字节掩码
fn width_mask(data_size: u8) -> u32 {
    match data_size {
        0 => 0x0000_00FF,
        1 => 0x0000_FFFF,
        _ => 0xFFFF_FFFF,
    }
}

/// Feed one observed unit into the accumulator / 将一个观察单元送入累加器
///
/// `BSWAP` reverses the unit's bytes first. CRC modes consume the unit in
/// little-endian byte order; the `R` variants reverse the bits of each byte
/// before feeding them to the same reflected/forward engine.
/// `BSWAP` 先反转单元的字节序。CRC 模式按小端字节序消费单元；`R` 变体在送入
/// 同一反射/正向引擎前把每个字节的位反转。
pub(super) fn accumulate(ctrl: u32, accumulator: &mut u32, value: u32, data_size: u8) {
    let mut value = value;
    if ctrl & SNIFF_BSWAP != 0 {
        value = match data_size {
            0 => value,
            1 => u32::from((value as u16).swap_bytes()),
            _ => value.swap_bytes(),
        };
    }
    let mask = width_mask(data_size);
    match calc(ctrl) {
        SNIFF_CALC_CRC32 => {
            for index in 0..unit_bytes(data_size) {
                let byte = ((value >> (index * BITS_PER_BYTE)) & 0xFF) as u8;
                *accumulator = crc32_byte(*accumulator, byte);
            }
        }
        SNIFF_CALC_CRC32R => {
            for index in 0..unit_bytes(data_size) {
                let byte = ((value >> (index * BITS_PER_BYTE)) & 0xFF) as u8;
                *accumulator = crc32_byte(*accumulator, byte.reverse_bits());
            }
        }
        SNIFF_CALC_CRC16 => {
            let mut crc = *accumulator as u16;
            for index in 0..unit_bytes(data_size) {
                let byte = ((value >> (index * BITS_PER_BYTE)) & 0xFF) as u8;
                crc = crc16_byte(crc, byte);
            }
            *accumulator = u32::from(crc);
        }
        SNIFF_CALC_CRC16R => {
            let mut crc = *accumulator as u16;
            for index in 0..unit_bytes(data_size) {
                let byte = ((value >> (index * BITS_PER_BYTE)) & 0xFF) as u8;
                crc = crc16_byte_reflected(crc, byte.reverse_bits());
            }
            *accumulator = u32::from(crc);
        }
        // XOR 折叠：所有数据位的奇偶，累加器只保留最低位。
        // XOR reduction: parity of every data bit, kept in the lowest bit.
        SNIFF_CALC_XOR => {
            let parity = (value & mask).count_ones() & 1;
            *accumulator = (*accumulator ^ parity) & 1;
        }
        SNIFF_CALC_SUM => {
            *accumulator = (*accumulator).wrapping_add(value & mask);
        }
        // CALC 0x4..=0xD 未建模：累加器保持不变。
        // CALC 0x4..=0xD are not modelled: the accumulator is left unchanged.
        _ => {}
    }
}

/// Value presented by a `SNIFF_DATA` read / `SNIFF_DATA` 读出的值
///
/// `OUT_REV` bit-reverses and `OUT_INV` complements the stored accumulator;
/// the stored value itself is never modified. The two transforms commute.
/// `OUT_REV` 对存储的累加器做位反转，`OUT_INV` 对其取反；存储值本身不变。
/// 两个变换可交换顺序。
#[must_use]
pub(super) fn read(ctrl: u32, accumulator: u32) -> u32 {
    let mut result = accumulator;
    if ctrl & SNIFF_OUT_REV != 0 {
        result = result.reverse_bits();
    }
    if ctrl & SNIFF_OUT_INV != 0 {
        result = !result;
    }
    result
}

/// Advance the reflected CRC-32 register by one byte / 按一个字节推进反射 CRC-32
fn crc32_byte(crc: u32, byte: u8) -> u32 {
    let mut value = crc ^ u32::from(byte);
    for _ in 0..BITS_PER_BYTE {
        // 反射算法：最低位为 1 时右移并异或多项式，否则只右移。
        // Reflected algorithm: shift right and merge the polynomial on a set
        // LSB, otherwise just shift right.
        value = if value & 1 != 0 {
            (value >> 1) ^ CRC32_POLY
        } else {
            value >> 1
        };
    }
    value
}

/// Advance the forward CRC-16-CCITT register by one byte / 按一个字节推进正向 CRC-16
fn crc16_byte(crc: u16, byte: u8) -> u16 {
    let mut value = crc ^ (u16::from(byte) << BITS_PER_BYTE);
    for _ in 0..BITS_PER_BYTE {
        // 正向算法：最高位为 1 时左移并异或多项式，否则只左移。
        // Forward algorithm: shift left and merge the polynomial on a set MSB,
        // otherwise just shift left.
        value = if value & 0x8000 != 0 {
            (value << 1) ^ CRC16_POLY
        } else {
            value << 1
        };
    }
    value
}

/// Advance the reflected CRC-16-CCITT register by one byte / 按一个字节推进反射 CRC-16
fn crc16_byte_reflected(crc: u16, byte: u8) -> u16 {
    let mut value = crc ^ u16::from(byte);
    for _ in 0..BITS_PER_BYTE {
        value = if value & 1 != 0 {
            (value >> 1) ^ CRC16_POLY_REFLECTED
        } else {
            value >> 1
        };
    }
    value
}
