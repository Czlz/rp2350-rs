//! `play` option resolution: CLI arguments, profile file, generic defaults
//! `play` 选项解析：命令行参数、profile 文件与通用默认值
//!
//! Precedence is **command line > profile > built-in generic default**
//! 优先级为**命令行 > profile > 内置通用默认值**
//!
//! The crate has no example-specific defaults: the firmware path, frame buffer,
//! steps per frame and the I2S wiring must come from a profile or the command
//! line. The remaining defaults are generic (no audio capture, no button pins,
//! 48 kHz stereo sink when audio is requested).
//! crate 内没有示例专用默认值：固件路径、帧缓冲、每帧步数与 I2S 接线必须来自
//! profile 或命令行。其余默认值都是通用的（默认不捕获音频、不驱动任何按键引脚；
//! 请求音频时使用 48 kHz 立体声 sink）。

use std::path::PathBuf;

use super::profile::{self, Profile};
use super::{FrameSpec, PlayAudioConfig};
use crate::run::{ArchArg, CoreThreadOptions, PsramOptions};

/// Default sample rate of the audio sink when audio is enabled
/// 启用音频时音频 sink 的默认采样率
///
/// Generic: it describes the sink, not any particular firmware. A profile or
/// `--audio-rate` overrides it with the guest's real I2S rate.
/// 通用值：它描述 sink 本身，而不是某个固件。profile 或 `--audio-rate` 会用客户机
/// 真实的 I2S 采样率覆盖它。
pub const DEFAULT_AUDIO_RATE_HZ: u32 = 48_000;

/// Default channel count of the audio sink / 音频 sink 的默认声道数
pub const DEFAULT_AUDIO_CHANNELS: u8 = 2;

/// Default number of samples retained by the audio sink (1 s at 48 kHz)
/// 音频 sink 默认保留的样本数（48 kHz 下 1 秒）
pub const DEFAULT_AUDIO_CAPACITY: usize = 48_000;

/// Default total instruction budget (`u64::MAX` = unlimited)
/// 默认总指令预算（`u64::MAX` 表示不限制）
pub const DEFAULT_MAX_CYCLES: u64 = u64::MAX;

/// Default number of `[trace]` steps printed for the first frame
/// 第一帧默认打印的 `[trace]` 步数
pub const DEFAULT_TRACE_STEPS: usize = 0;

/// Default number of machine steps between cross-core synchronisations
/// 跨核同步之间的默认机器步数
///
/// Matches `run`/`repl`'s `--core-window` default. A profile or the command line
/// overrides it; `0` and values above 4096 are rejected.
/// 与 `run`/`repl` 的 `--core-window` 默认值一致。profile 或命令行可覆盖；`0`
/// 与大于 4096 的值会被拒绝。
pub const DEFAULT_CORE_WINDOW: u32 = 64;

/// Raw `play` arguments as parsed from the command line
/// 从命令行解析出的原始 `play` 参数
///
/// Every field is optional because a `--profile` may supply it. [`resolve`]
/// turns this plus the profile into [`PlayOptions`].
/// 每个字段都是可选的，因为 `--profile` 可能提供它。[`resolve`] 把它与 profile
/// 合并为 [`PlayOptions`]。
#[derive(Debug, Clone, Default)]
pub struct PlayArgs {
    /// Profile file to read / 要读取的 profile 文件
    pub profile: Option<PathBuf>,
    /// Firmware file (UF2 or ELF) / 固件文件（UF2 或 ELF）
    pub file: Option<PathBuf>,
    /// Guest frame buffer / 客户机帧缓冲
    pub frame: Option<FrameSpec>,
    /// Steps executed per published frame / 每发布一帧执行的步数
    pub steps_per_frame: Option<u64>,
    /// Stop after this many frames / 跑满该帧数后停止
    pub max_frames: Option<u64>,
    /// Button pin bitmap / 按键引脚位图
    pub buttons: Option<u16>,
    /// Capture I2S audio / 捕获 I2S 音频
    pub audio: bool,
    /// PIO block that pushes I2S words / 推送 I2S 字的 PIO 块
    pub audio_block: Option<usize>,
    /// State machine inside that PIO block / 该 PIO 块内的状态机
    pub audio_sm: Option<usize>,
    /// Sample rate stored in the audio sink / 音频 sink 的采样率
    pub audio_rate: Option<u32>,
    /// Channel count stored in the audio sink / 音频 sink 的声道数
    pub audio_channels: Option<u8>,
    /// Samples the audio sink retains / 音频 sink 保留的样本数
    pub audio_capacity: Option<usize>,
    /// Architecture override / 架构覆盖
    pub arch: Option<ArchArg>,
    /// Halt on undefined instructions / 未定义指令即停机
    pub strict_decode: bool,
    /// Total instruction budget / 总指令预算
    pub max_cycles: Option<u64>,
    /// Leading `[trace]` steps / 开头的 `[trace]` 步数
    pub trace_steps: Option<usize>,
    /// Two-thread core executor switch from the command line (`None` = profile)
    /// 命令行给出的双核线程开关（`None` 表示取 profile 值）
    pub core_threads: Option<bool>,
    /// Cross-core window from the command line (`None` = profile)
    /// 命令行给出的跨核窗口（`None` 表示取 profile 值）
    pub core_window: Option<u32>,
    /// PSRAM window behind XIP chip select 1 / XIP 片选 1 后的 PSRAM 窗口
    pub psram: Option<PsramOptions>,
}

