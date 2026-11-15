//! Decoded-instruction cache and threaded dispatch for the Thumb fetch path
//! Thumb 取指路径的译码结果缓存与线程化分发
//!
//! Every step otherwise spends most of its time re-deriving the same facts from
//! the same bytes: the instruction length (one or two halfwords), the opcode
//! class, and the operand fields. Firmware spin loops execute the same few
//! dozen instructions millions of times, so a direct-mapped table keyed by `PC`
//! that stores the already-decoded result turns those repeated computations
//! into one tag compare plus one relaxed generation load.
//! 每一步原本都在从同样的字节重复推导同样的事实：指令长度（一个还是两个半字）、
//! 操作码类别、以及操作数字段。固件的自旋循环会把同样几十条指令执行几百万次，
//! 因此一张按 `PC` 索引、存放已译码结果的直接映射表，可以把这些重复计算变成
//! 一次标签比较加一次 relaxed 世代号读取。
//!
//! Consistency / 一致性：
//!
//! * Only addresses inside the two flash XIP windows are cached. Guest writes to
//!   flash are rejected and every host-side mutation publishes
//!   [`rp2350_core::flash::content_generation`], so a tag compare plus the
//!   generation check is exact.
//!   只缓存两个 Flash XIP 窗口内的地址。客户机写 Flash 会被拒绝，并且每次宿主侧
//!   变更都会发布 [`rp2350_core::flash::content_generation`]，因此标签比较加世代
//!   检查是精确的。
//! * `SRAM` is **never** cached: it is writable, so a decoded entry there could be
//!   stale on the very next step. Serving it exactly would need a per-16-byte-line
//!   generation table consulted on the hot path; that costs more per step than
//!   the fetch it saves, and no measured workload executes from `SRAM`.
//!   `SRAM` **永不**缓存：它可写，所以那里的译码条目下一步就可能过期。要做到精确
//!   就必须在热路径上查询按 16 字节行的世代表；那比它节省的取指更贵，而实测负载
//!   都不在 `SRAM` 中执行。
//! * Entries never change instruction semantics: they only carry the fields the
//!   shared executors in `execute_thumb16::fast` and
//!   `execute_thumb32::execute_group_tag` need, and both are also the code paths
//!   used when the cache is off.
//!   条目绝不改变指令语义：它只携带 `execute_thumb16::fast` 与
//!   `execute_thumb32::execute_group_tag` 中共享执行器所需的字段，而这两处也是
//!   缓存关闭时使用的代码路径。

use rp2350_core::flash::content_generation;
use rp2350_core::{FLASH_XIP_BASE, FLASH_XIP_NOCACHE_BASE, FLASH_XIP_SIZE};

/// Number of direct-mapped decoded-instruction entries / 直接映射译码缓存条目数
///
/// The measured emerald spin loop spans 0x42 bytes (24 instructions); a
/// 1024-entry table covers 2 KiB of code without conflicts, so the hot loop
/// always stays resident while the table stays small enough to share L1 with
/// the fetch cache.
/// 实测的 emerald 自旋循环跨 0x42 字节（24 条指令）；1024 条表覆盖 2 KiB 代码无冲突，
/// 热循环因此总能驻留，而整表仍小到可与取指缓存共享一级缓存。
pub const DECODE_CACHE_ENTRIES: usize = 1024;

/// Index mask for the direct-mapped table / 直接映射表的索引掩码
const DECODE_CACHE_MASK: usize = DECODE_CACHE_ENTRIES - 1;

/// Size of one flash XIP window as `u32` / 单个 Flash XIP 窗口大小（u32）
#[allow(clippy::cast_possible_truncation)]
const WINDOW_SIZE: u32 = FLASH_XIP_SIZE as u32;

