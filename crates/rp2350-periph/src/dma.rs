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
//! # Triggers, reload and chaining / 触发、重载与链式
//!
//! `TRANS_COUNT` (`0x08`) and the non-triggering count aliases latch a
//! *rewrite* value; a trigger copies it into the working count that [`Dma::run`]
//! decrements. That split is what keeps SDK control-block rings alive: the CB
//! channels are configured once with `TRANS_COUNT = 1` and then re-triggered
//! forever by `CHAIN_TO`. Only `CTRL_TRIG` and the `AL1_TRANS_COUNT_TRIG` /
//! `AL2_WRITE_ADDR_TRIG` / `AL3_READ_ADDR_TRIG` aliases trigger; `AL1_CTRL`,
//! `AL2_CTRL` and `AL3_CTRL` are plain configuration writes (this is what
//! `dma_channel_configure(..., false)` relies on), and `MULTI_CHAN_TRIGGER`
//! only starts channels whose `EN` is set. A trigger on a busy channel is
//! ignored, and a zero-length trigger completes immediately, raising `INTR`
//! unless `IRQ_QUIET` is set, then follows `CHAIN_TO`.
//! `TRANS_COUNT`（`0x08`）与非触发计数别名锁存*重载*值；触发器把它装载到
//! [`Dma::run`] 递减的工作计数。正是这一拆分让 SDK 控制块环持续运行：CB 通道
//! 只配置一次 `TRANS_COUNT = 1`，之后由 `CHAIN_TO` 无限次重新触发。只有
//! `CTRL_TRIG` 与 `AL1_TRANS_COUNT_TRIG` / `AL2_WRITE_ADDR_TRIG` /
//! `AL3_READ_ADDR_TRIG` 别名会触发；`AL1_CTRL`、`AL2_CTRL`、`AL3_CTRL` 是普通
//! 配置写入（`dma_channel_configure(..., false)` 正是依赖这一点），
//! `MULTI_CHAN_TRIGGER` 只启动 `EN` 置位的通道。忙碌通道上的触发被忽略；零长度
//! 触发立即完成，未置 `IRQ_QUIET` 时置位 `INTR`，随后跟踪 `CHAIN_TO`。
//!
//! # DREQ / DREQ 请求
//!
//! `TREQ_SEL = 0x3F` is the permanent request and always runs. Every other
//! selector reads the host-driven level array exposed by [`Dma::set_dreq`];
//! unmodelled selectors default to asserted so firmware that never programs a
//! DREQ still makes progress. The PIO FIFO selectors `0..=23` and their level
//! semantics are documented in [`crate::dreq`] and published by
//! [`crate::BootPeripherals::sync_pio_dreqs`]. Selectors `0x3B..=0x3E` are
//! driven by the four pacing timers: `TIMERx` uses `X[31:16]` as dividend and
//! `Y[15:0]` as divisor, and [`Dma::advance`] queues `(X / Y) * cycles` requests.
//! [`Dma::run`] consumes one queued pulse per sampled transfer (see
//! `dma_pacing`).
//! `TREQ_SEL = 0x3F` 为永久请求，始终可运行。其它选择读取 [`Dma::set_dreq`]
//! 暴露的宿主侧电平数组；未建模的选择默认断言，使未编程 DREQ 的固件仍能推进。
//! PIO FIFO 选择器 `0..=23` 及其电平语义见 [`crate::dreq`]，由
//! [`crate::BootPeripherals::sync_pio_dreqs`] 发布。`0x3B..=0x3E` 由四个节拍
//! 定时器驱动：`TIMERx` 以 `X[31:16]` 为被除数、`Y[15:0]` 为除数，
//! [`Dma::advance`] 排队 `(X / Y) * cycles` 个请求；[`Dma::run`] 每次采样到的
//! 传输消费一个排队脉冲（见 `dma_pacing`）。
//!
//! # Transfer modes, byte swap, sniffing and priority / 传输模式、字节交换、嗅探与优先级
//!
//! `TRANS_COUNT[31:28]` selects the transfer mode and is latched together with
//! the reload count. `0x0` NORMAL counts down and completes; `0x1`
//! TRIGGER_SELF behaves like NORMAL but re-triggers the channel from the reload
//! count after the completion interrupt and `CHAIN_TO` have been handled; `0xF`
//! ENDLESS freezes the working count, so the channel never completes, never
//! chains and never raises an interrupt until `CHAN_ABORT`. Any other mode is
//! treated as NORMAL. `CTRL_TRIG.BSWAP` swaps the two bytes of a halfword and
//! reverses the four bytes of a word on the write side (byte transfers are
//! unaffected). `CTRL_TRIG.SNIFF_EN` feeds every unit read by the observed
//! channel into the checksum engine documented in `dma_sniff`: sniffing also
//! requires `SNIFF_CTRL.EN` and a matching `SNIFF_CTRL.DMACH`, and the
//! `SNIFF_DATA` register is both the seed written by software (for example
//! `0xFFFF_FFFF` before CRC-32) and the running accumulator. The sniffer sits
//! downstream of the channel byte swap, so it observes the *already swapped*
//! unit; `SNIFF_CTRL.BSWAP` then applies on top, which is why the two swap bits
//! cancel when both are set. `SNIFF_CTRL.CALC` selects CRC-32/IEEE (reflected
//! input), CRC-32 with bit-reversed input bytes, CRC-16-CCITT in both bit
//! orders, the XOR reduction (parity) or the 32-bit addition checksum (`0x4`
//! through `0xD` are reserved by the register header and leave the accumulator
//! unchanged); `OUT_REV` (bit reverse) and `OUT_INV` (complement) apply only to
//! the presented value, as described in `dma_sniff`. `CTRL_TRIG.HIGH_PRIORITY`
//! channels are served before any normal channel in each scheduling round;
//! channels of the same priority keep round-robin order.
//! `TRANS_COUNT[31:28]` 选择传输模式，并与重载计数一起锁存。`0x0` NORMAL 递减
//! 计数并完成；`0x1` TRIGGER_SELF 与 NORMAL 相同，但在处理完成中断与 `CHAIN_TO`
//! 之后用重载计数重新触发自身；`0xF` ENDLESS 冻结工作计数，因此通道永不完成、
//! 不链式、也不产生中断，直到 `CHAN_ABORT`。其它模式按 NORMAL 处理。
//! `CTRL_TRIG.BSWAP` 在写侧交换半字的两字节、反转字的四字节（字节传输不受影响）。
//! `CTRL_TRIG.SNIFF_EN` 把被观察通道每次读出的数据送入 `dma_sniff` 记录的校验
//! 引擎：嗅探还要求 `SNIFF_CTRL.EN` 置位且 `SNIFF_CTRL.DMACH` 匹配；`SNIFF_DATA`
//! 既是软件写入的种子（例如 CRC-32 前写 `0xFFFF_FFFF`），也是运行累加器。
//! 嗅探器位于通道字节交换之后，因此它观察的是*已交换*的单元；`SNIFF_CTRL.BSWAP`
//! 再叠加其上，这就是两个交换位同时置位时相互抵消的原因。`SNIFF_CTRL.CALC`
//! 选择 CRC-32/IEEE（反射输入）、输入字节位反转的 CRC-32、两种位序的
//! CRC-16-CCITT、异或折叠（奇偶）或 32 位加法校验和（`0x4..=0xD` 在寄存器头文件中
//! 为保留值，累加器保持不变）；`OUT_REV`（位反转）与 `OUT_INV`（取反）只作用于
//! 展示值，语义见 `dma_sniff`。`CTRL_TRIG.HIGH_PRIORITY` 通道在每个调度轮次中
//! 先于任何普通通道被服务；同优先级的通道保持轮转顺序。
//!
//! # Bus accounting / 总线记账
//!
//! Every unit performs one bus read and one bus write; [`Dma::bus_counters`]
//! reports the completed units and the bus cycles those accesses consumed, plus
//! the number of arbitration grants [`Dma::run`] handed out. When the
//! controller is built with [`Dma::with_perf`] it also reports every completed
//! access, attributed by address to a crossbar downstream port, into a
//! [`crate::busctrl::BusPerf`] bank behind `BUSCTRL.PERFCTR0/1`. This turns "who
//! held the bus" into queryable data; it is not a cycle-exact arbiter (see the
//! not-modelled section).
//! 每个单元执行一次总线读与一次总线写；[`Dma::bus_counters`] 报告完成单元数与
//! 这些访问消耗的总线周期，以及 [`Dma::run`] 发出的仲裁授予次数。用
//! [`Dma::with_perf`] 构造时，控制器还会把每次完成的访问按地址归属到交叉开关
//! 下游端口，并上报到 `BUSCTRL.PERFCTR0/1` 背后的 [`crate::busctrl::BusPerf`]
//! 计数器组。这把「谁占用了总线」变成可查询数据；它不是周期精确仲裁器（见未建模
//! 一节）。
//!
//! # Not modelled / 未建模项
//!
//! The per-channel debug registers at `0x800..=0xBFF`, AHB error recovery
//! beyond clearing `BUSY`, and cycle-exact bus/arbitration timing are not
//! modelled: contention, downstream stalls and preemption need a shared
//! multi-master bus model, so `BUSCTRL.PERFSEL` events
//! `STALL_DOWNSTREAM`/`ACCESS_CONTESTED` stay 0 and DMA accesses are never
//! stalled by CPU traffic. `SNIFF_CTRL.CALC` values `0x4..=0xD` are reserved in
//! `hardware/regs/dma.h` (only `0x0..=0x3`, `0xE` and `0xF` are defined), so
//! they leave the accumulator unchanged. Bus timing is not cycle-exact and the
//! DMA window does not intercept CPU buses, so transfers are only performed
//! when the host calls [`Dma::run`].
//! `0x800..=0xBFF` 的通道调试寄存器、除清除 `BUSY` 之外的 AHB 错误恢复，以及周期
//! 精确的总线/仲裁时序未建模：争用、下游 stall 与抢占需要共享的多主设备总线模型，
//! 因此 `BUSCTRL.PERFSEL` 的 `STALL_DOWNSTREAM`/`ACCESS_CONTESTED` 事件保持 0，
//! DMA 访问也不会被 CPU 流量 stall。`SNIFF_CTRL.CALC` 的 `0x4..=0xD` 在
//! `hardware/regs/dma.h` 中为保留值（只定义了 `0x0..=0x3`、`0xE`、`0xF`），
//! 因此累加器保持不变。总线时序不是周期精确的，DMA 窗口也不拦截 CPU 总线，
//! 因此只有宿主调用 [`Dma::run`] 时才执行传输。

