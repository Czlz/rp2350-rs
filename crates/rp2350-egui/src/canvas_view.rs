//! Virtual Lab canvas painter — grid, nodes, ports, wires and input handling
//! 虚拟实验室画布绘制层 — 网格、设备、端口、连线与输入处理
//!
//! This module extends [`CanvasState`] with egui rendering and interaction. It
//! is separate from `canvas.rs` so both files stay under the project line cap.
//! 本模块为 [`CanvasState`] 提供 egui 绘制与交互；与 `canvas.rs` 分离，使两个
//! 文件都符合项目的单文件行数上限。

use egui::epaint::CubicBezierShape;
use egui::{
    Align2, Color32, FontId, Key, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2,
    pos2, vec2,
};

use crate::canvas::{CanvasState, DeviceNode, DragState, PortKind, Selection, Wire};
use crate::palette::DevicePalette;

/// World-space grid spacing / 世界坐标网格间距
const GRID_SPACING: f32 = 24.0;
/// Port dot radius at 100% zoom / 100% 缩放下的端口圆点半径
const PORT_RADIUS: f32 = 5.0;
const CANVAS_BG: Color32 = Color32::from_rgb(0x1B, 0x1F, 0x27);
const GRID_COLOR: Color32 = Color32::from_rgb(0x2A, 0x30, 0x3C);
const NODE_FILL: Color32 = Color32::from_rgb(0x2F, 0x36, 0x43);
const NODE_STROKE: Color32 = Color32::from_rgb(0x5A, 0x66, 0x78);
const NODE_TEXT: Color32 = Color32::from_rgb(0xE6, 0xE9, 0xEF);
const WIRE_COLOR: Color32 = Color32::from_rgb(0x6E, 0xC1, 0xFF);
const SELECTED_COLOR: Color32 = Color32::from_rgb(0xFF, 0xC1, 0x07);
const INPUT_COLOR: Color32 = Color32::from_rgb(0x66, 0xBB, 0x6A);
const OUTPUT_COLOR: Color32 = Color32::from_rgb(0xE5, 0x73, 0x73);
const BIDI_COLOR: Color32 = Color32::from_rgb(0xBA, 0x68, 0xC8);
const HINT_COLOR: Color32 = Color32::from_rgb(0x8A, 0x94, 0xA6);

/// Palette sidebar width for a given window width / 给定窗口宽度下的调色板侧栏宽度
///
/// The sidebar never claims more than 45% of a narrow window, so the canvas
/// keeps a usable strip even at 80×60.
/// 窄窗口下侧栏最多占 45%，因此 80×60 时画布仍有一条可用的绘制区域。
fn sidebar_width(available: f32) -> f32 {
    if available < 320.0 {
        (available * 0.45).max(16.0)
    } else {
        (available * 0.3).clamp(140.0, 190.0)
    }
}

