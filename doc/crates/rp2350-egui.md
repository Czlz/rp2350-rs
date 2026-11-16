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
> `play_view/cli_source.rs` 接到真实 `PlaySession`（`main.rs` 默认挂载），
> 并支持命令行直启（`cli.rs`：`--profile` 或显式参数，首帧前自动 Start）。
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
> by `main.rs` by default. Command-line launch is available too (`cli.rs`:
> `--profile` or explicit flags, automatic Start before the first frame). The
> HSTX video decoder panel and the memory/GPIO/terminal/debugger panels below
> remain target designs.

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
    ├── play_view.rs      # 游戏标签页：帧纹理、缩放、键盘映射、工具栏（已落地 / landed）
    ├── play_view/types.rs # 帧/统计/请求/PlaySource/PlayStarter（已落地 / landed）
    ├── play_view/fields.rs # 工具栏文本字段 → 数值 + profile 载入（已落地 / landed）
    ├── play_view/cli_source.rs # PlaySession → PlaySource 适配器（已接线 / wired）
    ├── play_view/audio.rs # 宿主音频输出接线：AudioOutput 启动/状态行（rp2350-audio）
    ├── play_view/reset.rs # 启动/复位生命周期：字段 → PlayRequest、Reset 语义（已落地 / landed）
    ├── play_view/tests.rs # 游戏标签页测试（仅 test 构建 / test-only）
    ├── play_view/reset_tests.rs # 复位测试（仅 test 构建 / test-only）
    ├── play_view/params_tests.rs # 字段与 profile 测试（仅 test 构建 / test-only）
    ├── canvas_tests.rs   # 调色板/持久化测试（仅 test 构建 / test-only）
    ├── cli.rs            # 命令行直启：clap 参数 → PlayFields → 自动 Start（已落地 / landed）
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
    /// Pre-fill the Play toolbar fields / 预填「游戏」工具栏字段
    pub fn with_play_fields(self, fields: PlayFields) -> Self;
    /// Request an automatic start before the first frame / 请求首帧前自动启动
    pub fn with_play_auto_start(self, auto_start: bool) -> Self;
    /// Consume the request and start once (window creator) / 消费请求并启动一次（窗口回调）
    pub fn auto_start_play(&mut self) -> bool;
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
出来，并把键盘映射成按键位图。它是 GUI 中**唯一**会写仿真器的视图（写的是按键
掩码），也是唯一会驱动宿主音频的视图（`rp2350-audio`，见下「音频输出」）。

**没有示例常量**（通用性规则）：固件路径、帧缓冲、每帧步数、按键引脚与 I2S 的
PIO 块/状态机/采样率全部是工具栏输入框或 `key = value` profile 的内容，crate 内
不预填任何示例值。启动时所有输入框均为空（音频 sink 的 48 kHz/2 声道/48 000 样本
是**宿主**属性，因此作为通用默认值保留）。

The "Play" tab (`play_view.rs`) shows the RGB565 frames published by the
background play worker and maps the keyboard onto a button bitmap. It is the only
GUI view that writes to the simulator (a button mask) and the only one that
drives host audio (`rp2350-audio`, see "Audio output" below); every other panel
stays read-only.

There are **no example constants** (genericity rule): the firmware path, frame
buffer, steps per frame, button pins and the I2S PIO block / state machine /
rate come from toolbar text fields or a `key = value` profile. Every field starts
empty; only the host audio sink metadata (48 kHz, 2 channels, 48 000 samples)
keeps a generic default because it describes the host, not a firmware.

### Wiring model / 接线模型

依赖方向为 **`rp2350-egui → rp2350-cli`**：旧的 `rp2350-cli → rp2350-egui` 依赖已删除
（CLI 源码从未使用它，却造成包循环），因此本 crate 现在可以在
`play_view/cli_source.rs` 里提供真实适配器 `CliPlaySource` / `CliPlayStarter`。视图本身
仍只与两个 trait（`PlaySource` / `PlayStarter`）交互，测试与其它宿主依然可以注入假
worker。引用 CLI 的地方只有两处：`cli_source.rs`（启动 worker）与 `fields.rs`
（profile 文本与帧文本解析），因此测试无需真实会话也能覆盖除启动以外的全部逻辑。

