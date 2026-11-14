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
fn rv32_zba_zbb_zbs_operations() {
    // Zba/Zbb/Zbs opcodes used by the RP2350 boot and MicroPython firmware.
    // RP2350 启动与 MicroPython 固件使用的 Zba/Zbb/Zbs 指令。
    let code = [
        i_type(1, 0, 0, 3),       // a1 = 3
        i_type(2, 0, 0, 5),       // a2 = 5
        r_type(3, 2, 1, 2, 0x10), // sh1add a3 = 2*3 + 5 = 11
        r_type(4, 4, 1, 2, 0x10), // sh2add a4 = 4*3 + 5 = 17
        r_type(5, 6, 1, 2, 0x10), // sh3add a5 = 8*3 + 5 = 29
        r_type(6, 7, 1, 2, 0x20), // andn a6 = 3 & ~5 = 2
        r_type(7, 1, 0, 1, 0x14), // bset a7 = 1 << 3 = 8
        r_type(8, 1, 7, 2, 0x24), // bclr a0 = 8 & ~(1<<5) = 8
        r_type(9, 5, 7, 1, 0x24), // bext a1 = (8 >> 3) & 1 = 1
    ];
    let (mut core, mut bus) = core_with_code(&code);
    for _ in 0..code.len() {
        core.step(&mut bus).unwrap();
    }
    assert_eq!(core.read_register(3), 11);
    assert_eq!(core.read_register(4), 17);
    assert_eq!(core.read_register(5), 29);
    assert_eq!(core.read_register(6), 2);
    assert_eq!(core.read_register(7), 8);
    assert_eq!(core.read_register(8), 8);
    assert_eq!(core.read_register(9), 1);
}

#[test]
fn byte_and_halfword_loads_sign_extend() {
    // LB/LH sign-extend 8/16-bit values to XLEN; LBU/LHU zero-extend.
    // LB/LH 将 8/16 位值符号扩展到 XLEN；LBU/LHU 做零扩展。
    let code = [
        u_type(5, 0x2000_0000),
        load_type(6, 0, 5, 0x100),
        load_type(7, 1, 5, 0x102),
        load_type(8, 4, 5, 0x100),
        load_type(9, 5, 5, 0x102),
    ];
    let mut bus = bus_with_code(&code);
    let mut cycles = 0u64;
    bus.write_u8(CODE_BASE + 0x100, 0x80, &mut cycles).unwrap();
    bus.write_u8(CODE_BASE + 0x102, 0x80, &mut cycles).unwrap();
    bus.write_u8(CODE_BASE + 0x103, 0xFF, &mut cycles).unwrap();
    let mut core = Hazard3Core::new(CpuId::Core0);
    core.set_boot_vectors(STACK_TOP, CODE_BASE);
    core.init(&mut bus).unwrap();
    for _ in 0..code.len() {
        core.step(&mut bus).unwrap();
    }
    assert_eq!(core.read_register(6), 0xFFFF_FF80);
    assert_eq!(core.read_register(7), 0xFFFF_FF80);
    assert_eq!(core.read_register(8), 0x80);
    assert_eq!(core.read_register(9), 0x0000_FF80);
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
    // Picobin IMAGE_DEF block at +0x14: marker, header, entry-point item,
    // then the boot2 entry and initial SP.
    // +0x14 处的 picobin IMAGE_DEF 块：标记、块头、入口点条目，随后是 boot2
    // 入口与初始 SP。
    bus.write_u32(0x1000_0014, RISCV_PICOBIN_MARKER_START, &mut cycles)
        .unwrap();
    bus.write_u32(0x1000_0018, 0x1101_0142, &mut cycles)
        .unwrap();
    bus.write_u32(0x1000_001C, 0x0000_0344, &mut cycles)
        .unwrap();
    bus.write_u32(0x1000_0020, 0x2000_0101, &mut cycles)
        .unwrap();
    bus.write_u32(0x1000_0024, 0x2000_0F00, &mut cycles)
        .unwrap();
    bus.write_u32(0x1000_0028, RISCV_PICOBIN_MARKER_END, &mut cycles)
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
    core.csr.xh3irq.enable[0] = 1 << 3;
    core.signal_interrupt(3);
    core.step(&mut bus).unwrap();
    assert_eq!(core.pc(), 0x2000_0080);
    assert_eq!(core.csr.mcause, 0x8000_000B);
    assert_eq!(core.csr.xh3irq.next, 3 << 2);
    assert_eq!(core.csr.xh3irq.pending[0] & (1 << 3), 0);
    assert_eq!(core.csr.xh3irq.preempt(), 1);
    assert_ne!(core.csr.xh3irq.context & 1, 0);
    assert_eq!(core.csr.mepc, CODE_BASE);
}

