//! Simple RAM region and flat address bus used by tests and the boot path
//! 供测试与启动路径使用的简易 RAM 区域和扁平地址总线

// 内存区域实现涉及 u32/usize 之间的地址换算，这是硬件仿真层的固有特性。
// The memory region implementation converts between u32 and usize for address
// arithmetic, which is inherent to the hardware emulation layer.
#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap
)]

use super::bus::{AddressBus, BusResult, MemoryRegion};
use super::error::{BusErrorKind, CoreError};
use super::types::Address;

/// A flat, byte-addressable RAM region backed by a heap buffer
/// 由堆缓冲区支持的扁平、字节可寻址 RAM 区域
#[derive(Debug, Clone)]
pub struct RamRegion {
    name: String,
    base: Address,
    writable: bool,
    data: Vec<u8>,
}

impl RamRegion {
    /// Create a zeroed, writable RAM region
    /// 创建一个清零的可写 RAM 区域
    ///
    /// The caller must ensure `base + size` stays within the 32-bit address space.
    /// 调用者需确保 `base + size` 不超出 32 位地址空间。
    #[must_use]
    pub fn new(name: impl Into<String>, base: Address, size: usize) -> Self {
        Self {
            name: name.into(),
            base,
            writable: true,
            data: vec![0; size],
        }
    }

    /// Create a zeroed, read-only region (used to model ROM)
    /// 创建一个清零的只读区域（用于模拟 ROM）
    ///
    /// The caller must ensure `base + size` stays within the 32-bit address space.
    /// 调用者需确保 `base + size` 不超出 32 位地址空间。
    #[must_use]
    pub fn new_rom(name: impl Into<String>, base: Address, size: usize) -> Self {
        Self {
            name: name.into(),
            base,
            writable: false,
            data: vec![0; size],
        }
    }

    /// Borrow the backing bytes / 借用底层字节
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    /// Readable/writable flag / 是否可写
    #[must_use]
    pub fn is_writable(&self) -> bool {
        self.writable
    }

    /// Translate an in-region address into a buffer offset
    /// 将区域内地址转换为缓冲区偏移
    fn offset(&self, addr: Address) -> BusResult<usize> {
        if !self.contains(addr) {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            });
        }
        Ok((addr - self.base) as usize)
    }

    /// Read one byte and account for one bus cycle, without watchpoint hooks
    /// 读取一个字节并记一次总线周期，不触发监视点钩子
    ///
    /// The wide accessors below build on this helper so a halfword or word read
    /// records exactly one read-watch hit instead of one per byte.
    /// 下面的宽访问器基于此函数，使半字/字读只记录一次读监视命中而不是每字节一次。
    fn read_byte(&self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        *cycles += 1;
        let offset = self.offset(addr)?;
        self.data.get(offset).copied().ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })
    }

    /// Copy `N` readable bytes at `addr`, or `None` when the span is not fully
    /// inside the buffer
    /// 复制 `addr` 处 `N` 个可读字节；跨度未完全落在缓冲区内时返回 `None`
    ///
    /// Wide accesses take one bounds check and one slice copy instead of one
    /// `Vec::get` per byte; the caller falls back to the byte-wise path when the
    /// span leaves the region so per-byte error semantics stay identical.
    /// 宽访问只做一次边界检查与一次切片拷贝，而不是每字节一次 `Vec::get`；
    /// 跨度越过区域时调用方回退到逐字节路径，逐字节错误语义保持不变。
    fn window<const N: usize>(&self, addr: Address) -> Option<[u8; N]> {
        let offset = addr.checked_sub(self.base)? as usize;
        let end = offset.checked_add(N)?;
        let slot = self.data.get(offset..end)?;
        let mut out = [0u8; N];
        out.copy_from_slice(slot);
        Some(out)
    }

    /// Borrow `N` writable bytes at `addr` / 借用 `addr` 处 `N` 个可写字节
    ///
    /// Returns `None` for a read-only region or a span that is not fully inside
    /// the buffer, so the caller can fall back to the per-byte path.
    /// 只读区域或跨度未完全落在缓冲区内时返回 `None`，调用方据此回退到逐字节路径。
    fn window_mut<const N: usize>(&mut self, addr: Address) -> Option<&mut [u8; N]> {
        if !self.writable {
            return None;
        }
        let offset = addr.checked_sub(self.base)? as usize;
        let end = offset.checked_add(N)?;
        let slot = self.data.get_mut(offset..end)?;
        slot.try_into().ok()
    }

    /// Store `bytes` one at a time through [`MemoryRegion::write_u8`]
    /// 通过 [`MemoryRegion::write_u8`] 逐字节存储
    ///
    /// This is the exact byte-wise default path, including one cycle, one
    /// write-watch report and one bounds check per byte.
    /// 这是与默认实现完全一致的逐字节路径：每字节一次周期、一次写监视上报和一次
    /// 边界检查。
    fn write_each(&mut self, addr: Address, bytes: &[u8], cycles: &mut u64) -> BusResult<()> {
        for (index, byte) in bytes.iter().enumerate() {
            self.write_u8(addr.wrapping_add(index as u32), *byte, cycles)?;
        }
        Ok(())
    }
}

