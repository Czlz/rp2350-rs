//! `run` plugin clock tests: injection, generation gating and mock recording
//! `run` 插件时钟测试：注入、世代门控与 mock 记录
//!
//! Split out of `plugins_tests.rs` so every source file stays under the
//! 500-line rule. Driver, epoch, wiring and thread-reclamation tests stay in
//! `plugins_tests.rs`; its mock helpers are shared through `use super::`.
//! 从 `plugins_tests.rs` 拆出以满足单文件不超过 500 行的规则。驱动、epoch、接线
//! 与线程回收测试保留在 `plugins_tests.rs`，其 mock helper 通过 `use super::` 共享。

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rp2350_plugin::error::PluginError;
use rp2350_plugin::metadata::PluginMetadata;
use rp2350_plugin::{PluginContext, RpPlugin};
use rp2350_virt_periph::error::ConfigError;
use rp2350_virt_periph::{DevicePlugin, DevicePort};
use serde_json::{Value, json};

use super::plugins_tests::{EchoDevice, machine, runtime_at, test_clock};
use super::*;

/// Drive several epochs, retune the real clock tree and check the gate
/// 连续推进多个 epoch，改写真实时钟树并检查世代门控
fn assert_clock_gating(threaded: bool) {
    use rp2350_core::AddressBus;

    let mut machine = machine();
    let first = HostClock::capture(machine.peripherals());
    assert!(first.clock_hz > 0 && first.sys_clock_hz > 0);
    let pads = machine.peripherals().pads().clone();
    let (mut runtime, _, ticks) = runtime_at(threaded, &pads, first);
    // 创建时立即注入一次 / one injection at construction.
    assert_eq!(runtime.clock_injections, 1);
    assert_eq!(runtime.clock, Some(first));
    // 载荷必须携带通用键名与实时 `clk_peri` / `clk_sys` 值。
    // The payload carries the universal keys and the live rates.
    let expected = format!(
        r#"{{"clock_hz":{},"sys_clock_hz":{}}}"#,
        first.clock_hz, first.sys_clock_hz
    );
    assert_eq!(runtime.last_payload.as_deref(), Some(expected.as_str()));

    for epoch in 1..=5u64 {
        assert!(runtime.drive(epoch * PLUGIN_EPOCH_STEPS, first));
    }
    // 同一世代不得每 epoch 重发（线程模式避免邮箱往返）。
    // The same generation is never resent, so threaded mode pays no mailbox
    // round trip per epoch.
    assert_eq!(runtime.clock_injections, 1);
    assert_eq!(ticks.load(Ordering::SeqCst), 5);

    // 通过公开的 `PLL_SYS` 寄存器改频：12 MHz × 126 / (6 × 1) = 252 MHz。
    // Retune through the public `PLL_SYS` registers: 12 × 126 / (6 × 1).
    let mut cycles = 0u64;
    machine
        .bus
        .write_u32(rp2350_periph::map::PLL_SYS_BASE + 0x08, 126, &mut cycles)
        .expect("fbdiv");
    machine
        .bus
        .write_u32(
            rp2350_periph::map::PLL_SYS_BASE + 0x0C,
            (6 << 16) | (1 << 12),
            &mut cycles,
        )
        .expect("prim");
    let retuned = HostClock::capture(machine.peripherals());
    assert!(retuned.generation > first.generation, "generation bumps");
    assert_ne!(retuned, first, "live rates changed");
    assert_eq!(retuned.sys_clock_hz, 252_000_000);
    assert_eq!(
        retuned.clock_hz, 252_000_000,
        "CLK_PERI follows CLK_SYS at reset defaults"
    );

    assert!(runtime.drive(6 * PLUGIN_EPOCH_STEPS, retuned));
    assert_eq!(runtime.clock_injections, 2, "retune injects once");
    assert!(runtime.drive(7 * PLUGIN_EPOCH_STEPS, retuned));
    assert_eq!(runtime.clock_injections, 2, "retune is not repeated");
    assert_eq!(runtime.clock, Some(retuned));
    let expected = format!(
        r#"{{"clock_hz":{},"sys_clock_hz":{}}}"#,
        retuned.clock_hz, retuned.sys_clock_hz
    );
    assert_eq!(runtime.last_payload.as_deref(), Some(expected.as_str()));

    // 状态行必须暴露注入后的实时值 / status exposes the injected values.
    let peri = format!("clk_peri={}", retuned.clock_hz);
    let sys = format!("clk_sys={}", retuned.sys_clock_hz);
    let lines = runtime.status_lines();
    assert!(
        lines.iter().any(|line| line.contains(&peri)
            && line.contains(&sys)
            && line.contains("injections=2")),
        "{lines:?}"
    );
}

