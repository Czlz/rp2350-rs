//! Stage-4 boot peripheral set / Stage 4 启动外设集合
//!
//! [`BootPeripherals::mount`] installs the APB router, SIO and PPB into a bus,
//! returning the shared handles the simulation driver needs: the UART0 transmit
//! sink, the [`Timebase`] used by TIMER0, the [`IrqAssert`] set carrying
//! asserted interrupt lines and the PPB system-write log that must be replayed
//! into the core with `CpuCore::system_write`.
//! [`BootPeripherals::mount`] 将 APB 路由器、SIO 与 PPB 安装到总线，并返回
//! 仿真驱动所需的共享句柄：UART0 发送缓冲、TIMER0 使用的 [`Timebase`]、携带
//! 中断线断言状态的 [`IrqAssert`]，以及需要用 `CpuCore::system_write` 回放
//! 到核中的 PPB 系统写入日志。
//!
//! It also owns the resident core-1 boot-ROM peer: [`BootPeripherals::service_core1_peer`]
//! acknowledges a `PSM` release with a `0` frame, echoes the SDK's
//! `{0, 0, 1, vtor, sp, entry}` launch stream and then retires, exposing the
//! vectors through [`BootPeripherals::take_core1_launch`] so a driver can boot a
//! real core 1. [`BootPeripherals::core1_irq_mask`] is that core's view of the
//! shared interrupt lines.
//! 它还持有常驻的 core1 boot ROM 对端：[`BootPeripherals::service_core1_peer`]
//! 用一帧 `0` 确认 `PSM` 释放，回显 SDK 的 `{0, 0, 1, vtor, sp, entry}` 启动流，
//! 随后退役并通过 [`BootPeripherals::take_core1_launch`] 暴露启动向量，使驱动器
//! 能启动真实的 core1。[`BootPeripherals::core1_irq_mask`] 是该核心视角下的共享
//! 中断线。

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use rp2350_core::bus::{AddressBus, BusResult};
use rp2350_core::mmio::{MMIO_BLOCK_SIZE, MmioRouter, MmioWindow, mount_device};

use super::adc::Adc;
use super::audio::AudioTap;
use super::bootram::BootRam;
use super::busctrl::BusCtrl;
use super::clocks::Clocks;
use super::cycle::CycleSink;
use super::dma::Dma;
use super::dreq;
use super::gpio_pads::GpioPads;
use super::hstx::Hstx;
use super::io_bank0::IoBank0;
use super::io_qspi::IoQspi;
use super::irq::IrqAssert;
use super::map;
use super::pads_bank0::PadsBank0;
use super::pads_qspi::PadsQspi;
use super::pio::{PIO_BLOCKS, PIO_SM_COUNT, Pio, SharedPio};
use super::pll::Pll;
use super::powman::Powman;
use super::ppb::{Ppb, SystemWriteLog, new_system_write_log};
use super::psm::Psm;
use super::pwm::Pwm;
use super::resets::Resets;
use super::rosc::Rosc;
use super::sha256::Sha256;
use super::sio::Sio;
use super::ticks::Ticks;
use super::timer::{Timer0, Timer1};
use super::trng::Trng;
use super::uart::{Uart0, UartSink};
use super::watchdog::Watchdog;
use super::xip_qmi::XipQmi;
use super::xosc::Xosc;

/// Default RP2350 system clock in cycles per microsecond (150 MHz)
/// 默认 RP2350 系统时钟每微秒周期数（150 MHz）
pub const DEFAULT_CYCLES_PER_US: u32 = 150;

/// Cycle-driven microsecond timebase shared with TIMER0
/// 与 TIMER0 共享的周期驱动微秒时基
#[derive(Debug, Clone, Copy)]
pub struct Timebase {
    cycles: u64,
    cycles_per_us: u32,
}

impl Timebase {
    /// Create a timebase with the given clock rate / 以给定时钟频率创建时基
    #[must_use]
    pub const fn new(cycles_per_us: u32) -> Self {
        Self {
            cycles: 0,
            cycles_per_us,
        }
    }

    /// Advance the timebase by executed CPU cycles / 按已执行 CPU 周期推进时基
    pub fn advance(&mut self, cycles: u64) {
        self.cycles = self.cycles.saturating_add(cycles);
    }

    /// Elapsed microseconds / 已经过的微秒数
    #[must_use]
    pub fn time_us(&self) -> u64 {
        match self.cycles_per_us {
            0 => 0,
            rate => self.cycles / u64::from(rate),
        }
    }

    /// Total elapsed cycles / 已经过的总周期数
    #[must_use]
    pub fn cycles(&self) -> u64 {
        self.cycles
    }
}

/// Words in the SDK's `multicore_launch_core1_raw` FIFO command stream
/// SDK `multicore_launch_core1_raw` 的 FIFO 命令流字数
///
/// The stream is `{0, 0, 1, vtor, sp, entry}`: two discard words, a protocol
/// version, then the vector table, stack pointer and entry point.
/// 命令流为 `{0, 0, 1, vtor, sp, entry}`：两个丢弃字、一个协议版本，随后是向量
/// 表、栈指针与入口点。
pub const CORE1_LAUNCH_WORDS: usize = 6;

/// DMA cycles advanced per [`BootPeripherals::pump_dma`] call
/// 每次 [`BootPeripherals::pump_dma`] 调用推进的 DMA 周期数
///
/// A fixed budget keeps the pacing timers ticking without threading the CPU
/// cycle delta through the host pump hook. Reset pacing timers (`X == 0`) never
/// assert, so this is invisible to firmware that does not use them.
/// 固定预算让节拍定时器无需宿主把 CPU 周期差传入泵送钩子；复位的节拍定时器
/// （`X == 0`）永不断言，因此不使用它的固件看不到差异。
const DMA_PUMP_CYCLES: u64 = 64;

/// Boot vectors handed to a launcher after the FIFO launch handshake
/// FIFO 启动握手完成后交给启动器的启动向量
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Core1Launch {
    /// Vector table base (`VTOR`) / 向量表基址（`VTOR`）
    pub vtor: u32,
    /// Initial stack pointer / 初始栈指针
    pub sp: u32,
    /// Entry point with the Thumb bit as sent by the SDK / SDK 发送的带 Thumb 位入口
    pub entry: u32,
}