impl MemoryRegion for RamRegion {
    fn name(&self) -> &str {
        &self.name
    }

    fn base(&self) -> Address {
        self.base
    }

    fn size(&self) -> usize {
        self.data.len()
    }

    fn read_u8(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        let value = self.read_byte(addr, cycles)?;
        // 诊断钩子：未安装读监视时只多一次原子 load。
        // Diagnostic hook: an unarmed read watch costs one atomic load.
        crate::watch::check_read(addr, 1);
        Ok(value)
    }

    fn read_u16(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        // 字节与周期开销与默认实现一致，但只记录一次宽度为 2 的命中。
        // Same bytes and cycle cost as the default, but one width-2 hit only.
        let value = if let Some(slot) = self.window::<2>(addr) {
            *cycles += 2;
            u16::from_le_bytes(slot)
        } else {
            let lo = u16::from(self.read_byte(addr, cycles)?);
            let hi = u16::from(self.read_byte(addr.wrapping_add(1), cycles)?);
            lo | (hi << 8)
        };
        crate::watch::check_read(addr, 2);
        Ok(value)
    }

    fn read_u32(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        let value = if let Some(slot) = self.window::<4>(addr) {
            *cycles += 4;
            u32::from_le_bytes(slot)
        } else {
            let mut value = 0u32;
            for index in 0..4u32 {
                let byte = u32::from(self.read_byte(addr.wrapping_add(index), cycles)?);
                value |= byte << (index * 8);
            }
            value
        };
        crate::watch::check_read(addr, 4);
        Ok(value)
    }

    /// A halfword store is one slice copy; a byte-wise fallback keeps the
    /// write-watch and error semantics
    /// 半字写入是一次切片拷贝；逐字节回退保留写监视与错误语义
    ///
    /// When a write watch is armed the per-byte path runs instead, because the
    /// environment diagnostic reports every stored byte individually.
    /// 已安装写监视时改走逐字节路径，因为环境变量诊断会逐个字节上报。
    fn write_u16(&mut self, addr: Address, value: u16, cycles: &mut u64) -> BusResult<()> {
        let bytes = value.to_le_bytes();
        if crate::watch::write_watch_armed() {
            return self.write_each(addr, &bytes, cycles);
        }
        match self.window_mut::<2>(addr) {
            Some(slot) => {
                slot.copy_from_slice(&bytes);
                *cycles += 2;
                Ok(())
            }
            None => self.write_each(addr, &bytes, cycles),
        }
    }

    fn write_u32(&mut self, addr: Address, value: u32, cycles: &mut u64) -> BusResult<()> {
        let bytes = value.to_le_bytes();
        if crate::watch::write_watch_armed() {
            return self.write_each(addr, &bytes, cycles);
        }
        match self.window_mut::<4>(addr) {
            Some(slot) => {
                slot.copy_from_slice(&bytes);
                *cycles += 4;
                Ok(())
            }
            None => self.write_each(addr, &bytes, cycles),
        }
    }

