//! Shared fixtures for the threaded-executor tests
//! 线程化执行器测试的共享夹具
//!
//! The fixture devices are **in-process** plugins: they exercise the mailbox,
//! the fixed delay and the failure paths without depending on a built `cdylib`.
//! Real dynamic-library coverage lives in `executor.rs` (`led_plugin_*`).
//! 夹具设备都是**进程内**插件：不依赖已构建的 `cdylib` 即可覆盖邮箱、固定延迟与
//! 失败路径。真实动态库覆盖在 `executor.rs`（`led_plugin_*`）中。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::ThreadId;
use std::time::Duration;

use rp2350_plugin::error::PluginError;
use rp2350_plugin::metadata::PluginMetadata;
use rp2350_plugin::{PluginContext, RpPlugin};

use crate::error::ConfigError;
use crate::{DeviceManager, DevicePlugin, DevicePort, ExecutorConfig, PinSnapshot};

/// Behaviour executed by [`TestDevice::tick`] / [`TestDevice::tick`] 执行的行为
#[derive(Debug, Clone, Copy)]
pub(crate) enum Behaviour {
    /// Copy the IN port level to the OUT port / 把 IN 端口电平复制到 OUT 端口
    Echo,
    /// Sleep before returning, modelling a hung device / 休眠以模拟卡死设备
    Hang(u64),
    /// Panic on the worker thread / 在工作线程上 panic
    Panic,
    /// Wait on a shared barrier before returning / 返回前等待共享屏障
    ///
    /// Used to prove that the manager dispatches to every device **before**
    /// collecting any response: a serial dispatcher would deadlock.
    /// 用于证明管理器先向所有设备派发、再统一收集：串行派发会死锁。
    Rendezvous,
    /// Fixed amount of integer work per tick / 每个 tick 的固定整数运算量
    Heavy(u64),
}

/// A two-port test device (IN = port 0, OUT = port 1)
/// 双端口测试设备（IN = 端口 0，OUT = 端口 1）
pub(crate) struct TestDevice {
    metadata: PluginMetadata,
    ports: Vec<DevicePort>,
    behaviour: Behaviour,
    /// Accumulator used by `Heavy` and `apply_config` / `Heavy` 与配置使用的累加器
    acc: u64,
    /// Number of ticks served / 已服务 tick 数
    ticks: u64,
    /// Last epoch reported by `PluginContext::current_cycles`
    /// 上下文报告的最后 epoch
    last_epoch: u64,
    /// Thread that served the last tick / 服务最后 tick 的线程
    last_thread: ThreadId,
    /// Optional liveness counter shared with the test / 与测试共享的存活计数器
    alive: Option<Arc<AtomicUsize>>,
    /// Barrier used by [`Behaviour::Rendezvous`] / [`Behaviour::Rendezvous`] 使用的屏障
    barrier: Option<Arc<std::sync::Barrier>>,
}

impl TestDevice {
    /// Create a device with the given ID and behaviour
    /// 创建具有指定 ID 与行为的设备
    pub(crate) fn new(id: &str, behaviour: Behaviour) -> Self {
        Self {
            metadata: PluginMetadata::builder()
                .id(id)
                .name("Threaded test device")
                .version("0.1.0")
                .description("test fixture / 测试夹具")
                .author("test")
                .abi_version(rp2350_plugin::ABI_VERSION)
                .min_host_version("0.1.0")
                .has_ui(false)
                .tags(vec!["test".to_string()])
                .build(),
            ports: vec![
                DevicePort::input("IN", "input port / 输入端口"),
                DevicePort::output("OUT", "output port / 输出端口"),
            ],
            behaviour,
            acc: 0,
            ticks: 0,
            last_epoch: 0,
            last_thread: std::thread::current().id(),
            alive: None,
            barrier: None,
        }
    }

    /// Create a device that waits on `barrier` in every `tick`
    /// 创建每次 `tick` 都等待 `barrier` 的设备
    pub(crate) fn rendezvous(id: &str, barrier: Arc<std::sync::Barrier>) -> Self {
        let mut device = Self::new(id, Behaviour::Rendezvous);
        device.barrier = Some(barrier);
        device
    }

    /// Create a device that counts itself in `alive` until it is dropped
    /// 创建在释放前会计入 `alive` 的设备
    ///
    /// Lets a test prove that the plugin box came back from the worker and was
    /// destroyed on the simulation thread. 让测试证明插件 box 已从工作线程交还
    /// 并在模拟线程上销毁。
    pub(crate) fn tracked(id: &str, behaviour: Behaviour, alive: Arc<AtomicUsize>) -> Self {
        let mut device = Self::new(id, behaviour);
        alive.fetch_add(1, Ordering::SeqCst);
        device.alive = Some(alive);
        device
    }

    /// Replace the port table (used to model a plugin whose ports changed)
    /// 替换端口表（用于模拟端口变化的插件）
    pub(crate) fn with_ports(mut self, ports: Vec<DevicePort>) -> Self {
        self.ports = ports;
        self
    }
}

impl RpPlugin for TestDevice {
    fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    fn init(&mut self, _ctx: &mut dyn PluginContext) -> Result<(), PluginError> {
        Ok(())
    }

