//! Flash XIP memory region
//! Flash XIP 内存区域
//!
//! The RP2350 executes code directly from a 16 MB XIP window at `0x1000_0000`.
//! This region models that window: the guest may read and execute from it, but
//! writes are rejected. Firmware loaders program it through
//! [`MemoryRegion::load_u8`], which bypasses the guest write policy.
//! RP2350 通过 `0x1000_0000` 处的 16 MB XIP 窗口直接执行代码。本区域模拟该窗口：
//! 客户机可读取与执行，但写入会被拒绝。固件加载器通过 [`MemoryRegion::load_u8`]
//! 编程该区域，从而绕过客户机写策略。
//!
//! # Concurrent reads / 并发读
//!
//! The backing store is an array of relaxed [`AtomicU32`] words, so a read is one
//! plain load with **no mutex** and readers never serialise against each other.
//! Guest writes are rejected, so the only writer is the host (loader, `FlashHandle`,
//! debugger), and every host mutation bumps [`content_generation`]. A single
//! access inside one word is atomic; a span crossing a word boundary can observe
//! the two words from different generations, which is why the bulk snapshot
//! helpers retry while the generation changes.
//! 底层存储是 relaxed [`AtomicU32`] 字数组，因此一次读就是一次普通加载、**不取
//! 互斥锁**，读者之间也永不串行。客户机写被拒绝，唯一写入方是宿主（加载器、
//! `FlashHandle`、调试器），每次宿主变更都递增 [`content_generation`]。落在同一字
//! 内的单次访问是原子的；跨字边界的跨度可能看到属于不同世代的两个字，因此批量
//! 快照辅助函数在世代变化时重试。

// Flash 区域实现涉及 u32/usize 之间的地址换算，这是硬件仿真层的固有特性。
// The flash region converts between u32 and usize for address arithmetic, which
// is inherent to the hardware emulation layer.
#![allow(clippy::cast_possible_truncation)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use super::bus::BusResult;
use super::bus::MemoryRegion;
use super::error::{BusErrorKind, CoreError};
use super::types::Address;

/// Monotonic generation of host-side flash content changes
/// 宿主侧 Flash 内容变更的单调世代号
///
/// Guest writes to flash are rejected, so the only way the programmed bytes can
/// change is a host-side loader or debugger write through [`FlashRegion`] or
/// [`FlashHandle`]. Every such mutation bumps this counter with `Release` and
/// readers load it with `Acquire`, so a reader that revalidates after a host
/// write cannot observe the new generation together with stale bytes. On x86 both
/// orderings compile to plain loads/stores, so the single-threaded path pays
/// nothing for the guarantee.
/// 客户机对 Flash 的写会被拒绝，因此已编程字节只可能经由 [`FlashRegion`] 或
/// [`FlashHandle`] 的宿主侧加载器/调试器写入而改变。每次变更都以 `Release` 递增本
/// 计数器，读取方以 `Acquire` 加载，因此宿主写之后重新校验的读者不会看到「新世代
/// 配旧字节」。在 x86 上两种顺序都编译为普通加载/存储，单线程路径零开销。
static CONTENT_GENERATION: AtomicU32 = AtomicU32::new(0);

/// Current flash content generation / 当前 Flash 内容世代号
///
/// The `Acquire` load pairs with the `Release` bump in
/// [`bump_content_generation`]: everything the host wrote before publishing a
/// generation is visible to a reader that observes that generation.
/// `Acquire` 加载与 [`bump_content_generation`] 的 `Release` 递增配对：宿主在发布
/// 世代之前写入的所有内容，对观察到该世代的读者可见。
#[must_use]
pub fn content_generation() -> u32 {
    CONTENT_GENERATION.load(Ordering::Acquire)
}

/// Publish a host-side content change / 发布一次宿主侧内容变更
fn bump_content_generation() {
    CONTENT_GENERATION.fetch_add(1, Ordering::Release);
}

/// Number of attempts a generation-validated bulk read makes
/// 世代校验批量读取的最大尝试次数
const SNAPSHOT_ATTEMPTS: u32 = 4;