impl CanvasState {
    /// Render and interact with the canvas inside `ui` / 在 `ui` 内渲染并交互画布
    ///
    /// A fixed-width palette sidebar is drawn on the left; clicking an entry
    /// drops a new device near the canvas centre. The canvas takes the rest of
    /// the space, and a tiny window only shrinks the viewport, it never panics.
    /// 左侧绘制固定宽度的调色板侧栏，点击条目会在画布中心附近放置新设备；
    /// 画布占用其余空间，窗口极小时只缩小视口，不会 panic。
    pub fn show(&mut self, ui: &mut Ui) {
        let full = ui.available_rect_before_wrap();
        let sidebar = sidebar_width(full.width());
        // 侧栏宽度加分隔符宽度，用于估算画布区域中心。
        // Sidebar plus separator width, used to estimate the canvas centre.
        let canvas_rect =
            Rect::from_min_max(pos2(full.min.x + sidebar + 8.0, full.min.y), full.max);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                vec2(sidebar, full.height().max(1.0)),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.show_palette(ui, canvas_rect),
            );
            ui.separator();
            let available = ui.available_size();
            let desired = vec2(available.x.max(64.0), available.y.max(64.0));
            let (response, painter) = ui.allocate_painter(desired, Sense::click_and_drag());
            let viewport = response.rect;
            self.handle_input(ui, &response, viewport);
            self.paint(&painter, viewport);
        });
    }

    /// Draw the device palette sidebar / 绘制设备调色板侧栏
    fn show_palette(&mut self, ui: &mut Ui, canvas_rect: Rect) {
        ui.label("Palette / 调色板");
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let palette = DevicePalette::default();
                for (index, entry) in palette.entries().iter().enumerate() {
                    let text = format!("{}  {}×{}", entry.name, entry.size.x, entry.size.y);
                    let width = ui.available_width().max(1.0);
                    if ui
                        .add_sized([width, 22.0], egui::Button::new(text))
                        .clicked()
                    {
                        let pos = self.spawn_position(canvas_rect, entry.size);
                        let _ = self.add_from_palette(index, pos);
                    }
                }
            });
    }

    /// Top-left corner for a new palette device near the canvas centre
    /// 新调色板设备在画布中心附近放置时的左上角
    fn spawn_position(&self, canvas_rect: Rect, size: Vec2) -> Pos2 {
        // 依次错开，连续点击不会完全重叠。
        // Cascade the drop position so repeated clicks do not fully overlap.
        const CASCADE: [f32; 6] = [0.0, 14.0, 28.0, 42.0, 56.0, 70.0];
        let step = CASCADE[self.nodes.len() % CASCADE.len()];
        let center = self.screen_to_world(canvas_rect, canvas_rect.center());
        pos2(
            center.x - size.x * 0.5 + step,
            center.y - size.y * 0.5 + step,
        )
    }

    fn handle_input(&mut self, ui: &Ui, response: &egui::Response, viewport: Rect) {
        let (pointer, pressed, down, released, middle, delta, scroll, zoom, space) =
            ui.input(|input| {
                (
                    input.pointer.interact_pos(),
                    input.pointer.button_pressed(PointerButton::Primary),
                    input.pointer.button_down(PointerButton::Primary),
                    input.pointer.button_released(PointerButton::Primary),
                    input.pointer.middle_down(),
                    input.pointer.delta(),
                    input.smooth_scroll_delta.y,
                    input.zoom_delta(),
                    input.key_down(Key::Space),
                )
            });
        if ui.input(|input| input.key_pressed(Key::Delete))
            && (response.hovered() || response.has_focus())
        {
            self.delete_selected();
        }
        let Some(screen) = pointer else {
            return;
        };
        // 滚轮缩放；ctrl+滚轮由 egui 转成 zoom_delta，二者只应用其一。
        // Wheel zoom; ctrl+wheel arrives as zoom_delta, so only one applies.
        if (zoom - 1.0).abs() > f32::EPSILON {
            self.zoom_at(viewport, screen, zoom);
        } else if scroll.abs() > 0.0 {
            self.zoom_at(viewport, screen, (scroll * 0.002).exp());
        }
        if middle || (space && down) {
            self.set_pan(self.pan + delta);
            return;
        }
        let world = self.screen_to_world(viewport, screen);
        if pressed {
            self.begin_drag(world);
        } else if down && self.drag.is_some() {
            self.update_drag(world);
        }
        if released {
            let _ = self.finish_drag(world);
        }
    }

    fn paint(&self, painter: &egui::Painter, viewport: Rect) {
        painter.rect_filled(viewport, 0.0, CANVAS_BG);
        self.paint_grid(painter, viewport);
        for wire in &self.wires {
            self.paint_wire(painter, viewport, wire);
        }
        if let Some(DragState::Wire { from }) = self.drag
            && let (Some(start), Some(cursor)) = (self.port_position(from), self.cursor)
        {
            painter.line_segment(
                [
                    self.world_to_screen(viewport, start),
                    self.world_to_screen(viewport, cursor),
                ],
                Stroke::new(2.0, SELECTED_COLOR),
            );
        }
        for (index, node) in self.nodes.iter().enumerate() {
            self.paint_node(painter, viewport, index, node);
        }
        painter.text(
            viewport.left_bottom() + vec2(8.0, -8.0),
            Align2::LEFT_BOTTOM,
            "click palette / 点调色板添加 · drag device / 拖设备 · drag port / 拖端口连线 · Delete · wheel / 滚轮 · middle or space / 中键或空格",
            FontId::monospace(10.0),
            HINT_COLOR,
        );
    }

    fn paint_grid(&self, painter: &egui::Painter, viewport: Rect) {
        let spacing = (GRID_SPACING * self.zoom).max(6.0);
        let stroke = Stroke::new(1.0, GRID_COLOR);
        let mut x = viewport.min.x + self.pan.x.rem_euclid(spacing);
        while x < viewport.max.x {
            painter.line_segment([pos2(x, viewport.min.y), pos2(x, viewport.max.y)], stroke);
            x += spacing;
        }
        let mut y = viewport.min.y + self.pan.y.rem_euclid(spacing);
        while y < viewport.max.y {
            painter.line_segment([pos2(viewport.min.x, y), pos2(viewport.max.x, y)], stroke);
            y += spacing;
        }
    }

    fn paint_wire(&self, painter: &egui::Painter, viewport: Rect, wire: &Wire) {
        let Some(world) = self.wire_points(wire) else {
            return;
        };
        let points = world.map(|point| self.world_to_screen(viewport, point));
        let selected = self.selected
            == Some(Selection::Wire(
                self.wires
                    .iter()
                    .position(|w| w == wire)
                    .unwrap_or(usize::MAX),
            ));
        let (width, color) = if selected {
            (3.5, SELECTED_COLOR)
        } else {
            (2.0, WIRE_COLOR)
        };
        painter.add(egui::Shape::CubicBezier(
            CubicBezierShape::from_points_stroke(
                points,
                false,
                Color32::TRANSPARENT,
                Stroke::new(width, color),
            ),
        ));
    }

    fn paint_node(&self, painter: &egui::Painter, viewport: Rect, index: usize, node: &DeviceNode) {
        let min = self.world_to_screen(viewport, node.pos);
        let max = self.world_to_screen(viewport, node.pos + node.size);
        let rect = Rect::from_min_max(min, max);
        let stroke = if self.selected == Some(Selection::Device(index)) {
            Stroke::new(2.5, SELECTED_COLOR)
        } else {
            Stroke::new(1.5, NODE_STROKE)
        };
        painter.rect(rect, 4.0, NODE_FILL, stroke, StrokeKind::Inside);
        painter.text(
            rect.min + vec2(8.0, 6.0),
            Align2::LEFT_TOP,
            node.name.as_str(),
            FontId::proportional((12.0 * self.zoom).clamp(9.0, 18.0)),
            NODE_TEXT,
        );
        let radius = (PORT_RADIUS * self.zoom).clamp(3.0, 9.0);
        let label = (10.0 * self.zoom).clamp(8.0, 14.0);
        for (port_index, port) in node.ports.iter().enumerate() {
            let Some(world) = node.port_pos(port_index) else {
                continue;
            };
            let position = self.world_to_screen(viewport, world);
            let color = match port.kind {
                PortKind::Input => INPUT_COLOR,
                PortKind::Output => OUTPUT_COLOR,
                PortKind::Bidirectional => BIDI_COLOR,
            };
            painter.circle_filled(position, radius, color);
            if self.zoom >= 0.6 {
                painter.text(
                    position + vec2(radius + 4.0, 0.0),
                    Align2::LEFT_CENTER,
                    port.name.as_str(),
                    FontId::monospace(label),
                    NODE_TEXT,
                );
            }
        }
    }
}

