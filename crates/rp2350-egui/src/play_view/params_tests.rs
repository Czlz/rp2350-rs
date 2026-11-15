//! Tests for the profile-driven toolbar fields / 工具栏字段（profile 驱动）测试
//!
//! These tests pin the two rules of the genericity policy: the GUI holds no
//! example constants (an empty field list on startup) and every guest-specific
//! value comes from the user or from a `key = value` profile. They also cover the
//! audio wiring handover to `rp2350_cli::play::PlayConfig`.
//! 这些测试锁定通用性规则的两条要求：GUI 内没有示例常量（启动时字段为空），每个
//! 客户机专用值都来自用户输入或 `key = value` profile。同时覆盖音频接线交给
//! `rp2350_cli::play::PlayConfig` 的过程。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rp2350_cli::play::FrameSpec;
use rp2350_cli::play::profile;

use super::tests::{FAKE_FRAME, FakeShared, spin_uf2, temp_firmware, view_with_fake};
use super::*;

/// Example profile shipped with the GBA sample, if it exists
/// 若存在则返回 GBA 示例随附的 profile
fn example_profile() -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../example/gba/profile.conf");
    path.exists().then_some(path)
}

/// Write a temporary profile and return its path / 写一个临时 profile 并返回路径
fn temp_profile(tag: &str, text: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("rp2350-egui-{tag}-{}.conf", std::process::id()));
    std::fs::write(&path, text).expect("temp profile write");
    path
}

/// View with a fake starter and every required field filled / 假 starter 且必填字段就绪的视图
fn ready_view(
    shared: &Arc<FakeShared>,
    requests: &Arc<Mutex<Vec<PlayRequest>>>,
    tag: &str,
) -> (PlayView, PathBuf) {
    let path = temp_firmware(tag);
    let mut view = view_with_fake(shared, requests);
    view.fields.firmware = path.display().to_string();
    view.fields.frame = FAKE_FRAME.to_owned();
    view.fields.steps_per_frame = String::from("200");
    (view, path)
}

/// Request carrying the synthetic image and `audio` / 携带合成映像与 `audio` 的请求
fn starter_request(audio: Option<AudioRequest>) -> PlayRequest {
    PlayRequest {
        image: spin_uf2(),
        frame: FAKE_FRAME.to_owned(),
        steps_per_frame: 200,
        audio,
        button_pins: 0,
    }
}

#[test]
fn default_fields_hold_no_example_values() {
    let fields = PlayFields::default();
    for (name, value) in [
        ("profile", &fields.profile),
        ("firmware", &fields.firmware),
        ("frame", &fields.frame),
        ("steps_per_frame", &fields.steps_per_frame),
        ("buttons", &fields.buttons),
        ("audio_block", &fields.audio_block),
        ("audio_sm", &fields.audio_sm),
    ] {
        assert!(value.is_empty(), "{name} must start empty, got `{value}`");
    }
    assert!(!fields.audio, "audio capture is opt-in");
    // 音频 sink 元数据描述宿主缓冲，不是某个固件，因此有通用默认值。
    // Sink metadata describes the host buffer, not a firmware, so it has
    // generic defaults.
    assert_eq!(fields.audio_rate, "48000");
    assert_eq!(fields.audio_channels, "2");
    assert_eq!(fields.audio_capacity, "48000");

    // 默认视图同样不预填示例固件 / 帧缓冲（防止示例常量回归）。
    // The default view pre-fills neither firmware nor frame buffer (this guards
    // against example constants coming back).
    let view = PlayView::default();
    assert_eq!(view.fields().firmware, "");
    assert_eq!(view.fields().frame, "");
    assert_eq!(view.fields().profile, "");
}

#[test]
fn start_requires_firmware_and_frame() {
    let mut view = PlayView::default();
    view.start();
    assert!(
        view.status().contains("missing firmware"),
        "status={}",
        view.status()
    );
    assert!(!view.is_running());

    let path = temp_firmware("noframe");
    view.fields.firmware = path.display().to_string();
    view.start();
    assert!(
        view.status().contains("missing frame buffer"),
        "status={}",
        view.status()
    );
    assert!(!view.is_running());
    let _ = std::fs::remove_file(path);
}

