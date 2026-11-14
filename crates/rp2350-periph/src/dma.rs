//! RP2350 DMA controller model / RP2350 DMA 控制器模型
//!
//! Models the 16-channel DMA register block at [`map::DMA_BASE`] (`0x5000_0000`):
//! per-channel `READ_ADDR`/`WRITE_ADDR`/`TRANS_COUNT`/`CTRL_TRIG` plus the
//! AL1/AL2/AL3 trigger aliases, the global `INTR`/`INTE0..3`/`INTF0..3`/
//! `INTS0..3` interrupt registers, `CHAN_ABORT` and the pacing timer registers.
//! Transfers are executed by the host through [`Dma::run`], which moves one
//! data unit per step at `DATA_SIZE` width (byte/halfword/word), honours
//! `INCR_READ`/`INCR_WRITE` and the simple `RING` wrap, decrements
//! `TRANS_COUNT`, and on completion clears `BUSY`, starts `CHAIN_TO` and sets
//! the channel's raw interrupt bit.
//! 建模 [`map::DMA_BASE`]（`0x5000_0000`）的 16 通道 DMA 寄存器块：每通道的
//! `READ_ADDR`/`WRITE_ADDR`/`TRANS_COUNT`/`CTRL_TRIG` 与 AL1/AL2/AL3 触发别名，
//! 全局 `INTR`/`INTE0..3`/`INTF0..3`/`INTS0..3` 中断寄存器，`CHAN_ABORT` 以及
//! 节拍定时器寄存器。传输由宿主通过 [`Dma::run`] 执行：每步按 `DATA_SIZE`
//! 宽度（字节/半字/字）搬运一个数据单元，遵循 `INCR_READ`/`INCR_WRITE` 与简单
//! 的 `RING` 环绕，递减 `TRANS_COUNT`；完成时清除 `BUSY`、启动 `CHAIN_TO` 并在
//! 该通道的原始中断位置位。
//!
//! # DREQ / DREQ 请求
//!
//! `TREQ_SEL = 0x3F` is the permanent request and always runs. Every other
//! selector reads the host-driven level array exposed by [`Dma::set_dreq`];
//! unmodelled selectors default to asserted so firmware that never programs a
//! DREQ still makes progress. Selectors `0x3B..=0x3E` (pacing timers) are
//! stored but the timers do not pace transfers yet.
//! `TREQ_SEL = 0x3F` 为永久请求，始终可运行。其它选择读取 [`Dma::set_dreq`]
//! 暴露的宿主侧电平数组；未建模的选择默认断言，使未编程 DREQ 的固件仍能推进。
//! `0x3B..=0x3E`（节拍定时器）仅存储寄存器，定时器尚未驱动传输节奏。
//!
//! # Not modelled / 未建模项
//!
//! `BSWAP` (byte swapping), `SNIFF_EN`/`SNIFF_CTRL` sniffing, `HIGH_PRIORITY`
//! arbitration, `TRANS_COUNT` transfer modes (`[31:28]`), AHB error recovery
//! beyond clearing `BUSY`, and the APB SET/CLR/XOR global aliases are accepted
//! but have no behavioural effect. `REGISTER` reads still return the stored
//! values. The DMA window also does not intercept CPU buses, so transfers are
//! only performed when the host calls [`Dma::run`].
//! `BSWAP`（字节交换）、`SNIFF_EN`/`SNIFF_CTRL` 嗅探、`HIGH_PRIORITY` 仲裁、
//! `TRANS_COUNT` 传输模式（`[31:28]`）、除清除 `BUSY` 之外的 AHB 错误恢复，
//! 以及 APB SET/CLR/XOR 全局别名均接受写入但无行为效果；读取仍返回存储值。
//! DMA 窗口也不拦截 CPU 总线，只有宿主调用 [`Dma::run`] 时才执行传输。

use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_core::bus::{AddressBus, BusResult};
use rp2350_core::mmio::{MmioDevice, MmioWindow};

use super::irq::IrqAssert;
use super::map;

/// Number of DMA channels / DMA 通道数量
pub const CHANNELS: usize = 16;

/// Number of DREQ (TREQ) selector inputs / DREQ（TREQ）选择输入数量
pub const DREQ_COUNT: usize = 64;

/// `TREQ_SEL` value selecting the permanent request / 选择永久请求的 `TREQ_SEL` 值
pub const TREQ_PERMANENT: u8 = 0x3F;