/// Decoded dispatch tag / 译码分发标签
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    /// Empty slot (never a hit) / 空槽位（永不命中）
    Empty = 0,
    /// Not specialised: run the full Thumb-16 decoder / 未特化：走完整 Thumb-16 译码器
    Slow16 = 1,
    /// Not specialised: run the full Thumb-32 decoder / 未特化：走完整 Thumb-32 译码器
    Slow32 = 2,
    /// `B` (unconditional branch), target in `aux` / `B`（无条件分支），目标在 `aux`
    B16 = 3,
    /// `B<cond>`, target in `aux`, condition in `group` / `B<cond>`，目标在 `aux`，条件在 `group`
    BCond16 = 4,
    /// `LDR (literal)`, address in `aux` / `LDR（文字池）`，地址在 `aux`
    LdrLiteral16 = 5,
    /// `LDR (immediate)` word form / `LDR（立即数）` 字形式
    LdrWordImm5 = 6,
    /// `STR (immediate)` word form / `STR（立即数）` 字形式
    StrWordImm5 = 7,
    /// Thumb-16 data-processing (register) group / Thumb-16 数据处理（寄存器）组
    DpRegister16 = 8,
    /// Thumb-16 hint space (`NOP`/`YIELD`/`WFE`/`WFI`/`SEV`) / Thumb-16 提示空间
    Hint16 = 9,
    /// Thumb-16 `CPS` (PRIMASK/FAULTMASK) / Thumb-16 `CPS`
    Cps16 = 10,
    /// Thumb-32 integer group, group id in `group` / Thumb-32 整数组，组号在 `group`
    Group32 = 11,
}

impl Kind {
    /// Decode a stored tag byte / 解析存储的标签字节
    #[cfg(test)]
    #[must_use]
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Slow16,
            2 => Self::Slow32,
            3 => Self::B16,
            4 => Self::BCond16,
            5 => Self::LdrLiteral16,
            6 => Self::LdrWordImm5,
            7 => Self::StrWordImm5,
            8 => Self::DpRegister16,
            9 => Self::Hint16,
            10 => Self::Cps16,
            11 => Self::Group32,
            _ => Self::Empty,
        }
    }
}

/// Thumb-32 integer group identifiers / Thumb-32 整数组编号
///
/// These mirror the `op1`/`op2` branches of
/// [`crate::execute_thumb32::execute_thumb32`] one for one; the cache only
/// stores the choice that the decoder would make again.
/// 这些编号与 [`crate::execute_thumb32::execute_thumb32`] 的 `op1`/`op2` 分支一一对应；
/// 缓存只保存译码器本来会重新做出的选择。
pub mod group32 {
    /// Load/store multiple, dual, exclusive, table branch / 多寄存器、双字、独占、表分支
    pub const LOAD_STORE_GROUP: u8 = 0;
    /// Data processing (shifted register) / 数据处理（移位寄存器）
    pub const SHIFTED_REGISTER: u8 = 1;
    /// Branch and miscellaneous control / 分支与杂项控制
    pub const BRANCH_MISC: u8 = 2;
    /// Data processing (modified immediate) / 数据处理（改良立即数）
    pub const MODIFIED_IMMEDIATE: u8 = 3;
    /// Plain binary immediate / 纯二进制立即数
    pub const PLAIN_IMMEDIATE: u8 = 4;
    /// `SSAT`/`USAT` and the 16-bit variants / `SSAT`/`USAT` 及 16 位变体
    pub const SATURATE: u8 = 5;
    /// Bitfield `BFC`/`BFI`/`SBFX`/`UBFX` / 位域指令
    pub const BITFIELD: u8 = 6;
    /// Load/store single / 单字加载存储
    pub const SINGLE: u8 = 7;
    /// Data processing (register) / 数据处理（寄存器）
    pub const REGISTER: u8 = 8;
    /// Multiply, divide and long multiply / 乘除法与长乘法
    pub const MULTIPLY: u8 = 9;
}

/// One decoded instruction / 一条已译码指令
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// Instruction address the entry was filled from (0 = empty)
    /// 填充该条目的指令地址（0 表示空）
    pub tag: u32,
    /// Instruction word, first halfword in bits 31:16 / 指令字，第一个半字在 31:16
    pub word: u32,
    /// Kind-specific pre-computed value (branch target, literal address)
    /// 与类别相关的预计算值（分支目标、文字池地址）
    pub aux: u32,
    /// Dispatch tag / 分发标签
    pub kind: Kind,
    /// Thumb-32 group id, or the condition code for [`Kind::BCond16`]
    /// Thumb-32 组号，或 [`Kind::BCond16`] 的条件码
    pub group: u8,
    /// Instruction length in bytes (2 or 4) / 指令长度（2 或 4 字节）
    pub len: u8,
    /// Reserved / 保留
    pub reserved: u8,
}

