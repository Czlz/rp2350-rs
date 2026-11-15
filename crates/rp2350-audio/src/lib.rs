//! rp2350-audio — host-side audio output for the RP2350 I2S tap
//! rp2350-audio — RP2350 I2S 采样出口的宿主音频输出
//!
//! [`rp2350_periph::AudioTap`] collects the `i16` samples that a guest
//! firmware pushes into a PIO TX FIFO. This crate turns that stream into
//! audible sound on the host: [`AudioOutput::start`] spawns one bounded worker
//! thread, and that thread pulls from a [`SampleSource`] on the wall clock and
//! hands the samples to a backend.
//! [`rp2350_periph::AudioTap`] 收集客户机固件压入 PIO TX FIFO 的 `i16` 样本。本
//! crate 把该样本流变成宿主上可听的声音：[`AudioOutput::start`] 启动一个有界工作
//! 线程，该线程按墙钟节奏从 [`SampleSource`] 取样本并交给后端。
//!
//! # Backends / 后端
//!
//! - With the `playback` feature **and** a usable default output device, the
//!   `cpal` backend plays the samples; [`AudioOutput::backend`] reports
//!   `"cpal"`.
//! - Otherwise (feature disabled, no device, unsupported format, stream error)
//!   the null backend consumes samples on the wall clock and discards them.
//!   [`AudioOutput::backend`] reports `"null"`, [`AudioOutput::start`] still
//!   succeeds and nothing panics.
//! - 启用 `playback` feature **且**存在可用默认输出设备时，`cpal` 后端播放样本，
//!   [`AudioOutput::backend`] 返回 `"cpal"`。
//! - 其它情况（未启用 feature、无设备、格式不支持、建流失败）走 null 后端，按
//!   墙钟节奏消费并丢弃样本，[`AudioOutput::backend`] 返回 `"null"`，
//!   [`AudioOutput::start`] 仍然成功且不会 panic。
//!
//! # Threading and bounds / 线程与有界性
//!
//! The worker never blocks the emulator: it pulls at most one
//! [`TICK`]-worth of samples per iteration, the internal queue is capped at
//! the `capacity` passed to [`AudioOutput::start`] (oldest samples are dropped
//! and counted), and [`AudioOutput::shutdown`] sets an `AtomicBool`, wakes
//! the condition variable and joins the thread. The worker's longest sleep is
//! one tick, so `shutdown` returns in milliseconds, far below the 2 s budget.
//! 工作线程绝不阻塞模拟器：每轮最多取一个 [`TICK`] 时长的样本，内部队列以
//! [`AudioOutput::start`] 的 `capacity` 为上限（满时丢最旧样本并计数）；
//! [`AudioOutput::shutdown`] 置位 `AtomicBool`、唤醒条件变量并 join。工作线程
//! 最长睡眠仅一个 tick，因此 `shutdown` 在毫秒级返回，远低于 2 s 预算。
//!
//! A [`SampleSource`] must therefore never block in
//! [`SampleSource::take`]; [`rp2350_periph::AudioTap`] only takes a short lock.
//! 因此 [`SampleSource::take`] 绝不能阻塞；[`rp2350_periph::AudioTap`] 只做一次
//! 短暂的加锁。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]
// 采样率、声道数与缓冲长度在 u32/usize/u16 之间换算，这是音频管线的固有特性。
// Sample rates, channel counts and buffer lengths are converted between u32,
// usize and u16, which is inherent to an audio pipeline.
#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::sync_channel;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rp2350_periph::AudioTap;

use crate::pipeline::{BACKEND_CPAL, BACKEND_NULL, Shared};

#[cfg(feature = "playback")]
mod cpal_backend;
mod pipeline;
#[cfg(test)]
mod tests;

/// How often the worker pulls from the source and paces the null backend
/// 工作线程从源取样本并驱动 null 后端的周期
pub const TICK: Duration = Duration::from_millis(5);

/// Upper bound for the backend probe that runs during [`AudioOutput::start`]
/// [`AudioOutput::start`] 期间后端探测的等待上限
const INIT_TIMEOUT: Duration = Duration::from_millis(1000);

/// Sample rate and channel count of a stream / 流的采样率与声道数
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AudioFormat {
    /// Sample rate in hertz / 采样率（Hz）
    pub rate_hz: u32,
    /// Channel count (1 = mono, 2 = stereo) / 声道数（1=单声道，2=立体声）
    pub channels: u8,
}

impl AudioFormat {
    /// Create a format from a rate in hertz and a channel count
    /// 用采样率（Hz）与声道数创建格式
    #[must_use]
    pub const fn new(rate_hz: u32, channels: u8) -> Self {
        Self { rate_hz, channels }
    }

