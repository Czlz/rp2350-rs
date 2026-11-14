# rp2350-egui — Public API / 公共 API

> egui GUI crate: Virtual Electronics Lab, HSTX video display, register/memory viewers, debug controls.
> egui GUI crate：虚拟电子实验室、HSTX 视频显示、寄存器/内存查看器、调试控制。
>
> **Status**: 骨架 — 实际仅实现 `EmuApp`（`app.rs`）、寄存器面板（`panels.rs` 的
> `RegisterPanel`/`RegisterSnapshot`）、只读 DMA/HSTX/PIO 外设面板
> （`panels.rs` 的快照类型与格式化函数 + `app.rs` 的渲染函数）与 `canvas`
> 模块；下文的 HSTX 视频解码面板、虚拟实验室画布、内存/GPIO/终端/调试器面板等
> 均为目标设计，尚未实现。
>
> **Status** (EN): skeleton — `EmuApp` (`app.rs`), the register panel
> (`panels.rs`: `RegisterPanel`, `RegisterSnapshot`), the read-only DMA/HSTX/PIO
> peripheral panels (`panels.rs` snapshot types and formatters, rendered by
> `app.rs`) and the `canvas` module are implemented; the HSTX video decoder
> panel, virtual-lab canvas and the memory/GPIO/terminal/debugger panels below
> are target designs, not yet implemented.

---

## Architecture / 架构

```
rp2350-egui/
└── src/
    ├── lib.rs            # module declarations + re-export EmuApp
    ├── app.rs            # EmuApp — eframe::App implementation (已落地 / landed)
    ├── panels.rs         # RegisterPanel / RegisterSnapshot / XpsrFlags (已落地 / landed)
    ├── canvas.rs         # Virtual Lab canvas — Phase 3 stub（桩）
    └── main.rs           # binary entry / 二进制入口

Read-only peripheral panels landed in `panels.rs` + `app.rs`:
    `DmaView`/`DmaChannelView`/`HstxView`/`PioView`/`PioBlockView`/`PioMachineView`
    snapshots, the `format_dma_*`/`format_hstx_*`/`format_pio_*` formatters and the
    `PeripheralSource` trait (`app.rs`) with the `dma_panel`/`hstx_panel`/`pio_panel`
    renderers.
    只读外设面板已落地于 `panels.rs` + `app.rs`：上述快照类型、`format_*` 格式化
    函数，以及 `app.rs` 中的 `PeripheralSource` trait 与三个渲染函数。

Planned target layout (Phase 3, not yet created / 规划中的目标结构，尚未创建):
    panels/{lab, hstx_view, registers→已并入 panels.rs, memory, pio_view,
            gpio_view, terminal, debugger}.rs
    canvas/{chip_view, pin_widget, device_widget, wire, drag_drop}.rs
    theme.rs, icons.rs
```

---

## Main App Structure / 主应用结构

**已落地 / Landed** (`app.rs`)：

```rust
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
    pub fn new(core: Box<dyn CpuCore>) -> Self;
    /// Create an application preloaded with sample Cortex-M33 state
    /// 创建一个预载 Cortex-M33 示例状态的应用
    pub fn sample() -> Self;
    /// Attach a read-only peripheral snapshot source (DMA/HSTX/PIO)
    /// 挂载只读外设快照来源（DMA/HSTX/PIO）
    pub fn with_peripheral_source(self, source: Box<dyn PeripheralSource>) -> Self;
    /// Run the application with a native window
    /// 以原生窗口运行应用
    pub fn run(self) -> Result<(), eframe::Error>;
}

impl eframe::App for EmuApp {
    /// One CentralPanel + RegisterPanel::new(core).show(ui)
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame);
}
```

**目标设计 / Target design（规划中 / planned，Phase 3）**：扩展为
`chip: Arc<Mutex<RP2350Chip>>` + `gdb_server` 句柄 + `plugin_host` + 左侧外设/
插件面板、中央 `VirtualLabCanvas`、右侧寄存器/HSTX 面板、底部 UART 终端与
状态栏，以及 `render_menu()` / `tick_simulator(multiplier)` 执行控制。

---

## Read-Only Peripheral Panels / 只读外设面板

