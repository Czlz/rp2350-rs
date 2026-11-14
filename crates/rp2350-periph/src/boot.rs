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

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use rp2350_core::bus::{AddressBus, BusResult};
use rp2350_core::mmio::{MMIO_BLOCK_SIZE, MmioRouter, MmioWindow, mount_device};

use super::adc::Adc;
use super::bootram::BootRam;
use super::clocks::Clocks;
use super::cycle::CycleSink;
use super::dma::Dma;
use super::hstx::Hstx;
use super::io_bank0::IoBank0;
use super::io_qspi::IoQspi;
use super::irq::IrqAssert;
use super::map;
use super::pads_bank0::PadsBank0;
use super::pads_qspi::PadsQspi;
use super::pio::Pio;
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
use super::timer::Timer0;
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

/// Handles for the peripherals mounted by [`BootPeripherals::mount`]
/// [`BootPeripherals::mount`] 挂载的外设句柄
pub struct BootPeripherals {
    uart0: Arc<Mutex<UartSink>>,
    cycles: CycleSink,
    timer: Timer0,
    sio: Sio,
    dma: Dma,
    hstx: Hstx,
    irq: IrqAssert,
    system_writes: SystemWriteLog,
    core1_release: Arc<AtomicBool>,
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
        let sio = Sio::new(irq.clone());
        let dma = Dma::new(irq.clone());
        let hstx = Hstx::new(irq.clone());
        let core1_release = Arc::new(AtomicBool::new(false));

        let mut apb = MmioRouter::new("APB", MmioWindow::new(map::APB_BASE, map::APB_SIZE));
        apb.mount(Box::new(Clocks::new()))?;
        apb.mount(Box::new(Resets::new()))?;
        apb.mount(Box::new(Psm::with_core1_release(Arc::clone(
            &core1_release,
        ))))?;
        apb.mount(Box::new(IoBank0::new()))?;
        apb.mount(Box::new(IoQspi::new()))?;
        apb.mount(Box::new(PadsBank0::new()))?;
        apb.mount(Box::new(PadsQspi::new()))?;
        apb.mount(Box::new(Xosc::new()))?;
        apb.mount(Box::new(Pll::sys()))?;
        apb.mount(Box::new(Pll::usb()))?;
        apb.mount(Box::new(Uart0::new(Arc::clone(&uart0))))?;
        apb.mount(Box::new(Adc::new()))?;
        apb.mount(Box::new(Pwm::new()))?;
        apb.mount(Box::new(timer.clone()))?;
        // HSTX 控制寄存器位于 APB（0x400C_0000），FIFO 在 AHB 侧独立挂载。
        // The HSTX control registers live in APB (0x400C_0000); the FIFO is
        // mounted separately in AHB space.
        apb.mount(hstx.ctrl_device())?;
        apb.mount(Box::new(Watchdog::new()))?;
        apb.mount(Box::new(XipQmi::new()))?;
        apb.mount(Box::new(BootRam::new()))?;
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
        // RP2350 的三个 PIO 块位于 AHB 空间，按基地址顺序独立挂载；
        // 本阶段仅建模寄存器、FIFO 与指令存储。
        // The three RP2350 PIO blocks live in AHB space and are mounted
        // standalone in base-address order; this stage models registers,
        // FIFOs and instruction memory only.
        mount_device(bus, Box::new(Pio::new(0)))?;
        mount_device(bus, Box::new(Pio::new(1)))?;
        mount_device(bus, Box::new(Pio::new(2)))?;
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
            sio,
            dma,
            hstx,
            irq,
            system_writes,
            core1_release,
        })
    }

    /// Emulate the resident core-1 peer once per driver pump
    /// 每次驱动器泵送时模拟常驻 core1 对端
    ///
    /// When `PSM_FRCE_OFF.PROC1` is released, the core-1 boot ROM drains its
    /// mailbox and pushes `0` to core 0. For ordinary FIFO writes the resident
    /// peer echoes the frame back, which is what the SDK launch and lockout
    /// handshakes expect. Core-1 code execution itself is not modelled.
    /// 当 `PSM_FRCE_OFF.PROC1` 释放时，core1 boot ROM 清空信箱并向 core0 压入
    /// `0`。普通 FIFO 写入由常驻对端回显，这符合 SDK 启动与锁定握手的预期。
    /// core1 的指令执行本身未建模。
    pub fn service_core1_peer(&self) {
        if self.core1_release.swap(false, Ordering::AcqRel) {
            let _ = self.sio.take_from_core();
            self.sio.post_to_core(0);
        }
        for frame in self.sio.take_from_core() {
            // 排查核间启动/锁定握手时打印收到的字。
            // Log received words when debugging inter-core launch/lockout.
            if std::env::var_os("RP2350_TRACE_CORE1").is_some() {
                eprintln!("[core1] echo {frame:#010X}");
            }
            self.sio.post_to_core(frame);
        }
    }

    /// Advance the shared timebase and refresh TIMER0 interrupt lines
    /// 推进共享时基并刷新 TIMER0 中断线
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
        self.hstx.advance(cycles);
        self.timer.refresh();
    }

    /// Currently asserted interrupt lines, refreshed for the new time
    /// 当前断言的中断线，已按新时间刷新
    #[must_use]
    pub fn irq_mask(&self) -> u64 {
        self.timer.refresh();
        self.irq.mask()
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
