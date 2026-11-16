//! Fault-handling and performance tests for the threaded executor
//! 线程化执行器的故障处理与性能测试

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::tests::fixtures::{
    Behaviour, TestDevice, make_manager, make_sync, register_echo, stale_fingerprint,
};
use crate::{ConfigApplyError, DeviceManager, DevicePort, PinSnapshot};

/// A hung device is latched high-Z and the simulation keeps running
/// 卡死设备被锁定为高阻，仿真继续运行
#[test]
fn timeout_latches_device_high_z_without_stopping_simulation() {
    // 管理器用生产默认 5 s：注册/接线/线程启动有充足余量；接线完成后只把
    // 卡死设备的卡死上限设为 400 ms，健康设备不受影响。卡死设备 3 s 不返回。
    // The manager uses the production 5 s bound for registration/wiring/spawn;
    // only the hung device is then given a 400 ms stall bound. The hung device
    // does not return within 3 s.
    let mut dm = make_manager(true);
    dm.register(
        "slow".to_string(),
        Box::new(TestDevice::new("slow", Behaviour::Hang(3000))),
    )
    .unwrap();
    dm.wire("slow", 0, 40).unwrap();
    dm.wire("slow", 1, 41).unwrap();
    register_echo(&mut dm, "fast", 42, 43);
    assert!(dm.set_step_timeout("slow", Duration::from_millis(400)));
    assert!(!dm.set_step_timeout("missing", Duration::from_millis(400)));

    let mut inputs = PinSnapshot::empty();
    inputs.set(40, true);
    inputs.set(42, true);

    let first = dm.step(0, inputs, 1).expect("step must not abort");
    // 400 ms 上限必须先于 3 s 的插件睡眠生效，否则 degraded 为空而测试失败。
    // The 400 ms bound must fire before the 3 s plugin sleep; otherwise `degraded`
    // is empty and this assertion fails.
    assert_eq!(
        first.degraded,
        vec!["slow".to_string()],
        "{:?}",
        dm.degraded_reasons()
    );
    assert!(
        first
            .produced
            .iter()
            .any(|(id, outputs)| id == "fast" && outputs.bits() == 1 << 43),
        "healthy device still produces: {:?}",
        first.produced
    );
    // 高阻设备保留在 produced 中，快照为空（不驱动任何引脚）。
    // The high-Z device stays in `produced` with an empty (no-drive) snapshot.
    assert!(
        first
            .produced
            .iter()
            .any(|(id, outputs)| id == "slow" && outputs.is_empty()),
        "high-Z device must not vanish from produced: {:?}",
        first.produced
    );

    // 后续步进不再触碰卡死设备；降级与健康输出保持稳定。
    // Later steps never touch the hung device; the latch and the healthy output
    // stay stable.
    for epoch in 1..5u64 {
        let report = dm.step(epoch, inputs, 1).expect("step must not abort");
        assert_eq!(report.degraded, vec!["slow".to_string()]);
        assert_eq!(
            report.effective.map(PinSnapshot::bits),
            Some(1 << 43),
            "healthy output stays effective"
        );
    }

    // 诊断行稳定（同一设备每次查询内容一致）。
    // The diagnostic line is stable across queries.
    let reasons = dm.degraded_reasons();
    assert_eq!(reasons, dm.degraded_reasons());
    assert_eq!(reasons.len(), 1);
    assert_eq!(reasons[0].0, "slow");
    assert!(reasons[0].1.contains("timed out"), "{reasons:?}");
    assert!(reasons[0].1.contains("high-Z"), "{reasons:?}");
    assert_eq!(dm.degraded_devices(), vec!["slow".to_string()]);

    // MCP 查询高阻设备返回稳定错误，健康设备继续正常。
    // MCP query of the high-Z device returns a stable error; the healthy one works.
    let state = dm.mcp_query_device("slow").expect("device still known");
    assert_eq!(state["degraded"], serde_json::json!(true));
    assert!(state["error"].as_str().unwrap().contains("high-Z"));
    assert_eq!(
        dm.mcp_query_device("fast").unwrap()["ticks"],
        serde_json::json!(5)
    );
}