    fn tick(&mut self, ctx: &mut dyn PluginContext, _cycles: u64) {
        self.ticks += 1;
        self.last_epoch = ctx.current_cycles();
        self.last_thread = std::thread::current().id();
        match self.behaviour {
            Behaviour::Echo => self.ports[1].level = self.ports[0].level,
            Behaviour::Hang(millis) => {
                std::thread::sleep(Duration::from_millis(millis));
                self.ports[1].level = self.ports[0].level;
            }
            Behaviour::Panic => panic!("test device panic / 测试设备 panic"),
            Behaviour::Rendezvous => {
                if let Some(barrier) = &self.barrier {
                    let _ = barrier.wait();
                }
                self.ports[1].level = self.ports[0].level;
            }
            Behaviour::Heavy(iterations) => {
                let mut acc = self.acc ^ self.last_epoch;
                for step in 0..iterations {
                    acc = acc
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(step);
                }
                self.acc = acc;
                self.ports[1].level = acc & 1 == 1;
            }
        }
    }

    fn shutdown(&mut self) {}

    fn state_snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "ticks": self.ticks,
            "epoch": self.last_epoch,
            "acc": self.acc,
            "out": self.ports[1].level,
            "thread": format!("{:?}", self.last_thread),
        })
    }
}

impl DevicePlugin for TestDevice {
    fn ports(&self) -> &[DevicePort] {
        &self.ports
    }
    fn ports_mut(&mut self) -> &mut [DevicePort] {
        &mut self.ports
    }

    fn on_gpio_change(&mut self, port_index: usize, level: bool) {
        if let Some(port) = self.ports.get_mut(port_index) {
            port.level = level;
        }
    }

    fn apply_config(&mut self, config: &serde_json::Value) -> Result<(), ConfigError> {
        let level = config
            .get("level")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| ConfigError::MissingField("level".to_string()))?;
        self.acc = u64::from(level);
        Ok(())
    }
}

impl Drop for TestDevice {
    fn drop(&mut self) {
        if let Some(alive) = &self.alive {
            alive.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// Build a manager in synchronous or threaded mode
/// 以同步或线程化模式构建管理器
///
/// Threaded mode uses the **production default** configuration, so the tests
/// exercise the same five-second stall bound as real hosts.
/// 线程化模式使用**生产默认**配置，因此测试与真实宿主使用同一个 5 秒卡死上限。
pub(crate) fn make_manager(threaded: bool) -> DeviceManager {
    if threaded {
        DeviceManager::with_executor(ExecutorConfig::default())
    } else {
        DeviceManager::new()
    }
}

/// Build a threaded manager with an explicit latency and timeout
/// 以指定延迟与超时构建线程化管理器
pub(crate) fn make_threaded(latency: u64, timeout: Duration) -> DeviceManager {
    DeviceManager::with_executor(ExecutorConfig::new(latency, timeout))
}

/// Build a synchronous manager with an explicit latency
/// 以指定延迟构建同步管理器
pub(crate) fn make_sync(latency: u64) -> DeviceManager {
    DeviceManager::with_latency(latency)
}

/// Fingerprint that always differs from the recorded one
/// 始终与已记录指纹不同的指纹
pub(crate) fn stale_fingerprint() -> crate::Fingerprint {
    crate::Fingerprint {
        len: 1,
        modified: None,
    }
}

/// Register a two-port device wired to `in_pin` (port 0) and `out_pin` (port 1)
/// 注册双端口设备，端口 0 → `in_pin`，端口 1 → `out_pin`
pub(crate) fn register_echo(dm: &mut DeviceManager, id: &str, in_pin: u8, out_pin: u8) {
    dm.register(
        id.to_string(),
        Box::new(TestDevice::new(id, Behaviour::Echo)),
    )
    .expect("register echo device");
    dm.wire(id, 0, in_pin).expect("wire input port");
    dm.wire(id, 1, out_pin).expect("wire output port");
}

/// Run `epochs` steps and collect the effective output mask of each epoch
/// 运行 `epochs` 个步进并收集每个 epoch 的生效输出掩码
pub(crate) fn run_sequence(
    dm: &mut DeviceManager,
    epochs: u64,
    in_pin: u8,
    levels: &[bool],
) -> Vec<Option<u64>> {
    let mut effective = Vec::with_capacity(usize::try_from(epochs).unwrap_or(usize::MAX));
    for epoch in 0..epochs {
        let mut inputs = PinSnapshot::empty();
        inputs.set(
            in_pin,
            levels[usize::try_from(epoch).unwrap_or(0) % levels.len()],
        );
        let report = dm.step(epoch, inputs, 1).expect("step must be accepted");
        assert_eq!(report.epoch, epoch);
        effective.push(report.effective.map(PinSnapshot::bits));
    }
    effective
}

/// Expected output sequence of an echo device with the given latency
/// 给定延迟下 echo 设备的期望输出序列
pub(crate) fn expected_echo(
    levels: &[bool],
    epochs: usize,
    latency: u64,
    out_pin: u8,
) -> Vec<Option<u64>> {
    (0..epochs)
        .map(|epoch| {
            let source = epoch.checked_sub(usize::try_from(latency).unwrap_or(usize::MAX))?;
            // 输出跟随输入电平；只有 OUT 引脚位被置位。
            // The output follows the input level; only the OUT pin bit is set.
            Some(if levels[source % levels.len()] {
                1u64 << out_pin
            } else {
                0
            })
        })
        .collect()
}
