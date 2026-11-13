# rp2350-riscv — Public API / 公共 API

> Hazard3 RISC-V (RV32IMAC) core emulation. Implements the `CpuCore` trait,
> including the machine-mode integer subset, compressed instructions and a
> minimal external-interrupt path.
> Hazard3 RISC-V（RV32IMAC）核心仿真。实现 `CpuCore` trait，包含机器模式整数
> 子集、压缩指令与最小的外部中断通路。
>
> **Status**: implemented — RV32I/M/A/C + Zcb, machine CSRs, vectored traps,
> `lr.w`/`sc.w` reservation. Not implemented: Zb* bit-manipulation extensions
> (illegal-instruction trap), F/D floating point, PMP enforcement.
> **状态**: 已实现 — RV32I/M/A/C + Zcb、机器模式 CSR、向量陷阱、
> `lr.w`/`sc.w` 保留区。未实现：Zb* 位操作扩展（按非法指令陷阱）、
> F/D 浮点、PMP 强制访问检查。

---

## Boot handoff / 启动交接

Arm images start with a vector table at flash address zero; RISC-V images
start with a picobin `IMAGE_DEF` header. The default RISC-V handoff validates
only the magic word and then reads the two vector words:

Arm 映像以 Flash 起始处的向量表开头；RISC-V 映像以 picobin `IMAGE_DEF` 头开头。
默认的 RISC-V 交接只校验魔数，然后读取两个向量字：

```rust
pub const RISCV_BOOT_MAGIC_ADDR: u32 = 0x1000_0000; // picobin magic / picobin 魔数
pub const RISCV_BOOT_MAGIC: u32 = 0x7188_EBF2;
pub const RISCV_BOOT_ENTRY_ADDR: u32 = 0x1000_0004; // entry PC / 入口 PC
pub const RISCV_BOOT_SP_ADDR: u32 = 0x1000_0020;    // initial SP / 初始栈指针
```

If the magic does not match, `boot` returns `CoreError::FirmwareError`. The
rest of the `IMAGE_DEF` block (chunks, extension words) is **not** parsed; the
emulator simply enters at the header's entry point. Hosts can bypass the
header entirely with `set_boot_vectors`.

魔数不匹配时 `boot` 返回 `CoreError::FirmwareError`。`IMAGE_DEF` 块的其余部分
（chunk、扩展字）**不**做解析；模拟器只是从头部给出的入口点开始执行。宿主可
用 `set_boot_vectors` 完全绕过该头部。

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
handler address through `trap_pc`. `trap_pc` implements vectored mode: with
`mtvec & 3 == 1` the handler is `mtvec + 4 * (cause & 0x7FFF_FFFF)`, otherwise
the direct base. `mret` restores `PC = mepc & !3` and copies `MPIE` back into
`MIE`/`MPIE`, clearing `MPP`.
`take_trap` 保存 `mepc = epc & !3`、`mcause`、`mtval`；把 `MIE` 移入 `MPIE`、
清零 `MIE`、置 `MPP = Machine`、清除 `mip.MEIP`，并通过 `trap_pc` 解析处理程序
地址。`trap_pc` 实现向量模式：`mtvec & 3 == 1` 时处理程序为
`mtvec + 4 * (cause & 0x7FFF_FFFF)`，否则为直接基址。`mret` 恢复
`PC = mepc & !3`，并把 `MPIE` 复制回 `MIE`/`MPIE`，同时清零 `MPP`。

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

Note on Xh3irq fidelity: `meiea`/`meifa`/`meipra` are stored and readable, but
interrupt routing is gated only by `mstatus.MIE` && `mie.MEIE`, and `meipa` is
written rather than derived from the priority scheme. This is sufficient for
the vector-table-plus-NVIC-style bring-up used by the test suite, but it is
not a full `Xh3irq` controller model.
关于 Xh3irq 保真度：`meiea`/`meifa`/`meipra` 有存储且可读，但中断路由仅由
`mstatus.MIE` 与 `mie.MEIE` 控制，`meipa` 是直接写入而不是按优先级方案推导。
这足以支撑测试套件使用的「向量表 + NVIC 风格」的启动流程，但并非完整的
`Xh3irq` 控制器模型。

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
  /`SUBW` group, `C.J`, `C.BEQZ`, `C.BNEZ`.
- **Quadrant 2**: `C.SLLI`, `C.LWSP`, `C.JR`/`C.JALR`, `C.MV`, `C.ADD`
  (incl. `EBREAK`/`NOP` encodings), `C.SWSP`.
- **象限 0**：`C.ADDI4SPN`、`C.LW`/`C.SW`，以及 **Zcb**（`C.LBU`、`C.LHU`、
  `C.SB`、`C.SH`）。
- **象限 1**：`C.ADDI`、`C.JAL`、`C.LI`、`C.ADDI16SP`、`C.LUI`、
  `C.SRLI`/`C.SRAI`/`C.ANDI`、子寄存器组 `AND`/`OR`/`XOR`/`SUB`/`ADDW`/
  `SUBW`、`C.J`、`C.BEQZ`、`C.BNEZ`。
- **象限 2**：`C.SLLI`、`C.LWSP`、`C.JR`/`C.JALR`、`C.MV`、`C.ADD`
  （含 `EBREAK`/`NOP` 编码）、`C.SWSP`。

Reserved, floating-point and unsupported compressed encodings return `None`
from `decompress` and are raised as illegal-instruction traps with the 16-bit
halfword in `mtval`.
保留、浮点与未支持的压缩编码会让 `decompress` 返回 `None`，随后按非法指令
陷阱处理，`mtval` 为 16 位半字。

---

## Not implemented / 未实现清单

- **Zb\* extensions** (`Zba`/`Zbb`/`Zbs`): no decoder support; these opcodes
  hit the illegal-instruction trap (cause 2, `mtval` = opcode).
  **Zb\* 扩展**（`Zba`/`Zbb`/`Zbs`）：译码器未支持；这些操作码命中非法指令
  陷阱（cause 2，`mtval` = 操作码）。
- **F/D floating point**: not present; `misa` does not advertise the F/D bits
  and compressed FP encodings are illegal.
  **F/D 浮点**：不存在；`misa` 不宣传 F/D 位，压缩浮点编码视为非法。
- **Full picobin `IMAGE_DEF` parsing**: only the magic at `0x1000_0000`,
  entry at `0x1000_0004` and SP at `0x1000_0020` are read.
  **完整 picobin `IMAGE_DEF` 解析**：只读取 `0x1000_0000` 魔数、
  `0x1000_0004` 入口与 `0x1000_0020` 栈指针。
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
