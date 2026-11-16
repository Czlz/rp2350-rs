//! Tests for the host audio wiring of the Play tab / 「游戏」标签页宿主音频接线测试
//!
//! These tests drive [`PlayView`](super::PlayView) with the fake worker from
//! [`tests`](super::tests) whose `audio_source()` publishes a silent source, so
//! the output lifecycle (create, stop, drop) is covered without a sound card.
//! 这些测试用 [`tests`](super::tests) 里的假 worker 驱动
//! [`PlayView`](super::PlayView)，其 `audio_source()` 发布静音来源，因此无需声卡
//! 即可覆盖输输出的生命周期（创建、停止、丢弃）。

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rp2350_audio::{AudioFormat, AudioReport, SampleSource};

use super::tests::{FakeShared, temp_firmware, view_with_fake};
use super::*;

/// Build a synthetic report for formatting tests / 为格式化测试构造合成快照
fn report(
    backend: &'static str,
    tap_samples: u64,
    tap_dropped: u64,
    queue_dropped: u64,
    underruns: u64,
    elapsed: Duration,
) -> AudioReport {
    AudioReport {
        backend,
        rate_hz: 13_440,
        channels: 2,
        tap_samples,
        tap_dropped,
        queue_dropped,
        underruns,
        queued: 0,
        elapsed,
    }
}

/// Silent sample source that reports when the worker released it
/// 报告工作线程何时释放自己的静音样本来源
struct SilentSource {
    /// Samples requested so far / 已请求的样本数
    taken: Arc<AtomicU64>,
    /// Set when the audio worker drops the source / 音频线程丢弃来源时置位
    released: Arc<AtomicBool>,
}

impl SilentSource {
    /// Silent 48 kHz stereo source / 静音 48 kHz 立体声来源
    fn new() -> Self {
        Self {
            taken: Arc::new(AtomicU64::new(0)),
            released: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Flag that flips when the worker thread drops the source
    /// 工作线程丢弃来源时翻转的标志
    fn release_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.released)
    }
}

impl SampleSource for SilentSource {
    fn take(&mut self, max: usize) -> Vec<i16> {
        self.taken.fetch_add(max as u64, Ordering::Relaxed);
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
        // 线程结束即丢弃来源，因此该标志为真等价于「音频线程已退出」。
        // The thread drops the source when it ends, so this flag means "exited".
        self.released.store(true, Ordering::Relaxed);
    }
}

/// Start a view whose fake worker publishes a [`SilentSource`] tap
/// 启动假 worker 发布 [`SilentSource`] tap 的视图
fn start_with_audio(enabled: bool, tag: &str) -> (PlayView, std::path::PathBuf, Arc<AtomicBool>) {
    let path = temp_firmware(tag);
    let shared = Arc::new(FakeShared::default());
    let source = SilentSource::new();
    let released = source.release_flag();
    *shared.audio.lock().unwrap() = Some(Box::new(source));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut view = view_with_fake(&shared, &requests);
    view.fields.firmware = path.display().to_string();
    view.fields.frame = String::from("0x20001000:4x4");
    view.fields.steps_per_frame = String::from("200");
    // 假 worker 不看接线，但音频开启时 Start 仍要求给出块与状态机。
    // The fake worker ignores the wiring, but Start still needs a block and SM
    // while audio is on.
    view.fields.audio_block = String::from("0");
    view.fields.audio_sm = String::from("0");
    view.set_audio(enabled);
    view.start();
    (view, path, released)
}

#[test]
fn audio_enabled_starts_a_host_output() {
    let (mut view, path, _released) = start_with_audio(true, "audio-on");
    assert!(view.is_running(), "status={}", view.status());
    assert!(view.audio_enabled());
    let backend = view.audio_backend().expect("host output must exist");
    assert!(
        matches!(backend, "cpal" | "null"),
        "unexpected backend {backend}"
    );
    assert!(view.audio_playing(), "backend={backend}");
    view.stop();
    let _ = std::fs::remove_file(path);
}

#[test]
fn audio_disabled_creates_no_host_output() {
    let (mut view, path, _released) = start_with_audio(false, "audio-off");
    assert!(view.is_running());
    assert!(!view.audio_enabled());
    assert!(view.audio_backend().is_none(), "no output without audio");
    assert!(!view.audio_playing());
    assert_eq!(view.audio_underruns(), 0);
    view.stop();
    let _ = std::fs::remove_file(path);
}

