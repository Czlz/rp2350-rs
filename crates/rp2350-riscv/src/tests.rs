// 测试用例使用 `code`/`core` 等惯用短名，名字相似告警在此不适用。
// Test cases use idiomatic short names such as `code`/`core`; the
// similar-names lint does not apply here.
#![allow(clippy::similar_names)]

use super::*;
use rp2350_core::memory::{RamRegion, SimpleBus};
use rp2350_cpu::CpuCore;

const CODE_BASE: u32 = 0x2000_0000;
const STACK_TOP: u32 = 0x2000_0F00;

/// Encode an I-type instruction / 编码 I 型指令
fn i_type(rd: u32, funct3: u32, rs1: u32, immediate: i32) -> u32 {
    0x13 | (rd << 7) | (funct3 << 12) | (rs1 << 15) | (((immediate as u32) & 0xFFF) << 20)
}

/// Encode an I-type load instruction / 编码 I 型加载指令
fn load_type(rd: u32, funct3: u32, rs1: u32, immediate: i32) -> u32 {
    0x03 | (rd << 7) | (funct3 << 12) | (rs1 << 15) | (((immediate as u32) & 0xFFF) << 20)
}

/// Encode a SYSTEM/CSR instruction / 编码 SYSTEM/CSR 指令
fn csr_type(rd: u32, funct3: u32, rs1: u32, csr: u32) -> u32 {
    0x73 | (rd << 7) | (funct3 << 12) | (rs1 << 15) | (csr << 20)
}

/// Encode an R-type instruction / 编码 R 型指令
fn r_type(rd: u32, funct3: u32, rs1: u32, rs2: u32, funct7: u32) -> u32 {
    0x33 | (rd << 7) | (funct3 << 12) | (rs1 << 15) | (rs2 << 20) | (funct7 << 25)
}

/// Encode an S-type instruction / 编码 S 型指令
fn s_type(funct3: u32, rs1: u32, rs2: u32, immediate: i32) -> u32 {
    let value = immediate as u32;
    0x23 | ((value & 0x1F) << 7)
        | (funct3 << 12)
        | (rs1 << 15)
        | (rs2 << 20)
        | (((value >> 5) & 0x7F) << 25)
}

/// Encode a B-type instruction / 编码 B 型指令
fn b_type(funct3: u32, rs1: u32, rs2: u32, immediate: i32) -> u32 {
    let value = immediate as u32;
    0x63 | (((value >> 11) & 1) << 7)
        | (((value >> 1) & 0xF) << 8)
        | (funct3 << 12)
        | (rs1 << 15)
        | (rs2 << 20)
        | (((value >> 5) & 0x3F) << 25)
        | (((value >> 12) & 1) << 31)
}

/// Encode a U-type instruction / 编码 U 型指令
fn u_type(rd: u32, immediate: u32) -> u32 {
    0x37 | (rd << 7) | (immediate & 0xFFFF_F000)
}

/// Encode a J-type instruction / 编码 J 型指令
fn j_type(rd: u32, immediate: i32) -> u32 {
    let value = immediate as u32;
    0x6F | (rd << 7)
        | (value & 0x000F_F000)
        | (((value >> 11) & 1) << 20)
        | (((value >> 1) & 0x3FF) << 21)
        | (((value >> 20) & 1) << 31)
}

/// Build a bus with code loaded at `CODE_BASE` / 在 `CODE_BASE` 装载代码的总线
fn bus_with_code(code: &[u32]) -> SimpleBus {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("sram", CODE_BASE, 0x1000)))
        .unwrap();
    let mut cycles = 0u64;
    for (index, word) in code.iter().enumerate() {
        let address = CODE_BASE + u32::try_from(index).unwrap() * 4;
        bus.write_u32(address, *word, &mut cycles).unwrap();
    }
    bus
}

