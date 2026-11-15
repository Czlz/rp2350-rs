# rp2350-riscv — Public API / 公共 API

> Hazard3 RISC-V (RV32IMAC + Zba/Zbb/Zbs, advertised as `misa.B`) core emulation.
> Implements the `CpuCore` trait, including the machine-mode integer subset,
> compressed instructions and a minimal external-interrupt path.
> Hazard3 RISC-V（RV32IMAC + Zba/Zbb/Zbs，通过 `misa.B` 宣传）核心仿真。实现
> `CpuCore` trait，包含机器模式整数子集、压缩指令与最小的外部中断通路。
>
> **Status**: implemented — RV32I/M/A/C + Zb\* (Zba/Zbb/Zbs, advertised through
> `misa.B`) + Zcb, machine CSRs with architectural illegal-CSR traps,
> Xh3irq external interrupts, exception/interrupt trap entry, picobin
> `IMAGE_DEF` boot, `lr.w`/`sc.w` reservation. Not implemented: F/D floating
> point, PMP enforcement.
> **状态**: 已实现 — RV32I/M/A/C + Zb\*（Zba/Zbb/Zbs，通过 `misa.B` 宣传）
> + Zcb、带架构非法 CSR 陷阱的机器模式 CSR、Xh3irq 外部中断、
> 异常/中断陷阱入口、picobin `IMAGE_DEF` 启动、`lr.w`/`sc.w` 保留区。
> 未实现：F/D 浮点、PMP 强制访问检查。

---

## Boot handoff / 启动交接

Arm images start with a vector table at flash address zero; RISC-V images
start with a picobin `IMAGE_DEF` block. `boot` scans the first flash page for
the block marker `0xFFFF_DED3` and the `0x44` entry-point item, then loads the
boot2 entry and initial stack pointer from that item. For the target
`RPI_PICO2-RISCV` image the block lives at `0x1000_0014`, the boot2 entry is
`0x1000_0036` and the initial SP is `0x2008_2000`.
Arm 映像以 Flash 起始处的向量表开头；RISC-V 映像以 picobin `IMAGE_DEF` 块开头。
`boot` 会扫描 Flash 首页的块标记 `0xFFFF_DED3` 与 `0x44` 入口点条目，再从该
条目加载 boot2 入口与初始栈指针。目标 `RPI_PICO2-RISCV` 映像的块位于
`0x1000_0014`，boot2 入口为 `0x1000_0036`，初始 SP 为 `0x2008_2000`。

```rust
pub const RISCV_BOOT_MAGIC_ADDR: u32 = 0x1000_0000; // boot2 header magic / boot2 头魔数
pub const RISCV_BOOT_MAGIC: u32 = 0x7188_EBF2;
pub const RISCV_PICOBIN_MARKER_START: u32 = 0xFFFF_DED3;
pub const RISCV_PICOBIN_ENTRY_POINT_ITEM: u32 = 0x44;
pub const RISCV_BOOT_ENTRY_ADDR: u32 = 0x1000_0004; // legacy fallback / 旧式回退
pub const RISCV_BOOT_SP_ADDR: u32 = 0x1000_0024;    // legacy fallback / 旧式回退
```

If no `IMAGE_DEF` item is found, `boot` falls back to the legacy boot2 header
fields above and requires the `0x7188_EBF2` magic. Hosts can bypass the header
entirely with `set_boot_vectors`.
若未找到 `IMAGE_DEF` 条目，`boot` 回退到上述旧式 boot2 头字段，并要求
`0x7188_EBF2` 魔数。宿主可用 `set_boot_vectors` 完全绕过该头部。

---

## Module Structure / 模块结构

| Module | Description / 描述 |
|--------|-------------------|
| `lib` | `Hazard3Core`, boot constants, `CpuCore` implementation |
| `registers` | `Rv32Registers` — x0..x31 with hard-wired zero, `pc` |
| `csr` | `CsrFile` — machine-mode CSRs, `take_trap` / `mret` |
| `execute` | RV32 fetch/decode/execute for the 32-bit encodings |
| `execute_c` | `decompress` — RV32C compressed-instruction expansion |
| `tests` | Instruction-level and boot tests (not part of the public API) |

---

## Key Types / 关键类型

### `Hazard3Core`

