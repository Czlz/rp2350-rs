//! Shared PIO block handle: MMIO adapter, host tick and IRQ routing
//! 共享 PIO 块句柄：MMIO 适配器、宿主 tick 与中断路由
//!
//! [`SharedPio`] owns one [`Pio`] register file behind `Arc<Mutex<_>>`. The bus
//! adapter installed by [`crate::BootPeripherals::mount`] and the handle
//! returned by [`crate::BootPeripherals::pio`] therefore share the same state:
//! a guest MMIO write is immediately visible to the host, and a host
//! [`SharedPio::tick`] advances the state machines the guest programmed. No
//! register or FIFO is copied.
//! [`SharedPio`] 用 `Arc<Mutex<_>>` 持有一个 [`Pio`] 寄存器文件。
//! [`crate::BootPeripherals::mount`] 安装的总线适配器与
//! [`crate::BootPeripherals::pio`] 返回的句柄因此共享同一份状态：客户机的 MMIO
//! 写入对宿主立即可见，宿主的 [`SharedPio::tick`] 推进的正是客户机编程的状态机。
//! 寄存器与 FIFO 都不会被复制。
//!
//! # Interrupt routing / 中断路由
//!
//! The eight `IRQ` flags are grouped exactly as on silicon: flags `0..=3`
//! assert the block's `PIOx_IRQ_0` line and flags `4..=7` assert `PIOx_IRQ_1`
//! (RP2350 interrupt numbers `15..=20`). Every path that can change a flag
//! publishes the resulting level to the shared [`IrqAssert`]: `IRQ`/`IRQ_FORCE`
//! and write-one-to-clear writes through the bus adapter, the `IRQ` instruction
//! executed by [`SharedPio::tick`], and every [`SharedPio::with_mut`] edit.
//! `IRQ0_INTE`/`IRQ1_INTE` are not modelled, so a valid flag asserts its line
//! directly, and a write-one-to-clear releases it again.
//! 八个 `IRQ` 标志按硬件方式分组：标志 `0..=3` 断言本块的 `PIOx_IRQ_0` 线，
//! 标志 `4..=7` 断言 `PIOx_IRQ_1`（RP2350 中断号 `15..=20`）。所有可能改变标志的
//! 路径都会把结果电平发布到共享 [`IrqAssert`]：总线适配器上的 `IRQ`/`IRQ_FORCE`
//! 与写 1 清零、[`SharedPio::tick`] 执行的 `IRQ` 指令，以及每次
//! [`SharedPio::with_mut`] 修改。`IRQ0_INTE`/`IRQ1_INTE` 未建模，因此有效标志
//! 直接断言对应线，写 1 清零后随即释放。

use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_core::mmio::{MmioDevice, MmioWindow};

use super::Pio;
use super::regs::EXECCTRL_EXEC_STALLED;
use crate::irq::{IrqAssert, PIO_IRQ_LINES};
use crate::map;

/// `IRQ` flags routed to the block's `IRQ_0` line / 路由到块 `IRQ_0` 线的 `IRQ` 标志
pub const IRQ0_FLAGS: u32 = 0x0000_000F;

/// `IRQ` flags routed to the block's `IRQ_1` line / 路由到块 `IRQ_1` 线的 `IRQ` 标志
pub const IRQ1_FLAGS: u32 = 0x0000_00F0;

/// Publish the raw `IRQ` flags of block `index` to its two shared lines
/// 把块 `index` 的 `IRQ` 原始标志发布到它的两条共享线
///
/// The datasheet puts flags `0..=3` behind `IRQ_0` and `4..=7` behind `IRQ_1`.
/// 数据手册把标志 `0..=3` 放在 `IRQ_0` 之后、`4..=7` 放在 `IRQ_1` 之后。
fn publish(irq: &IrqAssert, index: u8, flags: u32) {
    let lines = PIO_IRQ_LINES[usize::from(index)];
    irq.set(lines[0], flags & IRQ0_FLAGS != 0);
    irq.set(lines[1], flags & IRQ1_FLAGS != 0);
}

/// Shared handle for one RP2350 PIO block / 一个 RP2350 PIO 块的共享句柄
///
/// Clones share the register file, instruction memory, FIFOs and pin vectors,
/// so the bus adapter and every host-side clone observe the same words. The
/// handle also owns the block's two interrupt lines, which are refreshed from
/// the `IRQ` flags after any state change.
/// 克隆共享寄存器文件、指令存储、FIFO 与引脚向量，因此总线适配器与所有宿主侧
/// 克隆看到相同内容。该句柄同时持有本块的两条中断线，任何状态变化后都会按
/// `IRQ` 标志刷新。
#[derive(Debug, Clone)]
pub struct SharedPio {
    index: u8,
    inner: Arc<Mutex<Pio>>,
    irq: IrqAssert,
}

impl SharedPio {
    /// Create a shared PIO block by index (`0`..=`2`) / 按序号（`0`..=`2`）创建共享 PIO 块
    ///
    /// # Panics
    /// Panics when `index` is not a valid RP2350 PIO block index.
    /// 当 `index` 不是合法的 RP2350 PIO 块序号时 panic。
    #[must_use]
    pub fn new(index: u8, irq: IrqAssert) -> Self {
        Self {
            index,
            inner: Arc::new(Mutex::new(Pio::new(index))),
            irq,
        }
    }

    /// Block index (`0` = PIO0, `1` = PIO1, `2` = PIO2) / 块序号（`0` = PIO0，`1` = PIO1，`2` = PIO2）
    #[must_use]
    pub fn index(&self) -> u8 {
        self.index
    }

