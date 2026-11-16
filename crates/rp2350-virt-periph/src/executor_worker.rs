//! Worker side of the plugin mailbox — command loop, port delivery, panic isolation
//! 插件邮箱的工作线程侧 — 命令循环、端口投递、panic 隔离
//!
//! The worker owns the [`DevicePlugin`] box: no host thread may touch the plugin
//! while it runs. Every command carries the simulation epoch, so the plugin sees
//! `PluginContext::current_cycles()` equal to that epoch and never observes host
//! wall-clock time. Panics are caught per command; the plugin is then marked
//! poisoned and is only shut down afterwards.
//! 工作线程独占 [`DevicePlugin`] box：插件运行期间宿主线程不得触碰它。每条命令都
//! 携带仿真 epoch，因此插件看到的 `PluginContext::current_cycles()` 等于该 epoch，
//! 永远不会观察到宿主墙钟时间。panic 按命令捕获，随后插件被标记为中毒，只在关闭
//! 时再调用一次 `shutdown()`。

// 类型名沿用项目命名约定；文档引用使用内部标识符。
// Type names follow project conventions; doc references use internal identifiers.
#![allow(clippy::doc_markdown, clippy::module_name_repetitions)]

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{Receiver, Sender};

use rp2350_plugin::PluginContext;

use super::device::{DevicePlugin, PortDirection};
use super::executor::{PinSnapshot, PortBinding};

/// Maximum number of scheduled callbacks a [`WorkerContext`] remembers
/// [`WorkerContext`] 保存的已调度回调数量上限
const MAX_PENDING_CALLBACKS: usize = 64;

/// Command sent from the simulation thread to one plugin worker
/// 从模拟线程发送给单个插件工作线程的命令
///
/// Every variant is a request: the worker answers with exactly one
/// [`Response`], so the host can enforce a bounded wait per command.
/// 每个变体都是请求：工作线程恰好回复一个 [`Response`]，宿主因此可以对每条命令
/// 施加有界等待。
pub enum Command {
    /// Deliver the pin snapshot sampled at `epoch` and advance by `cycles`
    /// 投递在 `epoch` 采样的引脚快照，并推进 `cycles` 个周期
    Step {
        /// Simulation epoch of the input snapshot / 输入快照所在的仿真 epoch
        epoch: u64,
        /// GPIO levels visible to the device at `epoch` / `epoch` 时刻设备可见的引脚电平
        inputs: PinSnapshot,
        /// Cycles to advance the plugin by / 插件推进的周期数
        cycles: u64,
    },
    /// Apply a JSON configuration to the plugin
    /// 向插件应用 JSON 配置
    Configure {
        /// Configuration value forwarded to `apply_config`
        /// 转发给 `apply_config` 的配置值
        config: serde_json::Value,
    },
    /// Replace the plugin instance, returning the previous one to the host
    /// 替换插件实例，并把旧实例交还宿主
    ///
    /// The previous instance is returned **before** it is dropped so the host can
    /// destroy it while the backing library is still mapped. 旧实例在释放前交还
    /// 宿主，使宿主能在库仍映射时销毁它。
    Reload {
        /// New plugin instance / 新插件实例
        plugin: Box<dyn DevicePlugin>,
        /// Port → GPIO binding table for the new instance
        /// 新实例的端口 → GPIO 绑定表
        bindings: Vec<PortBinding>,
    },
    /// Replace the port → GPIO binding table without touching the plugin
    /// 仅替换端口 → GPIO 绑定表，不触碰插件
    SetBindings {
        /// New binding table / 新绑定表
        bindings: Vec<PortBinding>,
    },
    /// Request `state_snapshot()` / 请求 `state_snapshot()`
    QueryState,
    /// Shut down and hand the plugin back to the host
    /// 关闭并把插件交还宿主
    Shutdown,
}