/// Base address of the RP2350 flash XIP window / RP2350 Flash XIP 窗口基地址
pub const FLASH_XIP_BASE: Address = 0x1000_0000;

/// Base address of the non-cached flash XIP alias (pico-sdk `XIP_NOCACHE_NOALLOC_BASE`)
/// 非缓存 Flash XIP 别名基地址（pico-sdk `XIP_NOCACHE_NOALLOC_BASE`）
pub const FLASH_XIP_NOCACHE_BASE: Address = 0x1400_0000;

/// Size of the RP2350 flash XIP window (16 MB) / RP2350 Flash XIP 窗口大小（16 MB）
pub const FLASH_XIP_SIZE: usize = 16 * 1024 * 1024;

/// Lock-free word store shared by every XIP window / 所有 XIP 窗口共享的无锁字存储
///
/// Words are the natural NOR-flash granularity for a modelled XIP window: a byte,
/// halfword or aligned word access touches exactly one word, so it becomes one
/// relaxed load or store. Unaligned spans are assembled from two words.
/// 字是建模 XIP 窗口的自然粒度：字节、半字或对齐字访问恰好落在一个字内，因此就是
/// 一次 relaxed 加载或存储。非对齐跨度由两个字拼装。
struct FlashStore {
    words: Vec<AtomicU32>,
    size: usize,
}

impl Clone for FlashStore {
    /// Clone the store by copying every word / 逐字复制存储
    fn clone(&self) -> Self {
        Self {
            words: self
                .words
                .iter()
                .map(|word| AtomicU32::new(word.load(Ordering::Relaxed)))
                .collect(),
            size: self.size,
        }
    }
}

impl std::fmt::Debug for FlashStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlashStore")
            .field("size", &self.size)
            .field("words", &self.words.len())
            .finish()
    }
}

impl FlashStore {
    /// Allocate a zeroed store of `size` bytes / 分配指定字节数的清零存储
    fn new(size: usize) -> Self {
        Self {
            words: (0..size.div_ceil(4)).map(|_| AtomicU32::new(0)).collect(),
            size,
        }
    }

    /// One word, zero when the index is out of range / 读取一个字（越界读零）
    #[inline]
    fn word(&self, index: usize) -> u32 {
        self.words
            .get(index)
            .map_or(0, |word| word.load(Ordering::Relaxed))
    }

    /// Predicate for an in-store byte / 判断字节是否在存储内
    #[inline]
    fn holds(&self, offset: usize) -> bool {
        offset < self.size
    }

    /// Byte at `offset`, or `None` past the end / 偏移处的字节
    fn byte(&self, offset: usize) -> Option<u8> {
        if !self.holds(offset) {
            return None;
        }
        Some((self.word(offset >> 2) >> ((offset & 3) * 8)) as u8)
    }

    /// Little-endian `N`-byte value at `offset` / 偏移处的 `N` 字节小端值
    ///
    /// `N` must be 2 or 4 and the caller must have verified the span.
    /// `N` 只能是 2 或 4，调用方需已校验跨度。
    fn value<const N: usize>(&self, offset: usize) -> u32 {
        let index = offset >> 2;
        let shift = (offset & 3) * 8;
        if shift == 0 {
            return self.word(index) & mask::<N>();
        }
        let low = self.word(index) >> shift;
        let high = self.word(index + 1) << (32 - shift);
        (low | high) & mask::<N>()
    }

