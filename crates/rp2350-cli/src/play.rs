//! Background play worker: run `Machine`, publish frames, accept buttons
//! 后台游玩 worker：运行 `Machine`、发布帧、接收按键
//!
//! [`PlaySession`] owns one worker thread. The worker builds its own
//! [`Machine`] from the image bytes because the machine is not `Send`, advances
//! it by a fixed number of steps per frame, copies the guest frame buffer and
//! publishes the newest frame as an `Arc<Frame>`. Publishing never blocks: an
//! unconsumed frame is replaced (and counted). Input is a `u16` mask written by
//! the host; a set bit means "pressed", which pulls that GPIO pin low (the
//! firmware enables internal pull-ups, so released is high).
//! [`PlaySession`] 持有一个 worker 线程。由于机器不是 `Send`，worker 用映像字节
//! 自行构造 [`Machine`]，每帧推进固定步数，拷贝客户机帧缓冲，并把最新一帧以
//! `Arc<Frame>` 发布。发布永不阻塞：未被取走的帧会被替换（并计数）。输入是宿主
//! 写入的 `u16` 掩码，置位表示「按下」，即把对应 GPIO 拉低（固件启用内部上拉，
//! 因此松开为高电平）。

// 统计与几何换算在 u64/usize/f64 之间转换，这是帧率与像素格式统计的固有需求。
// Statistics and geometry convert between u64/usize/f64, inherent to frame-rate
// and pixel-format reporting.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

mod frame;
#[cfg(test)]
mod tests;

pub use frame::{Frame, FrameSpec, SharedFrame};

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use rp2350_periph::{AudioSink, AudioTap};

use crate::run::{ArchArg, Machine, MachineOptions};

/// Button pins driven by default: GP0..GP9 (`0x3FF`) / 默认驱动的按键引脚
pub const DEFAULT_BUTTON_PINS: u16 = 0x3FF;

/// Worker thread name / worker 线程名
const WORKER_NAME: &str = "rp2350-play";

/// Target frame period (60 fps ceiling) / 目标帧周期（60 fps 上限）
const FRAME_PERIOD: Duration = Duration::from_micros(16_667);

/// Steps between stop-flag and button samples / 停止标志与按键采样的步数间隔
///
/// Buttons are sampled at this cadence, not once per frame, so a press is visible
/// to the firmware within ~0.1 ms of guest time even when a frame takes 120 ms.
/// 按键按此步数节拍采样（而不是每帧一次），因此即使单帧需要 120 ms，固件也能在
/// 约 0.1 ms 的客户机时间内看到按下。
const STOP_POLL_STEPS: u64 = 4096;

/// Length of the rolling rate window / 滚动速率窗口长度（秒）
const STATS_WINDOW_SECS: f64 = 1.0;

/// Smallest elapsed time used for a rate, so an early snapshot is not NaN
/// 计算速率时使用的最小时间窗，避免早期快照得到 NaN
const MIN_ELAPSED_SECS: f64 = 1e-6;

/// PIO block of the example firmware's I2S state machine / 示例固件 I2S 状态机所在 PIO 块
const AUDIO_BLOCK: usize = 2;

/// State machine of the example firmware's I2S output / 示例固件 I2S 输出的状态机
const AUDIO_SM: usize = 0;

/// Sample rate assumed for captured I2S audio / 捕获 I2S 音频时假定的采样率
const AUDIO_RATE_HZ: u32 = 48_000;

/// Channel count assumed for captured I2S audio / 捕获 I2S 音频时假定的声道数
const AUDIO_CHANNELS: u8 = 2;

/// Samples retained by the audio tap (1 s stereo) / 音频 tap 保留的样本数（1 秒立体声）
const AUDIO_CAPACITY: usize = 48_000;

/// Largest frame buffer a session accepts (64 MiB) / 会话接受的最大帧缓冲（64 MiB）
///
/// A typo in `--fb` must fail the start instead of aborting the process on a
/// huge allocation.
/// `--fb` 里的笔误必须让启动失败，而不是在巨大分配上直接 abort 进程。
const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// Reason reported when the machine budget runs out / 机器预算耗尽时报告的原因
const BUDGET_REASON: &str = "instruction budget exhausted / 指令预算耗尽";

