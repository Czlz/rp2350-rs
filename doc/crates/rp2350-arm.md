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
| `execute_thumb32::multiply` | `MUL`/`MLA`/`MLS`, long multiplies (`SMULL`/`UMULL`/`SMLAL`/`UMLAL`/`SMLALxy`), divides, and the DSP multiply family (`SMULxy`, `SMULW*`, `SMLAW*`, `SMMUL`/`SMMLA`/`SMMLS`, `SMLAD`/`SMLSD`/`SMUAD`/`SMUSD`, `SMLALD`/`SMLSLD`, `USAD8`) |
| `execute_thumb32::coproc` | CP0 GPIO coprocessor and CP4 DCP stub (`MCR`/`MRC`/`MCRR`/`MRRC`) |
| `fpu` | FPv5-SP state: S0-S31 / D0-D15 views, FPSCR, FPEXC, FPCCR/FPCAR/FPDSCR, CPACR fields |
| `nvic` | NVIC: ISER/ICER/ISPR/ICPR/IABR/IPR/STIR, priority routing |
| `scb` | `SCB_ICSR` NMI/PendSV/SysTick pending latches, `SVCall` latch, `SHPR2`/`SHPR3` priorities |
| `faults` | `SHCSR` enables/views, `CFSR`/`HFSR` status, `AIRCR` (`VECTKEY`/`PRIS`/`SYSRESETREQ`), fault escalation |
| `system_exceptions` | NMI (2) / SVCall (11) / PendSV (14) / SysTick (15) arbitration, delivery, `ICSR` view and diagnostics |
| `scr` | `SCR` sleep-control bits (`SLEEPONEXIT`/`SLEEPDEEP`/`SEVONPEND`) |
| `systick` | SysTick 24-bit down counter with reload + interrupt |
| `system_access` | System control space routing (`SCB_VTOR`/`SCB_ICSR`/`SCB_AIRCR`/`SCB_SHCSR`/`SCB_CFSR`/`SCB_HFSR`/`SCB_SHPR2`/`SCB_SHPR3`/`SCB_CPACR`/`SCB_SCR`/`SCB_STIR`, FPU system registers, NVIC/IPR aliases, SysTick) |
| `fetch_cache` | Direct-mapped flash XIP instruction fetch cache (private, enabled per core) |
| `decode_cache` | Direct-mapped decoded-instruction cache: length, raw word, dispatch tag per flash `PC` |
| `execute_thumb16::fast` | Hot Thumb-16 executors shared by the decoder and the decode-cache fast path |
| `exceptions` | Exception entry/exit, EXC_RETURN decoding, standard and extended (lazy FP) stack frames |

---

## Key Types / 关键类型

### `CortexM33Core`

```rust
pub struct CortexM33Core {
    pub core_id: CpuId,          // Core 0 or 1
    pub registers: CortexM33Registers,
    pub nvic: Nvic,              // external IRQ lines 0..=255 only
    pub scb: Scb,                // NMI/SVCall/PendSV/SysTick latches + SHPR2/SHPR3
    pub systick: SysTick,
    // private: faults: faults::FaultState  // SHCSR enables, HFSR, AIRCR
}

impl CortexM33Core {
    pub fn new(core_id: CpuId) -> Self;

    // Boot / 启动
    pub fn boot(&mut self, bus: &mut dyn AddressBus) -> Result<(), CoreError>;
    pub fn boot_from(&mut self, bus: &mut dyn AddressBus, vector_base: u32)
        -> Result<(), CoreError>;
    pub fn boot_at(&mut self, msp: u32, reset: u32, vector_base: u32);

    // Decoding / 译码（strict 是宿主诊断选项，默认按硬件投递 HardFault）
    pub fn set_strict_decode(&mut self, strict: bool);
    pub fn write_xpsr(&mut self, value: u32);

    // Flash fetch cache / Flash 取指缓存
    pub fn set_flash_fetch_cache(&mut self, enabled: bool);

    // System exceptions / 系统异常
    pub fn pend_nmi(&mut self);            // ICSR.NMIPENDSET
    pub fn pend_svcall(&mut self);         // SHCSR.SVCALLPENDED
    pub fn pend_pendsv(&mut self);         // ICSR.PENDSVSET
    pub fn pend_systick(&mut self);        // ICSR.PENDSTSET + COUNTFLAG
    pub fn system_exception_state(&self) -> SystemExceptionState;

    // Faults / 故障
    pub fn shcsr(&self) -> u32;            // enables + live ACT/PENDED views
    pub fn hfsr(&self) -> u32;
    pub fn aircr(&self) -> u32;            // VECTKEYSTAT | PRIS
    pub fn write_aircr(&mut self, value: u32) -> bool;
    pub fn take_system_reset_request(&mut self) -> bool;
    pub fn vect_reset_requested(&self) -> bool;
}

impl CpuCore for CortexM33Core {
    fn init(&mut self, bus: &mut dyn AddressBus) -> Result<(), CoreError>;
    fn step(&mut self, bus: &mut dyn AddressBus) -> Result<CpuState, CoreError>;
    fn run_cycles(&mut self, bus: &mut dyn AddressBus, cycles: CycleCount) -> (CycleCount, CpuState);
    fn reset(&mut self);
    fn signal_interrupt(&mut self, irq: IrqNumber);
    fn servicing_irq(&self) -> Option<IrqNumber>;
    fn pc(&self) -> u32;
    fn read_register(&self, index: u8) -> u32;
    fn write_register(&mut self, index: u8, value: u32);
    fn cpu_id(&self) -> CpuId;
    fn state(&self) -> CpuState;
    fn total_cycles(&self) -> CycleCount;
}
```

`set_flash_fetch_cache(true)` opts a core into the flash instruction fetch
cache. It is **off by default** and is only exact when both flash XIP windows
are backed by a read-only `rp2350_core::FlashRegion`: guest writes to flash are
rejected, and every host-side mutation (loader, debugger, boot ROM) bumps
`rp2350_core::flash::content_generation`, which clears the table. SRAM is never
cached, so self-modifying SRAM code keeps its exact semantics. The table books
the same two bus cycles a region halfword read does, so `total_cycles` does not
move.

`set_flash_fetch_cache(true)` 让核心启用 Flash 取指缓存。该缓存**默认关闭**，
且只有在两个 Flash XIP 窗口都由只读 `rp2350_core::FlashRegion` 支撑时才精确：
客户机对 Flash 的写会被拒绝，而每次宿主侧变更（加载器、调试器、boot ROM）都会
递增 `rp2350_core::flash::content_generation` 并清空整表。SRAM 从不入缓存，因此
自修改 SRAM 代码语义完全不变。缓存按区域半字读记入同样的两个总线周期，因此
`total_cycles` 不会漂移。

