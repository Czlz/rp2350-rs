//! Dual-core Cortex-M33 scheduling scaffold
//! 双核 Cortex-M33 调度脚手架
//!
//! [`DualCoreM33`] owns core0 and core1 and advances each of them by one
//! instruction per [`DualCoreM33::step`]. Core1 stays halted until
//! [`DualCoreM33::release_core1`] initializes its vectors. PSM release polling,
//! SIO FIFO handshakes and CLI wiring are intentionally out of scope.
//! [`DualCoreM33`] 持有 core0 与 core1，每次 [`DualCoreM33::step`] 各推进一条
//! 指令。core1 在 [`DualCoreM33::release_core1`] 初始化其向量前保持停机。
//! PSM 释放轮询、SIO FIFO 握手与 CLI 接线刻意不在本模块范围内。

use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;
use rp2350_core::types::{CpuId, IrqNumber};
use rp2350_cpu::{CpuCore, CpuState};

use crate::CortexM33Core;

/// Core0/core1 pair with an explicit core1 release gate
/// 带显式 core1 释放门的 core0/core1 核心对
///
/// Core0 is supplied by the caller (already booted or still in reset); core1 is
/// created by [`Self::new`] in [`CpuState::Reset`] and never executes until
/// [`Self::release_core1`] succeeds.
/// core0 由调用方提供（可已启动或仍处于复位态）；core1 由 [`Self::new`] 以
/// [`CpuState::Reset`] 创建，在 [`Self::release_core1`] 成功前绝不执行。
pub struct DualCoreM33 {
    /// Primary core / 主核心
    core0: CortexM33Core,
    /// Secondary core / 次核心
    core1: CortexM33Core,
    /// Release gate: core1 only executes after a successful release
    /// 释放门：core1 仅在成功释放后才执行
    core1_released: bool,
}

impl DualCoreM33 {
    /// Create the pair from a caller-owned core0 and a fresh halted core1
    /// 用调用方提供的 core0 与全新的停机 core1 创建核心对
    #[must_use]
    pub fn new(core0: CortexM33Core) -> Self {
        Self {
            core0,
            core1: CortexM33Core::new(CpuId::Core1),
            core1_released: false,
        }
    }

    /// Borrow one core by ID / 按 ID 借用其中一个核心
    #[must_use]
    pub fn core(&self, id: CpuId) -> &CortexM33Core {
        match id {
            CpuId::Core0 => &self.core0,
            CpuId::Core1 => &self.core1,
        }
    }

    /// Mutably borrow one core by ID / 按 ID 可变借用其中一个核心
    #[must_use]
    pub fn core_mut(&mut self, id: CpuId) -> &mut CortexM33Core {
        match id {
            CpuId::Core0 => &mut self.core0,
            CpuId::Core1 => &mut self.core1,
        }
    }

    /// Report whether core1 has been released and can still make progress
    /// 报告 core1 是否已释放且仍可继续推进
    ///
    /// A released core 1 counts as live while it is `Running` **or**
    /// `Sleeping`: a sleeping core must still be stepped so its wake condition
    /// can be checked. Returns `false` before the first successful release and
    /// after core1 faults, halts or resets.
    /// 已释放的 core1 在 `Running` **或** `Sleeping` 时都算活跃：睡眠核仍须被推进，
    /// 才能检查其唤醒条件。首次成功释放前以及 core1 故障、停机或复位后返回 `false`。
    #[must_use]
    pub fn is_core1_running(&self) -> bool {
        self.core1_released && matches!(self.core1.state(), CpuState::Running | CpuState::Sleeping)
    }

