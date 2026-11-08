//! Boot ROM function/data table encoding
//! Boot ROM 函数/数据表编码
//!
//! An entry is `u16 tag`, `u16 flags`, then one `u16` value for every set flag
//! bit in ascending order. `tag == 0` terminates the table. Function pointers
//! carry the Thumb bit; data pointers are even.
//! 表项为 `u16 tag`、`u16 flags`，随后按置位标志位的升序为每个标志写一个
//! `u16` 值。`tag == 0` 终止表。函数指针带 Thumb 位，数据指针为偶数。

// 表代码由两个 ASCII 字节拼接，`u8 -> u16` 的转换是布局定义的一部分。
// Table codes concatenate two ASCII bytes; the `u8 -> u16` conversion is part
// of the layout definition.
#![allow(clippy::cast_lossless)]

/// Table entry tag built from two ASCII characters / 由两个 ASCII 字符构成的表项标签
pub type TableCode = u16;

/// Table flag bits / 表标志位
pub mod flags {
    /// ARM secure function pointer present / 存在 ARM 安全态函数指针
    pub const FUNC_ARM_SEC: u16 = 0x0004;
    /// ARM non-secure function pointer present / 存在 ARM 非安全态函数指针
    pub const FUNC_ARM_NONSEC: u16 = 0x0010;
    /// Data pointer present / 存在数据指针
    pub const DATA: u16 = 0x0040;
}

/// Build a table code from two ASCII characters / 由两个 ASCII 字符构造表代码
#[must_use]
pub const fn code(c1: u8, c2: u8) -> TableCode {
    (c1 as TableCode) | ((c2 as TableCode) << 8)
}

/// Build a boot ROM table entry by entry / 逐项构建 Boot ROM 表
#[derive(Debug, Default)]
pub struct RomTableBuilder {
    bytes: Vec<u8>,
}

impl RomTableBuilder {
    /// Create an empty table / 创建一个空表
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a function entry carrying both ARM secure and non-secure pointers
    /// 追加一个同时携带 ARM 安全态与非安全态指针的函数表项
    pub fn push_func(&mut self, tag: TableCode, secure: u16, nonsecure: u16) {
        self.push(
            tag,
            flags::FUNC_ARM_SEC | flags::FUNC_ARM_NONSEC,
            &[secure, nonsecure],
        );
    }

    /// Append a data entry / 追加一个数据表项
    pub fn push_data(&mut self, tag: TableCode, pointer: u16) {
        self.push(tag, flags::DATA, &[pointer]);
    }

    /// Append an entry with explicit flags and values / 追加带显式标志与值的表项
    fn push(&mut self, tag: TableCode, flags: u16, values: &[u16]) {
        self.bytes.extend_from_slice(&tag.to_le_bytes());
        self.bytes.extend_from_slice(&flags.to_le_bytes());
        for value in values {
            self.bytes.extend_from_slice(&value.to_le_bytes());
        }
    }

    /// Finish the table by appending the zero terminator / 追加零终止符并结束表
    #[must_use]
    pub fn finish(mut self) -> Vec<u8> {
        self.bytes.extend_from_slice(&0u16.to_le_bytes());
        self.bytes
    }
}

/// Look up `code` in `table`, honouring the requested flag `mask`
/// 在 `table` 中查找 `code`，并按请求的标志 `mask` 选择数据项
///
/// Returns the raw `u16` stored for the first matching flag bit in `mask`.
/// 返回 `mask` 中第一个命中标志位所对应的原始 `u16`。
#[must_use]
pub fn lookup(table: &[u8], code: TableCode, mask: u16) -> Option<u16> {
    let mut cursor = 0usize;
    loop {
        let tag = read_u16(table, cursor)?;
        cursor += 2;
        if tag == 0 {
            return None;
        }
        let entry_flags = read_u16(table, cursor)?;
        cursor += 2;
        let mut matched = None;
        for bit in 0..16 {
            let flag = 1u16 << bit;
            if entry_flags & flag == 0 {
                continue;
            }
            let value = read_u16(table, cursor)?;
            cursor += 2;
            if tag == code && matched.is_none() && mask & flag != 0 {
                matched = Some(value);
            }
        }
        if let Some(value) = matched {
            return Some(value);
        }
    }
}

/// Read a little-endian `u16` at `offset` / 读取 `offset` 处的小端 `u16`
fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let raw = bytes.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([raw[0], raw[1]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_concatenates_ascii_low_byte_first() {
        assert_eq!(code(b'G', b'S'), 0x5347);
        assert_eq!(code(b'I', b'F'), 0x4649);
    }

    #[test]
    fn lookup_selects_the_requested_flag() {
        let mut builder = RomTableBuilder::new();
        builder.push_func(code(b'I', b'F'), 0x0081, 0x0081);
        builder.push_func(code(b'G', b'S'), 0x008B, 0x008B);
        builder.push_data(code(b'F', b'D'), 0x7D00);
        let table = builder.finish();

        assert_eq!(
            lookup(&table, code(b'G', b'S'), flags::FUNC_ARM_SEC),
            Some(0x008B)
        );
        assert_eq!(lookup(&table, code(b'F', b'D'), flags::DATA), Some(0x7D00));
        assert_eq!(lookup(&table, code(b'X', b'X'), flags::DATA), None);
        // 请求函数标志不会命中纯数据表项。
        // A function mask never matches a data-only entry.
        assert_eq!(lookup(&table, code(b'F', b'D'), flags::FUNC_ARM_SEC), None);
    }
}