#[path = "dma_mmio.rs"]
mod mmio;
#[path = "dma_pacing.rs"]
mod pacing;
#[path = "dma_sniff.rs"]
mod sniff;

use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_core::bus::{AddressBus, BusResult};

use super::busctrl::BusPerf;
use super::irq::IrqAssert;
use super::map;

/// Number of DMA channels / DMA 通道数量
pub const CHANNELS: usize = 16;

/// Number of DREQ (TREQ) selector inputs / DREQ（TREQ）选择输入数量
pub const DREQ_COUNT: usize = 64;

/// `TREQ_SEL` value selecting the permanent request / 选择永久请求的 `TREQ_SEL` 值
pub const TREQ_PERMANENT: u8 = 0x3F;

/// First pacing timer DREQ selector (`DREQ_DMA_TIMER0`) / 首个节拍定时器 DREQ 选择器
pub const DREQ_TIMER0: u8 = 59;

/// Number of pacing timers / 节拍定时器数量
pub const TIMER_COUNT: usize = 4;

/// `CTRL_TRIG.EN` — channel enable / 通道使能
pub const CTRL_EN: u32 = 1 << 0;

/// `CTRL_TRIG.HIGH_PRIORITY` — served before normal channels
/// `CTRL_TRIG.HIGH_PRIORITY` — 先于普通通道被服务
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

