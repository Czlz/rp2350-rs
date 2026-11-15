//! Profile parser tests / profile 解析器测试

use super::*;
use crate::run::{ArchArg, PsramOptions};

/// A profile that sets every supported key / 设置全部受支持键的 profile
const FULL: &str = "\
# full profile / 完整 profile
firmware = C:\\games\\game.uf2
frame = 0x20000000:240x160:480
steps_per_frame = 2500000
max_frames = 20
buttons = 0x3FF
audio = true
audio_block = 2
audio_sm = 0
audio_rate = 13440
audio_channels = 2
audio_capacity = 48000
arch = riscv
strict_decode = true
max_cycles = 0x1000
psram = 8M
";

#[test]
fn parses_every_supported_key() {
    let profile = parse(FULL).expect("profile");
    assert_eq!(
        profile.firmware.as_deref(),
        Some(std::path::Path::new(r"C:\games\game.uf2"))
    );
    assert_eq!(
        profile.frame,
        Some(FrameSpec::parse("0x20000000:240x160").expect("frame"))
    );
    assert_eq!(profile.steps_per_frame, Some(2_500_000));
    assert_eq!(profile.max_frames, Some(20));
    assert_eq!(profile.buttons, Some(0x3FF));
    assert_eq!(profile.audio, Some(true));
    assert_eq!(profile.audio_block, Some(2));
    assert_eq!(profile.audio_sm, Some(0));
    assert_eq!(profile.audio_rate, Some(13_440));
    assert_eq!(profile.audio_channels, Some(2));
    assert_eq!(profile.audio_capacity, Some(48_000));
    assert_eq!(profile.arch, Some(ArchArg::RiscV));
    assert_eq!(profile.strict_decode, Some(true));
    // `0x1000` 十六进制预算。 / Hex budget.
    assert_eq!(profile.max_cycles, Some(0x1000));
    assert_eq!(
        profile.psram,
        Some(PsramOptions {
            size_bytes: 8 * 1024 * 1024
        })
    );
}

#[test]
fn psram_accepts_bytes_suffixes_and_hex() {
    for (text, expected) in [
        ("psram = 8388608\n", 8 * 1024 * 1024),
        ("psram = 1024K\n", 1024 * 1024),
        ("psram = 16MiB\n", 16 * 1024 * 1024),
        ("psram = 0x10000\n", 0x1_0000),
    ] {
        let profile = parse(text).expect(text);
        assert_eq!(
            profile.psram.map(|psram| psram.size_bytes),
            Some(expected),
            "text={text}"
        );
    }
}

#[test]
fn psram_rejects_zero_and_misspelled_sizes() {
    for text in [
        "psram = 0\n",
        "psram = 8G\n",
        "psram = -1\n",
        "psram = 8x\n",
    ] {
        let error = parse(text).expect_err(text);
        assert_eq!(error.line, 1, "text={text}");
        assert!(error.message.contains("psram"), "{text}: {error}");
    }
}

#[test]
fn ignores_comments_blank_lines_and_surrounding_spaces() {
    let text = "  # comment / 注释  \n\n\tfirmware   =   fw.uf2  \n   \nmax_frames=3\n";
    let profile = parse(text).expect("profile");
    assert_eq!(
        profile.firmware.as_deref(),
        Some(std::path::Path::new("fw.uf2"))
    );
    assert_eq!(profile.max_frames, Some(3));
    assert_eq!(profile.frame, None);
    assert_eq!(profile.audio, None);
}

#[test]
fn reports_unknown_keys_with_the_line_number() {
    let error = parse("firmware = fw.uf2\n# gap\ncolour = green\n").expect_err("unknown key");
    assert_eq!(error.line, 3);
    assert!(error.message.contains("colour"), "{error}");
}

#[test]
fn reports_duplicate_keys_with_the_line_number() {
    let error =
        parse("max_frames = 2\nmax_cycles = 10\nmax_frames = 3\n").expect_err("duplicate key");
    assert_eq!(error.line, 3);
    assert!(error.message.contains("max_frames"), "{error}");
}

