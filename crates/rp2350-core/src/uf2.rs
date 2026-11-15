//! UF2 firmware container parsing and loading
//! UF2 固件容器解析与加载
//!
//! A UF2 file is a sequence of fixed 512-byte blocks. Each block carries a target
//! address and up to 476 payload bytes, so a firmware image can be routed to flash
//! or SRAM without a linker file.
//! UF2 文件由固定 512 字节块组成，每块携带目标地址和最多 476 字节载荷，
//! 因此固件映像无需链接脚本即可路由到 Flash 或 SRAM。
//!
//! The loader intentionally does not interpret the RP2350 `IMAGE_DEF` metadata:
//! it only validates the container and writes bytes to the address bus.
//! 本加载器刻意不解析 RP2350 `IMAGE_DEF` 元数据，只校验容器并把字节写入地址总线。

// UF2 解析涉及地址与长度在 usize/u32 之间的换算，这是固件加载层的固有特性。
// UF2 parsing converts addresses and lengths between usize and u32, which is
// inherent to the firmware loading layer.
#![allow(clippy::cast_possible_truncation)]

use super::bus::AddressBus;
use super::error::CoreError;
use super::types::Address;

/// Size of one UF2 block in bytes / 单个 UF2 块大小（字节）
pub const UF2_BLOCK_SIZE: usize = 512;

/// Maximum payload bytes in one UF2 block / 单个 UF2 块的最大载荷字节数
pub const UF2_MAX_PAYLOAD: usize = 476;

/// First start magic (`"UF2\n"` little-endian) / 第一个起始魔数
pub const UF2_MAGIC_START0: u32 = 0x0A32_4655;

/// Second start magic / 第二个起始魔数
pub const UF2_MAGIC_START1: u32 = 0x9E5D_5157;

/// End magic / 结束魔数
pub const UF2_MAGIC_END: u32 = 0x0AB1_6F30;

/// Block must not be written to main flash / 该块不属于主 Flash
pub const UF2_FLAG_NOT_MAIN_FLASH: u32 = 0x0000_0001;

/// Block is part of a file container / 该块属于文件容器
pub const UF2_FLAG_FILE_CONTAINER: u32 = 0x0000_1000;

/// Block carries a `familyID` field / 该块包含 `familyID` 字段
pub const UF2_FLAG_FAMILY_ID_PRESENT: u32 = 0x0000_2000;

/// Block carries an MD5 checksum / 该块包含 MD5 校验和
pub const UF2_FLAG_MD5_PRESENT: u32 = 0x0000_4000;

/// Block carries extension tags / 该块包含扩展标签
pub const UF2_FLAG_EXTENSION_TAGS_PRESENT: u32 = 0x0000_8000;

/// Flash XIP window base / Flash XIP 窗口起始地址
pub const UF2_FLASH_BASE: Address = 0x1000_0000;

/// Flash XIP window end (exclusive, 16 MB) / Flash XIP 窗口结束地址（不含，16 MB）
pub const UF2_FLASH_END: Address = 0x1100_0000;

/// SRAM window base / SRAM 窗口起始地址
pub const UF2_SRAM_BASE: Address = 0x2000_0000;

/// SRAM window end (exclusive, 520 KB) / SRAM 窗口结束地址（不含，520 KB）
pub const UF2_SRAM_END: Address = 0x2008_2000;

/// CPU architecture encoded by a UF2 family ID / UF2 家族 ID 对应的 CPU 架构
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Uf2Arch {
    /// Arm Cortex-M33 image / Arm Cortex-M33 映像
    Arm,
    /// Hazard3 RISC-V image / Hazard3 RISC-V 映像
    RiscV,
}

