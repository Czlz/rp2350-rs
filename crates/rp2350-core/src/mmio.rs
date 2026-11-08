//! Generic memory-mapped I/O framework for peripheral devices
//! 面向外设设备的通用内存映射 I/O 框架
//!
//! # Mounting model / 挂载模型
//!
//! A device declares a fixed datasheet window ([`MmioWindow`]) and implements
//! [`MmioDevice`]. [`mount_device`] wraps it in [`MmioRegion`] and registers the
//! resulting bus region, rejecting overlaps. Devices that live inside a larger
//! address span (for example the APB block) are mounted into [`MmioRouter`],
//! which dispatches by offset and absorbs unmodelled offsets with a one-shot
//! warning instead of a data abort.
//! 设备声明固定的数据手册窗口（[`MmioWindow`]）并实现 [`MmioDevice`]。
//! [`mount_device`] 将其包装为 [`MmioRegion`] 并注册到总线，重叠会被拒绝。
//! 位于更大地址跨度内的设备（例如 APB 块）挂载到 [`MmioRouter`]，由其按偏移
//! 分发；未建模的偏移只记录一次警告并返回零，而不是产生数据中止。
//!
//! External crates (for example a USB controller) only need [`MmioDevice`] and
//! [`mount_device`]; they never depend on the peripheral collection.
//! 外部 crate（例如 USB 控制器）只需实现 [`MmioDevice`] 并调用 [`mount_device`]，
//! 不需要依赖外设集合。
//!
//! # Atomic aliases / 原子别名
//!
//! RP2350 APB registers are grouped in [`MMIO_BLOCK_SIZE`] blocks: the first
//! 4 KB holds the registers, and the next three 4 KB windows alias the same
//! registers with XOR (`+0x1000`), SET (`+0x2000`) and CLR (`+0x3000`)
//! semantics. [`MmioRouter`] folds those windows into one read-modify-write on
//! the child device, so devices never see an alias offset. Reads through an
//! alias return the underlying register value. A child window that would cross
//! its own or another child's alias span is rejected at mount time.
//! RP2350 APB 寄存器按 [`MMIO_BLOCK_SIZE`] 分块：首个 4 KB 是寄存器本体，随后
//! 三个 4 KB 窗口以 XOR（`+0x1000`）、SET（`+0x2000`）、CLR（`+0x3000`）语义
//! 别名同一批寄存器。[`MmioRouter`] 把这些窗口折叠成对子设备的一次读-改-写，
//! 设备本身不会看到别名偏移；经别名读回的是底层寄存器当前值。若子窗口会跨越
//! 自己或其它子设备的别名跨度，挂载时直接报错。

// MMIO 框架涉及地址与寄存器偏移之间的定宽换算，这是内存映射层的固有特性。
// The MMIO framework converts between addresses and register offsets, which is
// inherent to the memory-mapped layer.
#![allow(clippy::cast_possible_truncation)]

use std::collections::BTreeSet;

use super::bus::{AddressBus, BusResult, MemoryRegion};
use super::error::{BusErrorKind, CoreError};
use super::types::Address;

/// Register granularity of an MMIO window in bytes / MMIO 窗口的寄存器粒度（字节）
pub const MMIO_REGISTER_SIZE: u32 = 4;

/// Alignment of one APB register block including its alias windows
/// 含别名窗口的 APB 寄存器块对齐
pub const MMIO_BLOCK_SIZE: u32 = 0x4000;

/// Offset of the XOR alias window inside a block / 块内 XOR 别名窗口偏移
pub const MMIO_ALIAS_XOR: u32 = 0x1000;

/// Offset of the SET alias window inside a block / 块内 SET 别名窗口偏移
pub const MMIO_ALIAS_SET: u32 = 0x2000;

/// Offset of the CLR alias window inside a block / 块内 CLR 别名窗口偏移
pub const MMIO_ALIAS_CLR: u32 = 0x3000;