    /// Initialize core1 registers and start its execution
    /// 初始化 core1 寄存器并开始执行
    ///
    /// The vector table base must be readable through `bus`; its first two
    /// words are probed so a bad `vtor` fails at release time instead of at the
    /// first exception. Vectors are applied through
    /// [`CortexM33Core::boot_at`], so `SP = sp`, `PC = pc & !1` and
    /// `VTOR = vtor`.
    ///
    /// Repeated calls **re-initialize** core1: the register file and NVIC are
    /// cleared, the new vectors are applied and execution restarts at `pc`.
    /// They are not a no-op and do not preserve pending interrupts.
    /// 向量表基址必须能通过 `bus` 读取；释放前会探测其前两个字，使错误的
    /// `vtor` 在释放时报错而不是等到第一次异常。向量经由
    /// [`CortexM33Core::boot_at`] 应用，因此 `SP = sp`、`PC = pc & !1`、
    /// `VTOR = vtor`。
    ///
    /// 重复调用会**重新初始化** core1：寄存器文件与 NVIC 被清零，应用新向量
    /// 并从 `pc` 重新开始执行；它不是空操作，也不保留挂起中断。
    ///
    /// # Errors
    /// Returns a bus error if the vector table cannot be read through `bus`.
    /// 若向量表无法通过 `bus` 读取，返回总线错误。
    pub fn release_core1(
        &mut self,
        bus: &mut dyn AddressBus,
        sp: u32,
        pc: u32,
        vtor: u32,
    ) -> Result<(), CoreError> {
        // 先探测向量表可读性，再改动 core1 状态，失败时保持其停机。
        // Probe vector-table readability before mutating core1 so a failed
        // release leaves the core halted.
        let mut cycles = 0u64;
        bus.read_u32(vtor, &mut cycles)?;
        bus.read_u32(vtor.wrapping_add(4), &mut cycles)?;
        self.core1.boot_at(sp, pc, vtor);
        self.core1_released = true;
        Ok(())
    }

    /// Advance core0 and, when running, core1 by one instruction each
    /// 各推进 core0 与（运行中的）core1 一条指令
    ///
    /// Core0 is always stepped first, then core1. The returned array is
    /// `[core0_state, core1_state]`; a halted core1 contributes its current
    /// state without executing. Errors propagate immediately: when core0 fails,
    /// core1 is not stepped in that call.
    /// 先推进 core0，再推进 core1。返回数组为 `[core0_state, core1_state]`；
    /// 停机的 core1 只返回当前状态、不执行指令。错误立即向上传播：core0 失败
    /// 时该次调用不会推进 core1。
    ///
    /// # Errors
    /// Propagates the first [`CoreError`] returned by either core.
    /// 返回两个核心中最先出现的 [`CoreError`]。
    pub fn step(&mut self, bus: &mut dyn AddressBus) -> Result<[CpuState; 2], CoreError> {
        let primary = self.core0.step(bus)?;
        let secondary = if self.is_core1_running() {
            self.core1.step(bus)?
        } else {
            self.core1.state()
        };
        Ok([primary, secondary])
    }

    /// Run up to `cycles` [`Self::step`] calls and return the actual count
    /// 最多执行 `cycles` 次 [`Self::step`]，返回实际执行次数
    ///
    /// One step counts as one cycle regardless of the instruction width. A
    /// sleeping core keeps the loop alive (its clock still advances and an event
    /// or interrupt can wake it), so only `Halted`/`Faulted`/`Reset` stop the
    /// loop; a stopped core1 alone never stops a running core0.
    /// 每次 step 计 1 个周期，与指令宽度无关。睡眠核仍保持循环（时钟继续推进，
    /// 事件或中断可将其唤醒），因此只有 `Halted`/`Faulted`/`Reset` 会结束循环；
    /// 仅 core1 停机不会阻止 core0 运行。
    ///
    /// # Errors
    /// Propagates the first [`CoreError`]; the partially executed count is
    /// discarded because the function returns the error instead.
    /// 返回最先出现的 [`CoreError`]；此时函数以错误返回，已执行的计数被丢弃。
    pub fn run_cycles(&mut self, bus: &mut dyn AddressBus, cycles: u64) -> Result<u64, CoreError> {
        let mut executed = 0u64;
        while executed < cycles {
            let states = self.step(bus)?;
            executed += 1;
            let runnable =
                |state: CpuState| matches!(state, CpuState::Running | CpuState::Sleeping);
            if !states.iter().copied().any(runnable) {
                break;
            }
        }
        Ok(executed)
    }

