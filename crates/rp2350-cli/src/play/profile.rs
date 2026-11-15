//! `key = value` profile files for the `play` subcommand
//! `play` 子命令的 `key = value` profile 文件
//!
//! A profile keeps **example-specific** settings (firmware path, frame buffer
//! address/format, button pins, I2S PIO block/state machine/rate) out of crate
//! defaults. The format is deliberately tiny so it needs no dependency: one
//! `key = value` pair per line, `#` starts a whole-line comment, blank lines are
//! ignored, and unknown or duplicated keys are errors with the line number.
//! profile 把**示例专用**设置（固件路径、帧缓冲地址/格式、按键引脚、I2S 的 PIO
//! 块/状态机/采样率）挡在 crate 默认值之外。格式刻意保持极小、不引入依赖：每行
//! 一个 `key = value`，`#` 开头为整行注释，空行忽略；未知键、重复键、缺 `=` 与
//! 非法值都返回带行号的错误。
//!
//! Every field is optional so the caller can merge the profile with command-line
//! arguments (command line wins) and the built-in generic defaults.
//! 所有字段都是可选的，调用方据此把 profile 与命令行参数（命令行优先）以及内置
//! 通用默认值合并。
//!
//! ```text
//! # example profile / 示例 profile
//! firmware = /path/to/firmware.uf2
//! frame = 0x20000000:240x160
//! steps_per_frame = 1000000
//! buttons = 0x3FF
//! audio = true
//! audio_block = 2
//! audio_sm = 0
//! audio_rate = 13440
//! audio_channels = 2
//! audio_capacity = 48000
//! arch = arm
//! strict_decode = false
//! max_cycles = 0
//! ```

use std::collections::HashSet;
use std::fmt;
use std::path::PathBuf;

use super::FrameSpec;
use super::parse_button_mask;
use crate::run::ArchArg;

/// One malformed profile line / 一行 profile 的解析错误
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileError {
    /// 1-based line number of the offending input / 出错输入的 1 基行号
    pub line: usize,
    /// Human-readable reason (bilingual) / 可读原因（双语）
    pub message: String,
}

impl ProfileError {
    /// Build an error for `line` / 为第 `line` 行构造错误
    const fn new(line: usize, message: String) -> Self {
        Self { line, message }
    }
}

impl fmt::Display for ProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "profile line {}: {} / profile 第 {} 行: {}",
            self.line, self.message, self.line, self.message
        )
    }
}

impl std::error::Error for ProfileError {}

/// Parsed profile; every key is optional / 解析后的 profile，所有键可选
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profile {
    /// Firmware image path / 固件映像路径
    pub firmware: Option<PathBuf>,
    /// Guest frame buffer / 客户机帧缓冲
    pub frame: Option<FrameSpec>,
    /// Steps executed per published frame / 每发布一帧执行的步数
    pub steps_per_frame: Option<u64>,
    /// Stop after this many frames / 跑满该帧数后停止
    pub max_frames: Option<u64>,
    /// Button pin bitmap / 按键引脚位图
    pub buttons: Option<u16>,
    /// Enable I2S capture / 启用 I2S 捕获
    pub audio: Option<bool>,
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
    /// CPU architecture override / CPU 架构覆盖
    pub arch: Option<ArchArg>,
    /// Halt on undefined instructions / 未定义指令即停机
    pub strict_decode: Option<bool>,
    /// Total instruction budget / 总指令预算
    pub max_cycles: Option<u64>,
}

/// Parse a profile document / 解析 profile 文本
///
/// Accepts LF and CRLF line endings. Values may contain `=` (only the first
/// `=` separates key and value), and surrounding whitespace around the key,
/// the value and the whole line is ignored.
/// 接受 LF 与 CRLF 行尾。值中可以包含 `=`（只有第一个 `=` 分隔键与值），行首行尾
/// 与键、值两侧的空白都被忽略。
///
/// # Errors
/// Returns [`ProfileError`] with the 1-based line number when a line lacks `=`,
/// names an unknown key, repeats a key already seen, or carries an invalid
/// value for that key.
/// 某行缺少 `=`、键未知、键重复，或值对该键非法时，返回带 1 基行号的
/// [`ProfileError`]。
pub fn parse(text: &str) -> Result<Profile, ProfileError> {
    let mut profile = Profile::default();
    let mut seen: HashSet<&str> = HashSet::new();
    for (index, raw) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(ProfileError::new(
                line_number,
                format!("missing `=` in `{line}` / 缺少 `=`"),
            ));
        };
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() {
            return Err(ProfileError::new(
                line_number,
                "empty key / 键为空".to_owned(),
            ));
        }
        if value.is_empty() {
            return Err(ProfileError::new(
                line_number,
                format!("empty value for `{key}` / 值为空"),
            ));
        }
        if !seen.insert(key) {
            return Err(ProfileError::new(
                line_number,
                format!("duplicate key `{key}` / 重复的键"),
            ));
        }
        assign(&mut profile, key, value, line_number)?;
    }
    Ok(profile)
}

