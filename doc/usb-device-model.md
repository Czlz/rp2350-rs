# USB 设备控制器模型与 raw-REPL 主机桥 / USB Device Model and raw-REPL Host Bridge

> **Version**: 1.0 | **Date**: 2026-11-08 | **Status**: Design
> **Scope**: `crates/rp2350-usb`（设备侧模型）+ `tools/usb-repl`（主机侧桥）
> **目标**: 让 MicroPython `RPI_PICO2` UF2 通过 USB CDC 接受 raw REPL 驱动，并可承载 `run-perfbench.py` 等价流程。

---

## 1. 背景与已核实事实 / Background

| Item | Value | Source |
|------|-------|--------|
| DPRAM base | `0x5010_0000`, 4 KB | `addressmap.h` `USBCTRL_DPRAM_BASE` |
| REGS base | `0x5011_0000` | `addressmap.h` `USBCTRL_REGS_BASE` |
| 寄存器别名 | `+0x1000` XOR / `+0x2000` SET / `+0x3000` CLR | `REG_ALIAS_*_BITS` |
| 中断号 | `USBCTRL_IRQ = 14` | `rp2350/hardware/regs/intctrl.h` |
| 固件 USB 栈 | TinyUSB（`dcd_rp2040` 端口） | UF2 内字符串 `TinyUSB` |
| 端点缓冲控制位 | `FULL=0x8000`, `LAST=0x4000`, `DATA1=0x2000`, `SEL=0x1000`, `STALL=0x0800`, `AVAIL=0x0400`, `LEN=0x03FF` | `usb_dpram.h` |
| 端点控制位 | `ENABLE=1<<31`, `DOUBLE=1<<30`, `INT_PER_BUF=1<<29`, `INT_PER_DOUBLE=1<<28`, `ON_NAK=1<<16`, `ON_STALL=1<<17`, `TYPE=1<<26`, 缓冲偏移 `[15:6]`（64 字节对齐，掩码 `0xFFC0`） | `usb_dpram.h`, rp2350js `usb.ts` |
| 中断寄存器偏移 | `INTR=0x08C`, `INTE=0x090`, `INTF=0x094`, `INTS=0x098`（与 RP2040 不同） | `rp2350/regs/usb.h` |
| 中断聚合 | `INTS = (INTR & INTE) \| INTF` | `rp2350/regs/usb.h` |
| 固件地址 | SP=`0x2008_2000`，复位向量=`0x1000_015B` | 已知 |

**设计基准**：以 TinyUSB `dcd_rp2040` 实际使用的寄存器子集为准，而不是完整实现数据手册的
主机/设备双模控制器。未使用的寄存器按“可读写影子寄存器”保存，避免固件读回时出现
意外的 0 或总线错误。
**Reference**: TinyUSB `src/portable/raspberrypi/rp2040/{dcd_rp2040.c,rp2040_usb.c}`
and pico-sdk `hardware/{structs,regs}/usb*.h` (RP2350 branch).

---

## 2. 地址空间与区域划分 / Address Space

```
0x5010_0000 ┌────────────────────────────────────────────┐
            │ USBCTRL_DPRAM (4 KB, 字节/半字/字可访问)      │
            │  0x000 setup_packet[8]                      │
            │  0x008 EP1..EP15 ctrl (in/out, 8B each)     │
            │  0x080 EP0..EP15 buf ctrl (in/out, 8B each) │
            │  0x100 EP0 buffer A (64 B)                  │
            │  0x140 EP0 buffer B (64 B)                  │
            │  0x180 epx_data (3712 B，固件自由分配)        │
0x5010_1000 └────────────────────────────────────────────┘
0x5011_0000 ┌────────────────────────────────────────────┐
            │ USBCTRL_REGS (4 KB)                         │
            │  +0x1000 XOR 别名 / +0x2000 SET / +0x3000 CLR│
0x5011_4000 └────────────────────────────────────────────┘
```

`RegsRegion` 的 `size()` 为 `0x4000`，一次性覆盖三个原子别名窗口；地址解码为
`offset = addr - base`，`alias = offset >> 12`，`reg = offset & 0xFFF`。
这样 `usb_hw_set->x = v` / `usb_hw_clear->x = v` 无需主 line 总线额外支持。

