//! eframe application entry point
//! eframe 应用程序入口点
//!
//! The application owns a boxed [`CpuCore`] trait object, so the UI stays
//! independent of any concrete core implementation. Peripheral state is read
//! through the [`PeripheralSource`] trait, which keeps the GUI free of a
//! dependency on the peripheral crate: the host machine (for example the CLI
//! `Machine`) implements it, and panels render [`NOT_EXPOSED`] while no source
//! is attached.
//! 应用持有一个 [`CpuCore`] trait 对象，因此 UI 与任何具体核心实现解耦。外设状态
//! 通过 [`PeripheralSource`] trait 读取，使 GUI 不依赖外设 crate：由宿主机器
//! （例如 CLI 的 `Machine`）实现该 trait，未挂载来源时面板渲染
//! [`NOT_EXPOSED`]。
//!
//! All panels are read-only: they never write a register, they refresh on every
//! frame, and they live in a scroll area so a small window cannot overflow.
//! 所有面板均为只读：从不写寄存器、每帧刷新，并置于滚动区内，窗口很小时也不会
//! 溢出。

use rp2350_arm::CortexM33Core;
use rp2350_core::types::CpuId;
use rp2350_cpu::CpuCore;

use crate::panels::{
    DmaView, HstxView, NOT_EXPOSED, PioView, RegisterPanel, format_dma_busy, format_dma_channel,
    format_dma_count, format_hstx_fifo, format_hstx_row, format_hstx_scanline, format_pio_block,
    format_pio_row,
};

/// Read-only snapshot source for the DMA, HSTX and PIO panels
/// DMA、HSTX 与 PIO 面板的只读快照来源
///
/// Implement this for the host machine handle so the GUI can display
/// peripheral state without depending on the peripheral crate. The default
/// implementations return `None`, which renders [`NOT_EXPOSED`].
/// 由宿主机器句柄实现，使 GUI 可在不依赖外设 crate 的前提下展示外设状态。
/// 默认实现返回 `None`，面板渲染 [`NOT_EXPOSED`]。
pub trait PeripheralSource {
    /// DMA snapshot, or `None` when the handle is unavailable
    /// DMA 快照；句柄不可用时为 `None`
    fn dma_view(&self) -> Option<DmaView> {
        None
    }

    /// HSTX snapshot, or `None` when the handle is unavailable
    /// HSTX 快照；句柄不可用时为 `None`
    fn hstx_view(&self) -> Option<HstxView> {
        None
    }

    /// PIO snapshot, or `None` when the handle is unavailable
    /// PIO 快照；句柄不可用时为 `None`
    fn pio_view(&self) -> Option<PioView> {
        None
    }
}

/// The main egui application / 主 egui 应用程序
pub struct EmuApp {
    /// Window title / 窗口标题
    title: String,
    /// Inspected core (trait object keeps the UI core-agnostic)
    /// 被检视的核心（trait 对象使 UI 与具体核心解耦）
    core: Box<dyn CpuCore>,
    /// Optional peripheral snapshot source / 可选的外设快照来源
    peripherals: Option<Box<dyn PeripheralSource>>,
    /// Selected DMA channel shown by the DMA panel / DMA 面板选中的通道
    selected_dma_channel: usize,
}

impl EmuApp {
    /// Create an application for the given core / 为指定核心创建应用
    #[must_use]
    pub fn new(core: Box<dyn CpuCore>) -> Self {
        Self {
            title: String::from("RP2350 Emulator — Virtual Electronics Lab"),
            core,
            peripherals: None,
            selected_dma_channel: 0,
        }
    }

    /// Attach a read-only peripheral snapshot source / 挂载只读外设快照来源
    #[must_use]
    pub fn with_peripheral_source(mut self, source: Box<dyn PeripheralSource>) -> Self {
        self.peripherals = Some(source);
        self
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

    /// Render every panel into `ui` / 将所有面板渲染到 `ui`
    ///
    /// The whole stack lives in a vertical scroll area, so a small window
    /// scrolls instead of overflowing or panicking.
    /// 整栈面板置于纵向滚动区，窗口很小时滚动而不是溢出或 panic。
    fn show_panels(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                RegisterPanel::new(self.core.as_ref()).show(ui);

                let dma = self
                    .peripherals
                    .as_ref()
                    .and_then(|source| source.dma_view());
                let hstx = self
                    .peripherals
                    .as_ref()
                    .and_then(|source| source.hstx_view());
                let pio = self
                    .peripherals
                    .as_ref()
                    .and_then(|source| source.pio_view());

                ui.add_space(6.0);
                egui::CollapsingHeader::new("DMA / DMA 控制器")
                    .default_open(true)
                    .show(ui, |ui| {
                        dma_panel(ui, dma.as_ref(), &mut self.selected_dma_channel);
                    });

                ui.add_space(6.0);
                egui::CollapsingHeader::new("HSTX / HSTX 视频输出")
                    .default_open(true)
                    .show(ui, |ui| hstx_panel(ui, hstx.as_ref()));

                ui.add_space(6.0);
                egui::CollapsingHeader::new("PIO / PIO 状态机")
                    .default_open(true)
                    .show(ui, |ui| pio_panel(ui, pio.as_ref()));
            });
    }
}