`pend_pendsv()` / `pend_systick()` are the host-side entry points for the two
system exceptions (the same effect as `ICSR.PENDSVSET` / `ICSR.PENDSTSET`).
`system_exception_state()` returns a `Copy` snapshot whose `Display`/`summary()`
is the stable diagnostic string `systick=<state> pendsv=<state>`, where `<state>`
is one of `idle`, `pending`, `active` or `active+pending`.
`CpuCore::description()` appends that string in brackets
(`[systick=… pendsv=…]`), so `Machine::core_description()` and every frontend
that prints the core description show the two system exceptions without a CLI
change.
`pend_pendsv()` / `pend_systick()` 是两个系统异常的宿主侧入口（效果等于
`ICSR.PENDSVSET` / `ICSR.PENDSTSET`）。`system_exception_state()` 返回 `Copy`
快照，其 `Display`/`summary()` 为稳定诊断字符串
`systick=<state> pendsv=<state>`，`<state>` 为 `idle`、`pending`、`active` 或
`active+pending`。`CpuCore::description()` 以方括号追加该字符串
（`[systick=… pendsv=…]`），因此 `Machine::core_description()` 以及所有打印
核心描述的前端都无需修改 CLI 就能看到两个系统异常。

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

    /// Conservative "some enabled IRQ is pending" hint (`false` is exact).
    /// 保守的「存在已使能且挂起中断」提示位（`false` 精确）。
    pub fn has_runnable(&self) -> bool;

    /// Any pending external line, enabled or not (`ICSR.ISRPENDING` view).
    /// 任何挂起的外部线（无论是否使能，`ICSR.ISRPENDING` 视图）。
    pub fn has_any_pending(&self) -> bool;

    /// `BASEPRI` rule for an already-decoded priority (system exceptions too).
    /// 对已解码优先级应用 `BASEPRI` 规则（系统异常也可用）。
    pub fn basepri_allows_priority(&self, priority: u8, basepri: u32) -> bool;
}
```

NVIC `IPR` bytes are addressed one per IRQ: the absolute address
`0xE000_E400 + irq` selects byte `irq % 4` of word `irq / 4`. `system_access`
decodes that mapping directly, so byte writes to IRQ 4 and above no longer land
on IRQ0..3 (a pre-existing routing defect fixed together with the system
exception work); `IPR` word accesses still pack four IRQs little-endian.
NVIC 的 `IPR` 每个 IRQ 一个字节：绝对地址 `0xE000_E400 + irq` 选择字
`irq / 4` 的字节 `irq % 4`。`system_access` 直接按该映射解码，因此 IRQ4 以上的
字节写不再落到 IRQ0..3（该既有路由缺陷随系统异常工作一并修复）；字访问仍按小端
打包 4 个 IRQ。

### Priority selection / 优先级选择

`find_highest_priority` builds the candidate set as eight `u32` bitmaps
(`ISER & ISPR & !IABR`, one per 32-IRQ group) and then walks eight priority
buckets from 0 upwards. Each bucket is a `u32` mask of the IRQs that carry that
priority, maintained incrementally by `set_priority` / `write_ipr_word`
(`write_ipr_byte` only moves the bit when the *decoded* priority changes).

The result is identical to a 256-entry linear scan:

* a numerically smaller priority wins;
* within one priority the lowest IRQ number wins (`trailing_zeros` returns the
  least significant set bit);
* an active interrupt is never selected again (skipped by `!IABR`);
* `PRIMASK` / `FAULTMASK` / `BASEPRI` are applied by the core after the
  selector returns a candidate, and the same gate covers PendSV/SysTick; an
  active exception is only preempted by a strictly higher priority.
* `PRIMASK` / `FAULTMASK` / `BASEPRI` 由核在选择器返回候选后应用，同一门控也
  覆盖 PendSV/SysTick；活跃异常只能被严格更高的优先级抢占。

`nvic.rs` unit tests compare the bitmap selector against a linear reference
implementation over pseudorandom enable/pending/active/priority matrices.

`find_highest_priority` 先把候选集表示为 8 个 `u32` 位图
（`ISER & ISPR & !IABR`，每 32 个 IRQ 一组），再按优先级从 0 逐桶遍历。每个
优先级桶是一个 `u32` 掩码，由 `set_priority` / `write_ipr_word` 增量维护
（`write_ipr_byte` 只在*解码后*优先级变化时移动位）。结果与 256 项线性扫描完全
一致：数字越小优先级越高；同优先级取 IRQ 号最小者（`trailing_zeros` 返回最低
置位）；活跃中断不再被选中（由 `!IABR` 排除）；`PRIMASK`/`FAULTMASK`/`BASEPRI`
由核在选择器返回候选后应用，同一门控也覆盖 PendSV/SysTick；活跃异常只能被严格
更高的优先级抢占。单元测试用伪随机矩阵把位图选择器与线性参考实现对表。

### Pending gate / 挂起门控

`has_runnable` is a conservative cache of `any(ISER & ISPR)`. Every writer of the
enable or pending set maintains it — `write_iser`, `write_icer`, `write_ispr`,
`write_icpr`, `set_pending`, `clear_pending` and `write_stir` — so a `false`
value is exact: no interrupt can be selected while it is false. Active state is
deliberately not folded in, so `true` may still select nothing (an active IRQ
that is still pending); `CortexM33Core::check_pending_exceptions` therefore
short-circuits on `PRIMASK` **or** a closed gate and only runs the selector when
a delivery is actually possible. NVIC unit tests compare the gate against a
linear `enabled && pending` reference, and the core tests pin delivery across
enable/ICPR/`PRIMASK`/SysTick paths.

`has_runnable` 是 `any(ISER & ISPR)` 的保守缓存：使能/挂起集合的每个写入口
（`write_iser`、`write_icer`、`write_ispr`、`write_icpr`、`set_pending`、
`clear_pending`、`write_stir`）都会维护它，因此为 `false` 时精确——此时选择器
必然返回 `None`。活跃状态刻意不折算，所以 `true` 仍可能选不出中断（挂起且活跃
的中断）；`CortexM33Core::check_pending_exceptions` 因此在 `PRIMASK` **或**
门控为假时短路，只在真正可能投递时才运行选择器。NVIC 单元测试把门控与
`enabled && pending` 的线性参考对表，核心测试锁定使能/ICPR/`PRIMASK`/SysTick
各路径上的投递行为。

### Sleep, events and blocked loads / 睡眠、事件与阻塞载入

`WFI`, `WFE`, `SEV` and `SEVL` are real instructions, not `NOP`s, and a load that
hits a blocking device is retried instead of being skipped.

```rust
pub enum SleepReason {
    WaitForInterrupt,                    // WFI, SLEEPDEEP = 0
    WaitForEvent,                        // WFE, SLEEPDEEP = 0
    SleepDeep { wait_for_event: bool },  // WFI/WFE with SLEEPDEEP = 1
    SleepOnExit { deep: bool },          // SCR.SLEEPONEXIT return-to-Thread sleep
    BlockedRead,                         // the last load hit a retryable bus stall
}

impl CortexM33Core {
    pub fn sleep_reason(&self) -> Option<SleepReason>;
    pub fn signal_event(&mut self);        // host event source (FIFO, doorbell, peer SEV)
    pub fn event_register(&self) -> bool;  // diagnostics
    pub fn take_events_sent(&mut self) -> u32; // SEV count for peer forwarding
}

