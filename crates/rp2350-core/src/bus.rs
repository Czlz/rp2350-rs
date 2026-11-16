//! Address bus trait and memory region abstraction
//! 地址总线 trait 和内存区域抽象

use super::types::Address;

use super::error::{BusErrorKind, CoreError};

/// Result type for bus operations
/// 总线操作的结果类型
pub type BusResult<T> = Result<T, CoreError>;

/// Memory access width
/// 内存访问宽度
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessWidth {
    /// 8-bit byte access / 8 位字节访问
    Byte,
    /// 16-bit halfword access / 16 位半字访问
    HalfWord,
    /// 32-bit word access / 32 位字访问
    Word,
}

/// A region of memory or MMIO that can be read and written
/// 可读写的内存或 MMIO 区域
pub trait MemoryRegion: Send + Sync {
    /// Region name for debugging / 区域名称（调试用）
    fn name(&self) -> &str;

    /// Start address (inclusive) / 起始地址（含）
    fn base(&self) -> Address;

    /// Region size in bytes / 区域大小（字节）
    fn size(&self) -> usize;

    /// End address (exclusive) / 结束地址（不含）
    fn end(&self) -> Address {
        #[allow(clippy::cast_possible_truncation)]
        {
            self.base() + self.size() as u32
        }
    }

    /// Check if `addr` falls within this region / 检查地址是否在此区域内
    fn contains(&self, addr: Address) -> bool {
        addr >= self.base() && addr < self.end()
    }

    /// Whether a read of `addr` would currently stall
    /// 读取 `addr` 目前是否会 stall
    ///
    /// A region that can block (a blocking FIFO, a mailbox) reports `true` here
    /// while the data is missing. [`AddressBus`] then fails the read with
    /// [`BusErrorKind::Stall`] **before** the region is touched, so a stalled
    /// access can never consume, clear or advance device state; the CPU retries
    /// the identical access once an event arrives. The default is `false`, so
    /// every existing region keeps exactly its previous behaviour.
    /// 可能阻塞的区域（阻塞 FIFO、信箱）在数据缺失时返回 `true`。[`AddressBus`] 随后
    /// 在触碰区域**之前**以 [`BusErrorKind::Stall`] 结束读取，因此被 stall 的访问绝不
    /// 会消费、清除或推进设备状态；事件到来后 CPU 重试完全相同的访问。默认值为
    /// `false`，因此所有既有区域的行为逐字节不变。
    fn read_would_stall(&self, _addr: Address) -> bool {
        false
    }

    /// Exclusive variant of [`Self::read_would_stall`]
    /// [`Self::read_would_stall`] 的独占变体
    ///
    /// A bus that already holds `&mut self` for the access calls this before the
    /// region is touched. It exists so a region whose shared implementation must
    /// take a lock can answer the stall question from an exclusive borrow
    /// instead, keeping the single-threaded poll path free of atomics. The
    /// default forwards to [`Self::read_would_stall`], so every existing region
    /// keeps exactly its previous behaviour.
    /// 已持有 `&mut self` 的总线会在触碰区域之前调用它。它的存在使得「共享实现必须
    /// 加锁」的区域可以用独占借用回答 stall 问题，从而让单线程轮询路径不付原子操作。
    /// 默认转发到 [`Self::read_would_stall`]，因此所有既有区域的行为完全不变。
    fn read_would_stall_mut(&mut self, addr: Address) -> bool {
        self.read_would_stall(addr)
    }

    /// Read a single byte / 读取一个字节
    ///
    /// # Errors
    /// Returns `BusError` if the address is invalid or access is denied.
    /// 如果地址无效或访问被拒绝，返回 `BusError`。
    fn read_u8(&mut self, addr: Address, _cycles: &mut u64) -> BusResult<u8>;

    /// Write a single byte / 写入一个字节
    ///
    /// # Errors
    /// Returns `BusError` if the address is invalid or access is denied.
    /// 如果地址无效或访问被拒绝，返回 `BusError`。
    fn write_u8(&mut self, addr: Address, value: u8, _cycles: &mut u64) -> BusResult<()>;

    /// Read a 16-bit halfword / 读取 16 位半字
    ///
    /// # Errors
    /// Returns `BusError` if the address is invalid or access is denied.
    /// 如果地址无效或访问被拒绝，返回 `BusError`。
    fn read_u16(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        let lo = u16::from(self.read_u8(addr, cycles)?);
        let hi = u16::from(self.read_u8(addr + 1, cycles)?);
        Ok(lo | (hi << 8))
    }

