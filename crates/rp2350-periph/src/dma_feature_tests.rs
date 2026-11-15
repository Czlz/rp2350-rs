//! DMA modes, BSWAP, sniff, priority and pacing tests
//! DMA 传输模式、BSWAP、嗅探、优先级与节拍测试
//!
//! Split from `dma_tests` to keep each file's code size in check. The parent
//! module supplies the bus fixture, the RAM byte helpers and `configure`.
//! 从 `dma_tests` 拆出以控制单文件代码量；父模块提供总线夹具、RAM 字节辅助
//! 函数与 `configure`。

use super::*;

use crate::dma::{
    CTRL_BSWAP, CTRL_CHAIN_TO_SHIFT, CTRL_HIGH_PRIORITY, CTRL_SNIFF_EN, DREQ_TIMER0, SNIFF_BSWAP,
    SNIFF_CALC_CRC16, SNIFF_CALC_CRC32, SNIFF_CALC_SHIFT, SNIFF_CALC_SUM, SNIFF_CALC_XOR,
    SNIFF_DMACH_SHIFT, SNIFF_EN, SNIFF_OUT_INV, SNIFF_OUT_REV, TRANS_COUNT_MODE_ENDLESS,
    TRANS_COUNT_MODE_SHIFT, TRANS_COUNT_MODE_TRIGGER_SELF, TREQ_PERMANENT,
};

/// `TIMER0` offset / `TIMER0` 偏移
const TIMER0: u32 = 0x440;

/// `SNIFF_CTRL` offset / `SNIFF_CTRL` 偏移
const SNIFF_CTRL: u32 = 0x454;

/// `SNIFF_DATA` offset / `SNIFF_DATA` 偏移
const SNIFF_DATA: u32 = 0x458;

/// `TIMER0` value with `X = 1, Y = 4` / `X = 1, Y = 4` 的 `TIMER0` 值
const TIMER_X1_Y4: u32 = 0x0001_0004;

/// `TIMER0` value with `X = 1, Y = 1` / `X = 1, Y = 1` 的 `TIMER0` 值
const TIMER_X1_Y1: u32 = 0x0001_0001;

// ---------------------------------------------------------------------------
// Transfer modes, BSWAP, sniffing, priority and pacing / 传输模式、BSWAP、嗅探、优先级与节拍
// ---------------------------------------------------------------------------

/// Add a `TRANS_COUNT` mode to a control word / 给控制字加上 `TRANS_COUNT` 模式
fn with_mode(ctrl: u32, mode: u8) -> u32 {
    ctrl | (u32::from(mode) << TRANS_COUNT_MODE_SHIFT)
}

/// Build a `SNIFF_CTRL` value enabling channel `channel` with calc mode `calc`
/// 构造为通道 `channel` 启用嗅探并选择 `calc` 模式的 `SNIFF_CTRL` 值
fn sniff_for(calc: u32, channel: u32) -> u32 {
    SNIFF_EN | (channel << SNIFF_DMACH_SHIFT) | (calc << SNIFF_CALC_SHIFT)
}

#[test]
fn trigger_self_reloads_chains_and_raises_irq() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[1; 16]);
    // ch0：计数 2、自触发、链到 ch1（计数 1）。
    // ch0: count 2, self-triggering, chained to ch1 (count 1).
    let chained = ctrl_word(0, 0, TREQ_PERMANENT) | (1 << CTRL_CHAIN_TO_SHIFT);
    configure(
        &mut f,
        0,
        SRC,
        DST,
        with_mode(2, TRANS_COUNT_MODE_TRIGGER_SELF),
        chained,
    );
    configure(
        &mut f,
        1,
        SRC + 4,
        DST + 4,
        1,
        ctrl_word(1, 0, TREQ_PERMANENT),
    );
    assert_eq!(
        f.channel(0).trans_count_mode,
        TRANS_COUNT_MODE_TRIGGER_SELF,
        "the mode is latched with the reload count"
    );
    // 3 个单元：ch0 一个、ch1 完成、ch0 完成（置中断 + 链 + 自触发）。
    // Three units: ch0 once, ch1 completes, ch0 completes (IRQ + chain + retrigger).
    assert_eq!(f.run(3), 3);
    let ch0 = f.channel(0);
    assert_eq!(ch0.trans_count, 2, "the reload count is restored");
    assert!(ch0.busy, "the channel keeps running");
    assert_eq!(f.reg_read(INTR) & 0x1, 0x1, "completion IRQ still fires");
    assert!(f.channel(1).busy, "CHAIN_TO was started");
    assert_eq!(f.channel(1).trans_count, 1);
    // 环继续：再跑三个单元仍由 ch0 完成并自触发。
    // The ring continues: three more units complete ch0 again and retrigger it.
    assert_eq!(f.run(3), 3);
    assert_eq!(f.channel(0).trans_count, 2);
    assert!(f.channel(0).busy);
}