// Canvas model and painter tests live here, next to the rendering code they
// exercise; `canvas.rs` itself stays under the project line cap.
// 画布模型与绘制测试放在这里，紧邻被测的绘制代码；`canvas.rs` 因此不超过行数上限。
#[cfg(test)]
mod tests {
    use crate::canvas::{DeviceNode, Endpoint, MAX_ZOOM, MIN_ZOOM, PAN_LIMIT, Port, WireError};
    use egui::Pos2;

    use super::*;

    fn node(id: &str, x: f32, y: f32) -> DeviceNode {
        DeviceNode::new(id, id, pos2(x, y), vec2(100.0, 60.0))
            .with_port(Port::new("IN", PortKind::Input, vec2(0.0, 20.0)))
            .with_port(Port::new("OUT", PortKind::Output, vec2(0.0, 40.0)))
    }

    fn canvas() -> CanvasState {
        let mut state = CanvasState::new();
        let _ = state.add_device(node("a", 0.0, 0.0));
        let _ = state.add_device(node("b", 200.0, 0.0));
        state
    }

    fn viewport() -> Rect {
        Rect::from_min_size(pos2(10.0, 20.0), vec2(800.0, 600.0))
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn render(state: &mut CanvasState, size: egui::Vec2) {
        let context = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            ..egui::RawInput::default()
        };
        let output = context.run_ui(raw, |ui| state.show(ui));
        let shapes = output.shapes.len();
        output.drop_without_applying_deltas();
        assert!(shapes > 0, "canvas should paint at least one shape");
    }

