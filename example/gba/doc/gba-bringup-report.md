# GBA bring-up report / GBA 启动验证报告

> 本文件属于**示例项目**材料（`example/gba/`）；通用的模拟器设计文档在仓库根
> `doc/`。示例的运行命令、引脚表与外设依赖见 [../README.md](../README.md)。
> This document belongs to the **example project** (`example/gba/`); generic
> emulator design documents live in the repository `doc/`. Run instructions, the
> pin map and peripheral dependencies are in [../README.md](../README.md).

Target: `pokeemerald-rp2350` (Pokémon Emerald on RP2350) driven by this
emulator instead of real hardware. The goal was to boot the shipped firmware,
get a real game picture into the HSTX scanout ring, and keep it running without
crashes.
目标：用本模拟器代替真实硬件运行 `pokeemerald-rp2350`（RP2350 上的 Pokémon
Emerald），要求固件成功启动、HSTX 扫描输出真实游戏画面，并稳定运行不崩溃。

## Firmware under test / 被测固件

| Item / 项目 | Value / 值 |
|-------------|------------|
| Path | `E:\Source\mcu\pico\rp2350\gba\pokeemerald-rp2350\rp2350\hw\build_emerald\emerald.uf2` |
| Size | 24 671 232 bytes |
| SHA-256 | `5BEE593F4AFAC6C9A1436BB14CA60B7861EBC2544044226DA2B73DBEE9A9B4C4` |
| Built | 2026-11-14 23:37 |
| Emulated cores | 2 × Cortex-M33 @ 150 MHz model timebase, firmware raises `clk_sys` to 252 MHz |

The firmware is a complete GBA emulator: core 0 runs `AgbMain()` (the game
superloop, paced to 59.73 Hz) and core 1 runs `core1_render` (software PPU into
`g_fb`, HSTX scanout). Reproducing it therefore exercises most of the RP2350:
dual-core launch, PIO, DMA control-block rings, HSTX, USB CDC stdio, GPIO pads,
clocks/resets and flash XIP.
该固件本身是一个完整的 GBA 模拟器：core0 运行 `AgbMain()`（游戏主循环，按
59.73 Hz 节拍），core1 运行 `core1_render`（软件 PPU 渲染到 `g_fb`，HSTX 扫描
输出）。因此复现它几乎覆盖了 RP2350 的全部子系统：双核启动、PIO、DMA 控制块环、
HSTX、USB CDC stdio、GPIO pad、时钟/复位与 Flash XIP。

## Defects found and fixed / 发现并修复的缺陷

1. **`SCB_VTOR` was zero in the PPB window.** The core cached the boot vector
   base but the PPB register file still read 0, so the Pico SDK
   `runtime_init_install_ram_vector_table` copied an empty table from address 0
   and every later `irq_set_exclusive_handler` tripped `hard_assert` → `panic()`
   → `_exit` (`BKPT` spin). The CLI now seeds `0xE000_ED08` with the flash
   vector base right after boot. Fixed in `95d7719`.
   核心缓存了 VTOR，但 PPB 寄存器文件仍读 0，Pico SDK 的
   `runtime_init_install_ram_vector_table` 从地址 0 复制了空向量表，随后
   `irq_set_exclusive_handler` 触发 `hard_assert` → `panic()` → `_exit`
   （`BKPT` 自循环）。CLI 现在在启动后把 `0xE000_ED08` 写成 Flash 向量基址。
2. **PIO/DMA atomic alias windows were unmapped.** `pio_sm_clkdiv_restart`
   (`0x5040_10D0`, XOR alias) and `dma_hw->inte1` (`0x5000_2414`, SET alias)
   produced `NoDevice` bus errors. A shared `alias` module folds SET/CLR/XOR
   writes onto the 4 KB register block; both windows are 16 KB now.
   `pio_sm_clkdiv_restart`（`0x5040_10D0`，XOR 别名）与 `dma_hw->inte1`
   （`0x5000_2414`，SET 别名）触发 `NoDevice` 总线错误。新增共享 `alias` 模块把
   SET/CLR/XOR 写折叠到 4 KB 寄存器块；两个窗口现在都是 16 KB。
