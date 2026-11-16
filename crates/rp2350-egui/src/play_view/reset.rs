//! Start/reset lifecycle of the Play tab / 「游戏」标签页的启动与复位流程
//!
//! [`PlayView::start`] validates the toolbar fields and boots a worker;
//! [`PlayView::reset`] throws the current worker away and boots a fresh one from
//! the **same** fields, which is the GUI equivalent of "Stop + Start" and the
//! host-side answer to a guest `AIRCR.SYSRESETREQ`. Both live here so the widget
//! code in [`super`] stays inside the project's per-file line budget.
//! [`PlayView::start`] 校验工具栏字段并启动 worker；[`PlayView::reset`] 丢弃当前
//! worker 并用**相同**字段重建一个，等价于「停止 + 启动」，也是 GUI 对客户机
//! `AIRCR.SYSRESETREQ` 的宿主响应。两者都放在本模块，使 [`super`] 的控件代码保持
//! 在项目的单文件行数上限内。
//!
//! Reset is deliberately **not** a save state: the guest RAM, peripherals and
//! clock tree are rebuilt from the firmware image, so progress is lost. The
//! fields, profile and audio configuration survive because they describe the
//! host request, not the guest.
//! 复位**不是**存档：客户机 RAM、外设与时钟树都从固件映像重建，进度会丢失。字段、
//! profile 与音频配置保留，因为它们描述的是宿主请求而不是客户机状态。

use super::{PlayRequest, PlayStats, PlayView, format_core_threads};

/// Status line when no [`PlayStarter`](super::PlayStarter) is attached
/// 未挂载 [`PlayStarter`](super::PlayStarter) 时的状态行文本
pub(super) const NO_STARTER: &str =
    "no play source attached / 未挂载游玩来源（宿主需实现 PlayStarter）";

/// Prefix of the halt reason reported for `AIRCR.SYSRESETREQ`
/// 针对 `AIRCR.SYSRESETREQ` 报告的停机原因前缀
///
/// [`PlayStats::halted`] is free text, so the view can only inspect its prefix;
/// the value mirrors `rp2350_cli::run::SYSTEM_RESET_MESSAGE` and a test fails if
/// the two literals drift apart.
/// [`PlayStats::halted`] 是自由文本，视图只能检查其前缀；该值与
/// `rp2350_cli::run::SYSTEM_RESET_MESSAGE` 一致，字面量漂移会让测试失败。
pub(super) const SYSTEM_RESET_HALT: &str = "system reset requested";

/// Whether a statistics snapshot reports a guest system-reset halt
/// 统计快照是否报告了客户机系统复位导致的停机
fn halted_by_system_reset(stats: &PlayStats) -> bool {
    stats
        .halted
        .as_deref()
        .is_some_and(|reason| reason.starts_with(SYSTEM_RESET_HALT))
}

