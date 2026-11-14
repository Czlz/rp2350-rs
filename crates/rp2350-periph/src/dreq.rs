//! RP2350 DMA request (DREQ) selectors / RP2350 DMA 请求（DREQ）选择器
//!
//! `CTRL_TRIG.TREQ_SEL` indexes one of 64 request inputs. The three RP2350 PIO
//! blocks own selectors `0..=23`: block `n` owns `8*n .. 8*n + 7`, with
//! `TX0..TX3` first and `RX0..RX3` second. The numbers come from the RP2350
//! register header (`rp2350/hardware/regs/dreq.h`); RP2040 has only two PIO
//! blocks, so RP2040 selector numbers `16..=39` mean different peripherals and
//! must not be reused. This model only publishes the PIO selectors; every
//! other selector keeps the DMA controller's reset-asserted default.
//! `CTRL_TRIG.TREQ_SEL` 索引 64 个请求输入之一。RP2350 的三个 PIO 块占用选择器
//! `0..=23`：块 `n` 占用 `8*n .. 8*n + 7`，先是 `TX0..TX3`，随后是 `RX0..RX3`。
//! 编号取自 RP2350 寄存器头文件（`rp2350/hardware/regs/dreq.h`）；RP2040 只有两个
//! PIO 块，其 `16..=39` 指向的是不同外设，因此不能沿用 RP2040 编号。本模型只发布
//! PIO 选择器，其它选择器保持 DMA 控制器的复位断言默认值。
//!
//! # Semantics / 语义
//!
//! A PIO DREQ is a level, sampled by the DMA controller when it arbitrates:
//!
//! - `PIOx_TXy` is asserted while state machine `y`'s TX FIFO has room for one
//!   more word, i.e. while it is **not full**. The DMA channel feeds the FIFO.
//! - `PIOx_RXy` is asserted while state machine `y`'s RX FIFO holds at least
//!   one word, i.e. while it is **not empty**. The DMA channel drains the FIFO.
//!
//! When `SHIFTCTRL.FJOIN_TX`/`FJOIN_RX` joins a FIFO, the joined direction owns
//! one 8-deep FIFO and its DREQ follows the joined occupancy (`not full` up to
//! 8 words for TX, `not empty` for RX). The joined-away direction has no
//! storage, reads as both full and empty, and therefore holds its DREQ
//! deasserted. With both join bits set neither direction has storage and both
//! DREQs stay low.
//! PIO DREQ 是电平信号，由 DMA 控制器在仲裁时采样：
//!
//! - `PIOx_TXy` 在状态机 `y` 的 TX FIFO 还能再放一个字（即**未满**）时断言，
//!   DMA 通道据此向 FIFO 写入。
//! - `PIOx_RXy` 在状态机 `y` 的 RX FIFO 至少有一个字（即**非空**）时断言，
//!   DMA 通道据此从 FIFO 读出。
//!
//! `SHIFTCTRL.FJOIN_TX`/`FJOIN_RX` 合并 FIFO 时，被合并的方向独占一个 8 深度
//! FIFO，其 DREQ 跟随合并后的占用（TX 最多 8 字“未满”，RX “非空”）。被合并掉的
//! 方向没有存储，同时读为满和空，因此其 DREQ 保持去断言。两个 join 位同时置位时
//! 两个方向都没有存储，两条 DREQ 都为低。
//!
//! # Not modelled / 未建模项
//!
//! DREQ timing is not cycle-exact: [`crate::Dma::run`] samples the published
//! level array per call, and the host decides when to republish PIO levels
//! ([`crate::BootPeripherals::sync_pio_dreqs`], pumped by `rp2350-cli` before
//! each DMA batch). Non-PIO selectors (SPI, UART, ADC, HSTX, DMA pacing
//! timers ...) are not driven by their peripherals yet.
//! DREQ 时序不是周期精确的：[`crate::Dma::run`] 每次调用采样已发布的电平数组，
//! 由宿主决定何时重新发布 PIO 电平（[`crate::BootPeripherals::sync_pio_dreqs`]，
//! `rp2350-cli` 在每个 DMA 批次前泵送）。非 PIO 选择器（SPI、UART、ADC、HSTX、
//! DMA 节拍定时器等）尚未由对应外设驱动。

/// HSTX FIFO DREQ selector (`DREQ_HSTX`) / HSTX FIFO 的 DREQ 选择器
pub const HSTX_DREQ: u8 = 52;

/// Number of state machines with DREQ lines in one PIO block
/// 一个 PIO 块中带 DREQ 线的状态机数量
pub const PIO_DREQ_SMS: u8 = 4;

/// Number of TX DREQ selectors per PIO block / 每个 PIO 块的 TX DREQ 选择器数量
pub const PIO_TX_PER_BLOCK: u8 = 4;

