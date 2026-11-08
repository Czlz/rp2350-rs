//! BOOTRAM window with the boot locks / BOOTRAM 窗口与 boot 锁
//!
//! The boot ROM keeps a 2 KB scratch RAM at `0x400E_0000`, followed by two
//! write-once words (`+0x800`, `+0x804`) and eight boot locks (`+0x80C` ..
//! `+0x828`) with a status bitmap at `+0x808`. The scratch area stays readable
//! and writable here because firmware copies `0x400E_0000..0x400E_0100` out of
//! it (a 256-byte trampoline in MicroPython's boot path); only the register
//! tail is special-cased.
//! boot ROM 在 `0x400E_0000` 保留 2 KB 暂存 RAM，其后是两个只写一次的寄存器
//! （`+0x800`、`+0x804`）、状态位图 `+0x808` 和八个 boot 锁（`+0x80C` 至
//! `+0x828`）。暂存区在这里保持可读可写，因为固件会把
//! `0x400E_0000..0x400E_0100` 拷贝出去（MicroPython 启动路径里的 256 字节
//! 跳板）；只有寄存器尾部做特殊处理。
//!
//! Boot locks behave like the SIO spin locks: a read claims the lock and returns
//! `1 << n` on success or `0` if already claimed, a write releases it, and
//! `BOOTLOCK_STAT` reports `1 = unclaimed, 0 = claimed` (reset value `0xFF`).
//! boot 锁与 SIO 自旋锁行为一致：读取即认领，成功返回 `1 << n`、已占用返回 0；
//! 写入即释放；`BOOTLOCK_STAT` 为 `1 = 未认领、0 = 已认领`（复位值 `0xFF`）。

use rp2350_core::mmio::{MmioDevice, MmioWindow};

use super::map;

/// Size of the scratch RAM in front of the registers / 寄存器之前暂存 RAM 的大小
const DATA_SIZE: u32 = 0x800;

/// `WRITE_ONCE0` offset / `WRITE_ONCE0` 偏移
const WRITE_ONCE0: u32 = 0x800;

/// `WRITE_ONCE1` offset / `WRITE_ONCE1` 偏移
const WRITE_ONCE1: u32 = 0x804;

/// `BOOTLOCK_STAT` offset / `BOOTLOCK_STAT` 偏移
const BOOTLOCK_STAT: u32 = 0x808;

/// Offset of the first boot lock / 第一个 boot 锁的偏移
const BOOTLOCK0: u32 = 0x80C;

/// Number of boot locks / boot 锁数量
const BOOTLOCKS: u32 = 8;

/// BOOTRAM scratch area, write-once words and boot locks
/// BOOTRAM 暂存区、只写一次寄存器与 boot 锁
#[derive(Debug)]
pub struct BootRam {
    /// Scratch RAM shared with the boot ROM / 与 boot ROM 共享的暂存 RAM
    data: [u8; DATA_SIZE as usize],
    /// `WRITE_ONCE0`/`WRITE_ONCE1`, which only ever gain bits
    /// 只增不减的 `WRITE_ONCE0`/`WRITE_ONCE1`
    write_once: [u32; 2],
    /// Claimed boot locks, one bit per lock / 已认领的 boot 锁，每锁一位
    claimed: u8,
}

impl BootRam {
    /// Create the block / 创建寄存器块
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: [0; DATA_SIZE as usize],
            write_once: [0; 2],
            claimed: 0,
        }
    }

    /// Index of the boot lock at `offset` / `offset` 处 boot 锁的序号
    fn bootlock(offset: u32) -> Option<usize> {
        let index = offset.checked_sub(BOOTLOCK0)?;
        if index % 4 != 0 || index / 4 >= BOOTLOCKS {
            return None;
        }
        Some((index / 4) as usize)
    }

    /// Load four scratch bytes / 读取四个暂存字节
    fn load_word(&self, offset: u32) -> Option<u32> {
        let start = offset as usize;
        let bytes = self.data.get(start..start + 4)?;
        Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Store four scratch bytes / 写入四个暂存字节
    fn store_word(&mut self, offset: u32, value: u32) -> bool {
        let start = offset as usize;
        let Some(bytes) = self.data.get_mut(start..start + 4) else {
            return false;
        };
        bytes.copy_from_slice(&value.to_le_bytes());
        true
    }
}

