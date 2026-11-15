# rp2350-egui — Public API / 公共 API

> egui GUI crate: Virtual Electronics Lab, HSTX video display, register/memory viewers, debug controls.
> egui GUI crate：虚拟电子实验室、HSTX 视频显示、寄存器/内存查看器、调试控制。
>
> **Status**: 骨架 — 已实现 `EmuApp`（`app.rs`）、寄存器面板（`panels.rs` 的
> `RegisterPanel`/`RegisterSnapshot`）、只读 DMA/HSTX/PIO 外设面板
> （`panels.rs` 的快照类型与格式化函数 + `app.rs` 的渲染函数）、`canvas.rs` 的
> 画布数据模型/几何层、`canvas_view.rs` 的 egui 绘制与拖拽交互，以及设备调色板
> （`palette.rs`）与文本持久化（`canvas_io.rs`）。应用顶部标签栏可在「虚拟实验室」
> 「游戏」与「寄存器与外设」之间切换；虚拟实验室标签页带保存/加载工具栏，游戏
> 标签页（`play_view.rs`）实时显示 RGB565 帧并映射键盘，已通过
> `play_view/cli_source.rs` 接到真实 `PlaySession`（`main.rs` 默认挂载）。
> 下文的 HSTX 视频解码面板、内存/GPIO/终端/调试器面板等均为目标设计，尚未实现。
>
> **Status** (EN): skeleton — `EmuApp` (`app.rs`), the register panel
> (`panels.rs`: `RegisterPanel`, `RegisterSnapshot`), the read-only DMA/HSTX/PIO
> peripheral panels (`panels.rs` snapshot types and formatters, rendered by
> `app.rs`), the canvas data model/geometry layer (`canvas.rs`), the egui painter
> + drag interaction (`canvas_view.rs`), the built-in device palette
> (`palette.rs`) and canvas text persistence (`canvas_io.rs`) are implemented. A
> top tab bar switches between the Virtual Lab, the Play tab and the
> register/peripheral panels; the Virtual Lab tab carries a save/load toolbar and
> the Play tab (`play_view.rs`) shows live RGB565 frames with keyboard input,
> wired to the real `PlaySession` through `play_view/cli_source.rs` and attached
> by `main.rs` by default. The HSTX video decoder panel and the
> memory/GPIO/terminal/debugger panels below remain target designs.

---

## Architecture / 架构

```
rp2350-egui/
└── src/
    ├── lib.rs            # module declarations + re-export EmuApp, CanvasState, PlayView, CliPlayStarter
    ├── app.rs            # EmuApp — eframe::App + tab bar + save/load toolbar (已落地 / landed)
    ├── panels.rs         # RegisterPanel / RegisterSnapshot / XpsrFlags (已落地 / landed)
    ├── canvas.rs         # Virtual Lab 数据模型与几何（已落地 / landed）
    ├── canvas_view.rs    # Virtual Lab egui 绘制、输入与调色板侧栏（已落地 / landed）
    ├── palette.rs        # 内置设备调色板模板（已落地 / landed）
    ├── canvas_io.rs      # 画布文本格式 to_text/from_text（已落地 / landed）
    ├── play_view.rs      # 游戏标签页：帧纹理、缩放、键盘映射（已落地 / landed）
    ├── play_view/cli_source.rs # PlaySession → PlaySource 适配器（已接线 / wired）
    ├── play_view/tests.rs # 游戏标签页测试（仅 test 构建 / test-only）
    ├── canvas_tests.rs   # 调色板/持久化测试（仅 test 构建 / test-only）
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
    /// Play tab: live game picture and keyboard / 游戏标签页：实时画面与键盘
    Play,
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
    /// Last canvas text produced by the toolbar "Save"
    /// 工具栏「保存」产生的最近一份画布文本
    canvas_snapshot: String,
    /// Result of the last save/load shown in the toolbar
    /// 工具栏中显示的最近一次保存/加载结果
    canvas_status: String,
    /// Play tab model (worker, texture, toolbar fields), preserved across tabs
    /// 游戏标签页模型（worker、纹理、工具栏字段），切换标签页时保留
    play: PlayView,
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
    /// Play tab accessors / 游戏标签页访问器
    pub fn play(&self) -> &PlayView;
    pub fn play_mut(&mut self) -> &mut PlayView;
    /// Attach the play worker factory (host side) / 挂载游玩 worker 工厂（宿主侧）
    pub fn with_play_starter(self, starter: Box<dyn PlayStarter>) -> Self;
    /// Canvas model accessors / 画布模型访问器
    pub fn canvas(&self) -> &CanvasState;
    pub fn canvas_mut(&mut self) -> &mut CanvasState;
    /// Read or switch the active tab / 读取或切换当前标签页
    pub fn active_view(&self) -> ActiveView;
    pub fn set_active_view(&mut self, view: ActiveView);
    /// Toolbar actions: snapshot to / restore from the in-memory text
    /// 工具栏动作：保存到内存文本 / 从内存文本恢复
    fn save_canvas(&mut self);
    fn load_canvas(&mut self) -> Result<(), CanvasParseError>;
    /// Run the application with a native window
    /// 以原生窗口运行应用
    pub fn run(self) -> Result<(), eframe::Error>;
}

impl eframe::App for EmuApp {
    /// Heading + tab bar + active view (`VirtualLab`, `Play` or `Panels`)
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame);
}
```

