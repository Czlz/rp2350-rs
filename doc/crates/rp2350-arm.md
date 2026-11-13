# rp2350-arm — Public API / 公共 API

> ARM Cortex-M33 core implementation. Implements `CpuCore` trait.
> ARM Cortex-M33 核心实现。实现 `CpuCore` trait。
>
> **Status**: Phase 2 — Thumb-16/32 decoders, conditions, IT blocks, NVIC, SysTick, exception handling implemented.
> **状态**: Phase 2 — Thumb-16/32 解码器、条件码、IT 块、NVIC、SysTick、异常处理已实现。

---

## Module Structure / 模块结构

| Module | Description / 描述 |
|--------|-------------------|
| `registers` | `CortexM33Registers`, PSR flags, CONTROL register |
| `conditions` | `Condition` (15 codes), `ITBlockState` (1-4 instruction blocks) |
| `execute_thumb16` | Thumb-16 decoder + executor (~90 instructions) |
| `execute_thumb32` | Thumb-32 decoder + executor (60+ ALU / Load-Store / system instructions) |
| `execute_thumb32::coproc` | CP0 GPIO coprocessor and CP4 DCP stub (`MCR`/`MRC`/`MCRR`/`MRRC`) |
| `fpu` | FPv5-SP state: S0-S31 / D0-D15 views, FPSCR, FPEXC, CPACR fields |
| `nvic` | NVIC: ISER/ICER/ISPR/ICPR/IABR/IPR/STIR, priority routing |
| `systick` | SysTick 24-bit down counter with reload + interrupt |
| `exceptions` | Exception entry/exit, EXC_RETURN decoding, stacking/unstacking |

---

## Key Types / 关键类型

### `CortexM33Core`

```rust
pub struct CortexM33Core {
    pub core_id: CpuId,          // Core 0 or 1
    pub registers: CortexM33Registers,
    pub nvic: Nvic,
    pub systick: SysTick,
}

impl CortexM33Core {
    pub fn new(core_id: CpuId) -> Self;
}

impl CpuCore for CortexM33Core {
    fn init(&mut self, chip: &mut dyn AddressBus) -> Result<(), CoreError>;
    fn step(&mut self, chip: &mut dyn AddressBus) -> Result<CycleCount, CoreError>;
    fn reset(&mut self, chip: &mut dyn AddressBus) -> Result<(), CoreError>;
    fn signal_interrupt(&mut self, irq: IrqNumber);
    fn servicing_irq(&self) -> Option<IrqNumber>;
    fn pc(&self) -> u32;
    fn read_register(&self, index: u8) -> u32;
    fn write_register(&mut self, index: u8, value: u32);
    fn core_id(&self) -> CpuId;
    fn state(&self) -> CpuState;
}
```

### `CortexM33Registers`

```rust
pub struct CortexM33Registers {
    pub r: [u32; 13],       // R0-R12 — General purpose / 通用寄存器
    pub sp: u32,            // R13 — current stack pointer (MSP or PSP)
    pub lr: u32,            // R14 — Link Register
    pub pc: u32,            // R15 — Program Counter
    pub msp: u32,           // Main Stack Pointer / 主栈指针
    pub psp: u32,           // Process Stack Pointer / 进程栈指针
    pub xpsr: u32,          // Program Status Register (APSR + IPSR + EPSR)
    pub control: Control,   // CONTROL register (nPRIV, SPSEL, FPCA)
    pub primask: u32,       // Priority Mask / 优先级屏蔽
    pub basepri: u32,       // Base Priority / 基础优先级
    pub faultmask: u32,     // Fault Mask / 故障屏蔽
}

impl CortexM33Registers {
    pub fn new() -> Self;
    pub fn read(&self, index: u8) -> u32;
    pub fn write(&mut self, index: u8, value: u32);

    // APSR flag access
    pub fn apsr_n(&self) -> bool;
    pub fn apsr_z(&self) -> bool;
    pub fn apsr_c(&self) -> bool;
    pub fn apsr_v(&self) -> bool;
    pub fn apsr_q(&self) -> bool;
    pub fn set_apsr_nzcv(&mut self, n: bool, z: bool, c: bool, v: bool);
    pub fn set_apsr_q(&mut self, q: bool);

    // Stack control
    pub fn get_sp(&self, spsel: bool) -> u32;
}
```

