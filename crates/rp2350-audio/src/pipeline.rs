//! Bounded queue, wall-clock pacing and resampling primitives
//! 有界队列、墙钟节拍与重采样原语
//!
//! Everything in this module is a crate-private implementation detail of
//! [`crate::AudioOutput`]; only the queue and the pacer are shared with the
//! backend and worker threads.
//! 本模块全部内容都是 [`crate::AudioOutput`] 的 crate 私有实现细节；只有队列与
//! 节拍器在后端线程和工作线程之间共享。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::{Condvar, Mutex};

/// Backend still being probed / 后端仍在探测中
pub(crate) const BACKEND_PENDING: u8 = 0;
/// Null backend: samples are consumed on the wall clock and discarded
/// 空后端：样本按墙钟节奏消费后丢弃
pub(crate) const BACKEND_NULL: u8 = 1;
/// `cpal` backend: samples are played by the sound card
/// `cpal` 后端：样本由声卡播放
pub(crate) const BACKEND_CPAL: u8 = 2;

/// Bounded interleaved `i16` queue between the producer and the consumer
/// 生产者与消费者之间的有界交替 `i16` 队列
///
/// The producer never blocks and never grows past `capacity`: pushing into a
/// full queue drops the **oldest** samples first and counts them, mirroring
/// [`rp2350_periph::AudioSink`] semantics.
/// 生产者永不阻塞、也不会超过 `capacity`：向已满队列压入时先丢弃**最旧**样本并
/// 计数，语义与 [`rp2350_periph::AudioSink`] 一致。
#[derive(Debug)]
pub(crate) struct SampleQueue {
    samples: VecDeque<i16>,
    capacity: usize,
    dropped: u64,
}

impl SampleQueue {
    /// Create an empty queue holding at most `capacity` samples
    /// 创建最多容纳 `capacity` 个样本的空队列
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(capacity),
            capacity,
            dropped: 0,
        }
    }

    /// Append samples, evicting the oldest ones when the queue is full
    /// 追加样本，队列满时淘汰最旧的样本
    pub(crate) fn push(&mut self, samples: &[i16]) {
        if self.capacity == 0 {
            self.dropped = self.dropped.saturating_add(samples.len() as u64);
            return;
        }
        for &sample in samples {
            while self.samples.len() >= self.capacity {
                self.samples.pop_front();
                self.dropped = self.dropped.saturating_add(1);
            }
            self.samples.push_back(sample);
        }
    }

    /// Discard up to `max` samples and report how many were removed
    /// 丢弃最多 `max` 个样本并返回实际移除数量
    pub(crate) fn drain(&mut self, max: usize) -> usize {
        let count = max.min(self.samples.len());
        for _ in 0..count {
            self.samples.pop_front();
        }
        count
    }

    /// Remove one interleaved frame of `channels` samples
    /// 取出一个含 `channels` 个样本的交替帧
    ///
    /// Returns `false` (leaving the queue untouched) when a whole frame is not
    /// buffered, so the caller can emit silence instead of a partial frame.
    /// 整帧未缓冲时返回 `false` 且不改动队列，调用方可改为输出静音而不是半帧。
    #[cfg(feature = "playback")]
    pub(crate) fn pop_frame(&mut self, channels: usize, out: &mut [i16]) -> bool {
        if channels == 0 || self.samples.len() < channels || out.len() < channels {
            return false;
        }
        for slot in out.iter_mut().take(channels) {
            *slot = self.samples.pop_front().unwrap_or(0);
        }
        true
    }

    /// Number of buffered samples / 已缓冲样本数
    pub(crate) fn len(&self) -> usize {
        self.samples.len()
    }

    /// Number of samples dropped because the queue was full
    /// 因队列已满而被丢弃的样本数量
    pub(crate) fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// Nearest-neighbour resampler from the tap rate to the device rate
/// 从 tap 采样率到设备采样率的最近邻重采样器
///
/// The device rate may differ from the source rate (for example a 13 440 Hz
/// tap on a 48 000 Hz sound card). Samples are duplicated when upsampling and
/// skipped when downsampling; no interpolation is attempted, which keeps the
/// audio callback branch-free and allocation-free per frame.
/// 设备采样率可能不同于源采样率（例如 13 440 Hz 的 tap 播放到 48 000 Hz 声卡）。
/// 升采样时重复样本、降采样时跳过样本，不做插值，因此音频回调逐帧无分支开销
/// 且不分配内存。
#[cfg(feature = "playback")]
#[derive(Debug)]
pub(crate) struct Resampler {
    src_rate: u64,
    dst_rate: u64,
    channels: usize,
    /// Position accumulator in units of `1 / dst_rate` second
    /// 以 `1 / dst_rate` 秒为单位的位置累加器
    acc: u64,
    /// Most recent source frame / 最近取到的源帧
    current: Vec<i16>,
}