Tab switching only changes the `view` field: `CanvasState` (nodes, wires, pan,
zoom, selection) and `PlayView` (worker, texture, toolbar text) are owned by
`EmuApp` and survive switching back and forth. A running play worker keeps
stepping in the background because the session lives in its own thread.
切换标签页只改 `view` 字段：`CanvasState`（节点、连线、平移、缩放、选中项）与
`PlayView`（worker、纹理、工具栏文本）由 `EmuApp` 持有，来回切换不会丢失。正在
运行的游玩 worker 会继续在后台推进，因为会话位于自己的线程中。

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

## PlayView / 游戏标签页

「Play / 游戏」标签页（`play_view.rs`）把后台游玩 worker 发布的 RGB565 帧实时显示
出来，并把键盘映射成 GBA 按键位图。它是 GUI 中**唯一**会写仿真器的视图（写的是
按键掩码），其余面板仍然只读。

The "Play" tab (`play_view.rs`) shows the RGB565 frames published by the
background play worker and maps the keyboard onto the GBA button bitmap. It is
the only GUI view that writes to the simulator (a button mask); every other panel
stays read-only.

### Wiring model / 接线模型

依赖方向为 **`rp2350-egui → rp2350-cli`**：旧的 `rp2350-cli → rp2350-egui` 依赖已删除
（CLI 源码从未使用它，却造成包循环），因此本 crate 现在可以在
`play_view/cli_source.rs` 里提供真实适配器 `CliPlaySource` / `CliPlayStarter`。视图本身
仍只与两个 trait 交互，测试与其它宿主依然可以注入假 worker。

The dependency points **`rp2350-egui → rp2350-cli`**: the stale
`rp2350-cli → rp2350-egui` edge (never used by CLI source, but a package cycle) is
gone, so this crate ships the real adapter `CliPlaySource` / `CliPlayStarter` in
`play_view/cli_source.rs`. The view still talks only to the two traits, so tests
and alternate hosts can keep injecting a fake worker.

```rust
/// Running play worker as seen by the GUI / GUI 视角的运行中 worker
pub trait PlaySource {
    fn latest_frame(&self) -> Option<Arc<PlayFrame>>;
    fn set_buttons(&self, mask: u16);
    fn stats(&self) -> PlayStats;
    fn is_running(&self) -> bool;
    fn button_pins(&self) -> u16;
    fn shutdown(self: Box<Self>) -> PlayStats;
}

/// Factory that boots a PlaySource from a PlayRequest
/// 用 PlayRequest 启动 PlaySource 的工厂
pub trait PlayStarter {
    fn start(&self, request: PlayRequest) -> Result<Box<dyn PlaySource>, String>;
}
```

