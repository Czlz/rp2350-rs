//! MMIO router: one address span, one mutex per child device
//! MMIO 路由器：一个大地址跨度，每个子设备一把互斥锁
//!
//! Mounting happens once, before the bus is shared, and takes `&mut self`. After
//! that, the child list and the child windows are immutable. Each child sits
//! behind its own [`Mutex`], so a concurrent access to device A never waits for
//! device B. The 4 KiB page cache is an array of packed atomic tags, so the hot
//! poll path (a firmware repeatedly reading one register) stays a single atomic
//! load plus a window validation instead of a scan over every child.
//! 挂载只在总线被共享之前发生一次，取 `&mut self`。之后子设备列表与窗口不再改变。
//! 每个子设备位于自己的 [`Mutex`] 之后，因此对设备 A 的并发访问绝不会等待设备 B。
//! 4 KiB 页缓存是打包的原子标签数组，因此热轮询路径（固件反复读同一个寄存器）仍然
//! 只是一次原子加载加一次窗口校验，而不是遍历所有子设备。
//!
//! # Lock order / 锁顺序
//!
//! A router access takes at most one child-device mutex, and never takes a second
//! one while holding the first. The unmodelled-access warning set has its own
//! mutex that is only taken when an access misses every device and is never held
//! across a device lock. / 一次路由访问最多取一把子设备互斥锁，且绝不在持有时再取
//! 第二把。未建模访问的警告集合有独立互斥锁，只在访问未命中任何设备时获取，且绝不
//! 跨设备锁持有。

// 路由实现涉及地址、页号与寄存器偏移之间的定宽换算，这是内存映射层的固有特性。
// The router converts between addresses, page numbers and register offsets, which
// is inherent to the memory-mapped layer.
#![allow(
    clippy::cast_possible_truncation,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;

use super::{
    AliasOp, MMIO_ALIAS_CLR, MMIO_ALIAS_SET, MMIO_ALIAS_XOR, MMIO_BLOCK_SIZE, MmioDevice,
    MmioRegion, MmioWindow, warn_unhandled,
};
use crate::bus::BusResult;
use crate::bus::MemoryRegion;
use crate::error::{BusErrorKind, CoreError};
use crate::types::Address;

/// Number of router page-cache slots / 路由器页缓存槽数量
///
/// Each slot caches the child window that last resolved into its 4 KiB page, so a
/// firmware polling one register pays a tag compare instead of a scan over every
/// mounted device. The cache is direct-mapped and every hit is validated against
/// the cached child's window, so it can only ever return the device that really
/// owns the address.
/// 每个槽缓存最近解析到该 4 KiB 页的子设备窗口，因此固件反复轮询同一寄存器时只做
/// 一次标签比较，而不必遍历所有已挂载设备。缓存是直接映射的，每次命中都对照缓存的
/// 子设备窗口校验，因此只会返回真正拥有该地址的设备。
pub const MMIO_PAGE_SLOTS: usize = 64;

/// Empty page-cache slot / 空页缓存槽
const EMPTY_SLOT: u64 = 0;

/// Mask of the 12-bit in-page offset fields / 页内偏移字段的 12 位掩码
const PAGE_MASK: u64 = 0x0FFF;

/// Pack a child window into one 64-bit cache tag
/// 把一个子设备窗口打包进 64 位缓存标签
///
/// Layout: `tag: u16 | first: u12 @16 | last: u12 @28 | page: u24 @40`, where
/// `tag = index + 1` (zero means empty), `first`/`last` are the inclusive in-page
/// offsets of the child window, and `page` is `addr >> 12`. A mounted child window
/// always lies inside one 4 KiB page (mount rejects a window that crosses its
/// block's alias span), so the packed tag alone is a complete route: a hit costs
/// one atomic load and two comparisons, exactly like the single-threaded bus.
/// 布局：`tag: u16 | first: u12 @16 | last: u12 @28 | page: u24 @40`，其中
/// `tag = index + 1`（零表示空），`first`/`last` 是子设备窗口在页内的首尾闭区间
/// 偏移，`page` 为 `addr >> 12`。已挂载的子窗口总是落在同一个 4 KiB 页内（跨越
/// 所属块别名跨度的窗口在挂载时被拒绝），因此打包标签本身就是完整路由：命中只需
/// 一次原子加载加两次比较，与单线程总线完全一致。
#[inline]
fn pack(window: MmioWindow, index: usize) -> u64 {
    let page = u64::from(page_of(window.base));
    let first = u64::from(window.base & 0x0FFF);
    let last = u64::from(window.end().wrapping_sub(1) & 0x0FFF);
    (index as u64 + 1) | (first << 16) | (last << 28) | (page << 40)
}