3. **Core 1 never executed.** HSTX and the PPU live in `core1_render`, so the
   display stayed dead even though core 0 reached `AgbMain`. `Machine` now owns
   a real second `CortexM33Core`, plays the resident core-1 boot ROM side of the
   `{0, 0, 1, vtor, sp, entry}` SIO handshake, honours PSM `FRCE_OFF.PROC1`
   reset/release, replays PPB system writes per core, routes interrupts to each
   NVIC and steps both cores per instruction.
   HSTX 与 PPU 都在 `core1_render` 中，因此 core0 进入 `AgbMain` 后显示仍是死的。
   `Machine` 现在持有真正的第二个 `CortexM33Core`，扮演常驻 core1 boot ROM 完成
   `{0, 0, 1, vtor, sp, entry}` 的 SIO 握手，遵守 PSM `FRCE_OFF.PROC1` 的复位/
   释放，按核回放 PPB 系统写，把中断分别投递到两个 NVIC，并在每条指令上推进
   两个核。
4. **DMA control-block ring stalled after one or two entries.** `TRANS_COUNT` was
   used as both the reload and the working count, so the CB channels (count = 1)
   re-triggered as zero-length transfers. Splitting reload/working counts, making
   `AL1/AL2/AL3_CTRL` non-triggering and tightening the `*_TRIG` aliases makes
   the 3-channel ring stream indefinitely (a faithful test now runs three laps of
   42 912 words).
   `TRANS_COUNT` 同时充当重载值与工作计数，导致 CB 通道（count = 1）以零长度
   传输重新触发。拆分重载/工作计数、让 `AL1/AL2/AL3_CTRL` 不触发、收紧
   `*_TRIG` 别名语义后，3 通道环可无限流式运行（忠实测试跑满三圈 42 912 字）。
5. **HSTX FIFO pacing was 20× too fast.** The expander popped one FIFO word per
   `clk_sys` cycle, so a 64-cycle pump drained the 8-word FIFO and dropped the
   rest (`WOF`), losing data words and desynchronising the command/data phase
   (`unknown command opcode 0x5`) with ~4.1 M starvation episodes. It is now
   paced by the pixel clock (`2 × CSR.CLKDIV` `clk_sys` per pixel, per-command
   word timing), so a 256-word scanline block lasts exactly 800 pixel periods and
   the DMA ring keeps the FIFO fed: **0 underruns**, no dropped words.
   扩展器按每 `clk_sys` 周期一字弹出，64 周期的一批就能排空 8 字 FIFO 并丢弃
   其余字（`WOF`），从而丢数据字、命令/数据相位错位（`unknown command opcode
   0x5`）并产生约 410 万次断流。现在按像素时钟节拍（每像素 `2 × CSR.CLKDIV`
   个 `clk_sys`，按命令计时），256 字扫描行块正好占 800 个像素周期，DMA 环能
   持续补字：**0 次断流**，无丢字。
6. **GPIO inputs read as "all buttons held".** The GBA keys are active-low and the
   firmware pulls them up, but the pin matrix was not connected, so `gpio_get_all`
   returned 0 and the game saw A+B+Start+Select held, entering the RFU soft-reset
   path (`AgbRFU_SoftReset`) and never drawing. `gpio_pads::GpioPads` now models
   pull-up/pull-down/host drive between `PADS_BANK0` and `SIO_GPIO_IN`.
   GBA 按键低有效且固件启用了上拉，但引脚矩阵未连接，`gpio_get_all` 返回 0，
   游戏误判 A+B+Start+Select 同时按下而进入 RFU 软复位（`AgbRFU_SoftReset`），
   永不绘制。`gpio_pads::GpioPads` 现在建模 `PADS_BANK0` 与 `SIO_GPIO_IN` 之间的
   上拉/下拉/宿主驱动。