宿主侧适配现在就在本 crate 内（`play_view/cli_source.rs`，不再需要 CLI 的 `gui` 子命令）：

```rust
/// 由运行中的 CLI 游玩会话支撑的来源 / source backed by a live CLI session
pub struct CliPlaySource { session: PlaySession, cached: Mutex<Option<Arc<PlayFrame>>> }

impl PlaySource for CliPlaySource {
    fn latest_frame(&self) -> Option<Arc<PlayFrame>> { /* 按 index 缓存转换结果 */ }
    fn set_buttons(&self, mask: u16) { self.session.set_buttons(mask); }
    fn stats(&self) -> PlayStats { /* frames/steps/fps/steps_per_second + replaced */ }
    fn is_running(&self) -> bool { self.session.is_running() }
    fn button_pins(&self) -> u16 { self.session.button_pins() }
    fn shutdown(self: Box<Self>) -> PlayStats { /* 先取 replaced，再 shutdown */ }
}

pub struct CliPlayStarter;

impl PlayStarter for CliPlayStarter {
    fn start(&self, request: PlayRequest) -> Result<Box<dyn PlaySource>, String> {
        let frame = FrameSpec::parse(request.frame.trim())?;
        let config = PlayConfig {
            frame,
            steps_per_frame: request.steps_per_frame,
            max_frames: None,
            button_pins: request.button_pins,
            audio: request.audio,
            trace_steps: 0,
        };
        let options = MachineOptions { arch: None, max_cycles: u64::MAX, strict_decode: false };
        PlaySession::start(request.image, options, config)
            .map(|session| Box::new(CliPlaySource::new(session)) as Box<dyn PlaySource>)
    }
}
```

`arch: None` 让 UF2 家族 / ELF 自行选核，`max_cycles = u64::MAX` 表示交互式游玩没有
指令预算上限（用 Stop 结束，而不是因预算停机）。`latest_frame()` 把 CLI 的
`Arc<Frame>` 转成 GUI 的 `PlayFrame`，并按帧序号缓存，因此 UI 重绘快于游戏帧率时不会
重复拷贝像素缓冲。

`arch: None` lets the UF2 family / ELF pick the core and `max_cycles = u64::MAX`
means an interactive game has no instruction budget (Stop ends it, not a budget
halt). `latest_frame()` converts the CLI `Arc<Frame>` into the GUI `PlayFrame` and
caches it by index, so repaints faster than the guest never re-copy the pixels.

原生入口 `main.rs` 挂载现成适配器，因此 `cargo run -p rp2350-egui` 的「游戏」标签页
可以直接读固件并出画面；`EmuApp::sample()` 与 `EmuApp::default()` 保持不变：

```rust
EmuApp::sample()
    .with_play_starter(Box::new(CliPlayStarter))
    .run()
```

The native entry point attaches the ready-made starter, so the Play tab of
`cargo run -p rp2350-egui` can read firmware and show frames immediately;
`EmuApp::sample()` and `EmuApp::default()` stay unchanged.

未挂载 starter 的 `PlayView::default()` 仍然可用（测试与 `EmuApp::sample()` 依赖这一点）：
工具栏、占位画面与状态栏照常渲染，「Start / 启动」把
`no play source attached / 未挂载游玩来源` 写进状态行，不 panic。

A `PlayView::default()` without a starter still works (tests and `EmuApp::sample()`
rely on it): the toolbar, the placeholder picture and the status bar still render;
"Start" writes `no play source attached` into the status line instead of
panicking.

### Toolbar and fields / 工具栏与字段

| 字段 / Field | 默认值 / Default | 说明 / Notes |
|--------------|------------------|--------------|
| `file` | `DEFAULT_FIRMWARE` = `E:\Source\mcu\pico\rp2350\gba\pokeemerald-rp2350\rp2350\hw\build_emerald\emerald.uf2` | 启动时 `std::fs::read`；读失败写状态行 |
| `fb` | `0x20063DF4:240x160` | `ADDR:WxH[:STRIDE]`，由宿主用 `FrameSpec::parse` 解析 |
| `steps_per_frame` | `2500000` | 必须是正整数，否则写状态行 |
| `audio` | `false` | 传给 `PlayConfig::audio`（当前仅捕获，无播放后端） |
| `status` | `idle / 空闲` | `running` / `stopped` / `start failed` / `cannot read firmware` |