impl Default for BootRam {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for BootRam {
    fn name(&self) -> &str {
        "BOOTRAM"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::BOOTRAM_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        if offset < DATA_SIZE {
            return self.load_word(offset);
        }
        match offset {
            WRITE_ONCE0 => Some(self.write_once[0]),
            WRITE_ONCE1 => Some(self.write_once[1]),
            // 1 = 未认领。1 means unclaimed.
            BOOTLOCK_STAT => Some(u32::from(!self.claimed)),
            _ => {
                let index = Self::bootlock(offset)?;
                let bit = 1u8 << index;
                if self.claimed & bit != 0 {
                    // 已被占用，认领失败。Already claimed, so the claim fails.
                    Some(0)
                } else {
                    self.claimed |= bit;
                    Some(u32::from(bit))
                }
            }
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        if offset < DATA_SIZE {
            return self.store_word(offset, value);
        }
        match offset {
            // 只增不减。Only gains bits.
            WRITE_ONCE0 => {
                self.write_once[0] |= value;
                true
            }
            WRITE_ONCE1 => {
                self.write_once[1] |= value;
                true
            }
            // 状态位图只读。The status bitmap is read-only.
            BOOTLOCK_STAT => true,
            _ => {
                let Some(index) = Self::bootlock(offset) else {
                    return false;
                };
                // 写入即释放，写入值被忽略。A write releases the lock; the value is ignored.
                self.claimed &= !(1u8 << index);
                true
            }
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(ram: &mut BootRam, offset: u32) -> u32 {
        ram.read32(offset).unwrap()
    }

    #[test]
    fn first_read_claims_the_lock() {
        let mut ram = BootRam::new();
        assert_eq!(read(&mut ram, BOOTLOCK0), 1);
        assert_eq!(read(&mut ram, BOOTLOCK0 + 4 * 7), 1 << 7);
    }

    #[test]
    fn second_read_of_a_claimed_lock_returns_zero() {
        let mut ram = BootRam::new();
        assert_eq!(read(&mut ram, BOOTLOCK0 + 4 * 2), 1 << 2);
        assert_eq!(read(&mut ram, BOOTLOCK0 + 4 * 2), 0);
        assert_eq!(read(&mut ram, BOOTLOCK0 + 4 * 2), 0);
    }

    #[test]
    fn write_releases_the_lock_for_a_later_claim() {
        let mut ram = BootRam::new();
        assert_eq!(read(&mut ram, BOOTLOCK0 + 4 * 3), 1 << 3);
        assert!(ram.write32(BOOTLOCK0 + 4 * 3, 0));
        assert_eq!(read(&mut ram, BOOTLOCK0 + 4 * 3), 1 << 3);
    }

    #[test]
    fn stat_bitmap_marks_unclaimed_locks() {
        let mut ram = BootRam::new();
        // 复位值 0xFF：全部未认领。Reset value 0xFF: all unclaimed.
        assert_eq!(read(&mut ram, BOOTLOCK_STAT), 0xFF);
        assert_eq!(read(&mut ram, BOOTLOCK0), 1);
        assert_eq!(read(&mut ram, BOOTLOCK0 + 4 * 7), 1 << 7);
        assert_eq!(read(&mut ram, BOOTLOCK_STAT), 0xFF & !0x81);
        assert!(ram.write32(BOOTLOCK0, 1));
        assert_eq!(read(&mut ram, BOOTLOCK_STAT), 0xFF & !0x80);
    }

    #[test]
    fn stat_is_read_only() {
        let mut ram = BootRam::new();
        assert!(ram.write32(BOOTLOCK_STAT, 0));
        assert_eq!(read(&mut ram, BOOTLOCK_STAT), 0xFF);
    }

    #[test]
    fn write_once_registers_only_gain_bits() {
        let mut ram = BootRam::new();
        assert!(ram.write32(WRITE_ONCE0, 0b0101));
        assert!(ram.write32(WRITE_ONCE0, 0b0010));
        assert_eq!(read(&mut ram, WRITE_ONCE0), 0b0111);
        assert!(ram.write32(WRITE_ONCE1, 0x8000_0000));
        assert_eq!(read(&mut ram, WRITE_ONCE1), 0x8000_0000);
    }

    #[test]
    fn scratch_area_stays_readable_alongside_the_locks() {
        let mut ram = BootRam::new();
        assert!(ram.write32(0x0000, 0xDEAD_BEEF));
        assert!(ram.write32(0x00FC, 0x1234_5678));
        assert_eq!(read(&mut ram, BOOTLOCK0), 1);
        // 认领锁之后 256 字节搬运区仍然可读。The copied range is still readable.
        assert_eq!(read(&mut ram, 0x0000), 0xDEAD_BEEF);
        assert_eq!(read(&mut ram, 0x00FC), 0x1234_5678);
        assert_eq!(read(&mut ram, 0x0004), 0);
    }

    #[test]
    fn offsets_past_the_registers_are_unmodelled() {
        let mut ram = BootRam::new();
        assert_eq!(ram.read32(BOOTLOCK0 + 4 * BOOTLOCKS), None);
        assert!(!ram.write32(BOOTLOCK0 + 4 * BOOTLOCKS, 0));
        // 非 4 字节对齐的锁偏移未建模。Misaligned lock offsets are unmodelled.
        assert_eq!(ram.read32(BOOTLOCK0 + 2), None);
    }

    #[test]
    fn reset_restores_the_documented_values() {
        let mut ram = BootRam::new();
        assert!(ram.write32(0x0000, 0xFFFF_FFFF));
        assert!(ram.write32(WRITE_ONCE0, 3));
        assert_eq!(read(&mut ram, BOOTLOCK0), 1);
        ram.reset();
        assert_eq!(read(&mut ram, 0x0000), 0);
        assert_eq!(read(&mut ram, WRITE_ONCE0), 0);
        assert_eq!(read(&mut ram, BOOTLOCK_STAT), 0xFF);
        assert_eq!(read(&mut ram, BOOTLOCK0), 1);
    }
}