impl SleepReason {
    pub fn is_deep_sleep(self) -> bool;    // SleepDeep or SleepOnExit { deep: true }
}
```

* **`WFI`** retires normally (so `PC` already points past it) and then suspends
  only when no enabled interrupt is pending. A pending interrupt, masked or not,
  keeps the core running: the ARM ARM guarantees `WFI` wakes on it, and the
  instruction is never skipped. A sleeping core wakes on **any** enabled pending
  interrupt — `PRIMASK`/`FAULTMASK` only decide whether that interrupt is then
  *taken* — or on an event-source change surfaced by the host.
* **`WFE`** clears a set event register and continues, otherwise it suspends
  until an event (host `signal_event`, `SEV`, exception entry or return) or an
  enabled pending interrupt arrives. `WFI` never consumes the event register, so
  `SEV` cannot wake it, exactly as the architecture requires.
* **`SEV`** sets this core's event register and increments a counter the host
  forwards to the peer core (`take_events_sent`), implementing the ARM ARM rule
  that `SEV` signals every core in the multiprocessor. `SEVL` sets only the local
  register. Exception entry and exception return also set the event register,
  which is the architectural event source that makes `while (!flag) __wfe();`
  loops correct.
* **`SCR` sleep control (`0xE000_ED10`)**: only `SLEEPONEXIT`[1],
  `SLEEPDEEP`[2] and `SEVONPEND`[4] are stored; every reserved bit reads as zero
  and reset clears all three.
  * `SEVONPEND` converts **each 0-to-1 transition of an exception pending bit**
    into an event, whether the exception is enabled, disabled or masked by
    `PRIMASK`. `CpuCore::signal_interrupt`, `NVIC_ISPR` writes, `STIR` and the
    SysTick underflow all share one transition helper, so an already-pending
    line never re-sends the event (otherwise `WFE` would stop waiting). `WFE`
    consumes the event and continues; `WFI` ignores events and is never woken
    by `SEVONPEND` alone.
  * `SLEEPONEXIT` suspends the core after an exception return **to Thread
    mode**: unstacking has already restored `PC`/SP, and the restored
    instruction does not execute before the sleep. A Handler→Handler return
    never triggers it. The wake condition is the `WFI` one, so a runnable
    interrupt keeps the core awake and is taken in the same step.
  * `SLEEPDEEP` changes only the reported reason (`SleepDeep { wait_for_event }`
    for `WFI`/`WFE`, `SleepOnExit { deep: true }` for `SLEEPONEXIT`). This
    model has no separately powered domain, so deep sleep has exactly the same
    wake behaviour as shallow sleep; power-down and clock-gating electrical
    effects are not modeled.
  * The core-side register is reached through `CpuCore::system_write`
    (`0xE000_ED10`). In a full machine the `rp2350-periph` PPB stub also replays
    `VTOR`, `CPACR`, `ICSR`, `SHPR2`, `SHPR3` and NVIC writes into the core, so a
    firmware `STR` to `SCR` reaches it once the shared write log is drained.
    SysTick itself is deliberately **not** replayed: the peripheral
    `SysTickCounter` owns the counter and follows the live clock tree.
  * 核心侧寄存器通过 `CpuCore::system_write`（`0xE000_ED10`）访问。完整机器中
    `rp2350-periph` 的 PPB 桩还会把 `VTOR`、`CPACR`、`ICSR`、`SHPR2`、`SHPR3`
    与 NVIC 写入回放到核，因此固件对 `SCR` 的 `STR` 会在共享写入日志被排空时
    到达核心。SysTick 本身有意**不**回放：外设侧 `SysTickCounter` 拥有计数器并
    跟随实时时钟树。
* **Sleeping cores still age**: [`CpuCore::step`] on a sleeping core advances
  `SysTick` and `total_cycles` by one, then checks the wake condition. The first
  step whose condition holds executes the instruction at `PC` immediately
  (zero-step wake latency); when the wake is caused by a takeable interrupt the
  exception is entered in that step instead, so no thread instruction runs
  between sleep and the handler.
* **Blocked load (`BlockedRead`)**: when a load raises
  `CoreError::BusError { kind: BusErrorKind::Stall }` (an empty blocking FIFO
  read, see `rp2350-periph`), the instruction is **not** retired: `PC`, the IT
  state and every register keep their pre-instruction values, the attempt books
  no cycles, and the core reports [`CpuState::Sleeping`]. The stall gate lives in
  the bus (`MemoryRegion::read_would_stall`), so the device is never touched and
  nothing can be consumed or latched by a stalled access; the identical load is
  replayed once data arrives and then retires exactly once.
* `check_pending_exceptions` honours `FAULTMASK` and `BASEPRI` in addition to
  `PRIMASK`, applies the strictly-higher-priority preemption rule, and arbitrates
  external interrupts together with NMI (2), SVCall (11), PendSV (14) and
  SysTick (15); the sleep wake path uses the same gate, so a sleeping core never
  enters an exception it should keep masked. NMI and HardFault are fixed-priority
  exceptions and bypass every mask.
* `check_pending_exceptions` 在 `PRIMASK` 之外还遵守 `FAULTMASK` 与 `BASEPRI`，
  应用「严格更高优先级才能抢占」规则，并把外部中断与 NMI（2）、SVCall（11）、
  PendSV（14）、SysTick（15）一起仲裁；睡眠唤醒路径使用同一门控，因此睡眠核绝不
  会进入应保持屏蔽的异常。NMI 与 HardFault 为固定优先级异常，绕过所有屏蔽。

**Not modelled**: the electrical effect of `SLEEPDEEP` (power-down, clock
gating, state retention), `WFE`'s optional timeout (`-w`), and the `rp2350-cli`
forwarding of a peripheral `SysTickCounter` expiry into
[`CortexM33Core::pend_systick`] (the core side is implemented and covered by
unit tests; until the driver forwards
`BootPeripherals::take_systick_pending()`, a machine run keeps the expiry in the
peripheral latch). `SEVONPEND` covers NVIC external interrupts, `STIR`/`ISPR`
writes, `ICSR.NMIPENDSET`/`PENDSVSET`/`PENDSTSET` and the SysTick underflow. A
host that never signals an event and never enables an interrupt will see a
`WFE`/`WFI` sleeper stay asleep while the machine clock keeps running — that is
the hardware behaviour, and the CLI reports it as a `[run] sleep stall`
diagnostic instead of hanging silently.

未建模：`SLEEPDEEP` 的电气效果（掉电、时钟门控、状态保持）、`WFE` 的可选超时
（`-w`），以及 `rp2350-cli` 把外设 `SysTickCounter` 到期转发给
[`CortexM33Core::pend_systick`]（核侧已实现并有单元测试；在驱动转发
`BootPeripherals::take_systick_pending()` 之前，机器运行会把到期留在外设锁存中）。
`SEVONPEND` 覆盖 NVIC 外部中断、`STIR`/`ISPR` 写入、
`ICSR.NMIPENDSET`/`PENDSVSET`/`PENDSTSET` 与 SysTick 下溢。若宿主从不发送事件、
也从不使能中断，`WFE`/`WFI` 睡眠者会保持睡眠而机器时钟继续推进——这正是硬件行为，
CLI 以 `[run] sleep stall` 诊断报告，而不是静默耗尽预算。

### `SysTick`

```rust
pub mod csr {    pub const ENABLE: u32 = 1 << 0;
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
    pub fn write_csr(&mut self, value: u32);   // ENABLE/TICKINT/CLKSOURCE; clears COUNTFLAG
    pub fn set_countflag(&mut self);           // ICSR.PENDSTSET side effect
}
```

`tick()` pends the SysTick **system exception** through
`CortexM33Core::pend_systick()` (exception 15), never NVIC external IRQ 15.
`tick()` 通过 `CortexM33Core::pend_systick()` 挂起 SysTick **系统异常**（异常 15），
绝不会挂起 NVIC 外部 IRQ 15。

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
pub enum SleepKind { WaitForInterrupt, WaitForEvent }

pub enum StepResult {
    Ok,                                       // executed, one cycle
    SvCall(u8),                               // SVC
    Breakpoint(u8),                           // BKPT
    BranchTaken,                              // new PC written
    Sleep(SleepKind),                         // WFI/WFE: the core decides whether to suspend
    SendEvent { local: bool },                // SEV (local = false) / SEVL (local = true)
    ExceptionReturn,                          // PC written with an EXC_RETURN value
    Undefined,                                // reported through `note_undefined`
}
```

The wait hints only *report* which instruction ran; the core applies the
wake/suspend decision after the instruction retires, using live NVIC, mask and
event state (see "Sleep, events and blocked loads"). `SEV`/`SEVL` likewise only
report the event request.

**Supported instruction groups / 支持的指令组** (~90 instructions):
- **Data processing**: `MOV`, `ADD`, `SUB`, `CMP`, `AND`, `ORR`, `EOR`, `MVN`, `ADC`, `SBC`, `LSL`, `LSR`, `ASR`, `ROR`, `TST`, `NEG`, `MUL`, `MRS`, `MSR`
- **Branch**: `B`, `B<cc>`, `BL`, `BX`, `BLX`
- **Load/Store**: `LDR`, `STR`, `LDRH`, `STRH`, `LDRB`, `STRB`, `LDMIA`, `STMIA`, `PUSH`, `POP`
- **Miscellaneous**: `NOP`, `BKPT`, `SVC`, `CPS`, `IT`, `CBZ`, `CBNZ`, `REV`

### Shared executors / 共享执行器

`execute_thumb16::fast` holds the hot arms (`LDR (literal)`, `LDR`/`STR`
immediate word forms, data-processing (register), `B`, `B<cc>`, the hint space,
`CPS`). Both `execute_thumb16` and the decode-cache fast path call them, so the
cached and uncached paths can never diverge: the cache stores *how* to dispatch,
never *what* the instruction does.
`execute_thumb16::fast` 存放热点分支（`LDR（文字池）`、`LDR`/`STR` 立即数字形式、
数据处理（寄存器）、`B`、`B<cc>`、提示空间、`CPS`）。`execute_thumb16` 与译码缓存
快路径都调用它们，因此缓存路径与非缓存路径绝不可能分叉：缓存只保存**如何分派**，
从不保存指令**做什么**。

---

## Decoded-instruction cache / 译码结果缓存

```rust
pub struct DecodeCache { /* 1024 direct-mapped entries, 16 bytes each */ }
pub enum Kind { Empty, Slow16, Slow32, B16, BCond16, LdrLiteral16,
                LdrWordImm5, StrWordImm5, DpRegister16, Hint16, Cps16, Group32 }
pub fn decode16(pc: u32, word: u16) -> Entry;
pub fn decode32(pc: u32, word: u32) -> Entry;
```

