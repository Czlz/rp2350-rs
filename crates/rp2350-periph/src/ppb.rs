//! Private peripheral bus stub / 私有外设总线桩
//!
//! The PPB window (SCB, NVIC, SysTick, MPU) is plain storage so `ldr`/`str`
//! accesses from firmware do not halt on an unmapped address. `CPUID` reports a
//! Cortex-M33 part number. Writes to the NVIC, `ICSR`, `SHCSR`, `CFSR`, `HFSR`,
//! `AIRCR`, `CCR`, `SHPR1`/`SHPR2`/`SHPR3`, `MMFAR`, `BFAR`, `SCR`, `STIR` and
//! SysTick registers are also appended to a shared log; the simulation driver
//! replays them into the core through `CpuCore::system_write`, which is how
//! interrupt enables, fault enables, fault status clears, priorities, the
//! unaligned/divide-by-zero traps, fault-address read-back, PendSV/SysTick
//! pending requests, `SLEEPONEXIT`, `AIRCR.SYSRESETREQ` and software-triggered
//! interrupts reach the emulated core.
//!
//! `SHCSR`, `CFSR`, `HFSR`, `AIRCR`, `CCR`, `SHPR1`, `MMFAR` and `BFAR` are
//! architecturally owned by the core, so the PPB keeps only a read-back cache
//! for them and reproduces the core's write rules (write-one-to-clear for
//! `CFSR`/`HFSR`, the three fault enables for `SHCSR`, the `VECTKEY` gate plus
//! `PRIS` for `AIRCR`, `STKALIGN` read-as-one plus the five writable bits for
//! `CCR`, the three priority bytes for `SHPR1`, and the address field for
//! `MMFAR`/`BFAR`). The cache must not invent a second semantics: live
//! `*ACT`/`SVCALLPENDED` views, fault bits raised inside the core and fault
//! addresses recorded by the core's own fault paths stay core-only.
//!
//! SysTick is the exception: its counter follows the live clock tree, so this
//! module owns a [`SysTickCounter`] instead of replaying the four SysTick
//! registers. A write still reaches the shared log only for registers whose
//! behaviour lives inside the core (`ICSR`, `SHCSR`, `CFSR`, `HFSR`, `AIRCR`,
//! `CCR`, `SHPR1`/`SHPR2`/`SHPR3`, `MMFAR`, `BFAR`, `SCR`, `STIR`, NVIC).
//! PPB 窗口（SCB、NVIC、SysTick、MPU）为普通存储，固件发出的 `ldr`/`str`
//! 不会因地址未映射而停机。`CPUID` 报告 Cortex-M33 部件号。对 NVIC、`ICSR`、
//! `SHCSR`、`CFSR`、`HFSR`、`AIRCR`、`CCR`、`SHPR1`/`SHPR2`/`SHPR3`、`MMFAR`、
//! `BFAR`、`SCR`、`STIR` 与 SysTick 寄存器的写入还会追加到共享日志；仿真驱动
//! 通过 `CpuCore::system_write` 把它们回放到核中，中断使能、故障使能、故障状态
//! 清除、优先级、未对齐/除零陷阱、故障地址读回、PendSV/SysTick 挂起请求、
//! `SLEEPONEXIT`、`AIRCR.SYSRESETREQ` 与软件触发中断由此到达被模拟的核。
//!
//! `SHCSR`、`CFSR`、`HFSR`、`AIRCR`、`CCR`、`SHPR1`、`MMFAR` 与 `BFAR` 架构上
//! 属于核，因此 PPB 只为它们保留读回缓存，并复现核的写入规则（`CFSR`/`HFSR`
//! 写 1 清零、`SHCSR` 只保存三个故障使能、`AIRCR` 的 `VECTKEY` 门控与 `PRIS`、
//! `CCR` 的 `STKALIGN` 恒 1 与五个可写位、`SHPR1` 的三个优先级字节、
//! `MMFAR`/`BFAR` 的地址字段）。缓存不得产生第二套语义：`*ACT`/`SVCALLPENDED`
//! 实时视图、核内故障路径置位的状态位与核内记录的故障地址仍只存在于核内。
//!
//! SysTick 是例外：其计数器跟随实时时钟树，因此本模块自己持有
//! [`SysTickCounter`]，不把四个 SysTick 寄存器回放给核。只有行为属于核内部的
//! 寄存器（`ICSR`、`SHCSR`、`CFSR`、`HFSR`、`AIRCR`、`CCR`、`SHPR1`/`SHPR2`/
//! `SHPR3`、`MMFAR`、`BFAR`、`SCR`、`STIR`、NVIC）才进入共享日志。

