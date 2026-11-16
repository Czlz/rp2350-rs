//! Machine bus that can run exclusively or behind a shared thread-safe layer
//! 可在独占或共享线程安全层上运行的机器总线
//!
//! The default `run`/`repl`/`play` path keeps the exclusive `SimpleBus`:
//! every access is a direct call with no atomic operation. `--core-threads on`
//! builds the same region topology on `SharedBus` instead, so the two core
//! threads can access SRAM/Flash/MMIO concurrently. Both variants implement
//! [`AddressBus`], which is why the rest of the machine (firmware loading,
//! DMA/PIO pumps, GDB memory reads) is unchanged.
//! 默认 `run`/`repl`/`play` 路径继续使用独占的 `SimpleBus`：每次访问都是直接
//! 调用，不付任何原子操作。`--core-threads on` 改为在 `SharedBus` 上构建相同的
//! 区域拓扑，使两个核线程可以并发访问 SRAM/Flash/MMIO。两种变体都实现
//! [`AddressBus`]，因此机器其余部分（固件加载、DMA/PIO 泵送、GDB 内存读）无需改动。
//!
//! Regions whose implementation already has a shared access path (`RamRegion`,
//! `FlashRegion`) are registered through [`MachineBus::register_dual`] and keep
//! their bank sharding / lock-free reads. Everything else is wrapped in a
//! `rp2350_core::shared::LockedRegion` (one mutex per region, never a global bus
//! lock).
//! 本身已有共享访问路径的区域（`RamRegion`、`FlashRegion`）经
//! [`MachineBus::register_dual`] 注册，保留其 bank 分片 / 免锁读；其余区域用
//! `rp2350_core::shared::LockedRegion` 包装（每区域一把锁，绝不是全局总线锁）。

use std::sync::Arc;

use rp2350_core::SimpleBus;
use rp2350_core::bus::{AddressBus, BusResult, MemoryRegion};
use rp2350_core::error::{BusErrorKind, CoreError};
use rp2350_core::shared::{SharedAddressBus, SharedBus, SharedRegion};
use rp2350_core::types::Address;

/// Address bus used by [`Machine`](super::Machine)
/// [`Machine`](super::Machine) 使用的地址总线
///
/// `Exclusive` is the default, byte-for-byte compatible path. `Shared` is the
/// `--core-threads on` path: the concrete [`SharedBus`] is behind an `Arc` so
/// the two core workers can hold it while the coordinator keeps performing
/// DMA/PIO/timebase work.
/// `Exclusive` 是默认且逐字节兼容的路径。`Shared` 是 `--core-threads on` 路径：
/// 具体的 [`SharedBus`] 位于 `Arc` 之后，两个核工作线程可以持有它，同时协调线程
/// 继续执行 DMA/PIO/时基工作。
pub enum MachineBus {
    /// Single-threaded exclusive bus / 单线程独占总线
    Exclusive(Box<SimpleBus>),
    /// Concurrent bus shared by the core worker threads / 核工作线程共享的并发总线
    Shared(Arc<SharedBus>),
}

impl MachineBus {
    /// Create an empty exclusive or shared bus / 创建空的独占或共享总线
    #[must_use]
    pub fn new(shared: bool) -> Self {
        if shared {
            Self::Shared(Arc::new(SharedBus::new()))
        } else {
            Self::Exclusive(Box::new(SimpleBus::new()))
        }
    }

    /// Whether this is the shared (thread-safe) variant / 是否为共享（线程安全）变体
    #[must_use]
    pub fn is_shared(&self) -> bool {
        matches!(self, Self::Shared(_))
    }

    /// Clone the shared bus handle, if any / 克隆共享总线句柄（若有）
    #[must_use]
    pub fn shared_handle(&self) -> Option<Arc<SharedBus>> {
        match self {
            Self::Exclusive(_) => None,
            Self::Shared(bus) => Some(Arc::clone(bus)),
        }
    }

    /// PPB write epoch used by the system-register replay guard
    /// 系统寄存器回放门控使用的 PPB 写世代
    #[must_use]
    pub fn ppb_write_epoch(&self) -> u64 {
        match self {
            Self::Exclusive(bus) => bus.ppb_write_epoch(),
            Self::Shared(bus) => SharedAddressBus::ppb_write_epoch(bus.as_ref()),
        }
    }