/// UF2 family identifiers used by RP2040/RP2350 firmware / RP2040/RP2350 固件家族 ID
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Uf2Family {
    /// RP2040 family / RP2040 家族
    Rp2040,
    /// RP2350 Arm Secure family / RP2350 Arm 安全家族
    Rp2350ArmSecure,
    /// RP2350 Hazard3 RISC-V family / RP2350 Hazard3 RISC-V 家族
    Rp2350RiscV,
    /// RP2350 Arm Non-Secure family / RP2350 Arm 非安全家族
    Rp2350ArmNonSecure,
    /// RP2350 absolute-address payload / RP2350 绝对地址载荷
    Rp2350Absolute,
    /// RP2350 data payload / RP2350 数据载荷
    Rp2350Data,
    /// Block has no family ID / 块中没有家族 ID
    NoFamily,
    /// Unrecognised family ID / 未识别的家族 ID
    Unknown(u32),
}

impl Uf2Family {
    /// Decode a raw family ID / 解码原始家族 ID
    #[must_use]
    pub fn from_id(id: u32) -> Self {
        match id {
            0x68ED_2B88 | 0xE48B_FF56 => Self::Rp2040,
            0xE48B_FF59 => Self::Rp2350ArmSecure,
            0xE48B_FF5A => Self::Rp2350RiscV,
            0xE48B_FF5B => Self::Rp2350ArmNonSecure,
            0xE48B_FF57 => Self::Rp2350Absolute,
            0xE48B_FF58 => Self::Rp2350Data,
            other => Self::Unknown(other),
        }
    }

    /// Return the raw family ID / 返回原始家族 ID
    #[must_use]
    pub fn id(self) -> u32 {
        match self {
            Self::Rp2040 => 0x68ED_2B88,
            Self::Rp2350ArmSecure => 0xE48B_FF59,
            Self::Rp2350RiscV => 0xE48B_FF5A,
            Self::Rp2350ArmNonSecure => 0xE48B_FF5B,
            Self::Rp2350Absolute => 0xE48B_FF57,
            Self::Rp2350Data => 0xE48B_FF58,
            Self::NoFamily => 0,
            Self::Unknown(other) => other,
        }
    }

    /// Return the CPU architecture implied by this family / 返回该家族对应的 CPU 架构
    #[must_use]
    pub fn arch(self) -> Option<Uf2Arch> {
        match self {
            Self::Rp2350ArmSecure | Self::Rp2350ArmNonSecure | Self::Rp2040 => Some(Uf2Arch::Arm),
            Self::Rp2350RiscV => Some(Uf2Arch::RiscV),
            Self::Rp2350Absolute | Self::Rp2350Data | Self::NoFamily | Self::Unknown(_) => None,
        }
    }

    /// Whether this family denotes concrete RP2350 firmware / 是否为具体 RP2350 固件家族
    #[must_use]
    pub fn is_rp2350(self) -> bool {
        matches!(
            self,
            Self::Rp2350ArmSecure
                | Self::Rp2350ArmNonSecure
                | Self::Rp2350RiscV
                | Self::Rp2350Absolute
                | Self::Rp2350Data
        )
    }
}

impl std::fmt::Display for Uf2Family {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rp2040 => write!(f, "RP2040"),
            Self::Rp2350ArmSecure => write!(f, "RP2350 Arm Secure"),
            Self::Rp2350RiscV => write!(f, "RP2350 RISC-V"),
            Self::Rp2350ArmNonSecure => write!(f, "RP2350 Arm Non-Secure"),
            Self::Rp2350Absolute => write!(f, "RP2350 absolute"),
            Self::Rp2350Data => write!(f, "RP2350 data"),
            Self::NoFamily => write!(f, "no family"),
            Self::Unknown(id) => write!(f, "unknown (0x{id:08X})"),
        }
    }
}

/// Memory region targeted by a UF2 block / UF2 块的目标内存区域
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Uf2Region {
    /// Flash XIP window / Flash XIP 窗口
    Flash,
    /// SRAM window / SRAM 窗口
    Sram,
}

/// Classify an address into the flash or SRAM window / 将地址归类到 Flash 或 SRAM 窗口
#[must_use]
pub fn region_for(addr: Address) -> Option<Uf2Region> {
    if (UF2_FLASH_BASE..UF2_FLASH_END).contains(&addr) {
        Some(Uf2Region::Flash)
    } else if (UF2_SRAM_BASE..UF2_SRAM_END).contains(&addr) {
        Some(Uf2Region::Sram)
    } else {
        None
    }
}

