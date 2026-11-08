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
| `execute_thumb32` | Thumb-32 decoder + executor stub |
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

## Thumb-32 Decoder (stub) / Thumb-32 解码器（桩）

Currently a stub that returns `Halted(UndefinedInstruction)`.
Full implementation (60+ ALU + Load/Store instructions) planned for Phase 2 completion.
当前为桩代码，返回 `Halted(UndefinedInstruction)`。完整实现计划于 Phase 2 完成。

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