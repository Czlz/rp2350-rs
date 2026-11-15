//! Instruction-fetch cache for the read-only flash XIP windows
//! 只读 Flash XIP 窗口的取指缓存
//!
//! Every instruction fetch otherwise resolves an address through the dynamic
//! `AddressBus` trait object and takes the flash shared-store lock. Both costs
//! are avoidable: flashes never accept guest writes, and the only host-side
//! mutations publish a generation counter ([`rp2350_core::flash::content_generation`]).
//! A direct-mapped table keyed by `PC` therefore serves repeated fetches with a
//! tag compare plus one relaxed load.
//! 每次取指都要经动态 `AddressBus` 解析地址并获取 Flash 共享存储读锁。这两项都能
//! 省掉：Flash 从不接受客户机写入，而宿主侧变更会发布世代号
//! （[`rp2350_core::flash::content_generation`]）。因此按 `PC` 索引的直接映射表
//! 只需一次标签比较加一次 relaxed 读取即可服务重复取指。
//!
//! Consistency / 一致性：
//!
//! * Only addresses inside the two flash XIP windows are cached, so a cache hit
//!   can never bypass an MMIO side effect or a writable region.
//!   只缓存两个 Flash XIP 窗口内的地址，因此命中绝不会绕过 MMIO 副作用或可写区域。
//! * Guest writes to flash are rejected, so a guest cannot create self-modifying
//!   code there. SRAM stays uncached, keeping self-modifying SRAM code correct.
//!   客户机写 Flash 会被拒绝，因此客户机无法在 Flash 中生成自修改代码。SRAM 不入
//!   缓存，自修改 SRAM 代码语义不变。
//! * Every host-side flash mutation bumps the generation, and a mismatch clears
//!   the whole table before the next lookup.
//!   每次宿主侧 Flash 变更都会递增世代号，失配时在下一次查找前清空整表。

use rp2350_core::flash::content_generation;
use rp2350_core::{FLASH_XIP_BASE, FLASH_XIP_NOCACHE_BASE, FLASH_XIP_SIZE};

/// Number of direct-mapped fetch cache entries / 直接映射取指缓存条目数
///
/// 512 entries cover a whole hot loop plus its callees; each entry is eight
/// bytes, so the table fits comfortably in L1.
/// 512 条足以覆盖整个热循环及其被调用者；每条 8 字节，整表可轻松驻留一级缓存。
const FETCH_CACHE_ENTRIES: usize = 512;

/// Index mask for the direct-mapped table / 直接映射表的索引掩码
const FETCH_CACHE_MASK: usize = FETCH_CACHE_ENTRIES - 1;

/// Size of one flash XIP window as `u32` / 单个 Flash XIP 窗口大小（u32）
#[allow(clippy::cast_possible_truncation)]
const WINDOW_SIZE: u32 = FLASH_XIP_SIZE as u32;

/// One cached halfword / 一个缓存的半字
#[derive(Clone, Copy, Default)]
struct FetchEntry {
    /// Instruction address the entry was filled from / 填充该条目的取指地址
    tag: u32,
    /// Cached halfword / 缓存的半字
    half: u16,
}

/// Whether `addr` lies in a flash XIP window / `addr` 是否落在 Flash XIP 窗口内
#[must_use]
pub fn in_flash_window(addr: u32) -> bool {
    addr.wrapping_sub(FLASH_XIP_BASE) < WINDOW_SIZE
        || addr.wrapping_sub(FLASH_XIP_NOCACHE_BASE) < WINDOW_SIZE
}

/// Direct-mapped instruction fetch cache / 直接映射取指缓存
pub struct FetchCache {
    /// Cached halfwords / 缓存的半字
    entries: Box<[FetchEntry; FETCH_CACHE_ENTRIES]>,
    /// Flash generation the table was filled under / 填充该表时的 Flash 世代号
    generation: u32,
    /// Generation flushes performed so far / 已执行的世代清空次数
    clears: u32,
}

impl FetchCache {
    /// Create an empty cache / 创建空缓存
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Box::new([FetchEntry::default(); FETCH_CACHE_ENTRIES]),
            // 构造时把世代号置为当前值，条目全为零标签不会命中真实地址 0。
            // Seed with the current generation; the zero tags cannot match a
            // real flash address.
            generation: content_generation(),
            clears: 0,
        }
    }

    /// Drop every entry after a generation change / 世代变化后清空全部条目
    #[inline]
    fn refresh(&mut self, generation: u32) {
        if self.generation != generation {
            self.generation = generation;
            self.entries.fill(FetchEntry::default());
            self.clears = self.clears.wrapping_add(1);
        }
    }

    /// Look up the halfword at `pc` / 查找 `pc` 处的半字
    ///
    /// Returns `None` for addresses outside the flash windows or for a miss.
    /// 落在 Flash 窗口之外或未命中时返回 `None`。
    #[inline]
    pub fn lookup(&mut self, pc: u32) -> Option<u16> {
        if !in_flash_window(pc) {
            return None;
        }
        self.refresh(content_generation());
        let entry = &self.entries[((pc >> 1) as usize) & FETCH_CACHE_MASK];
        // 空条目的标签为 0，而 0 不在任何 Flash 窗口内，因此无需额外有效位。
        // Empty entries carry tag 0, which is outside both flash windows, so no
        // separate valid bit is needed.
        if entry.tag == pc {
            return Some(entry.half);
        }
        None
    }

    /// Store the halfword fetched at `pc` / 存入 `pc` 处取到的半字
    ///
    /// Entries are only filled for flash addresses, and a fill that races a
    /// host-side mutation is resolved by the next generation check.
    /// 只对 Flash 地址填充；若填充期间发生宿主侧变更，下一次世代检查会纠正。
    #[inline]
    pub fn store(&mut self, pc: u32, half: u16) {
        if !in_flash_window(pc) {
            return;
        }
        // 世代检查同样适用于填充：若填充发生在宿主改写之后，旧条目必须先失效，
        // 否则下一次查找会清掉刚刚填入的值。
        // The generation check applies to fills too: a fill after a host rewrite
        // must not leave an entry under the stale generation.
        self.refresh(content_generation());
        self.entries[((pc >> 1) as usize) & FETCH_CACHE_MASK] = FetchEntry { tag: pc, half };
    }

    /// Number of generation flushes since construction / 自构造以来的世代清空次数
    #[cfg(test)]
    #[must_use]
    pub fn generation_flushes(&self) -> u32 {
        self.clears
    }
}