/// One parsed UF2 block / 一个解析后的 UF2 块
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uf2Block {
    /// Sequential block number / 块序号
    pub block_number: u32,
    /// Total block count of this image group / 该映像组的块总数
    pub total_blocks: u32,
    /// Block flags / 块标志
    pub flags: u32,
    /// Family ID (valid only if `UF2_FLAG_FAMILY_ID_PRESENT`) / 家族 ID
    pub family: Uf2Family,
    /// Target load address / 目标加载地址
    pub target_addr: Address,
    /// Payload bytes / 载荷字节
    pub payload: Vec<u8>,
}

impl Uf2Block {
    /// Parse one 512-byte UF2 block / 解析一个 512 字节的 UF2 块
    ///
    /// # Errors
    /// Returns `CoreError::FirmwareError` if the block size, magic words, or
    /// payload size are invalid.
    /// 当块大小、魔数或载荷长度非法时返回 `CoreError::FirmwareError`。
    pub fn parse(bytes: &[u8]) -> Result<Self, CoreError> {
        if bytes.len() != UF2_BLOCK_SIZE {
            return Err(CoreError::FirmwareError(format!(
                "UF2 block has {} bytes, expected {UF2_BLOCK_SIZE} / UF2 块长度错误",
                bytes.len()
            )));
        }

        let magic0 = read_u32(bytes, 0);
        let magic1 = read_u32(bytes, 4);
        let magic_end = read_u32(bytes, UF2_BLOCK_SIZE - 4);
        if magic0 != UF2_MAGIC_START0 || magic1 != UF2_MAGIC_START1 || magic_end != UF2_MAGIC_END {
            return Err(CoreError::FirmwareError(format!(
                "UF2 magic mismatch (start0={magic0:#010X}, start1={magic1:#010X}, \
                 end={magic_end:#010X}) / UF2 魔数不匹配"
            )));
        }

        let flags = read_u32(bytes, 8);
        let target_addr = read_u32(bytes, 12);
        let payload_size = read_u32(bytes, 16);
        let block_number = read_u32(bytes, 20);
        let total_blocks = read_u32(bytes, 24);
        let family_word = read_u32(bytes, 28);

        let payload_size = usize::try_from(payload_size).map_err(|_| {
            CoreError::FirmwareError("UF2 payload size overflow / UF2 载荷长度溢出".to_string())
        })?;
        if payload_size > UF2_MAX_PAYLOAD {
            return Err(CoreError::FirmwareError(format!(
                "UF2 payload size {payload_size} exceeds {UF2_MAX_PAYLOAD} / UF2 载荷过大"
            )));
        }
        if total_blocks == 0 {
            return Err(CoreError::FirmwareError(
                "UF2 block reports zero total blocks / UF2 块总数为零".to_string(),
            ));
        }
        if block_number >= total_blocks {
            return Err(CoreError::FirmwareError(format!(
                "UF2 block {block_number} is outside declared count {total_blocks} / \
                 UF2 块序号越界"
            )));
        }

        let payload = bytes[32..32 + payload_size].to_vec();
        let family = if flags & UF2_FLAG_FAMILY_ID_PRESENT != 0 {
            Uf2Family::from_id(family_word)
        } else {
            Uf2Family::NoFamily
        };

        Ok(Self {
            block_number,
            total_blocks,
            flags,
            family,
            target_addr,
            payload,
        })
    }

    /// Whether this block should be written to the target memory / 该块是否应写入目标内存
    #[must_use]
    pub fn is_loadable(&self) -> bool {
        self.flags & (UF2_FLAG_NOT_MAIN_FLASH | UF2_FLAG_FILE_CONTAINER) == 0
    }

    /// End address (exclusive) of the payload / 载荷结束地址（不含）
    #[must_use]
    pub fn end_addr(&self) -> Option<Address> {
        let size = u64::try_from(self.payload.len()).ok()?;
        let end = u64::from(self.target_addr).checked_add(size)?;
        u32::try_from(end).ok()
    }
}