    /// Register a region that supports both exclusive and shared access
    /// 注册同时支持独占与共享访问的区域
    ///
    /// Only SRAM and Flash use this entry point; every other region goes
    /// through [`AddressBus::register_region`] and is wrapped in a
    /// `rp2350_core::shared::LockedRegion` on the shared bus.
    /// 只有 SRAM 与 Flash 使用该入口；其它区域经 [`AddressBus::register_region`]
    /// 注册，并在共享总线上包成 `rp2350_core::shared::LockedRegion`。
    ///
    /// # Errors
    /// Returns `Overlap` when the region window intersects an existing region.
    /// 区域窗口与已有区域相交时返回 `Overlap`。
    pub fn register_dual<S>(&mut self, region: S) -> BusResult<()>
    where
        S: MemoryRegion + SharedRegion + 'static,
    {
        match self {
            Self::Exclusive(bus) => bus.register_region(Box::new(region)),
            Self::Shared(bus) => Arc::get_mut(bus)
                .expect(
                    "shared bus registration happens before core threads / \
                     共享总线在核线程启动前注册",
                )
                .register_region(Arc::new(region)),
        }
    }

    /// Mutable exclusive bus, for the hot single-threaded stepping path
    /// 可变独占总线，供单线程热路径使用
    ///
    /// Returns `None` on the shared variant: the shared bus must be accessed
    /// through its `&self` methods while core workers hold clones.
    /// 共享变体返回 `None`：核工作线程持有克隆期间，共享总线只能经其 `&self`
    /// 方法访问。
    pub fn exclusive_mut(&mut self) -> Option<&mut SimpleBus> {
        match self {
            Self::Exclusive(bus) => Some(bus.as_mut()),
            Self::Shared(_) => None,
        }
    }
}

impl AddressBus for MachineBus {
    fn register_region(&mut self, region: Box<dyn MemoryRegion>) -> BusResult<()> {
        match self {
            Self::Exclusive(bus) => bus.register_region(region),
            Self::Shared(bus) => Arc::get_mut(bus)
                .expect(
                    "shared bus registration happens before core threads / \
                     共享总线在核线程启动前注册",
                )
                .register_locked(region),
        }
    }

    fn unregister_region(&mut self, name: &str) {
        match self {
            Self::Exclusive(bus) => bus.unregister_region(name),
            Self::Shared(bus) => Arc::get_mut(bus)
                .expect(
                    "shared bus unregistration happens before core threads / \
                     共享总线在核线程启动前注销",
                )
                .unregister_region(name),
        }
    }

    fn find_region_mut(&mut self, addr: Address) -> Option<&mut dyn MemoryRegion> {
        match self {
            Self::Exclusive(bus) => bus.find_region_mut(addr),
            // 共享区域只暴露 `&self` 访问器，无法返回可变借用；调用方应使用
            // `read_u8`/`write_u8` 等入口。
            // Shared regions only expose `&self` accessors, so no mutable borrow
            // can be returned; callers must use `read_u8`/`write_u8`.
            Self::Shared(_) => None,
        }
    }

    fn read_u8(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        match self {
            Self::Exclusive(bus) => bus.read_u8(addr, cycles),
            Self::Shared(bus) => SharedAddressBus::read_u8(bus.as_ref(), addr, cycles),
        }
    }

    fn read_u16(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        match self {
            Self::Exclusive(bus) => bus.read_u16(addr, cycles),
            Self::Shared(bus) => SharedAddressBus::read_u16(bus.as_ref(), addr, cycles),
        }
    }

    fn read_u32(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        match self {
            Self::Exclusive(bus) => bus.read_u32(addr, cycles),
            Self::Shared(bus) => SharedAddressBus::read_u32(bus.as_ref(), addr, cycles),
        }
    }

