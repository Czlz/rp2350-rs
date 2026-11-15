//! Unit tests for the Cortex-M33 core integration and boot sequence
//! Cortex-M33 核心集成与启动序列的单元测试

mod ccr;
mod decode_cache;
mod faults;
mod fp_lazy;
mod fpu_align;
mod irq_gate;
mod scr;
mod sleep;
mod systick;

use super::*;
use crate::test_support::flash_generation_lock;

/// `SCB_SHCSR` absolute address / `SCB_SHCSR` 绝对地址
const SCB_SHCSR: u32 = 0xE000_ED24;

#[test]
fn new_core_is_in_reset() {
    let core = CortexM33Core::new(CpuId::Core0);
    assert_eq!(core.cpu_id(), CpuId::Core0);
    assert_eq!(core.state, CpuState::Reset);
}

#[test]
fn new_core_has_systick_disabled() {
    let core = CortexM33Core::new(CpuId::Core0);
    assert!(!core.systick.is_enabled());
}

#[test]
fn new_core_nvic_all_disabled() {
    let core = CortexM33Core::new(CpuId::Core0);
    assert_eq!(core.nvic.find_highest_priority(), None);
}

#[test]
fn cpacr_controls_fpu_access() {
    let mut core = CortexM33Core::new(CpuId::Core0);
    assert_eq!(core.system_read(SCB_CPACR), Some(0));
    assert!(!core.registers.fp_access_enabled());

    core.system_write(SCB_CPACR, 0x00F0_0000);
    assert_eq!(core.system_read(SCB_CPACR), Some(0x00F0_0000));
    assert!(core.registers.fp_access_enabled());
}

#[test]
fn cpacr_ignores_unmodeled_bits() {
    let mut core = CortexM33Core::new(CpuId::Core0);
    core.system_write(SCB_CPACR, 0xFFFF_FFFF);
    // CP0-CP7 and CP10-CP11 are modeled; CP8/CP9 and CP12/CP13 are not.
    // CP4 is bits 9:8, so the modeled low half is 0x0000_FFFF, not 0x00FF.
    // 建模 CP0-CP7 与 CP10-CP11；不建模 CP8/CP9 与 CP12/CP13。
    // CP4 位于 bits 9:8，因此低半部分掩码为 0x0000_FFFF 而非 0x00FF。
    assert_eq!(core.system_read(SCB_CPACR), Some(0x00F0_FFFF));
}

#[test]
fn cpacr_stores_gpioc_and_dcp_keys() {
    let mut core = CortexM33Core::new(CpuId::Core0);
    // MicroPython/pico-sdk runtime_init enables CP10, CP4 and CP0.
    // MicroPython/pico-sdk 的 runtime_init 使能 CP10、CP4 与 CP0。
    core.system_write(SCB_CPACR, 0x0030_0303);
    assert_eq!(core.system_read(SCB_CPACR), Some(0x0030_0303));
    assert!(core.registers.cp_access_enabled(0));
    assert!(core.registers.cp_access_enabled(4));
    assert!(core.registers.cp_access_enabled(10));
}

#[test]
fn reset_restores_post_boot_rom_fpu_state() {
    let mut core = CortexM33Core::new(CpuId::Core0);
    let mut bus = boot_bus(0x2000_1000, 0x0000_0101);
    core.boot(&mut bus).unwrap();
    core.system_write(SCB_CPACR, 0);
    core.reset();
    // The image entry point is modeled after the boot ROM, which leaves the
    // FPU enabled; FP data registers themselves are cleared.
    // 映像入口被建模为 boot ROM 之后的状态，FPU 已使能；浮点数据寄存器清零。
    assert_eq!(core.registers.cpacr, 0x00F0_0000);
    assert_eq!(core.registers.fpu.s, [0; 32]);
}

#[test]
fn signal_interrupt_sets_pending() {
    let mut core = CortexM33Core::new(CpuId::Core0);
    core.signal_interrupt(42);
    assert!(core.nvic.is_pending(42));
}

#[test]
fn servicing_irq_returns_none_when_none_pending() {
    let core = CortexM33Core::new(CpuId::Core0);
    assert_eq!(core.servicing_irq(), None);
}

