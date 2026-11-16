//! Worker handle — mailbox, timeouts, panic latch and graceful shutdown
//! 工作线程句柄 — 邮箱、超时、panic 锁定与优雅关闭

// 类型名沿用项目命名约定；文档引用使用内部标识符。
// Type names follow project conventions; doc references use internal identifiers.
#![allow(clippy::doc_markdown, clippy::module_name_repetitions)]

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::device::{DevicePlugin, DevicePort};
use super::executor::{ExecutorConfig, ExecutorError, PinSnapshot, PortBinding};
use super::executor_worker::{Command, Response, run_mailbox_worker};
use super::manager::DeviceId;

/// Handle to one plugin worker thread / 单个插件工作线程的句柄
///
/// Owns the mailbox and joins the worker in [`DeviceExecutor::shutdown`] or on
/// drop. All waits are bounded by [`ExecutorConfig::step_timeout`].
/// 拥有邮箱，并在 [`DeviceExecutor::shutdown`] 或析构时回收工作线程。所有等待都受
/// [`ExecutorConfig::step_timeout`] 限制。
pub struct DeviceExecutor {
    /// Device ID used in diagnostics / 诊断用设备 ID
    id: DeviceId,
    /// Command channel to the worker / 发往工作线程的命令通道
    commands: Sender<Command>,
    /// Response channel from the worker / 来自工作线程的响应通道
    responses: Receiver<Response>,
    /// Worker handle; `None` after shutdown or spawn failure / 工作线程句柄
    handle: Option<JoinHandle<()>>,
    /// Port table captured at spawn / 启动时捕获的端口表
    ports: Vec<DevicePort>,
    /// Configuration / 配置
    config: ExecutorConfig,
    /// Latch reason once the device became high-Z / 高阻锁定原因
    degraded: Option<String>,
    /// Last epoch stepped by this executor / 本执行器步进过的最后 epoch
    last_epoch: Option<u64>,
    /// Successful steps / 成功步进次数
    steps: u64,
    /// Timed-out round trips / 超时往返次数
    timeouts: u64,
    /// Worker was detached and its library must stay mapped / 工作线程已分离
    detached: bool,
}