`DpramRegion` 只做原始字节存储；地址 `0x100/0x140` 的 EP0 缓冲由控制器内部按方向使用，
不做特殊映射。

---

## 3. 寄存器模型 / Register Model

### 3.1 设备侧寄存器（TinyUSB 实际使用）

| Offset | Name | 语义 |
|--------|------|------|
| `0x000` | `ADDR_ENDP` | `[6:0] ADDRESS` 设备地址 |
| `0x040` | `MAIN_CTRL` | bit0 `CONTROLLER_EN`；bit1 `HOST_NDEVICE`（本模型只支持 0） |
| `0x044` | `SOF_WR` | 主机写 SOF 帧号（设备模式忽略） |
| `0x048` | `SOF_RD` | 最近 SOF 帧号；读操作清除 `INTR.DEV_SOF` |
| `0x04C` | `SIE_CTRL` | bit16 `PULLUP_EN`（主机据此判定设备附着）；bit29 `EP0_INT_1BUF`；bit12 `RESUME`；bit31 `EP0_INT_STALL` |
| `0x050` | `SIE_STATUS` | bit0 `VBUS_DETECTED`, bit4 `SUSPENDED`, bit11 `RESUME`, bit16 `CONNECTED`, bit17 `SETUP_REC`, bit19 `BUS_RESET`；W1C |
| `0x054` | `INT_EP_CTRL` | 主机模式中断端点使能（只保存） |
| `0x058` | `BUFF_STATUS` | bit `2n`=EPn IN，`2n+1`=EPn OUT；W1C |
| `0x05C` | `BUFF_CPU_SHOULD_HANDLE` | 双缓冲时指示应由 CPU 处理哪一半（只读） |
| `0x060/0x064` | `EP_ABORT` / `EP_ABORT_DONE` | 写 1（W1S）中止端点：立即清零对应 buf ctrl 并置 done（W1C） |
| `0x068` | `EP_STALL_ARM` | EP0 IN/OUT 握手 STALL 前的 arm 位（只保存） |
| `0x070` | `EP_STATUS_STALL_NAK` | NAK/STALL 事件位（W1C，按需置位） |
| `0x08C` | `INTR` | 原始中断（只读） |
| `0x090` | `INTE` | 中断使能（读写，仅低 24 位有效） |
| `0x094` | `INTF` | 中断强制（读写） |
| `0x098` | `INTS` | `(INTR & INTE) \| INTF`（只读），`irq_asserted` 判据 |

`INTR` 位（**RP2350 专用布局，与 RP2040 不同**）：

| bit | Name | bit | Name |
|-----|------|-----|------|
| 0 | `HOST_CONN_DIS` | 12 | `BUS_RESET` |
| 1 | `HOST_RESUME` | 13 | `DEV_CONN_DIS` |
| 2 | `HOST_SOF` | 14 | `DEV_SUSPEND` |
| 3 | `TRANS_COMPLETE` | 15 | `DEV_RESUME_FROM_HOST` |
| 4 | `BUFF_STATUS` | 16 | `SETUP_REQ` |
| 5 | `ERROR_DATA_SEQ` | 17 | `DEV_SOF` |
| 6 | `ERROR_RX_TIMEOUT` | 18 | `ABORT_DONE` |
| 7 | `ERROR_RX_OVERFLOW` | 19 | `EP_STALL_NAK` |
| 8 | `ERROR_BIT_STUFF` | 20 | `RX_SHORT_PACKET` |
| 9 | `ERROR_CRC` | 21 | `ENDPOINT_ERROR` |
| 10 | `STALL` | 22 | `DEV_SM_WATCHDOG_FIRED` |
| 11 | `VBUS_DETECT` | 23 | `EPX_STOPPED_ON_NAK` |

### 3.2 DPRAM 布局常量

```
SETUP_PACKET      0x000
EP_CTRL_BASE      0x008   // EP1 起始；EPn (n>=1) at 0x008 + (n-1)*8, in/+0 out/+4
EP_BUF_CTRL_BASE  0x080   // EPn (n>=0) at 0x080 + n*8, in/+0 out/+4
EP0_BUF_A         0x100   // 单缓冲时 IN/OUT 共用
EP0_BUF_B         0x140   // EP0_DOUBLE_BUF=1 时 IN 的第二缓冲
EPX_DATA          0x180
```