    fn write_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        *cycles += 1;
        crate::watch::check_write(addr, u32::from(value));
        if !self.writable {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::AccessDenied,
            });
        }
        let offset = self.offset(addr)?;
        let slot = self.data.get_mut(offset).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })?;
        *slot = value;
        Ok(())
    }
}

/// A flat address bus that routes accesses to registered regions in order
/// 按注册顺序将访问路由到各区域的扁平地址总线
///
/// Routing is the hottest operation in the interpreter: every instruction fetch
/// and every data access resolves an address to a region. A direct-mapped table
/// keyed by the top address byte makes the steady state a few comparisons
/// instead of a linear scan of dynamic objects. A bucket that holds more than
/// one region falls back to the scan, so the table never changes which region
/// serves an address.
/// 路由是解释器中最热的操作：每次取指与每次数据访问都要把地址解析到区域。按地址
/// 最高字节索引的直接映射表把稳态查找从「遍历动态对象」降到几次比较。同一桶内
/// 存在多个区域时回退到线性扫描，因此该表绝不改变地址由哪个区域响应。
pub struct SimpleBus {
    regions: Vec<Box<dyn MemoryRegion>>,
    buckets: [RegionBucket; REGION_BUCKETS],
}

/// Number of region-lookup buckets covering the 32-bit address space
/// 覆盖 32 位地址空间的区域查找桶数量
///
/// One bucket covers 16 MiB, matching this chip's map: flash fills a whole
/// bucket, `SRAM`/`APB`/`PPB` each live inside one.
/// 每个桶覆盖 16 MiB，与本芯片的映射一致：Flash 占满整桶，`SRAM`/`APB`/`PPB`
/// 各落在单个桶内。
pub const REGION_BUCKETS: usize = 256;

/// Sentinel marking an empty bucket / 空桶标记
const NO_REGION: u16 = u16::MAX;

/// One cached region span / 一个缓存的区域跨度
#[derive(Clone, Copy)]
struct RegionBucket {
    /// First address served by the bucket / 桶服务的首个地址
    start: Address,
    /// End address (exclusive) / 结束地址（不含）
    end: Address,
    /// Index into [`SimpleBus::regions`] / [`SimpleBus::regions`] 下标
    index: u16,
}

impl Default for RegionBucket {
    fn default() -> Self {
        Self {
            start: 0,
            end: 0,
            index: NO_REGION,
        }
    }
}

impl SimpleBus {
    /// Create an empty bus / 创建一个空总线
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of registered regions / 已注册区域数量
    #[must_use]
    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    /// Drop every cached route after the region list changes
    /// 区域列表变化后丢弃全部缓存路由
    fn invalidate_buckets(&mut self) {
        self.buckets = [RegionBucket::default(); REGION_BUCKETS];
    }

    /// Resolve `addr` through the bucket table, if the cached span still covers it
    /// 若缓存跨度仍覆盖该地址则经桶表解析
    #[inline]
    fn cached_region(&self, addr: Address) -> Option<usize> {
        let bucket = &self.buckets[(addr >> 24) as usize];
        if addr >= bucket.start && addr < bucket.end {
            return Some(usize::from(bucket.index));
        }
        None
    }
}

impl Default for SimpleBus {
    fn default() -> Self {
        Self {
            regions: Vec::new(),
            buckets: [RegionBucket::default(); REGION_BUCKETS],
        }
    }
}

impl AddressBus for SimpleBus {
    fn register_region(&mut self, region: Box<dyn MemoryRegion>) -> BusResult<()> {
        let overlaps = self
            .regions
            .iter()
            .any(|existing| region.base() < existing.end() && existing.base() < region.end());
        if overlaps {
            return Err(CoreError::BusError {
                addr: region.base(),
                kind: BusErrorKind::Overlap,
            });
        }
        self.regions.push(region);
        self.invalidate_buckets();
        Ok(())
    }

