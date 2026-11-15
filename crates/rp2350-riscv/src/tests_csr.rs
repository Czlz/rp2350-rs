//! CSR legality, trap semantics and machine-CSR coverage tests
//! CSR 合法性、陷阱语义与机器 CSR 覆盖测试
//!
//! The instruction path must raise illegal-instruction (`mcause = 2`) for
//! unimplemented CSRs and for illegal accesses (writes to read-only registers),
//! and must keep working for the CSRs firmware polls during boot.
//! 指令路径对未实现 CSR 与非法访问（写只读寄存器）必须触发非法指令
//! （`mcause = 2`），同时保持固件启动期轮询的 CSR 正常工作。

// 测试用例反复使用 `code`/`core`/`csr` 等惯用短名，similar-names 告警不适用。
// Test cases reuse idiomatic short names such as `code`/`core`/`csr`; the
// similar-names lint does not apply here.
#![allow(clippy::similar_names)]

use super::*;
use rp2350_core::memory::{RamRegion, SimpleBus};
use rp2350_cpu::CpuCore;

const CODE_BASE: u32 = 0x2000_0000;
const STACK_TOP: u32 = 0x2000_0F00;
/// Trap vector used by the tests; word-aligned base, no vector mode
/// 测试使用的陷阱向量；按字对齐基址，不启用向量模式
const MT_VEC: u32 = 0x2000_0100;
/// CSR chosen because nothing models it (bits [11:10] = 0b01, reserved space)
/// 选用的 CSR 无人建模（位 [11:10] = 0b01，保留空间）
const UNIMPLEMENTED_CSR: u32 = 0x7C0;

/// Encode a SYSTEM/CSR instruction / 编码 SYSTEM/CSR 指令
///
/// `funct3` 1..=3 are the register forms (CSRRW/CSRRS/CSRRC); 5..=7 are the
/// immediate forms, where `rs1` carries a 5-bit unsigned immediate.
/// `funct3` 1..=3 是寄存器形式（CSRRW/CSRRS/CSRRC）；5..=7 是立即数形式，
/// 此时 `rs1` 承载 5 位无符号立即数。
fn csr_type(rd: u32, funct3: u32, rs1: u32, csr: u32) -> u32 {
    0x73 | (rd << 7) | (funct3 << 12) | (rs1 << 15) | (csr << 20)
}

/// Encode `lui rd, imm` / 编码 `lui rd, imm`
fn u_type(rd: u32, immediate: u32) -> u32 {
    0x37 | (rd << 7) | (immediate & 0xFFFF_F000)
}

/// Build a core executing `code` at `CODE_BASE` with `mtvec` already installed
/// 构建在 `CODE_BASE` 执行 `code` 且已安装 `mtvec` 的核心
fn core_with(code: &[u32], mtvec: u32) -> (Hazard3Core, SimpleBus) {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("sram", CODE_BASE, 0x1000)))
        .unwrap();
    let mut cycles = 0u64;
    for (index, word) in code.iter().enumerate() {
        let address = CODE_BASE + u32::try_from(index).unwrap() * 4;
        bus.write_u32(address, *word, &mut cycles).unwrap();
    }
    let mut core = Hazard3Core::new(CpuId::Core0);
    core.set_boot_vectors(STACK_TOP, CODE_BASE);
    core.init(&mut bus).unwrap();
    // `init` resets the CSR file, so the vector must be installed afterwards.
    // `init` 会复位 CSR 文件，因此必须在它之后安装向量。
    core.csr.mtvec = mtvec;
    (core, bus)
}

/// Assert that the last trap was an illegal-CSR trap for `csr`
/// 断言最近一次陷阱是针对 `csr` 的非法 CSR 陷阱
fn assert_illegal_csr(core: &Hazard3Core, csr: u32, faulting_pc: u32, handler_pc: u32) {
    assert_eq!(core.pc(), handler_pc, "trap must reach the handler");
    assert_eq!(core.csr.mcause, 2, "illegal CSR uses cause 2");
    assert_eq!(core.csr.mtval, csr, "mtval carries the CSR address");
    assert_eq!(
        core.csr.mepc, faulting_pc,
        "mepc points at the csr* instruction"
    );
}

#[test]
fn illegal_csr_read_traps_with_the_csr_address() {
    // csrrs a0, 0x7C0, x0 — a pure read of an unimplemented CSR.
    // csrrs a0, 0x7C0, x0 —— 对未实现 CSR 的纯读。
    let code = [csr_type(5, 2, 0, UNIMPLEMENTED_CSR)];
    let (mut core, mut bus) = core_with(&code, MT_VEC);
    core.step(&mut bus).unwrap();
    assert_illegal_csr(&core, UNIMPLEMENTED_CSR, CODE_BASE, MT_VEC);
    assert_eq!(core.read_register(5), 0, "rd must not be written on a trap");
}

