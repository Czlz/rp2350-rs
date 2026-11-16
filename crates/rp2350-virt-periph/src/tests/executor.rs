//! Determinism tests: pin encoding, fixed latency, sync vs threaded equivalence
//! 确定性测试：引脚编码、固定延迟、同步与线程化等价

use rp2350_core::gpio::{GpioPin, PinDirection};
use std::time::Duration;

use crate::executor::{PIN_SNAPSHOT_PINS, PublishSchedule};
use crate::tests::fixtures::{
    Behaviour, TestDevice, expected_echo, make_manager, make_sync, make_threaded, register_echo,
    run_sequence,
};
use crate::{DeviceManager, PinSnapshot};

/// Input levels replayed by the sequence tests / 序列测试重放的输入电平
const LEVELS: [bool; 5] = [false, true, true, false, true];

/// Snapshot encoding: bit order, boundaries, sampling and masked apply
/// 快照编码：位序、边界、采样与掩码写回
#[test]
fn pin_snapshot_encodes_gpio_levels() {
    let mut snapshot = PinSnapshot::empty();
    for pin in [0u8, 7, 47, 63] {
        snapshot.set(pin, true);
    }
    for pin in [0u8, 7, 47, 63] {
        assert!(snapshot.get(pin), "pin {pin} must be high");
    }
    assert_eq!(snapshot.bits(), (1 << 0) | (1 << 7) | (1 << 47) | (1 << 63));
    // RP2350 只有 GPIO0..47；64 及以上不可表示，写入被忽略。
    // RP2350 only has GPIO0..47; pin 64 and above are not representable.
    snapshot.set(64, true);
    assert!(!snapshot.get(64));
    assert_eq!(snapshot.bits() & 1, 1 << 0);

    snapshot.set(7, false);
    assert!(!snapshot.get(7));
    assert_eq!(
        snapshot.union(PinSnapshot::from_bits(1 << 7)).bits(),
        (1 << 0) | (1 << 7) | (1 << 47) | (1 << 63)
    );

    // 采样：GPIO 方向为 Output 时 level() 取输出电平。
    // Sampling: an Output pin reports its output level through level().
    let mut pins =
        std::array::from_fn::<GpioPin, 48, _>(|index| GpioPin::new(u8::try_from(index).unwrap()));
    pins[3].set_direction(PinDirection::Output);
    pins[3].set_output(true);
    let sampled = PinSnapshot::from_gpio_pins(&pins);
    assert!(sampled.get(3));
    assert!(!sampled.get(4));

    // 掩码写回只触碰被设备驱动的引脚。
    // Masked apply only touches pins driven by a device.
    let mut sink =
        std::array::from_fn::<GpioPin, 48, _>(|index| GpioPin::new(u8::try_from(index).unwrap()));
    sink[4].set_input(true);
    let mask = PinSnapshot::from_bits((1 << 3) | (1 << 5));
    PinSnapshot::from_bits(1 << 3).apply_masked(mask, &mut sink);
    assert!(sink[3].level());
    assert!(!sink[5].level());
    assert!(sink[4].level(), "unmasked pin keeps its host level");
    assert_eq!(PIN_SNAPSHOT_PINS, 64);
}

/// The publish queue releases an epoch's output exactly `latency` epochs later
/// 发布队列在恰好 `latency` 个 epoch 之后释放输出
#[test]
fn publish_schedule_holds_fixed_latency() {
    let mut schedule = PublishSchedule::new(3);
    assert_eq!(schedule.latency(), 3);
    schedule.publish(10, PinSnapshot::from_bits(0b1));
    assert_eq!(schedule.take_effective(12), None);
    assert_eq!(schedule.pending(), 1);
    assert_eq!(
        schedule.take_effective(13).map(PinSnapshot::bits),
        Some(0b1)
    );
    assert_eq!(schedule.pending(), 0);

    // 同一 epoch 的多个设备输出按线或合并；跳过的 epoch 不会被丢弃。
    // Several devices in one epoch are wired-ORed; skipped epochs are not lost.
    let mut schedule = PublishSchedule::new(2);
    schedule.publish(0, PinSnapshot::from_bits(0b1));
    schedule.publish(0, PinSnapshot::from_bits(0b10));
    schedule.publish(2, PinSnapshot::from_bits(0b100));
    assert_eq!(
        schedule.take_effective(9).map(PinSnapshot::bits),
        Some(0b111)
    );
    // 延迟下限为 1 个外设 tick。
    // Latency is clamped to at least one peripheral tick.
    assert_eq!(PublishSchedule::new(0).latency(), 1);
}

