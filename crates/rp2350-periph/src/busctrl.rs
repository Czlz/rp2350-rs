//! BUSCTRL register block / BUSCTRL 寄存器块
//!
//! Models `BUSCTRL` (`0x4006_8000`): the `BUS_PRIORITY` master priority hints,
//! the `PERFCTR_EN` gate, the two bus-fabric performance counters
//! (`PERFCTR0/1` with `PERFSEL0/1`) and the `BUS_PRIORITY_ACK` handshake.
//! 建模 `BUSCTRL`（`0x4006_8000`）：`BUS_PRIORITY` 主设备优先级提示、
//! `PERFCTR_EN` 门控、两个总线矩阵性能计数器（`PERFCTR0/1` 与 `PERFSEL0/1`）
//! 以及 `BUS_PRIORITY_ACK` 握手。
//!
//! # Performance counters / 性能计数器
//!
//! Each counter is a 24-bit saturating counter fed by one crossbar event chosen
//! by its `PERFSEL` register. The datasheet enumerates 17 downstream ports with
//! four events each: `STALL_UPSTREAM` (cycles any master stalled),
//! `STALL_DOWNSTREAM` (cycles a downstream port stalled), `ACCESS_CONTESTED`
//! (an access that had previously stalled) and `ACCESS` (an access took
//! place). Event code `n` therefore means port `n / 4`, kind `n % 4`.
//! Counters increment only while `PERFCTR_EN[0]` is set and saturate at
//! `0x00FF_FFFF`; a write of **any** value to `PERFCTR0/1` clears that counter
//! (the datasheet declares only that write-clear, so changing `PERFSEL` does
//! **not** clear the count — write `PERFCTR` for that).
//! 每个计数器是 24 位饱和计数器，由 `PERFSEL` 选中的交叉开关事件驱动。数据手册为
//! 17 个下游端口各定义四种事件：`STALL_UPSTREAM`（任一主设备 stall 的周期）、
//! `STALL_DOWNSTREAM`（下游端口 stall 的周期）、`ACCESS_CONTESTED`（此前曾因争用
//! 而 stall 的访问）与 `ACCESS`（发生了一次访问）。因此事件码 `n` 表示端口 `n / 4`、
//! 种类 `n % 4`。计数器只在 `PERFCTR_EN[0]` 置位时递增，并在 `0x00FF_FFFF` 饱和；
//! 向 `PERFCTR0/1` 写**任意**值都会清零该计数器（数据手册只规定这一种清零方式，
//! 因此改变 `PERFSEL` **不会**清零计数——需要清零时写 `PERFCTR`）。
//!
//! # What the model can observe / 模型可观测的事件
//!
//! The model has no cycle-exact crossbar, so only completed master accesses are
//! countable. [`Dma`](crate::dma::Dma) reports every read and write it performs
//! together with the bus cycles the access consumed, and
//! [`port_of_address`] attributes each access to its downstream port:
//! `ACCESS` counts accesses, `STALL_UPSTREAM` counts the consumed cycles.
//! `STALL_DOWNSTREAM` and `ACCESS_CONTESTED` need a multi-master arbiter that
//! can stall an upstream access; they stay 0 and are documented as not
//! modelled. CPU (`siob_proc0/1`) events are likewise not observable from this
//! crate because CPU accesses do not pass through the peripheral models.
//! 本模型没有周期精确的交叉开关，只能统计已完成的主设备访问。
//! [`Dma`](crate::dma::Dma) 会报告它执行的每次读/写以及该访问消耗的总线周期，
//! 由 [`port_of_address`] 归属到下游端口：`ACCESS` 统计访问次数，
//! `STALL_UPSTREAM` 统计消耗的周期。`STALL_DOWNSTREAM` 与 `ACCESS_CONTESTED`
//! 需要能 stall 上游访问的多主设备仲裁器，保持 0 并记录为未建模。CPU
//! （`siob_proc0/1`）事件同样不可观测，因为 CPU 访问不经过外设模型。
//!
//! # Sharing counters / 共享计数器
//!
//! [`BusPerf`] is a cloneable handle to the counter bank. [`BusCtrl`] owns one
//! and exposes it through [`BusCtrl::perf`]; a DMA controller constructed with
//! [`Dma::with_perf`](crate::dma::Dma::with_perf) reports into the same bank, so
//! firmware reads live values through `PERFCTR0/1`. [`BusCtrl::new`] creates a
//! private bank, so a `BusCtrl` that is never wired still stores
//! `PERFSEL`/`PERFCTR_EN`/`PERFCTR` correctly and simply counts nothing.
//! [`BusPerf`] 是计数器组的可克隆句柄。[`BusCtrl`] 持有一个并经
//! [`BusCtrl::perf`] 暴露；用 [`Dma::with_perf`](crate::dma::Dma::with_perf)
//! 构造的 DMA 控制器向同一组计数器上报，因此固件通过 `PERFCTR0/1` 读到实时值。
//! [`BusCtrl::new`] 创建私有计数器组，因此从未接线的 `BusCtrl` 仍能正确保存
//! `PERFSEL`/`PERFCTR_EN`/`PERFCTR`，只是不计数。
//!
//! # Arbitration accounting / 仲裁记账
//!
//! [`BusPerf`] also keeps monotonic, non-register counters the host can query:
//! DMA read/write units, DMA bus cycles, arbitration grants and per-port access
//! and stall totals ([`BusPerfSnapshot`]). [`BusCtrl::dma_occupancy`] turns the
//! DMA bus cycles into parts-per-thousand of the caller-supplied total cycle
//! count, which is how "who held the bus" becomes observable without a
//! cycle-exact arbiter.
//! [`BusPerf`] 还保存宿主可查询的单调非寄存器计数器：DMA 读/写单元数、DMA 总线
//! 周期、仲裁授予次数以及每端口访问与 stall 总数（[`BusPerfSnapshot`]）。
//! [`BusCtrl::dma_occupancy`] 把 DMA 总线周期换算为相对于调用方给出的总周期数的
//! 千分比，从而在没有周期精确仲裁器的情况下也能观测「谁占用了总线」。

