//! SIO peripheral model / SIO 外设模型
//!
//! Models the single-cycle GPIO output, output-enable and input registers, the
//! inter-core FIFO (8 entries), spinlocks and the RP2350 doorbells. GPIO values
//! follow the `IO_BANK0` `FUNCSEL` crossbar: `GPIO_IN` reports the resolved pad
//! level only on pins routed to `SIO`, and the output latch drives the pad only
//! there as well.
//!
//! The RP2350 SIO window is *per core*: both cores see it at `0xD000_0000`, but
//! `FIFO_WR` pushes into the peer's receive queue and `FIFO_RD` drains the queue
//! the peer filled. A single bus window therefore has to know which core is
//! currently accessing it; [`Sio::select_core`] switches that view (cheap
//! relaxed atomic store) so a driver that steps both cores can flip the FIFO,
//! doorbell and `CPUID` direction around each core's instruction.
//! 建模单周期 GPIO 输出、输出使能与输入寄存器、核间 FIFO（8 项）、自旋锁与
//! RP2350 门铃。GPIO 值遵循 `IO_BANK0` 的 `FUNCSEL` 交叉开关：`GPIO_IN` 只在
//! 路由到 `SIO` 的引脚上报告合成后的 pad 电平，输出锁存也只在那些引脚驱动 pad。
//!
//! RP2350 的 SIO 窗口是**按核**区分的：两个核心都在 `0xD000_0000` 看到它，但
//! `FIFO_WR` 压入对端的接收队列、`FIFO_RD` 排空对端写入的队列。因此单个总线
//! 窗口必须知道当前访问来自哪个核心；[`Sio::select_core`] 用于切换该视图
//! （廉价的 relaxed 原子存储），使同时推进两个核心的驱动器能在每个核心的指令
//! 前后翻转 FIFO、门铃与 `CPUID` 的方向。
//!
//! Register offsets follow the RP2350 register map (`hardware/regs/sio.h`):
//! `GPIO_HI_*` is interleaved with `GPIO_*`, and the doorbells sit at `0x180`.
//! 寄存器偏移遵循 RP2350 寄存器表（`hardware/regs/sio.h`）：`GPIO_HI_*` 与
//! `GPIO_*` 交错排列，门铃位于 `0x180`。

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use parking_lot::Mutex;
use rp2350_core::mmio::{MmioDevice, MmioWindow};
use rp2350_core::types::CpuId;

use super::gpio_pads::GpioPads;
use super::io_bank0::PinFunction;
use super::irq::IrqAssert;
use super::map;

/// `CPUID` offset / `CPUID` 偏移
const CPUID: u32 = 0x000;

/// `GPIO_IN` offset / `GPIO_IN` 偏移
const GPIO_IN: u32 = 0x004;

/// `GPIO_HI_IN` offset / `GPIO_HI_IN` 偏移
const GPIO_HI_IN: u32 = 0x008;

/// `GPIO_OUT` offset / `GPIO_OUT` 偏移
const GPIO_OUT: u32 = 0x010;

/// `GPIO_HI_OUT` offset / `GPIO_HI_OUT` 偏移
const GPIO_HI_OUT: u32 = 0x014;

/// `GPIO_OUT_SET` offset / `GPIO_OUT_SET` 偏移
const GPIO_OUT_SET: u32 = 0x018;

/// `GPIO_HI_OUT_SET` offset / `GPIO_HI_OUT_SET` 偏移
const GPIO_HI_OUT_SET: u32 = 0x01C;

/// `GPIO_OUT_CLR` offset / `GPIO_OUT_CLR` 偏移
const GPIO_OUT_CLR: u32 = 0x020;

/// `GPIO_HI_OUT_CLR` offset / `GPIO_HI_OUT_CLR` 偏移
const GPIO_HI_OUT_CLR: u32 = 0x024;

/// `GPIO_OUT_XOR` offset / `GPIO_OUT_XOR` 偏移
const GPIO_OUT_XOR: u32 = 0x028;

/// `GPIO_HI_OUT_XOR` offset / `GPIO_HI_OUT_XOR` 偏移
const GPIO_HI_OUT_XOR: u32 = 0x02C;

/// `GPIO_OE` offset / `GPIO_OE` 偏移
const GPIO_OE: u32 = 0x030;

/// `GPIO_HI_OE` offset / `GPIO_HI_OE` 偏移
const GPIO_HI_OE: u32 = 0x034;

/// `GPIO_OE_SET` offset / `GPIO_OE_SET` 偏移
const GPIO_OE_SET: u32 = 0x038;

/// `GPIO_HI_OE_SET` offset / `GPIO_HI_OE_SET` 偏移
const GPIO_HI_OE_SET: u32 = 0x03C;

