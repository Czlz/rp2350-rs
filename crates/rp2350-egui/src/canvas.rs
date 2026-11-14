//! Virtual Lab canvas — device/wire model and geometry
//! 虚拟实验室画布 — 设备/连线模型与几何
//!
//! The data model and geometry layer is pure logic (no painter needed) and is
//! unit tested; the egui painter and input handling live in `canvas_view.rs`.
//! 数据模型与几何层是纯逻辑（不需要画笔），由单元测试覆盖；egui 绘制与输入
//! 处理位于 `canvas_view.rs`。
//!
//! Interactions: left-drag a device to move it, drag from one port to another
//! to wire them, `Delete` removes the selection, the wheel zooms and the middle
//! button (or space + left) pans.
//! 交互：左键拖动设备、从端口拖到另一端口建立连线、`Delete` 删除选中项、
//! 滚轮缩放、中键（或空格 + 左键）平移。

use egui::{Pos2, Rect, Vec2, pos2, vec2};

/// Smallest allowed zoom factor / 允许的最小缩放系数
pub(crate) const MIN_ZOOM: f32 = 0.25;
/// Largest allowed zoom factor / 允许的最大缩放系数
pub(crate) const MAX_ZOOM: f32 = 4.0;
/// Pan limit in screen pixels / 屏幕平移上限（像素）
pub(crate) const PAN_LIMIT: f32 = 4096.0;
/// Device position limit in world units / 设备位置的世界坐标上限
const WORLD_LIMIT: f32 = 8192.0;
/// Screen-space port hit radius / 端口命中的屏幕半径
const PORT_HIT_RADIUS: f32 = 8.0;
/// Screen-space wire hit distance / 连线命中的屏幕距离
const WIRE_HIT_RADIUS: f32 = 6.0;

/// Signal direction of a canvas port / 画布端口的信号方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortKind {
    /// Signal enters the device / 信号流入设备
    Input,
    /// Signal leaves the device / 信号流出设备
    Output,
    /// Signal may flow both ways / 信号可双向流动
    Bidirectional,
}

impl PortKind {
    /// Whether this port may drive a wire / 该端口能否作为驱动端
    #[must_use]
    pub const fn can_drive(self) -> bool {
        matches!(self, Self::Output | Self::Bidirectional)
    }

    /// Whether this port may receive a wire / 该端口能否作为接收端
    #[must_use]
    pub const fn can_accept(self) -> bool {
        matches!(self, Self::Input | Self::Bidirectional)
    }
}

/// A connection point on a device node / 设备节点上的连接点
#[derive(Debug, Clone, PartialEq)]
pub struct Port {
    /// Display name / 显示名称
    pub name: String,
    /// Signal direction / 信号方向
    pub kind: PortKind,
    /// Offset from the node's top-left corner / 相对节点左上角的偏移
    pub offset: Vec2,
}

impl Port {
    /// Create a port / 创建端口
    #[must_use]
    pub fn new(name: impl Into<String>, kind: PortKind, offset: Vec2) -> Self {
        Self {
            name: name.into(),
            kind,
            offset,
        }
    }
}

/// A device placed on the canvas / 放置在画布上的设备
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceNode {
    /// Stable identifier (matches `WiringTable` device IDs)
    /// 稳定标识（与 `WiringTable` 的设备 ID 一致）
    pub id: String,
    /// Display name / 显示名称
    pub name: String,
    /// Top-left corner in world coordinates / 世界坐标中的左上角
    pub pos: Pos2,
    /// Box size in world units / 世界单位下的框尺寸
    pub size: Vec2,
    /// Ports attached to this device / 挂在该设备上的端口
    pub ports: Vec<Port>,
}

