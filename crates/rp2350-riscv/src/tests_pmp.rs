//! PMP enforcement tests / PMP 强制检查测试
//!
//! These tests pin the privileged-specification rules: the lowest-numbered
//! matching entry decides, locked entries also restrict M-mode, unlocked
//! entries are transparent to M-mode, and `mstatus.MPRV` redirects data
//! accesses through `MPP` while instruction fetches stay at the current
//! privilege. Violations raise cause 1/5/7 with the faulting address in
//! `mtval` and the instruction PC in `mepc`.
//! 本模块锁定特权规范规则：最低编号的匹配条目起决定作用；锁定条目同样限制 M 模式；
//! 未锁定条目对 M 模式透明；`mstatus.MPRV` 让数据访问按 `MPP` 检查，而取指保持当前
//! 特权级。违规分别触发 cause 1/5/7，`mtval` 为出错地址、`mepc` 为指令 PC。

#![allow(clippy::similar_names)]

use super::*;
use rp2350_core::memory::{RamRegion, SimpleBus};
use rp2350_cpu::CpuCore;

const CODE_BASE: u32 = 0x2000_0000;
const STACK_TOP: u32 = 0x2000_0F00;
const MT_VEC: u32 = 0x2000_0100;
const DATA: u32 = 0x2000_0800;
const DATA_VALUE: u32 = 0xCAFE_F00D;

/// `R` permission / `R` 权限
const R: u32 = 0b001;
/// `W` permission / `W` 权限
const W: u32 = 0b010;
/// `X` permission / `X` 权限
const X: u32 = 0b100;
/// `A = TOR` / `A = TOR`
const TOR: u32 = 0b01 << 3;
/// `A = NA4` / `A = NA4`
const NA4: u32 = 0b10 << 3;
/// `A = NAPOT` / `A = NAPOT`
const NAPOT: u32 = 0b11 << 3;
/// `L` lock bit / `L` 锁定位
const L: u32 = 1 << 7;

/// Encode `addi rd, rs1, imm` / 编码 `addi rd, rs1, imm`
fn addi(rd: u32, rs1: u32, immediate: i32) -> u32 {
    0x13 | (rd << 7) | (rs1 << 15) | ((immediate as u32 & 0xFFF) << 20)
}

/// Encode `lw rd, offset(rs1)` / 编码 `lw rd, offset(rs1)`
fn lw(rd: u32, rs1: u32, offset: i32) -> u32 {
    0x03 | (rd << 7) | (2 << 12) | (rs1 << 15) | ((offset as u32 & 0xFFF) << 20)
}

/// Encode `sw rs2, offset(rs1)` / 编码 `sw rs2, offset(rs1)`
fn sw(rs2: u32, rs1: u32, offset: i32) -> u32 {
    let offset = offset as u32;
    0x23 | ((offset & 0x1F) << 7)
        | (2 << 12)
        | (rs1 << 15)
        | (rs2 << 20)
        | (((offset >> 5) & 0x7F) << 25)
}

/// Encode a SYSTEM/CSR instruction / 编码 SYSTEM/CSR 指令
fn csr_type(rd: u32, funct3: u32, rs1: u32, csr: u32) -> u32 {
    0x73 | (rd << 7) | (funct3 << 12) | (rs1 << 15) | (csr << 20)
}

/// Encode `lui rd, imm` / 编码 `lui rd, imm`
fn u_type(rd: u32, immediate: u32) -> u32 {
    0x37 | (rd << 7) | (immediate & 0xFFFF_F000)
}

/// Return the NAPOT `pmpaddr` encoding for a `2^log2_size`-byte region
/// 返回覆盖 `2^log2_size` 字节区间的 NAPOT `pmpaddr` 编码
fn napot_addr(base: u32, log2_size: u32) -> u32 {
    (base >> 2) | ((1 << (log2_size - 3)) - 1)
}

/// Build a core executing `code`, with `DATA` holding a known word
/// 构建执行 `code` 且 `DATA` 处存放已知字的核心
fn core_with(code: &[u32]) -> (Hazard3Core, SimpleBus) {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("sram", CODE_BASE, 0x1000)))
        .unwrap();
    let mut cycles = 0u64;
    for (index, word) in code.iter().enumerate() {
        let address = CODE_BASE + u32::try_from(index).unwrap() * 4;
        bus.write_u32(address, *word, &mut cycles).unwrap();
    }
    // Fill the words used by the boundary tests so a successful load has a
    // recognizable value. / 填充边界测试用到的字，使成功的载入有可辨识的值。
    for offset in [0u32, 4, 0xFC] {
        bus.write_u32(DATA + offset, DATA_VALUE, &mut cycles)
            .unwrap();
    }
    let mut core = Hazard3Core::new(CpuId::Core0);
    core.set_boot_vectors(STACK_TOP, CODE_BASE);
    core.init(&mut bus).unwrap();
    core.csr.mtvec = MT_VEC;
    (core, bus)
}