### `Condition`

```rust
#[repr(u8)]
pub enum Condition {
    Eq = 0,  // Equal — Z == 1
    Ne = 1,  // Not equal — Z == 0
    Cs = 2,  // Carry set / Unsigned higher or same — C == 1
    Cc = 3,  // Carry clear / Unsigned lower — C == 0
    Mi = 4,  // Minus / Negative — N == 1
    Pl = 5,  // Plus / Positive or zero — N == 0
    Vs = 6,  // Overflow — V == 1
    Vc = 7,  // No overflow — V == 0
    Hi = 8,  // Unsigned higher — C == 1 && Z == 0
    Ls = 9,  // Unsigned lower or same — C == 0 || Z == 1
    Ge = 10, // Signed greater than or equal — N == V
    Lt = 11, // Signed less than — N != V
    Gt = 12, // Signed greater than — Z == 0 && N == V
    Le = 13, // Signed less than or equal — Z == 1 || N != V
    Al = 14, // Always (unconditional)
}

impl Condition {
    pub fn from_bits(bits: u8) -> Self;
    pub fn evaluate(&self, regs: &CortexM33Registers) -> bool;
}
```

### `ITBlockState`

```rust
pub struct ITBlockState;

impl ITBlockState {
    pub fn new() -> Self;

    /// Start an IT block. Returns number of instructions in the block.
    /// 启动 IT 块。返回块中的指令数。
    pub fn execute_it(&mut self, firstcond: u8, mask: u8) -> u8;

    /// Advance to next instruction. Returns condition for that instruction.
    /// 前进到下一条指令。返回该指令的条件。
    pub fn advance(&mut self) -> Condition;

    /// Check if currently inside an IT block.
    /// 检查是否在 IT 块内。
    pub fn in_it_block(&self) -> bool;
}
```

### `Nvic`

```rust
pub struct Nvic {
    pub iser: [u32; 8],     // Interrupt Set-Enable / 中断使能 (256 IRQs)
    pub icer: [u32; 8],     // Interrupt Clear-Enable / 中断使能清除
    pub ispr: [u32; 8],     // Interrupt Set-Pending / 中断挂起设置
    pub icpr: [u32; 8],     // Interrupt Clear-Pending / 中断挂起清除
    pub iabr: [u32; 8],     // Interrupt Active Bit / 中断活跃位
    pub ipr: [u8; 256],     // Interrupt Priority / 中断优先级 (8-bit each)
    pub stir: u32,          // Software Trigger Interrupt / 软件触发中断
}

impl Nvic {
    pub fn new() -> Self;

    // ISER access
    pub fn is_enabled(&self, irq: IrqNumber) -> bool;
    pub fn write_iser(&mut self, index: usize, value: u32);
    pub fn write_icer(&mut self, index: usize, value: u32);

    // ISPR/ICPR access
    pub fn is_pending(&self, irq: IrqNumber) -> bool;
    pub fn set_pending(&mut self, irq: IrqNumber);
    pub fn clear_pending(&mut self, irq: IrqNumber);
    pub fn write_ispr(&mut self, index: usize, value: u32);
    pub fn write_icpr(&mut self, index: usize, value: u32);

    // Active bit
    pub fn is_active(&self, irq: IrqNumber) -> bool;
    pub fn set_active(&mut self, irq: IrqNumber);
    pub fn clear_active(&mut self, irq: IrqNumber);

    // Priority
    pub fn get_priority(&self, irq: IrqNumber) -> u8;
    pub fn set_priority(&mut self, irq: IrqNumber, priority: u8);

    // Software trigger
    pub fn write_stir(&mut self, irq: IrqNumber);

    /// Find highest priority pending+enabled IRQ. Returns None if none.
    /// 查找最高优先级且挂起+已使能的中断。无则返回 None。
    pub fn find_highest_priority(&self) -> Option<IrqNumber>;
}
```