/// Resident core-1 boot-ROM peer state / 常驻 core1 boot ROM 对端状态
///
/// Until the final `entry` word arrives the peer echoes every launch command,
/// exactly like the mask ROM's core-1 launch handler. Once launched it stops
/// touching the FIFO so real core 1 ↔ core 0 messages pass through.
/// 在最后一个 `entry` 字到达之前，对端会逐字回显启动命令，与掩膜 ROM 的 core1
/// 启动处理程序一致。启动完成后它不再触碰 FIFO，使真实的 core1 ↔ core0 消息
/// 直接通过。
#[derive(Debug, Default)]
struct Core1Boot {
    /// Rolling window of the most recent launch words / 最近的启动字滚动窗口
    window: VecDeque<u32>,
    /// Launch vectors waiting for the driver / 等待驱动器的启动向量
    launch: Option<Core1Launch>,
    /// Whether the echo stub has been retired / 回显桩是否已退役
    launched: bool,
    /// Number of completed launches / 已完成的启动次数
    launches: u64,
}

/// Handles for the peripherals mounted by [`BootPeripherals::mount`]
/// [`BootPeripherals::mount`] 挂载的外设句柄
pub struct BootPeripherals {
    uart0: Arc<Mutex<UartSink>>,
    cycles: CycleSink,
    timer: Timer0,
    timer1: Timer1,
    sio: Sio,
    pads: GpioPads,
    dma: Dma,
    hstx: Hstx,
    pio: [SharedPio; PIO_BLOCKS],
    /// Audio tap installed by the host, if any / 宿主安装的音频 tap（若有）
    ///
    /// Kept so [`BootPeripherals::audio_tap`] can hand the same shared sink back
    /// to another consumer. The authoritative installation lives inside the PIO
    /// block; this field only remembers the most recent one.
    /// 保存该句柄使 [`BootPeripherals::audio_tap`] 能把同一共享缓冲交给另一个消费
    /// 者。实际的安装状态位于 PIO 块内部，本字段只记录最近一次安装。
    audio_tap: Option<AudioTap>,
    irq: IrqAssert,
    system_writes: SystemWriteLog,
    core1_release: Arc<AtomicBool>,
    core1_reset: Arc<AtomicBool>,
    core1_boot: Arc<Mutex<Core1Boot>>,
    core1_launch_pending: Arc<AtomicBool>,
}

impl BootPeripherals {
    /// Mount the Stage-4 boot peripherals on `bus` / 在 `bus` 上挂载 Stage 4 启动外设
    ///
    /// APB devices are installed into one router covering
    /// [`map::APB_BASE`]..[`map::APB_SIZE`]; SIO and PPB get standalone window
    /// regions. Offsets inside APB that no device models are absorbed by the
    /// router with a one-shot warning, which keeps boot firmware running.
    /// APB 设备安装到覆盖 [`map::APB_BASE`]..[`map::APB_SIZE`] 的路由器中；
    /// SIO 与 PPB 使用独立窗口区域。APB 中无设备建模的偏移由路由器吸收并只
    /// 警告一次，从而保持启动固件继续运行。
    ///
    /// # Errors
    /// Returns a bus error if a window is invalid or overlaps a region already
    /// registered on `bus`.
    /// 窗口非法或与 `bus` 已注册区域重叠时返回总线错误。
    pub fn mount(bus: &mut dyn AddressBus) -> BusResult<Self> {
        let uart0 = Arc::new(Mutex::new(UartSink::default()));
        let timebase = Arc::new(Mutex::new(Timebase::new(DEFAULT_CYCLES_PER_US)));
        let cycles = CycleSink::new(Arc::clone(&timebase));
        let irq = IrqAssert::new();
        let system_writes = new_system_write_log();
        let timer = Timer0::new(Arc::clone(&timebase), irq.clone());
        // TIMER1 与 TIMER0 共享同一个微秒时基，但 alarm/中断寄存器与中断线独立。
        // TIMER1 shares the microsecond timebase with TIMER0 but has independent
        // alarm/interrupt registers and interrupt lines.
        let timer1 = Timer1::new(Arc::clone(&timebase), irq.clone());
        // SIO 的 FIFO / 门铃中断按核投递：core0 用共享 `irq`，core1 用独立掩码。
        // SIO's FIFO/doorbell lines are per core: core 0 uses the shared `irq`
        // set and core 1 gets its own mask.
        let core1_irq = IrqAssert::new();
        let pads = GpioPads::new();
        let sio = Sio::with_peer_irq_and_pads(irq.clone(), core1_irq, pads.clone());
        let dma = Dma::new(irq.clone());
        let hstx = Hstx::new(irq.clone());
        let core1_release = Arc::new(AtomicBool::new(false));
        let core1_reset = Arc::new(AtomicBool::new(false));
        let core1_boot = Arc::new(Mutex::new(Core1Boot::default()));
        let core1_launch_pending = Arc::new(AtomicBool::new(false));

        let mut apb = MmioRouter::new("APB", MmioWindow::new(map::APB_BASE, map::APB_SIZE));
        apb.mount(Box::new(Clocks::new()))?;
        apb.mount(Box::new(Resets::new()))?;
        apb.mount(Box::new(Psm::with_core1_flags(
            Arc::clone(&core1_release),
            Arc::clone(&core1_reset),
        )))?;
        // IO_BANK0 与 PADS_BANK0、SIO 共享同一个 pad 句柄，`STATUS.INFROMPAD`
        // 才能反映 PADS_BANK0 配置的上下拉与宿主注入的外部电平。
        // IO_BANK0 shares one pad handle with PADS_BANK0 and SIO so
        // `STATUS.INFROMPAD` reflects the programmed pulls and host levels.
        apb.mount(Box::new(IoBank0::with_pads(pads.clone())))?;
        apb.mount(Box::new(IoQspi::new()))?;
        apb.mount(Box::new(PadsBank0::with_pads(pads.clone())))?;
        apb.mount(Box::new(PadsQspi::new()))?;
        apb.mount(Box::new(Xosc::new()))?;
        apb.mount(Box::new(Pll::sys()))?;
        apb.mount(Box::new(Pll::usb()))?;
        apb.mount(Box::new(Uart0::new(Arc::clone(&uart0))))?;
        apb.mount(Box::new(Adc::new()))?;
        apb.mount(Box::new(Pwm::new()))?;
        apb.mount(Box::new(timer.clone()))?;
        apb.mount(Box::new(timer1.clone()))?;
        // HSTX 控制寄存器位于 APB（0x400C_0000），FIFO 在 AHB 侧独立挂载。
        // The HSTX control registers live in APB (0x400C_0000); the FIFO is
        // mounted separately in AHB space.
        apb.mount(hstx.ctrl_device())?;
        apb.mount(Box::new(Watchdog::new()))?;
        apb.mount(Box::new(XipQmi::new()))?;
        apb.mount(Box::new(BootRam::new()))?;
        apb.mount(Box::new(BusCtrl::new()))?;
        apb.mount(Box::new(Rosc::new()))?;
        apb.mount(Box::new(Trng::new()))?;
        apb.mount(Box::new(Sha256::new()))?;
        bus.register_region(Box::new(apb))?;

        // TICKS 位于 APB 路由器跨度之上，独立挂载；写 CYCLES 会推进共享时基。
        // TICKS sits above the APB router span, so it is mounted standalone;
        // writing CYCLES advances the shared time base.
        mount_device(bus, Box::new(Ticks::new(cycles.clone())))?;
        // POWMAN 位于 APB 跨度之上，但它的 SET/CLR/XOR 别名仍按 4 KB 块折叠，
        // 因此用一个窗口覆盖整块的小路由器承载。
        // POWMAN sits above the APB span, but its SET/CLR/XOR aliases still
        // fold per 4 KB block, so a small router spanning the whole block hosts
        // it.
        let mut aon = MmioRouter::new(
            "AON",
            MmioWindow::new(map::POWMAN_BASE, MMIO_BLOCK_SIZE as usize),
        );
        aon.mount(Box::new(Powman::new()))?;
        bus.register_region(Box::new(aon))?;
        // RP2350 的三个 PIO 块位于 AHB 空间，按基地址顺序独立挂载。共享句柄与
        // 总线适配器共用同一个寄存器文件，因此宿主可以直接 tick 状态机并观察
        // `IRQ` 标志驱动的中断线。PIO 不参与 `advance` 的时基批量推进：机器在
        // DMA 泵送点按客户机步数显式 tick。三个块同时接到共享 pad 矩阵，使
        // `IO_BANK0.FUNCSEL` 选中的 PIO 块才能驱动/采样对应 pad。
        // The three RP2350 PIO blocks live in AHB space and are mounted
        // standalone in base-address order. The shared handles and the bus
        // adapters observe one register file, so the host can tick the state
        // machines and watch the `IRQ` flags drive the interrupt lines. PIO
        // does not take part in `advance`; the machine ticks it explicitly
        // from the DMA pump point. All three blocks share the pad matrix, so
        // only the block selected by `IO_BANK0.FUNCSEL` drives/samples a pad.
        let pio: [SharedPio; PIO_BLOCKS] = core::array::from_fn(|index| {
            SharedPio::with_pads(index as u8, irq.clone(), pads.clone())
        });
        for block in &pio {
            mount_device(bus, block.device())?;
        }
        // HSTX FIFO 位于 AHB 空间（0x5060_0000），与 PIO 块并列。
        // The HSTX FIFO lives in AHB space (0x5060_0000) next to the PIO blocks.
        mount_device(bus, hstx.fifo_device())?;
        // DMA 位于 AHB 空间，与 PIO 块并列；传输由宿主通过 `Dma::run` 驱动。
        // DMA lives in AHB space next to the PIO blocks; transfers are driven
        // by the host through `Dma::run`.
        mount_device(bus, Box::new(dma.clone()))?;
        mount_device(bus, Box::new(sio.clone()))?;
        mount_device(bus, Box::new(Ppb::with_log(Arc::clone(&system_writes))))?;

        Ok(Self {
            uart0,
            cycles,
            timer,
            timer1,
            sio,
            pads,
            dma,
            hstx,
            pio,
            audio_tap: None,
            irq,
            system_writes,
            core1_release,
            core1_reset,
            core1_boot,
            core1_launch_pending,
        })
    }