```rust
pub struct Hazard3Core { /* private: cpu_id, registers, csr, state,
                            pending_irqs: u64, boot_vectors, total_cycles,
                            reservation: Option<u32> */ }
impl Hazard3Core {
    pub fn new(cpu_id: CpuId) -> Self;

    /// Override the boot vectors used by `CpuCore::init`.
    /// `pc` has its low bit cleared, matching the alignment rule.
    /// 覆盖 `CpuCore::init` 使用的启动向量；`pc` 最低位会被清零。
    pub fn set_boot_vectors(&mut self, sp: u32, pc: u32);

    /// Load boot vectors (explicit or from the picobin header) and run.
    /// 加载启动向量（显式指定或来自 picobin 头）并进入运行状态。
    pub fn boot(&mut self, bus: &mut dyn AddressBus) -> Result<(), CoreError>;
}

impl CpuCore for Hazard3Core {
    fn cpu_id(&self) -> CpuId;
    fn arch(&self) -> CpuArch;                                  // CpuArch::RiscVHazard3
    fn init(&mut self, bus: &mut dyn AddressBus) -> Result<(), CoreError>; // = boot()
    fn run_cycles(&mut self, bus: &mut dyn AddressBus, cycles: CycleCount)
        -> (CycleCount, CpuState);
    fn step(&mut self, bus: &mut dyn AddressBus) -> Result<CpuState, CoreError>;
    fn signal_interrupt(&mut self, irq: IrqNumber);
    fn servicing_irq(&self) -> Option<IrqNumber>;
    fn pc(&self) -> u32;
    fn read_register(&self, index: u8) -> u32;   // x0 hard-wired zero / x0 恒为 0
    fn write_register(&mut self, index: u8, value: u32);
    fn reset(&mut self);
    fn description(&self) -> String;
    fn state(&self) -> CpuState;                 // overridden
    fn total_cycles(&self) -> CycleCount;        // overridden (instruction count)
}
```

`total_cycles` counts executed instructions (one increment per step or taken
interrupt); `xpsr()` keeps the trait default of `0` because RISC-V has no
program status register. `reset()` re-applies the cached boot vectors, which
clears registers, CSRs, pending IRQs and the A-extension reservation.
`total_cycles` 统计已执行指令数（每步或每次响应中断加一）；RISC-V 没有程序状
态寄存器，`xpsr()` 保持 trait 默认值 `0`。`reset()` 重新应用缓存的启动向量，
同时清空寄存器、CSR、挂起中断与 A 扩展保留区。

### `Rv32Registers`

```rust
pub struct Rv32Registers {
    pub x: [u32; 32],   // x0..x31 / 通用寄存器
    pub pc: u32,
}
impl Rv32Registers {
    pub fn new() -> Self;
    pub fn read(&self, index: u8) -> u32;          // x0 always 0 / x0 恒为 0
    pub fn write(&mut self, index: u8, value: u32); // writes to x0 discarded / 写 x0 被丢弃
}
```

---

## Machine CSRs / 机器模式 CSR

`CsrFile` is `pub(crate)`; it is reached through CSR instructions and the
interrupt path, not through the public API. Modeled registers:
`mstatus` / `mstatush` / `misa` / `mie` / `mtvec` / `mscratch` / `mepc` /
`mcause` / `mtval` / `mip` / `mcounteren` / `mcountinhibit` / `pmpcfg0/2/4/6` /
`pmpaddr[0..15]`, the read-only identification registers
`mvendorid`/`marchid`/`mimpid`/`mhartid` (all read 0), the Hazard3 `Xh3irq` set
`meiea` / `meipa` / `meifa` / `meipra` / `meinext` / `meicontext`, and the
counters `minstret` / `mcycle` (both `u64`, low and high halves readable/writable
via `0xB00/0xB80` and `0xB02/0xB82` mirrors, read-only aliases at
`0xC00/0xC80` and `0xC02/0xC82`).

`CsrFile` 是 `pub(crate)`，只能通过 CSR 指令与中断路径访问，不属于公共 API。
已建模寄存器：`mstatus`、`mstatush`、`misa`、`mie`、`mtvec`、`mscratch`、
`mepc`、`mcause`、`mtval`、`mip`、`mcounteren`、`mcountinhibit`、
`pmpcfg0/2/4/6`、`pmpaddr[0..15]`，只读识别寄存器
`mvendorid`/`marchid`/`mimpid`/`mhartid`（均读 0），Hazard3 `Xh3irq` 组
`meiea`、`meipa`、`meifa`、`meipra`、`meinext`、`meicontext`，以及计数器
`minstret` / `mcycle`（均为 `u64`，可通过 `0xB00/0xB80` 与 `0xB02/0xB82`
镜像读写低/高 32 位，并在 `0xC00/0xC80`、`0xC02/0xC82` 提供只读别名）。

