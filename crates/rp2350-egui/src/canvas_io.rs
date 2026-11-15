//! Canvas text serialisation / 画布文本序列化
//!
//! The format is line-oriented, semicolon-separated and dependency-free:
//!
//! ```text
//! rp2350-canvas;1
//! zoom;1
//! pan;0;0
//! device;mcu;RP2350;60;70;170;130
//! port;GPIO25;out;0;34
//! device;led;LED;360;60;120;70
//! port;A;in;0;34
//! wire;0;0;1;0
//! ```
//!
//! Fields are separated by `;`. Inside text fields (`id`, device `name`, port
//! `name`) the backslash is the escape character: `\\` is a literal backslash,
//! `\;` a semicolon, `\n` a newline and `\r` a carriage return. Names may
//! therefore contain spaces and semicolons. Blank lines and lines starting with
//! `#` are ignored. `port` records belong to the preceding `device`, and `wire`
//! records reference devices and ports by zero-based index, so they are
//! validated after the whole document has been read.
//! 字段以 `;` 分隔；文本字段（id、设备名称、端口名称）内的反斜杠是转义符，
//! 因此名称可以包含空格与 `;`。空行与 `#` 开头的行被忽略；`port` 记录隶属于
//! 它上面的 `device`，`wire` 记录用从 0 开始的索引引用设备与端口，因此在整篇
//! 文档读完后统一校验。
//!
//! Selection, drag and cursor state are transient and never serialised.
//! 选中项、拖拽与光标状态是临时的，不会被序列化。

use std::fmt::{self, Write as _};

use egui::{pos2, vec2};

use crate::canvas::{CanvasState, DeviceNode, Endpoint, Port, PortKind, WireError};

/// Magic header of the text format / 文本格式的魔数头部
pub const CANVAS_FORMAT_HEADER: &str = "rp2350-canvas;1";

/// Error returned when a canvas text cannot be parsed / 画布文本无法解析时的错误
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasParseError {
    /// 1-based line number (1 for whole-document errors) / 行号（整篇错误为 1）
    line: usize,
    /// Human-readable reason / 可读的原因
    message: String,
}

impl CanvasParseError {
    /// Create an error for a 1-based line / 为某一行（从 1 开始）创建错误
    fn new(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }

    /// Line number the error refers to (1-based) / 错误所在行号（从 1 开始）
    #[must_use]
    pub fn line(&self) -> usize {
        self.line
    }

    /// Human-readable reason / 可读的错误原因
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for CanvasParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "canvas line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for CanvasParseError {}

impl CanvasState {
    /// Serialise the canvas into the text format / 将画布序列化为文本格式
    ///
    /// Devices (with their ports) come first, then wires; the viewport zoom and
    /// pan are written as the first records. Selection and drag state are
    /// transient and skipped.
    /// 先写设备（含端口）再写连线；缩放与平移写在最前面的记录里。选中与拖拽
    /// 状态是临时的，不会写入。
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(CANVAS_FORMAT_HEADER);
        out.push('\n');
        let _ = writeln!(out, "zoom;{}", self.zoom);
        let _ = writeln!(out, "pan;{};{}", self.pan.x, self.pan.y);
        for node in &self.nodes {
            out.push_str("device;");
            escape_into(&mut out, &node.id);
            out.push(';');
            escape_into(&mut out, &node.name);
            // 位置与尺寸都是有限值（拖拽与添加时已钳制）。
            // Positions and sizes are finite: drags and inserts clamp them.
            let _ = writeln!(
                out,
                ";{};{};{};{}",
                node.pos.x, node.pos.y, node.size.x, node.size.y
            );
            for port in &node.ports {
                out.push_str("port;");
                escape_into(&mut out, &port.name);
                let _ = writeln!(
                    out,
                    ";{};{};{}",
                    port_kind_name(port.kind),
                    port.offset.x,
                    port.offset.y
                );
            }
        }
        for wire in &self.wires {
            let _ = writeln!(
                out,
                "wire;{};{};{};{}",
                wire.from.device, wire.from.port, wire.to.device, wire.to.port
            );
        }
        out
    }

