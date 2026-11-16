//! Bank-interleaved SRAM region / 按 bank 交错的 SRAM 区域
//!
//! The RP2350 SRAM is word-interleaved across ten banks: word `k` (bytes
//! `4k..4k+4`) lives in bank `k % 10`. This module mirrors that mapping, so a
//! two-core machine writing different addresses usually touches different banks
//! and therefore different locks. Three access paths exist:
//! RP2350 的 SRAM 按字交错分布在十个 bank 上：第 `k` 个字（字节 `4k..4k+4`）位于
//! bank `k % 10`。本模块照此映射，因此双核机器写不同地址时通常落在不同 bank、
//! 即不同锁上。共有三条访问路径：
//!
//! * the exclusive [`MemoryRegion`] path (`&mut self`) reaches the bank buffers
//!   through [`Mutex::get_mut`], which is a plain pointer dereference: the
//!   single-threaded interpreter pays **no** atomic operation for the banking;
//! * the shared [`SharedRegion`](crate::shared::SharedRegion) path (`&self`) locks one bank per access (two
//!   for a span that crosses a bank boundary, always in ascending bank order);
//! * host-side bulk loads write byte by byte through the same bank mapping.
//! * 独占的 [`MemoryRegion`] 路径（`&mut self`）经 [`Mutex::get_mut`] 直接访问
//!   bank 缓冲，这只是普通指针解引用：单线程解释器不为分 bank 付任何原子操作。
//! * 共享的 [`SharedRegion`](crate::shared::SharedRegion) 路径（`&self`）每次访问锁一个 bank（跨 bank 跨度锁
//!   两个，始终按 bank 下标升序）。
//! * 宿主侧批量写入按同一 bank 映射逐字节写入。

// 区域实现把地址在 u32/usize 之间换算并做定宽字节组装，这是硬件仿真层的固有特性。
// The region converts addresses between u32 and usize and assembles fixed-width
// byte groups, which is inherent to the hardware emulation layer.
#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap
)]

use parking_lot::Mutex;

use crate::bus::BusResult;
use crate::bus::MemoryRegion;
use crate::error::{BusErrorKind, CoreError};
use crate::types::Address;

/// Number of word-interleaved SRAM banks on the RP2350
/// RP2350 的按字交错 SRAM bank 数量
///
/// Bank of byte offset `o` is `(o >> 2) % RAM_BANKS`; four consecutive bytes
/// always share one bank.
/// 字节偏移 `o` 所属 bank 为 `(o >> 2) % RAM_BANKS`；连续四个字节总是同属一个
/// bank。
pub const RAM_BANKS: usize = 10;

/// Bank and bank-local byte index of an in-region offset
/// 区域偏移所属的 bank 及其 bank 内字节下标
#[inline]
pub(super) fn bank_slot(offset: usize) -> (usize, usize) {
    let word = offset >> 2;
    (word % RAM_BANKS, (word / RAM_BANKS) * 4 + (offset & 3))
}

/// A flat, byte-addressable RAM region backed by ten interleaved bank buffers
/// 由十个交错 bank 缓冲支持的扁平、字节可寻址 RAM 区域
///
/// Access semantics, cycle accounting, watchpoint reporting and error kinds are
/// identical to the previous flat buffer; only the storage layout changed.
/// [`MemoryRegion`] (exclusive) accesses never lock, [`SharedRegion`](crate::shared::SharedRegion) accesses
/// lock one bank (or two, in ascending order, for a span that crosses a word
/// boundary).
/// 访问语义、周期记账、监视点上报与错误类型都与先前的扁平缓冲完全一致，只有存储
/// 布局改变。[`MemoryRegion`]（独占）访问从不加锁；[`SharedRegion`](crate::shared::SharedRegion) 访问锁一个
/// bank（跨字边界的跨度按升序锁两个）。
#[derive(Debug)]
pub struct RamRegion {
    pub(super) name: String,
    pub(super) base: Address,
    pub(super) writable: bool,
    pub(super) size: usize,
    pub(super) banks: [Mutex<Vec<u8>>; RAM_BANKS],
}