use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use crate::alias::{apply_alias, split_alias};
use crate::map;

/// `BUS_PRIORITY` offset / `BUS_PRIORITY` 偏移
pub const BUS_PRIORITY: u32 = 0x00;

/// `BUS_PRIORITY_ACK` offset / `BUS_PRIORITY_ACK` 偏移
pub const BUS_PRIORITY_ACK: u32 = 0x04;

/// `PERFCTR_EN` offset / `PERFCTR_EN` 偏移
pub const PERFCTR_EN: u32 = 0x08;

/// `PERFCTR0` offset / `PERFCTR0` 偏移
pub const PERFCTR0: u32 = 0x0C;

/// `PERFSEL0` offset / `PERFSEL0` 偏移
pub const PERFSEL0: u32 = 0x10;

/// `PERFCTR1` offset / `PERFCTR1` 偏移
pub const PERFCTR1: u32 = 0x14;

/// `PERFSEL1` offset / `PERFSEL1` 偏移
pub const PERFSEL1: u32 = 0x18;

/// `BUS_PRIORITY.PROC0` — core 0 priority hint / 核心 0 优先级提示
pub const PROC0_PRIORITY_MASK: u32 = 1 << 0;

/// `BUS_PRIORITY.PROC1` — core 1 priority hint / 核心 1 优先级提示
pub const PROC1_PRIORITY_MASK: u32 = 1 << 4;

/// `BUS_PRIORITY.DMA_R` — DMA read priority hint / DMA 读优先级提示
pub const DMA_R_PRIORITY_MASK: u32 = 1 << 8;

/// `BUS_PRIORITY.DMA_W` — DMA write priority hint / DMA 写优先级提示
pub const DMA_W_PRIORITY_MASK: u32 = 1 << 12;

/// All writable `BUS_PRIORITY` bits / `BUS_PRIORITY` 全部可写位
pub const BUS_PRIORITY_MASK: u32 = 0x1111;