    fn unregister_region(&mut self, name: &str) {
        self.regions.retain(|region| region.name() != name);
        self.invalidate_buckets();
    }

    fn find_region_mut(&mut self, addr: Address) -> Option<&mut dyn MemoryRegion> {
        let index = if let Some(index) = self.cached_region(addr) {
            // 缓存命中：索引在插入时已校验，直接取可变借用。
            // Cache hit: the index was validated when the bucket was filled.
            index
        } else {
            let mut found = None;
            for (index, region) in self.regions.iter().enumerate() {
                if region.contains(addr) {
                    found = Some((index, region.base(), region.end()));
                    break;
                }
            }
            let (index, start, end) = found?;
            // 桶内出现新跨度就直接覆盖：只有真正包含该地址的区域会被记入，
            // 同桶多区域时下一次访问会重新扫描并改正。
            // Overwrite the bucket with the span that just matched: only a
            // region that really contains the address is recorded, so a shared
            // bucket simply re-scans on the next lookup.
            self.buckets[(addr >> 24) as usize] = RegionBucket {
                start,
                end,
                index: u16::try_from(index).ok()?,
            };
            index
        };
        let region = self.regions.get_mut(index)?;
        Some(region.as_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_round_trip_byte() {
        let mut ram = RamRegion::new("ram", 0x2000_0000, 16);
        let mut cycles = 0;
        ram.write_u8(0x2000_0003, 0xA5, &mut cycles).unwrap();
        assert_eq!(ram.read_u8(0x2000_0003, &mut cycles).unwrap(), 0xA5);
        assert_eq!(ram.bytes()[3], 0xA5);
    }

    #[test]
    fn ram_word_round_trip_uses_byte_accessors() {
        let mut ram = RamRegion::new("ram", 0x2000_0000, 16);
        let mut cycles = 0;
        ram.write_u32(0x2000_0004, 0xDEAD_BEEF, &mut cycles)
            .unwrap();
        assert_eq!(ram.read_u32(0x2000_0004, &mut cycles).unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn ram_rejects_out_of_range() {
        let mut ram = RamRegion::new("ram", 0x2000_0000, 16);
        let mut cycles = 0;
        assert!(ram.read_u8(0x2000_0010, &mut cycles).is_err());
    }

    #[test]
    fn ram_wide_writes_land_at_the_last_full_span() {
        let mut ram = RamRegion::new("ram", 0x2000_0000, 8);
        let mut cycles = 0;
        ram.write_u16(0x2000_0006, 0xBEEF, &mut cycles).unwrap();
        ram.write_u32(0x2000_0000, 0x1122_3344, &mut cycles)
            .unwrap();
        assert_eq!(ram.bytes(), &[0x44, 0x33, 0x22, 0x11, 0, 0, 0xEF, 0xBE]);
        // 两次宽写各记满宽度个周期。/ Each wide store books its full width.
        assert_eq!(cycles, 6);
        assert_eq!(ram.read_u32(0x2000_0000, &mut cycles).unwrap(), 0x1122_3344);
        assert_eq!(ram.read_u16(0x2000_0006, &mut cycles).unwrap(), 0xBEEF);
        assert_eq!(cycles, 12);
    }

    #[test]
    fn ram_wide_write_past_the_end_matches_the_byte_path() {
        // 默认实现先写能写下的字节，再在越界处报 NoDevice；快速路径必须一致。
        // The default stores the bytes that fit, then reports NoDevice; the fast
        // path must behave identically.
        let mut ram = RamRegion::new("ram", 0x2000_0000, 4);
        let mut cycles = 0;
        let err = ram.write_u16(0x2000_0003, 0x1122, &mut cycles).unwrap_err();
        match err {
            CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::NoDevice),
            other => panic!("unexpected error: {other:?}"),
        }
        assert_eq!(ram.bytes(), &[0, 0, 0, 0x22]);
        // 与逐字节路径一致：两个 `write_u8` 都先记周期，第二个随后越界。
        // Matches the byte path: both `write_u8` calls book a cycle first, the
        // second then reports the out-of-range address.
        assert_eq!(cycles, 2);
    }

    #[test]
    fn ram_wide_read_past_the_end_reports_no_device() {
        let mut ram = RamRegion::new("ram", 0x2000_0000, 4);
        let mut cycles = 0;
        let err = ram.read_u32(0x2000_0002, &mut cycles).unwrap_err();
        match err {
            CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::NoDevice),
            other => panic!("unexpected error: {other:?}"),
        }
        // 与逐字节路径一致：前两个字节成功，第三次字节读在记周期后越界。
        // Matches the byte path: two bytes succeed, the third byte read books a
        // cycle and then reports the out-of-range address.
        assert_eq!(cycles, 3);
    }

    #[test]
    fn rom_wide_write_is_denied_and_changes_nothing() {
        let mut rom = RamRegion::new_rom("rom", 0x0000_0000, 8);
        let mut cycles = 0;
        let err = rom
            .write_u32(0x0000_0000, 0xFFFF_FFFF, &mut cycles)
            .unwrap_err();
        match err {
            CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::AccessDenied),
            other => panic!("unexpected error: {other:?}"),
        }
        assert_eq!(rom.bytes(), &[0; 8]);
    }

    #[test]
    fn wide_writes_keep_the_byte_wise_path_while_a_write_watch_is_armed() {
        // 写监视开启时改走逐字节路径，存储结果与周期数不变。
        // With a write watch armed the byte-wise path runs; the stored value and
        // the cycle count stay the same.
        let mut ram = RamRegion::new("ram", 0x2000_0000, 16);
        crate::watch::set_write_watch(Some((0x2000_0000, 0x2000_0010)));
        assert!(crate::watch::write_watch_armed());
        let mut cycles = 0;
        ram.write_u32(0x2000_0004, 0x1122_3344, &mut cycles)
            .unwrap();
        ram.write_u16(0x2000_0008, 0xAABB, &mut cycles).unwrap();
        crate::watch::set_write_watch(None);
        assert!(!crate::watch::write_watch_armed());
        assert_eq!(&ram.bytes()[4..10], &[0x44, 0x33, 0x22, 0x11, 0xBB, 0xAA]);
        assert_eq!(cycles, 6);
    }

    #[test]
    fn read_watch_records_one_hit_per_access_with_its_width() {
        let mut ram = RamRegion::new("ram", 0x2000_0000, 16);
        let mut cycles = 0;
        crate::watch::set_read_watch(Some((0x2000_0000, 0x2000_0010)));

        ram.read_u8(0x2000_0001, &mut cycles).unwrap();
        assert_eq!(crate::watch::take_read_hit(), Some((0x2000_0001, 1)));
        ram.read_u16(0x2000_0002, &mut cycles).unwrap();
        assert_eq!(crate::watch::take_read_hit(), Some((0x2000_0002, 2)));
        ram.read_u32(0x2000_0004, &mut cycles).unwrap();
        assert_eq!(crate::watch::take_read_hit(), Some((0x2000_0004, 4)));
        // 未取走前有新的读命中时只保留最新一次。
        // A newer hit replaces an unconsumed one.
        ram.read_u8(0x2000_0000, &mut cycles).unwrap();
        ram.read_u8(0x2000_0003, &mut cycles).unwrap();
        assert_eq!(crate::watch::take_read_hit(), Some((0x2000_0003, 1)));

        crate::watch::set_read_watch(None);
        assert_eq!(cycles, 9, "byte cycles must match the default path");
    }

    #[test]
    fn rom_rejects_writes() {
        let mut rom = RamRegion::new_rom("rom", 0x0000_0000, 16);
        let mut cycles = 0;
        let err = rom.write_u8(0x0000_0000, 1, &mut cycles).unwrap_err();
        match err {
            CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::AccessDenied),
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(!rom.is_writable());
    }

    #[test]
    fn bus_routes_to_region() {
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(RamRegion::new("ram", 0x2000_0000, 64)))
            .unwrap();
        let mut cycles = 0;
        bus.write_u32(0x2000_0000, 0x1234_5678, &mut cycles)
            .unwrap();
        assert_eq!(bus.read_u32(0x2000_0000, &mut cycles).unwrap(), 0x1234_5678);
    }