#[cfg(feature = "playback")]
impl Resampler {
    /// Create a resampler from `src_rate` to `dst_rate` for `channels`
    /// 为 `channels` 个声道创建从 `src_rate` 到 `dst_rate` 的重采样器
    pub(crate) fn new(src_rate: u32, dst_rate: u32, channels: usize) -> Self {
        let channels = channels.max(1);
        Self {
            src_rate: u64::from(src_rate.max(1)),
            dst_rate: u64::from(dst_rate.max(1)),
            channels,
            acc: 0,
            current: vec![0; channels],
        }
    }

    /// Fill `out` (interleaved) and return the number of starved source frames
    /// 填充 `out`（交替排列）并返回缺样本的源帧数量
    ///
    /// A starved frame is replaced by silence, so the callback never blocks.
    /// 缺样本的帧由静音替代，因此回调永不阻塞。
    pub(crate) fn fill(&mut self, queue: &mut SampleQueue, out: &mut [i16]) -> u64 {
        let channels = self.channels;
        let mut missing = 0_u64;
        let mut have = false;
        for frame in out.chunks_mut(channels) {
            while !have || self.acc >= self.dst_rate {
                if have {
                    self.acc -= self.dst_rate;
                }
                have = true;
                if !queue.pop_frame(channels, &mut self.current) {
                    self.current.fill(0);
                    missing = missing.saturating_add(1);
                }
            }
            frame.copy_from_slice(&self.current[..frame.len()]);
            self.acc = self.acc.saturating_add(self.src_rate);
        }
        missing
    }
}

/// Condition-variable pacer used by the worker thread for its wall clock
/// 工作线程用于墙钟节拍的条件变量节拍器
///
/// The mutex is never held by the audio callback, so a pacing wait can never
/// stall playback.
/// 音频回调从不持有该互斥锁，因此节拍等待绝不会卡住播放。
#[derive(Debug)]
pub(crate) struct Pacer {
    lock: Mutex<()>,
    cond: Condvar,
}

impl Pacer {
    /// Create an idle pacer / 创建空闲节拍器
    pub(crate) const fn new() -> Self {
        Self {
            lock: Mutex::new(()),
            cond: Condvar::new(),
        }
    }

    /// Sleep for `duration`, waking early when `stop` becomes true
    /// 休眠 `duration`，`stop` 置位时提前唤醒
    pub(crate) fn wait(&self, stop: &AtomicBool, duration: Duration) {
        if duration.is_zero() || stop.load(Ordering::Acquire) {
            return;
        }
        let mut guard = self.lock.lock();
        if stop.load(Ordering::Acquire) {
            return;
        }
        self.cond.wait_for(&mut guard, duration);
    }

    /// Wake every waiter / 唤醒所有等待者
    pub(crate) fn wake(&self) {
        self.cond.notify_all();
    }
}

/// State shared by [`crate::AudioOutput`], the worker and the backend callback
/// [`crate::AudioOutput`]、工作线程与后端回调共享的状态
#[derive(Debug)]
pub(crate) struct Shared {
    /// Bounded sample queue / 有界样本队列
    pub(crate) queue: Mutex<SampleQueue>,
    /// Wall-clock pacer / 墙钟节拍器
    pub(crate) pacer: Pacer,
    /// Set by [`crate::AudioOutput::shutdown`] / 由 `shutdown` 置位
    pub(crate) stop: AtomicBool,
    /// Whether the worker thread is alive / 工作线程是否存活
    pub(crate) running: AtomicBool,
    /// Number of starved playback intervals / 缺样本的播放区间数量
    pub(crate) underruns: AtomicU64,
    backend: AtomicU8,
}

impl Shared {
    /// Create the shared state for a queue of `capacity` samples
    /// 为容量 `capacity` 的队列创建共享状态
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            queue: Mutex::new(SampleQueue::new(capacity)),
            pacer: Pacer::new(),
            stop: AtomicBool::new(false),
            running: AtomicBool::new(true),
            underruns: AtomicU64::new(0),
            backend: AtomicU8::new(BACKEND_PENDING),
        }
    }

    /// Append samples to the bounded queue / 向有界队列追加样本
    pub(crate) fn push(&self, samples: &[i16]) {
        self.queue.lock().push(samples);
    }

    /// Discard up to `max` samples / 丢弃最多 `max` 个样本
    pub(crate) fn drain(&self, max: usize) -> usize {
        self.queue.lock().drain(max)
    }

    /// Count one starved playback interval / 记一次缺样本的播放区间
    pub(crate) fn note_underrun(&self) {
        self.underruns.fetch_add(1, Ordering::Relaxed);
    }

    /// Publish the selected backend / 发布已选定的后端
    pub(crate) fn set_backend(&self, backend: u8) {
        self.backend.store(backend, Ordering::Release);
    }

    /// Name of the active backend, `"cpal"` or `"null"`
    /// 当前后端名称，`"cpal"` 或 `"null"`
    pub(crate) fn backend_name(&self) -> &'static str {
        if self.backend.load(Ordering::Acquire) == BACKEND_CPAL {
            "cpal"
        } else {
            "null"
        }
    }

    /// Whether [`crate::AudioOutput::shutdown`] already requested a stop
    /// `shutdown` 是否已请求停止
    pub(crate) fn is_stopping(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }
}