/// `CTRL_TRIG.EN` — channel enable / 通道使能
pub const CTRL_EN: u32 = 1 << 0;

/// `CTRL_TRIG.HIGH_PRIORITY` — stored, arbitration not modelled / 仅存储，仲裁未建模
pub const CTRL_HIGH_PRIORITY: u32 = 1 << 1;

/// `CTRL_TRIG.DATA_SIZE` shift / 数据宽度字段移位
pub const CTRL_DATA_SIZE_SHIFT: u32 = 2;

/// `CTRL_TRIG.DATA_SIZE` mask / 数据宽度字段掩码
pub const CTRL_DATA_SIZE_MASK: u32 = 0x3 << CTRL_DATA_SIZE_SHIFT;

/// `CTRL_TRIG.INCR_READ` — increment the read address / 递增读地址
pub const CTRL_INCR_READ: u32 = 1 << 4;

/// `CTRL_TRIG.INCR_WRITE` — increment the write address / 递增写地址
pub const CTRL_INCR_WRITE: u32 = 1 << 6;

/// `CTRL_TRIG.RING_SIZE` shift (log2 byte count) / 环大小字段移位（字节数的 log2）
pub const CTRL_RING_SIZE_SHIFT: u32 = 8;

/// `CTRL_TRIG.RING_SIZE` mask / 环大小字段掩码
pub const CTRL_RING_SIZE_MASK: u32 = 0xF << CTRL_RING_SIZE_SHIFT;

/// `CTRL_TRIG.RING_SEL` — apply the ring to the write side / 环应用到写侧
pub const CTRL_RING_SEL: u32 = 1 << 12;

/// `CTRL_TRIG.CHAIN_TO` shift / 链式目标字段移位
pub const CTRL_CHAIN_TO_SHIFT: u32 = 13;

/// `CTRL_TRIG.CHAIN_TO` mask / 链式目标字段掩码
pub const CTRL_CHAIN_TO_MASK: u32 = 0xF << CTRL_CHAIN_TO_SHIFT;

/// `CTRL_TRIG.TREQ_SEL` shift / DREQ 选择字段移位
pub const CTRL_TREQ_SEL_SHIFT: u32 = 17;

/// `CTRL_TRIG.TREQ_SEL` mask / DREQ 选择字段掩码
pub const CTRL_TREQ_SEL_MASK: u32 = 0x3F << CTRL_TREQ_SEL_SHIFT;

/// `CTRL_TRIG.IRQ_QUIET` — suppress the completion IRQ / 抑制完成中断
pub const CTRL_IRQ_QUIET: u32 = 1 << 23;

/// `CTRL_TRIG.BSWAP` — stored, byte swap not modelled / 仅存储，字节交换未建模
pub const CTRL_BSWAP: u32 = 1 << 24;

/// `CTRL_TRIG.SNIFF_EN` — stored, sniffing not modelled / 仅存储，嗅探未建模
pub const CTRL_SNIFF_EN: u32 = 1 << 25;

/// `CTRL_TRIG.BUSY` — read-only transfer-in-progress flag / 只读传输进行中标志
pub const CTRL_BUSY: u32 = 1 << 26;

/// `CTRL_TRIG.READ_ERROR` — read-side bus error, write-one-to-clear
/// `CTRL_TRIG.READ_ERROR` — 读侧总线错误，写 1 清零
const CTRL_READ_ERROR: u32 = 1 << 30;

/// `CTRL_TRIG.WRITE_ERROR` — write-side bus error, write-one-to-clear
/// `CTRL_TRIG.WRITE_ERROR` — 写侧总线错误，写 1 清零
const CTRL_WRITE_ERROR: u32 = 1 << 29;

/// Bits shared by both error flags / 两个错误标志的位掩码
const CTRL_ERROR_MASK: u32 = CTRL_READ_ERROR | CTRL_WRITE_ERROR;

/// Writable `CTRL_TRIG` bits (`EN` through `SNIFF_EN`) / `CTRL_TRIG` 可写位
const CTRL_WRITE_MASK: u32 = 0x03FF_FFFF;

/// `TRANS_COUNT` count field mask (transfer mode bits excluded)
/// `TRANS_COUNT` 计数字段掩码（不含传输模式位）
const TRANS_COUNT_MASK: u32 = 0x0FFF_FFFF;

/// Channel register stride / 通道寄存器步长
const CHANNEL_STRIDE: u32 = 0x40;

/// End of the per-channel register region / 每通道寄存器区域结束偏移
const CHANNEL_REGION_END: u32 = 0x400;