impl PlayView {
    /// Whether the Reset button can rebuild a session / 「复位」按钮是否可重建会话
    ///
    /// True while a worker runs, and also while a worker stopped because the
    /// guest requested `AIRCR.SYSRESETREQ`: that halt is a *request for a host
    /// action*, not a final state, so Reset must stay available.
    /// 运行中为真；worker 因客户机请求 `AIRCR.SYSRESETREQ` 而停止时也为真：这种停机
    /// 是需要宿主动作的**请求**而不是终态，因此「复位」必须保持可用。
    #[must_use]
    pub fn can_reset(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.is_running() || halted_by_system_reset(&session.stats()))
    }

    /// Number of successful resets since the view was created
    /// 视图创建以来成功复位的次数
    ///
    /// Monotonic across Stop/Start: it counts host reset actions, not sessions.
    /// 跨 Stop/Start 单调递增：它统计宿主复位动作，而不是会话数量。
    #[must_use]
    pub fn reset_count(&self) -> u32 {
        self.reset_count
    }

    /// Boot a worker from the toolbar fields / 用工具栏字段启动 worker
    ///
    /// Validates the text fields ([`PlayFields::inputs`]), reads the firmware
    /// file, then hands the request to the attached starter. Every failure is
    /// written to the toolbar status: this method never panics and never blocks
    /// the UI thread on the guest.
    /// 先校验文本字段（[`PlayFields::inputs`]）、读取固件文件，再把请求交给已挂载的
    /// 工厂。所有失败都写入工具栏状态：本方法不 panic，也不会在 UI 线程上阻塞等待
    /// 客户机。
    ///
    /// [`PlayFields::inputs`]: super::PlayFields::inputs
    pub fn start(&mut self) {
        let Some(request) = self.boot_request() else {
            return;
        };
        let Some(starter) = self.starter.as_ref() else {
            NO_STARTER.clone_into(&mut self.status);
            return;
        };
        let frame = request.frame.clone();
        let steps = request.steps_per_frame;
        let threads = format_core_threads(request.core_threads, &request.core_window.to_string());
        match starter.start(request) {
            Ok(session) => {
                // 状态行先就位，`attach` 里的音频静默提示才不会被覆盖。
                // Set the status line first so the audio notice from `attach`
                // is not overwritten.
                self.status = format!(
                    "running / 运行中 · {frame} · steps/frame / 每帧步数 {steps} · {threads}"
                );
                self.attach(session);
            }
            Err(message) => self.status = format!("start failed / 启动失败: {message}"),
        }
    }

    /// Rebuild the session from the current toolbar fields / 按当前工具栏字段重建会话
    ///
    /// The order is fixed and observable: validate the fields and re-read the
    /// firmware, join the host audio thread, shut down and join the old worker,
    /// then boot a new one. On success the reset counter grows, the new session
    /// starts at frame 1 and the host audio output is re-created when `Audio /
    /// 音频` is ticked; the text fields are never touched (Stop/Start keep them
    /// too). A starter error leaves no session and writes a bilingual status
    /// line — the call never panics.
    /// 顺序固定且可观察：校验字段并重读固件 → join 宿主音频线程 → 关闭并 join 旧
    /// worker → 启动新 worker。成功后复位计数递增、新会话从第 1 帧开始，勾选
    /// 「Audio / 音频」时重建宿主音频输出；文本字段从不改动（Stop/Start 同样保留）。
    /// starter 报错时不留会话并写入双语状态行——本方法不 panic。
    ///
    /// Validation failures (missing field, unreadable firmware, no starter) keep
    /// the running session alive: a typo must not kill a running game.
    /// 校验失败（缺字段、固件不可读、无 starter）会保持运行中的会话：笔误不该杀死
    /// 正在运行的游戏。
    pub fn reset(&mut self) {
        if !self.can_reset() {
            "reset unavailable / 无法复位：没有可重建的会话".clone_into(&mut self.status);
            return;
        }
        let Some(request) = self.boot_request() else {
            return;
        };
        if self.starter.is_none() {
            NO_STARTER.clone_into(&mut self.status);
            return;
        }
        let frame = request.frame.clone();
        let steps = request.steps_per_frame;
        let threads = format_core_threads(request.core_threads, &request.core_window.to_string());
        // 先停音频（join），再关旧会话（join）：与 stop/Drop 的顺序一致。
        // Stop audio (join) first, then the old worker (join), like stop/Drop.
        self.stop_audio();
        if let Some(session) = self.session.take() {
            let _ = session.shutdown();
        }
        let started = match self.starter.as_ref() {
            Some(starter) => starter.start(request),
            None => Err(NO_STARTER.to_owned()),
        };
        match started {
            Ok(session) => {
                self.reset_count = self.reset_count.saturating_add(1);
                self.status = format!(
                    "reset / 已复位 · reset count={} · {frame} · steps/frame / 每帧步数 {steps} · {threads}",
                    self.reset_count
                );
                self.attach(session);
            }
            Err(message) => self.status = format!("reset failed / 复位失败: {message}"),
        }
    }

    /// Validate the fields and read the firmware into a boot request
    /// 校验字段并把固件读入启动请求
    ///
    /// Writes the validation or read error to the status line and returns
    /// `None`, so both Start and Reset share one failure path.
    /// 校验或读取失败时把错误写入状态行并返回 `None`，因此 Start 与 Reset 共用同一条
    /// 失败路径。
    fn boot_request(&mut self) -> Option<PlayRequest> {
        let inputs = match self.fields.inputs() {
            Ok(inputs) => inputs,
            Err(message) => {
                self.status = message;
                return None;
            }
        };
        let image = match std::fs::read(&inputs.firmware) {
            Ok(image) => image,
            Err(error) => {
                self.status = format!("cannot read firmware / 无法读取固件: {error}");
                return None;
            }
        };
        Some(PlayRequest {
            image,
            frame: inputs.frame,
            steps_per_frame: inputs.steps_per_frame,
            audio: inputs.audio,
            button_pins: inputs.buttons,
            core_threads: inputs.core_threads,
            core_window: inputs.core_window,
        })
    }
}
