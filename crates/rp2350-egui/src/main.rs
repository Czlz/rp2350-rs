//! Minimal native entry point for the rp2350-egui application
//! rp2350-egui 应用的最小原生入口

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), eframe::Error> {
    rp2350_egui::EmuApp::sample().run()
}

#[cfg(target_arch = "wasm32")]
fn main() {
    // Native windowing is unavailable on wasm; the library remains usable.
    // wasm 上无法使用原生窗口；库仍可使用。
}