/// Distance between two PIO blocks' DREQ selectors / 相邻 PIO 块 DREQ 选择器的间距
pub const PIO_DREQ_BLOCK_STRIDE: u8 = 8;

/// Number of PIO blocks with DREQ selectors / 带 DREQ 选择器的 PIO 块数量
pub const PIO_DREQ_BLOCKS: u8 = 3;

/// PIO0 TX FIFO 0 DREQ (`0`) / PIO0 TX FIFO 0 的 DREQ（`0`）
pub const PIO0_TX0: u8 = 0;

/// PIO0 TX FIFO 1 DREQ / PIO0 TX FIFO 1 的 DREQ
pub const PIO0_TX1: u8 = 1;

/// PIO0 TX FIFO 2 DREQ / PIO0 TX FIFO 2 的 DREQ
pub const PIO0_TX2: u8 = 2;

/// PIO0 TX FIFO 3 DREQ / PIO0 TX FIFO 3 的 DREQ
pub const PIO0_TX3: u8 = 3;

/// PIO0 RX FIFO 0 DREQ / PIO0 RX FIFO 0 的 DREQ
pub const PIO0_RX0: u8 = 4;

/// PIO0 RX FIFO 1 DREQ / PIO0 RX FIFO 1 的 DREQ
pub const PIO0_RX1: u8 = 5;

/// PIO0 RX FIFO 2 DREQ / PIO0 RX FIFO 2 的 DREQ
pub const PIO0_RX2: u8 = 6;

/// PIO0 RX FIFO 3 DREQ / PIO0 RX FIFO 3 的 DREQ
pub const PIO0_RX3: u8 = 7;

/// PIO1 TX FIFO 0 DREQ / PIO1 TX FIFO 0 的 DREQ
pub const PIO1_TX0: u8 = 8;

/// PIO1 TX FIFO 1 DREQ / PIO1 TX FIFO 1 的 DREQ
pub const PIO1_TX1: u8 = 9;

/// PIO1 TX FIFO 2 DREQ / PIO1 TX FIFO 2 的 DREQ
pub const PIO1_TX2: u8 = 10;

/// PIO1 TX FIFO 3 DREQ / PIO1 TX FIFO 3 的 DREQ
pub const PIO1_TX3: u8 = 11;

/// PIO1 RX FIFO 0 DREQ / PIO1 RX FIFO 0 的 DREQ
pub const PIO1_RX0: u8 = 12;

/// PIO1 RX FIFO 1 DREQ / PIO1 RX FIFO 1 的 DREQ
pub const PIO1_RX1: u8 = 13;

/// PIO1 RX FIFO 2 DREQ / PIO1 RX FIFO 2 的 DREQ
pub const PIO1_RX2: u8 = 14;

/// PIO1 RX FIFO 3 DREQ / PIO1 RX FIFO 3 的 DREQ
pub const PIO1_RX3: u8 = 15;

/// PIO2 TX FIFO 0 DREQ / PIO2 TX FIFO 0 的 DREQ
pub const PIO2_TX0: u8 = 16;

/// PIO2 TX FIFO 1 DREQ / PIO2 TX FIFO 1 的 DREQ
pub const PIO2_TX1: u8 = 17;

/// PIO2 TX FIFO 2 DREQ / PIO2 TX FIFO 2 的 DREQ
pub const PIO2_TX2: u8 = 18;

/// PIO2 TX FIFO 3 DREQ / PIO2 TX FIFO 3 的 DREQ
pub const PIO2_TX3: u8 = 19;

/// PIO2 RX FIFO 0 DREQ / PIO2 RX FIFO 0 的 DREQ
pub const PIO2_RX0: u8 = 20;

/// PIO2 RX FIFO 1 DREQ / PIO2 RX FIFO 1 的 DREQ
pub const PIO2_RX1: u8 = 21;

/// PIO2 RX FIFO 2 DREQ / PIO2 RX FIFO 2 的 DREQ
pub const PIO2_RX2: u8 = 22;

/// PIO2 RX FIFO 3 DREQ, the last PIO selector / PIO2 RX FIFO 3 的 DREQ，最后一个 PIO 选择器
pub const PIO2_RX3: u8 = 23;

/// First PIO DREQ selector (`PIO0_TX0`) / 第一个 PIO DREQ 选择器（`PIO0_TX0`）
pub const PIO_DREQ_FIRST: u8 = PIO0_TX0;

/// Last PIO DREQ selector (`PIO2_RX3`) / 最后一个 PIO DREQ 选择器（`PIO2_RX3`）
pub const PIO_DREQ_LAST: u8 = PIO2_RX3;

/// First TX selector of `block`, or `None` for a block index above 2
/// 块 `block` 的第一个 TX 选择器；块序号大于 2 时返回 `None`
#[must_use]
pub const fn pio_tx_dreq_base(block: u8) -> Option<u8> {
    if block >= PIO_DREQ_BLOCKS {
        return None;
    }
    Some(block * PIO_DREQ_BLOCK_STRIDE)
}