/// Encode a Zb* immediate instruction with explicit `rs2`/`funct7`
/// 编码带显式 `rs2`/`funct7` 的 Zb* 立即数指令
fn imm_special(rd: u32, funct3: u32, funct7: u32, rs2: u32, rs1: u32) -> u32 {
    0x13 | (rd << 7) | (funct3 << 12) | (rs1 << 15) | (rs2 << 20) | (funct7 << 25)
}

#[test]
fn zb_pack_and_packh_combine_halfwords() {
    // GCC emits PACK with rs2 == zero as ZEXT.H; PACKH keeps the low bytes.
    // GCC 用 rs2 == 0 的 PACK 实现 ZEXT.H；PACKH 仅保留低字节。
    let code = [
        u_type(5, 0x1234_5000),
        i_type(5, 0, 5, 0x678),
        r_type(6, 4, 5, 0, 0x04),
        r_type(7, 4, 5, 6, 0x04),
        r_type(8, 7, 5, 0, 0x04),
    ];
    let (mut core, mut bus) = core_with_code(&code);
    for _ in 0..code.len() {
        core.step(&mut bus).unwrap();
    }
    assert_eq!(core.read_register(6), 0x0000_5678);
    assert_eq!(core.read_register(7), 0x5678_5678);
    assert_eq!(core.read_register(8), 0x78);
}

#[test]
fn zb_bit_count_and_sign_extend() {
    let code = [
        i_type(5, 0, 0, 0x80),
        imm_special(6, 1, 0x30, 0, 5),
        imm_special(7, 1, 0x30, 1, 5),
        imm_special(8, 1, 0x30, 2, 5),
        imm_special(9, 1, 0x30, 4, 5),
        u_type(10, 0x0000_8000),
        imm_special(11, 1, 0x30, 5, 10),
    ];
    let (mut core, mut bus) = core_with_code(&code);
    for _ in 0..code.len() {
        core.step(&mut bus).unwrap();
    }
    assert_eq!(core.read_register(6), 24); // clz
    assert_eq!(core.read_register(7), 7); // ctz
    assert_eq!(core.read_register(8), 1); // cpop
    assert_eq!(core.read_register(9), 0xFFFF_FF80); // sext.b
    assert_eq!(core.read_register(11), 0xFFFF_8000); // sext.h
}

#[test]
fn zbs_immediate_operations() {
    let code = [
        i_type(5, 0, 0, 0),
        imm_special(5, 1, 0x14, 3, 5), // bseti
        imm_special(5, 1, 0x24, 3, 5), // bclri
        imm_special(5, 1, 0x34, 2, 5), // binvi
        imm_special(5, 5, 0x24, 2, 5), // bexti
        imm_special(5, 5, 0x30, 1, 5), // rori
    ];
    let (mut core, mut bus) = core_with_code(&code);
    for _ in 0..code.len() {
        core.step(&mut bus).unwrap();
    }
    assert_eq!(core.read_register(5), 0x8000_0000);
}

#[test]
fn compressed_lui_sign_extends() {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("sram", CODE_BASE, 0x1000)))
        .unwrap();
    let mut cycles = 0u64;
    // c.lui ra, -16 -> 0xFFFF_0000 / 负立即数必须填充高 18 位
    bus.write_u16(CODE_BASE, 0x70C1, &mut cycles).unwrap();
    let mut core = Hazard3Core::new(CpuId::Core0);
    core.set_boot_vectors(STACK_TOP, CODE_BASE);
    core.init(&mut bus).unwrap();
    core.step(&mut bus).unwrap();
    assert_eq!(core.read_register(1), 0xFFFF_0000);
}

#[test]
fn exceptions_use_mtvec_base_but_interrupts_vector() {
    let mut bus = bus_with_code(&[load_type(5, 2, 0, 0x100)]);
    let mut core = Hazard3Core::new(CpuId::Core0);
    core.set_boot_vectors(STACK_TOP, CODE_BASE);
    core.init(&mut bus).unwrap();
    core.csr.mtvec = 0x2000_0101;
    core.step(&mut bus).unwrap();
    assert_eq!(core.pc(), 0x2000_0100);
    assert_eq!(core.csr.mcause, 5);

    let mut bus = bus_with_code(&[i_type(0, 0, 0, 0)]);
    let mut core = Hazard3Core::new(CpuId::Core0);
    core.set_boot_vectors(STACK_TOP, CODE_BASE);
    core.init(&mut bus).unwrap();
    core.csr.mtvec = 0x2000_0101;
    core.csr.mstatus = 0x8;
    core.csr.mie = 0x800;
    core.csr.xh3irq.enable[0] = 1 << 3;
    core.signal_interrupt(3);
    core.step(&mut bus).unwrap();
    assert_eq!(core.pc(), 0x2000_012C);
    assert_eq!(core.csr.mcause, 0x8000_000B);
}