/// Make data accesses use `MPP` as effective privilege; `MPP` stays 0 (U)
/// 让数据访问按 `MPP` 的有效特权级检查；`MPP` 保持 0（U）
fn enter_user_data_mode(core: &mut Hazard3Core) {
    core.csr.mstatus |= csr::MSTATUS_MPRV;
}

#[test]
fn pmp_disabled_entries_allow_machine_access() {
    // All entries are OFF after reset: M-mode load succeeds.
    // 复位后所有条目为 OFF：M 模式载入成功。
    let (mut core, mut bus) = core_with(&[lw(10, 11, 0)]);
    core.write_register(11, DATA);
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.mcause, 0);
    assert_eq!(core.read_register(10), DATA_VALUE);
}

#[test]
fn pmp_locked_napot_denies_unpermitted_load() {
    // Locked region without R: cause 5, mtval = address, mepc = instruction PC.
    // 无 R 的锁定区间：cause 5，mtval 为地址，mepc 为指令 PC。
    let (mut core, mut bus) = core_with(&[lw(10, 11, 0)]);
    core.write_register(11, DATA);
    core.csr.pmpaddr[0] = napot_addr(DATA, 8);
    core.csr.pmpcfg[0] = L | NAPOT;
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.mcause, 5);
    assert_eq!(core.csr.mtval, DATA);
    assert_eq!(core.csr.mepc, CODE_BASE);
    assert_eq!(core.pc(), MT_VEC, "trap must reach mtvec");
    assert_eq!(core.read_register(10), 0, "denied load must not write rd");
}

#[test]
fn pmp_locked_region_allows_permitted_load_and_store() {
    // R|W on a locked region: both accesses succeed.
    // 锁定区间具备 R|W：载入与存储都成功。
    let (mut core, mut bus) = core_with(&[lw(10, 11, 0), sw(10, 11, 4)]);
    core.write_register(11, DATA);
    core.csr.pmpaddr[0] = napot_addr(DATA, 8);
    core.csr.pmpcfg[0] = L | NAPOT | R | W;
    core.step(&mut bus).unwrap();
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.mcause, 0);
    assert_eq!(core.read_register(10), DATA_VALUE);
    let mut cycles = 0u64;
    assert_eq!(bus.read_u32(DATA + 4, &mut cycles).unwrap(), DATA_VALUE);
}

#[test]
fn pmp_locked_region_denies_store_without_w() {
    // R without W: the store raises cause 7 and never reaches the bus.
    // 有 R 无 W：存储触发 cause 7，且不会到达总线。
    let (mut core, mut bus) = core_with(&[sw(12, 11, 0)]);
    core.write_register(11, DATA);
    core.write_register(12, 0x1234_5678);
    core.csr.pmpaddr[0] = napot_addr(DATA, 8);
    core.csr.pmpcfg[0] = L | NAPOT | R;
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.mcause, 7);
    assert_eq!(core.csr.mtval, DATA);
    assert_eq!(core.csr.mepc, CODE_BASE);
    assert_eq!(core.pc(), MT_VEC);
    let mut cycles = 0u64;
    assert_eq!(bus.read_u32(DATA, &mut cycles).unwrap(), DATA_VALUE);
}

#[test]
fn pmp_na4_fetch_denial_raises_instruction_access_fault() {
    // Executable permission is required by fetch; cause 1 carries the PC.
    // 取指需要 X 权限；cause 1 携带出错 PC。
    let (mut core, mut bus) = core_with(&[lw(10, 11, 0)]);
    core.csr.pmpaddr[0] = CODE_BASE >> 2;
    core.csr.pmpcfg[0] = L | NA4 | R | W;
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.mcause, 1);
    assert_eq!(core.csr.mtval, CODE_BASE);
    assert_eq!(core.csr.mepc, CODE_BASE);
    assert_eq!(core.pc(), MT_VEC);
}

#[test]
fn pmp_na4_execute_permission_allows_fetch() {
    // The same NA4 region with X set lets the instruction retire normally.
    // 同一 NA4 区间置 X 后允许指令正常退休。
    let (mut core, mut bus) = core_with(&[u_type(2, 0)]);
    core.csr.pmpaddr[0] = CODE_BASE >> 2;
    core.csr.pmpcfg[0] = L | NA4 | X;
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.mcause, 0);
    assert_eq!(core.pc(), CODE_BASE + 4);
}

