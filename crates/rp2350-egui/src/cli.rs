//! Command-line launch mode for the native GUI / 原生 GUI 的命令行直启
//!
//! The native entry point parses these options before any window exists, merges
//! them with an optional `key = value` profile (**command line > profile >
//! empty**) through [`PlayFields`], and validates the result with the very same
//! [`PlayFields::inputs`] path the toolbar's Start button uses. Invalid options
//! print a bilingual message and exit with status 2, so a typo never flashes a
//! window.
//! 原生入口在任何窗口创建之前解析这些选项，通过 [`PlayFields`] 与可选
//! `key = value` profile 合并（**命令行 > profile > 空**），并用工具栏 Start 完全
//! 相同的 [`PlayFields::inputs`] 路径校验结果。非法选项打印双语消息并以状态码 2
//! 退出，因此输入错误不会闪现窗口。
//!
//! No example value is baked in: every guest-specific value (firmware path, frame
//! buffer, steps per frame, button pins, I2S wiring) comes from a flag or from the
//! profile, exactly like the GUI text fields. No arguments at all reproduces the
//! historical behaviour: empty fields, no auto-start.
//! 不内置任何示例值：每个客户机专用值（固件路径、帧缓冲、每帧步数、按键引脚、
//! I2S 接线）都来自参数或 profile，与 GUI 输入框一致。完全无参数时行为与以往一致：
//! 字段为空、不自动启动。
//!
//! ```text
//! rp2350-egui --profile example/gba/profile.conf
//! rp2350-egui --file demo.uf2 --fb 0x20000000:240x160 --steps-per-frame 1000000
//! rp2350-egui --profile example/gba/profile.conf --no-auto-start
//! ```

use std::ffi::OsString;
use std::path::PathBuf;

use clap::Parser;

use crate::play_view::PlayFields;

/// Prefix of the stable auto-start line written to stderr
/// 写入 stderr 的稳定自动启动日志前缀
const AUTO_START_LOG: &str = "[rp2350-egui] auto-start";

/// Command-line options of the native GUI / 原生 GUI 的命令行选项
///
/// Every value is optional: `--profile` may supply it, and a value given on the
/// command line wins over the profile. Both `--file` and `--fb` (plus
/// `--steps-per-frame`) are required unless the profile supplies them.
/// 每个值都是可选的：`--profile` 可以提供它，命令行给出的值覆盖 profile。除非 profile
/// 提供，否则 `--file` 与 `--fb`（以及 `--steps-per-frame`）都必须给出。
#[derive(Debug, Clone, Parser)]
#[command(
    name = "rp2350-egui",
    version,
    about = "RP2350 emulator GUI with command-line launch / 带命令行直启的 RP2350 模拟器 GUI",
    long_about = None
)]
struct Cli {
    /// Load this `key = value` profile, then start the game automatically
    /// 载入该 `key = value` profile 并自动开始游戏
    #[arg(long, value_name = "PATH")]
    profile: Option<PathBuf>,

    /// Firmware file (UF2 or ELF); required without a profile
    /// 固件文件（UF2 或 ELF）；没有 profile 时必填
    #[arg(long, value_name = "PATH")]
    file: Option<PathBuf>,

    /// Frame buffer spec `ADDR:WxH[:STRIDE]`; required without a profile
    /// 帧缓冲规格 `ADDR:WxH[:STRIDE]`；没有 profile 时必填
    #[arg(long, value_name = "ADDR:WxH[:STRIDE]")]
    fb: Option<String>,

    /// Guest steps executed per published frame / 每发布一帧执行的客户机步数
    #[arg(long, value_name = "N")]
    steps_per_frame: Option<String>,

    /// Button pin bitmap (bit set = pressed = pin low) / 按键引脚位图（置位 = 按下 = 低电平）
    #[arg(long, value_name = "MASK")]
    buttons: Option<String>,

    /// Capture I2S audio words (needs --audio-block/--audio-sm)
    /// 捕获 I2S 音频字（需要 --audio-block/--audio-sm）
    #[arg(long)]
    audio: bool,

    /// PIO block that pushes the I2S words / 推送 I2S 字的 PIO 块
    #[arg(long, value_name = "N")]
    audio_block: Option<String>,

    /// State machine inside that PIO block / 该 PIO 块内的状态机
    #[arg(long, value_name = "N")]
    audio_sm: Option<String>,

    /// Sample rate stored in the audio sink / 音频 sink 的采样率
    #[arg(long, value_name = "HZ")]
    audio_rate: Option<String>,