按键位（`GP0..GP9`，按下 = 低电平，固件内部上拉）：

| 键 / Key | 按键 / Button | 引脚 / Pin | 掩码位 / Bit |
|----------|---------------|-----------|--------------|
| `Z` | A | GP0 | `0x001` |
| `X` | B | GP1 | `0x002` |
| `Backspace` | Select | GP2 | `0x004` |
| `Enter` | Start | GP3 | `0x008` |
| `→` | Right | GP4 | `0x010` |
| `←` | Left | GP5 | `0x020` |
| `↑` | Up | GP6 | `0x040` |
| `↓` | Down | GP7 | `0x080` |
| `S` | R | GP8 | `0x100` |
| `A` | L | GP9 | `0x200` |

键盘只在「画面区域」拥有焦点时生效：点击画面获得焦点（蓝色描边，提示文字消失），
点击别处释放；为 `false` 时不写掩码。映射是纯函数
（`KeyState::from_pressed` / `KeyState::from_input` / `mask_from_keys`），可无窗口测试。

Keys only apply while the picture region owns focus: click the picture to focus it
(blue outline, hint disappears) and click elsewhere to release it; with no focus
the mask is not written. Mapping is pure (`KeyState`, `mask_from_keys`).

### Texture and scaling / 纹理与缩放

- 每帧轮询 `latest_frame()`；只有 `frame.index != last_frame` 时才把 RGB565 转成
  `egui::ColorImage` 并写入纹理（`TextureHandle::set`，首帧用
  `ctx.load_texture`），所以 UI 刷新率高于游戏帧率时不会重复上传。
- RGB565 → `Color32`：`(r << 3) | (r >> 2)`、`(g << 2) | (g >> 4)`、`(b << 3) | (b >> 2)`，
  满量程正好到 255；短帧用黑色补齐（不会 panic）。
- 采样用 `TextureOptions::NEAREST`，画面保持锐利。
- 缩放：`fit_size(available, w, h)` 取 `min(avail/w, avail/h)`，≥ 1 时向下取整为**整数倍**
  （240×160 在 1280×720 里按 4× → 960×640，在 320×240 里按 1× → 240×160），
  < 1 时保留小数倍率缩小（80×60 窗口 → 80×53.3），始终居中、保持 3:2。
- 极小窗口（80×60）只裁剪不 panic：画面可用尺寸钳制到 ≥ 1 px，零尺寸时跳过绘制。

### Status bar / 状态栏

```
frame=<index> steps=<..> fps=<..> steps/s=<..> replaced=<..> · running|stopped
buttons / 按键: A|Right
```

`halted` 存在时追加 ` · halted / 停机: <原因>`（例如指令预算耗尽、非法指令）。
`replaced` 是「未被 GUI 取走即被替换」的帧数；它持续增长说明 UI 或渲染跟不上 worker
（当前 worker 约 8 fps）。

### 帧率说明 / Frame rate

worker 每帧执行 `steps_per_frame` 步后发布一帧，因此帧率 ≈ `steps/s / steps_per_frame`。
当前实测约 **8 fps**（20.6 M steps/s，每帧 2.5 M 步），远低于 GBA 的 60 fps；画面
能看，但动作偏慢。提高办法见 [`doc/playability-plan.md`](../playability-plan.md)
的 P0（解释器热路径），或降低 `steps_per_frame`（画面更快但游戏内时间更慢）。

The worker publishes one frame per `steps_per_frame` guest steps, so the frame
rate is ≈ `steps/s / steps_per_frame`. The measured rate is about **8 fps**
(20.6 M steps/s at 2.5 M steps/frame), far below the GBA's 60 fps: the picture is
readable but slow. See the P0 rows of the playability plan for the way forward.

