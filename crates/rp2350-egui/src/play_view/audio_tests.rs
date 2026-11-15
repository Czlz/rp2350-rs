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

use rp2350_audio::{AudioFormat, SampleSource};

use super::tests::{FakeShared, temp_firmware, view_with_fake};
use super::*;

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
