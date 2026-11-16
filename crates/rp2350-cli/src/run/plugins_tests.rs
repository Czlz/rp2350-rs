//! `run` plugin runtime tests: drivers, epochs, wiring and thread reclamation
//! `run` 插件运行时测试：驱动、epoch、接线与线程回收
//!
//! Split out of `plugins.rs` so every source file stays under the 500-line rule.
//! The tests drive [`PluginRuntime`] through in-process mocks that implement
//! `DevicePlugin` directly, so no prebuilt `cdylib` is needed. Clock-injection,
//! generation-gate and recording/rejecting mocks live in `plugins_clock_tests.rs`.
//! 从 `plugins.rs` 拆出以满足单文件不超过 500 行的规则。测试用直接实现
//! `DevicePlugin` 的进程内 mock 驱动 [`PluginRuntime`]，无需预构建的 `cdylib`。
//! 时钟注入、世代门控与记录型/拒绝型 mock 位于 `plugins_clock_tests.rs`。

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use rp2350_plugin::error::PluginError;
use rp2350_plugin::metadata::PluginMetadata;
use rp2350_plugin::{PluginContext, RpPlugin};
use rp2350_virt_periph::{DevicePlugin, DevicePort};

use super::super::{ArchArg, Machine, MachineOptions};
use super::*;

pub(super) struct EchoDevice {
    metadata: PluginMetadata,
    ports: Vec<DevicePort>,
    live: Arc<AtomicUsize>,
    ticks: Arc<AtomicU64>,
}

impl EchoDevice {
    pub(super) fn new(live: &Arc<AtomicUsize>, ticks: &Arc<AtomicU64>) -> Self {
        live.fetch_add(1, Ordering::SeqCst);
        Self {
            metadata: PluginMetadata::builder()
                .id("test.echo")
                .name("Echo")
                .abi_version(rp2350_plugin::ABI_VERSION)
                .build(),
            ports: vec![
                DevicePort::input("IN", "input / 输入"),
                DevicePort::output("OUT", "output / 输出"),
            ],
            live: Arc::clone(live),
            ticks: Arc::clone(ticks),
        }
    }
}

impl Drop for EchoDevice {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}

impl RpPlugin for EchoDevice {
    fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }
    fn init(&mut self, _ctx: &mut dyn PluginContext) -> Result<(), PluginError> {
        Ok(())
    }
    fn tick(&mut self, _ctx: &mut dyn PluginContext, _cycles: u64) {
        self.ticks.fetch_add(1, Ordering::SeqCst);
    }
    fn shutdown(&mut self) {}
}

impl DevicePlugin for EchoDevice {
    fn ports(&self) -> &[DevicePort] {
        &self.ports
    }
    fn ports_mut(&mut self) -> &mut [DevicePort] {
        &mut self.ports
    }
    fn on_gpio_change(&mut self, port_index: usize, level: bool) {
        if port_index == 0 {
            self.ports[1].level = level;
        }
    }
}

fn mock_manager(threaded: bool, live: &Arc<AtomicUsize>, ticks: &Arc<AtomicU64>) -> DeviceManager {
    let mut manager = if threaded {
        DeviceManager::with_executor(ExecutorConfig::default())
    } else {
        DeviceManager::new()
    };
    manager
        .register(
            "test.echo".to_string(),
            Box::new(EchoDevice::new(live, ticks)),
        )
        .expect("register echo device");
    manager.wire("test.echo", 0, 20).expect("wire input");
    manager.wire("test.echo", 1, 21).expect("wire output");
    manager
}

fn runtime(threaded: bool, pads: &GpioPads) -> (PluginRuntime, Arc<AtomicUsize>, Arc<AtomicU64>) {
    runtime_at(threaded, pads, test_clock())
}

/// Build a runtime with an explicit host clock / 用显式宿主时钟构造运行时
pub(super) fn runtime_at(
    threaded: bool,
    pads: &GpioPads,
    clock: HostClock,
) -> (PluginRuntime, Arc<AtomicUsize>, Arc<AtomicU64>) {
    let (live, ticks) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicU64::new(0)));
    let manager = mock_manager(threaded, &live, &ticks);
    (
        PluginRuntime::with_manager(manager, pads, clock),
        live,
        ticks,
    )
}

/// Neutral fake host clock; not a default or firmware frequency
/// 测试用的中性伪宿主时钟，不是默认值或固件频率
pub(super) fn test_clock() -> HostClock {
    HostClock {
        generation: 3,
        clock_hz: 12_000_000,
        sys_clock_hz: 48_000_000,
    }
}

