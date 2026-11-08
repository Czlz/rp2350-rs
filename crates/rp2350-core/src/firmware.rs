//! ELF firmware image parsing and loading into the address bus
//! ELF 固件映像解析与加载到地址总线

// 固件加载涉及 u32/usize 之间的容量换算，这是硬件仿真层的固有特性。
// Firmware loading converts between u32 and usize for size arithmetic, which is
// inherent to the hardware emulation layer.
#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap
)]

use elf::ElfBytes;
use elf::abi::PT_LOAD;
use elf::endian::AnyEndian;

use super::bus::AddressBus;
use super::error::CoreError;
use super::types::Address;

/// One loadable ELF segment
/// 一个可加载的 ELF 段
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirmwareSegment {
    /// Load address from `p_vaddr` / 由 `p_vaddr` 得到的加载地址
    pub address: Address,
    /// Initialized bytes from the file / 来自文件的已初始化字节
    pub data: Vec<u8>,
    /// Total memory size (`p_memsz`), including the zero-filled `.bss` tail
    /// 总内存大小（`p_memsz`），包含零填充的 `.bss` 尾部
    pub mem_size: usize,
}

/// A parsed firmware image ready to be loaded into memory
/// 解析完成、可直接加载到内存的固件映像
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FirmwareImage {
    /// Entry point from the ELF header / ELF 头中的入口点
    pub entry_point: Address,
    /// `PT_LOAD` segments in file order / 按文件顺序排列的 `PT_LOAD` 段
    pub segments: Vec<FirmwareSegment>,
}

impl FirmwareImage {
    /// Parse an ELF image (ELF32 or ELF64, either endianness)
    /// 解析 ELF 映像（ELF32 或 ELF64，任意字节序）
    ///
    /// Only `PT_LOAD` segments are kept. A segment whose `p_memsz` exceeds its
    /// `p_filesz` carries `.bss`-style zero-filled data that `load_into` writes.
    /// 仅保留 `PT_LOAD` 段。若某段的 `p_memsz` 大于 `p_filesz`，则其携带
    /// `.bss` 风格的零填充数据，由 `load_into` 写入。
    ///
    /// # Errors
    /// Returns `CoreError::FirmwareError` if the ELF is malformed, a `PT_LOAD`
    /// segment lies outside the 32-bit address space, or its file size exceeds
    /// its memory size.
    /// 当 ELF 格式错误、`PT_LOAD` 段超出 32 位地址空间，或其文件大小超过
    /// 内存大小时，返回 `CoreError::FirmwareError`。
    pub fn from_elf_bytes(bytes: &[u8]) -> Result<Self, CoreError> {
        let elf = ElfBytes::<AnyEndian>::minimal_parse(bytes).map_err(|e| {
            CoreError::FirmwareError(format!("ELF parse failed / ELF 解析失败: {e}"))
        })?;

        let entry_point = u32::try_from(elf.ehdr.e_entry).map_err(|_| {
            CoreError::FirmwareError(format!(
                "entry point {:#X} exceeds 32-bit address space / 入口点超出 32 位地址空间",
                elf.ehdr.e_entry
            ))
        })?;

        let Some(phdrs) = elf.segments() else {
            return Ok(Self {
                entry_point,
                segments: Vec::new(),
            });
        };

        let mut segments = Vec::new();
        for phdr in phdrs.iter() {
            if phdr.p_type != PT_LOAD {
                continue;
            }
            if phdr.p_filesz > phdr.p_memsz {
                return Err(CoreError::FirmwareError(format!(
                    "PT_LOAD segment at {:#X} has filesz {:#X} > memsz {:#X} / \
                     段文件大小超过内存大小",
                    phdr.p_vaddr, phdr.p_filesz, phdr.p_memsz
                )));
            }
            let address = u32::try_from(phdr.p_vaddr).map_err(|_| {
                CoreError::FirmwareError(format!(
                    "segment address {:#X} exceeds 32-bit address space / 段地址超出 32 位地址空间",
                    phdr.p_vaddr
                ))
            })?;
            let mem_size = u32::try_from(phdr.p_memsz).map_err(|_| {
                CoreError::FirmwareError(format!(
                    "segment size {:#X} exceeds 32-bit address space / 段大小超出 32 位地址空间",
                    phdr.p_memsz
                ))
            })?;
            address.checked_add(mem_size).ok_or_else(|| {
                CoreError::FirmwareError(format!(
                    "segment at {address:#X} with size {mem_size:#X} wraps the address space / \
                     段地址加大小跨越地址空间边界"
                ))
            })?;
            let data = elf
                .segment_data(&phdr)
                .map_err(|e| {
                    CoreError::FirmwareError(format!(
                        "segment data unavailable / 段数据不可用: {e}"
                    ))
                })?
                .to_vec();
            segments.push(FirmwareSegment {
                address,
                data,
                mem_size: mem_size as usize,
            });
        }

        Ok(Self {
            entry_point,
            segments,
        })
    }

