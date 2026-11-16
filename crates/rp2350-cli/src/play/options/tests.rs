//! Option-resolution tests / 选项合并测试

use std::path::{Path, PathBuf};

use super::*;

/// Write a temporary profile and return its path / 写一个临时 profile 并返回路径
fn write_profile(name: &str, text: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("rp2350-play-{}-{name}.conf", std::process::id()));
    std::fs::write(&path, text).expect("write profile");
    path
}

/// Arguments that only name a profile / 只指定 profile 的参数
fn profile_args(path: &Path) -> PlayArgs {
    PlayArgs {
        profile: Some(path.to_path_buf()),
        ..PlayArgs::default()
    }
}

/// A profile with the full generic audio wiring / 含完整通用音频接线的 profile
const AUDIO_PROFILE: &str = "\
firmware = fw.uf2
frame = 0x20000000:4x4
steps_per_frame = 100
audio = true
audio_block = 1
audio_sm = 3
audio_rate = 13440
audio_channels = 2
audio_capacity = 48000
max_cycles = 5000
strict_decode = true
";

#[test]
fn missing_firmware_frame_and_steps_are_reported() {
    let error = resolve(&PlayArgs::default()).expect_err("empty arguments");
    assert!(error.contains("--file"), "{error}");
    assert!(error.contains("profile"), "{error}");

    let frame_only = PlayArgs {
        file: Some(PathBuf::from("fw.uf2")),
        ..PlayArgs::default()
    };
    let error = resolve(&frame_only).expect_err("missing frame");
    assert!(error.contains("--fb"), "{error}");

    let file_and_frame = PlayArgs {
        file: Some(PathBuf::from("fw.uf2")),
        frame: Some(FrameSpec::new(0x2000_0000, 4, 4)),
        ..PlayArgs::default()
    };
    let error = resolve(&file_and_frame).expect_err("missing steps");
    assert!(error.contains("--steps-per-frame"), "{error}");
}

#[test]
fn explicit_arguments_resolve_with_generic_defaults() {
    let args = PlayArgs {
        file: Some(PathBuf::from("fw.uf2")),
        frame: Some(FrameSpec::new(0x2000_0000, 4, 4)),
        steps_per_frame: Some(200),
        ..PlayArgs::default()
    };
    let options = resolve(&args).expect("resolve");
    assert_eq!(options.file, PathBuf::from("fw.uf2"));
    assert_eq!(options.steps_per_frame, 200);
    assert_eq!(options.max_frames, None);
    // 默认不驱动任何按键引脚。 / No button pin is driven by default.
    assert_eq!(options.buttons, 0);
    assert_eq!(options.audio, None);
    assert_eq!(options.arch, None);
    assert!(!options.strict_decode);
    assert_eq!(options.max_cycles, DEFAULT_MAX_CYCLES);
    assert_eq!(options.trace_steps, DEFAULT_TRACE_STEPS);
    // 默认不挂载 PSRAM。 / No PSRAM window by default.
    assert_eq!(options.psram, None);
}