### `SysTick`

```rust
pub mod csr {
    pub const ENABLE: u32 = 1 << 0;
    pub const TICKINT: u32 = 1 << 1;
    pub const CLKSOURCE: u32 = 1 << 2;
    pub const COUNTFLAG: u32 = 1 << 16;
}

pub struct SysTick {
    pub csr: u32,           // Control and Status Register / 控制和状态寄存器
    pub rvr: u32,           // Reload Value Register / 重载值寄存器
    pub cvr: u32,           // Current Value Register / 当前值寄存器
    pub calib: u32,         // Calibration Register / 校准寄存器
    pub interrupt_pending: bool,
}

impl SysTick {
    pub fn new() -> Self;

    /// Tick the counter `cycles` clock cycles. Returns true if interrupt generated.
    /// 定时器计时 `cycles` 个时钟周期。产生中断时返回 true。
    pub fn tick(&mut self, cycles: CycleCount) -> bool;

    pub fn is_enabled(&self) -> bool;
    pub fn interrupt_enabled(&self) -> bool;
    pub fn read_cvr(&self) -> u32;
    pub fn write_cvr(&mut self, value: u32);
}
```

### `ExceptionHandler`

```rust
pub struct ExcReturnInfo {
    pub return_to_handler: bool,  // EXC_RETURN mode flag
    pub return_to_thread: bool,
    pub spsel: bool,              // Stack pointer selection
}

/// Decode EXC_RETURN value from LR.
/// 从 LR 解码 EXC_RETURN 值。
pub fn decode_exc_return(lr: u32) -> Option<ExcReturnInfo>;

/// Push register context onto stack on exception entry.
/// 异常进入时将寄存器上下文压入栈。
pub fn exception_entry_stacking(
    regs: &CortexM33Registers,
    sp: &mut u32,
    isr_number: u8,
    bus: &mut dyn AddressBus,
) -> Result<(), CoreError>;

/// Pop register context from stack on exception return.
/// 异常返回时从栈弹出寄存器上下文。
pub fn exception_return_unstacking(
    regs: &mut CortexM33Registers,
    sp: &mut u32,
    bus: &mut dyn AddressBus,
) -> Result<(), CoreError>;

/// Fetch exception vector from vector table.
/// 从向量表获取异常向量。
pub fn fetch_exception_vector(
    vector_table: u32,
    exception_number: u8,
    bus: &dyn AddressBus,
) -> Result<u32, CoreError>;

/// Full exception entry sequence (stacking + vector fetch + mode switch).
/// 完整异常进入序列（栈帧保存 + 向量获取 + 模式切换）。
pub fn enter_exception(
    regs: &mut CortexM33Registers,
    exception_number: u8,
    bus: &mut dyn AddressBus,
) -> Result<(), CoreError>;

/// Full exception return sequence (unstacking + mode restore).
/// 完整异常返回序列（出栈 + 模式恢复）。
pub fn exit_exception(
    regs: &mut CortexM33Registers,
    exc_return: u32,
    bus: &mut dyn AddressBus,
) -> Result<(), CoreError>;
```

---

## Thumb-16 Decoder / Thumb-16 解码器

```rust
pub enum StepResult {
    Ok(CycleCount),    // Instruction executed successfully
    Halted(CoreError), // Execution halted
    Svc(u8),           // Supervisor call (SVC)
    Branch(u32),       // Branch taken to address (for instruction tracing)
}
```