/// `GPIO_OE_CLR` offset / `GPIO_OE_CLR` 偏移
const GPIO_OE_CLR: u32 = 0x040;

/// `GPIO_HI_OE_CLR` offset / `GPIO_HI_OE_CLR` 偏移
const GPIO_HI_OE_CLR: u32 = 0x044;

/// `GPIO_OE_XOR` offset / `GPIO_OE_XOR` 偏移
const GPIO_OE_XOR: u32 = 0x048;

/// `GPIO_HI_OE_XOR` offset / `GPIO_HI_OE_XOR` 偏移
const GPIO_HI_OE_XOR: u32 = 0x04C;

/// `FIFO_ST` offset / `FIFO_ST` 偏移
const FIFO_ST: u32 = 0x050;

/// `FIFO_WR` offset / `FIFO_WR` 偏移
const FIFO_WR: u32 = 0x054;

/// `FIFO_RD` offset / `FIFO_RD` 偏移
const FIFO_RD: u32 = 0x058;

/// `SPINLOCK_ST` offset / `SPINLOCK_ST` 偏移
const SPINLOCK_ST: u32 = 0x05C;

/// `SPINLOCK0` offset (RP2350 moves these to `0x100`) / `SPINLOCK0` 偏移（RP2350 移到 `0x100`）
const SPINLOCK0: u32 = 0x100;

/// Number of hardware spinlocks / 硬件自旋锁数量
const SPINLOCK_COUNT: u32 = 32;

/// Index of the `SPINLOCKn` register at `offset` / `offset` 处 `SPINLOCKn` 的下标
///
/// Returns `None` for unaligned or out-of-window offsets so callers fall through
/// to their normal unmapped-register path.
/// 未对齐或超出窗口时返回 `None`，调用方据此走正常的未映射寄存器路径。
fn spinlock_index(offset: u32) -> Option<usize> {
    let delta = offset.checked_sub(SPINLOCK0)?;
    if delta % 4 != 0 || delta >= 4 * SPINLOCK_COUNT {
        return None;
    }
    Some((delta / 4) as usize)
}

/// `DOORBELL_OUT_SET` offset / `DOORBELL_OUT_SET` 偏移
const DOORBELL_OUT_SET: u32 = 0x180;

/// `DOORBELL_OUT_CLR` offset / `DOORBELL_OUT_CLR` 偏移
const DOORBELL_OUT_CLR: u32 = 0x184;

/// `DOORBELL_IN_SET` offset / `DOORBELL_IN_SET` 偏移
const DOORBELL_IN_SET: u32 = 0x188;

/// `DOORBELL_IN_CLR` offset / `DOORBELL_IN_CLR` 偏移
const DOORBELL_IN_CLR: u32 = 0x18C;

/// Inter-core FIFO depth / 核间 FIFO 深度
const FIFO_DEPTH: usize = 8;

/// Atomic register update mode / 原子寄存器更新模式
#[derive(Debug, Clone, Copy)]
enum Patch {
    /// Bitwise OR / 按位或
    Set,
    /// Bitwise AND-NOT / 按位与非
    Clear,
    /// Bitwise XOR / 按位异或
    Xor,
}

/// Index of a core inside per-core arrays / 核心在按核数组中的下标
fn core_index(core: CpuId) -> usize {
    match core {
        CpuId::Core0 => 0,
        CpuId::Core1 => 1,
    }
}

/// Mutable SIO state shared by clones / 克隆间共享的可变 SIO 状态
#[derive(Debug, Default)]
struct SioState {
    /// `GPIO_OUT` value / `GPIO_OUT` 值
    out: u32,
    /// `GPIO_HI_OUT` value / `GPIO_HI_OUT` 值
    hi_out: u32,
    /// `GPIO_OE` value / `GPIO_OE` 值
    oe: u32,
    /// `GPIO_HI_OE` value / `GPIO_HI_OE` 值
    hi_oe: u32,
    /// Frames core 0 wrote, delivered to core 1 / core0 写入、送往 core1 的帧
    from_core0: VecDeque<u32>,
    /// Frames core 1 wrote, delivered to core 0 / core1 写入、送往 core0 的帧
    from_core1: VecDeque<u32>,
    /// Sticky write-overflow flag per writer / 按写入方区分的粘滞写溢出标志
    wof: [bool; 2],
    /// Sticky read-overflow flag per reader / 按读取方区分的粘滞读溢出标志
    roe: [bool; 2],
    /// Doorbells posted to core 0 / 投递给 core0 的门铃
    bell_core0: u8,
    /// Doorbells posted to core 1 / 投递给 core1 的门铃
    bell_core1: u8,
    /// Bitmap of claimed spinlocks, bit `n` = `SPINLOCKn` held
    /// 已占用自旋锁位图，位 `n` 对应 `SPINLOCKn`
    spinlocks: u32,
    /// Claiming core index + 1 per lock (0 = free); diagnostics only, not
    /// architecturally visible / 各锁占用核下标 + 1（0 = 空闲）；仅诊断用，
    /// 不对体系结构可见
    spinlock_owner: [u8; SPINLOCK_COUNT as usize],
}

