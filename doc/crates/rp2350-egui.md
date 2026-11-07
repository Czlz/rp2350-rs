# rp2350-egui — Public API / 公共 API

> egui GUI crate: Virtual Electronics Lab, HSTX video display, register/memory viewers, debug controls.
> egui GUI crate：虚拟电子实验室、HSTX 视频显示、寄存器/内存查看器、调试控制。

---

## Architecture / 架构

```
rp2350-egui/
└── src/
    ├── lib.rs              # module declarations
    ├── app.rs              # Rp2350EmuApp — eframe::App implementation
    ├── panels/
    │   ├── mod.rs
    │   ├── lab.rs          # Virtual Lab canvas (drag + wire connect)
    │   ├── hstx_view.rs    # HSTX video output panel
    │   ├── registers.rs    # Register viewer (R0-R15, NVIC, FPU)
    │   ├── memory.rs       # Memory hex viewer
    │   ├── pio_view.rs     # PIO instruction memory + SM state
    │   ├── gpio_view.rs    # GPIO pin monitor (48 pins)
    │   ├── terminal.rs     # Virtual UART terminal
    │   └── debugger.rs     # Debug control panel (breakpoints, step, trace)
    ├── canvas/
    │   ├── mod.rs
    │   ├── chip_view.rs    # RP2350 chip package view with draggable pins
    │   ├── pin_widget.rs   # Draggable pin component
    │   ├── device_widget.rs# Virtual peripheral device component
    │   ├── wire.rs         # Bezier curve wire rendering
    │   └── drag_drop.rs    # Drag-and-drop from peripheral palette
    ├── theme.rs            # egui visual theme configuration
    └── icons.rs            # Embedded SVG/PNG icon resources
```

---

## Main App Structure / 主应用结构

```rust
pub struct Rp2350EmuApp {
    /// Shared emulator instance.
    pub chip: Arc<Mutex<RP2350Chip>>,

    /// GDB server handle (for status display).
    pub gdb_server: Option<Arc<GdbServerHandle>>,

    /// Plugin host.
    pub plugin_host: Arc<Mutex<PluginHost>>,

    // --- Panel states ---
    pub left_panel: LeftPanel,
    pub canvas: VirtualLabCanvas,
    pub right_panel: RightPanel,
    pub terminal: TerminalPanel,
    pub status_bar: StatusBar,

    // --- Execution control ---
    pub running: bool,
    pub step_mode: StepMode,
    pub speed_multiplier: f64,
    pub cycle_budget: u64,
}

impl Rp2350EmuApp {
    pub fn new(chip: RP2350Chip) -> Self;
    pub fn set_gdb_server(&mut self, server: GdbServerHandle);
    pub fn load_plugin(&mut self, path: &Path) -> Result<(), PluginError>;

    /// Render one frame.
    fn render_menu(&mut self, ctx: &egui::Context);
    fn tick_simulator(&mut self, multiplier: f64);
}
```

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