    #[test]
    fn bus_reports_unmapped_address() {
        let mut bus = SimpleBus::new();
        let mut cycles = 0;
        let err = bus.read_u8(0x4000_0000, &mut cycles).unwrap_err();
        match err {
            CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::NoDevice),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn bus_rejects_overlapping_regions() {
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(RamRegion::new("a", 0x2000_0000, 64)))
            .unwrap();
        let err = bus
            .register_region(Box::new(RamRegion::new("b", 0x2000_0010, 64)))
            .unwrap_err();
        match err {
            CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::Overlap),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn bus_unregister_by_name() {
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(RamRegion::new("ram", 0x2000_0000, 64)))
            .unwrap();
        assert_eq!(bus.region_count(), 1);
        bus.unregister_region("ram");
        assert_eq!(bus.region_count(), 0);
    }

    #[test]
    fn bus_routes_regions_that_share_one_bucket() {
        // 0x2000_0000 与 0x20F0_0000 落在同一个 16 MiB 桶：缓存必须回退到扫描，
        // 两次访问都要落到各自的区域。
        // Both live in the same 16 MiB bucket: the cache must fall back to the
        // scan and each access must still reach its own region.
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(RamRegion::new("low", 0x2000_0000, 0x100)))
            .unwrap();
        bus.register_region(Box::new(RamRegion::new("high", 0x20F0_0000, 0x100)))
            .unwrap();
        let mut cycles = 0;
        for _ in 0..4 {
            bus.write_u32(0x2000_0010, 0x1111_1111, &mut cycles)
                .unwrap();
            bus.write_u32(0x20F0_0010, 0x2222_2222, &mut cycles)
                .unwrap();
        }
        assert_eq!(bus.read_u32(0x2000_0010, &mut cycles).unwrap(), 0x1111_1111);
        assert_eq!(bus.read_u32(0x20F0_0010, &mut cycles).unwrap(), 0x2222_2222);
    }

    #[test]
    fn bus_cache_does_not_survive_unregister() {
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(RamRegion::new("ram", 0x2000_0000, 0x100)))
            .unwrap();
        let mut cycles = 0;
        bus.write_u32(0x2000_0000, 0x1234_5678, &mut cycles)
            .unwrap();
        assert_eq!(bus.read_u32(0x2000_0000, &mut cycles).unwrap(), 0x1234_5678);
        bus.unregister_region("ram");
        // 撤除后同一地址必须报 NoDevice，而不是命中陈旧缓存。
        // After removal the same address must report NoDevice, not a stale hit.
        let err = bus.read_u32(0x2000_0000, &mut cycles).unwrap_err();
        match err {
            CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::NoDevice),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn bus_cache_picks_up_a_newly_registered_region() {
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(RamRegion::new("first", 0x2000_0000, 0x100)))
            .unwrap();
        let mut cycles = 0;
        let err = bus.read_u8(0x3000_0000, &mut cycles).unwrap_err();
        match err {
            CoreError::BusError { kind, .. } => assert_eq!(kind, BusErrorKind::NoDevice),
            other => panic!("unexpected error: {other:?}"),
        }
        bus.register_region(Box::new(RamRegion::new("second", 0x3000_0000, 0x100)))
            .unwrap();
        bus.write_u8(0x3000_0004, 0x5A, &mut cycles).unwrap();
        assert_eq!(bus.read_u8(0x3000_0004, &mut cycles).unwrap(), 0x5A);
    }
}
