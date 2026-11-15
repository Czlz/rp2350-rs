//! I2S sample capture / I2S 采样捕获
//!
//! Guest firmware that drives an I2S DAC through PIO writes one 32-bit word per
//! stereo frame into the state machine's TX FIFO. The low halfword is the first
//! sample of the frame (left channel) and the high halfword the second (right
//! channel); both halves are two's-complement `i16`. [`AudioSink`] turns those
//! words back into a flat `i16` sample stream so a host-side audio plugin can
//! play them, and [`AudioTap`] is the shareable handle installed on a PIO state
//! machine by [`crate::Pio::set_audio_tap`] or
//! [`crate::BootPeripherals::set_audio_tap`].
//! 客户机固件通过 PIO 驱动 I2S DAC 时，会为每个立体声帧向状态机的 TX FIFO 写入
//! 一个 32 位字。低半字是该帧的第一个样本（左声道），高半字是第二个样本（右
//! 声道），两半都是二进制补码 `i16`。[`AudioSink`] 把这些字还原为扁平的 `i16`
//! 样本流，供宿主侧音频插件播放；[`AudioTap`] 是可共享句柄，由
//! [`crate::Pio::set_audio_tap`] 或 [`crate::BootPeripherals::set_audio_tap`]
//! 安装到某个 PIO 状态机上。
//!
//! The buffer is bounded. A full sink drops its **oldest** sample (and counts the
//! drop) instead of blocking or growing, so an emulator that runs faster than
//! real time never stalls on audio and never leaks memory. The sink is inactive
//! until a tap is installed: with no tap the FIFO push path does a single
//! `Option` check.
//! 缓冲是有界的。缓冲满时丢弃**最旧**样本（并计数），既不阻塞也不增长，因此比
//! 实时跑得更快的模拟器不会因音频停顿，也不会泄漏内存。未安装 tap 时 sink 不
//! 生效：FIFO 压入路径只做一次 `Option` 判断。

// 32 位字按两个 16 位半字打包，u16 → i16 的 `as` 正是所需的位重解释（负数样本
// 依赖补码语义）。
// A 32-bit word packs two 16-bit halfwords, so the u16 → i16 `as` cast is
// exactly the required bit reinterpretation (negative samples rely on the
// two's-complement encoding).
#![allow(clippy::cast_possible_wrap)]

use std::collections::VecDeque;
use std::sync::Arc;

use parking_lot::Mutex;

/// Number of `i16` samples packed into one I2S FIFO word
/// 一个 I2S FIFO 字打包的 `i16` 样本数
pub const SAMPLES_PER_WORD: usize = 2;

/// Bounded stereo sample buffer fed by PIO TX FIFO pushes
/// 由 PIO TX FIFO 压入驱动的有界立体声样本缓冲
///
/// `push_word` appends two samples per word; `take_samples` drains them in
/// order. When the buffer already holds `capacity` samples the oldest one is
/// dropped first, so the consumer always sees the most recent audio and the
/// dropout count is reported through [`AudioSink::dropped`].
/// 每个字由 `push_word` 追加两个样本，`take_samples` 按顺序取走。缓冲已持有
/// `capacity` 个样本时先丢弃最旧的样本，因此消费者总是拿到最新的音频，丢样次数
/// 通过 [`AudioSink::dropped`] 报告。
#[derive(Debug)]
pub struct AudioSink {
    rate_hz: u32,
    channels: u8,
    samples: VecDeque<i16>,
    capacity: usize,
    dropped: u64,
}

impl AudioSink {
    /// Create a sink for `rate_hz` and `channels` holding up to `capacity` samples
    /// 为 `rate_hz` 采样率、`channels` 声道创建最多容纳 `capacity` 个样本的缓冲
    ///
    /// `channels` is informational (the playback plugin uses it together with
    /// [`AudioSink::rate_hz`]); `push_word` always appends both halves of the
    /// word, matching the firmware's interleaved stereo frames. A zero
    /// `capacity` sink drops every sample and never stores anything.
    /// `channels` 仅作信息（播放插件与 [`AudioSink::rate_hz`] 配合使用）；
    /// `push_word` 总是追加一个字的两半，与固件的交替立体声帧一致。`capacity`
    /// 为零的缓冲丢弃所有样本、不保存任何数据。
    #[must_use]
    pub fn new(rate_hz: u32, channels: u8, capacity: usize) -> Self {
        Self {
            rate_hz,
            channels,
            samples: VecDeque::with_capacity(capacity),
            capacity,
            dropped: 0,
        }
    }

    /// Append the two `i16` samples packed in one I2S FIFO word
    /// 追加一个 I2S FIFO 字中打包的两个 `i16` 样本
    ///
    /// The low halfword is appended first (the frame's left/first sample), the
    /// high halfword second (right/second sample); each half is interpreted as
    /// two's-complement `i16`, so `0x8000` becomes `-32768` and `0x7FFF` becomes
    /// `32767`. The call never blocks and never grows past the configured
    /// capacity: a full sink discards its oldest sample first and increments the
    /// drop counter.
    /// 先追加低半字（帧的左/第一个样本），再追加高半字（右/第二个样本）；每个
    /// 半字按二进制补码 `i16` 解释，因此 `0x8000` 变成 `-32768`、`0x7FFF` 变成
    /// `32767`。调用永不阻塞、也不会超过配置容量：缓冲满时先丢弃最旧样本并递增
    /// 丢样计数。
    pub fn push_word(&mut self, word: u32) {
        // 固件按 `(right << 16) | left` 打包，因此低半字先于高半字播放。
        // The firmware packs `(right << 16) | left`, so the low halfword plays
        // before the high halfword.
        let first = (word & 0xFFFF) as u16 as i16;
        let second = (word >> 16) as u16 as i16;
        self.push_sample(first);
        self.push_sample(second);
    }

