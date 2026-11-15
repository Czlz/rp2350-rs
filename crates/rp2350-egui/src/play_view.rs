//! Play tab: live RGB565 frames from a play session plus keyboard input
//! 「游戏」标签页：游玩会话的实时 RGB565 画面与键盘输入
//!
//! [`PlayView`] is the GUI end of the play pipeline: it owns a [`PlaySource`]
//! (a background worker that publishes guest frames and accepts a button mask),
//! uploads each new frame as an egui texture, paints it centred with an integer
//! zoom that keeps the guest aspect ratio, and maps the keyboard onto a button
//! bitmap.
//! [`PlayView`] 是游玩流水线的 GUI 端：持有一个 [`PlaySource`]（在后台发布客户机
//! 帧、接收按键掩码的 worker），把每个新帧上传为 egui 纹理，按整数倍缩放、保持
//! 客户机宽高比居中绘制，并把键盘映射为按键位图。
//!
//! The toolbar is **profile driven**: the firmware path, frame buffer, steps per
//! frame, button mask and the I2S wiring are typed by the user or loaded from a
//! `key = value` profile (`profile` field + `Load profile`), so no example
//! constant lives in this crate. See [`fields`] for the text/typed split and
//! [`types`] for the shared types.
//! 工具栏由 **profile 驱动**：固件路径、帧缓冲、每帧步数、按键掩码与 I2S 接线都由
//! 用户输入或从 `key = value` profile 载入（`profile` 输入框 + `Load profile`），
//! 因此 crate 内没有示例常量。文本与数值的拆分见 [`fields`]，共享类型见
//! [`types`]。
//!
//! The view itself only talks to the [`PlaySource`] / [`PlayStarter`] traits, so
//! tests and alternate hosts can inject a fake worker. The concrete worker is
//! `rp2350_cli::play::PlaySession`, bridged in [`cli_source`] and attached by the
//! native entry point; the dependency direction is `rp2350-egui → rp2350-cli`.
//! 视图本身只与 [`PlaySource`] / [`PlayStarter`] trait 交互，因此测试与其它宿主
//! 可以注入假 worker。具体 worker 为 `rp2350_cli::play::PlaySession`，由
//! [`cli_source`] 桥接并由原生入口挂载；依赖方向为 `rp2350-egui → rp2350-cli`。
//!
//! Host audio is optional and decoupled: when `Audio / 音频` is ticked before a
//! start, the view pulls the worker's I2S tap through
//! [`PlaySource::audio_source`] and hands it to `rp2350_audio::AudioOutput`
//! (see [`audio`]). The output thread is joined **before** the session drops, so
//! it never reads a released tap.
//! 宿主音频可选且解耦：启动前勾选「Audio / 音频」，视图就通过
//! [`PlaySource::audio_source`] 取走 worker 的 I2S tap 并交给
//! `rp2350_audio::AudioOutput`（见 [`audio`]）。音频线程严格在会话丢弃**之前**
//! join，因此绝不会读取已释放的 tap。
//!
//! Pixel math, key mapping and fitting are free functions so they can be tested
//! without a window; [`PlayView::ui`] only wires them together.
//! 像素换算、按键映射与缩放都是自由函数，无需窗口即可测试；[`PlayView::ui`] 只
//! 负责把它们串起来。

// RGB565 到 8 位通道的位复制会产生截断，但目标通道本来就是 8 位。
// Copying RGB565 bits into an 8-bit channel truncates by design: the target
// channel is 8 bits wide.
#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

pub mod audio;
#[cfg(test)]
mod audio_tests;
pub mod cli_source;
pub mod fields;
#[cfg(test)]
mod params_tests;
#[cfg(test)]
mod tests;
pub mod types;

pub use audio::{AUDIO_CAPACITY, AUDIO_NULL, SILENT_NOTICE, format_audio};
pub use cli_source::{CliPlaySource, CliPlayStarter};
pub use fields::{PlayFields, PlayInputs, format_frame};
pub use types::{
    AudioRequest, PlayFrame, PlayRequest, PlaySource, PlayStarter, PlayStats, format_stats,
};

