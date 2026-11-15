//! Test-only firmware fixtures / 仅测试使用的固件夹具
//!
//! These helpers build tiny ELF images so machine-level tests do not depend on
//! a real `MicroPython` firmware. / 这些辅助函数构造极小的 ELF 映像，使机器级
//! 测试不依赖真实 `MicroPython` 固件。

// 测试夹具按 ELF32 布局手工拼装字节，需要定长整数转换。
// SysTick 是 ARM 手册中的驼峰寄存器名，夹具文档保持硬件写法。
// 夹具被 lib 与 bin 两个测试目标共享，每个目标只用到其中一部分。
// The fixture assembles ELF32 bytes by hand, which requires fixed-width casts;
// SysTick is the CamelCase name used by the ARM reference manuals and the shared
// fixtures are used by both the lib and bin test targets.
#![allow(clippy::cast_possible_truncation, clippy::doc_markdown, dead_code)]

use rp2350_core::FLASH_XIP_BASE;

/// Thumb `b .` encoding (branch to self) / Thumb `b .` 编码（跳转到自身）
const THUMB_BRANCH_SELF: [u8; 2] = [0xFE, 0xE7];

/// Build a minimal little-endian ELF32 with one `PT_LOAD` per tuple entry
/// 为元组中的每一项构建一个带 `PT_LOAD` 的最小 LE ELF32 文件
pub(crate) fn build_elf(entry: u32, segments: &[(u32, &[u8], u32)]) -> Vec<u8> {
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

/// ELF image that counts in `r0` inside a two-instruction loop
/// 在两条指令的循环里用 `r0` 计数的 ELF 映像
///
/// Thumb program / Thumb 程序:
/// `movs r0, #0` → `adds r0, #1` → `b` back to `adds`, so the PC alternates
/// between two addresses and the recent-step ring holds three distinct PCs.
/// `movs r0, #0` → `adds r0, #1` → 跳回 `adds`，因此 PC 在两个地址间交替，
/// 最近步数环形缓冲包含三种不同的 PC。
pub(crate) fn counter_elf() -> Vec<u8> {
    // movs r0,#0 / adds r0,#1 / b <adds>
    let data = [0x00u8, 0x20, 0x01, 0x30, 0xFD, 0xE7];
    build_elf(
        FLASH_XIP_BASE | 1,
        &[(FLASH_XIP_BASE, &data, data.len() as u32)],
    )
}

/// ELF image that branches to the unmapped address `0x3000_0000`
/// 跳转到未映射地址 `0x3000_0000` 的 ELF 映像
///
/// Thumb program / Thumb 程序: `ldr r0, [pc, #0]` → `bx r0`, with the literal
/// `0x3000_0001` right behind it, so the fetch after the branch faults on an
/// unmapped address (a bus error, not an INVSTATE).
/// `ldr r0, [pc, #0]` → `bx r0`，字面量 `0x3000_0001` 紧随其后，因此跳转后的取指
/// 落在未映射地址上（总线错误，而不是 INVSTATE）。
pub(crate) fn unmapped_branch_elf() -> Vec<u8> {
    let mut data = vec![0u8; 8];
    // ldr r0, [pc, #0] / bx r0
    data[0..4].copy_from_slice(&[0x00, 0x48, 0x00, 0x47]);
    data[4..8].copy_from_slice(&0x3000_0001u32.to_le_bytes());
    build_elf(
        FLASH_XIP_BASE | 1,
        &[(FLASH_XIP_BASE, &data, data.len() as u32)],
    )
}

/// Shared-SRAM word the synthetic SysTick handler increments
/// 合成 SysTick 处理程序自增的共享 SRAM 字
pub(crate) const SYSTICK_COUNTER: u32 = 0x2000_9000;

/// Flash address of the synthetic SysTick handler
/// 合成 SysTick 处理程序的 Flash 地址
pub(crate) const SYSTICK_HANDLER: u32 = FLASH_XIP_BASE + 0x280;

/// Initial stack pointer used by the fixture (top of SRAM)
/// 夹具使用的初始栈指针（SRAM 顶部）
const FIXTURE_SP: u32 = 0x2008_2000;

/// Put one 16-bit Thumb halfword into an image / 在映像中写入一个 16 位 Thumb 半字
fn put_half(image: &mut [u8], offset: usize, value: u16) {
    image[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

/// Put one 32-bit word into an image / 在映像中写入一个 32 位字
fn put_word(image: &mut [u8], offset: usize, value: u32) {
    image[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// ELF image whose core 0 programs SysTick and counts handler entries
/// core0 编程 SysTick 并统计处理程序进入次数的 ELF 映像
///
/// The image holds a vector table at the flash base with `SysTick` (exception
/// 15) at `VTOR + 0x3C`, a guest program at `+0x200` that writes `SYST_RVR`,
/// `SYST_CVR` and `SYST_CSR` through the PPB, and a handler at `+0x280` that
/// increments [`SYSTICK_COUNTER`] in SRAM. With `masked_delay` the program
/// starts with `cpsid i` and only executes `cpsie i` after that many
/// decrement/branch iterations, so a test can hold the forwarded exception
/// behind `PRIMASK` for a deterministic number of steps.
/// 映像在 Flash 基址放置向量表（`SysTick` 异常 15 位于 `VTOR + 0x3C`），
/// `+0x200` 的客户机程序经 PPB 写 `SYST_RVR`、`SYST_CVR`、`SYST_CSR`，
/// `+0x280` 的处理程序自增 SRAM 中的 [`SYSTICK_COUNTER`]。传入
/// `masked_delay` 时程序以 `cpsid i` 开始，执行该数量的递减/分支迭代后才
/// `cpsie i`，测试因此能把转发后的异常稳定地挡在 `PRIMASK` 之后。
pub(crate) fn systick_elf(reload: u32, csr: u32, masked_delay: Option<u32>) -> Vec<u8> {
    let mut data = vec![0u8; 0x300];
    put_word(&mut data, 0x00, FIXTURE_SP);
    put_word(&mut data, 0x04, FLASH_XIP_BASE + 0x201);
    // 异常 15 的向量位于 VTOR + 15 * 4 = +0x3C。
    // Exception 15 vectors at VTOR + 15 * 4 = +0x3C.
    put_word(&mut data, 0x3C, SYSTICK_HANDLER | 1);

    // 0x200：写 RVR/CVR/CSR 后自旋；掩码变体在前面插入 cpsid i 与延时循环。
    // 0x200: program RVR/CVR/CSR then spin; the masked variant prefixes
    // `cpsid i` and a delay loop.
    let program: &[u16] = if masked_delay.is_some() {
        &[
            0xB672, // cpsid i
            0x4C0B, // ldr r4,[pc,#0x2C] -> RVR
            0x4D0E, // ldr r5,[pc,#0x38] -> reload value
            0x6025, // str r5,[r4]
            0x4C0A, // ldr r4,[pc,#0x28] -> CVR
            0x6025, // str r5,[r4]
            0x4C0B, // ldr r4,[pc,#0x2C] -> CSR
            0x4D0D, // ldr r5,[pc,#0x34] -> CSR value
            0x6025, // str r5,[r4]
            0x4F0D, // ldr r7,[pc,#0x34] -> delay iterations
            0x3F01, // subs r7,#1
            0xD1FD, // bne 0x214
            0xB662, // cpsie i
            0xE7FE, // b .
        ]
    } else {
        &[
            0x4C0B, // ldr r4,[pc,#0x2C] -> RVR
            0x4D0F, // ldr r5,[pc,#0x3C] -> reload value
            0x6025, // str r5,[r4]
            0x4C0B, // ldr r4,[pc,#0x2C] -> CVR
            0x6025, // str r5,[r4]
            0x4C0C, // ldr r4,[pc,#0x30] -> CSR
            0x4D0D, // ldr r5,[pc,#0x34] -> CSR value
            0x6025, // str r5,[r4]
            0xE7FE, // b .
        ]
    };
    for (index, halfword) in program.iter().enumerate() {
        put_half(&mut data, 0x200 + index * 2, *halfword);
    }

    put_word(&mut data, 0x230, 0xE000_E014); // SYST_RVR
    put_word(&mut data, 0x234, 0xE000_E018); // SYST_CVR
    put_word(&mut data, 0x23C, 0xE000_E010); // SYST_CSR
    put_word(&mut data, 0x240, reload);
    put_word(&mut data, 0x244, csr);
    if let Some(delay) = masked_delay {
        put_word(&mut data, 0x248, delay);
    }

    // 0x280：处理程序自增 SRAM 计数后异常返回。
    // 0x280: the handler bumps the SRAM counter and returns from the exception.
    put_half(&mut data, 0x280, 0x4803); // ldr r0,[pc,#12] -> 0x290
    put_half(&mut data, 0x282, 0x6801); // ldr r1,[r0]
    put_half(&mut data, 0x284, 0x3101); // adds r1,#1
    put_half(&mut data, 0x286, 0x6001); // str r1,[r0]
    put_half(&mut data, 0x288, 0x4770); // bx lr
    put_word(&mut data, 0x290, SYSTICK_COUNTER);

    let memsz = data.len() as u32;
    build_elf(FLASH_XIP_BASE + 0x201, &[(FLASH_XIP_BASE, &data, memsz)])
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
