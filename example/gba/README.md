# GBA 示例：pokeemerald-rp2350 / GBA example

本目录存放**示例项目专用**的内容：运行说明、外设/引脚配置、验证报告与工具。
通用的模拟器设计文档在仓库根目录的 `doc/`，示例相关材料不放那里。
This directory holds **example-specific** material: run instructions, peripheral
and pin configuration, verification reports and tools. Generic emulator design
documents live in the repository's `doc/`; example material does not go there.

## 目标固件 / Target firmware

| 项目 | 值 |
|------|-----|
| 固件 | `E:\Source\mcu\pico\rp2350\gba\pokeemerald-rp2350\rp2350\hw\build_emerald\emerald.uf2` |
| 大小 | 24 671 232 bytes |
| SHA-256 | `5BEE593F4AFAC6C9A1436BB14CA60B7861EBC2544044226DA2B73DBEE9A9B4C4` |
| 源码 | `E:\Source\mcu\pico\rp2350\gba\pokeemerald-rp2350`（独立仓库，本仓库只做模拟器侧） |
| 架构 | 2 × Cortex-M33（core0 游戏主循环 / core1 软件 PPU + HSTX） |

## 快速开始 / Quick start

```bash
# 启动 5 秒并打印诊断（双核、DMA、HSTX、core1）
rp2350-cli run --file emerald.uf2 --max-cycles 500000000 --trace-steps 0

# 抓一帧（240x160 RGB565）并转成 PNG
rp2350-cli run --file emerald.uf2 --max-cycles 1200000000 --trace-steps 0 \
    --dump 0x20063DF4:0x12C00:fb.bin
py example/gba/tools/fb2png.py fb.bin fb.png 3

# 通过 USB CDC 读取固件日志（stdin 必须保持打开：见下）
py example/gba/tools/hold_stdin.py 600 | rp2350-cli.exe repl --file emerald.uf2 --stdio --max-cycles 0
```

期望的 `run` 诊断（正常状态）：

```text
[run] dma=<millions> busy=0x0009
[run] hstx words=<millions> underruns=0 fifo=8 enabled=true wof=false unknown=0
[run] core1 state=Running pc=0x… launches=1
```

`repl --stdio` 的横幅在固件 `sleep_ms(1500)`（约 3.75 亿条模拟指令）之后出现：

```text
=== Pokemon Emerald on RP2350 ===
PPU + HSTX + core1 + I2S audio up (clk_sys 252000000 Hz); entering AgbMain()...
```

实测步数（双核与 PPU 全开；当前模拟器约 28–30 M 步/秒，见 `doc/performance.md`）：

| 阶段 | 步数 | 说明 |
|------|------|------|
| USB 枚举完成 | ~1.8 M | `USB CDC configured: IN=0x82/64 OUT=0x02/64 interface=1` |
| 启动横幅 | ~375 M | 固件 `sleep_ms(1500)` + `stdio_init_all` |
| 第一帧（Game Freak 标志） | ~1.2 G | `g_fb` 32 色、38 396/38 400 像素非零 |
| 稳定验收 | 2 G | `underruns=0 wof=false unknown=0`，`frame_done=527` |

## 游玩 / Playing

固件把每帧画到 SRAM 里的 `g_fb`（`0x20063DF4`，240×160 RGB565），模拟器按
「每帧固定步数」推进并抓这个缓冲区；按键通过 `GP0..GP9` 注入。

```bash
# 无 GUI：跑 20 帧，打印每帧与汇总（当前约 12 fps / 30 M steps/s）
rp2350-cli play --file emerald.uf2 --fb 0x20063DF4:240x160 \
    --steps-per-frame 2500000 --frames 20

# 带音频台（默认 null 后端；要真的出声需编译 cpal 后端）
cargo run -p rp2350-egui --features audio-playback --release
# 「游戏 / Play」标签页 → 填固件路径与帧缓冲 → Start
```

GUI 按键（画面获焦后生效）：