use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;
use super::systick::{CALIB, SysTickCounter};

/// `SCB_CPUID` offset / `SCB_CPUID` 偏移
const SCB_CPUID: u32 = 0xE000_ED00 - map::PPB_BASE;

/// Cortex-M33 `CPUID` value (implementer ARM, part 0xD21)
/// Cortex-M33 `CPUID` 值（实现者 ARM，部件号 0xD21）
const CPUID_CORTEX_M33: u32 = 0x410F_D210;

/// Maximum logged writes kept for replay / 为回放保留的最大日志条数
///
/// Boot-time configuration fits well within this bound; steady-state `ICPR`
/// writes beyond it are dropped, which only delays cleared pending bits.
/// 启动期配置远小于该上限；超出的稳态 `ICPR` 写会被丢弃，仅会延迟已清除
/// 的挂起位生效。
const LOG_CAPACITY: usize = 256;

/// `SYST_CSR` offset inside the PPB window / PPB 窗口中的 `SYST_CSR` 偏移
pub const SYST_CSR: u32 = 0xE010;

/// `SYST_RVR` offset / `SYST_RVR` 偏移
pub const SYST_RVR: u32 = 0xE014;

/// `SYST_CVR` offset / `SYST_CVR` 偏移
pub const SYST_CVR: u32 = 0xE018;

/// `SYST_CALIB` offset / `SYST_CALIB` 偏移
pub const SYST_CALIB: u32 = 0xE01C;

/// `SCB_SCR` absolute address / `SCB_SCR` 绝对地址
pub const SCB_SCR: u32 = 0xE000_ED10;

/// `SCB_ICSR` absolute address / `SCB_ICSR` 绝对地址
pub const SCB_ICSR: u32 = 0xE000_ED04;

/// `SCB_AIRCR` absolute address (`VECTKEY`/`PRIS`/reset requests)
/// `SCB_AIRCR` 绝对地址（`VECTKEY`/`PRIS`/复位请求）
pub const SCB_AIRCR: u32 = 0xE000_ED0C;

/// `SCB_SHCSR` absolute address (fault enables plus active/pending views)
/// `SCB_SHCSR` 绝对地址（故障使能与活跃/挂起视图）
pub const SCB_SHCSR: u32 = 0xE000_ED24;

/// `SCB_CFSR` absolute address (write-one-to-clear fault status)
/// `SCB_CFSR` 绝对地址（写 1 清零的故障状态）
pub const SCB_CFSR: u32 = 0xE000_ED28;

/// `SCB_HFSR` absolute address (write-one-to-clear HardFault status)
/// `SCB_HFSR` 绝对地址（写 1 清零的 HardFault 状态）
pub const SCB_HFSR: u32 = 0xE000_ED2C;

/// `SHCSR` bits software may change (`MEMFAULTENA`/`BUSFAULTENA`/`USGFAULTENA`)
/// 软件可写的 `SHCSR` 位（三个故障使能）
const SHCSR_WRITABLE_MASK: u32 = 0x0007_0000;

/// `SCB_CCR` absolute address (unaligned/divide-by-zero traps)
/// `SCB_CCR` 绝对地址（未对齐/除零陷阱）
pub const SCB_CCR: u32 = 0xE000_ED14;

/// `SCB_SHPR1` absolute address (MemManage/BusFault/UsageFault priorities)
/// `SCB_SHPR1` 绝对地址（MemManage/BusFault/UsageFault 优先级）
pub const SCB_SHPR1: u32 = 0xE000_ED18;

/// `SCB_MMFAR` absolute address (MemManage fault address field)
/// `SCB_MMFAR` 绝对地址（MemManage 故障地址字段）
pub const SCB_MMFAR: u32 = 0xE000_ED34;

/// `SCB_BFAR` absolute address (BusFault fault address field)
/// `SCB_BFAR` 绝对地址（BusFault 故障地址字段）
pub const SCB_BFAR: u32 = 0xE000_ED38;

/// `CCR.STKALIGN` — read-only one in ARMv8-M and the complete reset value
/// `CCR.STKALIGN` —— ARMv8-M 中只读 1，也是完整的复位值
const CCR_STKALIGN: u32 = 1 << 9;

