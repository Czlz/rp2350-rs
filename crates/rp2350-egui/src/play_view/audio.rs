//! Host audio output for the Play tab / 「游戏」标签页的宿主音频输出
//!
//! The view holds the worker's I2S tap through
//! [`PlaySource::audio_source`](super::PlaySource::audio_source) and
//! hands it to [`rp2350_audio::AudioOutput`] here. Everything that needs the
//! audio crate lives in this module, so [`PlayView`](super::PlayView) keeps
//! talking to traits only and tests may inject a synthetic source.
//! 视图通过 [`PlaySource::audio_source`](super::PlaySource::audio_source) 取走
//! worker 的 I2S tap，并在本模块交给 [`rp2350_audio::AudioOutput`]。所有需要音频
//! crate 的代码都在这里，因此 [`PlayView`](super::PlayView) 仍只与 trait 交互，
//! 测试也可注入合成来源。
//!
//! Playback is **opt-in at compile time**: `rp2350-audio` compiles without `cpal`
//! unless the `audio-playback` feature of this crate is enabled. Without it the
//! null backend consumes samples on the wall clock and stays silent, so a
//! sound-card-less host or CI still exercises the whole path.
//! 播放是**编译期可选**的：除非启用本 crate 的 `audio-playback` feature，
//! `rp2350-audio` 不会编译 `cpal`。此时 null 后端按墙钟消费样本并保持静默，
//! 因此无声卡宿主或 CI 仍会走完整条路径。

use std::fmt::Write as _;
use std::time::Duration;

use rp2350_audio::{AudioOutput, AudioReport, SampleSource};

/// Queue capacity in samples: 1 s of 48 kHz stereo audio
/// 队列容量（样本数）：48 kHz 立体声 1 秒
pub const AUDIO_CAPACITY: usize = 96_000;

/// Backend name of the silent fallback / 静默降级后端的名称
pub const AUDIO_NULL: &str = "null";

/// One-time notice appended to the toolbar status for [`AUDIO_NULL`]
/// 使用 [`AUDIO_NULL`] 时向工具栏状态追加一次的提示
pub const SILENT_NOTICE: &str = " · 无声卡，静默运行 / no sound card, silent";

/// Wall-clock points at which a stable audio diagnostic line is written
/// 写入稳定音频诊断行的墙钟时间点
pub const AUDIO_DIAGNOSTIC_TIMES: [Duration; 2] = [Duration::from_secs(5), Duration::from_secs(15)];

/// Production shortfall (frames/s) before the UI says "slower than real time"
/// 低于目标多少帧/秒时提示「慢于实时」
pub const SLOW_TOLERANCE_FPS: f64 = 1.0;

/// Adapter that lets [`AudioOutput`] pull from a boxed source
/// 让 [`AudioOutput`] 能从装箱来源取样本的适配器
///
/// [`AudioOutput::start`] consumes its source, while a [`PlaySource`] can only
/// hand out an owned box, so this thin owner moves the box into the worker.
/// [`AudioOutput::start`] 按值消费来源，而 [`PlaySource`] 只能交出装箱值，因此用
/// 这个轻量所有者把箱子移进工作线程。
///
/// [`PlaySource`]: super::PlaySource
struct BoxedSource(Box<dyn SampleSource>);

impl SampleSource for BoxedSource {
    fn take(&mut self, max: usize) -> Vec<i16> {
        self.0.take(max)
    }

    fn format(&self) -> rp2350_audio::AudioFormat {
        self.0.format()
    }

    fn dropped(&self) -> u64 {
        self.0.dropped()
    }
}