/// Page number of `addr` / `addr` 的页号
#[inline]
fn page_of(addr: Address) -> Address {
    addr >> 12
}

/// Cache slot index for `addr` / `addr` 的缓存槽下标
#[inline]
fn page_slot(addr: Address) -> usize {
    (page_of(addr) as usize) & (MMIO_PAGE_SLOTS - 1)
}

/// One MMIO access resolved to a child device / 解析到某个子设备的一次 MMIO 访问
struct Resolved {
    /// Index of the owning child / 拥有该地址的子设备索引
    index: usize,
    /// Address handed to the child, alias offset already folded
    /// 交给子设备的地址，别名偏移已折叠
    reg_addr: Address,
    /// Alias operation, `None` for a direct register access
    /// 别名操作；直接访问寄存器时为 `None`
    op: Option<AliasOp>,
}

/// Alias span of the block that owns `base` / `base` 所属块的别名跨度
///
/// The alias windows cover `block + 0x1000 .. block + 0x4000`, where the block
/// base is `base` rounded down to [`MMIO_BLOCK_SIZE`].
/// 别名窗口覆盖 `block + 0x1000 .. block + 0x4000`，块基址为 `base` 向下按
/// [`MMIO_BLOCK_SIZE`] 对齐后的地址。
fn alias_span(base: Address) -> MmioWindow {
    let block = base & !(MMIO_BLOCK_SIZE - 1);
    MmioWindow::new(
        block + MMIO_ALIAS_XOR,
        (MMIO_BLOCK_SIZE - MMIO_ALIAS_XOR) as usize,
    )
}

/// A router that owns a large address span and dispatches to child devices
/// 拥有大地址跨度并向子设备分发的路由器
pub struct MmioRouter {
    name: String,
    window: MmioWindow,
    /// Child devices, one mutex each / 每个子设备一把互斥锁
    regions: Vec<Mutex<MmioRegion>>,
    /// Child windows, immutable after mounting / 子设备窗口，挂载后不再改变
    windows: Vec<MmioWindow>,
    /// Addresses already warned about / 已警告过的地址
    warned: Mutex<BTreeSet<u32>>,
    /// Packed page-cache tags / 打包的页缓存标签
    pages: [AtomicU64; MMIO_PAGE_SLOTS],
}

impl MmioRouter {
    /// Create an empty router / 创建空路由器
    #[must_use]
    pub fn new(name: impl Into<String>, window: MmioWindow) -> Self {
        Self {
            name: name.into(),
            window,
            regions: Vec::new(),
            windows: Vec::new(),
            warned: Mutex::new(BTreeSet::new()),
            pages: [const { AtomicU64::new(EMPTY_SLOT) }; MMIO_PAGE_SLOTS],
        }
    }

    /// Router window / 路由器窗口
    #[must_use]
    pub fn window(&self) -> MmioWindow {
        self.window
    }

    /// Number of mounted devices / 已挂载设备数量
    #[must_use]
    pub fn device_count(&self) -> usize {
        self.regions.len()
    }

    /// Mount a child device; its window must lie inside the router window and
    /// must not overlap an existing child or any atomic alias span.
    /// 挂载子设备；其窗口必须位于路由器窗口内，且不与已有子设备或任何原子别名
    /// 跨度重叠。
    ///
    /// # Errors
    /// Returns `BusError` with `Overlap` when two windows (including alias
    /// spans) intersect, or `DeviceError` when the child window is invalid,
    /// not contained, or crosses its own block's alias span.
    /// 当两个窗口（含别名跨度）相交时返回 `Overlap`；子窗口非法、超出路由器
    /// 窗口或跨越自己所属块的别名跨度时返回 `DeviceError`。
    pub fn mount(&mut self, device: Box<dyn MmioDevice>) -> BusResult<MmioWindow> {
        let child = device.window();
        child.validate()?;
        if child.base < self.window.base || child.end() > self.window.end() {
            return Err(CoreError::BusError {
                addr: child.base,
                kind: BusErrorKind::DeviceError,
            });
        }
        // 寄存器窗口只能占块内前 0x1000 字节，否则会与自己的别名区重叠。
        // A register window may only use the first 0x1000 bytes of its block,
        // otherwise it would overlap its own alias windows.
        let own_alias = alias_span(child.base);
        if child.base < own_alias.end() && own_alias.base < child.end() {
            return Err(CoreError::BusError {
                addr: child.base,
                kind: BusErrorKind::DeviceError,
            });
        }
        let conflicts = self.windows.iter().any(|other| {
            let other_alias = alias_span(other.base);
            let window_clash = child.base < other.end() && other.base < child.end();
            let alias_clash = child.base < other_alias.end() && other_alias.base < child.end();
            let reverse_clash = other.base < own_alias.end() && own_alias.base < other.end();
            window_clash || alias_clash || reverse_clash
        });
        if conflicts {
            return Err(CoreError::BusError {
                addr: child.base,
                kind: BusErrorKind::Overlap,
            });
        }
        self.regions.push(Mutex::new(MmioRegion::new(device)));
        self.windows.push(child);
        // 新设备可能改变地址归属，页缓存必须整体失效。
        // A new device can change ownership, so every page tag is dropped.
        for slot in &self.pages {
            slot.store(EMPTY_SLOT, Ordering::Relaxed);
        }
        Ok(child)
    }

