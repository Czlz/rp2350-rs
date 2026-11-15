//! External PSRAM window behind XIP chip select 1 (CS1)
//! XIP 片选 1（CS1）背后的外部 PSRAM 窗口
//!
//! RP2350 reaches external PSRAM through the QMI on chip select 1, mapped as a
//! 16 MB alias window at `0x1100_0000`. Boards such as the WeAct Core2350B fit
//! 8 MB, so the model defaults to that size and clamps anything larger to the
//! window limit. Unlike flash, PSRAM is writable by the guest: this module
//! models it as byte-addressable RAM with one bus cycle per byte, which is all
//! the boot path needs. QMI CS1 timing/format registers (`M1_TIMING`,
//! `M1_RFMT`, `M1_RCMD`, ...) stay plain storage in [`super::xip_qmi`]; no
//! serial timing, latency or command encoding is interpreted.
//! RP2350 通过 QMI 的片选 1 访问外部 PSRAM，它作为 16 MB 别名窗口映射在
//! `0x1100_0000`。WeAct Core2350B 等板载 8 MB，因此模型默认 8 MB，超出窗口上限
//! 的大小会被钳制。与 Flash 不同，PSRAM 对客户机可写：本模块把它建模为字节可
//! 寻址 RAM，每字节一个总线周期，足以覆盖启动路径。QMI 的 CS1 时序/格式寄存器
//! （`M1_TIMING`、`M1_RFMT`、`M1_RCMD` 等）在 [`super::xip_qmi`] 中保持普通存储；
//! 不解释串行时序、延迟或命令编码。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use rp2350_core::bus::{AddressBus, BusResult, MemoryRegion};
use rp2350_core::error::{BusErrorKind, CoreError};
use rp2350_core::types::Address;

pub use super::map::{PSRAM_BASE, PSRAM_MAX_SIZE};

/// Default PSRAM size fitted by RP2350 PSRAM boards (8 MB)
/// RP2350 PSRAM 板常见的默认容量（8 MB）
pub const DEFAULT_PSRAM_SIZE: usize = 8 * 1024 * 1024;

/// Granularity PSRAM window sizes must respect (4 KiB)
/// PSRAM 窗口大小必须遵守的粒度（4 KiB）
const PSRAM_ALIGNMENT: usize = 4096;

/// Set once an invalid requested size has been reported
/// 非法请求大小是否已报告过一次
static WARNED_INVALID_SIZE: AtomicBool = AtomicBool::new(false);

/// A writable PSRAM window mapped at [`PSRAM_BASE`]
/// 映射在 [`PSRAM_BASE`] 的可写 PSRAM 窗口
///
/// Clones share one backing store, so the value returned by [`mount_psram`]
/// observes every guest write performed through the bus-registered region.
/// Guest reads and writes cost one cycle per byte (a halfword costs two and a
/// word four, matching [`rp2350_core::memory::RamRegion`]); accesses outside
/// the window report [`BusErrorKind::DeviceError`]. A window built with
/// [`Psram::read_only`] rejects guest writes with
/// [`BusErrorKind::AccessDenied`], which models a read-only CS1 window.
/// 克隆共享同一份底层存储，因此 [`mount_psram`] 返回的句柄能观察到经总线注册区域
/// 发生的每一次客户机写入。客户机读写每字节一个周期（半字两周期、字四周期，与
/// [`rp2350_core::memory::RamRegion`] 一致）；窗口外访问报
/// [`BusErrorKind::DeviceError`]。用 [`Psram::read_only`] 构造的窗口以
/// [`BusErrorKind::AccessDenied`] 拒绝客户机写入，用于模拟只读 CS1 窗口。
#[derive(Debug, Clone)]
pub struct Psram {
    name: String,
    base: Address,
    writable: bool,
    data: Arc<RwLock<Vec<u8>>>,
}

impl Psram {
    /// Create a zeroed PSRAM window of `size` bytes at [`PSRAM_BASE`]
    /// 在 [`PSRAM_BASE`] 创建 `size` 字节的清零 PSRAM 窗口
    ///
    /// A valid size is 4 KiB aligned and within `1..=PSRAM_MAX_SIZE`. Any other
    /// value is clamped to [`PSRAM_MAX_SIZE`] with a one-shot warning, since a
    /// silently truncated window would corrupt firmware layouts.
    /// 合法大小需 4 KiB 对齐且位于 `1..=PSRAM_MAX_SIZE`。其它值会被钳制到
    /// [`PSRAM_MAX_SIZE`] 并只警告一次，因为静默截断窗口会破坏固件布局。
    #[must_use]
    pub fn new(size: usize) -> Self {
        Self::with_policy(PSRAM_BASE, size, true)
    }

    /// Create the default 8 MB zeroed window / 创建默认 8 MB 清零窗口
    #[must_use]
    pub fn zeroed() -> Self {
        Self::new(DEFAULT_PSRAM_SIZE)
    }

