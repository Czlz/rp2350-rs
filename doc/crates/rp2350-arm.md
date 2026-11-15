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
| `dual_core` | `DualCoreM33` two-core scheduling scaffold (core1 release gate) |
| `conditions` | `Condition` (15 codes), `ITBlockState` (1-4 instruction blocks) |
| `execute_thumb16` | Thumb-16 decoder + executor (~90 instructions) |
| `execute_thumb32` | Thumb-32 decoder + executor (60+ ALU / Load-Store / system instructions) |
| `execute_thumb32::dsp` | Packed SIMD add/subtract (`SADD8` … `USAX`), halving (`SH*`/`UH*`), saturating `Q*`/`UQ*`, `QADD`/`QSUB`/`QDADD`/`QDSUB`, `SSAT`/`USAT`/`SSAT16`/`USAT16`, `SEL` |
| `execute_thumb32::multiply` | `MUL`/`MLA`/`MLS`, long multiplies, divides, and the DSP multiply family (`SMULxy`, `SMULW*`, `SMLAW*`, `SMMUL`/`SMMLA`/`SMMLS`, `SMLAD`/`SMLSD`/`SMUAD`/`SMUSD`, `SMLALD`/`SMLSLD`, `USAD8`) |
| `execute_thumb32::coproc` | CP0 GPIO coprocessor and CP4 DCP stub (`MCR`/`MRC`/`MCRR`/`MRRC`) |
| `fpu` | FPv5-SP state: S0-S31 / D0-D15 views, FPSCR, FPEXC, FPCCR/FPCAR/FPDSCR, CPACR fields |
| `nvic` | NVIC: ISER/ICER/ISPR/ICPR/IABR/IPR/STIR, priority routing |
| `systick` | SysTick 24-bit down counter with reload + interrupt |
| `exceptions` | Exception entry/exit, EXC_RETURN decoding, standard and extended (lazy FP) stack frames |

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
    pub msplim: u32,        // Main Stack Pointer limit / 主栈指针下限
    pub psplim: u32,        // Process Stack Pointer limit / 进程栈指针下限
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
    // APSR.Q has no getter; read `xpsr & (1 << 27)` directly.
    // APSR.Q 无 getter，直接读取 `xpsr & (1 << 27)`。
    pub fn set_q_flag(&mut self);
    pub fn apsr_ge(&self) -> u32;             // APSR[19:16]
    pub fn set_apsr_nzcv(&mut self, n: bool, z: bool, c: bool, v: bool);
    pub fn set_apsr_q(&mut self, q: bool);
    pub fn set_apsr_ge(&mut self, ge: u32);

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

## Sleep and events (`WFI` / `WFE` / `SEV`)
## 睡眠与事件（`WFI` / `WFE` / `SEV`）

### Event register / 事件寄存器

Every core owns one event bit in `CortexM33Registers` (reset value `false`); it
is never shared between the two cores.
每个核心在 `CortexM33Registers` 中拥有独立的事件位（复位为 `false`），两个
核心之间绝不共享。

| Instruction | Effect / 作用 |
|-------------|---------------|
| `SEV` | Sets the local event bit **and** latches a cross-core signal (`take_sev_signal`). Returns `Ok`. / 置本核事件位，并锁存一个跨核信号（`take_sev_signal`）。返回 `Ok`。 |
| `SEVL` | Sets only the local event bit. Returns `Ok`. / 仅置本核事件位。返回 `Ok`。 |
| `WFE` | Event bit set → clear it and continue (`Ok`); otherwise decode returns `Sleep`. / 事件位已置 → 清除并继续（`Ok`）；否则解码返回 `Sleep`。 |
| `WFI` | Always returns `Sleep`; the event bit is irrelevant (ARMv8-M). / 总是返回 `Sleep`，与事件位无关（ARMv8-M）。 |

`CortexM33Core` public entry points / `CortexM33Core` 公共入口：

```rust
impl CortexM33Core {
    pub fn set_event(&mut self);           // peer SEV / 对端 SEV
    pub fn has_event(&self) -> bool;
    pub fn clear_event(&mut self);
    pub fn take_sev_signal(&mut self) -> bool;  // true at most once / 最多一次 true
    pub fn has_pending_interrupt(&self) -> bool; // enabled+pending, PRIMASK ignored
}
```

The decoder stores the wait kind (`WFE` vs `WFI`) so a late event wakes a `WFE`
but never a `WFI`.

解码器记住等待类型（`WFE` 或 `WFI`），因此后到的事件可以唤醒 `WFE`，但绝不会
唤醒 `WFI`。

### `Sleep` and wake-up / 睡眠与唤醒

`step_instruction` turns a `Sleep` result into `self.state = CpuState::Sleeping`
**after** the hint retired, so `PC` already points past `WFI`/`WFE`. If a wake
condition was already present, the instruction completes immediately and the
core stays `Running`; a pending enabled exception is then taken in the same
step instead of after one more instruction.

`step_instruction` 在提示指令退休**之后**把 `Sleep` 结果转成
`self.state = CpuState::Sleeping`，因此 `PC` 已指向 `WFI`/`WFE` 之后。若唤醒
条件已满足，该指令立即完成、核心保持 `Running`，并且挂起的已使能异常会在同一次
`step` 中进入，而不是再执行一条指令之后。

`step()` / `run_cycles()` first consult `try_wake()` while sleeping:
`step()` / `run_cycles()` 在睡眠时先调用 `try_wake()`：

1. an enabled pending exception in the NVIC (`find_highest_priority`), or
2. the event bit when the wait came from `WFE` (the event is consumed).

If neither holds, no instruction executes and the core keeps reporting
`CpuState::Sleeping` (the driver decides whether time should advance). Wake-ups
themselves do not consume the instruction budget.

两者都不满足时不执行指令，核心持续报告 `CpuState::Sleeping`（由驱动决定是否推进
时间）。唤醒本身不消耗指令预算。

### Cross-core `SEV` / 跨核 `SEV`

The core does not know about its peer. The driver forwards
`core.take_sev_signal()` to the other core's `set_event()`, which is what makes a
`SEV`/`WFE` pair work across cores without shared state.

核心不知道对端存在。驱动器把 `core.take_sev_signal()` 转发到另一核心的
`set_event()`，跨核 `SEV`/`WFE` 配对因此无需共享状态即可工作。

### Limitations / 剩余限制

- No clock gating, no low-power state and no wake-up latency: a sleeping core is
  simply not stepped, and simulated time only advances when the driver says so.
  没有时钟门控、低功耗状态与唤醒延迟：睡眠核心只是不被推进，模拟时间仅在驱动
  要求时前进。