### Limits / 限制

- **无音频输出接线**：`audio` 只打开 worker 的 I2S 捕获（`AudioTap`），GUI 没有播放
  后端，也不消费采样；`rp2350-audio` 尚未接入本视图。
- **无存档 / 无复位**：不能保存进度，也不能从 GUI 复位客户机；只能停止后重启。
- **无手柄 / 无按键重绑**：键位写死为上表，不支持 gamepad 或自定义映射。
- **无暂停 / 单步 / 截图**：只有 Start 与 Stop；帧率与步数由 `steps_per_frame` 决定。
- 依赖宿主挂载 `PlayStarter`：`PlayView::default()` 仍无 starter（便于测试注入假实现），
  但原生入口 `main.rs` 已默认挂载 `CliPlayStarter`，所以 `cargo run -p rp2350-egui`
  的「游戏」标签页可直接出画面。
- The starter is still injectable (`PlayView::default()` has none, for tests and
  alternate hosts), but the native `main.rs` attaches `CliPlayStarter` by default,
  so the standalone `rp2350-egui` binary shows real frames. There is still no audio
  output wiring (capture only), no save states, no reset, no gamepad or rebinding,
  and no pause/step/screenshot.

### Tests / 测试

`play_view/tests.rs`（`#[cfg(test)] mod tests`，独立文件以免挤占行数预算）：

- RGB565 三原色 / 黑 / 白，5 位与 6 位通道的高位复制（`0x8410` → `#84/82/84` 等）；
- `color_image` 尺寸、行优先顺序与短输入黑填充；
- 键盘掩码：无键为空、单键逐位对应 GP0..GP9、组合键（`Z`+`S` → A|R）、全键 = `0x3FF`；
- `KeyState::from_input` 通过 `RawInput` 事件读出 `↑`+`X` → UP|B；
- `fit_size`：1280×720 → 4×（960×640）、320×240 → 1×（240×160）、80×60 → 小数
  缩小且保持 3:2、零尺寸/零帧返回 `Vec2::ZERO`；
- `format_stats` 的进度行与 `halted` 后缀、`describe_mask` 的按键名；
- 无会话时只渲染「未运行」占位且不 panic；
- 80×60 / 320×240 / 1280×720 三种尺寸无窗口渲染 `ui()`；
- 同一帧不重复上传（`uploads` 计数与 `last_frame`）；
- 未获焦点不写掩码、获焦点后 `Z` → `0x001` 送达假来源；
- 启动错误路径：读文件失败、未挂载 starter、非法 `steps_per_frame`、starter 返回
  `Err` 都只写状态行；成功路径把 `PlayRequest`（映像、帧缓冲、步数、音频、引脚）
  交给 starter，`stop()` 调用一次 `shutdown`；
- **真实适配器**（`CliPlaySource` / `CliPlayStarter`，用合成 UF2 启动真正的
  `PlaySession`）：首帧发布后 `latest_frame()` 有值、`button_pins() == 0x3FF`、
  `shutdown()` 后 `is_running() == false`；`set_buttons` 透传（读回一致）；统计中
  `frames`/`steps` 单调递增且 `steps_per_second > 0`；同一帧重复轮询返回同一个
  `Arc`（像素不重复拷贝）；非法帧文本 / 未知映像 / 零步数返回 `Err` 而不 panic；
- 端到端：`PlayView` 挂载 `CliPlayStarter` + 临时 UF2 文件启动后，无头渲染循环
  能上传至少一帧，`stop()` 后状态为 `stopped / 已停止`；
- 应用层：`ActiveView::Play` 在 760×600 与 80×60 下渲染，切到其它标签再切回后
  状态保持。

