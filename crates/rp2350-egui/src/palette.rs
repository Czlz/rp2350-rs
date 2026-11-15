//! Built-in device palette for the Virtual Lab canvas
//! 虚拟实验室画布的内置设备调色板
//!
//! Every entry is a static template: a display name, a box size and a set of
//! default ports. [`DevicePalette::instantiate`] turns a template into a
//! [`DeviceNode`] whose id has the form `palette-<index>-<serial>`, so repeated
//! instantiation never produces duplicate ids.
//! 每个条目都是一个静态模板：显示名称、框尺寸与默认端口集合。
//! [`DevicePalette::instantiate`] 把模板转成 [`DeviceNode`]，其 id 形如
//! `palette-<index>-<serial>`，因此重复实例化不会产生重复 id。

use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Pos2, Vec2};

use crate::canvas::{CanvasState, DeviceNode, Port, PortKind};

/// Monotonic serial keeping palette node ids unique / 单调序号，保证节点 id 唯一
static PALETTE_SERIAL: AtomicU64 = AtomicU64::new(0);

/// One default port of a palette template / 调色板模板的一个默认端口
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PalettePort {
    /// Display name / 显示名称
    pub name: &'static str,
    /// Signal direction / 信号方向
    pub kind: PortKind,
    /// Offset from the node's top-left corner / 相对节点左上角的偏移
    pub offset: Vec2,
}

/// A device template offered by the palette / 调色板提供的设备模板
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaletteEntry {
    /// Display name, also used as the node name / 显示名称，同时作为节点名
    pub name: &'static str,
    /// Box size in world units / 世界单位下的框尺寸
    pub size: Vec2,
    /// Default ports / 默认端口
    pub ports: &'static [PalettePort],
}

/// Ports of the `LED` template / `LED` 模板的端口
const LED_PORTS: &[PalettePort] = &[PalettePort {
    name: "A",
    kind: PortKind::Input,
    offset: Vec2::new(0.0, 25.0),
}];

/// Ports of the `UART Terminal` template / `UART Terminal` 模板的端口
const UART_PORTS: &[PalettePort] = &[
    PalettePort {
        name: "TX",
        kind: PortKind::Input,
        offset: Vec2::new(0.0, 22.0),
    },
    PalettePort {
        name: "RX",
        kind: PortKind::Output,
        offset: Vec2::new(140.0, 48.0),
    },
];

/// Ports of the `Button` template / `Button` 模板的端口
const BUTTON_PORTS: &[PalettePort] = &[PalettePort {
    name: "OUT",
    kind: PortKind::Output,
    offset: Vec2::new(90.0, 25.0),
}];

/// Ports of the `Resistor` template / `Resistor` 模板的端口
const RESISTOR_PORTS: &[PalettePort] = &[
    PalettePort {
        name: "P1",
        kind: PortKind::Bidirectional,
        offset: Vec2::new(0.0, 18.0),
    },
    PalettePort {
        name: "P2",
        kind: PortKind::Bidirectional,
        offset: Vec2::new(96.0, 18.0),
    },
];

/// Ports of the `Power` template / `Power` 模板的端口
const POWER_PORTS: &[PalettePort] = &[PalettePort {
    name: "VCC",
    kind: PortKind::Output,
    offset: Vec2::new(70.0, 25.0),
}];

/// Ports of the `Ground` template / `Ground` 模板的端口
const GROUND_PORTS: &[PalettePort] = &[PalettePort {
    name: "GND",
    kind: PortKind::Input,
    offset: Vec2::new(0.0, 25.0),
}];

/// Ports of the `Logic Probe` template / `Logic Probe` 模板的端口
const PROBE_PORTS: &[PalettePort] = &[PalettePort {
    name: "PROBE",
    kind: PortKind::Input,
    offset: Vec2::new(0.0, 25.0),
}];

/// Ports of the `Potentiometer` template / `Potentiometer` 模板的端口
const POT_PORTS: &[PalettePort] = &[
    PalettePort {
        name: "VCC",
        kind: PortKind::Input,
        offset: Vec2::new(0.0, 18.0),
    },
    PalettePort {
        name: "WIPER",
        kind: PortKind::Output,
        offset: Vec2::new(90.0, 30.0),
    },
    PalettePort {
        name: "GND",
        kind: PortKind::Input,
        offset: Vec2::new(0.0, 44.0),
    },
];