    /// Emulate the resident core-1 boot ROM once per driver pump
    /// 每次驱动器泵送时模拟常驻 core1 boot ROM
    ///
    /// Two behaviours are modelled. When `PSM_FRCE_OFF.PROC1` is released the
    /// core-1 boot ROM drains its mailbox and pushes `0` to core 0. While no
    /// launch has completed it consumes the `{0, 0, 1, vtor, sp, entry}` command
    /// stream one word at a time and echoes every word back, which is what
    /// `multicore_launch_core1_raw` and the lockout handshake expect. After the
    /// final `entry` word the echo stub retires: subsequent FIFO traffic belongs
    /// to the real core 1 and must not be stolen.
    /// 建模两种行为。`PSM_FRCE_OFF.PROC1` 释放时，core1 boot ROM 清空信箱并向
    /// core0 压入 `0`。在完成一次启动之前，它逐字消费
    /// `{0, 0, 1, vtor, sp, entry}` 命令流并回显每个字，这正是
    /// `multicore_launch_core1_raw` 与锁定握手所期望的。收到最后一个 `entry`
    /// 字后回显桩退役：之后的 FIFO 流量属于真实的 core1，不得被窃取。
    pub fn service_core1_peer(&self) {
        if self.core1_release.swap(false, Ordering::AcqRel) {
            let _ = self.sio.take_from_core();
            self.sio.post_to_core(0);
        }
        let mut boot = self.core1_boot.lock();
        if boot.launched {
            return;
        }
        while let Some(frame) = self.sio.pop_from_core() {
            // 排查核间启动/锁定握手时打印收到的字。
            // Log received words when debugging inter-core launch/lockout.
            if std::env::var_os("RP2350_TRACE_CORE1").is_some() {
                eprintln!("[core1] echo {frame:#010X}");
            }
            self.sio.post_to_core(frame);
            boot.window.push_back(frame);
            while boot.window.len() > CORE1_LAUNCH_WORDS {
                boot.window.pop_front();
            }
            if boot.window.len() < CORE1_LAUNCH_WORDS {
                continue;
            }
            let words: Vec<u32> = boot.window.iter().copied().collect();
            // `{0, 0, 1, ...}` 出现在任意位置都视为一次新的启动请求，与 SDK
            // 写 0、0、1 后依次等待回显的顺序一致。
            // A `{0, 0, 1, ...}` window anywhere in the stream starts a new
            // launch request, matching the SDK's 0, 0, 1 write order.
            if words[0] != 0 || words[1] != 0 || words[2] != 1 {
                continue;
            }
            boot.launch = Some(Core1Launch {
                vtor: words[3],
                sp: words[4],
                entry: words[5],
            });
            boot.launched = true;
            boot.launches = boot.launches.saturating_add(1);
            boot.window.clear();
            self.core1_launch_pending.store(true, Ordering::Release);
            break;
        }
    }