pub(super) fn machine() -> Machine {
    let options = MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles: 1_000_000,
        strict_decode: false,
        psram: None,
    };
    Machine::new(&crate::test_support::infinite_loop_elf(), &options).expect("build machine")
}
#[test]
fn inline_and_threaded_paths_match_for_thirty_epochs() {
    let (inline_pads, threaded_pads) = (GpioPads::new(), GpioPads::new());
    let (mut inline, _, inline_ticks) = runtime(false, &inline_pads);
    let (mut threaded, _, threaded_ticks) = runtime(true, &threaded_pads);
    assert_eq!(inline.manager.thread_count(), 0);
    assert_eq!(threaded.manager.thread_count(), 1);

    let mut snapshots = Vec::new();
    for epoch in 0..30u64 {
        let level = epoch % 3 != 0;
        inline_pads.set_external_pin(20, level);
        threaded_pads.set_external_pin(20, level);
        let steps = epoch * PLUGIN_EPOCH_STEPS;
        assert!(inline.drive(steps, test_clock()) && threaded.drive(steps, test_clock()));
        snapshots.push((inline.pins[21].level(), threaded.pins[21].level()));
    }
    // 30 个 epoch 的引脚快照逐点一致，且等于延迟 1 的回显序列。
    // 30 per-epoch snapshots agree pointwise and match the latency-1 echo.
    for (epoch, (inline_level, threaded_level)) in snapshots.iter().enumerate() {
        assert_eq!(inline_level, threaded_level, "epoch {epoch}");
        assert_eq!(*inline_level, epoch > 0 && (epoch - 1) % 3 != 0);
    }
    assert_eq!((inline.epochs, threaded.epochs), (30, 30));
    assert_eq!(inline_ticks.load(Ordering::SeqCst), 30);
    assert_eq!(threaded_ticks.load(Ordering::SeqCst), 30);

    // 状态行必须暴露设备与 JSON 状态，供 CLI 输出可见 / status lines are visible.
    let lines = threaded.status_lines();
    assert!(lines[0].contains("devices=1"), "{}", lines[0]);
    assert!(
        lines.iter().any(|line| line.contains("\"test.echo\"")),
        "{lines:?}"
    );
}
#[test]
fn host_clock_captures_live_peripheral_rates() {
    let machine = machine();
    let clock = HostClock::capture(machine.peripherals());
    // 值必须来自实时时钟树，而不是常量 / values come from the live clock tree.
    assert_eq!(clock.clock_hz, machine.peripherals().periclk_hz());
    assert_eq!(clock.sys_clock_hz, machine.peripherals().sysclk_hz());
    assert_eq!(clock.generation, machine.peripherals().clock_generation());
    assert!(clock.clock_hz > 0 && clock.sys_clock_hz > 0);
}

#[test]
fn epoch_advances_once_per_64_step_window() {
    let mut machine = machine();
    let (runtime, _, ticks) = runtime(true, machine.peripherals().pads());
    machine.install_plugins_for_test(runtime);
    assert_eq!(machine.plugin_epochs_for_test(), Some(0));

    // 一个 64 步窗口只提交一次，同一 epoch 不重复提交 / one commit per window.
    machine.run_steps(64).expect("first window");
    assert_eq!(machine.plugin_epochs_for_test(), Some(1));
    assert!(!machine.drive_plugins_for_test(64));

    // 128 步共提交两次 / two windows commit twice in total.
    machine.run_steps(64).expect("second window");
    assert_eq!(machine.plugin_epochs_for_test(), Some(2));
    assert_eq!(ticks.load(Ordering::SeqCst), 2);
    assert_eq!(machine.steps(), 128);
}
#[test]
fn shutdown_threads_leaves_no_workers() {
    let (mut device, live, _) = runtime(true, &GpioPads::new());
    assert_eq!(live.load(Ordering::SeqCst), 1);
    assert_eq!(device.manager.thread_count(), 1);
    assert_eq!(device.shutdown(), 1);
    assert_eq!(device.manager.thread_count(), 0);
    assert_eq!(live.load(Ordering::SeqCst), 0, "worker plugin is destroyed");
    assert_eq!(device.shutdown(), 0, "shutdown is idempotent");

    // 退出路径（Drop）也回收线程并销毁插件 / Drop also reclaims and destroys.
    let (dropped, drop_live, _) = runtime(true, &GpioPads::new());
    assert_eq!(drop_live.load(Ordering::SeqCst), 1);
    drop(dropped);
    assert_eq!(drop_live.load(Ordering::SeqCst), 0);
}
#[test]
fn absent_plugin_configuration_is_a_no_op() {
    let options = PluginOptions::default();
    assert!(!options.is_active());
    assert_eq!(options.plugin_dir(), Path::new(DEFAULT_RUN_PLUGIN_DIR));
    install(&options);
    assert!(
        runtime_from_active(&GpioPads::new(), test_clock())
            .expect("inactive")
            .is_none()
    );
    clear();
    assert!(
        runtime_from_active(&GpioPads::new(), test_clock())
            .expect("cleared")
            .is_none()
    );

    // 未配置插件时机器不持有运行时，也不产生 `[plugins]` 输出。
    // No runtime and no `[plugins]` line, so historical output is unchanged.
    let mut machine = machine();
    assert_eq!(machine.plugin_epochs_for_test(), None);
    assert_eq!(machine.plugin_status_lines(), Vec::<String>::new());
    machine.run_steps(128).expect("run without plugins");
    assert_eq!(machine.plugin_status_lines(), Vec::<String>::new());

    // `--wire` 解析仍可用，非法引脚被拒绝 / parsing works, bad pins rejected.
    assert_eq!(WireSpec::parse("dev:1:0x1A").expect("hex gpio").gpio, 26);
    assert!(WireSpec::parse("dev:0:300").is_err());
}