- `SEVONPEND` is not modelled: only *enabled* pending exceptions wake a wait.
  `SEVONPEND` 未建模：只有**已使能**且挂起的中断才能唤醒等待。
- `DualCoreM33` (the scaffold) still treats `Sleeping` as "not running" and does
  not wake core1; the CLI `Machine` implements the wake-up policy instead.
  `DualCoreM33`（脚手架）仍把 `Sleeping` 视为「未运行」，也不会唤醒 core1；
  唤醒策略由 CLI `Machine` 实现。

## Thumb-32 Decoder / Thumb-32 解码器

The Thumb-32 decoder is fully implemented (see `execute_thumb32`): 60+ ALU,
load/store and miscellaneous system instructions, plus the FPU and
coprocessor chapters below.

DSP coverage / DSP 覆盖清单:

- **Halfword multiply**: `SMULBB/SMULBT/SMULTB/SMULTT`, their `SMLA*`
  accumulating forms (including the `Q` overflow flag), the
  `SMMUL`/`SMMULR`/`SMMLA`/`SMMLAR` high-word modular forms, the wide
  `SMULWB`/`SMULWT`/`SMLAWB`/`SMLAWT` forms (top word of a 32×16 product,
  `Q` set when the 48-bit accumulation overflows), and the dual forms
  `SMLAD`/`SMLADX`/`SMUAD`/`SMUADX` (add) plus
  `SMLSD`/`SMLSDX`/`SMUSD`/`SMUSDX` (subtract). `X` rotates the second
  operand by 16 bits, so the products are top×bottom and bottom×top.
  These set `APSR.Q` exactly once, when the final infinite-precision sum does
  not fit in 32 bits — `SMUAD`/`SMUSD` (`Ra = 1111`) included. The long dual
  forms `SMLALD`/`SMLALDX`/`SMLSLD`/`SMLSLDX` accumulate into `RdHi:RdLo`
  and wrap modulo 2^64 without touching any flag.
- **Most-significant-word multiply subtract**: `SMMLS`/`SMMLSR` compute
  `(Ra<<32 - Rn*Rm)` and keep the high word, adding the rounding constant
  `0x8000_0000` before the extraction for `SMMLSR`.
- **Packed parallel add/subtract** (`execute_thumb32::dsp`):
  `SADD8/SSUB8/SADD16/SSUB16`, `UADD8/USUB8/UADD16/USUB16` and the
  exchanged forms `SASX/SSAX/UASX/USAX`. Each byte/halfword lane is
  computed independently and wraps modulo 2^8/2^16. `APSR.GE` receives one
  flag per byte lane, or a duplicated pair per halfword lane (`GE[1:0]` for
  bits[15:0], `GE[3:2]` for bits[31:16]). Signed variants set GE when the
  full-width sum or difference is `>= 0`; unsigned variants set GE on
  carry-out (add) or no-borrow (subtract). N, Z, C, V and Q are unchanged.
- **Halving parallel add/subtract**: `SHADD8/SHSUB8/SHADD16/SHSUB16`,
  `UHADD8/UHSUB8/UHADD16/UHSUB16`, `SHASX/SHSAX` and `UHASX/UHSAX`. Each lane
  writes `(a ± b) >> 1` (bits `[n:1]` of the full-width result, so a negative
  result shifts arithmetically) and wraps into its own lane width. Unlike the
  wrapping `S*`/`U*` group, the halving group updates **no flag at all**:
  `GE`, `Q` and `NZCV` are all preserved (ARM ARM `SHADD8` … `UHSUB16`).
- **Saturating parallel add/subtract**: `QADD8/QSUB8/QADD16/QSUB16`,
  `UQADD8/UQSUB8/UQADD16/UQSUB16`, `QASX/QSAX` and `UQASX/UQSAX`. Each lane
  saturates to the signed `[-2^(n-1), 2^(n-1)-1]` or unsigned
  `[0, 2^n-1]` range instead of wrapping; a negative value saturates to
  zero for the unsigned forms. Any saturated lane sets the sticky `APSR.Q`;
  GE and NZCV are left untouched.
- **32-bit saturating arithmetic**: `QADD` (`Rd = Sat(Rn + Rm)`), `QSUB`
  (`Rd = Sat(Rn - Rm)`), `QDADD` (`Rd = Sat(Rn + Sat(2*Rm))`) and `QDSUB`
  (`Rd = Sat(Rn - Sat(2*Rm))`). `QDADD`/`QDSUB` saturate the doubling first,
  and either saturation sets `APSR.Q`. Only `Q` is modified.
- **Saturation**: `SSAT`/`SSAT16` (`N = sat_imm + 1`, 1-32 signed bits) and
  `USAT`/`USAT16` (`N = sat_imm`, 0-31 unsigned bits), with the
  `lsl`/`asr` immediate shift (`imm3:imm2`, bit 21 selects `asr`). The
  16-bit forms saturate both halfwords independently. Saturated lanes are
  sign-extended to the lane width or to 32 bits, and any saturation sets
  `APSR.Q`. `USAT`/`USAT16` treat the input as signed and clamp negative
  values to zero.
- **Byte select**: `SEL` copies each byte from `Rn` when the matching
  `APSR.GE` bit is set and from `Rm` otherwise; it modifies no flag.
- **Packed byte extend**: `SXTB16/UXTB16` and the accumulating
  `SXTAB16/UXTAB16` forms, all with the `ror #0/#8/#16/#24` rotate field.
  The accumulating forms add the two extended bytes to the matching halfwords
  of `Rn` independently, so each 16-bit lane wraps on its own. Flags are
  unchanged.
- **Sum of absolute differences**: `USAD8` (Ra = 1111) and `USADA8`
  (accumulate into Ra). Flags are unchanged.

Flag rules / 标志规则 (ARM ARM):

| Group | `APSR.GE` | `APSR.Q` | `N/Z/C/V` |
|-------|-----------|----------|-----------|
| `SADD*`/`SSUB*`/`UADD*`/`USUB*`, `SASX`/`SSAX`/`UASX`/`USAX` | written (per lane) | unchanged | unchanged |
| `SHADD*`/`SHSUB*`/`SHASX`/`SHSAX`, `UHADD*`/`UHSUB*`/`UHASX`/`UHSAX` | unchanged | unchanged | unchanged |
| `QADD8`…`UQSAX` | unchanged | set when saturated (sticky) | unchanged |
| `QADD`/`QSUB`/`QDADD`/`QDSUB` | unchanged | set when saturated (sticky) | unchanged |
| `SSAT`/`USAT`/`SSAT16`/`USAT16` | unchanged | set when saturated (sticky) | unchanged |
| `SMLAxy`, `SMLAWy`, `SMLAD`/`SMLADX`, `SMLSD`/`SMLSDX`, `SMUAD`/`SMUADX`, `SMUSD`/`SMUSDX` | unchanged | set on 32-bit overflow (sticky) | unchanged |
| `SMLALD`/`SMLALDX`, `SMLSLD`/`SMLSLDX`, `SMULxy`, `SMULWy`, `SMMUL`/`SMMLA`/`SMMLS`, `USAD8`/`USADA8`, `SXTAB16`/`UXTAB16` | unchanged | unchanged | unchanged |
| `SEL` | read only | unchanged | unchanged |