/// Configuration of one play session / 一次游玩会话的配置
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayConfig {
    /// Guest frame buffer / 客户机帧缓冲
    pub frame: FrameSpec,
    /// Steps executed per published frame / 每发布一帧所执行的步数
    pub steps_per_frame: u64,
    /// Stop after this many frames (`None` runs until shutdown)
    /// 发布该帧数后停止（`None` 表示运行到 shutdown）
    pub max_frames: Option<u64>,
    /// Bitmap of the GPIO pins used as buttons (bit `n` = GP`n`)
    /// 用作按键的 GPIO 引脚位图（位 `n` = GP`n`）
    pub button_pins: u16,
    /// Capture I2S words through `AudioTap` / 通过 `AudioTap` 捕获 I2S 字
    pub audio: bool,
    /// Leading steps of the first frame printed as `[trace]` lines
    /// 第一帧开头以 `[trace]` 行打印的步数
    pub trace_steps: usize,
}

impl PlayConfig {
    /// Configuration with no frame limit and no audio capture
    /// 无帧数上限、不捕获音频的配置
    #[must_use]
    pub fn new(frame: FrameSpec, steps_per_frame: u64) -> Self {
        Self {
            frame,
            steps_per_frame,
            max_frames: None,
            button_pins: DEFAULT_BUTTON_PINS,
            audio: false,
            trace_steps: 0,
        }
    }
}

/// Snapshot of the worker's progress / worker 进度快照
#[derive(Debug, Clone, PartialEq)]
pub struct PlayStats {
    /// Frames published so far / 已发布帧数
    pub frames: u64,
    /// Guest steps executed so far / 已执行客户机步数
    pub steps: u64,
    /// Rolling wall-clock steps per second / 滚动窗口的每秒步数
    pub steps_per_second: f64,
    /// Rolling wall-clock frames per second / 滚动窗口的每秒帧数
    pub fps: f64,
    /// Why the worker stopped before `shutdown`, if it did
    /// worker 在 `shutdown` 之前停止的原因（若有）
    pub halted: Option<String>,
}

impl Default for PlayStats {
    fn default() -> Self {
        Self {
            frames: 0,
            steps: 0,
            steps_per_second: 0.0,
            fps: 0.0,
            halted: None,
        }
    }
}

/// Command-line options of the `play` subcommand / `play` 子命令的命令行选项
#[derive(Debug, Clone)]
pub struct PlayOptions {
    /// Firmware file (UF2 or ELF) / 固件文件（UF2 或 ELF）
    pub file: PathBuf,
    /// Guest frame buffer / 客户机帧缓冲
    pub frame: FrameSpec,
    /// Steps executed per published frame / 每发布一帧所执行的步数
    pub steps_per_frame: u64,
    /// Stop after this many frames / 跑满该帧数后停止
    pub max_frames: Option<u64>,
    /// Button pin bitmap / 按键引脚位图
    pub buttons: u16,
    /// Capture I2S audio / 捕获 I2S 音频
    pub audio: bool,
    /// Architecture override / 架构覆盖
    pub arch: Option<ArchArg>,
    /// Halt on undefined instructions / 未定义指令即停机
    pub strict_decode: bool,
    /// Total instruction budget (`u64::MAX` = unlimited)
    /// 总指令预算（`u64::MAX` 表示不限制）
    pub max_cycles: u64,
    /// Leading steps of the first frame printed as `[trace]` lines
    /// 第一帧开头以 `[trace]` 行打印的步数
    pub trace_steps: usize,
}

/// Parse a button mask (`0x` hex or decimal) / 解析按键掩码（`0x` 十六进制或十进制）
///
/// # Errors
/// Returns a message when the value is not a `u16` number.
/// 数值不是 `u16` 时返回错误消息。
pub fn parse_button_mask(spec: &str) -> Result<u16, String> {
    let text = spec.trim();
    let parsed = if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u16::from_str_radix(hex, 16)
    } else {
        text.parse()
    };
    parsed.map_err(|_| format!("bad button mask `{spec}` (expected u16) / 按键掩码非法"))
}

