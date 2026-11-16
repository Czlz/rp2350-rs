//! Core-executor switch of the Play tab / 「游戏」标签页的核执行器开关
//!
//! The mode is part of the host request, not of the guest: it is fixed when a
//! session starts and cannot change while the worker runs. Keeping the widgets
//! and the guarded setters here leaves [`PlayView`](super::PlayView) room for the
//! rest of the toolbar and keeps this file inside the project's line budget.
//! 该模式属于宿主请求而不是客户机：会话启动时固定，worker 运行期间不能改变。把
//! 控件与带锁的 setter 放在这里，既给 [`PlayView`](super::PlayView) 的其余工具栏
//! 留出空间，也让本文件保持在项目的行数预算内。

use super::PlayView;

impl PlayView {
    /// Whether the `Threads / 线程` box is ticked / 是否勾选「Threads / 线程」
    #[must_use]
    pub fn core_threads_enabled(&self) -> bool {
        self.fields.core_threads
    }

    /// Tick or untick the threaded core executor / 勾选或取消线程化核执行器
    ///
    /// The mode is fixed per session: the call is ignored while a worker runs, so
    /// the checkbox can never disagree with the worker that is already playing.
    /// 该模式按会话固定：worker 运行中调用会被忽略，因此复选框不会与已在播放的
    /// worker 冲突。
    pub fn set_core_threads(&mut self, enabled: bool) {
        if !self.is_running() {
            self.fields.core_threads = enabled;
        }
    }

    /// Cross-core window text / 跨核窗口文本
    #[must_use]
    pub fn core_window(&self) -> &str {
        &self.fields.core_window
    }

    /// Set the cross-core window text / 设置跨核窗口文本
    ///
    /// Fixed per session, like [`PlayView::set_core_threads`].
    /// 与 [`PlayView::set_core_threads`] 一样按会话固定。
    pub fn set_core_window(&mut self, window: &str) {
        if !self.is_running() {
            window.clone_into(&mut self.fields.core_window);
        }
    }

    /// Toolbar row of the core-executor switch / 核执行器开关的工具栏行
    ///
    /// Sits inside the toolbar's disabled region, so a running session locks both
    /// widgets without extra state.
    /// 位于工具栏的禁用区域内，因此运行中的会话无需额外状态即可锁定两个控件。
    pub(super) fn core_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.fields.core_threads, "Threads / 线程")
                .on_hover_text(
                    "core0/core1 on separate host threads; not bit-reproducible \
                     inside a window / 双核各用一条宿主线程；窗口内不可位复现",
                );
            ui.label("Window / 窗口");
            ui.add(egui::TextEdit::singleline(&mut self.fields.core_window).desired_width(50.0));
            ui.label("1..=4096");
        });
    }
}
