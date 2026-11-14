# rp2350-egui — Public API / 公共 API

> egui GUI crate: Virtual Electronics Lab, HSTX video display, register/memory viewers, debug controls.
> egui GUI crate：虚拟电子实验室、HSTX 视频显示、寄存器/内存查看器、调试控制。
>
> **Status**: 骨架 — 已实现 `EmuApp`（`app.rs`）、寄存器面板（`panels.rs` 的
> `RegisterPanel`/`RegisterSnapshot`）、只读 DMA/HSTX/PIO 外设面板
> （`panels.rs` 的快照类型与格式化函数 + `app.rs` 的渲染函数）、`canvas.rs` 的
> 画布数据模型/几何层，以及 `canvas_view.rs` 的 egui 绘制与拖拽交互。应用顶部
> 标签栏可在「虚拟实验室」与「寄存器与外设」之间切换。下文的 HSTX 视频解码面板、
> 内存/GPIO/终端/调试器面板等均为目标设计，尚未实现。
>
> **Status** (EN): skeleton — `EmuApp` (`app.rs`), the register panel
> (`panels.rs`: `RegisterPanel`, `RegisterSnapshot`), the read-only DMA/HSTX/PIO
> peripheral panels (`panels.rs` snapshot types and formatters, rendered by
> `app.rs`), the canvas data model/geometry layer (`canvas.rs`) and the egui
> painter + drag interaction (`canvas_view.rs`) are implemented. A top tab bar
> switches between the Virtual Lab and the register/peripheral panels. The HSTX
> video decoder panel and the memory/GPIO/terminal/debugger panels below remain
> target designs.

---

## Architecture / 架构

```
rp2350-egui/
└── src/
    ├── lib.rs            # module declarations + re-export EmuApp, CanvasState
    ├── app.rs            # EmuApp — eframe::App + tab bar (已落地 / landed)
    ├── panels.rs         # RegisterPanel / RegisterSnapshot / XpsrFlags (已落地 / landed)
    ├── canvas.rs         # Virtual Lab 数据模型与几何（已落地 / landed）
    ├── canvas_view.rs    # Virtual Lab egui 绘制与输入（已落地 / landed）
    └── main.rs           # binary entry / 二进制入口

Read-only peripheral panels landed in `panels.rs` + `app.rs`:
    `DmaView`/`DmaChannelView`/`HstxView`/`PioView`/`PioBlockView`/`PioMachineView`
    snapshots, the `format_dma_*`/`format_hstx_*`/`format_pio_*` formatters and the
    `PeripheralSource` trait (`app.rs`) with the `dma_panel`/`hstx_panel`/`pio_panel`
    renderers.
    只读外设面板已落地于 `panels.rs` + `app.rs`：上述快照类型、`format_*` 格式化
    函数，以及 `app.rs` 中的 `PeripheralSource` trait 与三个渲染函数。

Canvas split: `canvas.rs` holds the pure model/geometry (unit-testable without a
painter) while `canvas_view.rs` adds `CanvasState::show` and the painter. The
split keeps both files under the project's 500-code-line cap.
画布拆分：`canvas.rs` 保存纯数据模型与几何（无需画笔即可单元测试），
`canvas_view.rs` 提供 `CanvasState::show` 与绘制逻辑。拆分保证两个文件都符合
项目单文件 500 行代码上限。

Planned target layout (Phase 3, not yet created / 规划中的目标结构，尚未创建):
    panels/{hstx_view, memory, pio_view, gpio_view, terminal, debugger}.rs
    canvas/{chip_view, pin_widget, device_widget, drag_drop}.rs
    theme.rs, icons.rs
```

---

## Main App Structure / 主应用结构

**已落地 / Landed** (`app.rs`)：

