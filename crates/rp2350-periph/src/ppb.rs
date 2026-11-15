//! Private peripheral bus stub / 私有外设总线桩
//!
//! The PPB window (SCB, NVIC, SysTick, MPU) is plain storage so `ldr`/`str`
//! accesses from firmware do not halt on an unmapped address. `CPUID` reports a
//! Cortex-M33 part number. Writes to the NVIC, `ICSR`, `SHPR2`/`SHPR3`, `SCR`,
//! `STIR` and SysTick registers are also appended to a shared log; the
//! simulation driver replays them into the core through `CpuCore::system_write`,
//! which is how interrupt enables, priorities, PendSV/SysTick pending requests,
//! `SLEEPONEXIT` and software-triggered interrupts reach the emulated core.
//!
//! SysTick is the exception: its counter follows the live clock tree, so this
//! module owns a [`SysTickCounter`] instead of replaying the four SysTick
//! registers. A write still reaches the shared log only for registers whose
//! behaviour lives inside the core (`ICSR`, `SHPR2`/`SHPR3`, `SCR`, `STIR`,
//! NVIC).
//! PPB 窗口（SCB、NVIC、SysTick、MPU）为普通存储，固件发出的 `ldr`/`str`
//! 不会因地址未映射而停机。`CPUID` 报告 Cortex-M33 部件号。对 NVIC、`ICSR`、
//! `SHPR2`/`SHPR3`、`SCR`、`STIR` 与 SysTick 寄存器的写入还会追加到共享日志；
//! 仿真驱动通过 `CpuCore::system_write` 把它们回放到核中，中断使能、优先级、
//! PendSV/SysTick 挂起请求、`SLEEPONEXIT` 与软件触发中断由此到达被模拟的核。
//!
//! SysTick 是例外：其计数器跟随实时时钟树，因此本模块自己持有
//! [`SysTickCounter`]，不把四个 SysTick 寄存器回放给核。只有行为属于核内部的
//! 寄存器（`ICSR`、`SHPR2`/`SHPR3`、`SCR`、`STIR`、NVIC）才进入共享日志。

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
/// SysTick is deliberately **not** replayed: [`SysTickCounter`] follows the live
/// clock tree in this crate.
/// `CPACR` 必须回放，因为它属于核而不是 PPB 存储：固件在 `runtime_init` 里对
/// `arm_cpu_hw->cpacr |= ...` 的写入是启用 FPU 的唯一动作，不回放的话每条 VFP
/// 指令都会触发 UsageFault `NOCP`。`SCR` 与 `STIR` 同理：`SLEEPONEXIT` 与软件
/// 触发中断都是核状态。`ICSR`/`SHPR2`/`SHPR3` 承载 PendSV/SysTick 挂起锁存与
/// 系统异常优先级，只有核才能投递或抢占它们。SysTick 有意**不**回放：本 crate 的
/// [`SysTickCounter`] 跟随实时时钟树。
fn is_replayed(addr: u32) -> bool {
    addr == 0xE000_ED08 // SCB_VTOR
        || addr == SCB_ICSR
        || addr == SCB_SHPR2
        || addr == SCB_SHPR3
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
        Self {
            regs: RegisterFile::new(map::PPB_WINDOW_SIZE),
            system_writes,
            systick,
        }
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
        match offset {
            SCB_CPUID => Some(CPUID_CORTEX_M33),
            SYST_CSR..=SYST_CALIB if offset.trailing_zeros() >= 2 => {
                self.systick_access(offset, None)
            }
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
        if addr == SCB_ICSR {
            self.write_icsr_shadow(offset, value);
        } else {
            let _ = self.regs.write(offset, value);
        }
        true
    }

    fn reset(&mut self) {
        self.regs.reset();
        self.systick.lock().reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_id_is_readable() {
        let mut ppb = Ppb::new();
        assert_eq!(ppb.read32(SCB_CPUID), Some(CPUID_CORTEX_M33));
    }

    #[test]
    fn vtor_is_storage() {
        let mut ppb = Ppb::new();
        let offset = 0xE000_ED08 - map::PPB_BASE;
        assert!(ppb.write32(offset, 0x1000_0000));
        assert_eq!(ppb.read32(offset), Some(0x1000_0000));
    }

    #[test]
    fn nvic_writes_are_logged_for_replay() {
        let log = new_system_write_log();
        let mut ppb = Ppb::with_log(Arc::clone(&log));
        // ISER0 at 0xE000_E100. ISER0 位于 0xE000_E100。
        assert!(ppb.write32(0xE100, 1));
        assert_eq!(log.lock().as_slice(), &[(0xE000_E100, 1)]);
    }

    #[test]
    fn vtor_writes_are_logged_for_replay() {
        let log = new_system_write_log();
        let mut ppb = Ppb::with_log(Arc::clone(&log));
        // SCB_VTOR at 0xE000_ED08. SCB_VTOR 位于 0xE000_ED08。
        assert!(ppb.write32(0xED08, 0x1000_0000));
        assert_eq!(log.lock().as_slice(), &[(0xE000_ED08, 0x1000_0000)]);
    }

    #[test]
    fn cpacr_writes_are_logged_for_replay() {
        let log = new_system_write_log();
        let mut ppb = Ppb::with_log(Arc::clone(&log));
        // SCB_CPACR at 0xE000_ED88. SCB_CPACR 位于 0xE000_ED88。
        assert!(ppb.write32(0xED88, 0x00F0_0000));
        assert_eq!(log.lock().as_slice(), &[(0xE000_ED88, 0x00F0_0000)]);
    }

    #[test]
    fn scr_and_stir_writes_are_logged_for_replay() {
        // 真实固件的 `STR SCR`/`STR STIR` 必须到达核心：SCR 控制 SLEEPONEXIT，
        // STIR 挂起软件中断。
        // Real firmware's `STR SCR`/`STR STIR` must reach the core: SCR controls
        // SLEEPONEXIT and STIR pends a software interrupt.
        let log = new_system_write_log();
        let mut ppb = Ppb::with_log(Arc::clone(&log));
        assert!(ppb.write32(SCB_SCR - map::PPB_BASE, 2));
        assert!(ppb.write32(SCB_STIR - map::PPB_BASE, 9));
        assert_eq!(
            log.lock().as_slice(),
            &[(SCB_SCR, 2), (SCB_STIR, 9)],
            "SCR and STIR replay"
        );
    }

    #[test]
    fn plain_ppb_writes_are_not_logged() {
        let log = new_system_write_log();
        let mut ppb = Ppb::with_log(Arc::clone(&log));
        // AIRCR at 0xE000_ED0C is plain storage in this stub.
        // 本桩中 0xE000_ED0C 的 AIRCR 只是普通存储。
        let offset = 0xE000_ED0C - map::PPB_BASE;
        assert!(ppb.write32(offset, 0x05FA_0000));
        assert!(log.lock().is_empty());
    }

    #[test]
    fn systick_registers_live_in_the_counter_not_the_replay_log() {
        // SysTick 跟随实时时钟，不能回放给核（否则核内计数器会重复计时）。
        // SysTick follows the live clock and must not be replayed, or the core
        // counter would tick twice.
        let log = new_system_write_log();
        let mut ppb = Ppb::with_log(Arc::clone(&log));
        assert!(ppb.write32(SYST_RVR, 999));
        assert!(ppb.write32(SYST_CVR, 999));
        assert!(ppb.write32(SYST_CSR, 7));
        assert_eq!(ppb.read32(SYST_RVR), Some(999));
        assert_eq!(ppb.read32(SYST_CVR), Some(999));
        assert_eq!(ppb.read32(SYST_CSR), Some(7));
        assert_eq!(ppb.read32(SYST_CALIB), Some(CALIB));
        assert!(log.lock().is_empty(), "SysTick is not replayed");
        assert!(!ppb.systick().lock().interrupt_pending());
    }

    #[test]
    fn icsr_writes_are_replayed_and_follow_set_clear_semantics() {
        // ICSR 的 PendSV/SysTick 挂起位必须到达核；PPB 影子按写 1 置位/清除。
        // ICSR PendSV/SysTick latches must reach the core; the PPB shadow keeps
        // write-1-to-set / write-1-to-clear semantics.
        let log = new_system_write_log();
        let mut ppb = Ppb::with_log(Arc::clone(&log));
        let offset = SCB_ICSR - map::PPB_BASE;
        assert!(ppb.write32(offset, ICSR_PENDSVSET | ICSR_PENDSTSET));
        assert_eq!(ppb.read32(offset), Some(ICSR_PENDSVSET | ICSR_PENDSTSET));
        assert!(ppb.write32(offset, ICSR_PENDSTCLR));
        assert_eq!(ppb.read32(offset), Some(ICSR_PENDSVSET));
        assert!(ppb.write32(offset, ICSR_PENDSVCLR));
        assert_eq!(ppb.read32(offset), Some(0));
        assert_eq!(
            log.lock().as_slice(),
            &[
                (SCB_ICSR, ICSR_PENDSVSET | ICSR_PENDSTSET),
                (SCB_ICSR, ICSR_PENDSTCLR),
                (SCB_ICSR, ICSR_PENDSVCLR),
            ]
        );
    }

    #[test]
    fn shpr2_and_shpr3_writes_are_replayed_for_system_priorities() {
        let log = new_system_write_log();
        let mut ppb = Ppb::with_log(Arc::clone(&log));
        // PendSV 优先级 1（SHPR3 第 2 字节 0x20）、SysTick 优先级 2（第 3 字节 0x40）。
        // PendSV priority 1 (SHPR3 byte 2 = 0x20), SysTick priority 2 (byte 3 = 0x40).
        assert!(ppb.write32(SCB_SHPR3 - map::PPB_BASE, 0x40_2000));
        assert!(ppb.write32(SCB_SHPR2 - map::PPB_BASE, 0xE000_0000));
        assert_eq!(
            log.lock().as_slice(),
            &[(SCB_SHPR3, 0x40_2000), (SCB_SHPR2, 0xE000_0000)]
        );
    }
}