/// Handle to a running play worker / 运行中 worker 的句柄
pub struct PlaySession {
    /// Latest button mask written by the host / 宿主写入的最新按键掩码
    buttons: Arc<AtomicU16>,
    /// Pins the worker actually drives / worker 实际驱动的引脚
    button_pins: u16,
    /// Newest published frame / 最新发布的帧
    latest: Arc<Mutex<Option<SharedFrame>>>,
    /// Frames replaced before a consumer took them / 未被取走即被替换的帧数
    replaced: Arc<AtomicU64>,
    /// Stop request / 停止请求
    stop: Arc<AtomicBool>,
    /// Published statistics / 已发布统计
    stats: Arc<Mutex<PlayStats>>,
    /// Whether the worker thread still runs / worker 线程是否仍在运行
    running: Arc<AtomicBool>,
    /// Audio tap installed on the worker's machine / 安装在 worker 机器上的音频 tap
    audio: Option<AudioTap>,
    /// Worker join handle / worker 的 join 句柄
    worker: Option<JoinHandle<()>>,
}

impl PlaySession {
    /// Boot a machine on a worker thread and start publishing frames
    /// 在 worker 线程上启动机器并开始发布帧
    ///
    /// The image bytes and options are moved into the worker, which constructs
    /// the [`Machine`] itself (the machine is not `Send`). `start` returns only
    /// after the machine has booted, so a boot failure is reported here rather
    /// than silently on the worker.
    /// 映像字节与选项被移入 worker，由 worker 自行构造 [`Machine`]（机器不是
    /// `Send`）。`start` 等机器启动完成后才返回，因此启动失败会在这里报告，而不是
    /// 静默发生在 worker 内。
    ///
    /// # Errors
    /// Returns a message when `steps_per_frame`/frame geometry is invalid, when
    /// the worker thread cannot be spawned, or when the machine fails to boot.
    /// `steps_per_frame`/帧几何非法、worker 线程无法创建，或机器启动失败时返回
    /// 错误消息。
    pub fn start(
        image: Vec<u8>,
        options: MachineOptions,
        config: PlayConfig,
    ) -> Result<Self, String> {
        if config.steps_per_frame == 0 {
            return Err("steps_per_frame must be greater than zero / 每帧步数必须大于 0".into());
        }
        if config.frame.width == 0 || config.frame.height == 0 {
            return Err("frame width and height must be non-zero / 帧宽高必须大于 0".into());
        }
        if config.frame.stride_bytes < u32::from(config.frame.width) * 2 {
            return Err("frame stride is smaller than width*2 / 帧跨度小于行字节数".into());
        }
        if config.frame.buffer_len() > MAX_FRAME_BYTES {
            return Err(format!(
                "frame buffer is {} bytes, above the {MAX_FRAME_BYTES} byte limit / \
                 帧缓冲超过 {MAX_FRAME_BYTES} 字节上限",
                config.frame.buffer_len()
            ));
        }
        let buttons = Arc::new(AtomicU16::new(0));
        let latest = Arc::new(Mutex::new(None));
        let replaced = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(Mutex::new(PlayStats::default()));
        let running = Arc::new(AtomicBool::new(true));
        let audio = config.audio.then(|| {
            AudioTap::new(AudioSink::new(
                AUDIO_RATE_HZ,
                AUDIO_CHANNELS,
                AUDIO_CAPACITY,
            ))
        });
        let (ready_tx, ready_rx) = mpsc::channel();
        let worker = {
            let buttons = Arc::clone(&buttons);
            let latest = Arc::clone(&latest);
            let replaced = Arc::clone(&replaced);
            let stop = Arc::clone(&stop);
            let stats = Arc::clone(&stats);
            let running = Arc::clone(&running);
            let audio = audio.clone();
            thread::Builder::new()
                .name(WORKER_NAME.to_owned())
                .spawn(move || {
                    run_worker(
                        &image, &options, config, &buttons, &stop, &latest, &replaced, &stats,
                        &running, audio, &ready_tx,
                    );
                })
                .map_err(|error| format!("cannot spawn play worker / 无法创建 worker: {error}"))?
        };
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                buttons,
                button_pins: config.button_pins,
                latest,
                replaced,
                stop,
                stats,
                running,
                audio,
                worker: Some(worker),
            }),
            Ok(Err(message)) => {
                let _ = worker.join();
                Err(message)
            }
            Err(_) => {
                let _ = worker.join();
                Err("play worker exited during startup / worker 启动期间退出".into())
            }
        }
    }

    /// Newest frame, or `None` before the first frame is published
    /// 最新帧；首帧发布前返回 `None`
    #[must_use]
    pub fn latest_frame(&self) -> Option<SharedFrame> {
        lock(&self.latest).clone()
    }

    /// Write the button mask (bit set = pressed = pin low)
    /// 写入按键掩码（置位 = 按下 = 引脚低电平）
    pub fn set_buttons(&self, mask: u16) {
        self.buttons.store(mask, Ordering::Relaxed);
    }

    /// Current button mask / 当前按键掩码
    #[must_use]
    pub fn buttons(&self) -> u16 {
        self.buttons.load(Ordering::Relaxed)
    }

    /// Button pins this session drives / 本会话驱动的按键引脚
    #[must_use]
    pub fn button_pins(&self) -> u16 {
        self.button_pins
    }

    /// Statistics snapshot / 统计快照
    #[must_use]
    pub fn stats(&self) -> PlayStats {
        lock(&self.stats).clone()
    }

    /// Number of frames replaced before a consumer took them
    /// 未被取走即被替换的帧数
    #[must_use]
    pub fn replaced_frames(&self) -> u64 {
        self.replaced.load(Ordering::Relaxed)
    }

    /// Whether the worker thread still runs / worker 线程是否仍在运行
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Audio tap installed on the worker's machine, if `config.audio` was set
    /// 安装在 worker 机器上的音频 tap（`config.audio` 开启时才有）
    #[must_use]
    pub fn audio_tap(&self) -> Option<AudioTap> {
        self.audio.clone()
    }

    /// Stop the worker and return the final statistics / 停止 worker 并返回最终统计
    #[must_use]
    pub fn shutdown(mut self) -> PlayStats {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            // worker panic 时 join 返回 Err；统计仍然有效，调用方不 panic。
            // A worker panic makes join return Err; the statistics stay valid
            // and the caller never panics.
            let _ = worker.join();
        }
        self.stats()
    }
}