/// A panicking plugin only degrades itself / panic 插件只降级自己
#[test]
fn panic_is_isolated_to_the_failing_device() {
    let mut dm = make_manager(true);
    dm.register(
        "boom".to_string(),
        Box::new(TestDevice::new("boom", Behaviour::Panic)),
    )
    .unwrap();
    dm.wire("boom", 0, 30).unwrap();
    dm.wire("boom", 1, 31).unwrap();
    register_echo(&mut dm, "ok", 32, 33);

    let mut inputs = PinSnapshot::empty();
    inputs.set(32, true);
    let first = dm
        .step(0, inputs, 1)
        .expect("panic must not abort the step");
    assert_eq!(first.degraded, vec!["boom".to_string()]);
    assert!(
        first
            .produced
            .iter()
            .any(|(id, outputs)| id == "ok" && outputs.bits() == 1 << 33)
    );

    let reasons = dm.degraded_reasons();
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(reasons[0].1.contains("panic"), "{reasons:?}");
    assert_eq!(reasons, dm.degraded_reasons());

    let second = dm.step(1, PinSnapshot::empty(), 1).unwrap();
    assert_eq!(second.degraded, vec!["boom".to_string()]);
    assert_eq!(
        dm.mcp_query_device("ok").unwrap()["ticks"],
        serde_json::json!(2)
    );
}

