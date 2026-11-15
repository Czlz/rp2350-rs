//! Palette and text-persistence tests for the Virtual Lab canvas
//! 虚拟实验室画布的调色板与文本持久化测试
//!
//! Kept in a dedicated module so `canvas.rs`, `canvas_view.rs`, `palette.rs`
//! and `canvas_io.rs` all stay under the project line cap.
//! 独立成模块，使 `canvas.rs`、`canvas_view.rs`、`palette.rs` 与 `canvas_io.rs`
//! 都符合项目单文件行数上限。

use egui::{Pos2, Rect, Vec2, pos2, vec2};

use crate::app::EmuApp;
use crate::canvas::{CanvasState, DeviceNode, Endpoint, Port, PortKind};
use crate::canvas_io::CanvasParseError;
use crate::palette::DevicePalette;

/// Parse and expect an error / 解析并期待错误
fn parse_error(text: &str) -> CanvasParseError {
    CanvasState::from_text(text).expect_err("text should be rejected")
}

/// Render the canvas headlessly and return the shape count
/// 无窗口渲染画布并返回图元数量
fn render(canvas: &mut CanvasState, size: Vec2) -> usize {
    let context = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
        ..egui::RawInput::default()
    };
    let output = context.run_ui(raw, |ui| canvas.show(ui));
    let shapes = output.shapes.len();
    output.drop_without_applying_deltas();
    shapes
}

#[test]
fn palette_has_at_least_six_valid_entries() {
    let palette = DevicePalette::default();
    assert!(palette.len() >= 6);
    assert_eq!(palette.len(), palette.entries().len());
    assert!(!palette.is_empty());
    assert_eq!(DevicePalette::new().len(), palette.len());
    for entry in palette.entries() {
        assert_ne!(entry.name, "");
        assert!(entry.size.x > 0.0 && entry.size.y > 0.0);
        assert_ne!(entry.ports.len(), 0);
    }
}

#[test]
fn palette_instantiate_copies_size_and_ports() {
    let palette = DevicePalette::default();
    for (index, entry) in palette.entries().iter().enumerate() {
        let node = palette
            .instantiate(index, pos2(10.0, 20.0))
            .expect("known index");
        assert_eq!(node.pos, pos2(10.0, 20.0));
        assert_eq!(node.size, entry.size);
        assert_eq!(node.name, entry.name);
        assert_eq!(node.ports.len(), entry.ports.len());
        for (port, expected) in node.ports.iter().zip(entry.ports) {
            assert_eq!(port.name, expected.name);
            assert_eq!(port.kind, expected.kind);
            assert_eq!(port.offset, expected.offset);
        }
    }
}

#[test]
fn palette_instantiate_ids_are_unique_and_prefixed() {
    let palette = DevicePalette::default();
    let nodes: Vec<DeviceNode> = (0..8)
        .filter_map(|_| palette.instantiate(1, pos2(0.0, 0.0)))
        .collect();
    assert_eq!(nodes.len(), 8);
    for (index, node) in nodes.iter().enumerate() {
        assert!(node.id.starts_with("palette-1-"), "{}", node.id);
        assert!(!nodes[..index].iter().any(|other| other.id == node.id));
    }
}

#[test]
fn palette_instantiate_out_of_range_returns_none() {
    let palette = DevicePalette::default();
    assert!(palette.get(palette.len()).is_none());
    assert!(palette.get(0).is_some());
    assert!(palette.instantiate(palette.len(), pos2(0.0, 0.0)).is_none());
}

#[test]
fn add_from_palette_adds_node_and_rejects_bad_index() {
    let mut canvas = CanvasState::new();
    let index = canvas.add_from_palette(0, pos2(5.0, 6.0)).expect("entry 0");
    assert_eq!(index, 0);
    assert_eq!(canvas.nodes.len(), 1);
    assert_eq!(canvas.nodes[0].pos, pos2(5.0, 6.0));
    assert!(canvas.add_from_palette(999, pos2(0.0, 0.0)).is_none());
    assert_eq!(canvas.nodes.len(), 1);
}