    #[test]
    fn empty_canvas_defaults() {
        let state = CanvasState::new();
        assert!(state.nodes.is_empty() && state.wires.is_empty());
        assert!(close(state.zoom, 1.0));
        assert_eq!((state.selected, state.drag), (None, None));
    }

    #[test]
    fn device_and_port_hit_tests() {
        let state = canvas();
        assert_eq!(state.hit_test_device(pos2(50.0, 30.0)), Some(0));
        assert_eq!(state.hit_test_device(pos2(250.0, 30.0)), Some(1));
        assert_eq!(state.hit_test_device(pos2(150.0, 300.0)), None);
        assert_eq!(
            state.hit_test_port(pos2(0.0, 20.0)),
            Some(Endpoint::new(0, 0))
        );
        assert_eq!(
            state.hit_test_port(pos2(200.0, 40.0)),
            Some(Endpoint::new(1, 1))
        );
        assert_eq!(state.hit_test_port(pos2(150.0, 30.0)), None);
    }

    #[test]
    fn world_screen_roundtrip_after_zoom_and_pan() {
        let mut state = canvas();
        state.set_zoom(2.5);
        state.set_pan(vec2(-120.0, 45.0));
        let world = pos2(123.0, -45.0);
        let back = state.screen_to_world(viewport(), state.world_to_screen(viewport(), world));
        assert!(close(back.x, world.x) && close(back.y, world.y));
    }

    #[test]
    fn zoom_and_pan_are_clamped() {
        let mut state = canvas();
        state.set_zoom(0.001);
        assert!(close(state.zoom, MIN_ZOOM));
        state.set_zoom(99.0);
        assert!(close(state.zoom, MAX_ZOOM));
        state.set_pan(vec2(1.0e9, -1.0e9));
        assert!(close(state.pan.x, PAN_LIMIT) && close(state.pan.y, -PAN_LIMIT));
    }

    #[test]
    fn zoom_at_keeps_anchor_world_point() {
        let mut state = canvas();
        let anchor = pos2(400.0, 300.0);
        let before = state.screen_to_world(viewport(), anchor);
        state.zoom_at(viewport(), anchor, 1.5);
        let after = state.screen_to_world(viewport(), anchor);
        assert!(close(before.x, after.x) && close(before.y, after.y));
    }

    #[test]
    fn self_wire_and_same_device_are_rejected() {
        let state = canvas();
        assert_eq!(
            state.validate_wire(Endpoint::new(0, 1), Endpoint::new(0, 1)),
            Err(WireError::SamePort)
        );
        assert_eq!(
            state.validate_wire(Endpoint::new(0, 1), Endpoint::new(0, 0)),
            Err(WireError::SameDevice)
        );
        assert_eq!(
            state.validate_wire(Endpoint::new(9, 0), Endpoint::new(1, 0)),
            Err(WireError::UnknownEndpoint)
        );
    }

    #[test]
    fn input_to_input_is_rejected() {
        let mut state = canvas();
        assert_eq!(
            state.validate_wire(Endpoint::new(0, 0), Endpoint::new(1, 0)),
            Err(WireError::Direction)
        );
        assert!(
            state
                .add_wire(Endpoint::new(0, 0), Endpoint::new(1, 0))
                .is_err()
        );
        assert_eq!(state.wires, Vec::new());
    }

    #[test]
    fn duplicate_wire_is_rejected_in_both_orders() {
        let mut state = canvas();
        state
            .add_wire(Endpoint::new(0, 1), Endpoint::new(1, 0))
            .unwrap();
        for (from, to) in [
            (Endpoint::new(0, 1), Endpoint::new(1, 0)),
            (Endpoint::new(1, 0), Endpoint::new(0, 1)),
        ] {
            assert_eq!(state.add_wire(from, to), Err(WireError::Duplicate));
        }
        assert_eq!(state.wires.len(), 1);
    }