`misa` reads the constant `0x4000_1107`; the bitmap is:

| Bits | Value | Meaning / 含义 |
|------|-------|----------------|
| 31:30 | `0b01` | `MXL` = 32-bit / 32 位 |
| 12 | 1 | `M` multiply/divide / 乘除 |
| 8 | 1 | `I` base integer / 基础整数 |
| 2 | 1 | `C` compressed / 压缩指令 |
| 1 | 1 | `B` bit manipulation (Zba/Zbb/Zbs) / 位操作 |
| 0 | 1 | `A` atomics / 原子 |

All other extension bits are zero, so `F`/`D`/`V` are absent. `misa` sits in the
read/write encoding space but is hard-wired: writing it raises
illegal-instruction.

`misa` 读出常量 `0x4000_1107`，位图见上表。其他扩展位全为 0，因此 `F`/`D`/`V`
均不存在。`misa` 位于读写编码空间但为硬连线只读：写入会触发非法指令陷阱。

### CSR read/write semantics / CSR 读写语义

- **Unimplemented CSR** addresses raise illegal-instruction (`mcause = 2`) on
  both read and write; `mtval` carries the **CSR address** and `mepc` points at
  the faulting `csr*` instruction. They are never silently read as zero.
  **未实现的 CSR** 地址在读写时都触发非法指令（`mcause = 2`）；`mtval` 携带
  **CSR 地址**，`mepc` 指向出错的 `csr*` 指令。绝不静默读 0。
- **Read-only CSRs** (address bits `[11:10] == 0b11`, i.e. `cycle`/`instret` and
  the identification registers) trap on write; a `csrrs`/`csrrc` whose source is
  `x0` performs no write and therefore does not trap. `csrrw` always writes, so
  it traps even with `x0` as the source.
  **只读 CSR**（地址位 `[11:10] == 0b11`，即 `cycle`/`instret` 与识别寄存器）
  写入即陷阱；源为 `x0` 的 `csrrs`/`csrrc` 不执行写操作，因此不触发。`csrrw`
  总是写，即使源为 `x0` 也会触发。
- `mstatush` (0x310) is storage only: writes are kept and reads return the stored
  value, but no high-half `mstatus` bit is interpreted.
  `mstatush`（0x310）仅作存储：写入被保留、读取返回存储值，但不解释任何
  `mstatus` 高半部分语义。
- `mstatus` only models the bits `MIE` (bit 3), `MPIE` (bit 7) and `MPP`
  (bits 12:11); a write masks the value to those fields.
  `mstatus` 只建模 `MIE`（位 3）、`MPIE`（位 7）与 `MPP`（位 12:11）；
  写入会被掩码到这几个字段。
- `mepc` writes clear bit 0 (bit 1 stays writable because `C` allows 2-byte
  alignment); `mip` writes only keep the `MEIP` bit; `mcountinhibit` keeps
  bits 2:0. Writing `mip`'s read-only pending bits is ignored, not trapped,
  matching Hazard3's writable-mask behaviour.
  写 `mepc` 清除位 0（因支持 `C`，位 1 保持可写）；写 `mip` 只保留 `MEIP` 位；
  写 `mcountinhibit` 保留位 2:0。写 `mip` 的只读挂起位被忽略而非触发陷阱，
  与 Hazard3 的可写掩码行为一致。
- `mcycle`/`minstret` advance once per executed instruction, so firmware polling
  them sees monotonic progress; the high halves read 0 on RV32 within any
  realistic run. `mhartid` is 0 for both cores.
  `mcycle`/`minstret` 每执行一条指令递增一次，固件轮询时看到单调增长；在任何
  现实运行时长的 RV32 运行中高半部分读 0。两个核心的 `mhartid` 都是 0。
- PMP registers (`pmpcfg0/2/4/6`, `pmpaddr0..15`) are storage only — no access
  checking is enforced. Odd-numbered `pmpcfg` registers do not exist on RV32 and
  raise illegal-instruction.
  PMP 寄存器（`pmpcfg0/2/4/6`、`pmpaddr0..15`）仅作存储 —— 不实施访问检查。
  RV32 上奇数编号的 `pmpcfg` 不存在，访问会触发非法指令陷阱。

### Traps / 陷阱

```text
take_trap(cause, tval, epc) -> handler PC   // machine-mode trap entry
mret() -> restored PC                       // trap return
```

