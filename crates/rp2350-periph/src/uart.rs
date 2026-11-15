//! UART0 (PL011) peripheral model / UART0（PL011）外设模型
//!
//! Transmit data written to `UARTDR` is appended to a shared [`UartSink`],
//! tagged with the current simulated microsecond. The flag register always
//! reports the TX and RX FIFOs as empty (`TXFE | RXFE`), so polling transmit
//! loops never block. Receive data and interrupts are not modelled.
//! 写入 `UARTDR` 的发送数据会追加到共享的 [`UartSink`]，并附带当前仿真微秒。
//! 标志寄存器始终报告收发 FIFO 为空（`TXFE | RXFE`），因此轮询发送循环不会阻塞。
//! 不建模接收数据与中断。
//!
//! # Baud rate / 波特率
//!
//! `UARTIBRD`, `UARTFBRD` and `UARTLCR_H` are stored, and
//! [`Uart0::baud_rate`] derives the line rate from the live `clk_peri`:
//!
//! ```text
//! baud = clk_peri * 4 / (64 * IBRD + FBRD)
//! bit  = 1 / baud
//! ```
//!
//! Byte-level shifting, FIFO occupancy and framing errors are **not** modelled:
//! a byte enters the sink immediately (with its timestamp) even though a real
//! UART would spend ten bit periods on the wire. The reported baud rate and the
//! timestamp of the last byte still follow the real peripheral clock, so host
//! logs and throughput estimates are meaningful.
//! 保存 `UARTIBRD`、`UARTFBRD` 与 `UARTLCR_H`，[`Uart0::baud_rate`] 按实时
//! `clk_peri` 推导线速率：
//!
//! ```text
//! baud = clk_peri * 4 / (64 * IBRD + FBRD)
//! bit  = 1 / baud
//! ```
//!
//! **不**建模字节级移位、FIFO 占用与帧错误：字节立即进入 sink（带时间戳），而
//! 真实 UART 会在线上花费十个位周期。上报的波特率与最后字节的时间戳仍跟随真实
//! 外设时钟，因此宿主日志与吞吐估计是有意义的。

use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::clock_time::Timebase;
use super::clock_tree::ClockTree;
use super::map;

/// `UARTIBRD` offset / `UARTIBRD` 偏移
pub const IBRD: u32 = 0x24;

/// `UARTFBRD` offset / `UARTFBRD` 偏移
pub const FBRD: u32 = 0x28;

/// `UARTLCR_H` offset / `UARTLCR_H` 偏移
pub const LCR_H: u32 = 0x2C;

/// Shared UART transmit buffer / 共享 UART 发送缓冲区
#[derive(Debug, Default)]
pub struct UartSink {
    bytes: Vec<u8>,
    last_byte_us: Option<u64>,
}

impl UartSink {
    /// Append one byte without a timestamp / 追加一个字节（无时间戳）
    pub fn push(&mut self, byte: u8) {
        self.bytes.push(byte);
    }

    /// Append one byte at simulated time `time_us` / 在仿真时间 `time_us` 追加一个字节
    pub fn push_at(&mut self, byte: u8, time_us: u64) {
        self.bytes.push(byte);
        self.last_byte_us = Some(time_us);
    }

    /// Simulated microsecond of the most recent byte / 最近一个字节的仿真微秒
    #[must_use]
    pub fn last_byte_us(&self) -> Option<u64> {
        self.last_byte_us
    }

    /// Number of buffered bytes / 已缓冲字节数
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether the buffer is empty / 缓冲区是否为空
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Drain the buffer / 取出并清空缓冲区
    pub fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.bytes)
    }
}

/// PL011 `UARTFR` value with both FIFOs empty / 收发 FIFO 均为空的 PL011 `UARTFR` 值
const FR_TXFE_RXFE: u32 = (1 << 7) | (1 << 4);

/// UART0 register block / UART0 寄存器块
#[derive(Debug)]
pub struct Uart0 {
    sink: Arc<Mutex<UartSink>>,
    timebase: Arc<Mutex<Timebase>>,
    tree: ClockTree,
    regs: RegisterFile,
}

impl Uart0 {
    /// Create UART0 writing into `sink` with the bootrom clock
    /// 用 bootrom 时钟创建写入 `sink` 的 UART0
    #[must_use]
    pub fn new(sink: Arc<Mutex<UartSink>>) -> Self {
        let tree = ClockTree::new();
        let timebase = Arc::new(Mutex::new(Timebase::new(
            tree.sysclk_hz(),
            tree.refclk_hz(),
        )));
        Self::with_clock(sink, timebase, tree)
    }

    /// Create UART0 using the shared time base and clock tree
    /// 使用共享时基与时钟树创建 UART0
    #[must_use]
    pub fn with_clock(
        sink: Arc<Mutex<UartSink>>,
        timebase: Arc<Mutex<Timebase>>,
        tree: ClockTree,
    ) -> Self {
        Self {
            sink,
            timebase,
            tree,
            regs: RegisterFile::new(map::DEVICE_WINDOW_SIZE),
        }
    }

    /// PL011 baud-rate divisor: `64 * IBRD + FBRD` / PL011 波特率除数：`64 * IBRD + FBRD`
    #[must_use]
    pub fn baud_divisor(&self) -> u64 {
        let ibrd = u64::from(self.regs.read(IBRD).unwrap_or(0) & 0xFFFF);
        let fbrd = u64::from(self.regs.read(FBRD).unwrap_or(0) & 0x3F);
        ibrd * 64 + fbrd
    }