impl Clone for RamRegion {
    /// Deep-copy every bank / 深拷贝每个 bank
    ///
    /// Banks are locked one at a time, so cloning never holds two bank locks and
    /// cannot deadlock against a running access.
    /// 逐个加锁，克隆时不同时持有两把 bank 锁，因此不会与运行中的访问死锁。
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            base: self.base,
            writable: self.writable,
            size: self.size,
            banks: core::array::from_fn(|bank| Mutex::new(self.banks[bank].lock().clone())),
        }
    }
}

impl RamRegion {
    /// Create a zeroed, writable RAM region
    /// 创建一个清零的可写 RAM 区域
    ///
    /// The caller must ensure `base + size` stays within the 32-bit address space.
    /// 调用者需确保 `base + size` 不超出 32 位地址空间。
    #[must_use]
    pub fn new(name: impl Into<String>, base: Address, size: usize) -> Self {
        Self::with_writable(name, base, size, true)
    }

    /// Create a zeroed, read-only region (used to model ROM)
    /// 创建一个清零的只读区域（用于模拟 ROM）
    ///
    /// The caller must ensure `base + size` stays within the 32-bit address space.
    /// 调用者需确保 `base + size` 不超出 32 位地址空间。
    #[must_use]
    pub fn new_rom(name: impl Into<String>, base: Address, size: usize) -> Self {
        Self::with_writable(name, base, size, false)
    }

    /// Shared constructor / 共享构造实现
    fn with_writable(name: impl Into<String>, base: Address, size: usize, writable: bool) -> Self {
        let words = size.div_ceil(4);
        Self {
            name: name.into(),
            base,
            writable,
            size,
            // 每个 bank 容纳 word k 满足 k % 10 == bank 的全部字；末尾不足一个字的
            // 部分也按整字分配，越界访问由 `contains` 拒绝，填充字节永不可达。
            // Each bank holds every word k with k % 10 == bank; a trailing partial
            // word is padded to four bytes and stays unreachable because
            // `contains` rejects out-of-region addresses.
            banks: core::array::from_fn(|bank| {
                let count = if bank < words {
                    (words - 1 - bank) / RAM_BANKS + 1
                } else {
                    0
                };
                Mutex::new(vec![0u8; count * 4])
            }),
        }
    }

    /// Bank that serves the word containing `addr`, or `None` outside the region
    /// 处理 `addr` 所在字的 bank；区域外返回 `None`
    #[must_use]
    pub fn bank_of(&self, addr: Address) -> Option<usize> {
        if !self.span_contains(addr) {
            return None;
        }
        Some(((addr - self.base) as usize >> 2) % RAM_BANKS)
    }

    /// Whether `addr` is inside the configured window / 地址是否在配置窗口内
    ///
    /// An inherent helper so the two region traits in scope cannot make
    /// `contains` ambiguous.
    /// 固有辅助函数，避免作用域内两个区域 trait 让 `contains` 产生歧义。
    fn span_contains(&self, addr: Address) -> bool {
        let offset = addr.wrapping_sub(self.base) as usize;
        addr >= self.base && offset < self.size
    }