    /// Deliver an interrupt to exactly one core's NVIC
    /// 仅向指定核心的 NVIC 投递中断
    ///
    /// Each core owns an independent NVIC, so the other core observes no
    /// pending or servicing change.
    /// 每个核心拥有独立的 NVIC，因此另一核心不会观察到挂起或服务状态变化。
    pub fn signal_interrupt(&mut self, core: CpuId, irq: IrqNumber) {
        self.core_mut(core).signal_interrupt(irq);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::flash_generation_lock;
    use rp2350_core::flash::FlashRegion;
    use rp2350_core::memory::{RamRegion, SimpleBus};

    // `test_bus` creates and programs a FlashRegion (a process-global flash
    // generation bump), so tests that boot from it hold the generation lock to
    // avoid invalidating another test's fetch/decode cache.
    // `test_bus` 构造并编程 FlashRegion（递增进程级 Flash 世代号），因此从它启动
    // 的测试持有世代锁，避免使其它测试的取指/译码缓存失效。

    /// Code/vector RAM base / 代码与向量 RAM 基地址
    const CODE_BASE: u32 = 0x0000_0000;
    /// Shared SRAM base / 共享 SRAM 基地址
    const RAM_BASE: u32 = 0x2000_0000;
    /// Flash XIP base / Flash XIP 基地址
    const FLASH_BASE: u32 = 0x1000_0000;
    /// Cortex-M33 core0 spin loop `B .` / core0 自循环 `B .`
    const SPIN: [u16; 1] = [0xE7FE];
    /// core1 store program: `LDR r0,[pc,#4]; LDR r1,[pc,#8]; STR r0,[r1]; B .`
    /// core1 存储程序
    const STORE_PROGRAM: [u16; 4] = [0x4801, 0x4902, 0x6008, 0xE7FE];
    /// core0 copy program: `LDR r1,[pc,#8]; LDR r0,[r1]; STR r0,[r1,#4]; B .`
    /// core0 复制程序
    const COPY_PROGRAM: [u16; 4] = [0x4902, 0x6808, 0x6048, 0xE7FE];
    /// Counter loop: `LDR r1,[pc,#8]; LDR r0,[r1]; ADDS r0,#1; STR r0,[r1]; B`
    /// 计数器循环
    const COUNTER_LOOP: [u16; 5] = [0x4902, 0x6808, 0x3001, 0x6008, 0xE7FB];

    /// Build a bus with code RAM, shared SRAM and a flash vector table
    /// 构建带代码 RAM、共享 SRAM 与 Flash 向量表的总线
    fn test_bus(core0_pc: u32) -> SimpleBus {
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(RamRegion::new("code", CODE_BASE, 0x1000)))
            .unwrap();
        bus.register_region(Box::new(RamRegion::new("sram", RAM_BASE, 0x1000)))
            .unwrap();
        let mut flash = FlashRegion::new("flash", FLASH_BASE, 0x100);
        flash
            .load_bytes(FLASH_BASE, &(RAM_BASE + 0x800).to_le_bytes())
            .unwrap();
        flash
            .load_bytes(FLASH_BASE + 4, &core0_pc.to_le_bytes())
            .unwrap();
        bus.register_region(Box::new(flash)).unwrap();
        bus
    }

    /// Create a pair whose core0 has booted from the flash vector table
    /// 创建 core0 已从 Flash 向量表启动的核心对
    fn booted_pair(core0_pc: u32) -> (DualCoreM33, SimpleBus) {
        let mut bus = test_bus(core0_pc);
        let mut core0 = CortexM33Core::new(CpuId::Core0);
        core0.boot_from(&mut bus, FLASH_BASE).unwrap();
        (DualCoreM33::new(core0), bus)
    }