`Q` is cumulative: a saturating or overflowing instruction only ever sets it,
so software clears it explicitly (`MSR APSR_nzcvq`) before a sequence and tests
it once afterwards. GE never participates in the `GE`/`LT` condition codes
(those still test `N == V`). The accumulate-then-check rule matters for
`SMLAD`: the products `0x4000_0000 + 0x4000_0000 - 1` do **not** set `Q`,
because the architecturally checked value is the single infinite-precision
sum `0x7FFF_FFFF`.

仍未实现 / still undefined: `SMLALxy` (`SMLALBB`/`SMLALBT`/`SMLALTB`/`SMLALTT`,
the 64-bit accumulating halfword products). Everything else in the DSP
add/subtract, halving, saturating and multiply chapters that ARMv8-M defines
for Cortex-M33 is decoded.

仍未实现：`SMLALxy`（`SMLALBB`/`SMLALBT`/`SMLALTB`/`SMLALTT`，即 64 位累加的
半字乘积）。DSP 加/减、折半、饱和与乘法章节中 ARMv8-M 为 Cortex-M33 定义的
其余编码均已解码。

`SSAT`/`USAT`/`SSAT16`/`USAT16` live in the data-processing
(plain-binary-immediate) group; the top-level decoder routes
`bits[26:20] = 0x30/0x32/0x38/0x3A` to `dsp::execute_saturate` before the
bitfield group, keeping the existing dispatch layering. These four mnemonics
have **only** the 32-bit T1 encoding — ARMv8-M defines no 16-bit Thumb form
(`arm-none-eabi-as` rejects them for Thumb-only cores), so there is no
Thumb-16 `SSAT`/`USAT` gap to fill.

`SSAT`/`USAT`/`SSAT16`/`USAT16` 属于数据处理（纯二进制立即数）组；顶层解码器在
位域组之前把 `bits[26:20] = 0x30/0x32/0x38/0x3A` 分派给
`dsp::execute_saturate`，保持既有分派层次。这四条助记符**只有** 32 位 T1
编码 —— ARMv8-M 没有定义 16 位 Thumb 形式（`arm-none-eabi-as` 对仅支持 Thumb
的内核会直接拒绝），因此不存在待补齐的 Thumb-16 `SSAT`/`USAT`。

`APSR.GE` is stored in `xpsr[19:16]` (`apsr_ge`/`set_apsr_ge`), is
preserved by `set_apsr_nzcv` and exception stacking, and is readable and
writable through `MRS APSR` / `MSR APSR_g` / `MSR APSR_nzcvqg`.

Thumb-32 解码器已完整实现（见 `execute_thumb32`）：60+ 条 ALU、Load/Store
与杂项系统指令，以及下文的 FPU 与协处理器章节。

DSP 覆盖清单：

- **半字乘法**：`SMULBB/SMULBT/SMULTB/SMULTT`、对应的 `SMLA*` 累加形式
  （含 `Q` 溢出标志）、`SMMUL`/`SMMULR`/`SMMLA`/`SMMLAR` 高字模乘、宽形式
  `SMULWB`/`SMULWT`/`SMLAWB`/`SMLAWT`（32×16 乘积取高字，48 位累加上溢时置
  `Q`），以及双乘形式 `SMLAD`/`SMLADX`/`SMUAD`/`SMUADX`（加）与
  `SMLSD`/`SMLSDX`/`SMUSD`/`SMUSDX`（减）。`X` 把第二操作数循环右移 16 位，
  乘积为高×低与低×高。这些指令只在最终无限精度和不落入 32 位时置一次
  `APSR.Q`（`Ra = 1111` 的 `SMUAD`/`SMUSD` 同样置位）。长双乘形式
  `SMLALD`/`SMLALDX`/`SMLSLD`/`SMLSLDX` 累加到 `RdHi:RdLo`，按模 2^64
  回绕且不修改任何标志。
- **高字乘减**：`SMMLS`/`SMMLSR` 计算 `(Ra<<32 - Rn*Rm)` 并取高字，
  `SMMLSR` 在取高字前加舍入常量 `0x8000_0000`。
- **打包并行加/减**（`execute_thumb32::dsp`）：
  `SADD8/SSUB8/SADD16/SSUB16`、`UADD8/USUB8/UADD16/USUB16` 以及交换形式
  `SASX/SSAX/UASX/USAX`。每个字节/半字通道独立运算并按 2^8/2^16 回绕。
  `APSR.GE` 为每个字节通道写入一个标志，半字通道写入一对重复标志
  （bits[15:0] 对应 `GE[1:0]`，bits[31:16] 对应 `GE[3:2]`）。有符号变体在
  完整宽度和/差大于等于零时置 GE；无符号变体在加法产生进位、减法无借位时
  置 GE。N、Z、C、V、Q 不变。
- **折半并行加/减**：`SHADD8/SHSUB8/SHADD16/SHSUB16`、
  `UHADD8/UHSUB8/UHADD16/UHSUB16`、`SHASX/SHSAX` 与 `UHASX/UHSAX`。每个通道
  写入 `(a ± b) >> 1`（即完整宽度结果的 `[n:1]` 位，负值按算术右移），并按
  各自通道宽度回绕。与回绕的 `S*`/`U*` 组不同，折半组**不修改任何标志**：
  `GE`、`Q` 与 `NZCV` 全部保持（ARM ARM `SHADD8` … `UHSUB16`）。
- **饱和并行加/减**：`QADD8/QSUB8/QADD16/QSUB16`、
  `UQADD8/UQSUB8/UQADD16/UQSUB16`、`QASX/QSAX` 与 `UQASX/UQSAX`。每个通道
  饱和到有符号 `[-2^(n-1), 2^(n-1)-1]` 或无符号 `[0, 2^n-1]` 范围而非回绕；
  无符号形式的负值饱和为零。任一通道饱和即置保持型 `APSR.Q`，GE 与 NZCV
  不变。