    /// Index of the child owning `addr` / 拥有该地址的子设备索引
    fn child_index(&self, addr: Address) -> Option<usize> {
        self.windows.iter().position(|window| window.contains(addr))
    }

    /// Validate a packed page-cache tag against `addr` / 校验打包的页缓存标签
    #[inline]
    fn cached_child(&self, addr: Address) -> Option<usize> {
        let packed = self.pages[page_slot(addr)].load(Ordering::Relaxed);
        if packed == EMPTY_SLOT || u64::from(page_of(addr)) != (packed >> 40) {
            return None;
        }
        let within = u64::from(addr & 0x0FFF);
        if within >= ((packed >> 16) & PAGE_MASK) && within <= ((packed >> 28) & PAGE_MASK) {
            return Some((packed & 0xFFFF) as usize - 1);
        }
        None
    }

    /// Remember `index` as the owner of the page containing `addr`
    /// 记住 `index` 是包含 `addr` 的页的拥有者
    #[inline]
    fn fill_page(&self, window: MmioWindow, index: usize) {
        if u16::try_from(index + 1).is_ok() {
            self.pages[page_slot(window.base)].store(pack(window, index), Ordering::Relaxed);
        }
    }

    /// Resolve `addr` to a child, folding the atomic alias windows
    /// 将地址解析到子设备，并折叠原子别名窗口
    ///
    /// Inside each block `+0x1000` is XOR, `+0x2000` is SET and `+0x3000` is
    /// CLR over the register with the same low 12 bits. Alias reads return the
    /// underlying value; alias writes are folded into one read-modify-write by
    /// the caller. The child always receives the folded (non-alias) address.
    /// 每个块内 `+0x1000` 为 XOR、`+0x2000` 为 SET、`+0x3000` 为 CLR，作用于
    /// 低 12 位相同的寄存器。别名读返回底层值；别名写由调用方折叠成一次
    /// 读-改-写。子设备始终只看到折叠后的（非别名）地址。
    fn resolve(&self, addr: Address) -> Option<Resolved> {
        if let Some(index) = self.cached_child(addr) {
            return Some(Resolved {
                index,
                reg_addr: addr,
                op: None,
            });
        }
        if let Some(index) = self.child_index(addr) {
            self.fill_page(self.windows[index], index);
            return Some(Resolved {
                index,
                reg_addr: addr,
                op: None,
            });
        }
        for (index, window) in self.windows.iter().enumerate() {
            let block = window.base & !(MMIO_BLOCK_SIZE - 1);
            let (op, alias_base) = match addr.wrapping_sub(block) {
                MMIO_ALIAS_XOR..MMIO_ALIAS_SET => (AliasOp::Xor, MMIO_ALIAS_XOR),
                MMIO_ALIAS_SET..MMIO_ALIAS_CLR => (AliasOp::Set, MMIO_ALIAS_SET),
                MMIO_ALIAS_CLR..MMIO_BLOCK_SIZE => (AliasOp::Clr, MMIO_ALIAS_CLR),
                _ => continue,
            };
            let reg_addr = addr - alias_base;
            if window.contains(reg_addr) {
                return Some(Resolved {
                    index,
                    reg_addr,
                    op: Some(op),
                });
            }
        }
        None
    }

    /// Warn once per unmodelled address / 对每个未建模地址只警告一次
    fn warn(&self, addr: Address) {
        let mut warned = self.warned.lock();
        warn_unhandled(&mut warned, &self.name, addr);
    }
}

impl MemoryRegion for MmioRouter {
    fn name(&self) -> &str {
        &self.name
    }

    fn base(&self) -> Address {
        self.window.base
    }