    /// Create a read-only window that rejects guest writes
    /// 创建拒绝客户机写入的只读窗口
    ///
    /// `size` is clamped exactly like [`Psram::new`]. Hosts that need to
    /// preload contents can use a writable window and clone it read-only
    /// through the returned handle instead.
    /// `size` 的钳制规则与 [`Psram::new`] 相同。需要预置内容的宿主可以先构造可写
    /// 窗口，再由返回句柄派生只读视图。
    #[must_use]
    pub fn read_only(size: usize) -> Self {
        Self::with_policy(PSRAM_BASE, size, false)
    }

    /// Window length in bytes / 窗口长度（字节）
    #[must_use]
    pub fn len(&self) -> usize {
        self.read_store().len()
    }

    /// Whether the window holds no bytes (never for a clamped size)
    /// 窗口是否为空（钳制后的大小不会为空）
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether guest writes are accepted / 是否接受客户机写入
    #[must_use]
    pub fn is_writable(&self) -> bool {
        self.writable
    }

    /// Build a window with an explicit base and write policy
    /// 以显式基地址与写策略构造窗口
    fn with_policy(base: Address, size: usize, writable: bool) -> Self {
        Self {
            name: "psram".to_string(),
            base,
            writable,
            data: Arc::new(RwLock::new(vec![0; clamped_size(size)])),
        }
    }

    /// Translate an in-window address into a buffer offset
    /// 将窗口内地址转换为缓冲区偏移
    fn offset(&self, addr: Address) -> BusResult<usize> {
        // 窗口外访问按设备错误上报：地址本身可达，但没有对应的 PSRAM 存储单元。
        // Out-of-window accesses report a device error: the address is reachable
        // but has no backing PSRAM cell.
        if !self.contains(addr) {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::DeviceError,
            });
        }
        Ok((addr - self.base) as usize)
    }

    /// Acquire the shared store for reading / 获取共享存储的读锁
    fn read_store(&self) -> RwLockReadGuard<'_, Vec<u8>> {
        self.data.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Acquire the shared store for writing / 获取共享存储的写锁
    fn write_store(&mut self) -> RwLockWriteGuard<'_, Vec<u8>> {
        self.data.write().unwrap_or_else(PoisonError::into_inner)
    }
}

impl MemoryRegion for Psram {
    fn name(&self) -> &str {
        &self.name
    }

    fn base(&self) -> Address {
        self.base
    }

    fn size(&self) -> usize {
        self.read_store().len()
    }

    fn read_u8(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        *cycles += 1;
        let offset = self.offset(addr)?;
        let data = self.read_store();
        data.get(offset).copied().ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })
    }

    fn write_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        *cycles += 1;
        if !self.writable {
            return Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::AccessDenied,
            });
        }
        let offset = self.offset(addr)?;
        let mut data = self.write_store();
        let slot = data.get_mut(offset).ok_or(CoreError::BusError {
            addr,
            kind: BusErrorKind::DeviceError,
        })?;
        *slot = value;
        Ok(())
    }
}

/// Mount a PSRAM window at [`PSRAM_BASE`] and return a handle to its bytes
/// 在 [`PSRAM_BASE`] 挂载 PSRAM 窗口并返回其字节句柄
///
/// The bus receives a clone that shares the returned handle's backing store, so
/// host reads through the handle observe guest writes and vice versa. Mounting
/// is independent of [`super::boot::BootPeripherals::mount`]: neither touches
/// `0x1100_0000`, so the two may be installed in any order.
/// 总线获得的是与返回句柄共享底层存储的克隆，因此宿主经句柄的读取能看到客户机
/// 写入，反之亦然。挂载与 [`super::boot::BootPeripherals::mount`] 无关：两者都不
/// 触碰 `0x1100_0000`，因此可按任意顺序安装。
///
/// # Errors
/// Returns a bus error when the window overlaps a region already registered on
/// `bus` (for example a second PSRAM window).
/// 窗口与 `bus` 上已注册区域重叠（例如第二个 PSRAM 窗口）时返回总线错误。
pub fn mount_psram(bus: &mut dyn AddressBus, size: usize) -> BusResult<Psram> {
    let psram = Psram::new(size);
    bus.register_region(Box::new(psram.clone()))?;
    Ok(psram)
}

/// Clamp a requested window size to the modelled limits
/// 将请求的窗口大小钳制到建模范围
fn clamped_size(requested: usize) -> usize {
    if requested % PSRAM_ALIGNMENT == 0 && requested > 0 && requested <= PSRAM_MAX_SIZE {
        return requested;
    }
    // 只警告一次，避免反复构造窗口时刷屏。
    // Warn once so repeated window construction cannot flood the log.
    if !WARNED_INVALID_SIZE.swap(true, Ordering::Relaxed) {
        log::warn!(
            "psram: requested size {requested:#X} is not 4 KiB aligned within 1..={PSRAM_MAX_SIZE:#X}; clamping"
        );
    }
    PSRAM_MAX_SIZE
}