/// Atomic write alias applied to a register / 施加到寄存器上的原子写别名
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AliasOp {
    /// `underlying ^= value` / 异或
    Xor,
    /// `underlying |= value` / 置位
    Set,
    /// `underlying &= !value` / 清位
    Clr,
}

impl AliasOp {
    /// Fold `value` into the current register content / 将 `value` 合成进当前寄存器值
    fn apply(self, current: u32, value: u32) -> u32 {
        match self {
            Self::Xor => current ^ value,
            Self::Set => current | value,
            Self::Clr => current & !value,
        }
    }
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

/// An address window owned by one MMIO device / 单个 MMIO 设备拥有的地址窗口
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MmioWindow {
    /// First address of the window / 窗口起始地址
    pub base: Address,
    /// Window size in bytes / 窗口大小（字节）
    pub size: usize,
}

impl MmioWindow {
    /// Create a window descriptor / 创建窗口描述
    #[must_use]
    pub const fn new(base: Address, size: usize) -> Self {
        Self { base, size }
    }

    /// End address (exclusive) / 结束地址（不含）
    #[must_use]
    pub fn end(&self) -> Address {
        self.base.wrapping_add(self.size as u32)
    }

    /// Check whether `addr` falls inside the window / 检查地址是否位于窗口内
    #[must_use]
    pub fn contains(&self, addr: Address) -> bool {
        addr >= self.base && addr < self.end()
    }

    /// Convert an absolute address into a window offset / 将绝对地址转换为窗口偏移
    #[must_use]
    pub fn offset(&self, addr: Address) -> Option<u32> {
        if self.contains(addr) {
            Some(addr - self.base)
        } else {
            None
        }
    }

    /// Validate the window for registration / 校验窗口是否可注册
    ///
    /// The base must be 4-byte aligned, the size non-zero and a multiple of 4,
    /// and `base + size` must not wrap the 32-bit address space.
    /// 基地址必须 4 字节对齐，大小非零且为 4 的倍数，且 `base + size` 不能
    /// 跨越 32 位地址空间。
    ///
    /// # Errors
    /// Returns a bus error describing the first violation.
    /// 返回描述首个违规项的总线错误。
    pub fn validate(&self) -> BusResult<()> {
        if self.base % MMIO_REGISTER_SIZE != 0 {
            return Err(CoreError::BusError {
                addr: self.base,
                kind: BusErrorKind::Unaligned,
            });
        }
        if self.size == 0 || self.size % MMIO_REGISTER_SIZE as usize != 0 {
            return Err(CoreError::BusError {
                addr: self.base,
                kind: BusErrorKind::DeviceError,
            });
        }
        if u64::from(self.base) + self.size as u64 > u64::from(u32::MAX) + 1 {
            return Err(CoreError::BusError {
                addr: self.base,
                kind: BusErrorKind::DeviceError,
            });
        }
        Ok(())
    }
}

/// A memory-mapped device with 32-bit register semantics
/// 具有 32 位寄存器语义的内存映射设备
///
/// Register accesses outside the device model return `None`/`false`; the bus
/// adapter then records a one-shot warning and reads zero.
/// 设备模型之外的寄存器访问返回 `None`/`false`，总线适配器随后记录一次警告
/// 并读出零。
pub trait MmioDevice: Send + Sync {
    /// Device name used in logs / 日志中使用的设备名称
    fn name(&self) -> &str;

    /// Fixed address window declared by the device / 设备声明的固定地址窗口
    fn window(&self) -> MmioWindow;

    /// Read a 32-bit register at `offset` (relative to the window base)
    /// 读取相对窗口基地址偏移为 `offset` 的 32 位寄存器
    ///
    /// Returns `None` when the offset is not modelled.
    /// 偏移未建模时返回 `None`。
    fn read32(&mut self, offset: u32) -> Option<u32>;

    /// Write a 32-bit register at `offset` / 写入偏移为 `offset` 的 32 位寄存器
    ///
    /// Returns `false` when the offset is not modelled.
    /// 偏移未建模时返回 `false`。
    fn write32(&mut self, offset: u32, value: u32) -> bool;

