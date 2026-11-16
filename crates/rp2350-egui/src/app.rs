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
//! The Virtual Lab canvas, by contrast, is an editor model: it can move devices
//! and add/remove wires, but it never touches the simulator.
//! 所有面板均为只读：从不写寄存器、每帧刷新，并置于滚动区内，窗口很小时也不会
//! 溢出。虚拟实验室画布则是一个编辑模型：可移动设备、增删连线，但从不接触仿真器。

use rp2350_arm::CortexM33Core;
use rp2350_core::types::CpuId;
use rp2350_cpu::CpuCore;

use crate::canvas::CanvasState;
use crate::canvas_io::CanvasParseError;
use crate::panels::{
    DmaView, HstxView, NOT_EXPOSED, PioView, RegisterPanel, format_dma_busy, format_dma_channel,
    format_dma_count, format_hstx_fifo, format_hstx_row, format_hstx_scanline, format_pio_block,
    format_pio_row,
};
use crate::play_view::{PlayFields, PlayStarter, PlayView};

/// Top-level view selected by the tab bar / 标签栏选中的顶层视图
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveView {
    /// Virtual Lab wiring canvas / 虚拟实验室连线画布
    #[default]
    VirtualLab,
    /// Play tab: live game picture and keyboard input / 游戏标签页：实时画面与键盘输入
    Play,
    /// Register and peripheral panels / 寄存器与外设面板
    Panels,
}

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
    /// Virtual Lab canvas model, preserved while switching views
    /// 虚拟实验室画布模型，切换视图时保留
    canvas: CanvasState,
    /// Last canvas text produced by the toolbar "Save"
    /// 工具栏「保存」产生的最近一份画布文本
    pub(crate) canvas_snapshot: String,
    /// Result of the last save/load shown in the toolbar
    /// 工具栏中显示的最近一次保存/加载结果
    pub(crate) canvas_status: String,
    /// Play tab: live frames, keyboard and status bar, preserved across tabs
    /// 游戏标签页：实时画面、键盘与状态栏，切换标签页时保留
    play: PlayView,
    /// Whether the first frame should start the prefilled play session
    /// 首帧是否应启动已预填的游玩会话
    play_auto_start: bool,
    /// Active top-level view / 当前顶层视图
    view: ActiveView,
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
            canvas: CanvasState::sample(),
            canvas_snapshot: String::new(),
            canvas_status: String::from("snapshot: none / 快照：无"),
            play: PlayView::default(),
            play_auto_start: false,
            view: ActiveView::default(),
        }
    }

    /// Canvas model, for host integration and tests / 画布模型，供宿主集成与测试使用
    #[must_use]
    pub fn canvas(&self) -> &CanvasState {
        &self.canvas
    }

    /// Mutable canvas model / 可变画布模型
    pub fn canvas_mut(&mut self) -> &mut CanvasState {
        &mut self.canvas
    }

    /// Active top-level view / 当前顶层视图
    #[must_use]
    pub fn active_view(&self) -> ActiveView {
        self.view
    }

    /// Play tab model, for host integration and tests / 游戏标签页模型，供宿主集成与测试
    #[must_use]
    pub fn play(&self) -> &PlayView {
        &self.play
    }

    /// Mutable Play tab model / 可变游戏标签页模型
    pub fn play_mut(&mut self) -> &mut PlayView {
        &mut self.play
    }

    /// Attach the play worker factory / 挂载游玩 worker 工厂
    ///
    /// The native entry point passes [`CliPlayStarter`](crate::CliPlayStarter),
    /// which boots `rp2350_cli::play::PlaySession`; tests and alternate hosts can
    /// inject any other [`PlayStarter`].
    /// 原生入口传入 [`CliPlayStarter`](crate::CliPlayStarter)（启动
    /// `rp2350_cli::play::PlaySession`）；测试与其它宿主可注入任意
    /// [`PlayStarter`]。
    #[must_use]
    pub fn with_play_starter(mut self, starter: Box<dyn PlayStarter>) -> Self {
        self.play.set_starter(starter);
        self
    }

    /// Pre-fill the Play toolbar fields / 预填「游戏」工具栏字段
    ///
    /// The command-line frontend resolves the profile and the flags into the same
    /// [`PlayFields`] the toolbar edits, so the GUI shows exactly the values that
    /// Start would use. No session is created here.
    /// 命令行前端把 profile 与参数解析为工具栏同一套 [`PlayFields`]，因此 GUI 显示的
    /// 就是 Start 会使用的值。这里不创建会话。
    #[must_use]
    pub fn with_play_fields(mut self, fields: PlayFields) -> Self {
        *self.play.fields_mut() = fields;
        self
    }

    /// Request an automatic play start before the first frame
    /// 请求在首帧之前自动启动游玩
    ///
    /// Set by the native entry point when the command line resolved a launch
    /// request; [`EmuApp::auto_start_play`] consumes the request. False (the
    /// default) keeps the historical manual "Load profile → Start" flow.
    /// 命令行解析出启动请求时由原生入口设置；[`EmuApp::auto_start_play`] 消费该请求。
    /// 默认 `false`，保持既有的手动「载入配置 → Start」流程。
    #[must_use]
    pub fn with_play_auto_start(mut self, auto_start: bool) -> Self {
        self.play_auto_start = auto_start;
        self
    }

    /// Start the prefilled play session once, before the first frame
    /// 在首帧之前启动一次已预填的游玩会话
    ///
    /// The window creator calls this after the native window exists and before
    /// the first frame, which is exactly "Load profile + Start" without the two
    /// clicks. The request is consumed, so repeated calls are no-ops. The Play tab
    /// becomes active so the picture is visible, and a failed start (unreadable
    /// firmware, bad request, starter error) only writes the status line: the
    /// window stays open.
    /// 窗口创建回调在原生窗口存在之后、首帧之前调用它，等价于「载入配置 + Start」而
    /// 无需两次点击。请求会被消费，重复调用是空操作。视图切到「游戏」标签页以便看到
    /// 画面；启动失败（固件不可读、请求非法、starter 报错）只写状态行，窗口保持打开。
    ///
    /// Returns `true` when an automatic start was attempted, `false` when the
    /// request was absent or already consumed.
    /// 尝试了自动启动时返回 `true`；请求不存在或已消费时返回 `false`。
    #[must_use]
    pub fn auto_start_play(&mut self) -> bool {
        if !self.play_auto_start {
            return false;
        }
        self.play_auto_start = false;
        self.view = ActiveView::Play;
        self.play.start();
        true
    }

    /// Switch the active top-level view / 切换当前顶层视图
    pub fn set_active_view(&mut self, view: ActiveView) {
        self.view = view;
    }

    /// Save the canvas into the in-memory snapshot / 把画布保存到内存快照
    ///
    /// No file dialog is opened: the text lives inside `EmuApp` and the status
    /// line reports its size and record counts.
    /// 不打开文件对话框：文本仅保存在 `EmuApp` 内，状态行显示大小与记录数。
    pub(crate) fn save_canvas(&mut self) {
        self.canvas_snapshot = self.canvas.to_text();
        let bytes = self.canvas_snapshot.len();
        let devices = self.canvas.nodes.len();
        let wires = self.canvas.wires.len();
        self.canvas_status = format!(
            "saved {bytes} bytes / 已保存 {bytes} 字节 · {devices} devices / 设备 · \
             {wires} wires / 连线"
        );
    }

    /// Load the in-memory snapshot back into the canvas / 从内存快照恢复画布
    ///
    /// # Errors
    /// Returns the [`CanvasParseError`] of the snapshot and keeps the current
    /// canvas untouched; the same message is written to the toolbar status.
    /// 返回快照的 [`CanvasParseError`]，当前画布保持不变，同一错误也会写入
    /// 工具栏状态。
    pub(crate) fn load_canvas(&mut self) -> Result<(), CanvasParseError> {
        match CanvasState::from_text(&self.canvas_snapshot) {
            Ok(canvas) => {
                let devices = canvas.nodes.len();
                let wires = canvas.wires.len();
                self.canvas = canvas;
                self.canvas_status =
                    format!("loaded / 已加载 · {devices} devices / 设备 · {wires} wires / 连线");
                Ok(())
            }
            Err(error) => {
                self.canvas_status = format!("load failed / 加载失败 — {error}");
                Err(error)
            }
        }
    }

    /// Toolbar with the in-memory save/load buttons and their status
    /// 带内存保存/加载按钮及状态的工具栏
    fn show_canvas_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Save / 保存").clicked() {
                self.save_canvas();
            }
            if ui.button("Load / 加载").clicked() {
                let _ = self.load_canvas();
            }
            ui.separator();
            ui.label(self.canvas_status.as_str());
        });
        ui.separator();
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
        let mut app = self;

        eframe::run_native(
            &app_name,
            options,
            Box::new(move |_cc| {
                // 窗口已创建、首帧之前：命令行直启在这里启动 worker，失败只写状态行。
                // The window exists and the first frame has not been drawn yet: the
                // command-line launch starts the worker here; a failure only writes
                // the status line.
                let _ = app.auto_start_play();
                Ok(Box::new(app))
            }),
        )
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

    /// Render the top tab bar / 渲染顶部标签栏
    fn show_tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.view,
                ActiveView::VirtualLab,
                "Virtual Lab / 虚拟实验室",
            );
            ui.selectable_value(&mut self.view, ActiveView::Play, "Play / 游戏");
            ui.selectable_value(&mut self.view, ActiveView::Panels, "Panels / 寄存器与外设");
        });
        ui.separator();
    }

    /// Render whichever view the tab bar selected / 渲染标签栏选中的视图
    fn show_active_view(&mut self, ui: &mut egui::Ui) {
        match self.view {
            ActiveView::VirtualLab => {
                self.show_canvas_toolbar(ui);
                self.canvas.show(ui);
            }
            ActiveView::Play => self.play.ui(ui),
            ActiveView::Panels => self.show_panels(ui),
        }
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
            self.show_tab_bar(ui);
            self.show_active_view(ui);
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
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::panels::{DmaChannelView, PioBlockView, PioMachineView, format_pio_irq};
    use crate::play_view::{PlayFrame, PlayRequest, PlaySource, PlayStats};

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

    /// Render the active tab into a headless context / 在无窗口上下文中渲染当前标签页
    fn render_view(app: &mut EmuApp, size: egui::Vec2) -> usize {
        let context = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..egui::RawInput::default()
        };
        let output = context.run_ui(raw, |ui| app.show_active_view(ui));
        let shapes = output.shapes.len();
        output.drop_without_applying_deltas();
        assert!(shapes > 0, "active view should emit draw shapes");
        shapes
    }

    #[test]
    fn virtual_lab_tab_renders_headlessly() {
        let mut app = EmuApp::sample();
        assert_eq!(app.active_view(), ActiveView::VirtualLab);
        assert_eq!(app.canvas().nodes.len(), 3);
        render_view(&mut app, egui::vec2(760.0, 600.0));
        render_view(&mut app, egui::vec2(80.0, 60.0));
    }

    #[test]
    fn switching_views_preserves_canvas_state() {
        let mut app = EmuApp::sample();
        let nodes = app.canvas().nodes.len();
        let wires = app.canvas().wires.len();
        app.set_active_view(ActiveView::Panels);
        render_view(&mut app, egui::vec2(760.0, 600.0));
        app.set_active_view(ActiveView::VirtualLab);
        assert_eq!(app.canvas().nodes.len(), nodes);
        assert_eq!(app.canvas().wires.len(), wires);
        render_view(&mut app, egui::vec2(760.0, 600.0));
    }

    #[test]
    fn play_tab_renders_headlessly_and_keeps_its_state() {
        let mut app = EmuApp::sample();
        app.set_active_view(ActiveView::Play);
        assert_eq!(app.active_view(), ActiveView::Play);
        render_view(&mut app, egui::vec2(760.0, 600.0));
        render_view(&mut app, egui::vec2(80.0, 60.0));
        assert_eq!(app.play().status(), "idle / 空闲");
        assert!(!app.play().is_running());

        // 切走再切回不重建 PlayView：状态与控件文本保持。
        // Switching away and back keeps the same PlayView and its state.
        app.set_active_view(ActiveView::Panels);
        render_view(&mut app, egui::vec2(760.0, 600.0));
        app.set_active_view(ActiveView::Play);
        assert_eq!(app.play().status(), "idle / 空闲");
        render_view(&mut app, egui::vec2(760.0, 600.0));
    }

    /// Fake worker that reports "running" and publishes no frame
    /// 报告「运行中」且不发布帧的假 worker
    struct IdleSource;

    impl PlaySource for IdleSource {
        fn latest_frame(&self) -> Option<Arc<PlayFrame>> {
            None
        }

        fn set_buttons(&self, _mask: u16) {}

        fn stats(&self) -> PlayStats {
            PlayStats::default()
        }

        fn is_running(&self) -> bool {
            true
        }

        fn button_pins(&self) -> u16 {
            0
        }

        fn shutdown(self: Box<Self>) -> PlayStats {
            PlayStats::default()
        }
    }

    /// Temporary firmware path with dummy bytes / 带占位字节的临时固件路径
    fn temp_firmware(tag: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("rp2350-egui-{tag}-{}.uf2", std::process::id()));
        std::fs::write(&path, b"not a real uf2").expect("temp firmware write");
        path
    }

    /// App with resolved fields, a counting starter and the given auto-start flag
    /// 带已解析字段、计数 starter 与指定自动启动标记的应用
    fn launch_app(auto_start: bool, calls: &Arc<AtomicUsize>, tag: &str) -> (EmuApp, PathBuf) {
        let path = temp_firmware(tag);
        let counter = Arc::clone(calls);
        let starter = move |_request: PlayRequest| {
            counter.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(IdleSource) as Box<dyn PlaySource>)
        };
        let fields = PlayFields {
            firmware: path.display().to_string(),
            frame: String::from("0x20000000:4x4"),
            steps_per_frame: String::from("100"),
            ..PlayFields::default()
        };
        let app = EmuApp::sample()
            .with_play_starter(Box::new(starter))
            .with_play_fields(fields)
            .with_play_auto_start(auto_start);
        (app, path)
    }

    #[test]
    fn auto_start_play_calls_the_starter_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (mut app, path) = launch_app(true, &calls, "auto-on");

        assert!(app.auto_start_play(), "the request must be attempted");
        assert_eq!(calls.load(Ordering::Relaxed), 1, "starter call count");
        assert_eq!(app.active_view(), ActiveView::Play);
        assert!(app.play().is_running());
        assert!(
            app.play().status().starts_with("running / 运行中"),
            "status={}",
            app.play().status()
        );

        // 请求只消费一次：第二帧（或测试里第二次调用）不得再次启动。
        // The request is consumed once: the second frame (or a second test call)
        // must not start again.
        assert!(!app.auto_start_play());
        assert_eq!(calls.load(Ordering::Relaxed), 1, "still one starter call");

        app.play_mut().stop();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn auto_start_play_without_request_calls_no_starter() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (mut app, path) = launch_app(false, &calls, "auto-off");

        assert!(!app.auto_start_play());
        assert_eq!(calls.load(Ordering::Relaxed), 0, "no starter call");
        assert_eq!(app.active_view(), ActiveView::VirtualLab);
        assert!(!app.play().is_running());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn auto_started_play_renders_headlessly_in_a_tiny_window() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (mut app, path) = launch_app(true, &calls, "auto-render");

        assert!(app.auto_start_play());
        render_view(&mut app, egui::vec2(80.0, 60.0));
        render_view(&mut app, egui::vec2(760.0, 600.0));
        assert!(app.play().is_running());

        app.play_mut().stop();
        let _ = std::fs::remove_file(path);
    }
}
