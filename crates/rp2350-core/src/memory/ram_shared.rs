//! Concurrent access path for [`RamRegion`] / [`RamRegion`] 的并发访问路径
//!
//! Every access takes the locks of the one or two banks its span touches and
//! holds them for the whole transfer, so the bytes of one access are never
//! observed half-written by another bus access. Banks are always locked in
//! ascending index order and no other lock is taken while they are held, which
//! makes the lock graph acyclic.
//! 每次访问只获取其跨度涉及的一到两个 bank 的锁，并在整个传输期间持有，因此一次
//! 访问的字节绝不会被另一次总线访问看到写了一半。bank 始终按下标升序加锁，持锁期间
//! 不再获取其它锁，因此锁图无环。

// 共享路径涉及 usize/u32 之间的偏移换算与定宽字节组装，这是硬件仿真层的固有特性。
// The shared path converts offsets between usize and u32 and assembles
// fixed-width byte groups, which is inherent to the hardware emulation layer.
#![allow(clippy::cast_possible_truncation)]

use parking_lot::MutexGuard;

use super::ram::{RAM_BANKS, RamRegion, bank_slot};
use crate::bus::BusResult;
use crate::error::{BusErrorKind, CoreError};
use crate::shared::SharedRegion;
use crate::types::Address;

impl RamRegion {
    /// Byte at `offset` through this region's shared path / 共享路径读取一个字节
    fn shared_byte(&self, offset: usize) -> Option<u8> {
        if offset >= self.size {
            return None;
        }
        let (bank, local) = bank_slot(offset);
        self.banks[bank].lock().get(local).copied()
    }

    /// Read a little-endian span of at most four bytes through bank locks
    /// 经 bank 锁读取最多四字节的小端跨度
    fn shared_read_span(&self, offset: usize, len: usize) -> u32 {
        let mut value = 0u32;
        if (offset & 3) + len <= 4 {
            // 单 bank、单字：一把锁、一次切片拷贝。
            // One bank, one word: one lock and one slice copy.
            let (bank, local) = bank_slot(offset);
            let guard = self.banks[bank].lock();
            if let Some(slot) = guard.get(local..local + len) {
                for (index, byte) in slot.iter().enumerate() {
                    value |= u32::from(*byte) << (index * 8);
                }
            }
            return value;
        }
        let mut span = SpanGuard::new(self, offset, len);
        for index in 0..len {
            let at = offset + index;
            let (bank, local) = bank_slot(at);
            let byte = span.data(bank).get(local).copied().unwrap_or(0);
            value |= u32::from(byte) << (index * 8);
        }
        value
    }

    /// Write a little-endian span of at most four bytes through bank locks
    /// 经 bank 锁写入最多四字节的小端跨度
    fn shared_write_span(&self, offset: usize, value: u32, len: usize) {
        if (offset & 3) + len <= 4 {
            let (bank, local) = bank_slot(offset);
            let mut guard = self.banks[bank].lock();
            if let Some(slot) = guard.get_mut(local..local + len) {
                for (index, byte) in slot.iter_mut().enumerate() {
                    *byte = (value >> (index * 8)) as u8;
                }
            }
            return;
        }
        let mut span = SpanGuard::new(self, offset, len);
        for index in 0..len {
            let at = offset + index;
            let (bank, local) = bank_slot(at);
            if let Some(slot) = span.data(bank).get_mut(local) {
                *slot = (value >> (index * 8)) as u8;
            }
        }
    }
}

/// Guards for the one or two banks a span touches / 跨度涉及的一到两个 bank 的守卫
///
/// Banks are always locked in ascending index order. No other lock is taken while
/// these guards live, so the order rules out deadlock.
/// bank 始终按下标升序加锁。守卫存活期间不再获取其它锁，因此该顺序排除了死锁。
struct SpanGuard<'a> {
    /// Bank indices in locking order / 按加锁顺序排列的 bank 下标
    banks: [usize; 2],
    /// Guards matching [`Self::banks`] / 与 [`Self::banks`] 对应的守卫
    guards: [Option<MutexGuard<'a, Vec<u8>>>; 2],
}

impl<'a> SpanGuard<'a> {
    /// Lock the banks covering `len` bytes at `offset` / 锁定覆盖跨度的 bank
    fn new(region: &'a RamRegion, offset: usize, len: usize) -> Self {
        let first = (offset >> 2) % RAM_BANKS;
        let last = ((offset + len - 1) >> 2) % RAM_BANKS;
        if first == last {
            return Self {
                banks: [first, first],
                guards: [Some(region.banks[first].lock()), None],
            };
        }
        let (low, high) = if first < last {
            (first, last)
        } else {
            (last, first)
        };
        Self {
            banks: [low, high],
            guards: [
                Some(region.banks[low].lock()),
                Some(region.banks[high].lock()),
            ],
        }
    }

    /// Mutably borrow the buffer of one of the locked banks
    /// 可变借用已加锁的某个 bank 缓冲
    fn data(&mut self, bank: usize) -> &mut Vec<u8> {
        if self.banks[0] == bank {
            self.guards[0]
                .as_mut()
                .expect("first bank guard / 首个 bank 守卫")
        } else {
            debug_assert_eq!(self.banks[1], bank, "span touches at most two banks");
            self.guards[1]
                .as_mut()
                .expect("second bank guard / 第二个 bank 守卫")
        }
    }
}