#[test]
fn clock_is_injected_once_per_generation_inline() {
    assert_clock_gating(false);
}

#[test]
fn clock_reaches_threaded_workers_once_per_generation() {
    assert_clock_gating(true);
}

#[test]
fn host_clock_is_sent_to_every_loaded_device() {
    let pads = GpioPads::new();
    let (live, ticks) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicU64::new(0)));
    let mut manager = DeviceManager::new();
    for id in ["test.echo", "test.alpha"] {
        manager
            .register(id.to_string(), Box::new(EchoDevice::new(&live, &ticks)))
            .expect("register echo device");
    }
    let runtime = PluginRuntime::with_manager(manager, &pads, test_clock());
    // 注入覆盖全部已加载设备，顺序为管理器 ID 顺序 / every loaded device.
    assert_eq!(runtime.last_targets, ["test.alpha", "test.echo"]);
    assert_eq!(runtime.clock_injections, 1);
    assert_eq!(runtime.clock, Some(test_clock()));
}

/// Mock device that records every configuration payload it receives
/// 记录每次收到的配置载荷的 mock 设备
struct RecordingDevice {
    metadata: PluginMetadata,
    ports: Vec<DevicePort>,
    /// Shared record of every payload / 所有载荷的共享记录
    payloads: Arc<Mutex<Vec<Value>>>,
    /// Payloads applied on the owning thread / 在所属线程上已应用的载荷数
    applied: u64,
    /// Latest payload applied on the owning thread / 所属线程上最近应用的载荷
    current: Option<Value>,
}

impl RecordingDevice {
    fn new(payloads: &Arc<Mutex<Vec<Value>>>) -> Self {
        Self {
            metadata: PluginMetadata::builder()
                .id("test.recorder")
                .name("Recorder")
                .abi_version(rp2350_plugin::ABI_VERSION)
                .build(),
            ports: vec![DevicePort::input("IN", "input / 输入")],
            payloads: Arc::clone(payloads),
            applied: 0,
            current: None,
        }
    }
}

impl RpPlugin for RecordingDevice {
    fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }
    fn init(&mut self, _ctx: &mut dyn PluginContext) -> Result<(), PluginError> {
        Ok(())
    }
    fn tick(&mut self, _ctx: &mut dyn PluginContext, _cycles: u64) {}
    fn shutdown(&mut self) {}
    /// Snapshot on the owning thread; a payload is only visible if it arrived
    /// 所属线程上的快照；只有载荷真正到达插件才可见
    fn state_snapshot(&self) -> Value {
        json!({
            "applied": self.applied,
            "config": self.current.clone().unwrap_or(Value::Null),
        })
    }
}

impl DevicePlugin for RecordingDevice {
    fn ports(&self) -> &[DevicePort] {
        &self.ports
    }
    fn ports_mut(&mut self) -> &mut [DevicePort] {
        &mut self.ports
    }
    fn on_gpio_change(&mut self, _port_index: usize, _level: bool) {}
    fn apply_config(&mut self, config: &Value) -> Result<(), ConfigError> {
        self.applied += 1;
        self.current = Some(config.clone());
        self.payloads
            .lock()
            .expect("recording lock is never poisoned")
            .push(config.clone());
        Ok(())
    }
}

/// Mock device that rejects every configuration payload
/// 拒绝每次配置载荷的 mock 设备
struct RejectingDevice {
    metadata: PluginMetadata,
    ports: Vec<DevicePort>,
    /// Rejected configuration attempts / 被拒绝的配置尝试次数
    attempts: Arc<AtomicU64>,
}

impl RejectingDevice {
    fn new(attempts: &Arc<AtomicU64>) -> Self {
        Self {
            metadata: PluginMetadata::builder()
                .id("test.refuser")
                .name("Refuser")
                .abi_version(rp2350_plugin::ABI_VERSION)
                .build(),
            ports: vec![DevicePort::input("IN", "input / 输入")],
            attempts: Arc::clone(attempts),
        }
    }
}

