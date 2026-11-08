//! Byte-stream view over the CDC bulk endpoints
//! CDC 批量端点之上的字节流视图

use std::collections::VecDeque;

use rp2350_usb::controller::Controller;

use crate::bridge::CdcEndpoints;
use crate::repl::ByteLink;

/// Buffered CDC data channel between the host bridge and the guest firmware.
/// 主机桥与客户机固件之间有缓冲的 CDC 数据通道。
///
/// The channel pumps bulk endpoints only when asked, so callers stay in
/// control of simulation time: `service()` drains device-to-host packets,
/// `flush()` pushes host-to-device bytes and stops on the first NAK.
/// 通道仅在调用时搬运批量端点，因此时序由调用方掌握：`service()` 取走设备
/// 到主机的包，`flush()` 推送主机到设备的字节并在首次 NAK 时停止。
pub struct CdcChannel {
    controller: Controller,
    endpoints: CdcEndpoints,
    rx: VecDeque<u8>,
    tx: VecDeque<u8>,
}

impl CdcChannel {
    /// Create a channel over the enumerated CDC endpoints
    /// 在枚举得到的 CDC 端点之上创建通道
    #[must_use]
    pub fn new(controller: Controller, endpoints: CdcEndpoints) -> Self {
        Self {
            controller,
            endpoints,
            rx: VecDeque::new(),
            tx: VecDeque::new(),
        }
    }

    /// Discovery result this channel was built from / 构建该通道时使用的枚举结果
    #[must_use]
    pub fn endpoints(&self) -> CdcEndpoints {
        self.endpoints
    }

    /// Number of bytes waiting in the receive buffer / 接收缓冲中的字节数
    #[must_use]
    pub fn readable_len(&self) -> usize {
        self.rx.len()
    }

    /// Number of bytes queued for transmission / 待发送队列中的字节数
    #[must_use]
    pub fn writable_pending(&self) -> usize {
        self.tx.len()
    }

    /// Collect device-to-host packets until the endpoint NAKs.
    /// 取走设备到主机的包，直到端点返回 NAK。
    ///
    /// Returns the number of bytes appended to the receive buffer.
    /// 返回追加到接收缓冲的字节数。
    pub fn service(&mut self) -> usize {
        let mut appended = 0;
        let ep = usize::from(self.endpoints.in_address & 0x0F);
        while let rp2350_usb::endpoint::InResponse::Data(packet) = self.controller.host_in(ep) {
            self.rx.extend(packet.bytes().iter().copied());
            appended += packet.len();
            if packet.len() < usize::from(self.endpoints.in_max_packet.max(1)) {
                break;
            }
        }
        appended
    }

    /// Read one received byte / 读取一个已接收字节
    pub fn read_byte(&mut self) -> Option<u8> {
        self.rx.pop_front()
    }

    /// Read into `out`, returning the number of bytes copied
    /// 读取到 `out`，返回复制到的字节数
    pub fn read(&mut self, out: &mut [u8]) -> usize {
        let mut copied = 0;
        for slot in out.iter_mut() {
            let Some(byte) = self.rx.pop_front() else {
                break;
            };
            *slot = byte;
            copied += 1;
        }
        copied
    }

    /// Queue bytes and push as many packets as the device accepts
    /// 入队字节，并按设备可接受量推送数据包
    ///
    /// Returns the number of bytes accepted by the device.
    /// 返回设备已接收的字节数。
    pub fn write(&mut self, data: &[u8]) -> usize {
        self.tx.extend(data.iter().copied());
        self.flush()
    }

    /// Push queued bytes until a NAK or an empty queue
    /// 推送队列字节，直到 NAK 或队列为空
    ///
    /// Returns the number of bytes accepted by the device.
    /// 返回设备已接收的字节数。
    pub fn flush(&mut self) -> usize {
        let packet = usize::from(self.endpoints.out_max_packet.max(1));
        let ep = usize::from(self.endpoints.out_address & 0x0F);
        let mut accepted = 0;
        while !self.tx.is_empty() {
            let chunk_len = self.tx.len().min(packet);
            let chunk: Vec<u8> = self.tx.iter().take(chunk_len).copied().collect();
            match self.controller.host_out(ep, &chunk) {
                rp2350_usb::endpoint::OutResponse::Ack { .. } => {
                    for _ in 0..chunk_len {
                        self.tx.pop_front();
                    }
                    accepted += chunk_len;
                }
                _ => break,
            }
        }
        accepted
    }
}

/// [`ByteLink`] adapter that runs the guest and pumps the CDC endpoints
/// 运行客户机并泵送 CDC 端点的 [`ByteLink`] 适配器
///
/// `run_cycles` advances the simulated CPU by the requested number of cycles;
/// after each advance the link flushes queued host-to-device bytes and collects
/// device-to-host packets.
/// `run_cycles` 按请求周期推进模拟 CPU；每次推进后链路刷新主机到设备的待发
/// 字节并收集设备到主机的数据包。
pub struct CdcLink<F: FnMut(u64)> {
    channel: CdcChannel,
    run_cycles: F,
    sysclk_hz: u32,
    now_micros: u64,
}