#[test]
fn endless_mode_freezes_the_count_until_chan_abort() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[7; 16]);
    let ctrl = ctrl_word(0, 0, TREQ_PERMANENT);
    configure(
        &mut f,
        0,
        SRC,
        DST,
        with_mode(3, TRANS_COUNT_MODE_ENDLESS),
        ctrl,
    );
    assert_eq!(f.channel(0).trans_count_mode, TRANS_COUNT_MODE_ENDLESS);
    // 计数 2 倍以上仍不停、不置中断、不链式。More than twice the count keeps running.
    assert_eq!(f.run(8), 8);
    let state = f.channel(0);
    assert_eq!(state.trans_count, 3, "ENDLESS never decrements");
    assert!(state.busy);
    assert_eq!(state.read_addr, SRC + 8, "addresses still advance");
    assert_eq!(f.reg_read(INTR), 0);
    assert_eq!(read_bytes(&mut f, DST + 7, 1), vec![7u8]);
    // CHAN_ABORT 是唯一的停止方式。CHAN_ABORT is the only way to stop it.
    f.reg_write(CHAN_ABORT, 1);
    assert!(!f.channel(0).busy);
    assert_eq!(f.run(4), 0);
}

#[test]
fn bswap_swaps_halfword_bytes() {
    let mut f = fixture();
    f.write_half(SRC, 0x1234);
    f.write_half(SRC + 2, 0xABCD);
    let ctrl = ctrl_word(0, 1, TREQ_PERMANENT) | CTRL_BSWAP;
    configure(&mut f, 0, SRC, DST, 2, ctrl);
    assert_eq!(f.run(2), 2);
    assert_eq!(f.read_half(DST), 0x3412);
    assert_eq!(f.read_half(DST + 2), 0xCDAB);
}

#[test]
fn bswap_reverses_word_bytes() {
    let mut f = fixture();
    f.write_word(SRC, 0x1122_3344);
    let ctrl = ctrl_word(0, DATA_SIZE_WORD, TREQ_PERMANENT) | CTRL_BSWAP;
    configure(&mut f, 0, SRC, DST, 1, ctrl);
    assert_eq!(f.run(1), 1);
    assert_eq!(f.read_word(DST), 0x4433_2211);
}

#[test]
fn bswap_ignores_byte_transfers() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[0xA1, 0xB2]);
    let ctrl = ctrl_word(0, 0, TREQ_PERMANENT) | CTRL_BSWAP;
    configure(&mut f, 0, SRC, DST, 2, ctrl);
    assert_eq!(f.run(2), 2);
    assert_eq!(read_bytes(&mut f, DST, 2), vec![0xA1u8, 0xB2]);
}

#[test]
fn sniff_sum_accumulates_units() {
    let mut f = fixture();
    f.write_word(SRC, 0x1122_3344);
    f.write_word(SRC + 4, 0x0102_0304);
    f.reg_write(SNIFF_CTRL, sniff_for(SNIFF_CALC_SUM, 0));
    let ctrl = ctrl_word(0, DATA_SIZE_WORD, TREQ_PERMANENT) | CTRL_SNIFF_EN;
    configure(&mut f, 0, SRC, DST, 2, ctrl);
    assert_eq!(f.run(2), 2);
    assert_eq!(f.reg_read(SNIFF_DATA), 0x1224_3648);
    // 通道未置 CTRL.SNIFF_EN 时不计入。A channel without CTRL.SNIFF_EN contributes nothing.
    f.reg_write(SNIFF_CTRL, sniff_for(SNIFF_CALC_SUM, 1));
    configure(
        &mut f,
        1,
        SRC,
        DST + 0x100,
        1,
        ctrl_word(1, DATA_SIZE_WORD, TREQ_PERMANENT),
    );
    assert_eq!(f.run(1), 1);
    assert_eq!(f.reg_read(SNIFF_DATA), 0x1224_3648);
}