#[test]
fn start_reports_bad_frame_text() {
    let path = temp_firmware("badframe");
    let mut view = PlayView::default();
    view.fields.firmware = path.display().to_string();
    view.fields.frame = String::from("not-a-frame");
    view.fields.steps_per_frame = String::from("200");
    view.start();
    assert!(
        view.status().contains("帧缓冲非法"),
        "status={}",
        view.status()
    );
    assert!(!view.is_running());
    let _ = std::fs::remove_file(path);
}

#[test]
fn buttons_field_accepts_hex_decimal_and_empty() {
    let mut fields = PlayFields {
        firmware: String::from("unused.uf2"),
        frame: String::from(FAKE_FRAME),
        steps_per_frame: String::from("100"),
        ..PlayFields::default()
    };

    assert_eq!(
        fields.inputs().unwrap().buttons,
        0,
        "an empty mask drives no pin"
    );
    fields.buttons = String::from("0x3FF");
    assert_eq!(fields.inputs().unwrap().buttons, 0x3FF);
    fields.buttons = String::from("63");
    assert_eq!(fields.inputs().unwrap().buttons, 63);
    fields.buttons = String::from("zzz");
    assert!(
        fields.inputs().unwrap_err().contains("按键掩码"),
        "a bad mask must be named in the message"
    );
}

#[test]
fn audio_off_sends_no_wiring() {
    let shared = Arc::new(FakeShared::default());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (mut view, path) = ready_view(&shared, &requests, "params-audio-none");
    // 即使填了块与状态机，未勾选音频时也不得发送接线。
    // Even with a block and state machine typed, no wiring is sent while audio
    // is unticked.
    view.fields.audio_block = String::from("2");
    view.fields.audio_sm = String::from("0");
    view.start();
    assert!(view.is_running(), "status={}", view.status());
    assert!(requests.lock().unwrap()[0].audio.is_none());
    view.stop();
    let _ = std::fs::remove_file(path);
}

#[test]
fn audio_on_sends_the_wiring_from_the_fields() {
    let shared = Arc::new(FakeShared::default());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (mut view, path) = ready_view(&shared, &requests, "params-audio-on");
    view.set_audio(true);
    view.fields.audio_block = String::from("1");
    view.fields.audio_sm = String::from("3");
    view.fields.audio_rate = String::from("13440");
    view.fields.audio_channels = String::from("2");
    view.fields.audio_capacity = String::from("4096");
    view.start();
    assert!(view.is_running(), "status={}", view.status());

    let recorded = requests.lock().unwrap();
    assert_eq!(
        recorded[0].audio,
        Some(AudioRequest {
            block: 1,
            sm: 3,
            rate_hz: 13_440,
            channels: 2,
            capacity: 4_096,
        })
    );
    drop(recorded);
    view.stop();
    let _ = std::fs::remove_file(path);
}

#[test]
fn audio_on_requires_valid_wiring_fields() {
    let shared = Arc::new(FakeShared::default());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (mut view, path) = ready_view(&shared, &requests, "params-audio-bad");
    view.set_audio(true);

    view.start();
    assert!(
        view.status().contains("audio PIO block"),
        "status={}",
        view.status()
    );

    view.fields.audio_block = String::from("zero");
    view.start();
    assert!(
        view.status().contains("bad audio PIO block"),
        "status={}",
        view.status()
    );

    view.fields.audio_block = String::from("0");
    view.fields.audio_sm = String::from("0");
    view.fields.audio_rate = String::from("0");
    view.start();
    assert!(
        view.status().contains("greater than zero"),
        "status={}",
        view.status()
    );
    assert!(!view.is_running(), "no session may boot on invalid wiring");
    assert!(requests.lock().unwrap().is_empty());
    let _ = std::fs::remove_file(path);
}

#[test]
fn format_frame_round_trips_packed_and_padded_specs() {
    for text in ["0x20001000:4x4", "0x20001000:4x4:16", "1000:2x3"] {
        let frame = FrameSpec::parse(text).expect(text);
        let rendered = format_frame(frame);
        assert_eq!(
            FrameSpec::parse(&rendered).unwrap(),
            frame,
            "text={rendered}"
        );
    }
    let packed = FrameSpec::parse("0x20001000:4x4").unwrap();
    assert_eq!(format_frame(packed), "0x20001000:4x4");
}