English: RGB565 primaries plus bit replication, `color_image` padding and order,
empty/single/combined key masks with the GP0..GP9 layout, live `KeyState::from_input`,
integer/fractional `fit_size` cases, `format_stats`/`describe_mask`, idle
placeholder rendering, headless 80×60 / 320×240 / 1280×720 rendering, upload
deduplication per frame index, focus-gated mask delivery, every start/stop error
path, the real `PlaySession` adapter (first frame, mask round-trip, monotonic
statistics, cached `Arc`, bad frame text / unknown image / zero steps rejected),
the end-to-end `CliPlayStarter` + headless render path, and the app-level Play tab
that keeps its state across tab switches.

---

## Window Layout / 窗口布局

Current implementation: a heading, a three-tab bar (Virtual Lab / Play / Panels),
and the active view below it. The richer layout below is the target design.
当前实现：标题、三个标签页（虚拟实验室 / 游戏 / 寄存器与外设）以及下方的当前视图。
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

### Device palette / 设备调色板

`palette.rs` 提供 `DevicePalette`（8 个内置模板）。侧栏位于画布左侧，点击按钮即在
画布中心附近实例化一个节点（连续点击按 14 px 递增错开，避免完全重叠）：

| # | 条目 / Entry | 尺寸 / Size | 默认端口 / Default ports |
|---|--------------|-------------|--------------------------|
| 0 | LED / 发光二极管 | 90×50 | `A` in |
| 1 | UART Terminal / 终端 | 140×70 | `TX` in, `RX` out |
| 2 | Button / 按钮 | 90×50 | `OUT` out |
| 3 | Resistor / 电阻 | 96×36 | `P1` bidi, `P2` bidi |
| 4 | Power / 电源 | 70×50 | `VCC` out |
| 5 | Ground / 地 | 70×50 | `GND` in |
| 6 | Logic Probe / 逻辑探针 | 96×50 | `PROBE` in |
| 7 | Potentiometer / 电位器 | 90×60 | `VCC` in, `WIPER` out, `GND` in |

```rust
pub struct PalettePort {
    pub name: &'static str,
    pub kind: PortKind,
    pub offset: Vec2,
}

pub struct PaletteEntry {
    pub name: &'static str,
    pub size: Vec2,
    pub ports: &'static [PalettePort],
}

pub struct DevicePalette { /* 8 个静态模板 / 8 static templates */ }

impl DevicePalette {
    pub fn new() -> Self;
    pub fn entries(&self) -> &'static [PaletteEntry];
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn get(&self, index: usize) -> Option<&'static PaletteEntry>;
    /// 越界返回 None / returns None when out of range
    pub fn instantiate(&self, index: usize, pos: Pos2) -> Option<DeviceNode>;
}

impl CanvasState {
    /// 越界返回 None，画布不变 / returns None and leaves the canvas untouched
    pub fn add_from_palette(&mut self, index: usize, pos: Pos2) -> Option<usize>;
}
```

`instantiate` 生成的 id 固定为 `palette-<index>-<serial>`（进程级原子序号），因此同一
模板重复实例化也不会出现重复 id；`pos` 是节点左上角的世界坐标。

### Persistence / 持久化

「Virtual Lab / 虚拟实验室」标签页顶部有工具栏，含两个按钮和一行状态：

| Button / 按钮 | Effect / 效果 |
|---------------|---------------|
| `Save / 保存` | 把 `CanvasState::to_text()` 写入 `EmuApp` 的**内存快照**（不打开文件对话框），状态行显示字节数与设备/连线数。 |
| `Load / 加载` | 用 `CanvasState::from_text()` 解析内存快照并整体替换画布；失败时保留当前画布，状态行显示 `canvas line N: ...`。 |

文本格式（版本 1，首行为 `rp2350-canvas;1`）：

```text
rp2350-canvas;1
zoom;1
pan;0;0
device;mcu;RP2350;60;70;170;130
port;GPIO25;out;0;34
port;GPIO26;in;0;62
device;led;LED;360;60;120;70
port;A;in;0;34
wire;0;0;1;0
```

