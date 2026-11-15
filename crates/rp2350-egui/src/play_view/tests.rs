//! Tests for the Play tab / 「游戏」标签页测试
//!
//! These tests keep the pixel, key-mapping and layout helpers honest and drive
//! [`PlayView::ui`] headlessly with a fake [`PlaySource`], so texture upload and
//! keyboard plumbing are covered without a window or a real guest.
//! 这些测试校验像素、按键映射与布局辅助函数，并用假的 [`PlaySource`] 无窗口驱动
//! [`PlayView::ui`]，因此无需窗口或真实客户机即可覆盖纹理上传与键盘接线。

use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Key, Modifiers, Pos2, RawInput, Rect, Vec2};
use rp2350_audio::SampleSource;
use rp2350_cli::play::{FrameSpec, PlayConfig, PlaySession};
use rp2350_cli::run::MachineOptions;

use super::*;

/// Fake source state shared with the test / 与测试共享的假来源状态
#[derive(Default)]
pub(super) struct FakeShared {
    /// Frame handed to the view / 交给视图的帧
    frame: Mutex<Option<Arc<PlayFrame>>>,
    /// Mask written by the view / 视图写入的掩码
    buttons: AtomicU16,
    /// Statistics handed to the view / 交给视图的统计
    stats: Mutex<PlayStats>,
    /// Whether the worker still runs / worker 是否仍在运行
    running: AtomicBool,
    /// Button pins reported by the source / 来源报告的按键引脚
    pins: AtomicU16,
    /// Audio tap handed to the view once / 交给视图一次的音频 tap
    pub(super) audio: Mutex<Option<Box<dyn SampleSource>>>,
    /// Number of shutdowns / shutdown 次数
    shutdowns: AtomicU16,
}

/// Source backed by [`FakeShared`] / 由 [`FakeShared`] 支撑的来源
struct FakeSource {
    shared: Arc<FakeShared>,
}

impl PlaySource for FakeSource {
    fn latest_frame(&self) -> Option<Arc<PlayFrame>> {
        self.shared.frame.lock().unwrap().clone()
    }

    fn set_buttons(&self, mask: u16) {
        self.shared.buttons.store(mask, Ordering::Relaxed);
    }

    fn stats(&self) -> PlayStats {
        self.shared.stats.lock().unwrap().clone()
    }

    fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::Relaxed)
    }

    fn button_pins(&self) -> u16 {
        self.shared.pins.load(Ordering::Relaxed)
    }

    fn audio_source(&self) -> Option<Box<dyn SampleSource>> {
        self.shared.audio.lock().unwrap().take()
    }

    fn shutdown(self: Box<Self>) -> PlayStats {
        self.shared.running.store(false, Ordering::Relaxed);
        self.shared.shutdowns.fetch_add(1, Ordering::Relaxed);
        self.stats()
    }
}

/// Build a view whose starter always returns a [`FakeSource`]
/// 构造启动器总是返回 [`FakeSource`] 的视图
pub(super) fn view_with_fake(
    shared: &Arc<FakeShared>,
    requests: &Arc<Mutex<Vec<PlayRequest>>>,
) -> PlayView {
    let source = Arc::clone(shared);
    let recorded = Arc::clone(requests);
    PlayView::default().with_starter(Box::new(move |request: PlayRequest| {
        recorded.lock().unwrap().push(request);
        source.running.store(true, Ordering::Relaxed);
        Ok(Box::new(FakeSource {
            shared: Arc::clone(&source),
        }) as Box<dyn PlaySource>)
    }))
}

/// View already attached to a [`FakeSource`] / 已挂载 [`FakeSource`] 的视图
fn view_with_source(shared: &Arc<FakeShared>) -> PlayView {
    shared.running.store(true, Ordering::Relaxed);
    let mut view = PlayView::default();
    view.attach(Box::new(FakeSource {
        shared: Arc::clone(shared),
    }));
    view
}

/// Sample frame of `width × height` with `index` / 指定序号与尺寸的示例帧
fn frame(index: u64, width: u16, height: u16) -> Arc<PlayFrame> {
    Arc::new(PlayFrame {
        index,
        steps: index * 1000,
        width,
        height,
        rgb565: vec![0xF800; usize::from(width) * usize::from(height)],
        readable: true,
    })
}

