# rp2350-arm — Public API / 公共 API

> ARM Cortex-M33 core implementation. Implements `CpuCore` trait.
> ARM Cortex-M33 核心实现。实现 `CpuCore` trait。

---

## Key Types / 关键类型

### `CortexM33Core`

```rust
pub struct CortexM33Core {
    pub index: usize,            // 0 or 1
    pub registers: CortexM33Registers,
    pub nvic: Nvic,
    pub systick: SysTick,
    pub mpu: Mpu,
    pub sau: Sau,
    pub fpu: VfpFpu,
    pub debug: CoreDebugState,
}

impl CortexM33Core {
    /// Create a new Cortex-M33 core.
    /// 创建新的 Cortex-M33 核心。
    pub fn new(index: usize) -> Self;

    /// Execute one Thumb instruction.
    /// 执行一条 Thumb 指令。
    pub fn execute_instruction(&mut self, chip: &mut RP2350Chip) -> StepResult;

    /// Fetch opcode at PC (16-bit or 32-bit Thumb).
    /// 在 PC 处取指（16 位或 32 位 Thumb）。
    pub fn fetch(&self, chip: &RP2350Chip) -> u32;

    /// Decode fetched opcode.
    /// 解码取出的操作码。
    pub fn decode(&self, opcode: u32) -> DecodedInstruction;

    /// Check and dispatch pending exceptions.
    /// 检查并分发挂起的异常。
    pub fn check_exceptions(&mut self) -> Option<ExceptionEntry>;

    /// Enter exception handler.
    /// 进入异常处理程序。
    pub fn enter_exception(&mut self, exception: ExceptionEntry);

    /// Return from exception (BX LR with EXC_RETURN).
    /// 从异常返回。
    pub fn exit_exception(&mut self, exc_return: u32);
}
```

### `CortexM33Registers`

```rust
pub struct CortexM33Registers {
    pub r: [u32; 13],       // R0-R12 — General purpose / 通用寄存器
    pub msp: u32,           // Main Stack Pointer / 主栈指针
    pub psp: u32,           // Process Stack Pointer / 进程栈指针
    pub lr: u32,            // Link Register (R14) / 链接寄存器
    pub pc: u32,            // Program Counter (R15) / 程序计数器
    pub xpsr: Xpsr,         // Program Status Register / 程序状态寄存器
    pub control: Control,   // CONTROL register / 控制寄存器
    pub primask: Primask,   // Priority Mask / 优先级屏蔽
    pub basepri: Basepri,   // Base Priority / 基础优先级
    pub faultmask: Faultmask, // Fault Mask / 故障屏蔽

    // Secure extension
    pub msp_ns: u32,        // Non-secure MSP
    pub psp_ns: u32,        // Non-secure PSP
    pub msp_lim: u32,       // MSP limit register
    pub psp_lim: u32,       // PSP limit register
}
```

### `Nvic`

```rust
pub struct Nvic {
    pub iser: [u32; 8],     // Interrupt Set-Enable / 中断使能设置 (256 IRQs)
    pub icer: [u32; 8],     // Interrupt Clear-Enable / 中断使能清除
    pub ispr: [u32; 8],     // Interrupt Set-Pending / 中断挂起设置
    pub icpr: [u32; 8],     // Interrupt Clear-Pending / 中断挂起清除
    pub iabr: [u32; 8],     // Interrupt Active Bit
    pub ipr: [u8; 256],     // Interrupt Priority / 中断优先级 (8-bit each)
    pub stir: u32,          // Software Trigger Interrupt / 软件触发中断
}

impl Nvic {
    pub fn set_pending(&mut self, irq: u32);
    pub fn clear_pending(&mut self, irq: u32);
    pub fn is_pending(&self, irq: u32) -> bool;
    pub fn set_priority(&mut self, irq: u32, priority: u8);
    pub fn get_priority(&self, irq: u32) -> u8;
    pub fn get_highest_pending(&self, current_priority: u8) -> Option<u32>;
    pub fn set_enabled(&mut self, irq: u32, enabled: bool);
}
```

### `VfpFpu`

```rust
pub struct VfpFpu {
    pub s_regs: [f32; 32],  // S0-S31 single-precision / 单精度
    pub fpscr: Fpscr,       // FP Status and Control Register
}

impl VfpFpu {
    /// VADD, VSUB, VMUL, VNMUL, VDIV
    pub fn execute_data_processing(&mut self, opcode: u32) -> FpuResult;
    /// VLDR, VSTR
    pub fn execute_load_store(&mut self, opcode: u32, chip: &mut RP2350Chip) -> FpuResult;
    /// VMOV (immediate), VMOV (register), VCVT
    pub fn execute_move(&mut self, opcode: u32) -> FpuResult;
    /// VCMP, VCMPE
    pub fn execute_compare(&mut self, opcode: u32) -> FpuResult;
    /// VSQRT
    pub fn execute_sqrt(&mut self, opcode: u32) -> FpuResult;
}
```

---

## Instruction Decoding / 指令解码

```rust
pub enum DecodedInstruction {
    Thumb16(Thumb16Op),
    Thumb32(Thumb32Op),
    Fpu(FpuOp),
    Dsp(DspOp),
    Undefined,
}

// ~90 instructions
pub enum Thumb16Op {
    // Data processing: MOV, ADD, SUB, CMP, AND, ORR, EOR, LSL, LSR, ASR, ROR, ...
    // Branch: B, Bcc, BLX, BX
    // Load/Store: LDR, STR, LDRH, STRH, LDRB, STRB, LDMIA, STMIA
    // Miscellaneous: NOP, BKPT, SVC, CPS, ...
}

// ~120 instructions (Thumb-32 + DSP)
pub enum Thumb32Op {
    // Data processing (32-bit): MOVW, MOVT, ADDW, SUBW, ...
    // DSP: SADD8, SSUB8, USAD8, USADA8, SADD16, SSUB16, QADD, QSUB, ...
    // Saturating: SSAT, USAT, QDADD, QDSUB
    // Bitfield: BFC, BFI, SBFX, UBFX, RBIT, REV, CLZ
    // Multiply: MUL, MLA, MLS, SMULL, UMULL, SMLAL, UMLAL, SDIV, UDIV
    // SIMD: SADD8, SHADD16, UHADD16, ...
    // Load/Store (32-bit): LDR.W, STR.W, LDRD, STRD, ...
}
```

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