    /// Write a 16-bit halfword / 写入 16 位半字
    ///
    /// # Errors
    /// Returns `BusError` if the address is invalid or access is denied.
    /// 如果地址无效或访问被拒绝，返回 `BusError`。
    fn write_u16(&mut self, addr: Address, value: u16, cycles: &mut u64) -> BusResult<()> {
        self.write_u8(addr, (value & 0xFF) as u8, cycles)?;
        self.write_u8(addr + 1, ((value >> 8) & 0xFF) as u8, cycles)
    }

    /// Read a 32-bit word / 读取 32 位字
    ///
    /// # Errors
    /// Returns `BusError` if the address is invalid or access is denied.
    /// 如果地址无效或访问被拒绝，返回 `BusError`。
    fn read_u32(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        let lo = u32::from(self.read_u16(addr, cycles)?);
        let hi = u32::from(self.read_u16(addr + 2, cycles)?);
        Ok(lo | (hi << 16))
    }

    /// Write a 32-bit word / 写入 32 位字
    ///
    /// # Errors
    /// Returns `BusError` if the address is invalid or access is denied.
    /// 如果地址无效或访问被拒绝，返回 `BusError`。
    fn write_u32(&mut self, addr: Address, value: u32, cycles: &mut u64) -> BusResult<()> {
        self.write_u16(addr, (value & 0xFFFF) as u16, cycles)?;
        self.write_u16(addr + 2, ((value >> 16) & 0xFFFF) as u16, cycles)
    }

    /// Host-side byte write that bypasses guest write protection
    /// 宿主侧字节写入，绕过客户机写保护
    ///
    /// Firmware loaders and debuggers use this entry point to program ROM-like
    /// regions (for example flash) that reject guest writes. The default
    /// implementation forwards to [`Self::write_u8`].
    /// 固件加载器和调试器通过此入口写入 ROM 类区域（例如 Flash），这类区域
    /// 拒绝客户机写入。默认实现转发到 [`Self::write_u8`]。
    ///
    /// # Errors
    /// Returns `BusError` if the address is invalid or access is denied.
    /// 如果地址无效或访问被拒绝，返回 `BusError`。
    fn load_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        self.write_u8(addr, value, cycles)
    }

    /// Host-side bulk write into a region / 宿主侧批量写入区域
    ///
    /// The default forwards to [`Self::load_u8`] once per byte, so regions that
    /// do not specialise it keep byte-exact cycles, watchpoints and errors. A
    /// region that owns a flat buffer overrides it with one bounds check and one
    /// copy.
    /// 默认逐字节转发到 [`Self::load_u8`]，因此未特化的区域保持逐字节的周期、
    /// 监视点与错误语义；拥有扁平缓冲区的区域改为一次边界检查加一次拷贝。
    ///
    /// # Errors
    /// Returns `BusError` if the address is invalid or access is denied.
    /// 如果地址无效或访问被拒绝，返回 `BusError`。
    fn load_bytes(&mut self, addr: Address, bytes: &[u8], cycles: &mut u64) -> BusResult<()> {
        for (offset, byte) in bytes.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let offset = offset as u32;
            self.load_u8(addr.wrapping_add(offset), *byte, cycles)?;
        }
        Ok(())
    }
}

/// The central address bus that routes memory accesses to the correct region
/// 中央地址总线，将内存访问路由到正确区域
pub trait AddressBus: Send + Sync {
    /// Register a memory region / 注册一个内存区域
    ///
    /// # Errors
    /// Returns `BusError` if the region overlaps with an existing one.
    /// 如果区域与已有区域重叠，返回 `BusError`。
    fn register_region(&mut self, region: Box<dyn MemoryRegion>) -> BusResult<()>;

    /// Remove a region by name / 按名称移除区域
    fn unregister_region(&mut self, name: &str);

    /// Find which region handles this address / 查找处理此地址的区域
    fn find_region_mut(&mut self, addr: Address) -> Option<&mut dyn MemoryRegion>;

    /// Read from the bus (auto-routes to correct region) / 从总线读取（自动路由到正确区域）
    ///
    /// # Errors
    /// Returns `BusError` if no device is mapped at the given address, or
    /// [`BusErrorKind::Stall`] when the region reports a blocking read.
    /// 如果给定地址没有映射设备，返回 `BusError`；区域报告阻塞读时返回
    /// [`BusErrorKind::Stall`]。
    fn read_u8(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        self.resolve_readable(addr)?.read_u8(addr, cycles)
    }