impl DeviceExecutor {
    /// Spawn a worker thread that takes ownership of `plugin`
    /// 启动工作线程并移交 `plugin` 的所有权
    ///
    /// A spawn failure does not panic: the executor is created stopped with a
    /// diagnostic, and every command reports [`ExecutorError::Stopped`].
    /// 启动失败不会 panic：执行器以“已停止”状态创建并带诊断，所有命令返回
    /// [`ExecutorError::Stopped`]。
    #[must_use]
    pub fn spawn(
        id: DeviceId,
        plugin: Box<dyn DevicePlugin>,
        ports: Vec<DevicePort>,
        bindings: Vec<PortBinding>,
        config: ExecutorConfig,
    ) -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (response_tx, responses) = mpsc::channel();
        let worker_id = id.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("rp2350-plugin-{id}"))
            .spawn(move || {
                run_mailbox_worker(&worker_id, plugin, bindings, &command_rx, &response_tx);
            });
        let (handle, degraded) = match spawned {
            Ok(handle) => (Some(handle), None),
            Err(error) => {
                let reason = format!("failed to spawn worker thread: {error}");
                log::warn!("plugin '{id}' degraded to high-Z: {reason} / 插件降级为高阻");
                (None, Some(reason))
            }
        };
        Self {
            id,
            commands,
            responses,
            handle,
            ports,
            config,
            degraded,
            last_epoch: None,
            steps: 0,
            timeouts: 0,
            detached: false,
        }
    }

    /// Device ID / 设备 ID
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Port table captured at spawn / 启动时捕获的端口表
    #[must_use]
    pub fn ports(&self) -> &[DevicePort] {
        &self.ports
    }

    /// Configuration / 配置
    #[must_use]
    pub fn config(&self) -> ExecutorConfig {
        self.config
    }

    /// Is the worker still owned and running? / 工作线程是否仍被持有并运行？
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.handle.is_some()
    }

    /// Is the device latched high-Z? / 设备是否已锁定为高阻？
    #[must_use]
    pub fn is_degraded(&self) -> bool {
        self.degraded.is_some()
    }

    /// Stable degradation reason / 稳定的降级原因
    #[must_use]
    pub fn degraded_reason(&self) -> Option<&str> {
        self.degraded.as_deref()
    }

    /// Was the worker detached during shutdown (thread cannot be joined)?
    /// 关闭时工作线程是否被分离（无法 join）？
    #[must_use]
    pub fn is_detached(&self) -> bool {
        self.detached
    }

    /// Successful steps served / 已完成的成功步进数
    #[must_use]
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// Timed-out round trips / 超时的往返次数
    #[must_use]
    pub fn timeouts(&self) -> u64 {
        self.timeouts
    }

    /// Replace the mailbox round-trip bound / 替换邮箱往返上限
    ///
    /// Applies to the next command. A device already latched high-Z stays
    /// high-Z until [`DeviceExecutor::reload`] replaces the plugin.
    /// 从下一条命令开始生效；已锁定为高阻的设备在 [`DeviceExecutor::reload`] 替换
    /// 插件前保持高阻。
    pub fn set_step_timeout(&mut self, timeout: Duration) {
        self.config.step_timeout = timeout;
    }

    /// Replace the binding table and push it to the worker
    /// 替换绑定表并推送到工作线程
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError`] when the device is high-Z or the worker is gone.
    pub fn set_bindings(&mut self, bindings: Vec<PortBinding>) -> Result<(), ExecutorError> {
        self.ensure_healthy()?;
        let response = self.request(Command::SetBindings { bindings }, "set_bindings")?;
        match response {
            Response::Ack => Ok(()),
            other => Err(self.protocol_error("set_bindings", &other)),
        }
    }

    /// Step the plugin by one epoch and return the levels it produces
    /// 推进插件一个 epoch 并返回其产生的电平
    ///
    /// `inputs` is the pin snapshot sampled at `epoch`; the returned levels are
    /// published by the caller at `epoch + latency`.
    /// `inputs` 是在 `epoch` 采样的引脚快照；返回电平由调用方在 `epoch + latency`
    /// 发布。
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError`] when the device is high-Z, the epoch does not
    /// increase, the worker times out, the plugin panics, or the mailbox is
    /// closed. 设备高阻、epoch 未递增、工作线程超时、插件 panic 或邮箱关闭时返回
    /// [`ExecutorError`]。
    pub fn step(
        &mut self,
        epoch: u64,
        inputs: PinSnapshot,
        cycles: u64,
    ) -> Result<PinSnapshot, ExecutorError> {
        self.submit_step(epoch, inputs, cycles)?;
        self.collect_step(epoch)
    }

    /// Submit a step to the worker without waiting for its response
    /// 向工作线程提交步进，但不等待响应
    ///
    /// The manager uses this to dispatch every device before collecting any
    /// response, which is what makes independent devices run in parallel. The
    /// caller **must** follow up with [`DeviceExecutor::collect_step`] for the
    /// same epoch before sending another command.
    /// 管理器用它先向所有设备派发、再统一收集响应，从而让相互独立的设备真正并行。
    /// 调用方必须在发送下一条命令前对同一 epoch 调用
    /// [`DeviceExecutor::collect_step`]。
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError`] when the device is high-Z, the epoch does not
    /// increase, or the mailbox is closed.
    pub fn submit_step(
        &mut self,
        epoch: u64,
        inputs: PinSnapshot,
        cycles: u64,
    ) -> Result<(), ExecutorError> {
        self.ensure_healthy()?;
        if let Some(last) = self.last_epoch {
            if epoch <= last {
                return Err(ExecutorError::Protocol {
                    id: self.id.clone(),
                    op: "step (epoch not increasing)",
                });
            }
        }
        self.last_epoch = Some(epoch);
        self.send_command(
            Command::Step {
                epoch,
                inputs,
                cycles,
            },
            "step",
        )
    }

    /// Wait for the response of a step submitted with [`DeviceExecutor::submit_step`]
    /// 等待 [`DeviceExecutor::submit_step`] 所提交步进的响应
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError`] on timeout, plugin panic, protocol mismatch or
    /// when the worker is gone.
    pub fn collect_step(&mut self, epoch: u64) -> Result<PinSnapshot, ExecutorError> {
        let response = self.await_response("step")?;
        match response {
            Response::Stepped {
                epoch: replied,
                outputs,
            } if replied == epoch => {
                self.steps += 1;
                Ok(outputs)
            }
            Response::Failed {
                epoch: Some(_),
                message,
            } => {
                self.degrade(format!("plugin panic: {message}"));
                Err(ExecutorError::Panic {
                    id: self.id.clone(),
                    message,
                })
            }
            other => Err(self.protocol_error("step", &other)),
        }
    }

    /// Apply a JSON configuration on the worker thread
    /// 在工作线程上应用 JSON 配置
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError`] when the device is high-Z, the worker is gone,
    /// or the plugin rejects the configuration.
    pub fn configure(&mut self, config: &serde_json::Value) -> Result<(), ExecutorError> {
        self.ensure_healthy()?;
        let response = self.request(
            Command::Configure {
                config: config.clone(),
            },
            "configure",
        )?;
        match response {
            Response::Ack => Ok(()),
            Response::Failed { message, .. } => Err(ExecutorError::Plugin {
                id: self.id.clone(),
                op: "configure",
                message,
            }),
            other => Err(self.protocol_error("configure", &other)),
        }
    }

    /// Replace the plugin instance; returns the previous box to the host
    /// 替换插件实例；把旧实例交还宿主
    ///
    /// The previous instance is already stopped and is safe to drop once the
    /// call returns. 旧实例已经停止，调用返回后即可安全释放。
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError`] when the worker is gone or does not answer.
    pub fn reload(
        &mut self,
        plugin: Box<dyn DevicePlugin>,
        ports: Vec<DevicePort>,
        bindings: Vec<PortBinding>,
    ) -> Result<Box<dyn DevicePlugin>, ExecutorError> {
        if self.handle.is_none() {
            return Err(ExecutorError::Stopped {
                id: self.id.clone(),
            });
        }
        let response = self.request(Command::Reload { plugin, bindings }, "reload")?;
        match response {
            Response::Reloaded { previous } => {
                self.ports = ports;
                self.degraded = None;
                self.last_epoch = None;
                Ok(previous)
            }
            other => Err(self.protocol_error("reload", &other)),
        }
    }

    /// Query `state_snapshot()` through the mailbox / 通过邮箱查询 `state_snapshot()`
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError`] when the device is high-Z, the worker is gone,
    /// or the plugin panics in `state_snapshot`.
    pub fn query_state(&mut self) -> Result<serde_json::Value, ExecutorError> {
        self.ensure_healthy()?;
        let response = self.request(Command::QueryState, "query_state")?;
        match response {
            Response::State { value } => Ok(value),
            Response::Failed { message, .. } => {
                self.degrade(format!("plugin panic: {message}"));
                Err(ExecutorError::Panic {
                    id: self.id.clone(),
                    message,
                })
            }
            other => Err(self.protocol_error("query_state", &other)),
        }
    }

    /// Stop the worker and hand the plugin back to the host
    /// 停止工作线程并把插件交还宿主
    ///
    /// Returns `None` when the worker was already stopped or never answered
    /// within the shutdown deadline; a non-answering worker is **detached**
    /// (`is_detached()`), because Rust cannot kill a thread.
    /// 工作线程已停止、或在关闭期限内未响应时返回 `None`；未响应的线程会被
    /// **分离**（`is_detached()`），因为 Rust 无法终止线程。
    pub fn shutdown(&mut self) -> Option<Box<dyn DevicePlugin>> {
        let handle = self.handle.take()?;
        let _ = self.commands.send(Command::Shutdown);
        let deadline = Instant::now() + self.shutdown_timeout();
        let mut plugin = None;
        let mut answered = false;
        loop {
            let now = Instant::now();
            let Some(remaining) = deadline.checked_duration_since(now) else {
                break;
            };
            match self.responses.recv_timeout(remaining) {
                Ok(Response::Shutdown { plugin: owned }) => {
                    plugin = Some(owned);
                    answered = true;
                    break;
                }
                // 忽略超时步进遗留的响应，继续等待 Shutdown。
                // Ignore stale responses from a timed-out step and keep waiting.
                Ok(_) => {}
                Err(_) => break,
            }
        }
        if answered {
            let _ = handle.join();
        } else {
            self.detached = true;
            log::warn!(
                "plugin '{}' did not stop within {} ms; worker detached / \
                 插件未在期限内停止，工作线程已分离",
                self.id,
                self.shutdown_timeout().as_millis()
            );
            drop(handle);
        }
        plugin
    }

    /// Bound of one graceful shutdown / 优雅关闭的期限
    ///
    /// A device that already timed out is presumed stuck, so its shutdown wait
    /// is short. 已超时的设备视为卡死，因此关闭等待较短。
    fn shutdown_timeout(&self) -> Duration {
        if self.degraded.is_some() {
            return Duration::from_millis(100);
        }
        self.config.step_timeout.max(Duration::from_millis(100))
    }

    /// Fail fast when the device is high-Z or has no worker
    /// 设备高阻或没有工作线程时立即失败
    fn ensure_healthy(&self) -> Result<(), ExecutorError> {
        if let Some(reason) = &self.degraded {
            return Err(ExecutorError::Degraded {
                id: self.id.clone(),
                reason: reason.clone(),
            });
        }
        if self.handle.is_none() {
            return Err(ExecutorError::Stopped {
                id: self.id.clone(),
            });
        }
        Ok(())
    }

    /// Latch the device high-Z and log one stable diagnostic line
    /// 将设备锁定为高阻并记录一条稳定诊断
    fn degrade(&mut self, reason: String) {
        if self.degraded.is_some() {
            return;
        }
        log::warn!(
            "plugin '{}' degraded to high-Z: {} / 插件降级为高阻",
            self.id,
            reason
        );
        self.degraded = Some(reason);
    }

    /// Send one command and wait for exactly one response
    /// 发送一条命令并等待恰好一个响应
    fn request(&mut self, command: Command, op: &'static str) -> Result<Response, ExecutorError> {
        self.send_command(command, op)?;
        self.await_response(op)
    }

    /// Send one command without waiting / 发送一条命令但不等待
    fn send_command(&mut self, command: Command, op: &'static str) -> Result<(), ExecutorError> {
        if self.commands.send(command).is_err() {
            self.degrade(format!("{op} failed: mailbox closed"));
            return Err(ExecutorError::Disconnected {
                id: self.id.clone(),
            });
        }
        Ok(())
    }

    /// Wait for one response, bounded by `step_timeout` / 等待一个响应（受超时限制）
    fn await_response(&mut self, op: &'static str) -> Result<Response, ExecutorError> {
        match self.responses.recv_timeout(self.config.step_timeout) {
            Ok(response) => Ok(response),
            Err(RecvTimeoutError::Timeout) => {
                self.timeouts += 1;
                let timeout_ms = self.config.step_timeout.as_millis();
                self.degrade(format!(
                    "{op} timed out after {timeout_ms} ms; device is high-Z"
                ));
                Err(ExecutorError::Timeout {
                    id: self.id.clone(),
                    op,
                    timeout_ms,
                })
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.degrade(format!("{op} failed: worker exited"));
                Err(ExecutorError::Disconnected {
                    id: self.id.clone(),
                })
            }
        }
    }

    /// Turn an unexpected response into a protocol error / 将非预期响应转为协议错误
    fn protocol_error(&mut self, op: &'static str, _response: &Response) -> ExecutorError {
        self.degrade(format!("{op} returned an unexpected response"));
        ExecutorError::Protocol {
            id: self.id.clone(),
            op,
        }
    }
}

impl Drop for DeviceExecutor {
    fn drop(&mut self) {
        // 释放插件 box 时库仍由管理器持有，顺序安全。
        // The plugin box is dropped while the manager still owns the library.
        let _ = self.shutdown();
    }
}