#[test]
fn fatal_trap_vector_routes_cause_two_and_five() {
    let mut bus = bus_with_code(&[load_type(5, 2, 0, 0x100)]);
    let mut core = Hazard3Core::new(CpuId::Core0);
    core.set_boot_vectors(STACK_TOP, CODE_BASE);
    core.init(&mut bus).unwrap();
    core.set_fatal_trap_vector(0x0000_7E00);
    core.step(&mut bus).unwrap();
    assert_eq!(core.pc(), 0x0000_7E14);
    assert_eq!(core.csr.mcause, 5);
    assert_eq!(core.csr.mepc, CODE_BASE);
    assert_eq!(core.csr.mtval, 0x100);
}

#[test]
fn csrrc_keeps_the_meiea_window_index_from_the_raw_operand() {
    // `csrrc meiea, a1` 的读-改-写结果会把窗口索引位清零；若用结果选窗口，
    // 清除窗口 1 的 IRQ30 会误清窗口 0 的 IRQ14（USB）。
    // The read-modify-write result of `csrrc meiea, a1` zeroes the window
    // index; selecting the window from that result would clear USB IRQ 14 in
    // window 0 while disabling IRQ 30 in window 1.
    let code = [
        u_type(11, 0x4000_0000),
        i_type(11, 0, 11, 1),      // a1 = (1<<14)<<16 | 1
        csr_type(0, 3, 11, 0xBE0), // csrrc: disable IRQ 30 (window 1)
        csr_type(0, 2, 11, 0xBE0), // csrrs: re-enable IRQ 30 (window 1)
    ];
    let (mut core, mut bus) = core_with_code(&code);
    core.csr.xh3irq.enable = [0x4000, 0x4000, 0, 0];
    for _ in 0..3 {
        core.step(&mut bus).unwrap();
    }
    assert_eq!(core.csr.xh3irq.enable[1] & 0x4000, 0);
    assert_ne!(
        core.csr.xh3irq.enable[0] & 0x4000,
        0,
        "window 0 (USB IRQ 14) must not be touched"
    );
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.xh3irq.enable[1] & 0x4000, 0x4000);
    assert_eq!(core.csr.xh3irq.enable[0] & 0x4000, 0x4000);
}

#[test]
fn zcb_ext_and_mul_expand() {
    use crate::execute_c::decompress;
    assert_eq!(decompress(0x9C61), Some(0x0FF4_7413)); // c.zext.b x8
    assert_eq!(decompress(0x9C65), Some(0x6044_1413)); // c.sext.b x8
    assert_eq!(decompress(0x9C69), Some(0x0804_4433)); // c.zext.h x8
    assert_eq!(decompress(0x9C6D), Some(0x6054_1413)); // c.sext.h x8
    assert_eq!(decompress(0x9C75), Some(0xFFF4_4413)); // c.not x8
    assert_eq!(decompress(0x9C41), Some(0x0284_0433)); // c.mul x8,x8,x8
}

#[test]
fn zcmp_push_pop_encodings_decode() {
    use crate::execute_c::{ZcmpOp, zcmp_op};
    assert_eq!(zcmp_op(0xB842), Some(ZcmpOp::Push { rlist: 4, spimm: 0 }));
    assert_eq!(zcmp_op(0xB862), Some(ZcmpOp::Push { rlist: 6, spimm: 0 }));
    assert_eq!(zcmp_op(0xB846), Some(ZcmpOp::Push { rlist: 4, spimm: 1 }));
    assert_eq!(zcmp_op(0xBA42), Some(ZcmpOp::Pop { rlist: 4, spimm: 0 }));
    assert_eq!(
        zcmp_op(0xBC42),
        Some(ZcmpOp::PopRetZ { rlist: 4, spimm: 0 })
    );
    assert_eq!(zcmp_op(0xBE42), Some(ZcmpOp::PopRet { rlist: 4, spimm: 0 }));
    // 寄存器列表 0..3 是保留编码，应留给非法指令路径。
    // Register lists 0..3 are reserved and must stay on the illegal path.
    assert_eq!(zcmp_op(0xB802), None);
    assert_eq!(zcmp_op(0xB812), None);
}