use egui::{
    Align2, Color32, ColorImage, FontId, Key, Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle,
    TextureOptions, Vec2, pos2, vec2,
};
use rp2350_audio::AudioOutput;

/// Height reserved for the three-line status bar / 三行状态栏保留的高度
const STATUS_HEIGHT: f32 = 64.0;

/// Screen background behind the guest picture / 客户机画面背后的底色
const BACKDROP: Color32 = Color32::from_rgb(0x12, 0x14, 0x18);

/// Hint text colour / 提示文字颜色
const HINT: Color32 = Color32::from_rgb(0x8A, 0x94, 0xA6);

/// Outline of the focused screen region / 获得焦点的画面区域描边
const FOCUS: Color32 = Color32::from_rgb(0x6E, 0xC1, 0xFF);

/// Texture name shown in debug tools / 调试工具显示的纹理名
const TEXTURE_NAME: &str = "rp2350-play";

/// Button bit of `A` = GP0 / 按键 `A` 的位 = GP0
pub const BUTTON_A: u16 = 1 << 0;
/// Button bit of `B` = GP1 / 按键 `B` 的位 = GP1
pub const BUTTON_B: u16 = 1 << 1;
/// Button bit of `Select` = GP2 / 按键 `Select` 的位 = GP2
pub const BUTTON_SELECT: u16 = 1 << 2;
/// Button bit of `Start` = GP3 / 按键 `Start` 的位 = GP3
pub const BUTTON_START: u16 = 1 << 3;
/// Button bit of `Right` = GP4 / 按键 `Right` 的位 = GP4
pub const BUTTON_RIGHT: u16 = 1 << 4;
/// Button bit of `Left` = GP5 / 按键 `Left` 的位 = GP5
pub const BUTTON_LEFT: u16 = 1 << 5;
/// Button bit of `Up` = GP6 / 按键 `Up` 的位 = GP6
pub const BUTTON_UP: u16 = 1 << 6;
/// Button bit of `Down` = GP7 / 按键 `Down` 的位 = GP7
pub const BUTTON_DOWN: u16 = 1 << 7;
/// Button bit of `R` = GP8 / 按键 `R` 的位 = GP8
pub const BUTTON_R: u16 = 1 << 8;
/// Button bit of `L` = GP9 / 按键 `L` 的位 = GP9
pub const BUTTON_L: u16 = 1 << 9;

/// Pins the example firmware reads as buttons (GP0..GP9) / 示例固件读取的按键引脚
pub const GBA_BUTTONS: u16 = BUTTON_A
    | BUTTON_B
    | BUTTON_SELECT
    | BUTTON_START
    | BUTTON_RIGHT
    | BUTTON_LEFT
    | BUTTON_UP
    | BUTTON_DOWN
    | BUTTON_R
    | BUTTON_L;

/// Button names with their pin bit, in GP order / 按键名与其引脚位（按 GP 顺序）
pub const BUTTON_NAMES: [(&str, u16); 10] = [
    ("A", BUTTON_A),
    ("B", BUTTON_B),
    ("Select", BUTTON_SELECT),
    ("Start", BUTTON_START),
    ("Right", BUTTON_RIGHT),
    ("Left", BUTTON_LEFT),
    ("Up", BUTTON_UP),
    ("Down", BUTTON_DOWN),
    ("R", BUTTON_R),
    ("L", BUTTON_L),
];

/// Convert one little-endian RGB565 pixel to a colour / 把一个小端 RGB565 像素转成颜色
///
/// The 5/6/5-bit channels are expanded by copying their high bits into the low
/// ones (`(r << 3) | (r >> 2)`), so full-scale `5`/`6`-bit values reach `255`.
/// 5/6/5 位通道用高位复制扩展到 8 位（`(r << 3) | (r >> 2)`），因此满量程的
/// `5`/`6` 位值正好到达 `255`。
#[must_use]
pub fn rgb565_to_color32(pixel: u16) -> Color32 {
    let red = (pixel >> 11) & 0x1F;
    let green = (pixel >> 5) & 0x3F;
    let blue = pixel & 0x1F;
    Color32::from_rgb(
        ((red << 3) | (red >> 2)) as u8,
        ((green << 2) | (green >> 4)) as u8,
        ((blue << 3) | (blue >> 2)) as u8,
    )
}

