//! Tests for the core-executor switch of the Play tab
//! 「游戏」标签页核执行器开关测试
//!
//! The status line is a pure function, so its exact text is pinned without a
//! window; the rendering and lock tests reuse the fake worker from [`tests`].
//! 状态行是纯函数，其精确文本无需窗口即可锁定；渲染与锁定测试复用 [`tests`] 的
//! 假 worker。

use std::sync::Arc;

use egui::vec2;

use super::format_core_threads;
use super::tests::{FakeShared, frame, run, view_with_source};

#[test]
fn format_core_threads_reports_mode_and_window() {
    assert_eq!(format_core_threads(false, "64"), "threads=off win=64");
    let on = format_core_threads(true, "32");
    assert!(on.starts_with("threads=on win=32"), "{on}");
    assert!(on.contains("non-deterministic"), "{on}");
    assert_eq!(
        format_core_threads(false, " "),
        "threads=off win=?",
        "an empty window is shown as unknown, never as a made-up number"
    );
}

#[test]
fn threaded_view_renders_at_eighty_by_sixty() {
    let shared = Arc::new(FakeShared::default());
    *shared.frame.lock().unwrap() = Some(frame(1, 240, 160));
    let mut view = view_with_source(&shared);
    view.fields.core_threads = true;
    view.fields.core_window = String::from("32");
    let context = egui::Context::default();
    let shapes = run(&context, &mut view, vec2(80.0, 60.0), Vec::new());
    assert!(shapes > 0, "the threaded toolbar still paints");
}

#[test]
fn core_switch_is_locked_while_running() {
    let shared = Arc::new(FakeShared::default());
    let mut view = view_with_source(&shared);
    view.set_core_threads(true);
    view.set_core_window("32");
    assert!(
        !view.core_threads_enabled(),
        "a running session keeps its executor mode"
    );
    assert_eq!(view.core_window(), "64");

    view.stop();
    view.set_core_threads(true);
    view.set_core_window("32");
    assert!(
        view.core_threads_enabled(),
        "an idle view accepts the switch"
    );
    assert_eq!(view.core_window(), "32");
}
