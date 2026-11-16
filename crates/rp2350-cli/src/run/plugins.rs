//! Virtual device plugins driven from the `run` loop / `run` 循环中的虚拟设备插件
//!
//! [`PluginRuntime`] is driven once per [`PLUGIN_EPOCH_STEPS`] retired steps at the
//! machine's pump boundary. Both modes use [`DeviceManager::step_gpio`] (inline on
//! the simulation thread, threaded on workers), so the fixed publish latency
//! matches and pin snapshots are comparable. Without `--plugin-dir`/`--wire` no
//! manager is built (`None`) and the historical output is unchanged.
//! [`PluginRuntime`] 只在机器的泵送边界按固定步数推进一次。两种模式都用
//! [`DeviceManager::step_gpio`]（内联在模拟线程、线程模式在工作线程），固定发布
//! 延迟一致，引脚快照可逐点对照。未传插件参数时不构造管理器（`None`），历史输出不变。

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rp2350_core::gpio::GpioPin;
use rp2350_periph::BootPeripherals;
use rp2350_periph::gpio_pads::{GpioPads, PAD_COUNT};
use rp2350_virt_periph::{DeviceManager, ExecutorConfig};

/// Default plugin directory of `run` (Cargo `cdylib` output) / `run` 的默认插件目录
pub const DEFAULT_RUN_PLUGIN_DIR: &str = "target/release";

/// Machine steps per plugin epoch; equals the pump interval / 每个插件 epoch 的步数
pub const PLUGIN_EPOCH_STEPS: u64 = 64;

/// Live host clock snapshot pushed into every loaded device
/// 推送给每个已加载设备的宿主时钟快照
///
/// `clock_hz` is the clock of the pads the devices sample — `clk_peri`, the
/// UART/PIO peripheral clock — not `clk_sys`. `generation` is the clock tree's
/// write generation and is the re-injection gate: the same generation is never
/// sent twice, so threaded devices pay no mailbox round trip per epoch.
/// `clock_hz` 是设备采样 pad 的时钟——`clk_peri`，即 UART/PIO 外设时钟——而不是
/// `clk_sys`。`generation` 是时钟树写入世代，也是重发判据：同一世代绝不重发，
/// 因此线程化设备不会每个 epoch 都付一次邮箱往返。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostClock {
    /// Clock-tree write generation / 时钟树写入世代
    generation: u32,
    /// Live `clk_peri` in Hz / 实时 `clk_peri`（Hz）
    clock_hz: u32,
    /// Live `clk_sys` in Hz / 实时 `clk_sys`（Hz）
    sys_clock_hz: u32,
}

impl HostClock {
    /// Capture the live rates and generation of a booted machine
    /// 捕获已启动机器的实时频率与世代
    #[must_use]
    pub fn capture(peripherals: &BootPeripherals) -> Self {
        Self {
            generation: peripherals.clock_generation(),
            clock_hz: peripherals.periclk_hz(),
            sys_clock_hz: peripherals.sysclk_hz(),
        }
    }
}

/// One `--wire DEVICE_ID:PORT:GPIO` request / 一条 `--wire` 请求
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireSpec {
    /// Device ID from the plugin metadata / 插件元数据中的设备 ID
    pub device: String,
    /// Port index on the device / 设备上的端口索引
    pub port: usize,
    /// GPIO pin wired to that port / 连接到该端口的 GPIO 引脚
    pub gpio: u8,
}