    /// Write a halfword sequence into code RAM
    /// 将半字序列写入代码 RAM
    fn write_halfwords(bus: &mut SimpleBus, base: u32, halfwords: &[u16]) {
        let mut cycles = 0u64;
        for (index, hw) in halfwords.iter().enumerate() {
            let addr = base + (index as u32) * 2;
            bus.write_u16(addr, *hw, &mut cycles).unwrap();
        }
    }

    /// Write a program that stores the value 5 to `target`
    /// 写入把数值 5 存到 `target` 的程序
    fn write_store_program(bus: &mut SimpleBus, base: u32, target: u32) {
        write_halfwords(bus, base, &STORE_PROGRAM);
        let mut cycles = 0u64;
        bus.write_u32(base + 0x08, 5, &mut cycles).unwrap();
        bus.write_u32(base + 0x0C, target, &mut cycles).unwrap();
    }

    /// Write a self-contained increment loop over `counter`
    /// 写入针对 `counter` 的自包含自增循环
    fn write_counter_program(bus: &mut SimpleBus, base: u32, counter: u32) {
        write_halfwords(bus, base, &COUNTER_LOOP);
        let mut cycles = 0u64;
        bus.write_u32(base + 0x0C, counter, &mut cycles).unwrap();
    }

    #[test]
    fn core1_does_not_execute_before_release() {
        let _generation = flash_generation_lock();
        let (mut pair, mut bus) = booted_pair(0x0000_0101);
        write_halfwords(&mut bus, 0x0000_0100, &SPIN);
        write_store_program(&mut bus, 0x0000_0040, RAM_BASE + 0x20);
        let mut cycles = 0u64;
        bus.write_u32(RAM_BASE + 0x20, 0xDEAD_BEEF, &mut cycles)
            .unwrap();

        let states = pair.step(&mut bus).unwrap();
        assert_eq!(states, [CpuState::Running, CpuState::Reset]);
        assert_eq!(pair.run_cycles(&mut bus, 49).unwrap(), 49);

        assert!(!pair.is_core1_running());
        assert_eq!(pair.core(CpuId::Core1).pc(), 0);
        assert_eq!(pair.core(CpuId::Core1).state(), CpuState::Reset);
        // The sentinel is untouched because core1 never fetched an instruction.
        // 哨兵未被改写，因为 core1 从未取指。
        assert_eq!(
            bus.read_u32(RAM_BASE + 0x20, &mut cycles).unwrap(),
            0xDEAD_BEEF
        );
    }

    #[test]
    fn released_core1_shares_sram_with_core0() {
        let _generation = flash_generation_lock();
        let (mut pair, mut bus) = booted_pair(0x0000_0101);
        write_halfwords(&mut bus, 0x0000_0100, &SPIN);
        write_store_program(&mut bus, 0x0000_0040, RAM_BASE);
        pair.release_core1(&mut bus, RAM_BASE + 0x400, 0x0000_0041, CODE_BASE)
            .unwrap();

        assert!(pair.is_core1_running());
        assert_eq!(pair.core(CpuId::Core1).read_register(13), RAM_BASE + 0x400);
        for _ in 0..4 {
            pair.step(&mut bus).unwrap();
        }
        let mut cycles = 0u64;
        assert_eq!(bus.read_u32(RAM_BASE, &mut cycles).unwrap(), 5);

        // core0 copies the shared word through the same bus.
        // core0 通过同一总线复制该共享字。
        write_halfwords(&mut bus, 0x0000_0200, &COPY_PROGRAM);
        bus.write_u32(0x0000_020C, RAM_BASE, &mut cycles).unwrap();
        pair.core_mut(CpuId::Core0).set_pc(0x0000_0200);
        for _ in 0..3 {
            pair.step(&mut bus).unwrap();
        }
        assert_eq!(bus.read_u32(RAM_BASE + 4, &mut cycles).unwrap(), 5);
    }