impl<F: FnMut(u64)> CdcLink<F> {
    /// Wrap a channel and a cycle runner (150 MHz system clock)
    /// 包装通道与周期运行器（150 MHz 系统时钟）
    #[must_use]
    pub fn new(channel: CdcChannel, run_cycles: F) -> Self {
        Self {
            channel,
            run_cycles,
            sysclk_hz: 150_000_000,
            now_micros: 0,
        }
    }

    /// Override the system clock used to convert microseconds to cycles
    /// 覆盖将微秒换算为周期的系统时钟
    pub fn set_sysclk_hz(&mut self, hz: u32) {
        self.sysclk_hz = hz;
    }

    /// Underlying CDC channel / 底层 CDC 通道
    #[must_use]
    pub fn channel(&self) -> &CdcChannel {
        &self.channel
    }

    /// Underlying CDC channel, mutably / 可变访问底层 CDC 通道
    pub fn channel_mut(&mut self) -> &mut CdcChannel {
        &mut self.channel
    }
}

impl<F: FnMut(u64)> ByteLink for CdcLink<F> {
    fn send(&mut self, bytes: &[u8]) -> usize {
        // CdcChannel queues unboundedly, so every byte is accepted; flushing
        // happens in `advance` where the guest can run between packets.
        // CdcChannel 的队列无上限，因此所有字节都被接受；实际刷新发生在
        // `advance` 中，此时客户机可以在包之间运行。
        let _ = self.channel.write(bytes);
        bytes.len()
    }

    fn receive(&mut self, out: &mut [u8]) -> usize {
        self.channel.service();
        self.channel.read(out)
    }

    fn advance(&mut self, micros: u64) {
        let cycles = (u64::from(self.sysclk_hz) * micros).div_ceil(1_000_000);
        (self.run_cycles)(cycles);
        self.channel.flush();
        self.channel.service();
        self.now_micros += micros;
    }

    fn now_micros(&self) -> u64 {
        self.now_micros
    }
}

impl CdcChannel {
    /// Test-only helper to mutate the shared core like guest firmware
    /// 仅测试使用的辅助方法：像客户机固件一样修改共享核心
    #[cfg(test)]
    fn with_firmware_in(&mut self, f: impl FnOnce(&mut rp2350_usb::UsbCore)) {
        self.controller.with_core(f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp2350_usb::Controller;
    use rp2350_usb::regs::{
        self, Alias, Direction, EP_CTRL_BUFFER_OFFSET_MASK, EP_CTRL_ENABLE, SIE_CTRL_PULLUP_EN,
    };

    fn endpoints() -> CdcEndpoints {
        CdcEndpoints {
            in_address: 0x82,
            in_max_packet: 64,
            out_address: 0x02,
            out_max_packet: 64,
            interface: 1,
        }
    }

    fn ready_controller() -> Controller {
        let controller = Controller::new();
        controller.with_core(|core| {
            core.write_reg(regs::MAIN_CTRL, Alias::Rw, regs::MAIN_CTRL_CONTROLLER_EN);
            core.write_reg(regs::SIE_CTRL, Alias::Rw, SIE_CTRL_PULLUP_EN);
            core.host_attach();
        });
        controller
    }

    fn configure_endpoint(controller: &Controller, ep: usize, dir: Direction, offset: usize) {
        controller.with_core(|core| {
            let ctrl = EP_CTRL_ENABLE | (offset as u32 & EP_CTRL_BUFFER_OFFSET_MASK);
            core.write_dpram_u32(regs::ep_ctrl_offset(ep, dir), ctrl);
            core.write_dpram_u32(regs::ep_buf_ctrl_offset(ep, dir), regs::BUF_CTRL_AVAIL);
        });
    }

    #[test]
    fn write_splits_into_packets_and_reads_back_device_in() {
        let controller = ready_controller();
        configure_endpoint(&controller, 2, Direction::Out, 0x200);
        configure_endpoint(&controller, 2, Direction::In, 0x280);
        let mut channel = CdcChannel::new(controller.clone(), endpoints());

        assert_eq!(channel.write(b"\x03\x03\x01"), 3);
        channel.with_firmware_in(|core| {
            // Firmware loads a banner into the IN buffer and marks it last.
            let data = b"raw REPL; CTRL-B to exit\r\n>";
            for (i, byte) in data.iter().enumerate() {
                core.write_dpram_u8(0x280 + i, *byte);
            }
            let ctrl = regs::BUF_CTRL_AVAIL
                | regs::BUF_CTRL_FULL
                | regs::BUF_CTRL_LAST
                | data.len() as u32;
            core.write_dpram_u32(regs::ep_buf_ctrl_offset(2, Direction::In), ctrl);
        });
        let appended = channel.service();
        assert_eq!(appended, b"raw REPL; CTRL-B to exit\r\n>".len());
        assert_eq!(channel.readable_len(), appended);
        let mut out = [0u8; 8];
        assert_eq!(channel.read(&mut out), 8);
        assert_eq!(&out, b"raw REPL");
    }
}