impl SioState {
    /// Apply a set/clear/xor patch to `value` / 对 `value` 应用置位/清零/异或
    fn patch(value: &mut u32, mask: u32, mode: Patch) {
        *value = match mode {
            Patch::Set => *value | mask,
            Patch::Clear => *value & !mask,
            Patch::Xor => *value ^ mask,
        };
    }

    /// Queue holding the frames `core` wrote / 保存 `core` 写入帧的队列
    fn outbox(&self, core: CpuId) -> &VecDeque<u32> {
        match core {
            CpuId::Core0 => &self.from_core0,
            CpuId::Core1 => &self.from_core1,
        }
    }

    /// Queue holding the frames `core` reads / 保存 `core` 读取帧的队列
    fn inbox(&self, core: CpuId) -> &VecDeque<u32> {
        match core {
            CpuId::Core0 => &self.from_core1,
            CpuId::Core1 => &self.from_core0,
        }
    }

    /// Mutable outbox of `core` / `core` 的可变发送队列
    fn outbox_mut(&mut self, core: CpuId) -> &mut VecDeque<u32> {
        match core {
            CpuId::Core0 => &mut self.from_core0,
            CpuId::Core1 => &mut self.from_core1,
        }
    }

    /// Mutable inbox of `core` / `core` 的可变接收队列
    fn inbox_mut(&mut self, core: CpuId) -> &mut VecDeque<u32> {
        match core {
            CpuId::Core0 => &mut self.from_core1,
            CpuId::Core1 => &mut self.from_core0,
        }
    }

    /// `FIFO_ST` value as seen by `core` / `core` 视角的 `FIFO_ST` 值
    fn fifo_status(&self, core: CpuId) -> u32 {
        let index = core_index(core);
        let mut status = 0;
        if !self.inbox(core).is_empty() {
            status |= 1; // VLD
        }
        if self.outbox(core).len() < FIFO_DEPTH {
            status |= 1 << 1; // RDY
        }
        if self.wof[index] {
            status |= 1 << 2; // WOF
        }
        if self.roe[index] {
            status |= 1 << 3; // ROE
        }
        status
    }

    /// Doorbells posted to `core` / 投递给 `core` 的门铃
    fn bells_for(&self, core: CpuId) -> u8 {
        match core {
            CpuId::Core0 => self.bell_core0,
            CpuId::Core1 => self.bell_core1,
        }
    }

    /// Mutable doorbell set of `core` / `core` 的可变门铃集合
    fn bells_mut(&mut self, core: CpuId) -> &mut u8 {
        match core {
            CpuId::Core0 => &mut self.bell_core0,
            CpuId::Core1 => &mut self.bell_core1,
        }
    }

    /// Try to claim spinlock `index` for `core` / 尝试为 `core` 取得自旋锁 `index`
    ///
    /// The check-and-set happens in one step because the whole state sits behind
    /// a single mutex, which is the sequential-execution stand-in for the
    /// hardware's atomic claim. The first core to reach a free lock wins; core 0
    /// wins only because a sequential driver steps it first.
    /// 由于全部状态位于同一互斥锁之后，检查与置位是一步完成的，这正是顺序执行模型
    /// 对硬件原子取锁的替代。先到达空闲锁的核心获胜；core0 获胜只是顺序驱动器先
    /// 推进它的结果。
    fn claim_spinlock(&mut self, index: usize, core: CpuId) -> bool {
        let bit = 1u32 << index;
        if self.spinlocks & bit != 0 {
            return false;
        }
        self.spinlocks |= bit;
        self.spinlock_owner[index] = core_index(core) as u8 + 1;
        true
    }

    /// Release spinlock `index` regardless of writer / 释放自旋锁 `index`，与写入方无关
    fn release_spinlock(&mut self, index: usize) {
        self.spinlocks &= !(1u32 << index);
        self.spinlock_owner[index] = 0;
    }

    /// The other core / 另一个核心
    fn peer(core: CpuId) -> CpuId {
        match core {
            CpuId::Core0 => CpuId::Core1,
            CpuId::Core1 => CpuId::Core0,
        }
    }
}