The dependency points **`rp2350-egui → rp2350-cli`**: the stale
`rp2350-cli → rp2350-egui` edge (never used by CLI source, but a package cycle) is
gone, so this crate ships the real adapter `CliPlaySource` / `CliPlayStarter` in
`play_view/cli_source.rs`. The view still talks only to the two traits, so tests
and alternate hosts can keep injecting a fake worker. Only two modules reach into
the CLI crate: `cli_source.rs` (booting a worker) and `fields.rs` (profile and
frame text parsing), so tests can cover everything except booting without a real
session.

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
    fn audio_source(&self) -> Option<Box<dyn SampleSource>> {
        // `PlayConfig.audio` 为 `Some` 时把 AudioTap 交给 GUI 启动宿主输出
        self.session.audio_tap().map(|tap| Box::new(tap) as Box<dyn SampleSource>)
    }
    fn shutdown(self: Box<Self>) -> PlayStats { /* 先取 replaced，再 shutdown */ }
}

pub struct CliPlayStarter;

impl PlayStarter for CliPlayStarter {
    fn start(&self, request: PlayRequest) -> Result<Box<dyn PlaySource>, String> {
        let frame = FrameSpec::parse(request.frame.trim())?;
        // 接线逐字段搬运；`None` = 不捕获，不做任何示例猜测。
        // Copied field by field; `None` = no capture, with no example guess.
        let audio = request.audio.map(|wiring| PlayAudioConfig {
            block: wiring.block,
            sm: wiring.sm,
            rate_hz: wiring.rate_hz,
            channels: wiring.channels,
            capacity: wiring.capacity,
        });
        let config = PlayConfig {
            frame,
            steps_per_frame: request.steps_per_frame,
            max_frames: None,
            button_pins: request.button_pins,
            audio,
            trace_steps: 0,
        };
        let options = MachineOptions { arch: None, max_cycles: u64::MAX, strict_decode: false };
        PlaySession::start(request.image, options, config)
            .map(|session| Box::new(CliPlaySource::new(session)) as Box<dyn PlaySource>)
    }
}
```

`PlayRequest.audio` 是 `Option<AudioRequest>`，对应 `rp2350_cli::play::PlayConfig.audio`
（`None` 表示完全不捕获）；`AudioRequest { block, sm, rate_hz, channels, capacity }`
逐字段来自工具栏输入框或 profile，GUI 不提供任何示例默认值。
`PlayRequest.audio` is `Option<AudioRequest>` and maps onto
`rp2350_cli::play::PlayConfig.audio` (`None` = capture nothing); each field comes
from a toolbar input or a profile, so the GUI contributes no example defaults.

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
工具栏、占位画面与状态栏照常渲染，不会 panic。Start 的校验顺序是「字段 →
读固件 → starter」：字段缺失时状态行给出缺失项，字段齐全但没有 starter 时才写
`no play source attached / 未挂载游玩来源`。

A `PlayView::default()` without a starter still works (tests and `EmuApp::sample()`
rely on it): the toolbar, the placeholder picture and the status bar still render
and nothing panics. Start validates in the order "fields → firmware read →
starter", so a missing field is named first and `no play source attached` only
appears once the fields are complete.

### Command-line launch / 命令行直启

`crates/rp2350-egui/src/cli.rs`（clap derive）让原生入口在**任何窗口创建之前**解析
参数，并复用工具栏同一条字段/校验路径：

- `--profile <PATH>` 或显式 `--file/--fb/--steps-per-frame/...` 都会解析成
  `PlayFields`（与工具栏输入框完全同一套文本字段）；
- 合并优先级 **命令行 > profile > 空**（与 `rp2350-cli play` 一致），不引入任何示例
  默认值；`PlayFields::default()` 的音频 sink 元数据（48 kHz / 2 声道 / 48 000 样本）
  仍是唯一的通用默认；
- 合并后立即调用 `PlayFields::inputs()`（Start 使用的同一个函数）校验。缺
  `--file`/`--fb`/步数、`--audio` 缺 block/sm、坏 `--fb`、`--profile` 不可读或解析
  失败都在开窗前打印双语错误并以**退出码 2** 结束，不 panic、不闪现窗口；
- 参数合法时，`main.rs` 构造
  `EmuApp::sample().with_play_starter(CliPlayStarter).with_play_fields(fields).with_play_auto_start(true)`。
  `eframe` 的窗口创建回调在原生窗口存在之后、**首帧之前**调用
  `EmuApp::auto_start_play()`，等价于用户点「Load profile / 载入配置」+
  「Start / 启动」，并自动切到「游戏」标签页；
- 自动启动失败（固件不可读、starter 报错等）只写工具栏状态行，窗口保持打开；请求只
  消费一次，后续帧不会重复启动；
- 自动启动时向 **stderr** 打印一行稳定日志，便于脚本/验收判断：
  `[rp2350-egui] auto-start profile=<path>`（有 `--profile`）或
  `[rp2350-egui] auto-start file=<path> fb=<spec> steps_per_frame=<n>`（显式模式）；
- `--no-auto-start` 只预填字段、保持窗口手动操作（便于先检查参数）；
- 完全没有任何游玩选项时行为与以前**完全一致**：字段全空、默认「虚拟实验室」标签页、
  手动「Load profile → Start」。

| 选项 / Option | 值 / Value | 说明 / Notes |
|---------------|-----------|--------------|
| `--profile <PATH>` | 文件路径 | `key = value` profile；载入后自动开始 |
| `--file <PATH>` | 文件路径 | 固件（UF2/ELF）；无 profile 时必填 |
| `--fb <ADDR:WxH[:STRIDE]>` | 帧缓冲规格 | 由 `FrameSpec::parse` 校验；无 profile 时必填 |
| `--steps-per-frame <N>` | 正整数 | 每发布一帧执行的客户机步数 |
| `--buttons <MASK>` | `0x` 十六进制或十进制 | 按键引脚位图；默认不驱动任何引脚 |
| `--audio` | 开关 | 捕获 I2S；需要 `--audio-block`/`--audio-sm` |
| `--audio-block <N>` / `--audio-sm <N>` | usize | 推送 I2S 字的 PIO 块 / 状态机 |
| `--audio-rate <HZ>` | u32 | 音频 sink 采样率（通用默认 48 000） |
| `--audio-channels <N>` | u8 | 声道数（通用默认 2） |
| `--audio-capacity <N>` | usize | sink 保留样本数（通用默认 48 000） |
| `--no-auto-start` | 开关 | 只预填字段，不自动开始 |
| `--help` / `--version` | — | 由 clap 处理，退出码 0 |

```bash
# profile 直启（示例 profile 属于 example/，不是 crate 默认值）
cargo run -p rp2350-egui --release -- --profile example/gba/profile.conf

