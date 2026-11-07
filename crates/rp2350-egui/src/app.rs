//! eframe application entry point (Phase 3 stub)
//! eframe 应用程序入口点（Phase 3 桩）

/// The main egui application / 主 egui 应用程序
#[allow(dead_code)]
pub struct EmuApp {
    /// Window title / 窗口标题
    title: String,
}

impl EmuApp {
    /// Create a new emulator application / 创建新模拟器应用
    #[must_use]
    pub fn new() -> Self {
        Self {
            title: "RP2350 Emulator — Virtual Electronics Lab".to_string(),
        }
    }

    /// Run the application (stub — not yet rendering)
    /// 运行应用（桩 — 尚未渲染）
    pub fn run(self) {
        log::info!("EmuApp::run() stub — Virtual Lab not yet implemented");
    }
}

impl Default for EmuApp {
    fn default() -> Self {
        Self::new()
    }
}