/// `PERFSELn` field mask (7 bits) / `PERFSELn` 字段掩码（7 位）
pub const PERFSEL_MASK: u32 = 0x7F;

/// `PERFSELn` reset value: `sram6_access` / `PERFSELn` 复位值：`sram6_access`
pub const PERFSEL_RESET: u32 = 0x1F;

/// `PERFCTRn` field mask (24-bit saturating counter)
/// `PERFCTRn` 字段掩码（24 位饱和计数器）
pub const PERFCTR_MASK: u32 = 0x00FF_FFFF;

/// `PERFSEL` event kind `STALL_UPSTREAM` / `PERFSEL` 事件种类：上游 stall
pub const PERF_EVENT_STALL_UPSTREAM: u8 = 0x0;

/// `PERFSEL` event kind `STALL_DOWNSTREAM` / `PERFSEL` 事件种类：下游 stall
pub const PERF_EVENT_STALL_DOWNSTREAM: u8 = 0x1;

/// `PERFSEL` event kind `ACCESS_CONTESTED` / `PERFSEL` 事件种类：争用访问
pub const PERF_EVENT_ACCESS_CONTESTED: u8 = 0x2;

/// `PERFSEL` event kind `ACCESS` / `PERFSEL` 事件种类：访问
pub const PERF_EVENT_ACCESS: u8 = 0x3;

/// Number of `PERFSEL` event codes (17 ports × 4 kinds)
/// `PERFSEL` 事件编码数量（17 端口 × 4 种类）
pub const PERF_EVENT_COUNT: u8 = 0x44;

/// Number of crossbar downstream ports / 交叉开关下游端口数量
pub const PORT_COUNT: usize = 17;

/// `BUSCTRL` register window size (4 KB) / `BUSCTRL` 寄存器窗口大小（4 KB）
pub const BUSCTRL_SIZE: usize = 0x1000;

/// One downstream port of the main crossbar / 主交叉开关的一个下游端口
///
/// The discriminant order is the datasheet order of the `PERFSEL` event list:
/// `siob_proc1`, `siob_proc0`, `apb`, `fastperi`, `sram9..=sram0`,
/// `xip_main1`, `xip_main0`, `rom`.
/// 序号顺序即数据手册 `PERFSEL` 事件表的顺序：`siob_proc1`、`siob_proc0`、
/// `apb`、`fastperi`、`sram9..=sram0`、`xip_main1`、`xip_main0`、`rom`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusPort {
    /// Core 1 SIO block / 核心 1 的 SIO 块
    SiobProc1,
    /// Core 0 SIO block / 核心 0 的 SIO 块
    SiobProc0,
    /// APB peripheral window / APB 外设窗口
    Apb,
    /// Fast (AHB) peripheral window / 快速（AHB）外设窗口
    FastPeri,
    /// SRAM bank `0..=9` / SRAM 存储体 `0..=9`
    Sram(u8),
    /// XIP main manager 1 (uncached CS1 window) / XIP 主管理器 1（CS1 非缓存窗口）
    XipMain1,
    /// XIP main manager 0 (cached CS0 window) / XIP 主管理器 0（CS0 缓存窗口）
    XipMain0,
    /// Boot ROM / 启动 ROM
    Rom,
}

impl BusPort {
    /// `PERFSEL` port index of this port / 该端口的 `PERFSEL` 端口序号
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::SiobProc1 => 0,
            Self::SiobProc0 => 1,
            Self::Apb => 2,
            Self::FastPeri => 3,
            // sram9 是第一个 SRAM 端口，sram0 是最后一个。
            // sram9 is the first SRAM port and sram0 the last one.
            Self::Sram(bank) => 13usize.saturating_sub(bank as usize),
            Self::XipMain1 => 14,
            Self::XipMain0 => 15,
            Self::Rom => 16,
        }
    }

    /// Port for a `PERFSEL` port index; `None` above 16
    /// 由 `PERFSEL` 端口序号反查端口；大于 16 时返回 `None`
    #[must_use]
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::SiobProc1),
            1 => Some(Self::SiobProc0),
            2 => Some(Self::Apb),
            3 => Some(Self::FastPeri),
            4..=13 => Some(Self::Sram((13 - index) as u8)),
            14 => Some(Self::XipMain1),
            15 => Some(Self::XipMain0),
            16 => Some(Self::Rom),
            _ => None,
        }
    }

    /// `PERFSEL` event code for one event kind / 某一事件种类的 `PERFSEL` 编码
    #[must_use]
    pub const fn event_code(self, kind: u8) -> u8 {
        ((self.index() as u8) << 2) | (kind & 0x3)
    }
}