impl Default for Entry {
    fn default() -> Self {
        Self {
            tag: 0,
            word: 0,
            aux: 0,
            kind: Kind::Empty,
            group: 0,
            len: 0,
            reserved: 0,
        }
    }
}

impl Entry {
    /// Fetch cycles this instruction books (two per halfword, one extra per fetch)
    /// 该指令应记入的取指周期（每个半字两个，外加每次取指一个）
    #[must_use]
    pub fn fetch_cycles(self) -> u64 {
        if self.len == 4 { 6 } else { 3 }
    }
}

/// Whether `addr` lies in a flash XIP window / `addr` 是否落在 Flash XIP 窗口内
#[must_use]
pub fn in_flash_window(addr: u32) -> bool {
    addr.wrapping_sub(FLASH_XIP_BASE) < WINDOW_SIZE
        || addr.wrapping_sub(FLASH_XIP_NOCACHE_BASE) < WINDOW_SIZE
}

/// Direct-mapped decoded-instruction cache / 直接映射译码缓存
pub struct DecodeCache {
    /// Cached entries / 缓存的条目
    entries: Box<[Entry; DECODE_CACHE_ENTRIES]>,
    /// Flash generation the table was filled under / 填充该表时的 Flash 世代号
    generation: u32,
    /// Generation flushes performed so far / 已执行的世代清空次数
    clears: u32,
}

impl DecodeCache {
    /// Create an empty cache / 创建空缓存
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Box::new([Entry::default(); DECODE_CACHE_ENTRIES]),
            // 构造时把世代号置为当前值；零标签的条目不会命中真实 Flash 地址。
            // Seed with the current generation; zero-tagged entries never match
            // a real flash address.
            generation: content_generation(),
            clears: 0,
        }
    }

    /// Drop every entry after a generation change / 世代变化后清空全部条目
    #[inline]
    fn refresh(&mut self, generation: u32) {
        if self.generation != generation {
            self.generation = generation;
            self.entries.fill(Entry::default());
            self.clears = self.clears.wrapping_add(1);
        }
    }

    /// Look up the decoded instruction at `pc` / 查找 `pc` 处的已译码指令
    ///
    /// Returns `None` for addresses outside the flash windows or for a miss.
    /// Only [`Self::store`] can create an entry, and it refuses addresses
    /// outside the flash windows and empty kinds, so a tag match plus a
    /// non-[`Kind::Empty`] kind proves the hit came from a stored flash
    /// instruction. The `Empty` check is required because PC=0 equals the tag
    /// of a never-filled slot.
    /// 落在 Flash 窗口之外或未命中时返回 `None`。只有 [`Self::store`] 能创建
    /// 条目，而它拒绝窗口外地址与空类别，因此标签相等且类别不是 [`Kind::Empty`]
    /// 即证明命中来自已存储的 Flash 指令。`Empty` 检查是必需的，因为 PC=0 与
    /// 从未填充的槽位标签相等。
    #[inline]
    pub fn lookup(&mut self, pc: u32) -> Option<Entry> {
        self.refresh(content_generation());
        let entry = self.entries[((pc >> 1) as usize) & DECODE_CACHE_MASK];
        // 空条目的标签为 0；地址 0 本身不在 Flash 窗口内，但 PC=0 仍会与空条目
        // 的标签相等，因此必须显式排除 `Kind::Empty`，否则核心会在地址 0 上执行
        // 一条虚构的 `word = 0` 指令而不访问总线。
        // Empty entries carry tag 0. Address 0 is not inside a flash window, but
        // PC=0 still matches that tag, so `Kind::Empty` must be rejected
        // explicitly; otherwise the core would execute a phantom `word = 0`
        // instruction at address 0 without touching the bus.
        if entry.tag == pc && entry.kind != Kind::Empty {
            return Some(entry);
        }
        None
    }

    /// Store one decoded instruction / 存储一条已译码指令
    ///
    /// Entries outside the flash windows are ignored, and a fill that races a
    /// host-side mutation is resolved by the next generation check.
    /// 窗口外的条目会被忽略；若填充期间发生宿主侧变更，下一次世代检查会纠正。
    #[inline]
    pub fn store(&mut self, entry: Entry) {
        if !in_flash_window(entry.tag) || entry.kind == Kind::Empty {
            return;
        }
        self.refresh(content_generation());
        self.entries[((entry.tag >> 1) as usize) & DECODE_CACHE_MASK] = entry;
    }

    /// Number of generation flushes since construction / 自构造以来的世代清空次数
    #[must_use]
    pub fn generation_flushes(&self) -> u32 {
        self.clears
    }
}