    /// Channel count stored in the audio sink / 音频 sink 的声道数
    #[arg(long, value_name = "N")]
    audio_channels: Option<String>,

    /// Samples the audio sink retains / 音频 sink 保留的样本数
    #[arg(long, value_name = "N")]
    audio_capacity: Option<String>,

    /// Pre-fill the toolbar fields but do not start the game
    /// 只预填工具栏字段，不自动开始游戏
    #[arg(long)]
    no_auto_start: bool,
}

/// Resolved launch request handed to [`EmuApp`](crate::EmuApp)
/// 交给 [`EmuApp`](crate::EmuApp) 的解析后启动请求
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayLaunch {
    /// Toolbar fields resolved from the command line and the profile
    /// 由命令行与 profile 解析出的工具栏字段
    pub fields: PlayFields,
    /// Whether the session should start before the first frame
    /// 是否在首帧之前启动会话
    pub auto_start: bool,
    /// Stable stderr line to print when auto-starting
    /// 自动启动时打印的稳定 stderr 日志行
    pub log: Option<String>,
}

/// Parse process arguments and resolve an optional play launch
/// 解析进程参数并解析可选的游玩启动请求
///
/// `--help` and `--version` are handled by clap (exit code 0); syntax errors exit
/// with code 2. A semantic validation error is returned to the caller, which
/// prints it and exits with code 2 without creating a window. Arguments with no
/// play option at all resolve to `None`, which keeps the previous behaviour:
/// empty fields and manual Start.
/// `--help` 与 `--version` 交由 clap 处理（退出码 0）；语法错误退出码 2。语义校验
/// 错误返回给调用方，由它打印并在不创建窗口的情况下以状态码 2 退出。完全没有游玩
/// 选项时解析为 `None`，保持既有行为：字段为空、手动 Start。
///
/// # Errors
/// Returns a bilingual message when the profile cannot be read or parsed, when a
/// required field (`firmware`, `frame`, `steps per frame`) is missing, or when
/// audio capture is requested without a PIO block/state machine.
/// profile 无法读取或解析、必填字段（固件、帧缓冲、每帧步数）缺失，或请求音频捕获
/// 却没有 PIO 块/状态机时返回双语消息。
pub fn parse_and_resolve<I, T>(args: I) -> Result<Option<PlayLaunch>, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        // clap 自己处理 --help/--version（退出码 0）与语法错误（退出码 2）。
        // clap handles --help/--version (exit 0) and syntax errors (exit 2).
        Err(error) => error.exit(),
    };
    cli.resolve()
}

impl Cli {
    /// Whether any play option was supplied / 是否给出了任何游玩选项
    ///
    /// `--no-auto-start` on its own is not a launch request: nothing would be
    /// pre-filled, so it falls back to the plain manual mode.
    /// 单独给出 `--no-auto-start` 不构成启动请求：没有可预填的内容，因此回退到纯手动
    /// 模式。
    fn has_play_arguments(&self) -> bool {
        self.profile.is_some()
            || self.file.is_some()
            || self.fb.is_some()
            || self.steps_per_frame.is_some()
            || self.buttons.is_some()
            || self.audio
            || self.audio_block.is_some()
            || self.audio_sm.is_some()
            || self.audio_rate.is_some()
            || self.audio_channels.is_some()
            || self.audio_capacity.is_some()
    }

    /// Merge the command line, the profile and the empty defaults
    /// 合并命令行、profile 与空默认值
    ///
    /// Precedence is command line > profile > empty. The merged text is validated
    /// through [`PlayFields::inputs`], the same function Start uses, so the GUI
    /// never opens with parameters the Start button would reject.
    /// 优先级为命令行 > profile > 空。合并后的文本经 [`PlayFields::inputs`] 校验，
    /// 与 Start 使用同一函数，因此 GUI 不会带着 Start 会拒绝的参数打开。
    fn resolve(&self) -> Result<Option<PlayLaunch>, String> {
        if !self.has_play_arguments() {
            return Ok(None);
        }

        let mut fields = PlayFields::default();
        if let Some(path) = &self.profile {
            fields.profile = path.display().to_string();
            // 复用工具栏「载入配置」的同一条路径，包括行号格式的解析错误。
            // Reuse the toolbar's "Load profile" path, including `profile line N`
            // parse errors.
            fields
                .load_profile()
                .map_err(|message| format!("--profile {}: {message}", path.display()))?;
        }
        self.overlay_explicit(&mut fields);

        // 与点 Start 时完全相同的必填/格式校验；任何失败都在开窗前终止。
        // Exactly the same required/format validation Start performs; any failure
        // aborts before a window is created.
        if let Err(message) = fields.inputs() {
            return Err(format!("invalid play arguments / 游玩参数非法: {message}"));
        }

        let auto_start = !self.no_auto_start;
        let log = auto_start.then(|| self.log_line(&fields));
        Ok(Some(PlayLaunch {
            fields,
            auto_start,
            log,
        }))
    }