    /// Parse a canvas from the text format / 从文本格式解析画布
    ///
    /// # Errors
    /// Returns [`CanvasParseError`] with a 1-based line number and a reason when
    /// the header is missing or duplicated, a record has the wrong field count,
    /// a number or index is malformed, a port kind or escape is unknown, or a
    /// wire is invalid in the restored model.
    /// 头部缺失或重复、记录字段数不符、数字或索引非法、端口方向或转义未知、
    /// 以及恢复后不合法的连线都会返回带行号的 [`CanvasParseError`]。
    pub fn from_text(text: &str) -> Result<Self, CanvasParseError> {
        let mut canvas = Self::default();
        let mut header = false;
        let mut wires: Vec<(usize, Endpoint, Endpoint)> = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let fields = split_fields(trimmed);
            let Some(keyword) = fields.first().copied() else {
                continue;
            };
            if !header && keyword != "rp2350-canvas" {
                return Err(CanvasParseError::new(
                    line,
                    "missing header, expected rp2350-canvas;1 / 缺少头部",
                ));
            }
            match keyword {
                "rp2350-canvas" => {
                    if header {
                        return Err(CanvasParseError::new(line, "duplicate header / 重复头部"));
                    }
                    if fields.get(1).copied() != Some("1") {
                        return Err(CanvasParseError::new(
                            line,
                            "unsupported format version / 不支持的格式版本",
                        ));
                    }
                    header = true;
                }
                "zoom" => {
                    let value = field(&fields, 1, line, "zoom")?;
                    canvas.set_zoom(parse_f32(value, line, "zoom")?);
                }
                "pan" => {
                    let x = field_f32(&fields, 1, line, "pan x")?;
                    let y = field_f32(&fields, 2, line, "pan y")?;
                    canvas.set_pan(vec2(x, y));
                }
                "device" => {
                    let node = parse_device(&fields, line)?;
                    let _ = canvas.add_device(node);
                }
                "port" => {
                    let port = parse_port(&fields, line)?;
                    let node = canvas.nodes.last_mut().ok_or_else(|| {
                        CanvasParseError::new(line, "port before device / 端口出现在设备之前")
                    })?;
                    node.ports.push(port);
                }
                "wire" => {
                    let from = Endpoint::new(
                        field_usize(&fields, 1, line, "wire from device")?,
                        field_usize(&fields, 2, line, "wire from port")?,
                    );
                    let to = Endpoint::new(
                        field_usize(&fields, 3, line, "wire to device")?,
                        field_usize(&fields, 4, line, "wire to port")?,
                    );
                    wires.push((line, from, to));
                }
                other => {
                    return Err(CanvasParseError::new(
                        line,
                        format!("unknown record: {other}"),
                    ));
                }
            }
        }
        if !header {
            return Err(CanvasParseError::new(
                1,
                "missing header, expected rp2350-canvas;1 / 缺少头部",
            ));
        }
        // 设备已全部读入，此时才能校验连线索引与方向。
        // All devices are known now, so wire indices and directions can be checked.
        for (line, from, to) in wires {
            canvas.add_wire(from, to).map_err(|error| {
                CanvasParseError::new(line, format!("invalid wire: {}", wire_error_text(error)))
            })?;
        }
        Ok(canvas)
    }
}

/// Canonical record token of a port kind / 端口方向在文本中的规范写法
const fn port_kind_name(kind: PortKind) -> &'static str {
    match kind {
        PortKind::Input => "in",
        PortKind::Output => "out",
        PortKind::Bidirectional => "bidi",
    }
}

/// Parse a port kind token / 解析端口方向标记
fn parse_port_kind(value: &str, line: usize) -> Result<PortKind, CanvasParseError> {
    match value.trim() {
        "in" | "input" => Ok(PortKind::Input),
        "out" | "output" => Ok(PortKind::Output),
        "bidi" | "bidirectional" => Ok(PortKind::Bidirectional),
        other => Err(CanvasParseError::new(
            line,
            format!("unknown port kind: {other}"),
        )),
    }
}