impl RpPlugin for RejectingDevice {
    fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }
    fn init(&mut self, _ctx: &mut dyn PluginContext) -> Result<(), PluginError> {
        Ok(())
    }
    fn tick(&mut self, _ctx: &mut dyn PluginContext, _cycles: u64) {}
    fn shutdown(&mut self) {}
    fn state_snapshot(&self) -> Value {
        json!({ "attempts": self.attempts.load(Ordering::SeqCst) })
    }
}

impl DevicePlugin for RejectingDevice {
    fn ports(&self) -> &[DevicePort] {
        &self.ports
    }
    fn ports_mut(&mut self) -> &mut [DevicePort] {
        &mut self.ports
    }
    fn on_gpio_change(&mut self, _port_index: usize, _level: bool) {}
    fn apply_config(&mut self, _config: &Value) -> Result<(), ConfigError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        // 拒绝型 mock：值越界。运行必须只记录告警并继续。
        // Rejecting mock: out-of-range value; the run must log and continue.
        Err(ConfigError::OutOfRange {
            field: "clock_hz".to_string(),
            message: "mock rejects the host clock / mock 拒绝宿主时钟".to_string(),
        })
    }
}

/// Register one mock on the requested execution path
/// 在指定执行路径上注册一个 mock
fn manager_with(threaded: bool, id: &str, device: Box<dyn DevicePlugin>) -> DeviceManager {
    let mut manager = if threaded {
        DeviceManager::with_executor(ExecutorConfig::default())
    } else {
        DeviceManager::new()
    };
    manager
        .register(id.to_string(), device)
        .expect("register mock device");
    manager
}

/// Snapshot of every payload recorded so far / 目前已记录的载荷快照
fn recorded(payloads: &Arc<Mutex<Vec<Value>>>) -> Vec<Value> {
    payloads
        .lock()
        .expect("recording lock is never poisoned")
        .clone()
}

fn recording_runtime(
    threaded: bool,
    pads: &GpioPads,
    clock: HostClock,
) -> (PluginRuntime, Arc<Mutex<Vec<Value>>>) {
    let payloads = Arc::new(Mutex::new(Vec::new()));
    let manager = manager_with(
        threaded,
        "test.recorder",
        Box::new(RecordingDevice::new(&payloads)),
    );
    (PluginRuntime::with_manager(manager, pads, clock), payloads)
}

/// Assert one injection at construction, none per epoch, one after a retune
/// 断言构造注入一次、同世代逐 epoch 不重发、改频后再注入一次
fn assert_recorded_injections(threaded: bool) {
    let pads = GpioPads::new();
    let first = test_clock();
    let (mut runtime, payloads) = recording_runtime(threaded, &pads, first);
    assert_eq!(runtime.manager.thread_count(), usize::from(threaded));

    // 创建时恰好一次，字段等于实时 `clk_peri`/`clk_sys` 且为 JSON 数字。
    // Exactly once at construction, carrying the live rates as JSON numbers.
    let initial = recorded(&payloads);
    assert_eq!(initial.len(), 1, "construction injects exactly once");
    let payload = &initial[0];
    assert!(payload["clock_hz"].is_number(), "{payload}");
    assert!(payload["sys_clock_hz"].is_number(), "{payload}");
    assert_eq!(
        payload["clock_hz"].as_u64(),
        Some(u64::from(first.clock_hz))
    );
    assert_eq!(
        payload["sys_clock_hz"].as_u64(),
        Some(u64::from(first.sys_clock_hz))
    );

    // 同一世代 5 个 epoch 不重发 / five epochs of the same generation resend nothing.
    for epoch in 1..=5u64 {
        assert!(runtime.drive(epoch * PLUGIN_EPOCH_STEPS, first));
    }
    assert_eq!(
        recorded(&payloads).len(),
        1,
        "same generation is not resent"
    );

    // 改频后下一个 epoch 恰好再收到一次，值已更新。
    // The first epoch after a retune receives exactly one updated payload.
    let retuned = HostClock {
        generation: first.generation + 1,
        clock_hz: first.clock_hz / 2,
        sys_clock_hz: first.sys_clock_hz / 2,
    };
    assert!(runtime.drive(6 * PLUGIN_EPOCH_STEPS, retuned));
    let updated = recorded(&payloads);
    assert_eq!(updated.len(), 2, "a new generation injects once");
    assert!(updated[1]["clock_hz"].is_number(), "{}", updated[1]);
    assert_eq!(
        updated[1]["clock_hz"].as_u64(),
        Some(u64::from(retuned.clock_hz))
    );
    assert_eq!(
        updated[1]["sys_clock_hz"].as_u64(),
        Some(u64::from(retuned.sys_clock_hz))
    );

    // 状态快照在设备所属线程上读回最近载荷：线程路径据此证明载荷经邮箱到达。
    // The snapshot reads the latest payload on the owning thread, which is how
    // the threaded path proves the mailbox delivered it.
    let state = runtime.manager.mcp_query_all();
    let device = &state["test.recorder"];
    assert_eq!(device["applied"].as_u64(), Some(2), "{state}");
    assert_eq!(
        device["config"]["clock_hz"].as_u64(),
        Some(u64::from(retuned.clock_hz))
    );
    assert_eq!(
        device["config"]["sys_clock_hz"].as_u64(),
        Some(u64::from(retuned.sys_clock_hz))
    );

    // 世代门控继续抑制逐 epoch 重发 / the gate keeps suppressing per-epoch sends.
    assert!(runtime.drive(7 * PLUGIN_EPOCH_STEPS, retuned));
    assert_eq!(recorded(&payloads).len(), 2);
    assert_eq!(runtime.clock_injections, 2);
    assert_eq!(runtime.epochs, 7);
}

