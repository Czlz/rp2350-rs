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

    /// Borrow `len` writable bytes at `addr` / 借用 `addr` 处 `len` 个可写字节
    ///
    /// The variable-length form of [`Self::window_mut`], used by the host-side
    /// bulk loader. Returns `None` for a read-only region or a span that is not
    /// fully inside the buffer.
    /// [`Self::window_mut`] 的变长版本，供宿主侧批量加载使用。只读区域或跨度未
    /// 完全落在缓冲区内时返回 `None`。
    fn span_mut(&mut self, addr: Address, len: usize) -> Option<&mut [u8]> {
        if !self.writable {
            return None;
        }
        let offset = addr.checked_sub(self.base)? as usize;
        let end = offset.checked_add(len)?;
        self.data.get_mut(offset..end)
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

    /// A host-side bulk load is one bounds check and one copy
    /// 宿主侧批量加载只需一次边界检查与一次拷贝
    ///
    /// 逐字节回退保留写监视与只读/越界错误语义：安装写监视时诊断必须逐字节
    /// 上报，只读区域与非整块跨度也必须逐字节报错。
    /// The per-byte fallback keeps write-watch and read-only/out-of-range
    /// semantics: with a write watch armed the diagnostic reports every stored
    /// byte, and a read-only region or a span that leaves the buffer must still
    /// fail byte by byte.
    fn load_bytes(&mut self, addr: Address, bytes: &[u8], cycles: &mut u64) -> BusResult<()> {
        if crate::watch::write_watch_armed() || !self.writable {
            for (offset, byte) in bytes.iter().enumerate() {
                self.load_u8(addr.wrapping_add(offset as u32), *byte, cycles)?;
            }
            return Ok(());
        }
        if let Some(slot) = self.span_mut(addr, bytes.len()) {
            slot.copy_from_slice(bytes);
            *cycles += bytes.len() as u64;
            return Ok(());
        }
        for (offset, byte) in bytes.iter().enumerate() {
            self.load_u8(addr.wrapping_add(offset as u32), *byte, cycles)?;
        }
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
///
/// The bus also counts guest writes into the architectural PPB window
/// (`0xE000_0000..0xE010_0000`). Firmware writes there are replayed into the
/// core's system-register file by the frontend, so a caller can skip the drain
/// entirely while the counter is unchanged.
/// 总线还统计客户机对架构 PPB 窗口（`0xE000_0000..0xE010_0000`）的写入。前端会把
/// 这些写回放到核的系统寄存器组，因此计数未变时调用方可完全跳过回放。
pub struct SimpleBus {
    regions: Vec<Box<dyn MemoryRegion>>,
    buckets: [RegionBucket; REGION_BUCKETS],
    ppb_write_epoch: u64,
}

/// Base of the ARMv8-M Private Peripheral Bus window / ARMv8-M 私有外设总线窗口基址
const PPB_WINDOW_BASE: Address = 0xE000_0000;

/// Size of the ARMv8-M Private Peripheral Bus window / ARMv8-M 私有外设总线窗口大小
const PPB_WINDOW_SIZE: Address = 0x0010_0000;

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

    /// Number of guest writes into the PPB window since machine construction
    /// 自机器构造以来客户机对 PPB 窗口的写入次数
    ///
    /// The counter is monotonic and only advances for PPB addresses, which are
    /// the only source of core system-register replay entries. A frontend can
    /// therefore compare it with its own snapshot and skip the drain (and its
    /// lock) whenever nothing could have been queued.
    /// 该计数器单调递增且只在 PPB 地址上推进，而 PPB 是核系统寄存器回放条目的唯一
    /// 来源。因此前端可以用它和自身快照比较：没有新写入时直接跳过回放（及其锁）。
    #[must_use]
    pub fn ppb_write_epoch(&self) -> u64 {
        self.ppb_write_epoch
    }

    /// Count a write that lands in the PPB window / 统计落入 PPB 窗口的写
    #[inline]
    fn note_write(&mut self, addr: Address) {
        if addr.wrapping_sub(PPB_WINDOW_BASE) < PPB_WINDOW_SIZE {
            self.ppb_write_epoch = self.ppb_write_epoch.wrapping_add(1);
        }
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
            ppb_write_epoch: 0,
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

    /// Byte write that also counts PPB traffic / 同时统计 PPB 流量的字节写
    ///
    /// The body is the trait default; only the PPB epoch bookkeeping is added,
    /// so routing, error and watchpoint semantics are unchanged.
    /// 函数体即 trait 默认实现，只增加 PPB 世代记账，因此路由、错误与监视点语义
    /// 完全不变。
    ///
    /// # Errors
    /// Returns `BusError` if no device is mapped at the given address.
    /// 如果给定地址没有映射设备，返回 `BusError`。
    fn write_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        self.note_write(addr);
        self.find_region_mut(addr)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .write_u8(addr, value, cycles)
    }

    /// Halfword write that also counts PPB traffic / 同时统计 PPB 流量的半字写
    ///
    /// # Errors
    /// Returns `BusError` if no device is mapped at the given address.
    /// 如果给定地址没有映射设备，返回 `BusError`。
    fn write_u16(&mut self, addr: Address, value: u16, cycles: &mut u64) -> BusResult<()> {
        self.note_write(addr);
        self.find_region_mut(addr)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .write_u16(addr, value, cycles)
    }

    /// Word write that also counts PPB traffic / 同时统计 PPB 流量的字写
    ///
    /// # Errors
    /// Returns `BusError` if no device is mapped at the given address.
    /// 如果给定地址没有映射设备，返回 `BusError`。
    fn write_u32(&mut self, addr: Address, value: u32, cycles: &mut u64) -> BusResult<()> {
        crate::watch::check_write(addr, value);
        self.note_write(addr);
        self.find_region_mut(addr)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .write_u32(addr, value, cycles)
    }

    /// Host-side byte write that also counts PPB traffic
    /// 同时统计 PPB 流量的宿主侧字节写
    ///
    /// # Errors
    /// Returns `BusError` if no device is mapped at the given address.
    /// 如果给定地址没有映射设备，返回 `BusError`。
    fn load_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        self.note_write(addr);
        self.find_region_mut(addr)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .load_u8(addr, value, cycles)
    }

    /// Host-side bulk write, routed once for the whole span
    /// 宿主侧批量写入，整段只路由一次
    ///
    /// Firmware loading writes megabytes through the host channel. Resolving the
    /// region once and handing it the slice turns that into one bucket lookup
    /// plus one region transfer per contiguous run. The PPB write epoch is still
    /// advanced once per byte, because that counter records guest-visible PPB
    /// traffic and must not change meaning; a bulk run never targets the PPB
    /// window in practice.
    /// 固件加载经宿主通道写入数 MB。整段只解析一次区域并把切片交给它，就把这段
    /// 开销降为一次桶查找加一次区域传输。PPB 写世代仍按字节推进，因为该计数器
    /// 记录客户机可见的 PPB 流量、语义不得改变；实际批量写入从不指向 PPB 窗口。
    ///
    /// # Errors
    /// Returns `BusError` if no device is mapped at the given address.
    /// 如果给定地址没有映射设备，返回 `BusError`。
    fn load_bytes(&mut self, addr: Address, bytes: &[u8], cycles: &mut u64) -> BusResult<()> {
        // 空写入与逐字节路径一致：不路由、不校验映射。
        // An empty write matches the byte-wise path: no routing, no mapping check.
        if bytes.is_empty() {
            return Ok(());
        }
        // 只有整段可能与 PPB 窗口相交时才逐字节记账，语义与逐字节路径相同。
        // Only count per byte when the span can intersect the PPB window, which
        // is equivalent to the byte-wise path.
        let start = u64::from(addr);
        let end = start + bytes.len() as u64;
        if start < u64::from(PPB_WINDOW_BASE + PPB_WINDOW_SIZE) && end > u64::from(PPB_WINDOW_BASE)
        {
            for offset in 0..bytes.len() {
                self.note_write(addr.wrapping_add(offset as u32));
            }
        }
        self.find_region_mut(addr)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .load_bytes(addr, bytes, cycles)
    }
}

#[cfg(test)]
mod tests;