    /// Whether a launch is waiting for [`Self::take_core1_launch`]
    /// 是否有启动信息等待 [`Self::take_core1_launch`] 取走
    ///
    /// A cheap relaxed load intended for the per-step fast path.
    /// 供每步快速路径使用的廉价 relaxed 读取。
    #[must_use]
    pub fn core1_launch_pending(&self) -> bool {
        self.core1_launch_pending.load(Ordering::Relaxed)
    }

    /// Take the pending core-1 launch vectors / 取走挂起的 core1 启动向量
    ///
    /// Returns `None` when no launch has completed or the previous one was
    /// already consumed. The echo stub stays retired either way.
    /// 没有完成启动或上一次已被取走时返回 `None`；无论哪种情况回显桩都保持退役。
    #[must_use]
    pub fn take_core1_launch(&self) -> Option<Core1Launch> {
        let mut boot = self.core1_boot.lock();
        let launch = boot.launch.take();
        if launch.is_some() {
            self.core1_launch_pending.store(false, Ordering::Release);
        }
        launch
    }

    /// Number of completed core-1 launches / 已完成的 core1 启动次数
    #[must_use]
    pub fn core1_launch_count(&self) -> u64 {
        self.core1_boot.lock().launches
    }

    /// Whether the resident boot-ROM echo stub has retired
    /// 常驻 boot ROM 回显桩是否已退役
    #[must_use]
    pub fn core1_launched(&self) -> bool {
        self.core1_boot.lock().launched
    }

    /// Consume a latched `PSM_FRCE_OFF.PROC1` hard reset
    /// 消费锁存的 `PSM_FRCE_OFF.PROC1` 硬复位
    ///
    /// The driver discards the core-1 context and calls
    /// [`Self::reset_core1_boot`] so a later launch replays the echo stub.
    /// 驱动器据此丢弃 core1 上下文，并调用 [`Self::reset_core1_boot`]，使后续
    /// 启动重新启用回显桩。
    #[must_use]
    pub fn take_core1_reset(&self) -> bool {
        self.core1_reset.swap(false, Ordering::AcqRel)
    }

    /// Whether a `PSM` hard reset is waiting for [`Self::take_core1_reset`]
    /// 是否有 `PSM` 硬复位等待 [`Self::take_core1_reset`] 消费
    ///
    /// A cheap relaxed load used as the per-step guard so the hot path does not
    /// perform a read-modify-write atomic on every instruction.
    /// 作为每步守卫的廉价 relaxed 读取，使热路径不必每条指令都做读写原子操作。
    #[must_use]
    pub fn core1_reset_pending(&self) -> bool {
        self.core1_reset.load(Ordering::Relaxed)
    }

    /// Forget the launch state and re-arm the boot-ROM echo stub
    /// 清除启动状态并重新启用 boot ROM 回显桩
    pub fn reset_core1_boot(&self) {
        let mut boot = self.core1_boot.lock();
        boot.window.clear();
        boot.launch = None;
        boot.launched = false;
        self.core1_launch_pending.store(false, Ordering::Release);
    }

    /// Advance the shared timebase and refresh the TIMER interrupt lines
    /// 推进共享时基并刷新 TIMER 中断线
    ///
    /// The HSTX command expander is pumped with the same cycle budget; it does
    /// no work while `CSR.EN` is clear, so firmware that never touches HSTX is
    /// unaffected. DMA transfers are deliberately not pumped here: the host
    /// decides when to call [`Dma::run`] with its own bus handle and cycle
    /// budget. HSTX 命令扩展器按同一周期预算泵送；`CSR.EN` 清零时不做任何
    /// 事，因此不接触 HSTX 的固件不受影响。这里有意不泵送 DMA 传输：由宿主用
    /// 自己的总线句柄与周期预算决定何时调用 [`Dma::run`]。
    pub fn advance(&self, cycles: u64) {
        self.cycles.push(cycles);
        // HSTX is pumped by the host at the DMA batch cadence (see
        // `Machine::pump_dma_batch`), so it is not advanced here.
        // HSTX 由宿主按 DMA 批次节拍推进（见 `Machine::pump_dma_batch`），此处不推进。
        self.timer.refresh();
        self.timer1.refresh();
    }

    /// Currently asserted interrupt lines, refreshed for the new time
    /// 当前断言的中断线，已按新时间刷新
    #[must_use]
    pub fn irq_mask(&self) -> u64 {
        self.timer.refresh();
        self.timer1.refresh();
        self.irq.mask()
    }

    /// Asserted lines as seen by core 1 / core1 看到的断言线
    ///
    /// Every shared line is broadcast, but `SIO_IRQ_FIFO` and `SIO_IRQ_BELL`
    /// are per-core endpoints: core 0's levels are replaced by core 1's.
    /// 共享中断线全部广播，但 `SIO_IRQ_FIFO` 与 `SIO_IRQ_BELL` 是按核端点：
    /// core0 的电平被 core1 的电平替换。
    #[must_use]
    pub fn core1_irq_mask(&self) -> u64 {
        self.timer.refresh();
        self.timer1.refresh();
        let sio_bits = (1u64 << map::SIO_IRQ_FIFO) | (1u64 << map::SIO_IRQ_BELL);
        (self.irq.mask() & !sio_bits) | self.sio.core1_irq_mask()
    }

    /// Drain PPB system-space writes for replay into the core
    /// 取走待回放到核的 PPB 系统空间写入
    #[must_use]
    pub fn take_system_writes(&self) -> Vec<(u32, u32)> {
        std::mem::take(&mut *self.system_writes.lock())
    }

    /// Shared SIO handle, used by the core-1 doorbell hook
    /// 共享的 SIO 句柄，供 core 1 门铃挂钩使用
    #[must_use]
    pub fn sio(&self) -> &Sio {
        &self.sio
    }

    /// Shared pad electrical state (pulls and external levels)
    /// 共享的 pad 电气状态（上下拉与外部电平）
    #[must_use]
    pub fn pads(&self) -> &GpioPads {
        &self.pads
    }

    /// Shared TIMER0 handle / 共享的 TIMER0 句柄
    ///
    /// The handle shares state with the bus-mounted device, so guest writes are
    /// visible here and [`Timer0::refresh`] republishes the alarm lines.
    /// 该句柄与总线设备共享状态，客户机写入在此可见，[`Timer0::refresh`] 会重新
    /// 发布 alarm 中断线。
    #[must_use]
    pub fn timer(&self) -> &Timer0 {
        &self.timer
    }