/// Per-channel `READ_ADDR` offset / 通道 `READ_ADDR` 偏移
const CH_READ_ADDR: u32 = 0x00;

/// Per-channel `WRITE_ADDR` offset / 通道 `WRITE_ADDR` 偏移
const CH_WRITE_ADDR: u32 = 0x04;

/// Per-channel `TRANS_COUNT` offset / 通道 `TRANS_COUNT` 偏移
const CH_TRANS_COUNT: u32 = 0x08;

/// Per-channel `CTRL_TRIG` offset / 通道 `CTRL_TRIG` 偏移
const CH_CTRL_TRIG: u32 = 0x0C;

/// `AL1_CTRL` offset / `AL1_CTRL` 偏移
const CH_AL1_CTRL: u32 = 0x10;

/// `AL1_READ_ADDR` offset / `AL1_READ_ADDR` 偏移
const CH_AL1_READ_ADDR: u32 = 0x14;

/// `AL1_WRITE_ADDR` offset / `AL1_WRITE_ADDR` 偏移
const CH_AL1_WRITE_ADDR: u32 = 0x18;

/// `AL1_TRANS_COUNT_TRIG` offset / `AL1_TRANS_COUNT_TRIG` 偏移
const CH_AL1_TRANS_COUNT_TRIG: u32 = 0x1C;

/// `AL2_CTRL` offset / `AL2_CTRL` 偏移
const CH_AL2_CTRL: u32 = 0x20;

/// `AL2_TRANS_COUNT` offset / `AL2_TRANS_COUNT` 偏移
const CH_AL2_TRANS_COUNT: u32 = 0x24;

/// `AL2_READ_ADDR` offset / `AL2_READ_ADDR` 偏移
const CH_AL2_READ_ADDR: u32 = 0x28;

/// `AL2_WRITE_ADDR_TRIG` offset / `AL2_WRITE_ADDR_TRIG` 偏移
const CH_AL2_WRITE_ADDR_TRIG: u32 = 0x2C;

/// `AL3_CTRL` offset / `AL3_CTRL` 偏移
const CH_AL3_CTRL: u32 = 0x30;

/// `AL3_WRITE_ADDR` offset / `AL3_WRITE_ADDR` 偏移
const CH_AL3_WRITE_ADDR: u32 = 0x34;

/// `AL3_TRANS_COUNT` offset / `AL3_TRANS_COUNT` 偏移
const CH_AL3_TRANS_COUNT: u32 = 0x38;

/// `AL3_READ_ADDR_TRIG` offset / `AL3_READ_ADDR_TRIG` 偏移
const CH_AL3_READ_ADDR_TRIG: u32 = 0x3C;

/// `INTR` offset / `INTR` 偏移
const GLOBAL_INTR: u32 = 0x400;

/// `INTE0` offset / `INTE0` 偏移
const GLOBAL_INTE0: u32 = 0x404;

/// `INTS0` offset / `INTS0` 偏移
const GLOBAL_INTS0: u32 = 0x40C;

/// Stride between the four interrupt register groups / 四组中断寄存器的步长
const INT_LINE_STRIDE: u32 = 0x10;

/// `TIMER0` offset / `TIMER0` 偏移
const GLOBAL_TIMER0: u32 = 0x440;

/// `MULTI_CHAN_TRIGGER` offset / `MULTI_CHAN_TRIGGER` 偏移
const GLOBAL_MULTI_CHAN_TRIGGER: u32 = 0x450;

/// `SNIFF_CTRL` offset / `SNIFF_CTRL` 偏移
const GLOBAL_SNIFF_CTRL: u32 = 0x454;

/// `SNIFF_DATA` offset / `SNIFF_DATA` 偏移
const GLOBAL_SNIFF_DATA: u32 = 0x458;

/// `FIFO_LEVELS` offset / `FIFO_LEVELS` 偏移
const GLOBAL_FIFO_LEVELS: u32 = 0x460;

/// `CHAN_ABORT` offset / `CHAN_ABORT` 偏移
const GLOBAL_CHAN_ABORT: u32 = 0x464;

/// `N_CHANNELS` offset / `N_CHANNELS` 偏移
const GLOBAL_N_CHANNELS: u32 = 0x468;

