//! GPIOC (CP0) and DCP (CP4) coprocessor tests
//! GPIOC（CP0）与 DCP（CP4）协处理器测试

use crate::execute_thumb16::StepResult;
use crate::execute_thumb32::test_support::{TestBus, run};
use crate::registers::CortexM33Registers;

/// SIO base address used by the GPIO coprocessor / GPIO 协处理器使用的 SIO 基地址
const SIO_BASE: u32 = 0xD000_0000;

/// GPIO input register offset / GPIO 输入寄存器偏移
const GPIO_IN: u32 = 0x004;
/// GPIO output register offset / GPIO 输出寄存器偏移
const GPIO_OUT: u32 = 0x010;
/// GPIO high output register offset / GPIO 高输出寄存器偏移
const GPIO_HI_OUT: u32 = 0x014;
/// GPIO output set alias offset / GPIO 输出置位别名偏移
const GPIO_OUT_SET: u32 = 0x018;
/// GPIO high output set alias offset / GPIO 高输出置位别名偏移
const GPIO_HI_OUT_SET: u32 = 0x01C;
/// GPIO output clear alias offset / GPIO 输出清零别名偏移
const GPIO_OUT_CLR: u32 = 0x020;
/// GPIO output XOR alias offset / GPIO 输出翻转别名偏移
const GPIO_OUT_XOR: u32 = 0x028;

/// Coprocessor 0: GPIO coprocessor / 协处理器 0：GPIO 协处理器
const CP0: u32 = 0;
/// Coprocessor 4: double-precision coprocessor / 协处理器 4：双精度协处理器
const CP4: u32 = 4;

/// Build an `MCR` encoding with CRn = c0 and opc2 = 0
/// 构造 CRn = c0、opc2 = 0 的 `MCR` 编码
fn mcr(coproc: u32, opc1: u32, rt: u32, crm: u32) -> u32 {
    mcr_full(coproc, opc1, rt, 0, crm, 0)
}

/// Build an `MCR` encoding with explicit CRn and opc2
/// 构造带显式 CRn 与 opc2 的 `MCR` 编码
fn mcr_full(coproc: u32, opc1: u32, rt: u32, crn: u32, crm: u32, opc2: u32) -> u32 {
    0xEE00_0000
        | (opc1 << 21)
        | (crn << 16)
        | (rt << 12)
        | (coproc << 8)
        | (opc2 << 5)
        | (1 << 4)
        | crm
}

/// Build an `MRC` encoding with CRn = c0 and opc2 = 0
/// 构造 CRn = c0、opc2 = 0 的 `MRC` 编码
fn mrc(coproc: u32, opc1: u32, rt: u32, crm: u32) -> u32 {
    mcr_full(coproc, opc1, rt, 0, crm, 0) | (1 << 20)
}

/// Build an `MCRR` encoding / 构造 `MCRR` 编码
fn mcrr(coproc: u32, opc1: u32, rt: u32, rt2: u32, crm: u32) -> u32 {
    0xEC40_0000 | (rt2 << 16) | (rt << 12) | (coproc << 8) | (opc1 << 4) | crm
}

/// Build an `MRRC` encoding / 构造 `MRRC` 编码
fn mrrc(coproc: u32, opc1: u32, rt: u32, rt2: u32, crm: u32) -> u32 {
    mcrr(coproc, opc1, rt, rt2, crm) | (1 << 20)
}

/// Build a CPACR value that grants full access to the listed coprocessors
/// 构造授予所列协处理器完全访问权限的 CPACR 值
fn cpacr_for(coprocessors: &[u8]) -> u32 {
    coprocessors
        .iter()
        .fold(0, |value, &cp| value | (0b11 << (u32::from(cp) * 2)))
}

/// Build a register set with the listed coprocessors enabled
/// 构建设置了所列协处理器的寄存器集
fn regs_with_coprocessors(coprocessors: &[u8]) -> CortexM33Registers {
    let mut regs = CortexM33Registers::new();
    regs.cpacr = cpacr_for(coprocessors);
    regs
}

/// Register set with CP0 enabled / 使能 CP0 的寄存器集
fn regs_with_cp0() -> CortexM33Registers {
    regs_with_coprocessors(&[0])
}

/// Register set with CP4 enabled / 使能 CP4 的寄存器集
fn regs_with_cp4() -> CortexM33Registers {
    regs_with_coprocessors(&[4])
}

#[test]
fn dcp_engaged_flag_read_executes_and_returns_zero() {
    let mut regs = regs_with_cp4();
    regs.write(0, 0xDEAD_BEEF);
    // Exact instruction emitted by pico-sdk:
    // `mrc p4, #0, r0, c0, c0, #1`.
    // pico-sdk 发出的精确指令：`mrc p4, #0, r0, c0, c0, #1`。
    assert_eq!(
        run(0xEE10_0430, &mut regs, &mut TestBus::new()),
        StepResult::Ok
    );
    assert_eq!(regs.read(0), 0);
}

