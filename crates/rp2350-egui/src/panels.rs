//! GUI panels — register viewer
//! GUI 面板 — 寄存器查看器
//!
//! The register panel renders a [`CpuCore`] through its trait interface only,
//! so it works with any core implementation (Cortex-M33, RISC-V, mocks).
//! 寄存器面板仅通过 trait 接口渲染 [`CpuCore`]，因此适用于任意核心实现
//! （Cortex-M33、RISC-V、测试替身）。

use egui::{Color32, RichText};
use rp2350_core::types::IrqNumber;
use rp2350_cpu::{CpuCore, CpuState};

/// Number of architectural registers shown by the panel (R0-R15)
/// 面板显示的架构寄存器数量（R0-R15）
const REGISTER_COUNT: usize = 16;

/// Colour used for a set status flag / 置位状态标志使用的颜色
const FLAG_SET_COLOR: Color32 = Color32::from_rgb(0x4C, 0xAF, 0x50);

/// Snapshot of the architectural state needed by the register panel
/// 寄存器面板所需的架构状态快照
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisterSnapshot {
    /// General-purpose registers R0-R15 (R13=SP, R14=LR, R15=PC)
    /// 通用寄存器 R0-R15（R13=SP、R14=LR、R15=PC）
    pub registers: [u32; REGISTER_COUNT],
    /// Program status register (XPSR) / 程序状态寄存器（XPSR）
    pub xpsr: u32,
    /// Current execution state / 当前执行状态
    pub state: CpuState,
    /// Cumulative cycles executed since reset / 自复位以来累计执行的周期数
    pub total_cycles: u64,
    /// IRQ currently being serviced, if any / 当前正在服务的中断号（如果有）
    pub servicing_irq: Option<IrqNumber>,
}

impl RegisterSnapshot {
    /// Capture the state of `core` through its trait interface
    /// 通过 trait 接口捕获 `core` 的状态
    #[must_use]
    pub fn capture(core: &dyn CpuCore) -> Self {
        let mut registers = [0u32; REGISTER_COUNT];
        for (index, slot) in registers.iter_mut().enumerate() {
            *slot = core.read_register(u8::try_from(index).unwrap_or_default());
        }

        Self {
            registers,
            xpsr: core.xpsr(),
            state: core.state(),
            total_cycles: core.total_cycles(),
            servicing_irq: core.servicing_irq(),
        }
    }
}

/// Decoded XPSR flag bits displayed by the register panel
/// 寄存器面板显示解码后的 XPSR 标志位
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct XpsrFlags {
    /// Raw XPSR value the flags are decoded from / 标志位解码所依据的原始 XPSR 值
    bits: u32,
    /// Exception number from `IPSR[8:0]` / 来自 `IPSR[8:0]` 的异常号
    pub exception_number: u32,
}

impl XpsrFlags {
    /// Decode the interesting XPSR bits / 解码 XPSR 中关注的位
    #[must_use]
    pub fn from_xpsr(xpsr: u32) -> Self {
        Self {
            bits: xpsr,
            exception_number: xpsr & 0x1FF,
        }
    }

    /// Negative (N) flag, APSR bit 31 / 负数（N）标志，APSR 第 31 位
    #[must_use]
    pub fn negative(self) -> bool {
        self.bits & (1 << 31) != 0
    }

    /// Zero (Z) flag, APSR bit 30 / 零（Z）标志，APSR 第 30 位
    #[must_use]
    pub fn zero(self) -> bool {
        self.bits & (1 << 30) != 0
    }

    /// Carry (C) flag, APSR bit 29 / 进位（C）标志，APSR 第 29 位
    #[must_use]
    pub fn carry(self) -> bool {
        self.bits & (1 << 29) != 0
    }

    /// Overflow (V) flag, APSR bit 28 / 溢出（V）标志，APSR 第 28 位
    #[must_use]
    pub fn overflow(self) -> bool {
        self.bits & (1 << 28) != 0
    }

    /// Saturating cumulative (Q) flag, APSR bit 27
    /// 饱和累积（Q）标志，APSR 第 27 位
    #[must_use]
    pub fn saturation(self) -> bool {
        self.bits & (1 << 27) != 0
    }

    /// Thumb state bit (EPSR.T), bit 24 / Thumb 状态位（EPSR.T），第 24 位
    #[must_use]
    pub fn thumb(self) -> bool {
        self.bits & (1 << 24) != 0
    }
}

/// Register viewer panel for any [`CpuCore`] implementation
/// 适用于任意 [`CpuCore`] 实现的寄存器查看器面板
pub struct RegisterPanel<'a> {
    /// Core inspected through its trait interface / 通过 trait 接口检视的核心
    core: &'a dyn CpuCore,
}