/// Build a bus with 16-bit code halves loaded at `CODE_BASE`
/// 在 `CODE_BASE` 装载 16 位代码半字的总线
fn bus_with_compressed_code(code: &[u16]) -> SimpleBus {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("sram", CODE_BASE, 0x1000)))
        .unwrap();
    let mut cycles = 0u64;
    for (index, half) in code.iter().enumerate() {
        let address = CODE_BASE + u32::try_from(index).unwrap() * 2;
        bus.write_u16(address, *half, &mut cycles).unwrap();
    }
    bus
}

/// Create a core executing compressed code at `CODE_BASE`
/// 创建从 `CODE_BASE` 执行压缩指令代码的核心
fn core_with_compressed_code(code: &[u16]) -> (Hazard3Core, SimpleBus) {
    let mut bus = bus_with_compressed_code(code);
    let mut core = Hazard3Core::new(CpuId::Core0);
    core.set_boot_vectors(STACK_TOP, CODE_BASE);
    core.init(&mut bus).unwrap();
    (core, bus)
}

#[test]
fn zcmp_push_and_popret_round_trip_the_frame() {
    // `cm.push {ra}, -16` / `cm.popret {ra}, 16`，pico-sdk 中断处理链的入栈/返回。
    // pico-sdk's IRQ handler chain enters and leaves frames exactly this way.
    let (mut core, mut bus) = core_with_compressed_code(&[0xB842, 0xBE42]);
    let return_address = 0x2000_0080;
    core.registers.write(1, return_address);
    core.registers.write(2, STACK_TOP);

    core.step(&mut bus).unwrap();
    assert_eq!(core.read_register(2), STACK_TOP - 16);
    let mut cycles = 0u64;
    assert_eq!(
        bus.read_u32(STACK_TOP - 4, &mut cycles).unwrap(),
        return_address
    );

    core.step(&mut bus).unwrap();
    assert_eq!(core.read_register(2), STACK_TOP);
    assert_eq!(core.read_register(1), return_address);
    assert_eq!(core.pc(), return_address);
}

#[test]
fn zcmp_push_and_popret_restore_the_register_list() {
    // `cm.push {ra, s0-s1}, -16` / `cm.popret {ra, s0-s1}, 16`：高编号寄存器
    // 位于更高地址，ra 位于帧的最低地址。
    // Higher-numbered registers sit at higher addresses; ra sits lowest.
    let (mut core, mut bus) = core_with_compressed_code(&[0xB862, 0xBE62]);
    core.registers.write(1, 0x2000_0080);
    core.registers.write(8, 0x1111_1111);
    core.registers.write(9, 0x2222_2222);
    core.registers.write(2, STACK_TOP);

    core.step(&mut bus).unwrap();
    let mut cycles = 0u64;
    assert_eq!(
        bus.read_u32(STACK_TOP - 4, &mut cycles).unwrap(),
        0x2222_2222
    );
    assert_eq!(
        bus.read_u32(STACK_TOP - 8, &mut cycles).unwrap(),
        0x1111_1111
    );
    assert_eq!(
        bus.read_u32(STACK_TOP - 12, &mut cycles).unwrap(),
        0x2000_0080
    );

    core.registers.write(1, 0);
    core.registers.write(8, 0);
    core.registers.write(9, 0);
    core.step(&mut bus).unwrap();
    assert_eq!(core.read_register(1), 0x2000_0080);
    assert_eq!(core.read_register(8), 0x1111_1111);
    assert_eq!(core.read_register(9), 0x2222_2222);
    assert_eq!(core.read_register(2), STACK_TOP);
    assert_eq!(core.pc(), 0x2000_0080);
}

#[test]
fn external_irq_requires_enable_and_global_masks() {
    // 逐项验证 Xh3irq 的三层门控：meiea 使能位、mie.MEIE 与 mstatus.MIE。
    // Check all three Xh3irq gates: meiea enable, mie.MEIE and mstatus.MIE.
    for (enable, mie, mstatus) in [(0, 0x800, 0x8), (1 << 3, 0, 0x8), (1 << 3, 0x800, 0)] {
        let (mut core, mut bus) = core_with_code(&[i_type(0, 0, 0, 0)]);
        core.csr.mtvec = 0x2000_0080;
        core.csr.mie = mie;
        core.csr.mstatus = mstatus;
        core.csr.xh3irq.enable[0] = enable;
        core.signal_interrupt(3);
        core.step(&mut bus).unwrap();
        assert_eq!(
            core.pc(),
            CODE_BASE + 4,
            "gates enable={enable:#X} mie={mie:#X} mstatus={mstatus:#X} must block IRQ 3"
        );
        assert_eq!(core.csr.mcause, 0);
    }
}