    fn size(&self) -> usize {
        self.window.size
    }

    fn read_would_stall(&self, addr: Address) -> bool {
        // 只有直接落在子设备窗口内（非别名）的读才可能阻塞；别名窗口在本模型中
        // 从不承载阻塞寄存器。
        // Only a read landing directly in a child window (not an alias) can
        // block; alias windows never carry a blocking register in this model.
        self.child_index(addr)
            .is_some_and(|index| self.regions[index].lock().read_would_stall(addr))
    }

    /// Exclusive stall check that never touches a mutex
    /// 独占 stall 检查，绝不触碰互斥锁
    ///
    /// The single-threaded bus calls this before every MMIO read; taking the
    /// device mutex there would add an atomic pair to every polled register.
    /// 单线程总线在每次 MMIO 读之前调用它；在那里取设备互斥锁会给每个被轮询的
    /// 寄存器增加一对原子操作。
    fn read_would_stall_mut(&mut self, addr: Address) -> bool {
        self.child_index(addr)
            .is_some_and(|index| self.regions[index].get_mut().read_would_stall(addr))
    }

    fn read_u8(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        match self.resolve(addr) {
            Some(Resolved {
                index,
                reg_addr,
                op: None,
            }) => self.regions[index].get_mut().read_u8(reg_addr, cycles),
            Some(Resolved {
                index,
                reg_addr,
                op: Some(_),
            }) => {
                let word = self.regions[index]
                    .get_mut()
                    .read_u32(reg_addr & !3, cycles)?;
                Ok((word >> ((reg_addr & 3) * 8)) as u8)
            }
            None => {
                *cycles += 1;
                self.warn(addr);
                Ok(0)
            }
        }
    }

    fn write_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        match self.resolve(addr) {
            Some(Resolved {
                index,
                reg_addr,
                op: None,
            }) => self.regions[index]
                .get_mut()
                .write_u8(reg_addr, value, cycles),
            Some(Resolved {
                index,
                reg_addr,
                op: Some(op),
            }) => {
                let word_addr = reg_addr & !3;
                let device = self.regions[index].get_mut();
                let current = device.read_u32(word_addr, cycles)?;
                let updated = op.apply(current, u32::from(value) << ((reg_addr & 3) * 8));
                device.write_u32(word_addr, updated, cycles)
            }
            None => {
                *cycles += 1;
                self.warn(addr);
                Ok(())
            }
        }
    }

    fn read_u16(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        match self.resolve(addr) {
            Some(Resolved {
                index,
                reg_addr,
                op: None,
            }) => self.regions[index].get_mut().read_u16(reg_addr, cycles),
            Some(Resolved {
                index,
                reg_addr,
                op: Some(_),
            }) => {
                let word = self.regions[index]
                    .get_mut()
                    .read_u32(reg_addr & !3, cycles)?;
                Ok((word >> ((reg_addr & 3) * 8)) as u16)
            }
            None => {
                *cycles += 1;
                self.warn(addr);
                Ok(0)
            }
        }
    }

    fn write_u16(&mut self, addr: Address, value: u16, cycles: &mut u64) -> BusResult<()> {
        match self.resolve(addr) {
            Some(Resolved {
                index,
                reg_addr,
                op: None,
            }) => self.regions[index]
                .get_mut()
                .write_u16(reg_addr, value, cycles),
            Some(Resolved {
                index,
                reg_addr,
                op: Some(op),
            }) => {
                let word_addr = reg_addr & !3;
                let device = self.regions[index].get_mut();
                let current = device.read_u32(word_addr, cycles)?;
                let updated = op.apply(current, u32::from(value) << ((reg_addr & 3) * 8));
                device.write_u32(word_addr, updated, cycles)
            }
            None => {
                *cycles += 1;
                self.warn(addr);
                Ok(())
            }
        }
    }

    fn read_u32(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        // 别名区读出底层寄存器的当前值。Alias reads return the underlying value.
        if let Some(Resolved {
            index, reg_addr, ..
        }) = self.resolve(addr)
        {
            self.regions[index].get_mut().read_u32(reg_addr, cycles)
        } else {
            *cycles += 1;
            self.warn(addr);
            Ok(0)
        }
    }

    fn write_u32(&mut self, addr: Address, value: u32, cycles: &mut u64) -> BusResult<()> {
        match self.resolve(addr) {
            Some(Resolved {
                index,
                reg_addr,
                op: None,
            }) => self.regions[index]
                .get_mut()
                .write_u32(reg_addr, value, cycles),
            Some(Resolved {
                index,
                reg_addr,
                op: Some(op),
            }) => {
                let device = self.regions[index].get_mut();
                let current = device.read_u32(reg_addr, cycles)?;
                device.write_u32(reg_addr, op.apply(current, value), cycles)
            }
            None => {
                *cycles += 1;
                self.warn(addr);
                Ok(())
            }
        }
    }
}