# 等价显式参数（不使用 profile）
cargo run -p rp2350-egui --release -- \
    --file firmware.uf2 --fb 0x20000000:240x160 --steps-per-frame 1000000 \
    --buttons 0x3FF --audio --audio-block 2 --audio-sm 0 --audio-rate 48000

# 只预填、先检查参数
cargo run -p rp2350-egui --release -- --profile example/gba/profile.conf --no-auto-start
```

English: `cli.rs` (clap derive) parses the command line before any window exists
and resolves it into the same `PlayFields` the toolbar edits. Precedence is
**command line > profile > empty** with no example defaults; the merged text is
validated through `PlayFields::inputs()` (the Start path), and any semantic error
prints a bilingual message and exits with code 2 before a window appears.
`EmuApp::with_play_fields` shows the resolved values, and the eframe window
creator calls `EmuApp::auto_start_play()` after the native window exists but
before the first frame, which is "Load profile + Start" without the clicks; it
also activates the Play tab and consumes the request once, so a failed start only
writes the status line and the window stays open. Auto-starts log one stable
stderr line (`[rp2350-egui] auto-start profile=…` or `… file=… fb=…
steps_per_frame=…`). `--no-auto-start` prefills without starting, `--help` /
`--version` behave normally, and no play option at all keeps the historical empty
fields plus manual flow.

### Toolbar and fields / 工具栏与字段

工具栏分四行输入 + 一行 Start/Stop/Reset + 一行状态。worker 运行中输入区整体置灰
（会话只在启动时读取参数），Start 与 Stop 仍可点击；`Reset / 复位` 只在有可重建的
会话时可用（运行中，或客户机请求 `AIRCR.SYSRESETREQ` 后停机，见下节）。

| 字段 / Field | 默认值 / Default | 说明 / Notes |
|--------------|------------------|--------------|
| `profile` | 空 | profile 文件路径；点 `Load profile / 载入配置` 载入（见下节） |
| `firmware` | 空 | 启动时 `std::fs::read`；读失败写状态行；未填时 Start 报 `missing firmware` |
| `frame` | 空 | `ADDR:WxH[:STRIDE]`，`FrameSpec::parse` 校验；未填时报 `missing frame buffer` |
| `steps_per_frame` | 空 | 正整数，否则状态行报 `steps per frame …` |
| `buttons` | 空 | 按键掩码，`0x` 十六进制或十进制；空 = 不驱动任何引脚（`0`） |
| `audio` | `false` | 启动前设定（运行中置灰）；`true` 时 worker 捕获 I2S，GUI 同时启动宿主输出 |
| `audio_block` | 空 | 推送 I2S 字的 PIO 块（`0..3`）；开启音频时必填 |
| `audio_sm` | 空 | 该块内的状态机（`0..4`）；开启音频时必填 |
| `audio_rate` | `48000` | 写入音频 sink 的采样率（宿主属性，故有通用默认） |
| `audio_channels` | `2` | 写入音频 sink 的声道数 |
| `audio_capacity` | `48000` | sink 保留的样本数（48 kHz 下 1 秒） |
| `status` | `idle / 空闲` | `running` / `stopped` / `reset` / `start failed` / `reset failed` / `profile loaded` / `profile failed` |

字段的解析与校验集中在 `play_view/fields.rs`（`PlayFields::inputs`）：

- 先查 `firmware`、`frame`（为空 → `missing firmware` / `missing frame buffer`，双语）；
- 再查 `steps_per_frame`（必须 > 0）、`frame` 几何（`FrameSpec::parse`）、`buttons`
  （`parse_button_mask`）；
- 最后在音频开启时解析 `audio_*`：块/状态机为空报 `missing audio PIO block`，
  非整数报 `bad audio … : expected an integer`，采样率/声道/容量为 0 报
  `… must be greater than zero`。

### Profile loading / profile 载入

`profile` 输入框填文件路径后点 `Load profile / 载入配置`，`play_view/fields.rs`
调用 `rp2350_cli::play::profile::parse`：

| 键 / Key | 填充字段 / Field |
|----------|------------------|
| `firmware` | `firmware`（路径原样显示） |
| `frame` | `frame`，重建为 `0xADDR:WxH[:STRIDE]`（紧凑 RGB565 时省略跨度） |
| `steps_per_frame` | `steps_per_frame` |
| `buttons` | `buttons`，重建为 `0xNNN` |
| `audio` | `audio` 复选框 |
| `audio_block` / `audio_sm` / `audio_rate` / `audio_channels` / `audio_capacity` | 对应的音频输入框 |

- 只覆盖 profile 给出的键，其余字段保持用户当前输入；重复载入得到相同文本。
- **解析失败不 panic、不启动**：状态行显示 `profile failed / 载入失败 — profile line N: …`
  （行号来自解析器），字段保持不变。
- 路径为空 → `profile path is empty / profile 路径为空`；读不到文件 →
  `cannot read profile / 无法读取 profile …`。
- 没有工具栏对应的键（`max_frames`、`arch`、`strict_decode`、`max_cycles`）被忽略：
  GUI worker 始终不设预算（`max_cycles = u64::MAX`），架构由映像格式决定（`arch: None`）。
- 示例 profile 见 `example/gba/profile.conf`（示例专用，不在 crate 内）。

Profile loading is a second, file-level CLI touchpoint next to `cli_source`; it is
the only way the GUI adopts guest-specific values other than typing them.

按键位（`GP0..GP9`，按下 = 低电平，固件内部上拉）：`buttons` 掩码决定哪些引脚被
驱动，键盘映射本身是固定的前端约定：

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

### 音频输出 / Audio output

勾选「Audio / 音频」并填好 `audio_block`/`audio_sm`（采样率/声道/容量有通用默认值）
后，GUI 会真的出声：`PlaySource::audio_source()` 把 worker 的 `AudioTap`（
`PlayConfig.audio = Some(..)` 时由 `PlaySession` 安装在该块/状态机上）装箱交出，
`PlayView` 用 `rp2350_audio::AudioOutput::start(tap, 96_000)` 启动一个有界的宿主音频
工作线程（容量 1 s 的样本）。音频线程、会话、复选框的生命周期是：

1. `start()`：状态行先写 `running`，再启动宿主输出；后端探测最多 1 s，返回后
   `backend()` 立即有效。
2. `stop()` / `reset()` / `Drop`：**先** `AudioOutput::shutdown()`（join 音频线程），**再**
   `PlaySession::shutdown()`，因此输出线程绝不会读取已释放会话的 tap；`reset()` 在
   新会话挂载后立刻重建输出（见下节「Reset / 复位」）。
3. 运行中全部输入框（含「Audio / 音频」复选框）置灰并显示
   `inputs locked / 参数已锁定`：音频按会话固定，`PlayView::set_audio()` 在运行中
   调用也会被忽略。

`audio = false`（不捕获）时 worker 完全不碰 PIO FIFO，`audio_source()` 返回
`None`，状态栏保持 `audio=off underruns=0`。

启用真实声卡输出（`cpal`）需要 feature 转发；本 crate 默认**不**打开它：

```bash
cargo build -p rp2350-egui --features audio-playback   # audio-playback = ["rp2350-audio/playback"]
cargo test  -p rp2350-egui --features audio-playback
cargo run   -p rp2350-egui --features audio-playback   # 真实声卡输出
```

没有该 feature、或运行时没有可用输出设备时，`rp2350-audio` 走 **null 后端**：仍按
墙钟消费样本、统计 underrun，但完全静默；`start()` 不会失败也不会 panic。此时状态栏
显示 `audio=null`，状态行追加一次 `· 无声卡，静默运行 / no sound card, silent`
（只追加一次，可用 `PlayView::status()` 断言）。

With `Audio / 音频` ticked and `audio_block`/`audio_sm` filled in (rate, channels and
capacity keep generic sink defaults) the GUI really plays sound:
`PlaySource::audio_source()` hands out the worker's `AudioTap` (installed on that
block/state machine when `PlayConfig.audio = Some(..)`) and `PlayView` starts a
bounded host worker with `rp2350_audio::AudioOutput::start(tap, 96_000)`. The audio
thread is always joined **before** the session (`stop()`, `reset()` and `Drop`
alike), and `reset()` re-creates it for the new session; every input (the
checkbox included) is greyed out while a session runs. With
`audio = false` the worker never touches a PIO FIFO, `audio_source()` returns
`None` and the status bar stays `audio=off underruns=0`. Real `cpal` output is
opt-in through `--features audio-playback`; without the feature or without a
device the null backend consumes samples silently, `start()` still succeeds, the
status bar shows `audio=null` and the one-time notice
`无声卡，静默运行 / no sound card, silent` is appended to the status line.

### Reset / 复位

`Reset / 复位` 是**宿主侧重启**：用当前工具栏字段（含 profile 载入后的值）重建整个
会话，语义等价于「Stop + Start」，但一次点击完成且保留字段。按钮可用性由
`PlayView::can_reset()` 决定：

- **运行中可用**；
- **客户机请求 `AIRCR.SYSRESETREQ` 后仍可用**：`play` worker 检测到该请求后以
  `halted=system reset requested (AIRCR.SYSRESETREQ)` 停止（见
  [`doc/crates/rp2350-cli.md`](rp2350-cli.md)），状态栏显示 `halted / 停机: …`；
  这是需要宿主动作的**请求**而不是终态，因此 GUI 允许用户按当前字段重建（与 `run`
  自动重启 3 次的策略不同，GUI 不自动重启、也没有次数上限，完全由用户触发）；
- **空闲或正常 Stop 后置灰**：没有可重建的会话，`reset()` 只写
  `reset unavailable / 无法复位：没有可重建的会话`。

点击后的固定顺序（`play_view/reset.rs` 的 `PlayView::reset`）：

1. 校验字段并重读固件（`PlayFields::inputs` → `std::fs::read`）；失败只写状态行，
   **不杀正在运行的会话**（笔误不该中断游戏）；
2. `AudioOutput::shutdown()`（join 音频线程）；
3. 旧 `PlaySource::shutdown()`（`PlaySession` 在 shutdown/drop 时 join worker 线程）；
4. `PlayStarter::start(request)` 以相同字段启动新 worker；失败写
   `reset failed / 复位失败: <原因>`，此时旧会话已停止、不留下半启动状态；
5. 成功：`reset count` 递增，状态行
   `reset / 已复位 · reset count=N · <帧缓冲> · steps/frame …`，随后 `attach()` 清零
   帧序号/统计/纹理并从**第 1 帧**重新开始，勾选音频时重新创建 `AudioOutput`
   （新 sink、underrun 计数从 0 开始）。

边界语义：

- **不重置输入框**：字段、profile 路径与音频接线与 Stop/Start 一样保留；运行中输入区
  仍然置灰，因此正常操作下重建用的就是启动时那套参数。
- **不保存状态**：复位不是存档。客户机 RAM、外设寄存器、时钟树与 PIO 状态全部从
  固件映像冷重建，进度、按钮按下状态（掩码清零）与音频队列都会丢失，无法从复位点
  恢复；它也不是给客户机的软复位线，只是宿主侧重启。
- 状态栏的 `frame=` 来自新会话，`fps`/`steps` 在新帧到达后即切换为新会话的统计。

`Reset / 复位` is a **host-side restart** from the current toolbar fields, the
same as "Stop + Start" in one click. It is enabled while a worker runs and while
a worker stopped on a guest `AIRCR.SYSRESETREQ` (that halt is a request for host
action), and greyed out on an idle view or after a normal Stop. The order is
fixed: validate fields and re-read the firmware (a validation error leaves the
running session alive), shut down the host audio output, shut down and join the
old worker, then boot a new one with the same fields. Success increments
`reset count`, writes `reset / 已复位 · reset count=N · …`, resets the frame
counter to 1 and re-creates the audio output; a starter error writes
`reset failed / 复位失败: …` with no session left behind. Reset never touches
the text fields, and it is **not** a save state: guest RAM, peripherals, clocks
and PIO state are rebuilt from the image, so progress and the button mask are
lost and nothing can resume from the reset point.

### Status bar / 状态栏

```
frame=<index> steps=<..> fps=<..> steps/s=<..> replaced=<..> · running|stopped
buttons / 按键: A|Right
audio=<backend> underruns=<n>
```

`audio=` 是宿主输出后端（`cpal` / `null`），无输出时为 `audio=off underruns=0`；
`underruns` 每帧从输出线程重读，持续增长说明客户机产样本慢于播放节奏。

`halted` 存在时追加 ` · halted / 停机: <原因>`（例如指令预算耗尽、非法指令）。
`replaced` 是「未被 GUI 取走即被替换」的帧数；它持续增长说明 UI 或渲染跟不上 worker
（当前 worker 约 8 fps）。

### 帧率说明 / Frame rate

worker 每帧执行 `steps_per_frame` 步后发布一帧，因此帧率 ≈ `steps/s / steps_per_frame`。
用 `example/gba/profile.conf`（每帧 2.5 M 步）实测约 **8 fps**（20.6 M steps/s），
远低于 GBA 的 60 fps；画面能看，但动作偏慢。提高办法见
[`doc/playability-plan.md`](../playability-plan.md) 的 P0（解释器热路径），或降低
`steps_per_frame`（画面更快但游戏内时间更慢）。

The worker publishes one frame per `steps_per_frame` guest steps, so the frame
rate is ≈ `steps/s / steps_per_frame`. With the values of
`example/gba/profile.conf` (2.5 M steps/frame) the measured rate is about
**8 fps** (20.6 M steps/s), far below the GBA's 60 fps: the picture is readable
but slow. See the P0 rows of the playability plan for the way forward.

### Limits / 限制

- **没有示例默认值**：固件路径、帧缓冲、每帧步数、按键掩码与 I2S 接线都必须由用户
  输入或从 profile 载入；未填 `firmware`/`frame`/`steps_per_frame` 时 Start 只写
  状态行，不会启动也不会 panic。
- **profile 只覆盖有输入框的键**：`max_frames`、`arch`、`strict_decode`、`max_cycles`
  被忽略（GUI 始终不设预算、由映像选核）。
- **音频 sink 元数据是宿主默认值**：`audio_rate`/`audio_channels`/`audio_capacity`
  默认 48 kHz/2 声道/48 000 样本，可改；实际重采样与设备格式仍由 `rp2350-audio`
  决定（最近邻重采样），GUI 不提供音量、静音或设备选择。
- **无存档**：不能保存进度；`Reset / 复位` 只是按当前字段的宿主侧重启（见上节），
  不是存档恢复，复位后无法回到之前的状态。
- **无手柄 / 无按键重绑**：键盘→GP 的映射是固定前端约定（下表），但 `buttons` 掩码
  决定哪些引脚真的被驱动，因此换固件时改掩码即可。
- **无暂停 / 单步 / 截图**：只有 Start、Stop 与 Reset；帧率与步数由 `steps_per_frame` 决定。
- **命令行直启同样不做示例猜测**：`--profile` / 显式参数都由用户提供，缺字段或
  profile 非法时在开窗前以退出码 2 结束；`--no-auto-start` 只预填字段。
- 依赖宿主挂载 `PlayStarter`：`PlayView::default()` 仍无 starter（便于测试注入假实现），
  但原生入口 `main.rs` 已默认挂载 `CliPlayStarter`，所以 `cargo run -p rp2350-egui`
  的「游戏」标签页可直接出画面（仍需填入固件与帧缓冲，或使用命令行直启）。
- There are no example defaults: firmware path, frame buffer, steps per frame,
  button mask and I2S wiring all come from user input or a profile, and Start only
  writes the status line while a required field is missing. Profile keys without a
  toolbar field (`max_frames`, `arch`, `strict_decode`, `max_cycles`) are ignored.
  The sink metadata (48 kHz, 2 channels, 48 000 samples) is a host default and
  editable, while resampling and device format stay with `rp2350-audio` (nearest
  neighbour), with no volume, mute or device selection. Keyboard→GP mapping is a
  fixed frontend convention, but the `buttons` mask decides which pins are really
  driven. The starter is still injectable (`PlayView::default()` has none, for
  tests and alternate hosts), and the native `main.rs` attaches `CliPlayStarter`.
  Command-line launch follows the same rule: the profile and flags are supplied by
  the user, a missing/invalid request exits with code 2 before the window, and
  `--no-auto-start` only prefills. There are still no save states, no gamepad, no
  pause/step/screenshot; `Reset` rebuilds the session from the current fields but
  cannot resume prior progress.

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
  `Err` 都只写状态行；成功路径把 `PlayRequest`（映像、帧缓冲、步数、音频接线、引脚）
  交给 starter，`stop()` 调用一次 `shutdown`；
- **真实适配器**（`CliPlaySource` / `CliPlayStarter`，用合成 UF2 启动真正的
  `PlaySession`）：首帧发布后 `latest_frame()` 有值、`button_pins() == 0`（
  `PlayConfig::new` 不驱动任何引脚）、`shutdown()` 后 `is_running() == false`；
  `set_buttons` 透传（读回一致）；统计中 `frames`/`steps` 单调递增且
  `steps_per_second > 0`；同一帧重复轮询返回同一个 `Arc`（像素不重复拷贝）；
  非法帧文本 / 未知映像 / 零步数返回 `Err` 而不 panic；
- 端到端：`PlayView` 挂载 `CliPlayStarter` + 临时 UF2 文件启动后，无头渲染循环
  能上传至少一帧，`stop()` 后状态为 `stopped / 已停止`；
- 应用层：`ActiveView::Play` 在 760×600 与 80×60 下渲染，切到其它标签再切回后
  状态保持；
- **宿主音频**（`play_view/audio.rs`）：勾选音频且有 tap 时创建 `AudioOutput` 且
  `backend()` ∈ {`cpal`, `null`}、`playing() == true`；未勾选时不创建输出且
  `underruns() == 0`；`stop()` 在 2 s 内 join 音频线程（来源的 drop 标志置位）、
  输出句柄清空；`format_audio` 的 `audio=<backend> underruns=<n>` / `audio=off`
  格式；null 后端提示只追加一次（`cpal` 不提示）；`Drop` 同样 join 音频线程。

`play_view/params_tests.rs`（字段与 profile，`#[cfg(test)] mod params_tests`）：

