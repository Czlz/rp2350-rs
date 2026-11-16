# 截图资源 / Screenshot assets

本目录存放 README 与文档使用的截图。**所有图片都由仓库当前代码实际运行产生**，
不是示意图或合成图。

| 文件 | 内容 | 生成方式 |
|------|------|----------|
| `fv1-riscv-micropython.png` | RISC-V Hazard3 MicroPython 1.29.0 启动 → USB CDC raw REPL → 执行客户机 Python | FV-1 `repl`（见下） |
| `fv2-arm-micropython.png` | Arm Cortex-M33 MicroPython 1.29.0 启动 → USB CDC raw REPL → 执行客户机 Python | FV-2 `repl`（见下） |
| `fv3-circuitpython.png` | CircuitPython 10.3.1 启动 → USB CDC raw REPL → 执行客户机 Python | FV-3 `repl`（见下） |
| `gui-virtual-lab.png` | GUI「虚拟实验室」标签页：设备调色板、RP2350 节点、拖线画布 | `rp2350-egui`（无参数启动） |

游戏画面在示例目录：[`../../example/gba/doc/emerald-frame-1.2g.png`](../../example/gba/doc/emerald-frame-1.2g.png)。

## 复现命令 / Reproduction

```powershell
# FV-2 Arm MicroPython
target\release\rp2350-cli.exe repl `
  --file "E:\Source\mcu\pico\python\micropython\RPI_PICO2-20260824-v1.29.0.uf2" `
  --exec "import sys; print('RP2350 Arm MicroPython'); print(sys.implementation); print('6 * 7 =', 6 * 7)"

# FV-1 RISC-V MicroPython
target\release\rp2350-cli.exe repl `
  --file "E:\Source\mcu\pico\python\micropython\RPI_PICO2-RISCV-20260824-v1.29.0.uf2" `
  --exec "import sys; print('RP2350 RISC-V Hazard3'); print(sys.implementation); print('6 * 7 =', 6 * 7)"

# FV-3 CircuitPython
target\release\rp2350-cli.exe repl `
  --file "E:\Source\mcu\pico\python\circuitpython\adafruit-circuitpython-raspberry_pi_pico2-zh_Latn_pinyin-10.3.1.uf2" `
  --no-soft-reset `
  --exec "import sys; print('CircuitPython on RP2350'); print(sys.implementation); print('6 * 7 =', 6 * 7)"

# GUI 虚拟实验室（无参数 = 手动模式，默认标签页即 Virtual Lab）
target\release\rp2350-egui.exe

# 游戏帧（1.2 G 步）→ PNG
target\release\rp2350-cli.exe run `
  --file "E:\Source\mcu\pico\rp2350\gba\pokeemerald-rp2350\rp2350\hw\build_emerald\emerald.uf2" `
  --max-cycles 1200000000 --trace-steps 0 --dump 0x20063DF4:0x12C00:fb.bin
py example\gba\tools\fb2png.py fb.bin emerald-frame-1.2g.png 3
```

## 说明 / Notes

- 终端截图是把 `repl` 的 **stdout 原文**渲染成深色终端样式（保留原始行、颜色仅用于区分
  `[repl]` 诊断与客户机输出），未修改任何文字；原文可用上面命令逐字复现。
- 终端渲染不使用第三方库；GUI 截图是窗口的屏幕捕获（Play 页由 GPU 纹理绘制，屏幕捕获
  可能为黑，因此这里选虚拟实验室画布）。
- 固件不随仓库分发，路径与校验值见 [`../../example/gba/README.md`](../../example/gba/README.md)
  与 `.opencode/skills/firmware-verification/SKILL.md`。