/// Attribute an address to its crossbar downstream port / 把地址归属到下游端口
///
/// Returns `None` for addresses outside the modelled RP2350 map, which are not
/// counted at all. The SRAM decode follows the 520 KiB layout (banks `0..=7`
/// are 64 KiB at `0x2000_0000`, banks 8/9 are 4 KiB each at `0x2008_0000`).
/// 对不在已建模 RP2350 地址图中的地址返回 `None`（完全不计数）。SRAM 解码遵循
/// 520 KiB 布局（`0..=7` 号存储体各 64 KiB 位于 `0x2000_0000`，8/9 号各 4 KiB
/// 位于 `0x2008_0000`）。
#[must_use]
pub fn port_of_address(address: u32) -> Option<BusPort> {
    match address {
        0x0000_0000..=0x0000_7FFF => Some(BusPort::Rom),
        0x1000_0000..=0x10FF_FFFF | 0x1200_0000..=0x1FFF_FFFF => Some(BusPort::XipMain0),
        // `0x1100_0000` 是 pico-sdk `addressmap.h` 中的 XIP_MAIN1 窗口（CS1）。
        // `0x1100_0000` is the XIP_MAIN1 window (CS1) of pico-sdk `addressmap.h`.
        0x1100_0000..=0x11FF_FFFF => Some(BusPort::XipMain1),
        0x2000_0000..=0x2007_FFFF => Some(BusPort::Sram(((address >> 16) & 0x7) as u8)),
        0x2008_0000..=0x2008_0FFF => Some(BusPort::Sram(8)),
        0x2008_1000..=0x2008_1FFF => Some(BusPort::Sram(9)),
        0x4000_0000..=0x4FFF_FFFF => Some(BusPort::Apb),
        0x5000_0000..=0x5FFF_FFFF => Some(BusPort::FastPeri),
        _ => None,
    }
}

/// Read-only snapshot of the fabric counters / 总线矩阵计数器快照
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusPerfSnapshot {
    /// `PERFCTR_EN[0]` / `PERFCTR_EN[0]` 状态
    pub enabled: bool,
    /// `PERFSEL0/1` values / `PERFSEL0/1` 值
    pub selectors: [u8; 2],
    /// `PERFCTR0/1` values / `PERFCTR0/1` 值
    pub counters: [u32; 2],
    /// DMA read data units / DMA 读数据单元数
    pub dma_read_units: u64,
    /// DMA write data units / DMA 写数据单元数
    pub dma_write_units: u64,
    /// Bus cycles charged to DMA accesses / 记在 DMA 访问上的总线周期
    pub dma_cycles: u64,
    /// Arbitration grants (one per executed unit) / 仲裁授予次数（每个执行单元一次）
    pub grants: u64,
    /// Completed accesses per port / 各端口完成的访问数
    pub port_accesses: [u64; PORT_COUNT],
    /// Cycles charged per port / 各端口记入的周期
    pub port_cycles: [u64; PORT_COUNT],
}

/// Register-visible counter state / 寄存器可见的计数器状态
#[derive(Debug, Clone, Copy)]
struct PerfState {
    enabled: bool,
    selectors: [u8; 2],
    counters: [u32; 2],
    dma_read_units: u64,
    dma_write_units: u64,
    dma_cycles: u64,
    grants: u64,
    port_accesses: [u64; PORT_COUNT],
    port_cycles: [u64; PORT_COUNT],
}

