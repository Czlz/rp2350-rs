//! Play toolbar fields, their typed form and profile loading
//! 「游戏」工具栏字段、其数值形式与 profile 载入
//!
//! All example-specific values (firmware path, frame buffer, button pins, the
//! I2S PIO block / state machine / rate) are typed by the user here or adopted
//! from a profile file parsed with `rp2350_cli::play::profile`. The crate holds
//! **no** such defaults: the only built-in values describe the host audio sink
//! (48 kHz stereo, 1 s of samples), which is a property of the host, not of a
//! guest firmware.
//! 所有示例专用值（固件路径、帧缓冲、按键引脚、I2S 的 PIO 块/状态机/采样率）都由
//! 用户在这里输入，或从 `rp2350_cli::play::profile` 解析的 profile 文件采用。
//! crate 内**没有**这类默认值：唯一内置值是宿主音频 sink 的属性（48 kHz 立体声、
//! 1 秒样本），它属于宿主而不是客户机固件。
//!
//! Together with [`cli_source`](super::cli_source) this module is one of the two
//! places where the GUI crate touches `rp2350-cli`.
//! 本模块与 [`cli_source`](super::cli_source) 是 GUI crate 接触 `rp2350-cli` 的两个
//! 位置之一。

use rp2350_cli::play::FrameSpec;
use rp2350_cli::play::options::{
    DEFAULT_AUDIO_CAPACITY, DEFAULT_AUDIO_CHANNELS, DEFAULT_AUDIO_RATE_HZ,
};
use rp2350_cli::play::parse_button_mask;
use rp2350_cli::play::profile;
use rp2350_cli::run::{parse_core_threads, parse_core_window};

use super::AudioRequest;

/// Profile key of the threaded core executor / 线程化核执行器的 profile 键
///
/// `rp2350_cli::play::profile` predates this switch, so the GUI removes the two
/// core lines before parsing the rest and applies them to [`PlayFields`] itself.
/// The removed lines become blank, which keeps `profile line N` numbers stable.
/// `rp2350_cli::play::profile` 尚未包含该开关，因此 GUI 先摘出这两行再解析其余
/// 内容，并自行写入 [`PlayFields`]。被摘出的行替换为空行，保持 `profile line N`
/// 行号不变。
const CORE_THREADS_KEY: &str = "core_threads";

/// Profile key of the cross-core window / 跨核窗口的 profile 键
const CORE_WINDOW_KEY: &str = "core_window";

/// Editable text of every Play toolbar field / 「游戏」工具栏所有字段的文本
///
/// The view keeps text, not numbers, so a half-typed value never changes the
/// meaning of a running session. [`PlayFields::inputs`] validates the text only
/// when Start is pressed.
/// 视图保存文本而不是数值，因此输入到一半的值不会影响正在运行的会话。
/// [`PlayFields::inputs`] 只在按下 Start 时校验文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayFields {
    /// Profile file path (`--profile` equivalent) / profile 文件路径
    pub profile: String,
    /// Firmware path / 固件路径
    pub firmware: String,
    /// Frame buffer text (`ADDR:WxH[:STRIDE]`) / 帧缓冲文本
    pub frame: String,
    /// Steps executed per published frame / 每发布一帧执行的步数
    pub steps_per_frame: String,
    /// Button pin bitmap / 按键引脚位图
    pub buttons: String,
    /// Whether I2S capture is requested / 是否请求 I2S 捕获
    pub audio: bool,
    /// PIO block that pushes I2S words / 推送 I2S 字的 PIO 块
    pub audio_block: String,
    /// State machine inside that PIO block / 该 PIO 块内的状态机
    pub audio_sm: String,
    /// Sample rate written into the audio sink / 写入音频 sink 的采样率
    pub audio_rate: String,
    /// Channel count written into the audio sink / 写入音频 sink 的声道数
    pub audio_channels: String,
    /// Samples the audio sink retains / 音频 sink 保留的样本数
    pub audio_capacity: String,
    /// Whether core0/core1 run on separate host threads
    /// 是否让 core0/core1 运行在独立宿主线程上
    pub core_threads: bool,
    /// Machine steps per cross-core window (`1..=4096`) / 每个跨核窗口的机器步数
    pub core_window: String,
}