#[test]
fn servicing_irq_finds_highest_priority() {
    let mut core = CortexM33Core::new(CpuId::Core0);
    core.nvic.write_iser(0, 1 << 16); // Enable IRQ 16
    core.nvic.write_iser(1, 1 << 0); // Enable IRQ 32 (bit 0 of ISER[1])
    core.nvic.set_pending(16);
    core.nvic.set_pending(32);
    // Both same priority (0), lowest IRQ number wins
    assert_eq!(core.servicing_irq(), Some(16));
}

/// Build a bus with a boot vector table at address 0x0000_0000.
/// 在地址 0x0000_0000 构建带启动向量表的总线。
fn boot_bus(msp: u32, reset: u32) -> rp2350_core::memory::SimpleBus {
    let mut bus = rp2350_core::memory::SimpleBus::new();
    bus.register_region(Box::new(rp2350_core::memory::RamRegion::new(
        "vectors",
        0x0000_0000,
        0x100,
    )))
    .unwrap();
    let mut cycles = 0;
    bus.write_u32(0x0000_0000, msp, &mut cycles).unwrap();
    bus.write_u32(0x0000_0004, reset, &mut cycles).unwrap();
    bus
}

#[test]
fn boot_loads_sp_and_pc_from_vectors() {
    let mut core = CortexM33Core::new(CpuId::Core0);
    let mut bus = boot_bus(0x2000_1000, 0x0000_0101);
    core.boot(&mut bus).unwrap();

    assert_eq!(core.registers.msp, 0x2000_1000);
    assert_eq!(core.registers.sp, 0x2000_1000);
    // Thumb bit must be stripped from the reset vector.
    // 复位向量中的 Thumb 位必须被清除。
    assert_eq!(core.registers.pc, 0x0000_0100);
    assert_eq!(core.registers.xpsr & 0x0100_0000, 0x0100_0000);
    assert_eq!(core.state, CpuState::Running);
}

#[test]
fn init_boots_and_enters_running() {
    let mut core = CortexM33Core::new(CpuId::Core0);
    let mut bus = boot_bus(0x2000_2000, 0x0000_0201);
    core.init(&mut bus).unwrap();

    assert_eq!(core.state, CpuState::Running);
    assert_eq!(core.registers.sp, 0x2000_2000);
    assert_eq!(core.registers.pc, 0x0000_0200);
}

#[test]
fn reset_after_boot_reapplies_vectors() {
    let mut core = CortexM33Core::new(CpuId::Core0);
    let mut bus = boot_bus(0x2000_1000, 0x0000_0101);
    core.boot(&mut bus).unwrap();

    core.registers.r[0] = 0xDEAD_BEEF;
    core.registers.pc = 0xFFFF_FFFF;
    core.nvic.write_iser(0, 1);
    core.reset();

    assert_eq!(core.registers.r[0], 0);
    assert_eq!(core.registers.sp, 0x2000_1000);
    assert_eq!(core.registers.pc, 0x0000_0100);
    assert_eq!(core.state, CpuState::Running);
}

#[test]
fn reset_without_boot_stays_in_reset() {
    let mut core = CortexM33Core::new(CpuId::Core0);
    core.reset();
    assert_eq!(core.state, CpuState::Reset);
    assert_eq!(core.registers.pc, 0);
}

#[test]
fn boot_without_vector_table_fails() {
    let mut core = CortexM33Core::new(CpuId::Core0);
    let mut bus = rp2350_core::memory::SimpleBus::new();
    assert!(core.boot(&mut bus).is_err());
    assert_eq!(core.state, CpuState::Reset);
}

/// Warning messages captured from the `log` facade for assertions.
/// 供断言使用的 `log` 告警消息。
fn captured_warnings() -> &'static std::sync::Mutex<Vec<String>> {
    static WARNINGS: std::sync::OnceLock<std::sync::Mutex<Vec<String>>> =
        std::sync::OnceLock::new();
    WARNINGS.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Test logger that records warning-level messages.
/// 记录告警级别消息的测试日志器。
struct CaptureLogger;

impl log::Log for CaptureLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            captured_warnings()
                .lock()
                .unwrap()
                .push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

/// Install the capture logger once per test process.
/// 每个测试进程只安装一次捕获日志器。
fn install_capture_logger() {
    static LOGGER: CaptureLogger = CaptureLogger;
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let _ = log::set_logger(&LOGGER);
        log::set_max_level(log::LevelFilter::Warn);
    });
}