7. **USB CDC enumeration faulted at address 0.** TinyUSB writes
   `endpoint_control` (NULL for EP0) unconditionally; on silicon `0x0..0x7FFF` is
   mask ROM that ignores writes, but the emulator's ROM region rejected them and
   the first SETUP data stage died with `Bus error at 0x0`. `repl` installs a
   read-through/write-drop mask-ROM view before enumeration, so the firmware's
   stdio banner is now readable.
   TinyUSB 会无条件写 `endpoint_control`（EP0 为 NULL）；芯片上 `0x0..0x7FFF`
   是忽略写入的掩膜 ROM，而模拟器的 ROM 区域拒绝写入，首次 SETUP 数据阶段即以
   `Bus error at 0x0` 失败。`repl` 在枚举前安装读透传/写丢弃的掩膜 ROM 视图，
   现在能读到固件 stdio 横幅。
8. **`TIMER` register map and `IPSR`.** `sleep_ms` only forces the timer IRQ
   (`hw_set_bits(&TIMER0->intf, 1<<3)`) and lets `alarm_pool_irq_handler` program
   the hardware alarm. The model had the RP2040-era interrupt offsets
   (`0x38/0x3C/0x40/0x44`), treated `INTF` as write-one-to-clear instead of a
   force mask, never auto-disarmed a fired alarm and had no `TIMER1`; on top of
   that `enter_exception` never wrote `xPSR[8:0]`, so `MRS IPSR` read 0 and the
   handler picked the wrong timer/alarm and crashed at `0x00757C08`. All are
   fixed (see `doc/changelog.md`), and emerald boots through `sleep_ms` again.
   `sleep_ms` 只强制定时器中断（`hw_set_bits(&TIMER0->intf, 1<<3)`），再由
   `alarm_pool_irq_handler` 编程硬件 alarm。模型原本用 RP2040 时代的中断偏移
   （`0x38/0x3C/0x40/0x44`）、把 `INTF` 当作 W1C 而非强制位、alarm 到期不自动
   解除武装，也没有 `TIMER1`；此外 `enter_exception` 从不写 `xPSR[8:0]`，导致
   `MRS IPSR` 读到 0，处理程序选错 timer/alarm 并在 `0x00757C08` 崩溃。以上均已
   修复（见 `doc/changelog.md`），emerald 重新能穿过 `sleep_ms`。
9. **Smaller gaps.** `MSPLIM`/`PSPLIM` (`msr msplim, r0` in the SDK crt0) were
   undefined instructions; `BUSCTRL` (`0x4006_8000`) was unmapped and warned on
   every boot. Both are modelled now.
   `MSPLIM`/`PSPLIM`（SDK crt0 的 `msr msplim, r0`）曾是未定义指令；
   `BUSCTRL`（`0x4006_8000`）未映射，每次启动都产生警告。两者现已建模。

## Acceptance evidence / 验收证据

### Boot and stdio (USB CDC, `repl --stdio`) / 启动与 stdio

```text
USB CDC configured: IN=0x82/64 OUT=0x02/64 interface=1
=== Pokemon Emerald on RP2350 ===
PPU + HSTX + core1 + I2S audio up (clk_sys 252000000 Hz); entering AgbMain()...
```