- **无示例常量**：`PlayFields::default()` 的 `profile`/`firmware`/`frame`/
  `steps_per_frame`/`buttons`/`audio_block`/`audio_sm` 全为空、音频关闭；只有 sink
  元数据是通用默认值（`48000`/`2`/`48000`）；`PlayView::default()` 的输入框同样为空
  （防止示例常量回归）；
- 缺 `firmware` → `missing firmware`，缺 `frame` → `missing frame buffer`，两者都
  **不启动**；非法帧文本 → `帧缓冲非法`；
- `buttons` 空 = 0、`0x3FF` 与十进制 `63` 都被接受、`zzz` 报 `按键掩码`；
- `audio=false` → `PlayRequest.audio == None`（即使块/状态机已填）；
  `audio=true` → `Some(AudioRequest{block,sm,rate_hz,channels,capacity})` 且字段逐项
  等于输入框；
- 音频开启但字段缺失/非法 → `missing audio PIO block`、`bad audio PIO block`、
  `greater than zero`，且不启动、不调用 starter；
- `example/gba/profile.conf`（相对路径，文件不存在则跳过）载入后每个字段与
  `rp2350_cli::play::profile::parse` 的结果逐项一致；
- 非法 profile → 状态含 `profile line 3`，字段不变，`start()` 仍被拒绝；空路径 →
  `profile path is empty`；