/// Render the DMA panel / 渲染 DMA 面板
///
/// Shows the busy mask, the cumulative transfer count and one selected
/// channel snapshot. `selected` is clamped every frame, so a stale index from a
/// previous source cannot address past the channel list.
/// 显示忙碌掩码、累计传输数与选中通道快照。`selected` 每帧被钳制，因此上一来源
/// 遗留的越界索引不会访问到通道列表之外。
fn dma_panel(ui: &mut egui::Ui, view: Option<&DmaView>, selected: &mut usize) {
    let Some(view) = view else {
        ui.monospace(NOT_EXPOSED);
        return;
    };
    ui.monospace(format_dma_busy(view.busy_mask));
    ui.monospace(format_dma_count(view.transfers));

    if view.channels.is_empty() {
        ui.monospace(NOT_EXPOSED);
        return;
    }
    // 通道数来自当前快照，选中项据此钳制。
    // The channel count comes from the current snapshot, which clamps the selection.
    *selected = (*selected).min(view.channels.len() - 1);
    egui::ComboBox::from_id_salt("dma_channel_select")
        .selected_text(format!("ch{:02}", *selected))
        .show_ui(ui, |ui| {
            for index in 0..view.channels.len() {
                ui.selectable_value(selected, index, format!("ch{index:02}"));
            }
        });
    ui.monospace(format_dma_channel(&view.channels, *selected));
}

/// Render the HSTX panel / 渲染 HSTX 面板
fn hstx_panel(ui: &mut egui::Ui, view: Option<&HstxView>) {
    let Some(view) = view else {
        ui.monospace(NOT_EXPOSED);
        return;
    };
    ui.monospace(format_hstx_row(view));
    ui.monospace(format_hstx_fifo(view.fifo_level, view.fifo_depth));
    ui.monospace(format_hstx_scanline(view));
}

/// Render the PIO panel / 渲染 PIO 面板
fn pio_panel(ui: &mut egui::Ui, view: Option<&PioView>) {
    let Some(view) = view else {
        ui.monospace(NOT_EXPOSED);
        return;
    };
    if view.blocks.is_empty() {
        ui.monospace(NOT_EXPOSED);
        return;
    }
    for block in &view.blocks {
        ui.monospace(format_pio_block(block));
        for machine in &block.machines {
            ui.monospace(format_pio_row(block.index, machine));
        }
    }
}

impl eframe::App for EmuApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(self.title.as_str());
            ui.add_space(8.0);
            self.show_panels(ui);
        });
    }
}

impl Default for EmuApp {
    fn default() -> Self {
        Self::sample()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::{DmaChannelView, PioBlockView, PioMachineView, format_pio_irq};

    /// Source that returns sample snapshots / 返回示例快照的来源
    struct MockSource {
        dma: DmaView,
        hstx: HstxView,
        pio: PioView,
    }

    impl MockSource {
        fn new() -> Self {
            Self {
                dma: DmaView {
                    busy_mask: 0x0005,
                    transfers: 42,
                    channels: vec![
                        DmaChannelView {
                            index: 0,
                            read_addr: 0x2000_0000,
                            write_addr: 0x5000_0000,
                            trans_count: 4,
                            ctrl: 0x0400_0001,
                            busy: true,
                            enabled: true,
                        },
                        DmaChannelView {
                            index: 1,
                            ..DmaChannelView::default()
                        },
                    ],
                },
                hstx: HstxView {
                    enabled: true,
                    fifo_level: 3,
                    ..HstxView::default()
                },
                pio: PioView {
                    blocks: vec![PioBlockView {
                        index: 0,
                        machines: vec![PioMachineView::default()],
                        ..PioBlockView::default()
                    }],
                },
            }
        }
    }

    impl PeripheralSource for MockSource {
        fn dma_view(&self) -> Option<DmaView> {
            Some(self.dma.clone())
        }

        fn hstx_view(&self) -> Option<HstxView> {
            Some(self.hstx.clone())
        }

        fn pio_view(&self) -> Option<PioView> {
            Some(self.pio.clone())
        }
    }

    /// Render the three panels headlessly / 无窗口渲染三个面板
    fn render(view: &EmuApp, size: egui::Vec2) -> usize {
        let context = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..egui::RawInput::default()
        };
        let mut selected = view.selected_dma_channel;
        let output = context.run_ui(raw, |ui| {
            let dma = view
                .peripherals
                .as_ref()
                .and_then(|source| source.dma_view());
            let hstx = view
                .peripherals
                .as_ref()
                .and_then(|source| source.hstx_view());
            let pio = view
                .peripherals
                .as_ref()
                .and_then(|source| source.pio_view());
            dma_panel(ui, dma.as_ref(), &mut selected);
            hstx_panel(ui, hstx.as_ref());
            pio_panel(ui, pio.as_ref());
        });
        let shape_count = output.shapes.len();
        output.drop_without_applying_deltas();
        assert!(shape_count > 0, "panels should emit draw shapes");
        shape_count
    }