/// Store one validated `key = value` pair / 写入一个已校验的 `key = value`
fn assign(profile: &mut Profile, key: &str, value: &str, line: usize) -> Result<(), ProfileError> {
    match key {
        "firmware" => profile.firmware = Some(PathBuf::from(value)),
        "frame" => {
            profile.frame = Some(FrameSpec::parse(value).map_err(|reason| {
                ProfileError::new(line, format!("bad frame `{value}`: {reason} / 帧缓冲非法"))
            })?);
        }
        "steps_per_frame" => profile.steps_per_frame = Some(parse_u64(value, key, line)?),
        "max_frames" => profile.max_frames = Some(parse_u64(value, key, line)?),
        "buttons" => {
            profile.buttons = Some(parse_button_mask(value).map_err(|reason| {
                ProfileError::new(
                    line,
                    format!("bad buttons `{value}`: {reason} / 按键掩码非法"),
                )
            })?);
        }
        "audio" => profile.audio = Some(parse_bool(value, key, line)?),
        "audio_block" => profile.audio_block = Some(narrow_u64(value, key, line)?),
        "audio_sm" => profile.audio_sm = Some(narrow_u64(value, key, line)?),
        "audio_rate" => profile.audio_rate = Some(narrow_u32(value, key, line)?),
        "audio_channels" => profile.audio_channels = Some(narrow_u8(value, key, line)?),
        "audio_capacity" => profile.audio_capacity = Some(narrow_u64(value, key, line)?),
        "arch" => profile.arch = Some(parse_arch(value, line)?),
        "strict_decode" => profile.strict_decode = Some(parse_bool(value, key, line)?),
        "max_cycles" => profile.max_cycles = Some(parse_u64(value, key, line)?),
        other => {
            return Err(ProfileError::new(
                line,
                format!("unknown key `{other}` / 未知的键"),
            ));
        }
    }
    Ok(())
}

/// Parse a decimal or `0x`-prefixed hexadecimal `u64` / 解析十进制或 `0x` 十六进制 `u64`
fn parse_u64(value: &str, key: &str, line: usize) -> Result<u64, ProfileError> {
    let parsed = if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16)
    } else {
        value.parse()
    };
    parsed.map_err(|_| {
        ProfileError::new(line, format!("bad number `{value}` for `{key}` / 数值非法"))
    })
}

/// Parse a `usize` value / 解析 `usize` 值
fn narrow_u64(value: &str, key: &str, line: usize) -> Result<usize, ProfileError> {
    let parsed = parse_u64(value, key, line)?;
    usize::try_from(parsed).map_err(|_| {
        ProfileError::new(
            line,
            format!("`{value}` does not fit in usize for `{key}` / 超出 usize 范围"),
        )
    })
}

/// Parse a `u32` value / 解析 `u32` 值
fn narrow_u32(value: &str, key: &str, line: usize) -> Result<u32, ProfileError> {
    let parsed = parse_u64(value, key, line)?;
    u32::try_from(parsed).map_err(|_| {
        ProfileError::new(
            line,
            format!("`{value}` does not fit in u32 for `{key}` / 超出 u32 范围"),
        )
    })
}

/// Parse a `u8` value / 解析 `u8` 值
fn narrow_u8(value: &str, key: &str, line: usize) -> Result<u8, ProfileError> {
    let parsed = parse_u64(value, key, line)?;
    u8::try_from(parsed).map_err(|_| {
        ProfileError::new(
            line,
            format!("`{value}` does not fit in u8 for `{key}` / 超出 u8 范围"),
        )
    })
}

/// Parse `true`/`false` (case-insensitive) / 解析 `true`/`false`（忽略大小写）
fn parse_bool(value: &str, key: &str, line: usize) -> Result<bool, ProfileError> {
    match value.to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ProfileError::new(
            line,
            format!("bad boolean `{value}` for `{key}` (expected true/false) / 布尔值非法"),
        )),
    }
}

/// Parse the CPU architecture / 解析 CPU 架构
fn parse_arch(value: &str, line: usize) -> Result<ArchArg, ProfileError> {
    match value.to_ascii_lowercase().as_str() {
        "arm" => Ok(ArchArg::Arm),
        "riscv" | "risc-v" => Ok(ArchArg::RiscV),
        _ => Err(ProfileError::new(
            line,
            format!("bad arch `{value}` (expected arm|riscv) / 架构非法"),
        )),
    }
}

#[cfg(test)]
mod tests;