    fn write_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        match self {
            Self::Exclusive(bus) => bus.write_u8(addr, value, cycles),
            Self::Shared(bus) => SharedAddressBus::write_u8(bus.as_ref(), addr, value, cycles),
        }
    }

    fn write_u16(&mut self, addr: Address, value: u16, cycles: &mut u64) -> BusResult<()> {
        match self {
            Self::Exclusive(bus) => bus.write_u16(addr, value, cycles),
            Self::Shared(bus) => SharedAddressBus::write_u16(bus.as_ref(), addr, value, cycles),
        }
    }

    fn write_u32(&mut self, addr: Address, value: u32, cycles: &mut u64) -> BusResult<()> {
        match self {
            Self::Exclusive(bus) => bus.write_u32(addr, value, cycles),
            Self::Shared(bus) => SharedAddressBus::write_u32(bus.as_ref(), addr, value, cycles),
        }
    }

    fn load_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        match self {
            Self::Exclusive(bus) => bus.load_u8(addr, value, cycles),
            Self::Shared(bus) => SharedAddressBus::load_u8(bus.as_ref(), addr, value, cycles),
        }
    }

    fn load_bytes(&mut self, addr: Address, bytes: &[u8], cycles: &mut u64) -> BusResult<()> {
        match self {
            Self::Exclusive(bus) => bus.load_bytes(addr, bytes, cycles),
            Self::Shared(bus) => SharedAddressBus::load_bytes(bus.as_ref(), addr, bytes, cycles),
        }
    }
}

/// Bus adapter for a core worker thread / 核工作线程的总线适配器
///
/// All ordinary accesses go to the shared bus. The PPB window is intercepted so
/// the worker can replay its own system-control writes into **its** core
/// without stealing the other core's log entries, and SIO accesses are
/// serialised while `Sio::select_core` selects this worker's view.
/// 普通访问全部走共享总线。PPB 窗口被拦截，使工作线程能只把**本核**的系统控制
/// 写入回放到本核，而不会取走另一核的日志；SIO 访问被串行化，并在此期间用
/// `Sio::select_core` 选择本工作线程的视图。
pub(super) struct CoreBus<'a> {
    /// Shared bus serving every non-PPB access / 服务所有非 PPB 访问的共享总线
    pub(super) bus: &'a SharedBus,
    /// Core id selecting the SIO register view / 选择 SIO 寄存器视图的核 ID
    pub(super) core: rp2350_core::types::CpuId,
    /// SIO handle used only for `select_core` / 仅用于 `select_core` 的 SIO 句柄
    pub(super) sio: &'a rp2350_periph::sio::Sio,
    /// Serialises SIO accesses and the shared view switch
    /// 串行化 SIO 访问与共享视图切换
    pub(super) sio_lock: &'a std::sync::Mutex<()>,
    /// Serialises PPB read-modify-write and replay capture across cores
    /// 跨核串行化 PPB 读-改-写与回放捕获
    pub(super) ppb_lock: &'a std::sync::Mutex<()>,
    /// System-control writes to replay into this core / 待回放到本核的系统控制写入
    pub(super) ppb_writes: Vec<(u32, u32)>,
}

impl CoreBus<'_> {
    /// PPB window start address / PPB 窗口起始地址
    pub(super) const PPB_BASE: u32 = 0xE000_0000;
    /// PPB window size (1 MiB) / PPB 窗口大小（1 MiB）
    pub(super) const PPB_SIZE: u32 = 0x0010_0000;
    /// SIO window start address / SIO 窗口起始地址
    pub(super) const SIO_BASE: u32 = 0xD000_0000;
    /// SIO window size / SIO 窗口大小
    pub(super) const SIO_SIZE: u32 = 0x1000;

    /// Whether `addr` lies in the private peripheral bus / 地址是否位于私有外设总线
    fn is_ppb(addr: Address) -> bool {
        (Self::PPB_BASE..Self::PPB_BASE + Self::PPB_SIZE).contains(&addr)
    }

    /// Whether `addr` lies in the SIO register block / 地址是否位于 SIO 寄存器块
    fn is_sio(addr: Address) -> bool {
        (Self::SIO_BASE..Self::SIO_BASE + Self::SIO_SIZE).contains(&addr)
    }

    /// Run one SIO access under the shared view lock / 在共享视图锁下执行一次 SIO 访问
    fn with_sio<R>(&self, f: impl FnOnce() -> R) -> R {
        let _guard = self
            .sio_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.sio.select_core(self.core);
        f()
    }

    /// Record a PPB write that the owning core must apply immediately
    /// 记录必须由所属核心立即应用的 PPB 写入
    fn note_ppb_write(&mut self, addr: Address, value: u32) {
        if super::threads::replays_into_core(addr) {
            self.ppb_writes.push((addr, value));
        }
    }

    /// Take the recorded PPB writes / 取走已记录的 PPB 写入
    pub(super) fn take_ppb_writes(&mut self) -> Vec<(u32, u32)> {
        std::mem::take(&mut self.ppb_writes)
    }

    /// Read the containing word so a sub-word PPB write replays the merged value
    /// 读取所在字，使子字宽 PPB 写入回放合并后的值
    fn merged_word(&self, addr: Address, cycles: &mut u64) -> u32 {
        let word = addr & !3;
        SharedAddressBus::read_u32(self.bus, word, cycles).unwrap_or(0)
    }
}

