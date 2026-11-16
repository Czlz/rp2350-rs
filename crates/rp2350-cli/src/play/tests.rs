//! `play` worker tests / `play` worker 测试
//!
//! The machine fixtures are synthetic Arm UF2 images whose reset handler is a
//! `b .` spin loop, so a session can step forever and never needs real firmware.
//! 机器夹具是合成的 Arm UF2 映像，其复位处理程序为 `b .` 自旋循环，因此会话可以
//! 一直步进，无需真实固件。

use std::thread;
use std::time::{Duration, Instant};

use super::*;
use crate::run::Machine;
use rp2350_core::AddressBus;

/// SRAM base address used by the fixtures / 夹具使用的 SRAM 基地址
const SRAM_BASE: u32 = 0x2000_0000;

/// Flash XIP base address / Flash XIP 基地址
const FLASH_BASE: u32 = 0x1000_0000;

/// RP2350 Arm Secure UF2 family / RP2350 Arm 安全 UF2 家族
const FAMILY_ARM: u32 = 0xE48B_FF59;

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

/// UF2 image: reset vectors at the flash base plus a `b .` handler
/// UF2 映像：Flash 基地址处的复位向量与 `b .` 处理程序
fn spin_uf2() -> Vec<u8> {
    let mut vectors = Vec::with_capacity(8);
    // 栈顶 + 带 Thumb 位的复位向量。 / Stack top plus the Thumb-tagged reset vector.
    vectors.extend_from_slice(&0x2008_2000u32.to_le_bytes());
    vectors.extend_from_slice(&(FLASH_BASE + 0x09).to_le_bytes());
    let mut image = uf2_block(0, 2, FLASH_BASE, &vectors);
    image.extend(uf2_block(1, 2, FLASH_BASE + 8, &[0xFE, 0xE7]));
    image
}

/// Machine options for the synthetic spinning image / 合成自旋映像的机器选项
fn spin_options(max_cycles: u64) -> MachineOptions {
    MachineOptions {
        arch: Some(crate::run::ArchArg::Arm),
        max_cycles,
        strict_decode: false,
        psram: None,
    }
}

/// Boot the synthetic spinning machine / 启动合成的自旋机器
fn machine() -> Machine {
    Machine::new(&spin_uf2(), &spin_options(u64::MAX)).expect("machine")
}