- `format_frame` 对紧凑与带跨度规格往返稳定；
- **真实 starter 的音频接线**：`audio: None` → `audio_source()` 为 `None`（未安装
  tap）；`Some{rate 13440, channels 2}` → tap 的 `format()` 报告同样的采样率与声道；
  越界 PIO 块（`9`）被 `PlaySession::start` 拒绝。

English: the field/profile suite pins the genericity rule (every text field starts
empty; only host sink metadata has a generic default), the missing
firmware/frame rejections, the button mask parser, `audio = None` vs
`Some(AudioRequest)` handover (observed through the recorded `PlayRequest` and,
for the real starter, through `audio_source()`/`AudioFormat`), the audio field
validation messages, the `example/gba/profile.conf` round-trip which is skipped
when the file is absent, the `profile line N` rejection that leaves fields
untouched, `format_frame` round-trips, and the out-of-range PIO block rejection.

`play_view/reset_tests.rs`（复位按钮，`#[cfg(test)] mod reset_tests`）：

- **可用性**：空闲视图 `can_reset() == false`（Reset 只写 `reset unavailable`，不调用
  starter）；运行中为 `true`；客户机 `SYSRESETREQ` 停机后仍为 `true`；正常 Stop 后回到
  `false`；并断言 GUI 匹配前缀 `system reset requested` 与
  `rp2350_cli::run::SYSTEM_RESET_MESSAGE` 不漂移；