/// Sync and threaded paths produce the same per-pin output sequence
/// 同步与线程化路径产生逐 pin 相同的输出序列
#[test]
fn sync_and_threaded_paths_match_pin_for_pin() {
    let mut sync = make_manager(false);
    let mut threaded = make_manager(true);
    for dm in [&mut sync, &mut threaded] {
        register_echo(dm, "dev", 25, 26);
    }

    let sync_seq = run_sequence(&mut sync, 8, 25, &LEVELS);
    let threaded_seq = run_sequence(&mut threaded, 8, 25, &LEVELS);
    assert_eq!(sync_seq, threaded_seq, "paths must agree per pin and epoch");
    assert_eq!(sync_seq, expected_echo(&LEVELS, 8, 1, 26));

    // 设备确实在独立线程上运行（同步路径则在当前线程）。
    // The device really runs on its own thread (the sync path stays here).
    assert_eq!(sync.thread_count(), 0);
    assert_eq!(threaded.thread_count(), 1);
    let local = format!("{:?}", std::thread::current().id());
    assert_eq!(
        sync.mcp_query_device("dev").unwrap()["thread"],
        serde_json::json!(local)
    );
    assert_ne!(
        threaded.mcp_query_device("dev").unwrap()["thread"],
        serde_json::json!(local)
    );
}

/// A latency of 3 epochs delays every output by exactly 3 epochs on both paths
/// 3 epoch 的延迟让两条路径的输出都恰好延后 3 个 epoch
#[test]
fn latency_three_shifts_outputs_on_both_paths() {
    let mut sync = make_sync(3);
    let mut threaded = make_threaded(3, crate::DEFAULT_STEP_TIMEOUT);
    // 同步管理器使用同一延迟配置，便于逐 pin 对照。
    // The synchronous manager uses the same latency for a pin-for-pin comparison.
    for dm in [&mut sync, &mut threaded] {
        register_echo(dm, "dev", 25, 26);
    }
    let sync_seq = run_sequence(&mut sync, 9, 25, &LEVELS);
    let threaded_seq = run_sequence(&mut threaded, 9, 25, &LEVELS);
    assert_eq!(sync_seq, threaded_seq);
    assert_eq!(sync_seq, expected_echo(&LEVELS, 9, 3, 26));
    // 前 3 个 epoch 没有任何生效输出。
    // The first three epochs have no effective output at all.
    assert_eq!(sync_seq[0..3], [None, None, None]);
    // epoch 3 生效的是 epoch 0 的电平（低），epoch 4 生效的是 epoch 1（高）。
    // Epoch 3 carries epoch 0's low level, epoch 4 carries epoch 1's high level.
    assert_eq!(sync_seq[3], Some(0));
    assert_eq!(sync_seq[4], Some(1 << 26));
}

/// Three devices keep independent epochs and never cross-talk
/// 三台设备保持独立 epoch，输出互不串扰
#[test]
fn three_devices_keep_independent_outputs() {
    let mut dm = make_manager(true);
    register_echo(&mut dm, "dev-a", 1, 11);
    register_echo(&mut dm, "dev-b", 2, 12);
    register_echo(&mut dm, "dev-c", 3, 13);
    assert_eq!(dm.thread_count(), 3);

    // 每个 epoch 只驱动一台设备的输入，输出必须只出现在它自己的引脚上。
    // Each epoch drives one device input; outputs must stay on its own pin.
    let plan = [(1u8, true), (2, true), (3, false), (1, false)];
    for (epoch, (pin, level)) in plan.iter().enumerate() {
        let mut inputs = PinSnapshot::empty();
        inputs.set(*pin, *level);
        let report = dm.step(u64::try_from(epoch).unwrap(), inputs, 1).unwrap();
        assert_eq!(report.degraded, Vec::<String>::new());
        let produced: Vec<(String, u64)> = report
            .produced
            .iter()
            .map(|(id, snapshot)| (id.clone(), snapshot.bits()))
            .collect();
        let expected = |device: &str, pin: u8, high: bool| {
            (device.to_string(), if high { 1u64 << pin } else { 0 })
        };
        let high_a = *pin == 1 && *level;
        let high_b = *pin == 2 && *level;
        let high_c = *pin == 3 && *level;
        assert_eq!(
            produced,
            vec![
                expected("dev-a", 11, high_a),
                expected("dev-b", 12, high_b),
                expected("dev-c", 13, high_c),
            ]
        );
    }

    // 每个 worker 都看到自己的 epoch 计数。
    // Every worker observed its own tick count.
    for device in ["dev-a", "dev-b", "dev-c"] {
        let state = dm.mcp_query_device(device).unwrap();
        assert_eq!(state["ticks"], serde_json::json!(4));
        assert_eq!(state["epoch"], serde_json::json!(3));
    }
}