impl Default for PerfState {
    fn default() -> Self {
        // PERFSEL 复位为 sram6_access，两个计数器停用（省电）。
        // PERFSEL resets to sram6_access and both counters start disabled.
        Self {
            enabled: false,
            selectors: [PERFSEL_RESET as u8; 2],
            counters: [0; 2],
            dma_read_units: 0,
            dma_write_units: 0,
            dma_cycles: 0,
            grants: 0,
            port_accesses: [0; PORT_COUNT],
            port_cycles: [0; PORT_COUNT],
        }
    }
}

impl PerfState {
    /// Reset the register-visible state / 重置寄存器可见状态
    ///
    /// The host-side arbitration totals survive a device reset: they describe
    /// the fabric, not the register block.
    /// 宿主侧仲裁总计在设备复位后保留：它们描述总线矩阵而非寄存器块。
    fn reset_registers(&mut self) {
        self.enabled = false;
        self.selectors = [PERFSEL_RESET as u8; 2];
        self.counters = [0; 2];
    }

    /// Feed one completed access into the counters / 把一个完成的访问送入计数器
    fn record(&mut self, port: BusPort, cycles: u64) {
        let index = port.index();
        self.port_accesses[index] = self.port_accesses[index].saturating_add(1);
        self.port_cycles[index] = self.port_cycles[index].saturating_add(cycles);
        if !self.enabled {
            return;
        }
        for slot in 0..2 {
            let event = usize::from(self.selectors[slot]);
            // 事件码 n 表示端口 n / 4、种类 n % 4。
            // Event code n selects port n / 4 and kind n % 4.
            if event >> 2 != index {
                continue;
            }
            let increment = match (event & 0x3) as u8 {
                PERF_EVENT_ACCESS => 1,
                PERF_EVENT_STALL_UPSTREAM => u32::try_from(cycles).unwrap_or(u32::MAX),
                // STALL_DOWNSTREAM / ACCESS_CONTESTED 需要周期精确仲裁器。
                // STALL_DOWNSTREAM / ACCESS_CONTESTED need a cycle-exact arbiter.
                _ => 0,
            };
            let counter = &mut self.counters[slot];
            *counter = counter.saturating_add(increment).min(PERFCTR_MASK);
        }
    }
}

/// Cloneable handle to the fabric counter bank / 总线矩阵计数器组的可克隆句柄
///
/// Clones share one bank, so a [`BusCtrl`] and a
/// [`Dma`](crate::dma::Dma) wired with [`BusCtrl::perf`] observe and update the
/// same counters.
/// 克隆共享同一组计数器，因此 [`BusCtrl`] 与用 [`BusCtrl::perf`] 接线的
/// [`Dma`](crate::dma::Dma) 观察并更新同一组计数器。
#[derive(Debug, Clone)]
pub struct BusPerf {
    state: Arc<Mutex<PerfState>>,
}

