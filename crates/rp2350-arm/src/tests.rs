//! Unit tests for the Cortex-M33 core integration and boot sequence
//! Cortex-M33 核心集成与启动序列的单元测试

use super::*;

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
fn undefined_instruction_default_warns_once_and_continues() {
    install_capture_logger();
    captured_warnings().lock().unwrap().clear();

    // Two copies of the undefined opcode 0xB800 at 0x80 and 0x82.
    // 在 0x80 与 0x82 处放两条相同的未定义指令 0xB800。
    let mut bus = boot_bus(0x2000_1000, 0x0000_0081);
    let mut cycles = 0;
    bus.write_u16(0x0000_0080, 0xB800, &mut cycles).unwrap();
    bus.write_u16(0x0000_0082, 0xB800, &mut cycles).unwrap();

    let mut core = CortexM33Core::new(CpuId::Core0);
    core.boot(&mut bus).unwrap();

    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.registers.pc, 0x0000_0082);
    assert_eq!(core.step(&mut bus).unwrap(), CpuState::Running);
    assert_eq!(core.registers.pc, 0x0000_0084);

    let warnings = captured_warnings().lock().unwrap().clone();
    let undefined: Vec<&String> = warnings
        .iter()
        .filter(|message| message.contains("undefined instruction"))
        .collect();
    assert_eq!(undefined.len(), 1, "warnings: {warnings:?}");
    assert!(undefined[0].contains("0x00000080"));
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