/// Build a `width × height` opaque image from row-major RGB565 pixels
/// 用行优先 RGB565 像素构造 `width × height` 不透明图像
///
/// A short slice is padded with black instead of panicking, so a frame that
/// lost its tail (unmapped guest memory) still renders its readable prefix.
/// 切片过短时用黑色补齐而不是 panic，因此尾部缺失（客户机内存未映射）的帧仍会
/// 渲染可读前缀。
#[must_use]
pub fn color_image(width: u16, height: u16, pixels: &[u16]) -> ColorImage {
    let width = usize::from(width);
    let height = usize::from(height);
    let count = width * height;
    let mut colors = Vec::with_capacity(count);
    colors.extend(
        pixels
            .iter()
            .take(count)
            .map(|pixel| rgb565_to_color32(*pixel)),
    );
    colors.resize(count, Color32::BLACK);
    ColorImage::new([width, height], colors)
}

/// Keyboard state of the ten GBA buttons / GBA 十个按键的键盘状态
// 十个按键本来就是十个独立布尔值，拆成枚举反而更难与硬件位图对应。
// Ten buttons are ten independent flags; splitting them into enums would make
// the mapping onto the hardware bitmap harder to read, not easier.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyState {
    /// `Up` arrow / 上方向键
    pub up: bool,
    /// `Down` arrow / 下方向键
    pub down: bool,
    /// `Left` arrow / 左方向键
    pub left: bool,
    /// `Right` arrow / 右方向键
    pub right: bool,
    /// `Z` / `Z` 键
    pub a: bool,
    /// `X` / `X` 键
    pub b: bool,
    /// `Enter` / 回车键
    pub start: bool,
    /// `Backspace` / 退格键
    pub select: bool,
    /// `S` / `S` 键
    pub r: bool,
    /// `A` / `A` 键
    pub l: bool,
}

impl KeyState {
    /// Sample the state through a predicate / 通过谓词采样按键状态
    #[must_use]
    pub fn from_pressed(pressed: impl Fn(Key) -> bool) -> Self {
        Self {
            up: pressed(Key::ArrowUp),
            down: pressed(Key::ArrowDown),
            left: pressed(Key::ArrowLeft),
            right: pressed(Key::ArrowRight),
            a: pressed(Key::Z),
            b: pressed(Key::X),
            start: pressed(Key::Enter),
            select: pressed(Key::Backspace),
            r: pressed(Key::S),
            l: pressed(Key::A),
        }
    }

    /// Sample the live egui input state / 采样 egui 的实时输入状态
    #[must_use]
    pub fn from_input(input: &egui::InputState) -> Self {
        Self::from_pressed(|key| input.key_down(key))
    }

    /// Button bitmap for this state / 该状态对应的按键位图
    #[must_use]
    pub fn mask(self) -> u16 {
        mask_from_keys(&self)
    }
}

/// Map a keyboard state onto the `GP0..GP9` button bitmap / 把键盘状态映射为 GP0..GP9 按键位图
///
/// `A`=GP0, `B`=GP1, `Select`=GP2, `Start`=GP3, `Right`=GP4, `Left`=GP5,
/// `Up`=GP6, `Down`=GP7, `R`=GP8, `L`=GP9 — the layout of the GBA example
/// firmware (`gpio_get_all() & 0x3FF`, pressed = low).
/// 与 GBA 示例固件一致（`gpio_get_all() & 0x3FF`，按下为低电平）。
#[must_use]
pub fn mask_from_keys(keys: &KeyState) -> u16 {
    let mut mask = 0u16;
    for (pressed, bit) in [
        (keys.a, BUTTON_A),
        (keys.b, BUTTON_B),
        (keys.select, BUTTON_SELECT),
        (keys.start, BUTTON_START),
        (keys.right, BUTTON_RIGHT),
        (keys.left, BUTTON_LEFT),
        (keys.up, BUTTON_UP),
        (keys.down, BUTTON_DOWN),
        (keys.r, BUTTON_R),
        (keys.l, BUTTON_L),
    ] {
        if pressed {
            mask |= bit;
        }
    }
    mask
}