- **重建**：Reset 让 starter 被调用两次（旧会话 shutdown 一次 + 新会话 start），两次
  `PlayRequest` 完全相等（映像、帧缓冲、步数、按键掩码、音频接线），文本字段不变；
- **新会话计数**：`reset count` 从 0 → 1 → 2，状态行含 `reset / 已复位` 与
  `reset count=N`；Reset 后 `last_frame == 0`、统计清零、纹理释放，新会话重新发布
  `index=1` 的帧后状态行为 `frame=1 steps=200 fps=60.00`；
- **音频生命周期**：勾选音频启动后旧 `AudioOutput` 的 `SilentSource` drop 标志为真
  （旧线程已 join），Reset 后创建第二个输出且仍在播放，Stop 再 join 第二个；
- **失败路径**：starter 第二次返回 `Err` → 状态行 `reset failed / 复位失败: machine
  boot failed`、`is_running() == false`、旧会话 shutdown 一次、`reset count` 不增、
  再次 Reset 只写 `reset unavailable`，不 panic；
- **80×60 无头渲染**：整页在 80×60 下渲染两遍不 panic；再单独渲染
  Start/Stop/Reset 行，用真实指针按下/抬起点击第三个控件（Reset），`reset count`
  变为 1 且新会话运行。

English: the reset suite pins availability (idle, running, system-reset halt, normal
stop, plus the `SYSTEM_RESET_MESSAGE` prefix drift check), the two starter calls
with an identical `PlayRequest` and untouched fields, the monotonic `reset count`
with frame numbering restarting at 1 and the new session's `fps`/`steps` in the
status bar, the old audio output being joined before the new one is created, the
starter-error path (old session stopped, no panic, no count), and a headless
80×60 render plus a real pointer click on the Reset control.