#[test]
fn sniff_crc32_matches_the_known_vector() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, b"123456789");
    // 种子 0xFFFF_FFFF、CALC=0、OUT_INV=1 → 0xCBF4_3926。
    // Seed 0xFFFF_FFFF, CALC = 0, OUT_INV = 1 yields 0xCBF4_3926.
    f.reg_write(SNIFF_CTRL, sniff_for(SNIFF_CALC_CRC32, 0) | SNIFF_OUT_INV);
    f.reg_write(SNIFF_DATA, 0xFFFF_FFFF);
    let ctrl = ctrl_word(0, 0, TREQ_PERMANENT) | CTRL_SNIFF_EN;
    configure(&mut f, 0, SRC, DST, 9, ctrl);
    assert_eq!(f.run(9), 9);
    assert_eq!(f.reg_read(SNIFF_DATA), 0xCBF4_3926);
}

#[test]
fn sniff_crc16_matches_the_known_vector() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, b"123456789");
    f.reg_write(SNIFF_CTRL, sniff_for(SNIFF_CALC_CRC16, 0));
    f.reg_write(SNIFF_DATA, 0x0000_FFFF);
    let ctrl = ctrl_word(0, 0, TREQ_PERMANENT) | CTRL_SNIFF_EN;
    configure(&mut f, 0, SRC, DST, 9, ctrl);
    assert_eq!(f.run(9), 9);
    // CRC-16/CCITT-FALSE("123456789") = 0x29B1。
    assert_eq!(f.reg_read(SNIFF_DATA), 0x29B1);
}

#[test]
fn sniff_xor_reduces_to_parity() {
    let mut f = fixture();
    // 0x01 有一个 1、0x03 有两个 1 → 折叠奇偶为 1。
    // 0x01 has one set bit and 0x03 has two, so the folded parity is 1.
    write_bytes(&mut f, SRC, &[0x01, 0x03]);
    f.reg_write(SNIFF_CTRL, sniff_for(SNIFF_CALC_XOR, 0));
    let ctrl = ctrl_word(0, 0, TREQ_PERMANENT) | CTRL_SNIFF_EN;
    configure(&mut f, 0, SRC, DST, 2, ctrl);
    assert_eq!(f.run(2), 2);
    assert_eq!(f.reg_read(SNIFF_DATA), 1);
}

#[test]
fn sniff_output_bit_reverse_and_invert() {
    let mut f = fixture();
    f.write_word(SRC, 0x1234_5678);
    f.reg_write(SNIFF_CTRL, sniff_for(SNIFF_CALC_SUM, 0));
    let ctrl = ctrl_word(0, DATA_SIZE_WORD, TREQ_PERMANENT) | CTRL_SNIFF_EN;
    configure(&mut f, 0, SRC, DST, 1, ctrl);
    assert_eq!(f.run(1), 1);
    assert_eq!(f.reg_read(SNIFF_DATA), 0x1234_5678);
    // OUT_REV 是位反转（不是字节反转）。OUT_REV is a bit reverse, not a byte reverse.
    f.reg_write(SNIFF_CTRL, sniff_for(SNIFF_CALC_SUM, 0) | SNIFF_OUT_REV);
    assert_eq!(f.reg_read(SNIFF_DATA), 0x1234_5678u32.reverse_bits());
    // OUT_INV 只影响读出的值。OUT_INV only changes the presented value.
    f.reg_write(SNIFF_CTRL, sniff_for(SNIFF_CALC_SUM, 0) | SNIFF_OUT_INV);
    assert_eq!(f.reg_read(SNIFF_DATA), !0x1234_5678);
    // 两个变换可叠加。The two transforms combine.
    f.reg_write(
        SNIFF_CTRL,
        sniff_for(SNIFF_CALC_SUM, 0) | SNIFF_OUT_REV | SNIFF_OUT_INV,
    );
    assert_eq!(f.reg_read(SNIFF_DATA), !0x1234_5678u32.reverse_bits());
    // 存储值始终不变。The stored accumulator never changes.
    f.reg_write(SNIFF_CTRL, sniff_for(SNIFF_CALC_SUM, 0));
    assert_eq!(f.reg_read(SNIFF_DATA), 0x1234_5678);
}

#[test]
fn sniff_bswap_changes_the_crc() {
    let mut f = fixture();
    f.write_word(SRC, 0x1122_3344);
    let ctrl = ctrl_word(0, DATA_SIZE_WORD, TREQ_PERMANENT) | CTRL_SNIFF_EN;
    // 无 BSWAP。Without BSWAP.
    f.reg_write(SNIFF_CTRL, sniff_for(SNIFF_CALC_CRC32, 0));
    f.reg_write(SNIFF_DATA, 0xFFFF_FFFF);
    configure(&mut f, 0, SRC, DST, 1, ctrl);
    assert_eq!(f.run(1), 1);
    let plain = f.reg_read(SNIFF_DATA);
    // 有 BSWAP：同一数据得到不同 CRC。With BSWAP the same data gives another CRC.
    f.reg_write(SNIFF_CTRL, sniff_for(SNIFF_CALC_CRC32, 0) | SNIFF_BSWAP);
    f.reg_write(SNIFF_DATA, 0xFFFF_FFFF);
    configure(&mut f, 0, SRC, DST, 1, ctrl);
    assert_eq!(f.run(1), 1);
    assert_ne!(f.reg_read(SNIFF_DATA), plain);
}