| 键 | GBA | GPIO | 键 | GBA | GPIO |
|----|-----|------|----|-----|------|
| `Z` | A | GP0 | `Enter` | Start | GP3 |
| `X` | B | GP1 | `S` | R | GP8 |
| `Backspace` | Select | GP2 | `A` | L | GP9 |
| 方向键 | 十字键 | GP4..GP7 | | | |

- 每游戏帧约 **2.5 M 步**（客户机以 60 fps 自我配速，300 M 步 = 2.0 s 客户机时间）。
  因此「可玩帧率」直接取决于模拟器吞吐：30 M 步/秒 ≈ 12 fps，60 fps 需 ≥150 M 步/秒。
  提升路线见 `doc/performance.md`。
- 固件是**忙等**实现的（SDK 的 `tight_loop_contents()` 是空函数），双核大量时间花在
  互相等待的短循环上；`WFI` 睡眠语义对该固件没有帮助。
- 音频采样率：固件 I2S 为 13440 Hz 立体声（PIO2 + DMA）；`rp2350-audio` 播放时按
  tap 的元数据与设备支持的采样率做最近邻重采样。

## 引脚配置 / Pin configuration

来自固件 `rp2350/hw/game_main.c`、`rp2350/hstx_display.c`、`rp2350/hw/i2s_audio.pio`
（本仓库不编译它，此处仅作模拟器侧参考）。

| GPIO | 功能 / Function | 说明 |
|------|-----------------|------|
| GP0..GP9 | GBA 按键（A、B、Select、Start、Right、Left、Up、Down、R、L） | 低有效，固件启用内部上拉；模拟器用 `gpio_pads::GpioPads` 注入 |
| GP12..GP19 | HSTX TMDS 输出（clk / D0 / D1 / D2 差分对） | `hstx_setup_lanes()`；模拟器只建模命令展开与像素节拍 |
| GP20..GP22 | I2S 音频（PCM5102A） | 由 PIO2 + `DMA_IRQ_1` 驱动 |
| USB | USB CDC stdio（`printf`） | 主机侧 `repl --stdio` 桥接 |

## 外设依赖 / Peripheral dependencies

| 外设 | 用途 | 模拟器模型 |
|------|------|------------|
| SIO FIFO | core0 → core1 `{0,0,1,vtor,sp,entry}` 启动握手 | `rp2350-periph::sio`（按核视图） |
| PSM `FRCE_OFF.PROC1` | core1 复位/释放 | `rp2350-periph::psm` |
| DMA ch0/1/2 | HSTX 控制块环（DATA / CB_COUNT / CB_ADDR），`DREQ_HSTX=52` | `rp2350-periph::dma`（RING + CHAIN_TO + 重载计数） |
| HSTX | 640×480p60 TMDS 扫描输出 | `rp2350-periph::hstx`（像素节拍 + 命令展开） |
| PIO2 + DMA | I2S 音频 TX | `rp2350-periph::pio`（DREQ + 引脚模型） |
| USBCTRL | CDC stdio | `rp2350-usb` + `rp2350-usb-host`（掩膜 ROM 视图） |
| GPIO pads | 按键上拉 | `rp2350-periph::gpio_pads` |
| BUSCTRL | DMA 总线优先级提示 | `rp2350-periph::busctrl` |

## 报告与工具 / Reports and tools

| 文件 | 内容 |
|------|------|
| [doc/gba-bringup-report.md](doc/gba-bringup-report.md) | 启动验证报告：缺陷根因、修复、验收数据、已知限制 |
| [doc/gba-frame-gamefreak.png](doc/gba-frame-gamefreak.png) | 1.2G 步时 HSTX 扫描输出的游戏帧 |
| [tools/fb2png.py](tools/fb2png.py) | 把 `--dump` 的 RGB565 帧缓冲转成放大 PNG（仅标准库） |
| [tools/hold_stdin.py](tools/hold_stdin.py) | 保持 `repl --stdio` 的 stdin 打开，直到固件打印日志 |