/// `CCR` bits software may change: `NONBASETHRDENA`(0), `USERSETMPEND`(1),
/// `UNALIGN_TRP`(3), `DIV_0_TRP`(4) and `BFHFNMIGN`(8)
///
/// `STKALIGN`(9) is excluded because it is RAO/WI; every reserved bit reads zero.
/// 软件可写的 `CCR` 位：`NONBASETHRDENA`(0)、`USERSETMPEND`(1)、
/// `UNALIGN_TRP`(3)、`DIV_0_TRP`(4) 与 `BFHFNMIGN`(8)。
/// `STKALIGN`(9) 因 RAO/WI 被排除；所有保留位读 0。
const CCR_WRITABLE_MASK: u32 = 0x0000_011B;

/// `SHPR1` bytes software may change: bytes 0/1/2 keep the implemented
/// priority bits only, byte 3 is reserved and reads zero
///
/// The mask mirrors `rp2350-arm::scb` (`0xE0` per priority byte, three
/// implemented bits on this model).
/// 软件可写的 `SHPR1` 字节：第 0/1/2 字节只保存实现的优先级位，第 3 字节保留
/// 读 0。掩码与 `rp2350-arm::scb` 一致（每个优先级字节 `0xE0`，本模型三位）。
const SHPR1_WRITABLE_MASK: u32 = 0x00E0_E0E0;

/// Value `AIRCR[31:16]` must hold for a write to be accepted
/// `AIRCR[31:16]` 必须等于该值才接受写入
const AIRCR_VECTKEY: u32 = 0x05FA_0000;

/// Value always read back from `AIRCR.VECTKEYSTAT`
/// `AIRCR.VECTKEYSTAT` 恒读回该值
const AIRCR_VECTKEYSTAT: u32 = 0xFA05_0000;

/// `AIRCR.PRIS` storage bit / `AIRCR.PRIS` 存储位
const AIRCR_PRIS: u32 = 1 << 14;

/// `SCB_SHPR2` absolute address (SVCall priority in byte 3)
/// `SCB_SHPR2` 绝对地址（第 3 字节为 SVCall 优先级）
pub const SCB_SHPR2: u32 = 0xE000_ED1C;

/// `SCB_SHPR3` absolute address (PendSV byte 2, SysTick byte 3)
/// `SCB_SHPR3` 绝对地址（第 2 字节 PendSV、第 3 字节 SysTick）
pub const SCB_SHPR3: u32 = 0xE000_ED20;

/// `SCB_STIR` absolute address / `SCB_STIR` 绝对地址
pub const SCB_STIR: u32 = 0xE000_EF00;

/// `ICSR.PENDSVSET` latch / `ICSR.PENDSVSET` 锁存
const ICSR_PENDSVSET: u32 = 1 << 28;

/// `ICSR.PENDSVCLR` write-1-to-clear bit / `ICSR.PENDSVCLR` 写 1 清除位
const ICSR_PENDSVCLR: u32 = 1 << 27;

/// `ICSR.PENDSTSET` latch / `ICSR.PENDSTSET` 锁存
const ICSR_PENDSTSET: u32 = 1 << 26;

/// `ICSR.PENDSTCLR` write-1-to-clear bit / `ICSR.PENDSTCLR` 写 1 清除位
const ICSR_PENDSTCLR: u32 = 1 << 25;

/// Pend bits kept by the PPB storage view / PPB 存储视图保存的挂起位
const ICSR_LATCH_MASK: u32 = ICSR_PENDSVSET | ICSR_PENDSTSET;

/// Shared SysTick handle / 共享 SysTick 句柄
pub type SysTickHandle = Arc<Mutex<SysTickCounter>>;

/// Create a SysTick counter in its reset state / 创建复位状态的 SysTick 计数器
#[must_use]
pub fn new_systick() -> SysTickHandle {
    Arc::new(Mutex::new(SysTickCounter::new()))
}

/// Shared write log `(absolute address, value)` / 共享写入日志（绝对地址、值）
pub type SystemWriteLog = Arc<Mutex<Vec<(u32, u32)>>>;

/// Create an empty system write log / 创建空的系统写入日志
#[must_use]
pub fn new_system_write_log() -> SystemWriteLog {
    Arc::new(Mutex::new(Vec::new()))
}