/// Render `view` into a headless context and return the shape count
/// 在无窗口上下文中渲染 `view` 并返回图元数量
fn run(
    context: &egui::Context,
    view: &mut PlayView,
    size: Vec2,
    events: Vec<egui::Event>,
) -> usize {
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
        events,
        ..RawInput::default()
    };
    let output = context.run_ui(raw, |ui| view.ui(ui));
    let shapes = output.shapes.len();
    output.drop_without_applying_deltas();
    shapes
}

/// Key press event for `key` / `key` 的按下事件
fn key_press(key: Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

/// Write a temporary firmware file and return its path / 写一个临时固件文件并返回路径
pub(super) fn temp_firmware(tag: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("rp2350-egui-{tag}-{}.uf2", std::process::id()));
    std::fs::write(&path, b"not a real uf2").expect("temp firmware write");
    path
}

#[test]
fn rgb565_primaries() {
    assert_eq!(rgb565_to_color32(0xF800), Color32::from_rgb(255, 0, 0));
    assert_eq!(rgb565_to_color32(0x07E0), Color32::from_rgb(0, 255, 0));
    assert_eq!(rgb565_to_color32(0x001F), Color32::from_rgb(0, 0, 255));
    assert_eq!(rgb565_to_color32(0x0000), Color32::BLACK);
    assert_eq!(rgb565_to_color32(0xFFFF), Color32::WHITE);
}

#[test]
fn rgb565_channel_replication_reaches_full_scale() {
    // 5 位 0b10000 → 0x84，6 位 0b100000 → 0x82：高位复制而不是简单左移。
    // 5-bit 0b10000 → 0x84 and 6-bit 0b100000 → 0x82: bit replication, not a shift.
    assert_eq!(
        rgb565_to_color32(0x8410),
        Color32::from_rgb(0x84, 0x82, 0x84)
    );
    assert_eq!(rgb565_to_color32(0x0001), Color32::from_rgb(0, 0, 8));
    assert_eq!(rgb565_to_color32(0x0020), Color32::from_rgb(0, 4, 0));
}

#[test]
fn color_image_pads_short_input_with_black() {
    let image = color_image(4, 2, &[0xF800, 0x07E0]);
    assert_eq!(image.size, [4, 2]);
    assert_eq!(image.pixels.len(), 8);
    assert_eq!(image.pixels[0], Color32::from_rgb(255, 0, 0));
    assert_eq!(image.pixels[1], Color32::from_rgb(0, 255, 0));
    assert!(
        image.pixels[2..]
            .iter()
            .all(|pixel| *pixel == Color32::BLACK)
    );
}

#[test]
fn key_mask_is_empty_without_keys() {
    assert_eq!(mask_from_keys(&KeyState::default()), 0);
    assert_eq!(describe_mask(0), "none");
}

#[test]
fn key_mask_maps_single_keys_to_gba_pins() {
    let cases = [
        (Key::Z, BUTTON_A, 0),
        (Key::X, BUTTON_B, 1),
        (Key::Backspace, BUTTON_SELECT, 2),
        (Key::Enter, BUTTON_START, 3),
        (Key::ArrowRight, BUTTON_RIGHT, 4),
        (Key::ArrowLeft, BUTTON_LEFT, 5),
        (Key::ArrowUp, BUTTON_UP, 6),
        (Key::ArrowDown, BUTTON_DOWN, 7),
        (Key::S, BUTTON_R, 8),
        (Key::A, BUTTON_L, 9),
    ];
    for (key, bit, pin) in cases {
        let keys = KeyState::from_pressed(|pressed| pressed == key);
        assert_eq!(keys.mask(), bit, "{key:?} should map to {bit:#06b}");
        assert_eq!(bit, 1 << pin, "GP{pin}");
    }
}