端点控制寄存器 `[15:6]` 是 4 KB 内的缓冲字节偏移，64 字节对齐（TinyUSB 的
`hw_data_offset()` 直接 OR 入，rp2350js 用 `& 0xffc0` 取回）；其余高位按 §1 的位定义解释。
端点 n≥1 的数据缓冲位于 `epx_data` 内，由固件分配，本模型只按该偏移的 64 字节粒度读写。
双缓冲的第二半固定位于 `offset + 64`。

---

## 4. 端点与 FIFO 行为 / Endpoint Behaviour

每个端点方向只有一个 64 字节（或固件声明的更小）缓冲；双缓冲时 32 位寄存器高 16 位是第二半。

### 4.1 IN（设备 → 主机）

1. 固件写 `buf_ctrl.in = LEN | DATA1? | FULL | LAST | AVAIL`（先写非 AVAIL 位，再写 AVAIL）。
2. 主机发 IN token：
   - `AVAIL=0` → 返回 NAK（固件尚未装载数据）；
   - `STALL=1` → 返回 STALL；
   - 否则按 `LEN` 从对应 DPRAM 缓冲取数据，清除 `AVAIL|FULL`，置 `BUFF_STATUS[2n]` 与
     `INTR.BUFF_STATUS`；`LAST=1` 时同时表示整个传输完成。
3. 16 位写操作只影响对应半（双缓冲），与 `bufctrl_write16` 语义一致。

### 4.2 OUT（主机 → 设备）

1. 固件写 `buf_ctrl.out = AVAIL | LAST?`（`FULL=0` 表示已就绪接收）。
2. 主机发 OUT token：
   - `AVAIL=0` → NAK；
   - 否则把 ≤64 字节写入缓冲，置 `FULL | LEN`，清 `AVAIL`，置 `BUFF_STATUS[2n+1]` 与
     `INTR.BUFF_STATUS`。
3. 固件读完后写 `buf_ctrl.out = 0`（或重新 arm）并清 `BUFF_STATUS`。

### 4.3 EP0 与 SETUP

- 主机发 SETUP 时：8 字节写入 `0x000`，置 `SIE_STATUS.SETUP_REC` 与 `INTR.SETUP_REQ`；
  清 EP0 双方向缓冲控制（丢弃未完成的数据阶段），恢复 DATA0 期望。
- 固件在中断中读 setup packet、清 `SIE_STATUS.SETUP_REC`，随后按请求方向装载 EP0。
- 数据阶段沿用 §4.1/§4.2；状态阶段是一个 0 长度的反向传输。
- `EP0_DOUBLE_BUF=0` 时 EP0 IN/OUT 都使用 `0x100`（与 TinyUSB 的 `ep0_buf_a` 指针一致）。

### 4.4 总线事件

| 主机动作 | 模型响应 |
|----------|----------|
| `host_attach()` | `VBUS_DETECTED|CONNECTED`；`INTR.DEV_CONN_DIS` 仅在断开时置位 |
| `host_bus_reset()` | 地址清零、所有 EP ctrl/buf ctrl 清零、`SIE_STATUS.BUS_RESET` + `INTR.BUS_RESET` |
| `host_sof(frame)` | 更新 `SOF_RD`，置 `INTR.DEV_SOF`（读 `SOF_RD` 清除） |
| 固件清 `PULLUP_EN` | 主机视为设备拔出，回到 Detached |
| 固件未置 `MAIN_CTRL.CONTROLLER_EN` | 主机事务返回 `NoDevice` |

---

## 5. 枚举流程 / Enumeration

主机桥按 USB 2.0 全速设备流程（与 TinyUSB CDC 描述符兼容）：