/// Is `addr` a system control space register replayed into the core?
/// `addr` 是否为需要回放到核的系统控制空间寄存器？
///
/// `CPACR` must be replayed because it lives in the core, not in the PPB
/// storage: firmware's `arm_cpu_hw->cpacr |= ...` in `runtime_init` is the only
/// thing that enables the FPU, and without the replay every VFP instruction
/// raises UsageFault `NOCP`. `SCR` and `STIR` are replayed for the same reason:
/// `SLEEPONEXIT` and software-triggered interrupts are core state.
/// `ICSR`/`SHPR2`/`SHPR3` carry the PendSV/SysTick pending latches and the
/// system-exception priorities, which only the core can deliver or preempt.
/// `SHCSR`/`CFSR`/`HFSR`/`AIRCR` carry the fault enables, the fault status
/// latches and the reset requests: without the replay firmware never enables
/// UsageFault, never clears a fault status and never resets the chip.
/// `CCR` carries the `UNALIGN_TRP`/`DIV_0_TRP` traps and `SHPR1` the three fault
/// priorities; both gate core behaviour that the PPB cannot synthesise.
/// `MMFAR`/`BFAR` are the ARMv8-M read/write fault-address fields: the replay
/// makes firmware read-back match the core, and the `CFSR` valid bits stay
/// core-owned so a plain address write can never forge a valid fault address.
/// SysTick is deliberately **not** replayed: [`SysTickCounter`] follows the live
/// clock tree in this crate.
/// `CPACR` 必须回放，因为它属于核而不是 PPB 存储：固件在 `runtime_init` 里对
/// `arm_cpu_hw->cpacr |= ...` 的写入是启用 FPU 的唯一动作，不回放的话每条 VFP
/// 指令都会触发 UsageFault `NOCP`。`SCR` 与 `STIR` 同理：`SLEEPONEXIT` 与软件
/// 触发中断都是核状态。`ICSR`/`SHPR2`/`SHPR3` 承载 PendSV/SysTick 挂起锁存与
/// 系统异常优先级，只有核才能投递或抢占它们。`SHCSR`/`CFSR`/`HFSR`/`AIRCR`
/// 承载故障使能、故障状态锁存与复位请求：不回放则固件永远无法启用 UsageFault、
/// 无法清除故障状态，也无法复位芯片。`CCR` 承载 `UNALIGN_TRP`/`DIV_0_TRP`
/// 陷阱，`SHPR1` 承载三个故障优先级，都是 PPB 无法自行合成的核行为。
/// `MMFAR`/`BFAR` 是 ARMv8-M 的读写故障地址字段：回放让固件读回与核一致，而
/// `CFSR` 的 valid 位仍归核所有，普通地址写入绝不可能伪造有效故障地址。
/// SysTick 有意**不**回放：本 crate 的 [`SysTickCounter`] 跟随实时时钟树。
fn is_replayed(addr: u32) -> bool {
    addr == 0xE000_ED08 // SCB_VTOR
        || addr == SCB_ICSR
        || addr == SCB_AIRCR
        || addr == SCB_SHCSR
        || addr == SCB_CFSR
        || addr == SCB_HFSR
        || addr == SCB_CCR
        || addr == SCB_SHPR1
        || addr == SCB_SHPR2
        || addr == SCB_SHPR3
        || addr == SCB_MMFAR
        || addr == SCB_BFAR
        || addr == SCB_SCR
        || addr == SCB_STIR
        || addr == 0xE000_ED88 // SCB_CPACR
        || (0xE000_E100..=0xE000_E4FF).contains(&addr) // NVIC
}

/// PPB window stub / PPB 窗口桩
#[derive(Debug)]
pub struct Ppb {
    regs: RegisterFile,
    system_writes: SystemWriteLog,
    systick: SysTickHandle,
}

impl Ppb {
    /// Create the window with a private log and SysTick counter
    /// 创建窗口并使用私有日志与 SysTick 计数器
    #[must_use]
    pub fn new() -> Self {
        Self::with_log(new_system_write_log())
    }

    /// Create the window sharing `system_writes` / 创建窗口并共享 `system_writes`
    #[must_use]
    pub fn with_log(system_writes: SystemWriteLog) -> Self {
        Self::with_state(system_writes, new_systick())
    }

    /// Create the window sharing both the log and the SysTick counter
    /// 创建同时共享日志与 SysTick 计数器的窗口
    #[must_use]
    pub fn with_state(system_writes: SystemWriteLog, systick: SysTickHandle) -> Self {
        let mut ppb = Self {
            regs: RegisterFile::new(map::PPB_WINDOW_SIZE),
            system_writes,
            systick,
        };
        ppb.reset_shadow();
        ppb
    }

