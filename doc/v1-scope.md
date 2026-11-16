# V1 范围与精度声明 / V1 scope and accuracy statement

> 版本 `1.0.0`（2026-11-16）。本文件是 V1 的**发布口径**：包含什么、刻意不包含什么、
> 哪些行为是近似、如何复现验证。README、`doc/README.md` 索引与 release notes 均指向本文件；
> CLI/GUI 内置的「关于」链接未在 V1 提供（GUI 无关于页，CLI 帮助保持精简）。

**EN summary.** V1.0 is a general, deterministic, instruction-accurate (cycle-approximate)
RP2350 emulator. It boots stock MicroPython (Arm and RISC-V) and CircuitPython, and runs
pokeemerald as a real dual-core workload. Timing is **instruction-level** (`steps == cycles`),
not pipeline-level. No JIT, no TrustZone/MPU, no bit-level USB/HSTX timing, and no real-time
60 fps. The default path is bit-reproducible; the optional threaded executor is not.

---

## 1. 定位

**V1.0 = 通用、确定性、指令精确（周期近似）的 RP2350 模拟器。**

- **通用**：MicroPython / CircuitPython / pokeemerald 只是验收客户机；模拟器不按固件名、
  哈希、地址、PC 或循环形态特判。
- **确定性**：默认路径在相同输入下逐字节复现（计数器、stdout、SRAM 与基线一致）。
- **指令精确、周期近似**：每条客户机指令都执行，每个周期都记账；但成本模型是
  「1 条指令 ≈ 1 个周期」，不是流水线级周期精确。

## 2. 包含 / Included

| 领域 | V1 内容 |
|---|---|
| CPU | Arm Cortex-M33（Thumb-16/32、FPv5-SP、NVIC/异常/SysTick、MSPLIM/PSPLIM、惰性 FP 压栈、DSP/SIMD 子集、双核 core0+core1）；RISC-V Hazard3（RV32IMAC+Zcb、machine CSR、Xh3irq、picobin 启动；单核） |
| 内存/存储 | 16 MB XIP Flash（含非缓存别名与维护窗口）、520 KB SRAM、合成 32 KB boot ROM、QMI/XIP、PSRAM 镜像窗口 |
| 时钟/时基 | XOSC/ROSC、PLL_SYS/PLL_USB、`CLK_REF/SYS/PERI/USB/ADC/RTC` 的源与分频；运行时超频/降频立即生效；`cycles→µs`、SysTick、TIMER、PIO CLKDIV、HSTX 像素时钟、UART 波特率、ADC/RTC、DMA 节拍全部跟随实时频率 |
| 外设 | RESETS、CLOCKS、WATCHDOG、TIMER、SIO（自旋锁/FIFO/门铃）、IO_BANK0 + PADS_BANK0（FUNCSEL 功能选择）、UART、ADC、RTC、POWMAN/PSM、BOOTRAM、TICKS、IO_QSPI、DMA（16 通道、RING/CHAIN/DREQ/性能计数器）、PIO（3 块 × 4 SM，9 类指令、autopush/autopull、side-set、CLKDIV、FIFO join、IRQ 路由）、HSTX（RAW/TMDS 命令、像素节拍、FIFO/下溢）、USB 设备控制器 + 主机桥（枚举、CDC、raw REPL） |
| 睡眠/异常 | WFI/WFE/SEV/SEVL 与事件寄存器、阻塞式 SIO FIFO 读与总线 stall 重放、睡眠核快进、异常/故障进入与返回 |
| 音频 | I2S 采样捕获（`AudioSink`/`AudioTap`，CPU 与 DMA 共用 `Pio::push_tx`）；宿主播放后端 `cpal`（feature `playback`）或 null 降级 |
| 工具链 | CLI：`run`/`repl`/`play`/`perfbench`/`gdb`/`mcp-serve`/`list-devices`/`wire`（`run` 支持 `--plugin-dir`/`--wire`/`--plugin-threads` 挂接虚拟设备）；GUI：PlayView（实时画面 + 键盘 + 音频状态）与虚拟实验室（设备调色板、连线、缩放/平移）；插件加载/热重载/线程化执行器；GDB RSP（断点、读写/访问监视点、双核 `vCont`）；MCP（JSON-RPC + HTTP + Bearer） |
| 验证 | FV-1/2/3 固件门禁 + emerald 300 M 基线 + 2011 个测试（40 个测试目标）+ 四道质量门禁；Windows 本地复跑，CI 覆盖 Windows/Linux/macOS |

## 3. 不包含 / Not in V1

以下条目**不属于 V1**，release notes 必须原样保留；纳入需重新评估周期与风险。

1. **CPU 流水线级周期精确**：`steps == cycles`，1 条指令 ≈ 1 个周期，不建模取指/流水线
   停顿、双发射或缓存。
2. **TrustZone 安全态**、SAU、MPU、以及 `MemManage`/`BusFault` 的故障源建模。
3. **HSTX TMDS 10 位编码与 DVI 位时序**；**USB 全速位级时序、CRC、位填充**。
4. **真实 ROM 函数行为**：bootrom 为合成/桩实现（分区、OTP、链式装载等不完整）。
5. **实时游玩**：无 JIT，默认 ≈2 fps，线程模式 3.5–6.5 fps；达不到 60 fps。
6. **连续音频播放**：低帧率下 I2S 必然欠载，GUI 如实显示 `produced/s` vs `need/s`。
7. **MCP TLS**：仅明文 HTTP + Bearer。
8. **存档/状态保存**（`--save-state/--load-state`、GUI Save/Load）：V1 明确排除，
   留待后续版本；`reset` 是宿主侧重启，不是状态恢复。