    /// Shared TIMER1 handle / 共享的 TIMER1 句柄
    ///
    /// TIMER1 shares the microsecond timebase with [`Self::timer`] but keeps its
    /// own alarms, interrupt registers and `TIMER1_IRQ_0..3` lines.
    /// TIMER1 与 [`Self::timer`] 共享微秒时基，但拥有自己的 alarm、中断寄存器与
    /// `TIMER1_IRQ_0..3` 中断线。
    #[must_use]
    pub fn timer1(&self) -> &Timer1 {
        &self.timer1
    }

    /// Shared DMA controller handle / 共享的 DMA 控制器句柄
    #[must_use]
    pub fn dma(&self) -> &Dma {
        &self.dma
    }

    /// Mutable DMA handle used by the host to execute transfers
    /// 宿主执行传输所用的可变 DMA 句柄
    pub fn dma_mut(&mut self) -> &mut Dma {
        &mut self.dma
    }

    /// Shared HSTX handle / 共享的 HSTX 句柄
    ///
    /// The handle shares state with the APB control window and the AHB FIFO
    /// window, so writes through the bus are visible here immediately.
    /// 该句柄与 APB 控制窗口及 AHB FIFO 窗口共享状态，总线写入会立即在这里可见。
    #[must_use]
    pub fn hstx(&self) -> &Hstx {
        &self.hstx
    }

    /// Mutable HSTX handle used to drain output words
    /// 用于取走输出字的可变 HSTX 句柄
    pub fn hstx_mut(&mut self) -> &mut Hstx {
        &mut self.hstx
    }

    /// Shared PIO block handle (`index` `0..=2`) / 共享 PIO 块句柄（`index` `0..=2`）
    ///
    /// The handle shares one register file with the bus adapter, so guest MMIO
    /// writes are visible here and [`SharedPio::tick`] advances the state
    /// machines the guest programmed. Indices above 2 return `None`.
    /// 该句柄与总线适配器共享同一个寄存器文件，因此客户机 MMIO 写入在此可见，
    /// [`SharedPio::tick`] 推进的是客户机编程的状态机。序号大于 2 返回 `None`。
    #[must_use]
    pub fn pio(&self, index: usize) -> Option<&SharedPio> {
        self.pio.get(index)
    }

    /// Run `f` against a PIO block's register file / 在 PIO 块寄存器文件上执行 `f`
    ///
    /// The block's interrupt lines are refreshed after `f` returns. Indices
    /// above 2 return `None` without running the closure.
    /// `f` 返回后会刷新本块的中断线。序号大于 2 时直接返回 `None`，不执行闭包。
    pub fn with_pio<R>(&self, index: usize, f: impl FnOnce(&mut Pio) -> R) -> Option<R> {
        self.pio.get(index).map(|block| block.with_mut(f))
    }

    /// Install an I2S capture tap on `(block_index, sm)` / 在 `(block_index, sm)` 上安装 I2S 捕获 tap
    ///
    /// Every word accepted into state machine `sm`'s TX FIFO is appended to
    /// `tap`, no matter whether the CPU or a DMA channel wrote it, because both
    /// paths reach [`Pio::push_tx`]. [`BootPeripherals::mount`] deliberately
    /// installs no tap: capturing is a host/example decision, so firmware that
    /// ignores audio pays one `Option` check on the FIFO push path.
    /// Returns `false` and leaves the tap untouched when `block_index` is not a
    /// valid PIO block.
    /// 状态机 `sm` 的 TX FIFO 接受的每个字都会追加到 `tap`，无论写入来自 CPU 还是
    /// DMA 通道，因为两条路径都会到达 [`Pio::push_tx`]。
    /// [`BootPeripherals::mount`] 有意不安装 tap：是否捕获由宿主/示例决定，因此
    /// 不关心音频的固件在 FIFO 压入路径上只承担一次 `Option` 判断。
    /// `block_index` 不是合法 PIO 块时返回 `false` 且不改变 tap。
    pub fn set_audio_tap(&mut self, block_index: usize, sm: usize, tap: AudioTap) -> bool {
        let Some(block) = self.pio.get(block_index) else {
            return false;
        };
        let installed = tap.clone();
        block.with_mut(|pio| pio.set_audio_tap(installed, sm));
        self.audio_tap = Some(tap);
        true
    }

    /// Audio tap installed by the most recent [`Self::set_audio_tap`] call
    /// 最近一次 [`Self::set_audio_tap`] 安装的音频 tap
    ///
    /// The returned handle shares one [`crate::AudioSink`] with the PIO block, so
    /// a second consumer can drain samples concurrently. Returns `None` before
    /// any tap was installed.
    /// 返回的句柄与 PIO 块共享同一个 [`crate::AudioSink`]，因此第二个消费者可以
    /// 并发取走样本。尚未安装 tap 时返回 `None`。
    #[must_use]
    pub fn audio_tap(&self) -> Option<AudioTap> {
        self.audio_tap.clone()
    }

    /// Refresh all six PIO interrupt lines from the raw `IRQ` flags
    /// 按原始 `IRQ` 标志刷新全部六条 PIO 中断线
    ///
    /// [`SharedPio::tick`], [`BootPeripherals::with_pio`] and the bus adapters
    /// already publish flag changes; this entry point covers hosts that edit a
    /// block through another route and want a deterministic resynchronisation.
    /// [`SharedPio::tick`]、[`BootPeripherals::with_pio`] 与总线适配器已会发布
    /// 标志变化；本入口供通过其它路径修改块的宿主做确定性重同步。
    pub fn pump_pio_irqs(&self) {
        for block in &self.pio {
            block.sync_irqs();
        }
    }