/// SIO register block with a selectable per-core view
/// 带可选按核视图的 SIO 寄存器块
#[derive(Debug, Clone)]
pub struct Sio {
    state: Arc<Mutex<SioState>>,
    irq: IrqAssert,
    core1_irq: IrqAssert,
    view: Arc<AtomicU8>,
    pads: GpioPads,
}

impl Sio {
    /// Create the block with empty FIFOs and all pins low
    /// 创建 FIFO 为空、所有引脚为低的寄存器块
    ///
    /// The handle starts in the core-0 view and only publishes the core-0
    /// `SIO_IRQ_FIFO` / `SIO_IRQ_BELL` lines into `irq`.
    /// 句柄以 core0 视图启动，且只把 core0 的 `SIO_IRQ_FIFO` /
    /// `SIO_IRQ_BELL` 线发布到 `irq`。
    #[must_use]
    pub fn new(irq: IrqAssert) -> Self {
        Self::with_peer_irq(irq, IrqAssert::new())
    }

    /// Create the block with distinct per-core interrupt lines
    /// 创建带独立按核中断线的寄存器块
    ///
    /// `irq` receives the core-0 `SIO_IRQ_FIFO` / `SIO_IRQ_BELL` lines and
    /// `core1_irq` receives core 1's, so each NVIC only sees its own endpoint.
    /// `irq` 接收 core0 的 `SIO_IRQ_FIFO` / `SIO_IRQ_BELL` 线，`core1_irq`
    /// 接收 core1 的，因此每个 NVIC 只看到自己的端点。
    #[must_use]
    pub fn with_peer_irq(irq: IrqAssert, core1_irq: IrqAssert) -> Self {
        Self::with_peer_irq_and_pads(irq, core1_irq, GpioPads::new())
    }

    /// Create the block sharing pad electrical state with PADS_BANK0
    /// 创建与 PADS_BANK0 共享 pad 电气状态的寄存器块
    #[must_use]
    pub fn with_peer_irq_and_pads(irq: IrqAssert, core1_irq: IrqAssert, pads: GpioPads) -> Self {
        Self {
            state: Arc::new(Mutex::new(SioState::default())),
            irq,
            core1_irq,
            view: Arc::new(AtomicU8::new(0)),
            pads,
        }
    }

    /// Select which core's view the next bus accesses use
    /// 选择后续总线访问使用的核心视图
    ///
    /// A multi-core driver calls this before stepping each core so `FIFO_WR`,
    /// `FIFO_RD`, the doorbells and `CPUID` follow that core's direction. The
    /// switch is one relaxed atomic store and is shared by every clone,
    /// including the bus-mounted device.
    /// 多核驱动器在推进每个核心之前调用此方法，使 `FIFO_WR`、`FIFO_RD`、门铃与
    /// `CPUID` 遵循该核心的方向。切换只做一次 relaxed 原子存储，并由所有克隆
    /// （含总线挂载的设备）共享。
    pub fn select_core(&self, core: CpuId) {
        let value = match core {
            CpuId::Core0 => 0,
            CpuId::Core1 => 1,
        };
        self.view.store(value, Ordering::Relaxed);
    }

    /// Currently selected core view / 当前选定的核心视图
    #[must_use]
    pub fn selected_core(&self) -> CpuId {
        if self.view.load(Ordering::Relaxed) == 1 {
            CpuId::Core1
        } else {
            CpuId::Core0
        }
    }

    /// Hand a frame to core 0's receive FIFO (peer-core hook)
    /// 向 core0 的接收 FIFO 送入一帧（对端核心挂钩）
    ///
    /// The core-1 boot-ROM peer uses this to echo a core-0 `FIFO_WR`; tests can
    /// exercise the receive path without a second core.
    /// core1 boot ROM 对端用它回显 core0 的 `FIFO_WR`；测试也可借此在没有第二个
    /// 核心时验证接收路径。
    pub fn post_to_core(&self, value: u32) {
        self.state.lock().from_core1.push_back(value);
        self.refresh_irq();
    }

    /// Ring core 0's doorbell (peer-core hook) / 敲响 core0 的门铃（对端核心挂钩）
    ///
    /// The bits land in core 0's `DOORBELL_IN` and assert `SIO_IRQ_BELL` on core
    /// 0 only. / 位落入 core0 的 `DOORBELL_IN`，只断言 core0 的 `SIO_IRQ_BELL`。
    pub fn ring_core(&self, mask: u8) {
        self.state.lock().bell_core0 |= mask;
        self.refresh_irq();
    }

    /// Drain the frames core 0 sent to core 1 (peer-core hook)
    /// 取走 core0 发往 core1 的所有帧（对端核心挂钩）
    #[must_use]
    pub fn take_from_core(&self) -> Vec<u32> {
        let mut state = self.state.lock();
        let frames: Vec<u32> = state.from_core0.drain(..).collect();
        drop(state);
        self.refresh_irq();
        frames
    }

