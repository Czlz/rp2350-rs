//! Byte-stream view over the CDC bulk endpoints
//! CDC 批量端点之上的字节流视图

use std::collections::VecDeque;

use rp2350_usb::controller::Controller;

use crate::bridge::{CdcEndpoints, UsbHostBridge};
use crate::repl::ByteLink;
use crate::sof::SofScheduler;

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
    /// 1 ms SOF schedule that keeps running after enumeration
    /// 枚举结束后继续运行的 1 ms SOF 调度
    sof: SofScheduler,
    /// Simulated cycles elapsed since the channel was created
    /// 通道创建以来经过的模拟周期
    now_cycles: u64,
}

impl CdcChannel {
    /// Create a channel over the enumerated CDC endpoints
    /// 在枚举得到的 CDC 端点之上创建通道
    ///
    /// The SOF schedule continues from the frame number the device last saw, so
    /// TinyUSB's errata-15 path keeps getting fresh frames during the data
    /// phase instead of seeing `SOF_RD` jump back to zero.
    /// SOF 调度从设备最后看到的帧号继续，因此 TinyUSB 的 errata-15 路径在数据
    /// 阶段仍能拿到新帧号，而不会看到 `SOF_RD` 跳回零。
    #[must_use]
    pub fn new(controller: Controller, endpoints: CdcEndpoints) -> Self {
        Self::with_clock(
            controller,
            endpoints,
            UsbHostBridge::DEFAULT_SYSCLK_HZ,
            crate::sof::USB_FULL_SPEED_HZ,
        )
    }

    /// Create a channel with explicit `CLK_SYS` and `CLK_USB` rates
    /// 用显式的 `CLK_SYS` 与 `CLK_USB` 频率创建通道
    #[must_use]
    pub fn with_clock(
        controller: Controller,
        endpoints: CdcEndpoints,
        sysclk_hz: u32,
        usbclk_hz: u32,
    ) -> Self {
        let frame = controller.with_core(|core| core.sof_frame());
        let mut sof = SofScheduler::new_with_usb(sysclk_hz, usbclk_hz);
        sof.reset_at(0, frame);
        Self {
            controller,
            endpoints,
            rx: VecDeque::new(),
            tx: VecDeque::new(),
            sof,
            now_cycles: 0,
        }
    }

    /// Override the system clock used for the SOF frame period
    /// 覆盖用于 SOF 帧周期的系统时钟
    pub fn set_sysclk_hz(&mut self, hz: u32) {
        self.sof.set_sysclk_hz(hz);
    }

    /// Override both the system and USB clocks / 覆盖系统与 USB 时钟
    pub fn set_clocks(&mut self, sysclk_hz: u32, usbclk_hz: u32) {
        self.sof.set_clocks(sysclk_hz, usbclk_hz);
    }

    /// Advance simulated time and issue every SOF frame that is due
    /// 推进模拟时间并下发所有到期的 SOF 帧
    ///
    /// Callers must invoke this before pumping endpoints so the device observes
    /// a fresh frame before the frame's transactions, matching [`crate::bridge`].
    /// 调用方必须在搬运端点之前调用，使设备先看到新帧再看到本帧事务，与
    /// [`crate::bridge`] 的顺序一致。
    pub fn advance_time(&mut self, cycles: u64) -> u32 {
        self.now_cycles = self.now_cycles.saturating_add(cycles);
        self.sof.advance(&self.controller, self.now_cycles)
    }

    /// Simulated cycles elapsed since channel creation
    /// 通道创建以来经过的模拟周期
    #[must_use]
    pub const fn now_cycles(&self) -> u64 {
        self.now_cycles
    }

    /// Frame number of the last SOF handed to the device
    /// 最近交给设备的 SOF 帧号
    #[must_use]
    pub const fn sof_frame(&self) -> u16 {
        self.sof.frame()
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
    ///
    /// Use [`CdcLink::with_clock`] to follow the guest's live `CLK_SYS`.
    /// 使用 [`CdcLink::with_clock`] 跟随客户机的实时 `CLK_SYS`。
    #[must_use]
    pub fn new(channel: CdcChannel, run_cycles: F) -> Self {
        Self {
            channel,
            run_cycles,
            sysclk_hz: UsbHostBridge::DEFAULT_SYSCLK_HZ,
            now_micros: 0,
        }
    }

    /// Wrap a channel and runner at an explicit system clock
    /// 以显式系统时钟包装通道与运行器
    #[must_use]
    pub fn with_clock(channel: CdcChannel, run_cycles: F, sysclk_hz: u32) -> Self {
        let mut channel = channel;
        channel.set_sysclk_hz(sysclk_hz);
        Self {
            channel,
            run_cycles,
            sysclk_hz,
            now_micros: 0,
        }
    }

    /// Override the system clock used to convert microseconds to cycles
    /// 覆盖将微秒换算为周期的系统时钟
    pub fn set_sysclk_hz(&mut self, hz: u32) {
        self.sysclk_hz = hz;
        self.channel.set_sysclk_hz(hz);
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
        // SOF 先于本帧的端点事务，与主机桥 poll() 的顺序一致。
        // SOF precedes this window's endpoint transactions, matching `poll()`.
        self.channel.advance_time(cycles);
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
    fn cdc_link_keeps_the_one_millisecond_sof_cadence() {
        // The data phase must keep SOFs flowing, otherwise errata-15 sees a
        // frozen frame number once enumeration is over.
        // 数据阶段必须继续产生 SOF，否则枚举结束后 errata-15 会看到冻结的帧号。
        let controller = ready_controller();
        let channel = CdcChannel::new(controller.clone(), endpoints());
        let mut link = CdcLink::new(channel, |_cycles| {});
        assert_eq!(link.channel().sof_frame(), 0);
        link.advance(500);
        assert_eq!(link.channel().sof_frame(), 0);
        link.advance(500);
        assert_eq!(link.channel().sof_frame(), 1);
        let intr = controller.with_core(|core| core.intr());
        assert_ne!(intr & rp2350_usb::regs::INTR_DEV_SOF, 0);
        link.advance(500);
        assert_eq!(link.channel().sof_frame(), 1);
        link.advance(500);
        assert_eq!(link.channel().sof_frame(), 2);
        assert_eq!(link.channel().now_cycles(), 300_000);
    }

    #[test]
    fn a_cdc_link_at_252_mhz_keeps_the_one_millisecond_cadence() {
        // 252 MHz 下 500 µs 是 126 000 个客户机周期；两轮正好 1 ms，SOF 仍为
        // 1 ms 节拍，且 USB 时钟域每帧仍是 48 000 个周期。
        // At 252 MHz, 500 µs is 126_000 guest cycles; two rounds are exactly
        // 1 ms, so the SOF cadence stays 1 ms and the USB domain still runs
        // 48_000 cycles per frame.
        let controller = ready_controller();
        let channel = CdcChannel::with_clock(controller, endpoints(), 252_000_000, 48_000_000);
        let mut link = CdcLink::with_clock(channel, |_cycles| {}, 252_000_000);
        assert_eq!(link.channel().sof_frame(), 0);
        link.advance(500);
        assert_eq!(link.channel().sof_frame(), 0);
        assert_eq!(link.channel().now_cycles(), 126_000);
        link.advance(500);
        assert_eq!(link.channel().sof_frame(), 1);
        assert_eq!(link.channel().now_cycles(), 252_000);
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