/// Read-only snapshot of one DMA channel / 单个 DMA 通道的只读快照
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaChannelState {
    /// Current read address / 当前读地址
    pub read_addr: u32,
    /// Current write address / 当前写地址
    pub write_addr: u32,
    /// Remaining transfer count / 剩余传输次数
    pub trans_count: u32,
    /// Stored `CTRL_TRIG` value including `BUSY` / 存储的 `CTRL_TRIG` 值（含 `BUSY`）
    pub ctrl: u32,
    /// Transfer currently in progress / 当前是否有传输进行中
    pub busy: bool,
    /// `CTRL_TRIG.EN` state / `CTRL_TRIG.EN` 状态
    pub enabled: bool,
    /// `CHAIN_TO` target channel / `CHAIN_TO` 目标通道
    pub chain_to: u8,
    /// `TREQ_SEL` DREQ selector / `TREQ_SEL` DREQ 选择
    pub treq_sel: u8,
    /// `RING_SIZE` log2 byte count (0 = disabled) / `RING_SIZE` 字节数 log2（0 表示禁用）
    pub ring_size: u8,
    /// `IRQ_QUIET` completion interrupt suppression / `IRQ_QUIET` 完成中断抑制
    pub irq_quiet: bool,
}

/// Mutable per-channel state / 每通道可变状态
#[derive(Debug, Clone, Copy, Default)]
struct Channel {
    read_addr: u32,
    write_addr: u32,
    trans_count: u32,
    ctrl: u32,
    busy: bool,
}

/// Shared register file and channel state / 共享寄存器文件与通道状态
#[derive(Debug)]
struct DmaState {
    channels: [Channel; CHANNELS],
    intr: u16,
    inte: [u16; 4],
    intf: [u16; 4],
    dreq: [bool; DREQ_COUNT],
    timer: [u32; 4],
    sniff_ctrl: u32,
    sniff_data: u32,
    cursor: usize,
}

impl Default for DmaState {
    fn default() -> Self {
        Self {
            channels: [Channel::default(); CHANNELS],
            intr: 0,
            inte: [0; 4],
            intf: [0; 4],
            // 未建模的 DREQ 选择默认断言，仅永久请求与宿主门控例外。
            // Unmodelled DREQ selectors default to asserted; only the permanent
            // request and host gating change that.
            dreq: [true; DREQ_COUNT],
            timer: [0; 4],
            sniff_ctrl: 0,
            sniff_data: 0,
            cursor: 0,
        }
    }
}

impl DmaState {
    /// Restore power-on defaults; `CHAIN_TO` resets to the channel itself
    /// 恢复上电默认值；`CHAIN_TO` 复位为通道自身
    fn reset(&mut self) {
        *self = Self::default();
        for (index, channel) in self.channels.iter_mut().enumerate() {
            channel.ctrl = (index as u32) << CTRL_CHAIN_TO_SHIFT;
        }
    }
}

/// One transfer claimed by [`Dma::run`] / [`Dma::run`] 认领的一次传输
#[derive(Debug, Clone, Copy)]
struct DmaJob {
    channel: usize,
    read_addr: u32,
    write_addr: u32,
    data_size: u8,
}

/// Which side of a transfer hit a bus error / 传输哪一侧遇到总线错误
#[derive(Debug, Clone, Copy)]
enum BusFault {
    Read,
    Write,
}

/// RP2350 16-channel DMA controller / RP2350 16 通道 DMA 控制器
///
/// Clones share the register file, so the bus-mounted device and the handle
/// kept by [`crate::BootPeripherals`] observe the same state.
/// 克隆共享寄存器文件，因此总线上的设备与 [`crate::BootPeripherals`] 保存的
/// 句柄看到相同状态。
#[derive(Clone)]
pub struct Dma {
    state: Arc<Mutex<DmaState>>,
    irq: IrqAssert,
}

impl Dma {
    /// Create a DMA controller asserting lines into `irq`
    /// 创建向 `irq` 断言中断线的 DMA 控制器
    #[must_use]
    pub fn new(irq: IrqAssert) -> Self {
        let mut state = DmaState::default();
        state.reset();
        Self {
            state: Arc::new(Mutex::new(state)),
            irq,
        }
    }