impl Default for PlayFields {
    fn default() -> Self {
        // 没有固件、帧缓冲、每帧步数或按键映射：它们必须由用户或 profile 给出。
        // No firmware, frame buffer, steps per frame or button map: the user or a
        // profile must supply them.
        Self {
            profile: String::new(),
            firmware: String::new(),
            frame: String::new(),
            steps_per_frame: String::new(),
            buttons: String::new(),
            audio: false,
            audio_block: String::new(),
            audio_sm: String::new(),
            audio_rate: DEFAULT_AUDIO_RATE_HZ.to_string(),
            audio_channels: DEFAULT_AUDIO_CHANNELS.to_string(),
            audio_capacity: DEFAULT_AUDIO_CAPACITY.to_string(),
            // 默认关闭：单线程交错是确定性的；窗口默认值必须与 CLI 一致。
            // Off by default: the single-threaded interleaving is deterministic;
            // the window default matches the CLI.
            core_threads: false,
            core_window: "64".to_owned(),
        }
    }
}

/// Validated values of the toolbar fields / 工具栏字段的校验结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayInputs {
    /// Firmware path as typed (trimmed) / 用户输入的固件路径（去空白）
    pub firmware: String,
    /// Frame buffer text as typed (trimmed) / 用户输入的帧缓冲文本（去空白）
    pub frame: String,
    /// Steps executed per published frame / 每发布一帧执行的步数
    pub steps_per_frame: u64,
    /// Button pin bitmap (`0` drives nothing) / 按键引脚位图（`0` 不驱动任何引脚）
    pub buttons: u16,
    /// Whether core0/core1 run on separate host threads
    /// 是否让 core0/core1 运行在独立宿主线程上
    pub core_threads: bool,
    /// Machine steps per cross-core window / 每个跨核窗口的机器步数
    pub core_window: u32,
    /// Audio wiring, `None` when capture is off / 音频接线；不捕获时为 `None`
    pub audio: Option<AudioRequest>,
}

impl PlayFields {
    /// Validate the text fields into typed inputs / 把文本字段校验为数值输入
    ///
    /// # Errors
    /// Returns a bilingual message naming the first missing or malformed field:
    /// `firmware`, `frame`, `steps per frame`, `buttons`, `core window` or the
    /// audio wiring.
    /// 返回指出第一个缺失或非法字段（`firmware`、`frame`、每帧步数、按键掩码、
    /// 核窗口或音频接线）的双语消息。
    pub fn inputs(&self) -> Result<PlayInputs, String> {
        let firmware = self.firmware.trim();
        if firmware.is_empty() {
            return Err(
                "missing firmware: type a path or load a profile / 缺少固件：请输入路径或载入 profile"
                    .to_owned(),
            );
        }
        let frame = self.frame.trim();
        if frame.is_empty() {
            return Err(
                "missing frame buffer: type ADDR:WxH[:STRIDE] or load a profile / \
                 缺少帧缓冲：请输入 ADDR:WxH[:STRIDE] 或载入 profile"
                    .to_owned(),
            );
        }
        FrameSpec::parse(frame)
            .map_err(|reason| format!("bad frame `{frame}`: {reason} / 帧缓冲非法"))?;
        let steps_per_frame = Self::number::<u64>("steps per frame", &self.steps_per_frame)?;
        if steps_per_frame == 0 {
            return Err("steps per frame must be positive / 每帧步数必须大于 0".to_owned());
        }
        let buttons = if self.buttons.trim().is_empty() {
            // 空掩码 = 不驱动任何引脚，这是通用默认而不是隐藏映射。
            // An empty mask drives no pin: a generic default, not a hidden map.
            0
        } else {
            parse_button_mask(self.buttons.trim())
                .map_err(|reason| format!("buttons / 按键掩码: {reason}"))?
        };
        // 窗口必须始终合法（即使线程模式关闭），与 CLI 的 `--core-window` 一致。
        // The window must always be valid (even with threading off), matching the
        // CLI's `--core-window`.
        let core_window_text = self.core_window.trim();
        if core_window_text.is_empty() {
            return Err("missing core window: fill the field or load a profile / \
                 缺少核窗口：请填写输入框或载入 profile"
                .to_owned());
        }
        let core_window = parse_core_window(core_window_text)?;
        let audio = self.audio.then(|| self.audio_request()).transpose()?;
        Ok(PlayInputs {
            firmware: firmware.to_owned(),
            frame: frame.to_owned(),
            steps_per_frame,
            buttons,
            core_threads: self.core_threads,
            core_window,
            audio,
        })
    }