impl Default for DecodeCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Decode a Thumb-16 instruction word into an [`Entry`] / 把 Thumb-16 指令字译码成 [`Entry`]
///
/// `pc` is the address the word was fetched from; branch targets and literal
/// addresses are resolved here so the hot path never recomputes them.
/// `pc` 是取指地址；分支目标与文字池地址在此解析，热路径因此无需重算。
#[must_use]
pub fn decode16(pc: u32, word: u16) -> Entry {
    let ty = (word >> 11) & 0x1F;
    let mut entry = Entry {
        tag: pc,
        word: u32::from(word),
        kind: Kind::Slow16,
        len: 2,
        ..Entry::default()
    };
    match ty {
        // 0b01000 with bit10 clear: data-processing (register), 16 operations.
        // 0b01000 且 bit10 为 0：数据处理（寄存器），共 16 种操作。
        0b01000 if (word >> 10) & 1 == 0 => entry.kind = Kind::DpRegister16,
        // 0b01001: LDR (literal).
        // 0b01001：LDR（文字池）。
        0b01001 => {
            let imm8 = u32::from(word & 0xFF);
            entry.kind = Kind::LdrLiteral16;
            entry.aux = (pc.wrapping_add(4) & !0x3).wrapping_add(imm8 << 2);
        }
        // 0b01100 / 0b01101: STR / LDR (immediate) word forms.
        // 0b01100 / 0b01101：STR / LDR（立即数）字形式。
        0b01100 => entry.kind = Kind::StrWordImm5,
        0b01101 => entry.kind = Kind::LdrWordImm5,
        // 0b10110 with the 0xB6xx prefix: CPS for PRIMASK/FAULTMASK.
        // 0b10110 且前缀为 0xB6xx：PRIMASK/FAULTMASK 的 CPS。
        0b10110 if (word & 0xFFF0) == 0xB660 || (word & 0xFFF0) == 0xB670 => {
            entry.kind = Kind::Cps16;
        }
        // 0b10111 with the 0xBFxx hint prefix and a zero mask: hint space only.
        // 0b10111 且前缀为 0xBFxx 掩码为零：仅提示空间。
        0b10111 if (word >> 8) == 0xBF && word.trailing_zeros() >= 4 => entry.kind = Kind::Hint16,
        // 0b11010/0b11011: B<cond>, or SVC when cond == 0xF.
        // 0b11010/0b11011：B<cond>，cond == 0xF 时为 SVC。
        0b11010..=0b11011 => {
            let cond = ((word >> 8) & 0xF) as u8;
            if cond < 0xE {
                let offset = sign_extend(u32::from(word & 0xFF) << 1, 9);
                entry.kind = Kind::BCond16;
                entry.group = cond;
                entry.aux = pc.wrapping_add(4).wrapping_add(offset);
            }
        }
        // 0b11100: unconditional B.
        // 0b11100：无条件 B。
        0b11100 => {
            entry.kind = Kind::B16;
            entry.aux = pc
                .wrapping_add(4)
                .wrapping_add(sign_extend(u32::from(word & 0x7FF) << 1, 12));
        }
        _ => {}
    }
    entry
}