/// Built-in templates, in sidebar order / 内置模板，按侧栏顺序排列
const DEFAULT_ENTRIES: [PaletteEntry; 8] = [
    PaletteEntry {
        name: "LED",
        size: Vec2::new(90.0, 50.0),
        ports: LED_PORTS,
    },
    PaletteEntry {
        name: "UART Terminal",
        size: Vec2::new(140.0, 70.0),
        ports: UART_PORTS,
    },
    PaletteEntry {
        name: "Button",
        size: Vec2::new(90.0, 50.0),
        ports: BUTTON_PORTS,
    },
    PaletteEntry {
        name: "Resistor",
        size: Vec2::new(96.0, 36.0),
        ports: RESISTOR_PORTS,
    },
    PaletteEntry {
        name: "Power",
        size: Vec2::new(70.0, 50.0),
        ports: POWER_PORTS,
    },
    PaletteEntry {
        name: "Ground",
        size: Vec2::new(70.0, 50.0),
        ports: GROUND_PORTS,
    },
    PaletteEntry {
        name: "Logic Probe",
        size: Vec2::new(96.0, 50.0),
        ports: PROBE_PORTS,
    },
    PaletteEntry {
        name: "Potentiometer",
        size: Vec2::new(90.0, 60.0),
        ports: POT_PORTS,
    },
];

/// The built-in device palette / 内置设备调色板
///
/// # Examples / 示例
/// ```
/// use egui::pos2;
/// use rp2350_egui::DevicePalette;
///
/// let palette = DevicePalette::default();
/// assert!(palette.len() >= 6);
/// let node = palette.instantiate(0, pos2(10.0, 20.0)).expect("entry 0");
/// assert!(node.id.starts_with("palette-0-"));
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DevicePalette {
    /// Built-in templates / 内置模板
    entries: &'static [PaletteEntry],
}

impl Default for DevicePalette {
    /// Palette holding the eight built-in templates / 含八个内置模板的调色板
    fn default() -> Self {
        Self {
            entries: &DEFAULT_ENTRIES,
        }
    }
}

impl DevicePalette {
    /// Create the built-in palette / 创建内置调色板
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// All entries in sidebar order / 按侧栏顺序返回全部条目
    #[must_use]
    pub fn entries(&self) -> &'static [PaletteEntry] {
        self.entries
    }

    /// Number of entries / 条目数量
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the palette has no entries / 调色板是否为空
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entry at `index`, or `None` when out of range / 取第 `index` 个条目
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&'static PaletteEntry> {
        self.entries.get(index)
    }

    /// Instantiate entry `index` with its top-left corner at `pos`
    /// 以 `pos` 为左上角实例化第 `index` 个条目
    ///
    /// The returned node gets a unique id of the form
    /// `palette-<index>-<serial>` and copies the template size and ports.
    /// Returns `None` when `index` is out of range.
    /// 返回节点的 id 形如 `palette-<index>-<serial>`，并复制模板尺寸与端口；
    /// `index` 越界时返回 `None`。
    #[must_use]
    pub fn instantiate(&self, index: usize, pos: Pos2) -> Option<DeviceNode> {
        let entry = self.get(index)?;
        // 全局序号保证跨调色板实例也唯一。
        // A process-wide serial keeps ids unique across palette instances.
        let serial = PALETTE_SERIAL.fetch_add(1, Ordering::Relaxed);
        let id = format!("palette-{index}-{serial}");
        let mut node = DeviceNode::new(id, entry.name, pos, entry.size);
        for port in entry.ports {
            node = node.with_port(Port::new(port.name, port.kind, port.offset));
        }
        Some(node)
    }
}

impl CanvasState {
    /// Add a built-in palette device at `pos` and return its node index
    /// 在 `pos` 处添加内置调色板设备并返回节点索引
    ///
    /// Returns `None` when `index` is out of range; the canvas is untouched
    /// in that case.
    /// `index` 越界时返回 `None`，此时画布不变。
    #[must_use]
    pub fn add_from_palette(&mut self, index: usize, pos: Pos2) -> Option<usize> {
        let node = DevicePalette::default().instantiate(index, pos)?;
        Some(self.add_device(node))
    }
}