    /// Publish every PIO FIFO level onto its DMA DREQ selector
    /// 把每个 PIO FIFO 的状态发布到对应的 DMA DREQ 选择器
    ///
    /// A PIO DREQ is a level sampled by the DMA controller: the TX request of a
    /// state machine is asserted while its TX FIFO is **not full** (the channel
    /// may write), and its RX request while the RX FIFO is **not empty** (the
    /// channel may read). Joining turns one direction into a single 8-deep FIFO:
    /// the queries read the live occupancy, so a joined FIFO asserts its DREQ up
    /// to 8 words and the joined-away direction — which reads as both full and
    /// empty — stays deasserted. No FIFO state is copied; every call re-reads
    /// the shared register file. Only selectors `0..=23` are written
    /// ([`crate::dreq`]); other DREQ inputs keep their current level.
    /// PIO DREQ 是由 DMA 控制器采样的电平：状态机的 TX 请求在其 TX FIFO
    /// **未满**时断言（通道可写入），RX 请求在其 RX FIFO **非空**时断言（通道可
    /// 读出）。合并会把一个方向变成单个 8 深度 FIFO：查询直接读取实时占用，因此
    /// 合并后的 FIFO 最多 8 字断言其 DREQ，被合并掉的方向同时读为满和空、其 DREQ
    /// 保持去断言。不复制任何 FIFO 状态，每次调用都重新读取共享寄存器文件。仅写
    /// `0..=23` 的选择器（[`crate::dreq`]），其它 DREQ 输入保持当前电平。
    pub fn sync_pio_dreqs(&self) {
        for (index, block) in self.pio.iter().enumerate() {
            let (tx, rx) = block.with(|pio| {
                let mut tx = 0u32;
                let mut rx = 0u32;
                for sm in 0..PIO_SM_COUNT {
                    // TX 请求 = 未满，RX 请求 = 非空；join 时同一存储按 8 深度判定。
                    // TX request = not full, RX request = not empty; a joined FIFO
                    // is judged at its 8-word depth by the same queries.
                    if pio.tx_fifo_full(sm) == Some(false) {
                        tx |= 1 << sm;
                    }
                    if pio.rx_fifo_empty(sm) == Some(false) {
                        rx |= 1 << sm;
                    }
                }
                (tx, rx)
            });
            let Ok(block_index) = u8::try_from(index) else {
                continue;
            };
            let Some(tx_base) = dreq::pio_tx_dreq_base(block_index) else {
                continue;
            };
            self.dma.set_dreq_bank(tx_base, tx);
            self.dma.set_dreq_bank(tx_base + dreq::PIO_TX_PER_BLOCK, rx);
        }
    }

    /// Run one DMA pump, refreshing PIO DREQ levels before every unit batch
    /// 执行一次 DMA 泵送，在每个单元批次前刷新 PIO DREQ 电平
    ///
    /// The PIO and HSTX levels are republished first, because the FIFO state may
    /// have changed since the previous pump. Two pacing rules keep a batch from
    /// overrunning a FIFO:
    ///
    /// - Channels paced by a PIO selector (`TREQ_SEL` `0..=23`) that are busy
    ///   advance one unit at a time, since a PIO FIFO can fill inside the batch.
    /// - When an enabled channel selects `DREQ_HSTX`, the batch budget is capped
    ///   at the HSTX FIFO's free space, so at most that many words can enter it
    ///   regardless of how the round-robin cursor interleaves other channels.
    ///   This keeps the single-call fast path (no per-unit mutex churn) while
    ///   making an overrun impossible — the same cap would be too coarse for PIO
    ///   because several 4-deep FIFOs can be targeted by one batch.
    ///
    /// Returns the units attempted.
    /// 先重新发布 PIO 与 HSTX 电平，因为上次泵送后 FIFO 状态可能已改变。两条节拍
    /// 规则防止批次冲垮 FIFO：
    ///
    /// - 忙碌且由 PIO 选择器（`TREQ_SEL` `0..=23`）节拍的通道逐单元推进，因为
    ///   PIO FIFO 可能在批次内被填满。
    /// - 当有已使能通道选择 `DREQ_HSTX` 时，批次预算被限制为 HSTX FIFO 的空闲
    ///   空间，因此无论轮转游标如何穿插其它通道，最多只有这么多字能进入 FIFO。
    ///   这样既保留单次调用的快路径（不逐单元加锁），又让溢出不可能发生；同样的
    ///   上限对 PIO 太粗糙，因为一批可能写入多个 4 深度 FIFO。
    ///
    /// 返回尝试的单元数。
    pub fn pump_dma(&mut self, bus: &mut dyn AddressBus, max_units: u64) -> u64 {
        // 先推进节拍定时器：复位值 X == 0 时请求永不断言，因此未使用定时器的
        // 固件行为不变。Advance the pacing timers first: with the reset value
        // X == 0 no request is asserted, so firmware that never programs a
        // timer is unaffected.
        self.dma.advance(DMA_PUMP_CYCLES);
        let mut total = 0u64;
        while total < max_units {
            self.sync_pio_dreqs();
            self.sync_hstx_dreq();
            let pio_paced = self
                .dma
                .busy_selects_dreq(dreq::PIO_DREQ_FIRST, dreq::PIO_DREQ_LAST);
            let hstx_paced = self
                .dma
                .enabled_selects_dreq(dreq::HSTX_DREQ, dreq::HSTX_DREQ);
            let mut budget = if pio_paced { 1 } else { max_units - total };
            if hstx_paced {
                budget = budget.min(u64::from(self.hstx.fifo_room()));
            }
            if budget == 0 {
                // The HSTX FIFO is full: nothing can move until the expander
                // drains a word. / HSTX FIFO 已满：扩展器腾出空间前无法推进。
                break;
            }
            let moved = self.dma.run(bus, budget);
            if moved == 0 {
                break;
            }
            total += moved;
        }
        total
    }

    /// Publish `DREQ_HSTX`: asserted while the HSTX FIFO has room
    /// 发布 `DREQ_HSTX`：HSTX FIFO 有空间时断言
    pub fn sync_hstx_dreq(&self) {
        self.dma
            .set_dreq(dreq::HSTX_DREQ, self.hstx.fifo_not_full());
    }

    /// Elapsed microseconds since boot / 自启动以来的微秒数
    #[must_use]
    pub fn time_us(&self) -> u64 {
        self.cycles.time_us()
    }

    /// Drain buffered UART0 transmit bytes / 取出 UART0 已缓冲的发送字节
    #[must_use]
    pub fn take_uart_output(&self) -> Vec<u8> {
        self.uart0.lock().take()
    }