impl DeviceNode {
    /// Create a node without ports / 创建不含端口的节点
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>, pos: Pos2, size: Vec2) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            pos,
            size,
            ports: Vec::new(),
        }
    }

    /// Add a port and return the node (builder) / 添加端口并返回自身（构建器）
    #[must_use]
    pub fn with_port(mut self, port: Port) -> Self {
        self.ports.push(port);
        self
    }

    /// World-space bounding rectangle / 世界空间包围矩形
    #[must_use]
    pub fn rect(&self) -> Rect {
        Rect::from_min_size(self.pos, self.size)
    }

    /// World position of port `index` / 端口 `index` 的世界坐标
    #[must_use]
    pub fn port_pos(&self, index: usize) -> Option<Pos2> {
        self.ports.get(index).map(|port| self.pos + port.offset)
    }
}

/// Reference to one port of one device / 指向某个设备某个端口的引用
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Endpoint {
    /// Device index in [`CanvasState::nodes`] / [`CanvasState::nodes`] 中的设备索引
    pub device: usize,
    /// Port index in [`DeviceNode::ports`] / [`DeviceNode::ports`] 中的端口索引
    pub port: usize,
}

impl Endpoint {
    /// Create an endpoint / 创建端点引用
    #[must_use]
    pub const fn new(device: usize, port: usize) -> Self {
        Self { device, port }
    }
}

/// A wire between two device ports / 两个设备端口之间的连线
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Wire {
    /// Driving endpoint (output or bidirectional) / 驱动端（输出或双向）
    pub from: Endpoint,
    /// Receiving endpoint (input or bidirectional) / 接收端（输入或双向）
    pub to: Endpoint,
}

/// Why a wire was rejected / 连线被拒绝的原因
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    /// The endpoint device or port does not exist / 端点设备或端口不存在
    UnknownEndpoint,
    /// Both endpoints belong to the same device / 两端属于同一设备
    SameDevice,
    /// The two endpoints are identical / 两个端点完全相同
    SamePort,
    /// Direction mismatch: no driver or no receiver / 方向不匹配：无驱动端或无接收端
    Direction,
    /// An equivalent wire already exists / 等价连线已存在
    Duplicate,
}

/// Current canvas selection / 当前画布选中项
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// A device node / 设备节点
    Device(usize),
    /// A wire / 连线
    Wire(usize),
}

/// Active drag operation / 正在进行的拖拽操作
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DragState {
    /// Moving a device by the grab offset / 按抓取偏移移动设备
    Device {
        /// Device index being moved / 正在移动的设备索引
        device: usize,
        /// Pointer offset from the node origin / 指针相对节点原点的偏移
        grab: Vec2,
    },
    /// Drawing a wire from an endpoint / 从端点拉出连线
    Wire {
        /// Fixed start endpoint / 固定的起始端点
        from: Endpoint,
    },
}

/// Canvas model: devices, wires, viewport and interaction state
/// 画布模型：设备、连线、视口与交互状态
#[derive(Debug, Clone, PartialEq)]
pub struct CanvasState {
    /// Placed devices / 已放置的设备
    pub nodes: Vec<DeviceNode>,
    /// Established wires / 已建立的连线
    pub wires: Vec<Wire>,
    /// Screen-space pan offset / 屏幕空间平移量
    pub pan: Vec2,
    /// Zoom factor, clamped to `[MIN_ZOOM, MAX_ZOOM]`
    /// 缩放系数，钳制在 `[MIN_ZOOM, MAX_ZOOM]`
    pub zoom: f32,
    /// Current selection, if any / 当前选中项（如果有）
    pub selected: Option<Selection>,
    /// Active drag, if any / 当前拖拽（如果有）
    pub drag: Option<DragState>,
    /// Last pointer position in world coordinates / 最后一次指针位置（世界坐标）
    pub(crate) cursor: Option<Pos2>,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            wires: Vec::new(),
            pan: Vec2::ZERO,
            zoom: 1.0,
            selected: None,
            drag: None,
            cursor: None,
        }
    }
}

