//! Flat single-threaded address bus / 扁平单线程地址总线
//!
//! Routing is the hottest operation in the interpreter: every instruction fetch
//! and every data access resolves an address to a region. A direct-mapped table
//! keyed by the top address byte makes the steady state a few comparisons
//! instead of a linear scan of dynamic objects. A bucket that holds more than
//! one region falls back to the scan, so the table never changes which region
//! serves an address.
//! 路由是解释器中最热的操作：每次取指与每次数据访问都要把地址解析到区域。按地址
//! 最高字节索引的直接映射表把稳态查找从「遍历动态对象」降到几次比较。同一桶内
//! 存在多个区域时回退到线性扫描，因此该表绝不改变地址由哪个区域响应。
//!
//! This is the default path. The thread-safe counterpart lives in
//! [`crate::shared::SharedBus`]; it uses the same bucket scheme with atomic tags.
//! 这是默认路径。线程安全的对应物在 [`crate::shared::SharedBus`]，它使用相同的桶
//! 方案，但标签是原子量。

// 偏移到地址的换算涉及 usize→u32，这是内存映射层的固有特性。
// The offset-to-address conversion involves usize→u32, which is inherent to the
// memory-mapped layer.
#![allow(clippy::cast_possible_truncation)]

use crate::bus::{AddressBus, BusResult, MemoryRegion};
use crate::error::{BusErrorKind, CoreError};
use crate::types::Address;

/// Base of the ARMv8-M Private Peripheral Bus window / ARMv8-M 私有外设总线窗口基址
pub(crate) const PPB_WINDOW_BASE: Address = 0xE000_0000;

/// Size of the ARMv8-M Private Peripheral Bus window / ARMv8-M 私有外设总线窗口大小
pub(crate) const PPB_WINDOW_SIZE: Address = 0x0010_0000;

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

/// A flat address bus that routes accesses to registered regions in order
/// 按注册顺序将访问路由到各区域的扁平地址总线
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