#[test]
fn example_profile_fills_every_field() {
    let Some(path) = example_profile() else {
        // 示例 profile 不存在时跳过：crate 不应依赖它才能测试。
        // Skip when the example profile is absent: the crate must test without it.
        return;
    };
    let text = std::fs::read_to_string(&path).expect("read example profile");
    let profile = profile::parse(&text).expect("example profile parses");

    let mut view = PlayView::default();
    view.fields.profile = path.display().to_string();
    view.load_profile();
    assert!(
        view.status().contains("profile loaded"),
        "status={}",
        view.status()
    );

    assert_eq!(
        view.fields.firmware,
        profile.firmware.unwrap().display().to_string()
    );
    assert_eq!(
        FrameSpec::parse(&view.fields.frame).unwrap(),
        profile.frame.unwrap()
    );
    assert_eq!(
        view.fields.steps_per_frame.parse::<u64>().unwrap(),
        profile.steps_per_frame.unwrap()
    );
    assert_eq!(
        view.fields.buttons,
        format!("0x{:03X}", profile.buttons.unwrap())
    );
    assert_eq!(view.fields.audio, profile.audio.unwrap());
    assert_eq!(
        view.fields.audio_block,
        profile.audio_block.unwrap().to_string()
    );
    assert_eq!(view.fields.audio_sm, profile.audio_sm.unwrap().to_string());
    assert_eq!(
        view.fields.audio_rate,
        profile.audio_rate.unwrap().to_string()
    );
    assert_eq!(
        view.fields.audio_channels,
        profile.audio_channels.unwrap().to_string()
    );
    assert_eq!(
        view.fields.audio_capacity,
        profile.audio_capacity.unwrap().to_string()
    );
}

#[test]
fn profile_error_reports_its_line_and_does_not_start() {
    let path = temp_profile("bad", "# comment\nsteps_per_frame = 10\nframe = nope\n");
    let mut view = PlayView::default();
    view.fields.profile = path.display().to_string();
    view.load_profile();
    assert!(
        view.status().contains("profile line 3"),
        "status={}",
        view.status()
    );
    assert!(
        view.fields.frame.is_empty() && view.fields.steps_per_frame.is_empty(),
        "a rejected profile must not change any field"
    );

    view.start();
    assert!(!view.is_running(), "a rejected profile cannot start a game");
    let _ = std::fs::remove_file(path);
}

#[test]
fn profile_load_requires_a_path() {
    let mut view = PlayView::default();
    view.load_profile();
    assert!(
        view.status().contains("profile path is empty"),
        "status={}",
        view.status()
    );
}

#[test]
fn cli_starter_without_audio_leaves_the_tap_unset() {
    let source = CliPlayStarter
        .start(starter_request(None))
        .expect("session");
    assert!(
        source.audio_source().is_none(),
        "audio: None must not install a tap"
    );
    let _ = source.shutdown();
}

#[test]
fn cli_starter_audio_on_installs_a_tap_with_the_given_sink() {
    let source = CliPlayStarter
        .start(starter_request(Some(AudioRequest {
            block: 0,
            sm: 1,
            rate_hz: 13_440,
            channels: 2,
            capacity: 4_096,
        })))
        .expect("session");
    let tap = source.audio_source().expect("audio tap");
    let format = tap.format();
    assert_eq!(format.rate_hz, 13_440, "rate comes from the request");
    assert_eq!(format.channels, 2, "channels come from the request");
    let _ = source.shutdown();
}

#[test]
fn cli_starter_rejects_an_out_of_range_audio_block() {
    let result = CliPlayStarter.start(starter_request(Some(AudioRequest {
        block: 9,
        sm: 0,
        rate_hz: 48_000,
        channels: 2,
        capacity: 1_024,
    })));
    let error = result.err().expect("an unknown PIO block must be rejected");
    assert!(error.contains("PIO block"), "{error}");
}