    /// Line baud rate derived from the live `clk_peri` / 由实时 `clk_peri` 推导的线速率
    ///
    /// Returns 0 while the divisors are zero (the reset state) or `clk_peri` is
    /// stopped. `/ 除数为零（复位状态）或 `clk_peri` 停止时返回 0。`
    #[must_use]
    pub fn baud_rate(&self) -> u32 {
        let divisor = self.baud_divisor();
        let peri = u64::from(self.tree.periclk_hz());
        if divisor == 0 || peri == 0 {
            return 0;
        }
        // baud = clk_peri / (16 * (IBRD + FBRD/64)) = clk_peri * 4 / divisor.
        u32::try_from(peri * 4 / divisor).unwrap_or(u32::MAX)
    }

    /// Duration of one UART bit in nanoseconds / 一个 UART 位的纳秒时长
    #[must_use]
    pub fn bit_time_ns(&self) -> Option<u64> {
        let baud = u64::from(self.baud_rate());
        (baud != 0).then(|| 1_000_000_000 / baud)
    }

    /// Time the last transmitted byte entered the sink / 最后发送字节进入 sink 的时间
    #[must_use]
    pub fn last_byte_us(&self) -> Option<u64> {
        self.sink.lock().last_byte_us()
    }
}

impl MmioDevice for Uart0 {
    fn name(&self) -> &str {
        "UART0"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::UART0_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        match offset {
            // UARTDR / UARTRIS / UARTMIS: no received data or interrupt sources.
            // UARTDR / UARTRIS / UARTMIS：无接收数据、无中断源。
            0x00 | 0x3C | 0x40 => Some(0),
            // UARTFR: TX and RX FIFOs empty. UARTFR：收发 FIFO 为空。
            0x18 => Some(FR_TXFE_RXFE),
            _ => self.regs.read(offset),
        }
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        match offset {
            // UARTDR: enqueue the low byte with its simulated timestamp.
            // UARTDR：将低字节连同仿真时间戳入队。
            0x00 => {
                let now = self.timebase.lock().time_us();
                self.sink.lock().push_at(value as u8, now);
                true
            }
            // UARTICR: clear interrupts (nothing pending). UARTICR：清除中断（无待处理项）。
            0x44 => true,
            _ => self.regs.write(offset, value),
        }
    }

    fn reset(&mut self) {
        self.regs.reset();
    }

    fn irq_number(&self) -> Option<u16> {
        Some(map::UART0_IRQ)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock_tree::CLK_PERI;

    #[test]
    fn transmit_bytes_reach_the_sink() {
        let sink = Arc::new(Mutex::new(UartSink::default()));
        let mut uart = Uart0::new(Arc::clone(&sink));
        assert!(uart.write32(0x00, u32::from(b'H')));
        assert!(uart.write32(0x00, u32::from(b'i')));
        let bytes = sink.lock().take();
        assert_eq!(bytes, b"Hi");
    }

    #[test]
    fn flag_register_reports_empty_fifos() {
        let sink = Arc::new(Mutex::new(UartSink::default()));
        let mut uart = Uart0::new(sink);
        assert_eq!(uart.read32(0x18), Some(FR_TXFE_RXFE));
    }

    #[test]
    fn baud_rate_follows_clk_peri_and_the_pl011_divisors() {
        // 115200 baud @ 48 MHz：IBRD=26、FBRD=3 → 48M*4/(64*26+3) = 115384。
        // 115200 baud at 48 MHz: IBRD=26 and FBRD=3 give 115384 baud.
        let tree = ClockTree::new();
        tree.write_channel_div(CLK_PERI, 3 << 16); // 150/3 = 50 MHz
        let sink = Arc::new(Mutex::new(UartSink::default()));
        let mut uart = Uart0::with_clock(
            sink,
            Arc::new(Mutex::new(Timebase::new(150_000_000, 12_000_000))),
            tree.clone(),
        );
        assert_eq!(uart.read32(IBRD), Some(0));
        assert!(uart.write32(IBRD, 26));
        assert!(uart.write32(FBRD, 3));
        assert_eq!(uart.baud_rate(), 50_000_000 * 4 / (64 * 26 + 3));
        // 2 位分频字段：0 表示 /4，因此 150/4 = 37.5 MHz。
        // The 2-bit divider treats 0 as /4, so 150/4 = 37.5 MHz.
        tree.write_channel_div(CLK_PERI, 0);
        assert_eq!(uart.baud_rate(), 37_500_000 * 4 / (64 * 26 + 3));
    }

    #[test]
    fn transmitted_bytes_carry_the_simulated_timestamp() {
        let timebase = Arc::new(Mutex::new(Timebase::new(150_000_000, 12_000_000)));
        timebase.lock().advance(3_000, 150_000_000, 12_000_000, 1);
        let sink = Arc::new(Mutex::new(UartSink::default()));
        let mut uart = Uart0::with_clock(Arc::clone(&sink), timebase, ClockTree::new());
        assert!(uart.write32(0x00, u32::from(b'X')));
        assert_eq!(uart.last_byte_us(), Some(20));
    }
}