- `CortexM33Core::set_decode_cache(bool)` toggles it; **off by default**.
  `CortexM33Core::set_flash_fetch_cache(true)` enables both caches because they
  rely on the same invariants.
  `CortexM33Core::set_decode_cache(bool)` 开关该缓存，**默认关闭**；
  `CortexM33Core::set_flash_fetch_cache(true)` 会同时启用两者，因为它们依赖相同的
  不变量。
- An entry holds the instruction address (tag), the raw word, the length, a
  dispatch tag and one pre-computed value (branch target or literal address).
  A hit replaces two fetch lookups, the Thumb-16/32 width decision and the
  opcode-class chain with one tag compare. The index is `(pc >> 1) & 1023`, so
  1024 entries cover 2 KiB of code without conflicts.
  条目保存指令地址（标签）、原始指令字、长度、分派标签与一个预计算值（分支目标或
  文字池地址）。一次命中以一次标签比较取代两次取指查找、Thumb-16/32 宽度判定与
  操作码类别链。索引为 `(pc >> 1) & 1023`，因此 1024 条覆盖 2 KiB 代码无冲突。
- A cache hit books exactly the cycles the fetch path books (3 for a 16-bit
  instruction, 6 for a 32-bit one), and the fast executors keep the original
  per-arm cycle additions — including the fact that a **taken** branch books no
  extra cycle, because the generic arm returns before the shared tail.
  一次命中记入与取指路径完全相同的周期（16 位指令 3 个、32 位指令 6 个），快路径
  也保留原始的逐分支周期记账——包括「**已执行**的分支不额外记周期」这一点，因为
  通用分支在共享尾部之前就返回了。
- Empty slots carry tag 0, which is also the value of `PC = 0`; `lookup` rejects
  `Kind::Empty` explicitly so a jump to address 0 falls back to a bus fetch
  instead of executing a phantom `word = 0` instruction. This matters for the
  boot-ROM window and for a HardFault whose vector is 0.
  空槽位的标签为 0，而 `PC = 0` 也是 0；`lookup` 显式拒绝 `Kind::Empty`，因此跳转到
  地址 0 会回落到总线取指，而不是执行一条虚构的 `word = 0` 指令。这对 boot ROM
  窗口以及向量为 0 的 HardFault 都很重要。
- Inside an IT block every instruction still runs the full decoder, so slot
  consumption and the `setflags = !InITBlock()` flag suppression are untouched.
  `--strict-decode`, `watch`, `trace` and GDB single-step/breakpoints keep their
  behaviour: they all use this same `step`, which the cache only accelerates.
  IT 块内的指令仍走完整译码器，因此槽位消费与 `setflags = !InITBlock()` 的标志抑制
  完全不变。`--strict-decode`、`watch`、`trace` 与 GDB 单步/断点行为不变：它们都
  使用同一条 `step`，缓存只是加速了它。

### Invalidation / 失效

| Source / 来源 | Mechanism / 机制 |
|---|---|
| Host flash write/erase (`FlashHandle`, `load_bytes`, UF2/ELF load) | `flash::content_generation` bump → the whole table is dropped on the next lookup |
| Guest write to flash | rejected by `FlashRegion`, so flash can never be self-modified |
| `SRAM` write (self-modifying code) | **not cached at all** — `SRAM` code always takes the normal fetch path |
| `reset` / `boot_at` | register file only; entries stay valid because they only describe flash bytes |

`SRAM` 写入（自修改代码）**完全不缓存**：`SRAM` 代码始终走常规取指路径，因此不需要
失效钩子，也不会读到过期条目。要做到精确失效，必须在热路径上查询按 16 字节行的世代
表，其成本高于它节省的取指；实测负载也都不在 `SRAM` 中执行。

`CortexM33Core::decode_cache_flushes()` exposes the generation-flush counter for
diagnostics and tests.
`CortexM33Core::decode_cache_flushes()` 暴露世代清空计数，供诊断与测试使用。

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
- **Halfword-accumulating long multiply**: `SMLALBB`/`SMLALBT`/`SMLALTB`/
  `SMLALTT` add one signed 16×16 product of the selected halfwords to
  `RdHi:RdLo`. `RdLo` is `bits[15:12]`, `RdHi` is `bits[11:8]`, bit 5 of
  `bits[7:4]` selects the top halfword of `Rn` and bit 4 the top halfword of
  `Rm`. The product is exact and the 64-bit accumulation wraps modulo 2^64
  exactly like the ARM pseudocode `result<63:0> = product +
  SInt(R[dHi]:R[dLo])`; no flag is modified. `bits[7:6] = 0b10` is the
  discriminator against `SMLAL` (`bits[7:4] = 0b0000`) and
  `SMLALD`/`SMLALDX` (`bits[7:4] = 0b110x`); every other pattern, and any PC
  operand, stays undefined. Encodings verified with `arm-none-eabi-as`:
  `fbc2 0183`/`fbc2 0193`/`fbc2 01a3`/`fbc2 01b3` for
  `SMLALBB`/`SMLALBT`/`SMLALTB`/`SMLALTT r0, r1, r2, r3`.
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
| `SMLALD`/`SMLALDX`, `SMLSLD`/`SMLSLDX`, `SMLALxy`, `SMULxy`, `SMULWy`, `SMMUL`/`SMMLA`/`SMMLS`, `USAD8`/`USADA8`, `SXTAB16`/`UXTAB16` | unchanged | unchanged | unchanged |
| `SEL` | read only | unchanged | unchanged |

`Q` is cumulative: a saturating or overflowing instruction only ever sets it,
so software clears it explicitly (`MSR APSR_nzcvq`) before a sequence and tests
it once afterwards. GE never participates in the `GE`/`LT` condition codes
(those still test `N == V`). The accumulate-then-check rule matters for
`SMLAD`: the products `0x4000_0000 + 0x4000_0000 - 1` do **not** set `Q`,
because the architecturally checked value is the single infinite-precision
sum `0x7FFF_FFFF`.

The DSP multiply chapter is now complete for Cortex-M33: `SMLALxy` (the
64-bit accumulating halfword products) was the last missing encoding.

DSP 乘法章节对 Cortex-M33 已完整：`SMLALxy`（64 位累加的半字乘积）是最后
一条缺失编码。

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
- **半字长累加乘**：`SMLALBB`/`SMLALBT`/`SMLALTB`/`SMLALTT` 把选定半字的一个
  有符号 16×16 乘积累加到 `RdHi:RdLo`。`RdLo` 为 `bits[15:12]`、`RdHi` 为
  `bits[11:8]`，`bits[7:4]` 的位 5 选择 `Rn` 的高半字、位 4 选择 `Rm` 的高
  半字。乘积为精确值，64 位累加按模 2^64 回绕，与 ARM 伪代码
  `result<63:0> = product + SInt(R[dHi]:R[dLo])` 一致；不修改任何标志。
  `bits[7:6] = 0b10` 是与 `SMLAL`（`bits[7:4] = 0b0000`）及
  `SMLALD`/`SMLALDX`（`bits[7:4] = 0b110x`）区分的判据；其余组合与任何 PC
  操作数保持未定义。编码已用 `arm-none-eabi-as` 核对：
  `SMLALBB`/`SMLALBT`/`SMLALTB`/`SMLALTT r0, r1, r2, r3` 分别为
  `fbc2 0183`/`fbc2 0193`/`fbc2 01a3`/`fbc2 01b3`。
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
`SMLALxy` 与长双乘形式一样按模 2^64 回绕且不写任何标志。
`SEL` 只读 GE。`Q` 为累积位，饱和/上溢指令只会置位，因此软件需先用
`MSR APSR_nzcvq` 显式清除，执行完序列后再统一测试。
GE 绝不参与 `GE`/`LT` 条件码（后者仍比较 `N == V`）。累加后再判断的规则对
`SMLAD` 尤其重要：乘积 `0x4000_0000 + 0x4000_0000 - 1` **不会**置 `Q`，
因为架构检查的是最终那一个无限精度和 `0x7FFF_FFFF`。