/// `CTRL_TRIG.BSWAP` — swap bytes of the transferred unit
/// `CTRL_TRIG.BSWAP` — 交换传输单元的字节顺序
pub const CTRL_BSWAP: u32 = 1 << 24;

/// `CTRL_TRIG.SNIFF_EN` — feed read units into the checksum engine
/// `CTRL_TRIG.SNIFF_EN` — 把读出的数据送入校验引擎
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

/// `TRANS_COUNT.MODE` shift / `TRANS_COUNT.MODE` 移位
pub const TRANS_COUNT_MODE_SHIFT: u32 = 28;

/// `TRANS_COUNT.MODE` mask / `TRANS_COUNT.MODE` 掩码
pub const TRANS_COUNT_MODE_MASK: u32 = 0xF << TRANS_COUNT_MODE_SHIFT;

/// `MODE = 0x0` — count down and complete / `MODE = 0x0`：递减计数并完成
pub const TRANS_COUNT_MODE_NORMAL: u8 = 0x0;

/// `MODE = 0x1` — retrigger the channel on completion
/// `MODE = 0x1`：完成时重新触发自身
pub const TRANS_COUNT_MODE_TRIGGER_SELF: u8 = 0x1;

/// `MODE = 0xF` — endless: count frozen, never completes
/// `MODE = 0xF`：无限模式，计数冻结、永不完成
pub const TRANS_COUNT_MODE_ENDLESS: u8 = 0xF;

/// `SNIFF_CTRL.EN` — sniffer master enable / 嗅探器总开关
pub const SNIFF_EN: u32 = 1 << 0;

/// `SNIFF_CTRL.DMACH` shift / `SNIFF_CTRL.DMACH` 移位
pub const SNIFF_DMACH_SHIFT: u32 = 1;

/// `SNIFF_CTRL.DMACH` mask / `SNIFF_CTRL.DMACH` 掩码
pub const SNIFF_DMACH_MASK: u32 = 0xF << SNIFF_DMACH_SHIFT;

/// `SNIFF_CTRL.CALC` shift / `SNIFF_CTRL.CALC` 移位
pub const SNIFF_CALC_SHIFT: u32 = 5;

/// `SNIFF_CTRL.CALC` mask / `SNIFF_CTRL.CALC` 掩码
pub const SNIFF_CALC_MASK: u32 = 0xF << SNIFF_CALC_SHIFT;

/// `CALC = 0x0` — CRC-32/IEEE with reflected input / CRC-32/IEEE（反射输入）
pub const SNIFF_CALC_CRC32: u32 = 0x0;

/// `CALC = 0x1` — CRC-32/IEEE with bit-reversed input bytes
/// `CALC = 0x1`：CRC-32/IEEE（输入字节位反转）
pub const SNIFF_CALC_CRC32R: u32 = 0x1;

/// `CALC = 0x2` — CRC-16-CCITT (`0x1021`) / CRC-16-CCITT（`0x1021`）
pub const SNIFF_CALC_CRC16: u32 = 0x2;

/// `CALC = 0x3` — CRC-16-CCITT (`0x8408`) with bit-reversed input
/// `CALC = 0x3`：CRC-16-CCITT（`0x8408`，输入位反转）
pub const SNIFF_CALC_CRC16R: u32 = 0x3;

/// `CALC = 0xE` — XOR reduction over all data bits (result 0 or 1)
/// `CALC = 0xE`：全部数据位异或折叠（结果 0 或 1）
pub const SNIFF_CALC_XOR: u32 = 0xE;