#[test]
fn sniff_requires_enable_channel_and_dmach() {
    let mut f = fixture();
    f.write_word(SRC, 0x1122_3344);
    let sniffing = ctrl_word(0, DATA_SIZE_WORD, TREQ_PERMANENT) | CTRL_SNIFF_EN;
    // SNIFF_CTRL.EN 未置位。SNIFF_CTRL.EN clear.
    f.reg_write(SNIFF_CTRL, SNIFF_CALC_SUM << SNIFF_CALC_SHIFT);
    configure(&mut f, 0, SRC, DST, 1, sniffing);
    assert_eq!(f.run(1), 1);
    assert_eq!(f.reg_read(SNIFF_DATA), 0);
    // 通道未置 CTRL.SNIFF_EN。Channel CTRL.SNIFF_EN clear.
    f.reg_write(SNIFF_CTRL, sniff_for(SNIFF_CALC_SUM, 0));
    configure(
        &mut f,
        0,
        SRC,
        DST,
        1,
        ctrl_word(0, DATA_SIZE_WORD, TREQ_PERMANENT),
    );
    assert_eq!(f.run(1), 1);
    assert_eq!(f.reg_read(SNIFF_DATA), 0);
    // DMACH 指向别的通道。DMACH selects another channel.
    f.reg_write(SNIFF_CTRL, sniff_for(SNIFF_CALC_SUM, 1));
    configure(&mut f, 0, SRC, DST, 1, sniffing);
    assert_eq!(f.run(1), 1);
    assert_eq!(f.reg_read(SNIFF_DATA), 0);
    // 三个条件齐全才累加。All three conditions accumulate.
    f.reg_write(SNIFF_CTRL, sniff_for(SNIFF_CALC_SUM, 0));
    configure(&mut f, 0, SRC, DST, 1, sniffing);
    assert_eq!(f.run(1), 1);
    assert_eq!(f.reg_read(SNIFF_DATA), 0x1122_3344);
}

#[test]
fn sniff_data_write_sets_the_seed() {
    let mut f = fixture();
    f.reg_write(SNIFF_CTRL, sniff_for(SNIFF_CALC_SUM, 0));
    // 写入即设置种子（不是清零）。A write stores the seed verbatim.
    f.reg_write(SNIFF_DATA, 0xDEAD_BEEF);
    assert_eq!(f.reg_read(SNIFF_DATA), 0xDEAD_BEEF);
    f.reg_write(SNIFF_DATA, 0);
    assert_eq!(f.reg_read(SNIFF_DATA), 0);
    // 不同 CRC 种子得到不同结果。A different CRC seed gives a different result.
    write_bytes(&mut f, SRC, b"123456789");
    let ctrl = ctrl_word(0, 0, TREQ_PERMANENT) | CTRL_SNIFF_EN;
    f.reg_write(SNIFF_CTRL, sniff_for(SNIFF_CALC_CRC32, 0) | SNIFF_OUT_INV);
    f.reg_write(SNIFF_DATA, 0);
    configure(&mut f, 0, SRC, DST, 9, ctrl);
    assert_eq!(f.run(9), 9);
    let zero_seed = f.reg_read(SNIFF_DATA);
    f.reg_write(SNIFF_DATA, 0xFFFF_FFFF);
    configure(&mut f, 0, SRC, DST, 9, ctrl);
    assert_eq!(f.run(9), 9);
    assert_eq!(f.reg_read(SNIFF_DATA), 0xCBF4_3926);
    assert_ne!(zero_seed, 0xCBF4_3926);
}