`EmuApp` 在一个 `ScrollArea` 中按「寄存器 → DMA → HSTX → PIO」顺序渲染可折叠面板
（`egui::CollapsingHeader`，默认展开）。面板每帧重新取快照，从不写寄存器；窗口
很小（测试用 120×90）时只产生滚动条，不 panic。

`EmuApp` renders the register viewer followed by three collapsible
(`egui::CollapsingHeader`, open by default) peripheral panels inside one
`ScrollArea`. Every panel re-reads its snapshot each frame, never writes a
register, and a very small window (120×90 in tests) only produces scrollbars.

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
rows, plus headless rendering with and without a source and at 120×90.

---

## Window Layout / 窗口布局

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

### Core Components / 核心组件

```rust
/// The central canvas where users wire GPIO pins to virtual peripherals.
/// 用户将 GPIO 引脚连线到虚拟外设的中央画布。
pub struct VirtualLabCanvas {
    /// Chip view widget (RP2350 package + 48 GPIO pins).
    pub chip_view: ChipViewWidget,

    /// Placed device widgets.
    pub devices: Vec<PlacedDevice>,

    /// Established connections.
    pub wires: Vec<Wire>,

    /// Current drag operation.
    pub dragging: Option<DragWire>,

    /// Canvas pan/zoom.
    pub viewport: CanvasViewport,

    /// Grid snap.
    pub snap_to_grid: bool,
    pub grid_size: f32,
}

impl VirtualLabCanvas {
    /// Render the full canvas.
    pub fn show(&mut self, ui: &mut egui::Ui, chip: &RP2350Chip);

    /// Add a device from the palette.
    pub fn add_device(&mut self, device_type: DeviceType, position: Pos2);

    /// Remove a device.
    pub fn remove_device(&mut self, device_id: &str);

    /// Connect two ports with a wire.
    pub fn add_wire(&mut self, source: WireEndpoint, target: WireEndpoint);

    /// Remove a wire.
    pub fn remove_wire(&mut self, wire_index: usize);

    /// Auto-arrange devices.
    pub fn auto_layout(&mut self);

    /// Save canvas layout to file.
    pub fn save_layout(&self, path: &Path);

    /// Load canvas layout from file.
    pub fn load_layout(&mut self, path: &Path);
}
```

### Chip View Widget / 芯片视图组件

```rust
pub struct ChipViewWidget {
    /// Pin positions on canvas (48 pins + 6 QSPI pins).
    pub pin_positions: HashMap<u8, Pos2>,

    /// Pin labels.
    pub pin_labels: HashMap<u8, String>,

    /// Pin color mapping by function.
    pub pin_colors: HashMap<PinFunction, Color32>,

    /// Chip body rectangle.
    pub chip_rect: Rect,

    /// Internal structure labels.
    pub internal_labels: Vec<InternalLabel>,
}

impl ChipViewWidget {
    /// Render the chip package.
    pub fn show(&mut self, ui: &mut egui::Ui, chip_state: &ChipState);

    /// Draw a single pin (circle + label + direction arrow).
    fn draw_pin(&self, ui: &mut egui::Ui, pin: u8, pos: Pos2, state: &GpioPin);

    /// Highlight connected pins.
    fn highlight_connected(&self, ui: &mut egui::Ui, pin: u8, pos: Pos2);
}
```

### Wire Rendering / 连线渲染

```rust
pub struct Wire {
    pub source: WireEndpoint,
    pub target: WireEndpoint,
    pub color: Color32,
    pub width: f32,
}

pub fn draw_wire(ui: &mut egui::Ui, wire: &Wire) {
    // Cubic bezier from source to target
    let control_offset = Vec2::new(50.0, 0.0);
    let p0 = wire.source.position;
    let p1 = p0 + control_offset;
    let p2 = wire.target.position - control_offset;
    let p3 = wire.target.position;

    // Signal animation: moving dash pattern
    let animated_offset = (ui.ctx().input(|i| i.time) * 100.0) % 30.0;

    // Draw bezier curve
    ui.painter().add(Shape::CubicBezier(epaint::CubicBezierShape {
        points: [p0, p1, p2, p3],
        closed: false,
        fill: Color32::TRANSPARENT,
        stroke: Stroke::new(wire.width, wire.color),
    }));
}
```

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

## Drag-and-Drop Interaction / 拖拽交互

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