impl crate::shared::SharedRegion for MmioRouter {
    fn name(&self) -> &str {
        &self.name
    }

    fn base(&self) -> Address {
        self.window.base
    }

    fn size(&self) -> usize {
        self.window.size
    }

    fn read_would_stall(&self, addr: Address) -> bool {
        self.child_index(addr)
            .is_some_and(|index| self.regions[index].lock().read_would_stall(addr))
    }

    fn read_u8(&self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        match self.resolve(addr) {
            Some(Resolved {
                index,
                reg_addr,
                op: None,
            }) => self.regions[index].lock().read_u8(reg_addr, cycles),
            Some(Resolved {
                index,
                reg_addr,
                op: Some(_),
            }) => {
                let word = self.regions[index].lock().read_u32(reg_addr & !3, cycles)?;
                Ok((word >> ((reg_addr & 3) * 8)) as u8)
            }
            None => {
                *cycles += 1;
                self.warn(addr);
                Ok(0)
            }
        }
    }

    fn write_u8(&self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        match self.resolve(addr) {
            Some(Resolved {
                index,
                reg_addr,
                op: None,
            }) => self.regions[index].lock().write_u8(reg_addr, value, cycles),
            Some(Resolved {
                index,
                reg_addr,
                op: Some(op),
            }) => {
                let word_addr = reg_addr & !3;
                // 读-改-写必须在同一把设备锁内完成，否则并发别名写会丢失置位。
                // The read-modify-write must stay inside one device lock,
                // otherwise concurrent alias writes lose bits.
                let mut device = self.regions[index].lock();
                let current = device.read_u32(word_addr, cycles)?;
                let updated = op.apply(current, u32::from(value) << ((reg_addr & 3) * 8));
                device.write_u32(word_addr, updated, cycles)
            }
            None => {
                *cycles += 1;
                self.warn(addr);
                Ok(())
            }
        }
    }

    fn read_u16(&self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        match self.resolve(addr) {
            Some(Resolved {
                index,
                reg_addr,
                op: None,
            }) => self.regions[index].lock().read_u16(reg_addr, cycles),
            Some(Resolved {
                index,
                reg_addr,
                op: Some(_),
            }) => {
                let word = self.regions[index].lock().read_u32(reg_addr & !3, cycles)?;
                Ok((word >> ((reg_addr & 3) * 8)) as u16)
            }
            None => {
                *cycles += 1;
                self.warn(addr);
                Ok(0)
            }
        }
    }

    fn write_u16(&self, addr: Address, value: u16, cycles: &mut u64) -> BusResult<()> {
        match self.resolve(addr) {
            Some(Resolved {
                index,
                reg_addr,
                op: None,
            }) => self.regions[index]
                .lock()
                .write_u16(reg_addr, value, cycles),
            Some(Resolved {
                index,
                reg_addr,
                op: Some(op),
            }) => {
                let word_addr = reg_addr & !3;
                let mut device = self.regions[index].lock();
                let current = device.read_u32(word_addr, cycles)?;
                let updated = op.apply(current, u32::from(value) << ((reg_addr & 3) * 8));
                device.write_u32(word_addr, updated, cycles)
            }
            None => {
                *cycles += 1;
                self.warn(addr);
                Ok(())
            }
        }
    }

    fn read_u32(&self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        if let Some(Resolved {
            index, reg_addr, ..
        }) = self.resolve(addr)
        {
            self.regions[index].lock().read_u32(reg_addr, cycles)
        } else {
            *cycles += 1;
            self.warn(addr);
            Ok(0)
        }
    }

    fn write_u32(&self, addr: Address, value: u32, cycles: &mut u64) -> BusResult<()> {
        match self.resolve(addr) {
            Some(Resolved {
                index,
                reg_addr,
                op: None,
            }) => self.regions[index]
                .lock()
                .write_u32(reg_addr, value, cycles),
            Some(Resolved {
                index,
                reg_addr,
                op: Some(op),
            }) => {
                let mut device = self.regions[index].lock();
                let current = device.read_u32(reg_addr, cycles)?;
                device.write_u32(reg_addr, op.apply(current, value), cycles)
            }
            None => {
                *cycles += 1;
                self.warn(addr);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests;