/// Port directions decide what is delivered and what is published
/// 端口方向决定投递与发布的内容
#[test]
fn port_direction_controls_delivery_and_publish() {
    let mut dm = make_manager(true);
    dm.register(
        "dev".to_string(),
        Box::new(TestDevice::new("dev", Behaviour::Echo)),
    )
    .unwrap();
    dm.wire("dev", 0, 20).unwrap();
    // 输出端口未接线：设备可以改变其电平，但不会出现在输出快照里。
    // The output port is unwired: the device may change it but it is not published.
    assert_eq!(dm.driven_pins().bits(), 0);
    let mut inputs = PinSnapshot::empty();
    inputs.set(20, true);
    dm.step(0, inputs, 1).unwrap();
    let report = dm.step(1, inputs, 1).unwrap();
    assert_eq!(report.produced[0].1.bits(), 0);
    // 发布队列会记录“无驱动”的 epoch（低电平），因此生效值是 0 而不是缺失。
    // The publish queue records a "nothing driven" epoch as low, so the effective
    // value is 0 rather than absent.
    assert_eq!(report.effective.map(PinSnapshot::bits), Some(0));

    // 接线后，只有输出端口对应的位被驱动。
    // After wiring, only the bit of the output port is driven.
    dm.wire("dev", 1, 21).unwrap();
    assert_eq!(dm.driven_pins().bits(), 1 << 21);
    dm.step(2, inputs, 1).unwrap();
    let report = dm.step(3, inputs, 1).unwrap();
    assert_eq!(report.produced[0].1.bits(), 1 << 21);
    assert_eq!(report.effective.map(PinSnapshot::bits), Some(1 << 21));
}

/// Epochs must strictly increase; a rejected epoch does not disturb the device
/// epoch 必须严格递增；被拒绝的 epoch 不会扰动设备
#[test]
fn non_monotonic_epoch_is_rejected() {
    let mut dm = make_manager(true);
    register_echo(&mut dm, "dev", 25, 26);
    dm.step(5, PinSnapshot::empty(), 1).unwrap();
    assert!(matches!(
        dm.step(5, PinSnapshot::empty(), 1),
        Err(crate::StepError::NonMonotonicEpoch {
            last: 5,
            requested: 5
        })
    ));
    assert!(matches!(
        dm.step(4, PinSnapshot::empty(), 1),
        Err(crate::StepError::NonMonotonicEpoch {
            last: 5,
            requested: 4
        })
    ));
    // 之后仍可继续，且设备状态没有被回退。
    // Stepping continues afterwards and the device state was not rolled back.
    dm.step(6, PinSnapshot::empty(), 1).unwrap();
    assert_eq!(
        dm.mcp_query_device("dev").unwrap()["ticks"],
        serde_json::json!(2)
    );
}

/// The default manager never spawns workers and keeps the legacy drive paths
/// 默认管理器从不创建线程，并保留旧驱动路径
#[test]
fn synchronous_default_spawns_no_threads() {
    let mut dm = make_manager(false);
    assert!(dm.executor_config().is_none());
    register_echo(&mut dm, "dev", 25, 26);
    assert_eq!(dm.thread_count(), 0);
    assert!(
        !dm.set_step_timeout("dev", Duration::from_millis(1)),
        "inline devices have no mailbox timeout"
    );
    assert_eq!(dm.latency_ticks(), 1);
    dm.step(0, PinSnapshot::empty(), 1).unwrap();
    assert_eq!(dm.thread_count(), 0);

    // 旧路径保持不变：GPIO 电平经 sync_gpio 送到输入端口。
    // The legacy path is unchanged: sync_gpio delivers the pin level.
    let mut pins =
        std::array::from_fn::<GpioPin, 48, _>(|index| GpioPin::new(u8::try_from(index).unwrap()));
    pins[25].set_direction(PinDirection::Output);
    pins[25].set_output(true);
    dm.sync_gpio(&mut pins);
    assert_eq!(
        dm.mcp_query_device("dev").unwrap()["out"],
        serde_json::json!(false)
    );
    pins[25].set_output(false);
    dm.sync_gpio(&mut pins);
    assert_eq!(dm.thread_count(), 0);
}