    /// Store one byte through a compare-exchange loop / 经比较交换循环存储一个字节
    ///
    /// The loop keeps concurrent host writers from losing each other's bytes: a
    /// plain load/store pair would drop a byte written between the two.
    /// 该循环避免并发宿主写入互相丢失字节：普通的「加载-存储」对会丢掉两者之间写入
    /// 的字节。
    fn store_byte(&self, offset: usize, value: u8) {
        let index = offset >> 2;
        let Some(word) = self.words.get(index) else {
            return;
        };
        let shift = (offset & 3) * 8;
        let mask = 0xFFu32 << shift;
        let mut current = word.load(Ordering::Relaxed);
        loop {
            let next = (current & !mask) | (u32::from(value) << shift);
            match word.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return,
                Err(observed) => current = observed,
            }
        }
    }

    /// Store a byte range / 存储一段字节
    fn store_range(&self, offset: usize, bytes: &[u8]) {
        let mut index = 0usize;
        let mut at = offset;
        while index < bytes.len() {
            let in_word = at & 3;
            let take = (4 - in_word).min(bytes.len() - index);
            if in_word == 0 && take == 4 {
                if let Some(word) = self.words.get(at >> 2) {
                    let chunk: [u8; 4] = bytes[index..index + 4]
                        .try_into()
                        .expect("four bytes / 四个字节");
                    word.store(u32::from_le_bytes(chunk), Ordering::Relaxed);
                }
            } else {
                for step in 0..take {
                    self.store_byte(at + step, bytes[index + step]);
                }
            }
            index += take;
            at += take;
        }
    }

    /// Fill a byte range with the erased state `0xFF` / 将一段字节填为擦除态 `0xFF`
    fn erase_range(&self, offset: usize, len: usize) {
        let mut at = offset;
        let end = offset + len;
        while at < end {
            let in_word = at & 3;
            let take = (4 - in_word).min(end - at);
            if in_word == 0 && take == 4 {
                if let Some(word) = self.words.get(at >> 2) {
                    word.store(u32::MAX, Ordering::Relaxed);
                }
            } else {
                for step in 0..take {
                    self.store_byte(at + step, 0xFF);
                }
            }
            at += take;
        }
    }

    /// Copy a range out, word by word / 逐字复制一段字节
    fn snapshot(&self, offset: usize, len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        let end = offset + len;
        let mut at = offset;
        while at < end {
            let in_word = at & 3;
            let take = (4 - in_word).min(end - at);
            let word = self.word(at >> 2).to_le_bytes();
            out.extend_from_slice(&word[in_word..in_word + take]);
            at += take;
        }
        out
    }

    /// Copy a range out and retry while a host write races it
    /// 复制一段字节，宿主写入竞争时重试
    ///
    /// Bounded retries keep a busy loader from spinning a debugger forever; the
    /// last attempt is returned as-is. Every returned buffer is at least
    /// per-word consistent.
    /// 有界重试避免繁忙的加载器让调试器无限自旋；最后一次尝试原样返回。返回的缓冲区
    /// 至少保证每个字内部一致。
    fn snapshot_stable(&self, offset: usize, len: usize) -> Vec<u8> {
        for _ in 0..SNAPSHOT_ATTEMPTS {
            let before = content_generation();
            let bytes = self.snapshot(offset, len);
            if content_generation() == before {
                return bytes;
            }
        }
        self.snapshot(offset, len)
    }
}

/// Keep only the low `N` bytes of a little-endian word / 只保留小端字的低 `N` 字节
fn mask<const N: usize>() -> u32 {
    if N >= 4 {
        u32::MAX
    } else {
        (1u32 << (N * 8)) - 1
    }
}

/// A flash XIP region that is read-only to the guest and host-programmable
/// 对客户机只读、对宿主可编程的 Flash XIP 区域
///
/// Guest writes return [`BusErrorKind::AccessDenied`]. Flash programming through
/// the QMI/XIP controller is not modelled in Stage 4; the loader is the only
/// writer. Clones and [`FlashRegion::alias`] windows share one backing store.
/// 客户机写入返回 [`BusErrorKind::AccessDenied`]。Stage 4 不模拟经由 QMI/XIP
/// 控制器的 Flash 编程，只有加载器可以写入。克隆与 [`FlashRegion::alias`]
/// 窗口共享同一份底层存储。
#[derive(Debug, Clone)]
pub struct FlashRegion {
    name: String,
    base: Address,
    size: usize,
    data: Arc<FlashStore>,
}

impl FlashRegion {
    /// Create a zeroed flash region / 创建一个清零的 Flash 区域
    ///
    /// The caller must ensure `base + size` stays within the 32-bit address space.
    /// 调用者需确保 `base + size` 不超出 32 位地址空间。
    #[must_use]
    pub fn new(name: impl Into<String>, base: Address, size: usize) -> Self {
        // 新建底层存储等同于内容变更：任何按地址缓存的 Flash 字节都必须失效。
        // A fresh backing store counts as a content change so any address-keyed
        // cache of flash bytes is dropped.
        bump_content_generation();
        Self {
            name: name.into(),
            base,
            size,
            data: Arc::new(FlashStore::new(size)),
        }
    }

