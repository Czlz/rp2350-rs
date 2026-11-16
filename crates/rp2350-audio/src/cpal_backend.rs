//! Optional `cpal` sound-card backend / 可选的 `cpal` 声卡后端
//!
//! The stream is created **inside** the audio worker thread because
//! `cpal::Stream` is not `Send` on every platform (ALSA/CoreAudio). Any
//! failure — no default device, unsupported channel count, unsupported sample
//! format, stream build error — returns `None` and the caller silently falls
//! back to the null backend.
//! 音频流在**工作线程内部**创建，因为 `cpal::Stream` 在部分平台（ALSA/CoreAudio）
//! 不是 `Send`。任何失败（无默认设备、声道数不支持、样本格式不支持、建流失败）
//! 都返回 `None`，由调用方静默降级到 null 后端。

use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, SupportedStreamConfig};

use crate::AudioFormat;
use crate::pipeline::{Resampler, Shared};

/// A started stream plus the probe detail shown by the host
/// 已启动的流以及宿主展示的探测详情
pub(crate) struct OpenedStream {
    /// Live output stream; dropping it stops playback
    /// 活动的输出流；丢弃即停止播放
    pub(crate) stream: cpal::Stream,
    /// Device name and selected stream format / 设备名与选定的流格式
    pub(crate) detail: String,
}

/// Open and start a default output stream for `format`
/// 为 `format` 打开并启动默认输出流
///
/// Returns the reason when the host has no usable default output device or no
/// supported configuration, so the null backend can explain itself instead of
/// silently pretending to play.
/// 宿主无可用默认输出设备或没有受支持配置时返回原因，使 null 后端能解释自己，
/// 而不是静默地假装在播放。
pub(crate) fn open(format: AudioFormat, shared: &Arc<Shared>) -> Result<OpenedStream, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "no default output device / 没有默认输出设备".to_owned())?;
    let device_name = device.to_string();
    let channels = u16::from(format.channels.max(1));
    let rate = format.rate_hz.max(1);
    let config = select_config(&device, channels, rate)?;
    let sample_format = config.sample_format();
    let stream = match sample_format {
        SampleFormat::F32 => build::<f32>(&device, &config, format, shared),
        SampleFormat::I16 => build::<i16>(&device, &config, format, shared),
        other => {
            return Err(format!(
                "device {device_name:?} has no f32/i16 output config (best was {other:?}) / \
                 设备没有 f32/i16 输出配置（最佳为 {other:?}）"
            ));
        }
    }
    .map_err(|error| {
        format!("cannot build output stream on {device_name:?}: {error} / 无法在设备上建流")
    })?;
    // 流创建成功后必须显式播放，否则部分后端保持暂停状态。
    // A created stream stays paused on some backends until `play` succeeds.
    stream.play().map_err(|error| {
        format!("cannot start output stream on {device_name:?}: {error} / 无法启动输出流")
    })?;
    let detail = format!(
        "device={device_name:?} stream={}Hz/{}ch/{:?}",
        config.sample_rate(),
        config.channels(),
        sample_format
    );
    Ok(OpenedStream { stream, detail })
}

/// Pick the supported config closest to the requested rate and channel count
/// 选择最接近请求采样率与声道数的受支持配置
///
/// Strategy (documented in `doc/crates/rp2350-audio.md`): only ranges whose
/// channel count equals the tap's are considered; a range containing `rate`
/// is used verbatim (no resampling); otherwise the range whose nearest bound
/// is closest to `rate` wins and the difference is removed by nearest-neighbour
/// resampling; `f32` is preferred over `i16` on ties.
/// 策略（详见 `doc/crates/rp2350-audio.md`）：只考虑声道数与 tap 相同的范围；
/// 覆盖 `rate` 的范围直接使用（不重采样）；否则取最近边界最接近 `rate` 的范围，
/// 差值由最近邻重采样消除；并列时优先 `f32`，其次 `i16`。
fn select_config(
    device: &cpal::Device,
    channels: u16,
    rate: u32,
) -> Result<SupportedStreamConfig, String> {
    let mut best: Option<(u32, u8, SupportedStreamConfig)> = None;
    let mut saw_channel = false;
    let ranges = device
        .supported_output_configs()
        .map_err(|error| format!("cannot query output configs: {error} / 无法查询输出配置"))?;
    for range in ranges {
        if range.channels() != channels {
            continue;
        }
        saw_channel = true;
        let rank = format_rank(range.sample_format());
        if rank == 0 {
            continue;
        }
        let target = rate.clamp(range.min_sample_rate(), range.max_sample_rate());
        let distance = target.abs_diff(rate);
        let Some(config) = range.try_with_sample_rate(target) else {
            continue;
        };
        let better = match &best {
            None => true,
            Some((best_distance, best_rank, _)) => {
                distance < *best_distance || (distance == *best_distance && rank > *best_rank)
            }
        };
        if better {
            best = Some((distance, rank, config));
        }
    }
    best.map(|(_, _, config)| config).ok_or_else(|| {
        if saw_channel {
            format!(
                "no f32/i16 config for {channels} channels at {rate} Hz / \
                 没有 {channels} 声道 {rate} Hz 的 f32/i16 配置"
            )
        } else {
            format!(
                "device exposes no {channels}-channel output config / \
                 设备没有 {channels} 声道输出配置"
            )
        }
    })
}

/// Preference rank of a sample format, `0` meaning unsupported
/// 样本格式优先级，`0` 表示不支持
fn format_rank(format: SampleFormat) -> u8 {
    match format {
        SampleFormat::F32 => 2,
        SampleFormat::I16 => 1,
        _ => 0,
    }
}

/// Build a typed output stream that drains the shared queue
/// 构建从共享队列取样本的类型化输出流
fn build<S>(
    device: &cpal::Device,
    config: &SupportedStreamConfig,
    format: AudioFormat,
    shared: &Arc<Shared>,
) -> Result<cpal::Stream, cpal::Error>
where
    S: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels());
    let stream_config = cpal::StreamConfig {
        channels: config.channels(),
        sample_rate: config.sample_rate(),
        buffer_size: cpal::BufferSize::Default,
    };
    let mut resampler = Resampler::new(format.rate_hz, config.sample_rate(), channels);
    let mut scratch = vec![0_i16; channels];
    let queue = Arc::clone(shared);
    device.build_output_stream::<S, _, _>(
        stream_config,
        move |data: &mut [S], _info| {
            let frames = data.len() / channels;
            scratch.resize(frames * channels, 0);
            let missing = {
                let mut samples = queue.queue.lock();
                resampler.fill(&mut samples, &mut scratch)
            };
            if missing > 0 {
                // 一个回调区间内任一帧缺样本即记一次 underrun。
                // Any starved frame inside one callback counts as one underrun.
                queue.note_underrun();
            }
            for (slot, sample) in data.iter_mut().zip(scratch.iter()) {
                *slot = S::from_sample(f32::from(*sample) / 32768.0);
            }
            // 若回调长度不是声道数的整数倍，尾部补静音而不是保留旧数据。
            // If the callback length is not a whole number of frames, pad the
            // tail with silence instead of leaving stale samples.
            for slot in data.iter_mut().skip(scratch.len()) {
                *slot = S::from_sample(0.0_f32);
            }
        },
        |_error| {
            // 设备错误（拔出/独占）静默忽略：下次 `start` 会重新探测。
            // Device errors (unplugged/busy) stay silent; the next `start`
            // probes again.
        },
        None,
    )
}