/// `step_gpio` writes only pins driven by a device, leaving host pins alone
/// `step_gpio` 只写设备驱动的引脚，宿主引脚保持不变
#[test]
fn step_gpio_only_drives_wired_outputs() {
    let mut dm = make_manager(true);
    register_echo(&mut dm, "dev", 20, 21);
    let mut pins =
        std::array::from_fn::<GpioPin, 48, _>(|index| GpioPin::new(u8::try_from(index).unwrap()));
    pins[20].set_direction(PinDirection::Output);
    pins[20].set_output(true);
    pins[22].set_input(true);

    dm.step_gpio(0, &mut pins, 1).unwrap();
    dm.step_gpio(1, &mut pins, 1).unwrap();
    assert!(pins[21].level(), "OUT pin follows the device output");
    assert!(pins[22].level(), "unwired pin keeps the host level");

    pins[20].set_output(false);
    dm.step_gpio(2, &mut pins, 1).unwrap();
    dm.step_gpio(3, &mut pins, 1).unwrap();
    assert!(!pins[21].level(), "OUT pin drops with its input");
    assert!(pins[22].level());
}

/// A real `cdylib` LED plugin gives identical state on both paths
/// 真实 `cdylib` LED 插件在两条路径上给出相同状态
#[test]
fn led_plugin_matches_between_paths() {
    let dir = crate::tests::temp_dir("thread-led");
    if crate::tests::copy_led_plugin(&dir).is_none() {
        eprintln!("SKIP: LED plugin not built / 未构建 LED 插件，跳过");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let mut sync = make_manager(false);
    let mut threaded = make_manager(true);
    for dm in [&mut sync, &mut threaded] {
        assert_eq!(dm.scan_and_load(&dir).len(), 1);
        dm.wire("com.rp2350-rs.led", 0, 25).expect("wire LED");
    }
    assert_eq!(threaded.thread_count(), 1);

    for epoch in 0..6u64 {
        let mut inputs = PinSnapshot::empty();
        inputs.set(25, epoch % 2 == 0);
        let sync_report = sync.step(epoch, inputs, 1).unwrap();
        let threaded_report = threaded.step(epoch, inputs, 1).unwrap();
        assert_eq!(sync_report.produced, threaded_report.produced);
        assert_eq!(
            sync.mcp_query_device("com.rp2350-rs.led"),
            threaded.mcp_query_device("com.rp2350-rs.led"),
            "LED state must match at epoch {epoch}"
        );
    }
    assert_eq!(
        threaded.mcp_query_device("com.rp2350-rs.led").unwrap()["transitions"],
        serde_json::json!(6)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The default executor budget is a stall bound, not a scheduling knob
/// 默认执行器上限是「卡死上限」，不是调度参数
#[test]
fn default_executor_config_is_a_stall_bound() {
    let config = crate::ExecutorConfig::default();
    assert_eq!(config.latency_ticks, crate::DEFAULT_LATENCY_TICKS);
    assert_eq!(config.step_timeout, crate::DEFAULT_STEP_TIMEOUT);
    // 5 秒：远大于任何合法插件步进，避免并行测试/构建误判为高阻。
    // Five seconds: far above any legitimate step, so parallel tests/builds
    // cannot latch a healthy device high-Z.
    assert_eq!(config.step_timeout, Duration::from_secs(5));
}

/// An unwired device produces nothing but is still stepped
/// 未接线的设备不产生输出，但仍会被推进
#[test]
fn unwired_device_still_advances() {
    let mut dm: DeviceManager = make_manager(true);
    dm.register(
        "dev".to_string(),
        Box::new(TestDevice::new("dev", Behaviour::Echo)),
    )
    .unwrap();
    let report = dm.step(0, PinSnapshot::empty(), 1).unwrap();
    assert_eq!(report.produced.len(), 1);
    assert_eq!(report.produced[0].1.bits(), 0);
    assert_eq!(report.effective, None);
    assert_eq!(
        dm.mcp_query_device("dev").unwrap()["ticks"],
        serde_json::json!(1)
    );
}