impl AddressBus for CoreBus<'_> {
    fn register_region(&mut self, _region: Box<dyn MemoryRegion>) -> BusResult<()> {
        // 工作线程运行期间总线拓扑不可变；挂载发生在构造阶段。
        // The bus topology is immutable while workers run; mounting happens
        // during construction.
        Err(CoreError::BusError {
            addr: 0,
            kind: BusErrorKind::DeviceError,
        })
    }

    fn unregister_region(&mut self, _name: &str) {}

    fn find_region_mut(&mut self, _addr: Address) -> Option<&mut dyn MemoryRegion> {
        None
    }

    fn read_u8(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        if Self::is_sio(addr) {
            return self.with_sio(|| SharedAddressBus::read_u8(self.bus, addr, cycles));
        }
        SharedAddressBus::read_u8(self.bus, addr, cycles)
    }

    fn read_u16(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        if Self::is_sio(addr) {
            return self.with_sio(|| SharedAddressBus::read_u16(self.bus, addr, cycles));
        }
        SharedAddressBus::read_u16(self.bus, addr, cycles)
    }

    fn read_u32(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        if Self::is_sio(addr) {
            return self.with_sio(|| SharedAddressBus::read_u32(self.bus, addr, cycles));
        }
        SharedAddressBus::read_u32(self.bus, addr, cycles)
    }

    fn write_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        if Self::is_ppb(addr) {
            let _guard = self
                .ppb_lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let word = self.merged_word(addr, cycles);
            let shift = (addr & 3) * 8;
            let merged = (word & !(0xFF << shift)) | (u32::from(value) << shift);
            SharedAddressBus::write_u8(self.bus, addr, value, cycles)?;
            self.note_ppb_write(addr & !3, merged);
            return Ok(());
        }
        if Self::is_sio(addr) {
            return self.with_sio(|| SharedAddressBus::write_u8(self.bus, addr, value, cycles));
        }
        SharedAddressBus::write_u8(self.bus, addr, value, cycles)
    }

    fn write_u16(&mut self, addr: Address, value: u16, cycles: &mut u64) -> BusResult<()> {
        if Self::is_ppb(addr) {
            let _guard = self
                .ppb_lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let word = self.merged_word(addr, cycles);
            let shift = (addr & 3) * 8;
            let mask = u32::from(u16::MAX) << shift;
            let merged = (word & !mask) | (u32::from(value) << shift);
            SharedAddressBus::write_u16(self.bus, addr, value, cycles)?;
            self.note_ppb_write(addr & !3, merged);
            return Ok(());
        }
        if Self::is_sio(addr) {
            return self.with_sio(|| SharedAddressBus::write_u16(self.bus, addr, value, cycles));
        }
        SharedAddressBus::write_u16(self.bus, addr, value, cycles)
    }

    fn write_u32(&mut self, addr: Address, value: u32, cycles: &mut u64) -> BusResult<()> {
        if Self::is_ppb(addr) {
            let _guard = self
                .ppb_lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            SharedAddressBus::write_u32(self.bus, addr, value, cycles)?;
            self.note_ppb_write(addr & !3, value);
            return Ok(());
        }
        if Self::is_sio(addr) {
            return self.with_sio(|| SharedAddressBus::write_u32(self.bus, addr, value, cycles));
        }
        SharedAddressBus::write_u32(self.bus, addr, value, cycles)
    }

    fn load_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        SharedAddressBus::load_u8(self.bus, addr, value, cycles)
    }

    fn load_bytes(&mut self, addr: Address, bytes: &[u8], cycles: &mut u64) -> BusResult<()> {
        SharedAddressBus::load_bytes(self.bus, addr, bytes, cycles)
    }
}