`cli.rs`（命令行直启，`#[cfg(test)] mod tests`，11 项）：

- **profile-only**：`--profile <临时 profile>` 的每个字段与 profile 一致、
  `auto_start == true`、日志行 `auto-start profile=<path>`；
- **显式模式**：`--file/--fb/--steps-per-frame/--buttons/--audio/--audio-*` 全量填充，
  日志行 `auto-start file=… fb=… steps_per_frame=…`；
- **混合优先级**：命令行覆盖 profile 的 firmware/frame/steps/buttons/audio-rate，
  profile 继续提供未覆盖的 audio_block/sm/channels；
- **`--no-auto-start`**：字段预填、`auto_start == false`、没有日志行；
- **拒绝路径**（开窗前错误）：缺 `--fb`、`--audio` 缺 block、`--audio` 缺 sm、
  坏 `--fb`、`--profile` 不存在，错误消息分别点名缺失项/`帧缓冲非法`/`cannot read profile`；
- **`--help` / `--version`**：clap `ErrorKind::DisplayHelp` / `DisplayVersion` 且退出码 0；
- **无参数**：解析为 `None`，`PlayFields` 与 `EmuApp::sample().play().fields()` 仍全空
  （防止示例常量回归）；单独 `--no-auto-start` 也不构成启动；
- **示例 profile 逐项一致**：`example/gba/profile.conf` 经 CLI 解析后的每个字段与
  `rp2350_cli::play::profile::parse` 的结果逐项相等（文件不存在则跳过）。

`app.rs` 追加 3 项自动启动测试：假 starter 在 `auto_start = true` 时被调用**一次**
（再次调用 `auto_start_play()` 不再启动）、`auto_start = false` 时一次都不调用且保持
虚拟实验室标签页、自动启动后 80×60 与 760×600 无头渲染不 panic。

English: `cli.rs` covers profile-only and explicit launches (fields plus the exact
stderr log line), CLI-over-profile precedence, `--no-auto-start`, every rejection
path (missing `--fb`, audio without block/state machine, malformed `--fb`, missing
profile), `--help`/`--version` exit code 0, the no-argument case still yielding
empty fields, and a field-by-field comparison against the parsed
`example/gba/profile.conf`. Three `app.rs` tests pin the automatic start: the fake
starter is called exactly once when requested (and not again on a second call),
never when the flag is false (the view stays on Virtual Lab), and the auto-started
view renders headlessly at 80×60 and 760×600 without panicking.

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