//! Unit and integration tests for the UART terminal plugin
//! UART 终端插件的单元与集成测试

use rp2350_core::gpio::{GpioPin, PinDirection};
use rp2350_plugin::{PluginContext, RpPlugin};
use rp2350_virt_periph::device::PortDirection;
use rp2350_virt_periph::error::ConfigError;
use rp2350_virt_periph::{DeviceManager, DevicePlugin};
use serde_json::{Value, json};

use super::*;

/// Minimal [`PluginContext`] with a manually advanced cycle counter
/// 带手动推进周期计数器的最小 [`PluginContext`]
#[derive(Default)]
struct TestCtx {
    /// Simulated cycle counter / 仿真周期计数器
    cycles: u64,
}

impl PluginContext for TestCtx {
    fn log_info(&self, _message: &str) {}

    fn log_warn(&self, _message: &str) {}

    fn log_error(&self, _message: &str) {}

    fn current_cycles(&self) -> u64 {
        self.cycles
    }

    fn schedule_callback(&mut self, _cycles: u64, _tag: &str) {}

    fn get_service_erased(&self) -> Option<&dyn std::any::Any> {
        None
    }
}

/// Drives the plugin one cycle at a time, like a host stepping the clock
/// 像主机那样逐周期推进插件
struct Harness {
    /// Plugin under test / 被测插件
    plugin: UartTerminalPlugin,
    /// Context carrying the cycle counter / 携带周期计数器的上下文
    ctx: TestCtx,
}

impl Harness {
    /// Build a harness with the default 115200 8N1 configuration
    /// 使用默认 115200 8N1 配置构建测试装置
    fn new() -> Self {
        Self {
            plugin: UartTerminalPlugin::new(),
            ctx: TestCtx::default(),
        }
    }

    /// Cycles per bit of the current configuration / 当前配置的每比特周期数
    fn bit(&self) -> u64 {
        self.plugin.bit_cycles()
    }

    /// Advance the clock by `cycles`, ticking the plugin once per cycle
    /// 推进 `cycles` 个周期，每周期 tick 一次插件
    fn advance(&mut self, cycles: u64) {
        for _ in 0..cycles {
            self.ctx.cycles += 1;
            self.plugin.tick(&mut self.ctx, 1);
        }
    }

    /// Hold the `TX` level for `cycles` / 保持 `TX` 电平 `cycles` 个周期
    fn drive(&mut self, level: bool, cycles: u64) {
        self.plugin.on_gpio_change(0, level);
        self.advance(cycles);
    }

    /// Transmit one 8N1 frame (idle, start, LSB-first data, stop)
    /// 发送一个 8N1 帧（空闲、起始位、LSB 优先数据、停止位）
    fn frame(&mut self, byte: u8) {
        let bit = self.bit();
        self.drive(true, bit);
        self.drive(false, bit);
        for index in 0..self.plugin.data_bits {
            self.drive((byte >> index) & 1 == 1, bit);
        }
        self.drive(true, 2 * bit);
    }
}

#[test]
fn metadata_reports_uart_identity() {
    let plugin = UartTerminalPlugin::new();
    assert_eq!(plugin.metadata().id, PLUGIN_ID);
    assert_eq!(plugin.metadata().name, "UART_TERMINAL");
    assert_eq!(plugin.metadata().abi_version, ABI_VERSION);
    assert!(!plugin.metadata().has_ui);
    assert_eq!(
        plugin.metadata().tags,
        vec!["uart".to_string(), "terminal".to_string()]
    );
}

#[test]
fn exposes_tx_input_and_rx_output_ports() {
    let plugin = UartTerminalPlugin::new();
    assert_eq!(plugin.ports().len(), 2);
    assert_eq!(plugin.ports()[0].name, "TX");
    assert_eq!(plugin.ports()[0].direction, PortDirection::Input);
    assert_eq!(plugin.ports()[1].name, "RX");
    assert_eq!(plugin.ports()[1].direction, PortDirection::Output);
    // 两条线路初始均为空闲高电平。
    // Both lines start at the idle-high level.
    assert!(plugin.ports()[0].level);
    assert!(plugin.ports()[1].level);
}