#[test]
fn xh3irq_priorities_control_meinext_and_preemption() {
    let (mut core, mut bus) = core_with_code(&[i_type(0, 0, 0, 0)]);
    core.csr.mtvec = 0x2000_0080;
    core.csr.mstatus = 0x8;
    core.csr.mie = 0x800;
    core.csr.xh3irq.enable[0] = (1 << 3) | (1 << 4);
    // IRQ 3 priority 5, IRQ 4 priority 7 (row 0, shift 12 / row 1, shift 0).
    // IRQ 3 优先级 5，IRQ 4 优先级 7。
    core.csr.xh3irq.priority[0] = 5 << 12;
    core.csr.xh3irq.priority[1] = 7;
    core.signal_interrupt(3);
    core.signal_interrupt(4);
    // The higher-priority IRQ wins MEINEXT and the trap. / 高优先级 IRQ 赢得 MEINEXT 与陷阱。
    assert_eq!(core.csr.xh3irq.next, 4 << 2);
    assert_ne!(core.csr.mip & (1 << 11), 0);
    core.step(&mut bus).unwrap();
    assert_eq!(core.pc(), 0x2000_0080);
    assert_eq!(core.csr.xh3irq.preempt(), 8);

    // 处理程序重新开中断后，低优先级 IRQ 3 不得嵌套。
    // After the handler re-enables interrupts, the lower-priority IRQ 3 must wait.
    core.csr.mstatus |= 0x8;
    core.signal_interrupt(3);
    let mut cycles = 0u64;
    bus.write_u32(0x2000_0080, i_type(0, 0, 0, 0), &mut cycles)
        .unwrap();
    core.step(&mut bus).unwrap();
    assert_eq!(core.pc(), 0x2000_0084);
    assert_eq!(core.csr.mcause, 0x8000_000B);

    // A higher-priority IRQ 5 (priority 12) may preempt the level-8 handler.
    // 更高优先级的 IRQ 5（优先级 12）可以抢占级别 8 的处理程序。
    core.csr.xh3irq.enable[0] |= 1 << 5;
    core.csr.xh3irq.priority[1] |= 12 << 4;
    core.signal_interrupt(5);
    core.step(&mut bus).unwrap();
    assert_eq!(core.pc(), 0x2000_0080);
    assert_eq!(core.csr.xh3irq.preempt(), 13);
    assert_eq!(core.csr.xh3irq.ppreempt(), 8);
    assert_ne!(core.csr.xh3irq.context & 1, 0);
}

#[test]
fn nested_low_priority_irq_is_delivered_after_mret() {
    // 低优先级挂起 IRQ 在 handler 内不抢占，但 mret 之后必须仍能投递。
    // A pending lower-priority IRQ must not preempt inside the handler, yet it
    // must still be delivered after mret restores the preemption level.
    let (mut core, mut bus) = core_with_code(&[i_type(0, 0, 0, 0)]);
    core.csr.mtvec = 0x2000_0080;
    core.csr.mstatus = 0x8;
    core.csr.mie = 0x800;
    core.csr.xh3irq.enable[0] = (1 << 3) | (1 << 4);
    core.csr.xh3irq.priority[0] = 3 << 12;
    core.csr.xh3irq.priority[1] = 6;
    core.signal_interrupt(3);
    core.signal_interrupt(4);
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.xh3irq.preempt(), 7);

    // Handler drops to the mret path with IRQ 3 still pending.
    // 处理程序在 IRQ 3 仍挂起的状态下走到 mret。
    core.csr.mstatus |= 0x8;
    core.signal_interrupt(3);
    let mut cycles = 0u64;
    bus.write_u32(0x2000_0080, 0x3020_0073, &mut cycles)
        .unwrap();
    core.csr.mepc = CODE_BASE;
    core.step(&mut bus).unwrap();
    assert_eq!(core.pc(), CODE_BASE);
    assert_eq!(core.csr.xh3irq.preempt(), 0);
    assert_eq!(core.csr.xh3irq.context & 1, 0);

    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.xh3irq.preempt(), 4); // 1 + priority 3 / 1 + 优先级 3
}