impl WireSpec {
    /// Parse `DEVICE_ID:PORT:GPIO` (GPIO accepts decimal or `0x` hex)
    /// 解析 `DEVICE_ID:PORT:GPIO`（GPIO 支持十进制或 `0x` 十六进制）
    ///
    /// # Errors
    ///
    /// Names the malformed field. / 指出格式错误的字段。
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut parts = spec.rsplitn(3, ':');
        let gpio = parts.next().unwrap_or_default();
        let port = parts.next().unwrap_or_default();
        let device = parts.next().unwrap_or_default().trim();
        if device.is_empty() {
            return Err(format!("missing device id in `{spec}` / 缺少设备 ID"));
        }
        let port = port
            .trim()
            .parse::<usize>()
            .map_err(|_| format!("bad port index `{port}` in `{spec}` / 端口索引非法"))?;
        let gpio = gpio.trim();
        let pin = match gpio.strip_prefix("0x").or_else(|| gpio.strip_prefix("0X")) {
            Some(hex) => u8::from_str_radix(hex, 16).ok(),
            None => gpio.parse().ok(),
        }
        .ok_or_else(|| format!("bad GPIO pin `{gpio}` in `{spec}` / GPIO 引脚非法"))?;
        Ok(Self {
            device: device.to_string(),
            port,
            gpio: pin,
        })
    }
}

/// Plugin configuration of one `run` invocation / 一次 `run` 的插件配置
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginOptions {
    /// Explicit plugin directory, `None` = default / 显式插件目录，`None` 为默认值
    pub plugin_dir: Option<PathBuf>,
    /// Ports to wire, in command-line order / 需接线的端口，按命令行顺序
    pub wires: Vec<WireSpec>,
    /// Drive plugins on worker threads instead of inline / 用工作线程而非内联驱动
    pub threads: bool,
}

impl PluginOptions {
    /// Whether any flag activates plugins / 是否有参数启用插件
    ///
    /// `--plugin-threads` alone does nothing; `--plugin-dir` alone still scans.
    /// 仅 `--plugin-threads` 无效果；仅 `--plugin-dir` 仍会扫描。
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.plugin_dir.is_some() || !self.wires.is_empty()
    }

    /// Plugin directory to scan / 要扫描的插件目录
    #[must_use]
    pub fn plugin_dir(&self) -> &Path {
        self.plugin_dir
            .as_deref()
            .unwrap_or_else(|| Path::new(DEFAULT_RUN_PLUGIN_DIR))
    }
}

thread_local! {
    static ACTIVE: RefCell<Option<PluginOptions>> = const { RefCell::new(None) };
}

/// Install the plugin configuration for machines built on this thread
/// 在当前线程上为将要构造的机器安装插件配置
pub fn install(options: &PluginOptions) {
    ACTIVE.with(|slot| *slot.borrow_mut() = Some(options.clone()));
}

/// Remove the installed plugin configuration / 移除已安装的插件配置
pub fn clear() {
    ACTIVE.with(|slot| *slot.borrow_mut() = None);
}

/// Build a runtime from the installed configuration, `None` when inactive
/// 依据已安装配置构造运行时；未启用时返回 `None`
///
/// The host clock is injected into every loaded device once at construction.
/// 宿主时钟在构造时向每个已加载设备注入一次。
///
/// # Errors
///
/// Propagates directory, load and wiring failures. / 传播目录、加载与接线错误。
pub(crate) fn runtime_from_active(
    pads: &GpioPads,
    clock: HostClock,
) -> Result<Option<PluginRuntime>> {
    let options = ACTIVE.with(|slot| slot.borrow().clone());
    let Some(options) = options.filter(PluginOptions::is_active) else {
        return Ok(None);
    };
    PluginRuntime::new(&options, pads, clock).map(Some)
}

/// Virtual device runtime attached to one [`super::Machine`] / 挂接到机器的运行时
pub struct PluginRuntime {
    manager: DeviceManager,
    pins: Vec<GpioPin>,
    pads: GpioPads,
    last_epoch: Option<u64>,
    last_steps: u64,
    /// Host clock last pushed into every device / 上次推送给所有设备的宿主时钟
    clock: Option<HostClock>,
    /// Completed host-clock injections / 已完成的宿主时钟注入次数
    clock_injections: u64,
    /// Last payload rendered as text, test diagnostics only
    /// 最近一次载荷的文本形式，仅用于测试诊断
    #[cfg(test)]
    last_payload: Option<String>,
    /// Devices targeted by the last injection, test diagnostics only
    /// 最近一次注入的目标设备，仅用于测试诊断
    #[cfg(test)]
    last_targets: Vec<String>,
    pub(crate) epochs: u64,
    error: Option<String>,
}