#[test]
fn key_mask_combines_keys() {
    let keys = KeyState::from_pressed(|pressed| matches!(pressed, Key::Z | Key::S));
    assert_eq!(keys.mask(), BUTTON_A | BUTTON_R);
    assert_eq!(describe_mask(keys.mask()), "A|R");

    let all = KeyState {
        up: true,
        down: true,
        left: true,
        right: true,
        a: true,
        b: true,
        start: true,
        select: true,
        r: true,
        l: true,
    };
    assert_eq!(all.mask(), GBA_BUTTONS);
    assert_eq!(GBA_BUTTONS, 0x03FF);
}

#[test]
fn key_state_from_input_reads_live_keys() {
    let context = egui::Context::default();
    let raw = RawInput {
        events: vec![key_press(Key::ArrowUp), key_press(Key::X)],
        ..RawInput::default()
    };
    let mut mask = 0;
    let output = context.run_ui(raw, |ui| {
        mask = ui.input(KeyState::from_input).mask();
    });
    output.drop_without_applying_deltas();
    assert_eq!(mask, BUTTON_UP | BUTTON_B);
}

#[test]
fn fit_size_uses_integer_zoom_when_up_scaling() {
    // 240×160 在 1280×720 上按 4× 绘制，在 320×240 上按 1×。
    // A 240×160 picture is drawn 4× inside 1280×720 and 1× inside 320×240.
    assert_eq!(fit_size(vec2(1280.0, 720.0), 240, 160), vec2(960.0, 640.0));
    assert_eq!(fit_size(vec2(320.0, 240.0), 240, 160), vec2(240.0, 160.0));
    let small = fit_size(vec2(80.0, 60.0), 240, 160);
    assert!((small.x - 80.0).abs() < 1e-3 && (small.y - 160.0 / 3.0).abs() < 1e-3);
    assert!((small.x / small.y - 1.5).abs() < 1e-3);
}

#[test]
fn fit_size_handles_empty_areas_and_frames() {
    assert_eq!(fit_size(Vec2::ZERO, 240, 160), Vec2::ZERO);
    assert_eq!(fit_size(vec2(320.0, 240.0), 0, 160), Vec2::ZERO);
    assert_eq!(fit_size(vec2(320.0, 240.0), 240, 0), Vec2::ZERO);
    assert_eq!(fit_size(vec2(0.5, 0.5), 240, 160), Vec2::ZERO);
}

#[test]
fn format_stats_reports_progress_and_halt() {
    let stats = PlayStats {
        frames: 7,
        steps: 17_500_000,
        fps: 8.02,
        steps_per_second: 20_100_000.0,
        replaced: 3,
        halted: None,
    };
    assert_eq!(
        format_stats(&stats, 7, true),
        "frame=7 steps=17500000 fps=8.02 steps/s=20100000 replaced=3 · running / 运行中"
    );
    let halted = PlayStats {
        halted: Some("instruction budget exhausted / 指令预算耗尽".to_owned()),
        ..stats
    };
    let text = format_stats(&halted, 7, false);
    assert!(text.contains("stopped / 已停止"));
    assert!(text.ends_with("halted / 停机: instruction budget exhausted / 指令预算耗尽"));
}

#[test]
fn idle_view_renders_placeholders() {
    let mut view = PlayView::default();
    assert!(!view.is_running());
    assert_eq!(view.status(), "idle / 空闲");
    let context = egui::Context::default();
    let shapes = run(&context, &mut view, vec2(320.0, 240.0), Vec::new());
    assert!(shapes > 0, "idle view should still paint its widgets");
    assert_eq!(view.uploads(), 0);
}

#[test]
fn ui_renders_at_small_medium_and_large_sizes() {
    let shared = Arc::new(FakeShared::default());
    *shared.frame.lock().unwrap() = Some(frame(1, 240, 160));
    let mut view = view_with_source(&shared);
    for size in [vec2(80.0, 60.0), vec2(320.0, 240.0), vec2(1280.0, 720.0)] {
        let context = egui::Context::default();
        assert!(run(&context, &mut view, size, Vec::new()) > 0);
    }
}