    /// Pop one frame core 0 sent to core 1 / 弹出 core0 发往 core1 的一帧
    ///
    /// The resident core-1 boot ROM consumes the launch command stream one word
    /// at a time so a launch leaves any later frames queued for the real core 1.
    /// 常驻 core1 boot ROM 逐字消费启动命令流，使启动后到达的帧仍留在队列中交给
    /// 真实的 core1。
    #[must_use]
    pub fn pop_from_core(&self) -> Option<u32> {
        let mut state = self.state.lock();
        let frame = state.from_core0.pop_front();
        drop(state);
        if frame.is_some() {
            self.refresh_irq();
        }
        frame
    }

    /// Doorbells posted to core 1 / 投递给 core1 的门铃位
    #[must_use]
    pub fn peer_bells(&self) -> u8 {
        self.state.lock().bell_core1
    }

    /// Bitmap of held hardware spinlocks / 已占用硬件自旋锁位图
    ///
    /// Bit `n` is set when `SPINLOCKn` is claimed, exactly what `SPINLOCK_ST`
    /// reports. This accessor skips the bus so tests and the CLI can observe the
    /// locks directly; it is a diagnostic, not a writable register.
    /// 位 `n` 置位表示 `SPINLOCKn` 已被占用，与 `SPINLOCK_ST` 一致。该访问器绕过
    /// 总线，便于测试与 CLI 直接观察锁状态；它是诊断接口，不是可写寄存器。
    #[must_use]
    pub fn spinlock_state(&self) -> u32 {
        self.state.lock().spinlocks
    }

    /// Interrupt mask of core 1's SIO lines / core1 SIO 中断线的掩码
    ///
    /// Excludes core 0's endpoint because `SIO_IRQ_FIFO` / `SIO_IRQ_BELL` are
    /// per-core despite sharing one number.
    /// 不含 core0 的端点：`SIO_IRQ_FIFO` / `SIO_IRQ_BELL` 虽然编号相同，但按核
    /// 独立。
    #[must_use]
    pub fn core1_irq_mask(&self) -> u64 {
        self.core1_irq.mask()
    }

    /// Recompute the SIO interrupt lines / 重新计算 SIO 中断线
    ///
    /// `SIO_IRQ_FIFO` follows the receiving core's `VLD` flag and
    /// `SIO_IRQ_BELL` that core's doorbells, so the two cores get independent
    /// levels for the same line numbers.
    /// `SIO_IRQ_FIFO` 跟随接收核心的 `VLD` 标志、`SIO_IRQ_BELL` 跟随该核心的
    /// 门铃，因此两个核心在同一线号上得到独立的电平。
    pub fn refresh_irq(&self) {
        let (fifo0, bell0, fifo1, bell1) = {
            let state = self.state.lock();
            (
                !state.from_core1.is_empty(),
                state.bell_core0 != 0,
                !state.from_core0.is_empty(),
                state.bell_core1 != 0,
            )
        };
        self.irq.set(map::SIO_IRQ_FIFO, fifo0);
        self.irq.set(map::SIO_IRQ_BELL, bell0);
        self.core1_irq.set(map::SIO_IRQ_FIFO, fifo1);
        self.core1_irq.set(map::SIO_IRQ_BELL, bell1);
    }

    /// Publish `GPIO_OUT`/`GPIO_OE` (and the `GPIO_HI_*` pair) into the pin matrix
    /// 把 `GPIO_OUT`/`GPIO_OE`（以及 `GPIO_HI_*` 对）发布到引脚矩阵
    ///
    /// The matrix only applies the latch to pads whose `IO_BANK0.FUNCSEL`
    /// selects `SIO`, so a pad routed to a PIO block keeps following that block.
    /// 矩阵只把锁存应用到 `IO_BANK0.FUNCSEL` 选中 `SIO` 的 pad，因此路由到 PIO 的
    /// pad 仍然跟随 PIO 块。
    fn publish_pins(&self) {
        let (levels, enables) = {
            let state = self.state.lock();
            (
                u64::from(state.out) | (u64::from(state.hi_out) << 32),
                u64::from(state.oe) | (u64::from(state.hi_oe) << 32),
            )
        };
        self.pads
            .set_driver(PinFunction::Sio.to_bits(), levels, enables);
    }