    #[test]
    fn cores_keep_independent_register_files() {
        let _generation = flash_generation_lock();
        let (mut pair, mut bus) = booted_pair(0x0000_0101);
        write_halfwords(&mut bus, 0x0000_0100, &SPIN);
        pair.core_mut(CpuId::Core0).write_register(0, 0xAAAA_AAAA);

        pair.release_core1(&mut bus, RAM_BASE + 0x400, 0x0000_0041, CODE_BASE)
            .unwrap();
        assert_eq!(pair.core(CpuId::Core0).read_register(0), 0xAAAA_AAAA);
        assert_eq!(pair.core(CpuId::Core1).read_register(0), 0);

        pair.core_mut(CpuId::Core1).write_register(0, 0x5555_5555);
        pair.core_mut(CpuId::Core1)
            .write_register(13, RAM_BASE + 0x100);
        assert_eq!(pair.core(CpuId::Core1).read_register(0), 0x5555_5555);
        assert_eq!(pair.core(CpuId::Core0).read_register(0), 0xAAAA_AAAA);
        // core0 kept its flash-table MSP while core1 has the released SP.
        // core0 保留 Flash 向量表的 MSP，core1 则使用释放时传入的 SP。
        assert_eq!(pair.core(CpuId::Core0).read_register(13), RAM_BASE + 0x800);
        assert_eq!(pair.core(CpuId::Core1).read_register(13), RAM_BASE + 0x100);
    }

    #[test]
    fn alternating_steps_advance_both_counters() {
        let _generation = flash_generation_lock();
        let (mut pair, mut bus) = booted_pair(0x0000_0101);
        write_counter_program(&mut bus, 0x0000_0100, RAM_BASE + 0x10);
        write_counter_program(&mut bus, 0x0000_0200, RAM_BASE + 0x14);
        pair.release_core1(&mut bus, RAM_BASE + 0x800, 0x0000_0201, CODE_BASE)
            .unwrap();

        for _ in 0..40 {
            let states = pair.step(&mut bus).unwrap();
            assert_eq!(states, [CpuState::Running, CpuState::Running]);
        }

        // The loop body is four instructions; the literal load runs once.
        // 循环体为 4 条指令；字面量加载只在进入循环前执行一次。
        let mut cycles = 0u64;
        assert_eq!(bus.read_u32(RAM_BASE + 0x10, &mut cycles).unwrap(), 10);
        assert_eq!(bus.read_u32(RAM_BASE + 0x14, &mut cycles).unwrap(), 10);
    }

    #[test]
    fn run_cycles_advances_core1_only_after_release() {
        let _generation = flash_generation_lock();
        let (mut pair, mut bus) = booted_pair(0x0000_0101);
        write_counter_program(&mut bus, 0x0000_0100, RAM_BASE + 0x10);
        write_counter_program(&mut bus, 0x0000_0200, RAM_BASE + 0x14);

        assert_eq!(pair.run_cycles(&mut bus, 25).unwrap(), 25);
        assert_eq!(pair.core(CpuId::Core1).pc(), 0);
        let mut cycles = 0u64;
        assert_eq!(bus.read_u32(RAM_BASE + 0x10, &mut cycles).unwrap(), 6);
        assert_eq!(bus.read_u32(RAM_BASE + 0x14, &mut cycles).unwrap(), 0);

        pair.release_core1(&mut bus, RAM_BASE + 0x800, 0x0000_0201, CODE_BASE)
            .unwrap();
        assert_eq!(pair.run_cycles(&mut bus, 25).unwrap(), 25);
        assert_eq!(bus.read_u32(RAM_BASE + 0x10, &mut cycles).unwrap(), 12);
        assert_eq!(bus.read_u32(RAM_BASE + 0x14, &mut cycles).unwrap(), 6);
    }

    #[test]
    fn run_cycles_respects_the_budget() {
        let _generation = flash_generation_lock();
        let (mut pair, mut bus) = booted_pair(0x0000_0101);
        write_counter_program(&mut bus, 0x0000_0100, RAM_BASE + 0x10);

        assert_eq!(pair.run_cycles(&mut bus, 0).unwrap(), 0);
        for budget in [1u64, 7, 33] {
            assert_eq!(pair.run_cycles(&mut bus, budget).unwrap(), budget);
        }
        assert!(pair.run_cycles(&mut bus, 5).unwrap() <= 5);
    }