9. **Web (WASM) / GitHub Pages 演示**：GUI 依赖宿主专用 crate（tokio full、libloading、
   文件系统固件加载），仓库也没有 `index.html`/`Trunk.toml` web 入口；V1 只发布原生二进制。

## 4. 已知近似与限制 / Known approximations

1. `steps == cycles`；周期数字用于相对比较，不作为硬件周期计数器。
2. PIO `CTRL`/`CLKDIV` 写入在下一个 64 步泵送边界生效。
3. PPB 读回看不到核内故障位；`FAULTMASK`/`BASEPRI` 门控已建模，但故障状态字段不完整。
4. bootrom 只有 `rom_table_lookup`/`get_sys_info`/`reboot`/flash 擦写等合成实现。
5. PIO/DMA 仲裁为批量轮转（64 步窗口），不是逐周期仲裁。
6. HSTX 输出以命令字/扫描行为单位，宿主拿到的是 32 位字而非 TMDS 位流。
7. RISC-V 没有 core1；线程模式自动退化为串行。
8. GDB 监视点在线程化执行器模式下不受支持（默认串行模式下可用）。
9. `--core-threads on` 只保证 64 步窗口边界一致，窗口内访存顺序依赖宿主调度，
   **非位可复现**；需要可复现时使用默认模式。
10. `IO_BANK0.FUNCSEL` 已解析并按 pad 选通，但当前只有 `SIO` 与 `PIO0..2` 发布 pad 驱动；
    `UART`/`HSTX`/`PWM` 的 pad 输出与 UART 输入采样尚未接入同一电气矩阵。
11. 虚拟设备插件目前只在 `run` 中接线（`repl`/`play` 未接线、无接线持久化）；插件时钟由
    宿主在创建时与时钟世代变化后的下一个 64 步泵送边界注入（改频后最初 ≤64 步可能仍用旧频率）。

## 5. 确定性与线程模式

| 模式 | 开关 | 可复现性 | 用途 |
|---|---|---|---|
| 默认确定性 | 无（默认） | 逐字节可复现（验收基准） | 回归、门禁、发布证据 |
| 线程化执行 | `--core-threads on` / `--plugin-threads` | 窗口边界一致，窗口内非位可复现 | 交互加速 |

铁律：任何加速路径都必须能关闭；关闭后与默认路径逐字节等同。

## 6. 性能与音频（V1 实测口径）

- emerald 300 M 步：默认 ≈9–11 s 墙钟，真实游戏段 ≈8.5 M 步/秒；`--core-threads on`
  约 1.8–2.0×（真实游戏段 14.9–27.4 M 步/秒）。
- 帧率：252 MHz 下 60 fps 一帧 = 4.2 M 周期；默认 ≈2 fps，线程模式 3.5–6.5 fps。
- 音频：I2S 实测 13 441.7 字/秒（配置 13 440）；模拟器只产出数百帧/秒，必然欠载，
  GUI 显示欠载计数，不伪造、不循环旧样本。
- 性能数字只按**工作负载窗口**报告，不用启动/睡眠段代表整体。

## 7. 验证范围 / Verification

固件门禁（详见 [`implementation-plan.md`](implementation-plan.md#3-固件验证清单v1-门禁)）：

| ID | 固件 | 架构 | 通过标准 |
|---|---|---|---|
| FV-1 | MicroPython `RPI_PICO2-RISCV-20260824-v1.29.0.uf2` | Hazard3 | `repl --exec "print(6 * 7)"` → `42` |
| FV-2 | MicroPython `RPI_PICO2-20260824-v1.29.0.uf2` | Cortex-M33 | 同上 |
| FV-3 | CircuitPython `adafruit-circuitpython-raspberry_pi_pico2-...-10.3.1.uf2` | Cortex-M33 | 同上（`--no-soft-reset`） |
| FV-4 | `emerald.uf2`（示例） | Cortex-M33 ×2 | 300 M 步基线计数器一致 |

复跑命令：

```powershell
cargo fmt --all -- --check
$env:CARGO_INCREMENTAL=0; cargo clippy --all-targets --all-features -- -D warnings -W clippy::pedantic
cargo test --all-features
cargo doc --no-deps
cargo build --release -p rp2350-cli

target\release\rp2350-cli.exe repl --file "<FV-1 UF2>" --exec "print(6 * 7)"
target\release\rp2350-cli.exe repl --file "<FV-2 UF2>" --exec "print(6 * 7)"
target\release\rp2350-cli.exe repl --file "<FV-3 UF2>" --exec "print(6 * 7)" --no-soft-reset
target\release\rp2350-cli.exe run --file "<emerald.uf2>" --max-cycles 300000000 --trace-steps 0
```

平台：V1 在 Windows x86_64 本地完成全量验证；CI 工作流覆盖 Windows / Linux / macOS。

## 8. 发布后路线 / After V1

存档/状态保存（R5）、周期成本模型、TrustZone/MPU、HSTX TMDS/DVI 位时序、USB 位级时序、
MCP TLS、实时游玩（在「不做 JIT」前提下不可达）等，均不在 V1 承诺内，按后续版本重新排期。