#[test]
fn undefined_instruction_default_enters_hardfault_and_warns_once() {
    install_capture_logger();
    captured_warnings().lock().unwrap().clear();

    // 两条未定义指令 0xB800 位于 0x80/0x82；HardFault 处理程序位于 0x90。
    // Two copies of the undefined opcode 0xB800 at 0x80/0x82; the HardFault
    // handler lives at 0x90.
    let mut bus = fpu_test_bus(0x0000_0081);
    let mut cycles = 0u64;
    bus.write_u32(0x0000_000C, 0x0000_0091, &mut cycles)
        .unwrap();
    bus.write_u16(0x0000_0080, 0xB800, &mut cycles).unwrap();
    bus.write_u16(0x0000_0082, 0xB800, &mut cycles).unwrap();
    bus.write_u16(0x0000_0090, 0x4770, &mut cycles).unwrap(); // BX LR

    let mut core = CortexM33Core::new(CpuId::Core0);
    core.boot(&mut bus).unwrap();

    // 默认（非严格）行为：未定义指令升级为 HardFault（异常 3）。
    // Default (non-strict) behaviour: the undefined instruction escalates to
    // HardFault (exception 3).
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.registers.pc, 0x0000_0090);
    assert_eq!(core.registers.xpsr & 0x1FF, 3);
    assert_ne!(core.cfsr & crate::faults::cfsr::UNDEFINSTR, 0);
    assert_ne!(core.hfsr() & crate::faults::hfsr::FORCED, 0);
    // 压栈返回地址为故障指令本身。 / The stacked PC is the faulting instruction.
    assert_eq!(bus.read_u32(0x2000_0FF8, &mut cycles).unwrap(), 0x0000_0080);

    // 处理程序 BX LR 返回故障指令；第二次故障不得重复告警。
    // BX LR returns to the faulting instruction; the second fault must not log
    // the same opcode again.
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.registers.pc, 0x0000_0080);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.registers.pc, 0x0000_0090);

    let warnings = captured_warnings().lock().unwrap().clone();
    // 其它并行测试也会写日志；按本测试的 PC 过滤，断言去重后只剩一条。
    // Other tests running in parallel also log; filter by this test's PC so the
    // deduplicated warning is exactly one entry.
    let undefined: Vec<&String> = warnings
        .iter()
        .filter(|message| {
            message.contains("undefined instruction") && message.contains("0x00000080")
        })
        .collect();
    assert_eq!(undefined.len(), 1, "warnings: {warnings:?}");
    assert!(undefined[0].contains("0x0000B800"));
    assert!(undefined[0].contains("2 bytes"));
}

#[test]
fn undefined_instruction_strict_mode_halts_with_context() {
    let mut bus = boot_bus(0x2000_1000, 0x0000_0081);
    let mut cycles = 0;
    bus.write_u16(0x0000_0080, 0xB800, &mut cycles).unwrap();

    let mut core = CortexM33Core::new(CpuId::Core0);
    core.set_strict_decode(true);
    core.boot(&mut bus).unwrap();

    let error = core.step(&mut bus).unwrap_err();
    match error {
        CoreError::UndefinedInstruction { pc, opcode, width } => {
            assert_eq!(pc, 0x0000_0080);
            assert_eq!(opcode, 0x0000_B800);
            assert_eq!(width, 2);
        }
        other => panic!("unexpected error: {other:?}"),
    }
    assert_eq!(core.state, CpuState::Halted);
}

/// Build a bus with a vector table, a code area, and a stack region
/// 构建带向量表、代码区与栈区域的总线
fn fpu_test_bus(reset: u32) -> rp2350_core::memory::SimpleBus {
    let mut bus = rp2350_core::memory::SimpleBus::new();
    bus.register_region(Box::new(rp2350_core::memory::RamRegion::new(
        "code",
        0x0000_0000,
        0x200,
    )))
    .unwrap();
    bus.register_region(Box::new(rp2350_core::memory::RamRegion::new(
        "stack",
        0x2000_0000,
        0x1000,
    )))
    .unwrap();
    let mut cycles = 0;
    bus.write_u32(0x0000_0000, 0x2000_1000, &mut cycles)
        .unwrap();
    bus.write_u32(0x0000_0004, reset, &mut cycles).unwrap();
    bus
}

