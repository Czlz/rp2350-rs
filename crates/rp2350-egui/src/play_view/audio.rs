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

use rp2350_audio::{AudioOutput, SampleSource};

/// Queue capacity in samples: 1 s of 48 kHz stereo audio
/// 队列容量（样本数）：48 kHz 立体声 1 秒
pub const AUDIO_CAPACITY: usize = 96_000;

/// Backend name of the silent fallback / 静默降级后端的名称
pub const AUDIO_NULL: &str = "null";

/// One-time notice appended to the toolbar status for [`AUDIO_NULL`]
/// 使用 [`AUDIO_NULL`] 时向工具栏状态追加一次的提示
pub const SILENT_NOTICE: &str = " · 无声卡，静默运行 / no sound card, silent";

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

/// Audio line of the status bar / 状态栏的音频行
///
/// `backend` is the name reported by the running output, or `None` while no
/// output exists (`audio=off`). The line always carries both fields so a UI
/// probe (`grep audio=`) shows whether the host is playing.
/// `backend` 为运行中输出报告的名称，无输出时为 `None`（`audio=off`）。该行始终
/// 带两个字段，便于用 `grep audio=` 判断宿主是否在播放。
#[must_use]
pub fn format_audio(backend: Option<&str>, underruns: u64) -> String {
    backend.map_or_else(
        || "audio=off underruns=0".to_owned(),
        |name| format!("audio={name} underruns={underruns}"),
    )
}