/// Create a core ready to execute at `CODE_BASE` / 创建从 `CODE_BASE` 开始执行的核心
fn core_with_code(code: &[u32]) -> (Hazard3Core, SimpleBus) {
    let mut bus = bus_with_code(code);
    let mut core = Hazard3Core::new(CpuId::Core0);
    core.set_boot_vectors(STACK_TOP, CODE_BASE);
    core.init(&mut bus).unwrap();
    (core, bus)
}

#[test]
fn arch_is_hazard3() {
    let core = Hazard3Core::new(CpuId::Core0);
    assert_eq!(core.arch(), CpuArch::RiscVHazard3);
    assert_eq!(core.cpu_id(), CpuId::Core0);
    assert_eq!(core.state(), CpuState::Reset);
}

#[test]
fn rv32i_arithmetic_and_memory() {
    let code = [
        u_type(5, 0x2000_0000),
        i_type(1, 0, 0, 5),
        i_type(2, 0, 0, 7),
        r_type(3, 0, 1, 2, 0),
        s_type(2, 5, 3, 0x100),
        load_type(4, 2, 5, 0x100),
        i_type(6, 0, 4, 1),
    ];
    let (mut core, mut bus) = core_with_code(&code);
    for _ in 0..code.len() {
        core.step(&mut bus).unwrap();
    }
    assert_eq!(core.read_register(3), 12);
    assert_eq!(core.read_register(4), 12);
    assert_eq!(core.read_register(6), 13);

    let mut cycles = 0u64;
    assert_eq!(bus.read_u32(CODE_BASE + 0x100, &mut cycles).unwrap(), 12);
    assert_eq!(
        core.pc(),
        CODE_BASE + 4 * u32::try_from(code.len()).unwrap()
    );
}

#[test]
fn branches_and_jumps() {
    let code = [
        i_type(1, 0, 0, 3),
        i_type(2, 0, 0, 3),
        b_type(0, 1, 2, 8),
        i_type(3, 0, 0, 99),
        i_type(4, 0, 0, 7),
        j_type(5, 8),
        i_type(6, 0, 0, 99),
        i_type(7, 0, 0, 9),
    ];
    let (mut core, mut bus) = core_with_code(&code);
    // One branch is taken, so seven of the eight words execute.
    // 分支命中会跳过一条指令，因此八条指令中实际执行七条。
    for _ in 0..7 {
        core.step(&mut bus).unwrap();
    }
    assert_eq!(core.read_register(3), 0);
    assert_eq!(core.read_register(4), 7);
    assert_eq!(core.read_register(5), CODE_BASE + 5 * 4 + 4);
    assert_eq!(core.read_register(6), 0);
    assert_eq!(core.read_register(7), 9);
}

#[test]
fn m_extension_operations() {
    let code = [
        i_type(1, 0, 0, 6),
        i_type(2, 0, 0, 7),
        r_type(3, 0, 1, 2, 0x01),
        r_type(4, 4, 2, 1, 0x01),
        r_type(5, 6, 2, 1, 0x01),
        r_type(6, 5, 2, 0, 0x01),
        r_type(7, 7, 2, 0, 0x01),
    ];
    let (mut core, mut bus) = core_with_code(&code);
    for _ in 0..code.len() {
        core.step(&mut bus).unwrap();
    }
    assert_eq!(core.read_register(3), 42);
    assert_eq!(core.read_register(4), 1);
    assert_eq!(core.read_register(5), 1);
    assert_eq!(core.read_register(6), u32::MAX);
    assert_eq!(core.read_register(7), 7);
}

#[test]
fn csr_round_trip() {
    let code = [
        u_type(1, 0x1234_5000),
        csr_type(2, 1, 1, 0x340),
        csr_type(3, 2, 0, 0x340),
    ];
    let (mut core, mut bus) = core_with_code(&code);
    for _ in 0..code.len() {
        core.step(&mut bus).unwrap();
    }
    assert_eq!(core.read_register(2), 0);
    assert_eq!(core.read_register(3), 0x1234_5000);
    assert_eq!(core.csr.mscratch, 0x1234_5000);
}