#[test]
fn fpu_access_without_cpacr_enters_usage_fault() {
    // VLDR S0, [R0, #4] stored as first/second halfword.
    // 按第一/第二半字顺序存放 VLDR S0, [R0, #4]。
    let mut bus = fpu_test_bus(0x0000_0101);
    let mut cycles = 0;
    bus.write_u32(0x0000_0018, 0x0000_0301, &mut cycles)
        .unwrap();
    bus.write_u16(0x0000_0100, 0xED90, &mut cycles).unwrap();
    bus.write_u16(0x0000_0102, 0x0A01, &mut cycles).unwrap();

    let mut core = CortexM33Core::new(CpuId::Core0);
    core.boot(&mut bus).unwrap();
    // 使能 UsageFault，使 NOCP 进入异常 6；未使能时按 ARMv8-M 升级为 HardFault。
    // Enable UsageFault so NOCP enters exception 6; without the enable ARMv8-M
    // escalates it to HardFault.
    core.system_write(SCB_SHCSR, crate::faults::shcsr::USGFAULTENA);
    // Deny FPU access explicitly, as firmware would when disabling the unit.
    // 像固件关闭浮点单元那样显式拒绝 FPU 访问。
    core.system_write(SCB_CPACR, 0);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.registers.pc, 0x0000_0300);
    assert_eq!(core.registers.lr, 0xFFFF_FFF9);
    assert_ne!(core.cfsr & exceptions::CFSR_UFSR_NOCP, 0);
    assert!(
        core.active_exceptions
            .contains(&exceptions::EXC_USAGE_FAULT)
    );
    // Stacked PC points at the faulting instruction, not the next one.
    // 压栈 PC 指向引发故障的指令，而不是下一条。
    assert_eq!(bus.read_u32(0x2000_0FF8, &mut cycles).unwrap(), 0x0000_0100);
    assert_eq!(core.registers.fpu.fpexc & fpu::fpexc::EN, 0);
}

#[test]
fn fpu_usage_fault_from_psp_thread_stacks_on_psp() {
    let mut bus = fpu_test_bus(0x0000_0101);
    let mut cycles = 0;
    bus.write_u32(0x0000_0018, 0x0000_0301, &mut cycles)
        .unwrap();
    bus.write_u16(0x0000_0100, 0xED90, &mut cycles).unwrap();
    bus.write_u16(0x0000_0102, 0x0A01, &mut cycles).unwrap();

    let mut core = CortexM33Core::new(CpuId::Core0);
    core.boot(&mut bus).unwrap();
    core.system_write(SCB_SHCSR, crate::faults::shcsr::USGFAULTENA);
    core.system_write(SCB_CPACR, 0);
    core.registers.control = 0x2;
    core.registers.psp = 0x2000_0800;
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);

    assert_eq!(core.registers.lr, 0xFFFF_FFFD);
    assert_eq!(core.registers.psp, 0x2000_07E0);
    assert_eq!(bus.read_u32(0x2000_07F8, &mut cycles).unwrap(), 0x0000_0100);
}

#[test]
fn fpu_instruction_executes_when_cpacr_allows_it() {
    let mut bus = fpu_test_bus(0x0000_0101);
    let mut cycles = 0;
    bus.write_u16(0x0000_0100, 0xED90, &mut cycles).unwrap();
    bus.write_u16(0x0000_0102, 0x0A01, &mut cycles).unwrap();
    bus.write_u32(0x2000_0004, 0x4142_4300, &mut cycles)
        .unwrap();

    let mut core = CortexM33Core::new(CpuId::Core0);
    core.boot(&mut bus).unwrap();
    assert_eq!(core.registers.cpacr, 0x00F0_0000);
    core.write_register(0, 0x2000_0000);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);

    assert_eq!(core.registers.pc, 0x0000_0104);
    assert_eq!(core.registers.fpu.s[0], 0x4142_4300);
    assert_eq!(core.cfsr, 0);
    assert_eq!(core.active_exceptions.len(), 0);
}

/// Program offset used by the flash fetch-cache tests / Flash 取指缓存测试使用的程序偏移
const CACHE_PROGRAM_OFFSET: u32 = 0x100;