```rust
/// Top-level view selected by the tab bar / 标签栏选中的顶层视图
pub enum ActiveView {
    /// Virtual Lab wiring canvas (default) / 虚拟实验室连线画布（默认）
    VirtualLab,
    /// Register and peripheral panels / 寄存器与外设面板
    Panels,
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
    /// Active top-level view / 当前顶层视图
    view: ActiveView,
}

impl EmuApp {
    /// Create an application for the given core / 为指定核心创建应用
    pub fn new(core: Box<dyn CpuCore>) -> Self;
    /// Create an application preloaded with sample Cortex-M33 state
    /// 创建一个预载 Cortex-M33 示例状态的应用
    pub fn sample() -> Self;
    /// Attach a read-only peripheral snapshot source (DMA/HSTX/PIO)
    /// 挂载只读外设快照来源（DMA/HSTX/PIO）
    pub fn with_peripheral_source(self, source: Box<dyn PeripheralSource>) -> Self;
    /// Canvas model accessors / 画布模型访问器
    pub fn canvas(&self) -> &CanvasState;
    pub fn canvas_mut(&mut self) -> &mut CanvasState;
    /// Read or switch the active tab / 读取或切换当前标签页
    pub fn active_view(&self) -> ActiveView;
    pub fn set_active_view(&mut self, view: ActiveView);
    /// Run the application with a native window
    /// 以原生窗口运行应用
    pub fn run(self) -> Result<(), eframe::Error>;
}

impl eframe::App for EmuApp {
    /// Heading + tab bar + active view (`VirtualLab` or `Panels`)
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame);
}
```

Tab switching only changes the `view` field: `CanvasState` (nodes, wires, pan,
zoom, selection) is owned by `EmuApp` and survives switching back and forth.
切换标签页只改 `view` 字段：`CanvasState`（节点、连线、平移、缩放、选中项）
由 `EmuApp` 持有，来回切换不会丢失。

**目标设计 / Target design（规划中 / planned，Phase 3）**：扩展为
`chip: Arc<Mutex<RP2350Chip>>` + `gdb_server` 句柄 + `plugin_host` + 左侧外设/
插件面板、右侧寄存器/HSTX 面板、底部 UART 终端与状态栏，以及
`render_menu()` / `tick_simulator(multiplier)` 执行控制。

---

## Read-Only Peripheral Panels / 只读外设面板

`EmuApp` 在「Panels / 寄存器与外设」标签页中，于一个 `ScrollArea` 内按
「寄存器 → DMA → HSTX → PIO」顺序渲染可折叠面板（`egui::CollapsingHeader`，
默认展开）。面板每帧重新取快照，从不写寄存器；窗口很小（测试用 120×90）时只产生
滚动条，不 panic。启动默认标签页是「Virtual Lab / 虚拟实验室」。

`EmuApp` renders the register viewer followed by three collapsible
(`egui::CollapsingHeader`, open by default) peripheral panels inside one
`ScrollArea` on the "Panels" tab. Every panel re-reads its snapshot each frame,
never writes a register, and a very small window (120×90 in tests) only produces
scrollbars. The default tab on startup is "Virtual Lab".

### Snapshot source / 快照来源

```rust
/// Read-only snapshot source for the DMA, HSTX and PIO panels
/// DMA、HSTX 与 PIO 面板的只读快照来源
pub trait PeripheralSource {
    fn dma_view(&self) -> Option<DmaView> { None }
    fn hstx_view(&self) -> Option<HstxView> { None }
    fn pio_view(&self) -> Option<PioView> { None }
}
```

- GUI 不依赖 `rp2350-periph`：宿主机器（例如 CLI 的 `Machine`）在自己的 crate 中
  实现该 trait，`EmuApp::with_peripheral_source` 挂载它。
- 未挂载来源或某个句柄不可达时，面板渲染 `NOT_EXPOSED`（`"not exposed / 未暴露"`），
  不会编造数值。当前 `BootPeripherals` 只暴露 `dma()`/`hstx()`，PIO 尚无公开只读
  入口，因此 PIO 面板暂按占位渲染。
- The GUI never depends on `rp2350-periph`: the host machine (for example the CLI
  `Machine`) implements the trait in its own crate and attaches it through
  `EmuApp::with_peripheral_source`. Missing handles render `NOT_EXPOSED`; as of
  writing `BootPeripherals` exposes `dma()`/`hstx()` only, so the PIO panel
  renders placeholders.

### Panel contents / 面板内容

| Panel | Data / 数据 | Formatter |
|-------|-------------|-----------|
| DMA | 16 通道 busy 掩码 + 通道列表、累计传输数（十进制/十六进制）、选中通道 `READ_ADDR`/`WRITE_ADDR`/`TRANS_COUNT`/`CTRL_TRIG`/`BUSY`/`EN`，通道可用下拉框选择，越界索引每帧被钳制 | `format_dma_busy`、`format_dma_count`、`format_dma_channel` |
| HSTX | `CSR.EN`、FIFO 级别（`EMPTY`/`FULL` 标记）、待取输出字数、underrun 次数、未知命令数、最近扫描线摘要（无 API 时占位） | `format_hstx_row`、`format_hstx_fifo`、`format_hstx_scanline` |
| PIO | 三个 block 的 `SM_ENABLE` 掩码、`DBG_PADOUT`/`DBG_PADOE`、`IRQ` 标志位列表、每个 SM 的 `SMx_ADDR`（PC，十六进制+十进制）与 TX/RX FIFO 级别 | `format_pio_block`、`format_pio_row`、`format_pio_irq` |