```
Detached
  └─(SIE_CTRL.PULLUP_EN=1)→ Attached
       └─(总线复位 10 ms)→ Default
            ├─ GET_DESCRIPTOR(DEVICE, 8)      addr 0
            ├─ SET_ADDRESS(1)                 addr 0 → 1
            ├─ GET_DESCRIPTOR(DEVICE, 18)     addr 1
            ├─ GET_DESCRIPTOR(CONFIG, 9)      addr 1
            ├─ GET_DESCRIPTOR(CONFIG, total)  addr 1
            ├─ GET_DESCRIPTOR(STRING, 0/1/2/3)（失败可跳过）
            └─ SET_CONFIGURATION(1)           addr 1
                 └─ CDC class:
                      SET_CONTROL_LINE_STATE(DTR|RTS)
                      GET_LINE_CODING（可选，失败不致命）
                      → Configured
```

- 配置描述符解析：识别 `bInterfaceClass=0x02`（CDC 控制）与 `0x0A`（CDC 数据），
  从数据接口取 Bulk IN / Bulk OUT 端点号与 `wMaxPacketSize`；端点号不做硬编码。
- 控制传输由 `SetupStage → DataStage(IN/OUT)* → StatusStage` 组成；每个阶段对应一次
  主机令牌，固件 NAK 时按重试间隔轮询。
- 每个阶段有独立超时（默认 25 ms 仿真时间）；整轮枚举上限 200 ms。

---

## 6. 主机桥状态机 / Host Bridge State Machine

```
HostState:
  Detached → Attached → Reset → Address → Enumerate → Configured → CdcReady → Failed

每一次 poll(core, now_cycles):
  1. 若 PullUp 消失 → Detached / 重枚举
  2. 按 next_action_at 判定是否已到时间片
  3. 执行一个“主机动作”：SETUP / IN / OUT / 状态阶段 one-shot
  4. 返回 Advised(cycles)：固件尚未响应时给出下一次重试等待
  5. 阶段超时 → HostError::Timeout{stage}
```

**时序模型（周期折算）**：

- 仿真器以 CPU 周期推进；USB 时间全部换算为周期：
  `cycles_per_ms = sysclk_hz / 1000`（150 MHz 时 150,000 周期/ms）。
- 主机动作粒度为“一次事务”，而非位级：每次 SETUP/IN/OUT 记 `1000` 个周期开销（≈6.7 μs
  含总线周转），帧边界由 `1 ms` 计数器对齐。rp2350js 实测其端点交付延迟为
  `10 μs`（`readDelayMicroseconds` / `writeDelayMicroseconds`），本模型以 10 μs 作为
  默认重试粒度。
- 固件 NAK 时 `poll` 返回 `Advised(100 μs)`；调用方用该值推进 guest 后再次 `poll`。
- 不进行位级/包级抢占：仿真器串行执行 CPU，USB 事务只在 `poll` 时发生，因此不存在
  需要中断 CPU 的并发点。
- **取舍**：该模型不模拟 SOF 周期抖动、CRC、位填充和总线错误；TinyUSB 的 errata
  路径（E2/E15）按“立即完成 abort / 忽略”处理。对 REPL 与 perfbench 语义无影响。
  与 rp2350js 的“AVAIL 写入即触发 + alarm 延迟”相比，本模型采用“主机轮询 + 周期调度”，
  优点是 host 策略与控制器解耦、可在无 CPU 环境下单测；代价是需要调用方显式推进时间。

---

## 7. raw-REPL 与传输分层 / raw REPL & Transport Layering

### 7.1 协议状态机（对齐 pyexec.c / pyboard.py / rp2040py）

```
enter:  Ctrl-C Ctrl-C Ctrl-A   → 读满 "raw REPL; CTRL-B to exit\r\n>"
exec:   写 source + Ctrl-D     → 读 "OK"
        → stdout 直到 0x04 → stderr 直到 0x04（可再读到 '>'）
exit:   Ctrl-B                 → 回到友好 REPL
abort:  Ctrl-C                 → 中断当前执行
```

- 进入序列先发两次 Ctrl-C（终止已在运行的程序），再发 Ctrl-A；与
  rp2040py `RawReplRunner._send_enter_sequence()` 和 `pyboard.py enter_raw_repl()` 一致。
- 必须匹配**完整** banner 字符串，不能只匹配结尾 `>`，否则会把友好 REPL 的 `>>> ` 误判为已就绪。
- 固件可能尚未启动完成而丢弃首批字节（rp2040py 以 500 ms 周期重发进入序列）；主机桥在
  `enter` 超时后按同一策略重发，而不是直接失败。