#[test]
fn recorder_receives_one_payload_per_generation_inline() {
    assert_recorded_injections(false);
}

#[test]
fn recorder_receives_one_payload_per_generation_threaded() {
    assert_recorded_injections(true);
}

/// A rejected clock only warns: driving continues and attempts keep counting
/// 被拒绝的时钟只告警：运行继续推进，注入尝试继续累计
fn assert_rejections_are_logged(threaded: bool) {
    let pads = GpioPads::new();
    let first = test_clock();
    let attempts = Arc::new(AtomicU64::new(0));
    let manager = manager_with(
        threaded,
        "test.refuser",
        Box::new(RejectingDevice::new(&attempts)),
    );
    let mut runtime = PluginRuntime::with_manager(manager, &pads, first);
    assert_eq!(runtime.manager.thread_count(), usize::from(threaded));

    // 构造期那次注入已被拒绝，但注入计数仍然递增。
    // The construction injection was rejected, yet it is still counted.
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(runtime.clock_injections, 1);
    assert_eq!(runtime.epochs, 0);
    assert_eq!(runtime.error, None);

    // 20 个连续 epoch 各改一次频率：每次重发、每次被拒绝、绝不上抛 panic。
    // Twenty consecutive epochs with a retune each: every send is rejected and
    // ignored, and nothing panics.
    let mut clock = first;
    for epoch in 1..=20u64 {
        clock = HostClock {
            generation: clock.generation + 1,
            ..clock
        };
        assert!(runtime.drive(epoch * PLUGIN_EPOCH_STEPS, clock));
        assert_eq!(runtime.epochs, epoch, "epoch keeps advancing");
        assert_eq!(
            runtime.clock_injections,
            epoch + 1,
            "injection attempts keep counting"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), epoch + 1);
    }
    assert_eq!(runtime.error, None);

    // 拒绝不降级设备；工作线程侧计数器证明每次拒绝都发生在 worker 上。
    // A rejection never degrades the device; the worker-side counter proves every
    // rejection happened on the worker.
    assert!(
        runtime.manager.degraded_devices().is_empty(),
        "rejection does not degrade"
    );
    let state = runtime.manager.mcp_query_all();
    assert_eq!(
        state["test.refuser"]["attempts"].as_u64(),
        Some(21),
        "{state}"
    );

    // 诊断只有 `log::warn!`，`status_lines` 不出现 `[plugins] error`。
    // Diagnostics are warnings only; `status_lines` never grows an error line.
    let lines = runtime.status_lines();
    assert!(
        !lines.iter().any(|line| line.contains("[plugins] error")),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("injections=21")),
        "{lines:?}"
    );
}

#[test]
fn rejected_clock_config_only_warns_inline() {
    assert_rejections_are_logged(false);
}

#[test]
fn rejected_clock_config_only_warns_threaded() {
    assert_rejections_are_logged(true);
}
