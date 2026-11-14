//! RISC-V fatal-exception vector tests / RISC-V 致命异常向量测试

use super::*;
use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;
use rp2350_core::types::{CpuId, CycleCount, IrqNumber};
use rp2350_core::{FlashRegion, SimpleBus};
use rp2350_cpu::{CpuArch, CpuCore, CpuState};

fn bootrom() -> Rp2350Bootrom {
    let flash = FlashRegion::rp2350_xip();
    Rp2350Bootrom::new(flash.handle())
}

/// Minimal RISC-V core used to exercise the exception vector dispatcher
/// 用于测试异常向量分发器的最小 RISC-V 核心
struct MockCore {
    pc: u32,
    regs: [u32; 32],
    csrs: [u32; 0x400],
}

impl CpuCore for MockCore {
    fn cpu_id(&self) -> CpuId {
        CpuId::Core0
    }
    fn arch(&self) -> CpuArch {
        CpuArch::RiscVHazard3
    }
    fn init(&mut self, _bus: &mut dyn AddressBus) -> Result<(), CoreError> {
        Ok(())
    }
    fn run_cycles(
        &mut self,
        _bus: &mut dyn AddressBus,
        cycles: CycleCount,
    ) -> (CycleCount, CpuState) {
        (cycles, CpuState::Running)
    }
    fn step(&mut self, _bus: &mut dyn AddressBus) -> Result<CpuState, CoreError> {
        Ok(CpuState::Running)
    }
    fn signal_interrupt(&mut self, _irq: IrqNumber) {}
    fn servicing_irq(&self) -> Option<IrqNumber> {
        None
    }
    fn pc(&self) -> u32 {
        self.pc
    }
    fn set_pc(&mut self, pc: u32) {
        self.pc = pc;
    }
    fn read_register(&self, index: u8) -> u32 {
        self.regs[usize::from(index)]
    }
    fn write_register(&mut self, index: u8, value: u32) {
        if index != 0 {
            self.regs[usize::from(index)] = value;
        }
    }
    fn reset(&mut self) {}
    fn description(&self) -> String {
        String::from("mock RISC-V core")
    }
    fn csr_read(&self, addr: u32) -> u32 {
        self.csrs.get(addr as usize).copied().unwrap_or(0)
    }
}

fn mock_riscv(pc: u32, mcause: u32, mepc: u32, mtval: u32) -> MockCore {
    let mut csrs = [0u32; 0x400];
    csrs[0x342] = mcause;
    csrs[0x341] = mepc;
    csrs[0x343] = mtval;
    MockCore {
        pc,
        regs: [0; 32],
        csrs,
    }
}

#[test]
fn riscv_exception_vector_image_jumps_to_the_handler() {
    let rom = bootrom();
    for cause in RISCV_FATAL_CAUSES {
        let slot = RISCV_EXCEPTION_VECTOR_BASE + 4 * cause;
        let word = u32::from_le_bytes(
            rom.image()[slot as usize..slot as usize + 4]
                .try_into()
                .unwrap(),
        );
        assert_eq!(word, riscv_jal_x0(slot, RISCV_EXCEPTION_HANDLER));
        assert_eq!(word & 0x7F, 0x6F);
        let imm = ((word >> 31) & 1) << 20
            | ((word >> 12) & 0xFF) << 12
            | ((word >> 20) & 1) << 11
            | ((word >> 21) & 0x3FF) << 1;
        assert_eq!(slot.wrapping_add(imm), RISCV_EXCEPTION_HANDLER);
    }
    let handler = RISCV_EXCEPTION_HANDLER as usize;
    let word = u32::from_le_bytes(rom.image()[handler..handler + 4].try_into().unwrap());
    assert_eq!(word, 0x0010_0073);
}

#[test]
fn riscv_exception_vectors_are_host_traps() {
    let rom = bootrom();
    assert!(rom.handles(RISCV_EXCEPTION_VECTOR_BASE + 4 * 2));
    assert!(rom.handles(RISCV_EXCEPTION_VECTOR_BASE + 4 * 5));
    assert!(!rom.handles(RISCV_EXCEPTION_VECTOR_BASE + 4 * 3));
    assert_eq!(rom.riscv_exception_vector(), RISCV_EXCEPTION_VECTOR_BASE);
}

#[test]
fn riscv_data_lookup_slot_returns_the_devinfo_pointer() {
    let rom = bootrom();
    assert_eq!(
        u16::from_le_bytes(
            rom.image()[RV_DATA_LOOKUP_PTR_OFFSET..RV_DATA_LOOKUP_PTR_OFFSET + 2]
                .try_into()
                .unwrap()
        ),
        RV_DATA_LOOKUP_STUB as u16
    );
    assert!(rom.handles(RV_DATA_LOOKUP_STUB));
    assert!(rom.handles(RV_LOOKUP_STUB));

    let flash = FlashRegion::rp2350_xip();
    let handler = Rp2350TrapHandler {
        image: Arc::new(build_image()),
        flash: flash.handle(),
        boot_random: [0; 4],
    };
    let mut bus = SimpleBus::new();
    rom.mount(&mut bus).unwrap();
    let mut cycles = 0u64;
    let outcome = handler
        .invoke(
            TrapCall {
                id: TrapId(TRAP_RV_DATA_LOOKUP),
                args: [u32::from(code(b'F', b'D')), 0x40, 0, 0],
            },
            &mut bus,
            &mut cycles,
        )
        .unwrap();
    assert_eq!(outcome.result, DEVINFO_PTR_OFFSET as u32);
    assert_eq!(
        bus.read_u32(DEVINFO_PTR_OFFSET as u32, &mut cycles)
            .unwrap(),
        DEVINFO_OFFSET as u32
    );
    assert_eq!(
        bus.read_u16(DEVINFO_OFFSET as u32, &mut cycles).unwrap(),
        DEVINFO_VALUE
    );
}

#[test]
fn riscv_fatal_trap_halts_with_csr_diagnostics() {
    let rom = bootrom();
    let mut bus = SimpleBus::new();
    let mut core = mock_riscv(
        RISCV_EXCEPTION_VECTOR_BASE + 4 * 5,
        5,
        0x1003_30AA,
        0x0E0F_070B,
    );
    let outcome = rom.dispatch(&mut core, &mut bus).unwrap().unwrap();
    let halt = outcome.halt.expect("fatal trap must halt");
    assert!(matches!(
        halt,
        HaltReason::RiscvException {
            mcause: 5,
            mepc: 0x1003_30AA,
            mtval: 0x0E0F_070B
        }
    ));
    assert_eq!(core.pc(), 0x1003_30AA);
    assert!(halt.to_string().contains("load access fault"));
}