#[test]
fn pmp_na4_covers_exactly_four_bytes() {
    // NA4 matches only [DATA, DATA+4); DATA+4 has no entry, so U-mode denies.
    // NA4 只匹配 [DATA, DATA+4)；DATA+4 无条目，U 模式因此拒绝。
    let (mut core, mut bus) = core_with(&[lw(10, 11, 0), lw(12, 11, 4)]);
    core.write_register(11, DATA);
    core.csr.pmpaddr[0] = DATA >> 2;
    core.csr.pmpcfg[0] = L | NA4 | R;
    enter_user_data_mode(&mut core);
    core.step(&mut bus).unwrap();
    assert_eq!(core.read_register(10), DATA_VALUE);
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.mcause, 5);
    assert_eq!(core.csr.mtval, DATA + 4);
    assert_eq!(core.csr.mepc, CODE_BASE + 4);
    assert_eq!(core.read_register(12), 0);
}

#[test]
fn pmp_tor_covers_the_half_open_range() {
    // TOR region [DATA, DATA+0x100): the lower bound is inclusive, the upper
    // exclusive. Entry 0 stays OFF so entry 1 provides the bound.
    // TOR 区间 [DATA, DATA+0x100)：下界包含、上界不包含。条目 0 保持 OFF，
    // 由条目 1 提供界限。
    let (mut core, mut bus) = core_with(&[lw(10, 11, 0), lw(12, 11, 0xFC), lw(13, 11, 0x100)]);
    core.write_register(11, DATA);
    core.csr.pmpaddr[0] = DATA >> 2;
    core.csr.pmpaddr[1] = (DATA + 0x100) >> 2;
    core.csr.pmpcfg[0] = (L | TOR | R) << 8;
    enter_user_data_mode(&mut core);
    core.step(&mut bus).unwrap();
    core.step(&mut bus).unwrap();
    assert_eq!(core.read_register(10), DATA_VALUE);
    assert_eq!(core.read_register(12), DATA_VALUE);
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.mcause, 5);
    assert_eq!(core.csr.mtval, DATA + 0x100);
    assert_eq!(core.csr.mepc, CODE_BASE + 8);
}

#[test]
fn pmp_napot_spans_the_encoded_power_of_two_range() {
    // 256-byte NAPOT region: last word inside succeeds, first word outside fails.
    // 256 字节 NAPOT 区间：区间内最后一个字成功，区间外第一个字失败。
    let (mut core, mut bus) = core_with(&[lw(10, 11, 0), lw(12, 11, 0xFC), lw(13, 11, 0x100)]);
    core.write_register(11, DATA);
    core.csr.pmpaddr[0] = napot_addr(DATA, 8);
    core.csr.pmpcfg[0] = L | NAPOT | R;
    enter_user_data_mode(&mut core);
    core.step(&mut bus).unwrap();
    core.step(&mut bus).unwrap();
    assert_eq!(core.read_register(10), DATA_VALUE);
    assert_eq!(core.read_register(12), DATA_VALUE);
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.mcause, 5);
    assert_eq!(core.csr.mtval, DATA + 0x100);
}

#[test]
fn pmp_napot_eight_byte_minimum_is_decoded() {
    // NAPOT with no trailing ones encodes the architectural minimum: 8 bytes.
    // 无尾部 1 的 NAPOT 编码架构最小尺寸：8 字节。
    let (mut core, mut bus) = core_with(&[lw(10, 11, 4), lw(12, 11, 8)]);
    core.write_register(11, DATA);
    core.csr.pmpaddr[0] = DATA >> 2;
    core.csr.pmpcfg[0] = L | NAPOT | R;
    enter_user_data_mode(&mut core);
    core.step(&mut bus).unwrap();
    assert_eq!(core.read_register(10), DATA_VALUE);
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.mcause, 5);
    assert_eq!(core.csr.mtval, DATA + 8);
}

#[test]
fn pmp_locked_entry_ignores_csr_writes() {
    // Once L is set, descriptor and address writes are ignored without a trap.
    // L 置位后，描述符与地址写入被忽略且不触发陷阱。
    let code = [
        addi(11, 0, 0x9F),
        csr_type(0, 1, 11, 0x3B0), // pmpaddr0 = 0x9F
        csr_type(0, 1, 11, 0x3A0), // pmpcfg0 = 0x9F (lock entry 0)
        csr_type(0, 1, 0, 0x3A0),  // try to clear pmpcfg0
        csr_type(0, 1, 0, 0x3B0),  // try to clear pmpaddr0
        csr_type(10, 2, 0, 0x3A0), // read back pmpcfg0
        csr_type(12, 2, 0, 0x3B0), // read back pmpaddr0
    ];
    let (mut core, mut bus) = core_with(&code);
    for _ in 0..code.len() {
        core.step(&mut bus).unwrap();
    }
    assert_eq!(core.csr.mcause, 0);
    assert_eq!(core.read_register(10), 0x9F);
    assert_eq!(core.read_register(12), 0x9F);
    assert_eq!(core.csr.pmpcfg[0], 0x9F);
    assert_eq!(core.csr.pmpaddr[0], 0x9F);
}