`take_trap` saves `mepc = epc & !3`, `mcause`, `mtval`; moves `MIE` into
`MPIE`, clears `MIE`, sets `MPP = Machine`, clears `mip.MEIP` and resolves the
handler address through `trap_pc`. Hazard3 vectors only **interrupts**: when
`mtvec & 3 == 1` and the cause has bit 31 set, the handler is
`mtvec + 4 * (cause & 0x7FFF_FFFF)`; synchronous exceptions always use the
`mtvec` base. `mret` restores `PC = mepc & !3` and copies `MPIE` back into
`MIE`/`MPIE`, clearing `MPP`.
`take_trap` 保存 `mepc = epc & !3`、`mcause`、`mtval`；把 `MIE` 移入 `MPIE`、
清零 `MIE`、置 `MPP = Machine`、清除 `mip.MEIP`，并通过 `trap_pc` 解析处理程序
地址。Hazard3 只对**中断**做向量化：`mtvec & 3 == 1` 且 cause 的 bit 31 置位时
处理程序为 `mtvec + 4 * (cause & 0x7FFF_FFFF)`；同步异常一律使用 `mtvec` 基址。
`mret` 恢复 `PC = mepc & !3`，并把 `MPIE` 复制回 `MIE`/`MPIE`，同时清零 `MPP`。

Typical `mcause` values produced by the core / 核心产生的典型 `mcause`：

| Cause | Source / 来源 |
|-------|--------------|
| 2 | Illegal instruction, or illegal/unimplemented CSR (`mtval` = opcode for a decode failure, **CSR address** for a CSR fault) / 非法指令，或非法/未实现 CSR（解码失败时 `mtval` 为操作码，CSR 故障时为 **CSR 地址**） |
| 3 | `EBREAK` (`mtval` = 0) |
| 5 | Load bus error (`mtval` = address) |
| 7 | Store bus error (`mtval` = address) |
| 11 | `ECALL` (`mtval` = 0) |
| 0x8000_000B | External interrupt (`mtval` = 0) |

---

## External interrupts / 外部中断

`signal_interrupt(irq)` sets bit `irq` in a 64-bit `pending_irqs` mask (IRQs ≥
64 are ignored) and sets `mip.MEIP`. Signalling does **not** change the core
state: a sleeping core wakes on the next step because `wfi` resumes as
soon as `pending_interrupt()` reports a takeable IRQ (the same rule that decides
whether the interrupt is entered). On the next step, when both `mstatus.MIE` and
`mie.MEIE` are set, the core enters the handler: the interrupt bit is cleared
from `pending_irqs` (and `mip.MEIP` when nothing else is pending),
`meipa` records the IRQ number, and a trap with
`mcause = 0x8000_000B` is taken.
`signal_interrupt(irq)` 在 64 位 `pending_irqs` 掩码中置位（IRQ ≥ 64 被忽略）
并置 `mip.MEIP`。断言**不会**改变核心状态：睡眠核在下一步被唤醒，因为
`wfi` 只要 `pending_interrupt()` 报告可进入的中断就恢复（与「是否进入
中断」使用同一条规则）。在下一次 step 时，若 `mstatus.MIE` 与 `mie.MEIE` 均已
置位，核心进入处理程序：从 `pending_irqs` 清除该中断位（无其他挂起时同时清
`mip.MEIP`），`meipa` 记录 IRQ 编号，然后以 `mcause = 0x8000_000B` 进入陷阱。

### Sleep, events and blocked loads / 睡眠、事件与阻塞载入

```rust
pub enum SleepReason { WaitForInterrupt, BlockedRead }

impl Hazard3Core {
    pub fn sleep_reason(&self) -> Option<SleepReason>;
    /// Host event source (FIFO traffic, peer SEV). / 宿主事件源（FIFO 流量、对端 SEV）。
    pub fn signal_event(&mut self);
    pub fn event_pending(&self) -> bool;
}
```

* **`WFI`** retires normally and then suspends unless an enabled interrupt is
  already pending (`pending_interrupt()` agrees with the entry path, so the core
  never sleeps while the interrupt it waits for could be taken). A sleeping core
  advances `csr.tick()` and `total_cycles` every step, wakes on a takeable
  interrupt (which is then entered in the same step) or on a host event, and
  resumes at `PC` — the instruction after `WFI`.
  Hazard3 has no architectural `WFE`, but the pico-sdk compiles `__wfe()` to
  `wfi` on RISC-V, so the inter-core mailbox needs a wake source that is not an
  interrupt: `signal_event()` is that source, and the CLI drives it from the SIO
  event generation.
