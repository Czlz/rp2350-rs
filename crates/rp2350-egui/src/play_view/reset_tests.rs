//! Tests for the Play tab Reset button / 「游戏」标签页「复位」按钮测试
//!
//! The tests drive [`PlayView::reset`] with a fake worker: the starter records
//! every [`PlayRequest`], the source counts shutdowns and can publish a silent
//! audio tap, so the whole lifecycle (stop old, boot new, re-create audio) is
//! covered without a window or a real guest.
//! 这些测试用假 worker 驱动 [`PlayView::reset`]：starter 记录每个
//! [`PlayRequest`]，来源统计 shutdown 次数并可发布静音音频 tap，因此无需窗口或
//! 真实客户机即可覆盖完整生命周期（停旧、启新、重建音频）。

use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};

use rp2350_audio::{AudioFormat, SampleSource};
use rp2350_cli::run::SYSTEM_RESET_MESSAGE;

use egui::{Pos2, RawInput, Rect, Vec2, vec2};

use super::reset::SYSTEM_RESET_HALT;
use super::tests::{FAKE_FRAME, temp_firmware};
use super::*;

/// Fake worker state shared with the test / 与测试共享的假 worker 状态
#[derive(Default)]
struct FakeShared {
    /// Frame handed to the view / 交给视图的帧
    frame: Mutex<Option<Arc<PlayFrame>>>,
    /// Statistics handed to the view / 交给视图的统计
    stats: Mutex<PlayStats>,
    /// Whether the worker still runs / worker 是否仍在运行
    running: AtomicBool,
    /// Number of shutdowns / shutdown 次数
    shutdowns: AtomicU16,
    /// Audio tap handed to the view once per start / 每次启动交给视图一次的音频 tap
    audio: Mutex<Option<Box<dyn SampleSource>>>,
}

/// Source backed by [`FakeShared`] / 由 [`FakeShared`] 支撑的来源
struct FakeSource {
    shared: Arc<FakeShared>,
}

impl PlaySource for FakeSource {
    fn latest_frame(&self) -> Option<Arc<PlayFrame>> {
        self.shared.frame.lock().unwrap().clone()
    }

    fn set_buttons(&self, _mask: u16) {}

    fn stats(&self) -> PlayStats {
        self.shared.stats.lock().unwrap().clone()
    }

    fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::Relaxed)
    }

    fn button_pins(&self) -> u16 {
        0
    }

    fn audio_source(&self) -> Option<Box<dyn SampleSource>> {
        self.shared.audio.lock().unwrap().take()
    }

    fn shutdown(self: Box<Self>) -> PlayStats {
        self.shared.running.store(false, Ordering::Relaxed);
        self.shared.shutdowns.fetch_add(1, Ordering::Relaxed);
        self.stats()
    }
}

/// Start one [`FakeSource`] and reset its statistics like a fresh worker
/// 启动一个 [`FakeSource`]，并像新 worker 那样清零统计
fn boot(shared: &Arc<FakeShared>) -> Box<dyn PlaySource> {
    shared.running.store(true, Ordering::Relaxed);
    *shared.stats.lock().unwrap() = PlayStats::default();
    Box::new(FakeSource {
        shared: Arc::clone(shared),
    })
}

/// View whose starter records requests and boots fake workers
/// 启动器记录请求并启动假 worker 的视图
fn view_with_fake(shared: &Arc<FakeShared>, requests: &Arc<Mutex<Vec<PlayRequest>>>) -> PlayView {
    let source = Arc::clone(shared);
    let recorded = Arc::clone(requests);
    PlayView::default().with_starter(Box::new(move |request: PlayRequest| {
        recorded.lock().unwrap().push(request);
        Ok(boot(&source))
    }))
}

/// Fill the minimum fields Start/Reset needs / 填好 Start/Reset 所需的最少字段
fn ready(view: &mut PlayView, firmware: &std::path::Path) {
    view.fields.firmware = firmware.display().to_string();
    view.fields.frame = FAKE_FRAME.to_owned();
    view.fields.steps_per_frame = String::from("200");
    view.fields.buttons = String::from("0x3FF");
}