#[test]
fn stop_joins_the_audio_worker_within_two_seconds() {
    let (mut view, path, released) = start_with_audio(true, "audio-stop");
    assert!(view.audio_playing());
    assert!(!released.load(Ordering::Relaxed));

    let started = Instant::now();
    view.stop();
    let elapsed = started.elapsed();

    assert!(elapsed < Duration::from_secs(2), "stop took {elapsed:?}");
    assert!(view.audio_backend().is_none(), "output must be released");
    assert!(!view.audio_playing());
    assert!(
        released.load(Ordering::Relaxed),
        "the audio thread must exit before stop returns"
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn audio_status_line_names_backend_and_underruns() {
    assert_eq!(format_audio(Some("cpal"), 3), "audio=cpal underruns=3");
    let silent = format_audio(Some("null"), 0);
    assert_eq!(silent, "audio=null underruns=0");
    let off = format_audio(None, 0);
    assert_eq!(off, "audio=off underruns=0");
    for line in [silent, off] {
        assert!(line.contains("audio="), "{line}");
        assert!(line.contains("underruns="), "{line}");
    }
}

#[test]
fn silent_backend_notice_is_shown_once() {
    let mut view = PlayView::default();
    view.note_silent_backend("null");
    view.note_silent_backend("null");
    view.note_silent_backend("null");
    assert_eq!(
        view.status().matches("无声卡").count(),
        1,
        "status={}",
        view.status()
    );

    // 真实后端不提示。 / A real backend shows no notice.
    let mut hardware = PlayView::default();
    hardware.note_silent_backend("cpal");
    assert!(
        !hardware.status().contains("无声卡"),
        "{}",
        hardware.status()
    );
}

#[test]
fn dropping_a_view_with_audio_joins_the_worker() {
    let (view, path, released) = start_with_audio(true, "audio-drop");
    assert!(view.audio_playing());
    drop(view);
    assert!(
        released.load(Ordering::Relaxed),
        "Drop must join the audio worker"
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn audio_status_line_reports_all_counters() {
    // 1 s of 48 kHz stereo production is healthy; drops are still reported.
    // 1 秒 48 kHz 立体声生产是健康的；丢样仍然要显示。
    let mut healthy = report("cpal", 96_000, 3, 4, 0, Duration::from_secs(1));
    healthy.rate_hz = 48_000;
    let line = format_audio_report(Some(&healthy), None);
    assert!(
        line.starts_with("audio=cpal tap_samples=96000 dropped=7 underruns=0"),
        "line={line}"
    );
    assert!(line.contains("produced=48000/s"), "line={line}");
    assert!(line.contains("need=48000/s"), "line={line}");
    assert!(!line.contains("slower than real time"), "line={line}");

    let off = format_audio_report(None, None);
    assert_eq!(off, "audio=off underruns=0 tap_samples=0 dropped=0");
    for field in ["audio=", "tap_samples=", "dropped=", "underruns="] {
        assert!(off.contains(field), "off={off}");
    }
}

#[test]
fn slow_production_shows_the_realtime_notice() {
    // 448 frames/s for 5 s on a 13 440 Hz source: the emerald case.
    // 13 440 Hz 来源上 5 秒只有 448 帧/秒：即 emerald 的现状。
    let starved = report("cpal", 4_480, 0, 0, 500, Duration::from_secs(5));
    let line = format_audio_report(Some(&starved), None);
    assert!(line.contains("produced=448/s"), "line={line}");
    assert!(line.contains("need=13440/s"), "line={line}");
    assert!(
        line.contains(
            "audio: emulation slower than real time (underruns=500, produced=448/s, need=13440/s)"
        ),
        "line={line}"
    );
    // 明确不伪造声音：同一行仍报告真实的 underrun 数。
    // No fake audio: the same line still reports the real underrun count.
    assert!(line.contains("underruns=500"), "line={line}");
}

#[test]
fn audio_diagnostic_has_the_stable_fields() {
    let starved = report("null", 4_480, 2, 5, 500, Duration::from_secs(5));
    let line = format_audio_diagnostic(&starved, 448.0);
    assert!(
        line.starts_with(
            "[rp2350-egui] audio backend=null tap_samples=4480 dropped=7 underruns=500"
        ),
        "line={line}"
    );
    assert!(line.contains("produced=448/s"), "line={line}");
    assert!(line.contains("need=13440/s"), "line={line}");
    assert!(line.contains("elapsed_s=5.0"), "line={line}");

    // 空 tap 必须提示接线检查，而不是只报 produced=0。
    // An empty tap must point at the wiring instead of only reporting 0.
    let empty = report("cpal", 0, 0, 0, 500, Duration::from_secs(5));
    let hint = format_audio_diagnostic(&empty, 0.0);
    assert!(hint.contains("tap_samples=0"), "hint={hint}");
    assert!(hint.contains("audio_block/audio_sm"), "hint={hint}");
    assert!(hint.contains("audio init"), "hint={hint}");
}

#[test]
fn first_sample_line_reports_when_capture_starts() {
    let first = report("cpal", 896, 0, 0, 0, Duration::from_secs(48));
    let line = format_audio_first_sample(&first);
    assert!(
        line.starts_with("[rp2350-egui] audio first-sample backend=cpal tap_samples=896"),
        "line={line}"
    );
    assert!(line.contains("elapsed_s=48.0"), "line={line}");
    assert!(line.contains("need=13440/s"), "line={line}");
}

#[test]
fn live_output_report_feeds_the_status_line() {
    let (mut view, path, _released) = start_with_audio(true, "audio-report");
    let report = view
        .audio_report()
        .expect("host output must publish a report");
    assert_eq!(report.backend, view.audio_backend().expect("backend"));
    assert_eq!(report.rate_hz, 48_000);
    assert_eq!(report.channels, 2);
    assert!(report.elapsed > Duration::ZERO);
    let line = view.audio_status_line();
    for field in [
        "audio=",
        "tap_samples=",
        "dropped=",
        "underruns=",
        "produced=",
        "need=",
    ] {
        assert!(line.contains(field), "line={line}");
    }
    view.stop();
    assert_eq!(
        view.audio_status_line(),
        "audio=off underruns=0 tap_samples=0 dropped=0"
    );
    assert!(view.audio_report().is_none());
    assert_eq!(
        view.audio_reported, 0,
        "a new session restarts the diagnostics"
    );
    let _ = std::fs::remove_file(path);
}
