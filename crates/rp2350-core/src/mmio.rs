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
    /// must not overlap an existing child.
    /// 挂载子设备；其窗口必须位于路由器窗口内且不与已有子设备重叠。
    ///
    /// # Errors
    /// Returns `BusError` with `Overlap` when two windows intersect, or another
    /// kind when the child window is invalid or not contained.
    /// 当两个窗口相交时返回 `Overlap`，子窗口非法或超出路由器窗口时返回其他类型。
    pub fn mount(&mut self, device: Box<dyn MmioDevice>) -> BusResult<MmioWindow> {
        let child = device.window();
        child.validate()?;
        if child.base < self.window.base || child.end() > self.window.end() {
            return Err(CoreError::BusError {
                addr: child.base,
                kind: BusErrorKind::DeviceError,
            });
        }
        let overlaps = self
            .regions
            .iter()
            .any(|region| child.base < region.window().end() && region.window().base < child.end());
        if overlaps {
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
        if let Some(index) = self.child_index(addr) {
            self.regions[index].read_u8(addr, cycles)
        } else {
            *cycles += 1;
            warn_unhandled(&mut self.warned, &self.name, addr);
            Ok(0)
        }
    }

    fn write_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        if let Some(index) = self.child_index(addr) {
            self.regions[index].write_u8(addr, value, cycles)
        } else {
            *cycles += 1;
            warn_unhandled(&mut self.warned, &self.name, addr);
            Ok(())
        }
    }

    fn read_u16(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        if let Some(index) = self.child_index(addr) {
            self.regions[index].read_u16(addr, cycles)
        } else {
            *cycles += 1;
            warn_unhandled(&mut self.warned, &self.name, addr);
            Ok(0)
        }
    }

    fn write_u16(&mut self, addr: Address, value: u16, cycles: &mut u64) -> BusResult<()> {
        if let Some(index) = self.child_index(addr) {
            self.regions[index].write_u16(addr, value, cycles)
        } else {
            *cycles += 1;
            warn_unhandled(&mut self.warned, &self.name, addr);
            Ok(())
        }
    }

    fn read_u32(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        if let Some(index) = self.child_index(addr) {
            self.regions[index].read_u32(addr, cycles)
        } else {
            *cycles += 1;
            warn_unhandled(&mut self.warned, &self.name, addr);
            Ok(0)
        }
    }

    fn write_u32(&mut self, addr: Address, value: u32, cycles: &mut u64) -> BusResult<()> {
        if let Some(index) = self.child_index(addr) {
            self.regions[index].write_u32(addr, value, cycles)
        } else {
            *cycles += 1;
            warn_unhandled(&mut self.warned, &self.name, addr);
            Ok(())
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
}
