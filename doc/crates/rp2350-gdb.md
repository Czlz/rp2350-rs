# rp2350-gdb — Public API / 公共 API

> GDB Remote Serial Protocol (RSP) Server for multi-core ARM Cortex-M33 debugging.
> 用于多核 ARM Cortex-M33 调试的 GDB 远程串行协议（RSP）服务器。

---

## Key Types / 关键类型

### `GdbTarget` Trait

```rust
pub trait GdbTarget: Send + Sync {
    fn architecture(&self) -> GdbArchitecture;
    fn target_xml(&self) -> &str;
    fn read_registers(&self) -> Vec<u8>;
    fn write_registers(&mut self, data: &[u8]);
    fn read_register(&self, regnum: usize) -> u32;
    fn write_register(&mut self, regnum: usize, value: u32);
    fn read_memory(&self, addr: u32, len: usize) -> Vec<u8>;
    fn write_memory(&mut self, addr: u32, data: &[u8]);
    fn insert_breakpoint(&mut self, addr: u32, kind: BreakpointKind) -> bool;
    fn remove_breakpoint(&mut self, addr: u32, kind: BreakpointKind) -> bool;
    fn insert_hw_breakpoint(&mut self, addr: u32) -> bool;
    fn remove_hw_breakpoint(&mut self, addr: u32) -> bool;
    fn insert_watchpoint(&mut self, addr: u32, len: u32, kind: WatchKind) -> bool;
    fn remove_watchpoint(&mut self, addr: u32) -> bool;

    fn step(&mut self);
    fn resume(&mut self);
    fn halt(&mut self, reason: StopReason);
    fn stop_reason(&self) -> Option<StopReason>;
    fn num_threads(&self) -> usize;          // 2 for RP2350
    fn current_thread(&self) -> usize;       // active core
    fn set_current_thread(&mut self, id: usize);

    fn monitor_command(&self, cmd: &str) -> String;
}
```

### `GdbServer`

```rust
pub struct GdbServer<T: GdbTarget> {
    pub target: Arc<Mutex<T>>,
    pub port: u16,
    pub connections: Vec<GdbConnectionHandle>,
    pub breakpoints: HashMap<u32, BreakpointEntry>,
    pub watchpoints: Vec<WatchpointEntry>,
    pub no_ack_mode: bool,
    pub extended_mode: bool,
}

impl<T: GdbTarget + 'static> GdbServer<T> {
    /// Create new server bound to a port.
    pub async fn new(target: T, port: u16) -> Result<Self, GdbError>;

    /// Start accepting connections.
    pub async fn serve(&mut self) -> Result<(), GdbError>;

    /// Stop server and disconnect all clients.
    pub async fn shutdown(&mut self);
}
```

---

## RSP Protocol Handling / RSP 协议处理

| Packet | Command | Description |
|--------|---------|-------------|
| `?` | halt reason | 停止原因查询 |
| `g` | read registers | 读取所有寄存器 |
| `G` | write registers | 写入所有寄存器 |
| `p N` | read single reg | 读取单个寄存器 |
| `P N=V` | write single reg | 写入单个寄存器 |
| `m addr,len` | read memory | 读取内存 |
| `M addr,len:data` | write memory | 写入内存 |
| `c [addr]` | continue | 继续执行 |
| `s [addr]` | step | 单步 |
| `Z0,addr,kind` | insert sw bp | 插入软件断点 |
| `z0,addr,kind` | remove sw bp | 移除软件断点 |
| `Z1,addr,kind` | insert hw bp | 插入硬件断点 |
| `z1,addr,kind` | remove hw bp | 移除硬件断点 |
| `Z2/Z3/Z4` | watchpoints | 读写访问监视点 |
| `Hc T` | set thread | 设置当前线程 |
| `Hg T` | set thread (general) | 设置通用操作线程 |
| `qC` | current thread | 当前线程 ID |
| `qfThreadInfo` / `qsThreadInfo` | list threads | 列出线程 |
| `qSupported` | feature negotiation | 特性协商 |
| `qXfer:features:read:target.xml` | target description | 目标描述 XML |
| `vCont?` / `vCont;c` / `vCont;s:T` | multi-core control | 多核运行控制 |
| `vAttach;PID` | attach | 附加到进程 |
| `qRcmd,cmd` | monitor command | 自定义 monitor 命令 |
| `k` | kill | 终止调试 |

---

## Breakpoint Model / 断点模型

### Software Breakpoints

```rust
pub struct BreakpointEntry {
    pub addr: u32,
    pub kind: BreakpointKind, // 2 (Thumb 16-bit) or 4 (Thumb 32-bit)
    pub original_bytes: Vec<u8>,  // saved for restore
    pub enabled: bool,
}

// On insert: replace [addr..addr+kind] with BKPT #0 (0xBE00 / 0xF000BE00)
// On remove: restore original bytes
```

### Hardware Breakpoints

```rust
// 8 hardware breakpoint slots (FPB unit on Cortex-M33)
// Each slot can match instruction address exactly.
pub fn insert_hw_breakpoint(&mut self, addr: u32) -> bool {
    if let Some(slot) = self.hw_breakpoints.iter_mut().find(|s| s.is_none()) {
        *slot = Some(addr);
        true
    } else {
        false
    }
}
```

### Watchpoints

```rust
pub struct WatchpointEntry {
    pub addr: u32,
    pub len: u32,      // 1, 2, or 4 bytes
    pub kind: WatchKind,
    pub enabled: bool,
}

pub enum WatchKind {
    Write,    // trigger on write
    Read,     // trigger on read
    Access,   // trigger on read or write
}
```

---

## Multi-Core Debugging / 多核调试

```rust
// Thread mapping:
// thread 1 = core0 (Cortex-M33)
// thread 2 = core1 (Cortex-M33)

// Select core0 as active:
// → Hg1, Hc1

// Single-step core0 only:
// → vCont;s:1

// Continue all:
// → vCont;c

// Continue core1, stop core0:
// → vCont;c:2
```

---

## Monitor Commands / Monitor 命令

```
qRcmd,<hex-encoded-command>

Commands:
  pio         → dump all PIO state machine states
  pio <N>     → dump PIO block N (0/1/2)
  gpio        → dump all 48 GPIO pin states
  gpio <N>    → dump GPIO pin N
  dma         → dump all 16 DMA channel states
  dma <N>     → dump DMA channel N
  hstx        → dump HSTX frame statistics
  registers   → dump CPU register values
  memory <addr> <len> → hex dump memory
  breakpoints → list all breakpoints
  watchpoints → list all watchpoints
  info        → system info (clock, cycles, flash usage)
  trace       → dump ITM trace buffer
```

---

## Usage Example / 使用示例

```bash
# Terminal 1: Start emulator with GDB
cargo run -p rp2350-cli -- gdb --port 3333 --file firmware.elf

# Terminal 2: Connect with arm-none-eabi-gdb
arm-none-eabi-gdb
(gdb) target remote :3333
(gdb) monitor info
(gdb) load firmware.elf       # load via GDB
(gdb) break main
(gdb) continue
(gdb) info registers
(gdb) monitor gpio
(gdb) stepi
(gdb) detach
```