/// Wait until `predicate` accepts the statistics / 等到统计满足条件
fn wait_for(session: &PlaySession, predicate: impl Fn(&PlayStats) -> bool) -> PlayStats {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let stats = session.stats();
        if predicate(&stats) {
            return stats;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {stats:?}");
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn frame_spec_parses_address_size_and_default_stride() {
    let spec = FrameSpec::parse("0x20000000:240x160").expect("spec");
    assert_eq!(spec.base, 0x2000_0000);
    assert_eq!(spec.width, 240);
    assert_eq!(spec.height, 160);
    assert_eq!(spec.stride_bytes, 480);
    assert_eq!(spec.buffer_len(), 160 * 480);

    let decimal = FrameSpec::parse("537000000:8X4").expect("decimal spec");
    assert_eq!(decimal.base, 537_000_000);
    assert_eq!(decimal.width, 8);
    assert_eq!(decimal.height, 4);
    assert_eq!(decimal.stride_bytes, 16);
}

#[test]
fn frame_spec_parses_explicit_row_stride() {
    let spec = FrameSpec::parse("0x20000000:4x2:12").expect("padded spec");
    assert_eq!(spec.width, 4);
    assert_eq!(spec.height, 2);
    assert_eq!(spec.stride_bytes, 12);
    assert_eq!(spec.buffer_len(), 24);
}

#[test]
fn frame_spec_rejects_malformed_specs() {
    for bad in [
        "",
        "0x20000000",
        "0x20000000:240",
        "zz:240x160",
        "0x20000000:240xzz",
        "0x20000000:0x160",
        "0x20000000:240x0",
        "0x20000000:240x160:100",
        "0x20000000:240x160:",
        "0x20000000:240x160:0x100:extra",
        "0x20000000:240xx160",
    ] {
        assert!(FrameSpec::parse(bad).is_err(), "`{bad}` must be rejected");
    }
}

#[test]
fn frame_decode_reads_little_endian_rgb565_and_skips_padding() {
    let spec = FrameSpec {
        base: 0,
        width: 2,
        height: 2,
        stride_bytes: 6,
    };
    let mut bytes = vec![0xA5u8; 12];
    bytes[0..2].copy_from_slice(&0x1234u16.to_le_bytes());
    bytes[2..4].copy_from_slice(&0xABCDu16.to_le_bytes());
    // 行 0 末尾两字节是填充，解码必须跳过。
    // The last two row-0 bytes are padding.
    bytes[6..8].copy_from_slice(&0x0001u16.to_le_bytes());
    bytes[8..10].copy_from_slice(&0x8000u16.to_le_bytes());

    let frame = spec.frame(7, 42, &bytes);
    assert_eq!(frame.index, 7);
    assert_eq!(frame.steps, 42);
    assert_eq!(frame.width, 2);
    assert_eq!(frame.height, 2);
    assert_eq!(frame.rgb565, vec![0x1234, 0xABCD, 0x0001, 0x8000]);
    assert!(frame.readable);
    assert!(!frame.is_blank());
}

#[test]
fn frame_decode_zero_pads_a_short_read() {
    let spec = FrameSpec {
        base: 0,
        width: 2,
        height: 2,
        stride_bytes: 4,
    };
    // 只读到第一行。 / Only the first row is readable.
    let bytes = [0x34u8, 0x12, 0xCD, 0xAB];
    let frame = spec.frame(1, 1, &bytes);
    assert_eq!(frame.rgb565, vec![0x1234, 0xABCD, 0, 0]);
    assert!(!frame.readable);
    assert_eq!(frame.rgb565.len(), 4);
}

#[test]
fn button_mask_maps_press_to_low_and_release_to_high() {
    let machine = machine();
    let pins = 1u16 << 5;
    // 引脚 6 是无关输入，保持高电平。 / Pin 6 is an unrelated input held high.
    machine.set_pad_pin(6, true);
    assert_ne!(machine.peripherals().pads().input_levels() & (1 << 6), 0);

    apply_buttons(&machine, pins, pins);
    let levels = machine.peripherals().pads().input_levels();
    assert_eq!(levels & (1 << 5), 0, "pressed is low / 按下为低");
    assert_ne!(
        levels & (1 << 6),
        0,
        "unrelated pin untouched / 无关引脚不变"
    );

    apply_buttons(&machine, 0, pins);
    let levels = machine.peripherals().pads().input_levels();
    assert_eq!(levels & (1 << 5), 1 << 5, "released is high / 松开为高");
}

#[test]
fn button_mask_only_drives_configured_pins() {
    let machine = machine();
    // 掩码里的 GP9 不在 `button_pins` 中：它不会被驱动，因此保持复位下拉的低电平，
    // 而同一掩码下已配置的 GP5 被显式驱动为「松开」的高电平。
    // GP9 is in the mask but not in `button_pins`, so it is never driven and keeps
    // its reset pull-down; GP5, which is configured, is explicitly driven high
    // (released) by the same mask.
    apply_buttons(&machine, 1u16 << 9, 1u16 << 5);
    let levels = machine.peripherals().pads().input_levels();
    assert_eq!(levels & (1 << 9), 0, "unconfigured pin stays undriven");
    assert_eq!(levels & (1 << 5), 1 << 5, "configured pin follows the mask");
}

#[test]
fn button_mask_parser_accepts_hex_and_decimal() {
    assert_eq!(parse_button_mask("0x3FF").expect("hex mask"), 0x3FF);
    assert_eq!(parse_button_mask("1023").expect("decimal mask"), 1023);
    assert_eq!(parse_button_mask(" 0X0F ").expect("padded mask"), 0x0F);
    assert!(parse_button_mask("zz").is_err());
    assert!(parse_button_mask("0x10000").is_err());
}

#[test]
fn headless_session_publishes_two_frames_and_shuts_down() {
    let config = PlayConfig {
        max_frames: Some(2),
        ..PlayConfig::new(FrameSpec::new(SRAM_BASE + 0x1000, 4, 4), 200)
    };
    let session = PlaySession::start(spin_uf2(), spin_options(u64::MAX), config).expect("session");
    let stats = wait_for(&session, |stats| stats.frames >= 2);
    assert_eq!(stats.frames, 2);
    assert!(stats.steps >= 400, "steps={}", stats.steps);
    assert_eq!(stats.halted, None);
    assert!(session.latest_frame().is_some());

    let final_stats = session.shutdown();
    assert_eq!(final_stats.frames, 2);
    assert!(final_stats.steps >= 400);
    assert!(final_stats.steps_per_second > 0.0);
    assert_eq!(final_stats.halted, None, "clean shutdown is not a halt");
}

#[test]
fn headless_session_runs_with_the_threaded_core_executor() {
    // 线程化开关与 PlaySession 的生命周期兼容：worker 自己构造 Machine，批次内
    // 启动核线程并在帧边界 join；shutdown 后播放 worker 与核线程都已退出。
    // The threaded switch is compatible with PlaySession's lifecycle: the worker
    // builds the Machine itself, starts core threads per batch and joins them at
    // frame boundaries; after shutdown both the play worker and core threads are
    // gone.
    let config = PlayConfig {
        max_frames: Some(2),
        ..PlayConfig::new(FrameSpec::new(SRAM_BASE + 0x1000, 4, 4), 200)
    };
    let threads = crate::run::CoreThreadOptions {
        enabled: true,
        window: 32,
    };
    let session =
        PlaySession::start_with_core_threads(spin_uf2(), spin_options(u64::MAX), threads, config)
            .expect("session");
    let stats = wait_for(&session, |stats| stats.frames >= 2);
    assert_eq!(stats.frames, 2);
    assert_eq!(stats.halted, None);
    let final_stats = session.shutdown();
    assert_eq!(final_stats.frames, 2);
    assert_eq!(final_stats.halted, None);
}

#[test]
fn frame_indices_and_steps_are_monotonic() {
    let config = PlayConfig::new(FrameSpec::new(SRAM_BASE + 0x1000, 4, 4), 200);
    let session = PlaySession::start(spin_uf2(), spin_options(u64::MAX), config).expect("session");
    wait_for(&session, |stats| stats.frames >= 1);
    let first = session.latest_frame().expect("first frame");
    wait_for(&session, |stats| stats.frames >= 2);
    let second = session.latest_frame().expect("second frame");

    assert!(second.index > first.index, "{second:?}");
    assert!(second.steps > first.steps, "{second:?}");
    assert_eq!(second.rgb565.len(), 16);
    let _ = session.shutdown();
}

#[test]
fn unreadable_frame_buffer_yields_black_frame_without_panic() {
    let config = PlayConfig {
        max_frames: Some(1),
        ..PlayConfig::new(FrameSpec::new(0x3000_0000, 8, 8), 200)
    };
    let session = PlaySession::start(spin_uf2(), spin_options(u64::MAX), config).expect("session");
    wait_for(&session, |stats| stats.frames >= 1);
    let frame = session.latest_frame().expect("frame");
    assert!(!frame.readable);
    assert!(frame.is_blank());
    assert_eq!(frame.rgb565.len(), 64);
    let _ = session.shutdown();
}

#[test]
fn budget_exhaustion_reports_halted_with_finite_rates() {
    let config = PlayConfig::new(FrameSpec::new(SRAM_BASE + 0x1000, 4, 4), 50_000);
    let session = PlaySession::start(spin_uf2(), spin_options(1_000), config).expect("session");
    let stats = wait_for(&session, |stats| stats.halted.is_some());
    let reason = stats.halted.as_deref().unwrap_or_default();
    assert!(
        reason.contains("budget") || reason.contains("预算"),
        "unexpected halt reason: {reason}"
    );
    assert!(!stats.fps.is_nan(), "fps must be finite: {}", stats.fps);
    assert!(!stats.steps_per_second.is_nan());
    let _ = session.shutdown();
}

#[test]
fn stats_report_a_positive_step_rate() {
    let config = PlayConfig::new(FrameSpec::new(SRAM_BASE + 0x1000, 4, 4), 500);
    let session = PlaySession::start(spin_uf2(), spin_options(u64::MAX), config).expect("session");
    let stats = wait_for(&session, |stats| stats.frames >= 1);
    assert!(
        stats.steps_per_second > 0.0,
        "steps/s={}",
        stats.steps_per_second
    );
    assert!(stats.fps > 0.0, "fps={}", stats.fps);
    assert!(stats.fps.is_finite() && stats.steps_per_second.is_finite());
    let _ = session.shutdown();
}

#[test]
fn start_rejects_invalid_configurations() {
    let frame = FrameSpec::new(SRAM_BASE, 4, 4);
    assert!(
        PlaySession::start(
            spin_uf2(),
            spin_options(u64::MAX),
            PlayConfig::new(frame, 0)
        )
        .is_err()
    );

    let zero_width = FrameSpec {
        base: SRAM_BASE,
        width: 0,
        height: 4,
        stride_bytes: 0,
    };
    assert!(
        PlaySession::start(
            spin_uf2(),
            spin_options(u64::MAX),
            PlayConfig::new(zero_width, 10)
        )
        .is_err()
    );

    let short_stride = FrameSpec {
        base: SRAM_BASE,
        width: 4,
        height: 4,
        stride_bytes: 2,
    };
    assert!(
        PlaySession::start(
            spin_uf2(),
            spin_options(u64::MAX),
            PlayConfig::new(short_stride, 10)
        )
        .is_err()
    );

    let huge = FrameSpec {
        base: SRAM_BASE,
        width: 8192,
        height: 8192,
        stride_bytes: 16_384,
    };
    assert!(
        PlaySession::start(
            spin_uf2(),
            spin_options(u64::MAX),
            PlayConfig::new(huge, 10)
        )
        .is_err()
    );
}

#[test]
fn play_config_defaults_are_generic() {
    // 默认配置不驱动任何按键引脚、不捕获音频：示例接线必须由调用方显式给出。
    // Generic defaults drive no button pin and capture no audio: example wiring
    // must be supplied explicitly by the caller.
    let config = PlayConfig::new(FrameSpec::new(SRAM_BASE, 4, 4), 10);
    assert_eq!(config.button_pins, 0);
    assert_eq!(config.audio, None);
    assert_eq!(config.max_frames, None);
    assert_eq!(config.trace_steps, 0);
}

#[test]
fn audio_configuration_exposes_a_tap() {
    let config = PlayConfig {
        audio: Some(PlayAudioConfig {
            block: 0,
            sm: 1,
            rate_hz: 13_440,
            channels: 2,
            capacity: 1_000,
        }),
        max_frames: Some(1),
        ..PlayConfig::new(FrameSpec::new(SRAM_BASE + 0x1000, 4, 4), 200)
    };
    let session = PlaySession::start(spin_uf2(), spin_options(u64::MAX), config).expect("session");
    let tap = session.audio_tap().expect("audio tap");
    assert_eq!(tap.rate_hz(), 13_440);
    assert_eq!(tap.channels(), 2);
    wait_for(&session, |stats| stats.frames >= 1);
    let _ = session.shutdown();
}

#[test]
fn audio_wiring_outside_the_pio_ranges_fails_the_start() {
    let frame = FrameSpec::new(SRAM_BASE + 0x1000, 4, 4);
    let bad_block = PlayConfig {
        audio: Some(PlayAudioConfig {
            block: usize::from(u8::MAX),
            sm: 0,
            rate_hz: 48_000,
            channels: 2,
            capacity: 1_000,
        }),
        ..PlayConfig::new(frame, 200)
    };
    let error = PlaySession::start(spin_uf2(), spin_options(u64::MAX), bad_block)
        .err()
        .expect("out-of-range block must fail");
    assert!(error.contains("PIO block"), "{error}");

    let bad_sm = PlayConfig {
        audio: Some(PlayAudioConfig {
            block: 0,
            sm: usize::from(u8::MAX),
            rate_hz: 48_000,
            channels: 2,
            capacity: 1_000,
        }),
        ..PlayConfig::new(frame, 200)
    };
    let error = PlaySession::start(spin_uf2(), spin_options(u64::MAX), bad_sm)
        .err()
        .expect("out-of-range state machine must fail");
    assert!(error.contains("state machine"), "{error}");

    let zero_rate = PlayConfig {
        audio: Some(PlayAudioConfig {
            block: 0,
            sm: 0,
            rate_hz: 0,
            channels: 2,
            capacity: 1_000,
        }),
        ..PlayConfig::new(frame, 200)
    };
    assert!(PlaySession::start(spin_uf2(), spin_options(u64::MAX), zero_rate).is_err());
}

#[test]
fn buttons_setter_round_trips_and_drop_stops_the_worker() {
    let pins = 0x0F;
    let config = PlayConfig {
        max_frames: Some(1),
        button_pins: pins,
        ..PlayConfig::new(FrameSpec::new(SRAM_BASE + 0x1000, 4, 4), 200)
    };
    let session = PlaySession::start(spin_uf2(), spin_options(u64::MAX), config).expect("session");
    session.set_buttons(0x2A);
    assert_eq!(session.buttons(), 0x2A);
    assert_eq!(session.button_pins(), pins);
    wait_for(&session, |stats| stats.frames >= 1);
    // Drop 必须安全停止线程：不能 panic，也不能重复 join。
    // Dropping must stop the thread safely: no panic and no double join.
    drop(session);
}

// ---- 帧缓冲批量读与按键采样 / batched frame reads and button sampling ------

/// Write a byte pattern through the machine bus / 通过机器总线写入字节图案
fn write_bytes(machine: &mut Machine, addr: u32, bytes: &[u8]) {
    let mut cycles = 0u64;
    for (offset, byte) in bytes.iter().enumerate() {
        machine
            .bus
            .write_u8(addr + offset as u32, *byte, &mut cycles)
            .expect("bus write");
    }
}

/// Byte-wise reference implementation of `read_block` / `read_block` 的逐字节参考实现
fn read_byte_wise(machine: &mut Machine, addr: u32, len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut cycles = 0u64;
    for offset in 0..len {
        match machine
            .bus
            .read_u8(addr.wrapping_add(offset as u32), &mut cycles)
        {
            Ok(byte) => out.push(byte),
            Err(_) => break,
        }
    }
    out
}

#[test]
fn read_block_matches_byte_wise_reads_at_every_alignment() {
    // 32 位批量读必须与逐字节读返回完全相同的字节，包括不对齐的起点与不足一字的尾部。
    // The 32-bit batched read must return exactly the byte-wise bytes, including
    // unaligned starts and sub-word tails.
    let mut machine = machine();
    let base = SRAM_BASE + 0x2000;
    let pattern: Vec<u8> = (0..64u32).map(|index| (index * 7 + 3) as u8).collect();
    write_bytes(&mut machine, base, &pattern);

    for start in 0..4u32 {
        for len in [0usize, 1, 2, 3, 4, 5, 7, 8, 17, 32, 61] {
            let address = base + start;
            let batched = machine.read_block(address, len);
            let expected = read_byte_wise(&mut machine, address, len);
            assert_eq!(batched, expected, "start={start} len={len}");
        }
    }
}

#[test]
fn read_block_stops_at_the_same_unreadable_byte_as_byte_reads() {
    // 区间跨过 SRAM 末尾：宽读失败必须回退逐字节读，返回与逐字节路径相同的前缀。
    // The range crosses the end of SRAM: a failed wide read must fall back to
    // byte reads and return the same prefix as the byte-wise path.
    let mut machine = machine();
    let end = SRAM_BASE + 520 * 1024;
    let start = end - 6;
    write_bytes(&mut machine, start, &[1, 2, 3, 4, 5, 6]);

    let batched = machine.read_block(start, 16);
    let expected = read_byte_wise(&mut machine, start, 16);
    assert_eq!(batched, expected);
    assert_eq!(batched.len(), 6, "only the readable prefix is returned");

    // 完全未映射的起点两条路径都返回空。
    // A fully unmapped start returns nothing on both paths.
    assert_eq!(machine.read_block(0x3000_0000, 16), Vec::<u8>::new());
    assert_eq!(
        read_byte_wise(&mut machine, 0x3000_0000, 16),
        Vec::<u8>::new()
    );
}

#[test]
fn step_frame_applies_the_current_button_mask() {
    // 帧开始时就必须把掩码写到 pad 上，固件在帧内第一次读引脚即可看到按下。
    // The mask must reach the pads at the start of a frame, so the firmware's
    // first pin read inside the frame already sees the press.
    let mut machine = machine();
    let config = PlayConfig {
        button_pins: 1 << 3,
        ..PlayConfig::new(FrameSpec::new(SRAM_BASE + 0x1000, 4, 4), 32)
    };
    let stop = Arc::new(AtomicBool::new(false));
    let buttons = Arc::new(AtomicU16::new(1 << 3));
    let mut trace_left = 0;
    assert_eq!(
        step_frame(&mut machine, &config, &stop, &buttons, &mut trace_left),
        None
    );
    assert_eq!(machine.steps(), 32);
    let levels = machine.peripherals().pads().input_levels();
    assert_eq!(levels & (1 << 3), 0, "pressed is low / 按下为低");
}

#[test]
fn step_frame_reports_a_system_reset_and_stops() {
    // `play` 的停止策略：帧内检测到 AIRCR.SYSRESETREQ 就返回诊断，worker 随
    // `halted` 停止，绝不自动重启。
    // The `play` stop policy: a frame that sees AIRCR.SYSRESETREQ returns the
    // diagnostic and the worker stops via `halted`; it never reboots.
    let mut machine = Machine::new(&aircr_uf2(), &spin_options(u64::MAX)).expect("machine");
    let config = PlayConfig::new(FrameSpec::new(SRAM_BASE + 0x1000, 4, 4), 64);
    let stop = Arc::new(AtomicBool::new(false));
    let buttons = Arc::new(AtomicU16::new(0));
    let mut trace_left = 0;
    let reason = step_frame(&mut machine, &config, &stop, &buttons, &mut trace_left);
    assert_eq!(reason.as_deref(), Some(SYSTEM_RESET_MESSAGE));
    assert!(
        machine.steps() < 64,
        "the frame stops at the request / 帧在请求处停止"
    );
}

#[test]
fn buttons_are_resampled_inside_a_frame() {
    // 一帧内部（而不是帧边界）就要重新读取按键掩码：宿主在帧中途按下时，
    // 本帧内后续的切片必须已经看到。
    // The button mask must be re-read inside a frame, not only at frame
    // boundaries: a press arriving mid-frame must be visible to a later slice.
    let mut machine = machine();
    let config = PlayConfig {
        button_pins: 1 << 4,
        ..PlayConfig::new(FrameSpec::new(SRAM_BASE + 0x1000, 4, 4), 400_000)
    };
    let stop = Arc::new(AtomicBool::new(false));
    let buttons = Arc::new(AtomicU16::new(0));
    let mut trace_left = 0;

    let host = Arc::clone(&buttons);
    let presser = thread::spawn(move || {
        thread::sleep(Duration::from_millis(2));
        host.store(1 << 4, Ordering::Relaxed);
    });
    assert_eq!(
        step_frame(&mut machine, &config, &stop, &buttons, &mut trace_left),
        None
    );
    presser.join().expect("button thread");
    // 一帧耗时远大于 2 ms，因此本帧内至少有一次采样看到按下。
    // A frame takes far longer than 2 ms, so at least one in-frame sample saw
    // the press and left the pin low.
    let levels = machine.peripherals().pads().input_levels();
    assert_eq!(levels & (1 << 4), 0, "press sampled inside the frame");
}

/// UF2 image whose reset handler requests `AIRCR.SYSRESETREQ`
/// 复位处理程序请求 `AIRCR.SYSRESETREQ` 的 UF2 映像
///
/// Thumb program / Thumb 程序: `ldr r0,[pc,#12]` → `0xE000_ED0C`,
/// `ldr r1,[pc,#16]` → `0x05FA_0004`, `str r1,[r0]`, `b .`, then the literals.
/// `play` must report the reset and stop the worker instead of rebooting.
/// `play` 必须报告该复位并停止 worker，而不是自动重启。
fn aircr_uf2() -> Vec<u8> {
    let mut vectors = Vec::with_capacity(8);
    vectors.extend_from_slice(&0x2008_2000u32.to_le_bytes());
    vectors.extend_from_slice(&(FLASH_BASE + 0x09).to_le_bytes());
    let mut code = vec![0u8; 0x18];
    // ldr r0,[pc,#12] / ldr r1,[pc,#16] / str r1,[r0] / b .
    code[0..8].copy_from_slice(&[0x03, 0x48, 0x04, 0x49, 0x01, 0x60, 0xFE, 0xE7]);
    code[0x10..0x14].copy_from_slice(&0xE000_ED0Cu32.to_le_bytes());
    code[0x14..0x18].copy_from_slice(&0x05FA_0004u32.to_le_bytes());
    let mut image = uf2_block(0, 2, FLASH_BASE, &vectors);
    image.extend(uf2_block(1, 2, FLASH_BASE + 8, &code));
    image
}

/// UF2 image whose reset handler stores one byte into the PSRAM window
/// 复位处理程序向 PSRAM 窗口写入一个字节的 UF2 映像
///
/// Thumb program / Thumb 程序: `ldr r0, [pc, #4]` → `0x1100_0000`,
/// `movs r1, #0x5A`, `strb r1, [r0]`, `b .`, then the literal. Without a mounted
/// PSRAM window the `strb` faults immediately and stops the worker, so the
/// session statistics distinguish "mounted" from "unmounted" by themselves.
/// 未挂载 PSRAM 时这条 `strb` 立即触发总线错误并让 worker 停机，因此「是否挂载」
/// 可以直接由会话统计区分。
fn psram_store_uf2() -> Vec<u8> {
    let mut vectors = Vec::with_capacity(8);
    vectors.extend_from_slice(&0x2008_2000u32.to_le_bytes());
    vectors.extend_from_slice(&(FLASH_BASE + 0x09).to_le_bytes());
    // ldr r0,[pc,#4] / movs r1,#0x5A / strb r1,[r0] / b . / literal
    let mut code = Vec::with_capacity(12);
    code.extend_from_slice(&[0x01, 0x48, 0x5A, 0x21, 0x01, 0x70, 0xFE, 0xE7]);
    code.extend_from_slice(&0x1100_0000u32.to_le_bytes());
    let mut image = uf2_block(0, 2, FLASH_BASE, &vectors);
    image.extend(uf2_block(1, 2, FLASH_BASE + 8, &code));
    image
}

#[test]
fn play_options_forward_the_psram_size_to_the_machine() {
    let psram = Some(crate::run::PsramOptions {
        size_bytes: 8 * 1024 * 1024,
    });
    let options = PlayOptions {
        file: "fw.uf2".into(),
        frame: FrameSpec::new(SRAM_BASE + 0x1000, 4, 4),
        steps_per_frame: 200,
        max_frames: None,
        buttons: 0,
        audio: None,
        arch: None,
        strict_decode: false,
        max_cycles: 1000,
        trace_steps: 0,
        core_threads: crate::run::CoreThreadOptions::default(),
        psram,
    };
    let mapped = machine_options(&options);
    assert_eq!(mapped.psram, psram);
    assert_eq!(mapped.max_cycles, 1000);
}

#[test]
fn psram_option_reaches_the_play_worker() {
    // 固件向 `0x1100_0000` 写一个字节：挂载 PSRAM 时会话正常发布帧，未挂载时
    // 同一条 `strb` 就是总线错误。这是 play 路径的端到端接线测试。
    // The firmware stores one byte at `0x1100_0000`: with PSRAM the session
    // publishes frames normally, without it the same `strb` is a bus error.
    // This is the end-to-end wiring test for the `play` path.
    let config = PlayConfig {
        max_frames: Some(2),
        ..PlayConfig::new(FrameSpec::new(SRAM_BASE + 0x1000, 4, 4), 200)
    };
    let mut options = spin_options(u64::MAX);
    options.psram = Some(crate::run::PsramOptions {
        size_bytes: 64 * 1024,
    });
    let session = PlaySession::start(psram_store_uf2(), options, config).expect("session");
    let stats = wait_for(&session, |stats| {
        stats.frames >= 2 || stats.halted.is_some()
    });
    assert_eq!(stats.halted, None, "the PSRAM window must absorb the store");
    let _ = session.shutdown();

    let session = PlaySession::start(psram_store_uf2(), spin_options(u64::MAX), config)
        .expect("unmounted session");
    let stats = wait_for(&session, |stats| stats.halted.is_some());
    let reason = stats.halted.as_deref().unwrap_or_default().to_uppercase();
    assert!(
        reason.contains("11000000"),
        "the fixture must fault on the unmapped window: {reason}"
    );
    let _ = session.shutdown();
}
