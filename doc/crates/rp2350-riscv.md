# rp2350-riscv — Public API / 公共 API

> Hazard3 RISC-V (RV32IMAC) core emulation. Implements the `CpuCore` trait,
> including the machine-mode integer subset, compressed instructions and a
> minimal external-interrupt path.
> Hazard3 RISC-V（RV32IMAC）核心仿真。实现 `CpuCore` trait，包含机器模式整数
> 子集、压缩指令与最小的外部中断通路。
>
> **Status**: implemented — RV32I/M/A/C + Zb\* (Zba/Zbb/Zbs) + Zcb, machine
> CSRs, Xh3irq external interrupts, exception/interrupt trap entry, picobin
> `IMAGE_DEF` boot, `lr.w`/`sc.w` reservation. Not implemented: F/D floating
> point, PMP enforcement.
> **状态**: 已实现 — RV32I/M/A/C + Zb\*（Zba/Zbb/Zbs）+ Zcb、机器模式 CSR、
> Xh3irq 外部中断、异常/中断陷阱入口、picobin `IMAGE_DEF` 启动、
> `lr.w`/`sc.w` 保留区。未实现：F/D 浮点、PMP 强制访问检查。

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
`mstatus` / `mie` / `mtvec` / `mscratch` / `mepc` / `mcause` / `mtval` / `mip`
/ `mcounteren` / `mcountinhibit` / `pmpcfg0` / `pmpaddr[0..15]`, the Hazard3
`Xh3irq` set `meiea` / `meipa` / `meifa` / `meipra`, and the counters
`minstret` / `mcycle` (both `u64`, low and high halves readable/writable via
`0xB00/0xB80` and `0xB02/0xB82` mirrors). `misa` reads the constant
`0x4000_1105` (RV32IMAC, MXL = 32-bit).

`CsrFile` 是 `pub(crate)`，只能通过 CSR 指令与中断路径访问，不属于公共 API。
已建模寄存器：`mstatus`、`mie`、`mtvec`、`mscratch`、`mepc`、`mcause`、
`mtval`、`mip`、`mcounteren`、`mcountinhibit`、`pmpcfg0`、`pmpaddr[0..15]`、
Hazard3 `Xh3irq` 组 `meiea`、`meipa`、`meifa`、`meipra`，以及计数器
`minstret` / `mcycle`（均为 `u64`，可通过 `0xB00/0xB80` 与 `0xB02/0xB82`
镜像读写低/高 32 位）。`misa` 读出常量 `0x4000_1105`（RV32IMAC，MXL = 32 位）。

### CSR read/write semantics / CSR 读写语义

- Unmodelled CSRs **read as zero** so vendor polling loops keep running; they
  silently ignore writes. `mstatush` (0x310) always reads 0.
  未建模的 CSR **读为 0**，让厂商固件的轮询循环得以继续；对它们的写被静默
  忽略。`mstatush`（0x310）恒读 0。
- `mstatus` only models the bits `MIE` (bit 3), `MPIE` (bit 7) and `MPP`
  (bits 12:11); a write masks the value to those fields.
  `mstatus` 只建模 `MIE`（位 3）、`MPIE`（位 7）与 `MPP`（位 12:11）；
  写入会被掩码到这几个字段。
- `mepc` writes clear the low 2 bits; `mip` writes only the `MEIP` bit is
  kept; `mcountinhibit` keeps bits 2:0.
  写 `mepc` 清除低 2 位；写 `mip` 只保留 `MEIP` 位；写 `mcountinhibit`
  保留位 2:0。
- PMP registers are storage only — no access checking is enforced.
  PMP 寄存器仅作存储 —— 不实施访问检查。

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
| 2 | Illegal / unsupported instruction (`mtval` = opcode) |
| 3 | `EBREAK` (`mtval` = 0) |
| 5 | Load bus error (`mtval` = address) |
| 7 | Store bus error (`mtval` = address) |
| 11 | `ECALL` (`mtval` = 0) |
| 0x8000_000B | External interrupt (`mtval` = 0) |