    #[test]
    fn output_input_and_bidirectional_wires_connect() {
        let mut state = canvas();
        let index = state
            .add_wire(Endpoint::new(1, 0), Endpoint::new(0, 1))
            .unwrap();
        assert_eq!(state.wires[index].from, Endpoint::new(0, 1));
        assert_eq!(state.wires[index].to, Endpoint::new(1, 0));

        let mut bidi = CanvasState::new();
        let _ =
            bidi.add_device(
                DeviceNode::new("io", "IO", pos2(0.0, 0.0), vec2(100.0, 60.0))
                    .with_port(Port::new("IO", PortKind::Bidirectional, vec2(0.0, 20.0))),
            );
        let _ = bidi.add_device(node("b", 200.0, 0.0));
        assert!(PortKind::Bidirectional.can_drive() && PortKind::Bidirectional.can_accept());
        bidi.add_wire(Endpoint::new(0, 0), Endpoint::new(1, 0))
            .unwrap();
        bidi.add_wire(Endpoint::new(1, 1), Endpoint::new(0, 0))
            .unwrap();
        assert_eq!(bidi.wires.len(), 2);
    }

    #[test]
    fn removing_node_cascades_wires_and_remaps_indices() {
        let mut state = CanvasState::new();
        let _ = state.add_device(node("a", 0.0, 0.0));
        let _ = state.add_device(node("b", 200.0, 0.0));
        let _ = state.add_device(node("c", 400.0, 0.0));
        state
            .add_wire(Endpoint::new(0, 1), Endpoint::new(1, 0))
            .unwrap();
        state
            .add_wire(Endpoint::new(1, 1), Endpoint::new(2, 0))
            .unwrap();
        assert!(state.remove_node(0));
        assert_eq!(state.nodes.len(), 2);
        assert_eq!(state.nodes[0].id, "b");
        assert_eq!(state.wires.len(), 1);
        assert_eq!(state.wires[0].from, Endpoint::new(0, 1));
        assert_eq!(state.wires[0].to, Endpoint::new(1, 0));
        assert!(!state.remove_node(9));
    }

    #[test]
    fn device_drag_moves_node_and_keeps_port_offsets() {
        let mut state = canvas();
        let start = pos2(50.0, 30.0);
        let offset = state.nodes[0].port_pos(1).unwrap() - state.nodes[0].pos;
        assert!(state.begin_drag(start));
        state.update_drag(start + vec2(40.0, 25.0));
        assert!(close(state.nodes[0].pos.x, 40.0) && close(state.nodes[0].pos.y, 25.0));
        let moved = state.nodes[0].port_pos(1).unwrap() - state.nodes[0].pos;
        assert!(close(moved.x, offset.x) && close(moved.y, offset.y));
        assert!(!state.finish_drag(start));
    }

    #[test]
    fn wire_drag_creates_wire_on_port_release() {
        let mut state = canvas();
        let output = state.nodes[0].port_pos(1).unwrap();
        let input = state.nodes[1].port_pos(0).unwrap();
        assert!(state.begin_drag(output));
        state.update_drag(input);
        assert!(state.finish_drag(input));
        assert_eq!(state.wires.len(), 1);
        assert_eq!(state.wires[0].from, Endpoint::new(0, 1));
        assert_eq!(state.wires[0].to, Endpoint::new(1, 0));
        assert_eq!(state.selected, Some(Selection::Wire(0)));
    }

    #[test]
    fn wire_drag_released_on_empty_space_creates_nothing() {
        let mut state = canvas();
        assert!(state.begin_drag(state.nodes[0].port_pos(1).unwrap()));
        assert!(!state.finish_drag(pos2(150.0, 300.0)));
        assert!(state.wires.is_empty() && state.drag.is_none());
    }

    #[test]
    fn delete_selected_device_and_wire() {
        let mut state = canvas();
        state.select(Some(Selection::Device(0)));
        assert!(state.delete_selected());
        assert_eq!(state.nodes.len(), 1);
        assert!(!state.delete_selected());

        let mut state = canvas();
        let index = state
            .add_wire(Endpoint::new(0, 1), Endpoint::new(1, 0))
            .unwrap();
        state.select(Some(Selection::Wire(index)));
        assert!(state.delete_selected());
        assert_eq!(state.wires, Vec::new());
        assert_eq!(state.remove_wire(0), None);
    }

    #[test]
    fn empty_canvas_and_single_node_render() {
        render(&mut CanvasState::new(), vec2(320.0, 200.0));
        let mut state = CanvasState::new();
        let _ = state.add_device(node("a", 0.0, 0.0));
        render(&mut state, vec2(800.0, 600.0));
        render(&mut state, vec2(20.0, 20.0));
    }
}