impl BusPerf {
    /// Create a zeroed counter bank / 创建清零的计数器组
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(PerfState::default())),
        }
    }

    /// Current `PERFCTR_EN[0]` state / 当前 `PERFCTR_EN[0]` 状态
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.state.lock().enabled
    }

    /// One `PERFSELn` value / 某个 `PERFSELn` 值
    #[must_use]
    pub fn selector(&self, slot: usize) -> u8 {
        self.state.lock().selectors.get(slot).copied().unwrap_or(0)
    }

    /// One `PERFCTRn` value / 某个 `PERFCTRn` 值
    #[must_use]
    pub fn counter(&self, slot: usize) -> u32 {
        self.state.lock().counters.get(slot).copied().unwrap_or(0)
    }

    /// Latch `PERFCTR_EN[0]` / 锁存 `PERFCTR_EN[0]`
    pub fn set_enabled(&self, enabled: bool) {
        self.state.lock().enabled = enabled;
    }

    /// Store one `PERFSELn` / 保存某个 `PERFSELn`
    ///
    /// The counter keeps its value: only a write to `PERFCTR` clears it.
    /// 计数器保持原值：只有写 `PERFCTR` 才会清零。
    pub fn set_selector(&self, slot: usize, selector: u8) {
        if let Some(entry) = self.state.lock().selectors.get_mut(slot) {
            *entry = selector & PERFSEL_MASK as u8;
        }
    }

    /// Clear one `PERFCTRn` / 清零某个 `PERFCTRn`
    pub fn clear_counter(&self, slot: usize) {
        if let Some(entry) = self.state.lock().counters.get_mut(slot) {
            *entry = 0;
        }
    }

    /// Record one arbitration grant / 记录一次仲裁授予
    pub fn record_grant(&self) {
        let mut state = self.state.lock();
        state.grants = state.grants.saturating_add(1);
    }

    /// Record one completed DMA read / 记录一次完成的 DMA 读
    ///
    /// Addresses outside the modelled map still add to the DMA unit counters
    /// but are attributed to no port.
    /// 地址图之外的地址仍计入 DMA 单元计数，但不归属任何端口。
    pub fn record_read(&self, address: u32, cycles: u64) {
        let mut state = self.state.lock();
        state.dma_read_units = state.dma_read_units.saturating_add(1);
        state.dma_cycles = state.dma_cycles.saturating_add(cycles);
        if let Some(port) = port_of_address(address) {
            state.record(port, cycles);
        }
    }

    /// Record one completed DMA write / 记录一次完成的 DMA 写
    ///
    /// See [`Self::record_read`] for the attribution rules.
    /// 归属规则见 [`Self::record_read`]。
    pub fn record_write(&self, address: u32, cycles: u64) {
        let mut state = self.state.lock();
        state.dma_write_units = state.dma_write_units.saturating_add(1);
        state.dma_cycles = state.dma_cycles.saturating_add(cycles);
        if let Some(port) = port_of_address(address) {
            state.record(port, cycles);
        }
    }

    /// Snapshot every counter / 快照全部计数器
    #[must_use]
    pub fn snapshot(&self) -> BusPerfSnapshot {
        let state = self.state.lock();
        BusPerfSnapshot {
            enabled: state.enabled,
            selectors: state.selectors,
            counters: state.counters,
            dma_read_units: state.dma_read_units,
            dma_write_units: state.dma_write_units,
            dma_cycles: state.dma_cycles,
            grants: state.grants,
            port_accesses: state.port_accesses,
            port_cycles: state.port_cycles,
        }
    }

    /// Bus cycles charged to DMA accesses / 记在 DMA 访问上的总线周期
    #[must_use]
    pub fn dma_cycles(&self) -> u64 {
        self.state.lock().dma_cycles
    }

    /// DMA bus occupancy in parts per thousand / DMA 总线占用率（千分比）
    ///
    /// `total_cycles` is the window the caller wants to relate to (CPU cycles
    /// plus DMA cycles); `0` yields `0`. The value saturates at `1000` because
    /// the model charges DMA accesses without modelling CPU contention.
    /// `total_cycles` 是调用方想对照的窗口（CPU 周期加 DMA 周期），为 `0` 时返回
    /// `0`。由于模型只记 DMA 访问而不模拟 CPU 争用，结果在 `1000` 饱和。
    #[must_use]
    pub fn occupancy(&self, total_cycles: u64) -> u32 {
        if total_cycles == 0 {
            return 0;
        }
        let cycles = self.state.lock().dma_cycles;
        u32::try_from(cycles.saturating_mul(1000) / total_cycles)
            .unwrap_or(1000)
            .min(1000)
    }

    /// Reset the counter registers (device reset) / 重置计数器寄存器（设备复位）
    pub fn reset_registers(&self) {
        self.state.lock().reset_registers();
    }
}

impl Default for BusPerf {
    fn default() -> Self {
        Self::new()
    }
}

/// BUSCTRL block / BUSCTRL 寄存器块
#[derive(Debug, Clone)]
pub struct BusCtrl {
    regs: RegisterFile,
    perf: BusPerf,
}