    /// Restore power-on defaults / 恢复上电默认值
    fn reset(&mut self) {}

    /// Interrupt line raised by this device, if any / 该设备产生的中断线（如果有）
    fn irq_number(&self) -> Option<u16> {
        None
    }
}

/// A flat 32-bit register file used by simple peripheral stubs
/// 简单外设桩使用的扁平 32 位寄存器文件
#[derive(Debug, Clone)]
pub struct RegisterFile {
    words: Vec<u32>,
}

impl RegisterFile {
    /// Create a zeroed register file covering `size_bytes` / 创建覆盖指定字节数的清零寄存器文件
    #[must_use]
    pub fn new(size_bytes: usize) -> Self {
        Self {
            words: vec![0; size_bytes.div_ceil(MMIO_REGISTER_SIZE as usize)],
        }
    }

    /// Read a register, or `None` when the offset is outside the file
    /// 读取寄存器；偏移超出文件时返回 `None`
    #[must_use]
    pub fn read(&self, offset: u32) -> Option<u32> {
        self.words
            .get((offset / MMIO_REGISTER_SIZE) as usize)
            .copied()
    }

    /// Write a register, returning `false` when the offset is outside the file
    /// 写入寄存器；偏移超出文件时返回 `false`
    pub fn write(&mut self, offset: u32, value: u32) -> bool {
        let Some(slot) = self.words.get_mut((offset / MMIO_REGISTER_SIZE) as usize) else {
            return false;
        };
        *slot = value;
        true
    }

    /// Fill one register / 写入单个寄存器
    pub fn fill(&mut self, offset: u32, value: u32) {
        let _ = self.write(offset, value);
    }

    /// Zero every register / 清零所有寄存器
    pub fn reset(&mut self) {
        self.words.fill(0);
    }
}

/// Bus adapter that exposes an [`MmioDevice`] as a [`MemoryRegion`]
/// 将 [`MmioDevice`] 暴露为 [`MemoryRegion`] 的总线适配器
pub struct MmioRegion {
    device: Box<dyn MmioDevice>,
    warned: BTreeSet<u32>,
}

impl MmioRegion {
    /// Wrap a device / 包装一个设备
    #[must_use]
    pub fn new(device: Box<dyn MmioDevice>) -> Self {
        Self {
            device,
            warned: BTreeSet::new(),
        }
    }

    /// Device name / 设备名称
    #[must_use]
    pub fn name(&self) -> &str {
        self.device.name()
    }

    /// Device window / 设备窗口
    #[must_use]
    pub fn window(&self) -> MmioWindow {
        self.device.window()
    }

    /// Access the wrapped device / 访问被包装的设备
    #[must_use]
    pub fn device(&self) -> &dyn MmioDevice {
        self.device.as_ref()
    }

    /// Mutable access to the wrapped device / 可变访问被包装的设备
    pub fn device_mut(&mut self) -> &mut dyn MmioDevice {
        self.device.as_mut()
    }

    /// Restore the device to power-on defaults / 将设备恢复到上电默认值
    pub fn reset(&mut self) {
        self.device.reset();
    }

    /// Read a full word and log unmodelled offsets once / 读取整字并对未建模偏移记录一次
    fn read_word(&mut self, addr: Address) -> u32 {
        let offset = addr - self.window().base;
        if let Some(value) = self.device.read32(offset) {
            return value;
        }
        warn_unhandled(&mut self.warned, self.device.name(), addr);
        0
    }

    /// Write a full word and log unmodelled offsets once / 写入整字并对未建模偏移记录一次
    fn write_word(&mut self, addr: Address, value: u32) {
        let offset = addr - self.window().base;
        if !self.device.write32(offset, value) {
            warn_unhandled(&mut self.warned, self.device.name(), addr);
        }
    }
}

impl MemoryRegion for MmioRegion {
    fn name(&self) -> &str {
        self.device.name()
    }