/// Response produced by one plugin worker
/// 单个插件工作线程产生的响应
pub enum Response {
    /// The step completed; `outputs` are the levels produced at `epoch`
    /// 步进完成；`outputs` 是在 `epoch` 产生的电平
    Stepped {
        /// Epoch echoed from the request / 请求中的 epoch 原样回传
        epoch: u64,
        /// Output pin levels produced by this step / 本次步进产生的输出引脚电平
        outputs: PinSnapshot,
    },
    /// Command accepted / 命令已被接受
    Ack,
    /// Plugin instance replaced / 插件实例已替换
    Reloaded {
        /// Previous instance, owned again by the host / 交还宿主的旧实例
        previous: Box<dyn DevicePlugin>,
    },
    /// Snapshot of the plugin state / 插件状态快照
    State {
        /// Value returned by `state_snapshot()` / `state_snapshot()` 的返回值
        value: serde_json::Value,
    },
    /// Worker stopped and returned its plugin / 工作线程已停止并交还插件
    Shutdown {
        /// Plugin instance owned by the host again / 重新由宿主拥有的插件实例
        plugin: Box<dyn DevicePlugin>,
    },
    /// The command failed (plugin error or panic)
    /// 命令失败（插件错误或 panic）
    Failed {
        /// Epoch of the failed step, `None` for non-step commands
        /// 失败步进的 epoch；非步进命令为 `None`
        epoch: Option<u64>,
        /// Bilingual failure description / 双语失败描述
        message: String,
    },
}

/// Plugin context handed to a plugin running on its own thread
/// 交给运行在独立线程上的插件的上下文
///
/// It exposes the simulation epoch through `current_cycles()` and routes log
/// calls to the host `log` facade. `schedule_callback` records the request (the
/// device mailbox has no timer queue of its own) so a plugin never silently
/// loses it; at most 64 callbacks are remembered.
/// 它通过 `current_cycles()` 暴露仿真 epoch，并把日志转发到宿主 `log` 门面。
/// `schedule_callback` 只**记录**请求（设备邮箱本身没有定时器队列），保证插件不会
/// 静默丢失请求；最多记录 64 条。
#[derive(Debug, Default)]
pub struct WorkerContext {
    /// Simulation epoch of the command being processed / 当前命令的仿真 epoch
    cycles: u64,
    /// Callbacks requested by the plugin: `(due_epoch, tag)` / 插件请求的回调
    callbacks: Vec<(u64, String)>,
}

impl WorkerContext {
    /// Create a context positioned at epoch 0 / 创建位于 epoch 0 的上下文
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Move the context to `cycles` / 将上下文移动到 `cycles`
    pub(crate) fn set_cycles(&mut self, cycles: u64) {
        self.cycles = cycles;
    }

    /// Callbacks recorded so far, in scheduling order / 已记录的回调（按调度顺序）
    #[must_use]
    pub fn pending_callbacks(&self) -> &[(u64, String)] {
        &self.callbacks
    }
}

impl PluginContext for WorkerContext {
    fn log_info(&self, message: &str) {
        log::info!("[plugin-worker] {message}");
    }

    fn log_warn(&self, message: &str) {
        log::warn!("[plugin-worker] {message}");
    }

    fn log_error(&self, message: &str) {
        log::error!("[plugin-worker] {message}");
    }

    fn current_cycles(&self) -> u64 {
        self.cycles
    }

    fn schedule_callback(&mut self, cycles: u64, tag: &str) {
        if self.callbacks.len() < MAX_PENDING_CALLBACKS {
            self.callbacks
                .push((self.cycles.saturating_add(cycles), tag.to_string()));
        }
    }

    fn get_service_erased(&self) -> Option<&dyn Any> {
        // 工作线程没有宿主服务；需要宿主服务的插件应使用同步路径。
        // Worker threads have no host services; plugins needing them use the sync path.
        None
    }
}