    /// Execute at most `max_transfers` data units across all channels
    /// 在所有通道上最多执行 `max_transfers` 个数据单元
    ///
    /// Channels are visited round-robin. A channel is eligible when `EN` and
    /// `BUSY` are set and its `TREQ_SEL` request is asserted. One call performs
    /// one bus read plus one bus write per unit; completion clears `BUSY`,
    /// raises `INTR` unless `IRQ_QUIET` is set, and starts `CHAIN_TO`. Returns
    /// the number of units attempted, so a caller can stop at its cycle budget.
    /// 通道按轮转顺序访问。通道在 `EN` 与 `BUSY` 置位且其 `TREQ_SEL` 请求断言时
    /// 可执行。每次调用对每个数据单元执行一次总线读与一次总线写；完成时清除
    /// `BUSY`、在未置 `IRQ_QUIET` 时置位 `INTR`，并启动 `CHAIN_TO`。返回尝试
    /// 执行的单元数，调用方可据此按周期预算停止。
    #[must_use]
    pub fn run(&mut self, bus: &mut dyn AddressBus, max_transfers: u64) -> u64 {
        let mut performed = 0u64;
        let mut cycles = 0u64;
        while performed < max_transfers {
            let Some(job) = self.claim_job() else {
                break;
            };
            let outcome = match read_unit(bus, job.read_addr, job.data_size, &mut cycles) {
                Ok(value) => write_unit(bus, job.write_addr, job.data_size, value, &mut cycles)
                    .map_err(|_| BusFault::Write),
                Err(_) => Err(BusFault::Read),
            };
            self.finish_job(job.channel, outcome.err());
            performed += 1;
        }
        performed
    }

    /// Drive one host DREQ input; selectors `>= 64` are ignored
    /// 驱动一个宿主 DREQ 输入；`>= 64` 的选择被忽略
    ///
    /// The level is sampled by [`Self::run`] before every unit. The permanent
    /// request (`0x3F`) ignores this array entirely.
    /// 每次执行数据单元前由 [`Self::run`] 采样该电平。永久请求（`0x3F`）完全
    /// 忽略该数组。
    pub fn set_dreq(&mut self, dreq: u8, asserted: bool) {
        let mut state = self.state.lock();
        if let Some(level) = state.dreq.get_mut(usize::from(dreq)) {
            *level = asserted;
        }
    }

    /// Current level of one host DREQ input / 某个宿主 DREQ 输入的当前电平
    ///
    /// Returns `None` for selectors `>= 64`.
    /// 对 `>= 64` 的选择返回 `None`。
    #[must_use]
    pub fn dreq(&self, dreq: u8) -> Option<bool> {
        self.state.lock().dreq.get(usize::from(dreq)).copied()
    }

    /// Abort the channels selected by `mask`, leaving counters and addresses
    /// 中止 `mask` 选中的通道，保留计数与地址
    ///
    /// `EN` is not cleared, matching `CHAN_ABORT`; the host can restart the
    /// channel with a new `CTRL_TRIG` write.
    /// 不清除 `EN`，与 `CHAN_ABORT` 一致；宿主可通过新的 `CTRL_TRIG` 写重启通道。
    pub fn abort(&mut self, mask: u16) {
        let mut state = self.state.lock();
        for (index, channel) in state.channels.iter_mut().enumerate() {
            if mask & (1u16 << index) != 0 {
                channel.busy = false;
            }
        }
    }

    /// Bitmask of channels currently busy / 当前忙碌通道的位掩码
    #[must_use]
    pub fn busy_mask(&self) -> u16 {
        let state = self.state.lock();
        state
            .channels
            .iter()
            .enumerate()
            .fold(0u16, |mask, (index, channel)| {
                mask | (u16::from(channel.busy) << index)
            })
    }

    /// Read-only snapshot of one channel; `None` for `index >= 16`
    /// 单个通道的只读快照；`index >= 16` 时返回 `None`
    #[must_use]
    pub fn channel_state(&self, index: usize) -> Option<DmaChannelState> {
        let state = self.state.lock();
        let channel = state.channels.get(index)?;
        let busy = u32::from(channel.busy) * CTRL_BUSY;
        Some(DmaChannelState {
            read_addr: channel.read_addr,
            write_addr: channel.write_addr,
            trans_count: channel.trans_count,
            ctrl: channel.ctrl | busy,
            busy: channel.busy,
            enabled: channel.ctrl & CTRL_EN != 0,
            chain_to: ((channel.ctrl & CTRL_CHAIN_TO_MASK) >> CTRL_CHAIN_TO_SHIFT) as u8,
            treq_sel: ((channel.ctrl & CTRL_TREQ_SEL_MASK) >> CTRL_TREQ_SEL_SHIFT) as u8,
            ring_size: ((channel.ctrl & CTRL_RING_SIZE_MASK) >> CTRL_RING_SIZE_SHIFT) as u8,
            irq_quiet: channel.ctrl & CTRL_IRQ_QUIET != 0,
        })
    }

