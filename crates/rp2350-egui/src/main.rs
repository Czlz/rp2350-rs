//! Minimal native entry point for the rp2350-egui application
//! rp2350-egui 应用的最小原生入口

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), eframe::Error> {
    // 挂载真实 worker：`rp2350-egui → rp2350-cli` 不再是循环依赖，因此「游戏」
    // 标签页可以直接读取固件并出画面。`EmuApp::sample()` 保持不变，仍可供测试与
    // `EmuApp::default()` 使用。
    // Attach the real worker: `rp2350-egui → rp2350-cli` is no longer a cycle, so
    // the Play tab can read firmware and show frames. `EmuApp::sample()` stays
    // untouched for tests and `EmuApp::default()`.
    rp2350_egui::EmuApp::sample()
        .with_play_starter(Box::new(rp2350_egui::CliPlayStarter))
        .run()
}

#[cfg(target_arch = "wasm32")]
fn main() {
    // Native windowing is unavailable on wasm; the library remains usable.
    // wasm 上无法使用原生窗口；库仍可使用。
}
