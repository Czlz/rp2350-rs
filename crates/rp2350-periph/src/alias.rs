//! Atomic SET/CLR/XOR alias windows used by RP2350 peripherals
//! RP2350 外设使用的 SET/CLR/XOR 原子别名窗口
//!
//! Many RP2350 register blocks expose a plain 4 KiB window plus three aliases
//! at `+0x1000` (XOR), `+0x2000` (SET) and `+0x3000` (CLR). A write through an
//! alias performs the corresponding read-modify-write on the plain register.
//! 许多 RP2350 寄存器块在普通 4 KiB 窗口之外提供三个别名：`+0x1000`（XOR）、
//! `+0x2000`（SET）、`+0x3000`（CLR）。通过别名写入会对普通寄存器执行相应的
//! 读-改-写。

/// Alias window selected by offset bits 13:12 / 由偏移位 13:12 选择的别名窗口
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Alias {
    /// Plain write / 普通写
    Normal,
    /// `value = current ^ value` / 异或写
    Xor,
    /// `value = current | value` / 置位写
    Set,
    /// `value = current & !value` / 清零写
    Clr,
}

/// Strip the alias window from an offset / 从偏移中剥离别名窗口
pub(crate) fn split_alias(offset: u32) -> (u32, Alias) {
    match offset & 0x3000 {
        0x1000 => (offset & 0x0FFF, Alias::Xor),
        0x2000 => (offset & 0x0FFF, Alias::Set),
        0x3000 => (offset & 0x0FFF, Alias::Clr),
        _ => (offset & 0x0FFF, Alias::Normal),
    }
}

/// Merge a write with the current register value / 将写入与当前寄存器值合并
pub(crate) fn apply_alias(current: u32, value: u32, alias: Alias) -> u32 {
    match alias {
        Alias::Normal => value,
        Alias::Xor => current ^ value,
        Alias::Set => current | value,
        Alias::Clr => current & !value,
    }
}

#[cfg(test)]
mod tests {
    use super::{Alias, apply_alias, split_alias};

    #[test]
    fn splits_plain_and_alias_windows() {
        assert_eq!(split_alias(0x0C8), (0x0C8, Alias::Normal));
        assert_eq!(split_alias(0x10C8), (0x0C8, Alias::Xor));
        assert_eq!(split_alias(0x20C8), (0x0C8, Alias::Set));
        assert_eq!(split_alias(0x30C8), (0x0C8, Alias::Clr));
    }

    #[test]
    fn applies_alias_semantics() {
        assert_eq!(apply_alias(0x0F0, 0xF00, Alias::Normal), 0xF00);
        assert_eq!(apply_alias(0x0F0, 0x00F, Alias::Xor), 0x0FF);
        assert_eq!(apply_alias(0x0F0, 0x00F, Alias::Set), 0x0FF);
        assert_eq!(apply_alias(0x0FF, 0x00F, Alias::Clr), 0x0F0);
    }
}