    /// Pick the next eligible channel / 选择下一个可执行通道
    fn claim_job(&self) -> Option<DmaJob> {
        let mut state = self.state.lock();
        let start = state.cursor;
        for step in 0..CHANNELS {
            let index = (start + step) % CHANNELS;
            let (busy, enabled, treq) = {
                let channel = &state.channels[index];
                (
                    channel.busy,
                    channel.ctrl & CTRL_EN != 0,
                    ((channel.ctrl & CTRL_TREQ_SEL_MASK) >> CTRL_TREQ_SEL_SHIFT) as u8,
                )
            };
            if !busy || !enabled {
                continue;
            }
            if treq != TREQ_PERMANENT && !state.dreq[usize::from(treq)] {
                continue;
            }
            let channel = &state.channels[index];
            let job = DmaJob {
                channel: index,
                read_addr: channel.read_addr,
                write_addr: channel.write_addr,
                data_size: ((channel.ctrl & CTRL_DATA_SIZE_MASK) >> CTRL_DATA_SIZE_SHIFT) as u8,
            };
            state.cursor = (index + 1) % CHANNELS;
            return Some(job);
        }
        None
    }

    /// Commit one unit: advance addresses, decrement the count, chain and IRQ
    /// 提交一个数据单元：推进地址、递减计数、链式启动与中断
    fn finish_job(&self, index: usize, fault: Option<BusFault>) {
        {
            let mut state = self.state.lock();
            let completed;
            let quiet;
            let chain;
            {
                let channel = &mut state.channels[index];
                if let Some(fault) = fault {
                    // 总线错误会终止通道并记录错误位，不再产生完成中断。
                    // A bus error stops the channel and latches the error bit;
                    // it does not raise a completion interrupt.
                    channel.ctrl |= match fault {
                        BusFault::Read => CTRL_READ_ERROR,
                        BusFault::Write => CTRL_WRITE_ERROR,
                    };
                    channel.busy = false;
                    completed = false;
                    quiet = true;
                    chain = index;
                } else {
                    let ring_size = (channel.ctrl & CTRL_RING_SIZE_MASK) >> CTRL_RING_SIZE_SHIFT;
                    let ring_sel = channel.ctrl & CTRL_RING_SEL != 0;
                    // 数据宽度 0/1/2 = 1/2/4 字节；保留值 3 按字处理。
                    // Data size 0/1/2 = 1/2/4 bytes; reserved 3 is treated as word.
                    let step = 1u32 << u32::min(data_size_field(channel.ctrl), 2);
                    if channel.ctrl & CTRL_INCR_READ != 0 {
                        channel.read_addr = advance(channel.read_addr, step, ring_size, !ring_sel);
                    }
                    if channel.ctrl & CTRL_INCR_WRITE != 0 {
                        channel.write_addr = advance(channel.write_addr, step, ring_size, ring_sel);
                    }
                    channel.trans_count = channel.trans_count.saturating_sub(1);
                    completed = channel.trans_count == 0;
                    quiet = channel.ctrl & CTRL_IRQ_QUIET != 0;
                    chain = ((channel.ctrl & CTRL_CHAIN_TO_MASK) >> CTRL_CHAIN_TO_SHIFT) as usize;
                    if completed {
                        channel.busy = false;
                    }
                }
            }
            if completed {
                if !quiet {
                    state.intr |= 1u16 << index;
                }
                if chain != index {
                    start_channel(&mut state, chain);
                }
            }
        }
        self.refresh_irq();
    }

    /// Publish `INTS0..3` onto the shared interrupt lines / 将 `INTS0..3` 发布到共享中断线
    fn refresh_irq(&self) {
        let state = self.state.lock();
        for line in 0..4 {
            let status = u32::from(state.intr & state.inte[line]) | u32::from(state.intf[line]);
            self.irq.set(map::DMA_IRQ_0 + line as u16, status != 0);
        }
    }
}

/// Extract the `DATA_SIZE` field / 提取 `DATA_SIZE` 字段
fn data_size_field(ctrl: u32) -> u32 {
    (ctrl & CTRL_DATA_SIZE_MASK) >> CTRL_DATA_SIZE_SHIFT
}

/// Apply a normal or ring-wrapped address increment / 应用普通或环回地址递增
fn advance(address: u32, step: u32, ring_size: u32, ring: bool) -> u32 {
    if !ring || ring_size == 0 {
        return address.wrapping_add(step);
    }
    let mask = (1u32 << ring_size) - 1;
    (address & !mask) | (address.wrapping_add(step) & mask)
}