impl Drop for PlaySession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Lock a mutex, recovering the data when a worker panicked while holding it
/// 获取互斥锁；worker panic 导致锁中毒时仍取回数据
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Publish a statistics snapshot / 发布统计快照
fn publish_stats(slot: &Mutex<PlayStats>, stats: &PlayStats) {
    *lock(slot) = stats.clone();
}

/// Copy the button mask onto the pads the machine is asked to drive
/// 把按键掩码映射到机器需要驱动的 pad 上
///
/// Only bits present in `button_pins` are touched, so unrelated injected inputs
/// survive a mask update.
/// 只处理 `button_pins` 中置位的位，因此与按键无关的注入输入不会被掩码更新破坏。
pub(crate) fn apply_buttons(machine: &Machine, mask: u16, button_pins: u16) {
    for pin in 0..u16::BITS {
        let bit = 1u16 << pin;
        if button_pins & bit == 0 {
            continue;
        }
        // 按下 = 低电平：固件为按键启用了内部上拉。 / Pressed = low: the firmware enables pull-ups.
        machine.set_pad_pin(pin, mask & bit == 0);
    }
}

/// Execute one frame's step budget and report an early stop reason
/// 执行一帧的步数预算，并返回提前停止的原因
///
/// Returns `None` when the budget was consumed normally. `trace_left` counts the
/// `[trace]` steps still owed to the first frame.
/// The frame is executed in [`STOP_POLL_STEPS`] slices: the stop flag **and the
/// button mask** are re-read at every slice, so input latency is one slice of
/// guest time instead of one whole frame.
/// 预算正常耗尽时返回 `None`。`trace_left` 是第一帧尚未打印的 `[trace]` 步数。
/// 一帧按 [`STOP_POLL_STEPS`] 步切片执行：每个切片重新读取停止标志**与按键掩码**，
/// 因此输入延迟是一个切片的客户机时间，而不是一整帧。
fn step_frame(
    machine: &mut Machine,
    config: &PlayConfig,
    stop: &AtomicBool,
    buttons: &AtomicU16,
    trace_left: &mut usize,
) -> Option<String> {
    let mut stepped = 0u64;
    while stepped < config.steps_per_frame {
        if stepped % STOP_POLL_STEPS == 0 {
            apply_buttons(machine, buttons.load(Ordering::Relaxed), config.button_pins);
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            if machine.budget_exhausted() {
                return Some(BUDGET_REASON.to_owned());
            }
            if let Some(reason) = machine.halt_reason() {
                return Some(reason.to_owned());
            }
        }
        let slice = (config.steps_per_frame - stepped).min(STOP_POLL_STEPS);
        if *trace_left > 0 {
            // 前 `trace_steps` 步必须逐步执行，才能打印与 `run --trace-steps`
            // 相同的 `[trace]` 行。
            // The leading `trace_steps` must be stepped one by one so the
            // `[trace]` lines match `run --trace-steps` exactly.
            let mut done = 0u64;
            while done < slice && *trace_left > 0 {
                *trace_left -= 1;
                let index = config.trace_steps - *trace_left - 1;
                let outcome = machine.trace_step(index);
                for line in machine.take_notices() {
                    println!("{line}");
                }
                if let Err(error) = outcome {
                    return Some(error.to_string());
                }
                done += 1;
            }
            stepped += done;
            let rest = slice - done;
            if rest > 0 {
                if let Err(error) = machine.run_steps(rest) {
                    return Some(error.to_string());
                }
                stepped += rest;
            }
        } else {
            if let Err(error) = machine.run_steps(slice) {
                return Some(error.to_string());
            }
            stepped += slice;
        }
    }
    None
}