    /// Create the standard 16 MB RP2350 XIP window / 创建标准的 16 MB RP2350 XIP 窗口
    #[must_use]
    pub fn rp2350_xip() -> Self {
        Self::new("flash-xip", FLASH_XIP_BASE, FLASH_XIP_SIZE)
    }

    /// Create another read-only window over the same flash bytes
    /// 在同一份 Flash 字节上创建另一个只读窗口
    ///
    /// RP2350 exposes the flash at `0x1000_0000` (cached) and at the
    /// `XIP_NOCACHE_NOALLOC_BASE` alias `0x1400_0000` (non-cached). Both windows
    /// must observe the same programmed content, so the alias shares the backing
    /// store rather than copying it.
    /// RP2350 在 `0x1000_0000`（缓存）与 `XIP_NOCACHE_NOALLOC_BASE` 别名
    /// `0x1400_0000`（非缓存）同时暴露 Flash。两个窗口必须看到相同的已编程
    /// 内容，因此别名共享底层存储而非复制。
    #[must_use]
    pub fn alias(&self, name: impl Into<String>, base: Address) -> Self {
        Self {
            name: name.into(),
            base,
            size: self.size,
            data: Arc::clone(&self.data),
        }
    }

    /// Borrow the backing bytes (snapshot, retried while a writer races it)
    /// 借用底层字节（快照；宿主写入竞争时重试）
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        self.data.snapshot_stable(0, self.size)
    }

    /// Read a slice of flash bytes without going through the bus
    /// 不经过总线读取一段 Flash 字节
    ///
    /// Returns `None` when the requested range leaves the region.
    /// 请求范围超出区域时返回 `None`。
    #[must_use]
    pub fn slice(&self, addr: Address, len: usize) -> Option<Vec<u8>> {
        let offset = usize::try_from(addr.checked_sub(self.base)?).ok()?;
        if offset.checked_add(len)? > self.size {
            return None;
        }
        Some(self.data.snapshot_stable(offset, len))
    }

    /// Create a host-only handle over the same backing store
    /// 在同一份底层存储上创建宿主专用句柄
    ///
    /// Bootrom emulation runs on the host but must erase and program exactly the
    /// bytes the guest reads through the XIP windows. The handle exposes that
    /// path without weakening [`MemoryRegion`]'s guest write rejection.
    /// bootrom 仿真运行在宿主侧，但必须擦除和编程客户机经 XIP 窗口读取的同一份
    /// 字节。该句柄提供这条路径，同时不削弱 [`MemoryRegion`] 对客户机写入的拒绝。
    #[must_use]
    pub fn handle(&self) -> FlashHandle {
        FlashHandle {
            base: self.base,
            data: Arc::clone(&self.data),
        }
    }

    /// Program bytes through the host-side loader path
    /// 通过宿主侧加载器路径编程字节
    ///
    /// # Errors
    /// Returns a bus error if the range leaves the region.
    /// 范围超出区域时返回总线错误。
    pub fn load_bytes(&mut self, addr: Address, bytes: &[u8]) -> BusResult<()> {
        let offset = self.offset(addr)?;
        let end = offset.checked_add(bytes.len()).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })?;
        if end > self.size {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::DeviceError,
            });
        }
        self.data.store_range(offset, bytes);
        bump_content_generation();
        Ok(())
    }

    /// Translate an in-region address into a buffer offset / 将区域内地址转换为缓冲区偏移
    fn offset(&self, addr: Address) -> BusResult<usize> {
        if addr < self.base || addr.wrapping_sub(self.base) as usize >= self.size {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            });
        }
        Ok((addr - self.base) as usize)
    }

    /// Read one byte / 读取一个字节
    fn fetch_u8(&self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        *cycles += 1;
        let offset = self.offset(addr)?;
        self.data.byte(offset).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })
    }

    /// Read a halfword with one word load when the span allows it
    /// 跨度允许时用一次字加载读取半字
    fn fetch_u16(&self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        if let Ok(offset) = self.offset(addr) {
            if offset + 2 <= self.size {
                *cycles += 2;
                return Ok(self.data.value::<2>(offset) as u16);
            }
        }
        let lo = u16::from(self.fetch_u8(addr, cycles)?);
        let hi = u16::from(self.fetch_u8(addr.wrapping_add(1), cycles)?);
        Ok(lo | (hi << 8))
    }

    /// Read a word with one or two word loads / 读取字（一到两次字加载）
    fn fetch_u32(&self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        if let Ok(offset) = self.offset(addr) {
            if offset + 4 <= self.size {
                *cycles += 4;
                return Ok(self.data.value::<4>(offset));
            }
        }
        let mut value = 0u32;
        for index in 0..4u32 {
            let byte = u32::from(self.fetch_u8(addr.wrapping_add(index), cycles)?);
            value |= byte << (index * 8);
        }
        Ok(value)
    }

    /// Program one host byte / 编程一个宿主字节
    fn store_u8(&self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        *cycles += 1;
        let offset = self.offset(addr)?;
        if !self.data.holds(offset) {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::DeviceError,
            });
        }
        self.data.store_byte(offset, value);
        bump_content_generation();
        Ok(())
    }
}