* **Blocked load**: a load that raises
  `CoreError::BusError { kind: BusErrorKind::Stall }` (an empty blocking FIFO
  read) is **not** turned into a guest-visible exception. The core keeps `PC`
  and the destination register unchanged, reports `CpuState::Sleeping` with
  `SleepReason::BlockedRead`, and replays the same instruction once the data
  arrives; then it retires exactly once. Stores never stall.
* Not modelled: the RISC-V machine timer (`sio_mtime`/`mtimecmp`) and the
  `mip.MTIP` wake source, `WFI`'s "resume after any enabled interrupt even if
  masked" nuance (this model requires the interrupt to be takeable), and any
  power/clock-gating entry.

`WFI` 正常退役后，只要没有已使能中断挂起就挂起（`pending_interrupt()` 与进入路径
一致，因此绝不会在「等待的中断本可进入」时睡下）。睡眠核每步推进 `csr.tick()` 与
`total_cycles`，在可进入中断（并在同一步进入）或宿主事件到来时唤醒，并从 `PC`
（`WFI` 之后那条指令）继续。Hazard3 没有架构 `WFE`，但 pico-sdk 在 RISC-V 上把
`__wfe()` 编译为 `wfi`，因此核间信箱需要一个非中断唤醒源：`signal_event()` 就是
它，CLI 依据 SIO 事件世代驱动。

**阻塞载入**：抛出 `CoreError::BusError { kind: BusErrorKind::Stall }`（空阻塞
FIFO 读）的载入**不会**变成客户机可见异常：核心保持 `PC` 与目的寄存器不变，以
`SleepReason::BlockedRead` 报告 `CpuState::Sleeping`，数据到达后重放同一条指令并
恰好退役一次。存储永不 stall。未建模：RISC-V 机器定时器
（`sio_mtime`/`mtimecmp`）与 `mip.MTIP` 唤醒源、`WFI`「即使被屏蔽也可被使能中断
唤醒」的细节（本模型要求中断可进入），以及任何功耗/时钟门控进入。

Xh3irq model: 16-bit `meiea` windows gate external IRQs on top of
`mstatus.MIE` && `mie.MEIE`; `meipa` derives from pending/force state and
`meinext.update` claims the pending bit for a given priority. Interrupt entry
selects the highest-priority pending IRQ and records it in `meipa`/`meinext`;
same-priority interrupts do not preempt (higher priority does), and `mret`
restores `ppreempt`/`pppreempt` while preserving NOIRQ so the pico-sdk handler
bookkeeping stays consistent. This is sufficient for the MicroPython RISC-V
interrupt chain, but not a complete Xh3irq controller (no external
interrupt-array block or latency counters).
Xh3irq 模型：16 位 `meiea` 窗口在 `mstatus.MIE` 与 `mie.MEIE` 之上再对
外部中断做三级屏蔽；`meipa` 由挂起/强制状态推导，`meinext.update` 按优先级
认领挂起位。中断入口选择最高优先级的挂起 IRQ 并记录到 `meipa`/`meinext`；
同优先级中断不再抢占（更高优先级可以），`mret` 恢复 `ppreempt`/`pppreempt`
并保留 NOIRQ，使 pico-sdk 的处理程序记账保持一致。该模型足以支撑
MicroPython RISC-V 的中断链，但并非完整 Xh3irq 控制器（缺少外部中断阵列
与延迟计数器）。

---

## Instruction coverage / 指令覆盖

### 32-bit encodings / 32 位编码

- **RV32I**: `LUI`, `AUIPC`, `JAL`, `JALR`, the 6 conditional branches,
  `LB`/`LH`/`LW`/`LBU`/`LHU` (load faults raise cause 5, stores cause 7),
  stores, OP-IMM, OP (including arithmetic right shifts), `FENCE` (no-op).
- **M**: `MUL`, `MULH`, `MULHSU`, `MULHU`, `DIV`, `DIVU`, `REM`, `REMU` with
  the architectural div-by-zero and overflow results.
- **A** (`funct3 = 010`): `LR.W`/`SC.W` via a single in-core `reservation`
  address (no bus-side reservation), plus the full AMO family (`AMOSWAP`,
  `AMOADD`, `AMOXOR`, `AMOAND`, `AMOOR`, `AMOMIN/U/MAX/U`).
