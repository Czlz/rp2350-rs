//! SIO peripheral model / SIO 外设模型
//!
//! Models the single-cycle GPIO output, output-enable and input registers, the
//! inter-core FIFO (8 entries), spinlocks and the RP2350 doorbells. GPIO values
//! loop back as inputs; the pin matrix between SIO, IO_BANK0 and PADS_BANK0 is
//! not connected yet. This block represents core 0's SIO window: FIFO writes go
//! to the peer core's receive queue and FIFO reads drain the queue fed by the
//! peer, so no core-1 scheduler is required to model the registers faithfully.
//! 建模单周期 GPIO 输出、输出使能与输入寄存器、核间 FIFO（8 项）、自旋锁与
//! RP2350 门铃。GPIO 值回环为输入；SIO、IO_BANK0 与 PADS_BANK0 之间的引脚
//! 矩阵尚未连接。本块代表 core 0 的 SIO 窗口：FIFO 写进入对端核心的接收队列，
//! FIFO 读取消费对端送入的数据，因此无需 core 1 调度器即可忠实建模寄存器。
//!
//! Register offsets follow the RP2350 register map (足 `hardware/regs/sio.h`):
//! `GPIO_HI_*` is interleaved with `GPIO_*`, and the doorbells sit at `0x180`.
//! 寄存器偏移遵循 RP2350 寄存器表（`hardware/regs/sio.h`）：`GPIO_HI_*` 与
//! `GPIO_*` 交错排列，门铃位于 `0x180`。

use std::collections::VecDeque;
use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_core::mmio::{MmioDevice, MmioWindow};

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
    /// Frames sent to the peer core / 发往对端核心的数据
    tx: VecDeque<u32>,
    /// Frames received from the peer core / 来自对端核心的数据
    rx: VecDeque<u32>,
    /// Sticky write-overflow flag / 粘滞写溢出标志
    wof: bool,
    /// Sticky read-overflow flag / 粘滞读溢出标志
    roe: bool,
    /// Doorbells posted to this core / 投递给本核心的门铃
    bell_in: u8,
    /// Doorbells posted to the peer core / 投递给对端核心的门铃
    bell_out: u8,
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

    /// `FIFO_ST` value / `FIFO_ST` 值
    fn fifo_status(&self) -> u32 {
        let mut status = 0;
        if !self.rx.is_empty() {
            status |= 1; // VLD
        }
        if self.tx.len() < FIFO_DEPTH {
            status |= 1 << 1; // RDY
        }
        if self.wof {
            status |= 1 << 2; // WOF
        }
        if self.roe {
            status |= 1 << 3; // ROE
        }
        status
    }
}

/// SIO register block for core 0 / core 0 的 SIO 寄存器块
#[derive(Debug, Clone)]
pub struct Sio {
    state: Arc<Mutex<SioState>>,
    irq: IrqAssert,
}

impl Sio {
    /// Create the block with empty FIFOs and all pins low
    /// 创建 FIFO 为空、所有引脚为低的寄存器块
    #[must_use]
    pub fn new(irq: IrqAssert) -> Self {
        Self {
            state: Arc::new(Mutex::new(SioState::default())),
            irq,
        }
    }

    /// Hand a frame to this core's receive FIFO (peer-core hook)
    /// 向本核心的接收 FIFO 送入一帧（对端核心挂钩）
    ///
    /// A future core-1 scheduler uses this to answer a core-0 `FIFO_WR`; it also
    /// lets tests exercise the receive path without a second core.
    /// 未来的 core 1 调度器用它回应 core 0 的 `FIFO_WR`；测试也可借此在没有
    /// 第二个核心时验证接收路径。
    pub fn post_to_core(&self, value: u32) {
        self.state.lock().rx.push_back(value);
        self.refresh_irq();
    }