- **32 位饱和算术**：`QADD`（`Rd = Sat(Rn + Rm)`）、`QSUB`
  （`Rd = Sat(Rn - Rm)`）、`QDADD`（`Rd = Sat(Rn + Sat(2*Rm))`）与 `QDSUB`
  （`Rd = Sat(Rn - Sat(2*Rm))`）。`QDADD`/`QDSUB` 先饱和加倍值，任一次饱和
  都置 `APSR.Q`；只修改 `Q`。
- **饱和**：`SSAT`/`SSAT16`（`N = sat_imm + 1`，1-32 位有符号）与
  `USAT`/`USAT16`（`N = sat_imm`，0-31 位无符号），支持 `lsl`/`asr` 立即数
  移位（`imm3:imm2`，位 21 选择 `asr`）。16 位形式对两个半字独立饱和。
  饱和后的通道按符号扩展到通道宽度或 32 位，任一次饱和都置 `APSR.Q`。
  `USAT`/`USAT16` 把输入视为有符号数，负值钳制为零。
- **字节选择**：`SEL` 在对应 `APSR.GE` 位置位时从 `Rn` 复制该字节，否则取
  `Rm`；不修改任何标志。
- **打包字节扩展**：`SXTB16/UXTB16` 及累加形式 `SXTAB16/UXTAB16`，均支持
  `ror #0/#8/#16/#24` rotate 字段。累加形式把扩展后的两个字节分别加到 `Rn`
  的对应半字上，每个 16 位通道独立回绕。不更新标志。
- **绝对差之和**：`USAD8`（Ra = 1111）与 `USADA8`（累加到 Ra），
  不更新标志。

标志规则（ARM ARM）见上文英文表格：并行 `S*`/`U*` 与交换形式写 GE；
折半 `SH*`/`UH*` 组不写任何标志；饱和形式只置保持型 Q；乘加类
（`SMLAxy`、`SMLAWy`、`SMLAD`/`SMLSD`/`SMUAD`/`SMUSD`）在 32 位上溢时置 Q；
`SEL` 只读 GE。`Q` 为累积位，饱和/上溢指令只会置位，因此软件需先用
`MSR APSR_nzcvq` 显式清除，执行完序列后再统一测试。
GE 绝不参与 `GE`/`LT` 条件码（后者仍比较 `N == V`）。累加后再判断的规则对
`SMLAD` 尤其重要：乘积 `0x4000_0000 + 0x4000_0000 - 1` **不会**置 `Q`，
因为架构检查的是最终那一个无限精度和 `0x7FFF_FFFF`。

仍未实现：`SMLALxy`（`SMLALBB`/`SMLALBT`/`SMLALTB`/`SMLALTT`，即 64 位累加的
半字乘积）。DSP 加/减、折半、饱和与乘法章节中 ARMv8-M 为 Cortex-M33 定义的
其余编码均已解码。

`APSR.GE` 存放于 `xpsr[19:16]`（`apsr_ge`/`set_apsr_ge`），
`set_apsr_nzcv` 与异常压栈都会保留它，并可通过 `MRS APSR` /
`MSR APSR_g` / `MSR APSR_nzcvqg` 读写。

### Exclusive Monitor / 独占监视器

`LDREX`/`LDREXB`/`LDREXH`/`LDREXD`, their `LDAEX*` acquire forms and the
`STREX*`/`STLEX*` stores share one monitor **per core**, held in
`CortexM33Registers` as `exclusive: Option<(u32, u8)>` — watched address plus
transfer width in bytes. `arm_exclusive` / `consume_exclusive` /
`clear_exclusive` are the only entry points, and `CortexM33Registers::new()`
starts disarmed (`None`). Firmware sees the architectural contract:

- `LDREX*` overwrites the monitor with its own address and width; a second
  load never keeps the first.
- `STREX*` succeeds (`Rd = 0`) **only** when address *and* width both match,
  and clears the monitor either way. Failure writes `Rd = 1` and performs no
  store; `Rd == 15` discards the status. `STREXB` therefore fails against a
  word `LDREX` on the same address.
- `CLREX` (`0xF3BF_8F2F`, selector `2` in bits[7:4] with option `1111`) clears
  the executing core's monitor and returns `Ok`. `DSB`/`DMB`/`ISB` remain
  ordering-only no-ops.

Because the monitor lives in the register file rather than in a
`thread_local!`, the two RP2350 cores — advanced in sequence on one host
thread — cannot observe each other's exclusive accesses: `LDREX` on core0
followed by `STREX` on core1 fails, as the hardware requires, and `CLREX` on
one core leaves the other armed. Sharing one monitor through thread-local
state was a real correctness bug, and `CLREX` used to fall through the barrier
group as an undefined instruction. `CpuCore::reset`
and `boot_at` rebuild the register file, so the monitor starts disarmed exactly
like a hardware reset.

Not modelled: **bus snooping** — a write by another core to the watched
address does not clear the monitor, so a `STREX` that hardware would reject
can still succeed; acquire/release ordering (these instructions execute as
plain loads/stores); and monitor clearing on exception entry/return. If a
firmware ever depends on snooping, a shared generation counter bumped by every
store could approximate it.

`LDREX`/`LDREXB`/`LDREXH`/`LDREXD` 及其 `LDAEX*` 获取形式与 `STREX*`/`STLEX*`
存储共享**每核一个**监视器，保存在 `CortexM33Registers` 的
`exclusive: Option<(u32, u8)>` 中——被监视地址加按字节计的传输宽度。
`arm_exclusive` / `consume_exclusive` / `clear_exclusive` 是唯一入口，
`CortexM33Registers::new()` 初始为未武装（`None`）。固件可见的架构契约：

- `LDREX*` 用自身的地址与宽度覆盖监视器；第二次加载不会保留第一次的。
- `STREX*` **仅当**地址与宽度同时匹配才成功（`Rd = 0`），且无论成败都清除
  监视器。失败写入 `Rd = 1` 且不产生存储；`Rd == 15` 丢弃状态。因此同地址上
  的 `STREXB` 对字 `LDREX` 必然失败。
- `CLREX`（`0xF3BF_8F2F`，bits[7:4] 选择子为 `2`、选项字段 `1111`）清除正在
  执行核心的监视器并返回 `Ok`。`DSB`/`DMB`/`ISB` 仍是仅影响顺序的无操作。

由于监视器位于寄存器文件而非 `thread_local!`，在同一宿主线程中顺序推进的两个
RP2350 核心无法看到彼此的独占访问：core0 的 `LDREX` 之后 core1 的 `STREX`
必然失败，与硬件一致；一个核心执行 `CLREX` 也不会解除另一个核心的武装。用
线程局部状态共享单个监视器曾是真实的正确性缺陷，且 `CLREX` 此前落入屏障分组
被判为未定义指令（启动固件会打印告警）。`CpuCore::reset` 与 `boot_at` 会重建
寄存器文件，因此监视器与硬件复位一样从解除武装开始。