    /// Frames (one sample per channel) per second / 每秒帧数（每帧每声道一个样本）
    #[must_use]
    pub const fn frames_per_second(self) -> u32 {
        self.rate_hz
    }

    /// Interleaved samples per second / 每秒交替样本数
    #[must_use]
    pub const fn samples_per_second(self) -> u32 {
        self.rate_hz.saturating_mul(self.channels as u32)
    }
}

/// Anything the audio worker can pull interleaved `i16` samples from
/// 音频工作线程可从中取交替 `i16` 样本的任何对象
///
/// Implemented for [`rp2350_periph::AudioTap`]; implement it directly to feed
/// synthetic audio in tests.
/// 已为 [`rp2350_periph::AudioTap`] 实现；测试中可直接实现以注入合成音频。
pub trait SampleSource: Send {
    /// Remove and return up to `max` samples, oldest first
    /// 按从旧到新的顺序移除并返回最多 `max` 个样本
    ///
    /// Must return promptly and must never block on playback.
    /// 必须立即返回，且绝不等待播放。
    fn take(&mut self, max: usize) -> Vec<i16>;

    /// Sample rate and channel count of the stream / 流的采样率与声道数
    fn format(&self) -> AudioFormat;

    /// Cumulative samples the source itself dropped before this consumer
    /// 本消费者之前源自身累计丢弃的样本数
    fn dropped(&self) -> u64;
}

impl SampleSource for AudioTap {
    fn take(&mut self, max: usize) -> Vec<i16> {
        self.take_samples(max)
    }

    fn format(&self) -> AudioFormat {
        AudioFormat::new(AudioTap::rate_hz(self), AudioTap::channels(self))
    }

    fn dropped(&self) -> u64 {
        AudioTap::dropped(self)
    }
}

/// Handle to one running audio worker / 一个运行中的音频工作线程句柄
///
/// Dropping the handle stops the worker, exactly like
/// [`AudioOutput::shutdown`].
/// 丢弃句柄会停止工作线程，效果与 [`AudioOutput::shutdown`] 相同。
///
/// # Examples
///
/// ```
/// use rp2350_audio::{AudioFormat, AudioOutput, SampleSource};
///
/// struct Silence {
///     left: usize,
/// }
///
/// impl SampleSource for Silence {
///     fn take(&mut self, max: usize) -> Vec<i16> {
///         let count = max.min(self.left);
///         self.left -= count;
///         vec![0; count]
///     }
///     fn format(&self) -> AudioFormat {
///         AudioFormat::new(13_440, 2)
///     }
///     fn dropped(&self) -> u64 {
///         0
///     }
/// }
///
/// let output = AudioOutput::start(Silence { left: 4_096 }, 8_192);
/// assert!(output.playing());
/// assert!(matches!(output.backend(), "cpal" | "null"));
/// output.shutdown();
/// ```
#[derive(Debug)]
pub struct AudioOutput {
    shared: Arc<Shared>,
    capacity: usize,
    handle: Option<JoinHandle<()>>,
}

impl AudioOutput {
    /// Start the background worker for `source` with a queue of `capacity`
    /// samples / 用容量为 `capacity` 个样本的队列启动 `source` 的后台工作线程
    ///
    /// This never fails and never panics: when no backend can be opened the
    /// worker falls back to the null backend and keeps consuming samples.
    /// Return is delayed by at most 1 s while the backend is probed, so
    /// [`AudioOutput::backend`] is already meaningful.
    /// 永不失败、永不 panic：无法打开后端时回落到 null 后端并继续消费样本。返回前
    /// 最多等待 1 s 以完成后端探测，因此 [`AudioOutput::backend`] 立即有效。
    ///
    /// A `capacity` of zero drops every sample, matching
    /// [`rp2350_periph::AudioSink`].
    /// `capacity` 为零时丢弃所有样本，与 [`rp2350_periph::AudioSink`] 一致。
    #[must_use]
    pub fn start<S>(source: S, capacity: usize) -> Self
    where
        S: SampleSource + 'static,
    {
        let shared = Arc::new(Shared::new(capacity));
        let (ready_tx, ready_rx) = sync_channel(1);
        let worker_shared = Arc::clone(&shared);
        let spawned = thread::Builder::new()
            .name("rp2350-audio".to_string())
            .spawn(move || run_worker(source, worker_shared, ready_tx));
        let handle = if let Ok(handle) = spawned {
            // 等待后端探测结果，使 `backend()` 在 `start` 返回后即可用。
            // Wait for the probe so `backend()` is meaningful on return.
            let _ = ready_rx.recv_timeout(INIT_TIMEOUT);
            Some(handle)
        } else {
            // 线程创建失败时降级为「不播放」句柄，仍不 panic。
            // Thread creation failure degrades to an inert handle.
            shared.set_backend(BACKEND_NULL);
            shared.running.store(false, Ordering::Release);
            None
        };
        Self {
            shared,
            capacity,
            handle,
        }
    }