#[test]
fn pmp_locked_tor_entry_protects_its_lower_bound() {
    // A locked TOR entry also freezes pmpaddr[i-1].
    // 锁定的 TOR 条目同时冻结 pmpaddr[i-1]。
    let code = [
        addi(11, 0, 0x123),
        csr_type(0, 1, 11, 0x3B0), // pmpaddr0 = 0x123
        addi(11, 0, 0x456),
        csr_type(0, 1, 11, 0x3B4), // pmpaddr1 = 0x456
        u_type(11, 0x9000),
        addi(11, 11, -0x800), // 0x8800: entry 1 = L | TOR
        csr_type(0, 1, 11, 0x3A0),
        csr_type(0, 1, 0, 0x3B0), // try to clear pmpaddr0
        csr_type(0, 1, 0, 0x3B4), // try to clear pmpaddr1
        csr_type(10, 2, 0, 0x3B0),
        csr_type(12, 2, 0, 0x3B4),
    ];
    let (mut core, mut bus) = core_with(&code);
    for _ in 0..code.len() {
        core.step(&mut bus).unwrap();
    }
    assert_eq!(core.csr.mcause, 0);
    assert_eq!(core.read_register(10), 0x123);
    assert_eq!(core.read_register(12), 0x456);
    assert_eq!(core.csr.pmpaddr[0], 0x123);
    assert_eq!(core.csr.pmpaddr[1], 0x456);
}

#[test]
fn pmp_mprv_checks_data_but_fetch_stays_machine() {
    // MPRV with MPP=U: the fetch executes in M-mode, the load has no matching
    // entry and is denied. / MPRV 且 MPP=U：取指以 M 模式执行，载入无匹配条目被拒。
    let (mut core, mut bus) = core_with(&[lw(10, 11, 0)]);
    core.write_register(11, DATA);
    enter_user_data_mode(&mut core);
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.mcause, 5);
    assert_eq!(core.csr.mtval, DATA);
    assert_eq!(core.csr.mepc, CODE_BASE);
    assert_eq!(core.pc(), MT_VEC);
}

#[test]
fn pmp_mprv_with_machine_mpp_keeps_data_access() {
    // MPRV only redirects when MPP is not M; with MPP=M the access is M-mode.
    // MPRV 只在 MPP 非 M 时重定向；MPP=M 时访问仍按 M 模式处理。
    let (mut core, mut bus) = core_with(&[lw(10, 11, 0)]);
    core.write_register(11, DATA);
    core.csr.mstatus |= csr::MSTATUS_MPRV | csr::MSTATUS_MPP;
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.mcause, 0);
    assert_eq!(core.read_register(10), DATA_VALUE);
}

#[test]
fn pmp_unlocked_entry_is_transparent_to_machine_mode() {
    // Privileged spec 3.7.1.2: an unlocked match succeeds in M-mode, while the
    // same entry still governs U-mode permissions.
    // 特权规范 3.7.1.2：未锁定条目在 M 模式匹配即成功，但同一 entry 仍约束 U 模式。
    let (mut core, mut bus) = core_with(&[lw(10, 11, 0)]);
    core.write_register(11, DATA);
    core.csr.pmpaddr[0] = napot_addr(DATA, 8);
    core.csr.pmpcfg[0] = NAPOT; // no L, no R
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.mcause, 0, "M-mode must not be restricted");
    assert_eq!(core.read_register(10), DATA_VALUE);

    let (mut core, mut bus) = core_with(&[lw(10, 11, 0)]);
    core.write_register(11, DATA);
    core.csr.pmpaddr[0] = napot_addr(DATA, 8);
    core.csr.pmpcfg[0] = NAPOT;
    enter_user_data_mode(&mut core);
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.mcause, 5, "U-mode needs the R bit");
}

#[test]
fn pmp_violation_mepc_points_at_the_faulting_instruction() {
    // A second-instruction violation must return exactly to that instruction.
    // 第二条指令违规时，mepc 必须精确指向该指令。
    let (mut core, mut bus) = core_with(&[addi(10, 0, 1), lw(12, 11, 0)]);
    core.write_register(11, DATA);
    core.csr.pmpaddr[0] = napot_addr(DATA, 8);
    core.csr.pmpcfg[0] = L | NAPOT;
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.mcause, 0);
    core.step(&mut bus).unwrap();
    assert_eq!(core.csr.mcause, 5);
    assert_eq!(core.csr.mtval, DATA);
    assert_eq!(core.csr.mepc, CODE_BASE + 4);
}