#[test]
fn sample_canvas_text_roundtrip() {
    let original = CanvasState::sample();
    assert_ne!(original.wires.len(), 0);
    let text = original.to_text();
    assert!(text.starts_with("rp2350-canvas;1"));
    let restored = CanvasState::from_text(&text).expect("sample text parses");

    assert_eq!(restored.nodes.len(), original.nodes.len());
    assert_eq!(restored.wires, original.wires);
    assert_eq!(restored.zoom.to_bits(), original.zoom.to_bits());
    assert_eq!(restored.pan, original.pan);
    for (restored, original) in restored.nodes.iter().zip(&original.nodes) {
        assert_eq!(restored.id, original.id);
        assert_eq!(restored.name, original.name);
        assert_eq!(restored.pos, original.pos);
        assert_eq!(restored.size, original.size);
        assert_eq!(restored.ports, original.ports);
    }
    assert_eq!(restored, original);
}

#[test]
fn escaped_names_roundtrip() {
    let mut canvas = CanvasState::new();
    let source = canvas.add_device(
        DeviceNode::new("a;b\\c", "LED; \\ 名称", pos2(1.5, 2.5), vec2(80.0, 40.0))
            .with_port(Port::new("P;1\\x", PortKind::Output, vec2(0.0, 10.0))),
    );
    let sink =
        canvas.add_device(
            DeviceNode::new("sink", "Sink", pos2(200.0, 2.5), vec2(80.0, 40.0))
                .with_port(Port::new("IN", PortKind::Input, vec2(0.0, 10.0))),
        );
    canvas
        .add_wire(Endpoint::new(source, 0), Endpoint::new(sink, 0))
        .expect("legal wire");

    let text = canvas.to_text();
    assert!(text.contains("\\;"), "semicolons must be escaped");
    assert!(text.contains("\\\\"), "backslashes must be escaped");
    let restored = CanvasState::from_text(&text).expect("escaped text parses");
    assert_eq!(restored.nodes[0].id, "a;b\\c");
    assert_eq!(restored.nodes[0].name, "LED; \\ 名称");
    assert_eq!(restored.nodes[0].ports[0].name, "P;1\\x");
    assert_eq!(restored, canvas);
}

#[test]
fn empty_canvas_text_roundtrip() {
    let text = CanvasState::new().to_text();
    let restored = CanvasState::from_text(&text).expect("empty text parses");
    assert_eq!(restored.nodes.len(), 0);
    assert_eq!(restored.wires.len(), 0);
    assert_eq!(restored, CanvasState::new());
}

#[test]
fn zoom_and_pan_roundtrip() {
    let mut canvas = CanvasState::sample();
    canvas.set_zoom(1.75);
    canvas.set_pan(vec2(-123.5, 67.25));
    let restored = CanvasState::from_text(&canvas.to_text()).expect("viewport parses");
    assert_eq!(restored.zoom.to_bits(), 1.75f32.to_bits());
    assert_eq!(restored.pan, vec2(-123.5, 67.25));
}

#[test]
fn palette_devices_roundtrip_with_wires() {
    let mut canvas = CanvasState::sample();
    let led = canvas
        .add_from_palette(0, pos2(520.0, 40.0))
        .expect("LED entry");
    let power = canvas
        .add_from_palette(4, pos2(520.0, 200.0))
        .expect("power entry");
    canvas
        .add_wire(Endpoint::new(power, 0), Endpoint::new(led, 0))
        .expect("power drives the LED");

    let restored = CanvasState::from_text(&canvas.to_text()).expect("palette text parses");
    assert_eq!(restored.nodes.len(), canvas.nodes.len());
    assert_eq!(restored.wires.len(), canvas.wires.len());
    assert_eq!(restored.nodes[3].id, canvas.nodes[3].id);
    assert_eq!(restored, canvas);
}

#[test]
fn text_rejects_missing_device_field() {
    let error = parse_error("rp2350-canvas;1\ndevice;mcu;RP2350;0;0\n");
    assert_eq!(error.line(), 2);
    assert!(error.message().contains("fields"), "{}", error.message());
}

#[test]
fn text_rejects_out_of_range_wire_endpoint() {
    let error =
        parse_error("rp2350-canvas;1\ndevice;a;A;0;0;10;10\nport;p;out;0;0\nwire;0;0;7;0\n");
    assert_eq!(error.line(), 4);
    assert!(error.message().contains("endpoint"), "{}", error.message());
}

#[test]
fn text_rejects_unknown_port_kind() {
    let error = parse_error("rp2350-canvas;1\ndevice;a;A;0;0;10;10\nport;p;sideways;0;0\n");
    assert_eq!(error.line(), 3);
    assert!(error.message().contains("kind"), "{}", error.message());
}

