//! Types shared by the Play tab: frames, statistics, requests and traits
//! 「游戏」标签页共享类型：帧、统计、请求与 trait
//!
//! The view, the CLI adapter and the tests all speak these types. Keeping them
//! in one module leaves [`PlayView`](super::PlayView) room for the widgets and
//! keeps every file inside the project's 500-code-line cap.
//! 视图、CLI 适配器与测试都使用这些类型。把它们集中在一个模块里，
//! [`PlayView`](super::PlayView) 才有空间实现界面，同时每个文件都在项目的 500 行
//! 代码上限之内。

use std::sync::Arc;

/// One guest frame handed to the Play tab / 交给「游戏」标签页的一帧客户机画面
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayFrame {
    /// 1-based frame counter / 从 1 开始的帧序号
    pub index: u64,
    /// Guest steps executed when the frame was captured / 抓取该帧时的客户机步数
    pub steps: u64,
    /// Pixels per row / 每行像素数
    pub width: u16,
    /// Number of rows / 行数
    pub height: u16,
    /// Row-major RGB565 pixels / 行优先 RGB565 像素
    pub rgb565: Vec<u16>,
    /// Whether the whole guest frame buffer was readable / 客户机帧缓冲是否完整可读
    pub readable: bool,
}

/// Progress snapshot displayed in the status bar / 状态栏显示的进度快照
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayStats {
    /// Frames published so far / 已发布帧数
    pub frames: u64,
    /// Guest steps executed so far / 已执行客户机步数
    pub steps: u64,
    /// Rolling wall-clock frames per second / 滚动窗口的每秒帧数
    pub fps: f64,
    /// Rolling wall-clock steps per second / 滚动窗口的每秒步数
    pub steps_per_second: f64,
    /// Frames replaced before the GUI took them / 未被 GUI 取走即被替换的帧数
    pub replaced: u64,
    /// Why the worker stopped early, if it did / worker 提前停止的原因（若有）
    pub halted: Option<String>,
}

/// Host audio wiring typed by the user, `None` when capture is off
/// 用户输入的宿主音频接线；不捕获时为 `None`
///
/// Every field is guest-specific, so the view has no default for them: they come
/// from the toolbar fields or from a profile file.
/// 每个字段都是客户机专用的，因此视图不提供默认值：它们来自工具栏输入或 profile
/// 文件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioRequest {
    /// PIO block that pushes I2S words / 推送 I2S 字的 PIO 块索引
    pub block: usize,
    /// State machine index inside that block / 该块内的状态机索引
    pub sm: usize,
    /// Sample rate written into the audio sink / 写入音频 sink 的采样率
    pub rate_hz: u32,
    /// Channel count written into the audio sink / 写入音频 sink 的声道数
    pub channels: u8,
    /// Samples the sink retains / sink 保留的样本数
    pub capacity: usize,
}

/// Request sent to a [`PlayStarter`] / 发送给 [`PlayStarter`] 的启动请求
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayRequest {
    /// Firmware image bytes / 固件映像字节
    pub image: Vec<u8>,
    /// Frame buffer text (`ADDR:WxH[:STRIDE]`) / 帧缓冲文本
    pub frame: String,
    /// Steps executed per published frame / 每发布一帧执行的步数
    pub steps_per_frame: u64,
    /// Audio wiring to capture, or `None` to leave the PIO FIFOs untouched
    /// 要捕获的音频接线；`None` 表示完全不碰 PIO FIFO
    pub audio: Option<AudioRequest>,
    /// Button pin bitmap the worker drives / worker 驱动的按键引脚位图
    pub button_pins: u16,
    /// Whether core0/core1 run on separate host threads
    /// 是否让 core0/core1 运行在独立宿主线程上
    ///
    /// `false` (the default) is the deterministic single-threaded interleaving;
    /// `true` is the interactive threaded executor and is not bit-reproducible
    /// inside one cross-core window.
    /// `false`（默认）是确定性的单线程交错；`true` 是交互式线程执行器，在单个跨核
    /// 窗口内不可位复现。
    pub core_threads: bool,
    /// Machine steps per cross-core window / 每个跨核窗口的机器步数
    pub core_window: u32,
}