    /// Append one sample, evicting the oldest when the buffer is full
    /// 追加一个样本，缓冲满时淘汰最旧的样本
    fn push_sample(&mut self, sample: i16) {
        if self.capacity == 0 {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        while self.samples.len() >= self.capacity {
            self.samples.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.samples.push_back(sample);
    }

    /// Remove and return up to `max` samples, oldest first
    /// 按从旧到新的顺序移除并返回最多 `max` 个样本
    ///
    /// `max` larger than [`AudioSink::len`] drains everything. Samples are
    /// consumed, so a second call returns only what was pushed afterwards.
    /// `max` 大于 [`AudioSink::len`] 时取空全部样本。样本被消费，因此第二次调用只
    /// 返回此后压入的内容。
    pub fn take_samples(&mut self, max: usize) -> Vec<i16> {
        let count = max.min(self.samples.len());
        self.samples.drain(..count).collect()
    }

    /// Number of samples discarded because the buffer was full
    /// 因缓冲已满而被丢弃的样本数量
    ///
    /// The counter is cumulative and survives [`AudioSink::take_samples`];
    /// [`AudioSink::clear`] leaves it untouched so a host can still report
    /// dropouts after resetting the stream.
    /// 计数是累计值，经历 [`AudioSink::take_samples`] 后保持不变；
    /// [`AudioSink::clear`] 不修改它，因此宿主在重置流之后仍能报告丢样。
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Number of samples currently buffered / 当前缓冲的样本数
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether the buffer holds no samples / 缓冲是否为空
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Sample rate the sink was created with, in hertz / 创建缓冲时的采样率（Hz）
    #[must_use]
    pub fn rate_hz(&self) -> u32 {
        self.rate_hz
    }

    /// Channel count the sink was created with / 创建缓冲时的声道数
    #[must_use]
    pub fn channels(&self) -> u8 {
        self.channels
    }

    /// Maximum number of samples the sink retains / 缓冲保留的最大样本数
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Discard every buffered sample, keeping the drop counter
    /// 丢弃所有已缓冲样本，保留丢样计数
    ///
    /// The capacity and stream metadata are unchanged, so the sink can keep
    /// receiving words immediately.
    /// 容量与流元数据不变，缓冲可立即继续接收字。
    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

/// Shareable handle to an [`AudioSink`] / [`AudioSink`] 的可共享句柄
///
/// Clones share one sink, so the tap installed on a PIO state machine and the
/// handle held by the host observe the same sample stream. Every method takes
/// `&self`; the sink itself sits behind a `parking_lot::Mutex`.
/// 克隆共享同一个缓冲，因此安装到 PIO 状态机的 tap 与宿主持有的句柄观察同一条
/// 样本流。所有方法都取 `&self`；缓冲本身位于 `parking_lot::Mutex` 之后。
#[derive(Debug, Clone)]
pub struct AudioTap {
    sink: Arc<Mutex<AudioSink>>,
}

impl AudioTap {
    /// Wrap an [`AudioSink`] in a shareable tap / 把 [`AudioSink`] 包装为可共享 tap
    #[must_use]
    pub fn new(sink: AudioSink) -> Self {
        Self {
            sink: Arc::new(Mutex::new(sink)),
        }
    }

    /// Append one I2S FIFO word to the shared sink / 向共享缓冲追加一个 I2S FIFO 字
    ///
    /// The PIO TX FIFO push path calls this once per accepted word; hosts may
    /// also use it to inject samples directly (for example in tests).
    /// PIO TX FIFO 压入路径在每次接受一个字时调用；宿主也可以直接用它注入样本
    /// （例如测试中）。
    pub fn push_word(&self, word: u32) {
        self.sink.lock().push_word(word);
    }

    /// Remove and return up to `max` samples, oldest first
    /// 按从旧到新的顺序移除并返回最多 `max` 个样本
    #[must_use]
    pub fn take_samples(&self, max: usize) -> Vec<i16> {
        self.sink.lock().take_samples(max)
    }

    /// Number of buffered samples / 已缓冲样本数
    #[must_use]
    pub fn len(&self) -> usize {
        self.sink.lock().len()
    }

    /// Whether no samples are buffered / 是否没有缓冲样本
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sink.lock().is_empty()
    }

    /// Cumulative number of dropped samples / 累计丢样数量
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.sink.lock().dropped()
    }

    /// Sample rate of the wrapped sink, in hertz / 被包装缓冲的采样率（Hz）
    #[must_use]
    pub fn rate_hz(&self) -> u32 {
        self.sink.lock().rate_hz()
    }

    /// Channel count of the wrapped sink / 被包装缓冲的声道数
    #[must_use]
    pub fn channels(&self) -> u8 {
        self.sink.lock().channels()
    }

    /// Discard every buffered sample / 丢弃所有已缓冲样本
    pub fn clear(&self) {
        self.sink.lock().clear();
    }
}