#[test]
fn psram_comes_from_the_profile_and_the_command_line_wins() {
    let path = write_profile(
        "psram",
        "firmware = fw.uf2\nframe = 0x20000000:4x4\nsteps_per_frame = 100\npsram = 8M\n",
    );
    let from_profile = resolve(&profile_args(&path)).expect("profile psram");
    assert_eq!(
        from_profile.psram,
        Some(crate::run::PsramOptions {
            size_bytes: 8 * 1024 * 1024
        })
    );

    let args = PlayArgs {
        psram: Some(crate::run::PsramOptions {
            size_bytes: 16 * 1024 * 1024,
        }),
        ..profile_args(&path)
    };
    let options = resolve(&args).expect("cli psram");
    assert_eq!(
        options.psram,
        Some(crate::run::PsramOptions {
            size_bytes: 16 * 1024 * 1024
        })
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn command_line_values_win_over_the_profile() {
    let path = write_profile(
        "cli-wins",
        "firmware = profile.uf2\nframe = 0x20000000:4x4\nsteps_per_frame = 100\nmax_frames = 7\nbuttons = 0x0F\n",
    );
    let args = PlayArgs {
        file: Some(PathBuf::from("cli.uf2")),
        max_frames: Some(2),
        buttons: Some(0x01),
        ..profile_args(&path)
    };
    let options = resolve(&args).expect("resolve");
    assert_eq!(options.file, PathBuf::from("cli.uf2"));
    assert_eq!(options.max_frames, Some(2));
    assert_eq!(options.buttons, 0x01);
    // profile 仍提供命令行没有给出的字段。
    // The profile still supplies fields the command line omitted.
    assert_eq!(options.steps_per_frame, 100);
    assert_eq!(options.frame, FrameSpec::new(0x2000_0000, 4, 4));
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_profile_alone_can_configure_the_session() {
    let path = write_profile("solo", AUDIO_PROFILE);
    let options = resolve(&profile_args(&path)).expect("resolve");
    assert_eq!(options.file, PathBuf::from("fw.uf2"));
    assert_eq!(options.steps_per_frame, 100);
    assert_eq!(options.max_cycles, 5000);
    assert!(options.strict_decode);
    let audio = options.audio.expect("audio wiring");
    assert_eq!(audio.block, 1);
    assert_eq!(audio.sm, 3);
    assert_eq!(audio.rate_hz, 13_440);
    assert_eq!(audio.channels, 2);
    assert_eq!(audio.capacity, 48_000);
    let _ = std::fs::remove_file(path);
}

#[test]
fn audio_requires_a_block_and_a_state_machine() {
    let args = PlayArgs {
        file: Some(PathBuf::from("fw.uf2")),
        frame: Some(FrameSpec::new(0x2000_0000, 4, 4)),
        steps_per_frame: Some(100),
        audio: true,
        ..PlayArgs::default()
    };
    let error = resolve(&args).expect_err("missing audio block");
    assert!(error.contains("--audio-block"), "{error}");

    let with_block = PlayArgs {
        audio_block: Some(2),
        ..args
    };
    let error = resolve(&with_block).expect_err("missing audio sm");
    assert!(error.contains("--audio-sm"), "{error}");
}

#[test]
fn command_line_audio_wiring_uses_generic_sink_defaults() {
    let args = PlayArgs {
        file: Some(PathBuf::from("fw.uf2")),
        frame: Some(FrameSpec::new(0x2000_0000, 4, 4)),
        steps_per_frame: Some(100),
        audio: true,
        audio_block: Some(2),
        audio_sm: Some(0),
        ..PlayArgs::default()
    };
    let audio = resolve(&args).expect("resolve").audio.expect("audio");
    assert_eq!(audio.block, 2);
    assert_eq!(audio.sm, 0);
    assert_eq!(audio.rate_hz, DEFAULT_AUDIO_RATE_HZ);
    assert_eq!(audio.channels, DEFAULT_AUDIO_CHANNELS);
    assert_eq!(audio.capacity, DEFAULT_AUDIO_CAPACITY);
}

#[test]
fn command_line_overrides_the_profile_audio_rate() {
    let path = write_profile("audio-rate", AUDIO_PROFILE);
    let args = PlayArgs {
        audio_rate: Some(48_000),
        audio_channels: Some(1),
        audio_capacity: Some(512),
        ..profile_args(&path)
    };
    let audio = resolve(&args).expect("resolve").audio.expect("audio");
    assert_eq!(audio.rate_hz, 48_000);
    assert_eq!(audio.channels, 1);
    assert_eq!(audio.capacity, 512);
    // 块/状态机仍取自 profile。 / Block and state machine still come from the profile.
    assert_eq!(audio.block, 1);
    assert_eq!(audio.sm, 3);
    let _ = std::fs::remove_file(path);
}

#[test]
fn audio_off_in_the_profile_keeps_capture_disabled() {
    let path = write_profile(
        "audio-off",
        "firmware = fw.uf2\nframe = 0x20000000:4x4\nsteps_per_frame = 100\naudio = false\n",
    );
    assert_eq!(resolve(&profile_args(&path)).expect("resolve").audio, None);
    let _ = std::fs::remove_file(path);
}

#[test]
fn zero_steps_per_frame_and_zero_audio_fields_are_rejected() {
    let zero_steps = PlayArgs {
        file: Some(PathBuf::from("fw.uf2")),
        frame: Some(FrameSpec::new(0x2000_0000, 4, 4)),
        steps_per_frame: Some(0),
        ..PlayArgs::default()
    };
    assert!(resolve(&zero_steps).is_err());

    let zero_rate = PlayArgs {
        steps_per_frame: Some(1),
        audio: true,
        audio_block: Some(0),
        audio_sm: Some(0),
        audio_rate: Some(0),
        ..zero_steps
    };
    let error = resolve(&zero_rate).expect_err("zero rate");
    assert!(error.contains("greater than zero"), "{error}");
}

#[test]
fn core_executor_settings_come_from_the_profile_and_the_command_line_wins() {
    // 核执行器：默认 off/64，profile 可设，命令行覆盖；窗口在合并阶段同样校验。
    // Core executor: off/64 by default, settable from the profile, and the command
    // line wins; the merge also validates the window range.
    let path = write_profile(
        "core",
        "firmware = fw.uf2\nframe = 0x20000000:4x4\nsteps_per_frame = 100\n\
         core_threads = true\ncore_window = 32\n",
    );
    let from_profile = resolve(&profile_args(&path)).expect("profile core");
    assert!(from_profile.core_threads.enabled);
    assert_eq!(from_profile.core_threads.window, 32);

    let args = PlayArgs {
        core_threads: Some(false),
        core_window: Some(128),
        ..profile_args(&path)
    };
    let from_cli = resolve(&args).expect("cli core");
    assert!(!from_cli.core_threads.enabled);
    assert_eq!(from_cli.core_threads.window, 128);
    let _ = std::fs::remove_file(path);

    let defaults = resolve(&PlayArgs {
        file: Some(PathBuf::from("fw.uf2")),
        frame: Some(FrameSpec::new(0x2000_0000, 4, 4)),
        steps_per_frame: Some(100),
        ..PlayArgs::default()
    })
    .expect("defaults");
    assert!(!defaults.core_threads.enabled);
    assert_eq!(defaults.core_threads.window, DEFAULT_CORE_WINDOW);

    let bad = PlayArgs {
        core_window: Some(0),
        ..PlayArgs::default()
    };
    let error = resolve(&PlayArgs {
        file: Some(PathBuf::from("fw.uf2")),
        frame: Some(FrameSpec::new(0x2000_0000, 4, 4)),
        steps_per_frame: Some(100),
        ..bad
    })
    .expect_err("zero window");
    assert!(error.contains("core window"), "{error}");
}

#[test]
fn a_missing_profile_file_is_reported() {
    let path = std::env::temp_dir().join("rp2350-play-does-not-exist.conf");
    let error = resolve(&profile_args(&path)).expect_err("missing profile");
    assert!(error.contains("cannot read profile"), "{error}");
}

#[test]
fn profile_parse_errors_carry_the_path_and_line() {
    let path = write_profile("bad-key", "firmware = fw.uf2\nbogus = 1\n");
    let error = resolve(&profile_args(&path)).expect_err("bad key");
    assert!(error.contains(&path.display().to_string()), "{error}");
    assert!(error.contains("line 2"), "{error}");
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_profile_audio_block_without_the_flag_still_enables_capture() {
    let path = write_profile(
        "audio-from-profile",
        "firmware = fw.uf2\nframe = 0x20000000:4x4\nsteps_per_frame = 100\naudio = true\naudio_block = 0\naudio_sm = 1\n",
    );
    let options = resolve(&profile_args(&path)).expect("resolve");
    let audio = options.audio.expect("audio");
    assert_eq!(audio.block, 0);
    assert_eq!(audio.sm, 1);
    let _ = std::fs::remove_file(path);
}