    fn base(&self) -> Address {
        self.window().base
    }

    fn size(&self) -> usize {
        self.window().size
    }

    fn read_u8(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        *cycles += 1;
        let offset = addr - self.window().base;
        let word_offset = offset - offset % MMIO_REGISTER_SIZE;
        let word = self.read_word(self.window().base + word_offset);
        let shift = (offset % MMIO_REGISTER_SIZE) * 8;
        Ok((word >> shift) as u8)
    }

    fn write_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        *cycles += 1;
        let offset = addr - self.window().base;
        let word_offset = offset - offset % MMIO_REGISTER_SIZE;
        let shift = (offset % MMIO_REGISTER_SIZE) * 8;
        let word_addr = self.window().base + word_offset;
        let current = self.device.read32(word_offset).unwrap_or(0);
        let mask = !(0xFFu32 << shift);
        self.write_word(word_addr, (current & mask) | (u32::from(value) << shift));
        Ok(())
    }

    fn read_u16(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        *cycles += 1;
        let offset = addr - self.window().base;
        let word_offset = offset - offset % MMIO_REGISTER_SIZE;
        let word = self.read_word(self.window().base + word_offset);
        let shift = (offset % MMIO_REGISTER_SIZE) * 8;
        Ok((word >> shift) as u16)
    }

    /// Halfword writes are merged into the containing register word
    /// 半字写入会合并进所在的寄存器字，避免拆分后出现多次副作用
    fn write_u16(&mut self, addr: Address, value: u16, cycles: &mut u64) -> BusResult<()> {
        *cycles += 1;
        let offset = addr - self.window().base;
        let word_offset = offset - offset % MMIO_REGISTER_SIZE;
        let shift = (offset % MMIO_REGISTER_SIZE) * 8;
        let word_addr = self.window().base + word_offset;
        let current = self.device.read32(word_offset).unwrap_or(0);
        let mask = !(0xFFFFu32 << shift);
        self.write_word(word_addr, (current & mask) | (u32::from(value) << shift));
        Ok(())
    }

    /// A 32-bit access is one device register access, not four byte writes
    /// 一次 32 位访问等同于一次设备寄存器访问，而不是四次字节写
    fn read_u32(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        *cycles += 1;
        let offset = addr - self.window().base;
        let word_offset = offset - offset % MMIO_REGISTER_SIZE;
        Ok(self.read_word(self.window().base + word_offset))
    }

    fn write_u32(&mut self, addr: Address, value: u32, cycles: &mut u64) -> BusResult<()> {
        *cycles += 1;
        let offset = addr - self.window().base;
        let word_offset = offset - offset % MMIO_REGISTER_SIZE;
        self.write_word(self.window().base + word_offset, value);
        Ok(())
    }
}

/// A router that owns a large address span and dispatches to child devices
/// 拥有大地址跨度并向子设备分发的路由器
pub struct MmioRouter {
    name: String,
    window: MmioWindow,
    regions: Vec<MmioRegion>,
    warned: BTreeSet<u32>,
}