    #[test]
    fn run_cycles_stops_when_both_cores_are_stopped() {
        // core0 was never booted (Reset) and core1 was never released.
        // core0 从未启动（Reset），core1 也从未释放。
        let mut pair = DualCoreM33::new(CortexM33Core::new(CpuId::Core0));
        let mut bus = SimpleBus::new();
        assert_eq!(pair.run_cycles(&mut bus, 100).unwrap(), 1);
    }

    #[test]
    fn interrupts_reach_only_the_requested_core() {
        let _generation = flash_generation_lock();
        let (mut pair, mut bus) = booted_pair(0x0000_0101);
        write_halfwords(&mut bus, 0x0000_0100, &SPIN);
        pair.release_core1(&mut bus, RAM_BASE + 0x800, 0x0000_0041, CODE_BASE)
            .unwrap();

        // Enable IRQ 16 on both NVICs, then signal only core1.
        // 在两个 NVIC 上都使能 IRQ 16，然后只向 core1 投递。
        pair.core_mut(CpuId::Core0)
            .system_write(0xE000_E100, 1 << 16);
        pair.core_mut(CpuId::Core1)
            .system_write(0xE000_E100, 1 << 16);
        pair.signal_interrupt(CpuId::Core1, 16);
        assert_eq!(pair.core(CpuId::Core1).servicing_irq(), Some(16));
        assert_eq!(pair.core(CpuId::Core0).servicing_irq(), None);

        pair.core_mut(CpuId::Core0)
            .system_write(0xE000_E100, 1 << 17);
        pair.signal_interrupt(CpuId::Core0, 17);
        assert_eq!(pair.core(CpuId::Core0).servicing_irq(), Some(17));
        assert_eq!(pair.core(CpuId::Core1).servicing_irq(), Some(16));
    }

    #[test]
    fn releasing_core1_again_reinitialises_it() {
        let _generation = flash_generation_lock();
        let (mut pair, mut bus) = booted_pair(0x0000_0101);
        write_halfwords(&mut bus, 0x0000_0100, &SPIN);
        write_store_program(&mut bus, 0x0000_0040, RAM_BASE);
        pair.release_core1(&mut bus, RAM_BASE + 0x800, 0x0000_0041, CODE_BASE)
            .unwrap();
        pair.core_mut(CpuId::Core1).write_register(0, 0xFFFF_FFFF);
        pair.core_mut(CpuId::Core1)
            .system_write(0xE000_E100, 1 << 16);
        pair.signal_interrupt(CpuId::Core1, 16);
        assert_eq!(pair.core(CpuId::Core1).servicing_irq(), Some(16));

        pair.release_core1(&mut bus, RAM_BASE + 0x400, 0x0000_0041, CODE_BASE)
            .unwrap();

        assert!(pair.is_core1_running());
        assert_eq!(pair.core(CpuId::Core1).read_register(0), 0);
        assert_eq!(pair.core(CpuId::Core1).read_register(13), RAM_BASE + 0x400);
        assert_eq!(pair.core(CpuId::Core1).pc(), 0x0000_0040);
        // Re-initialization cleared the NVIC pending bit.
        // 重新初始化清除了 NVIC 挂起位。
        assert_eq!(pair.core(CpuId::Core1).servicing_irq(), None);
    }

    #[test]
    fn release_core1_rejects_an_unmapped_vector_table() {
        let _generation = flash_generation_lock();
        let (mut pair, mut bus) = booted_pair(0x0000_0101);
        let error = pair
            .release_core1(&mut bus, RAM_BASE + 0x800, 0x0000_0041, 0x4000_0000)
            .unwrap_err();
        assert!(matches!(error, CoreError::BusError { .. }));
        assert!(!pair.is_core1_running());
        assert_eq!(pair.core(CpuId::Core1).pc(), 0);
        assert_eq!(pair.core(CpuId::Core1).state(), CpuState::Reset);
    }
}