/// Running play worker as seen by the GUI / GUI 视角的运行中 worker
pub trait PlaySource {
    /// Newest published frame, or `None` before the first one
    /// 最新发布的帧；首帧之前为 `None`
    fn latest_frame(&self) -> Option<Arc<PlayFrame>>;

    /// Write the button mask (bit set = pressed = pin low)
    /// 写入按键掩码（置位 = 按下 = 引脚低电平）
    fn set_buttons(&self, mask: u16);

    /// Statistics snapshot / 统计快照
    fn stats(&self) -> PlayStats;

    /// Whether the worker thread still runs / worker 线程是否仍在运行
    fn is_running(&self) -> bool;

    /// Button pins this worker drives / 本 worker 驱动的按键引脚
    fn button_pins(&self) -> u16;

    /// Owned audio tap the host output should pull from, if the worker captured
    /// I2S audio / worker 捕获了 I2S 音频时，交给宿主输出的音频 tap
    ///
    /// The default is `None` (no audio); workers that can capture audio return
    /// a boxed source, which [`PlayView`](super::PlayView) moves into a host
    /// output.
    /// 默认返回 `None`（无音频）；能捕获音频的 worker 返回装箱来源，由
    /// [`PlayView`](super::PlayView) 移入宿主输出。
    fn audio_source(&self) -> Option<Box<dyn rp2350_audio::SampleSource>> {
        None
    }

    /// Stop the worker and return the final statistics / 停止 worker 并返回最终统计
    fn shutdown(self: Box<Self>) -> PlayStats;
}

/// Factory that boots a [`PlaySource`] from a [`PlayRequest`]
/// 用 [`PlayRequest`] 启动 [`PlaySource`] 的工厂
///
/// The host implements this with `rp2350_cli::play::PlaySession::start`; the
/// ready-made adapter is [`CliPlayStarter`](super::CliPlayStarter), which the
/// native entry point attaches. Alternate hosts and tests may still inject their
/// own worker.
/// 宿主用 `rp2350_cli::play::PlaySession::start` 实现它；现成适配器为
/// [`CliPlayStarter`](super::CliPlayStarter)，由原生入口挂载。其它宿主与测试仍可
/// 注入自己的 worker。
pub trait PlayStarter {
    /// Boot a worker / 启动 worker
    ///
    /// # Errors
    /// Returns the message to show in the status bar when the firmware cannot
    /// be booted or the worker cannot be spawned.
    /// 固件无法启动或 worker 无法创建时返回要显示在状态栏的消息。
    fn start(&self, request: PlayRequest) -> Result<Box<dyn PlaySource>, String>;
}

impl<F> PlayStarter for F
where
    F: Fn(PlayRequest) -> Result<Box<dyn PlaySource>, String>,
{
    fn start(&self, request: PlayRequest) -> Result<Box<dyn PlaySource>, String> {
        self(request)
    }
}

/// Progress line of the status bar / 状态栏的进度行
#[must_use]
pub fn format_stats(stats: &PlayStats, frame: u64, running: bool) -> String {
    let phase = if running {
        "running / 运行中"
    } else {
        "stopped / 已停止"
    };
    let mut text = format!(
        "frame={frame} steps={} fps={:.2} steps/s={:.0} replaced={} · {phase}",
        stats.steps, stats.fps, stats.steps_per_second, stats.replaced
    );
    if let Some(halted) = &stats.halted {
        text.push_str(" · halted / 停机: ");
        text.push_str(halted);
    }
    text
}

/// Status line of the core-executor switch / 核执行器开关的状态行
///
/// `threads=on` adds the non-determinism marker required by the interactive
/// mode; `threads=off` keeps the deterministic default unadorned. An empty
/// window text renders as `win=?` (validation rejects it before a start).
/// `threads=on` 追加交互模式所必需的非确定性标记；`threads=off` 保持确定性默认且
/// 不加标记。窗口文本为空时渲染为 `win=?`（启动前的校验会拒绝它）。
#[must_use]
pub fn format_core_threads(enabled: bool, window: &str) -> String {
    let window = window.trim();
    let window = if window.is_empty() { "?" } else { window };
    let mode = if enabled { "on" } else { "off" };
    let mut text = format!("threads={mode} win={window}");
    if enabled {
        text.push_str(" · non-deterministic within window / 窗口内非确定性");
    }
    text
}