/// Deliver a pin snapshot to every wired input/bidirectional port
/// 将引脚快照投递到每个已接线的输入/双向端口
///
/// The same level is delivered on every step, exactly like the synchronous
/// `sync_gpio` path: levels are not merged, dropped or edge-filtered.
/// 每个步进都投递当前电平，与同步 `sync_gpio` 路径完全一致：不合并、不丢弃、
/// 不做边沿过滤。
pub(crate) fn deliver_inputs(
    plugin: &mut dyn DevicePlugin,
    bindings: &[PortBinding],
    inputs: PinSnapshot,
) {
    for binding in bindings {
        if matches!(
            binding.direction,
            PortDirection::Input | PortDirection::Bidirectional
        ) {
            if let Some(pin) = binding.effective_pin() {
                plugin.on_gpio_change(binding.port_index, inputs.get(pin));
            }
        }
    }
}

/// Collect the levels of every wired output/bidirectional port
/// 收集每个已接线的输出/双向端口的电平
#[must_use]
pub(crate) fn collect_outputs(plugin: &dyn DevicePlugin, bindings: &[PortBinding]) -> PinSnapshot {
    let ports = plugin.ports();
    let mut outputs = PinSnapshot::empty();
    for binding in bindings {
        if matches!(
            binding.direction,
            PortDirection::Output | PortDirection::Bidirectional
        ) {
            if let Some(pin) = binding.effective_pin() {
                if let Some(port) = ports.get(binding.port_index) {
                    outputs.set(pin, port.level);
                }
            }
        }
    }
    outputs
}

/// Deliver inputs, tick the plugin and collect outputs (one device step)
/// 投递输入、推进插件并收集输出（单设备一步）
pub(crate) fn step_plugin(
    plugin: &mut dyn DevicePlugin,
    bindings: &[PortBinding],
    epoch: u64,
    inputs: PinSnapshot,
    cycles: u64,
    ctx: &mut WorkerContext,
) -> PinSnapshot {
    ctx.set_cycles(epoch);
    deliver_inputs(plugin, bindings, inputs);
    plugin.tick(ctx, cycles);
    collect_outputs(plugin, bindings)
}

/// Render a panic payload as a stable single-line message
/// 将 panic 载荷渲染为稳定的单行消息
fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "non-string panic payload / 非字符串 panic 载荷".to_string()
    }
}

/// Owned plugin plus the poison flag of one worker
/// 单个工作线程持有的插件与中毒标志
struct WorkerPlugin {
    /// Plugin instance, owned by the worker until shutdown
    /// 插件实例，关闭前由工作线程拥有
    plugin: Option<Box<dyn DevicePlugin>>,
    /// Reason the plugin panicked, if it did / 插件 panic 原因（若已 panic）
    poisoned: Option<String>,
}

impl WorkerPlugin {
    /// Run `f` with panic isolation; on panic remember the reason
    /// 以 panic 隔离执行 `f`；panic 时记录原因
    fn guarded<T>(&mut self, f: impl FnOnce(&mut Box<dyn DevicePlugin>) -> T) -> Result<T, String> {
        let Some(plugin) = self.plugin.as_mut() else {
            return Err("plugin already shut down / 插件已关闭".to_string());
        };
        let outcome = catch_unwind(AssertUnwindSafe(|| f(plugin)));
        match outcome {
            Ok(value) => Ok(value),
            Err(payload) => {
                let message = panic_message(payload.as_ref());
                self.poisoned = Some(message.clone());
                Err(message)
            }
        }
    }

    /// Current poison reason / 当前中毒原因
    fn poison(&self) -> Option<String> {
        self.poisoned.clone()
    }
}

/// Send one response; `false` means the host closed the mailbox
/// 发送一个响应；`false` 表示宿主已关闭邮箱
fn respond(responses: &Sender<Response>, response: Response) -> bool {
    responses.send(response).is_ok()
}

/// Reply to a command that was rejected because the worker is poisoned
/// 对因中毒而被拒绝的命令回包
fn poisoned_response(epoch: Option<u64>, reason: &str) -> Response {
    Response::Failed {
        epoch,
        message: format!("worker poisoned by earlier panic: {reason}"),
    }
}