#[test]
fn texture_is_uploaded_once_per_frame_index() {
    let shared = Arc::new(FakeShared::default());
    *shared.frame.lock().unwrap() = Some(frame(1, 8, 4));
    let mut view = view_with_source(&shared);

    let context = egui::Context::default();
    run(&context, &mut view, vec2(320.0, 240.0), Vec::new());
    assert_eq!(view.uploads(), 1);
    run(&context, &mut view, vec2(320.0, 240.0), Vec::new());
    assert_eq!(view.uploads(), 1, "same frame must not be uploaded twice");
    assert_eq!(view.last_frame, 1);

    *shared.frame.lock().unwrap() = Some(frame(2, 8, 4));
    run(&context, &mut view, vec2(320.0, 240.0), Vec::new());
    assert_eq!(view.uploads(), 2);
    assert_eq!(view.last_frame, 2);
}

#[test]
fn keyboard_mask_reaches_the_source_only_when_focused() {
    let shared = Arc::new(FakeShared::default());
    let mut view = view_with_source(&shared);
    let context = egui::Context::default();

    // 未获得焦点时不写入掩码。 / Without focus no mask is written.
    run(
        &context,
        &mut view,
        vec2(320.0, 240.0),
        vec![key_press(Key::Z)],
    );
    assert_eq!(shared.buttons.load(Ordering::Relaxed), 0);

    view.focused = true;
    run(
        &context,
        &mut view,
        vec2(320.0, 240.0),
        vec![key_press(Key::Z)],
    );
    assert_eq!(shared.buttons.load(Ordering::Relaxed), BUTTON_A);
    assert_eq!(view.buttons(), BUTTON_A);
}

#[test]
fn start_reports_read_error_without_panicking() {
    let mut view = PlayView::default();
    view.fields.firmware = String::from("definitely-missing-firmware.uf2");
    view.fields.frame = String::from(FAKE_FRAME);
    view.fields.steps_per_frame = String::from("200");
    view.start();
    assert!(
        view.status()
            .contains("cannot read firmware / 无法读取固件")
    );
    assert!(!view.is_running());
}

#[test]
fn start_reports_missing_starter() {
    let path = temp_firmware("nostarter");
    let mut view = PlayView::default();
    view.fields.firmware = path.display().to_string();
    view.fields.frame = String::from(FAKE_FRAME);
    view.fields.steps_per_frame = String::from("200");
    view.start();
    assert!(view.status().contains("no play source attached"));
    assert!(!view.is_running());
    let _ = std::fs::remove_file(path);
}

#[test]
fn start_reports_bad_steps_per_frame() {
    let mut view = PlayView::default();
    // 固件与帧文本有效，因此校验会走到每帧步数；文件在步数校验之后才读取。
    // Firmware and frame text are valid so validation reaches the step count; the
    // file is only read after that.
    view.fields.firmware = String::from("unused.uf2");
    view.fields.frame = String::from(FAKE_FRAME);
    view.fields.steps_per_frame = String::from("0");
    view.start();
    assert!(view.status().contains("steps per frame"));
    view.fields.steps_per_frame = String::from("abc");
    view.start();
    assert!(view.status().contains("steps per frame"));
}

#[test]
fn start_boots_source_and_stop_shuts_it_down() {
    let path = temp_firmware("boot");
    let shared = Arc::new(FakeShared::default());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut view = view_with_fake(&shared, &requests);
    view.fields.firmware = path.display().to_string();
    view.fields.frame = String::from(FAKE_FRAME);
    view.fields.steps_per_frame = String::from("200");
    view.fields.buttons = String::from("0x3FF");
    view.start();

    assert!(view.is_running());
    assert!(view.status().contains("running / 运行中"));
    let recorded = requests.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].image, b"not a real uf2");
    assert_eq!(recorded[0].frame, FAKE_FRAME);
    assert_eq!(recorded[0].steps_per_frame, 200);
    // 音频默认关闭，因此请求里没有接线；引脚掩码来自输入框。
    // Audio is off by default, so the request carries no wiring; the pin mask
    // comes from its field.
    assert!(recorded[0].audio.is_none(), "audio off => no wiring");
    assert_eq!(recorded[0].button_pins, GBA_BUTTONS);
    drop(recorded);

    view.stop();
    assert!(!view.is_running());
    assert!(view.status().contains("stopped / 已停止"));
    assert_eq!(shared.shutdowns.load(Ordering::Relaxed), 1);
    let _ = std::fs::remove_file(path);
}