- **SYSTEM**: `ECALL` (cause 11), `EBREAK` (cause 3), `MRET`, `WFI`
  (real sleep unless an enabled interrupt is pending, see above), and the
  `CSRR*` family including immediate forms; writes to read-only
  (`(csr >> 10) & 3 == 3`, plus `misa`) CSRs and any access to an
  unimplemented CSR are illegal-instruction traps with the CSR address in
  `mtval`.
- **RV32I**：`LUI`、`AUIPC`、`JAL`、`JALR`、6 条条件分支、
  `LB`/`LH`/`LW`/`LBU`/`LHU`（加载总线错误触发 cause 5，存储为 cause 7）、
  存储指令、OP-IMM、OP（含算术右移）、`FENCE`（空操作）。
- **M**：`MUL`、`MULH`、`MULHSU`、`MULHU`、`DIV`、`DIVU`、`REM`、`REMU`，
  含架构规定的除零与溢出结果。
- **A**（`funct3 = 010`）：`LR.W`/`SC.W` 通过核内单一 `reservation` 地址实现
  （不在总线侧保留），外加完整的 AMO 家族（`AMOSWAP`、`AMOADD`、`AMOXOR`、
  `AMOAND`、`AMOOR`、`AMOMIN/U/MAX/U`）。
- **SYSTEM**：`ECALL`（cause 11）、`EBREAK`（cause 3）、`MRET`、`WFI`
  （无待处理使能中断时真正睡眠，见上文），以及含立即数形式的 `CSRR*` 家族；写只读
  （`(csr >> 10) & 3 == 3`，外加 `misa`）CSR 与访问任何未实现 CSR 都会触发
  非法指令陷阱，`mtval` 为 CSR 地址。

### Compressed (RV32C + Zcb) / 压缩指令（RV32C + Zcb）

Compressed halfwords are expanded to their 32-bit equivalents by
`decompress` and then executed by the normal decoder, so semantics have a
single implementation. Covered quadrants:

压缩半字先由 `decompress` 展开为等价 32 位指令，再由常规译码器执行，语义只有
一份实现。覆盖的象限：

- **Quadrant 0**: `C.ADDI4SPN`, `C.LW`/`C.SW`, and **Zcb** (`C.LBU`, `C.LHU`,
  `C.SB`, `C.SH`).
- **Quadrant 1**: `C.ADDI`, `C.JAL`, `C.LI`, `C.ADDI16SP`, `C.LUI`,
  `C.SRLI`/`C.SRAI`/`C.ANDI`, the sub-register `AND`/`OR`/`XOR`/`SUB`/`ADDW`
  /`SUBW` group (plus Zcb `C.ZEXT.B`/`C.ZEXT.H`, `C.SEXT.B`/`C.SEXT.H`,
  `C.NOT`, `C.MUL`), `C.J`, `C.BEQZ`, `C.BNEZ`.
- **Quadrant 2**: `C.SLLI`, `C.LWSP`, `C.JR`/`C.JALR`, `C.MV`, `C.ADD`
  (incl. `EBREAK`/`NOP` encodings), `C.SWSP`.
- **Zcmp**: `cm.push {reg_list}, -spimm` / `cm.pop {reg_list}, spimm` /
  `cm.popret`/`cm.popretz` — compressed push/pop with register lists and
  stack adjustment (used by the pico-sdk RISC-V interrupt chain).
- **象限 0**：`C.ADDI4SPN`、`C.LW`/`C.SW`，以及 **Zcb**（`C.LBU`、`C.LHU`、
  `C.SB`、`C.SH`）。
- **象限 1**：`C.ADDI`、`C.JAL`、`C.LI`、`C.ADDI16SP`、`C.LUI`、
  `C.SRLI`/`C.SRAI`/`C.ANDI`、子寄存器组 `AND`/`OR`/`XOR`/`SUB`/`ADDW`/
  `SUBW`（另含 Zcb `C.ZEXT.B`/`C.ZEXT.H`、`C.SEXT.B`/`C.SEXT.H`、
  `C.NOT`、`C.MUL`）、`C.J`、`C.BEQZ`、`C.BNEZ`。
- **象限 2**：`C.SLLI`、`C.LWSP`、`C.JR`/`C.JALR`、`C.MV`、`C.ADD`
  （含 `EBREAK`/`NOP` 编码）、`C.SWSP`。
- **Zcmp**：`cm.push {reg_list}, -spimm` / `cm.pop {reg_list}, spimm` /
  `cm.popret`/`cm.popretz` —— 带寄存器列表与栈调整的压缩 push/pop
  （pico-sdk 的 RISC-V 中断链使用）。