**Supported instruction groups / 支持的指令组** (~90 instructions):
- **Data processing**: `MOV`, `ADD`, `SUB`, `CMP`, `AND`, `ORR`, `EOR`, `MVN`, `ADC`, `SBC`, `LSL`, `LSR`, `ASR`, `ROR`, `TST`, `NEG`, `MUL`, `MRS`, `MSR`
- **Branch**: `B`, `B<cc>`, `BL`, `BX`, `BLX`
- **Load/Store**: `LDR`, `STR`, `LDRH`, `STRH`, `LDRB`, `STRB`, `LDMIA`, `STMIA`, `PUSH`, `POP`
- **Miscellaneous**: `NOP`, `BKPT`, `SVC`, `CPS`, `IT`, `CBZ`, `CBNZ`, `REV`

## Thumb-32 Decoder / Thumb-32 解码器

The Thumb-32 decoder is fully implemented (see `execute_thumb32`): 60+ ALU,
load/store and miscellaneous system instructions, plus the FPU and
coprocessor chapters below. DSP/SIMD encodings (e.g. `SMLA*`, `USADA8`) are
still treated as undefined instructions, exactly as documented in the
`execute_thumb32` module header.
Thumb-32 解码器已完整实现（见 `execute_thumb32`）：60+ 条 ALU、Load/Store
与杂项系统指令，以及下文的 FPU 与协处理器章节。DSP/SIMD 编码（如 `SMLA*`、
`USADA8`）仍按未定义指令处理，与 `execute_thumb32` 模块头的说明一致。

---

## FPU (FPv5-SP, D16) / 浮点单元

RP2350's Cortex-M33 exposes the FPv5-SP unit: 32 single-precision registers
S0-S31, the D0-D15 doubleword view, FPSCR and FPEXC. Access is gated by
CPACR CP10, which is modeled through the `SCB->CPACR` system register.
CP11 is stored but only gates double-precision arithmetic (not implemented),
so it is not required for FPv5-SP access — the target firmware runs VFP
instructions with CP11 at `0b00`.

RP2350 的 Cortex-M33 集成 FPv5-SP 单元：32 个单精度寄存器 S0-S31、D0-D15
双字视图、FPSCR 与 FPEXC。访问由 CPACR CP10 控制，已通过 `SCB->CPACR`
系统寄存器建模。CP11 会被保存，但它只控制双精度算术（未实现），因此
FPv5-SP 访问不要求 CP11 —— 目标固件就是在 CP11 为 `0b00` 时执行 VFP 指令的。

### Implemented / 已实现

- **State**: S0-S31 with `D0-D15` as S-pair aliases; `FPSCR`, `FPEXC`;
  `CPACR` CP0-CP7 and CP10-CP11 read/write.
- **Data movement**:
  - `VLDR` / `VSTR` S and D forms, including PC-relative literals,
    positive/negative 8-bit word offsets, immediate/pre-indexed only.
  - `VLDM` / `VSTM` (and `VPUSH` / `VPOP` aliases) in IA and DB modes,
    with and without writeback.
  - `VMOV` core register ↔ S register, `VMOV` two core registers ↔ D register.
  - `VMOV.F32 Sd, #imm` (VFP 8-bit immediate expansion).
  - `VMRS` / `VMSR` (FPSCR, FPEXC) including `VMRS APSR_nzcv, FPSCR`.
- **Enable/fault**: every FPU encoding checks CPACR first. Denied access
  raises a synchronous UsageFault through the real exception path:
  `CFSR.UFSR.NOCP` set, exception number 6, standard frame stacked on
  MSP/PSP per CONTROL, `EXC_RETURN` 0xFFFFFFF9 / 0xFFFFFFFD / 0xFFFFFFF1.
  The stacked PC is the faulting instruction (PC is rewound before entry).
  Because this emulator enters at the image entry point (the mask-ROM boot
  ROM is not executed), booting applies the boot ROM's post-ROM state and
  leaves CP10/CP11 enabled. A power-on register file still has CPACR
  cleared, and writing CPACR to 0 is fully honored, so the NOCP path is
  reachable and tested.

- **状态**：S0-S31，`D0-D15` 为 S 对别名；`FPSCR`、`FPEXC`；`CPACR`
  CP0-CP7 与 CP10-CP11 读写。