impl MemoryRegion for FlashRegion {
    fn name(&self) -> &str {
        &self.name
    }

    fn base(&self) -> Address {
        self.base
    }

    fn size(&self) -> usize {
        // 长度在构造时固定，因此无需加锁读取。
        // The length is fixed at construction, so it needs no lock.
        self.size
    }

    /// Flash never stalls, answered without a dynamic dispatch
    /// Flash 永不 stall，无需动态分派即可回答
    ///
    /// Instruction fetch is the hottest bus operation; the trait default would
    /// add a second virtual call to every flash read.
    /// 取指是最热的总线操作；trait 默认实现会给每次 Flash 读增加一次虚调用。
    fn read_would_stall_mut(&mut self, _addr: Address) -> bool {
        false
    }

    fn read_u8(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        self.fetch_u8(addr, cycles)
    }

    fn read_u16(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        self.fetch_u16(addr, cycles)
    }

    fn read_u32(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        self.fetch_u32(addr, cycles)
    }

    fn write_u8(&mut self, addr: Address, _value: u8, _cycles: &mut u64) -> BusResult<()> {
        Err(CoreError::BusError {
            addr,
            kind: BusErrorKind::AccessDenied,
        })
    }

    fn load_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        self.store_u8(addr, value, cycles)
    }

    /// Host-side bulk programming books one cycle per byte and one generation bump
    /// 宿主侧批量编程每字节记一个周期，整段只递增一次世代
    ///
    /// # Errors
    /// Returns `BusError` when the span leaves the region.
    /// 跨度超出区域时返回 `BusError`。
    fn load_bytes(&mut self, addr: Address, bytes: &[u8], cycles: &mut u64) -> BusResult<()> {
        // 跨度完整落在区域内才走快路径；否则逐字节回退，使越界处的错误类型
        // 与「能写下的字节已写下」的行为和逐字节路径完全一致。
        // The fast path needs the whole span inside the region; otherwise the
        // byte-wise fallback reproduces both the error kind at the crossing and
        // the "bytes before the crossing landed" behaviour.
        let span = addr
            .checked_sub(self.base)
            .map(|offset| offset as usize)
            .filter(|offset| *offset <= self.size && bytes.len() <= self.size - *offset);
        let Some(offset) = span else {
            for (index, byte) in bytes.iter().enumerate() {
                self.load_u8(addr.wrapping_add(index as u32), *byte, cycles)?;
            }
            return Ok(());
        };
        *cycles += bytes.len() as u64;
        self.data.store_range(offset, bytes);
        bump_content_generation();
        Ok(())
    }
}

impl crate::shared::SharedRegion for FlashRegion {
    fn name(&self) -> &str {
        &self.name
    }

    fn base(&self) -> Address {
        self.base
    }

    fn size(&self) -> usize {
        self.size
    }

    fn read_u8(&self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        self.fetch_u8(addr, cycles)
    }