未建模：**总线监听** —— 另一核心对被监视地址的写入不会清除监视器，因此硬件本
应拒绝的 `STREX` 仍可能成功；获取/释放的顺序保证（这些指令按普通加载/存储
执行）；以及异常进入/返回时的监视器清除。若某固件依赖总线监听，可用由每次
存储递增的共享世代计数近似模拟。

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
- **Arithmetic** (single precision, Stage 3, `execute_thumb32::fpu_arith`):
  `VADD`, `VSUB`, `VMUL`, `VNMUL`, `VMLA`/`VMLS`, `VNMLA`/`VNMLS`, `VDIV`,
  the fused `VFMA`/`VFMS`/`VFNMA`/`VFNMS` family, plus `VMOV.F32 Sd, Sm`,
  `VABS`, `VNEG` and `VSQRT`. Fused ops use `f32::mul_add`, so the
  multiply-add has a single rounding like the hardware; `VMLA`/`VMLS` keep
  the two roundings the architecture specifies. `VABS`/`VNEG` are pure
  sign-bit operations, so NaN payloads survive unchanged.
- **Comparison**: `VCMP`/`VCMPE` against a register or `#0.0`. FPSCR
  `N`/`Z`/`C`/`V` follow ARM semantics (equal: `Z`+`C`; less: `N`;
  greater: `C`; unordered: `C`+`V`) and reach APSR through the existing
  `VMRS APSR_nzcv, FPSCR`.
- **Conversion**: `VCVT.F32.S32`, `VCVT.F32.U32`, `VCVT.S32.F32`,
  `VCVT.U32.F32` and the `VCVTR` forms. `VCVT` float→integer truncates
  toward zero as the architecture specifies (`VCVTR` rounds to nearest, ties
  to even); NaN yields 0, out-of-range values yield the integer indefinite
  value, and both set `IOC`.
- **Conditional select**: `VSELEQ`, `VSELVS`, `VSELGE` and `VSELGT` in the
  `0xFE` encoding space, driven by the FPSCR condition flags. The selected
  register is copied bit-for-bit. MicroPython's `mp_format_float` relies on
  this when printing floats.
- **Round to integral**: `VRINTR`, `VRINTZ`, `VRINTX` and the directed
  `VRINTA`/`VRINTN`/`VRINTP`/`VRINTM` forms. The result stays in a
  single-precision register and no cumulative exception bits are updated.
- **FPSCR modeling**: `IOC` (invalid operation), `DZC` (divide by zero),
  `OFC | IXC` (overflow to infinity) and `IXC` (inexact conversion) are set.
  Underflow (`UFC`), flush-to-zero (`FZ`), default NaN (`DN`), `AHP`,
  `FPSCR.RMode` values other than round-to-nearest-even, and the trap-enable
  bits are **not** modeled; NaN propagation follows IEEE-754 quiet-NaN rules.
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
- **Lazy / extended exception frames** (see the dedicated section below):
  `FPCCR`, `FPCAR` and `FPDSCR` are modeled at `0xE000EF34`, `0xE000EF38`
  and `0xE000EF3C`; exception entry reserves or stacks the 17-word FP
  context according to `FPCCR.ASPEN`/`LSPEN`, and exception return reclaims
  or restores it.

- **状态**：S0-S31，`D0-D15` 为 S 对别名；`FPSCR`、`FPEXC`；`CPACR`
  CP0-CP7 与 CP10-CP11 读写。
- **数据搬移**：`VLDR`/`VSTR`（S/D、PC 相对字面量、正负 8 位字偏移）、
  `VLDM`/`VSTM`（含 `VPUSH`/`VPOP` 别名，IA/DB，可带写回）、
  `VMOV` 内核↔S、`VMOV` 两个内核↔D、`VMOV.F32 Sd, #imm`、
  `VMRS`/`VMSR`（含 `VMRS APSR_nzcv, FPSCR`）。
- **算术**（单精度，Stage 3，`execute_thumb32::fpu_arith`）：`VADD`、
  `VSUB`、`VMUL`、`VNMUL`、`VMLA`/`VMLS`、`VNMLA`/`VNMLS`、`VDIV`、
  融合乘加 `VFMA`/`VFMS`/`VFNMA`/`VFNMS`，以及 `VMOV.F32 Sd, Sm`、
  `VABS`、`VNEG`、`VSQRT`。融合指令使用 `f32::mul_add`，乘加只做一次
  舍入，与硬件一致；`VMLA`/`VMLS` 按架构规定保留两次舍入。
  `VABS`/`VNEG` 是纯符号位操作，NaN 载荷原样保留。
- **比较**：`VCMP`/`VCMPE`，操作数可为寄存器或 `#0.0`。FPSCR 的
  `N`/`Z`/`C`/`V` 遵循 ARM 语义（相等：`Z`+`C`；小于：`N`；大于：`C`；
  无序：`C`+`V`），并通过现有的 `VMRS APSR_nzcv, FPSCR` 传到 APSR。
- **转换**：`VCVT.F32.S32`、`VCVT.F32.U32`、`VCVT.S32.F32`、
  `VCVT.U32.F32` 以及 `VCVTR` 形式。`VCVT` 浮点→整数按架构规定向零
  截断（`VCVTR` 为就近舍入、偶数优先）；NaN 返回 0，越界返回整数
  不确定值，两种情况都置 `IOC`。
- **条件选择**：`0xFE` 编码空间的 `VSELEQ`、`VSELVS`、`VSELGE`、`VSELGT`，
  由 FPSCR 条件标志驱动，被选中的寄存器按位复制。MicroPython 的
  `mp_format_float` 打印浮点数时依赖该指令。
- **取整**：`VRINTR`、`VRINTZ`、`VRINTX` 以及定向取整
  `VRINTA`/`VRINTN`/`VRINTP`/`VRINTM`。结果仍为单精度浮点，不更新
  累积异常位。
- **FPSCR 建模**：置 `IOC`（无效操作）、`DZC`（除零）、`OFC | IXC`
  （上溢为无穷）与 `IXC`（不精确转换）。**不建模** `UFC`（下溢）、
  `FZ` 清零模式、`DN` 默认 NaN、`AHP`、`FPSCR.RMode` 除就近偶数之外的
  取值以及陷阱使能位；NaN 传播遵循 IEEE-754 静默 NaN 规则。