/// Human-readable text for a rejected wire / 连线被拒时的可读文本
const fn wire_error_text(error: WireError) -> &'static str {
    match error {
        WireError::UnknownEndpoint => "unknown endpoint / 端点越界",
        WireError::SameDevice => "both endpoints on one device / 两端同设备",
        WireError::SamePort => "identical endpoints / 两端相同",
        WireError::Direction => "direction mismatch / 方向不匹配",
        WireError::Duplicate => "duplicate wire / 重复连线",
    }
}

/// Build a device record / 构建设备记录
fn parse_device(fields: &[&str], line: usize) -> Result<DeviceNode, CanvasParseError> {
    if fields.len() != 7 {
        return Err(CanvasParseError::new(
            line,
            format!(
                "device needs 7 fields (device;id;name;x;y;w;h), got {} / 字段数不符",
                fields.len()
            ),
        ));
    }
    let id = unescape_field(fields[1], line)?;
    let name = unescape_field(fields[2], line)?;
    let x = parse_f32(fields[3], line, "device x")?;
    let y = parse_f32(fields[4], line, "device y")?;
    let width = parse_f32(fields[5], line, "device width")?;
    let height = parse_f32(fields[6], line, "device height")?;
    Ok(DeviceNode::new(id, name, pos2(x, y), vec2(width, height)))
}

/// Build a port record / 构建端口记录
fn parse_port(fields: &[&str], line: usize) -> Result<Port, CanvasParseError> {
    if fields.len() != 5 {
        return Err(CanvasParseError::new(
            line,
            format!(
                "port needs 5 fields (port;name;kind;ox;oy), got {} / 字段数不符",
                fields.len()
            ),
        ));
    }
    let name = unescape_field(fields[1], line)?;
    let kind = parse_port_kind(fields[2], line)?;
    let offset = vec2(
        parse_f32(fields[3], line, "port x")?,
        parse_f32(fields[4], line, "port y")?,
    );
    Ok(Port::new(name, kind, offset))
}

/// Parse a finite `f32` field / 解析有限 `f32` 字段
fn parse_f32(value: &str, line: usize, name: &str) -> Result<f32, CanvasParseError> {
    value
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|number| number.is_finite())
        .ok_or_else(|| CanvasParseError::new(line, format!("bad number for {name}: {value}")))
}

/// Parse a `usize` field / 解析 `usize` 字段
fn field_usize(
    fields: &[&str],
    index: usize,
    line: usize,
    name: &str,
) -> Result<usize, CanvasParseError> {
    let value = field(fields, index, line, name)?;
    value
        .trim()
        .parse::<usize>()
        .map_err(|_| CanvasParseError::new(line, format!("bad index for {name}: {value}")))
}

/// Parse a required `f32` field by position / 按位置解析必填 `f32` 字段
fn field_f32(
    fields: &[&str],
    index: usize,
    line: usize,
    name: &str,
) -> Result<f32, CanvasParseError> {
    parse_f32(field(fields, index, line, name)?, line, name)
}

/// Fetch a required field by position / 按位置取必填字段
fn field<'a>(
    fields: &[&'a str],
    index: usize,
    line: usize,
    name: &str,
) -> Result<&'a str, CanvasParseError> {
    fields.get(index).copied().ok_or_else(|| {
        CanvasParseError::new(line, format!("missing field {index} ({name}) / 缺少字段"))
    })
}

/// Split a record on unescaped semicolons / 按未转义的分号拆分记录
fn split_fields(line: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut start = 0;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            ';' => {
                fields.push(&line[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    fields.push(&line[start..]);
    fields
}

/// Escape a text field into `out` / 把文本字段转义写入 `out`
fn escape_into(out: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
}

/// Unescape one text field / 反转义一个文本字段
fn unescape_field(value: &str, line: usize) -> Result<String, CanvasParseError> {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(value.len());
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        index += 1;
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(escaped) = chars.get(index) else {
            return Err(CanvasParseError::new(
                line,
                "trailing backslash / 结尾反斜杠",
            ));
        };
        index += 1;
        match escaped {
            '\\' => out.push('\\'),
            ';' => out.push(';'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            other => {
                return Err(CanvasParseError::new(
                    line,
                    format!("unknown escape: \\{other}"),
                ));
            }
        }
    }
    Ok(out)
}