#[test]
fn starter_error_is_shown_in_the_toolbar() {
    let path = temp_firmware("err");
    let mut view = PlayView::default().with_starter(Box::new(
        |_request: PlayRequest| -> Result<Box<dyn PlaySource>, String> {
            Err("machine boot failed / 机器启动失败".to_owned())
        },
    ));
    view.fields.firmware = path.display().to_string();
    view.fields.frame = String::from(FAKE_FRAME);
    view.fields.steps_per_frame = String::from("200");
    view.start();
    assert!(view.status().contains("start failed / 启动失败"));
    assert!(view.status().contains("machine boot failed"));
    assert!(!view.is_running());
    let _ = std::fs::remove_file(path);
}

// ---------------------------------------------------------------------------
// 真实 `rp2350_cli::play::PlaySession` 适配器测试。
// Tests of the adapter over a real `rp2350_cli::play::PlaySession`.
// ---------------------------------------------------------------------------

/// Flash XIP base address / Flash XIP 基地址
const FLASH_BASE: u32 = 0x1000_0000;

/// RP2350 Arm Secure UF2 family / RP2350 Arm 安全 UF2 家族
const FAMILY_ARM: u32 = 0xE48B_FF59;

/// Frame buffer text handed to the starter / 交给启动器的帧缓冲文本
pub(super) const FAKE_FRAME: &str = "0x20001000:4x4";

/// Build one 512-byte UF2 block / 构造一个 512 字节 UF2 块
fn uf2_block(block_number: u32, total_blocks: u32, address: u32, payload: &[u8]) -> Vec<u8> {
    let mut block = vec![0u8; 512];
    block[0..4].copy_from_slice(&0x0A32_4655u32.to_le_bytes());
    block[4..8].copy_from_slice(&0x9E5D_5157u32.to_le_bytes());
    // Family-ID-present flag. / 置位 family ID 存在标志。
    block[8..12].copy_from_slice(&0x0000_2000u32.to_le_bytes());
    block[12..16].copy_from_slice(&address.to_le_bytes());
    block[16..20].copy_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
    block[20..24].copy_from_slice(&block_number.to_le_bytes());
    block[24..28].copy_from_slice(&total_blocks.to_le_bytes());
    block[28..32].copy_from_slice(&FAMILY_ARM.to_le_bytes());
    block[32..32 + payload.len()].copy_from_slice(payload);
    block[508..512].copy_from_slice(&0x0AB1_6F30u32.to_le_bytes());
    block
}

/// Synthetic Arm UF2 whose reset handler is `b .` / 复位处理程序为 `b .` 的合成 Arm UF2
pub(super) fn spin_uf2() -> Vec<u8> {
    let vectors = [
        0x2008_2000u32.to_le_bytes(),
        (FLASH_BASE + 0x09).to_le_bytes(),
    ]
    .concat();
    let mut image = uf2_block(0, 2, FLASH_BASE, &vectors);
    image.extend(uf2_block(1, 2, FLASH_BASE + 8, &[0xFE, 0xE7]));
    image
}

/// Machine options the adapter uses (`arch: None`) / 适配器使用的机器选项
const ADAPTER_OPTIONS: MachineOptions = MachineOptions {
    arch: None,
    max_cycles: u64::MAX,
    strict_decode: false,
    // 适配器不挂载 PSRAM。 / The adapter mounts no PSRAM.
    psram: None,
};

/// Start a real session wrapped by [`CliPlaySource`] / 启动被 [`CliPlaySource`] 包装的真实会话
fn cli_source(steps_per_frame: u64) -> CliPlaySource {
    let frame = FrameSpec::parse(FAKE_FRAME).expect("frame spec");
    let config = PlayConfig::new(frame, steps_per_frame);
    let session = PlaySession::start(spin_uf2(), ADAPTER_OPTIONS, config).expect("play session");
    CliPlaySource::new(session)
}