    /// Resolve `addr` to a region that can serve a read right now
    /// 把 `addr` 解析到当前可服务读取的区域
    ///
    /// A region whose [`MemoryRegion::read_would_stall`] is `true` is rejected
    /// with [`BusErrorKind::Stall`] before any device code runs.
    /// [`MemoryRegion::read_would_stall`] 为 `true` 的区域会在任何设备代码运行前以
    /// [`BusErrorKind::Stall`] 拒绝。
    ///
    /// # Errors
    /// Returns `NoDevice` when nothing is mapped and `Stall` for a blocked read.
    /// 未映射返回 `NoDevice`，阻塞读返回 `Stall`。
    fn resolve_readable(&mut self, addr: Address) -> BusResult<&mut dyn MemoryRegion> {
        let region = self.find_region_mut(addr).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::NoDevice,
        })?;
        if region.read_would_stall_mut(addr) {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::Stall,
            });
        }
        Ok(region)
    }

    /// Read a 16-bit halfword / 读取 16 位半字
    ///
    /// # Errors
    /// Returns `BusError` if no device is mapped at the given address, or
    /// [`BusErrorKind::Stall`] when the region reports a blocking read.
    /// 如果给定地址没有映射设备，返回 `BusError`；区域报告阻塞读时返回
    /// [`BusErrorKind::Stall`]。
    fn read_u16(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        self.resolve_readable(addr)?.read_u16(addr, cycles)
    }

    /// Read a 32-bit word / 读取 32 位字
    ///
    /// # Errors
    /// Returns `BusError` if no device is mapped at the given address, or
    /// [`BusErrorKind::Stall`] when the region reports a blocking read.
    /// 如果给定地址没有映射设备，返回 `BusError`；区域报告阻塞读时返回
    /// [`BusErrorKind::Stall`]。
    fn read_u32(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        self.resolve_readable(addr)?.read_u32(addr, cycles)
    }

    /// Write to the bus (auto-routes to correct region) / 写入总线（自动路由到正确区域）
    ///
    /// # Errors
    /// Returns `BusError` if no device is mapped at the given address.
    /// 如果给定地址没有映射设备，返回 `BusError`。
    fn write_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        self.find_region_mut(addr)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .write_u8(addr, value, cycles)
    }

    /// Write a 16-bit halfword / 写入 16 位半字
    ///
    /// # Errors
    /// Returns `BusError` if no device is mapped at the given address.
    /// 如果给定地址没有映射设备，返回 `BusError`。
    fn write_u16(&mut self, addr: Address, value: u16, cycles: &mut u64) -> BusResult<()> {
        self.find_region_mut(addr)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .write_u16(addr, value, cycles)
    }

    /// Write a 32-bit word / 写入 32 位字
    ///
    /// # Errors
    /// Returns `BusError` if no device is mapped at the given address.
    /// 如果给定地址没有映射设备，返回 `BusError`。
    fn write_u32(&mut self, addr: Address, value: u32, cycles: &mut u64) -> BusResult<()> {
        crate::watch::check_write(addr, value);
        self.find_region_mut(addr)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .write_u32(addr, value, cycles)
    }

    /// Host-side byte write (loader and debugger path)
    /// 宿主侧字节写入（加载器与调试器路径）
    ///
    /// Unlike [`Self::write_u8`], this entry point asks the region to accept the
    /// write even when it normally rejects guest writes. Firmware loaders use it
    /// to program flash.
    /// 与 [`Self::write_u8`] 不同，此入口会让区域接受写入，即使该区域通常拒绝
    /// 客户机写入。固件加载器用它写入 Flash。
    ///
    /// # Errors
    /// Returns `BusError` if no device is mapped at the given address.
    /// 如果给定地址没有映射设备，返回 `BusError`。
    fn load_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        self.find_region_mut(addr)
            .ok_or(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })?
            .load_u8(addr, value, cycles)
    }

    /// Host-side bulk write (loader path) / 宿主侧批量写入（加载器路径）
    ///
    /// Writes `bytes` starting at `addr` through the host-write channel. The
    /// default implementation forwards to [`Self::load_u8`] once per byte, so a
    /// bus that does not specialise it keeps byte-exact cycles, watchpoints and
    /// error reporting. Buses that can prove a region accepts the whole span
    /// override it with a bulk region transfer.
    /// 经宿主写入通道把 `bytes` 写到 `addr`。默认实现逐字节转发到
    /// [`Self::load_u8`]，因此未特化的总线保持逐字节的周期、监视点与错误语义；
    /// 能证明区域接受整段写入的总线改为一次区域批量传输。
    ///
    /// # Errors
    /// Returns `BusError` if no device is mapped at the given address.
    /// 如果给定地址没有映射设备，返回 `BusError`。
    fn load_bytes(&mut self, addr: Address, bytes: &[u8], cycles: &mut u64) -> BusResult<()> {
        for (offset, byte) in bytes.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let offset = offset as u32;
            self.load_u8(addr.wrapping_add(offset), *byte, cycles)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod stall_tests {
    //! Bus-stall contract tests / 总线 stall 契约测试
    //!
    //! They prove the two properties the sleep path depends on: the default is
    //! "never stalls" (no behaviour change for existing regions) and a stalled
    //! read is rejected **before** the region is touched.
    //! 它们验证睡眠路径依赖的两条性质：默认「永不 stall」（既有区域零行为变化），
    //! 以及被 stall 的读取在触碰区域**之前**就被拒绝。

    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    use super::*;
    use crate::memory::{RamRegion, SimpleBus};

    /// RAM region that reports a stall while `blocked` is set
    /// 在 `blocked` 置位时报告 stall 的 RAM 区域
    struct BlockingRegion {
        ram: RamRegion,
        blocked: Arc<AtomicBool>,
        reads: Arc<AtomicU32>,
    }

    impl MemoryRegion for BlockingRegion {
        fn name(&self) -> &str {
            self.ram.name()
        }

        fn base(&self) -> Address {
            self.ram.base()
        }

        fn size(&self) -> usize {
            self.ram.size()
        }

        fn read_would_stall(&self, _addr: Address) -> bool {
            self.blocked.load(Ordering::Relaxed)
        }

        fn read_u8(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            self.ram.read_u8(addr, cycles)
        }

        fn write_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
            self.ram.write_u8(addr, value, cycles)
        }
    }

    fn blocking_bus() -> (SimpleBus, Arc<AtomicBool>, Arc<AtomicU32>) {
        let blocked = Arc::new(AtomicBool::new(false));
        let reads = Arc::new(AtomicU32::new(0));
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(BlockingRegion {
            ram: RamRegion::new("blocking", 0x1000_0000, 0x100),
            blocked: Arc::clone(&blocked),
            reads: Arc::clone(&reads),
        }))
        .expect("region");
        (bus, blocked, reads)
    }

    #[test]
    fn default_region_never_stalls() {
        let ram = RamRegion::new("ram", 0x2000_0000, 16);
        assert!(
            !ram.read_would_stall(0x2000_0000),
            "the trait default must never stall / trait 默认必须永不 stall"
        );
    }

    #[test]
    fn default_bus_read_path_is_unchanged() {
        // 一个从不 stall 的总线上，32 位读仍是成功的一字读取。
        // On a bus whose region never stalls, a word read is one successful read.
        let (mut bus, _blocked, reads) = blocking_bus();
        let mut cycles = 0u64;
        assert_eq!(bus.read_u32(0x1000_0000, &mut cycles).expect("read"), 0);
        assert_eq!(reads.load(Ordering::Relaxed), 4);
        assert_eq!(cycles, 4, "one word read books four byte cycles");
    }

    #[test]
    fn stalled_read_reports_the_stall_kind() {
        let (mut bus, blocked, _reads) = blocking_bus();
        blocked.store(true, Ordering::Relaxed);
        let mut cycles = 0u64;
        let error = bus.read_u32(0x1000_0000, &mut cycles).expect_err("stall");
        assert!(error.is_stall());
        assert!(matches!(
            error,
            CoreError::BusError {
                kind: BusErrorKind::Stall,
                ..
            }
        ));
        assert!(error.to_string().contains("Stall"));
    }

    #[test]
    fn stalled_read_never_touches_the_region() {
        // stall 必须在任何区域代码之前拒绝读取，否则设备会被消费或锁存错误。
        // A stall must reject the read before any region code, otherwise the
        // device would be consumed or latch an error flag.
        let (mut bus, blocked, reads) = blocking_bus();
        blocked.store(true, Ordering::Relaxed);
        let mut cycles = 0u64;
        for _ in 0..3 {
            let _ = bus.read_u32(0x1000_0000, &mut cycles);
            let _ = bus.read_u16(0x1000_0000, &mut cycles);
            let _ = bus.read_u8(0x1000_0000, &mut cycles);
        }
        assert_eq!(
            reads.load(Ordering::Relaxed),
            0,
            "no region read may run while stalled / stall 期间不得执行区域读"
        );
        assert_eq!(cycles, 0, "a stalled access books no cycles");
    }

    #[test]
    fn releasing_the_stall_serves_the_read() {
        let (mut bus, blocked, _reads) = blocking_bus();
        blocked.store(true, Ordering::Relaxed);
        let mut cycles = 0u64;
        assert!(bus.read_u32(0x1000_0000, &mut cycles).is_err());
        blocked.store(false, Ordering::Relaxed);
        assert!(bus.read_u32(0x1000_0000, &mut cycles).is_ok());
    }

    #[test]
    fn other_error_kinds_are_not_stalls() {
        let error = CoreError::BusError {
            addr: 0,
            kind: BusErrorKind::NoDevice,
        };
        assert!(!error.is_stall());
        assert!(!CoreError::InvalidIrq(300).is_stall());
    }
}