- **数据搬移**：`VLDR`/`VSTR`（S/D、PC 相对字面量、正负 8 位字偏移）、
  `VLDM`/`VSTM`（含 `VPUSH`/`VPOP` 别名，IA/DB，可带写回）、
  `VMOV` 内核↔S、`VMOV` 两个内核↔D、`VMOV.F32 Sd, #imm`、
  `VMRS`/`VMSR`（含 `VMRS APSR_nzcv, FPSCR`）。
- **使能与故障**：每条浮点编码先检查 CPACR。拒绝访问时走真实异常通路：
  置 `CFSR.UFSR.NOCP`，异常号 6，按 CONTROL 在 MSP/PSP 上压标准栈帧，
  `EXC_RETURN` 为 0xFFFFFFF9 / 0xFFFFFFFD / 0xFFFFFFF1；压栈 PC 为
  引发故障的指令（进入前已回退 PC）。

  由于本模拟器直接从映像入口开始（不执行掩膜 ROM 中的 boot ROM），
  启动时复现 boot ROM 交付状态，使 CP10/CP11 保持使能。上电寄存器文件
  的 CPACR 仍为清零状态；把 CPACR 写 0 也会被完全遵守，因此 NOCP
  通路可达且已被测试覆盖。

### Not implemented / 未实现清单

These encodings are currently reported as **undefined instructions**
(PC and opcode logged), never silently skipped. They are tracked so that
firmware relying on them can be identified immediately.

以下编码当前按**未定义指令**上报（记录 PC 与 opcode），绝不静默跳过。
列表用于快速定位依赖这些功能的固件。

1. `VCVT` between single and double precision (`VCVT.F64.F32`,
   `VCVT.F32.F64`) — deferred; FPv5-SP arithmetic is single precision only.
2. `VMOV.F32 Sd, Sm` (FP register to FP register copy).
3. All floating-point arithmetic, comparisons and conversions (Stage 3):
   `VADD`, `VSUB`, `VMUL`, `VDIV`, `VSQRT`, `VCMP`, `VCVT` (int↔float),
   `VABS`, `VNEG`, `VMAXNM`, `VMINNM`, `VCVTR`, etc.
4. Double-precision arithmetic (`VADD.F64` …) — the D view is storage only.
5. Lazy FP context stacking: `LSPACT` / `FPCAR` / `FPCCR` are **not modeled**.
   Exception entry always stacks the standard 8-word frame immediately;
   there is no extended (26-word) FP frame. If an exception is entered while
   the FPU is active, the core reports this unsupported condition explicitly
   (one warning per core) instead of pretending FP context was preserved.
6. `CPACR` fields outside CP0-CP7 and CP10-CP11 (CP8/CP9, CP12/CP13)
   always read zero.

1. 单双精度之间的 `VCVT`（`VCVT.F64.F32`、`VCVT.F32.F64`）——延后，
   FPv5-SP 算术仅支持单精度。
2. `VMOV.F32 Sd, Sm`（浮点寄存器到浮点寄存器的复制）。
3. 全部浮点算术、比较与转换（Stage 3）：`VADD`、`VSUB`、`VMUL`、
   `VDIV`、`VSQRT`、`VCMP`、整数↔浮点 `VCVT`、`VABS`、`VNEG`、
   `VMAXNM`、`VMINNM`、`VCVTR` 等。
4. 双精度算术（`VADD.F64` 等）——D 视图仅作存储。
5. 惰性浮点上下文压栈：**不建模** `LSPACT` / `FPCAR` / `FPCCR`。异常进入
   始终立即压入标准 8 字帧，不存在 26 字扩展帧；若浮点单元处于活跃状态时
   进入异常，核心会显式报告这一未支持条件（每个核仅告警一次），而不是
   假装已经保存了浮点上下文。
6. CP0-CP7 与 CP10-CP11 之外的 CPACR 字段（CP8/CP9、CP12/CP13）恒读为 0。

---