#[test]
fn illegal_csr_write_traps_with_the_csr_address() {
    // csrrw x0, 0x7C0, a0 — an explicit write to an unimplemented CSR.
    // csrrw x0, 0x7C0, a0 —— 对未实现 CSR 的显式写。
    let code = [
        u_type(10, 0x0000_1000),
        csr_type(0, 1, 10, UNIMPLEMENTED_CSR),
    ];
    let (mut core, mut bus) = core_with(&code, MT_VEC);
    core.step(&mut bus).unwrap();
    core.step(&mut bus).unwrap();
    assert_illegal_csr(&core, UNIMPLEMENTED_CSR, CODE_BASE + 4, MT_VEC);
}

#[test]
fn illegal_csr_immediate_form_traps() {
    // csrrsi x0, 0x7C0, 1 — the immediate encoding must take the same path.
    // csrrsi x0, 0x7C0, 1 —— 立即数编码必须走同一条路径。
    let code = [csr_type(0, 6, 1, UNIMPLEMENTED_CSR)];
    let (mut core, mut bus) = core_with(&code, MT_VEC);
    core.step(&mut bus).unwrap();
    assert_illegal_csr(&core, UNIMPLEMENTED_CSR, CODE_BASE, MT_VEC);
}

#[test]
fn read_only_counter_write_traps() {
    // csrrw x0, cycle, x0 — bits [11:10] == 0b11 make `cycle` read-only, so even
    // a zero write is illegal. / `cycle` 的位 [11:10] == 0b11 表示只读，即使写 0
    // 也非法。
    let code = [csr_type(0, 1, 0, 0xC00)];
    let (mut core, mut bus) = core_with(&code, MT_VEC);
    core.step(&mut bus).unwrap();
    assert_illegal_csr(&core, 0xC00, CODE_BASE, MT_VEC);
}

#[test]
fn read_only_counter_read_is_legal() {
    // csrrs a0, instret, x0 — the firmware-readable retired counter.
    // csrrs a0, instret, x0 —— 固件可读的退休指令计数器。
    let code = [csr_type(5, 2, 0, 0xC02), csr_type(6, 2, 0, 0xC02)];
    let (mut core, mut bus) = core_with(&code, MT_VEC);
    core.step(&mut bus).unwrap();
    // The counter ticks after each instruction, so the second read is larger.
    // 计数器在每条指令后递增，因此第二次读取更大。
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.mcause, 0, "reading cycle/instret must not trap");
    assert!(core.read_register(6) > core.read_register(5));
}

#[test]
fn zero_source_read_does_not_count_as_a_write() {
    // csrrs x0, cycle, x0 performs no write, so the read-only check must not fire.
    // csrrs x0, cycle, x0 不执行写操作，只读检查不得触发。
    let code = [csr_type(0, 2, 0, 0xC00)];
    let (mut core, mut bus) = core_with(&code, MT_VEC);
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.mcause, 0);
    assert_eq!(core.pc(), CODE_BASE + 4);
}

#[test]
fn misa_advertises_the_b_extension() {
    // csrrs a0, misa, x0 / 读取 misa
    let code = [csr_type(5, 2, 0, 0x301)];
    let (mut core, mut bus) = core_with(&code, MT_VEC);
    core.step(&mut bus).unwrap();
    let misa = core.read_register(5);
    assert_eq!(core.csr.mcause, 0);
    assert_eq!(misa >> 30, 1, "MXL must be 32-bit");
    assert_eq!(misa & (1 << 0), 1, "A");
    assert_eq!(misa & (1 << 1), 1 << 1, "B (Zba/Zbb/Zbs)");
    assert_eq!(misa & (1 << 2), 1 << 2, "C");
    assert_eq!(misa & (1 << 8), 1 << 8, "I");
    assert_eq!(misa & (1 << 12), 1 << 12, "M");
    assert_eq!(misa & (1 << 5), 0, "F must stay absent");
}

#[test]
fn misa_write_is_illegal() {
    // `misa` lives in the read/write encoding space but is hard-wired in Hazard3.
    // `misa` 位于读写编码空间，但在 Hazard3 中硬连线只读。
    let code = [csr_type(0, 1, 0, 0x301)];
    let (mut core, mut bus) = core_with(&code, MT_VEC);
    core.step(&mut bus).unwrap();
    assert_illegal_csr(&core, 0x301, CODE_BASE, MT_VEC);
}

#[test]
fn mhartid_reads_zero_and_rejects_writes() {
    let code = [csr_type(5, 2, 0, 0xF14)];
    let (mut core, mut bus) = core_with(&code, MT_VEC);
    core.step(&mut bus).unwrap();
    assert_eq!(core.read_register(5), 0);
    assert_eq!(core.csr.mcause, 0);

    let code = [csr_type(0, 1, 0, 0xF14)];
    let (mut core, mut bus) = core_with(&code, MT_VEC);
    core.step(&mut bus).unwrap();
    assert_illegal_csr(&core, 0xF14, CODE_BASE, MT_VEC);
}