    fn read_u16(&self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        self.fetch_u16(addr, cycles)
    }

    fn read_u32(&self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        self.fetch_u32(addr, cycles)
    }

    fn write_u8(&self, addr: Address, _value: u8, _cycles: &mut u64) -> BusResult<()> {
        Err(CoreError::BusError {
            addr,
            kind: BusErrorKind::AccessDenied,
        })
    }

    fn load_u8(&self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        self.store_u8(addr, value, cycles)
    }

    fn load_bytes(&self, addr: Address, bytes: &[u8], cycles: &mut u64) -> BusResult<()> {
        let span = addr
            .checked_sub(self.base)
            .map(|offset| offset as usize)
            .filter(|offset| *offset <= self.size && bytes.len() <= self.size - *offset);
        let Some(offset) = span else {
            for (index, byte) in bytes.iter().enumerate() {
                self.load_u8(addr.wrapping_add(index as u32), *byte, cycles)?;
            }
            return Ok(());
        };
        *cycles += bytes.len() as u64;
        self.data.store_range(offset, bytes);
        bump_content_generation();
        Ok(())
    }
}

/// Host-only handle to the flash bytes shared by every XIP window
/// 由所有 XIP 窗口共享的 Flash 字节的宿主专用句柄
///
/// The guest reaches flash only through [`MemoryRegion`], which rejects writes.
/// Bootrom emulation runs on the host and uses this handle to erase and program
/// the same backing store that the guest reads.
/// 客户机只能通过 [`MemoryRegion`] 访问 Flash，而该路径拒绝写入。bootrom 仿真
/// 运行在宿主侧，通过本句柄擦除和编程客户机读取的同一份底层存储。
#[derive(Debug, Clone)]
pub struct FlashHandle {
    base: Address,
    data: Arc<FlashStore>,
}

impl FlashHandle {
    /// Base address of the window this handle was created from
    /// 创建本句柄时所用窗口的基地址
    #[must_use]
    pub fn base(&self) -> Address {
        self.base
    }

    /// Capacity of the backing store in bytes / 底层存储容量（字节）
    #[must_use]
    pub fn size(&self) -> usize {
        self.data.size
    }

    /// Read a range of flash bytes / 读取一段 Flash 字节
    ///
    /// # Errors
    /// Returns a bus error if the range leaves the backing store.
    /// 范围超出底层存储时返回总线错误。
    pub fn read(&self, addr: Address, len: usize) -> BusResult<Vec<u8>> {
        let offset = self.offset(addr)?;
        let end = offset.checked_add(len).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })?;
        if end > self.data.size {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::DeviceError,
            });
        }
        Ok(self.data.snapshot_stable(offset, len))
    }

    /// Program a byte range / 编程一段字节
    ///
    /// # Errors
    /// Returns a bus error if the range leaves the backing store.
    /// 范围超出底层存储时返回总线错误。
    pub fn write(&self, addr: Address, bytes: &[u8]) -> BusResult<()> {
        let offset = self.offset(addr)?;
        let end = offset.checked_add(bytes.len()).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })?;
        if end > self.data.size {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::DeviceError,
            });
        }
        self.data.store_range(offset, bytes);
        bump_content_generation();
        Ok(())
    }

    /// Erase a byte range to the NOR flash erased state (`0xFF`)
    /// 将一段字节擦除为 NOR Flash 的擦除态（`0xFF`）
    ///
    /// # Errors
    /// Returns a bus error if the range leaves the backing store.
    /// 范围超出底层存储时返回总线错误。
    pub fn erase(&self, addr: Address, len: usize) -> BusResult<()> {
        let offset = self.offset(addr)?;
        let end = offset.checked_add(len).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })?;
        if end > self.data.size {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::DeviceError,
            });
        }
        self.data.erase_range(offset, len);
        bump_content_generation();
        Ok(())
    }

    /// Translate an in-region address into a buffer offset / 将区域内地址转换为缓冲区偏移
    fn offset(&self, addr: Address) -> BusResult<usize> {
        if addr < self.base || addr - self.base >= self.data.size as u32 {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            });
        }
        Ok((addr - self.base) as usize)
    }
}

#[cfg(test)]
mod tests;