impl CanvasState {
    /// Create an empty canvas / 创建空画布
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a demo canvas with three devices and two wires
    /// 创建包含三个设备和两条连线的示例画布
    #[must_use]
    pub fn sample() -> Self {
        let mut state = Self::default();
        let mcu = state.add_device(
            DeviceNode::new("mcu", "RP2350", pos2(60.0, 70.0), vec2(170.0, 130.0))
                .with_port(Port::new("GPIO25", PortKind::Output, vec2(0.0, 34.0)))
                .with_port(Port::new("GPIO26", PortKind::Input, vec2(0.0, 62.0)))
                .with_port(Port::new(
                    "GPIO27",
                    PortKind::Bidirectional,
                    vec2(0.0, 90.0),
                )),
        );
        let led = state.add_device(
            DeviceNode::new("led", "LED", pos2(360.0, 60.0), vec2(120.0, 70.0))
                .with_port(Port::new("A", PortKind::Input, vec2(0.0, 34.0))),
        );
        let button = state.add_device(
            DeviceNode::new("btn", "Button", pos2(360.0, 220.0), vec2(120.0, 70.0))
                .with_port(Port::new("OUT", PortKind::Output, vec2(0.0, 34.0))),
        );
        let _ = state.add_wire(Endpoint::new(mcu, 0), Endpoint::new(led, 0));
        let _ = state.add_wire(Endpoint::new(button, 0), Endpoint::new(mcu, 1));
        state
    }

    /// Add a device node and return its index / 添加设备节点并返回索引
    #[must_use]
    pub fn add_device(&mut self, node: DeviceNode) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    /// World position of an endpoint's port / 端点端口的世界坐标
    #[must_use]
    pub fn port_position(&self, endpoint: Endpoint) -> Option<Pos2> {
        self.nodes.get(endpoint.device)?.port_pos(endpoint.port)
    }

    /// Convert world coordinates to screen coordinates / 世界坐标转屏幕坐标
    #[must_use]
    pub fn world_to_screen(&self, viewport: Rect, world: Pos2) -> Pos2 {
        viewport.min + (world.to_vec2() * self.zoom + self.pan)
    }

    /// Convert screen coordinates to world coordinates / 屏幕坐标转世界坐标
    #[must_use]
    pub fn screen_to_world(&self, viewport: Rect, screen: Pos2) -> Pos2 {
        ((screen - viewport.min - self.pan) / self.zoom).to_pos2()
    }