    /// MMIO adapter for the block's 4 KB window / 本块 4 KB 窗口的 MMIO 适配器
    ///
    /// The adapter shares the register file with this handle, so it can be
    /// mounted on a bus while the host keeps calling [`SharedPio::tick`].
    /// 该适配器与本句柄共享寄存器文件，因此可以挂载到总线，同时宿主继续调用
    /// [`SharedPio::tick`]。
    #[must_use]
    pub fn device(&self) -> Box<dyn MmioDevice> {
        Box::new(PioWindow {
            index: self.index,
            inner: Arc::clone(&self.inner),
            irq: self.irq.clone(),
        })
    }

    /// Run `f` against the shared register file / 在共享寄存器文件上执行 `f`
    pub fn with<R>(&self, f: impl FnOnce(&Pio) -> R) -> R {
        f(&self.inner.lock())
    }

    /// Run `f` against the shared register file and publish IRQ flags
    /// 在共享寄存器文件上执行 `f` 并发布 IRQ 标志
    ///
    /// The block's two interrupt lines are refreshed after the closure returns,
    /// because `f` may set or clear `IRQ` flags directly.
    /// 闭包返回后会刷新本块的两条中断线，因为 `f` 可能直接置位或清除 `IRQ` 标志。
    pub fn with_mut<R>(&self, f: impl FnOnce(&mut Pio) -> R) -> R {
        let mut pio = self.inner.lock();
        let result = f(&mut pio);
        publish(&self.irq, self.index, pio.irq);
        result
    }

    /// Advance every enabled state machine by `cycles` PIO clock cycles
    /// 让所有已启用状态机推进 `cycles` 个 PIO 时钟周期
    ///
    /// One PIO clock cycle is modelled per caller-supplied cycle; the API
    /// matches [`Pio::tick`]. A block with `CTRL.SM_ENABLE = 0` returns after
    /// one uncontended lock and does no per-machine work, and the interrupt
    /// lines are only republished when the `IRQ` flags actually changed.
    /// 每个调用方给出的周期对应一个 PIO 时钟周期，语义与 [`Pio::tick`] 一致。
    /// `CTRL.SM_ENABLE = 0` 的块在一次无竞争加锁后即返回、不做任何状态机工作；
    /// 只有 `IRQ` 标志真正变化时才重新发布中断线。
    pub fn tick(&self, cycles: u64) {
        let mut pio = self.inner.lock();
        if pio.enabled == 0 {
            return;
        }
        pio.tick(cycles);
        publish(&self.irq, self.index, pio.irq);
    }

    /// Refresh the block's interrupt lines from its raw `IRQ` flags
    /// 按原始 `IRQ` 标志刷新本块的中断线
    pub fn sync_irqs(&self) {
        let pio = self.inner.lock();
        publish(&self.irq, self.index, pio.irq);
    }

    /// Current `IRQ` flag register / 当前的 `IRQ` 标志寄存器
    #[must_use]
    pub fn irq_flags(&self) -> u32 {
        self.inner.lock().irq
    }

    /// `CTRL.SM_ENABLE` readback / `CTRL.SM_ENABLE` 回读
    #[must_use]
    pub fn enabled_mask(&self) -> u8 {
        self.inner.lock().enabled
    }

    /// Machines blocked in `EXECCTRL.EXEC_STALLED` / 阻塞在 `EXECCTRL.EXEC_STALLED` 的状态机
    #[must_use]
    pub fn stalled_mask(&self) -> u8 {
        let pio = self.inner.lock();
        let mut mask = 0;
        for (index, machine) in pio.sm.iter().enumerate() {
            if machine.execctrl & EXECCTRL_EXEC_STALLED != 0 {
                mask |= 1 << index;
            }
        }
        mask
    }

    /// Program counter of state machine `sm` / 状态机 `sm` 的程序计数器
    #[must_use]
    pub fn pc(&self, sm: usize) -> Option<u8> {
        self.inner.lock().sm.get(sm).map(|machine| machine.pc)
    }

    /// Output levels currently driven by the block / 本块当前驱动的输出电平
    #[must_use]
    pub fn output_levels(&self) -> u32 {
        self.inner.lock().output_levels
    }

    /// Output enables currently driven by the block / 本块当前驱动的输出使能
    #[must_use]
    pub fn output_enables(&self) -> u32 {
        self.inner.lock().output_enables
    }
}

/// MMIO proxy for one shared PIO block / 一个共享 PIO 块的 MMIO 代理
///
/// It forwards every access to the shared register file and republishes the
/// interrupt lines after a state-changing write or reset.
/// 它把所有访问转发到共享寄存器文件，并在改变状态的写入或复位后重新发布中断线。
#[derive(Debug)]
struct PioWindow {
    index: u8,
    inner: Arc<Mutex<Pio>>,
    irq: IrqAssert,
}

impl MmioDevice for PioWindow {
    fn name(&self) -> &str {
        Pio::block_name(self.index)
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(Pio::block_base(self.index), map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        self.inner.lock().read32(offset)
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        let mut pio = self.inner.lock();
        let accepted = pio.write32(offset, value);
        if accepted {
            publish(&self.irq, self.index, pio.irq);
        }
        accepted
    }

    fn reset(&mut self) {
        let mut pio = self.inner.lock();
        pio.reset();
        publish(&self.irq, self.index, pio.irq);
    }

    /// First interrupt line of the block (`PIOx_IRQ_0`)
    /// 本块的第一条中断线（`PIOx_IRQ_0`）
    ///
    /// A PIO block owns two lines; the pair is exposed through
    /// [`PIO_IRQ_LINES`] and this single-value hook reports line 0.
    /// PIO 块有两条中断线；成对定义见 [`PIO_IRQ_LINES`]，这个单值接口报告 0 号线。
    fn irq_number(&self) -> Option<u16> {
        Some(PIO_IRQ_LINES[usize::from(self.index)][0])
    }
}