    /// Load the profile named by [`PlayFields::profile`] into the other fields
    /// 把 [`PlayFields::profile`] 指定的 profile 载入其它字段
    ///
    /// Only the keys the profile sets are overwritten. The frame text is rebuilt
    /// as `0xADDR:WxH[:STRIDE]` and the button mask as `0xNNN`, so a reload
    /// produces the same text. The two core-executor keys (`core_threads`,
    /// `core_window`) are read by this method itself because the CLI profile
    /// schema predates them. Keys without a toolbar field (`max_frames`,
    /// `arch`, `strict_decode`, `max_cycles`) are ignored: the GUI worker always
    /// runs unbudgeted with the architecture of the image.
    /// 只覆盖 profile 给出的键。帧文本重建为 `0xADDR:WxH[:STRIDE]`、按键掩码重建为
    /// `0xNNN`，因此重复载入得到相同文本。两个核执行器键（`core_threads`、
    /// `core_window`）由本方法自行读取，因为 CLI 的 profile 模式尚早于它们。没有
    /// 对应输入框的键（`max_frames`、`arch`、`strict_decode`、`max_cycles`）被
    /// 忽略：GUI worker 始终不设预算，并按映像自身选择架构。
    ///
    /// # Errors
    /// Returns a message when the path is empty, the file cannot be read, the
    /// profile is malformed (the parser message already carries `profile line N`),
    /// or a core-executor value is invalid.
    /// 路径为空、文件无法读取、profile 非法（解析器消息已带 `profile line N`）或
    /// 核执行器值非法时返回消息。
    pub fn load_profile(&mut self) -> Result<usize, String> {
        let path = self.profile.trim();
        if path.is_empty() {
            return Err("profile path is empty / profile 路径为空".to_owned());
        }
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read profile / 无法读取 profile {path}: {error}"))?;
        // 先摘出 GUI 自己解析的核执行器键，其余内容仍交给 CLI 的 profile 解析器。
        // Remove the core-executor keys this GUI parses itself; the rest still
        // goes through the CLI profile parser.
        let (text, core_threads, core_window) = extract_core_keys(&text)?;
        // 解析器的 `Display` 已是 `profile line N: ...` 形式，直接作为状态消息。
        // The parser's `Display` already reads `profile line N: ...`, which is the
        // status message we want.
        let profile = profile::parse(&text).map_err(|error| error.to_string())?;
        let mut applied = 0;
        if let Some(firmware) = profile.firmware {
            self.firmware = firmware.display().to_string();
            applied += 1;
        }
        if let Some(frame) = profile.frame {
            self.frame = format_frame(frame);
            applied += 1;
        }
        if let Some(steps) = profile.steps_per_frame {
            self.steps_per_frame = steps.to_string();
            applied += 1;
        }
        if let Some(buttons) = profile.buttons {
            self.buttons = format!("0x{buttons:03X}");
            applied += 1;
        }
        if let Some(audio) = profile.audio {
            self.audio = audio;
            applied += 1;
        }
        if let Some(block) = profile.audio_block {
            self.audio_block = block.to_string();
            applied += 1;
        }
        if let Some(sm) = profile.audio_sm {
            self.audio_sm = sm.to_string();
            applied += 1;
        }
        if let Some(rate) = profile.audio_rate {
            self.audio_rate = rate.to_string();
            applied += 1;
        }
        if let Some(channels) = profile.audio_channels {
            self.audio_channels = channels.to_string();
            applied += 1;
        }
        if let Some(capacity) = profile.audio_capacity {
            self.audio_capacity = capacity.to_string();
            applied += 1;
        }
        if let Some(enabled) = core_threads {
            self.core_threads = enabled;
            applied += 1;
        }
        if let Some(window) = core_window {
            self.core_window = window.to_string();
            applied += 1;
        }
        Ok(applied)
    }

    /// Typed audio wiring of the audio fields / 音频字段的数值接线
    ///
    /// # Errors
    /// Returns a message when a field is missing, not an integer of the right
    /// width, or zero where a positive value is required.
    /// 字段缺失、不是对应宽度的整数，或本应为正的值为 0 时返回消息。
    fn audio_request(&self) -> Result<AudioRequest, String> {
        let block = Self::number::<usize>("audio PIO block", &self.audio_block)?;
        let sm = Self::number::<usize>("audio state machine", &self.audio_sm)?;
        let rate_hz = Self::number::<u32>("audio rate", &self.audio_rate)?;
        let channels = Self::number::<u8>("audio channels", &self.audio_channels)?;
        let capacity = Self::number::<usize>("audio capacity", &self.audio_capacity)?;
        if rate_hz == 0 || channels == 0 || capacity == 0 {
            return Err(
                "audio rate, channels and capacity must be greater than zero / \
                 音频采样率、声道数与容量必须大于 0"
                    .to_owned(),
            );
        }
        Ok(AudioRequest {
            block,
            sm,
            rate_hz,
            channels,
            capacity,
        })
    }

