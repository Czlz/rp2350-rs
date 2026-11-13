//! Test-only firmware fixtures / 仅测试使用的固件夹具
//!
//! These helpers build tiny ELF images so machine-level tests do not depend on
//! a real `MicroPython` firmware. / 这些辅助函数构造极小的 ELF 映像，使机器级
//! 测试不依赖真实 `MicroPython` 固件。

// 测试夹具按 ELF32 布局手工拼装字节，需要定长整数转换。
// The fixture assembles ELF32 bytes by hand, which requires fixed-width casts.
#![allow(clippy::cast_possible_truncation)]

use rp2350_core::FLASH_XIP_BASE;

/// Thumb `b .` encoding (branch to self) / Thumb `b .` 编码（跳转到自身）
const THUMB_BRANCH_SELF: [u8; 2] = [0xFE, 0xE7];

/// Build a minimal little-endian ELF32 with one `PT_LOAD` per tuple entry
/// 为元组中的每一项构建一个带 `PT_LOAD` 的最小 LE ELF32 文件
fn build_elf(entry: u32, segments: &[(u32, &[u8], u32)]) -> Vec<u8> {
    const EHDR_SIZE: usize = 52;
    const PHDR_SIZE: usize = 32;
    let phnum = segments.len();
    let data_start = EHDR_SIZE + PHDR_SIZE * phnum;
    let data_len: usize = segments.iter().map(|(_, data, _)| data.len()).sum();
    let mut buf = vec![0u8; data_start + data_len];

    buf[0..4].copy_from_slice(b"\x7FELF");
    buf[4] = 1; // ELFCLASS32
    buf[5] = 1; // ELFDATA2LSB
    buf[6] = 1; // EV_CURRENT
    buf[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    buf[18..20].copy_from_slice(&40u16.to_le_bytes()); // EM_ARM
    buf[20..24].copy_from_slice(&1u32.to_le_bytes()); // e_version
    buf[24..28].copy_from_slice(&entry.to_le_bytes());
    buf[28..32].copy_from_slice(&(EHDR_SIZE as u32).to_le_bytes()); // e_phoff
    buf[40..42].copy_from_slice(&(EHDR_SIZE as u16).to_le_bytes());
    buf[42..44].copy_from_slice(&(PHDR_SIZE as u16).to_le_bytes());
    buf[44..46].copy_from_slice(&(phnum as u16).to_le_bytes());
    buf[46..48].copy_from_slice(&40u16.to_le_bytes()); // e_shentsize

    let mut data_offset = data_start;
    for (i, (vaddr, data, memsz)) in segments.iter().enumerate() {
        let ph = EHDR_SIZE + PHDR_SIZE * i;
        let ph = &mut buf[ph..ph + PHDR_SIZE];
        ph[0..4].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
        ph[4..8].copy_from_slice(&(data_offset as u32).to_le_bytes());
        ph[8..12].copy_from_slice(&vaddr.to_le_bytes()); // p_vaddr
        ph[12..16].copy_from_slice(&vaddr.to_le_bytes()); // p_paddr
        ph[16..20].copy_from_slice(&(data.len() as u32).to_le_bytes());
        ph[20..24].copy_from_slice(&memsz.to_le_bytes());
        ph[24..28].copy_from_slice(&5u32.to_le_bytes()); // PF_R | PF_X
        ph[28..32].copy_from_slice(&4u32.to_le_bytes());
        data_offset += data.len();
    }

    let mut offset = data_start;
    for (_, data, _) in segments {
        buf[offset..offset + data.len()].copy_from_slice(data);
        offset += data.len();
    }

    buf
}

/// ELF image whose entry point branches to itself at the flash XIP base
/// 入口点在 Flash XIP 基地址处无限自跳的 ELF 映像
pub(crate) fn infinite_loop_elf() -> Vec<u8> {
    let data = THUMB_BRANCH_SELF;
    build_elf(
        FLASH_XIP_BASE | 1,
        &[(FLASH_XIP_BASE, &data, data.len() as u32)],
    )
}

/// ELF image that writes `SCB_CPACR` through the PPB and then spins
/// 通过 PPB 写 `SCB_CPACR` 后自旋的 ELF 映像
///
/// Thumb program / Thumb 程序:
/// `ldr r0, [pc, #12]` → `0xE000_ED88`,
/// `ldr r1, [pc, #16]` → `0x0030_0303`,
/// `str r1, [r0]`, `b .`; literals at `+0x10` and `+0x14`.
/// 写入值取自真实 `MicroPython` 固件（CP0/CP4/CP10 全开），与核心启动时
/// 默认的 `CP10_CP11_MASK` 明显不同，便于断言回放确实发生。
/// The value comes from the real `MicroPython` firmware (CP0/CP4/CP10 full
/// access) and differs from the core's power-on `CP10_CP11_MASK`, so the
/// assertion proves the replay actually happened.
/// 若系统写入只在 64 步批量回放，自旋时核内 CPACR 仍为 0，随后的协处理器
/// 指令会误触发 NOCP `UsageFault`（真实 `MicroPython` 固件的故障方式）。
/// If system writes are only replayed every 64 steps, the in-core CPACR stays
/// zero during the spin and later coprocessor instructions spuriously raise a
/// NOCP `UsageFault` (exactly how the real `MicroPython` firmware failed).
pub(crate) fn cpacr_write_elf() -> Vec<u8> {
    let mut data = vec![0u8; 0x18];
    // ldr r0, [pc, #12] / ldr r1, [pc, #16] / str r1, [r0] / b .
    data[0..8].copy_from_slice(&[0x03, 0x48, 0x04, 0x49, 0x01, 0x60, 0xFE, 0xE7]);
    data[0x10..0x14].copy_from_slice(&0xE000_ED88u32.to_le_bytes());
    data[0x14..0x18].copy_from_slice(&0x0030_0303u32.to_le_bytes());
    let memsz = data.len() as u32;
    build_elf(FLASH_XIP_BASE | 1, &[(FLASH_XIP_BASE, &data, memsz)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_is_a_valid_elf_with_thumb_entry() {
        let elf = infinite_loop_elf();
        let image = rp2350_core::FirmwareImage::from_elf_bytes(&elf).expect("ELF");
        assert_eq!(image.entry_point, FLASH_XIP_BASE | 1);
        assert_eq!(image.segments.len(), 1);
        assert_eq!(image.segments[0].address, FLASH_XIP_BASE);
        assert_eq!(image.segments[0].data, THUMB_BRANCH_SELF);
    }
}