`repl --file emerald.uf2 --stdio --max-cycles 0` reaches the game loop and stays
up. The banner appears only after the firmware's `sleep_ms(1500)`, i.e. ~375 M
emulated steps, so stdin must stay open for several minutes of wall-clock time
(the emulator runs this firmware at ~1.5–2 M steps/s because both cores and the
PPU are active). No `panic`, hard fault or bus error occurs; the HSTX
`unknown command` / `WOF` warnings that appeared before the pacing fix are gone.
`repl --file emerald.uf2 --stdio --max-cycles 0` 能进入游戏主循环并保持运行。
横幅要等固件的 `sleep_ms(1500)` 之后才出现，即约 3.75 亿条模拟指令，因此 stdin
需要保持打开数分钟（双核与 PPU 都在运行时本模拟器约 1.5–2 M 步/秒）。整个过程
未出现 `panic`、硬故障或总线错误；节拍修复前出现的 HSTX `unknown command` /
`WOF` 告警已消失。

### Scanout counters / 扫描输出计数

```text
[run] executed 1200000000 steps, 8000000 us simulated, final PC=0x10000516
[run] dma=28752244 busy=0x000A
[run] hstx words=70514080 underruns=0 fifo=8 enabled=true wof=false unknown=0
[run] core1 state=Running pc=0x10001B46 sp=0x2003D540 launches=1
```

Before the pacing fix the same firmware reported `underruns=4133282`,
`unknown=5`, `wof=true`, and the DMA ring stopped after 256 transfers. Now the
ring keeps running (`dma` in the tens of millions, `ch0` busy with `TREQ_HSTX`),
the HSTX expander never starves, no word is ever dropped and command/data phase
never desynchronises.
节拍修复前同一固件报告 `underruns=4133282`、`unknown=5`、`wof=true`，DMA 环在
256 次传输后停摆。现在环持续运行（`dma` 数千万次，`ch0` 以 `TREQ_HSTX` 忙碌），
HSTX 扩展器不再断流，从未丢字，命令/数据相位也不再错位。

The last two defects were found *because* the counters improved: once pacing was
correct, a single `WOF` remained. The new one-shot diagnostic named the dropped
word (`0x354D52AB`) and the following `unknown opcode 0x5` on the same word
proved the overrun→desync chain. The cause was `pump_dma` choosing a whole-batch
transfer while a channel was enabled but not yet `BUSY`, so the FIFO room limit
was not applied; the batch budget is now capped at the HSTX FIFO's free space,
which makes an overrun impossible without per-unit locking.
最后两个缺陷正是靠计数改善才暴露出来的：节拍正确后仍残留一次 `WOF`。新增的
一次性诊断给出了被丢弃的字（`0x354D52AB`），紧随其后的同一字报
`unknown opcode 0x5`，证明「溢出→失步」链条。根因是 `pump_dma` 在通道已使能但
尚未置 `BUSY` 时选择了整批传输，从而没有应用 FIFO 余量上限；现在批次预算被限制
为 HSTX FIFO 的空闲空间，无需逐单元加锁即可让溢出不可能发生。

### Game picture / 游戏画面

`g_fb` (`0x20063DF4`, 240×160 RGB565 = 76 800 bytes) is not blank: 38 396 of
38 400 pixels are non-zero with 32 distinct RGB565 colours. Dumping it with
`run --dump 0x20063DF4:0x12C00:fb.bin` after 1.2 G steps and converting to PNG
shows the Game Freak logo of the retail intro, i.e. the PPU output that the HSTX
ring is scanning out. At 2 G steps the same scene is mid-fade (19 colours,
`frame_done=527`), so the game loop keeps producing new frames.
`g_fb`（`0x20063DF4`，240×160 RGB565 = 76 800 字节）非空：38 400 个像素中
38 396 个非零，共 32 种 RGB565 颜色。运行 1.2 G 步后用
`run --dump 0x20063DF4:0x12C00:fb.bin` 导出并转 PNG，可以看到零售版开场的
Game Freak 标志，即 HSTX 环正在扫描输出的 PPU 结果。2 G 步时同一场景正在淡出
（19 色，`frame_done=527`），说明游戏主循环仍在持续产出新帧。

```text
pixels=38400 nonzero=38396 unique=32
0x08C1 x12156   0x11C0 x5638   0x3380 x5613   0x0940 x4307
0x2B00 x3734    0x1A80 x2609   0x339F x962    0x3BC0 x717
```

