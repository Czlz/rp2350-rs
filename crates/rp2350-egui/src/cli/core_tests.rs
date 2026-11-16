//! Tests for the core-executor command-line options
//! 核执行器命令行选项测试
//!
//! The GUI flag is a plain boolean (`--core-threads`) while a numeric window is
//! validated by the CLI's own `parse_core_window`, so the executable rejects a
//! bad value before any window exists. Profile values use the same parser after
//! the GUI removes the two keys from the CLI profile schema.
//! GUI 开关是纯布尔值（`--core-threads`），数值窗口复用 CLI 的
//! `parse_core_window` 校验，因此可执行文件在任何窗口创建前拒绝非法值。profile
//! 值在 GUI 从 CLI profile 模式中摘出这两个键后使用同一个解析器。

use std::path::PathBuf;

use rp2350_cli::run::parse_core_window;

use super::*;

/// Required explicit arguments shared by the tests / 测试共用的显式必填参数
const REQUIRED: [&str; 6] = [
    "--file",
    "demo.uf2",
    "--fb",
    "0x20000000:240x160",
    "--steps-per-frame",
    "1000000",
];

/// Write a temporary profile and return its path / 写临时 profile 并返回路径
fn temp_profile(tag: &str, text: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rp2350-egui-core-{tag}-{}.conf",
        std::process::id()
    ));
    std::fs::write(&path, text).expect("temp profile write");
    path
}

/// Resolve one argument list into a launch / 解析一组参数为启动请求
fn launch(args: &[&str]) -> PlayLaunch {
    let mut full = vec!["rp2350-egui"];
    full.extend_from_slice(args);
    parse_and_resolve(full)
        .expect("arguments resolve")
        .expect("a launch request")
}

#[test]
fn core_threads_defaults_to_off_with_window_64() {
    let launch = launch(&REQUIRED);
    assert!(
        !launch.fields.core_threads,
        "the deterministic single-threaded interleaving must stay the default"
    );
    assert_eq!(launch.fields.core_window, "64");
}

#[test]
fn core_threads_flag_turns_the_executor_on() {
    let mut args = REQUIRED.to_vec();
    args.push("--core-threads");
    let launch = launch(&args);
    assert!(launch.fields.core_threads);
    assert!(launch.auto_start);
}

#[test]
fn core_window_flag_sets_the_window() {
    for window in ["1", "32", "4096"] {
        let mut args = REQUIRED.to_vec();
        args.extend(["--core-window", window]);
        let launch = launch(&args);
        assert_eq!(launch.fields.core_window, window);
    }
}

#[test]
fn invalid_core_window_values_are_rejected() {
    for value in ["0", "4097", "-1", "abc"] {
        let error = Cli::try_parse_from(["rp2350-egui", "--core-window", value]).expect_err(value);
        assert_eq!(error.exit_code(), 2, "{value}: {error}");
    }
    // 解析器本身也必须在所有非法输入上失败，包括负数与空值。
    // The parser itself must reject every invalid input, negatives and empty too.
    for value in ["0", "4097", "-1", "abc", ""] {
        assert!(
            parse_core_window(value).is_err(),
            "{value} must be rejected"
        );
    }
}

#[test]
fn profile_loads_core_threads_and_window() {
    let path = temp_profile(
        "profile",
        "firmware = E:\\firmware\\demo.uf2\n\
         frame = 0x20000000:240x160\n\
         steps_per_frame = 100\n\
         core_threads = true\n\
         core_window = 32\n",
    );
    let path_text = path.display().to_string();
    let launch = launch(&["--profile", &path_text]);
    assert!(launch.fields.core_threads);
    assert_eq!(launch.fields.core_window, "32");
    let _ = std::fs::remove_file(path);
}

#[test]
fn command_line_overrides_profile_core_settings() {
    let path = temp_profile(
        "override",
        "firmware = profile.uf2\n\
         frame = 0x20000000:240x160\n\
         steps_per_frame = 100\n\
         core_threads = false\n\
         core_window = 128\n",
    );
    let path_text = path.display().to_string();
    let launch = launch(&[
        "--profile",
        &path_text,
        "--core-threads",
        "--core-window",
        "32",
    ]);
    assert!(launch.fields.core_threads, "the flag must win over profile");
    assert_eq!(launch.fields.core_window, "32");
    let _ = std::fs::remove_file(path);
}

#[test]
fn invalid_profile_core_window_names_its_line() {
    let path = temp_profile(
        "bad-window",
        "# comment\n\
         firmware = demo.uf2\n\
         frame = 0x20000000:240x160\n\
         steps_per_frame = 100\n\
         core_window = 0\n",
    );
    let path_text = path.display().to_string();
    let error = parse_and_resolve(["rp2350-egui", "--profile", &path_text])
        .expect_err("a zero window must be rejected before the window opens");
    assert!(error.contains("profile line 5"), "error={error}");
    assert!(error.contains("核窗口大小非法"), "error={error}");
    let _ = std::fs::remove_file(path);
}