#[test]
fn gpioc_without_cpacr_rewinds_pc_and_raises_usage_fault() {
    let mut regs = CortexM33Registers::new();
    let mut bus = TestBus::new();
    regs.pc = 0x104;
    assert_eq!(run(mcr(CP0, 2, 0, 0), &mut regs, &mut bus), StepResult::Ok);
    assert_eq!(regs.pc, 0x100);
    assert!(regs.pending_usage_fault);
    assert_eq!(
        bus.peek_u32(SIO_BASE + GPIO_OUT_SET),
        0,
        "no SIO write may occur on NOCP"
    );
}

#[test]
fn gpioc_mcr_put_set_clr_xor_write_aliases() {
    let cases = [
        (0, GPIO_OUT),
        (2, GPIO_OUT_SET),
        (3, GPIO_OUT_CLR),
        (1, GPIO_OUT_XOR),
    ];
    for (opc1, offset) in cases {
        let mut regs = regs_with_cp0();
        regs.write(0, 0xA5A5_5A5A);
        let mut bus = TestBus::new();
        assert_eq!(
            run(mcr(CP0, opc1, 0, 0), &mut regs, &mut bus),
            StepResult::Ok
        );
        assert_eq!(
            bus.peek_u32(SIO_BASE + offset),
            0xA5A5_5A5A,
            "opc1 {opc1} must target offset {offset:#05X}"
        );
    }
}

#[test]
fn gpioc_mcr_bit_set_pin_33_targets_hi_out_set() {
    let mut regs = regs_with_cp0();
    regs.write(0, 33);
    let mut bus = TestBus::new();
    assert_eq!(run(mcr(CP0, 6, 0, 0), &mut regs, &mut bus), StepResult::Ok);
    assert_eq!(bus.peek_u32(SIO_BASE + GPIO_HI_OUT_SET), 1 << 1);
}

#[test]
fn gpioc_mcrr_put_writes_both_output_registers() {
    let mut regs = regs_with_cp0();
    regs.write(0, 0x1122_3344);
    regs.write(1, 0x5566_7788);
    let mut bus = TestBus::new();
    assert_eq!(
        run(mcrr(CP0, 0, 0, 1, 0), &mut regs, &mut bus),
        StepResult::Ok
    );
    assert_eq!(bus.peek_u32(SIO_BASE + GPIO_OUT), 0x1122_3344);
    assert_eq!(bus.peek_u32(SIO_BASE + GPIO_HI_OUT), 0x5566_7788);
}

#[test]
fn gpioc_mcrr_indexed_set_selects_high_register() {
    let mut regs = regs_with_cp0();
    regs.write(0, 0x8000_0000);
    regs.write(1, 1);
    let mut bus = TestBus::new();
    // opc1 = 10 is an indexed set; Rt2 = 1 selects GPIO_HI_OUT_SET.
    // opc1 = 10 为索引置位；Rt2 = 1 选择 GPIO_HI_OUT_SET。
    assert_eq!(
        run(mcrr(CP0, 10, 0, 1, 0), &mut regs, &mut bus),
        StepResult::Ok
    );
    assert_eq!(bus.peek_u32(SIO_BASE + GPIO_HI_OUT_SET), 0x8000_0000);
}

#[test]
fn gpioc_mrc_reads_gpio_in() {
    let mut regs = regs_with_cp0();
    let mut bus = TestBus::new();
    bus.poke_u32(SIO_BASE + GPIO_IN, 0xCAFE_F00D);
    assert_eq!(run(mrc(CP0, 0, 0, 8), &mut regs, &mut bus), StepResult::Ok);
    assert_eq!(regs.read(0), 0xCAFE_F00D);
}

#[test]
fn gpioc_mrrc_reads_output_pair() {
    let mut regs = regs_with_cp0();
    let mut bus = TestBus::new();
    bus.poke_u32(SIO_BASE + GPIO_OUT, 0x1111_2222);
    bus.poke_u32(SIO_BASE + GPIO_HI_OUT, 0x3333_4444);
    assert_eq!(
        run(mrrc(CP0, 0, 0, 1, 0), &mut regs, &mut bus),
        StepResult::Ok
    );
    assert_eq!(regs.read(0), 0x1111_2222);
    assert_eq!(regs.read(1), 0x3333_4444);
}

#[test]
fn gpioc_mrc_with_nonzero_opc1_is_undefined() {
    let mut regs = regs_with_cp0();
    let mut bus = TestBus::new();
    regs.write(0, 0xDEAD_BEEF);
    bus.poke_u32(SIO_BASE + GPIO_OUT, 0xCAFE_F00D);
    // GPIOC reads require opc1 = 0. / GPIOC 读取要求 opc1 = 0。
    assert_eq!(
        run(mrc(CP0, 1, 0, 0), &mut regs, &mut bus),
        StepResult::Undefined
    );
    assert_eq!(regs.read(0), 0xDEAD_BEEF, "undefined MRC must not write Rt");
    // The legal opc1 = 0 encoding still reads GPIO_OUT.
    // 合法的 opc1 = 0 编码仍能读取 GPIO_OUT。
    assert_eq!(run(mrc(CP0, 0, 0, 0), &mut regs, &mut bus), StepResult::Ok);
    assert_eq!(regs.read(0), 0xCAFE_F00D);
}