    /// Clamp and set the zoom factor / 钳制并设置缩放系数
    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    }

    /// Clamp and set the pan offset in screen pixels / 钳制并设置屏幕像素平移量
    pub fn set_pan(&mut self, pan: Vec2) {
        self.pan = vec2(
            pan.x.clamp(-PAN_LIMIT, PAN_LIMIT),
            pan.y.clamp(-PAN_LIMIT, PAN_LIMIT),
        );
    }

    /// Zoom around a screen anchor, keeping the world point under it fixed
    /// 围绕屏幕锚点缩放，保持锚点下的世界坐标不动
    pub fn zoom_at(&mut self, viewport: Rect, anchor: Pos2, factor: f32) {
        let world = self.screen_to_world(viewport, anchor);
        self.set_zoom(self.zoom * factor);
        self.set_pan(anchor - viewport.min - world.to_vec2() * self.zoom);
    }

    /// Topmost device whose box contains `world`, if any
    /// 包含 `world` 的最上层设备（如果有）
    #[must_use]
    pub fn hit_test_device(&self, world: Pos2) -> Option<usize> {
        self.nodes
            .iter()
            .rposition(|node| node.rect().contains(world))
    }

    /// Topmost port within the hit radius of `world` / 命中半径内最上层的端口
    #[must_use]
    pub fn hit_test_port(&self, world: Pos2) -> Option<Endpoint> {
        let radius = (PORT_HIT_RADIUS / self.zoom).max(1.0);
        let mut hit = None;
        for (device, node) in self.nodes.iter().enumerate() {
            for port in 0..node.ports.len() {
                if let Some(position) = node.port_pos(port)
                    && position.distance_sq(world) <= radius * radius
                {
                    hit = Some(Endpoint::new(device, port));
                }
            }
        }
        hit
    }

    /// Topmost wire within the hit radius of `world` / 命中半径内最上层的连线
    #[must_use]
    pub fn hit_test_wire(&self, world: Pos2) -> Option<usize> {
        let tolerance = (WIRE_HIT_RADIUS / self.zoom).max(1.0);
        self.wires.iter().rposition(|wire| {
            self.wire_points(wire)
                .is_some_and(|points| distance_to_wire(world, &points) <= tolerance)
        })
    }

    /// Bezier control points of a wire in world space / 连线在世界空间的贝塞尔控制点
    #[must_use]
    pub fn wire_points(&self, wire: &Wire) -> Option<[Pos2; 4]> {
        let from = self.port_position(wire.from)?;
        let to = self.port_position(wire.to)?;
        let reach = ((to.x - from.x).abs() * 0.5).max(40.0);
        Some([
            from,
            pos2(from.x + reach, from.y),
            pos2(to.x - reach, to.y),
            to,
        ])
    }

    /// Validate a prospective wire / 校验一条待建立的连线
    ///
    /// # Errors
    /// Returns [`WireError`] for unknown endpoints, self-wiring, a direction
    /// mismatch, or an equivalent duplicate wire.
    /// 端点不存在、自连、方向不匹配或等价连线重复时返回 [`WireError`]。
    pub fn validate_wire(&self, from: Endpoint, to: Endpoint) -> Result<(), WireError> {
        let from_port = self
            .nodes
            .get(from.device)
            .and_then(|node| node.ports.get(from.port))
            .ok_or(WireError::UnknownEndpoint)?;
        let to_port = self
            .nodes
            .get(to.device)
            .and_then(|node| node.ports.get(to.port))
            .ok_or(WireError::UnknownEndpoint)?;
        if from == to {
            return Err(WireError::SamePort);
        }
        if from.device == to.device {
            return Err(WireError::SameDevice);
        }
        let forward = from_port.kind.can_drive() && to_port.kind.can_accept();
        let backward = to_port.kind.can_drive() && from_port.kind.can_accept();
        if !forward && !backward {
            return Err(WireError::Direction);
        }
        let duplicate = self.wires.iter().any(|wire| {
            (wire.from == from && wire.to == to) || (wire.from == to && wire.to == from)
        });
        if duplicate {
            return Err(WireError::Duplicate);
        }
        Ok(())
    }

    /// Validate and add a wire, returning its index / 校验并添加连线，返回索引
    ///
    /// Endpoints are normalised so the driver is always [`Wire::from`].
    /// 端点会被规范化，驱动端始终位于 [`Wire::from`]。
    ///
    /// # Errors
    /// Same as [`CanvasState::validate_wire`] / 与 [`CanvasState::validate_wire`] 相同。
    pub fn add_wire(&mut self, from: Endpoint, to: Endpoint) -> Result<usize, WireError> {
        self.validate_wire(from, to)?;
        let forward = self
            .nodes
            .get(from.device)
            .and_then(|node| node.ports.get(from.port))
            .is_some_and(|port| port.kind.can_drive());
        let (driver, receiver) = if forward { (from, to) } else { (to, from) };
        self.wires.push(Wire {
            from: driver,
            to: receiver,
        });
        Ok(self.wires.len() - 1)
    }

    /// Remove a wire by index, returning it when the index is valid
    /// 按索引删除连线；索引有效时返回被删除的连线
    pub fn remove_wire(&mut self, index: usize) -> Option<Wire> {
        if index >= self.wires.len() {
            return None;
        }
        if matches!(self.selected, Some(Selection::Wire(_))) {
            self.selected = None;
        }
        Some(self.wires.remove(index))
    }

    /// Remove a device and every wire touching it / 删除设备及其所有连线
    ///
    /// Surviving wires have their device indices remapped.
    /// 保留下来的连线会重映射设备索引。
    pub fn remove_node(&mut self, index: usize) -> bool {
        if index >= self.nodes.len() {
            return false;
        }
        self.nodes.remove(index);
        self.wires
            .retain(|wire| wire.from.device != index && wire.to.device != index);
        // 删除后索引前移，遍历剩余连线修正设备引用。
        // Indices shift down after removal, so remap surviving wire endpoints.
        for wire in &mut self.wires {
            if wire.from.device > index {
                wire.from.device -= 1;
            }
            if wire.to.device > index {
                wire.to.device -= 1;
            }
        }
        self.selected = None;
        self.drag = None;
        true
    }

    /// Delete the current selection, returning whether something was removed
    /// 删除当前选中项，返回是否真的删除了内容
    pub fn delete_selected(&mut self) -> bool {
        match self.selected.take() {
            Some(Selection::Device(index)) => self.remove_node(index),
            Some(Selection::Wire(index)) => self.remove_wire(index).is_some(),
            None => false,
        }
    }

    /// Set the current selection / 设置当前选中项
    pub fn select(&mut self, selection: Option<Selection>) {
        self.selected = selection;
    }

    /// Start a drag at a world position / 在世界坐标处开始拖拽
    ///
    /// A port hit starts a wire, a device hit starts a move, and a wire hit
    /// only selects it. Returns whether a drag started.
    /// 命中端口则开始连线，命中设备则开始移动，命中连线只选中它。返回是否开始拖拽。
    pub fn begin_drag(&mut self, world: Pos2) -> bool {
        self.cursor = Some(world);
        if let Some(from) = self.hit_test_port(world) {
            self.selected = None;
            self.drag = Some(DragState::Wire { from });
            return true;
        }
        if let Some(device) = self.hit_test_device(world) {
            let grab = world - self.nodes[device].pos;
            self.selected = Some(Selection::Device(device));
            self.drag = Some(DragState::Device { device, grab });
            return true;
        }
        self.selected = self.hit_test_wire(world).map(Selection::Wire);
        self.drag = None;
        false
    }

    /// Update the active drag to a world position / 将当前拖拽更新到世界坐标
    pub fn update_drag(&mut self, world: Pos2) {
        self.cursor = Some(world);
        if let Some(DragState::Device { device, grab }) = self.drag
            && let Some(node) = self.nodes.get_mut(device)
        {
            node.pos = pos2(
                (world.x - grab.x).clamp(-WORLD_LIMIT, WORLD_LIMIT),
                (world.y - grab.y).clamp(-WORLD_LIMIT, WORLD_LIMIT),
            );
        }
    }

    /// Finish the active drag; releasing on a compatible port adds a wire
    /// 结束当前拖拽；在兼容端口上释放会建立连线
    #[must_use]
    pub fn finish_drag(&mut self, world: Pos2) -> bool {
        self.cursor = Some(world);
        let Some(DragState::Wire { from }) = self.drag.take() else {
            return false;
        };
        let Some(to) = self.hit_test_port(world) else {
            return false;
        };
        if self.add_wire(from, to).is_ok() {
            self.selected = Some(Selection::Wire(self.wires.len() - 1));
            return true;
        }
        false
    }
}

/// Distance from a point to a line segment / 点到线段的距离
fn distance_to_segment(point: Pos2, a: Pos2, b: Pos2) -> f32 {
    let span = b - a;
    let length_sq = span.length_sq();
    if length_sq <= f32::EPSILON {
        return point.distance(a);
    }
    let t = ((point - a).dot(span) / length_sq).clamp(0.0, 1.0);
    point.distance(a + span * t)
}

/// Distance from a point to the sampled cubic bezier of a wire
/// 点到连线三次贝塞尔采样折线的距离
fn distance_to_wire(point: Pos2, control: &[Pos2; 4]) -> f32 {
    let mut previous = control[0];
    let mut best = f32::MAX;
    for step in 1..=8u8 {
        let t = f32::from(step) / 8.0;
        let u = 1.0 - t;
        let weights = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
        let mut sum = Vec2::ZERO;
        for (point, weight) in control.iter().zip(weights) {
            sum += point.to_vec2() * weight;
        }
        let current = sum.to_pos2();
        best = best.min(distance_to_segment(point, previous, current));
        previous = current;
    }
    best
}