/// Summary of a UF2 load operation / UF2 加载操作摘要
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uf2LoadReport {
    /// Primary family of the image / 映像的主要家族
    pub family: Uf2Family,
    /// Total blocks in the file / 文件中的块总数
    pub blocks_total: usize,
    /// Blocks written to the bus / 写入总线的块数
    pub blocks_written: usize,
    /// Blocks skipped (metadata or non-main-flash) / 跳过的块数
    pub blocks_skipped: usize,
    /// Lowest written address / 写入的最低地址
    pub min_address: Address,
    /// Highest written address (exclusive) / 写入的最高地址（不含）
    pub max_address: Address,
    /// Whether the flash window received bytes / Flash 窗口是否收到字节
    pub wrote_flash: bool,
    /// Whether the SRAM window received bytes / SRAM 窗口是否收到字节
    pub wrote_sram: bool,
}

/// A parsed UF2 image / 解析完成的 UF2 映像
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uf2Image {
    blocks: Vec<Uf2Block>,
    family: Uf2Family,
}

impl Uf2Image {
    /// Parse a complete UF2 file / 解析完整 UF2 文件
    ///
    /// # Errors
    /// Returns `CoreError::FirmwareError` when the file is empty, not a multiple
    /// of 512 bytes, contains malformed blocks, or repeats a block number inside
    /// the same family group.
    /// 当文件为空、长度不是 512 的倍数、块格式错误，或同一家族组内块号重复时，
    /// 返回 `CoreError::FirmwareError`。
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CoreError> {
        if bytes.is_empty() {
            return Err(CoreError::FirmwareError(
                "UF2 file is empty / UF2 文件为空".to_string(),
            ));
        }
        if bytes.len() % UF2_BLOCK_SIZE != 0 {
            return Err(CoreError::FirmwareError(format!(
                "UF2 size {} is not a multiple of {UF2_BLOCK_SIZE} / UF2 长度不是 512 的倍数",
                bytes.len()
            )));
        }

        let mut blocks = Vec::with_capacity(bytes.len() / UF2_BLOCK_SIZE);
        for chunk in bytes.chunks_exact(UF2_BLOCK_SIZE) {
            blocks.push(Uf2Block::parse(chunk)?);
        }

        // 用哈希集合代替 `Vec::contains`：emerald 这类 12 MB 映像有 4.8 万块，
        // 线性查重是 O(n²)（十亿级比较），实测占启动时间的绝大部分。
        // A hash set replaces `Vec::contains`: a 12 MB image like emerald has
        // ~48 k blocks, and the linear duplicate scan is O(n²) (billions of
        // comparisons), which dominated startup time.
        let mut seen: std::collections::HashSet<(u32, u32)> =
            std::collections::HashSet::with_capacity(blocks.len());
        for block in &blocks {
            let key = (block.family.id(), block.block_number);
            if !seen.insert(key) {
                return Err(CoreError::FirmwareError(format!(
                    "duplicate UF2 block {} for family {} / UF2 块号重复",
                    block.block_number, block.family
                )));
            }
        }

        let family = select_primary_family(&blocks);
        Ok(Self { blocks, family })
    }

    /// Parse a UF2 file from disk / 从磁盘解析 UF2 文件
    ///
    /// # Errors
    /// Returns `CoreError::FirmwareError` if the file cannot be read or parsed.
    /// 文件读取或解析失败时返回 `CoreError::FirmwareError`。
    pub fn from_file(path: &std::path::Path) -> Result<Self, CoreError> {
        let bytes = std::fs::read(path).map_err(|e| {
            CoreError::FirmwareError(format!(
                "cannot read UF2 file {}: {e} / 无法读取 UF2 文件",
                path.display()
            ))
        })?;
        Self::from_bytes(&bytes)
    }

    /// Primary image family / 映像主要家族
    #[must_use]
    pub fn family(&self) -> Uf2Family {
        self.family
    }