/// Hot reload replaces a worker-hosted device and keeps the wiring
/// 热重载替换工作线程上的设备并保持接线
#[test]
fn reload_swaps_worker_hosted_device_and_keeps_wiring() {
    let dir = crate::tests::temp_dir("thread-reload");
    let Some(plugin) = crate::tests::copy_led_plugin(&dir) else {
        eprintln!("SKIP: LED plugin not built / 未构建 LED 插件，跳过");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    };
    let id = "com.rp2350-rs.led";
    let mut dm = make_manager(true);
    // 旧实例声明同名端口，重载后接线应被保留。
    // The old instance exposes the same port name, so wiring must be kept.
    dm.register(
        id.to_string(),
        Box::new(
            TestDevice::new(id, Behaviour::Echo)
                .with_ports(vec![DevicePort::input("LED", "LED input")]),
        ),
    )
    .unwrap();
    dm.wire(id, 0, 25).expect("wire LED");
    assert_eq!(dm.thread_count(), 1);
    assert!(dm.set_source_for_test(id, plugin, Some(stale_fingerprint())));

    let report = dm.reload(&dir);
    assert_eq!(report.reloaded.len(), 1, "{report:?}");
    assert_eq!(report.rewired.len(), 1, "{report:?}");
    assert_eq!(report.unwired, Vec::<(String, String)>::new(), "{report:?}");
    assert_eq!(report.failed, Vec::<(String, String)>::new(), "{report:?}");
    assert_eq!(dm.thread_count(), 1, "new instance runs on a worker");
    crate::tests::assert_wiring_consistent(&dm);

    // 重载后的真实 LED 插件仍然响应接线输入。
    // The reloaded real LED plugin still reacts to its wired input.
    let mut inputs = PinSnapshot::empty();
    inputs.set(25, true);
    dm.step(0, inputs, 1).expect("step reloaded device");
    assert_eq!(
        dm.mcp_query_device(id).unwrap()["on"],
        serde_json::json!(true)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `unregister` returns only after the worker is joined and its plugin dropped
/// `unregister` 在工作线程 join 且插件释放后才返回
#[test]
fn unregister_joins_before_returning() {
    let alive = Arc::new(AtomicUsize::new(0));
    let mut dm = make_manager(true);
    for (index, device) in ["dev-a", "dev-b", "dev-c"].iter().enumerate() {
        dm.register(
            device.to_string(),
            Box::new(TestDevice::tracked(
                device,
                Behaviour::Echo,
                Arc::clone(&alive),
            )),
        )
        .unwrap();
        let base = u8::try_from(index * 2).unwrap();
        dm.wire(device, 0, base + 1).unwrap();
        dm.wire(device, 1, base + 2).unwrap();
    }
    assert_eq!(dm.thread_count(), 3);
    assert_eq!(alive.load(Ordering::SeqCst), 3);

    dm.unregister(&"dev-b".to_string()).expect("unregister");

    // 返回即已 join：插件 box 已被释放，线程计数立即准确。
    // Returned means joined: the plugin box is already dropped and the thread
    // count is accurate immediately.
    assert_eq!(
        alive.load(Ordering::SeqCst),
        2,
        "the stopped worker's plugin must be dropped before unregister returns"
    );
    assert_eq!(dm.thread_count(), 2);
    assert!(dm.ports_of("dev-b").is_none());

    let mut inputs = PinSnapshot::empty();
    inputs.set(1, true);
    let report = dm.step(0, inputs, 1).expect("step");
    let ids: Vec<&str> = report.produced.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, vec!["dev-a", "dev-c"], "unregistered device is gone");
    assert_eq!(report.degraded, Vec::<String>::new());
}

/// Unregistering one device stops only its worker
/// 注销一台设备只停止它自己的工作线程
#[test]
fn unregister_stops_only_its_worker() {
    let mut dm = make_manager(true);
    register_echo(&mut dm, "dev-a", 1, 11);
    register_echo(&mut dm, "dev-b", 2, 12);
    register_echo(&mut dm, "dev-c", 3, 13);
    assert_eq!(dm.thread_count(), 3);

    dm.unregister(&"dev-b".to_string()).expect("unregister");
    assert_eq!(dm.thread_count(), 2);
    assert!(dm.ports_of("dev-b").is_none());
    assert!(
        dm.wiring()
            .all()
            .iter()
            .all(|entry| entry.device_id != "dev-b")
    );
    crate::tests::assert_wiring_consistent(&dm);
    assert!(matches!(
        dm.unregister(&"dev-b".to_string()),
        Err(crate::DeviceError::NotFound(_))
    ));

    // 其余设备继续按各自 epoch 推进。
    // The remaining devices keep stepping independently.
    let mut inputs = PinSnapshot::empty();
    inputs.set(1, true);
    let started = Instant::now();
    let report = dm.step(0, inputs, 1).unwrap();
    let elapsed = started.elapsed();
    assert_eq!(
        report.degraded,
        Vec::<String>::new(),
        "a healthy echo device must never time out; produced={:?} reasons={:?} elapsed={elapsed:?}",
        report.produced,
        dm.degraded_reasons()
    );
    assert_eq!(
        report.produced.len(),
        2,
        "produced={:?} degraded={:?} reasons={:?} elapsed={elapsed:?}",
        report.produced,
        report.degraded,
        dm.degraded_reasons()
    );
    assert!(
        report
            .produced
            .iter()
            .any(|(id, outputs)| id == "dev-a" && outputs.bits() == 1 << 11)
    );
    assert!(
        report
            .produced
            .iter()
            .any(|(id, outputs)| id == "dev-c" && outputs.bits() == 0)
    );
}

/// `shutdown_threads` joins every worker and returns the plugin boxes
/// `shutdown_threads` 回收所有工作线程并交还插件
#[test]
fn shutdown_threads_reclaims_all_workers() {
    let alive = Arc::new(AtomicUsize::new(0));
    let mut dm = make_manager(true);
    for device in ["dev-a", "dev-b", "dev-c"] {
        dm.register(
            device.to_string(),
            Box::new(TestDevice::tracked(
                device,
                Behaviour::Echo,
                Arc::clone(&alive),
            )),
        )
        .unwrap();
        dm.wire(device, 0, 25).unwrap();
        dm.wire(device, 1, 26).unwrap();
    }
    assert_eq!(dm.thread_count(), 3);
    assert_eq!(alive.load(Ordering::SeqCst), 3);
    dm.step(0, PinSnapshot::empty(), 1).unwrap();

    assert_eq!(dm.shutdown_threads(), 3);
    assert_eq!(dm.thread_count(), 0);
    assert_eq!(
        alive.load(Ordering::SeqCst),
        0,
        "every plugin box must come back and be dropped"
    );
    // 幂等：没有线程可回收时返回 0。
    // Idempotent: nothing left to reclaim.
    assert_eq!(dm.shutdown_threads(), 0);
    // 设备仍保持注册，但已无法推进；它们以高阻（空快照）出现在 `produced`。
    // Devices stay registered but can no longer be stepped; they are reported as
    // high-Z (empty snapshots) in `produced`.
    assert_eq!(dm.mcp_list_devices().len(), 3);
    let report = dm.step(1, PinSnapshot::empty(), 1).unwrap();
    assert_eq!(
        report.degraded,
        vec![
            "dev-a".to_string(),
            "dev-b".to_string(),
            "dev-c".to_string()
        ]
    );
    assert_eq!(report.produced.len(), 3);
    assert!(
        report
            .produced
            .iter()
            .all(|(_, outputs)| outputs.is_empty()),
        "stopped workers are high-Z, not absent: {:?}",
        report.produced
    );
    assert!(dm.mcp_query_device("dev-a").unwrap()["error"].is_string());
}

/// `apply_config` is executed on the worker thread and rejects invalid input
/// `apply_config` 在工作线程执行，并拒绝非法输入
#[test]
fn configure_reaches_the_worker_thread() {
    let mut threaded = make_manager(true);
    let mut sync = make_sync(1);
    for dm in [&mut threaded, &mut sync] {
        dm.register(
            "dev".to_string(),
            Box::new(TestDevice::new("dev", Behaviour::Echo)),
        )
        .unwrap();
        dm.wire("dev", 0, 50).unwrap();
        dm.wire("dev", 1, 51).unwrap();
    }
    threaded
        .apply_config("dev", &serde_json::json!({ "level": true }))
        .expect("threaded configure");
    sync.apply_config("dev", &serde_json::json!({ "level": true }))
        .expect("inline configure");
    assert_eq!(
        threaded.mcp_query_device("dev").unwrap()["acc"],
        serde_json::json!(1)
    );
    assert_eq!(
        sync.mcp_query_device("dev").unwrap()["acc"],
        serde_json::json!(1)
    );

    // 非法配置：线程化路径经邮箱返回执行器错误，内联路径保持类型化错误。
    // Invalid config: the threaded path reports an executor error, the inline path
    // keeps the typed configuration error.
    let threaded_error = threaded
        .apply_config("dev", &serde_json::json!({}))
        .expect_err("missing field must fail");
    assert!(matches!(threaded_error, ConfigApplyError::Executor(_)));
    let sync_error = sync
        .apply_config("dev", &serde_json::json!({}))
        .expect_err("missing field must fail");
    assert!(matches!(
        sync_error,
        ConfigApplyError::Invalid(crate::error::ConfigError::MissingField(_))
    ));
    assert!(matches!(
        threaded.apply_config("missing", &serde_json::json!({ "level": true })),
        Err(ConfigApplyError::DeviceNotFound(_))
    ));
    // 配置失败不会让设备降级。
    // A rejected configuration does not degrade the device.
    assert!(!threaded.mcp_query_device("dev").unwrap()["acc"].is_null());
    assert_eq!(threaded.thread_count(), 1);
}

/// Heavy plugins reach the same state on both paths, and never time out
/// 重插件在两条路径上状态一致，且不会超时降级
///
/// Wall-clock speedup is only printed, never asserted: `cargo test` runs this
/// suite in parallel with other tests and builds, so a host-time ratio measures
/// machine load rather than the executor. True parallel dispatch is proven
/// deterministically by `parallel_dispatch_is_proven_by_rendezvous_plugins`.
/// 墙钟加速比只打印、不断言：`cargo test` 会与本套件其它测试/构建并行，用宿主时间
/// 比值度量的是机器负载而不是执行器。真正的并行派发由
/// `parallel_dispatch_is_proven_by_rendezvous_plugins` 确定性证明。
#[test]
fn heavy_plugins_match_sequential_state() {
    const ITERATIONS: u64 = 400_000;
    const EPOCHS: u64 = 10;
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let mut sync = make_manager(false);
    let mut threaded = make_manager(true);
    for dm in [&mut sync, &mut threaded] {
        for device in ["heavy-a", "heavy-b", "heavy-c"] {
            dm.register(
                device.to_string(),
                Box::new(TestDevice::new(device, Behaviour::Heavy(ITERATIONS))),
            )
            .unwrap();
        }
    }

    // 预热：第一次步进包含线程启动与邮箱建立。
    // Warm-up: the first step includes thread startup and mailbox setup.
    sync.step(0, PinSnapshot::empty(), 1).unwrap();
    threaded.step(0, PinSnapshot::empty(), 1).unwrap();

    let started = Instant::now();
    for epoch in 1..=EPOCHS {
        sync.step(epoch, PinSnapshot::empty(), 1).unwrap();
    }
    let sequential = started.elapsed();

    let started = Instant::now();
    for epoch in 1..=EPOCHS {
        threaded.step(epoch, PinSnapshot::empty(), 1).unwrap();
    }
    let parallel = started.elapsed();

    // 任何健康重插件都不能在默认上限内被判超时。
    // No healthy heavy plugin may be latched high-Z within the default bound.
    assert_eq!(
        threaded.degraded_devices(),
        Vec::<String>::new(),
        "heavy workers must not degrade: {:?}",
        threaded.degraded_reasons()
    );

    // 两条路径必须得到相同的最终状态（同样的客户机工作）。
    // Both paths must reach the same final state (identical guest work).
    for device in ["heavy-a", "heavy-b", "heavy-c"] {
        let inline = sync.mcp_query_device(device).expect("inline state");
        let threaded = threaded.mcp_query_device(device).expect("threaded state");
        for field in ["ticks", "epoch", "acc", "out"] {
            assert_eq!(inline[field], threaded[field], "{device}.{field} diverged");
        }
    }

    let ratio = parallel.as_secs_f64() / sequential.as_secs_f64();
    println!("MT-5 perf: sync {sequential:?} threaded {parallel:?} ratio {ratio:.2} cores {cores}");
}

/// The manager dispatches every device before collecting any response
/// 管理器先向所有设备派发，再统一收集
///
/// Three plugins wait on a barrier of three parties inside `tick`. Only a
/// dispatcher that sends to all devices before collecting can let them meet; a
/// serial dispatch would deadlock and trip the executor timeout.
/// 三个插件在 `tick` 内等待三方屏障。只有「先全部派发、再收集」的调度才能让它们
/// 汇合；串行派发会死锁并触发执行器超时。
#[test]
fn parallel_dispatch_is_proven_by_rendezvous_plugins() {
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let mut dm = make_manager(true);
    for device in ["rdv-a", "rdv-b", "rdv-c"] {
        dm.register(
            device.to_string(),
            Box::new(TestDevice::rendezvous(device, Arc::clone(&barrier))),
        )
        .unwrap();
    }

    let started = Instant::now();
    let report = dm.step(0, PinSnapshot::empty(), 1).expect("step");
    let elapsed = started.elapsed();
    assert_eq!(
        report.degraded,
        Vec::<String>::new(),
        "serial dispatch would time out: produced={:?} reasons={:?}",
        report.produced,
        dm.degraded_reasons()
    );
    assert_eq!(
        report.produced.len(),
        3,
        "every rendezvous device participates: {:?}",
        report.produced
    );
    assert!(
        elapsed < crate::DEFAULT_STEP_TIMEOUT,
        "the barrier must be reached by all workers in one dispatch round, took {elapsed:?}"
    );
}

/// 16 busy host threads must not starve healthy workers into a false timeout
/// 16 个繁忙宿主线程不得把健康工作线程饿成误超时
///
/// Models a fully loaded `cargo test` host: the default five-second bound leaves
/// ample scheduling headroom, so every device still reports its own output.
/// 模拟满载的 `cargo test` 宿主：默认 5 秒上限留有充足调度余量，因此每台设备仍
/// 返回自己的输出。
#[test]
fn sixteen_host_threads_do_not_starve_device_workers() {
    const DEVICES: usize = 8;
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut burners = Vec::with_capacity(16);
    for _ in 0..16 {
        let stop = Arc::clone(&stop);
        burners.push(std::thread::spawn(move || {
            let mut acc: u64 = 0;
            while !stop.load(Ordering::Relaxed) {
                acc = acc
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                std::hint::black_box(acc);
            }
        }));
    }

    let mut dm = make_manager(true);
    let ids: Vec<String> = (0..DEVICES)
        .map(|index| format!("load-{index:02}"))
        .collect();
    for (index, id) in ids.iter().enumerate() {
        let in_pin = u8::try_from(index * 2).unwrap();
        register_echo(&mut dm, id, in_pin, in_pin + 1);
    }

    let mut inputs = PinSnapshot::empty();
    for index in 0..DEVICES {
        inputs.set(u8::try_from(index * 2).unwrap(), index % 2 == 0);
    }
    let started = Instant::now();
    let report = dm.step(0, inputs, 1).expect("step under load");
    let elapsed = started.elapsed();

    stop.store(true, Ordering::Relaxed);
    for burner in burners {
        burner.join().expect("burner thread");
    }

    assert_eq!(
        report.degraded,
        Vec::<String>::new(),
        "host load must not degrade healthy devices: {:?} elapsed={elapsed:?}",
        dm.degraded_reasons()
    );
    assert_eq!(report.produced.len(), DEVICES, "{:?}", report.produced);
    for (index, id) in ids.iter().enumerate() {
        let expected = if index % 2 == 0 {
            1u64 << (u8::try_from(index * 2).unwrap() + 1)
        } else {
            0
        };
        assert!(
            report
                .produced
                .iter()
                .any(|(device, outputs)| device == id && outputs.bits() == expected),
            "{id} must produce {expected:#x}: {:?}",
            report.produced
        );
    }
}

/// Dropping a manager with workers must not leak handles (drop joins them)
/// 带工作线程的管理器析构时必须回收句柄（析构即 join）
#[test]
fn dropping_threaded_manager_stops_workers() {
    let alive = Arc::new(AtomicUsize::new(0));
    {
        let mut dm: DeviceManager = make_manager(true);
        for device in ["dev-a", "dev-b"] {
            dm.register(
                device.to_string(),
                Box::new(TestDevice::tracked(
                    device,
                    Behaviour::Echo,
                    Arc::clone(&alive),
                )),
            )
            .unwrap();
        }
        assert_eq!(dm.thread_count(), 2);
        assert_eq!(alive.load(Ordering::SeqCst), 2);
    }
    assert_eq!(
        alive.load(Ordering::SeqCst),
        0,
        "manager drop must reclaim both plugins"
    );
}