/// Start host playback for `source` / 为 `source` 启动宿主播放
///
/// Never fails and never panics: when no backend opens, the null backend takes
/// over and [`rp2350_audio::AudioOutput::backend`] reports
/// [`AUDIO_NULL`]. The call waits ≤ 1 s for the backend probe, so the backend
/// name is valid on return.
/// 永不失败、永不 panic：无法打开后端时由 null 后端接管，
/// [`rp2350_audio::AudioOutput::backend`] 返回 [`AUDIO_NULL`]。返回前最多等待 1 s
/// 完成探测，因此返回后后端名已有效。
#[must_use]
pub fn start(source: Box<dyn SampleSource>) -> AudioOutput {
    AudioOutput::start(BoxedSource(source), AUDIO_CAPACITY)
}

/// Compact audio line used by older hosts / 旧宿主使用的紧凑音频行
///
/// `backend` is the name reported by the running output, or `None` while no
/// output exists (`audio=off`). The two fields keep a simple `grep audio=`
/// probe working; the GUI status bar itself uses [`format_audio_report`].
/// `backend` 为运行中输出报告的名称，无输出时为 `None`（`audio=off`）。这两个字段
/// 保持简单的 `grep audio=` 探针可用；GUI 状态栏本身使用 [`format_audio_report`]。
#[must_use]
pub fn format_audio(backend: Option<&str>, underruns: u64) -> String {
    backend.map_or_else(
        || "audio=off underruns=0".to_owned(),
        |name| format!("audio={name} underruns={underruns}"),
    )
}

/// Stable stderr line printed at 5 s and 15 s / 5 秒与 15 秒时打印的稳定 stderr 行
///
/// The first four fields are the acceptance contract:
/// `backend`, `tap_samples`, `dropped`, `underruns`. `produced` is the window
/// rate (frames/s since the previous diagnostic) and `need` is `rate_hz`, so a
/// slow emulator is visible without reading the GUI.
/// 前四个字段是验收契约：`backend`、`tap_samples`、`dropped`、`underruns`。
/// `produced` 是窗口生产帧率（相对上一条诊断的增量），`need` 即 `rate_hz`，因此
/// 不打开 GUI 也能看出模拟器是否慢于实时。
#[must_use]
pub fn format_audio_diagnostic(report: &AudioReport, produced_fps: f64) -> String {
    let mut line = format!(
        "[rp2350-egui] audio backend={} tap_samples={} dropped={} underruns={} \
         produced={produced_fps:.0}/s need={}/s elapsed_s={:.1}",
        report.backend,
        report.tap_samples,
        report.dropped(),
        report.underruns,
        report.rate_hz,
        report.elapsed.as_secs_f64()
    );
    if report.tap_samples == 0 {
        // 没装到正确的块/状态机时 tap 永远为空；慢启动也可能尚未到达音频初始化。
        // 给出可操作的提示，不猜测具体块号。
        // A tap on the wrong block/state machine stays empty forever; a slow
        // boot may also not have reached audio initialisation yet. Give an
        // actionable hint without guessing the firmware's block number.
        line.push_str(
            " note=\"tap received no samples yet; check audio_block/audio_sm or wait for the \
             guest to reach audio init\"",
        );
    }
    line
}

/// Stable stderr line written when the tap first delivers samples
/// tap 首次交付样本时写入的稳定 stderr 行
///
/// A slow guest can take minutes to reach its audio initialisation, so this line
/// makes the transition observable (and proves the capture wiring works) even
/// when the 5 s/15 s diagnostics were still empty.
/// 慢速客户机可能需要几分钟才到达音频初始化，因此用该行把这一刻变得可观察（并证明
/// 捕获接线有效），即使 5 秒/15 秒诊断时仍为空。
#[must_use]
pub fn format_audio_first_sample(report: &AudioReport) -> String {
    format!(
        "[rp2350-egui] audio first-sample backend={} tap_samples={} elapsed_s={:.1} \
         produced={:.0}/s need={}/s",
        report.backend,
        report.tap_samples,
        report.elapsed.as_secs_f64(),
        report.frames_per_second(),
        report.rate_hz
    )
}