### Tests / 测试

`cargo test -p rp2350-egui` 覆盖：busy 掩码（含空掩码）、传输计数的十六进制格式、
通道行十六进制快照、通道选择越界、FIFO 空/满、HSTX underrun 行、缺失 API 的占位
文本、`IRQ` 掩码解码、PIO SM 行的十六进制格式，以及三面板在有/无来源与 120×90
小窗口下的无窗口渲染。

Covers the busy mask (including the empty mask), hexadecimal transfer count and
channel rows, out-of-range channel selection, FIFO empty/full, the HSTX underrun
row, missing-API placeholders, `IRQ` mask decoding, hexadecimal PIO state-machine
rows, plus headless rendering with and without a source and at 120×90. Two
app-level tests render the Virtual Lab tab (including an 80×60 window) and check
that switching tabs preserves the canvas nodes and wires.
此外，应用级测试渲染虚拟实验室标签页（含 80×60 小窗口），并验证切换标签页后
画布节点与连线保持不变。

---

## Window Layout / 窗口布局

Current implementation: a heading, a two-tab bar (Virtual Lab / Panels), and the
active view below it. The richer layout below is the target design.
当前实现：标题、两个标签页（虚拟实验室 / 寄存器与外设）以及下方的当前视图。
下面更丰富的布局是目标设计。

```
┌──────────────────────────────────────────────────────────────────┐
│  File │ View │ Debug │ Plugins │ Help                     _ □ X  │
├─────────────┬────────────────────────────────┬───────────────────┤
│  Peripherals│                                │  Registers │ HSTX  │
│  ┌────────┐ │                                │ ┌───────────────┐ │
│  │ ● LED  │ │    ┌────────────────────┐      │ │ R0  0x00000000 │ │
│  │ 💾 TF  │ │    │                    │      │ │ R1  0x20001000 │ │
│  │ 📟UART │ │    │   Virtual Lab      │      │ │ ...            │ │
│  │ 🔍Debug│ │    │   Canvas           │      │ │ PC  0x100001A4 │ │
│  │ 🧠PSRAM│ │    │                    │      │ │ LR  0x10000200 │ │
│  │ 💿Flash│ │    │   (Drag & Drop     │      │ │ SP  0x20040000 │ │
│  └────────┘ │    │    Wire Connect)   │      │ │ XPSR 0x61000000│ │
│             │    │                    │      │ └───────────────┘ │
│  Plugins    │    └────────────────────┘      │ ┌───────────────┐ │
│  ┌────────┐ │                                │ │  HSTX Output  │ │
│  │my_sensor│                                │ │ ┌───────────┐ │ │
│  └────────┘ │                                │ │ │           │ │ │
│             │                                │ │ │  Video    │ │ │
│             │                                │ │ │  Frame    │ │ │
│             │                                │ │ │           │ │ │
│             │                                │ │ └───────────┘ │ │
│             │                                │ │ Scale: 1.0x   │ │
│             │                                │ │ [📷 Screen]  │ │
│             │                                │ └───────────────┘ │
├─────────────┴────────────────────────────────┴───────────────────┤
│  📟 UART Terminal                                                 │
│  ─────────────────────────────────────────────────────────────── │
│  Hello, World!                                                    │
│  System clock: 125 MHz                                            │
│  > ▌                                                             │
├──────────────────────────────────────────────────────────────────┤
│  🟢 Running │ Cycles: 12,345,678 │ PC: 0x1000_01A4 │ FPS: 60.0  │
└──────────────────────────────────────────────────────────────────┘
```

---

## Virtual Lab Canvas / 虚拟实验室画布

### Data model / 数据模型