- **使能与故障**：每条浮点编码先检查 CPACR。拒绝访问时走真实异常通路：
  置 `CFSR.UFSR.NOCP`，异常号 6，按 CONTROL 在 MSP/PSP 上压标准栈帧，
  `EXC_RETURN` 为 0xFFFFFFF9 / 0xFFFFFFFD / 0xFFFFFFF1；压栈 PC 为
  引发故障的指令（进入前已回退 PC）。

  由于本模拟器直接从映像入口开始（不执行掩膜 ROM 中的 boot ROM），
  启动时复现 boot ROM 交付状态，使 CP10/CP11 保持使能。上电寄存器文件
  的 CPACR 仍为清零状态；把 CPACR 写 0 也会被完全遵守，因此 NOCP
  通路可达且已被测试覆盖。
- **惰性 / 扩展异常帧**（详见下一节）：在 `0xE000EF34`、`0xE000EF38`、
  `0xE000EF3C` 建模 `FPCCR`、`FPCAR`、`FPDSCR`；异常进入按
  `FPCCR.ASPEN`/`LSPEN` 预留或立即压入 17 字浮点上下文，异常返回按需
  回收或恢复。

### Lazy FP Stacking and Extended Frames / 惰性浮点压栈与扩展帧

**System registers / 系统寄存器** — all accessed through `CpuCore::system_read`
and `system_write` / 全部通过 `CpuCore::system_read` 与 `system_write` 访问：

| Address | Register | Modeled bits | Reset |
|---------|----------|--------------|-------|
| `0xE000EF34` | `FPCCR` | `ASPEN`[31], `LSPEN`[30], `LSPACT`[0] (read-only) | `0xC000_0000` |
| `0xE000EF38` | `FPCAR` | `[31:3]` address; `[2:0]` reserved, forced to 0 on write | `0` |
| `0xE000EF3C` | `FPDSCR` | `AHP`[26], `DN`[25], `FZ`[24], `RMode`[23:22] | `0` |

`FPCCR` bits other than `ASPEN`/`LSPEN`/`LSPACT` read as zero, and `LSPACT`
is hardware-managed (software writes to it are ignored). `FPDSCR` is plain
storage: no arithmetic path reads it, so the rounding mode always stays
round-to-nearest-even. `FPCAR` keeps the reserved FP context area 8-byte
aligned.

`FPCCR` 中除 `ASPEN`/`LSPEN`/`LSPACT` 之外的位读为 0，`LSPACT` 由硬件管理
（软件写入被忽略）。`FPDSCR` 为纯存储：没有任何算术路径读取它，舍入模式
恒为就近偶数。`FPCAR` 保证预留的浮点上下文区按 8 字节对齐。

**"FP is active" / 「浮点活跃」判据**: ARMv8-M uses `CONTROL.FPCA`, but this
core does not track that bit (`MSR CONTROL` only stores `bits[1:0]`). It uses
`FPEXC.EN` instead — every FPU instruction executed after the CPACR check
sets it — through `CortexM33Registers::fp_context_active`. The two are
equivalent for firmware that never writes `CONTROL.FPCA` directly, which is
true for the supported images.

ARMv8-M 用 `CONTROL.FPCA` 判定，但本核心不跟踪该位（`MSR CONTROL` 只保存
`bits[1:0]`），改用 `FPEXC.EN`（每条通过 CPACR 检查的浮点指令都会置位），
通过 `CortexM33Registers::fp_context_active` 暴露。对于从不直接写
`CONTROL.FPCA` 的固件（当前支持的映像均如此）二者等价。

**Exception entry / 异常进入** (`exceptions::enter_exception`):

- FP inactive → standard 8-word frame (R0-R3, R12, LR, PC, xPSR), `EXC_RETURN`
  bit 4 = 1, no FP space reserved, `FPCAR` untouched.
- FP active and `FPCCR.LSPEN = 1` → extended frame. `FPCAR` = entry SP − 0x48,
  i.e. the S0 slot right above the state frame; 17 words of FP context plus
  one architectural padding word are reserved (0x48), `LSPACT = 1`,
  `EXC_RETURN` bit 4 = 0, and **nothing is written** to the reserved area.
  If the interrupted SP is not 8-byte aligned it is rounded down, a 4-byte
  aligner word is inserted above the frame and stacked xPSR bit 9 records it,
  so the return path restores the exact SP.
- FP active and `FPCCR.LSPEN = 0` (covers `ASPEN = 1`, and eager-only
  configurations) → immediate save of S0-S15 and FPSCR into the same area,
  `LSPACT = 0`, `EXC_RETURN` bit 4 = 0.
- When both `ASPEN` and `LSPEN` are set (the reset value), lazy preservation
  takes precedence and nothing is written at entry.

Resulting extended frame (SP after entry, 26 words = 104 bytes):

```
SP + 0x64  padding (keeps the frame size a multiple of 8)
SP + 0x60  FPSCR
SP + 0x5C  S15
   ...     ...
SP + 0x20  S0            <- FPCAR == SP + 0x20 == entry SP - 0x48
SP + 0x1C  xPSR
SP + 0x18  PC
SP + 0x14  LR
SP + 0x10  R12
SP + 0x0C  R3
SP + 0x08  R2
SP + 0x04  R1
SP + 0x00  R0
```

- 浮点不活跃 → 标准 8 字帧，`EXC_RETURN` bit4 = 1，不预留空间，`FPCAR` 不变。
- 浮点活跃且 `FPCCR.LSPEN = 1` → 扩展帧：`FPCAR` = 进入时 SP − 0x48
  （状态帧上方的 S0 槽）；预留 17 字浮点上下文 + 1 字架构填充（0x48），
  `LSPACT = 1`，`EXC_RETURN` bit4 = 0，预留区**不写入任何内容**。
  若被打断的 SP 未按 8 字节对齐，则先向下对齐，并在帧上方插入 4 字节对齐字、
  记录到压栈 xPSR 的 bit 9，返回时恢复精确的 SP。
- 浮点活跃且 `FPCCR.LSPEN = 0`（覆盖 `ASPEN = 1` 与仅立即压栈的配置）→
  立即把 S0-S15 与 FPSCR 写入同一区域，`LSPACT = 0`，bit4 = 0。
- `ASPEN` 与 `LSPEN` 同时置位（复位值）时惰性保存优先，进入时不写内存。

**Deferred save trigger / 延迟保存触发**: before executing any Thumb-32
encoding that belongs to the FPU (CP10/CP11 — arithmetic, transfers,
load/store, `VMRS`/`VMSR`), the core calls
`exceptions::commit_lazy_fp_context` when `LSPACT = 1`: S0-S15 and FPSCR are
written to `FPCAR` (+0x40 for FPSCR) and `LSPACT` is cleared. The commit
happens exactly once per frame and before the CPACR access check, so a
handler whose first VFP instruction faults with NOCP still preserves the
interrupted context. Non-FPU coprocessor accesses (CP0 GPIO, CP4 DCP) never
trigger it; `fpu::is_fpu_encoding` is the predicate.