---

## External interrupts / 外部中断

`signal_interrupt(irq)` sets bit `irq` in a 64-bit `pending_irqs` mask (IRQs ≥
64 are ignored) and sets `mip.MEIP`; it also wakes the core if it is
`CpuState::Sleeping`. On the next step, when both `mstatus.MIE` and
`mie.MEIE` are set, the core enters the handler: the interrupt bit is cleared
from `pending_irqs` (and `mip.MEIP` when nothing else is pending),
`meipa` records the IRQ number, and a trap with
`mcause = 0x8000_000B` is taken.
`signal_interrupt(irq)` 在 64 位 `pending_irqs` 掩码中置位（IRQ ≥ 64 被忽略）
并置 `mip.MEIP`；若核心处于 `CpuState::Sleeping` 还会将其唤醒。在下一次 step
时，若 `mstatus.MIE` 与 `mie.MEIE` 均已置位，核心进入处理程序：从
`pending_irqs` 清除该中断位（无其他挂起时同时清 `mip.MEIP`），`meipa` 记录
IRQ 编号，然后以 `mcause = 0x8000_000B` 进入陷阱。

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
  (treated as a NOP so single-core bring-up does not deadlock), and the
  `CSRR*` family including immediate forms; writes to read-only
  (`(csr >> 10) & 3 == 3`) CSRs are illegal-instruction traps.
- **RV32I**：`LUI`、`AUIPC`、`JAL`、`JALR`、6 条条件分支、
  `LB`/`LH`/`LW`/`LBU`/`LHU`（加载总线错误触发 cause 5，存储为 cause 7）、
  存储指令、OP-IMM、OP（含算术右移）、`FENCE`（空操作）。
- **M**：`MUL`、`MULH`、`MULHSU`、`MULHU`、`DIV`、`DIVU`、`REM`、`REMU`，
  含架构规定的除零与溢出结果。
- **A**（`funct3 = 010`）：`LR.W`/`SC.W` 通过核内单一 `reservation` 地址实现
  （不在总线侧保留），外加完整的 AMO 家族（`AMOSWAP`、`AMOADD`、`AMOXOR`、
  `AMOAND`、`AMOOR`、`AMOMIN/U/MAX/U`）。
- **SYSTEM**：`ECALL`（cause 11）、`EBREAK`（cause 3）、`MRET`、`WFI`
  （视为 NOP，避免单核引导死锁），以及含立即数形式的 `CSRR*` 家族；写只读
  （`(csr >> 10) & 3 == 3`）CSR 会触发非法指令陷阱。

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
- **RISC-V boot ROM coverage**: the synthetic ROM implements the data/function
  lookup slots used by boot2 (`0x7DF8` → `rom_data_lookup` at `0x75FE`,
  `0x7DFA` → `rom_func_lookup` at `0x75C2`) and the flash helpers; other
  RISC-V ROM table entries are not modelled.
  **RISC-V boot ROM 覆盖**：合成 ROM 实现了 boot2 使用的数据/函数查找槽
  （`0x7DF8` → `0x75FE` 的 `rom_data_lookup`、`0x7DFA` → `0x75C2` 的
  `rom_func_lookup`）与 flash 辅助函数；其余 RISC-V ROM 表项未建模。
- **`mstatus`**: only `MIE`/`MPIE`/`MPP` are modeled; other status bits
  (FS, VS, UXL …) do not exist.
  **`mstatus`**：仅建模 `MIE`/`MPIE`/`MPP`；其他状态位（FS、VS、UXL 等）
  不存在。
- **PMP enforcement**: configuration and address registers are storage only.
  **PMP 强制检查**：配置与地址寄存器仅作存储。
- **Unmodelled CSRs** read 0 and ignore writes; the architecture's illegal-CSR
  exception for such reads is not raised.
  **未建模 CSR** 读 0 且忽略写；读取这类 CSR 不触发架构规定的非法 CSR 异常。
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