impl<'a> RegisterPanel<'a> {
    /// Create a panel bound to `core` / 创建绑定到 `core` 的面板
    #[must_use]
    pub fn new(core: &'a dyn CpuCore) -> Self {
        Self { core }
    }

    /// Render the panel / 渲染面板
    pub fn show(&self, ui: &mut egui::Ui) {
        let snapshot = RegisterSnapshot::capture(self.core);

        ui.heading(RichText::new("Registers / 寄存器").strong());
        ui.add_space(4.0);
        Self::show_registers(ui, &snapshot);

        ui.separator();
        Self::show_xpsr(ui, &snapshot);

        ui.separator();
        Self::show_execution_state(ui, &snapshot);
    }

    /// Render the R0-R15 grid, highlighting SP/LR/PC aliases
    /// 渲染 R0-R15 表格，突出 SP/LR/PC 别名
    fn show_registers(ui: &mut egui::Ui, snapshot: &RegisterSnapshot) {
        egui::Grid::new("register_viewer_grid")
            .num_columns(4)
            .spacing([24.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                for pair in 0..(REGISTER_COUNT / 2) {
                    let low = pair * 2;
                    register_cell(ui, low, snapshot.registers[low]);
                    register_cell(ui, low + 1, snapshot.registers[low + 1]);
                    ui.end_row();
                }
            });
    }

    /// Render XPSR value, flags, and exception number
    /// 渲染 XPSR 值、标志位和异常号
    fn show_xpsr(ui: &mut egui::Ui, snapshot: &RegisterSnapshot) {
        ui.heading(RichText::new("XPSR / 状态寄存器").strong());
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label("XPSR");
            ui.monospace(format!("0x{:08X}", snapshot.xpsr));
        });

        let flags = XpsrFlags::from_xpsr(snapshot.xpsr);
        ui.horizontal_wrapped(|ui| {
            flag_chip(ui, "N", flags.negative());
            flag_chip(ui, "Z", flags.zero());
            flag_chip(ui, "C", flags.carry());
            flag_chip(ui, "V", flags.overflow());
            flag_chip(ui, "Q", flags.saturation());
            flag_chip(ui, "T", flags.thumb());
        });

        ui.horizontal(|ui| {
            ui.label("IPSR / exception");
            ui.monospace(flags.exception_number.to_string());
        });
    }

    /// Render execution state, cumulative cycles, and serviced IRQ
    /// 渲染执行状态、累计周期数和正在服务的中断
    fn show_execution_state(ui: &mut egui::Ui, snapshot: &RegisterSnapshot) {
        ui.horizontal(|ui| {
            ui.label("State / 状态");
            ui.label(
                RichText::new(format!("{:?}", snapshot.state))
                    .monospace()
                    .color(state_color(snapshot.state)),
            );
        });

        ui.horizontal(|ui| {
            ui.label("Cycles / 累计周期");
            ui.monospace(snapshot.total_cycles.to_string());
        });

        ui.horizontal(|ui| {
            ui.label("Servicing IRQ / 服务中中断");
            match snapshot.servicing_irq {
                Some(irq) => ui.monospace(irq.to_string()),
                None => ui.monospace("none"),
            };
        });
    }
}

/// Render a single register label and value / 渲染单个寄存器标签和值
fn register_cell(ui: &mut egui::Ui, index: usize, value: u32) {
    ui.label(register_label(index));
    ui.monospace(format!("0x{value:08X}"));
}

/// Return the display label for a register index / 返回寄存器索引对应的显示标签
fn register_label(index: usize) -> String {
    match index {
        13 => "R13/SP".to_string(),
        14 => "R14/LR".to_string(),
        15 => "R15/PC".to_string(),
        _ => format!("R{index}"),
    }
}

/// Render a single XPSR flag chip / 渲染单个 XPSR 标志位
fn flag_chip(ui: &mut egui::Ui, name: &str, set: bool) {
    let bit = u8::from(set);
    let text = RichText::new(format!("{name}={bit}")).monospace();
    let text = if set {
        text.color(FLAG_SET_COLOR).strong()
    } else {
        text.weak()
    };
    ui.label(text);
}

/// Pick a display colour for the execution state / 为执行状态选择显示颜色
fn state_color(state: CpuState) -> Color32 {
    match state {
        CpuState::Running => Color32::from_rgb(0x4C, 0xAF, 0x50),
        CpuState::Sleeping => Color32::from_rgb(0xFF, 0xC1, 0x07),
        CpuState::Breakpoint => Color32::from_rgb(0x4F, 0xC1, 0xF5),
        CpuState::Faulted | CpuState::Halted => Color32::from_rgb(0xE5, 0x73, 0x73),
        CpuState::Reset => Color32::GRAY,
    }
}

