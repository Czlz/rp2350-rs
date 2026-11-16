//! Minimal native entry point for the rp2350-egui application
//! rp2350-egui 应用的最小原生入口
//!
//! The entry point parses the command line before any window exists. A
//! `--profile` or an explicit `--file/--fb/...` request pre-fills the Play tab
//! and starts it before the first frame; with no play option the application
//! behaves exactly as before (empty fields, manual Load profile + Start).
//! 入口在任何窗口创建之前解析命令行。`--profile` 或显式的 `--file/--fb/...` 请求
//! 会预填「游戏」标签页并在首帧之前启动；没有任何游玩选项时应用行为与以往完全一致
//! （空字段、手动「载入配置 + Start」）。

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), eframe::Error> {
    // 语义错误（缺字段、坏 profile、音频缺块/状态机）在开窗前以退出码 2 结束。
    // Semantic errors (missing field, bad profile, audio without wiring) end with
    // exit code 2 before a window is created. clap itself handles --help/--version
    // (exit 0) and syntax errors (exit 2).
    let launch = match rp2350_egui::cli::parse_and_resolve(std::env::args_os()) {
        Ok(launch) => launch,
        Err(message) => {
            eprintln!("[rp2350-egui] error / 错误: {message}");
            std::process::exit(2);
        }
    };

    // 挂载真实 worker：`rp2350-egui → rp2350-cli` 不再是循环依赖，因此「游戏」
    // 标签页可以直接读取固件并出画面。`EmuApp::sample()` 保持不变，仍可供测试与
    // `EmuApp::default()` 使用。
    // Attach the real worker: `rp2350-egui → rp2350-cli` is no longer a cycle, so
    // the Play tab can read firmware and show frames. `EmuApp::sample()` stays
    // untouched for tests and `EmuApp::default()`.
    let mut app =
        rp2350_egui::EmuApp::sample().with_play_starter(Box::new(rp2350_egui::CliPlayStarter));

    if let Some(launch) = launch {
        // 稳定的一行日志便于脚本/验收判断是否走了自动启动。
        // One stable log line lets scripts and acceptance checks detect the
        // automatic start.
        if let Some(log) = &launch.log {
            eprintln!("{log}");
        }
        app = app
            .with_play_fields(launch.fields)
            .with_play_auto_start(launch.auto_start);
    }

    app.run()
}

#[cfg(target_arch = "wasm32")]
fn main() {
    // Native windowing is unavailable on wasm; the library remains usable.
    // wasm 上无法使用原生窗口；库仍可使用。
}