impl BusCtrl {
    /// Create the block with a private counter bank / 用私有计数器组创建寄存器块
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: RegisterFile::new(BUSCTRL_SIZE),
            perf: BusPerf::new(),
        }
    }

    /// Create the block sharing `perf` with a DMA controller
    /// 创建与 DMA 控制器共享 `perf` 的寄存器块
    ///
    /// Mount this block and construct the DMA with
    /// [`Dma::with_perf`](crate::dma::Dma::with_perf) and the same handle to let
    /// firmware read live `PERFCTR0/1` values.
    /// 挂载本块并用 [`Dma::with_perf`](crate::dma::Dma::with_perf) 与同一句柄构造
    /// DMA，固件即可读到实时的 `PERFCTR0/1`。
    #[must_use]
    pub fn with_perf(perf: BusPerf) -> Self {
        Self {
            regs: RegisterFile::new(BUSCTRL_SIZE),
            perf,
        }
    }

    /// Shared counter bank / 共享计数器组
    #[must_use]
    pub fn perf(&self) -> BusPerf {
        self.perf.clone()
    }

    /// Current programmed bus priority / 当前配置的总线优先级
    #[must_use]
    pub fn priority(&self) -> u32 {
        self.regs.read(BUS_PRIORITY).unwrap_or(0)
    }

    /// Bus cycles charged to DMA accesses / 记在 DMA 访问上的总线周期
    #[must_use]
    pub fn dma_bus_cycles(&self) -> u64 {
        self.perf.dma_cycles()
    }

    /// Number of arbitration grants seen by the fabric / 总线矩阵记录的仲裁授予次数
    #[must_use]
    pub fn dma_grants(&self) -> u64 {
        self.perf.snapshot().grants
    }

    /// DMA bus occupancy in parts per thousand / DMA 总线占用率（千分比）
    #[must_use]
    pub fn dma_occupancy(&self, total_cycles: u64) -> u32 {
        self.perf.occupancy(total_cycles)
    }

    /// Snapshot of every fabric counter / 全部总线矩阵计数器的快照
    #[must_use]
    pub fn perf_snapshot(&self) -> BusPerfSnapshot {
        self.perf.snapshot()
    }
}

impl Default for BusCtrl {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for BusCtrl {
    fn name(&self) -> &str {
        "BUSCTRL"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::BUSCTRL_BASE, BUSCTRL_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        let (offset, _alias) = split_alias(offset);
        match offset {
            // The arbiter latches a priority write immediately in this model.
            // 本模型中仲裁器立即锁存优先级写入。
            BUS_PRIORITY_ACK => Some(1),
            PERFCTR_EN => Some(u32::from(self.perf.enabled())),
            PERFSEL0 => Some(u32::from(self.perf.selector(0))),
            PERFSEL1 => Some(u32::from(self.perf.selector(1))),
            PERFCTR0 => Some(self.perf.counter(0)),
            PERFCTR1 => Some(self.perf.counter(1)),
            offset => self.regs.read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        let (offset, alias) = split_alias(offset);
        match offset {
            BUS_PRIORITY_ACK => true,
            BUS_PRIORITY => {
                let merged = apply_alias(self.priority(), value, alias);
                let _ = self.regs.write(BUS_PRIORITY, merged & BUS_PRIORITY_MASK);
                true
            }
            PERFCTR_EN => {
                let merged = apply_alias(u32::from(self.perf.enabled()), value, alias);
                self.perf.set_enabled(merged & 0x1 != 0);
                true
            }
            PERFSEL0 | PERFSEL1 => {
                let slot = usize::from(offset == PERFSEL1);
                let merged = apply_alias(u32::from(self.perf.selector(slot)), value, alias);
                self.perf.set_selector(slot, (merged & PERFSEL_MASK) as u8);
                true
            }
            // PERFCTRn 是“写任意值清零”的寄存器。Any write clears PERFCTRn.
            PERFCTR0 | PERFCTR1 => {
                self.perf.clear_counter(usize::from(offset == PERFCTR1));
                true
            }
            _ => {
                let merged = apply_alias(self.regs.read(offset).unwrap_or(0), value, alias);
                self.regs.write(offset, merged)
            }
        }
    }

    fn reset(&mut self) {
        self.regs.reset();
        // 计数器寄存器回到复位值，总线矩阵记账保留。
        // Counter registers return to their reset values; fabric accounting stays.
        self.perf.reset_registers();
    }
}