#[test]
fn decodes_bytes_lsb_first_at_115200() {
    let mut harness = Harness::new();
    assert_eq!(harness.plugin.baud(), DEFAULT_BAUD);
    assert_eq!(harness.plugin.bit_cycles(), DEFAULT_CLOCK_HZ / 115_200);

    harness.frame(0x41);
    harness.frame(0xA5);

    let snapshot = harness.plugin.state_snapshot();
    assert_eq!(snapshot["rx_bytes"], 2);
    assert_eq!(snapshot["last_byte"], 0xA5);
    assert_eq!(harness.plugin.rx_buffer(), &[0x41, 0xA5]);
    assert_eq!(snapshot["recent_hex"], "41 a5");
    // 0xA5 不可打印，以 `.` 呈现。
    // 0xA5 is not printable and shows up as `.`.
    assert_eq!(snapshot["recent_ascii"], "A.");
}

#[test]
fn idle_line_produces_no_bytes() {
    let mut harness = Harness::new();
    let bit = harness.bit();
    harness.drive(true, 50 * bit);

    let snapshot = harness.plugin.state_snapshot();
    assert_eq!(snapshot["rx_bytes"], 0);
    assert_eq!(snapshot["last_byte"], Value::Null);
    assert_eq!(snapshot["buffer_len"], 0);
    assert_eq!(snapshot["framing_errors"], 0);
    assert_eq!(harness.plugin.rx_buffer(), &[] as &[u8]);
}

#[test]
fn wrong_stop_bit_discards_frame() {
    let mut harness = Harness::new();
    let bit = harness.bit();
    harness.drive(true, bit);
    harness.drive(false, bit);
    for index in 0..8 {
        harness.drive((0x41u8 >> index) & 1 == 1, bit);
    }
    // 停止位应为空闲高，此处故意保持低电平。
    // The stop bit must be idle-high; hold it low on purpose.
    harness.drive(false, bit);
    harness.drive(true, 2 * bit);

    let snapshot = harness.plugin.state_snapshot();
    assert_eq!(snapshot["rx_bytes"], 0);
    assert_eq!(snapshot["buffer_len"], 0);
    assert_eq!(snapshot["framing_errors"], 1);
    assert_eq!(snapshot["last_byte"], Value::Null);
}

#[test]
fn apply_config_changes_sampling_period() {
    let mut harness = Harness::new();

    harness
        .plugin
        .apply_config(&json!({ "baud": 9_600 }))
        .unwrap();
    assert_eq!(harness.plugin.baud(), 9_600);
    assert_eq!(harness.plugin.bit_cycles(), DEFAULT_CLOCK_HZ / 9_600);
    harness.frame(0x55);
    assert_eq!(harness.plugin.last_rx_byte(), Some(0x55));

    harness
        .plugin
        .apply_config(&json!({ "baud": 115_200 }))
        .unwrap();
    assert_eq!(harness.plugin.bit_cycles(), DEFAULT_CLOCK_HZ / 115_200);
    harness.frame(0x48);
    assert_eq!(harness.plugin.last_rx_byte(), Some(0x48));
    assert_eq!(harness.plugin.state_snapshot()["rx_bytes"], 2);
}

#[test]
fn state_snapshot_exposes_required_fields() {
    let mut harness = Harness::new();
    harness.frame(0x41);

    let snapshot = harness.plugin.state_snapshot();
    assert_eq!(snapshot["baud"], 115_200);
    assert_eq!(snapshot["data_bits"], 8);
    assert_eq!(snapshot["stop_bits"], 1);
    assert_eq!(snapshot["rx_line"], true);
    assert_eq!(snapshot["rx_bytes"], 1);
    assert_eq!(snapshot["tx_bytes"], 0);
    assert_eq!(snapshot["last_byte"], 0x41);
    assert_eq!(snapshot["buffer_len"], 1);
    assert_eq!(snapshot["recent_hex"], "41");
    assert_eq!(snapshot["recent_ascii"], "A");
    assert_eq!(snapshot["framing_errors"], 0);
    assert_eq!(snapshot["clock_hz"], DEFAULT_CLOCK_HZ);
    assert_eq!(snapshot["name"], "UART_TERMINAL");
}