```rust
/// Signal direction of a canvas port / 画布端口的信号方向
pub enum PortKind { Input, Output, Bidirectional }

/// A connection point on a device node / 设备节点上的连接点
pub struct Port {
    pub name: String,
    pub kind: PortKind,
    /// Offset from the node's top-left corner / 相对节点左上角的偏移
    pub offset: Vec2,
}

/// A device placed on the canvas / 放置在画布上的设备
pub struct DeviceNode {
    /// Stable identifier (matches WiringTable device IDs) / 稳定标识
    pub id: String,
    pub name: String,
    pub pos: Pos2,
    pub size: Vec2,
    pub ports: Vec<Port>,
}

/// Reference to one port of one device / 指向某个设备某个端口的引用
pub struct Endpoint { pub device: usize, pub port: usize }
/// A wire between two device ports / 两个设备端口之间的连线
pub struct Wire { pub from: Endpoint, pub to: Endpoint }

pub enum Selection { Device(usize), Wire(usize) }
pub enum DragState {
    Device { device: usize, grab: Vec2 },
    Wire { from: Endpoint },
}
pub enum WireError {
    UnknownEndpoint, SameDevice, SamePort, Direction, Duplicate,
}

/// Canvas model: devices, wires, viewport and interaction state
/// 画布模型：设备、连线、视口与交互状态
pub struct CanvasState {
    pub nodes: Vec<DeviceNode>,
    pub wires: Vec<Wire>,
    pub pan: Vec2,
    pub zoom: f32,
    pub selected: Option<Selection>,
    pub drag: Option<DragState>,
}
```

`CanvasState::sample()` builds the demo net shown on startup: an RP2350 node
(`GPIO25` output, `GPIO26` input, `GPIO27` bidirectional) wired to an LED input
and a button output. `CanvasState::show(ui)` lives in `canvas_view.rs` and
draws the grid, device boxes, coloured port dots, cubic-bezier wires and the
selected highlight.
`CanvasState::sample()` 构建启动时的示例网络：RP2350 节点（`GPIO25` 输出、
`GPIO26` 输入、`GPIO27` 双向）连接 LED 输入与按钮输出。`CanvasState::show(ui)`
位于 `canvas_view.rs`，绘制网格、设备框、按方向着色的端口圆点、三次贝塞尔
连线与选中高亮。

### Geometry and API / 几何与 API

| API | Behaviour / 行为 |
|-----|------------------|
| `world_to_screen(viewport, world)` / `screen_to_world(viewport, screen)` | `screen = viewport.min + world * zoom + pan`; the inverse divides by `zoom`. |
| `set_zoom(zoom)` / `set_pan(pan)` | Zoom is clamped to `[0.25, 4.0]`, pan to `±4096 px`. |
| `hit_test_device(world)` | Topmost node whose box contains the point. / 命中最上层设备框。 |
| `hit_test_port(world)` | Topmost port within an 8 px screen radius (scaled by zoom). / 命中 8 px 屏幕半径内的端口。 |
| `hit_test_wire(world)` | Nearest wire within 6 px of its sampled bezier. / 命中贝塞尔采样线 6 px 内的连线。 |
| `begin_drag` / `update_drag` / `finish_drag` | Port hit starts a wire, device hit starts a move with a grab offset, wire hit only selects. `finish_drag` returns whether a wire was added. |
| `validate_wire(from, to)` / `add_wire` | Wires must join different devices; the source must be `Output`/`Bidirectional` and the sink `Input`/`Bidirectional`. `add_wire` swaps reversed input→output calls, and rejects duplicates in either order. |
| `remove_wire(index)` / `remove_node(index)` | `remove_node` cascades all touching wires and remaps surviving endpoint device indices. |
| `delete_selected()` | Deletes the selected device or wire; returns whether something was removed. |
| `select(selection)` | Replaces the selection (`None` clears it). |
| `port_position(endpoint)` / `wire_points(wire)` | Resolve an endpoint / a wire to world-space positions and bezier control points. |

Wire legality in short: 自连（同设备或同端点）、输入→输入、输出→输出、重复连线都被
拒绝；反向的输入→输出调用会被规范化为输出→输入。

### Interaction / 交互

| Input / 输入 | Effect / 效果 |
|--------------|---------------|
| Left press on a device, then drag | Moves the device by the grab offset; ports keep their relative offsets. / 按抓取偏移移动设备，端口相对位置不变。 |
| Left press on a port, drag, release on another port | Shows a yellow preview line; on release adds a wire when legal. / 显示黄色预览线，合法的端口上释放即建线。 |
| Left press on a wire | Selects the wire (yellow highlight); `Delete` removes it. |
| `Delete` while the canvas is hovered/focused | Deletes the selected device (cascading wires) or wire. |
| Mouse wheel / ctrl+wheel | Zooms the whole canvas within the clamp. / 在钳制范围内缩放画布。 |
| Middle-button drag, or space + left drag | Pans the canvas by the pointer delta. / 按指针增量平移画布。 |