/// First RX selector of `block`, or `None` for a block index above 2
/// 块 `block` 的第一个 RX 选择器；块序号大于 2 时返回 `None`
#[must_use]
pub const fn pio_rx_dreq_base(block: u8) -> Option<u8> {
    if block >= PIO_DREQ_BLOCKS {
        return None;
    }
    Some(block * PIO_DREQ_BLOCK_STRIDE + PIO_TX_PER_BLOCK)
}

/// TX selector of state machine `sm` in `block`
/// 块 `block` 中状态机 `sm` 的 TX 选择器
///
/// Returns `None` for a block index above 2 or a machine index above 3.
/// 块序号大于 2 或状态机序号大于 3 时返回 `None`。
#[must_use]
pub const fn pio_tx_dreq(block: u8, sm: u8) -> Option<u8> {
    if block >= PIO_DREQ_BLOCKS || sm >= PIO_DREQ_SMS {
        return None;
    }
    Some(block * PIO_DREQ_BLOCK_STRIDE + sm)
}

/// RX selector of state machine `sm` in `block`
/// 块 `block` 中状态机 `sm` 的 RX 选择器
///
/// Returns `None` for a block index above 2 or a machine index above 3.
/// 块序号大于 2 或状态机序号大于 3 时返回 `None`。
#[must_use]
pub const fn pio_rx_dreq(block: u8, sm: u8) -> Option<u8> {
    if block >= PIO_DREQ_BLOCKS || sm >= PIO_DREQ_SMS {
        return None;
    }
    Some(block * PIO_DREQ_BLOCK_STRIDE + PIO_TX_PER_BLOCK + sm)
}

/// Whether `selector` belongs to a PIO FIFO (`0..=23`)
/// `selector` 是否属于 PIO FIFO（`0..=23`）
#[must_use]
pub const fn is_pio_dreq(selector: u8) -> bool {
    selector <= PIO_DREQ_LAST
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pio_selectors_follow_the_rp2350_datasheet_table() {
        // RP2350：PIO0 0..7、PIO1 8..15、PIO2 16..23，先 TX 后 RX。
        // RP2350: PIO0 0..7, PIO1 8..15, PIO2 16..23, TX before RX.
        assert_eq!(
            [
                PIO0_TX0, PIO0_TX1, PIO0_TX2, PIO0_TX3, PIO0_RX0, PIO0_RX1, PIO0_RX2, PIO0_RX3,
            ],
            [0, 1, 2, 3, 4, 5, 6, 7]
        );
        assert_eq!(
            [
                PIO1_TX0, PIO1_TX1, PIO1_TX2, PIO1_TX3, PIO1_RX0, PIO1_RX1, PIO1_RX2, PIO1_RX3,
            ],
            [8, 9, 10, 11, 12, 13, 14, 15]
        );
        assert_eq!(
            [
                PIO2_TX0, PIO2_TX1, PIO2_TX2, PIO2_TX3, PIO2_RX0, PIO2_RX1, PIO2_RX2, PIO2_RX3,
            ],
            [16, 17, 18, 19, 20, 21, 22, 23]
        );
        assert_eq!((PIO_DREQ_FIRST, PIO_DREQ_LAST), (0, 23));
    }

    #[test]
    fn mapping_helpers_cover_every_block_and_machine() {
        for block in 0..PIO_DREQ_BLOCKS {
            let tx_base = pio_tx_dreq_base(block).expect("tx base");
            let rx_base = pio_rx_dreq_base(block).expect("rx base");
            assert_eq!(rx_base, tx_base + PIO_TX_PER_BLOCK);
            for sm in 0..PIO_DREQ_SMS {
                assert_eq!(pio_tx_dreq(block, sm), Some(tx_base + sm));
                assert_eq!(pio_rx_dreq(block, sm), Some(rx_base + sm));
                let tx = pio_tx_dreq(block, sm).expect("tx");
                let rx = pio_rx_dreq(block, sm).expect("rx");
                assert!(is_pio_dreq(tx) && is_pio_dreq(rx));
                assert_eq!(PIO_DREQ_LAST - tx, 23 - tx);
                assert!(rx > tx);
            }
        }
    }

    #[test]
    fn out_of_range_blocks_and_machines_have_no_selector() {
        assert_eq!(pio_tx_dreq_base(3), None);
        assert_eq!(pio_rx_dreq_base(3), None);
        assert_eq!(pio_tx_dreq(0, 4), None);
        assert_eq!(pio_rx_dreq(2, 4), None);
        assert_eq!(pio_tx_dreq(3, 0), None);
        assert!(!is_pio_dreq(24));
        assert!(!is_pio_dreq(63));
    }
}