impl MmioRouter {
    /// Create an empty router / 创建空路由器
    #[must_use]
    pub fn new(name: impl Into<String>, window: MmioWindow) -> Self {
        Self {
            name: name.into(),
            window,
            regions: Vec::new(),
            warned: BTreeSet::new(),
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
        let conflicts = self.regions.iter().any(|region| {
            let other = region.window();
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
        self.regions.push(MmioRegion::new(device));
        Ok(child)
    }

    /// Index of the child owning `addr` / 拥有该地址的子设备索引
    fn child_index(&self, addr: Address) -> Option<usize> {
        self.regions
            .iter()
            .position(|region| region.window().contains(addr))
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
        if let Some(index) = self.child_index(addr) {
            return Some(Resolved {
                index,
                reg_addr: addr,
                op: None,
            });
        }
        for (index, region) in self.regions.iter().enumerate() {
            let block = region.window().base & !(MMIO_BLOCK_SIZE - 1);
            let (op, alias_base) = match addr.wrapping_sub(block) {
                MMIO_ALIAS_XOR..MMIO_ALIAS_SET => (AliasOp::Xor, MMIO_ALIAS_XOR),
                MMIO_ALIAS_SET..MMIO_ALIAS_CLR => (AliasOp::Set, MMIO_ALIAS_SET),
                MMIO_ALIAS_CLR..MMIO_BLOCK_SIZE => (AliasOp::Clr, MMIO_ALIAS_CLR),
                _ => continue,
            };
            let reg_addr = addr - alias_base;
            if region.window().contains(reg_addr) {
                return Some(Resolved {
                    index,
                    reg_addr,
                    op: Some(op),
                });
            }
        }
        None
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

    fn read_u8(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        match self.resolve(addr) {
            Some(Resolved {
                index,
                reg_addr,
                op: None,
            }) => self.regions[index].read_u8(reg_addr, cycles),
            Some(Resolved {
                index,
                reg_addr,
                op: Some(_),
            }) => {
                let word = self.regions[index].read_u32(reg_addr & !3, cycles)?;
                Ok((word >> ((reg_addr & 3) * 8)) as u8)
            }
            None => {
                *cycles += 1;
                warn_unhandled(&mut self.warned, &self.name, addr);
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
            }) => self.regions[index].write_u8(reg_addr, value, cycles),
            Some(Resolved {
                index,
                reg_addr,
                op: Some(op),
            }) => {
                let word_addr = reg_addr & !3;
                let current = self.regions[index].read_u32(word_addr, cycles)?;
                let updated = op.apply(current, u32::from(value) << ((reg_addr & 3) * 8));
                self.regions[index].write_u32(word_addr, updated, cycles)
            }
            None => {
                *cycles += 1;
                warn_unhandled(&mut self.warned, &self.name, addr);
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
            }) => self.regions[index].read_u16(reg_addr, cycles),
            Some(Resolved {
                index,
                reg_addr,
                op: Some(_),
            }) => {
                let word = self.regions[index].read_u32(reg_addr & !3, cycles)?;
                Ok((word >> ((reg_addr & 3) * 8)) as u16)
            }
            None => {
                *cycles += 1;
                warn_unhandled(&mut self.warned, &self.name, addr);
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
            }) => self.regions[index].write_u16(reg_addr, value, cycles),
            Some(Resolved {
                index,
                reg_addr,
                op: Some(op),
            }) => {
                let word_addr = reg_addr & !3;
                let current = self.regions[index].read_u32(word_addr, cycles)?;
                let updated = op.apply(current, u32::from(value) << ((reg_addr & 3) * 8));
                self.regions[index].write_u32(word_addr, updated, cycles)
            }
            None => {
                *cycles += 1;
                warn_unhandled(&mut self.warned, &self.name, addr);
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
            self.regions[index].read_u32(reg_addr, cycles)
        } else {
            *cycles += 1;
            warn_unhandled(&mut self.warned, &self.name, addr);
            Ok(0)
        }
    }

    fn write_u32(&mut self, addr: Address, value: u32, cycles: &mut u64) -> BusResult<()> {
        match self.resolve(addr) {
            Some(Resolved {
                index,
                reg_addr,
                op: None,
            }) => self.regions[index].write_u32(reg_addr, value, cycles),
            Some(Resolved {
                index,
                reg_addr,
                op: Some(op),
            }) => {
                let current = self.regions[index].read_u32(reg_addr, cycles)?;
                self.regions[index].write_u32(reg_addr, op.apply(current, value), cycles)
            }
            None => {
                *cycles += 1;
                warn_unhandled(&mut self.warned, &self.name, addr);
                Ok(())
            }
        }
    }
}

/// Mount one device window on a bus / 将一个设备窗口挂载到总线
///
/// The bus rejects overlaps, so callers see an explicit error instead of two
/// devices silently sharing an address range.
/// 总线会拒绝重叠，调用方因此会看到明确错误，而不是两个设备静默共享地址范围。
///
/// # Errors
/// Returns `BusError` when the device window is invalid or overlaps a region
/// already registered on `bus`.
/// 设备窗口非法或与总线已注册区域重叠时返回 `BusError`。
pub fn mount_device(
    bus: &mut dyn AddressBus,
    device: Box<dyn MmioDevice>,
) -> BusResult<MmioWindow> {
    let window = device.window();
    window.validate()?;
    bus.register_region(Box::new(MmioRegion::new(device)))?;
    Ok(window)
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

/// Emit one warning per unmodelled address / 对每个未建模地址只警告一次
fn warn_unhandled(warned: &mut BTreeSet<u32>, owner: &str, addr: Address) {
    if warned.insert(addr) {
        log::warn!(
            "{owner}: unimplemented MMIO access at {addr:#010X} (read as 0, write ignored) / \
             {owner}: 未实现的 MMIO 访问 {addr:#010X}（读为 0，写被忽略）"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SimpleBus;

    struct Dummy {
        window: MmioWindow,
        value: u32,
    }

    impl MmioDevice for Dummy {
        // 测试桩返回字面量名称，与 trait 的省略生命周期签名一致。
        // The test stub returns a literal name, matching the trait's elided signature.
        #[allow(clippy::unnecessary_literal_bound)]
        fn name(&self) -> &str {
            "dummy"
        }

        fn window(&self) -> MmioWindow {
            self.window
        }

        fn read32(&mut self, offset: u32) -> Option<u32> {
            (offset == 0).then_some(self.value)
        }

        fn write32(&mut self, offset: u32, value: u32) -> bool {
            if offset == 0 {
                self.value = value;
                return true;
            }
            false
        }
    }

    fn dummy() -> Box<dyn MmioDevice> {
        Box::new(Dummy {
            window: MmioWindow::new(0x4000_0000, 0x1000),
            value: 0,
        })
    }

    fn dummy_at(base: u32, size: usize) -> Box<dyn MmioDevice> {
        Box::new(Dummy {
            window: MmioWindow::new(base, size),
            value: 0,
        })
    }

    #[test]
    fn word_read_and_write_round_trip() {
        let mut region = MmioRegion::new(dummy());
        let mut cycles = 0;
        region
            .write_u32(0x4000_0000, 0x1234_5678, &mut cycles)
            .unwrap();
        assert_eq!(
            region.read_u32(0x4000_0000, &mut cycles).unwrap(),
            0x1234_5678
        );
    }

    #[test]
    fn byte_access_extracts_and_merges() {
        let mut region = MmioRegion::new(dummy());
        let mut cycles = 0;
        region
            .write_u32(0x4000_0000, 0x1122_3344, &mut cycles)
            .unwrap();
        assert_eq!(region.read_u8(0x4000_0002, &mut cycles).unwrap(), 0x22);
        region.write_u8(0x4000_0003, 0xAA, &mut cycles).unwrap();
        assert_eq!(
            region.read_u32(0x4000_0000, &mut cycles).unwrap(),
            0xAA22_3344
        );
    }

    #[test]
    fn unmodelled_offset_reads_zero() {
        let mut region = MmioRegion::new(dummy());
        let mut cycles = 0;
        assert_eq!(region.read_u32(0x4000_0008, &mut cycles).unwrap(), 0);
        region.write_u32(0x4000_0008, 0xFF, &mut cycles).unwrap();
        assert_eq!(region.read_u32(0x4000_0000, &mut cycles).unwrap(), 0);
    }

    #[test]
    fn router_dispatches_and_rejects_overlap() {
        let mut router = MmioRouter::new("apb", MmioWindow::new(0x4000_0000, 0x10_0000));
        router.mount(dummy()).unwrap();
        assert_eq!(router.device_count(), 1);
        assert!(router.mount(dummy()).is_err());

        let mut cycles = 0;
        router.write_u32(0x4000_0000, 0x55, &mut cycles).unwrap();
        assert_eq!(router.read_u32(0x4000_0000, &mut cycles).unwrap(), 0x55);
        // Unmapped APB offset reads as zero instead of halting the bus.
        // 未映射的 APB 偏移读出零，而不是让总线中止。
        assert_eq!(router.read_u32(0x400F_0000, &mut cycles).unwrap(), 0);
    }

    #[test]
    fn mount_device_rejects_outside_window() {
        let mut bus = SimpleBus::new();
        assert!(mount_device(&mut bus, dummy()).is_ok());
        assert!(mount_device(&mut bus, dummy()).is_err());
    }

    #[test]
    fn window_validation() {
        assert!(MmioWindow::new(0x4000_0001, 4).validate().is_err());
        assert!(MmioWindow::new(0x4000_0000, 0).validate().is_err());
        assert!(MmioWindow::new(0x4000_0000, 2).validate().is_err());
        assert!(MmioWindow::new(0xFFFF_F000, 0x2000).validate().is_err());
        assert!(MmioWindow::new(0x4000_0000, 0x1000).validate().is_ok());
    }

    #[test]
    fn alias_writes_fold_into_one_register() {
        let mut router = MmioRouter::new("apb", MmioWindow::new(0x4000_0000, 0x10_0000));
        router.mount(dummy()).unwrap();
        let mut cycles = 0;
        router
            .write_u32(0x4000_0000, 0x0000_000F, &mut cycles)
            .unwrap();
        router
            .write_u32(0x4000_2000, 0x0000_00F0, &mut cycles)
            .unwrap();
        assert_eq!(
            router.read_u32(0x4000_0000, &mut cycles).unwrap(),
            0x0000_00FF,
            "SET alias must OR into the register"
        );
        router
            .write_u32(0x4000_3000, 0x0000_000F, &mut cycles)
            .unwrap();
        assert_eq!(
            router.read_u32(0x4000_0000, &mut cycles).unwrap(),
            0x0000_00F0,
            "CLR alias must clear bits"
        );
        router
            .write_u32(0x4000_1000, 0x0000_00FF, &mut cycles)
            .unwrap();
        assert_eq!(
            router.read_u32(0x4000_0000, &mut cycles).unwrap(),
            0x0000_000F,
            "XOR alias must toggle bits"
        );
    }

    #[test]
    fn alias_reads_return_the_underlying_value() {
        let mut router = MmioRouter::new("apb", MmioWindow::new(0x4000_0000, 0x10_0000));
        router.mount(dummy()).unwrap();
        let mut cycles = 0;
        router
            .write_u32(0x4000_0000, 0xA5A5_5A5A, &mut cycles)
            .unwrap();
        assert_eq!(
            router.read_u32(0x4000_1000, &mut cycles).unwrap(),
            0xA5A5_5A5A
        );
        assert_eq!(
            router.read_u32(0x4000_2000, &mut cycles).unwrap(),
            0xA5A5_5A5A
        );
        assert_eq!(
            router.read_u32(0x4000_3000, &mut cycles).unwrap(),
            0xA5A5_5A5A
        );
    }

    #[test]
    fn alias_spans_reject_conflicting_windows() {
        let mut router = MmioRouter::new("apb", MmioWindow::new(0x4000_0000, 0x10_0000));
        router.mount(dummy()).unwrap();
        // 落进已挂载设备的别名区。Lands inside a mounted device's alias span.
        assert!(router.mount(dummy_at(0x4000_2000, 0x1000)).is_err());

        let mut fresh = MmioRouter::new("apb", MmioWindow::new(0x4000_0000, 0x10_0000));
        // 跨进自己所在块的别名区。Crosses its own block's alias span.
        assert!(fresh.mount(dummy_at(0x4000_0800, 0x1000)).is_err());
        // 只占块内前 4 KB 则合法。Staying inside the first 4 KB is accepted.
        assert!(fresh.mount(dummy_at(0x4000_0800, 0x800)).is_ok());
    }
}