#[test]
fn high_priority_channels_are_served_first() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[1; 8]);
    write_bytes(&mut f, SRC + 0x100, &[2; 8]);
    configure(&mut f, 0, SRC, DST, 8, ctrl_word(0, 0, TREQ_PERMANENT));
    configure(
        &mut f,
        1,
        SRC + 0x100,
        DST + 0x100,
        8,
        ctrl_word(1, 0, TREQ_PERMANENT) | CTRL_HIGH_PRIORITY,
    );
    // 高优先级 ch1 先跑，普通 ch0 等待。High-priority ch1 runs first.
    assert_eq!(f.run(1), 1);
    assert_eq!(f.channel(1).trans_count, 7);
    assert_eq!(f.channel(0).trans_count, 8);
    // 高优先级通道忙碌期间普通通道被完全饿死。The normal channel is starved.
    assert_eq!(f.run(7), 7);
    assert_eq!(f.channel(1).trans_count, 0);
    assert_eq!(f.channel(0).trans_count, 8);
    // 高优先级完成后才轮到普通通道。Only then does the normal channel run.
    assert_eq!(f.run(1), 1);
    assert_eq!(f.channel(0).trans_count, 7);
}

#[test]
fn pacing_timer_divides_cycles_by_y() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[1, 2]);
    // X = 1, Y = 4：每 4 个周期产生一个请求。One request every four cycles.
    f.reg_write(TIMER0, TIMER_X1_Y4);
    configure(&mut f, 0, SRC, DST, 2, ctrl_word(0, 0, DREQ_TIMER0));
    f.dma.advance(3);
    assert_eq!(f.dma.dreq(DREQ_TIMER0), Some(false));
    assert_eq!(f.run(2), 0);
    f.dma.advance(1);
    assert_eq!(f.dma.dreq(DREQ_TIMER0), Some(true));
    // 一个脉冲只放行一个单元。One pulse allows exactly one unit.
    assert_eq!(f.run(2), 1);
    assert_eq!(f.dma.dreq(DREQ_TIMER0), Some(false));
    assert_eq!(f.run(1), 0);
    // 再 4 个周期又是新脉冲。Four more cycles yield another pulse.
    f.dma.advance(4);
    assert_eq!(f.dma.dreq(DREQ_TIMER0), Some(true));
    assert_eq!(f.run(1), 1);
}

#[test]
fn pacing_timer_carries_the_fraction() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[1, 2]);
    // X = 2, Y = 4：两个周期才凑够一个脉冲。Two cycles make one pulse.
    f.reg_write(TIMER0, 0x0002_0004);
    configure(&mut f, 0, SRC, DST, 2, ctrl_word(0, 0, DREQ_TIMER0));
    f.dma.advance(1);
    assert_eq!(f.dma.dreq(DREQ_TIMER0), Some(false));
    f.dma.advance(1);
    assert_eq!(f.dma.dreq(DREQ_TIMER0), Some(true));
    assert_eq!(f.run(2), 1);
    assert_eq!(f.dma.dreq(DREQ_TIMER0), Some(false));
    f.dma.advance(2);
    assert_eq!(f.dma.dreq(DREQ_TIMER0), Some(true));
    assert_eq!(f.run(1), 1);
}

#[test]
fn pacing_timer_equal_x_and_y_pulses_every_cycle() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[1; 5]);
    f.reg_write(TIMER0, TIMER_X1_Y1);
    configure(&mut f, 0, SRC, DST, 5, ctrl_word(0, 0, DREQ_TIMER0));
    f.dma.advance(1);
    assert_eq!(f.run(5), 1);
    assert_eq!(f.dma.dreq(DREQ_TIMER0), Some(false));
    // 3 个周期排队 3 个脉冲。Three cycles queue three pulses.
    f.dma.advance(3);
    assert_eq!(f.dma.dreq(DREQ_TIMER0), Some(true));
    assert_eq!(f.run(5), 3);
    assert_eq!(f.dma.dreq(DREQ_TIMER0), Some(false));
    f.dma.advance(1);
    assert_eq!(f.run(5), 1);
    assert_eq!(f.channel(0).trans_count, 0);
}

#[test]
fn pacing_timer_zero_dividend_or_divisor_never_asserts() {
    let mut f = fixture();
    write_bytes(&mut f, SRC, &[1]);
    // X == 0：永不断言。X == 0 never asserts.
    f.reg_write(TIMER0, 0x0000_0004);
    // Y == 0：永不断言，且不产生除零。Y == 0 never asserts and avoids a division.
    f.reg_write(TIMER0 + 4, 0x0001_0000);
    configure(&mut f, 0, SRC, DST, 1, ctrl_word(0, 0, DREQ_TIMER0));
    f.dma.advance(1000);
    assert_eq!(f.dma.dreq(DREQ_TIMER0), Some(false));
    assert_eq!(f.dma.dreq(DREQ_TIMER0 + 1), Some(false));
    assert_eq!(f.run(1), 0);
}