    /// Parse one required integer field / 解析一个必需的整数字段
    ///
    /// # Errors
    /// Returns a bilingual message for an empty value or a parse failure.
    /// 值为空或解析失败时返回双语消息。
    fn number<T: std::str::FromStr>(what: &str, text: &str) -> Result<T, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err(format!(
                "missing {what}: fill the field or load a profile / \
                 缺少{what}：请填写输入框或载入 profile"
            ));
        }
        text.parse().map_err(|_| {
            format!("bad {what} `{text}`: expected an integer / {what} 非法：需要整数")
        })
    }
}

/// Split the core-executor keys out of a profile document
/// 从 profile 文本中摘出核执行器键
///
/// Returns the remaining text (with the two keys blanked so line numbers stay
/// stable) plus the parsed values. Duplicate, empty or invalid values are
/// reported with the same `profile line N` shape as the CLI parser.
/// 返回剩余文本（两个键被置空以保持行号稳定）与解析结果。重复、空值或非法值都按
/// CLI 解析器相同的 `profile line N` 形式报告。
fn extract_core_keys(text: &str) -> Result<(String, Option<bool>, Option<u32>), String> {
    let mut core_threads = None;
    let mut core_window = None;
    let mut rest = String::with_capacity(text.len());
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let mut keep = true;
        let pair = (!line.is_empty() && !line.starts_with('#'))
            .then(|| line.split_once('='))
            .flatten();
        if let Some((key, value)) = pair {
            let key = key.trim();
            if key == CORE_THREADS_KEY || key == CORE_WINDOW_KEY {
                keep = false;
                let number = index + 1;
                let value = value.trim();
                if value.is_empty() {
                    return Err(profile_error(
                        number,
                        &format!("empty value for `{key}` / 值为空"),
                    ));
                }
                if key == CORE_THREADS_KEY {
                    if core_threads.is_some() {
                        return Err(profile_error(
                            number,
                            &format!("duplicate key `{key}` / 重复的键"),
                        ));
                    }
                    core_threads = Some(
                        parse_core_threads(value)
                            .map_err(|message| profile_error(number, &message))?,
                    );
                } else {
                    if core_window.is_some() {
                        return Err(profile_error(
                            number,
                            &format!("duplicate key `{key}` / 重复的键"),
                        ));
                    }
                    core_window = Some(
                        parse_core_window(value)
                            .map_err(|message| profile_error(number, &message))?,
                    );
                }
            }
        }
        if keep {
            rest.push_str(raw);
        }
        // 被摘出的行留空，保持后续行的行号。 / Blank removed lines to keep numbers.
        rest.push('\n');
    }
    Ok((rest, core_threads, core_window))
}

/// Render a profile error exactly like `rp2350_cli`'s parser does
/// 按 `rp2350_cli` 解析器的格式渲染 profile 错误
fn profile_error(line: usize, message: &str) -> String {
    profile::ProfileError {
        line,
        message: message.to_owned(),
    }
    .to_string()
}

/// Render a frame specification as `0xADDR:WxH[:STRIDE]`
/// 把帧规格渲染为 `0xADDR:WxH[:STRIDE]`
///
/// The trailing stride is omitted for packed RGB565 (`stride == width * 2`), so
/// a round trip through [`PlayFields`] keeps the text stable.
/// 紧凑 RGB565（`stride == width * 2`）时省略跨度，因此经由 [`PlayFields`] 往返后
/// 文本保持稳定。
#[must_use]
pub fn format_frame(frame: FrameSpec) -> String {
    let packed = u32::from(frame.width) * 2;
    if frame.stride_bytes == packed {
        format!("0x{:08X}:{}x{}", frame.base, frame.width, frame.height)
    } else {
        format!(
            "0x{:08X}:{}x{}:{}",
            frame.base, frame.width, frame.height, frame.stride_bytes
        )
    }
}
