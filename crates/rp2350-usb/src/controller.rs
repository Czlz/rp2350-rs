//! Shared controller facade and `Peripheral` integration
//! 共享控制器门面与 `Peripheral` 集成

use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_core::bus::{AddressBus, BusResult, MemoryRegion};
use rp2350_core::types::Address;
use rp2350_periph::Peripheral;

use crate::device::UsbCore;
use crate::endpoint::{InResponse, OutResponse};
use crate::mmio::{DpramMmio, RegsMmio, mount_usb};
use crate::regions::{DpramRegion, RegsRegion, UsbPeripheral};
use crate::setup::SetupPacket;
use crate::{DPRAM_BASE, USBCTRL_IRQ};

/// Shared handle to the USB device controller.
/// USB 设备控制器的共享句柄。
///
/// The controller owns the single [`UsbCore`] and hands out bus regions and
/// host-side transaction helpers. Both the guest bus and the host bridge see
/// the same state through the `Arc<Mutex<_>>`.
/// 控制器持有唯一的 [`UsbCore`]，并对外提供总线区域与主机侧事务辅助方法。
/// 客户机总线与主机桥通过同一个 `Arc<Mutex<_>>` 看到相同状态。
#[derive(Clone)]
pub struct Controller {
    core: Arc<Mutex<UsbCore>>,
}

impl Default for Controller {
    fn default() -> Self {
        Self::new()
    }
}

impl Controller {
    /// Create a controller in its reset state / 创建复位状态的控制器
    #[must_use]
    pub fn new() -> Self {
        Self {
            core: Arc::new(Mutex::new(UsbCore::new())),
        }
    }

    /// Shared core handle for regions or external drivers
    /// 供区域或外部驱动使用的共享核心句柄
    #[must_use]
    pub fn shared_core(&self) -> Arc<Mutex<UsbCore>> {
        Arc::clone(&self.core)
    }

    /// Run `f` with exclusive access to the core / 以独占方式访问核心执行 `f`
    pub fn with_core<R>(&self, f: impl FnOnce(&mut UsbCore) -> R) -> R {
        f(&mut self.core.lock())
    }

    /// DPSRAM-only bus region at `0x5010_0000` / `0x5010_0000` 的 DPSRAM 总线区域
    #[must_use]
    pub fn dpram_region(&self) -> DpramRegion {
        DpramRegion::new(self.shared_core())
    }

    /// Register window at `0x5011_0000` (16 KB alias window)
    /// `0x5011_0000` 的寄存器窗口（16 KB 别名窗口）
    #[must_use]
    pub fn regs_region(&self) -> RegsRegion {
        RegsRegion::new(self.shared_core())
    }

    /// Combined region covering DPSRAM and registers / 覆盖 DPSRAM 与寄存器的合并区域
    #[must_use]
    pub fn peripheral_region(&self) -> UsbPeripheral {
        UsbPeripheral::new(self.shared_core())
    }

    /// DPSRAM window as a mainline MMIO device
    /// 作为主线 MMIO 设备的 DPSRAM 窗口
    #[must_use]
    pub fn mmio_dpram(&self) -> DpramMmio {
        DpramMmio::new(self.shared_core())
    }

    /// Register window as a mainline MMIO device
    /// 作为主线 MMIO 设备的寄存器窗口
    #[must_use]
    pub fn mmio_regs(&self) -> RegsMmio {
        RegsMmio::new(self.shared_core())
    }

    /// Mount both USBCTRL windows through the mainline `mount_device` API
    /// 通过主线 `mount_device` API 挂载两个 USBCTRL 窗口
    ///
    /// # Errors
    /// Returns a bus error when a window is invalid or overlaps a region
    /// already registered on `bus`.
    /// 窗口非法或与 `bus` 上已注册区域重叠时返回总线错误。
    pub fn mount_mmio(&self, bus: &mut dyn AddressBus) -> BusResult<()> {
        mount_usb(bus, self.shared_core())
    }

    /// Whether the controller asserts `USBCTRL_IRQ` / 控制器是否拉高 `USBCTRL_IRQ`
    #[must_use]
    pub fn irq_asserted(&self) -> bool {
        self.core.lock().irq_asserted()
    }

    /// Reset the controller / 复位控制器
    pub fn reset(&self) {
        self.core.lock().reset();
    }

    /// Host: announce device attach / 主机：宣告设备附着
    pub fn host_attach(&self) {
        self.core.lock().host_attach();
    }

    /// Host: announce device detach / 主机：宣告设备拔出
    pub fn host_detach(&self) {
        self.core.lock().host_detach();
    }

    /// Host: issue a bus reset / 主机：发起总线复位
    pub fn host_bus_reset(&self) {
        self.core.lock().host_bus_reset();
    }

    /// Host: deliver a start-of-frame token / 主机：送达帧起始令牌
    pub fn host_sof(&self, frame: u16) {
        self.core.lock().host_sof(frame);
    }