/// Worker body: boot, loop over frames, publish, then finalize
/// worker 主体：启动、逐帧循环、发布，最后收尾
#[allow(clippy::too_many_arguments)]
fn run_worker(
    image: &[u8],
    options: &MachineOptions,
    config: PlayConfig,
    buttons: &AtomicU16,
    stop: &AtomicBool,
    latest: &Mutex<Option<SharedFrame>>,
    replaced: &AtomicU64,
    stats_slot: &Mutex<PlayStats>,
    running: &AtomicBool,
    audio: Option<AudioTap>,
    ready: &Sender<Result<(), String>>,
) {
    // 线程退出（含提前 return）时清除运行标志，宿主据此结束轮询。
    // Clearing the running flag on every exit path lets the host stop polling.
    struct RunningGuard<'a>(&'a AtomicBool);
    impl Drop for RunningGuard<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Relaxed);
        }
    }
    let _guard = RunningGuard(running);

    let mut machine = match Machine::new(image, options) {
        Ok(machine) => machine,
        Err(error) => {
            let _ = ready.send(Err(format!(
                "machine boot failed / 机器启动失败: {error:#}"
            )));
            return;
        }
    };
    if let Some(tap) = audio {
        machine
            .peripherals_mut()
            .set_audio_tap(AUDIO_BLOCK, AUDIO_SM, tap);
    }
    if ready.send(Ok(())).is_err() {
        // 宿主已放弃会话（channel 关闭），不必继续跑机器。
        // The host dropped the session (channel closed); do not keep stepping.
        return;
    }

    let started = Instant::now();
    let mut window = RateWindow::new(started);
    let mut stats = PlayStats::default();
    let mut trace_left = config.trace_steps;

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if config.max_frames.is_some_and(|limit| stats.frames >= limit) {
            break;
        }
        let frame_started = Instant::now();

        let halted = step_frame(&mut machine, &config, stop, buttons, &mut trace_left);

        let bytes = machine.read_block(config.frame.base, config.frame.buffer_len());
        stats.frames += 1;
        stats.steps = machine.steps();
        let frame = Arc::new(config.frame.frame(stats.frames, stats.steps, &bytes));
        {
            let mut slot = lock(latest);
            if slot.is_some() {
                // 生产者不阻塞消费者：旧帧被直接替换并计数。
                // The producer never blocks: the old frame is replaced and counted.
                replaced.fetch_add(1, Ordering::Relaxed);
            }
            *slot = Some(Arc::clone(&frame));
        }
        let (fps, steps_per_second) = window.rates(Instant::now(), stats.steps, stats.frames);
        stats.fps = fps;
        stats.steps_per_second = steps_per_second;
        stats.halted = halted;
        publish_stats(stats_slot, &stats);
        if stats.halted.is_some() {
            break;
        }
        let elapsed = frame_started.elapsed();
        let budget = FRAME_PERIOD.saturating_sub(elapsed);
        if !budget.is_zero() {
            thread::sleep(budget);
        }
    }

    machine.finalize();
    stats.steps = machine.steps();
    if stats.frames == 0 && stats.steps > 0 {
        // 早停且没有任何完整帧：仍给出一个正值速率，避免 0/NaN 统计。
        // Stopped before a full frame: still report a positive rate instead of 0/NaN.
        let elapsed = started.elapsed().as_secs_f64().max(MIN_ELAPSED_SECS);
        stats.steps_per_second = stats.steps as f64 / elapsed;
    }
    publish_stats(stats_slot, &stats);
}

