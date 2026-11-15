//! Unit tests for the host audio output / 宿主音频输出的单元测试

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use rp2350_periph::{AudioSink, AudioTap};

#[cfg(feature = "playback")]
use crate::pipeline::SampleQueue;
use crate::{AudioFormat, AudioOutput, SampleSource};

const RATE: u32 = 13_440;
const CHANNELS: u8 = 2;

/// Build a tap with the emerald-compatible stereo format
/// 构建与 emerald 兼容的立体声 tap
fn tap_with(capacity: usize) -> AudioTap {
    AudioTap::new(AudioSink::new(RATE, CHANNELS, capacity))
}

/// Poll `condition` until it holds or `timeout` expires
/// 轮询 `condition` 直到成立或 `timeout` 到期
fn wait_until(mut condition: impl FnMut() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        thread::sleep(Duration::from_millis(2));
    }
    condition()
}

/// Source that hands out a fixed number of constant samples
/// 交出固定数量常量样本的源
struct ConstantSource {
    remaining: usize,
}

impl SampleSource for ConstantSource {
    fn take(&mut self, max: usize) -> Vec<i16> {
        let count = max.min(self.remaining);
        self.remaining -= count;
        vec![1_000; count]
    }

    fn format(&self) -> AudioFormat {
        AudioFormat::new(RATE, CHANNELS)
    }

    fn dropped(&self) -> u64 {
        0
    }
}

/// Source that never runs dry, used to prove the queue stays bounded
/// 永不枯竭的源，用于验证队列有界
struct FloodSource;

impl SampleSource for FloodSource {
    fn take(&mut self, max: usize) -> Vec<i16> {
        vec![1; max]
    }

    fn format(&self) -> AudioFormat {
        AudioFormat::new(RATE, CHANNELS)
    }

    fn dropped(&self) -> u64 {
        0
    }
}

/// Source that flips `alive` off when the worker drops it
/// 被工作线程丢弃时把 `alive` 置否的源
struct DropFlagSource {
    alive: Arc<AtomicBool>,
}

impl SampleSource for DropFlagSource {
    fn take(&mut self, max: usize) -> Vec<i16> {
        vec![0; max.min(8)]
    }

    fn format(&self) -> AudioFormat {
        AudioFormat::new(RATE, CHANNELS)
    }

    fn dropped(&self) -> u64 {
        0
    }
}