    /// Shared SysTick counter / 共享的 SysTick 计数器
    #[must_use]
    pub fn systick(&self) -> SysTickHandle {
        Arc::clone(&self.systick)
    }

    /// Handle one SysTick register access / 处理一次 SysTick 寄存器访问
    fn systick_access(&mut self, offset: u32, write: Option<u32>) -> Option<u32> {
        let mut counter = self.systick.lock();
        match offset {
            SYST_CSR => {
                if let Some(value) = write {
                    counter.write_csr(value);
                }
                Some(counter.csr())
            }
            SYST_RVR => {
                if let Some(value) = write {
                    counter.write_rvr(value);
                }
                Some(counter.rvr())
            }
            SYST_CVR => {
                if let Some(value) = write {
                    counter.write_cvr(value);
                }
                Some(counter.cvr())
            }
            SYST_CALIB => Some(CALIB),
            _ => None,
        }
    }

    /// Keep an architecturally sane `ICSR` shadow: `PENDSV*`/`PENDST*` follow
    /// write-1-to-set / write-1-to-clear, every other bit reads zero.
    /// 维护架构上合理的 `ICSR` 影子：`PENDSV*`/`PENDST*` 遵循写 1 置位 / 写 1 清除，
    /// 其它位读 0。
    ///
    /// The authoritative latches and the `VECTPENDING`/`VECTACTIVE` view live in
    /// the core (see `rp2350-arm`); this shadow exists so a firmware read-back of
    /// its own `ICSR` write sees the same latch instead of the raw written word.
    /// 权威锁存与 `VECTPENDING`/`VECTACTIVE` 视图位于核内（见 `rp2350-arm`）；这里
    /// 的影子只让固件读回自己写入的 `ICSR` 时看到同样的锁存，而不是原始写入字。
    fn write_icsr_shadow(&mut self, offset: u32, value: u32) {
        let mut latches = self.regs.read(offset).unwrap_or(0) & ICSR_LATCH_MASK;
        if value & ICSR_PENDSVSET != 0 {
            latches |= ICSR_PENDSVSET;
        }
        if value & ICSR_PENDSVCLR != 0 {
            latches &= !ICSR_PENDSVSET;
        }
        if value & ICSR_PENDSTSET != 0 {
            latches |= ICSR_PENDSTSET;
        }
        if value & ICSR_PENDSTCLR != 0 {
            latches &= !ICSR_PENDSTSET;
        }
        let _ = self.regs.write(offset, latches);
    }

    /// Load the reset values the core reports for the cached registers
    /// 载入核为缓存寄存器报告的复位值
    ///
    /// `CCR.STKALIGN` reads as one in ARMv8-M, so a firmware read before the
    /// first write must not see the all-zero `RegisterFile` reset value. The
    /// other shadows (`SHCSR`, `CFSR`, `HFSR`, `AIRCR`, `SHPR1`, `MMFAR`,
    /// `BFAR`) reset to zero, which already matches the core.
    /// ARMv8-M 中 `CCR.STKALIGN` 恒读 1，因此固件在首次写入前读到的不能是全零的
    /// `RegisterFile` 复位值。其它影子（`SHCSR`、`CFSR`、`HFSR`、`AIRCR`、
    /// `SHPR1`、`MMFAR`、`BFAR`）复位为 0，与核一致。
    fn reset_shadow(&mut self) {
        self.regs.reset();
        let _ = self.regs.write(SCB_CCR - map::PPB_BASE, CCR_STKALIGN);
    }