DSP 乘法章节对 Cortex-M33 已完整：`SMLALxy`（`SMLALBB`/`SMLALBT`/`SMLALTB`/
`SMLALTT`，即 64 位累加的半字乘积）是最后一条缺失编码，现已实现。

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
group as an undefined instruction (startup firmware logged it). `CpuCore::reset`
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
  `CFSR.UFSR.NOCP` set, exception number 6 when `SHCSR.USGFAULTENA = 1`,
  otherwise escalated to HardFault (3) with `HFSR.FORCED`; standard frame
  stacked on MSP/PSP per CONTROL, `EXC_RETURN` 0xFFFFFFF9 / 0xFFFFFFFD /
  0xFFFFFFF1. The stacked PC is the faulting instruction (PC is rewound before
  entry). Because this emulator enters at the image entry point (the mask-ROM
  boot ROM is not executed), booting applies the boot ROM's post-ROM state and
  leaves CP10/CP11 enabled. A power-on register file still has CPACR
  cleared, and writing CPACR to 0 is fully honored, so the NOCP path is
  reachable and tested both with and without the UsageFault enable.
- **Lazy / extended exception frames** (see the dedicated section below):
  `FPCCR`, `FPCAR` and `FPDSCR` are modeled at `0xE000EF34`, `0xE000EF38`
  and `0xE000EF3C`; exception entry reserves or stacks the 17-word FP
  context according to `FPCCR.ASPEN`/`LSPEN`, exception return reclaims or
  restores it, and `FPCCR.CLRONRET`/`CLRONRETS` clear the caller-saved
  context on return. `CONTROL.FPCA` is tracked, raised by every FPU
  instruction, software-writable through `MSR CONTROL`, and restored from the
  returned frame type.

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
  置 `CFSR.UFSR.NOCP`；`SHCSR.USGFAULTENA = 1` 时进入异常号 6，否则以
  `HFSR.FORCED` 升级为 HardFault（3）；按 CONTROL 在 MSP/PSP 上压标准栈帧，
  `EXC_RETURN` 为 0xFFFFFFF9 / 0xFFFFFFFD / 0xFFFFFFF1；压栈 PC 为
  引发故障的指令（进入前已回退 PC）。

  由于本模拟器直接从映像入口开始（不执行掩膜 ROM 中的 boot ROM），
  启动时复现 boot ROM 交付状态，使 CP10/CP11 保持使能。上电寄存器文件
  的 CPACR 仍为清零状态；把 CPACR 写 0 也会被完全遵守，因此 NOCP
  通路在使能与未使能 UsageFault 两种情形下都可达且已被测试覆盖。
- **惰性 / 扩展异常帧**（详见下一节）：在 `0xE000EF34`、`0xE000EF38`、
  `0xE000EF3C` 建模 `FPCCR`、`FPCAR`、`FPDSCR`；异常进入按
  `FPCCR.ASPEN`/`LSPEN` 预留或立即压入 17 字浮点上下文，异常返回按需
  回收或恢复，并由 `FPCCR.CLRONRET`/`CLRONRETS` 在返回时清除调用者保存
  上下文。`CONTROL.FPCA` 被跟踪，由每条浮点指令置位，可经 `MSR CONTROL`
  写入，并按返回帧类型恢复。

### Lazy FP Stacking and Extended Frames / 惰性浮点压栈与扩展帧

**System registers / 系统寄存器** — all accessed through `CpuCore::system_read`
and `system_write` / 全部通过 `CpuCore::system_read` 与 `system_write` 访问：

| Address | Register | Modeled bits | Reset |
|---------|----------|--------------|-------|
| `0xE000EF34` | `FPCCR` | behaviour: `ASPEN`[31], `LSPEN`[30], `CLRONRET`[28], `CLRONRETS`[27], `LSPACT`[0] (read-only); hard-wired: `TS`[26]; storage only: `LSPENS`[29], `THREAD`[3], `USER`[1] | `0xC400_0000` |
| `0xE000EF38` | `FPCAR` | `[31:3]` address; `[2:0]` reserved, forced to 0 on write | `0` |
| `0xE000EF3C` | `FPDSCR` | `AHP`[26], `DN`[25], `FZ`[24], `RMode`[23:22] | `0` |

`FPCCR` field classes / `FPCCR` 字段分类 (ARM ARM B3.4.7):

| Bits | Field | Class | Behaviour / 行为 |
|------|-------|-------|------------------|
| [31] | `ASPEN` | stored | automatic state preservation enable; the model records `CONTROL.FPCA` on FPU execution regardless, and frame selection also consults `FPEXC.EN`. / 自动状态保存使能；本模型执行浮点指令时总记录 `CONTROL.FPCA`，帧选择另外参考 `FPEXC.EN`。 |
| [30] | `LSPEN` | behaviour | lazy reservation on entry (`1`) vs immediate save (`0`). / 进入时惰性预留（1）或立即保存（0）。 |
| [29] | `LSPENS` | storage | no secure state exists that it could protect `LSPEN` from, so writes to `LSPEN` are never gated. / 没有可供它保护 `LSPEN` 的安全状态，因此 `LSPEN` 的写入永不被拦截。 |
| [28] | `CLRONRET` | behaviour | exception return clears the caller-saved FP context, see below. / 异常返回清除调用者保存的浮点上下文，见下文。 |
| [27] | `CLRONRETS` | behaviour | same effect as `CLRONRET` (single security state). / 与 `CLRONRET` 等效（单安全态）。 |
| [26] | `TS` | hard-wired 1 | reads 1, writes ignored, present after reset. / 读为 1、写忽略、复位后保持 1。 |
| [25:4] | reserved | read 0 | `THREAD`[3] is the only exception, see next row. / 唯一例外是下一行的 `THREAD`[3]。 |
| [3] | `THREAD` | storage | the model never allocates a frame from Handler mode, so nothing to report. / 模型不会从 Handler 模式分配栈帧，无内容可上报。 |
| [2] | `S` | read 0 | no TrustZone. / 无 TrustZone。 |
| [1] | `USER` | storage | the core always runs privileged. / 核心恒以特权执行。 |
| [0] | `LSPACT` | hardware | set while a lazy frame is still pending, cleared by the commit or the return. / 惰性帧待保存时置位，提交或返回时清除。 |

`TS` reads as 1 on this model even though the Cortex-M33 reset value has it
clear: with a single security state the FP context is always treated as secure,
so the callee-saved half of the bank is part of the frame. `LSPACT` is
hardware-managed (software writes to it are ignored). `FPDSCR` is plain
storage: no arithmetic path reads it, so the rounding mode always stays
round-to-nearest-even. `FPCAR` keeps the reserved FP context area 8-byte
aligned.

本模型把 `TS` 恒读为 1，而 Cortex-M33 的复位值中该位为 0：单安全态下浮点上下文
始终按安全上下文处理，因此寄存器组中被调用者保存的那一半属于栈帧。`LSPACT`
由硬件管理（软件写入被忽略）。`FPDSCR` 为纯存储：没有任何算术路径读取它，
舍入模式恒为就近偶数。`FPCAR` 保证预留的浮点上下文区按 8 字节对齐。

**"FP is active" / 「浮点活跃」判据**: ARMv8-M uses `CONTROL.FPCA`. The core
tracks it: `CortexM33Core::prepare_fpu_instruction` raises `FPCA` for every
Thumb-32 encoding that belongs to the FPU whenever `CPACR` grants access,
exactly like the architectural rule. `MRS CONTROL` returns all three modeled
bits (`nPRIV`, `SPSEL`, `FPCA`) and `MSR CONTROL` stores `bits[2:0]`, so
software can set or clear `FPCA` directly. An exception return restores it from
the frame type (`FPCA = !EXC_RETURN[4]`: basic frame → 0, extended frame → 1),
independent of what the handler wrote; `FPCCR.CLRONRET` additionally clears it
when a basic frame is returned. `CortexM33Registers::fp_context_active` returns
`CONTROL.FPCA || FPEXC.EN`; `FPEXC.EN` (set by the same instructions, never
cleared by `MSR CONTROL`) acts as a fallback so a context can never be silently
lost.