#[test]
fn gpioc_mrrc_with_nonzero_opc1_is_undefined() {
    let mut regs = regs_with_cp0();
    let mut bus = TestBus::new();
    regs.write(0, 0xDEAD_BEEF);
    regs.write(1, 0xBAAD_F00D);
    bus.poke_u32(SIO_BASE + GPIO_OUT, 0x1111_2222);
    bus.poke_u32(SIO_BASE + GPIO_HI_OUT, 0x3333_4444);
    // GPIOC MRRC requires opc1 = 0. / GPIOC MRRC 要求 opc1 = 0。
    assert_eq!(
        run(mrrc(CP0, 1, 0, 1, 0), &mut regs, &mut bus),
        StepResult::Undefined
    );
    assert_eq!(
        regs.read(0),
        0xDEAD_BEEF,
        "undefined MRRC must not write Rt"
    );
    assert_eq!(
        regs.read(1),
        0xBAAD_F00D,
        "undefined MRRC must not write Rt2"
    );
    // The legal opc1 = 0 encoding still reads the output pair.
    // 合法的 opc1 = 0 编码仍能读取输出寄存器对。
    assert_eq!(
        run(mrrc(CP0, 0, 0, 1, 0), &mut regs, &mut bus),
        StepResult::Ok
    );
    assert_eq!(regs.read(0), 0x1111_2222);
    assert_eq!(regs.read(1), 0x3333_4444);
}

#[test]
fn dcp_mcr_and_mrc_are_side_effect_free() {
    let mut regs = regs_with_cp4();
    let mut bus = TestBus::new();
    regs.write(0, 0xDEAD_BEEF);
    assert_eq!(
        run(mcr_full(CP4, 1, 0, 2, 10, 3), &mut regs, &mut bus),
        StepResult::Ok
    );
    assert_eq!(regs.read(0), 0xDEAD_BEEF);
    // The DCP stub remains lenient about opc1.
    // DCP 桩对 opc1 保持宽松。
    assert_eq!(run(mrc(CP4, 3, 0, 10), &mut regs, &mut bus), StepResult::Ok);
    assert_eq!(regs.read(0), 0);
}

#[test]
fn dcp_mcrr_and_mrrc_are_side_effect_free() {
    let mut regs = regs_with_cp4();
    let mut bus = TestBus::new();
    regs.write(0, 0x1111_1111);
    regs.write(1, 0x2222_2222);
    assert_eq!(
        run(mcrr(CP4, 1, 0, 1, 0), &mut regs, &mut bus),
        StepResult::Ok
    );
    assert_eq!(regs.read(0), 0x1111_1111);
    assert_eq!(regs.read(1), 0x2222_2222);
    // The DCP stub remains lenient about opc1.
    // DCP 桩对 opc1 保持宽松。
    assert_eq!(
        run(mrrc(CP4, 2, 0, 1, 8), &mut regs, &mut bus),
        StepResult::Ok
    );
    assert_eq!(regs.read(0), 0);
    assert_eq!(regs.read(1), 0);
}

#[test]
fn gpioc_undefined_encodings_are_rejected() {
    let mut regs = regs_with_cp0();
    let mut bus = TestBus::new();
    // opc1 = 4 is not defined for MCR. / MCR 的 opc1 = 4 未定义。
    assert_eq!(
        run(mcr(CP0, 4, 0, 0), &mut regs, &mut bus),
        StepResult::Undefined
    );
    // CRn must be c0. / CRn 必须为 c0。
    assert_eq!(
        run(mcr_full(CP0, 0, 0, 1, 0, 0), &mut regs, &mut bus),
        StepResult::Undefined
    );
    // CRm = 2 is not a readable GPIOC register. / CRm = 2 不是可读的 GPIOC 寄存器。
    assert_eq!(
        run(mrc(CP0, 0, 0, 2), &mut regs, &mut bus),
        StepResult::Undefined
    );
    // Rt = 15 is architecturally undefined. / Rt = 15 在体系结构上未定义。
    assert_eq!(
        run(mcr(CP0, 0, 15, 0), &mut regs, &mut bus),
        StepResult::Undefined
    );
    // opc1 = 12 is outside the MCRR GPIOC operation range.
    // opc1 = 12 超出 MCRR GPIOC 操作范围。
    assert_eq!(
        run(mcrr(CP0, 12, 0, 1, 0), &mut regs, &mut bus),
        StepResult::Undefined
    );
}