    /// Stop the worker, release the device and join the thread
    /// 停止工作线程、释放设备并 join 线程
    ///
    /// Returns in milliseconds: the worker sleeps at most one [`TICK`] and is
    /// woken immediately by the internal condition variable.
    /// 毫秒级返回：工作线程最长只睡一个 [`TICK`]，且会被内部条件变量立即唤醒。
    pub fn shutdown(mut self) {
        self.stop_and_join();
    }

    /// Whether the worker thread is still running / 工作线程是否仍在运行
    #[must_use]
    pub fn playing(&self) -> bool {
        self.shared.running.load(Ordering::Acquire)
    }

    /// Number of playback intervals that ran out of samples
    /// 缺样本的播放区间数量
    ///
    /// The null backend counts one per tick that could not be filled; the
    /// `cpal` callback counts one per callback that needed silence.
    /// null 后端按每个取不满的 tick 计一次；`cpal` 回调按每次需要补静音的回调计一次。
    #[must_use]
    pub fn underruns(&self) -> u64 {
        self.shared.underruns.load(Ordering::Relaxed)
    }

    /// Name of the active backend, `"cpal"` or `"null"`
    /// 当前后端名称，`"cpal"` 或 `"null"`
    #[must_use]
    pub fn backend(&self) -> &'static str {
        self.shared.backend_name()
    }

    /// Samples currently waiting in the internal queue
    /// 内部队列中当前等待的样本数
    ///
    /// Never exceeds the `capacity` passed to [`AudioOutput::start`].
    /// 永不超过 [`AudioOutput::start`] 传入的 `capacity`。
    #[must_use]
    pub fn queued(&self) -> usize {
        self.shared.queue.lock().len()
    }

    /// Samples the internal queue dropped because it was full
    /// 内部队列因已满而丢弃的样本数
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.shared.queue.lock().dropped()
    }

    /// Queue capacity in samples / 队列容量（样本数）
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Request a stop and wait for the worker to finish
    /// 请求停止并等待工作线程结束
    fn stop_and_join(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.pacer.wake();
        if let Some(handle) = self.handle.take() {
            if handle.join().is_err() {
                // 工作线程 panic 已被隔离；音频路径静默降级。
                // A worker panic stays contained; the audio path degrades silently.
            }
        }
        self.shared.running.store(false, Ordering::Release);
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

/// Worker loop: pull on the wall clock, push into the queue, drive the backend
/// 工作线程主循环：按墙钟取样本、压入队列并驱动后端
// 该函数就是工作线程主体，必须按值持有共享状态与握手通道以满足 `'static`。
// This function is the thread body and must own the shared state and the
// handshake sender to satisfy the `'static` closure bound.
#[allow(clippy::needless_pass_by_value)]
fn run_worker<S>(
    mut source: S,
    shared: Arc<Shared>,
    ready: std::sync::mpsc::SyncSender<&'static str>,
) where
    S: SampleSource,
{
    let format = source.format();
    let channels = usize::from(format.channels.max(1));

    #[cfg(feature = "playback")]
    let stream = cpal_backend::open(format, &shared);
    #[cfg(feature = "playback")]
    let hardware = stream.is_some();
    #[cfg(not(feature = "playback"))]
    let hardware = false;

    shared.set_backend(if hardware { BACKEND_CPAL } else { BACKEND_NULL });
    let _ = ready.try_send(shared.backend_name());

    let batch = batch_samples(format.rate_hz, channels);
    let mut deadline = Instant::now();
    while !shared.is_stopping() {
        let samples = source.take(batch);
        shared.push(&samples);
        if !hardware {
            // null 后端按墙钟节奏消费并丢弃；取不满即记一次 underrun。
            // The null backend consumes on the wall clock; a short fill is an
            // underrun.
            if shared.drain(batch) < batch {
                shared.note_underrun();
            }
        }
        deadline += TICK;
        let now = Instant::now();
        if deadline <= now {
            // 落后时不追帧，避免忙等与时间漂移。
            // Do not chase lost time: avoids busy looping and drift.
            deadline = now + TICK;
        }
        shared.pacer.wait(&shared.stop, deadline - now);
    }
    shared.running.store(false, Ordering::Release);
}

/// Samples the worker pulls per [`TICK`] for a given rate and channel count
/// 给定采样率与声道数时工作线程每个 [`TICK`] 取的样本数
fn batch_samples(rate_hz: u32, channels: usize) -> usize {
    let channels = channels.max(1);
    let per_tick = u64::from(rate_hz.max(1)) * channels as u64 * TICK.as_millis() as u64 / 1000;
    (per_tick as usize).max(channels)
}
