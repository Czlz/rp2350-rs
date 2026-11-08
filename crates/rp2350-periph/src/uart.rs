//! UART0 (PL011) peripheral stub / UART0（PL011）外设桩
//!
//! Transmit data written to `UARTDR` is appended to a shared [`UartSink`]. The
//! flag register always reports the TX and RX FIFOs as empty (`TXFE | RXFE`), so
//! polling transmit loops never block. Receive data and interrupts are not
//! modelled in Stage 4.
//! 写入 `UARTDR` 的发送数据会追加到共享的 [`UartSink`]。标志寄存器始终报告
//! 收发 FIFO 为空（`TXFE | RXFE`），因此轮询发送循环不会阻塞。Stage 4 不建模
//! 接收数据与中断。

use std::sync::Arc;

use parking_lot::Mutex;
use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::map;

/// Shared UART transmit buffer / 共享 UART 发送缓冲区
#[derive(Debug, Default)]
pub struct UartSink {
    bytes: Vec<u8>,
}

impl UartSink {
    /// Append one byte / 追加一个字节
    pub fn push(&mut self, byte: u8) {
        self.bytes.push(byte);
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
    regs: RegisterFile,
}

impl Uart0 {
    /// Create UART0 writing into `sink` / 创建写入 `sink` 的 UART0
    #[must_use]
    pub fn new(sink: Arc<Mutex<UartSink>>) -> Self {
        Self {
            sink,
            regs: RegisterFile::new(map::DEVICE_WINDOW_SIZE),
        }
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
            // UARTDR: enqueue the low byte. UARTDR：将低字节入队。
            0x00 => {
                self.sink.lock().push(value as u8);
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
}