/// Names of the pressed buttons (`none` when idle) / 已按下按键名（空闲时 `none`）
#[must_use]
pub fn describe_mask(mask: u16) -> String {
    let names: Vec<&str> = BUTTON_NAMES
        .iter()
        .filter_map(|(name, bit)| (mask & bit != 0).then_some(*name))
        .collect();
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join("|")
    }
}

/// Largest centred size that fits `available`, integer-zoomed when possible
/// 在 `available` 内的最大居中尺寸，能整数倍时按整数倍放大
///
/// Up-scaling uses whole pixels (`3×`, `4×`, …) so guest pixels stay crisp;
/// when the window is smaller than the frame the scale is fractional and the
/// picture is down-scaled instead of disappearing. A zero-size area yields a
/// zero-size result, which the caller must skip.
/// 放大时使用整像素倍率（`3×`、`4×`…），客户机像素保持锐利；窗口小于画面时改用
/// 小数倍率缩小，而不是让画面消失。可用尺寸为零时返回零尺寸，由调用方跳过绘制。
#[must_use]
pub fn fit_size(available: Vec2, width: usize, height: usize) -> Vec2 {
    if available.x < 1.0 || available.y < 1.0 || width == 0 || height == 0 {
        return Vec2::ZERO;
    }
    let width = width as f32;
    let height = height as f32;
    let scale = (available.x / width).min(available.y / height);
    if !scale.is_finite() || scale <= 0.0 {
        return Vec2::ZERO;
    }
    // 能放大时取整数倍；缩小时保留小数倍率，否则画面会完全消失。
    // Whole-pixel zoom when up-scaling; keep the fraction when down-scaling,
    // otherwise the picture would vanish.
    let scale = if scale >= 1.0 { scale.floor() } else { scale };
    vec2(width * scale, height * scale)
}

/// The "Play / 游戏" tab / 「游戏」标签页
///
/// Owns the optional worker, the uploaded texture and the toolbar text fields.
/// Dropping the view drops the worker, which stops its thread (the sources are
/// documented to join on drop), so closing the window cannot leak a spinner.
/// 持有可选的 worker、已上传纹理与工具栏文本框。丢弃视图即丢弃 worker（来源实现
/// 需在 drop 时 join），因此关闭窗口不会遗留后台线程。
// 视图状态就是几个独立布尔开关（焦点提示、焦点、静默提示），拆成枚举只会增加
// 状态机噪音，不提升可读性。
// The view state is a handful of independent flags (focus hint, focus, silent
// notice); an enum would add state-machine noise without clarity.
#[allow(clippy::struct_excessive_bools)]
pub struct PlayView {
    /// Running worker, if any / 运行中的 worker（若有）
    session: Option<Box<dyn PlaySource>>,
    /// Factory that boots a worker / 启动 worker 的工厂
    starter: Option<Box<dyn PlayStarter>>,
    /// Uploaded guest texture / 已上传的客户机纹理
    texture: Option<TextureHandle>,
    /// Frame index currently in `texture` / 当前纹理中的帧序号
    last_frame: u64,
    /// Last statistics snapshot shown by the status bar / 状态栏显示的最近统计快照
    stats: PlayStats,
    /// Toolbar text fields (profile, firmware, frame, buttons, audio)
    /// 工具栏文本字段（profile、固件、帧缓冲、按键、音频）
    fields: PlayFields,
    /// Running host audio output, if any / 运行中的宿主音频输出（若有）
    audio_output: Option<AudioOutput>,
    /// Whether the silent-backend notice was already appended / 静默后端提示是否已追加
    audio_notice_shown: bool,
    /// Toolbar status of the last action / 最近一次操作的状态
    status: String,
    /// Current button mask / 当前按键掩码
    buttons: u16,
    /// Whether the keyboard hint is still shown / 是否仍显示键盘提示
    focus_hint: bool,
    /// Whether the picture region owns the keyboard / 画面区域是否拥有键盘焦点
    focused: bool,
    /// Number of texture uploads (diagnostics and tests) / 纹理上传次数（诊断与测试）
    uploads: u64,
}

