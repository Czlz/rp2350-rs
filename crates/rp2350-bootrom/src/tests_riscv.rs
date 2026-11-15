//! RISC-V fatal-exception vector tests / RISC-V 致命异常向量测试

use super::*;
use crate::table::flags;
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

/// Call one `rom_func_lookup` / `rom_data_lookup` trap directly
/// 直接调用一次 `rom_func_lookup` / `rom_data_lookup` 陷阱
fn lookup_trap(rom: &Rp2350Bootrom, id: u16, tag: TableCode) -> u32 {
    let flash = FlashRegion::rp2350_xip();
    let handler = Rp2350TrapHandler {
        image: Arc::new(build_image()),
        flash: flash.handle(),
        boot_random: [0; 4],
    };
    let mut bus = SimpleBus::new();
    rom.mount(&mut bus).unwrap();
    let mut cycles = 0u64;
    handler
        .invoke(
            TrapCall {
                id: TrapId(id),
                args: [u32::from(tag), 0x20, 0, 0],
            },
            &mut bus,
            &mut cycles,
        )
        .unwrap()
        .result
}

#[test]
fn riscv_implemented_tags_expose_riscv_pointers() {
    // 每个真实实现的 ROM 函数在 RISC-V 标志下都给出非零桩地址。
    // Every implemented ROM function exposes a non-zero stub under the RISC-V flag.
    let rom = bootrom();
    let table = &rom.image()[TABLE_OFFSET..];
    for trap in ALL_TRAPS {
        let entry = table::lookup(table, trap.tag(), flags::FUNC_RISCV)
            .unwrap_or_else(|| panic!("missing RISC-V entry for {:#06X}", trap.tag()));
        assert_eq!(u32::from(entry), trap.rv_stub());
        assert!(rom.handles(trap.rv_stub()));
    }
}

#[test]
fn riscv_common_function_tags_resolve_to_valid_stubs() {
    // 未实现的常用表项必须有有效（非 0）且被分发器识别的桩地址。
    // Unimplemented common tags must resolve to valid, dispatchable stub addresses.
    let rom = bootrom();
    let table = &rom.image()[TABLE_OFFSET..];
    for tag in RV_STUB_FUNC_TAGS {
        let entry = table::lookup(table, tag, flags::FUNC_RISCV)
            .unwrap_or_else(|| panic!("missing RISC-V entry for {tag:#06X}"));
        assert_ne!(entry, 0, "tag {tag:#06X} must not be a null pointer");
        assert!(
            rom.handles(u32::from(entry)),
            "tag {tag:#06X} must be dispatchable"
        );
        assert_eq!(lookup_trap(&rom, TRAP_RV_LOOKUP, tag), u32::from(entry));
    }
}

#[test]
fn riscv_unknown_function_tag_returns_zero() {
    // 未知标签必须返回 ROM 的「未找到」值 0，而不是任意桩地址。
    // Unknown tags must return the ROM's "not found" value 0, not a stray stub.
    let rom = bootrom();
    for tag in [code(b'Z', b'Z'), code(b'?', b'?'), 0] {
        assert_eq!(lookup_trap(&rom, TRAP_RV_LOOKUP, tag), 0);
    }
}

#[test]
fn riscv_required_tags_are_present() {
    // 任务要求的四个入口：_reset_to_usb_boot('UB')、_flash_exit_xip('EX')、
    // _flash_flush_cache('FC') 与 _bootrom_func_table（0x14 处的表指针）。
    // The four required entry points: _reset_to_usb_boot('UB'), _flash_exit_xip
    // ('EX'), _flash_flush_cache('FC') and _bootrom_func_table (the 0x14 pointer).
    let rom = bootrom();
    for tag in [code(b'U', b'B'), code(b'E', b'X'), code(b'F', b'C')] {
        let address = lookup_trap(&rom, TRAP_RV_LOOKUP, tag);
        assert_ne!(address, 0, "tag {tag:#06X} must resolve");
        assert!(rom.handles(address));
    }
    let table_pointer = u16::from_le_bytes(
        rom.image()[FUNC_TABLE_PTR_OFFSET..FUNC_TABLE_PTR_OFFSET + 2]
            .try_into()
            .unwrap(),
    );
    assert_eq!(u32::from(table_pointer), TABLE_OFFSET as u32);
    let table = &rom.image()[TABLE_OFFSET..];
    assert!(
        table.iter().any(|byte| *byte != 0),
        "table must not be empty"
    );
}

#[test]
fn riscv_data_tags_point_at_a_zero_literal() {
    // 未实现的数据标签返回有效指针，指向的 32 位字为 0。
    // Unimplemented data tags return a valid pointer to a 32-bit zero word.
    let rom = bootrom();
    for tag in RV_STUB_DATA_TAGS {
        let address = lookup_trap(&rom, TRAP_RV_DATA_LOOKUP, tag);
        assert_eq!(address, RV_ZERO_DATA_PTR_OFFSET as u32);
    }
    let mut cycles = 0u64;
    let mut bus = SimpleBus::new();
    rom.mount(&mut bus).unwrap();
    assert_eq!(
        bus.read_u32(RV_ZERO_DATA_PTR_OFFSET as u32, &mut cycles)
            .unwrap(),
        0
    );
    // `FD` 仍走两级间接并指向 Flash 设备信息字。
    // `FD` keeps its two-level indirection to the flash device-info word.
    assert_eq!(
        lookup_trap(&rom, TRAP_RV_DATA_LOOKUP, code(b'F', b'D')),
        DEVINFO_PTR_OFFSET as u32
    );
}

#[test]
fn riscv_zero_stub_returns_zero() {
    // 桩本身被分发器识别，调用结果为 0 且不停机。
    // The stub is dispatchable, returns 0 and never halts.
    let rom = bootrom();
    for index in 0..RV_STUB_FUNC_TAGS.len() {
        assert!(rom.handles(rv_zero_stub(index)));
    }
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
                id: TrapId(TRAP_RV_ZERO_STUB),
                args: [0; 4],
            },
            &mut bus,
            &mut cycles,
        )
        .unwrap();
    assert_eq!(outcome.result, 0);
    assert!(outcome.halt.is_none());
}