    /// All parsed blocks in file order / 按文件顺序返回所有块
    #[must_use]
    pub fn blocks(&self) -> &[Uf2Block] {
        &self.blocks
    }

    /// Number of blocks in the file / 文件中的块总数
    #[must_use]
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Whether the image targets the given architecture / 映像是否面向指定架构
    #[must_use]
    pub fn matches_arch(&self, arch: Uf2Arch) -> bool {
        self.blocks
            .iter()
            .filter(|block| block.is_loadable())
            .filter_map(|block| block.family.arch())
            .any(|block_arch| block_arch == arch)
    }

    /// Write every loadable block into the address bus / 将所有可加载块写入地址总线
    ///
    /// # Errors
    /// Returns `CoreError::FirmwareError` if a block targets an address outside
    /// the modelled flash/SRAM windows or wraps the 32-bit address space. Returns
    /// a bus error if the target region is not mapped.
    /// 当块目标地址不在 Flash/SRAM 窗口内或跨越 32 位地址空间时返回
    /// `CoreError::FirmwareError`；目标区域未映射时返回总线错误。
    pub fn load_into(&self, bus: &mut dyn AddressBus) -> Result<Uf2LoadReport, CoreError> {
        let mut cycles = 0u64;
        let mut written = 0usize;
        let mut skipped = 0usize;
        let mut min_address = Address::MAX;
        let mut max_address = 0u32;
        let mut wrote_flash = false;
        let mut wrote_sram = false;

        for block in &self.blocks {
            if !block.is_loadable() {
                skipped += 1;
                continue;
            }
            let Some(end_addr) = block.end_addr() else {
                return Err(CoreError::FirmwareError(format!(
                    "UF2 block at {:#010X} wraps the address space / UF2 块跨越地址空间",
                    block.target_addr
                )));
            };
            if region_for(block.target_addr).is_none() {
                return Err(CoreError::FirmwareError(format!(
                    "UF2 block targets {:#010X}, outside flash/SRAM / \
                     UF2 块目标地址不在 Flash/SRAM 窗口",
                    block.target_addr
                )));
            }

            // 整块载荷一次写入：区域只解析一次、只取一次锁、只递增一次世代。
            // Write the whole payload in one call: the region is resolved once,
            // takes one lock and bumps the content generation once.
            bus.load_bytes(block.target_addr, &block.payload, &mut cycles)?;

            written += 1;
            min_address = min_address.min(block.target_addr);
            max_address = max_address.max(end_addr);
            match region_for(block.target_addr) {
                Some(Uf2Region::Flash) => wrote_flash = true,
                Some(Uf2Region::Sram) => wrote_sram = true,
                None => {}
            }
        }

        if written == 0 {
            return Err(CoreError::FirmwareError(
                "UF2 contains no loadable blocks / UF2 没有可加载的块".to_string(),
            ));
        }

        Ok(Uf2LoadReport {
            family: self.family,
            blocks_total: self.blocks.len(),
            blocks_written: written,
            blocks_skipped: skipped,
            min_address,
            max_address,
            wrote_flash,
            wrote_sram,
        })
    }
}

/// Pick the primary family, ignoring absolute/data/no-family helper blocks.
/// 选择主要家族，忽略绝对地址、数据和无家族辅助块。
fn select_primary_family(blocks: &[Uf2Block]) -> Uf2Family {
    blocks
        .iter()
        .map(|block| block.family)
        .find(|family| {
            !matches!(
                family,
                Uf2Family::NoFamily | Uf2Family::Rp2350Absolute | Uf2Family::Rp2350Data
            )
        })
        .or_else(|| {
            blocks
                .iter()
                .map(|block| block.family)
                .find(|family| *family != Uf2Family::NoFamily)
        })
        .unwrap_or(Uf2Family::NoFamily)
}

/// Read a little-endian u32 from a fixed block / 从固定块中读取小端 u32
fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes[offset..offset + 4]);
    u32::from_le_bytes(word)
}

#[cfg(test)]
#[path = "uf2_tests.rs"]
mod tests;
