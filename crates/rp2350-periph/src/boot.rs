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

use parking_lot::Mutex;
use rp2350_core::bus::{AddressBus, BusResult};
use rp2350_core::mmio::{MMIO_BLOCK_SIZE, MmioRouter, MmioWindow, mount_device};

use super::bootram::BootRam;
use super::clocks::Clocks;
use super::cycle::CycleSink;
use super::io_bank0::IoBank0;
use super::io_qspi::IoQspi;
use super::irq::IrqAssert;
use super::map;
use super::pads_bank0::PadsBank0;
use super::pll::Pll;
use super::powman::Powman;
use super::ppb::{Ppb, SystemWriteLog, new_system_write_log};
use super::resets::Resets;
use super::sio::Sio;
use super::ticks::Ticks;
use super::timer::Timer0;
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
    irq: IrqAssert,
    system_writes: SystemWriteLog,
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

        let mut apb = MmioRouter::new("APB", MmioWindow::new(map::APB_BASE, map::APB_SIZE));
        apb.mount(Box::new(Clocks::new()))?;
        apb.mount(Box::new(Resets::new()))?;
        apb.mount(Box::new(IoBank0::new()))?;
        apb.mount(Box::new(IoQspi::new()))?;
        apb.mount(Box::new(PadsBank0::new()))?;
        apb.mount(Box::new(Xosc::new()))?;
        apb.mount(Box::new(Pll::sys()))?;
        apb.mount(Box::new(Pll::usb()))?;
        apb.mount(Box::new(Uart0::new(Arc::clone(&uart0))))?;
        apb.mount(Box::new(timer.clone()))?;
        apb.mount(Box::new(Watchdog::new()))?;
        apb.mount(Box::new(XipQmi::new()))?;
        apb.mount(Box::new(BootRam::new()))?;
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
        mount_device(bus, Box::new(sio.clone()))?;
        mount_device(bus, Box::new(Ppb::with_log(Arc::clone(&system_writes))))?;

        Ok(Self {
            uart0,
            cycles,
            timer,
            sio,
            irq,
            system_writes,
        })
    }

    /// Advance the shared timebase and refresh TIMER0 interrupt lines
    /// 推进共享时基并刷新 TIMER0 中断线
    pub fn advance(&self, cycles: u64) {
        self.cycles.push(cycles);
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
}