    /// Input register value for the low 32 pins / 低 32 个引脚的输入寄存器值
    ///
    /// Only pads whose `FUNCSEL` selects `SIO` report the routed pad level; the
    /// other bits keep the SIO latch loopback so a block that was never given
    /// the pad function still reads its own output.
    /// 只有 `FUNCSEL` 选中 `SIO` 的 pad 报告路由后的 pad 电平；其它位保持 SIO 锁存
    /// 回环，使未被授予 pad 功能的位仍能读回自己的输出。
    fn input_value(&self, state: &SioState) -> u32 {
        let mask = self.pads.funcsel_mask(PinFunction::Sio.to_bits()) as u32;
        let pad = self.pads.input_levels() as u32;
        (pad & mask) | (state.out & state.oe & !mask)
    }

    /// `GPIO_HI_IN` value for pins 32..47 / 引脚 32..47 的 `GPIO_HI_IN` 值
    fn hi_input_value(&self, state: &SioState) -> u32 {
        let mask = (self.pads.funcsel_mask(PinFunction::Sio.to_bits()) >> 32) as u32;
        let pad = (self.pads.input_levels() >> 32) as u32;
        (pad & mask) | (state.hi_out & state.hi_oe & !mask)
    }
}

impl MmioDevice for Sio {
    fn name(&self) -> &str {
        "SIO"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::SIO_BASE, map::SIO_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        let view = self.selected_core();
        let mut state = self.state.lock();
        let value = match offset {
            CPUID => Some(core_index(view) as u32),
            // GPIO 输入只在本引脚 `FUNCSEL == SIO` 时报告 pad 合成电平；
            // 其余引脚保持 SIO 输出回环，未使能时读 0。
            // GPIO inputs report the resolved pad level only where
            // `FUNCSEL == SIO`; other pins keep the SIO latch loopback and read
            // 0 when the driver is off.
            GPIO_IN => Some(self.input_value(&state)),
            GPIO_OUT => Some(state.out),
            GPIO_HI_IN => Some(self.hi_input_value(&state)),
            GPIO_HI_OUT => Some(state.hi_out),
            GPIO_OE => Some(state.oe),
            GPIO_HI_OE => Some(state.hi_oe),
            FIFO_ST => Some(state.fifo_status(view)),
            FIFO_RD => {
                let index = core_index(view);
                let value = state.inbox_mut(view).pop_front().unwrap_or_else(|| {
                    state.roe[index] = true;
                    0
                });
                Some(value)
            }
            // `SPINLOCK_ST` is a read-only snapshot of the claim bitmap: 1 = held.
            // `SPINLOCK_ST` 是占用位图的只读快照：1 = 已锁。
            SPINLOCK_ST => Some(state.spinlocks),
            DOORBELL_OUT_SET | DOORBELL_OUT_CLR => {
                Some(u32::from(state.bells_for(SioState::peer(view))))
            }
            DOORBELL_IN_SET | DOORBELL_IN_CLR => Some(u32::from(state.bells_for(view))),
            _ => spinlock_index(offset).map(|index| {
                // Reading claims the lock atomically and yields 1 << n on success,
                // 0 when another core already holds it. Locks ignore the core view.
                // 读即原子取锁：成功返回 1 << n，被占则返回 0；锁与核心视图无关。
                if state.claim_spinlock(index, view) {
                    1u32 << index
                } else {
                    0
                }
            }),
        };
        drop(state);
        self.refresh_irq();
        value
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        let view = self.selected_core();
        let peer = SioState::peer(view);
        let mut state = self.state.lock();
        match offset {
            GPIO_OUT => state.out = value,
            GPIO_HI_OUT => state.hi_out = value,
            GPIO_OUT_SET => SioState::patch(&mut state.out, value, Patch::Set),
            GPIO_HI_OUT_SET => SioState::patch(&mut state.hi_out, value, Patch::Set),
            GPIO_OUT_CLR => SioState::patch(&mut state.out, value, Patch::Clear),
            GPIO_HI_OUT_CLR => SioState::patch(&mut state.hi_out, value, Patch::Clear),
            GPIO_OUT_XOR => SioState::patch(&mut state.out, value, Patch::Xor),
            GPIO_HI_OUT_XOR => SioState::patch(&mut state.hi_out, value, Patch::Xor),
            GPIO_OE => state.oe = value,
            GPIO_HI_OE => state.hi_oe = value,
            GPIO_OE_SET => SioState::patch(&mut state.oe, value, Patch::Set),
            GPIO_HI_OE_SET => SioState::patch(&mut state.hi_oe, value, Patch::Set),
            GPIO_OE_CLR => SioState::patch(&mut state.oe, value, Patch::Clear),
            GPIO_HI_OE_CLR => SioState::patch(&mut state.hi_oe, value, Patch::Clear),
            GPIO_OE_XOR => SioState::patch(&mut state.oe, value, Patch::Xor),
            GPIO_HI_OE_XOR => SioState::patch(&mut state.hi_oe, value, Patch::Xor),
            // FIFO_WR pushes towards the peer; a full queue sets that writer's
            // WOF. FIFO_WR 向对端压入；队列满时置位该写入方的 WOF。
            FIFO_WR => {
                let index = core_index(view);
                let outbox = state.outbox_mut(view);
                if outbox.len() < FIFO_DEPTH {
                    outbox.push_back(value);
                } else {
                    state.wof[index] = true;
                }
            }
            DOORBELL_OUT_SET => *state.bells_mut(peer) |= (value & 0xFF) as u8,
            DOORBELL_OUT_CLR => *state.bells_mut(peer) &= !(value & 0xFF) as u8,
            DOORBELL_IN_SET => *state.bells_mut(view) |= (value & 0xFF) as u8,
            DOORBELL_IN_CLR => *state.bells_mut(view) &= !(value & 0xFF) as u8,
            _ => {
                // Writing any value to `SPINLOCKn` releases it; the data is ignored.
                // 向 `SPINLOCKn` 写任意值即释放该锁，写入数据被忽略。
                let Some(index) = spinlock_index(offset) else {
                    drop(state);
                    return false;
                };
                state.release_spinlock(index);
            }
        }
        drop(state);
        self.publish_pins();
        self.refresh_irq();
        true
    }