    /// Overwrite the profile values with the explicit command-line values
    /// 用显式命令行值覆盖 profile 值
    fn overlay_explicit(&self, fields: &mut PlayFields) {
        if let Some(file) = &self.file {
            fields.firmware = file.display().to_string();
        }
        if let Some(frame) = &self.fb {
            fields.frame.clone_from(frame);
        }
        if let Some(steps) = &self.steps_per_frame {
            fields.steps_per_frame.clone_from(steps);
        }
        if let Some(buttons) = &self.buttons {
            fields.buttons.clone_from(buttons);
        }
        if self.audio {
            fields.audio = true;
        }
        if let Some(block) = &self.audio_block {
            fields.audio_block.clone_from(block);
        }
        if let Some(sm) = &self.audio_sm {
            fields.audio_sm.clone_from(sm);
        }
        if let Some(rate) = &self.audio_rate {
            fields.audio_rate.clone_from(rate);
        }
        if let Some(channels) = &self.audio_channels {
            fields.audio_channels.clone_from(channels);
        }
        if let Some(capacity) = &self.audio_capacity {
            fields.audio_capacity.clone_from(capacity);
        }
    }

    /// Stable stderr line describing the automatic start / 描述自动启动的稳定 stderr 行
    ///
    /// A profile launch names the profile (scripts already know the rest); an
    /// explicit launch names the three required values so a log reader can check
    /// them without opening the profile.
    /// profile 启动只记录路径（其余值在 profile 里）；显式启动记录三个必填值，日志
    /// 读者无需打开 profile 就能核对。
    fn log_line(&self, fields: &PlayFields) -> String {
        match &self.profile {
            Some(path) => format!("{AUTO_START_LOG} profile={}", path.display()),
            None => format!(
                "{AUTO_START_LOG} file={} fb={} steps_per_frame={}",
                fields.firmware, fields.frame, fields.steps_per_frame
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use rp2350_cli::play::FrameSpec;
    use rp2350_cli::play::profile;

    use super::*;

    /// Write a temporary profile and return its path / 写临时 profile 并返回路径
    fn temp_profile(tag: &str, text: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("rp2350-egui-cli-{tag}-{}.conf", std::process::id()));
        std::fs::write(&path, text).expect("temp profile write");
        path
    }

    /// Example profile shipped with the GBA sample, if it exists
    /// 若存在则返回 GBA 示例随附的 profile
    fn example_profile() -> Option<PathBuf> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../example/gba/profile.conf");
        path.exists().then_some(path)
    }

    /// Resolve one argument list / 解析一组参数
    fn launch(args: &[&str]) -> Option<PlayLaunch> {
        let mut full = vec!["rp2350-egui"];
        full.extend_from_slice(args);
        parse_and_resolve(full).expect("arguments resolve")
    }

    #[test]
    fn profile_only_launch_starts_from_the_profile() {
        let path = temp_profile(
            "profile-only",
            "# comment\n\
             firmware = E:\\firmware\\demo.uf2\n\
             frame = 0x20000000:240x160\n\
             steps_per_frame = 1000000\n\
             buttons = 0x3FF\n\
             audio = true\n\
             audio_block = 2\n\
             audio_sm = 0\n\
             audio_rate = 13440\n\
             audio_channels = 2\n\
             audio_capacity = 48000\n",
        );
        let path_text = path.display().to_string();
        let launch = launch(&["--profile", &path_text]).expect("launch");
        assert!(launch.auto_start);
        assert_eq!(launch.fields.profile, path_text);
        assert_eq!(launch.fields.firmware, "E:\\firmware\\demo.uf2");
        assert_eq!(launch.fields.frame, "0x20000000:240x160");
        assert_eq!(launch.fields.steps_per_frame, "1000000");
        assert_eq!(launch.fields.buttons, "0x3FF");
        assert!(launch.fields.audio);
        assert_eq!(launch.fields.audio_block, "2");
        assert_eq!(launch.fields.audio_sm, "0");
        assert_eq!(launch.fields.audio_rate, "13440");
        assert_eq!(launch.fields.audio_channels, "2");
        let expected = format!("[rp2350-egui] auto-start profile={path_text}");
        assert_eq!(launch.log.as_deref(), Some(expected.as_str()));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn explicit_launch_fills_every_field() {
        let launch = launch(&[
            "--file",
            "demo.uf2",
            "--fb",
            "0x20000000:240x160",
            "--steps-per-frame",
            "1000000",
            "--buttons",
            "0x3FF",
            "--audio",
            "--audio-block",
            "2",
            "--audio-sm",
            "0",
            "--audio-rate",
            "13440",
            "--audio-channels",
            "2",
            "--audio-capacity",
            "48000",
        ])
        .expect("launch");
        assert!(launch.auto_start);
        assert_eq!(launch.fields.firmware, "demo.uf2");
        assert_eq!(launch.fields.frame, "0x20000000:240x160");
        assert_eq!(launch.fields.steps_per_frame, "1000000");
        assert_eq!(launch.fields.buttons, "0x3FF");
        assert!(launch.fields.audio);
        assert_eq!(launch.fields.audio_block, "2");
        assert_eq!(launch.fields.audio_sm, "0");
        assert_eq!(launch.fields.audio_rate, "13440");
        assert_eq!(launch.fields.audio_channels, "2");
        assert_eq!(launch.fields.audio_capacity, "48000");
        assert_eq!(
            launch.log.as_deref(),
            Some(
                "[rp2350-egui] auto-start file=demo.uf2 \
                 fb=0x20000000:240x160 steps_per_frame=1000000"
            )
        );
    }

    #[test]
    fn command_line_overrides_profile_values() {
        let path = temp_profile(
            "override",
            "firmware = profile.uf2\n\
             frame = 0x20000000:240x160\n\
             steps_per_frame = 100\n\
             buttons = 0x3FF\n\
             audio = true\n\
             audio_block = 2\n\
             audio_sm = 0\n\
             audio_rate = 13440\n\
             audio_channels = 2\n\
             audio_capacity = 48000\n",
        );
        let path_text = path.display().to_string();
        let launch = launch(&[
            "--profile",
            &path_text,
            "--file",
            "cli.uf2",
            "--fb",
            "0x20001000:4x4",
            "--steps-per-frame",
            "200",
            "--buttons",
            "0x00F",
            "--audio-rate",
            "22050",
        ])
        .expect("launch");
        assert_eq!(launch.fields.firmware, "cli.uf2");
        assert_eq!(launch.fields.frame, "0x20001000:4x4");
        assert_eq!(launch.fields.steps_per_frame, "200");
        assert_eq!(launch.fields.buttons, "0x00F");
        assert_eq!(launch.fields.audio_rate, "22050");
        // profile 仍然提供未在命令行覆盖的值。
        // The profile still supplies the values not overridden on the command line.
        assert!(launch.fields.audio);
        assert_eq!(launch.fields.audio_block, "2");
        assert_eq!(launch.fields.audio_sm, "0");
        assert_eq!(launch.fields.audio_channels, "2");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn no_auto_start_prefills_without_starting() {
        let launch = launch(&[
            "--file",
            "demo.uf2",
            "--fb",
            "0x20000000:240x160",
            "--steps-per-frame",
            "1000000",
            "--no-auto-start",
        ])
        .expect("launch");
        assert!(!launch.auto_start);
        assert!(launch.log.is_none());
        assert_eq!(launch.fields.firmware, "demo.uf2");
        assert_eq!(launch.fields.frame, "0x20000000:240x160");
    }

    #[test]
    fn missing_frame_buffer_is_rejected() {
        let error = parse_and_resolve([
            "rp2350-egui",
            "--file",
            "demo.uf2",
            "--steps-per-frame",
            "1000000",
        ])
        .expect_err("a launch without --fb must fail");
        assert!(error.contains("missing frame buffer"), "error={error}");
    }

    #[test]
    fn audio_without_wiring_is_rejected() {
        let error = parse_and_resolve([
            "rp2350-egui",
            "--file",
            "demo.uf2",
            "--fb",
            "0x20000000:240x160",
            "--steps-per-frame",
            "1000000",
            "--audio",
        ])
        .expect_err("audio without block/sm must fail");
        assert!(error.contains("audio PIO block"), "error={error}");

        let error = parse_and_resolve([
            "rp2350-egui",
            "--file",
            "demo.uf2",
            "--fb",
            "0x20000000:240x160",
            "--steps-per-frame",
            "1000000",
            "--audio",
            "--audio-block",
            "2",
        ])
        .expect_err("audio without a state machine must fail");
        assert!(error.contains("audio state machine"), "error={error}");
    }

    #[test]
    fn malformed_frame_buffer_is_rejected() {
        let error = parse_and_resolve([
            "rp2350-egui",
            "--file",
            "demo.uf2",
            "--fb",
            "not-a-frame",
            "--steps-per-frame",
            "1000000",
        ])
        .expect_err("a malformed --fb must fail");
        assert!(error.contains("帧缓冲非法"), "error={error}");
    }

    #[test]
    fn missing_or_malformed_profile_is_rejected() {
        let path = std::env::temp_dir().join(format!(
            "rp2350-egui-cli-does-not-exist-{}.conf",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let path_text = path.display().to_string();
        let error = parse_and_resolve(["rp2350-egui", "--profile", &path_text])
            .expect_err("an unreadable profile must fail");
        assert!(error.contains("cannot read profile"), "error={error}");
        assert!(error.contains(&path_text), "error={error}");

        let malformed = temp_profile(
            "malformed",
            "# comment\nsteps_per_frame = 100\nframe = nope\n",
        );
        let malformed_text = malformed.display().to_string();
        let error = parse_and_resolve(["rp2350-egui", "--profile", &malformed_text])
            .expect_err("a malformed profile must fail");
        assert!(error.contains("profile line 3"), "error={error}");
        assert!(error.contains("帧缓冲非法"), "error={error}");
        let _ = std::fs::remove_file(malformed);
    }

    #[test]
    fn help_and_version_exit_successfully() {
        use clap::error::ErrorKind;
        for (flag, kind) in [
            ("--help", ErrorKind::DisplayHelp),
            ("--version", ErrorKind::DisplayVersion),
        ] {
            let error = Cli::try_parse_from(["rp2350-egui", flag]).expect_err(flag);
            assert_eq!(error.kind(), kind, "{flag}");
            assert_eq!(error.exit_code(), 0, "{flag}");
        }
    }

    #[test]
    fn no_arguments_leave_the_fields_empty() {
        assert!(
            parse_and_resolve(["rp2350-egui"])
                .expect("empty arguments")
                .is_none(),
            "no play option must keep the manual mode"
        );
        assert!(
            parse_and_resolve(["rp2350-egui", "--no-auto-start"])
                .expect("flag only")
                .is_none(),
            "a lone --no-auto-start has nothing to prefill"
        );
        // 默认视图与字段仍然全空（防止示例常量回归）。
        // The default view and fields are still empty (guards against example
        // constants coming back).
        let app = crate::EmuApp::sample();
        assert_eq!(app.play().fields(), &PlayFields::default());
        for value in [
            &app.play().fields().profile,
            &app.play().fields().firmware,
            &app.play().fields().frame,
            &app.play().fields().steps_per_frame,
            &app.play().fields().buttons,
        ] {
            assert!(value.is_empty(), "field must stay empty, got `{value}`");
        }
    }

    #[test]
    fn example_profile_fields_match_the_parsed_profile() {
        let Some(path) = example_profile() else {
            // 示例 profile 不存在时跳过：crate 不应依赖它才能测试。
            // Skip when the example profile is absent: the crate must test
            // without it.
            return;
        };
        let path_text = path.display().to_string();
        let launch = launch(&["--profile", &path_text, "--no-auto-start"]).expect("launch");
        let text = std::fs::read_to_string(&path).expect("read example profile");
        let parsed = profile::parse(&text).expect("example profile parses");

        assert_eq!(
            launch.fields.firmware,
            parsed.firmware.expect("firmware").display().to_string()
        );
        assert_eq!(
            FrameSpec::parse(&launch.fields.frame).expect("frame"),
            parsed.frame.expect("frame")
        );
        assert_eq!(
            launch.fields.steps_per_frame.parse::<u64>().expect("steps"),
            parsed.steps_per_frame.expect("steps")
        );
        assert_eq!(
            launch.fields.buttons,
            format!("0x{:03X}", parsed.buttons.expect("buttons"))
        );
        assert_eq!(launch.fields.audio, parsed.audio.expect("audio"));
        assert_eq!(
            launch.fields.audio_block,
            parsed.audio_block.expect("audio block").to_string()
        );
        assert_eq!(
            launch.fields.audio_sm,
            parsed.audio_sm.expect("audio sm").to_string()
        );
        assert_eq!(
            launch.fields.audio_rate,
            parsed.audio_rate.expect("audio rate").to_string()
        );
        assert_eq!(
            launch.fields.audio_channels,
            parsed.audio_channels.expect("channels").to_string()
        );
        assert_eq!(
            launch.fields.audio_capacity,
            parsed.audio_capacity.expect("capacity").to_string()
        );
    }
}
