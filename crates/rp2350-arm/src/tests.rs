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
    assert_eq!(core.system_read(SCB_CPACR), Some(0x00F0_0000));
}

#[test]
fn cpacr_is_cleared_by_reset() {
    let mut core = CortexM33Core::new(CpuId::Core0);
    let mut bus = boot_bus(0x2000_1000, 0x0000_0101);
    core.boot(&mut bus).unwrap();
    core.system_write(SCB_CPACR, 0x00F0_0000);
    core.reset();
    assert_eq!(core.registers.cpacr, 0);
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