    /// Host: deliver a SETUP packet / 主机：送达 SETUP 包
    #[must_use]
    pub fn host_setup(&self, packet: &SetupPacket) -> bool {
        self.core.lock().host_setup(packet)
    }

    /// Host: deliver an IN token / 主机：送达 IN 令牌
    #[must_use]
    pub fn host_in(&self, ep: usize) -> InResponse {
        self.core.lock().host_in(ep)
    }

    /// Host: deliver an OUT token / 主机：送达 OUT 令牌
    #[must_use]
    pub fn host_out(&self, ep: usize, data: &[u8]) -> OutResponse {
        self.core.lock().host_out(ep, data)
    }

    /// Whether firmware enabled the pull-up / 固件是否使能上拉
    #[must_use]
    pub fn device_visible(&self) -> bool {
        self.core.lock().device_visible()
    }
}

impl MemoryRegion for Controller {
    fn name(&self) -> &'static str {
        "USBCTRL"
    }

    fn base(&self) -> Address {
        DPRAM_BASE
    }

    fn size(&self) -> usize {
        self.peripheral_region().size()
    }

    fn read_u8(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u8> {
        self.peripheral_region().read_u8(addr, cycles)
    }

    fn write_u8(&mut self, addr: Address, value: u8, cycles: &mut u64) -> BusResult<()> {
        self.peripheral_region().write_u8(addr, value, cycles)
    }

    fn read_u16(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u16> {
        self.peripheral_region().read_u16(addr, cycles)
    }

    fn write_u16(&mut self, addr: Address, value: u16, cycles: &mut u64) -> BusResult<()> {
        self.peripheral_region().write_u16(addr, value, cycles)
    }

    fn read_u32(&mut self, addr: Address, cycles: &mut u64) -> BusResult<u32> {
        self.peripheral_region().read_u32(addr, cycles)
    }

    fn write_u32(&mut self, addr: Address, value: u32, cycles: &mut u64) -> BusResult<()> {
        self.peripheral_region().write_u32(addr, value, cycles)
    }
}

impl Peripheral for Controller {
    fn peripheral_name(&self) -> &'static str {
        "USBCTRL"
    }

    fn reset(&mut self) {
        self.core.lock().reset();
    }

    fn irq_number(&self) -> Option<u16> {
        Some(USBCTRL_IRQ)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp2350_core::bus::AddressBus;
    use rp2350_core::memory::SimpleBus;

    use crate::REGS_BASE;
    use crate::regs::{self, MAIN_CTRL, MAIN_CTRL_CONTROLLER_EN, SIE_CTRL_PULLUP_EN};

    const MAIN_CTRL_ADDR: Address = REGS_BASE + MAIN_CTRL as Address;

    #[test]
    fn controller_registers_on_a_bus() {
        let controller = Controller::new();
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(controller.peripheral_region()))
            .unwrap();
        let mut cycles = 0;
        bus.write_u32(MAIN_CTRL_ADDR, MAIN_CTRL_CONTROLLER_EN, &mut cycles)
            .unwrap();
        assert_eq!(
            bus.read_u32(MAIN_CTRL_ADDR, &mut cycles).unwrap(),
            MAIN_CTRL_CONTROLLER_EN
        );
        bus.write_u32(DPRAM_BASE + 0x180, 0x1234_5678, &mut cycles)
            .unwrap();
        assert_eq!(
            bus.read_u32(DPRAM_BASE + 0x180, &mut cycles).unwrap(),
            0x1234_5678
        );
    }

    #[test]
    fn controller_as_peripheral_reports_irq_and_resets() {
        let controller = Controller::new();
        controller.with_core(|core| {
            core.write_reg(regs::SIE_CTRL, regs::Alias::Rw, SIE_CTRL_PULLUP_EN);
        });
        assert_eq!(Peripheral::irq_number(&controller), Some(USBCTRL_IRQ));
        let mut controller = controller;
        Peripheral::reset(&mut controller);
        assert!(!controller.with_core(|core| core.pullup_enabled()));
    }

    #[test]
    fn separate_regions_share_one_core() {
        let controller = Controller::new();
        let mut dpram = controller.dpram_region();
        let mut regs = controller.regs_region();
        let mut cycles = 0;
        dpram
            .write_u32(DPRAM_BASE + 0x180, 0xAAAA_5555, &mut cycles)
            .unwrap();
        controller.with_core(|core| {
            core.write_reg(regs::MAIN_CTRL, regs::Alias::Rw, MAIN_CTRL_CONTROLLER_EN);
        });
        assert_eq!(regs.read_u8(MAIN_CTRL_ADDR, &mut cycles).unwrap(), 1);
        assert_eq!(controller.host_in(1), InResponse::NoDevice);
        assert_eq!(controller.host_out(1, &[]), OutResponse::NoDevice);
    }
}