#[cfg(test)]
mod tests {
    use rp2350_core::bus::AddressBus;
    use rp2350_core::error::CoreError;
    use rp2350_core::types::{CpuId, CycleCount};
    use rp2350_cpu::CpuArch;

    use super::*;

    /// Minimal test double implementing the trait surface used by the panel
    /// 供面板使用的最小测试替身
    struct MockCore {
        registers: [u32; REGISTER_COUNT],
        xpsr: u32,
        state: CpuState,
        cycles: CycleCount,
        irq: Option<IrqNumber>,
    }

    impl MockCore {
        fn new() -> Self {
            Self {
                registers: [0; REGISTER_COUNT],
                xpsr: 0,
                state: CpuState::Running,
                cycles: 0,
                irq: None,
            }
        }
    }

    impl CpuCore for MockCore {
        fn cpu_id(&self) -> CpuId {
            CpuId::Core0
        }

        fn arch(&self) -> CpuArch {
            CpuArch::ArmCortexM33
        }

        fn init(&mut self, _bus: &mut dyn AddressBus) -> Result<(), CoreError> {
            Ok(())
        }

        fn run_cycles(
            &mut self,
            _bus: &mut dyn AddressBus,
            _cycles: CycleCount,
        ) -> (CycleCount, CpuState) {
            (0, self.state)
        }

        fn step(&mut self, _bus: &mut dyn AddressBus) -> Result<CpuState, CoreError> {
            Ok(self.state)
        }

        fn signal_interrupt(&mut self, irq: IrqNumber) {
            self.irq = Some(irq);
        }

        fn servicing_irq(&self) -> Option<IrqNumber> {
            self.irq
        }

        fn pc(&self) -> u32 {
            self.registers[15]
        }

        fn read_register(&self, index: u8) -> u32 {
            self.registers[usize::from(index)]
        }

        fn write_register(&mut self, index: u8, value: u32) {
            self.registers[usize::from(index)] = value;
        }

        fn reset(&mut self) {}

        fn description(&self) -> String {
            String::from("mock core")
        }

        fn state(&self) -> CpuState {
            self.state
        }

        fn xpsr(&self) -> u32 {
            self.xpsr
        }

        fn total_cycles(&self) -> CycleCount {
            self.cycles
        }
    }

    #[test]
    fn snapshot_matches_core_reads() {
        let mut core = MockCore::new();
        core.registers = [
            0x1000_0000,
            0x1000_0001,
            0x1000_0002,
            0x1000_0003,
            0x1000_0004,
            0x1000_0005,
            0x1000_0006,
            0x1000_0007,
            0x1000_0008,
            0x1000_0009,
            0x1000_000A,
            0x1000_000B,
            0x1000_000C,
            0x2004_2000,
            0x0000_0125,
            0x0000_0200,
        ];
        core.xpsr = 0xF800_0107;
        core.state = CpuState::Breakpoint;
        core.cycles = 42;
        core.irq = Some(5);

        let snapshot = RegisterSnapshot::capture(&core);

        assert_eq!(snapshot.registers, core.registers);
        assert_eq!(snapshot.registers[13], core.read_register(13));
        assert_eq!(snapshot.registers[14], core.read_register(14));
        assert_eq!(snapshot.registers[15], core.pc());
        assert_eq!(snapshot.xpsr, core.xpsr());
        assert_eq!(snapshot.state, core.state());
        assert_eq!(snapshot.total_cycles, core.total_cycles());
        assert_eq!(snapshot.servicing_irq, core.servicing_irq());
    }

    #[test]
    fn xpsr_flags_decode() {
        // N, Z, C, V set; Q clear; T set; exception number 0x108.
        // N、Z、C、V 置位；Q 清零；T 置位；异常号 0x108。
        let flags = XpsrFlags::from_xpsr(0xF100_0108);

        assert!(flags.negative());
        assert!(flags.zero());
        assert!(flags.carry());
        assert!(flags.overflow());
        assert!(!flags.saturation());
        assert!(flags.thumb());
        assert_eq!(flags.exception_number, 0x108);
    }

    #[test]
    fn register_labels_include_aliases() {
        assert_eq!(register_label(13), "R13/SP");
        assert_eq!(register_label(14), "R14/LR");
        assert_eq!(register_label(15), "R15/PC");
        assert_eq!(register_label(7), "R7");
    }

    #[test]
    fn panel_renders_headlessly() {
        let core = MockCore::new();
        let context = egui::Context::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            RegisterPanel::new(&core).show(ui);
        });
        let shape_count = output.shapes.len();
        output.drop_without_applying_deltas();

        assert!(shape_count > 0, "panel should emit at least one draw shape");
    }
}