ARMv8-M 用 `CONTROL.FPCA` 判定。本核心跟踪该位：当 `CPACR` 允许访问时，
`CortexM33Core::prepare_fpu_instruction` 对每条属于浮点单元的 Thumb-32 编码置位
`FPCA`，与架构规则一致。`MRS CONTROL` 返回建模的三位（`nPRIV`、`SPSEL`、
`FPCA`），`MSR CONTROL` 保存 `bits[2:0]`，因此软件可直接置位/清除 `FPCA`。
异常返回按帧类型恢复它（`FPCA = !EXC_RETURN[4]`：基础帧 → 0，扩展帧 → 1），
与处理程序写入的值无关；返回基础帧时 `FPCCR.CLRONRET` 还会再次清除它。
`CortexM33Registers::fp_context_active` 返回 `CONTROL.FPCA || FPEXC.EN`；
`FPEXC.EN`（由同样的指令置位，且不会被 `MSR CONTROL` 清除）作为回退，保证
上下文绝不会被静默丢弃。

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
- `FPCCR.CLRONRET`/`CLRONRETS` (`clear_on_return`): either bit enables the
  clear, because with no TrustZone model the secure (`CLRONRETS`) and
  non-secure (`CLRONRET`) variants collapse onto the single security state.
  An extended frame that was actually unstacked (`LSPACT = 0`) additionally
  zeroes S0-S15 and FPSCR after the restore, as ARMv8-M requires
  (`CLRONRET` exists so privileged FP data cannot leak to unprivileged
  software across an exception return); S16-S31 and the `FPCA`/`FPEXC.EN`
  context stay live. A basic frame (`EXC_RETURN` bit 4 = 1) instead
  deactivates the context: `CONTROL.FPCA` and `FPEXC.EN` are cleared, so the
  next exception entry starts from a standard frame again. A pending lazy
  frame (`LSPACT` still 1) keeps its registers untouched — they still belong
  to the interrupted context.
- MSP/PSP selection and `EXC_RETURN` validation are unchanged. The
  `EXC_RETURN` encodings therefore become `0xFFFF_FFF1`/`0xFFFF_FFE1`
  (handler), `0xFFFF_FFF9`/`0xFFFF_FFE9` (thread MSP) and
  `0xFFFF_FFFD`/`0xFFFF_FFED` (thread PSP), with bit 4 = 0 meaning extended.
- Every exception return also assigns `CONTROL.FPCA = !EXC_RETURN[4]`
  (ARMv8-M B3.4.4): a basic frame resumes with `FPCA = 0`, an extended frame
  with `FPCA = 1`. Any value written by `MSR CONTROL` inside the handler is
  therefore overridden by the returned frame type. `nPRIV` is not restored
  (software owns it), and `SPSEL` follows `EXC_RETURN[2]` only on a return to
  Thread mode.
- A nested exception entered from Handler mode produces
  `EXC_RETURN = 0xFFFF_FFF1` (bit 3 = 0), so it returns to the preempted
  handler and never triggers `SLEEPONEXIT`.

- `EXC_RETURN` bit4 = 0 且 `LSPACT = 1` → 处理程序从未使用浮点单元：仅回收
  0x48 字节预留区、清除 `LSPACT`，不写任何浮点寄存器。
- bit4 = 0 且 `LSPACT = 0` → 从本帧浮点区（帧底 + 0x20）恢复 S0-S15 与
  FPSCR，然后回收预留区（若压栈 xPSR 的 bit 9 置位，再补回 4 字节对齐字）。
- bit4 = 1 → 标准帧：只回收 8 字状态帧，浮点寄存器不动。
- `FPCCR.CLRONRET`/`CLRONRETS`（`clear_on_return`）：任一位即可启用清除，
  因为本模型没有 TrustZone，安全（`CLRONRETS`）与非安全（`CLRONRET`）两个
  变体折叠到唯一的安全状态。若返回的扩展帧确实卸载了上下文（`LSPACT = 0`），
  在恢复之后额外把 S0-S15 与 FPSCR 清零，符合 ARMv8-M 的意图（`CLRONRET`
  的存在就是为了防止特权浮点数据跨异常返回泄漏给非特权软件）；S16-S31 与
  `FPCA`/`FPEXC.EN` 上下文保持活跃。返回基础帧（`EXC_RETURN` bit4 = 1）时
  则关闭上下文：清除 `CONTROL.FPCA` 与 `FPEXC.EN`，使下一次异常进入重新使用
  标准帧。惰性帧尚未提交（`LSPACT` 仍为 1）时寄存器原样保留 —— 它们仍属于
  被打断的上下文。
- MSP/PSP 选择与 `EXC_RETURN` 校验逻辑保持不变。因此 `EXC_RETURN` 取值为
  `0xFFFF_FFF1`/`0xFFFF_FFE1`（Handler）、`0xFFFF_FFF9`/`0xFFFF_FFE9`
  （Thread MSP）、`0xFFFF_FFFD`/`0xFFFF_FFED`（Thread PSP），bit4 = 0 表示
  扩展帧。
- 每次异常返回还会置 `CONTROL.FPCA = !EXC_RETURN[4]`（ARMv8-M B3.4.4）：
  基础帧恢复后 `FPCA = 0`，扩展帧恢复后 `FPCA = 1`。因此处理程序内
  `MSR CONTROL` 写入的值会被返回帧类型覆盖。`nPRIV` 不恢复（由软件负责），
  `SPSEL` 仅在返回到 Thread 模式时跟随 `EXC_RETURN[2]`。
- 从 Handler 模式进入的嵌套异常产生 `EXC_RETURN = 0xFFFF_FFF1`（bit3 = 0），
  因此返回到被抢占的处理程序，绝不会触发 `SLEEPONEXIT`。

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

**Not modeled / 未建模**: `FPCCR.LSPENS`/`THREAD`/`USER` are storage only (no
secure state for `LSPENS` to protect, and the core never allocates a frame
from Handler or unprivileged mode so `THREAD`/`USER` have nothing to report);
`ASPEN = 0` does not suppress `CONTROL.FPCA` in this model; `FPDSCR` does not
influence arithmetic; double precision and FPSCR traps are absent. The
secure/non-secure FP banks of the TrustZone configuration are not modeled, so
`CLRONRETS` collapses onto the single non-secure state, and `TS` is hard-wired
to 1 instead of following the Cortex-M33 reset value of 0.

未建模：`FPCCR.LSPENS`/`THREAD`/`USER` 仅作存储（没有可供 `LSPENS` 保护的安全
状态；核心也不会从 Handler 或非特权模式分配栈帧，因此 `THREAD`/`USER` 无内容
可上报）；本模型中 `ASPEN = 0` 不会抑制 `CONTROL.FPCA`；`FPDSCR` 不影响算术；
不实现双精度与 FPSCR 陷阱。TrustZone 的安全/非安全浮点寄存器组未建模，因此
`CLRONRETS` 折叠到唯一的非安全状态，`TS` 恒为 1 而非遵循 Cortex-M33 的复位
值 0。

### Not implemented / 未实现清单

These encodings are currently reported as **undefined instructions** (PC and
opcode logged) and, by default, escalate to HardFault; `set_strict_decode(true)`
turns them into a host-visible halt with the opcode in the error. They are
never silently skipped. They are tracked so that firmware relying on them can
be identified immediately.

以下编码当前按**未定义指令**上报（记录 PC 与 opcode），默认升级为 HardFault；
`set_strict_decode(true)` 会把它们变成宿主可见的停机，并在错误中携带操作码。
绝不静默跳过。列表用于快速定位依赖这些功能的固件。

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

**Vectors** are fetched from `VTOR + 4 * exception_number`: PendSV at
`VTOR + 0x38`, SysTick at `VTOR + 0x3C`, external IRQ `n` at
`VTOR + 0x40 + 4n`. `VTOR` masks its low 7 bits, so the table is
128-byte aligned.
**向量**取自 `VTOR + 4 * 异常号`：PendSV 位于 `VTOR + 0x38`、SysTick 位于
`VTOR + 0x3C`、外部 IRQ `n` 位于 `VTOR + 0x40 + 4n`。`VTOR` 低 7 位被屏蔽，
因此向量表按 128 字节对齐。