| 记录 / Record | 字段 / Fields | 说明 / Notes |
|---------------|---------------|--------------|
| `rp2350-canvas` | `rp2350-canvas;1` | 必须是首个非注释行；目前只支持版本 `1`。 |
| `zoom` | `zoom;<f32>` | 载入后钳制到 `[0.25, 4.0]`。 |
| `pan` | `pan;<x>;<y>` | 载入后钳制到 `±4096`。 |
| `device` | `device;<id>;<name>;<x>;<y>;<w>;<h>` | 7 个字段；设备出现顺序就是 `wire` 使用的索引。 |
| `port` | `port;<name>;<kind>;<ox>;<oy>` | 5 个字段，隶属上一条 `device`；`kind` 为 `in`/`out`/`bidi`（也接受 `input`/`output`/`bidirectional`）。 |
| `wire` | `wire;<fdev>;<fport>;<tdev>;<tport>` | 读完全部设备后校验索引、方向、自连与重复。 |

转义规则：字段以 `;` 分隔。文本字段（设备 id、设备名、端口名）内 `\` 是转义符：
`\\` → `\`、`\;` → `;`、`\n` → 换行、`\r` → 回车，因此名称可以包含空格与 `;`。
其它 `\x` 组合与结尾孤立的 `\` 都报错。空行与 `#` 开头的行被忽略。

错误处理：`CanvasParseError` 带 1 起始行号与原因（缺字段、坏数字/索引、未知 kind、
未知转义、未知记录、缺/重复头部、非法连线）。解析失败不会 panic，也不会部分替换
画布。`selected`/`drag`/`cursor` 属于临时状态，不写入文本。

The palette and the versioned text format live in dedicated modules so `canvas.rs`
stays a pure model; the format is an internal snapshot, not a stable interchange
format.

### Interaction / 交互

| Input / 输入 | Effect / 效果 |
|--------------|---------------|
| Left click a palette entry | Instantiates that template near the canvas centre, cascaded by 14 px per click. / 在画布中心附近实例化该模板，每次点击错开 14 px。 |
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
- 持久化只在内存中：没有文件对话框、路径配置或自动写盘，关闭应用即丢失快照。
- 调色板固定为 8 个内置模板，不能自定义或从插件加载；`instantiate` 生成的 id
  只保证进程内唯一。
- 没有自动布局、网格吸附、撤销/重做、主题系统。
- 没有芯片视图：连线只在设备端口之间建立，尚未连接到 RP2350 芯片引脚。
- The canvas is editor-only: it never drives the simulator. Persistence is an
  in-memory text snapshot (no file dialog, no autosave), the palette is a fixed
  built-in set, and there is no auto-layout, snapping, undo/redo or chip view.

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

调色板与持久化测试位于 `canvas_tests.rs`（`#[cfg(test)] mod canvas_tests`，独立模块
以免测试代码挤占 `canvas.rs`/`canvas_view.rs` 的行数预算）：

- 调色板条目数 ≥ 6、条目名称/尺寸/端口有效；实例化复制尺寸与端口；连续实例化 id
  形如 `palette-<index>-<serial>` 且互不相同；越界实例化与 `add_from_palette` 返回
  `None` 且不改变画布；
- `sample()` 画布与含调色板设备的画布 `to_text`→`from_text` 往返（设备、端口、连线、
  缩放、平移逐项相等），空画布同样往返；
- 名称含 `;` 与 `\` 的转义往返；
- 非法输入：缺字段、越界连线端点、未知端口方向、坏数字/索引、缺/重复/错误版本头部、
  未知记录、未知转义、方向不匹配连线——均返回带行号错误且不 panic；
- 调色板侧栏在 80×60、320×240、800×600 下的无窗口渲染；
- 应用层：保存→删除设备→加载后 `EmuApp` 的节点数与连线数恢复；加载失败时画布不变，
  状态行包含 `load failed`。

English: palette entry count/size/ports, unique instantiation ids, out-of-range
`None`, sample/palette/empty text round-trips (devices, ports, wires, zoom, pan),
`;`/`\` escaping, malformed-input errors (missing fields, bad indices/numbers,
unknown kind/record/escape, header problems, direction mismatch), headless
palette+canvas rendering at 80×60/320×240/800×600, and app-level save→mutate→load
round-trips plus failure handling.

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