## GPIO Coprocessor (CP0) and DCP (CP4) / GPIO 协处理器与 DCP

Thumb-32 coprocessor transfers are decoded after the FPU decoder:
`MCR`/`MRC` (`0xEE`) and `MCRR`/`MRRC` (`0xEC`) are claimed only when
`coproc` is 0 (GPIO coprocessor) or 4 (double-precision coprocessor).
Access is checked against the matching CPACR field through
`CortexM33Registers::cp_access_enabled`; denial rewinds PC by 4 and raises
a synchronous NOCP UsageFault, matching the FPU path. `Rt == 15` (and
`Rt2 == 15` for the 64-bit forms) is undefined.

`SCB->CPACR` now stores CP0-CP7 as well as CP10-CP11, using the mask
`0x00F0_FFFF` (`fpu::cpacr::MODELED_MASK`). CP0-CP7 occupy bits[15:0];
CP4 in particular is bits[9:8], so a low mask of `0x00FF` would only
cover CP0-CP3 and drop the DCP key. This preserves the
`CPACR = 0x00300303` value written by MicroPython/pico-sdk, which enables
CP10, CP4 and CP0.

Thumb-32 协处理器传输在 FPU 解码器之后处理：仅当 `coproc` 为 0（GPIO
协处理器）或 4（双精度协处理器）时才认领 `MCR`/`MRC`（`0xEE`）与
`MCRR`/`MRRC`（`0xEC`）。访问权限通过
`CortexM33Registers::cp_access_enabled` 检查对应的 CPACR 字段；拒绝访问
时 PC 回退 4 并触发同步 NOCP 用法故障，与 FPU 通路一致。`Rt == 15`
（64 位形式还包括 `Rt2 == 15`）视为未定义。

`SCB->CPACR` 现在保存 CP0-CP7 与 CP10-CP11，掩码为 `0x00F0_FFFF`
（`fpu::cpacr::MODELED_MASK`）。CP0-CP7 占用 bits[15:0]，其中 CP4 位于
bits[9:8]；若低半部分只用 `0x00FF`，就只会覆盖 CP0-CP3 并丢掉 DCP 键。
该掩码从而保留 MicroPython/pico-sdk 写入的 `CPACR = 0x00300303`
（使能 CP10、CP4 与 CP0）。

### CP0 GPIOC / CP0 GPIO 协处理器

CP0 maps onto the SIO GPIO registers at `0xD000_0000`; the address is kept
private in `execute_thumb32::coproc` to avoid a dependency on
`rp2350-periph`.

- `MCR p0, opc1, Rt, c0, CRm`: CRm 0/1/4/5 select GPIO_OUT, GPIO_HI_OUT,
  GPIO_OE and GPIO_HI_OE; opc1 0/1/2/3 write the plain, XOR, set and clear
  aliases. opc1 5/6/7 operate on one pin 0-63 (low/high selected by bit 5)
  using `1 << (pin & 31)`. opc1 4 and other CRm/CRn/opc2 combinations are
  undefined.
- `MRC p0, #0, Rt, c0, CRm`: CRm 0/1/4/5 read OUT/HI_OUT/OE/HI_OE; CRm
  8/9 read IN/HI_IN; other CRm values are undefined.
- `MCRR p0, opc1, Rt, Rt2, cX`: CRm 0/4 select the OUT and OE pairs.
  opc1 0-3 are paired put/XOR/set/clear, 4-7 are single-pin conditional
  operations, and 8-11 are indexed put/XOR/set/clear with Rt2 = 0 (low)
  or 1 (high).
- `MRRC p0, #0, Rt, Rt2, cX`: CRm 0/4/8 read OUT/OE/IN pairs into Rt/Rt2.
- Undefined opc1/CRm combinations return `StepResult::Undefined`.

CP0 直接映射到 `0xD000_0000` 处的 SIO GPIO 寄存器；该地址在
`execute_thumb32::coproc` 内保持私有，避免依赖 `rp2350-periph`。

