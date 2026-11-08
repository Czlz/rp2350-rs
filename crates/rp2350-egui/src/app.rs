//! eframe application entry point
//! eframe 应用程序入口点
//!
//! The application owns a boxed [`CpuCore`] trait object, so the UI stays
//! independent of any concrete core implementation.
//! 应用持有一个 [`CpuCore`] trait 对象，因此 UI 与任何具体核心实现解耦。

use rp2350_arm::CortexM33Core;
use rp2350_core::types::CpuId;
use rp2350_cpu::CpuCore;

use crate::panels::RegisterPanel;

/// The main egui application / 主 egui 应用程序
pub struct EmuApp {
    /// Window title / 窗口标题
    title: String,
    /// Inspected core (trait object keeps the UI core-agnostic)
    /// 被检视的核心（trait 对象使 UI 与具体核心解耦）
    core: Box<dyn CpuCore>,
}

impl EmuApp {
    /// Create an application for the given core / 为指定核心创建应用
    #[must_use]
    pub fn new(core: Box<dyn CpuCore>) -> Self {
        Self {
            title: String::from("RP2350 Emulator — Virtual Electronics Lab"),
            core,
        }
    }

    /// Create an application preloaded with sample Cortex-M33 state
    /// 创建一个预载 Cortex-M33 示例状态的应用
    #[must_use]
    pub fn sample() -> Self {
        let mut core = CortexM33Core::new(CpuId::Core0);
        core.write_register(0, 0x0000_0001);
        core.write_register(1, 0x2000_1000);
        core.write_register(2, 0xDEAD_BEEF);
        core.write_register(3, 0x1234_5678);
        core.write_register(4, 0x0000_0000);
        core.write_register(5, 0x0000_0000);
        core.write_register(6, 0x0000_0000);
        core.write_register(7, 0x0000_0000);
        core.write_register(13, 0x2004_2000);
        core.write_register(14, 0x0000_0125);
        core.write_register(15, 0x0000_0200);
        Self::new(Box::new(core))
    }

    /// Run the application with a native window / 以原生窗口运行应用
    ///
    /// # Errors
    /// Returns an [`eframe::Error`] if window creation or the event loop fails.
    /// 若窗口创建或事件循环失败，返回 [`eframe::Error`]。
    pub fn run(self) -> Result<(), eframe::Error> {
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([760.0, 600.0]),
            ..eframe::NativeOptions::default()
        };
        let app_name = self.title.clone();

        eframe::run_native(&app_name, options, Box::new(|_cc| Ok(Box::new(self))))
    }
}

impl eframe::App for EmuApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(self.title.as_str());
            ui.add_space(8.0);
            RegisterPanel::new(self.core.as_ref()).show(ui);
        });
    }
}

impl Default for EmuApp {
    fn default() -> Self {
        Self::sample()
    }
}