/// Decode a Thumb-32 instruction word into an [`Entry`] / 把 Thumb-32 指令字译码成 [`Entry`]
///
/// Encodings that the FPU or the coprocessor decoders claim are left at
/// [`Kind::Slow32`] so the full decoder path (including its usage-fault
/// handling) runs unchanged. Everything else records the integer group that
/// `execute_thumb32` would select.
/// 由 FPU 或协处理器解码器认领的编码保持 [`Kind::Slow32`]，以便完整解码路径（含其
/// 用法故障处理）原样执行；其余编码记录 `execute_thumb32` 会选择的整数组。
#[must_use]
pub fn decode32(pc: u32, word: u32) -> Entry {
    let mut entry = Entry {
        tag: pc,
        word,
        kind: Kind::Slow32,
        len: 4,
        ..Entry::default()
    };
    // 0xEC..0xEE 与 0xFE 前缀同时被 FPU（搬移/算术/块传输）和协处理器（CP0/CP4
    // MCR/MRC/MCRR/MRRC）认领；这两种解码器都在常规整数树之前运行，因此这些前缀
    // 一律走完整路径。
    // The 0xEC..0xEE and 0xFE prefixes are claimed by the FPU (transfers,
    // arithmetic, block transfers) and by the coprocessor (CP0/CP4 MCR/MRC/
    // MCRR/MRRC) decoders; both run before the integer tree, so those prefixes
    // always take the full path.
    if matches!(word >> 24, 0xEC..=0xEE | 0xFE) {
        return entry;
    }
    let op1 = (word >> 27) & 0x3;
    let op = (word >> 20) & 0x7F;
    let group = match op1 {
        0b01 => {
            let op2 = (word >> 25) & 0x3;
            match op2 {
                0 => Some(group32::LOAD_STORE_GROUP),
                1 => Some(group32::SHIFTED_REGISTER),
                _ => None,
            }
        }
        0b10 => {
            if word & 0x8000 != 0 {
                Some(group32::BRANCH_MISC)
            } else if (word >> 25) & 1 == 0 {
                Some(group32::MODIFIED_IMMEDIATE)
            } else if matches!(op, 0x20..=0x2F | 0x60..=0x6F) {
                Some(group32::PLAIN_IMMEDIATE)
            } else if matches!(op, 0x30 | 0x32 | 0x38 | 0x3A) {
                Some(group32::SATURATE)
            } else if matches!(op, 0x34 | 0x36 | 0x3C) {
                Some(group32::BITFIELD)
            } else {
                None
            }
        }
        0b11 => {
            if op < 0x20 {
                Some(group32::SINGLE)
            } else if op < 0x30 {
                Some(group32::REGISTER)
            } else if op <= 0x3F {
                Some(group32::MULTIPLY)
            } else {
                None
            }
        }
        _ => None,
    };
    if let Some(group) = group {
        entry.kind = Kind::Group32;
        entry.group = group;
    }
    entry
}