impl PluginRuntime {
    /// Scan, load every device and apply the wiring / 扫描、加载全部设备并应用接线
    ///
    /// The host clock is part of the generic device contract: every device
    /// receives `{"clock_hz": <clk_peri>, "sys_clock_hz": <clk_sys>}` before its
    /// first step.
    /// 宿主时钟属于通用设备契约：每个设备在首次步进前都会收到
    /// `{"clock_hz": <clk_peri>, "sys_clock_hz": <clk_sys>}`。
    ///
    /// # Errors
    ///
    /// Missing directory, no loadable plugin, or an unknown `--wire` target.
    /// 目录缺失、无插件可加载，或 `--wire` 指向未知设备/端口时返回错误。
    pub fn new(options: &PluginOptions, pads: &GpioPads, clock: HostClock) -> Result<Self> {
        let dir = options.plugin_dir();
        if !dir.is_dir() {
            bail!("plugin dir not found: {} / 插件目录不存在", dir.display());
        }
        let mut manager = if options.threads {
            DeviceManager::with_executor(ExecutorConfig::default())
        } else {
            DeviceManager::new()
        };
        let loaded = manager.scan_and_load(dir);
        log::info!(
            "run: loaded {} plugin(s) from {} / 已加载设备插件",
            loaded.len(),
            dir.display()
        );
        if manager.device_count() == 0 {
            bail!(
                "no plugins loaded from {} / 未加载到设备插件",
                dir.display()
            );
        }
        for wire in &options.wires {
            manager
                .wire(&wire.device, wire.port, wire.gpio)
                .with_context(|| {
                    format!(
                        "failed to wire '{}' port {} to GPIO{} / 接线失败",
                        wire.device, wire.port, wire.gpio
                    )
                })?;
        }
        Ok(Self::with_manager(manager, pads, clock))
    }

    /// Wrap an already populated manager and inject the host clock
    /// 包装已就绪的管理器并注入宿主时钟
    fn with_manager(manager: DeviceManager, pads: &GpioPads, clock: HostClock) -> Self {
        let mut runtime = Self {
            manager,
            pins: new_pins(),
            pads: pads.clone(),
            last_epoch: None,
            last_steps: 0,
            clock: None,
            clock_injections: 0,
            #[cfg(test)]
            last_payload: None,
            #[cfg(test)]
            last_targets: Vec::new(),
            epochs: 0,
            error: None,
        };
        runtime.inject_clock(clock);
        runtime
    }

    /// Step every device once if `steps` reached a new epoch
    /// `steps` 进入新 epoch 时推进所有设备一次
    ///
    /// Repeated calls inside one 64-step window are rejected. A clock-tree
    /// generation change seen at the pump boundary is re-injected before the
    /// devices step, so the new epoch already decodes at the new rate.
    /// 同窗口重复调用被拒。泵送边界发现时钟树世代变化时，会在设备步进前重新注入，
    /// 因此新 epoch 已按新频率解码。
    pub fn drive(&mut self, steps: u64, clock: HostClock) -> bool {
        if self.error.is_some() {
            return false;
        }
        let epoch = steps / PLUGIN_EPOCH_STEPS;
        if self.last_epoch.is_some_and(|last| epoch <= last) {
            return false;
        }
        let cycles = steps.saturating_sub(self.last_steps).max(1);
        self.last_epoch = Some(epoch);
        self.last_steps = steps;
        self.inject_clock(clock);
        self.sync_inputs();
        match self.manager.step_gpio(epoch, &mut self.pins, cycles) {
            Ok(_) => {
                self.epochs += 1;
                self.sync_outputs();
                true
            }
            Err(error) => {
                self.error = Some(error.to_string());
                false
            }
        }
    }