(Image: `fb_1200m.png` — game frame at 1.2 G steps. / 图片：1.2 G 步时的游戏帧。)

![Game Freak logo rendered by the emulated PPU and scanned out over HSTX](gba-frame-gamefreak.png)

*`example/gba/doc/gba-frame-gamefreak.png` — the frame the HSTX ring is scanning out after
1.2 G steps, captured with `run --dump 0x20063DF4:0x12C00:fb.bin`.*
*`example/gba/doc/gba-frame-gamefreak.png` —— 1.2 G 步时 HSTX 环扫描输出的帧，用
`run --dump 0x20063DF4:0x12C00:fb.bin` 抓取。*

### Regression gates / 回归门禁

| Gate / 门禁 | Result / 结果 |
|-------------|----------------|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --all-targets --all-features -- -D warnings -W clippy::pedantic` | pass |
| `cargo test --all-features` | pass (counts grow with each task; see `doc/changelog.md` — arm 380, periph 281, cli 87 + 1 ignored, gdb 66, core 65, usb 44, usb-host 20, egui 35, …) |
| `cargo doc --no-deps` | pass, no warnings |
| FV-1 (`RPI_PICO2-RISCV` `print(6 * 7)`) | `42` |
| FV-2 (`RPI_PICO2` `print(6 * 7)`) | `42` |
| FV-3 (CircuitPython, `--no-soft-reset`) | `42` |
| emerald smoke (`cargo test -p rp2350-cli -- --ignored emerald`) | pass (asserts `underruns == 0`) |
| emerald `run` 1.2 G steps | `underruns=0 wof=false unknown=0`, core 1 `Running`, no bus error |
| emerald `run` 2 G steps | `underruns=0 wof=false unknown=0`, `frame_done=527`, core 1 `Running`, no bus error (13.3 s simulated) |

## Known limitations / 已知限制

- The TMDS serialiser, 10-bit symbol packing and DVI bit timing are not modelled;
  the command expander emits 32-bit words and counts them (that is what the
  renderer needs).
  TMDS 串行器、10 位符号打包与 DVI 位时序未建模；命令扩展器输出并统计 32 位字。
- With `EXPAND_EN = 0` the raw pass-through path is unpaced.
  `EXPAND_EN = 0` 的原始透传路径不按像素时钟计时。
- The firmware's `SetGpuReg` buffering uses `REG_VCOUNT`, which the port never
  writes (it is plain SRAM), so copyrighted-screen register writes are applied at
  VBlank. This is the firmware's own behaviour, not an emulator defect; the intro
  scenes enable display registers through the normal VBlank flush.
  固件的 `SetGpuReg` 缓冲依赖 `REG_VCOUNT`，而该端口从不写它（只是普通 SRAM），
  因此版权画面期间的寄存器写在 VBlank 生效。这是固件自身行为，不是模拟器缺陷；
  开场场景通过正常 VBlank 刷新启用显示寄存器。
- HSTX has no dedicated interrupt on silicon; the model borrows `SPARE_IRQ_0`
  (46) for underrun/command-done reporting.
  芯片上 HSTX 没有专用中断；模型借用 `SPARE_IRQ_0`（46）报告断流/命令完成。
- `WFI`/`WFE`/`SEV` sleep and event scheduling, multi-core GDB `vCont`, PSRAM and
  the `IO_BANK0` pin-function mux are still unmodelled. SIO hardware spinlocks and
  the per-core `LDREX`/`STREX` exclusive monitors **are** modelled now.
  `WFI`/`WFE`/`SEV` 睡眠与事件调度、多核 GDB `vCont`、PSRAM 与 `IO_BANK0` 引脚
  功能复用仍未建模；SIO 硬件自旋锁与每核 `LDREX`/`STREX` 独占监视器**已建模**。
