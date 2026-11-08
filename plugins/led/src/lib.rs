//! Reference LED plugin — a loadable device plugin driving a single LED
//! 参考 LED 插件 — 驱动单路 LED 的可加载设备插件
//!
//! The crate builds both an `rlib` (for host-side tests) and a `cdylib`
//! (the loadable plugin). It exports the `rp2350-plugin` ABI symbols so that
//! [`DeviceManager`](rp2350_virt_periph::DeviceManager) can load it at runtime.
//! 本 crate 同时构建 `rlib`（供主机侧测试）与 `cdylib`（可加载插件），并导出
//! `rp2350-plugin` ABI 符号，使 [`DeviceManager`](rp2350_virt_periph::DeviceManager)
//! 能在运行时加载它。

// 插件 ABI 通过 trait 对象指针跨越动态库边界，依赖同一编译器与 ABI 版本。
// The plugin ABI passes trait-object pointers across the dynamic library
// boundary, relying on the same compiler and ABI version.
#![allow(improper_ctypes_definitions)]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

use rp2350_plugin::abi::ABI_VERSION;
use rp2350_plugin::{PluginContext, PluginError, PluginMetadata, RpPlugin};
use rp2350_virt_periph::{DevicePlugin, DevicePort};

/// Stable plugin identifier / 稳定的插件标识符
const PLUGIN_ID: &str = "com.rp2350-rs.led";

/// A single-LED device driven by one GPIO input port
/// 由一路 GPIO 输入端口驱动的单 LED 设备
pub struct LedPlugin {
    /// Plugin metadata returned to the host / 返回给主机的插件元数据
    metadata: PluginMetadata,
    /// Exposed ports (one input named "LED") / 暴露的端口（一个名为 "LED" 的输入）
    ports: Vec<DevicePort>,
    /// Current LED level / 当前 LED 电平
    level: bool,
    /// Number of level transitions observed / 观察到的电平跳变次数
    transitions: u64,
}

impl LedPlugin {
    /// Create a new LED plugin in the off state
    /// 创建一个处于熄灭状态的新 LED 插件
    #[must_use]
    pub fn new() -> Self {
        Self {
            metadata: PluginMetadata::builder()
                .id(PLUGIN_ID)
                .name("LED")
                .version("0.1.0")
                .description("Reference single-LED virtual device / 参考单 LED 虚拟设备")
                .author("RP2350-rs Team")
                .abi_version(ABI_VERSION)
                .min_host_version("0.1.0")
                .has_ui(false)
                .tags(vec!["reference".to_string(), "gpio".to_string()])
                .build(),
            ports: vec![DevicePort::input(
                "LED",
                "GPIO-driven LED input / GPIO 驱动的 LED 输入",
            )],
            level: false,
            transitions: 0,
        }
    }
}

impl Default for LedPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl RpPlugin for LedPlugin {
    fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    fn init(&mut self, _ctx: &mut dyn PluginContext) -> Result<(), PluginError> {
        Ok(())
    }

    fn tick(&mut self, _ctx: &mut dyn PluginContext, _cycles: u64) {
        // LED 是纯电平驱动设备，无需随时间推进状态。
        // An LED is purely level-driven; no time-based state to advance.
    }

    fn shutdown(&mut self) {
        self.level = false;
        self.ports[0].level = false;
    }

    fn state_snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "port": "LED",
            "on": self.level,
            "transitions": self.transitions,
        })
    }
}

impl DevicePlugin for LedPlugin {
    fn ports(&self) -> &[DevicePort] {
        &self.ports
    }

    fn ports_mut(&mut self) -> &mut [DevicePort] {
        &mut self.ports
    }

    fn on_gpio_change(&mut self, port_index: usize, level: bool) {
        if port_index != 0 {
            return;
        }
        if self.level != level {
            self.transitions += 1;
        }
        self.level = level;
        self.ports[0].level = level;
    }
}

/// Return the ABI version this plugin was compiled against
/// 返回此插件编译时使用的 ABI 版本
#[unsafe(no_mangle)]
pub extern "C" fn rp2350_plugin_version() -> u32 {
    ABI_VERSION
}

/// Report that this library exports a device plugin
/// 报告此库导出的是设备插件
#[unsafe(no_mangle)]
pub extern "C" fn rp2350_plugin_is_device() -> u32 {
    1
}

/// Create a boxed LED device plugin for the host
/// 为主机创建 boxed LED 设备插件
#[unsafe(no_mangle)]
pub extern "C" fn rp2350_device_plugin_create() -> *mut dyn DevicePlugin {
    Box::into_raw(Box::new(LedPlugin::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp2350_core::gpio::{GpioPin, PinDirection};
    use rp2350_virt_periph::DeviceManager;

    #[test]
    fn metadata_reports_led_identity() {
        let plugin = LedPlugin::new();
        assert_eq!(plugin.metadata().id, PLUGIN_ID);
        assert_eq!(plugin.metadata().abi_version, ABI_VERSION);
        assert!(!plugin.metadata().has_ui);
    }

    #[test]
    fn exposes_one_input_port() {
        let plugin = LedPlugin::new();
        assert_eq!(plugin.ports().len(), 1);
        assert_eq!(plugin.ports()[0].name, "LED");
    }

    #[test]
    fn tracks_input_level_and_transitions() {
        let mut plugin = LedPlugin::new();
        plugin.on_gpio_change(0, true);
        assert!(plugin.state_snapshot()["on"].as_bool().unwrap());
        plugin.on_gpio_change(0, true);
        plugin.on_gpio_change(0, false);
        assert_eq!(plugin.state_snapshot()["transitions"], 2);
        assert!(!plugin.state_snapshot()["on"].as_bool().unwrap());
    }

    #[test]
    fn ignores_unknown_ports() {
        let mut plugin = LedPlugin::new();
        plugin.on_gpio_change(1, true);
        assert!(!plugin.state_snapshot()["on"].as_bool().unwrap());
        assert_eq!(plugin.state_snapshot()["transitions"], 0);
    }

    #[test]
    fn abi_exports_are_consistent() {
        assert_eq!(rp2350_plugin_version(), ABI_VERSION);
        assert_eq!(rp2350_plugin_is_device(), 1);

        let raw = rp2350_device_plugin_create();
        assert!(!raw.is_null());
        // SAFETY: `raw` came from this crate's factory and is reclaimed once.
        // SAFETY: `raw` 来自本 crate 的工厂，且仅回收一次。
        unsafe {
            drop(Box::from_raw(raw));
        }
    }

    #[test]
    fn device_manager_drives_led() {
        let mut manager = DeviceManager::new();
        manager
            .register("led0".to_string(), Box::new(LedPlugin::new()))
            .unwrap();
        manager.wire("led0", 0, 25).unwrap();

        let mut pins =
            std::array::from_fn::<GpioPin, 48, _>(|i| GpioPin::new(u8::try_from(i).unwrap()));
        pins[25].set_direction(PinDirection::Output);
        pins[25].set_output(true);
        manager.sync_gpio(&mut pins);

        let query = manager.mcp_query_all();
        assert_eq!(query["led0"]["on"], true);
        assert_eq!(query["led0"]["transitions"], 1);

        pins[25].set_output(false);
        manager.sync_gpio(&mut pins);
        assert_eq!(manager.mcp_query_all()["led0"]["transitions"], 2);
    }
}