执行任何属于浮点单元（CP10/CP11：算术、搬移、加载/存储、`VMRS`/`VMSR`）的
Thumb-32 编码前，若 `LSPACT = 1`，核心调用
`exceptions::commit_lazy_fp_context`：把 S0-S15 与 FPSCR（位于 `FPCAR+0x40`）
写入预留区并清除 `LSPACT`。每个栈帧只提交一次，且发生在 CPACR 权限检查
之前，因此处理程序第一条浮点指令触发 NOCP 时被中断的上下文仍已保存。
CP0 GPIO / CP4 DCP 等非浮点协处理器访问不会触发；判据为
`fpu::is_fpu_encoding`。

**Exception return / 异常返回** (`exceptions::exit_exception`):

- `EXC_RETURN` bit 4 = 0 and `LSPACT = 1` → the handler never used the FPU;
  only the 0x48-byte reservation is reclaimed, `LSPACT` is cleared and no FP
  register is written.
- `EXC_RETURN` bit 4 = 0 and `LSPACT = 0` → S0-S15 and FPSCR are restored
  from this frame's FP area (`frame base + 0x20`), then the reservation is
  reclaimed (plus the 4-byte aligner word when stacked xPSR bit 9 is set).
- `EXC_RETURN` bit 4 = 1 → standard frame: only the 8-word state frame is
  reclaimed and the FP registers are untouched.
- MSP/PSP selection and `EXC_RETURN` validation are unchanged. The
  `EXC_RETURN` encodings therefore become `0xFFFF_FFF1`/`0xFFFF_FFE1`
  (handler), `0xFFFF_FFF9`/`0xFFFF_FFE9` (thread MSP) and
  `0xFFFF_FFFD`/`0xFFFF_FFED` (thread PSP), with bit 4 = 0 meaning extended.

- `EXC_RETURN` bit4 = 0 且 `LSPACT = 1` → 处理程序从未使用浮点单元：仅回收
  0x48 字节预留区、清除 `LSPACT`，不写任何浮点寄存器。
- bit4 = 0 且 `LSPACT = 0` → 从本帧浮点区（帧底 + 0x20）恢复 S0-S15 与
  FPSCR，然后回收预留区（若压栈 xPSR 的 bit 9 置位，再补回 4 字节对齐字）。
- bit4 = 1 → 标准帧：只回收 8 字状态帧，浮点寄存器不动。
- MSP/PSP 选择与 `EXC_RETURN` 校验逻辑保持不变。因此 `EXC_RETURN` 取值为
  `0xFFFF_FFF1`/`0xFFFF_FFE1`（Handler）、`0xFFFF_FFF9`/`0xFFFF_FFE9`
  （Thread MSP）、`0xFFFF_FFFD`/`0xFFFF_FFED`（Thread PSP），bit4 = 0 表示
  扩展帧。

**Nested exceptions / 嵌套异常**: if a lazy frame is still pending
(`LSPACT = 1`) when another exception is taken, the pending context is
committed to its own `FPCAR` first, then the new frame reserves a fresh lazy
area. Restores are derived from each frame's own stack slot
(`frame base + 0x20`), so an inner return cannot corrupt the outer frame.
`FPCAR` and `LSPACT` describe only the innermost frame, exactly like the
single architectural register pair.

若另一异常到来时仍有惰性帧待保存（`LSPACT = 1`），先把该上下文提交到它自己的
`FPCAR`，再由新帧预留新的惰性区。恢复地址始终由各帧自己的栈槽
（帧底 + 0x20）推导，因此内层返回不会破坏外层帧。`FPCAR` 与 `LSPACT` 只描述
最内层帧，与架构中唯一的一对寄存器一致。

**Not modeled / 未建模**: `FPCCR.LSPENS`/`CLRONRET`/`CLRONRETS`/`TS`/
`USER`/`THREAD`, `FPDSCR` influencing arithmetic, `CONTROL.FPCA` tracking,
double precision and FPSCR traps. The secure/non-secure FP banks of the
TrustZone configuration are not modeled; the register values above are the
non-secure reset values.

未建模：`FPCCR.LSPENS`/`CLRONRET`/`CLRONRETS`/`TS`/`USER`/`THREAD`、
`FPDSCR` 对算术的影响、`CONTROL.FPCA` 跟踪、双精度与 FPSCR 陷阱。
TrustZone 的安全/非安全浮点寄存器组未建模；上述复位值为非安全复位值。

### Not implemented / 未实现清单

These encodings are currently reported as **undefined instructions**
(PC and opcode logged), never silently skipped. They are tracked so that
firmware relying on them can be identified immediately.

以下编码当前按**未定义指令**上报（记录 PC 与 opcode），绝不静默跳过。
列表用于快速定位依赖这些功能的固件。

1. Double-precision arithmetic and conversions (`VADD.F64` …, `VCVT.F64.F32`,
   `VCVT.F32.F64`) — the `D0-D15` view is storage only, and the encodings are
   reported as undefined instructions.
2. Half-precision `VCVTB`/`VCVTT`, the 16-bit integer conversion forms
   (`VCVT.F32.S16`, `VCVT.F32.U16`, `VCVT.S16.F32`, `VCVT.U16.F32`) and the
   directed float→integer `VCVTA`/`VCVTN`/`VCVTP`/`VCVTM` forms — reported as
   undefined instructions.
3. FPSCR `FZ` flush-to-zero, `DN` default-NaN, `AHP`, `UFC` underflow
   tracking and non-round-to-nearest rounding modes: arithmetic always runs
   as IEEE-754 binary32 round-to-nearest-even and `VCVTR` always rounds ties
   to even. No FP trap or exception entry is generated; only the cumulative
   status bits are updated.
4. `CPACR` fields outside CP0-CP7 and CP10-CP11 (CP8/CP9, CP12/CP13)
   always read zero.

1. 双精度算术与转换（`VADD.F64` 等、`VCVT.F64.F32`、`VCVT.F32.F64`）——
   `D0-D15` 视图仅作存储，这些编码按未定义指令上报。
2. 半精度 `VCVTB`/`VCVTT`、16 位整数转换形式（`VCVT.F32.S16`、
   `VCVT.F32.U16`、`VCVT.S16.F32`、`VCVT.U16.F32`）以及定向浮点→整数
   `VCVTA`/`VCVTN`/`VCVTP`/`VCVTM`——按未定义指令上报。