/// A [`PlayRequest`] for the synthetic image / 针对合成映像的 [`PlayRequest`]
fn request(frame: &str) -> PlayRequest {
    PlayRequest {
        image: spin_uf2(),
        frame: frame.to_owned(),
        steps_per_frame: 200,
        audio: None,
        button_pins: GBA_BUTTONS,
    }
}

/// Wait until a real source reports statistics matching `predicate`
/// 等到真实来源报告满足 `predicate` 的统计
fn wait_for(source: &CliPlaySource, predicate: impl Fn(&PlayStats) -> bool) -> PlayStats {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let stats = source.stats();
        if predicate(&stats) {
            return stats;
        }
        assert!(Instant::now() < deadline, "timed out with {stats:?}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn cli_source_publishes_frames_and_shuts_down() {
    let source = cli_source(200);
    let stats = wait_for(&source, |stats| stats.frames >= 1);
    // `PlayConfig::new` 不驱动任何引脚：按键映射必须由前端显式给出。
    // `PlayConfig::new` drives no pin: the frontend must supply the button map.
    assert_eq!(source.button_pins(), 0);

    source.set_buttons(BUTTON_A | BUTTON_START);
    assert_eq!(source.session().buttons(), BUTTON_A | BUTTON_START);
    source.set_buttons(0);
    assert_eq!(source.session().buttons(), 0);

    let frame = source.latest_frame().expect("first frame");
    assert_eq!(frame.index, 1);
    assert_eq!(frame.rgb565.len(), 16);
    assert!(stats.steps >= 200, "steps={}", stats.steps);

    let final_stats = Box::new(source).shutdown();
    assert!(final_stats.halted.is_none(), "clean stop is not a halt");
}

#[test]
fn cli_source_stats_are_monotonic() {
    let source = cli_source(100);
    wait_for(&source, |stats| stats.frames >= 1);
    let first = source.stats();
    let second = wait_for(&source, |stats| stats.frames > first.frames);

    assert!(second.frames > first.frames, "{second:?}");
    assert!(second.steps > first.steps, "{second:?}");
    assert!(second.steps_per_second > 0.0, "{second:?}");
    let _ = Box::new(source).shutdown();
}

#[test]
fn latest_frame_cache_reuses_the_same_arc() {
    let source = cli_source(100);
    wait_for(&source, |stats| stats.frames >= 1);
    let first = source.latest_frame().expect("frame");
    let again = source.latest_frame().expect("cached frame");
    assert!(
        Arc::ptr_eq(&first, &again),
        "an unchanged frame must not be copied again"
    );
    let _ = Box::new(source).shutdown();
}

#[test]
fn cli_starter_rejects_bad_frame_text_and_images() {
    let bad_frame = CliPlayStarter.start(request("not-a-frame-spec"));
    assert!(bad_frame.is_err(), "bad frame text must not boot");
    assert!(
        bad_frame
            .err()
            .is_some_and(|message| message.contains("WxH")),
        "the parse error should name the missing size"
    );

    let mut bad_image = request(FAKE_FRAME);
    bad_image.image = b"not a uf2 or elf".to_vec();
    assert!(CliPlayStarter.start(bad_image).is_err());
}

#[test]
fn play_view_with_cli_starter_boots_and_uploads_a_frame() {
    let image = spin_uf2();
    let path =
        std::env::temp_dir().join(format!("rp2350-egui-cli-source-{}.uf2", std::process::id()));
    std::fs::write(&path, &image).expect("temp firmware write");

    let mut view = PlayView::default().with_starter(Box::new(CliPlayStarter));
    view.fields.firmware = path.display().to_string();
    view.fields.frame = FAKE_FRAME.to_owned();
    view.fields.steps_per_frame = String::from("200");
    view.start();
    assert!(view.is_running(), "status={}", view.status());

    let context = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(30);
    while view.uploads() == 0 {
        run(&context, &mut view, vec2(320.0, 240.0), Vec::new());
        assert!(
            Instant::now() < deadline,
            "no frame uploaded, status={}",
            view.status()
        );
        std::thread::sleep(Duration::from_millis(2));
    }

    view.stop();
    assert!(!view.is_running());
    assert!(view.status().contains("stopped / 已停止"));
    let _ = std::fs::remove_file(path);
}