    /// Write every segment into `bus`, zero-filling any `.bss` tail
    /// 将所有段写入 `bus`，并对 `.bss` 尾部进行零填充
    ///
    /// # Errors
    /// Returns a bus error if any segment address is not mapped.
    /// 若任一段地址未映射，返回总线错误。
    pub fn load_into(&self, bus: &mut dyn AddressBus) -> Result<(), CoreError> {
        let mut cycles = 0u64;
        for segment in &self.segments {
            for (offset, byte) in segment.data.iter().enumerate() {
                let addr = segment.address.wrapping_add(offset as u32);
                bus.write_u8(addr, *byte, &mut cycles)?;
            }
            for offset in segment.data.len()..segment.mem_size {
                let addr = segment.address.wrapping_add(offset as u32);
                bus.write_u8(addr, 0, &mut cycles)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // 测试辅助函数按 ELF32 布局手工拼装字节，需要定长整数转换。
    // The test helper assembles ELF32 bytes by hand, which requires fixed-width casts.
    #![allow(clippy::cast_possible_truncation)]

    use super::*;
    use crate::memory::{RamRegion, SimpleBus};

    fn put_u16(buf: &mut [u8], offset: usize, value: u16) {
        buf[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(buf: &mut [u8], offset: usize, value: u32) {
        buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// Build a minimal little-endian ELF32 with one `PT_LOAD` per tuple entry.
    /// 为元组中的每一项构建一个带 `PT_LOAD` 的最小 LE ELF32 文件。
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
        put_u16(&mut buf, 16, 2); // ET_EXEC
        put_u16(&mut buf, 18, 40); // EM_ARM
        put_u32(&mut buf, 20, 1); // e_version
        put_u32(&mut buf, 24, entry);
        put_u32(&mut buf, 28, EHDR_SIZE as u32); // e_phoff
        put_u16(&mut buf, 40, EHDR_SIZE as u16);
        put_u16(&mut buf, 42, PHDR_SIZE as u16);
        put_u16(&mut buf, 44, phnum as u16);
        put_u16(&mut buf, 46, 40); // e_shentsize

        let mut data_offset = data_start;
        for (i, (vaddr, data, memsz)) in segments.iter().enumerate() {
            let ph = EHDR_SIZE + PHDR_SIZE * i;
            put_u32(&mut buf, ph, PT_LOAD);
            put_u32(&mut buf, ph + 4, data_offset as u32);
            put_u32(&mut buf, ph + 8, *vaddr);
            put_u32(&mut buf, ph + 12, *vaddr);
            put_u32(&mut buf, ph + 16, data.len() as u32);
            put_u32(&mut buf, ph + 20, *memsz);
            put_u32(&mut buf, ph + 24, 5); // PF_R | PF_X
            put_u32(&mut buf, ph + 28, 4);
            buf[data_offset..data_offset + data.len()].copy_from_slice(data);
            data_offset += data.len();
        }
        buf
    }

    #[test]
    fn parses_entry_point_and_load_segment() {
        let elf = build_elf(0x1000_0001, &[(0x1000_0000, &[0xAA, 0xBB], 4)]);
        let image = FirmwareImage::from_elf_bytes(&elf).unwrap();
        assert_eq!(image.entry_point, 0x1000_0001);
        assert_eq!(image.segments.len(), 1);
        assert_eq!(image.segments[0].address, 0x1000_0000);
        assert_eq!(image.segments[0].data, vec![0xAA, 0xBB]);
        assert_eq!(image.segments[0].mem_size, 4);
    }

    #[test]
    fn load_into_writes_data_and_zero_fills_bss() {
        let elf = build_elf(0x2000_0000, &[(0x2000_0000, &[0x11, 0x22, 0x33], 8)]);
        let image = FirmwareImage::from_elf_bytes(&elf).unwrap();

        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(RamRegion::new("ram", 0x2000_0000, 64)))
            .unwrap();
        // Pre-fill with 0xFF so zero-fill is observable.
        // 预先填充 0xFF，以便观察零填充效果。
        let mut cycles = 0;
        for addr in 0x2000_0000..0x2000_0008 {
            bus.write_u8(addr, 0xFF, &mut cycles).unwrap();
        }

        image.load_into(&mut bus).unwrap();
        assert_eq!(bus.read_u8(0x2000_0000, &mut cycles).unwrap(), 0x11);
        assert_eq!(bus.read_u8(0x2000_0002, &mut cycles).unwrap(), 0x33);
        for addr in 0x2000_0003..0x2000_0008 {
            assert_eq!(bus.read_u8(addr, &mut cycles).unwrap(), 0);
        }
    }

    #[test]
    fn load_into_unmapped_address_fails() {
        let elf = build_elf(0x1000_0000, &[(0x1000_0000, &[1, 2, 3], 3)]);
        let image = FirmwareImage::from_elf_bytes(&elf).unwrap();
        let mut bus = SimpleBus::new();
        assert!(image.load_into(&mut bus).is_err());
    }

    #[test]
    fn malformed_elf_returns_firmware_error() {
        let err = FirmwareImage::from_elf_bytes(b"not an elf file").unwrap_err();
        match err {
            CoreError::FirmwareError(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