/// Audio line of the status bar / 状态栏的音频行
///
/// Always names `backend`, `tap_samples`, `dropped` and `underruns`. When
/// production is below the source rate the line additionally carries the
/// explicit `audio: emulation slower than real time (…)` notice; the host stays
/// silent rather than looping stale samples, so this text is the only audible
/// diagnostic.
/// 始终包含 `backend`、`tap_samples`、`dropped` 与 `underruns`。生产帧率低于来源
/// 采样率时额外给出明确的 `audio: emulation slower than real time (…)` 提示；
/// 宿主保持静音而不是循环旧样本，因此这段文字就是唯一可用的诊断。
#[must_use]
pub fn format_audio_report(report: Option<&AudioReport>, produced_fps: Option<f64>) -> String {
    let Some(report) = report else {
        return "audio=off underruns=0 tap_samples=0 dropped=0".to_owned();
    };
    let produced = produced_fps.unwrap_or_else(|| report.frames_per_second());
    let mut line = format!(
        "audio={} tap_samples={} dropped={} underruns={} produced={produced:.0}/s need={}/s",
        report.backend,
        report.tap_samples,
        report.dropped(),
        report.underruns,
        report.rate_hz
    );
    if report.slower_than_real_time(SLOW_TOLERANCE_FPS) {
        // 写入固定的 String 不会失败；忽略 `fmt::Result` 即可。
        // Writing to a `String` cannot fail; the `fmt::Result` is ignored.
        let _ = write!(
            line,
            " · audio: emulation slower than real time (underruns={}, produced={produced:.0}/s, \
             need={}/s)",
            report.underruns, report.rate_hz
        );
    }
    line
}

impl super::PlayView {
    /// Snapshot of the running host audio output, if any
    /// 运行中的宿主音频输出快照（若有）
    #[must_use]
    pub fn audio_report(&self) -> Option<AudioReport> {
        self.audio_output.as_ref().map(AudioOutput::report)
    }

    /// Audio line of the status bar / 状态栏的音频行
    #[must_use]
    pub fn audio_status_line(&self) -> String {
        let report = self.audio_report();
        format_audio_report(report.as_ref(), self.audio_produced_fps)
    }

    /// Emit the 5 s and 15 s stderr diagnostics at most once each
    /// 最多各输出一次 5 秒与 15 秒的 stderr 诊断
    ///
    /// Called from [`super::PlayView::ui`] on every repaint. The production rate
    /// is the `tap_samples` delta over the wall-clock delta since the previous
    /// line, so a long boot or a stalled guest cannot be hidden by the
    /// cumulative average.
    /// 每次重绘由 [`super::PlayView::ui`] 调用。生产速率取相对上一条诊断的
    /// `tap_samples` 增量除以墙钟增量，因此长启动或客户机停摆不会被累计平均值掩盖。
    pub(super) fn pump_audio_diagnostics(&mut self) {
        let Some(report) = self.audio_report() else {
            return;
        };
        self.audio_produced_fps = Some(report.frames_per_second());
        // 首次收到样本单独记一行：慢启动下 5 s/15 s 诊断可能都还是 0。
        // Log the first captured sample separately: on a slow boot both the 5 s
        // and 15 s diagnostics can still be empty.
        if report.tap_samples > 0 && !self.audio_first_sample_reported {
            self.audio_first_sample_reported = true;
            eprintln!("{}", format_audio_first_sample(&report));
        }
        while self.audio_reported < AUDIO_DIAGNOSTIC_TIMES.len()
            && report.elapsed >= AUDIO_DIAGNOSTIC_TIMES[self.audio_reported]
        {
            let produced = match self.audio_previous {
                Some(previous) if report.elapsed > previous.elapsed => {
                    report.frames_per_second_since(&previous)
                }
                _ => report.frames_per_second(),
            };
            eprintln!("{}", format_audio_diagnostic(&report, produced));
            self.audio_reported += 1;
            self.audio_previous = Some(report);
        }
    }
}