    fn reset(&mut self) {
        *self.state.lock() = SioState::default();
        self.publish_pins();
        self.refresh_irq();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sio() -> (Sio, IrqAssert) {
        let irq = IrqAssert::new();
        (Sio::new(irq.clone()), irq)
    }

    #[test]
    fn set_clear_xor_updates_output() {
        let (mut sio, _irq) = sio();
        assert!(sio.write32(GPIO_OUT_SET, 0b1010));
        assert_eq!(sio.read32(GPIO_OUT), Some(0b1010));
        assert!(sio.write32(GPIO_OUT_CLR, 0b0010));
        assert_eq!(sio.read32(GPIO_OUT), Some(0b1000));
        assert!(sio.write32(GPIO_OUT_XOR, 0b1111));
        // The input register reflects the pad: enable the outputs first, then
        // the SIO latch drives them.
        // 输入寄存器反映 pad：先使能输出，SIO 锁存才会驱动引脚。
        assert!(sio.write32(GPIO_OE, 0xF));
        assert_eq!(sio.read32(GPIO_IN), Some(0b0111));
    }

    #[test]
    fn rp2350_hi_registers_are_interleaved() {
        let (mut sio, _irq) = sio();
        // 0x014 is GPIO_HI_OUT on RP2350, not an output-set alias.
        // 在 RP2350 上 0x014 是 GPIO_HI_OUT，而不是输出置位别名。
        assert!(sio.write32(GPIO_HI_OUT, 0xAAAA));
        assert_eq!(sio.read32(GPIO_HI_OUT), Some(0xAAAA));
        assert_eq!(sio.read32(GPIO_IN), Some(0));
        assert_eq!(sio.read32(GPIO_OUT), Some(0));
    }

    #[test]
    fn fifo_reports_ready_then_overflow() {
        let (mut sio, _irq) = sio();
        assert_eq!(sio.read32(FIFO_ST), Some(0b10));
        for value in 0..FIFO_DEPTH as u32 {
            assert!(sio.write32(FIFO_WR, value));
        }
        assert_eq!(
            sio.read32(FIFO_ST),
            Some(0b0),
            "TX full clears RDY / 发送满时清除 RDY"
        );
        assert!(sio.write32(FIFO_WR, 0xFFFF_FFFF));
        assert_eq!(sio.read32(FIFO_ST), Some(0b100));
        assert_eq!(
            sio.take_from_core(),
            (0..FIFO_DEPTH as u32).collect::<Vec<_>>()
        );
    }

    #[test]
    fn receive_fifo_asserts_fifo_irq() {
        let (mut sio, irq) = sio();
        sio.post_to_core(0x1234);
        assert_eq!(irq.mask(), 1 << map::SIO_IRQ_FIFO);
        assert_eq!(sio.read32(FIFO_ST), Some(0b11));
        assert_eq!(sio.read32(FIFO_RD), Some(0x1234));
        assert_eq!(irq.mask(), 0);
        // Reading an empty FIFO sets ROE. 读取空 FIFO 会置位 ROE。
        assert_eq!(sio.read32(FIFO_RD), Some(0));
        assert_eq!(sio.read32(FIFO_ST), Some(0b1010));
    }

    #[test]
    fn doorbell_in_asserts_bell_irq() {
        let (mut sio, irq) = sio();
        assert!(sio.write32(DOORBELL_IN_SET, 0b0101));
        assert_eq!(sio.read32(DOORBELL_IN_CLR), Some(0b0101));
        assert_eq!(irq.mask(), 1 << map::SIO_IRQ_BELL);
        assert!(sio.write32(DOORBELL_IN_CLR, 0b0101));
        assert_eq!(irq.mask(), 0);
    }

    #[test]
    fn outgoing_doorbell_targets_the_peer() {
        let (mut sio, irq) = sio();
        assert!(sio.write32(DOORBELL_OUT_SET, 0b10));
        assert_eq!(sio.peer_bells(), 0b10);
        assert_eq!(
            irq.mask(),
            0,
            "core 0 must not ring itself / core 0 不应敲响自己"
        );
    }

    #[test]
    fn peer_core_can_ring_this_core() {
        let (mut sio, irq) = sio();
        sio.ring_core(0b100);
        assert_eq!(irq.mask(), 1 << map::SIO_IRQ_BELL);
        assert_eq!(sio.read32(DOORBELL_IN_SET), Some(0b100));
        assert!(sio.write32(DOORBELL_IN_CLR, 0b100));
        assert_eq!(irq.mask(), 0);
    }

    #[test]
    fn the_core1_view_reverses_the_fifo_direction() {
        // core1 的 FIFO_WR 必须进入 core0 的接收队列，FIFO_RD 读取 core0 写入的帧。
        // Core 1's FIFO_WR must feed core 0's receive queue, and its FIFO_RD must
        // drain what core 0 wrote.
        let irq0 = IrqAssert::new();
        let irq1 = IrqAssert::new();
        let mut sio = Sio::with_peer_irq(irq0.clone(), irq1.clone());

        sio.select_core(CpuId::Core0);
        assert!(sio.write32(FIFO_WR, 0xC0DE));
        assert_eq!(sio.read32(CPUID), Some(0));
        assert_eq!(
            irq1.mask(),
            1 << map::SIO_IRQ_FIFO,
            "core 1 sees the pending frame"
        );
        assert_eq!(irq0.mask(), 0);

        sio.select_core(CpuId::Core1);
        assert_eq!(sio.read32(CPUID), Some(1));
        assert_eq!(sio.read32(FIFO_RD), Some(0xC0DE));
        assert_eq!(irq1.mask(), 0);
        assert!(sio.write32(FIFO_WR, 0xBEEF));
        assert_eq!(irq0.mask(), 1 << map::SIO_IRQ_FIFO);

        sio.select_core(CpuId::Core0);
        assert_eq!(sio.read32(FIFO_RD), Some(0xBEEF));
        assert_eq!(sio.pop_from_core(), None, "core 1 drained its inbox");
    }

    #[test]
    fn doorbells_cross_between_cores() {
        // core1 敲响的门铃只断言 core1 的 SIO_IRQ_BELL 线到 core0 的 NVIC。
        // A doorbell rung by core 1 asserts only core 0's SIO_IRQ_BELL line.
        let irq0 = IrqAssert::new();
        let irq1 = IrqAssert::new();
        let mut sio = Sio::with_peer_irq(irq0.clone(), irq1.clone());

        sio.select_core(CpuId::Core1);
        assert!(sio.write32(DOORBELL_OUT_SET, 0b10));
        sio.select_core(CpuId::Core0);
        assert_eq!(irq0.mask(), 1 << map::SIO_IRQ_BELL);
        assert_eq!(sio.read32(DOORBELL_IN_SET), Some(0b10));
        assert_eq!(irq1.mask(), 0);
        assert_eq!(sio.peer_bells(), 0, "core 0 rang nobody");
    }

    #[test]
    fn pad_pull_up_reaches_the_gpio_input_register() {
        let pads = GpioPads::new();
        pads.set_pad(0, super::super::gpio_pads::PAD_PUE);
        // 只有 `FUNCSEL == SIO` 的引脚才读 pad：pin 0 路由到 SIO，pin 1 不是。
        // Only pins with `FUNCSEL == SIO` read the pad: pin 0 is routed to SIO,
        // pin 1 is not.
        pads.set_pad(1, super::super::gpio_pads::PAD_PUE);
        pads.set_funcsel(0, PinFunction::Sio.to_bits());
        let mut sio = Sio::with_peer_irq_and_pads(IrqAssert::new(), IrqAssert::new(), pads);
        assert_eq!(sio.read32(GPIO_IN), Some(1), "routed pin reads the pad");

        // Enabling the output and driving it low overrides the pull-up.
        // 使能输出并驱动低电平会覆盖上拉。
        assert!(sio.write32(GPIO_OE, 1));
        assert!(sio.write32(GPIO_OUT, 0));
        assert_eq!(sio.read32(GPIO_IN), Some(0));
    }
}