Reserved, floating-point and unsupported compressed encodings return `None`
from `decompress` and are raised as illegal-instruction traps with the 16-bit
halfword in `mtval`.
保留、浮点与未支持的压缩编码会让 `decompress` 返回 `None`，随后按非法指令
陷阱处理，`mtval` 为 16 位半字。

### Bit manipulation (Zba/Zbb/Zbs) / 位操作扩展

`Zba`: `sh1add`/`sh2add`/`sh3add` address generation.
`Zbb`: `andn`/`orn`/`xnor`, `pack`/`packh` (`pack` with `rs2 = 0` behaves as
`zext.h`), `clz`/`ctz`/`cpop`, `sext.b`/`sext.h`, `min`/`max`/`minu`/`maxu`,
`rol`/`ror`/`rori`, `orc.b`, `rev8`, `brev8`.
`Zbs`: `bset`/`bclr`/`binv`/`bext` and the immediate forms (`bseti`/`bclri`/
`binvi`/`bexti`). The target MicroPython image exercises `pack`/`packh`, the
bit-count group, the immediate bit ops and `orc.b`/`rev8` during startup.
`Zba`：`sh1add`/`sh2add`/`sh3add` 地址生成。`Zbb`：`andn`/`orn`/`xnor`、
`pack`/`packh`（`rs2 = 0` 时等价 `zext.h`）、`clz`/`ctz`/`cpop`、
`sext.b`/`sext.h`、`min`/`max`/`minu`/`maxu`、`rol`/`ror`/`rori`、
`orc.b`、`rev8`、`brev8`。`Zbs`：`bset`/`bclr`/`binv`/`bext` 及立即数形式
（`bseti`/`bclri`/`binvi`/`bexti`）。目标 MicroPython 映像在启动阶段实际使用
`pack`/`packh`、位计数组、立即数位操作与 `orc.b`/`rev8`。

---

## Not implemented / 未实现清单

- **F/D floating point**: not present; `misa` does not advertise the F/D bits
  and compressed FP encodings are illegal.
  **F/D 浮点**：不存在；`misa` 不宣传 F/D 位，压缩浮点编码视为非法。
- **Full picobin `IMAGE_DEF` parsing**: the parser locates the `0xFFFF_DED3`
  marker and the `0x44` entry-point item (boot2 entry + SP). It does not parse
  load-map, hash, signature, partition-table or hash-value items.
  **完整 picobin `IMAGE_DEF` 解析**：解析器定位 `0xFFFF_DED3` 标记与 `0x44`
  入口点条目（boot2 入口 + SP）；不解析 load-map、哈希、签名、分区表或
  hash-value 条目。