/// Build a bus with a flash XIP image, SRAM and a reset vector into the image
/// 构建带 Flash XIP 映像、SRAM 与指向映像的复位向量的总线
fn flash_bus(program: &[u16]) -> (rp2350_core::memory::SimpleBus, rp2350_core::FlashHandle) {
    let mut bus = rp2350_core::memory::SimpleBus::new();
    let flash = rp2350_core::FlashRegion::new("flash", rp2350_core::FLASH_XIP_BASE, 0x1000);
    let handle = flash.handle();
    bus.register_region(Box::new(rp2350_core::memory::RamRegion::new(
        "sram",
        0x2000_0000,
        0x1000,
    )))
    .unwrap();
    bus.register_region(Box::new(flash)).unwrap();
    let mut image = Vec::with_capacity(8);
    image.extend_from_slice(&0x2000_1000u32.to_le_bytes());
    image
        .extend_from_slice(&(rp2350_core::FLASH_XIP_BASE + CACHE_PROGRAM_OFFSET + 1).to_le_bytes());
    handle.write(rp2350_core::FLASH_XIP_BASE, &image).unwrap();
    let mut program_bytes = Vec::with_capacity(program.len() * 2);
    for half in program {
        program_bytes.extend_from_slice(&half.to_le_bytes());
    }
    handle
        .write(
            rp2350_core::FLASH_XIP_BASE + CACHE_PROGRAM_OFFSET,
            &program_bytes,
        )
        .unwrap();
    (bus, handle)
}

#[test]
fn flash_fetch_cache_observes_host_rewrites() {
    // `flash_bus` creates and programs a FlashRegion, which bumps the
    // process-global flash generation; hold the lock so this test and any
    // concurrent cache test cannot invalidate each other.
    // `flash_bus` 构造并编程 FlashRegion，会递增进程级 Flash 世代号；持锁可避免
    // 本测试与并发的缓存测试互相使对方失效。
    let _generation = flash_generation_lock();
    // MOVS r0, #1 / MOVS r0, #2
    // MOVS r0, #1 / MOVS r0, #2
    let (mut bus, handle) = flash_bus(&[0x2001, 0x2002]);
    let mut core = CortexM33Core::new(CpuId::Core0);
    core.set_flash_fetch_cache(true);
    core.boot_from(&mut bus, rp2350_core::FLASH_XIP_BASE)
        .unwrap();
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.read_register(0), 1);

    // 宿主改写 Flash 后，下一次取指必须看到新指令（缓存整表失效）。
    // After a host rewrite the next fetch must observe the new instruction.
    handle
        .write(
            rp2350_core::FLASH_XIP_BASE + CACHE_PROGRAM_OFFSET,
            &0x2007u16.to_le_bytes(),
        )
        .unwrap();
    core.registers.pc = rp2350_core::FLASH_XIP_BASE + CACHE_PROGRAM_OFFSET;
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.read_register(0), 7);
}

#[test]
fn flash_fetch_cache_books_the_same_cycles_and_state() {
    let _generation = flash_generation_lock();
    let program = [0x2001, 0x2002, 0x2003, 0x2004, 0x2005, 0x2006];
    let (mut cached_bus, _cached_handle) = flash_bus(&program);
    let (mut plain_bus, _plain_handle) = flash_bus(&program);

    let mut cached = CortexM33Core::new(CpuId::Core0);
    cached.set_flash_fetch_cache(true);
    cached
        .boot_from(&mut cached_bus, rp2350_core::FLASH_XIP_BASE)
        .unwrap();

    let mut plain = CortexM33Core::new(CpuId::Core0);
    plain
        .boot_from(&mut plain_bus, rp2350_core::FLASH_XIP_BASE)
        .unwrap();

    for _ in 0..12 {
        assert_eq!(
            cached.step(&mut cached_bus).unwrap(),
            plain.step(&mut plain_bus).unwrap()
        );
    }
    assert_eq!(cached.pc(), plain.pc());
    assert_eq!(cached.read_register(0), plain.read_register(0));
    // 周期记账必须逐周期一致，否则 total_cycles 诊断会漂移。
    // Cycle accounting must match exactly or the `total_cycles` diagnostic drifts.
    assert_eq!(cached.total_cycles(), plain.total_cycles());
}