impl Default for FetchCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::flash_generation_lock;
    use rp2350_core::bus::MemoryRegion;
    use rp2350_core::flash::{FlashHandle, FlashRegion};

    // These tests observe the process-global flash generation, and two of them
    // bump it through a host handle, so they must not overlap with any other
    // flash-touching test (see `crate::test_support`).
    // 这些测试观察进程级 Flash 世代号，其中两个还经宿主句柄递增它，因此不得与
    // 其它接触 Flash 的测试重叠（见 `crate::test_support`）。

    #[test]
    fn flash_halfwords_round_trip_and_sram_is_never_cached() {
        let _generation = flash_generation_lock();
        let mut cache = FetchCache::new();
        assert_eq!(cache.lookup(FLASH_XIP_BASE), None);
        cache.store(FLASH_XIP_BASE, 0xB672);
        assert_eq!(cache.lookup(FLASH_XIP_BASE), Some(0xB672));
        // 非 Flash 地址既不命中也不填充。
        // Non-flash addresses neither hit nor fill.
        assert_eq!(cache.lookup(0x2000_0010), None);
        cache.store(0x2000_0010, 0x1234);
        assert_eq!(cache.lookup(0x2000_0010), None);
    }

    #[test]
    fn nocache_alias_and_cached_window_are_both_cached() {
        let _generation = flash_generation_lock();
        let mut cache = FetchCache::new();
        // 两个窗口偏移 0x100 与 0x102 映射到不同的表项（同偏移会因别名差值
        // 是 1 KiB 的倍数而落在同一索引上）。
        // Offsets 0x100 and 0x102 map to different entries (the same offset would
        // share an index because the alias delta is a multiple of 1 KiB).
        cache.store(FLASH_XIP_BASE + 0x100, 0x4770);
        cache.store(FLASH_XIP_NOCACHE_BASE + 0x102, 0x4771);
        assert_eq!(cache.lookup(FLASH_XIP_BASE + 0x100), Some(0x4770));
        assert_eq!(cache.lookup(FLASH_XIP_NOCACHE_BASE + 0x102), Some(0x4771));
        // 窗口之后一个字节就不再是 Flash。
        // One byte past the window is no longer flash.
        assert_eq!(cache.lookup(FLASH_XIP_BASE + WINDOW_SIZE), None);
    }

    #[test]
    fn a_host_flash_write_invalidates_the_cached_halfword() {
        let _generation = flash_generation_lock();
        let mut flash = FlashRegion::new("flash", FLASH_XIP_BASE, 0x1000);
        let handle: FlashHandle = flash.handle();
        let mut cache = FetchCache::new();
        handle.write(FLASH_XIP_BASE + 4, &[0x11, 0x22]).unwrap();
        let mut cycles = 0;
        let value = flash.read_u16(FLASH_XIP_BASE + 4, &mut cycles).unwrap();
        cache.store(FLASH_XIP_BASE + 4, value);
        assert_eq!(cache.lookup(FLASH_XIP_BASE + 4), Some(0x2211));
        let before = cache.generation_flushes();
        handle.write(FLASH_XIP_BASE + 4, &[0x33, 0x44]).unwrap();
        assert_eq!(cache.lookup(FLASH_XIP_BASE + 4), None);
        assert!(cache.generation_flushes() > before);
        let value = flash.read_u16(FLASH_XIP_BASE + 4, &mut cycles).unwrap();
        cache.store(FLASH_XIP_BASE + 4, value);
        assert_eq!(cache.lookup(FLASH_XIP_BASE + 4), Some(0x4433));
    }

    #[test]
    fn erase_also_invalidates() {
        let _generation = flash_generation_lock();
        let flash = FlashRegion::new("flash", FLASH_XIP_BASE, 0x1000);
        let handle = flash.handle();
        let mut cache = FetchCache::new();
        handle.write(FLASH_XIP_BASE, &[1, 2]).unwrap();
        cache.store(FLASH_XIP_BASE, 0x0201);
        assert_eq!(cache.lookup(FLASH_XIP_BASE), Some(0x0201));
        handle.erase(FLASH_XIP_BASE, 2).unwrap();
        assert_eq!(cache.lookup(FLASH_XIP_BASE), None);
    }
}