    /// Push the universal host-clock JSON to every loaded device
    /// 向每个已加载设备推送通用宿主时钟 JSON
    ///
    /// The payload is `{"clock_hz": <clk_peri>, "sys_clock_hz": <clk_sys>}`.
    /// Threaded devices receive it through the manager's mailbox; a device that
    /// rejects it is only logged, and the run continues.
    /// 载荷为 `{"clock_hz": <clk_peri>, "sys_clock_hz": <clk_sys>}`。线程化设备
    /// 经管理器邮箱收到配置；拒绝配置的设备只记录日志，运行继续。
    fn inject_clock(&mut self, clock: HostClock) {
        if self.clock == Some(clock) {
            return;
        }
        // 载荷用 `serde_json::json!` 直接构造，键名与数值一目了然。
        // Build the payload with `serde_json::json!` so keys and values stay legible.
        let payload = serde_json::json!({
            "clock_hz": clock.clock_hz,
            "sys_clock_hz": clock.sys_clock_hz,
        });
        #[cfg(test)]
        {
            self.last_payload = Some(payload.to_string());
        }
        for device in self.manager.mcp_list_devices() {
            let Some(id) = device.get("id").and_then(|value| value.as_str()) else {
                continue;
            };
            #[cfg(test)]
            self.last_targets.push(id.to_string());
            if let Err(error) = self.manager.apply_config(id, &payload) {
                log::warn!("plugin '{id}' rejected host clock {payload}: {error}");
            }
        }
        self.clock = Some(clock);
        self.clock_injections += 1;
    }

    /// Sample the resolved pad levels into the GPIO shadow array
    /// 把解析后的 pad 电平采样到 GPIO 影子数组
    fn sync_inputs(&mut self) {
        let levels = self.pads.input_levels();
        for (index, pin) in self.pins.iter_mut().enumerate() {
            let high = (levels >> u32::try_from(index).unwrap_or(u32::MAX)) & 1 == 1;
            pin.set_input(high);
        }
    }

    /// Write device-driven output levels back to the pads / 把设备输出电平回写到 pad
    fn sync_outputs(&self) {
        let mask = self.manager.driven_pins();
        for (number, pin) in (0u8..).zip(self.pins.iter()) {
            if mask.get(number) {
                self.pads.set_external_pin(u32::from(number), pin.level());
            }
        }
    }

    /// Stop and join every plugin worker; returns how many were running
    /// 停止并回收所有插件工作线程；返回被停止的线程数
    pub fn shutdown(&mut self) -> usize {
        self.manager.shutdown_threads()
    }

    /// `[plugins]` diagnostic lines, including the MCP state JSON
    /// `[plugins]` 诊断行，含 MCP 状态 JSON
    pub fn status_lines(&mut self) -> Vec<String> {
        let state = self.manager.mcp_query_all();
        let mut lines = vec![format!(
            "[plugins] devices={} epochs={} threads={} degraded={}",
            self.manager.device_count(),
            self.epochs,
            self.manager.thread_count(),
            self.manager.degraded_devices().len()
        )];
        if let Some(clock) = self.clock {
            lines.push(format!(
                "[plugins] clock: clk_peri={} clk_sys={} generation={} injections={}",
                clock.clock_hz, clock.sys_clock_hz, clock.generation, self.clock_injections
            ));
        }
        if state.as_object().is_some_and(|object| !object.is_empty()) {
            lines.push(format!("[plugins] state: {state}"));
        }
        for (id, reason) in self.manager.degraded_reasons() {
            lines.push(format!("[plugins] high-Z {id}: {reason}"));
        }
        if let Some(error) = &self.error {
            lines.push(format!("[plugins] error: {error}"));
        }
        lines
    }
}

impl Drop for PluginRuntime {
    fn drop(&mut self) {
        // 退出与错误路径都必须有界地回收工作线程。
        // Every exit and error path reclaims worker threads with a bounded wait.
        let _ = self.shutdown();
    }
}

/// Build the GPIO shadow array with every pin in input mode / 构造全输入的影子数组
fn new_pins() -> Vec<GpioPin> {
    (0..PAD_COUNT)
        .map(|index| GpioPin::new(u8::try_from(index).unwrap_or(u8::MAX)))
        .collect()
}

#[cfg(test)]
#[path = "plugins_tests.rs"]
mod plugins_tests;

#[cfg(test)]
#[path = "plugins_clock_tests.rs"]
mod plugins_clock_tests;