/// Rolling wall-clock rate estimator / 滚动墙钟速率估计器
struct RateWindow {
    /// Session start, never reset / 会话起点，永不重置
    origin: Instant,
    /// Start of the current window / 当前窗口起点
    start: Instant,
    /// Steps at the window start / 窗口起点的步数
    steps: u64,
    /// Frames at the window start / 窗口起点的帧数
    frames: u64,
}

impl RateWindow {
    /// Start a window at `now` / 在 `now` 处开启窗口
    const fn new(now: Instant) -> Self {
        Self {
            origin: now,
            start: now,
            steps: 0,
            frames: 0,
        }
    }

    /// Rates of the current window / 当前窗口的速率
    ///
    /// Until the window has closed the estimate falls back to the totals since
    /// the session started, so the first snapshots are meaningful (and never
    /// `NaN`).
    /// 在窗口关闭前使用自会话启动以来的总量估计，因此早期快照仍有意义且不会是
    /// `NaN`。
    fn rates(&mut self, now: Instant, total_steps: u64, total_frames: u64) -> (f64, f64) {
        let window = now.duration_since(self.start).as_secs_f64();
        if window >= STATS_WINDOW_SECS {
            let steps = total_steps.saturating_sub(self.steps);
            let frames = total_frames.saturating_sub(self.frames);
            self.start = now;
            self.steps = total_steps;
            self.frames = total_frames;
            (frames as f64 / window, steps as f64 / window)
        } else {
            let elapsed = now
                .duration_since(self.origin)
                .as_secs_f64()
                .max(MIN_ELAPSED_SECS);
            (total_frames as f64 / elapsed, total_steps as f64 / elapsed)
        }
    }
}

/// Run the CLI `play` subcommand / 运行 CLI 的 `play` 子命令
///
/// Prints one `[play] frame=…` line per published frame and a final summary. With
/// `max_frames` set it exits after that many frames; otherwise it runs until the
/// machine halts or the process is interrupted.
/// 每发布一帧打印一行 `[play] frame=…`，结束打印汇总。设置了 `max_frames` 时跑满
/// 即退出；否则运行到机器停机或进程被中断。
///
/// # Errors
/// Returns an error when the firmware cannot be read, the worker cannot start,
/// or stdout cannot be written.
/// 固件读取失败、worker 无法启动或 stdout 写入失败时返回错误。
pub fn run(options: &PlayOptions) -> Result<()> {
    let image = std::fs::read(&options.file).with_context(|| {
        format!(
            "cannot read firmware / 无法读取固件: {}",
            options.file.display()
        )
    })?;
    let machine_options = MachineOptions {
        arch: options.arch,
        max_cycles: options.max_cycles,
        strict_decode: options.strict_decode,
    };
    let config = PlayConfig {
        frame: options.frame,
        steps_per_frame: options.steps_per_frame,
        max_frames: options.max_frames,
        button_pins: options.buttons,
        audio: options.audio,
        trace_steps: options.trace_steps,
    };
    let session = PlaySession::start(image, machine_options, config)
        .map_err(|message| anyhow::anyhow!(message))?;
    session.set_buttons(0);

    let mut printed = 0u64;
    let mut warned = false;
    loop {
        let stats = session.stats();
        if stats.frames > printed {
            if let Some(frame) = session.latest_frame() {
                printed = stats.frames;
                println!(
                    "[play] frame={} steps={} fps={:.2} steps/s={:.0}",
                    frame.index, frame.steps, stats.fps, stats.steps_per_second
                );
                if !warned && !frame.readable {
                    println!("[play] warn: frame buffer unreadable");
                    warned = true;
                }
            }
        }
        if stats.halted.is_some() {
            break;
        }
        if !session.is_running() && stats.frames <= printed {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }

    let stats = session.shutdown();
    println!(
        "[play] summary frames={} steps={} fps={:.2} steps/s={:.0} halted={}",
        stats.frames,
        stats.steps,
        stats.fps,
        stats.steps_per_second,
        stats.halted.as_deref().unwrap_or("none")
    );
    Ok(())
}