**Stack frame on exception entry** (8 words): R0-R3, R12, LR, PC, XPSR.
When the FPU is active the frame is **extended** with the 17-word FP context
(S0-S15, FPSCR) plus one padding word; see
[Lazy FP Stacking and Extended Frames](#lazy-fp-stacking-and-extended-frames--惰性浮点压栈与扩展帧).
**异常进入栈帧**（8 字）：R0-R3、R12、LR、PC、XPSR。浮点单元活跃时栈帧为**扩展帧**，
额外包含 17 字浮点上下文（S0-S15、FPSCR）与 1 字填充；详见
[惰性浮点压栈与扩展帧](#lazy-fp-stacking-and-extended-frames--惰性浮点压栈与扩展帧).
**EXC_RETURN** values: `0xFFFFFFF1` (return to handler on MSP), `0xFFFFFFF5`
(return to handler on PSP), `0xFFFFFFF9` (thread MSP), `0xFFFFFFFD` (thread PSP).
With an extended frame bit 4 is cleared, giving `0xFFFFFFE1` / `0xFFFFFFE5` /
`0xFFFFFFE9` / `0xFFFFFFED`. Exception entry always switches the handler to MSP
(`CONTROL.SPSEL = 0`); bit 2 records the interrupted SPSEL and the return
restores it, so a thread or handler that was using PSP resumes on PSP.
**EXC_RETURN** 取值：`0xFFFFFFF1`（返回 Handler，MSP）、`0xFFFFFFF5`（返回
Handler，PSP）、`0xFFFFFFF9`（Thread MSP）、`0xFFFFFFFD`（Thread PSP）。扩展帧会
清除 bit4，得到 `0xFFFFFFE1` / `0xFFFFFFE5` / `0xFFFFFFE9` / `0xFFFFFFED`。
异常进入总是让处理程序使用 MSP（`CONTROL.SPSEL = 0`）；bit2 记录被打断的 SPSEL，
返回时恢复，因此原本使用 PSP 的线程或处理程序会继续使用 PSP。

### System exceptions: NMI, SVCall, PendSV, SysTick / 系统异常

Execution numbering and pending representation are kept separate from the NVIC:

- `Nvic` owns only external IRQ lines `0..=255`: `ISPR`/`ICPR`, `ISER`/`ICER`,
  `IABR` and `IPR`. Bit `n` always means external interrupt `n`, whose exception
  number is `16 + n`. Software-pending bit 15 through `ISPR`/`STIR` therefore
  enters **exception 31**, never SysTick.
- `Scb` (`scb.rs`) owns four system-exception latches — `ICSR.NMIPENDSET` (2),
  `SHCSR.SVCALLPENDED` (11), `ICSR.PENDSVSET` (14) and `ICSR.PENDSTSET` (15) —
  plus `SHPR2` (SVCall byte 3) and `SHPR3` (PendSV byte 2, SysTick byte 3).
- `SVC #imm` retires, pends exception 11, and the shared arbitration at the end
  of the same `step` enters `VTOR + 0x2C` when the priority gate allows. The
  handler recovers `imm` from the stacked return address: the frame holds the
  address after the `SVC`, so the instruction at `stacked_pc - 2` is the `SVC`
  itself. `SHCSR.SVCALLACT` / `SVCALLPENDED` mirror the live state. `PRIMASK`
  does not delay SVCall (a synchronous supervisor call latched in `SHCSR`), but
  `FAULTMASK` and `BASEPRI` do.
- An `ICSR.NMIPENDSET` write of 1 pends NMI; a write of 0 is ignored, and only
  taking exception 2 clears the latch. NMI enters `VTOR + 0x08` and cannot be
  masked by `PRIMASK`, `FAULTMASK` or `BASEPRI`.
- SysTick expiry (`SYST_CSR.TICKINT` plus a counter wrap) calls `pend_systick()`,
  which latches `PENDSTSET` and `SYST_CSR.COUNTFLAG`. Writing `PENDSTSET` from
  software has exactly the same effect (ARMv8-M B3.2.6).
- Taking an exception clears its latch (one-shot); `PENDSVCLR` / `PENDSTCLR`
  clear PendSV/SysTick from software, while NMI and SVCall have no software
  clear. `VECTPENDING` reads the highest-priority pending exception number
  (0 when none, including NMI = 2 and SVCall = 11), `VECTACTIVE` mirrors `IPSR`,
  `RETTOBASE` is 1 while at most one exception is active, and `ISRPENDING`
  reports any NVIC line even when disabled.

Arbitration is one ranked pass over all sources (`faults::RANK_*`):

1. NMI has fixed rank 0 and HardFault rank 1. NMI therefore preempts every
   configurable exception **and** HardFault, matching ARMv8-M B1.5.4
   (`Reset > NMI > HardFault`).
2. External candidates come from `Nvic::find_highest_priority()` (enabled,
   pending, not active); SVCall/PendSV/SysTick candidates come from their
   latches. Configurable rank is `2 + priority`. A numerically smaller priority
   wins; ties compare the **exception number**, so SVCall (11) beats PendSV (14),
   which beats SysTick (15), and all three beat every external interrupt (16+).
3. `FAULTMASK` blocks every configurable exception, `PRIMASK` blocks every
   configurable exception except SVCall, `BASEPRI` blocks priorities numerically
   at or above its value, and an active exception is only preempted by a
   **strictly** higher priority. NMI and HardFault bypass all three masks.
   Sleeping cores use the same gate, so a wake-up never takes a masked exception.
4. `WFI`, `WFE` and `SLEEPONEXIT` treat any pending system-exception latch
   (including NMI and SVCall) as a wake source exactly like an enabled NVIC line.

Diagnostics: `system_exception_state()` (`systick=<state> pendsv=<state>`) is
the stable string exposed by the core; `pend_nmi()`, `pend_svcall()`,
`pend_pendsv()` and `pend_systick()` are the host entry points. In the full
machine the `rp2350-periph` PPB stub replays `ICSR`/`SHPR2`/`SHPR3` writes into
the core, so firmware pends NMI/PendSV and sets system priorities; delivering a
peripheral-side SysTick expiry to the core needs the driver to forward
`BootPeripherals::take_systick_pending()` to `pend_systick()` (CLI wiring,
outside this crate). The PPB stub does **not** yet replay `SHCSR`, `CFSR`,
`HFSR` or `AIRCR`, so firmware-side fault enables and `SYSRESETREQ` only take
effect when a host writes them through `CpuCore::system_write` directly.

执行编号与挂起表示与 NVIC 分开：

- `Nvic` 只拥有外部中断线 `0..=255`：`ISPR`/`ICPR`、`ISER`/`ICER`、`IABR` 与
  `IPR`。位 `n` 永远是外部中断 `n`（异常号 `16 + n`）。因此通过 `ISPR`/`STIR`
  软件挂起位 15 进入的是**异常 31**，绝不是 SysTick。
- `Scb`（`scb.rs`）拥有四个系统异常锁存——`ICSR.NMIPENDSET`（2）、
  `SHCSR.SVCALLPENDED`（11）、`ICSR.PENDSVSET`（14）与 `ICSR.PENDSTSET`（15）
  ——以及 `SHPR2`（第 3 字节 SVCall）与 `SHPR3`（第 2 字节 PendSV、第 3 字节
  SysTick）。
- `SVC #imm` 退役后挂起异常 11，同一步末尾的统一仲裁在优先级允许时进入
  `VTOR + 0x2C`。处理程序从压栈返回地址取回 `imm`：栈帧保存 `SVC` 的后继地址，
  因此 `stacked_pc - 2` 处的指令就是 `SVC` 本身。`SHCSR.SVCALLACT`/
  `SVCALLPENDED` 反映实时状态。`PRIMASK` 不会延迟 SVCall（同步监管调用已锁存在
  `SHCSR`），但 `FAULTMASK` 与 `BASEPRI` 会。
- `ICSR.NMIPENDSET` 写 1 挂起 NMI；写 0 被忽略，只有取走异常 2 才清除锁存。NMI
  从 `VTOR + 0x08` 进入，`PRIMASK`、`FAULTMASK`、`BASEPRI` 都无法屏蔽它。
- SysTick 到期（`SYST_CSR.TICKINT` 且计数器回绕）调用 `pend_systick()`，置位
  `PENDSTSET` 与 `SYST_CSR.COUNTFLAG`。软件写 `PENDSTSET` 效果完全相同
  （ARMv8-M B3.2.6）。
- 取走异常即清除锁存（一次性）；`PENDSVCLR`/`PENDSTCLR` 由软件清除
  PendSV/SysTick，NMI 与 SVCall 没有软件清除位。`VECTPENDING` 读出最高优先级
  挂起异常的异常号（无挂起为 0，含 NMI = 2 与 SVCall = 11），`VECTACTIVE`
  等于 `IPSR`，最多只有一个活跃异常时 `RETTOBASE` 为 1，`ISRPENDING` 报告任何
  NVIC 挂起线（即使未使能）。

仲裁对所有来源做一次等级遍历（`faults::RANK_*`）：

1. NMI 固定等级 0、HardFault 等级 1。因此 NMI 抢占所有可配置异常**以及**
   HardFault，符合 ARMv8-M B1.5.4（`Reset > NMI > HardFault`）。
2. 外部候选来自 `Nvic::find_highest_priority()`（已使能、挂起、非活跃）；
   SVCall/PendSV/SysTick 候选来自各自锁存。可配置等级为 `2 + 优先级`。优先级
   数值更小者胜；并列时比较**异常号**，因此 SVCall（11）优先于 PendSV（14），
   后者优先于 SysTick（15），三者都优先于任何外部中断（16+）。
3. `FAULTMASK` 屏蔽所有可配置异常；`PRIMASK` 屏蔽除 SVCall 外的可配置异常；
   `BASEPRI` 屏蔽数值大于等于其值的优先级；活跃异常只能被**严格**更高优先级
   抢占。NMI 与 HardFault 绕过这三种屏蔽。睡眠核使用同一门控，因此唤醒绝不会
   取走被屏蔽的异常。
4. `WFI`、`WFE` 与 `SLEEPONEXIT` 把任何系统异常挂起锁存（含 NMI 与 SVCall）
   视作与已使能 NVIC 线相同的唤醒源。

诊断：`system_exception_state()`（`systick=<state> pendsv=<state>`）是核心暴露的
稳定字符串；`pend_nmi()`、`pend_svcall()`、`pend_pendsv()`、`pend_systick()` 是
宿主入口。完整机器中 `rp2350-periph` 的 PPB 桩已把 `ICSR`/`SHPR2`/`SHPR3` 写入
回放到核，因此固件可挂起 NMI/PendSV 并设置系统优先级；把外设侧 SysTick 到期投递
到核仍需驱动把 `BootPeripherals::take_systick_pending()` 转发到 `pend_systick()`
（属 CLI 接线，不在本 crate 内）。PPB 桩**尚未**回放 `SHCSR`、`CFSR`、`HFSR`、
`AIRCR`，因此固件侧写故障使能或 `SYSRESETREQ` 只有在宿主直接通过
`CpuCore::system_write` 写入时才生效。

### Fault escalation / 故障升级

Every synchronous fault funnels through `CortexM33Core::finish_instruction` and
`raise_sync_fault`, so the enabled-handler rule and the stacked return address
are identical for all sources:

| Source / 来源 | CFSR / HFSR | Handler / 处理程序 |
|---------------|-------------|--------------------|
| Undefined instruction / 未定义指令 | `UFSR.UNDEFINSTR` (16) | UsageFault 6 if `USGFAULTENA` **and** priority allows, else HardFault 3 + `HFSR.FORCED` |
| Illegal `EXC_RETURN` / 非法返回 | `UFSR.INVSTATE` (17) | same / 同上 |
| Coprocessor access denied / 协处理器拒绝 | `UFSR.NOCP` (19) | same / 同上 |
| `SVC` / 监管调用 | — | SVCall 11 (pending in `SHCSR.SVCALLPENDED`) |
| `BKPT` without debugger / 无调试器 | `HFSR.DEBUGEVT` (31) | HardFault 3 |
| Frame crosses `MSPLIM`/`PSPLIM` | `UFSR.STKOF` (20) + `HFSR.FORCED` | HardFault 3 |
| Bus error while stacking / 压栈总线错误 | `BFSR.STKERR` (12) + `HFSR.FORCED` | HardFault 3 |

- Vectors: NMI `VTOR + 0x08`, HardFault `VTOR + 0x0C`, UsageFault `VTOR + 0x18`,
  SVCall `VTOR + 0x2C`.
- HardFault has the highest configurable level and ignores `PRIMASK`, `BASEPRI`
  and `FAULTMASK`; only NMI outranks it (ARMv8-M B1.5.4).
- A synchronous fault frame stacks the **faulting instruction** address (not its
  successor), which is the ARM ARM rule for `UNDEFINSTR`/`INVSTATE`; `SVC` is the
  exception and stacks its successor so the handler can read `imm` at
  `stacked_pc - 2`. `MSPLIM`/`PSPLIM` overflow is detected before stacking; the
  escalated HardFault frame deliberately skips the limit check so the handler is
  still reachable (real hardware would lock up when the second frame also
  overflows, which this model reports as a propagated bus error).
- `set_strict_decode(true)` keeps the host diagnostic behaviour: an undefined
  instruction returns `CoreError::UndefinedInstruction` and halts the core
  instead of delivering HardFault. The default (`false`) is architectural.

- 向量：NMI `VTOR + 0x08`、HardFault `VTOR + 0x0C`、UsageFault `VTOR + 0x18`、
  SVCall `VTOR + 0x2C`。
- HardFault 处于最高可配置层级，且忽略 `PRIMASK`、`BASEPRI`、`FAULTMASK`；
  只有 NMI 高于它（ARMv8-M B1.5.4）。
- 同步故障栈帧压入**故障指令**地址（不是后继地址），这是 ARM ARM 对
  `UNDEFINSTR`/`INVSTATE` 的规则；`SVC` 是例外，压入后继地址，使处理程序能在
  `stacked_pc - 2` 读到 `imm`。`MSPLIM`/`PSPLIM` 溢出在压栈前检测；升级后的
  HardFault 帧刻意跳过上限检查，保证处理程序仍可到达（真实硬件在第二个帧也溢出
  时会进入 lockup，本模型改为传播总线错误）。
- `set_strict_decode(true)` 保留宿主诊断行为：未定义指令返回
  `CoreError::UndefinedInstruction` 并停机，而不是投递 HardFault。默认（`false`）
  为架构行为。

### Fault and control registers / 故障与控制寄存器

| Register | Address | Model / 建模 |
|----------|---------|--------------|
| `SHCSR` | `0xE000_ED24` | Writable `MEMFAULTENA`/`BUSFAULTENA`/`USGFAULTENA`; `MEMFAULTACT`/`BUSFAULTACT`/`USGFAULTACT`/`NMIACT`/`SVCALLACT`/`PENDSVACT`/`SYSTICKACT`/`HARDFAULTACT` are live views of the exception stack; `SVCALLPENDED` mirrors the SCB latch; `HARDFAULTPENDED` reads 0 (HardFault is always entered synchronously). |
| `CFSR` | `0xE000_ED28` | MMFSR/BFSR/UFSR bits for `UNDEFINSTR`, `INVSTATE`, `INVPC`, `NOCP`, `STKOF`, `STKERR`, `UNSTKERR`, `IBUSERR`, `PRECISERR`, `IMPRECISERR`, `UNALIGNED`, `DIVBYZERO`; write-one-to-clear. |
| `HFSR` | `0xE000_ED2C` | `FORCED` (30), `DEBUGEVT` (31), `VECTTBL` (1); write-one-to-clear. `VECTTBL` has no source yet. |
| `AIRCR` | `0xE000_ED0C` | A write is accepted only with `VECTKEY = 0x05FA`; `PRIS` (14) is stored; `SYSRESETREQ` (2) latches a one-shot event read by `take_system_reset_request()`; `VECTRESET` (0) is latched for diagnostics (`vect_reset_requested()`); `VECTKEYSTAT` reads `0xFA05`, write-only request bits read 0. |
| `SHPR1` | `0xE000_ED18` | **Not modelled**: MemManage/BusFault/UsageFault priorities report 0. |
| `CCR` | `0xE000_ED14` | **Not modelled**: `UNALIGN_TRP`/`DIV_0_TRP`/`STKALIGN` behave as 0. |

The table above is the complete modelled fault surface. **Still unmodelled /
仍未建模**: TrustZone secure/non-secure state (there is one register file and one
`SHCSR`; `SECUREFAULT*` bits are absent), MemManage/BusFault sources from MPU or
data/instruction accesses (bus errors still surface as `CoreError` for the host
and halt the core), `SHPR1` priorities, `CCR` traps, `HFSR.VECTTBL`, debug-monitor
(`MONITORACT`) and lockup semantics, and `SHCSR.HARDFAULTPENDED`.

上表是完整的故障建模范围。**仍未建模**：TrustZone 安全/非安全态（只有一套
寄存器文件与一个 `SHCSR`，没有 `SECUREFAULT*` 位）、来自 MPU 或数据/取指访问的
MemManage/BusFault 源（总线错误仍以 `CoreError` 上报宿主并停机）、`SHPR1`
优先级、`CCR` 陷阱、`HFSR.VECTTBL`、调试监视器（`MONITORACT`）与 lockup 语义，
以及 `SHCSR.HARDFAULTPENDED`。

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