/// Start a channel and follow zero-length chains / 启动通道并追踪零长度链
fn start_channel(state: &mut DmaState, mut index: usize) {
    // 零长度传输会立即完成并继续链式传递；最多遍历一轮以避免环路死循环。
    // A zero-length transfer completes immediately and continues chaining;
    // iterate at most one full round to avoid a cyclic chain spinning forever.
    for _ in 0..CHANNELS {
        let (enabled, count, quiet, chain) = {
            let channel = &state.channels[index];
            (
                channel.ctrl & CTRL_EN != 0,
                channel.trans_count,
                channel.ctrl & CTRL_IRQ_QUIET != 0,
                ((channel.ctrl & CTRL_CHAIN_TO_MASK) >> CTRL_CHAIN_TO_SHIFT) as usize,
            )
        };
        if !enabled {
            return;
        }
        if count != 0 {
            state.channels[index].busy = true;
            return;
        }
        if !quiet {
            state.intr |= 1u16 << index;
        }
        if chain == index {
            return;
        }
        index = chain;
    }
}

/// Read one data unit at the requested width / 按指定宽度读取一个数据单元
fn read_unit(
    bus: &mut dyn AddressBus,
    address: u32,
    data_size: u8,
    cycles: &mut u64,
) -> BusResult<u32> {
    match data_size {
        0 => bus.read_u8(address, cycles).map(u32::from),
        1 => bus.read_u16(address, cycles).map(u32::from),
        _ => bus.read_u32(address, cycles),
    }
}

/// Write one data unit at the requested width / 按指定宽度写入一个数据单元
fn write_unit(
    bus: &mut dyn AddressBus,
    address: u32,
    data_size: u8,
    value: u32,
    cycles: &mut u64,
) -> BusResult<()> {
    match data_size {
        0 => bus.write_u8(address, value as u8, cycles),
        1 => bus.write_u16(address, value as u16, cycles),
        _ => bus.write_u32(address, value, cycles),
    }
}

/// Apply a `CTRL_TRIG` write to a channel / 将 `CTRL_TRIG` 写入应用到通道
fn write_ctrl(channel: &mut Channel, value: u32) {
    // 错误位为写 1 清零，其余可写位按写入值覆盖。
    // Error flags are write-one-to-clear; every other writable bit is replaced.
    let errors = channel.ctrl & CTRL_ERROR_MASK & !(value & CTRL_ERROR_MASK);
    channel.ctrl = (value & CTRL_WRITE_MASK) | errors;
    if channel.ctrl & CTRL_EN == 0 {
        channel.busy = false;
    }
}

/// Compute `INTSn` for one interrupt line / 计算某条中断线的 `INTSn`
fn line_status(state: &DmaState, line: usize) -> u32 {
    u32::from(state.intr & state.inte[line]) | u32::from(state.intf[line])
}