/// `CALC = 0xF` — 32-bit addition checksum / 32 位加法校验和
pub const SNIFF_CALC_SUM: u32 = 0xF;

/// `SNIFF_CTRL.BSWAP` — byte-reverse the sniffed data before the checksum
/// `SNIFF_CTRL.BSWAP` — 送入校验器前把嗅探数据按字节反转
pub const SNIFF_BSWAP: u32 = 1 << 9;

/// `SNIFF_CTRL.OUT_REV` — bit-reverse the presented result
/// `SNIFF_CTRL.OUT_REV` — 对读出的结果做位反转
pub const SNIFF_OUT_REV: u32 = 1 << 10;

/// `SNIFF_CTRL.OUT_INV` — complement the presented result
/// `SNIFF_CTRL.OUT_INV` — 对读出的结果取反
pub const SNIFF_OUT_INV: u32 = 1 << 11;

/// Writable `SNIFF_CTRL` bits (`EN` through `OUT_INV`) / `SNIFF_CTRL` 可写位
const SNIFF_WRITE_MASK: u32 = 0x0000_0FFF;

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
    /// Working transfer count (not the rewrite value) / 工作传输计数（非重载值）
    pub trans_count: u32,
    /// Latched `TRANS_COUNT.MODE` / 锁存的 `TRANS_COUNT.MODE`
    pub trans_count_mode: u8,
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
    /// Working count, decremented once per data unit / 工作计数，每个数据单元递减一次
    trans_count: u32,
    /// Rewrite value latched by a `TRANS_COUNT` write and copied into
    /// `trans_count` by the next trigger / 写 `TRANS_COUNT` 锁存的重载值，
    /// 由下一次触发装载到 `trans_count`
    trans_count_reload: u32,
    /// Transfer mode latched with the reload value / 与重载值一起锁存的传输模式
    trans_count_mode: u8,
    ctrl: u32,
    busy: bool,
}

/// Arbitration and bus-occupancy totals of the DMA master
/// DMA 主设备的仲裁与总线占用总计
///
/// Every executed unit performs one bus read and (unless the read faulted) one
/// bus write; `read_cycles`/`write_cycles` are the bus cycles those accesses
/// consumed. `grants` counts the arbitration decisions [`Dma::run`] made, so a
/// host can compare granted units against moved data. This is *accounting*, not
/// cycle-exact preemption: the model never stalls a CPU access against a DMA
/// access.
/// 每个执行单元执行一次总线读与（读未出错时）一次总线写；`read_cycles` /
/// `write_cycles` 是这些访问消耗的总线周期。`grants` 统计 [`Dma::run`] 做出的
/// 仲裁决定数，宿主可据此把授予单元数与搬运数据对照。这是*记账*而非周期精确
/// 抢占：模型不会让 CPU 访问与 DMA 访问互相 stall。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DmaBusCounters {
    /// Completed read units / 完成的读单元数
    pub read_units: u64,
    /// Completed write units / 完成的写单元数
    pub write_units: u64,
    /// Bus cycles consumed by reads / 读消耗的总线周期
    pub read_cycles: u64,
    /// Bus cycles consumed by writes / 写消耗的总线周期
    pub write_cycles: u64,
    /// Arbitration grants (one per attempted unit) / 仲裁授予次数（每个尝试单元一次）
    pub grants: u64,
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
    /// Fractional pacing accumulator (`X` fraction carried across calls)
    /// 小数节拍累加器（`X` 的余数跨调用结转）
    timer_acc: [u128; 4],
    /// Queued pacing pulses, consumed one per sampled transfer
    /// 已排队的节拍脉冲，每次采样到的传输消费一个
    timer_pending: [u32; 4],
    sniff_ctrl: u32,
    sniff_data: u32,
    cursor: usize,
    /// Bus-occupancy totals of this master / 本主设备的总线占用总计
    bus: DmaBusCounters,
    /// `clk_sys` cycles credited to the pacing timers / 计入节拍定时器的 `clk_sys` 周期
    pacing_cycles: u64,
}