    /// Cache one `SHCSR`/`CFSR`/`HFSR`/`AIRCR`/`CCR`/`SHPR1`/`MMFAR`/`BFAR`
    /// write with the core's rules
    /// 按核的规则缓存一次 `SHCSR`/`CFSR`/`HFSR`/`AIRCR`/`CCR`/`SHPR1`/`MMFAR`/
    /// `BFAR` 写入
    ///
    /// The authoritative state lives in `rp2350-arm`; this cache only exists so
    /// a firmware read-back sees the same value. `SHCSR` keeps the three fault
    /// enables, `CFSR`/`HFSR` are write-one-to-clear, and `AIRCR` is ignored
    /// without `VECTKEY = 0x05FA` and otherwise keeps only `PRIS`. `CCR` keeps
    /// `STKALIGN` as one plus the five writable bits and reads every reserved
    /// bit as zero; `SHPR1` keeps the three priority bytes and reads the
    /// reserved byte 3 as zero. `MMFAR`/`BFAR` are read/write address fields in
    /// ARMv8-M, so the whole word is cached for read-back, but their validity
    /// bits live in `CFSR` and stay core-owned: an address write can never forge
    /// a valid fault address.
    /// 权威状态位于 `rp2350-arm`；缓存只为让固件读回看到相同的值。`SHCSR` 只保存
    /// 三个故障使能，`CFSR`/`HFSR` 写 1 清零，`AIRCR` 在缺少
    /// `VECTKEY = 0x05FA` 时整次忽略，否则只保存 `PRIS`。`CCR` 保存恒为 1 的
    /// `STKALIGN` 与五个可写位，其余保留位读 0；`SHPR1` 保存三个优先级字节，
    /// 保留的第 3 字节读 0。`MMFAR`/`BFAR` 在 ARMv8-M 中是读写地址字段，因此整字
    /// 缓存以便读回，但其 valid 位位于 `CFSR` 且归核所有：地址写入绝不可能伪造
    /// 有效故障地址。
    fn write_status_shadow(&mut self, offset: u32, addr: u32, value: u32) {
        let current = self.regs.read(offset).unwrap_or(0);
        let next = match addr {
            SCB_SHCSR => value & SHCSR_WRITABLE_MASK,
            SCB_CFSR | SCB_HFSR => current & !value,
            SCB_AIRCR => {
                if value & 0xFFFF_0000 != AIRCR_VECTKEY {
                    return;
                }
                value & AIRCR_PRIS
            }
            SCB_CCR => CCR_STKALIGN | (value & CCR_WRITABLE_MASK),
            SCB_SHPR1 => value & SHPR1_WRITABLE_MASK,
            // ARMv8-M 的 MMFAR/BFAR 是读写地址字段；valid 位在 CFSR 中单独保存。
            // MMFAR/BFAR are read/write address fields in ARMv8-M; the valid
            // bits live separately in CFSR.
            SCB_MMFAR | SCB_BFAR => value,
            _ => return,
        };
        let _ = self.regs.write(offset, next);
    }
}

impl Default for Ppb {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for Ppb {
    fn name(&self) -> &str {
        "PPB"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::PPB_BASE, map::PPB_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        let addr = map::PPB_BASE.saturating_add(offset);
        match offset {
            SCB_CPUID => Some(CPUID_CORTEX_M33),
            SYST_CSR..=SYST_CALIB if offset.trailing_zeros() >= 2 => {
                self.systick_access(offset, None)
            }
            // AIRCR.VECTKEYSTAT 恒读 0xFA05；只写的复位请求读 0，PRIS 读缓存值。
            // AIRCR.VECTKEYSTAT always reads 0xFA05; the write-only reset
            // requests read zero and PRIS reads the cached bit.
            _ if addr == SCB_AIRCR => self
                .regs
                .read(offset)
                .map(|value| value | AIRCR_VECTKEYSTAT),
            _ => self.regs.read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        if offset == SCB_CPUID {
            return true;
        }
        if (SYST_CSR..=SYST_CALIB).contains(&offset) && offset.trailing_zeros() >= 2 {
            let _ = self.systick_access(offset, Some(value));
            return true;
        }
        let addr = map::PPB_BASE.saturating_add(offset);
        if is_replayed(addr) {
            let mut log = self.system_writes.lock();
            if log.len() < LOG_CAPACITY {
                log.push((addr, value));
            }
        }
        match addr {
            SCB_ICSR => self.write_icsr_shadow(offset, value),
            SCB_SHCSR | SCB_CFSR | SCB_HFSR | SCB_AIRCR | SCB_CCR | SCB_SHPR1 | SCB_MMFAR
            | SCB_BFAR => {
                self.write_status_shadow(offset, addr, value);
            }
            _ => {
                let _ = self.regs.write(offset, value);
            }
        }
        true
    }

    fn reset(&mut self) {
        self.reset_shadow();
        self.systick.lock().reset();
    }
}

// 回放/影子测试放在独立文件，使本文件的寄存器实现保持在行数预算内。
// Replay/shadow tests live in their own file to keep the register
// implementation within the line budget.
#[cfg(test)]
#[path = "ppb_tests.rs"]
mod tests;