    /// Number of bytes waiting in the UART0 sink / UART0 缓冲中等待的字节数
    #[must_use]
    pub fn uart_pending(&self) -> usize {
        self.uart0.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use rp2350_core::bus::AddressBus;
    use rp2350_core::memory::SimpleBus;

    use super::BootPeripherals;

    #[test]
    fn core1_release_pushes_a_zero_to_the_mailbox() {
        // `multicore_reset_core1` clears `PSM_FRCE_OFF.PROC1` and then waits
        // for `multicore_fifo_pop_blocking()` to return the core-1 boot ROM's
        // acknowledgement. The resident peer must provide that zero.
        // `multicore_reset_core1` 清除 `PSM_FRCE_OFF.PROC1` 后等待
        // `multicore_fifo_pop_blocking()` 取得 core1 boot ROM 的确认；常驻
        // 对端必须提供该 `0`。
        let mut bus = SimpleBus::new();
        let peripherals = BootPeripherals::mount(&mut bus).expect("mount");
        let mut cycles = 0u64;
        bus.write_u32(0x4001_A004, 0x0100_0000, &mut cycles)
            .expect("set alias");
        bus.write_u32(0x4001_B004, 0x0100_0000, &mut cycles)
            .expect("clr alias");
        peripherals.service_core1_peer();
        assert_eq!(
            bus.read_u32(0xD000_0058, &mut cycles).expect("fifo rd"),
            0,
            "core-1 ack"
        );
    }

    #[test]
    fn core1_peer_echoes_fifo_frames() {
        // The SDK launch/lockout handshakes push a word and expect the peer to
        // echo it. The resident peer models that boot-ROM behaviour.
        // SDK 的启动/锁定握手会压入一个字并期望对端回显；常驻对端模拟该
        // boot ROM 行为。
        let mut bus = SimpleBus::new();
        let peripherals = BootPeripherals::mount(&mut bus).expect("mount");
        let mut cycles = 0u64;
        bus.write_u32(0xD000_0054, 0x1234_5678, &mut cycles)
            .expect("FIFO_WR");
        peripherals.service_core1_peer();
        assert_eq!(
            bus.read_u32(0xD000_0058, &mut cycles).expect("fifo rd"),
            0x1234_5678
        );
    }

    /// Push one launch command and verify its echo / 写入一条启动命令并校验回显
    fn push_command(bus: &mut SimpleBus, peripherals: &BootPeripherals, value: u32) {
        let mut cycles = 0u64;
        bus.write_u32(0xD000_0054, value, &mut cycles).expect("wr");
        peripherals.service_core1_peer();
        assert_eq!(
            bus.read_u32(0xD000_0058, &mut cycles).expect("rd"),
            value,
            "the boot-ROM peer must echo every launch word"
        );
    }

    #[test]
    fn the_launch_stream_hands_over_the_vectors_and_retires_the_stub() {
        // `multicore_launch_core1_raw` 写 {0,0,1,vtor,sp,entry}、逐字等待回显，
        // 之后 FIFO 属于真实 core1，回显桩必须停止。
        // `multicore_launch_core1_raw` writes {0,0,1,vtor,sp,entry} and waits for
        // every echo; afterwards the FIFO belongs to the real core 1 and the echo
        // stub must stop.
        let mut bus = SimpleBus::new();
        let peripherals = BootPeripherals::mount(&mut bus).expect("mount");
        assert!(!peripherals.core1_launch_pending());

        for word in [0, 0, 1, 0x2000_0000, 0x2008_1000, 0x1000_0C35] {
            push_command(&mut bus, &peripherals, word);
        }

        let launch = peripherals.take_core1_launch().expect("launch");
        assert_eq!(launch.vtor, 0x2000_0000);
        assert_eq!(launch.sp, 0x2008_1000);
        assert_eq!(launch.entry, 0x1000_0C35);
        assert_eq!(peripherals.core1_launch_count(), 1);
        assert!(peripherals.core1_launched());
        assert!(!peripherals.core1_launch_pending());
        assert!(peripherals.take_core1_launch().is_none());

        // 启动后的消息留在 core1 收件箱中，桩不再回显。
        // Post-launch frames stay in core 1's inbox and are not echoed.
        let mut cycles = 0u64;
        bus.write_u32(0xD000_0054, 0xBEEF, &mut cycles).expect("wr");
        peripherals.service_core1_peer();
        assert_eq!(
            bus.read_u32(0xD000_0050, &mut cycles).expect("fifo_st") & 1,
            0,
            "the retired stub does not echo / 退役桩不回显"
        );
        peripherals
            .sio()
            .select_core(rp2350_core::types::CpuId::Core1);
        assert_eq!(
            bus.read_u32(0xD000_0058, &mut cycles).expect("fifo rd"),
            0xBEEF
        );
        peripherals
            .sio()
            .select_core(rp2350_core::types::CpuId::Core0);
    }

    #[test]
    fn a_psm_hard_reset_rearms_the_echo_stub() {
        // `multicore_reset_core1` 置位后立即清除 PROC1：reset 请求被锁存并可被
        // 驱动器消费，清除后的 0 确认仍由回显桩提供。
        // `multicore_reset_core1` sets then clears PROC1: the reset request is
        // latched for the driver, and the post-reset `0` ack still comes from
        // the boot-ROM peer.
        let mut bus = SimpleBus::new();
        let peripherals = BootPeripherals::mount(&mut bus).expect("mount");
        let mut cycles = 0u64;
        bus.write_u32(0x4001_A004, 0x0100_0000, &mut cycles)
            .expect("set");
        assert!(peripherals.take_core1_reset());
        assert!(!peripherals.take_core1_reset(), "one-shot latch");
        peripherals.reset_core1_boot();
        bus.write_u32(0x4001_B004, 0x0100_0000, &mut cycles)
            .expect("clr");
        peripherals.service_core1_peer();
        assert_eq!(
            bus.read_u32(0xD000_0058, &mut cycles).expect("ack"),
            0,
            "boot ROM ack / boot ROM 确认"
        );
        assert!(!peripherals.core1_launched());
    }

    #[test]
    fn core1_irq_mask_swaps_only_the_sio_endpoints() {
        // SIO 的两条线按核独立：core0 的收件箱水位不得投递给 core1。
        // The two SIO lines are per core: core 0's inbox level must not be
        // delivered to core 1.
        let mut bus = SimpleBus::new();
        let peripherals = BootPeripherals::mount(&mut bus).expect("mount");
        let fifo = 1u64 << crate::map::SIO_IRQ_FIFO;
        let bell = 1u64 << crate::map::SIO_IRQ_BELL;

        peripherals.sio().post_to_core(0x1234);
        assert_eq!(peripherals.irq_mask() & fifo, fifo, "core0 endpoint");
        assert_eq!(peripherals.core1_irq_mask() & (fifo | bell), 0);

        let mut cycles = 0u64;
        // core0 写一帧 → 只有 core1 的收件箱被填充。
        // Core 0 writes a frame → only core 1's inbox is filled.
        bus.write_u32(0xD000_0054, 7, &mut cycles).expect("wr");
        assert_eq!(peripherals.core1_irq_mask() & fifo, fifo);
        assert_eq!(peripherals.irq_mask() & fifo, fifo, "echo is not consumed");
    }

    #[test]
    fn powman_set_alias_updates_the_timer_register() {
        // 0x4010_2088 是 TIMER（0x88）的 SET 别名，写口令后应能直接读回数据位。
        // 0x4010_2088 is the SET alias of TIMER (0x88); after the password write
        // the data bits must read back directly.
        let mut bus = SimpleBus::new();
        let _peripherals = BootPeripherals::mount(&mut bus).expect("mount");
        let mut cycles = 0u64;
        bus.write_u32(0x4010_2088, 0x5AFE_0002, &mut cycles)
            .expect("alias write");
        assert_eq!(bus.read_u32(0x4010_0088, &mut cycles).expect("read"), 2);
    }

    #[test]
    fn previously_unmodelled_boot_blocks_are_mounted() {
        // 这些地址在 CircuitPython 启动时曾落入 APB 路由器的“未实现”兜底路径。
        // These addresses used to fall through the APB router's unimplemented path
        // during CircuitPython boot.
        let mut bus = SimpleBus::new();
        let _peripherals = BootPeripherals::mount(&mut bus).expect("mount");
        let mut cycles = 0u64;

        assert_eq!(
            bus.read_u32(0x4004_0004, &mut cycles).expect("qspi pad"),
            0x156
        );
        assert_eq!(
            bus.read_u32(0x400E_801C, &mut cycles).expect("rosc status") & (1 << 31),
            1 << 31
        );
        assert_eq!(
            bus.read_u32(0x400F_01B8, &mut cycles).expect("trng busy"),
            0
        );
        assert_eq!(
            bus.read_u32(0x400F_8000, &mut cycles).expect("sha csr"),
            0x1206
        );
        assert_eq!(
            bus.read_u32(0x400A_0000, &mut cycles).expect("adc cs") & 0x100,
            0x100
        );
        assert_eq!(
            bus.read_u32(0x400A_8004, &mut cycles).expect("pwm div"),
            0x10
        );

        // PWM1_BASE 是 PWM0 的 XOR 别名窗口，读回底层 CH0_CSR。
        // PWM1_BASE is PWM0's XOR alias window; it reads back the underlying CH0_CSR.
        bus.write_u32(0x400A_8000, 1, &mut cycles).expect("pwm en");
        assert_eq!(
            bus.read_u32(0x400A_9000, &mut cycles).expect("pwm1 alias"),
            1
        );
    }

    #[test]
    fn pio_blocks_are_mounted_at_their_ahb_bases() {
        // PIO0/PIO1/PIO2 位于 AHB 空间的 0x5020_0000、0x5030_0000、
        // 0x5040_0000；三个块各自独立挂载且可读写。
        // PIO0/PIO1/PIO2 sit at 0x5020_0000, 0x5030_0000 and 0x5040_0000 in
        // AHB space; each block is mounted independently and is readable.
        use crate::map;

        let mut bus = SimpleBus::new();
        let _peripherals = BootPeripherals::mount(&mut bus).expect("mount");
        let mut cycles = 0u64;
        for (index, base) in [
            (0u8, map::PIO0_BASE),
            (1, map::PIO1_BASE),
            (2, map::PIO2_BASE),
        ] {
            assert_eq!(
                bus.read_u32(base + 0x044, &mut cycles)
                    .expect("dbg_cfginfo"),
                0x1020_0404,
                "PIO{index} DBG_CFGINFO"
            );
            for word in 1..=4u32 {
                bus.write_u32(base + 0x010, word, &mut cycles)
                    .expect("txf0 push");
            }
            assert_eq!(
                bus.read_u32(base + 0x004, &mut cycles).expect("fstat") & (1 << 16),
                1 << 16,
                "PIO{index} SM0 TXFULL"
            );
        }
    }

    #[test]
    fn timer1_is_mounted_and_independent_of_timer0() {
        // TIMER1 位于 0x400B_8000：寄存器布局与 TIMER0 相同、中断独立，且共享
        // 同一个 64 位微秒计数器。
        // TIMER1 sits at 0x400B_8000 with TIMER0's layout, independent interrupt
        // state and the same 64-bit microsecond counter.
        use crate::map;

        let mut bus = SimpleBus::new();
        let peripherals = BootPeripherals::mount(&mut bus).expect("mount");
        let mut cycles = 0u64;

        // 使能四条线并强制 alarm 3，INTS 读回该位、中断线为 TIMER1_IRQ_3。
        // Enable all four lines and force alarm 3: INTS reads the bit and the
        // line is TIMER1_IRQ_3.
        bus.write_u32(map::TIMER1_BASE + 0x40, 0xF, &mut cycles)
            .expect("timer1 inte");
        bus.write_u32(map::TIMER1_BASE + 0x44, 1 << 3, &mut cycles)
            .expect("timer1 intf");
        assert_eq!(
            bus.read_u32(map::TIMER1_BASE + 0x48, &mut cycles)
                .expect("timer1 ints"),
            1 << 3
        );
        assert_eq!(
            bus.read_u32(map::TIMER0_BASE + 0x48, &mut cycles)
                .expect("timer0 ints"),
            0
        );
        assert_ne!(
            peripherals.irq_mask() & (1 << map::TIMER1_IRQ_3),
            0,
            "TIMER1 alarm 3 line"
        );
        assert_eq!(peripherals.irq_mask() & 0xF, 0, "TIMER0 lines stay clear");

        // 两个块的 TIMERAWL 读回同一计数器。 / Both TIMERAWL registers agree.
        let now0 = bus
            .read_u32(map::TIMER0_BASE + 0x28, &mut cycles)
            .expect("timer0 timerawl");
        let now1 = bus
            .read_u32(map::TIMER1_BASE + 0x28, &mut cycles)
            .expect("timer1 timerawl");
        assert_eq!(now0, now1);

        // 清 TIMER0 的强制位不应影响 TIMER1。 / Clearing TIMER0 leaves TIMER1.
        bus.write_u32(map::TIMER0_BASE + 0x44, 0, &mut cycles)
            .expect("timer0 intf");
        assert_eq!(
            bus.read_u32(map::TIMER1_BASE + 0x48, &mut cycles)
                .expect("timer1 ints"),
            1 << 3
        );
    }

    #[test]
    fn dma_block_is_mounted_and_shared() {
        // DMA 位于 0x5000_0000；N_CHANNELS 读回 16，句柄与总线设备共享状态。
        // DMA sits at 0x5000_0000; N_CHANNELS reads 16 and the handle shares
        // state with the bus-mounted device.
        use crate::map;

        let mut bus = SimpleBus::new();
        let peripherals = BootPeripherals::mount(&mut bus).expect("mount");
        let mut cycles = 0u64;
        assert_eq!(
            bus.read_u32(map::DMA_BASE + 0x468, &mut cycles)
                .expect("n channels"),
            16
        );
        assert_eq!(peripherals.dma().channel_state(0).expect("ch0").chain_to, 0);
        assert_eq!(peripherals.dma().busy_mask(), 0);
    }
}