impl Default for DmaState {
    fn default() -> Self {
        // 复位时 TIMERx 的 X == 0，按硬件语义永不断言；其余未建模选择器默认断言。
        // At reset TIMERx has X == 0 and never asserts; the other unmodelled
        // selectors default to asserted.
        let mut dreq = [true; DREQ_COUNT];
        let first = usize::from(DREQ_TIMER0);
        for level in &mut dreq[first..first + TIMER_COUNT] {
            *level = false;
        }
        Self {
            channels: [Channel::default(); CHANNELS],
            intr: 0,
            inte: [0; 4],
            intf: [0; 4],
            dreq,
            timer: [0; 4],
            timer_acc: [0; 4],
            timer_pending: [0; 4],
            sniff_ctrl: 0,
            sniff_data: 0,
            cursor: 0,
            bus: DmaBusCounters::default(),
            pacing_cycles: 0,
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
    /// Snapshot of `CTRL_TRIG` for byte swap / sniff gating
    /// 用于字节交换与嗅探门控的 `CTRL_TRIG` 快照
    ctrl: u32,
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
    /// Optional bus-fabric counter bank (`BUSCTRL.PERFCTR0/1`)
    /// 可选的总线矩阵计数器组（`BUSCTRL.PERFCTR0/1`）
    perf: Option<BusPerf>,
    irq: IrqAssert,
}

impl Dma {
    /// Create a DMA controller asserting lines into `irq`
    /// 创建向 `irq` 断言中断线的 DMA 控制器
    ///
    /// The controller is not wired to a bus-fabric counter bank; use
    /// [`Self::with_perf`] to report into [`crate::busctrl::BusCtrl`].
    /// 控制器未接线到总线矩阵计数器组；用 [`Self::with_perf`] 向
    /// [`crate::busctrl::BusCtrl`] 上报。
    #[must_use]
    pub fn new(irq: IrqAssert) -> Self {
        let mut state = DmaState::default();
        state.reset();
        Self {
            state: Arc::new(Mutex::new(state)),
            perf: None,
            irq,
        }
    }

    /// Create a DMA controller reporting into a shared counter bank
    /// 创建向共享计数器组上报的 DMA 控制器
    ///
    /// Pass `BusCtrl::perf()` so firmware reads the accumulated `PERFCTR0/1`
    /// values; every completed read and write is attributed to its crossbar
    /// downstream port (see [`crate::busctrl::port_of_address`]).
    /// 传入 `BusCtrl::perf()` 后固件即可读到累积的 `PERFCTR0/1`；每次完成的读/写
    /// 都会归属到对应的交叉开关下游端口（见 [`crate::busctrl::port_of_address`]）。
    #[must_use]
    pub fn with_perf(irq: IrqAssert, perf: BusPerf) -> Self {
        let mut state = DmaState::default();
        state.reset();
        Self {
            state: Arc::new(Mutex::new(state)),
            perf: Some(perf),
            irq,
        }
    }

    /// Shared counter bank, when one was wired / 已接线时的共享计数器组
    #[must_use]
    pub fn perf(&self) -> Option<&BusPerf> {
        self.perf.as_ref()
    }

    /// Bus-occupancy totals of this master / 本主设备的总线占用总计
    ///
    /// Unlike `PERFCTR0/1` these totals are always available, whether or not a
    /// [`BusPerf`] bank is wired, because they live in the register file.
    /// 与 `PERFCTR0/1` 不同，这些总计始终可用（无论是否接线 [`BusPerf`] 计数器组），
    /// 因为它们保存在寄存器文件内。
    #[must_use]
    pub fn bus_counters(&self) -> DmaBusCounters {
        self.state.lock().bus
    }

    /// Advance the four pacing timers by `cycles` DMA cycles
    /// 按 `cycles` 个 DMA 周期推进四个节拍定时器
    ///
    /// `TIMERx` `<31:16>` (X) is the dividend and `<15:0>` (Y) the divisor, so a
    /// timer requests at `(X / Y) * sys_clk`: `X == Y` yields one request per
    /// cycle and `X = 1, Y = 4` one every four cycles. Pulses are queued, and a
    /// [`Self::run`] transfer that samples the asserted `DREQ_DMA_TIMERn`
    /// selector (`59..=62`) consumes one. `X == 0` or `Y == 0` never asserts.
    /// [`crate::BootPeripherals::pump_dma`] calls this with the real elapsed
    /// `clk_sys` cycles.
    /// `TIMERx` 的 `<31:16>`（X）是被除数，`<15:0>`（Y）是除数，因此定时器以
    /// `(X / Y) * sys_clk` 的速率请求：`X == Y` 每个周期一次，`X = 1, Y = 4`
    /// 每四个周期一次。脉冲进入队列，[`Self::run`] 中采样到 `DREQ_DMA_TIMERn`
    /// （`59..=62`）电平的传输消费一个。`X == 0` 或 `Y == 0` 永不断言。
    /// [`crate::BootPeripherals::pump_dma`] 以真实经过的 `clk_sys` 周期调用本方法。
    pub fn advance(&mut self, cycles: u64) {
        let mut state = self.state.lock();
        state.pacing_cycles = state.pacing_cycles.saturating_add(cycles);
        pacing::advance(&mut state, cycles);
    }

    /// `clk_sys` cycles credited to the pacing timers so far
    /// 累计计入节拍定时器的 `clk_sys` 周期
    #[must_use]
    pub fn pacing_cycles(&self) -> u64 {
        self.state.lock().pacing_cycles
    }

    /// Microseconds represented by the credited pacing cycles at `sysclk_hz`
    /// 已计入的节拍周期在 `sysclk_hz` 下对应的微秒数
    ///
    /// Zero when `sysclk_hz` is zero. / `sysclk_hz` 为零时返回 0。
    #[must_use]
    pub fn time_us(&self, sysclk_hz: u32) -> u64 {
        if sysclk_hz == 0 {
            return 0;
        }
        self.pacing_cycles() * 1_000_000 / u64::from(sysclk_hz)
    }

    /// Execute at most `max_transfers` data units across all channels
    /// 在所有通道上最多执行 `max_transfers` 个数据单元
    ///
    /// Channels are visited round-robin. A channel is eligible when `EN` and
    /// `BUSY` are set and its `TREQ_SEL` request is asserted. One call performs
    /// one bus read plus one bus write per unit; completion clears `BUSY`,
    /// raises `INTR` unless `IRQ_QUIET` is set, and starts `CHAIN_TO`. Returns
    /// the number of units attempted, so a caller can stop at its cycle budget.
    /// Every unit also updates [`Self::bus_counters`] and, when a counter bank
    /// is wired, the `BUSCTRL` performance counters.
    /// 通道按轮转顺序访问。通道在 `EN` 与 `BUSY` 置位且其 `TREQ_SEL` 请求断言时
    /// 可执行。每次调用对每个数据单元执行一次总线读与一次总线写；完成时清除
    /// `BUSY`、在未置 `IRQ_QUIET` 时置位 `INTR`，并启动 `CHAIN_TO`。返回尝试
    /// 执行的单元数，调用方可据此按周期预算停止。每个单元还会更新
    /// [`Self::bus_counters`]，已接线计数器组时也更新 `BUSCTRL` 性能计数器。
    #[must_use]
    pub fn run(&mut self, bus: &mut dyn AddressBus, max_transfers: u64) -> u64 {
        let mut performed = 0u64;
        let mut cycles = 0u64;
        while performed < max_transfers {
            let Some(job) = self.claim_job() else {
                break;
            };
            let read_start = cycles;
            let mut read = None;
            let mut write = None;
            let outcome = match read_unit(bus, job.read_addr, job.data_size, &mut cycles) {
                Ok(value) => {
                    let read_cycles = cycles - read_start;
                    read = Some(read_cycles);
                    // 嗅探位于通道字节交换之后：先做 BSWAP，再送入校验引擎，
                    // 于是 CTRL.BSWAP 与 SNIFF_CTRL.BSWAP 同时置位时相互抵消。
                    // The sniffer sits downstream of the channel byte swap, so
                    // BSWAP runs first: CTRL.BSWAP and SNIFF_CTRL.BSWAP cancel.
                    let value = apply_bswap(job.ctrl, value, job.data_size);
                    if let Some(perf) = &self.perf {
                        perf.record_read(job.read_addr, read_cycles);
                    }
                    self.sniff_unit(job.channel, job.ctrl, value, job.data_size);
                    let write_start = cycles;
                    let outcome =
                        write_unit(bus, job.write_addr, job.data_size, value, &mut cycles)
                            .map_err(|_| BusFault::Write);
                    let write_cycles = cycles - write_start;
                    if outcome.is_ok() {
                        write = Some(write_cycles);
                        if let Some(perf) = &self.perf {
                            perf.record_write(job.write_addr, write_cycles);
                        }
                    }
                    outcome
                }
                Err(_) => Err(BusFault::Read),
            };
            self.finish_job(job.channel, outcome.err(), read, write);
            performed += 1;
        }
        performed
    }

    /// Feed one observed read unit into the sniff accumulator
    /// 把一个观察到的读单元送入嗅探累加器
    ///
    /// Accumulation requires all three conditions: `SNIFF_CTRL.EN` is set, the
    /// channel carries `CTRL_TRIG.SNIFF_EN`, and the channel index equals
    /// `SNIFF_CTRL.DMACH`.
    /// 累加需要同时满足三个条件：`SNIFF_CTRL.EN` 置位、通道置
    /// `CTRL_TRIG.SNIFF_EN`、且通道号等于 `SNIFF_CTRL.DMACH`。
    fn sniff_unit(&self, channel: usize, ctrl: u32, value: u32, data_size: u8) {
        if ctrl & CTRL_SNIFF_EN == 0 {
            return;
        }
        let mut state = self.state.lock();
        let sniff_ctrl = state.sniff_ctrl;
        if sniff_ctrl & SNIFF_EN == 0 {
            return;
        }
        let dmach = ((sniff_ctrl & SNIFF_DMACH_MASK) >> SNIFF_DMACH_SHIFT) as usize;
        if dmach != channel {
            return;
        }
        sniff::accumulate(sniff_ctrl, &mut state.sniff_data, value, data_size);
    }

    /// Drive one host DREQ input; selectors `>= 64` are ignored
    /// 驱动一个宿主 DREQ 输入；`>= 64` 的选择被忽略
    ///
    /// The level is sampled by [`Self::run`] before every unit. The permanent
    /// request (`0x3F`) ignores this array entirely. The register file lives
    /// behind a mutex, so this only needs `&self`.
    /// 每次执行数据单元前由 [`Self::run`] 采样该电平。永久请求（`0x3F`）完全
    /// 忽略该数组。寄存器文件位于互斥锁之后，因此只需要 `&self`。
    pub fn set_dreq(&self, dreq: u8, asserted: bool) {
        let mut state = self.state.lock();
        if let Some(level) = state.dreq.get_mut(usize::from(dreq)) {
            *level = asserted;
        }
    }

    /// Drive eight consecutive DREQ inputs from a bitmask
    /// 由位掩码驱动连续八个 DREQ 输入
    ///
    /// Bit `n` of `levels` becomes the level of selector `first + n`; selectors
    /// at or above 64 are skipped. One lock covers the whole bank, which is how
    /// the PIO blocks publish their four TX and four RX levels at once.
    /// `levels` 的第 `n` 位写入选择器 `first + n` 的电平；`>= 64` 的选择被跳过。
    /// 整组共用一次加锁，PIO 块据此一次发布四个 TX 与四个 RX 电平。
    pub fn set_dreq_bank(&self, first: u8, levels: u32) {
        let mut state = self.state.lock();
        for bit in 0..8u8 {
            let Some(level) = state.dreq.get_mut(usize::from(first.saturating_add(bit))) else {
                continue;
            };
            *level = levels & (1 << bit) != 0;
        }
    }

    /// Whether a busy channel currently selects a DREQ in `first..=last`
    /// 当前是否有忙碌通道选择了 `first..=last` 范围内的 DREQ
    ///
    /// The host uses this to pick the DREQ refresh granularity: a channel paced
    /// by a PIO FIFO must be resynchronised per unit, while permanent or
    /// unmodelled selectors can run a whole batch. The permanent request is
    /// never reported.
    /// 宿主据此选择 DREQ 刷新粒度：由 PIO FIFO 节拍的通道必须逐单元重新同步，
    /// 永久请求或未建模选择器可整批执行。永久请求不计入。
    #[must_use]
    pub fn busy_selects_dreq(&self, first: u8, last: u8) -> bool {
        let state = self.state.lock();
        state.channels.iter().any(|channel| {
            if !channel.busy || channel.ctrl & CTRL_EN == 0 {
                return false;
            }
            let selector = ((channel.ctrl & CTRL_TREQ_SEL_MASK) >> CTRL_TREQ_SEL_SHIFT) as u8;
            selector != TREQ_PERMANENT && selector >= first && selector <= last
        })
    }

    /// Whether any enabled channel selects a DREQ in `first..=last`
    /// 是否有任一已使能通道选择了 `first..=last` 范围内的 DREQ
    ///
    /// The host uses this to pick the DREQ refresh granularity. Unlike
    /// [`Self::busy_selects_dreq`] it ignores `BUSY`, which matters at the
    /// moment a channel is triggered: a channel that is enabled but not yet
    /// marked busy must still be resynchronised per unit, otherwise a whole
    /// batch runs with a stale DREQ level and overruns a small FIFO (HSTX).
    /// 宿主据此选择 DREQ 刷新粒度。与 [`Self::busy_selects_dreq`] 不同，它忽略
    /// `BUSY`，这在通道刚被触发的瞬间很关键：已使能但尚未置 busy 的通道仍必须
    /// 逐单元重新同步，否则整批会用陈旧的 DREQ 电平执行并冲垮小 FIFO（HSTX）。
    ///
    /// The permanent request is never reported.
    /// 永久请求不计入。
    #[must_use]
    pub fn enabled_selects_dreq(&self, first: u8, last: u8) -> bool {
        let state = self.state.lock();
        state.channels.iter().any(|channel| {
            if channel.ctrl & CTRL_EN == 0 {
                return false;
            }
            let selector = ((channel.ctrl & CTRL_TREQ_SEL_MASK) >> CTRL_TREQ_SEL_SHIFT) as u8;
            selector != TREQ_PERMANENT && selector >= first && selector <= last
        })
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
            trans_count_mode: channel.trans_count_mode,
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
    ///
    /// High-priority channels are considered first; normal channels are then
    /// visited round-robin. Within one priority class the cursor keeps the
    /// existing rotation order.
    /// 先考虑高优先级通道，随后轮转普通通道；同一优先级内游标保持原有轮转顺序。
    fn claim_job(&self) -> Option<DmaJob> {
        let mut state = self.state.lock();
        let start = state.cursor;
        for high_priority in [true, false] {
            for step in 0..CHANNELS {
                let index = (start + step) % CHANNELS;
                let channel = &state.channels[index];
                let eligible = channel.busy
                    && channel.ctrl & CTRL_EN != 0
                    && (channel.ctrl & CTRL_HIGH_PRIORITY != 0) == high_priority;
                if !eligible {
                    continue;
                }
                let treq = ((channel.ctrl & CTRL_TREQ_SEL_MASK) >> CTRL_TREQ_SEL_SHIFT) as u8;
                if treq != TREQ_PERMANENT && !state.dreq[usize::from(treq)] {
                    continue;
                }
                let job = DmaJob {
                    channel: index,
                    read_addr: channel.read_addr,
                    write_addr: channel.write_addr,
                    data_size: ((channel.ctrl & CTRL_DATA_SIZE_MASK) >> CTRL_DATA_SIZE_SHIFT) as u8,
                    ctrl: channel.ctrl,
                };
                // 定时器节拍是单次脉冲：被通道接受后立即撤销并重载 Y。
                // A pacing tick is a one-shot pulse: accepting it deasserts the
                // request and reloads the countdown from Y.
                if let Some(timer) = pacing::timer_index(treq) {
                    pacing::consume(&mut state, timer);
                }
                state.cursor = (index + 1) % CHANNELS;
                return Some(job);
            }
        }
        None
    }

    /// Commit one unit: advance addresses, decrement the count, chain and IRQ
    /// 提交一个数据单元：推进地址、递减计数、链式启动与中断
    ///
    /// `read`/`write` carry the bus cycles each side consumed; `None` means the
    /// access failed, so it is not counted as a completed unit.
    /// `read`/`write` 携带两侧各消耗的总线周期；`None` 表示访问失败，因此不计入
    /// 完成单元。
    fn finish_job(
        &self,
        index: usize,
        fault: Option<BusFault>,
        read: Option<u64>,
        write: Option<u64>,
    ) {
        let mut state = self.state.lock();
        state.bus.grants = state.bus.grants.saturating_add(1);
        if let Some(perf) = &self.perf {
            perf.record_grant();
        }
        if let Some(cycles) = read {
            state.bus.read_units = state.bus.read_units.saturating_add(1);
            state.bus.read_cycles = state.bus.read_cycles.saturating_add(cycles);
        }
        if let Some(cycles) = write {
            state.bus.write_units = state.bus.write_units.saturating_add(1);
            state.bus.write_cycles = state.bus.write_cycles.saturating_add(cycles);
        }
        let mut completed = false;
        let mut quiet = true;
        let mut chain = index;
        let mut retrigger = false;
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
                // ENDLESS 冻结计数：永不完成、不链式、不产生中断，直到 CHAN_ABORT。
                // ENDLESS freezes the count: no completion, chaining or IRQ until
                // CHAN_ABORT.
                if channel.trans_count_mode != TRANS_COUNT_MODE_ENDLESS {
                    channel.trans_count = channel.trans_count.saturating_sub(1);
                    completed = channel.trans_count == 0;
                    quiet = channel.ctrl & CTRL_IRQ_QUIET != 0;
                    chain = ((channel.ctrl & CTRL_CHAIN_TO_MASK) >> CTRL_CHAIN_TO_SHIFT) as usize;
                    if completed {
                        channel.busy = false;
                        retrigger = channel.trans_count_mode == TRANS_COUNT_MODE_TRIGGER_SELF;
                    }
                }
            }
        }
        if completed {
            if !quiet {
                state.intr |= 1u16 << index;
            }
            if chain != index {
                trigger_channel(&mut state, chain);
            }
            if retrigger {
                // 自触发：用重载计数立即重启；重载为 0 时保持空闲。
                // Self-trigger: restart immediately from the reload count; a
                // zero reload leaves the channel idle.
                let channel = &mut state.channels[index];
                channel.trans_count = channel.trans_count_reload;
                channel.busy = channel.trans_count != 0;
            }
        }
        drop(state);
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

/// Start a channel from a trigger, reloading its working count
/// 由触发器启动通道并重新装载工作计数
///
/// Trigger rules modelled from the datasheet and the Pico SDK: only an
/// enabled channel that is not already busy can be started, `TRANS_COUNT` is
/// reloaded from the rewrite value on **every** start, and a zero-length
/// channel completes immediately (raising `INTR` unless `IRQ_QUIET`, then
/// following `CHAIN_TO`). The reload is what keeps control-block rings alive:
/// the SDK configures the CB channels with `TRANS_COUNT = 1` once and then
/// re-triggers them forever through `CHAIN_TO`.
/// 依据数据手册与 Pico SDK 建模的触发规则：只有已使能且不忙碌的通道能被启动；
/// **每次**启动都会从重载值重新装载 `TRANS_COUNT`；零长度通道立即完成（未置
/// `IRQ_QUIET` 时置位 `INTR`，随后跟踪 `CHAIN_TO`）。重载是控制块环持续运行
/// 的关键：SDK 只把 CB 通道的 `TRANS_COUNT` 配置为 1，之后完全依赖 `CHAIN_TO`
/// 无限次重新触发。
fn trigger_channel(state: &mut DmaState, mut index: usize) {
    // 零长度传输会立即完成并继续链式传递；最多遍历一轮以避免环路死循环。
    // A zero-length transfer completes immediately and continues chaining;
    // iterate at most one full round to avoid a cyclic chain spinning forever.
    for _ in 0..CHANNELS {
        let (enabled, busy, reload, quiet, chain) = {
            let channel = &state.channels[index];
            (
                channel.ctrl & CTRL_EN != 0,
                channel.busy,
                channel.trans_count_reload,
                channel.ctrl & CTRL_IRQ_QUIET != 0,
                ((channel.ctrl & CTRL_CHAIN_TO_MASK) >> CTRL_CHAIN_TO_SHIFT) as usize,
            )
        };
        // 已禁用的通道被忽略；已在传输的通道不会被打断，也不会重置计数。
        // A disabled channel is ignored; a busy channel is neither restarted
        // nor forced to reload its count.
        if !enabled || busy {
            return;
        }
        state.channels[index].trans_count = reload;
        if reload != 0 {
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

/// Apply the `BSWAP` byte transformation to one data unit
/// 对一个数据单元应用 `BSWAP` 字节变换
///
/// Byte units are unaffected, halfwords swap their two bytes and words reverse
/// all four; the reserved `DATA_SIZE = 3` follows the word path.
/// 字节单元不受影响，半字交换两个字节，字反转四个字节；保留的 `DATA_SIZE = 3`
/// 跟随字路径。
fn apply_bswap(ctrl: u32, value: u32, data_size: u8) -> u32 {
    if ctrl & CTRL_BSWAP == 0 {
        return value;
    }
    match data_size {
        0 => value,
        1 => u32::from((value as u16).swap_bytes()),
        _ => value.swap_bytes(),
    }
}