/// Resolved `play` options (CLI + profile + defaults) / 解析完成的 `play` 选项
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayOptions {
    /// Firmware file (UF2 or ELF) / 固件文件（UF2 或 ELF）
    pub file: PathBuf,
    /// Guest frame buffer / 客户机帧缓冲
    pub frame: FrameSpec,
    /// Steps executed per published frame / 每发布一帧执行的步数
    pub steps_per_frame: u64,
    /// Stop after this many frames / 跑满该帧数后停止
    pub max_frames: Option<u64>,
    /// Button pin bitmap / 按键引脚位图
    pub buttons: u16,
    /// Audio wiring, or `None` when capture is disabled
    /// 音频接线；不捕获时为 `None`
    pub audio: Option<PlayAudioConfig>,
    /// Architecture override / 架构覆盖
    pub arch: Option<ArchArg>,
    /// Halt on undefined instructions / 未定义指令即停机
    pub strict_decode: bool,
    /// Total instruction budget / 总指令预算
    pub max_cycles: u64,
    /// Leading `[trace]` steps / 开头的 `[trace]` 步数
    pub trace_steps: usize,
    /// Optional two-thread core executor / 可选双核线程执行器
    pub core_threads: CoreThreadOptions,
    /// PSRAM window behind XIP chip select 1, or `None` to leave it unmapped
    /// XIP 片选 1 后的 PSRAM 窗口；`None` 表示保持未映射
    pub psram: Option<PsramOptions>,
}