#[test]
fn ecall_and_mret() {
    let mut bus = bus_with_code(&[0x0000_0073]);
    let mut cycles = 0u64;
    bus.write_u32(0x2000_0080, 0x3020_0073, &mut cycles)
        .unwrap();
    let mut core = Hazard3Core::new(CpuId::Core0);
    core.set_boot_vectors(STACK_TOP, CODE_BASE);
    core.init(&mut bus).unwrap();
    // Set the trap vector after `init`, which resets architectural CSR state.
    // 在 `init` 之后设置陷阱向量，因为 `init` 会复位架构 CSR 状态。
    core.csr.mtvec = 0x2000_0080;

    core.step(&mut bus).unwrap();
    assert_eq!(core.pc(), 0x2000_0080);
    assert_eq!(core.csr.mcause, 11);
    assert_eq!(core.csr.mepc, CODE_BASE);

    core.step(&mut bus).unwrap();
    assert_eq!(core.pc(), CODE_BASE);
}

#[test]
fn compressed_instructions_execute() {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("sram", CODE_BASE, 0x1000)))
        .unwrap();
    let mut cycles = 0u64;
    // c.addi x1, 5 ; c.addi x1, 7 / 压缩加法
    for (index, half) in [0x0095u16, 0x009D].iter().enumerate() {
        let address = CODE_BASE + u32::try_from(index).unwrap() * 2;
        bus.write_u16(address, *half, &mut cycles).unwrap();
    }
    let mut core = Hazard3Core::new(CpuId::Core0);
    core.set_boot_vectors(STACK_TOP, CODE_BASE);
    core.init(&mut bus).unwrap();
    core.step(&mut bus).unwrap();
    core.step(&mut bus).unwrap();
    assert_eq!(core.read_register(1), 12);
    assert_eq!(core.pc(), CODE_BASE + 4);
}

#[test]
fn decompress_examples() {
    use crate::execute_c::decompress;
    assert_eq!(decompress(0x0001), Some(0x0000_0013)); // c.nop
    assert_eq!(decompress(0x0095), Some(0x0050_8093)); // c.addi x1, 5
    assert_eq!(decompress(0x8082), Some(0x0000_8067)); // c.jr ra / ret
    assert_eq!(decompress(0x9002), Some(0x0010_0073)); // c.ebreak
    assert_eq!(decompress(0x4505), Some(0x0010_0513)); // c.li a0, 1
}

#[test]
fn boot_from_picobin_header() {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("flash", 0x1000_0000, 0x1000)))
        .unwrap();
    bus.register_region(Box::new(RamRegion::new("sram", 0x2000_0000, 0x1000)))
        .unwrap();
    let mut cycles = 0u64;
    bus.write_u32(0x1000_0000, RISCV_BOOT_MAGIC, &mut cycles)
        .unwrap();
    bus.write_u32(0x1000_0004, 0x2000_0101, &mut cycles)
        .unwrap();
    bus.write_u32(0x1000_0020, 0x2000_0F00, &mut cycles)
        .unwrap();

    let mut core = Hazard3Core::new(CpuId::Core0);
    core.init(&mut bus).unwrap();
    assert_eq!(core.pc(), 0x2000_0100);
    assert_eq!(core.read_register(2), 0x2000_0F00);
    assert_eq!(core.state(), CpuState::Running);
}

#[test]
fn external_interrupt_enters_handler() {
    let mut bus = bus_with_code(&[i_type(0, 0, 0, 0)]);
    let mut core = Hazard3Core::new(CpuId::Core0);
    core.set_boot_vectors(STACK_TOP, CODE_BASE);
    core.init(&mut bus).unwrap();
    core.csr.mtvec = 0x2000_0080;
    core.csr.mstatus = 0x8;
    core.csr.mie = 0x800;
    core.signal_interrupt(3);
    core.step(&mut bus).unwrap();
    assert_eq!(core.pc(), 0x2000_0080);
    assert_eq!(core.csr.mcause, 0x8000_000B);
    assert_eq!(core.csr.meipa, 3);
    assert_eq!(core.csr.mepc, CODE_BASE);
}