#[test]
fn send_byte_drives_rx_output_with_bit_timing() {
    let mut harness = Harness::new();
    let bit = harness.bit();
    harness.plugin.send_byte(b'A');
    assert_eq!(harness.plugin.pending_tx(), 1);
    // 未推进时钟前线路保持空闲（驱动方尚未移出起始位）。
    // The line stays idle until the clock advances.
    assert!(harness.plugin.ports()[1].level);

    harness.advance(1);
    assert!(!harness.plugin.ports()[1].level, "start bit must be low");

    harness.advance(bit);
    // 'A' = 0b0100_0001，bit0 = 1 → 数据位 0 为高。
    // 'A' = 0b0100_0001, bit0 = 1 → data bit 0 is high.
    assert!(harness.plugin.ports()[1].level);
    harness.advance(bit);
    assert!(!harness.plugin.ports()[1].level, "data bit 1 is low");

    harness.advance(8 * bit);
    let snapshot = harness.plugin.state_snapshot();
    assert_eq!(snapshot["tx_bytes"], 1);
    assert_eq!(snapshot["last_tx_byte"], 0x41);
    assert_eq!(harness.plugin.pending_tx(), 0);
    assert!(harness.plugin.ports()[1].level, "line returns to idle");
}

#[test]
fn invalid_config_is_rejected() {
    let mut plugin = UartTerminalPlugin::new();

    assert!(matches!(
        plugin.apply_config(&json!({ "baud": 0 })),
        Err(ConfigError::OutOfRange { .. })
    ));
    assert!(matches!(
        plugin.apply_config(&json!({ "baud": "fast" })),
        Err(ConfigError::InvalidType { .. })
    ));
    assert!(matches!(
        plugin.apply_config(&json!({ "data_bits": 9 })),
        Err(ConfigError::OutOfRange { .. })
    ));
    assert!(matches!(
        plugin.apply_config(&json!({ "stop_bits": 0 })),
        Err(ConfigError::OutOfRange { .. })
    ));
    assert!(matches!(
        plugin.apply_config(&json!({ "clock_hz": 0 })),
        Err(ConfigError::OutOfRange { .. })
    ));
    assert!(matches!(
        plugin.apply_config(&json!({ "rx_line": 3 })),
        Err(ConfigError::InvalidType { .. })
    ));
    // 失败的配置不得污染现有状态。
    // A rejected configuration must not disturb the current state.
    assert_eq!(plugin.baud(), DEFAULT_BAUD);
    assert_eq!(plugin.state_snapshot()["rx_line"], true);
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
fn device_manager_reports_and_drives_uart_device() {
    let mut manager = DeviceManager::new();
    manager
        .register("uart0".to_string(), Box::new(UartTerminalPlugin::new()))
        .unwrap();
    manager.wire("uart0", 1, 2).unwrap();

    let devices = manager.mcp_list_devices();
    assert_eq!(devices[0]["id"], "uart0");
    assert_eq!(devices[0]["name"], "UART_TERMINAL");
    assert_eq!(devices[0]["has_ui"], false);
    assert_eq!(devices[0]["ports"], json!(["TX", "RX"]));

    let mut pins =
        std::array::from_fn::<GpioPin, 4, _>(|index| GpioPin::new(u8::try_from(index).unwrap()));
    pins[2].set_direction(PinDirection::Input);
    manager.sync_gpio(&mut pins);
    // RX 输出端口驱动 GPIO2 的输入电平（空闲高）。
    // The RX output port drives GPIO2's input level (idle high).
    assert!(pins[2].level());

    let mut ctx = TestCtx::default();
    manager.tick_all(&mut ctx, 1);
    assert_eq!(manager.mcp_query_all()["uart0"]["baud"], 115_200);
}