3. FPSCR 的 `FZ` 清零模式、`DN` 默认 NaN、`AHP`、`UFC` 下溢统计以及
   非就近舍入模式：算术恒按 IEEE-754 binary32 就近舍入、偶数优先执行，
   `VCVTR` 恒按就近偶数舍入。不产生浮点陷阱或异常进入，只更新累积
   状态位。
4. CP0-CP7 与 CP10-CP11 之外的 CPACR 字段（CP8/CP9、CP12/CP13）恒读为 0。

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
When the FPU is active the frame is **extended** with the 17-word FP context
(S0-S15, FPSCR) plus one padding word; see
[Lazy FP Stacking and Extended Frames](#lazy-fp-stacking-and-extended-frames--惰性浮点压栈与扩展帧).
**EXC_RETURN** values: `0xFFFFFFF1` (return to handler), `0xFFFFFFF9` (thread MSP),
`0xFFFFFFFD` (thread PSP). With an extended frame bit 4 is cleared, giving
`0xFFFFFFE1` / `0xFFFFFFE9` / `0xFFFFFFED`.

**异常进入栈帧**（8 字）：R0-R3、R12、LR、PC、XPSR。浮点单元活跃时栈帧为**扩展帧**，
额外包含 17 字浮点上下文（S0-S15、FPSCR）与 1 字填充；详见
[惰性浮点压栈与扩展帧](#lazy-fp-stacking-and-extended-frames--惰性浮点压栈与扩展帧)。
**EXC_RETURN** 取值：`0xFFFFFFF1`（返回 Handler）、`0xFFFFFFF9`（Thread MSP）、
`0xFFFFFFFD`（Thread PSP）。扩展帧会清除 bit4，得到 `0xFFFFFFE1` / `0xFFFFFFE9` /
`0xFFFFFFED`。

---

## Dual-core scheduling / 双核调度

`DualCoreM33` (module `dual_core`) owns both Cortex-M33 cores and advances
them in a fixed order. It is a reusable scaffold: the CLI `Machine` drives its
own pair of `CortexM33Core`s directly (see
[`rp2350-cli`](rp2350-cli.md#dual-core-execution--双核执行)) because it also
needs per-core PPB replay, SIO views and PSM handling.
`DualCoreM33`（模块 `dual_core`）持有两个 Cortex-M33 核心，并按固定顺序推进。
它是可复用的脚手架：CLI `Machine` 直接驱动自己的一对 `CortexM33Core`（见
[`rp2350-cli`](rp2350-cli.md#dual-core-execution--双核执行)），因为它还需要
按核回放 PPB、SIO 视图与 PSM 处理。

```rust
pub struct DualCoreM33 { /* core0, core1, core1_released */ }

impl DualCoreM33 {
    pub fn new(core0: CortexM33Core) -> Self;
    pub fn core(&self, id: CpuId) -> &CortexM33Core;
    pub fn core_mut(&mut self, id: CpuId) -> &mut CortexM33Core;
    pub fn is_core1_running(&self) -> bool;
    pub fn release_core1(
        &mut self, bus: &mut dyn AddressBus,
        sp: u32, pc: u32, vtor: u32,
    ) -> Result<(), CoreError>;
    pub fn step(&mut self, bus: &mut dyn AddressBus)
        -> Result<[CpuState; 2], CoreError>;
    pub fn run_cycles(&mut self, bus: &mut dyn AddressBus, cycles: u64)
        -> Result<u64, CoreError>;
    pub fn signal_interrupt(&mut self, core: CpuId, irq: IrqNumber);
}
```

Semantics / 语义:

- `new(core0)` creates core1 with `CortexM33Core::new(CpuId::Core1)` in
  `CpuState::Reset`. Core1 never fetches an instruction before a successful
  `release_core1`.
  `new(core0)` 用 `CortexM33Core::new(CpuId::Core1)` 创建处于
  `CpuState::Reset` 的 core1；成功调用 `release_core1` 之前 core1 绝不取指。
- `release_core1` first probes `vtor` and `vtor + 4` through `bus` (a bus error
  leaves core1 halted), then applies `CortexM33Core::boot_at(sp, pc, vtor)`, so
  `SP = sp`, `PC = pc & !1`, `VTOR = vtor`. Repeated calls **re-initialize**
  core1: register file and NVIC are cleared, vectors are reapplied and
  execution restarts at `pc`; pending interrupts are not preserved.
  `release_core1` 先通过 `bus` 探测 `vtor` 与 `vtor + 4`（总线错误时 core1
  保持停机），再调用 `CortexM33Core::boot_at(sp, pc, vtor)`，因此
  `SP = sp`、`PC = pc & !1`、`VTOR = vtor`。重复调用会**重新初始化** core1：
  寄存器文件与 NVIC 清零、重新应用向量并从 `pc` 重新开始执行；挂起中断不保留。
- `is_core1_running()` is true only after a successful release while core1 is
  `CpuState::Running`; a faulted/halted core1 reports `false`.
  `is_core1_running()` 仅在成功释放且 core1 处于 `CpuState::Running` 时为真；
  core1 故障或停机后返回 `false`。
- `step` advances core0 first, then core1 (when running), returning
  `[core0_state, core1_state]`. A halted core1 contributes its current state
  without executing. Errors propagate immediately; if core0 fails, core1 is not
  stepped in that call.
  `step` 先推进 core0，再推进（运行中的）core1，返回
  `[core0_state, core1_state]`。停机的 core1 只返回当前状态、不执行。
  错误立即传播；core0 失败时该次调用不推进 core1。
- `run_cycles` counts one per `step` (instruction width is irrelevant) and
  stops early once both states are non-`Running`; a halted core1 alone never
  stops core0.
  `run_cycles` 每次 `step` 计 1 个周期（与指令宽度无关），两核状态都非
  `Running` 时提前结束；仅 core1 停机不会阻止 core0 运行。
- `signal_interrupt(core, irq)` targets exactly one core's NVIC; the other
  core observes no pending/servicing change because the NVICs are independent.
  `signal_interrupt(core, irq)` 只投递到指定核心的 NVIC；由于两核 NVIC 独立，
  另一核心不会观察到挂起/服务状态变化。

Not wired yet / 尚未接线:

- PSM core1 release polling (`FRCE_OFF` / `PROC1` handshake) and SIO FIFO
  boot messages.
- CLI `Machine` dual-core mode; it still boots and steps core0 only and
  emulates core1 through `BootPeripherals::service_core1_peer`.
- Multicore GDB / debugger attachment.

- PSM core1 释放轮询（`FRCE_OFF` / `PROC1` 握手）与 SIO FIFO 启动消息。
- CLI `Machine` 双核模式；它仍只启动并单步 core0，并通过
  `BootPeripherals::service_core1_peer` 模拟 core1。
- 多核 GDB / 调试器接入。