impl MmioDevice for Dma {
    fn name(&self) -> &str {
        "DMA"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::DMA_BASE, map::DMA_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        let state = self.state.lock();
        match offset {
            0x000..CHANNEL_REGION_END => {
                let index = (offset / CHANNEL_STRIDE) as usize;
                let register = offset % CHANNEL_STRIDE;
                let channel = state.channels.get(index)?;
                Some(match register {
                    CH_READ_ADDR | CH_AL1_READ_ADDR | CH_AL2_READ_ADDR | CH_AL3_READ_ADDR_TRIG => {
                        channel.read_addr
                    }
                    CH_WRITE_ADDR
                    | CH_AL1_WRITE_ADDR
                    | CH_AL2_WRITE_ADDR_TRIG
                    | CH_AL3_WRITE_ADDR => channel.write_addr,
                    CH_TRANS_COUNT
                    | CH_AL1_TRANS_COUNT_TRIG
                    | CH_AL2_TRANS_COUNT
                    | CH_AL3_TRANS_COUNT => channel.trans_count,
                    CH_CTRL_TRIG | CH_AL1_CTRL | CH_AL2_CTRL | CH_AL3_CTRL => {
                        channel.ctrl | (u32::from(channel.busy) * CTRL_BUSY)
                    }
                    _ => return None,
                })
            }
            GLOBAL_INTR => Some(u32::from(state.intr)),
            0x404..=0x43C => {
                let line = ((offset - GLOBAL_INTE0) / INT_LINE_STRIDE) as usize;
                match (offset - GLOBAL_INTE0) % INT_LINE_STRIDE {
                    0 => Some(u32::from(state.inte[line])),
                    4 => Some(u32::from(state.intf[line])),
                    8 => Some(line_status(&state, line)),
                    _ => None,
                }
            }
            GLOBAL_TIMER0..=0x44C => Some(state.timer[((offset - GLOBAL_TIMER0) / 4) as usize]),
            GLOBAL_MULTI_CHAN_TRIGGER | GLOBAL_FIFO_LEVELS | GLOBAL_CHAN_ABORT => Some(0),
            GLOBAL_SNIFF_CTRL => Some(state.sniff_ctrl),
            GLOBAL_SNIFF_DATA => Some(state.sniff_data),
            GLOBAL_N_CHANNELS => Some(CHANNELS as u32),
            _ => None,
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        let handled;
        {
            let mut state = self.state.lock();
            handled = match offset {
                0x000..CHANNEL_REGION_END => {
                    let index = (offset / CHANNEL_STRIDE) as usize;
                    let register = offset % CHANNEL_STRIDE;
                    let trigger;
                    {
                        let Some(channel) = state.channels.get_mut(index) else {
                            return false;
                        };
                        trigger = match register {
                            CH_READ_ADDR | CH_AL1_READ_ADDR | CH_AL2_READ_ADDR => {
                                channel.read_addr = value;
                                false
                            }
                            CH_AL3_READ_ADDR_TRIG => {
                                channel.read_addr = value;
                                value != 0
                            }
                            CH_WRITE_ADDR | CH_AL1_WRITE_ADDR | CH_AL3_WRITE_ADDR => {
                                channel.write_addr = value;
                                false
                            }
                            CH_AL2_WRITE_ADDR_TRIG => {
                                channel.write_addr = value;
                                value != 0
                            }
                            CH_TRANS_COUNT | CH_AL2_TRANS_COUNT | CH_AL3_TRANS_COUNT => {
                                channel.trans_count = value & TRANS_COUNT_MASK;
                                false
                            }
                            CH_AL1_TRANS_COUNT_TRIG => {
                                channel.trans_count = value & TRANS_COUNT_MASK;
                                value != 0
                            }
                            CH_CTRL_TRIG | CH_AL1_CTRL | CH_AL2_CTRL | CH_AL3_CTRL => {
                                write_ctrl(channel, value);
                                channel.ctrl & CTRL_EN != 0 && value != 0
                            }
                            _ => return false,
                        };
                    }
                    if trigger {
                        start_channel(&mut state, index);
                    }
                    true
                }
                GLOBAL_INTR | GLOBAL_INTS0 | 0x41C | 0x42C | 0x43C => {
                    // INTR 与 INTSn 均为写 1 清零。INTR and INTSn are W1C.
                    state.intr &= !(value as u16);
                    true
                }
                0x404..=0x43C => {
                    let line = ((offset - GLOBAL_INTE0) / INT_LINE_STRIDE) as usize;
                    match (offset - GLOBAL_INTE0) % INT_LINE_STRIDE {
                        0 => state.inte[line] = value as u16,
                        4 => state.intf[line] = value as u16,
                        _ => return false,
                    }
                    true
                }
                GLOBAL_TIMER0..=0x44C => {
                    state.timer[((offset - GLOBAL_TIMER0) / 4) as usize] = value;
                    true
                }
                GLOBAL_MULTI_CHAN_TRIGGER => {
                    for index in 0..CHANNELS {
                        if value & (1u32 << index) != 0 {
                            start_channel(&mut state, index);
                        }
                    }
                    true
                }
                GLOBAL_SNIFF_CTRL => {
                    state.sniff_ctrl = value;
                    true
                }
                GLOBAL_SNIFF_DATA => {
                    state.sniff_data = value;
                    true
                }
                GLOBAL_CHAN_ABORT => {
                    for index in 0..CHANNELS {
                        if value & (1u32 << index) != 0 {
                            state.channels[index].busy = false;
                        }
                    }
                    true
                }
                GLOBAL_FIFO_LEVELS | GLOBAL_N_CHANNELS => true,
                _ => false,
            };
        }
        self.refresh_irq();
        handled
    }

    fn reset(&mut self) {
        self.state.lock().reset();
        self.refresh_irq();
    }

    fn irq_number(&self) -> Option<u16> {
        Some(map::DMA_IRQ_0)
    }
}