#[test]
fn reports_bad_numbers() {
    for (text, line) in [
        ("max_frames = many", 1),
        ("audio_block = 2x", 1),
        ("audio_channels = 999", 1),
        ("audio_rate = -1", 1),
    ] {
        let error = parse(text).expect_err("bad number");
        assert_eq!(error.line, line, "text={text}");
    }
}

#[test]
fn reports_missing_equals() {
    let error = parse("firmware fw.uf2\n").expect_err("missing equals");
    assert_eq!(error.line, 1);
    assert!(error.message.contains('='), "{error}");
}

#[test]
fn reports_invalid_frame_specs() {
    for bad in [
        "0x20000000",
        "0x20000000:240",
        "nope:240x160",
        "0x20000000:240x0",
    ] {
        let error = parse(&format!("frame = {bad}\n")).expect_err("bad frame");
        assert_eq!(error.line, 1, "bad={bad}");
        assert!(error.message.contains("frame"), "{error}");
    }
}

#[test]
fn audio_without_wiring_parses_and_is_reported_by_the_merge() {
    // profile 解析只做逐行语法/数值校验；`audio = true` 缺少 block/sm 是合并阶段
    // 的错误（命令行的 `--audio-block` 仍可能补上）。
    // Parsing is per-line; a missing block/sm is a merge error because the
    // command line may still supply it.
    let profile = parse("audio = true\n").expect("profile");
    assert_eq!(profile.audio, Some(true));
    assert_eq!(profile.audio_block, None);
    assert_eq!(profile.audio_sm, None);
}

#[test]
fn crlf_input_parses_the_same_as_lf() {
    let lf = "firmware = fw.uf2\nbuttons = 0x3FF\naudio = false\n";
    let crlf = lf.replace('\n', "\r\n");
    let profile = parse(&crlf).expect("crlf profile");
    assert_eq!(profile, parse(lf).expect("lf profile"));
    assert_eq!(profile.buttons, Some(0x3FF));
    assert_eq!(profile.audio, Some(false));

    let error = parse("first = 1\r\nsecond = 2\r\n").expect_err("unknown key");
    assert_eq!(error.line, 1);
}

#[test]
fn buttons_accept_decimal_and_hex() {
    assert_eq!(parse("buttons = 1023\n").expect("dec").buttons, Some(1023));
    assert_eq!(
        parse("buttons = 0x3FF\n").expect("hex").buttons,
        Some(0x3FF)
    );
    assert!(parse("buttons = 0x10000\n").is_err());
}

#[test]
fn arch_accepts_arm_and_riscv_spellings() {
    assert_eq!(parse("arch = arm\n").expect("arm").arch, Some(ArchArg::Arm));
    assert_eq!(
        parse("arch = RISC-V\n").expect("riscv").arch,
        Some(ArchArg::RiscV)
    );
    assert!(parse("arch = x86\n").is_err());
}

#[test]
fn empty_key_or_value_is_rejected() {
    assert_eq!(parse(" = 1\n").expect_err("empty key").line, 1);
    let error = parse("firmware =   \n").expect_err("empty value");
    assert_eq!(error.line, 1);
    assert!(error.message.contains("firmware"), "{error}");
}

#[test]
fn booleans_are_case_insensitive_and_strict() {
    assert_eq!(parse("audio = TRUE\n").expect("upper").audio, Some(true));
    assert_eq!(
        parse("strict_decode = False\n")
            .expect("mixed")
            .strict_decode,
        Some(false)
    );
    assert!(parse("audio = yes\n").is_err());
}

#[test]
fn display_mentions_the_line_and_both_languages() {
    let error = parse("bogus = 1\n").expect_err("unknown key");
    let text = error.to_string();
    assert!(text.contains("line 1"), "{text}");
    assert!(text.contains("第 1 行"), "{text}");
}