    /// Ring this core's doorbell (peer-core hook)
    /// 敲响本核心的门铃（对端核心挂钩）
    ///
    /// A core-1 scheduler calls this when core 1 writes its `DOORBELL_OUT_SET`;
    /// the bits land in this core's `DOORBELL_IN` and assert `SIO_IRQ_BELL`.
    /// core 1 写入自己的 `DOORBELL_OUT_SET` 时，core 1 调度器调用此函数；位落入
    /// 本核心的 `DOORBELL_IN` 并断言 `SIO_IRQ_BELL`。
    pub fn ring_core(&self, mask: u8) {
        self.state.lock().bell_in |= mask;
        self.refresh_irq();
    }

    /// Drain the frames this core sent to the peer (peer-core hook)
    /// 取走本核心发往对端的所有帧（对端核心挂钩）
    #[must_use]
    pub fn take_from_core(&self) -> Vec<u32> {
        self.state.lock().tx.drain(..).collect()
    }

    /// Doorbells posted to the peer core / 投递给对端核心的门铃位
    #[must_use]
    pub fn peer_bells(&self) -> u8 {
        self.state.lock().bell_out
    }

    /// Recompute the SIO interrupt lines / 重新计算 SIO 中断线
    ///
    /// `SIO_IRQ_FIFO` follows the receive FIFO's `VLD` flag and `SIO_IRQ_BELL`
    /// follows the doorbells posted to this core.
    /// `SIO_IRQ_FIFO` 跟随接收 FIFO 的 `VLD` 标志，`SIO_IRQ_BELL` 跟随投递给
    /// 本核心的门铃。
    pub fn refresh_irq(&self) {
        let (fifo, bell) = {
            let state = self.state.lock();
            (!state.rx.is_empty(), state.bell_in != 0)
        };
        self.irq.set(map::SIO_IRQ_FIFO, fifo);
        self.irq.set(map::SIO_IRQ_BELL, bell);
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
        let mut state = self.state.lock();
        let value = match offset {
            CPUID => Some(0),
            // GPIO inputs loop the outputs back. GPIO 输入回环输出值。
            GPIO_IN | GPIO_OUT => Some(state.out),
            GPIO_HI_IN | GPIO_HI_OUT => Some(state.hi_out),
            GPIO_OE => Some(state.oe),
            GPIO_HI_OE => Some(state.hi_oe),
            FIFO_ST => Some(state.fifo_status()),
            FIFO_RD => {
                let value = state.rx.pop_front().unwrap_or_else(|| {
                    state.roe = true;
                    0
                });
                Some(value)
            }
            // All 32 spinlocks report free. 全部 32 个自旋锁报告空闲。
            SPINLOCK_ST => Some(0xFFFF_FFFF),
            DOORBELL_OUT_SET | DOORBELL_OUT_CLR => Some(u32::from(state.bell_out)),
            DOORBELL_IN_SET | DOORBELL_IN_CLR => Some(u32::from(state.bell_in)),
            _ => None,
        };
        drop(state);
        self.refresh_irq();
        value
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
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
            // FIFO_WR pushes towards the peer; a full FIFO sets WOF.
            // FIFO_WR 向对端压入；FIFO 满时置位 WOF。
            FIFO_WR => {
                if state.tx.len() < FIFO_DEPTH {
                    state.tx.push_back(value);
                } else {
                    state.wof = true;
                }
            }
            DOORBELL_OUT_SET => state.bell_out |= (value & 0xFF) as u8,
            DOORBELL_OUT_CLR => state.bell_out &= !(value & 0xFF) as u8,
            DOORBELL_IN_SET => state.bell_in |= (value & 0xFF) as u8,
            DOORBELL_IN_CLR => state.bell_in &= !(value & 0xFF) as u8,
            _ => {
                drop(state);
                return false;
            }
        }
        drop(state);
        self.refresh_irq();
        true
    }

    fn reset(&mut self) {
        *self.state.lock() = SioState::default();
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
}