#[test]
fn identification_csrs_are_read_only() {
    // mvendorid/marchid/mimpid read zero and raise cause 2 on write.
    // mvendorid/marchid/mimpid 读为 0，写入触发 cause 2。
    for csr in [0xF11u32, 0xF12, 0xF13] {
        let code = [csr_type(5, 2, 0, csr)];
        let (mut core, mut bus) = core_with(&code, MT_VEC);
        core.step(&mut bus).unwrap();
        assert_eq!(core.read_register(5), 0, "csr {csr:#05X} reads zero");
        assert_eq!(core.csr.mcause, 0);

        let code = [csr_type(0, 1, 0, csr)];
        let (mut core, mut bus) = core_with(&code, MT_VEC);
        core.step(&mut bus).unwrap();
        assert_illegal_csr(&core, csr, CODE_BASE, MT_VEC);
    }
}

#[test]
fn counters_advance_with_the_time_base() {
    // mcycle/minstret are driven by the core's per-instruction tick; firmware
    // polling them must see monotonic progress.
    // mcycle/minstret 由核心每条指令的 tick 推进；固件轮询时必须看到单调增长。
    let code = [
        csr_type(5, 2, 0, 0xB02),
        csr_type(6, 2, 0, 0xB02),
        csr_type(7, 2, 0, 0xB00),
    ];
    let (mut core, mut bus) = core_with(&code, MT_VEC);
    for _ in 0..code.len() {
        core.step(&mut bus).unwrap();
    }
    assert_eq!(core.csr.mcause, 0);
    assert!(
        core.read_register(6) > core.read_register(5),
        "minstret advances"
    );
    assert!(core.read_register(7) >= 2, "mcycle advances");
    assert_eq!(core.csr_read(0xB82), 0, "high half stays zero on RV32");
}

#[test]
fn pmp_registers_round_trip() {
    // RV32 defines pmpcfg0/2/4/6 and pmpaddr0..15 as writable storage.
    // RV32 将 pmpcfg0/2/4/6 与 pmpaddr0..15 定义为可写存储。
    let code = [
        u_type(1, 0x1234_5000),
        csr_type(0, 1, 1, 0x3A2), // pmpcfg2
        csr_type(0, 1, 1, 0x3BE), // pmpaddr3 (0x3B0 + 0x0C)
        csr_type(2, 2, 0, 0x3A2),
        csr_type(3, 2, 0, 0x3BE),
    ];
    let (mut core, mut bus) = core_with(&code, MT_VEC);
    for _ in 0..code.len() {
        core.step(&mut bus).unwrap();
    }
    assert_eq!(core.csr.mcause, 0);
    assert_eq!(core.read_register(2), 0x1234_5000);
    assert_eq!(core.read_register(3), 0x1234_5000);
}

#[test]
fn mstatush_is_writable_storage() {
    // `mstatus`'s high half is not interpreted, but a write must not fault.
    // `mstatus` 的高半部分不做解释，但写入不得触发陷阱。
    let code = [
        u_type(1, 0x0020_0000),
        csr_type(0, 1, 1, 0x310),
        csr_type(2, 2, 0, 0x310),
    ];
    let (mut core, mut bus) = core_with(&code, MT_VEC);
    for _ in 0..code.len() {
        core.step(&mut bus).unwrap();
    }
    assert_eq!(core.csr.mcause, 0);
    assert_eq!(core.csr.mstatush, 0x0020_0000);
    assert_eq!(core.read_register(2), 0x0020_0000);
}

#[test]
fn illegal_csr_uses_mtvec_base_even_in_vector_mode() {
    // Hazard3 only vectors asynchronous interrupts, so a synchronous cause-2
    // must land on the `mtvec` base. / Hazard3 只对异步中断做向量化，因此同步的
    // cause 2 必须落在 `mtvec` 基址。
    let code = [csr_type(5, 2, 0, UNIMPLEMENTED_CSR)];
    let (mut core, mut bus) = core_with(&code, MT_VEC | 1);
    core.step(&mut bus).unwrap();
    assert_illegal_csr(&core, UNIMPLEMENTED_CSR, CODE_BASE, MT_VEC);
}

#[test]
fn reserved_encoding_space_traps_on_read_and_write() {
    // 0xC03 sits in the read-only space but is not `cycle`/`instret`: it must be
    // treated as unimplemented, not silently readable.
    // 0xC03 位于只读空间但不是 `cycle`/`instret`：必须按未实现处理，不得静默可读。
    let code = [csr_type(5, 2, 0, 0xC03)];
    let (mut core, mut bus) = core_with(&code, MT_VEC);
    core.step(&mut bus).unwrap();
    assert_illegal_csr(&core, 0xC03, CODE_BASE, MT_VEC);

    let code = [csr_type(0, 1, 0, 0xC03)];
    let (mut core, mut bus) = core_with(&code, MT_VEC);
    core.step(&mut bus).unwrap();
    assert_illegal_csr(&core, 0xC03, CODE_BASE, MT_VEC);
}