- 上传大于设备 TX FIFO 余量时按 `pump()` 语义分片：每片等待固件通过 CDC IN 端点取走数据，
  避免终止 Ctrl-D 被静默丢弃（rp2040py 记录过 890 字节脚本导致的永久挂起）。
- 支持 `soft_reset` 选项：默认关闭（模拟器更接近 `RPI_PICO2` 直接启动），开启时在 banner
  后追加一次 Ctrl-D 并等待 `soft reboot`。

### 7.2 传输抽象（照搬 rp2040py 的分层）

```
HostTransport (stdio / TCP)  ⇄  RawReplSession  ⇄  CdcChannel  ⇄  UsbHostBridge  ⇄  UsbCore
```

- `HostTransport`：只负责字节读写与生命周期，提供 `StdioTransport`、`TcpTransport`；
  Unix 下的 pty 传输以 `#[cfg(unix)]` 提供，Windows 不启用。交互式 REPL 与 perfbench
  共用同一传输接口。
- `CdcChannel`：把 CDC Bulk IN/OUT 端点抽象成有界字节流，提供 `read()` / `write()` /
  `readable_len()`；写入受端点包长与设备 NAK 背压约束。
- `RawReplSession`：只依赖 `CdcChannel`，不知道 USB 事务细节，可在单元测试里用内存
  `CdcChannel` 测试协议状态机。
- `UsbHostBridge`：负责枚举与端点事务，不知道文本协议。

### 7.3 perfbench 驱动

`run-perfbench.py` 的实际流程（已核实）：

1. `target.enter_raw_repl()`
2. 对每个用例 `target.exec_(script)`，脚本 = 基准文件 + `perf_bench/benchrun.py` +
   `bm_run(N, M)`；`benchrun.py` 在设备侧用 `time.ticks_us()/ticks_diff()` 计时，
   打印 `ticks norm result`（或 `-1 -1 SKIP`）。
3. 解析 stdout 的最后一行为 `(time, norm, result)`，按 `N`、`M` 重复并取统计。

本工具 `perfbench` 子命令实现同样的 `enter_raw_repl → exec → 解析 → exit` 流程，
并内置相同的默认 `N/M` 选择与统计输出。外部 `run-perfbench.py` 若需直连，
可用 `--tcp <port>` 暴露同一 `Read+Write` 流；pyserial 侧的串口映射属于部署细节。

---

## 8. 集成接口形状（需主线提供）/ Integration Contract

本设计不修改 `rp2350-core`、`rp2350-arm`、`rp2350-riscv`、`rp2350-egui`。需要主线接入时：

1. **总线注册**：`Controller::dpram_region()`（`0x5010_0000`, 4 KB）与
   `Controller::regs_region()`（`0x5011_0000`, 16 KB，含别名）。
2. **中断投递**：每个仿真批量结束后检查 `Controller::irq_asserted()`（等价
   `INTS != 0`），若为真调用 `CpuCore::signal_interrupt(14)`。固件清 `INTS` 后
   条件自然撤销；无需额外去抖。
3. **CPU 侧缺口（实机固件运行所需，非本 crate 可解决）**：
   - `rp2350-arm` 的异常入口目前把 VTOR 固定为 0；MicroPython 的向量表位于
     `0x1000_0100`，需要 `CpuCore` 支持 VTOR，否则 USB 中断无法正确分发。
   - `rp2350-arm` 的 NVIC MMIO 处理（`handle_nvic_access`）当前未接入总线；
     固件写 `NVIC.ISER` 会得到 `NoDevice`。
   - MicroPython 启动还依赖 `RESETS`、`CLOCKS`、`XIP_CTRL`、`TIMER`、`SIO/PPB`
     等区域；这些属于 chip 主线，不在本 crate 内。
4. **主机桥驱动接口**：调用方循环：
   ```rust
   loop {
       let hint = bridge.poll(&mut usb_core, now_cycles)?;
       guest.advance(hint.cycles);
       now_cycles += hint.cycles;
   }
   ```
   `guest.advance` 内部按小片推进 CPU，并在每片后检查 `irq_asserted()` 投递 IRQ 14。