    /// Copy the backing bytes out of the banks / 从各 bank 复制出底层字节
    ///
    /// The storage is physically interleaved, so there is no contiguous slice to
    /// borrow; the copy is assembled in address order so it is byte-identical to
    /// the flat layout this region used to expose.
    /// 存储物理上是交错的，没有可借用的连续切片；副本按地址顺序拼装，因此与原先
    /// 扁平布局逐字节一致。
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.size];
        for (bank, buffer) in self.banks.iter().enumerate() {
            let guard = buffer.lock();
            for (local, byte) in guard.iter().enumerate() {
                let offset = ((local / 4) * RAM_BANKS + bank) * 4 + (local % 4);
                if offset < self.size {
                    out[offset] = *byte;
                }
            }
        }
        out
    }

    /// Readable/writable flag / 是否可写
    #[must_use]
    pub fn is_writable(&self) -> bool {
        self.writable
    }

    /// Translate an in-region address into a buffer offset
    /// 将区域内地址转换为缓冲区偏移
    pub(super) fn offset(&self, addr: Address) -> BusResult<usize> {
        if !self.span_contains(addr) {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            });
        }
        Ok((addr - self.base) as usize)
    }

    /// Byte at `offset`, or `None` outside the region / 偏移处的字节
    fn stored_byte(&mut self, offset: usize) -> Option<u8> {
        if offset >= self.size {
            return None;
        }
        let (bank, local) = bank_slot(offset);
        self.banks[bank].get_mut().get(local).copied()
    }

    /// Store one byte at `offset` / 在偏移处存储一个字节
    fn store_byte(&mut self, offset: usize, value: u8) -> Option<()> {
        if offset >= self.size {
            return None;
        }
        let (bank, local) = bank_slot(offset);
        let slot = self.banks[bank].get_mut().get_mut(local)?;
        *slot = value;
        Some(())
    }

    /// Translate an in-region address into a buffer offset, if it is mapped
    /// 若地址在区域内则转换为缓冲区偏移
    pub(super) fn maybe_offset(&self, addr: Address) -> Option<usize> {
        if !self.span_contains(addr) {
            return None;
        }
        Some((addr - self.base) as usize)
    }

    /// Read one byte and account for one bus cycle, without watchpoint hooks
    /// 读取一个字节并记一次总线周期，不触发监视点钩子
    fn read_byte(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        *cycles += 1;
        let offset = self.offset(addr)?;
        self.stored_byte(offset).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })
    }

    /// Copy `N` bytes when the span stays inside one bank and one word
    /// 跨度落在同一个 bank 的同一个字内时复制 `N` 个字节
    ///
    /// Wide accesses then take one `Vec` slice copy, exactly like the previous
    /// flat implementation. Spans that cross a word boundary return `None` and
    /// fall back to the byte-wise path so bank-crossing semantics stay explicit.
    /// One helper does the bounds check, the word/bank split and the copy, which
    /// keeps the single-threaded hot path free of duplicated range tests.
    /// 宽访问随后只需一次 `Vec` 切片拷贝，与先前扁平实现相同。跨字边界的跨度返回
    /// `None` 并回退到逐字节路径，使跨 bank 语义保持显式。边界检查、字/bank 拆分
    /// 与拷贝都在同一个辅助函数里，使单线程热路径没有重复的区间比较。
    #[inline]
    fn window<const N: usize>(&mut self, addr: Address) -> Option<[u8; N]> {
        let (bank, local) = self.window_slot::<N>(addr)?;
        let slot = self.banks[bank].get_mut().get(local..local + N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(slot);
        Some(out)
    }

    /// Borrow `N` writable bytes when the span stays inside one bank and word
    /// 跨度落在同一个 bank 的同一个字内时借用 `N` 个可写字节
    #[inline]
    fn window_mut<const N: usize>(&mut self, addr: Address) -> Option<&mut [u8; N]> {
        if !self.writable {
            return None;
        }
        let (bank, local) = self.window_slot::<N>(addr)?;
        let slot = self.banks[bank].get_mut().get_mut(local..local + N)?;
        slot.try_into().ok()
    }

    /// Bank and bank-local index of an `N`-byte span inside one word
    /// 落在同一个字内的 `N` 字节跨度的 bank 与 bank 内下标
    ///
    /// A span that starts at word offset `w` and stays within four bytes never
    /// crosses into another bank, so the bank buffer holds it contiguously.
    /// 起点在字内偏移 `w` 且不超出四个字节的跨度不会跨到另一个 bank，因此 bank
    /// 缓冲中是连续存放的。
    #[inline]
    fn window_slot<const N: usize>(&self, addr: Address) -> Option<(usize, usize)> {
        let offset = addr.wrapping_sub(self.base) as usize;
        if addr < self.base || (offset & 3) + N > 4 || offset + N > self.size {
            return None;
        }
        let word = offset >> 2;
        Some((word % RAM_BANKS, (word / RAM_BANKS) * 4 + (offset & 3)))
    }

    /// Store `bytes` one at a time through [`MemoryRegion::write_u8`]
    /// 通过 [`MemoryRegion::write_u8`] 逐字节存储
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
        self.size
    }

    /// SRAM never stalls, answered without a dynamic dispatch
    /// SRAM 永不 stall，无需动态分派即可回答
    ///
    /// The single-threaded bus asks this before every read; the trait default
    /// would route through `read_would_stall` and add a second virtual call to
    /// each of the ~50 M SRAM accesses in a 300 M-step run.
    /// 单线程总线在每次读之前询问；trait 默认实现会转到 `read_would_stall`，给
    /// 300 M 步运行中约 5000 万次 SRAM 访问各增加一次虚调用。
    fn read_would_stall_mut(&mut self, _addr: Address) -> bool {
        false
    }

    fn read_u8(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        let value = self.read_byte(addr, cycles)?;
        // 诊断钩子：未安装读监视时只多一次原子 load。
        // Diagnostic hook: an unarmed read watch costs one atomic load.
        crate::watch::check_read(addr, 1);
        Ok(value)
    }

    fn read_u16(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        if let Some(slot) = self.window::<2>(addr) {
            *cycles += 2;
            crate::watch::check_read(addr, 2);
            return Ok(u16::from_le_bytes(slot));
        }
        let lo = u16::from(self.read_byte(addr, cycles)?);
        let hi = u16::from(self.read_byte(addr.wrapping_add(1), cycles)?);
        let value = lo | (hi << 8);
        crate::watch::check_read(addr, 2);
        Ok(value)
    }

    fn read_u32(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        if let Some(slot) = self.window::<4>(addr) {
            *cycles += 4;
            crate::watch::check_read(addr, 4);
            return Ok(u32::from_le_bytes(slot));
        }
        let mut value = 0u32;
        for index in 0..4u32 {
            let byte = u32::from(self.read_byte(addr.wrapping_add(index), cycles)?);
            value |= byte << (index * 8);
        }
        crate::watch::check_read(addr, 4);
        Ok(value)
    }

    /// A halfword store is one slice copy; a byte-wise fallback keeps the
    /// write-watch and error semantics
    /// 半字写入是一次切片拷贝；逐字节回退保留写监视与错误语义
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
        self.store_byte(offset, value).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })
    }

    /// A host-side bulk load keeps the byte-wise watch and error semantics
    /// 宿主侧批量加载保留逐字节的监视与错误语义
    fn load_bytes(&mut self, addr: Address, bytes: &[u8], cycles: &mut u64) -> BusResult<()> {
        if crate::watch::write_watch_armed() || !self.writable {
            for (offset, byte) in bytes.iter().enumerate() {
                self.load_u8(addr.wrapping_add(offset as u32), *byte, cycles)?;
            }
            return Ok(());
        }
        let span = self
            .offset(addr)
            .ok()
            .filter(|offset| bytes.len() <= self.size - *offset);
        let Some(offset) = span else {
            for (offset, byte) in bytes.iter().enumerate() {
                self.load_u8(addr.wrapping_add(offset as u32), *byte, cycles)?;
            }
            return Ok(());
        };
        for (index, byte) in bytes.iter().enumerate() {
            let (bank, local) = bank_slot(offset + index);
            // 跨度已整体校验过，`bank` 缓冲必然覆盖该下标。
            // The span was validated, so the bank buffer covers the index.
            self.banks[bank].get_mut()[local] = *byte;
        }
        *cycles += bytes.len() as u64;
        Ok(())
    }
}