/// Sample `4 × 4` frame with `index` / 指定序号的 4×4 示例帧
fn frame(index: u64) -> Arc<PlayFrame> {
    Arc::new(PlayFrame {
        index,
        steps: index * 200,
        width: 4,
        height: 4,
        rgb565: vec![0xF800; 16],
        readable: true,
    })
}

/// Raw input of `size` carrying `events` / 携带 `events` 的 `size` 原始输入
fn raw_input(size: Vec2, events: Vec<egui::Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
        events,
        ..RawInput::default()
    }
}

/// Render the view headlessly at `size` / 在 `size` 下无窗口渲染视图
fn render(context: &egui::Context, view: &mut PlayView, size: Vec2) -> usize {
    let output = context.run_ui(raw_input(size, Vec::new()), |ui| view.ui(ui));
    let shapes = output.shapes.len();
    output.drop_without_applying_deltas();
    shapes
}

/// Primary-button event at `pos` / `pos` 处的主键事件
fn pointer_button(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn reset_is_available_only_with_a_rebuildable_session() {
    // CLI 的停机消息必须以 GUI 匹配的前缀开头，否则「复位」会在客机请求后失效。
    // The CLI halt message must start with the prefix the GUI matches, otherwise
    // Reset would disappear after a guest request.
    assert!(
        SYSTEM_RESET_MESSAGE.starts_with(SYSTEM_RESET_HALT),
        "GUI prefix {SYSTEM_RESET_HALT:?} drifted from {SYSTEM_RESET_MESSAGE:?}"
    );

    let path = temp_firmware("reset-available");
    let shared = Arc::new(FakeShared::default());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut view = view_with_fake(&shared, &requests);

    // 空闲视图没有可重建的会话：Reset 只写状态行，不调用 starter。
    // An idle view has nothing to rebuild: Reset only writes the status line.
    assert!(!view.can_reset());
    view.reset();
    assert!(view.status().contains("reset unavailable / 无法复位"));
    assert!(
        requests.lock().unwrap().is_empty(),
        "idle Reset must not boot"
    );
    assert_eq!(view.reset_count(), 0);

    ready(&mut view, &path);
    view.start();
    assert!(view.can_reset(), "status={}", view.status());

    // 客机请求 AIRCR.SYSRESETREQ：worker 停下，但 Reset 必须保持可用。
    // A guest AIRCR.SYSRESETREQ stops the worker, but Reset must stay available.
    shared.running.store(false, Ordering::Relaxed);
    shared.stats.lock().unwrap().halted = Some(SYSTEM_RESET_MESSAGE.to_owned());
    assert!(view.can_reset(), "a system-reset halt is a rebuild request");

    view.reset();
    assert!(view.is_running());
    assert_eq!(view.reset_count(), 1);
    assert_eq!(requests.lock().unwrap().len(), 2);

    view.stop();
    assert!(!view.is_running());
    assert!(!view.can_reset(), "a normal stop leaves nothing to rebuild");
    let _ = std::fs::remove_file(path);
}

#[test]
fn reset_boots_a_second_session_with_identical_fields() {
    let path = temp_firmware("reset-fields");
    let shared = Arc::new(FakeShared::default());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut view = view_with_fake(&shared, &requests);
    ready(&mut view, &path);
    view.start();

    let fields_before = view.fields().clone();
    assert_eq!(requests.lock().unwrap().len(), 1);

    view.reset();

    let recorded = requests.lock().unwrap();
    assert_eq!(
        recorded.len(),
        2,
        "old session shutdown + new session start"
    );
    assert_eq!(recorded[0], recorded[1], "the fields must be forwarded 1:1");
    assert_eq!(recorded[1].image, b"not a real uf2");
    assert_eq!(recorded[1].frame, FAKE_FRAME);
    assert_eq!(recorded[1].steps_per_frame, 200);
    assert_eq!(recorded[1].button_pins, GBA_BUTTONS);
    drop(recorded);

    assert_eq!(shared.shutdowns.load(Ordering::Relaxed), 1);
    assert_eq!(
        view.fields(),
        &fields_before,
        "Reset must not touch the text fields"
    );
    assert!(view.is_running());
    view.stop();
    let _ = std::fs::remove_file(path);
}

#[test]
fn reset_restarts_frame_numbering_and_counts_resets() {
    let path = temp_firmware("reset-frames");
    let shared = Arc::new(FakeShared::default());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut view = view_with_fake(&shared, &requests);
    ready(&mut view, &path);
    view.start();

    *shared.frame.lock().unwrap() = Some(frame(9));
    *shared.stats.lock().unwrap() = PlayStats {
        frames: 9,
        steps: 1_800,
        fps: 12.0,
        steps_per_second: 2_400.0,
        replaced: 0,
        halted: None,
    };
    let context = egui::Context::default();
    render(&context, &mut view, vec2(320.0, 240.0));
    assert_eq!(view.last_frame, 9);
    assert_eq!(view.uploads(), 1);

    view.reset();
    assert_eq!(view.reset_count(), 1);
    assert!(
        view.status().contains("reset / 已复位"),
        "status={}",
        view.status()
    );
    assert!(
        view.status().contains("reset count=1"),
        "status={}",
        view.status()
    );
    // 新会话：纹理、统计与帧计数都从头开始。
    // New session: texture, statistics and frame counter start over.
    assert_eq!(view.last_frame, 0);
    assert_eq!(view.stats, PlayStats::default());
    assert!(view.texture.is_none());

    // 新会话重新发布第 1 帧，状态栏跟随新会话的 fps/steps。
    // The new session publishes frame 1 again; the status bar follows it.
    *shared.frame.lock().unwrap() = Some(frame(1));
    *shared.stats.lock().unwrap() = PlayStats {
        frames: 1,
        steps: 200,
        fps: 60.0,
        steps_per_second: 12_000.0,
        replaced: 0,
        halted: None,
    };
    render(&context, &mut view, vec2(320.0, 240.0));
    assert_eq!(view.last_frame, 1);
    assert_eq!(view.uploads(), 2);
    let line = format_stats(&view.stats, view.last_frame, true);
    assert!(line.contains("frame=1"), "{line}");
    assert!(line.contains("steps=200"), "{line}");
    assert!(line.contains("fps=60.00"), "{line}");

    view.reset();
    assert_eq!(view.reset_count(), 2);
    view.stop();
    let _ = std::fs::remove_file(path);
}

/// Silent sample source that reports when the audio thread released it
/// 报告音频线程何时释放自己的静音样本来源
struct SilentSource {
    /// Set when the audio worker drops the source / 音频线程丢弃来源时置位
    released: Arc<AtomicBool>,
}

impl SilentSource {
    /// Silent 48 kHz stereo source / 静音 48 kHz 立体声来源
    fn new() -> Self {
        Self {
            released: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Flag that flips when the audio thread drops the source
    /// 音频线程丢弃来源时翻转的标志
    fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.released)
    }
}

impl SampleSource for SilentSource {
    fn take(&mut self, max: usize) -> Vec<i16> {
        vec![0; max]
    }

    fn format(&self) -> AudioFormat {
        AudioFormat::new(48_000, 2)
    }

    fn dropped(&self) -> u64 {
        0
    }
}

impl Drop for SilentSource {
    fn drop(&mut self) {
        self.released.store(true, Ordering::Relaxed);
    }
}

#[test]
fn reset_joins_the_old_audio_output_and_rebuilds_it() {
    let path = temp_firmware("reset-audio");
    let shared = Arc::new(FakeShared::default());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let releases: Arc<Mutex<Vec<Arc<AtomicBool>>>> = Arc::new(Mutex::new(Vec::new()));
    let source = Arc::clone(&shared);
    let recorded = Arc::clone(&requests);
    let flags = Arc::clone(&releases);
    let mut view = PlayView::default().with_starter(Box::new(move |request: PlayRequest| {
        recorded.lock().unwrap().push(request);
        let silent = SilentSource::new();
        flags.lock().unwrap().push(silent.flag());
        *source.audio.lock().unwrap() = Some(Box::new(silent));
        Ok(boot(&source))
    }));
    ready(&mut view, &path);
    view.fields.audio_block = String::from("0");
    view.fields.audio_sm = String::from("0");
    view.set_audio(true);
    view.start();
    assert!(view.audio_playing(), "status={}", view.status());
    let first = {
        let flags = releases.lock().unwrap();
        assert_eq!(flags.len(), 1);
        Arc::clone(&flags[0])
    };
    assert!(!first.load(Ordering::Relaxed));

    view.reset();

    assert!(
        first.load(Ordering::Relaxed),
        "the old audio thread must be joined before the new session"
    );
    assert!(view.audio_playing(), "Reset must re-create the host output");
    assert!(view.audio_backend().is_some());
    let second = {
        let flags = releases.lock().unwrap();
        assert_eq!(flags.len(), 2);
        Arc::clone(&flags[1])
    };
    assert!(
        !second.load(Ordering::Relaxed),
        "the new output must still run"
    );

    view.stop();
    assert!(
        second.load(Ordering::Relaxed),
        "Stop must join the re-created output"
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn reset_failure_reports_the_error_and_stops_the_old_session() {
    let path = temp_firmware("reset-fail");
    let shared = Arc::new(FakeShared::default());
    let attempts = Arc::new(AtomicU16::new(0));
    let source = Arc::clone(&shared);
    let calls = Arc::clone(&attempts);
    let mut view = PlayView::default().with_starter(Box::new(move |_request: PlayRequest| {
        if calls.fetch_add(1, Ordering::Relaxed) == 0 {
            Ok(boot(&source))
        } else {
            Err("machine boot failed / 机器启动失败".to_owned())
        }
    }));
    ready(&mut view, &path);
    view.start();
    assert!(view.is_running());

    view.reset();

    assert!(!view.is_running());
    assert!(
        view.status().contains("reset failed / 复位失败"),
        "status={}",
        view.status()
    );
    assert!(
        view.status().contains("machine boot failed"),
        "status={}",
        view.status()
    );
    assert_eq!(
        shared.shutdowns.load(Ordering::Relaxed),
        1,
        "the old session must be joined before the new start"
    );
    assert_eq!(view.reset_count(), 0, "a failed reset is not counted");
    assert!(!view.can_reset());

    // 再次 Reset 只写状态行，不 panic。
    // A second Reset only writes the status line and never panics.
    view.reset();
    assert!(view.status().contains("reset unavailable / 无法复位"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn small_window_reset_click_does_not_panic() {
    let path = temp_firmware("reset-small");
    let shared = Arc::new(FakeShared::default());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut view = view_with_fake(&shared, &requests);
    ready(&mut view, &path);
    view.start();

    // 80×60 下整页渲染两遍：状态栏与画面都被裁到 1 px，但不得 panic。
    // Render the whole page twice at 80×60: the picture and status bar are
    // clamped to 1 px, and nothing may panic.
    let context = egui::Context::default();
    render(&context, &mut view, vec2(80.0, 60.0));
    render(&context, &mut view, vec2(80.0, 60.0));

    // 单独渲染 Start/Stop/Reset 行，取出第三个控件（Reset）的真实矩形，再用
    // 指针按下/抬起触发它——与用户点击走同一条 `clicked()` 路径。
    // Render the control row alone, take the third widget (Reset), then press
    // and release the pointer over it: the same `clicked()` path a user takes.
    let size = vec2(320.0, 240.0);
    let output = context.run_ui(raw_input(size, Vec::new()), |ui| {
        view.controls(ui);
    });
    output.drop_without_applying_deltas();
    let rects = context.interactive_rects_last_pass();
    assert!(rects.len() >= 3, "rects={rects:?}");
    let target = rects[2].center();

    let press = vec![
        egui::Event::PointerMoved(target),
        pointer_button(target, true),
    ];
    let output = context.run_ui(raw_input(size, press), |ui| {
        view.controls(ui);
    });
    output.drop_without_applying_deltas();
    let output = context.run_ui(raw_input(size, vec![pointer_button(target, false)]), |ui| {
        view.controls(ui);
    });
    output.drop_without_applying_deltas();

    assert_eq!(
        view.reset_count(),
        1,
        "clicking Reset must rebuild the session, status={}",
        view.status()
    );
    assert!(view.is_running());
    render(&context, &mut view, vec2(80.0, 60.0));
    view.stop();
    let _ = std::fs::remove_file(path);
}