- **RISC-V boot ROM function behaviour**: the table itself is complete for
  RP2350 (see below). What is *not* modelled is the behaviour behind the
  zero-returning stubs, and tags outside the RP2350 `bootrom_constants.h` set
  resolve to 0 (the ROM's "not found" result).
  **RISC-V boot ROM 函数行为**：表项本身对 RP2350 已完备（见下）。*未建模*的是
  返回 0 占位桩背后的函数行为；RP2350 `bootrom_constants.h` 集合之外的标签解析
  为 0（ROM 的「未找到」结果）。
  The synthetic ROM (`rp2350-bootrom`) exposes the two well-known entry slots
  and a full RP2350 function/data table.
  合成 ROM（`rp2350-bootrom`）暴露两个 well-known 入口槽与完整的 RP2350
  函数/数据表。

  Well-known slots / well-known 槽位:

  | Slot | Value | Role / 作用 |
  |------|-------|-------------|
  | `0x7DFA` | `0x75C2` | `rom_func_lookup(code, mask)` trap stub / 函数查询桩 |
  | `0x7DF8` | `0x75FE` | `rom_data_lookup(code)` value wrapper / 数据查询包装 |
  | `0x0014` | `0x7C00` | `_bootrom_func_table` pointer / 函数表指针 |

  Function tags with **real behaviour** (host trap) / 有**真实行为**的函数标签
  （宿主陷阱）: `IF` `_connect_internal_flash`, `EX` (also `IX`)
  `_flash_exit_xip`, `RE` `_flash_range_erase`, `RP` `_flash_range_program`,
  `FC` `_flash_flush_cache`, `CX` `_flash_enter_cmd_xip`, `GS`
  `_get_sys_info`, `RB` `_reboot`, `SR` `_bootrom_state_reset`.

  Function tags resolving to a **zero-returning stub** (valid pointer, call
  returns 0 and has no side effects) / 解析为**返回 0 占位桩**的函数标签（指针
  有效，调用返回 0 且无副作用）: `UB` `_reset_to_usb_boot`, `AB`
  `_pick_ab_partition`, `CI` `_chain_image`, `EB` `_explicit_buy`, `FA`
  `_flash_runtime_to_storage_addr`, `FO` `_flash_op`, `GB` `_get_b_partition`,
  `GP` `_get_partition_table_info`, `GU` `_get_uf2_target_partition`, `LP`
  `_load_partition_table`, `OA` `_otp_access`, `RA`
  `_flash_reset_address_trans`, `RC` `_set_rom_callback`, `SC` `_secure_call`,
  `SP` `_set_ns_api_permission`, `SS` `_set_bootrom_stack`, `XM`
  `_flash_select_xip_read_mode`, `VB` `_validate_ns_buffer`.

  Data tags / 数据标签: `FD` `_flash_device_info` (two-level pointer to the
  16-bit `FLASH_DEVINFO` word, `CS0` = 16 MB); `GR` `_software_git_revision`,
  `PT` `_partition_table`, `XF` `_saved_xip_setup_func_ptr` (valid pointer to a
  zero literal) /（前者的两级指针指向 16 位 `FLASH_DEVINFO` 字，`CS0` = 16 MB；
  后三者指向零字面量）.

  **Stub vs unmodelled / 桩与未建模的区别**: a *stub* has a valid table entry
  and stub address — lookups succeed and callers' NULL checks pass — but the
  call performs no work and returns 0. An *unmodelled tag* has no table entry;
  `rom_func_lookup`/`rom_data_lookup` return `0`, which is the ROM's own
  "not found" result. Fatal RISC-V exceptions are additionally routed through
  the synthetic vector table at `0x7E00` (cause 2/5 → `0x7F00`).
  **桩与未建模的区别**：*桩*有有效的表项与桩地址 —— 查询成功、调用者的 NULL
  检查通过 —— 但调用不做任何工作并返回 0。*未建模标签*没有表项，
  `rom_func_lookup`/`rom_data_lookup` 返回 `0`，即 ROM 自身的「未找到」结果。
  致命 RISC-V 异常另经 `0x7E00` 的合成向量表路由（cause 2/5 → `0x7F00`）。
- **`mstatus`**: only `MIE`/`MPIE`/`MPP` are modeled; other status bits
  (FS, VS, UXL …) do not exist.
  **`mstatus`**：仅建模 `MIE`/`MPIE`/`MPP`；其他状态位（FS、VS、UXL 等）
  不存在。
- **PMP enforcement**: configuration and address registers are storage only.
  **PMP 强制检查**：配置与地址寄存器仅作存储。
- **`mstatush`/`mcountinhibit` semantics**: both keep written values, but the
  high-half `mstatus` fields and the counter-inhibit mask (`CY`/`IR`/`HPM` gating
  of `mcycle`/`minstret`) are not interpreted; the counters always advance.
  **`mstatush`/`mcountinhibit` 语义**：两者保留写入值，但不解释 `mstatus`
  高半字段与计数器抑制掩码（`CY`/`IR`/`HPM` 对 `mcycle`/`minstret` 的门控）；
  计数器始终递增。
- **Misaligned accesses** and bus-side atomics/reservations are not modeled.
  **非对齐访问**与总线侧原子/保留机制未建模。

---

## Usage Example / 使用示例

```rust
use rp2350_core::memory::{RamRegion, SimpleBus};
use rp2350_core::types::CpuId;
use rp2350_cpu::CpuCore;
use rp2350_riscv::Hazard3Core;

let mut bus = SimpleBus::new();
bus.register_region(Box::new(RamRegion::new("flash", 0x1000_0000, 0x1000)))?;
bus.register_region(Box::new(RamRegion::new("sram", 0x2000_0000, 0x1000)))?;

// Boot from a picobin header already present in flash at 0x1000_0000,
// or bypass it:
let mut core = Hazard3Core::new(CpuId::Core0);
core.set_boot_vectors(0x2000_0F00, 0x2000_0000);   // (SP, PC)
core.init(&mut bus)?;                              // loads vectors, Running
let (executed, state) = core.run_cycles(&mut bus, 1000);

// Raise external IRQ 3 (delivered once mstatus.MIE && mie.MEIE are set)
core.signal_interrupt(3);                          // IrqNumber = u16
```