/// Merge raw CLI arguments with an optional profile and the generic defaults
/// 把原始命令行参数与可选 profile、通用默认值合并
///
/// # Errors
/// Returns a bilingual message when the profile cannot be read or parsed, when
/// `firmware`/`frame`/`steps_per_frame` is still missing, or when audio capture
/// is requested without a PIO block/state machine.
/// profile 读取/解析失败、`firmware`/`frame`/`steps_per_frame` 仍然缺失，或请求
/// 音频捕获却没有 PIO 块/状态机时，返回双语错误消息。
pub fn resolve(args: &PlayArgs) -> Result<PlayOptions, String> {
    let profile = load_profile(args)?;
    let file = args
        .file
        .clone()
        .or_else(|| profile.firmware.clone())
        .ok_or_else(|| missing("firmware file", "--file", "firmware"))?;
    let frame = args
        .frame
        .or(profile.frame)
        .ok_or_else(|| missing("frame buffer", "--fb", "frame"))?;
    let steps_per_frame = args
        .steps_per_frame
        .or(profile.steps_per_frame)
        .ok_or_else(|| missing("steps per frame", "--steps-per-frame", "steps_per_frame"))?;
    if steps_per_frame == 0 {
        return Err("steps per frame must be greater than zero / 每帧步数必须大于 0".to_owned());
    }
    // 跨核窗口按 `--core-window` 的同一范围校验：命令行已由 clap 校验，profile
    // 也在解析时校验，这里兜住直接构造 `PlayArgs` 的调用方。
    // The cross-core window uses the same range as `--core-window`: clap checks
    // the command line and the profile parser checks the file, so this guards
    // callers that build `PlayArgs` directly.
    let core_window = args
        .core_window
        .or(profile.core_window)
        .unwrap_or(DEFAULT_CORE_WINDOW);
    if !(1..=4096).contains(&core_window) {
        return Err(format!(
            "bad core window {core_window} (expected 1..=4096) / \
             核窗口大小非法，应为 1..=4096"
        ));
    }
    Ok(PlayOptions {
        file,
        frame,
        steps_per_frame,
        max_frames: args.max_frames.or(profile.max_frames),
        // 默认不驱动任何引脚：按键映射属于 profile / 前端。
        // No pin is driven by default: button wiring belongs to a profile/frontend.
        buttons: args.buttons.or(profile.buttons).unwrap_or(0),
        audio: resolve_audio(args, &profile)?,
        arch: args.arch.or(profile.arch),
        strict_decode: args.strict_decode || profile.strict_decode.unwrap_or(false),
        max_cycles: args
            .max_cycles
            .or(profile.max_cycles)
            .unwrap_or(DEFAULT_MAX_CYCLES),
        trace_steps: args.trace_steps.unwrap_or(DEFAULT_TRACE_STEPS),
        // 默认不挂载 PSRAM：与引入该选项前一致，`0x1100_0000` 保持 NoDevice。
        // No PSRAM by default: `0x1100_0000` keeps reporting NoDevice exactly as
        // it did before this option existed.
        psram: args.psram.or(profile.psram),
        // 核执行器：命令行 > profile > 默认（off / 64）。默认 off 保持确定性，
        // on 是交互提速模式，窗口内访问顺序由宿主调度决定。
        // Core executor: command line > profile > default (off / 64). The default
        // off stays deterministic; on is the interactive mode whose intra-window
        // access order is decided by the host scheduler.
        core_threads: CoreThreadOptions {
            enabled: args.core_threads.or(profile.core_threads).unwrap_or(false),
            window: core_window,
        },
    })
}

/// Read and parse the profile named by `--profile` / 读取并解析 `--profile` 指定的 profile
fn load_profile(args: &PlayArgs) -> Result<Profile, String> {
    let Some(path) = args.profile.as_deref() else {
        return Ok(Profile::default());
    };
    let text = std::fs::read_to_string(path).map_err(|error| {
        format!(
            "cannot read profile / 无法读取 profile {}: {error}",
            path.display()
        )
    })?;
    profile::parse(&text).map_err(|error| format!("{}: {error}", path.display()))
}

/// Resolve the audio wiring, if capture was requested
/// 若请求捕获，解析音频接线
fn resolve_audio(args: &PlayArgs, profile: &Profile) -> Result<Option<PlayAudioConfig>, String> {
    let enabled = args.audio || profile.audio.unwrap_or(false);
    if !enabled {
        return Ok(None);
    }
    let block = args
        .audio_block
        .or(profile.audio_block)
        .ok_or_else(|| missing("audio PIO block", "--audio-block", "audio_block"))?;
    let sm = args
        .audio_sm
        .or(profile.audio_sm)
        .ok_or_else(|| missing("audio state machine", "--audio-sm", "audio_sm"))?;
    let rate_hz = args
        .audio_rate
        .or(profile.audio_rate)
        .unwrap_or(DEFAULT_AUDIO_RATE_HZ);
    let channels = args
        .audio_channels
        .or(profile.audio_channels)
        .unwrap_or(DEFAULT_AUDIO_CHANNELS);
    let capacity = args
        .audio_capacity
        .or(profile.audio_capacity)
        .unwrap_or(DEFAULT_AUDIO_CAPACITY);
    if rate_hz == 0 || channels == 0 || capacity == 0 {
        return Err(
            "audio rate, channels and capacity must be greater than zero / \
             音频采样率、声道数与容量必须大于 0"
                .to_owned(),
        );
    }
    Ok(Some(PlayAudioConfig {
        block,
        sm,
        rate_hz,
        channels,
        capacity,
    }))
}

/// Bilingual "this value is missing" message / 双语「缺少该值」消息
fn missing(what: &str, flag: &str, key: &str) -> String {
    format!(
        "missing {what}: pass `{flag}` or set `{key}` in a --profile / \
         缺少{what}：请传入 `{flag}` 或在 --profile 中设置 `{key}`"
    )
}

#[cfg(test)]
mod tests;