---

## 9. 测试策略 / Test Strategy

| 层 | 内容 |
|----|------|
| 寄存器单元测试 | DPRAM 布局常量、别名 SET/CLR/XOR、`BUFF_STATUS` W1C、`INTS = (INTR & INTE) \| INTF` |
| 端点单元测试 | IN 装载→主机读取→AVAIL/FULL 清除与 BUFF_STATUS；OUT 装载→主机写入→FULL/LEN；NAK/STALL |
| 枚举测试 | 用脚本化“假固件”在 `UsbCore` 上响应 SETUP；主机桥完成复位→取描述符→设地址→设配置 |
| REPL 测试 | 假固件在 CDC Bulk 端点上回环 raw REPL 帧；`RawReplClient` 完成 Ctrl-A/执行/读取 |
| 实机测试 | 加载真实 UF2 到 `SimpleBus` + `CortexM33`，驱动主机桥；如实记录 CPU 覆盖缺口 |

---

## 10. 参考实现对照与取舍 / Reference Implementations

| 参考 | 借鉴内容 | 取舍 |
|------|----------|------|
| **rp2040py** `o-murphy/rp2040py` | `RawReplRunner` 的 raw-REPL 状态机与字节流分片；transport 抽象（stdio / pty / TCP socket）；`USBCDC` 把 CDC 端点接到模拟控制器 | 照搬协议层与传输分层（§7.1/§7.2）；不移植其 Python/`uio` 依赖，改为 Rust trait；只采用标准库 IO，pty 仅在 Unix 提供 |
| **rp2350js** `c1570/rp2350js` | `usb.ts` 的寄存器偏移、`INTS` 聚合、端点 `AVAIL→回调` 模型；实测端点交付延迟 10 μs | 采纳其寄存器语义与 10 μs 重试粒度（§6）；不采纳“写 AVAIL 即回调 + 定时器”耦合模型，改为主机显式 poll，便于无 CPU 单测 |
| **QEMU RP2040 USB patch**（"shallow RP2040 USB controller"） | 以影子寄存器覆盖 `USBCTRL_REGS_BASE`，保证固件读写不触发总线错误 | 采纳“未实现寄存器可读写”策略（§1/§3）；其寄存器布局为 RP2040，偏移与 `INTR` 位须全部替换为 RP2350 定义 |
| **MicroPython `lib/utils/pyexec.c` / `pyboard.py`** | Ctrl-A/B/C/D 语义、banner 匹配、raw-REPL 流控 | 以 `pyexec.c` 为语义基准；进入流程按 `pyboard.py` 的 `Ctrl-C Ctrl-C Ctrl-A` |
| **TinyUSB `dcd_rp2040`** | EP 控制/缓冲控制位、EP0 共用 `0x100`、abort/errata 处理路径 | 以其实际使用的寄存器子集为实现基准（§1/§4） |
| **pico-sdk `dev_lowlevel`** | 不经 TinyUSB 直接操作 USB 控制器的寄存器序列 | 用作枚举流程与寄存器顺序的交叉验证（§5） |

---

## 11. 参考 / References

- RP2350 Datasheet, Chapter 12 (USB), §12.7 register list.
- pico-sdk `src/rp2350/hardware_regs/include/hardware/regs/{usb.h,intctrl.h,addressmap.h}`.
- pico-sdk `src/rp2350/hardware_structs/include/hardware/structs/usb_dpram.h`.
- pico-sdk `examples/pico-examples/usb/device/dev_lowlevel/dev_lowlevel.c`.
- TinyUSB `src/portable/raspberrypi/rp2040/{dcd_rp2040.c,rp2040_usb.c,rp2040_usb.h}`.
- MicroPython `lib/utils/pyexec.c`（raw REPL）、`tools/pyboard.py`、`tests/run-perfbench.py`、`tests/perf_bench/benchrun.py`.
- rp2040py `o-murphy/rp2040py`（`raw_repl.py`、`repl_runner.py`、`cdc.py`）。
- rp2350js `c1570/rp2350js`（`usb.ts`）。
- QEMU RP2040 USB controller patch（"shallow RP2040 USB controller"）。