impl Default for PlayView {
    fn default() -> Self {
        Self {
            session: None,
            starter: None,
            texture: None,
            last_frame: 0,
            stats: PlayStats::default(),
            fields: PlayFields::default(),
            audio_output: None,
            audio_notice_shown: false,
            status: "idle / 空闲".to_owned(),
            buttons: 0,
            focus_hint: true,
            focused: false,
            uploads: 0,
        }
    }
}

impl PlayView {
    /// Create an idle view / 创建一个空闲视图
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach the worker factory / 挂载 worker 工厂
    #[must_use]
    pub fn with_starter(mut self, starter: Box<dyn PlayStarter>) -> Self {
        self.starter = Some(starter);
        self
    }

    /// Replace the worker factory / 替换 worker 工厂
    pub fn set_starter(&mut self, starter: Box<dyn PlayStarter>) {
        self.starter = Some(starter);
    }

    /// Whether a worker is attached and still running / worker 是否已挂载且仍在运行
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.is_running())
    }

    /// Toolbar status text / 工具栏状态文本
    #[must_use]
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Current button mask / 当前按键掩码
    #[must_use]
    pub fn buttons(&self) -> u16 {
        self.buttons
    }

    /// Number of texture uploads since start / 启动以来的纹理上传次数
    #[must_use]
    pub fn uploads(&self) -> u64 {
        self.uploads
    }

    /// Whether the `Audio / 音频` box is ticked / 是否勾选「Audio / 音频」
    #[must_use]
    pub fn audio_enabled(&self) -> bool {
        self.fields.audio
    }

    /// Tick or untick audio capture / 勾选或取消音频捕获
    ///
    /// Audio is fixed per session: the call is ignored while a worker runs, so
    /// the checkbox can never disagree with the worker that is already playing.
    /// 音频按会话固定：worker 运行中调用会被忽略，因此复选框不会与已在播放的
    /// worker 冲突。
    pub fn set_audio(&mut self, enabled: bool) {
        if !self.is_running() {
            self.fields.audio = enabled;
        }
    }

    /// Toolbar fields, for hosts that prefill them / 工具栏字段，供宿主预填
    #[must_use]
    pub fn fields(&self) -> &PlayFields {
        &self.fields
    }

    /// Mutable toolbar fields / 可变的工具栏字段
    pub fn fields_mut(&mut self) -> &mut PlayFields {
        &mut self.fields
    }

    /// Active host audio backend (`"cpal"` / `"null"`), `None` when silent
    /// 当前宿主音频后端（`"cpal"` / `"null"`），无输出时为 `None`
    #[must_use]
    pub fn audio_backend(&self) -> Option<&'static str> {
        self.audio_output.as_ref().map(AudioOutput::backend)
    }

    /// Playback underruns counted by the host output / 宿主输出统计的欠载次数
    #[must_use]
    pub fn audio_underruns(&self) -> u64 {
        self.audio_output.as_ref().map_or(0, AudioOutput::underruns)
    }

    /// Whether the host audio worker still runs / 宿主音频工作线程是否仍在运行
    #[must_use]
    pub fn audio_playing(&self) -> bool {
        self.audio_output.as_ref().is_some_and(AudioOutput::playing)
    }

    /// Start the host output for the attached session when audio is enabled
    /// 勾选音频时，为已挂载的会话启动宿主输出
    ///
    /// Runs only when the checkbox is ticked **and** the worker published a tap;
    /// a worker without audio capture stays silent without an output.
    /// 仅在勾选复选框**且** worker 发布了 tap 时启动；无音频捕获的 worker 不会创建
    /// 输出，保持静默。
    fn start_audio(&mut self) {
        self.stop_audio();
        if !self.fields.audio {
            return;
        }
        let Some(source) = self
            .session
            .as_ref()
            .and_then(|session| session.audio_source())
        else {
            return;
        };
        let output = audio::start(source);
        self.note_silent_backend(output.backend());
        self.audio_output = Some(output);
    }

    /// Stop and join the host audio worker / 停止并 join 宿主音频工作线程
    ///
    /// Must run before the session is dropped: the output thread would otherwise
    /// keep pulling from a tap owned by the released session.
    /// 必须在会话丢弃前运行：否则输出线程会继续从已释放会话的 tap 取样本。
    fn stop_audio(&mut self) {
        if let Some(output) = self.audio_output.take() {
            output.shutdown();
        }
    }

    /// Append the silent-backend notice to the toolbar status at most once
    /// 向工具栏状态最多追加一次静默后端提示
    fn note_silent_backend(&mut self, backend: &str) {
        if self.audio_notice_shown || backend != AUDIO_NULL {
            return;
        }
        self.audio_notice_shown = true;
        self.status.push_str(SILENT_NOTICE);
    }

    /// Boot a worker from the toolbar fields / 用工具栏字段启动 worker
    ///
    /// Validates the text fields ([`PlayFields::inputs`]), reads the firmware
    /// file, then hands the request to the attached starter. Every failure is
    /// written to the toolbar status: this method never panics and never blocks
    /// the UI thread on the guest.
    /// 先校验文本字段（[`PlayFields::inputs`]）、读取固件文件，再把请求交给已挂载的
    /// 工厂。所有失败都写入工具栏状态：本方法不 panic，也不会在 UI 线程上阻塞等待
    /// 客户机。
    pub fn start(&mut self) {
        let inputs = match self.fields.inputs() {
            Ok(inputs) => inputs,
            Err(message) => {
                self.status = message;
                return;
            }
        };
        let image = match std::fs::read(&inputs.firmware) {
            Ok(image) => image,
            Err(error) => {
                self.status = format!("cannot read firmware / 无法读取固件: {error}");
                return;
            }
        };
        let Some(starter) = self.starter.as_ref() else {
            "no play source attached / 未挂载游玩来源（宿主需实现 PlayStarter）"
                .clone_into(&mut self.status);
            return;
        };
        let request = PlayRequest {
            image,
            frame: inputs.frame.clone(),
            steps_per_frame: inputs.steps_per_frame,
            audio: inputs.audio,
            button_pins: inputs.buttons,
        };
        match starter.start(request) {
            Ok(session) => {
                // 状态行先就位，`attach` 里的音频静默提示才不会被覆盖。
                // Set the status line first so the audio notice from `attach`
                // is not overwritten.
                self.status = format!(
                    "running / 运行中 · {} · steps/frame / 每帧步数 {}",
                    inputs.frame, inputs.steps_per_frame
                );
                self.attach(session);
            }
            Err(message) => self.status = format!("start failed / 启动失败: {message}"),
        }
    }

    /// Load the profile named by the toolbar into the fields
    /// 载入工具栏指定的 profile 到各字段
    ///
    /// A malformed profile writes the parser message (`profile line N: ...`)
    /// into the status line and leaves every field untouched; the load never
    /// starts a session and never panics.
    /// 非法的 profile 把解析器消息（`profile line N: ...`）写入状态行，字段保持不变；
    /// 载入本身不会启动会话，也不会 panic。
    fn load_profile(&mut self) {
        match self.fields.load_profile() {
            Ok(applied) => {
                self.status = format!("profile loaded / 已载入 · {applied} values / 项");
            }
            Err(message) => self.status = format!("profile failed / 载入失败 — {message}"),
        }
    }

    /// Attach an already-running worker / 挂载已在运行的 worker
    ///
    /// Used by hosts that boot the worker themselves (for example with custom
    /// machine options) and by tests. The frame counter, mask and texture are
    /// reset so a new session starts from a clean picture, and the host audio
    /// output is (re)started when the `Audio / 音频` box is ticked. The previous
    /// session's audio thread is joined before the session is replaced.
    /// 供自行启动 worker 的宿主（例如使用自定义机器选项）与测试使用。帧计数、掩码与
    /// 纹理都会重置，因此新会话从干净画面开始；勾选音频时会（重新）启动宿主输出。
    /// 替换会话前会先 join 旧会话的音频线程。
    pub fn attach(&mut self, session: Box<dyn PlaySource>) {
        // 旧会话的音频线程必须先 join，再替换会话。
        // Join the old session's audio thread before swapping the session.
        self.stop_audio();
        self.audio_notice_shown = false;
        self.buttons = 0;
        self.last_frame = 0;
        self.stats = PlayStats::default();
        self.texture = None;
        self.session = Some(session);
        // tap 只存在于新会话上：音频必须在会话挂载之后才启动。
        // The tap lives on the new session, so audio starts after attaching it.
        self.start_audio();
    }

    /// Stop the worker and report its final statistics / 停止 worker 并报告最终统计
    ///
    /// The host audio output stops first, so its thread never reads the tap of a
    /// session that is being shut down. Returns in milliseconds.
    /// 先停宿主音频输出，因此其线程绝不会读取正在关闭会话的 tap。毫秒级返回。
    pub fn stop(&mut self) {
        self.stop_audio();
        if let Some(session) = self.session.take() {
            let stats = session.shutdown();
            self.status = format!(
                "stopped / 已停止 · frames={} steps={}",
                stats.frames, stats.steps
            );
        }
    }

    /// Render the tab: toolbar, picture, keyboard and status bar
    /// 渲染标签页：工具栏、画面、键盘与状态栏
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.toolbar(ui);
        ui.separator();
        self.pump(ui.ctx());
        self.show_screen(ui);
        self.status_bar(ui);
    }

    /// Four input rows plus the start/stop row and the status line
    /// 四行输入框，加启动/停止行与状态行
    ///
    /// Every parameter is a user field, so the view has no example defaults: the
    /// profile row can fill them all at once. While a worker runs the whole input
    /// area is greyed out (the session reads them only once, at start), leaving
    /// Start and Stop clickable.
    /// 每个参数都是用户输入框，因此视图没有示例默认值：profile 行可以一次填好全部。
    /// worker 运行中输入区整体置灰（会话只在启动时读取它们），Start 与 Stop 仍可点击。
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let running = self.is_running();
        // 载入动作在闭包之后执行：闭包同时借用了 `self.fields`，方法调用会让借用
        // 检查器报冲突。
        // The load runs after the closure: the closure borrows `self.fields`, so a
        // method call inside it would conflict with the field borrows.
        let mut load_profile = false;
        ui.add_enabled_ui(!running, |ui| {
            ui.horizontal(|ui| {
                ui.label("Profile / 配置");
                ui.add(egui::TextEdit::singleline(&mut self.fields.profile).desired_width(240.0));
                load_profile = ui.button("Load profile / 载入配置").clicked();
            });
            ui.horizontal(|ui| {
                ui.label("FW / 固件");
                ui.add(egui::TextEdit::singleline(&mut self.fields.firmware).desired_width(260.0));
                ui.label("FB / 帧缓冲");
                ui.add(egui::TextEdit::singleline(&mut self.fields.frame).desired_width(150.0));
            });
            ui.horizontal(|ui| {
                ui.label("Steps/frame / 每帧步数");
                ui.add(
                    egui::TextEdit::singleline(&mut self.fields.steps_per_frame)
                        .desired_width(90.0),
                );
                ui.label("Buttons / 按键掩码");
                ui.add(egui::TextEdit::singleline(&mut self.fields.buttons).desired_width(70.0));
                ui.checkbox(&mut self.fields.audio, "Audio / 音频");
            });
            ui.horizontal(|ui| {
                ui.label("Audio PIO block / 块");
                ui.add(
                    egui::TextEdit::singleline(&mut self.fields.audio_block).desired_width(40.0),
                );
                ui.label("SM / 状态机");
                ui.add(egui::TextEdit::singleline(&mut self.fields.audio_sm).desired_width(40.0));
                ui.label("Rate / 采样率");
                ui.add(egui::TextEdit::singleline(&mut self.fields.audio_rate).desired_width(70.0));
                ui.label("Ch / 声道");
                ui.add(
                    egui::TextEdit::singleline(&mut self.fields.audio_channels).desired_width(34.0),
                );
                ui.label("Capacity / 容量");
                ui.add(
                    egui::TextEdit::singleline(&mut self.fields.audio_capacity).desired_width(70.0),
                );
            });
        });
        if load_profile {
            self.load_profile();
        }
        ui.horizontal(|ui| {
            if ui.button("Start / 启动").clicked() {
                self.start();
            }
            if ui.button("Stop / 停止").clicked() {
                self.stop();
            }
            if running {
                ui.label("inputs locked / 参数已锁定");
            }
        });
        ui.horizontal(|ui| {
            ui.label(self.status.as_str());
        });
    }

    /// Poll the worker and upload a new frame when the index changed
    /// 轮询 worker，并在帧序号变化时上传新帧
    fn pump(&mut self, ctx: &egui::Context) {
        let Some((stats, frame)) = self
            .session
            .as_ref()
            .map(|session| (session.stats(), session.latest_frame()))
        else {
            return;
        };
        self.stats = stats;
        let Some(frame) = frame else {
            return;
        };
        if frame.index == self.last_frame {
            // 同一帧不重复上传：最新帧语义下 UI 帧率高于游戏帧率。
            // Never re-upload the same frame: the UI refreshes faster than the game.
            return;
        }
        let image = color_image(frame.width, frame.height, &frame.rgb565);
        match &mut self.texture {
            Some(texture) => texture.set(image, TextureOptions::NEAREST),
            None => {
                self.texture = Some(ctx.load_texture(TEXTURE_NAME, image, TextureOptions::NEAREST));
            }
        }
        self.last_frame = frame.index;
        self.uploads += 1;
    }

    /// Paint the picture and read the keyboard while it owns focus
    /// 绘制画面，并在其拥有焦点时读取键盘
    fn show_screen(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_size();
        // 状态栏在画面之后绘制，因此先为它预留高度；极小窗口钳制到 1 px。
        // The status bar is drawn after the picture, so reserve its height first;
        // clamp to 1 px in a tiny window.
        let size = vec2(available.x.max(1.0), (available.y - STATUS_HEIGHT).max(1.0));
        let (response, painter) = ui.allocate_painter(size, Sense::click());
        let rect = response.rect;
        painter.rect_filled(rect, 0.0, BACKDROP);

        if let Some(texture) = &self.texture {
            let [width, height] = texture.size();
            let fitted = fit_size(rect.size(), width, height);
            if fitted.x >= 1.0 && fitted.y >= 1.0 {
                let picture = Rect::from_center_size(rect.center(), fitted);
                painter.image(
                    texture.id(),
                    picture,
                    Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        } else {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "not running / 未运行",
                FontId::monospace(12.0),
                HINT,
            );
        }

        if response.clicked() {
            self.focused = true;
            self.focus_hint = false;
        } else if ui.input(|input| input.pointer.any_click()) && !response.hovered() {
            self.focused = false;
        }
        if self.focused {
            // 每帧写入掩码：worker 只读原子值，重复写入开销可忽略。
            // Write the mask every frame: the worker only reads an atomic, so a
            // repeated store is free.
            let mask = mask_from_keys(&ui.input(KeyState::from_input));
            self.buttons = mask;
            if let Some(session) = self.session.as_ref() {
                session.set_buttons(mask);
            }
            painter.rect_stroke(rect, 0.0, Stroke::new(1.0, FOCUS), StrokeKind::Inside);
        }
        if self.focus_hint {
            painter.text(
                rect.center_bottom() - vec2(0.0, 6.0),
                Align2::CENTER_BOTTOM,
                "click the picture, then use the keys / 点击画面后使用键盘",
                FontId::monospace(10.0),
                HINT,
            );
        }
    }

    /// Three-line status bar / 三行状态栏
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let running = self.is_running();
        ui.monospace(format_stats(&self.stats, self.last_frame, running));
        ui.monospace(format!("buttons / 按键: {}", describe_mask(self.buttons)));
        // 音频行每帧重读输出线程的计数：欠载数实时增长，无需缓存。
        // Reread the output counters every frame so underruns stay live.
        ui.monospace(format_audio(self.audio_backend(), self.audio_underruns()));
    }
}

impl Drop for PlayView {
    fn drop(&mut self) {
        // 关闭窗口时先停音频线程，再停 worker：来源的 shutdown 会 join 线程。
        // Stop the audio thread first, then the worker; the source joins its
        // thread on shutdown.
        self.stop_audio();
        if let Some(session) = self.session.take() {
            let _ = session.shutdown();
        }
    }
}