impl SharedRegion for RamRegion {
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
        *cycles += 1;
        let offset = self.offset(addr)?;
        let value = self.shared_byte(offset).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })?;
        crate::watch::check_read(addr, 1);
        Ok(value)
    }

    fn read_u16(&self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        if let Some(offset) = self.maybe_offset(addr) {
            if offset + 2 <= self.size {
                let value = self.shared_read_span(offset, 2) as u16;
                *cycles += 2;
                crate::watch::check_read(addr, 2);
                return Ok(value);
            }
        }
        let lo = u16::from(self.read_u8(addr, cycles)?);
        let hi = u16::from(self.read_u8(addr.wrapping_add(1), cycles)?);
        let value = lo | (hi << 8);
        crate::watch::check_read(addr, 2);
        Ok(value)
    }

    fn read_u32(&self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        if let Some(offset) = self.maybe_offset(addr) {
            if offset + 4 <= self.size {
                let value = self.shared_read_span(offset, 4);
                *cycles += 4;
                crate::watch::check_read(addr, 4);
                return Ok(value);
            }
        }
        let mut value = 0u32;
        for index in 0..4u32 {
            let byte = u32::from(self.read_u8(addr.wrapping_add(index), cycles)?);
            value |= byte << (index * 8);
        }
        crate::watch::check_read(addr, 4);
        Ok(value)
    }

    fn write_u8(&self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        *cycles += 1;
        crate::watch::check_write(addr, u32::from(value));
        if !self.writable {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::AccessDenied,
            });
        }
        let offset = self.offset(addr)?;
        if offset >= self.size {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::DeviceError,
            });
        }
        let (bank, local) = bank_slot(offset);
        let mut guard = self.banks[bank].lock();
        let slot = guard.get_mut(local).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })?;
        *slot = value;
        Ok(())
    }

    fn write_u16(&self, addr: Address, value: u16, cycles: &mut u64) -> BusResult<()> {
        // 已安装写监视时仍逐字节上报，但整段在 bank 锁内一次完成，避免并发观察
        // 到写了一半的字。
        // With a write watch armed every byte is still reported, but the span is
        // written inside one bank lock so a concurrent observer never sees a
        // half-written word.
        let span = if self.writable {
            self.maybe_offset(addr)
                .filter(|offset| offset + 2 <= self.size)
        } else {
            None
        };
        if let Some(offset) = span {
            let bytes = value.to_le_bytes();
            if crate::watch::write_watch_armed() {
                for (index, byte) in bytes.iter().enumerate() {
                    crate::watch::check_write(addr.wrapping_add(index as u32), u32::from(*byte));
                }
            }
            self.shared_write_span(offset, u32::from(value), 2);
            *cycles += 2;
            return Ok(());
        }
        for (index, byte) in value.to_le_bytes().iter().enumerate() {
            self.write_u8(addr.wrapping_add(index as u32), *byte, cycles)?;
        }
        Ok(())
    }

    fn write_u32(&self, addr: Address, value: u32, cycles: &mut u64) -> BusResult<()> {
        let span = if self.writable {
            self.maybe_offset(addr)
                .filter(|offset| offset + 4 <= self.size)
        } else {
            None
        };
        if let Some(offset) = span {
            let bytes = value.to_le_bytes();
            if crate::watch::write_watch_armed() {
                for (index, byte) in bytes.iter().enumerate() {
                    crate::watch::check_write(addr.wrapping_add(index as u32), u32::from(*byte));
                }
            }
            self.shared_write_span(offset, value, 4);
            *cycles += 4;
            return Ok(());
        }
        for (index, byte) in value.to_le_bytes().iter().enumerate() {
            self.write_u8(addr.wrapping_add(index as u32), *byte, cycles)?;
        }
        Ok(())
    }

    fn load_bytes(&self, addr: Address, bytes: &[u8], cycles: &mut u64) -> BusResult<()> {
        if crate::watch::write_watch_armed() || !self.writable {
            for (offset, byte) in bytes.iter().enumerate() {
                self.load_u8(addr.wrapping_add(offset as u32), *byte, cycles)?;
            }
            return Ok(());
        }
        let span = self
            .maybe_offset(addr)
            .filter(|offset| bytes.len() <= self.size - *offset);
        let Some(offset) = span else {
            for (offset, byte) in bytes.iter().enumerate() {
                self.load_u8(addr.wrapping_add(offset as u32), *byte, cycles)?;
            }
            return Ok(());
        };
        let mut index = 0usize;
        while index < bytes.len() {
            let at = offset + index;
            // 每个 bank 段最多四个字节，段内只取一次锁。
            // A bank segment is at most four bytes long and takes one lock.
            let chunk = (4 - (at & 3)).min(bytes.len() - index);
            let (bank, local) = bank_slot(at);
            let mut guard = self.banks[bank].lock();
            guard[local..local + chunk].copy_from_slice(&bytes[index..index + chunk]);
            drop(guard);
            index += chunk;
        }
        *cycles += bytes.len() as u64;
        Ok(())
    }
}