/// Sign-extend a value from `bits` bits to 32 bits / 将值从 `bits` 位符号扩展到 32 位
fn sign_extend(value: u32, bits: u32) -> u32 {
    let mask = (1u32 << bits) - 1;
    let sign = 1u32 << (bits - 1);
    let truncated = value & mask;
    if truncated & sign != 0 {
        truncated | !mask
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash_windows_are_cacheable_and_sram_is_not() {
        assert!(in_flash_window(FLASH_XIP_BASE));
        assert!(in_flash_window(FLASH_XIP_NOCACHE_BASE));
        assert!(!in_flash_window(0x2000_0000));
        let mut cache = DecodeCache::new();
        cache.store(decode16(0x2000_0010, 0x4770));
        assert!(cache.lookup(0x2000_0010).is_none());
    }

    #[test]
    fn empty_slots_never_match_pc_zero() {
        // PC=0 与空槽位的 tag=0 相等，但空槽位不是有效条目：核心必须回落到
        // 总线取指（真实固件在 boot ROM 地址 0 上执行）。
        // PC=0 matches an empty slot's tag 0, but an empty slot is not a valid
        // entry: the core must fall back to a bus fetch (real firmware executes
        // at boot-ROM address 0).
        let mut cache = DecodeCache::new();
        assert!(cache.lookup(0).is_none());
        cache.store(decode16(FLASH_XIP_BASE, 0xBF00));
        assert!(cache.lookup(0).is_none());
        assert!(cache.lookup(FLASH_XIP_BASE).is_some());
    }

    #[test]
    fn decoded_fields_round_trip() {
        let mut cache = DecodeCache::new();
        // `b.n 0x1000B222` at 0x1000B262.
        let entry = decode16(0x1000_B262, 0xE7DE);
        assert_eq!(entry.kind, Kind::B16);
        assert_eq!(entry.len, 2);
        assert_eq!(entry.aux, 0x1000_B222);
        cache.store(entry);
        let hit = cache.lookup(0x1000_B262).expect("hit");
        assert_eq!(hit, entry);
        assert_eq!(hit.fetch_cycles(), 3);
    }

    #[test]
    fn conditional_branch_precomputes_target_and_condition() {
        let entry = decode16(0x1000_B226, 0xD804);
        assert_eq!(entry.kind, Kind::BCond16);
        assert_eq!(entry.group, 0x8);
        assert_eq!(entry.aux, 0x1000_B232);
        // cond == 0xF is SVC, never a fast branch.
        // cond == 0xF 是 SVC，绝不当快速分支。
        assert_eq!(decode16(0x1000_B226, 0xDF04).kind, Kind::Slow16);
    }

    #[test]
    fn thumb32_groups_match_the_decoder_tree() {
        // `ldaexb r8,[r3]` / `ldaexb r8,[r3]`
        let entry = decode32(0x1000_B23A, 0xE8D3_8FCF);
        assert_eq!(entry.kind, Kind::Group32);
        assert_eq!(entry.group, group32::LOAD_STORE_GROUP);
        // `movs.w lr,#1` / `movs.w lr,#1`
        let entry = decode32(0x1000_B23E, 0xF05F_0E01);
        assert_eq!(entry.group, group32::MODIFIED_IMMEDIATE);
        // `mrs r0,PRIMASK` / `mrs r0,PRIMASK`
        let entry = decode32(0x1000_B234, 0xF3EF_8010);
        assert_eq!(entry.group, group32::BRANCH_MISC);
        // FPU/coprocessor prefixes stay on the slow path.
        // FPU/协处理器前缀保持慢路径。
        assert_eq!(decode32(0x1000_B234, 0xEEF1_0A10).kind, Kind::Slow32);
        assert_eq!(decode32(0x1000_B234, 0xFE00_0000).kind, Kind::Slow32);
        // Undefined integer encodings stay on the slow path so the undefined
        // instruction report keeps its original text.
        // 未定义的整数编码保持慢路径，使未定义指令报告保持原文。
        assert_eq!(decode32(0x1000_B234, 0xEF00_0000).kind, Kind::Slow32);
    }

    #[test]
    fn host_flash_write_flushes_the_table() {
        use rp2350_core::flash::{FlashHandle, FlashRegion};

        let flash = FlashRegion::new("flash", FLASH_XIP_BASE, 0x1000);
        let handle: FlashHandle = flash.handle();
        let mut cache = DecodeCache::new();
        cache.store(decode16(FLASH_XIP_BASE, 0xE7FE));
        assert!(cache.lookup(FLASH_XIP_BASE).is_some());
        let before = cache.generation_flushes();
        handle.write(FLASH_XIP_BASE, &[0x00, 0xBF]).unwrap();
        assert!(cache.lookup(FLASH_XIP_BASE).is_none());
        assert!(cache.generation_flushes() > before);
    }

    #[test]
    fn conflicting_pcs_replace_each_other() {
        let mut cache = DecodeCache::new();
        let a = FLASH_XIP_BASE;
        let b = FLASH_XIP_BASE + (DECODE_CACHE_ENTRIES as u32 * 2);
        cache.store(decode16(a, 0xE7FE));
        cache.store(decode16(b, 0xE7FE));
        assert!(cache.lookup(a).is_none());
        assert!(cache.lookup(b).is_some());
    }

    #[test]
    fn kind_tags_round_trip_through_their_byte() {
        for kind in [
            Kind::Slow16,
            Kind::Slow32,
            Kind::B16,
            Kind::BCond16,
            Kind::LdrLiteral16,
            Kind::LdrWordImm5,
            Kind::StrWordImm5,
            Kind::DpRegister16,
            Kind::Hint16,
            Kind::Cps16,
            Kind::Group32,
        ] {
            assert_eq!(Kind::from_u8(kind as u8), kind);
        }
        assert_eq!(Kind::from_u8(200), Kind::Empty);
    }
}