#[test]
fn text_rejects_bad_numbers_and_indices() {
    let error = parse_error("rp2350-canvas;1\nzoom;abc\n");
    assert_eq!(error.line(), 2);
    assert!(error.message().contains("number"), "{}", error.message());

    let error = parse_error("rp2350-canvas;1\ndevice;a;A;x;0;10;10\n");
    assert_eq!(error.line(), 2);
    assert!(error.message().contains("number"), "{}", error.message());

    let error = parse_error("rp2350-canvas;1\nwire;-1;0;0;0\n");
    assert_eq!(error.line(), 2);
    assert!(error.message().contains("index"), "{}", error.message());
}

#[test]
fn text_rejects_bad_headers_and_records() {
    let error = parse_error("device;a;A;0;0;10;10\n");
    assert_eq!(error.line(), 1);
    assert!(error.message().contains("header"), "{}", error.message());

    let error = parse_error("rp2350-canvas;1\nrp2350-canvas;1\n");
    assert_eq!(error.line(), 2);
    assert!(error.message().contains("header"), "{}", error.message());

    let error = parse_error("rp2350-canvas;2\n");
    assert_eq!(error.line(), 1);
    assert!(error.message().contains("version"), "{}", error.message());

    let error = parse_error("rp2350-canvas;1\nbogus;1\n");
    assert_eq!(error.line(), 2);
    assert!(error.message().contains("record"), "{}", error.message());

    let error = parse_error("rp2350-canvas;1\nport;p;in;0;0\n");
    assert_eq!(error.line(), 2);
    assert!(
        error.message().contains("before device"),
        "{}",
        error.message()
    );
}

#[test]
fn text_rejects_unknown_escape() {
    let error = parse_error("rp2350-canvas;1\ndevice;a;A\\q;0;0;10;10\n");
    assert_eq!(error.line(), 2);
    assert!(error.message().contains("escape"), "{}", error.message());
}

#[test]
fn text_rejects_direction_mismatch_wire() {
    let text = "rp2350-canvas;1\ndevice;a;A;0;0;10;10\nport;p;in;0;0\n\
                device;b;B;40;0;10;10\nport;q;in;0;0\nwire;0;0;1;0\n";
    let error = parse_error(text);
    assert_eq!(error.line(), 6);
    assert!(error.message().contains("direction"), "{}", error.message());
}

#[test]
fn palette_sidebar_renders_at_all_sizes() {
    let mut canvas = CanvasState::sample();
    let mut added = 0;
    for size in [vec2(80.0, 60.0), vec2(320.0, 240.0), vec2(800.0, 600.0)] {
        let shapes = render(&mut canvas, size);
        assert!(shapes > 0, "palette + canvas should paint shapes");
        added += 1;
    }
    assert_eq!(added, 3);
    assert!(canvas.nodes.len() >= 3);
}

#[test]
fn app_save_and_load_restore_canvas_counts() {
    let mut app = EmuApp::sample();
    let led = app
        .canvas_mut()
        .add_from_palette(0, pos2(520.0, 40.0))
        .expect("LED entry");
    let power = app
        .canvas_mut()
        .add_from_palette(4, pos2(520.0, 200.0))
        .expect("power entry");
    app.canvas_mut()
        .add_wire(Endpoint::new(power, 0), Endpoint::new(led, 0))
        .expect("power drives the LED");
    let nodes = app.canvas().nodes.len();
    let wires = app.canvas().wires.len();

    app.save_canvas();
    assert!(app.canvas_status.contains("saved"));
    assert_ne!(app.canvas_snapshot, "");

    assert!(app.canvas_mut().remove_node(0));
    assert_ne!(app.canvas().nodes.len(), nodes);

    app.load_canvas().expect("snapshot should parse");
    assert_eq!(app.canvas().nodes.len(), nodes);
    assert_eq!(app.canvas().wires.len(), wires);
    assert!(app.canvas_status.contains("loaded"));
}

#[test]
fn app_load_failure_keeps_canvas_and_reports_error() {
    let mut app = EmuApp::sample();
    let nodes = app.canvas().nodes.len();
    app.canvas_snapshot = String::from("rp2350-canvas;1\ndevice;only-id\n");

    let error = app.load_canvas().expect_err("malformed snapshot");
    assert_eq!(error.line(), 2);
    assert_eq!(app.canvas().nodes.len(), nodes);
    assert!(app.canvas_status.contains("load failed"));
}