    #[test]
    fn panels_render_with_peripheral_source() {
        let app = EmuApp::sample().with_peripheral_source(Box::new(MockSource::new()));
        render(&app, egui::vec2(760.0, 600.0));
    }

    #[test]
    fn panels_render_placeholders_without_source() {
        let app = EmuApp::sample();
        render(&app, egui::vec2(760.0, 600.0));
    }

    #[test]
    fn panels_render_in_small_window() {
        let app = EmuApp::sample().with_peripheral_source(Box::new(MockSource::new()));
        render(&app, egui::vec2(120.0, 90.0));
    }

    #[test]
    fn dma_selection_clamped_to_channel_count() {
        let context = egui::Context::default();
        let view = MockSource::new().dma;
        let mut selected = 99;
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            dma_panel(ui, Some(&view), &mut selected);
        });
        output.drop_without_applying_deltas();

        assert_eq!(selected, view.channels.len() - 1);
    }

    #[test]
    fn dma_busy_mask_lists_channels() {
        assert_eq!(format_dma_busy(0x0005), "busy=0x0005 (2 set) ch:0,2");
        assert_eq!(format_dma_busy(0x8001), "busy=0x8001 (2 set) ch:0,15");
    }

    #[test]
    fn dma_busy_mask_empty() {
        assert_eq!(format_dma_busy(0), "busy=0x0000 (0 set) ch:none");
    }

    #[test]
    fn dma_count_shows_decimal_and_hex() {
        assert_eq!(format_dma_count(1234), "transfers=1234 (0x000004D2)");
    }

    #[test]
    fn dma_channel_row_is_hexadecimal() {
        let channel = DmaChannelView {
            index: 0,
            read_addr: 0x2000_0000,
            write_addr: 0x5000_0000,
            trans_count: 16,
            ctrl: 0x0400_0001,
            busy: true,
            enabled: true,
        };
        assert_eq!(
            format_dma_channel(&[channel], 0),
            "ch00 READ=0x20000000 WRITE=0x50000000 COUNT=0x00000010 \
             CTRL=0x04000001 BUSY=1 EN=1"
        );
    }

    #[test]
    fn dma_channel_selection_out_of_range() {
        assert_eq!(format_dma_channel(&[], 16), "ch16: out of range / 通道越界");
    }

    #[test]
    fn hstx_fifo_empty_and_full() {
        assert_eq!(format_hstx_fifo(0, 8), "EMPTY 0/8");
        assert_eq!(format_hstx_fifo(8, 8), "FULL 8/8");
        assert_eq!(format_hstx_fifo(3, 8), "3/8");
    }

    #[test]
    fn hstx_row_reports_underrun_and_missing_scanline() {
        let view = HstxView {
            enabled: true,
            fifo_level: 8,
            output_level: 12,
            underruns: 2,
            unknown_commands: 1,
            ..HstxView::default()
        };
        assert_eq!(
            format_hstx_row(&view),
            "EN=1 FIFO=8/8 OUT=12 UNDERRUN=2 UNKNOWN=1"
        );
        assert_eq!(
            format_hstx_scanline(&view),
            "scan-line not exposed / 未暴露"
        );
    }

    #[test]
    fn pio_rows_render_placeholder_without_api() {
        let machine = PioMachineView {
            index: 1,
            ..PioMachineView::default()
        };
        assert_eq!(format_pio_row(0, &machine), "PIO0 SM1 not exposed / 未暴露");

        let block = PioBlockView {
            index: 2,
            ..PioBlockView::default()
        };
        assert_eq!(format_pio_block(&block), "PIO2 not exposed / 未暴露");
        assert_eq!(format_pio_irq(None), "IRQ=not exposed / 未暴露");
    }

    #[test]
    fn pio_irq_mask_decodes_bits() {
        assert_eq!(format_pio_irq(Some(0x5)), "IRQ=0x00000005 (0,2)");
        assert_eq!(format_pio_irq(Some(0)), "IRQ=0x00000000 (none)");
    }

    #[test]
    fn pio_state_machine_row_is_hexadecimal() {
        let machine = PioMachineView {
            index: 2,
            pc: Some(0x0010),
            tx_level: Some(4),
            rx_level: Some(0),
        };
        assert_eq!(
            format_pio_row(1, &machine),
            "PIO1 SM2 PC=0x0010 (16) TX=4/4 RX=0/4"
        );
    }
}