Colours: input ports green, output ports red, bidirectional ports purple;
selected devices/wires are yellow. A tiny window only shrinks the viewport
(`ui.available_size()` is clamped to at least 64×64), so painting never panics.

### Limits / 限制

- 画布是纯编辑模型：**不驱动仿真**，端口不会读写 `GPIO`，也不会调用
  `DeviceManager`/`WiringTable`。
- **未持久化**：没有布局保存/加载、自动布局、网格吸附、撤销/重做、主题系统。
- 没有设备面板/芯片视图：设备只能通过 `add_device` 或 `sample()` 代码添加，
  连线只在设备端口之间建立，尚未连接到 RP2350 芯片引脚。
- The canvas is editor-only: it never drives the simulator, never persists, and
  has no device palette, chip view, snapping, undo/redo or theming yet.

### Tests / 测试

`cargo test -p rp2350-egui` 中画布相关测试（`canvas_view.rs` 的 `tests`）：

- 空画布默认值；设备/端口命中与未命中；
- 缩放 + 平移后的世界↔屏幕坐标往返（1e-3 容差）、缩放与平移边界钳制；
- 同端点/同设备/未知端点拒绝、输入→输入拒绝、正反序重复连线拒绝；
- 输出→输入成功且方向被规范化、双向端口两个方向都可连；
- 删除设备级联删除连线并重映射索引；
- 设备拖拽更新位置且端口偏移不变、端口拖拽建线、空白释放不建线；
- 选中设备/连线的删除、`remove_wire` 越界返回 `None`；
- 空画布、单节点与 20×20 极小窗口的无窗口渲染不 panic。

English: default/empty state, device and port hits and misses, world↔screen
round-trip after zoom/pan, zoom/pan clamping, self/same-device/unknown endpoint
rejection, input→input rejection, duplicate rejection in both orders,
output→input normalisation, bidirectional ports, cascade deletion with index
remapping, device drag with stable port offsets, wire drag/release, selected
deletion, and headless rendering of empty/single-node canvases at 20×20.

---

## HSTX Video Panel / HSTX 视频面板

```rust
pub struct HstxView {
    pub frame_buffer: Vec<u8>,         // Decoded RGBA8
    pub width: u32,
    pub height: u32,
    pub texture: Option<egui::TextureHandle>,
    pub decoder: HstxDecoder,
    pub auto_scale: bool,
    pub scale: f32,
    pub last_screenshot: Option<PathBuf>,
    pub fps: f32,
    pub frame_count: u32,
    pub last_frame_time: Instant,
    pub pixel_format: PixelFormat,
    pub show_stats: bool,
}

impl HstxView {
    pub fn show(&mut self, ui: &mut egui::Ui, hstx: &Hstx);

    /// Decode raw HSTX pixel stream to RGBA8.
    pub fn decode_frame(&mut self, raw: &[u32]);

    /// Upload decoded frame to egui texture.
    pub fn upload_texture(&mut self, ctx: &egui::Context);

    /// Save current frame as PNG.
    pub fn save_screenshot(&self, path: &Path) -> Result<()>;
}
```

---

## Drag-and-Drop Interaction (target) / 拖拽交互（目标设计）

The implemented canvas interaction is documented in
[Interaction / 交互](#interaction--交互) above. The richer palette/chip workflow
below is still a target design and is not implemented yet.
已实现的画布交互见上文「Interaction / 交互」。下面更丰富的调色板/芯片工作流仍是
目标设计，尚未实现。

```
1. User clicks a peripheral card in the left panel.
   用户点击左侧面板的外设卡片。

2. Card follows cursor as a drag preview.
   卡片跟随光标作为拖拽预览。

3. User releases over the virtual lab canvas.
   用户在虚拟实验室画布上释放。

4. DeviceWidget is created at the drop position.
   在释放位置创建 DeviceWidget。

5. User drags from a GPIO pin on the chip (or a device port).
   用户从芯片上的 GPIO 引脚（或设备端口）拖动。

6. A red/green dashed line follows the cursor.
   一条红/绿虚线跟随光标。

7. User releases over a compatible device port.
   用户在兼容的设备端口上释放。

8. A permanent bezier Wire is created, color-coded by signal type.
   创建一条永久的贝塞尔连线，按信号类型颜色编码:
   - GPIO output → Red
   - GPIO input  → Green
   - PIO signal  → Yellow
   - SPI signal  → Blue
   - I2C signal  → Cyan
```