/// Serve exactly one command; `false` means the worker must stop
/// 处理恰好一条命令；`false` 表示工作线程应停止
fn serve_command(
    id: &str,
    worker: &mut WorkerPlugin,
    bindings: &mut Vec<PortBinding>,
    ctx: &mut WorkerContext,
    responses: &Sender<Response>,
    command: Command,
) -> bool {
    match command {
        Command::Step {
            epoch,
            inputs,
            cycles,
        } => {
            if let Some(reason) = worker.poison() {
                return respond(responses, poisoned_response(Some(epoch), &reason));
            }
            let outcome = worker.guarded(|plugin| {
                step_plugin(plugin.as_mut(), bindings, epoch, inputs, cycles, ctx)
            });
            let response = match outcome {
                Ok(outputs) => Response::Stepped { epoch, outputs },
                Err(message) => Response::Failed {
                    epoch: Some(epoch),
                    message,
                },
            };
            respond(responses, response)
        }
        Command::Configure { config } => {
            if let Some(reason) = worker.poison() {
                return respond(responses, poisoned_response(None, &reason));
            }
            let response = match worker.guarded(|plugin| plugin.apply_config(&config)) {
                Ok(Ok(())) => Response::Ack,
                Ok(Err(error)) => Response::Failed {
                    epoch: None,
                    message: error.to_string(),
                },
                Err(message) => Response::Failed {
                    epoch: None,
                    message,
                },
            };
            respond(responses, response)
        }
        Command::Reload {
            plugin,
            bindings: new_bindings,
        } => {
            let previous = worker.plugin.replace(plugin);
            let mut previous = previous.expect("plugin is present before reload / 重载前插件存在");
            // 旧实例的 shutdown 也可能 panic；忽略其影响，实例仍交还宿主。
            // The old instance may panic in shutdown; ignore it, still hand it back.
            let _ = catch_unwind(AssertUnwindSafe(|| previous.shutdown()));
            worker.poisoned = None;
            *bindings = new_bindings;
            respond(responses, Response::Reloaded { previous })
        }
        Command::SetBindings {
            bindings: new_bindings,
        } => {
            *bindings = new_bindings;
            respond(responses, Response::Ack)
        }
        Command::QueryState => {
            if let Some(reason) = worker.poison() {
                return respond(responses, poisoned_response(None, &reason));
            }
            let response = match worker.guarded(|plugin| plugin.state_snapshot()) {
                Ok(value) => Response::State { value },
                Err(message) => Response::Failed {
                    epoch: None,
                    message,
                },
            };
            respond(responses, response)
        }
        Command::Shutdown => {
            if let Some(mut plugin) = worker.plugin.take() {
                let _ = catch_unwind(AssertUnwindSafe(|| plugin.shutdown()));
                let _ = responses.send(Response::Shutdown { plugin });
            }
            log::debug!("Plugin worker '{id}' stopped / 插件工作线程已停止");
            false
        }
    }
}

/// Worker entry point: serve the mailbox until `Shutdown` or channel close
/// 工作线程入口：服务邮箱直到收到 `Shutdown` 或通道关闭
pub(crate) fn run_mailbox_worker(
    id: &str,
    plugin: Box<dyn DevicePlugin>,
    bindings: Vec<PortBinding>,
    commands: &Receiver<Command>,
    responses: &Sender<Response>,
) {
    let mut worker = WorkerPlugin {
        plugin: Some(plugin),
        poisoned: None,
    };
    let mut bindings = bindings;
    let mut ctx = WorkerContext::new();

    loop {
        // 宿主已释放邮箱：插件在工作线程内释放，随后退出。
        // Host dropped the mailbox: the plugin is dropped here and the worker exits.
        let Ok(command) = commands.recv() else {
            return;
        };
        if !serve_command(id, &mut worker, &mut bindings, &mut ctx, responses, command) {
            return;
        }
    }
}