- `MCR p0, opc1, Rt, c0, CRm`：CRm 0/1/4/5 选择 GPIO_OUT、
  GPIO_HI_OUT、GPIO_OE、GPIO_HI_OE；opc1 0/1/2/3 分别写普通、翻转、
  置位、清零别名。opc1 5/6/7 操作单个 0-63 引脚（由 bit 5 选择低/高
  寄存器），掩码为 `1 << (pin & 31)`。opc1 4 及其他 CRm/CRn/opc2
  组合未定义。
- `MRC p0, #0, Rt, c0, CRm`：CRm 0/1/4/5 读 OUT/HI_OUT/OE/HI_OE；
  CRm 8/9 读 IN/HI_IN；其他 CRm 未定义。
- `MCRR p0, opc1, Rt, Rt2, cX`：CRm 0/4 选择 OUT 与 OE 寄存器对。
  opc1 0-3 为成对普通写/翻转/置位/清零，4-7 为单引脚条件操作，
  8-11 为索引写/翻转/置位/清零，Rt2 = 0（低）或 1（高）。
- `MRRC p0, #0, Rt, Rt2, cX`：CRm 0/4/8 将 OUT/OE/IN 寄存器对读入
  Rt/Rt2。
- 未定义的 opc1/CRm 组合返回 `StepResult::Undefined`。

### CP4 DCP stub / CP4 DCP 桩

The DCP is modeled as permanently idle. `MRC`/`MRRC` return zero for the
destination registers, while `MCR`/`MCRR` accept and ignore their operands.
This is sufficient for pico-sdk's `runtime_init_per_core_enable_coprocessors`,
whose `mrc p4,#0,r0,c0,c0,#1` (opcode `0xEE100430`) clears the engaged flag.
No DCP operation, result register, engaged/busy signalling or `CDP` transfer
is modeled; those encodings remain undefined.

DCP 被建模为永远空闲。`MRC`/`MRRC` 将目标寄存器置零，`MCR`/`MCRR`
接受并忽略操作数。这足以执行 pico-sdk 的
`runtime_init_per_core_enable_coprocessors`：其中
`mrc p4,#0,r0,c0,c0,#1`（操作码 `0xEE100430`）用于清除 engaged 标志。
不建模任何 DCP 运算、结果寄存器、engaged/busy 信号或 `CDP` 传输；
这些编码仍视为未定义。

### Not implemented / 未实现清单

- `CDP`, `LDC`/`STC` and the `...2` coprocessor encodings.
- DCP arithmetic and status/engaged reporting (idle stub only).
- CP1-CP3 and CP5-CP7 remain unmodeled; their instructions are undefined.

- `CDP`、`LDC`/`STC` 及 `...2` 协处理器编码。
- DCP 算术与状态/engaged 上报（仅空闲桩）。
- CP1-CP3、CP5-CP7 未建模，其指令视为未定义。

---

## Exception Model / 异常模型

| Number | Exception | Priority | Description |
|--------|-----------|----------|-------------|
| 1 | Reset | -3 (highest) | Power-on reset |
| 2 | NMI | -2 | Non-maskable interrupt |
| 3 | HardFault | -1 | Unrecoverable fault |
| 4 | MemManage | configurable | MPU violation |
| 5 | BusFault | configurable | Bus error |
| 6 | UsageFault | configurable | Undefined instruction, etc. |
| 7 | SecureFault | configurable | TrustZone violation |
| 11 | SVCall | configurable | Supervisor call |
| 12 | DebugMon | configurable | Debug monitor |
| 14 | PendSV | configurable | Pendable service |
| 15 | SysTick | configurable | System tick timer |
| 16+ | IRQ(n) | configurable | External interrupts |

**Stack frame on exception entry** (8 words): R0-R3, R12, LR, PC, XPSR.
**EXC_RETURN** values: `0xFFFFFFF1` (return to handler), `0xFFFFFFF9` (thread MSP), `0xFFFFFFFD` (thread PSP).