impl Drop for DropFlagSource {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

#[test]
fn tap_source_passes_samples_and_format_through() {
    let tap = tap_with(64);
    tap.push_word(0x0001_0002);
    tap.push_word(0xFFFF_FFFE);
    tap.push_word(0x7FFF_8000);
    tap.push_word(0x0000_0000);

    let mut source = tap.clone();
    assert_eq!(source.format(), AudioFormat::new(RATE, CHANNELS));
    assert_eq!(source.take(8), vec![2, 1, -2, -1, -32_768, 32_767, 0, 0]);
    assert_eq!(source.dropped(), 0);
    assert!(tap.is_empty());
}

#[test]
fn tap_source_reports_dropped_and_honours_max() {
    let tap = tap_with(4);
    for _ in 0..4 {
        tap.push_word(0x0003_0004);
    }
    let mut source = tap.clone();
    // 4 个样本的容量只能留下最后一帧，前 4 个样本被丢弃。
    // A capacity of 4 keeps only the last frame, so 4 samples were dropped.
    assert_eq!(source.format(), AudioFormat::new(RATE, CHANNELS));
    assert_eq!(source.dropped(), 4);
    assert_eq!(source.take(3).len(), 3);
    assert_eq!(tap.len(), 1);
    assert_eq!(source.take(usize::MAX).len(), 1);
}

#[test]
fn sources_are_send() {
    fn assert_send<T: Send>() {}
    assert_send::<AudioTap>();
    assert_send::<AudioOutput>();
}

#[test]
fn null_backend_consumes_the_source() {
    let tap = tap_with(16_384);
    for _ in 0..2_000 {
        tap.push_word(0x0002_0001);
    }
    assert_eq!(tap.len(), 4_000);

    let output = AudioOutput::start(tap.clone(), 4_096);
    assert!(output.playing(), "worker must be running after start");
    assert_eq!(output.capacity(), 4_096);
    assert!(
        wait_until(|| tap.len() < 4_000, Duration::from_secs(2)),
        "worker must pull samples from the tap"
    );
    output.shutdown();
}

#[test]
fn empty_source_counts_underruns_without_panicking() {
    let tap = tap_with(4_096);
    let output = AudioOutput::start(tap.clone(), 4_096);
    let grew = wait_until(|| output.underruns() > 0, Duration::from_secs(2));
    if output.backend() == "null" {
        assert!(
            grew,
            "null backend must report underruns on an empty source"
        );
    }
    assert!(output.underruns() <= 1_000, "underruns must not explode");
    output.shutdown();
}

#[test]
fn queue_capacity_is_bounded_and_drops_are_counted() {
    let output = AudioOutput::start(FloodSource, 64);
    assert!(wait_until(|| output.dropped() > 0, Duration::from_secs(2)));
    assert!(output.queued() <= 64, "queue must never exceed capacity");
    assert!(output.dropped() > 0, "overflow must be counted");
    output.shutdown();
}

#[test]
fn two_outputs_can_share_one_tap() {
    let tap = tap_with(8_192);
    for _ in 0..1_000 {
        tap.push_word(0x0004_0003);
    }
    let first = AudioOutput::start(tap.clone(), 1_024);
    let second = AudioOutput::start(tap.clone(), 1_024);
    assert!(wait_until(|| tap.is_empty(), Duration::from_secs(2)));
    first.shutdown();
    second.shutdown();
}

#[test]
fn format_reports_emerald_stream() {
    let tap = tap_with(256);
    let format = SampleSource::format(&tap);
    assert_eq!(format, AudioFormat::new(13_440, 2));
    assert_eq!(format.frames_per_second(), 13_440);
    assert_eq!(format.samples_per_second(), 26_880);
    assert_eq!(AudioFormat::default(), AudioFormat::new(0, 0));
}

#[test]
fn shutdown_joins_the_worker_within_two_seconds() {
    let alive = Arc::new(AtomicBool::new(true));
    let output = AudioOutput::start(
        DropFlagSource {
            alive: Arc::clone(&alive),
        },
        1_024,
    );
    assert!(output.playing());
    let started = Instant::now();
    output.shutdown();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "shutdown took {:?}",
        started.elapsed()
    );
    assert!(
        !alive.load(Ordering::Acquire),
        "the worker must drop its source when joined"
    );
}

#[test]
fn dropping_the_handle_stops_the_worker() {
    let alive = Arc::new(AtomicBool::new(true));
    let output = AudioOutput::start(
        DropFlagSource {
            alive: Arc::clone(&alive),
        },
        1_024,
    );
    drop(output);
    assert!(!alive.load(Ordering::Acquire));
}

#[test]
fn zero_capacity_never_panics() {
    let tap = tap_with(1_024);
    for _ in 0..64 {
        tap.push_word(0x0006_0005);
    }
    let output = AudioOutput::start(tap.clone(), 0);
    assert_eq!(output.queued(), 0);
    assert!(wait_until(|| output.dropped() > 0, Duration::from_secs(2)));
    output.shutdown();
}

#[test]
fn backend_name_is_known() {
    let output = AudioOutput::start(ConstantSource { remaining: 0 }, 128);
    let backend = output.backend();
    assert!(
        backend == "cpal" || backend == "null",
        "unexpected backend {backend}"
    );
    output.shutdown();
}

#[cfg(feature = "playback")]
#[test]
fn resampler_duplicates_frames_when_upsampling() {
    let mut queue = SampleQueue::new(64);
    queue.push(&[10, 20, 30, 40]);
    let mut resampler = crate::pipeline::Resampler::new(2, 4, 2);
    let mut out = [0_i16; 8];
    let missing = resampler.fill(&mut queue, &mut out);
    assert_eq!(missing, 0);
    assert_eq!(out, [10, 20, 10, 20, 30, 40, 30, 40]);
    assert_eq!(queue.len(), 0);

    let mut empty = SampleQueue::new(4);
    let mut silence = [9_i16; 4];
    let starved = resampler.fill(&mut empty, &mut silence);
    assert!(starved > 0);
    assert_eq!(silence, [0, 0, 0, 0]);
}
