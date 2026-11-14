//! Host-facing endpoint transactions (IN / OUT / SETUP)
//! 面向主机的端点事务（IN / OUT / SETUP）

use crate::device::UsbCore;
use crate::regs::{
    self, BUF_CTRL_AVAIL, BUF_CTRL_FULL, BUF_CTRL_LAST, BUF_CTRL_LEN_MASK, BUF_CTRL_STALL,
    Direction, MAX_PACKET_SIZE, SETUP_PACKET, buf_ctrl_half, buf_ctrl_set_half,
};
use crate::setup::SetupPacket;

/// A full-speed IN packet captured from the device buffer
/// 从设备缓冲捕获的全速 IN 包
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InPacket {
    data: [u8; MAX_PACKET_SIZE],
    len: usize,
    /// Whether the device marked this buffer as the last of the transfer
    /// 设备是否将该缓冲标记为传输的最后一个
    pub last: bool,
}

impl Default for InPacket {
    fn default() -> Self {
        Self {
            data: [0; MAX_PACKET_SIZE],
            len: 0,
            last: false,
        }
    }
}

impl InPacket {
    /// Captured payload / 捕获的负载
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.data[..self.len]
    }

    /// Payload length / 负载长度
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the payload is empty / 负载是否为空
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Outcome of an IN token / IN 令牌的结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InResponse {
    /// No device is currently able to answer / 当前无设备可应答
    NoDevice,
    /// Endpoint not armed yet / 端点尚未布防
    Nak,
    /// Endpoint is stalled / 端点处于 STALL
    Stall,
    /// Data delivered to the host / 已向主机交付数据
    Data(InPacket),
}

/// Outcome of an OUT token / OUT 令牌的结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutResponse {
    /// No device is currently able to answer / 当前无设备可应答
    NoDevice,
    /// Endpoint not armed yet / 端点尚未布防
    Nak,
    /// Endpoint is stalled / 端点处于 STALL
    Stall,
    /// Data accepted; `last` marks the end of the transfer
    /// 数据已被接收；`last` 表示传输结束
    Ack {
        /// Whether the packet terminated the transfer / 该包是否结束传输
        last: bool,
    },
}

impl UsbCore {
    /// Deliver an IN token to endpoint `ep`.
    /// 向端点 `ep` 送达一个 IN 令牌。
    ///
    /// Consumes whichever armed half is next in the double-buffer rotation,
    /// clears `AVAIL|FULL`, raises `BUFF_STATUS` and returns the payload.
    /// 消费双缓冲轮转中最先就绪的已布防半，清除 `AVAIL|FULL`，置位
    /// `BUFF_STATUS` 并返回负载。
    pub fn host_in(&mut self, ep: usize) -> InResponse {
        if !self.transaction_ready() {
            return InResponse::NoDevice;
        }
        if ep > 15 {
            return InResponse::NoDevice;
        }
        let offset = regs::ep_buf_ctrl_offset(ep, Direction::In);
        let value = self.read_dpram_u32(offset);
        let double = self.double_buffered(ep, Direction::In);
        if self.any_stalled(value, ep, Direction::In) {
            return InResponse::Stall;
        }
        let Some((half, ctrl)) = self.armed_half(value, double, ep, Direction::In) else {
            return InResponse::Nak;
        };
        let Some(buffer) = self.buffer_offset(ep, Direction::In, half) else {
            return InResponse::Nak;
        };
        let len = (ctrl & BUF_CTRL_LEN_MASK) as usize;
        let len = len.min(MAX_PACKET_SIZE);
        let mut packet = InPacket {
            data: [0; MAX_PACKET_SIZE],
            len,
            last: ctrl & BUF_CTRL_LAST != 0,
        };
        packet.data[..len].copy_from_slice(&self.dpram()[buffer..buffer + len]);
        let cleared = ctrl & !(BUF_CTRL_AVAIL | BUF_CTRL_FULL);
        let updated = buf_ctrl_set_half(value, half, cleared);
        self.set_buf_ctrl(offset, updated);
        self.schedule_next_half(ep, Direction::In, half, double);
        self.complete_buffer(ep, Direction::In, packet.last);
        InResponse::Data(packet)
    }

    /// Deliver an OUT token carrying `data` to endpoint `ep`.
    /// 向端点 `ep` 送达携带 `data` 的 OUT 令牌。
    ///
    /// Copies at most 64 bytes into the armed buffer, marks it full and raises
    /// `BUFF_STATUS`. A short packet (or an armed `LAST`) ends the transfer.
    /// 将最多 64 字节复制到已布防缓冲，标记为满并置位 `BUFF_STATUS`。短包
    /// （或已布防的 `LAST`）将结束本次传输。
    pub fn host_out(&mut self, ep: usize, data: &[u8]) -> OutResponse {
        if !self.transaction_ready() {
            return OutResponse::NoDevice;
        }
        if ep > 15 {
            return OutResponse::NoDevice;
        }
        let offset = regs::ep_buf_ctrl_offset(ep, Direction::Out);
        let value = self.read_dpram_u32(offset);
        let double = self.double_buffered(ep, Direction::Out);
        if self.any_stalled(value, ep, Direction::Out) {
            return OutResponse::Stall;
        }
        let Some((half, ctrl)) = self.armed_half(value, double, ep, Direction::Out) else {
            return OutResponse::Nak;
        };
        let Some(buffer) = self.buffer_offset(ep, Direction::Out, half) else {
            return OutResponse::Nak;
        };
        let len = data.len().min(MAX_PACKET_SIZE);
        for (i, byte) in data[..len].iter().enumerate() {
            self.write_dpram_u8(buffer + i, *byte);
        }
        let last = len < MAX_PACKET_SIZE || ctrl & BUF_CTRL_LAST != 0;
        let mut updated = ctrl & !(BUF_CTRL_AVAIL | BUF_CTRL_FULL | BUF_CTRL_LEN_MASK);
        updated |= BUF_CTRL_FULL | (len as u32 & BUF_CTRL_LEN_MASK);
        let updated = buf_ctrl_set_half(value, half, updated);
        self.set_buf_ctrl(offset, updated);
        self.schedule_next_half(ep, Direction::Out, half, double);
        self.complete_buffer(ep, Direction::Out, last);
        OutResponse::Ack { last }
    }

    /// Deliver a SETUP packet to endpoint 0.
    /// 向端点 0 送达 SETUP 包。
    ///
    /// Writes the 8 packet bytes to DPSRAM `0x000`, aborts pending EP0 buffers,
    /// clears the defined `EP_STALL_ARM` bits (USB 2.0 requires a SETUP to
    /// clear an EP0 STALL) and raises `SETUP_REQ`. Returns `false` when the
    /// device is not attached.
    /// 将 8 字节包写入 DPSRAM `0x000`，中止未完成的 EP0 缓冲，清除
    /// `EP_STALL_ARM` 的已定义位（USB 2.0 要求 SETUP 清除 EP0 的 STALL）
    /// 并置位 `SETUP_REQ`。设备未附着时返回 `false`。
    pub fn host_setup(&mut self, packet: &SetupPacket) -> bool {
        if !self.transaction_ready() {
            return false;
        }
        let bytes = packet.to_bytes();
        for (i, byte) in bytes.iter().enumerate() {
            self.write_dpram_u8(SETUP_PACKET + i, *byte);
        }
        self.clear_buffers(0);
        self.raise_setup();
        true
    }

    /// The SETUP packet stored in DPSRAM (`0x000..0x008`) / DPSRAM 中保存的 SETUP 包
    #[must_use]
    pub fn setup_packet(&self) -> SetupPacket {
        let mut bytes = [0u8; 8];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = self.read_dpram_u8(SETUP_PACKET + i);
        }
        SetupPacket::from_bytes(&bytes)
    }

    /// Clear `SIE_STATUS.SETUP_REC` after the guest consumed the packet
    /// 客户机取走 SETUP 包后清除 `SIE_STATUS.SETUP_REC`
    pub fn acknowledge_setup(&mut self) {
        self.clear_sie_bits(regs::SIE_STATUS_SETUP_REC);
    }

    /// Whether either half of the given endpoint reports STALL
    /// 给定端点的任一半是否报告 STALL
    fn any_stalled(&self, value: u32, ep: usize, dir: Direction) -> bool {
        let halves: &[u8] = if self.double_buffered(ep, dir) {
            &[0, 1]
        } else {
            &[0]
        };
        halves
            .iter()
            .any(|half| buf_ctrl_half(value, *half) & BUF_CTRL_STALL != 0)
    }

    /// Select the first armed half in the rotation order
    /// 按轮转顺序选择第一个已布防的半
    fn armed_half(&self, value: u32, double: bool, ep: usize, dir: Direction) -> Option<(u8, u32)> {
        if !double {
            let ctrl = buf_ctrl_half(value, 0);
            return (ctrl & BUF_CTRL_AVAIL != 0).then_some((0, ctrl));
        }
        let first = u8::from(self.next_half(ep, dir));
        for half in [first, 1 - first] {
            let ctrl = buf_ctrl_half(value, half);
            if ctrl & BUF_CTRL_AVAIL != 0 {
                return Some((half, ctrl));
            }
        }
        None
    }

    /// Advance the double-buffer rotation after consuming `half`
    /// 消费 `half` 后推进双缓冲轮转
    fn schedule_next_half(&mut self, ep: usize, dir: Direction, half: u8, double: bool) {
        self.set_next_half(ep, dir, double && half == 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regs::{
        self, Alias, Direction, EP_CTRL_BUFFER_OFFSET_MASK, EP_CTRL_DOUBLE_BUFFERED,
        EP_CTRL_ENABLE, SIE_CTRL_PULLUP_EN,
    };

    /// Build a core with the controller and pull-up enabled and bus attached.
    fn ready_core() -> UsbCore {
        let mut core = UsbCore::new();
        core.write_reg(regs::MAIN_CTRL, Alias::Rw, regs::MAIN_CTRL_CONTROLLER_EN);
        core.write_reg(regs::SIE_CTRL, Alias::Rw, SIE_CTRL_PULLUP_EN);
        core.host_attach();
        core
    }

    /// Configure a bulk IN endpoint with one buffer at `offset`.
    fn configure_in_ep(core: &mut UsbCore, ep: usize, offset: usize) {
        let ctrl = EP_CTRL_ENABLE | ((offset as u32) & EP_CTRL_BUFFER_OFFSET_MASK);
        core.write_dpram_u32(regs::ep_ctrl_offset(ep, Direction::In), ctrl);
        core.write_dpram_u32(regs::ep_buf_ctrl_offset(ep, Direction::In), BUF_CTRL_AVAIL);
    }

    #[test]
    fn in_transaction_delivers_and_clears_avail() {
        let mut core = ready_core();
        configure_in_ep(&mut core, 1, 0x180);
        for i in 0..8 {
            core.write_dpram_u8(0x180 + i, i as u8);
        }
        let ctrl = BUF_CTRL_AVAIL | BUF_CTRL_FULL | 8 | BUF_CTRL_LAST;
        core.write_dpram_u32(regs::ep_buf_ctrl_offset(1, Direction::In), ctrl);
        let InResponse::Data(packet) = core.host_in(1) else {
            panic!("expected data");
        };
        assert_eq!(packet.bytes(), &[0, 1, 2, 3, 4, 5, 6, 7]);
        assert!(packet.last);
        assert_eq!(
            core.read_dpram_u32(regs::ep_buf_ctrl_offset(1, Direction::In)) & BUF_CTRL_AVAIL,
            0
        );
        assert_ne!(core.read_reg(regs::BUFF_STATUS), 0);
        assert_eq!(core.host_in(1), InResponse::Nak);
    }

    #[test]
    fn out_transaction_stores_payload_and_reports_short_packet() {
        let mut core = ready_core();
        let ctrl = EP_CTRL_ENABLE | (0x1C0 & EP_CTRL_BUFFER_OFFSET_MASK);
        core.write_dpram_u32(regs::ep_ctrl_offset(1, Direction::Out), ctrl);
        core.write_dpram_u32(regs::ep_buf_ctrl_offset(1, Direction::Out), BUF_CTRL_AVAIL);
        let response = core.host_out(1, &[0xAA, 0xBB, 0xCC]);
        assert_eq!(response, OutResponse::Ack { last: true });
        let stored = core.read_dpram_u32(regs::ep_buf_ctrl_offset(1, Direction::Out));
        assert_ne!(stored & BUF_CTRL_FULL, 0);
        assert_eq!(stored & BUF_CTRL_LEN_MASK, 3);
        assert_eq!(stored & BUF_CTRL_AVAIL, 0);
        assert_eq!(core.read_dpram_u8(0x1C0), 0xAA);
        assert_eq!(core.read_dpram_u8(0x1C2), 0xCC);
    }

    #[test]
    fn out_transaction_replaces_stale_length_bits() {
        // The DCD arms an OUT endpoint with a transfer size in LEN (for
        // example 64); real hardware overwrites that field with the received
        // byte count. Leaving the old bits OR-ed in makes every short packet
        // look full-sized, and the guest reads trailing DPRAM zeros as data.
        // DCD 会用 LEN 字段表示 OUT 传输大小（例如 64）；真实硬件收包后会
        // 用实际字节数覆盖该字段。若保留旧位做 OR，短包会被当成满长度包，
        // 客户机就会把 DPRAM 尾部的零读成数据。
        let mut core = ready_core();
        let ctrl = EP_CTRL_ENABLE | (0x1C0 & EP_CTRL_BUFFER_OFFSET_MASK);
        core.write_dpram_u32(regs::ep_ctrl_offset(1, Direction::Out), ctrl);
        core.write_dpram_u32(
            regs::ep_buf_ctrl_offset(1, Direction::Out),
            BUF_CTRL_AVAIL | 64,
        );
        let response = core.host_out(1, &[0xAA, 0xBB, 0xCC]);
        assert_eq!(response, OutResponse::Ack { last: true });
        let stored = core.read_dpram_u32(regs::ep_buf_ctrl_offset(1, Direction::Out));
        assert_ne!(stored & BUF_CTRL_FULL, 0);
        assert_eq!(stored & BUF_CTRL_LEN_MASK, 3);
    }

    #[test]
    fn setup_clears_only_defined_ep0_stall_arm_bits() {
        // Datasheet 12.7.3.8.1: the controller clears EP_STALL_ARM on SETUP
        // because USB 2.0 requires a SETUP to clear an EP0 STALL condition.
        // 数据手册 12.7.3.8.1：SETUP 到达时控制器清除 EP_STALL_ARM，因为
        // USB 2.0 要求 SETUP 清除 EP0 的 STALL 条件。
        let mut core = ready_core();
        core.write_reg(regs::EP_STALL_ARM, Alias::Rw, 0b111);
        let packet = SetupPacket::standard_in(0x06, 0x0100, 0, 18);
        assert!(core.host_setup(&packet));
        assert_eq!(core.read_reg(regs::EP_STALL_ARM) & 0b11, 0);
        // Bits 31:2 are reserved and outside the model's clear mask.
        // 31:2 为保留位，不在模型的清除掩码内。
        assert_ne!(core.read_reg(regs::EP_STALL_ARM) & 0b100, 0);
    }

    #[test]
    fn setup_packet_is_stored_and_flagged() {
        let mut core = ready_core();
        let packet = SetupPacket::standard_in(0x06, 0x0100, 0, 18);
        assert!(core.host_setup(&packet));
        assert_eq!(core.setup_packet(), packet);
        assert_ne!(
            core.read_reg(regs::SIE_STATUS) & regs::SIE_STATUS_SETUP_REC,
            0
        );
        assert_ne!(core.intr() & regs::INTR_SETUP_REQ, 0);
        core.acknowledge_setup();
        assert_eq!(
            core.read_reg(regs::SIE_STATUS) & regs::SIE_STATUS_SETUP_REC,
            0
        );
    }

    #[test]
    fn transactions_require_an_attached_device() {
        let mut core = UsbCore::new();
        assert_eq!(core.host_in(1), InResponse::NoDevice);
        assert_eq!(core.host_out(1, &[]), OutResponse::NoDevice);
        assert!(!core.host_setup(&SetupPacket::standard_in(6, 0x100, 0, 8)));
    }

    #[test]
    fn double_buffered_in_alternates_halves() {
        let mut core = ready_core();
        let ctrl = EP_CTRL_ENABLE | EP_CTRL_DOUBLE_BUFFERED | (0x180 & EP_CTRL_BUFFER_OFFSET_MASK);
        core.write_dpram_u32(regs::ep_ctrl_offset(1, Direction::In), ctrl);
        for i in 0..4 {
            core.write_dpram_u8(0x180 + i, 0xA0 + i as u8);
            core.write_dpram_u8(0x1C0 + i, 0xB0 + i as u8);
        }
        let half_a = BUF_CTRL_AVAIL | BUF_CTRL_FULL | 4;
        let half_b = BUF_CTRL_AVAIL | BUF_CTRL_FULL | 4;
        let both = half_a | (half_b << 16);
        core.write_dpram_u32(regs::ep_buf_ctrl_offset(1, Direction::In), both);
        let InResponse::Data(first) = core.host_in(1) else {
            panic!("expected first packet");
        };
        assert_eq!(first.bytes(), &[0xA0, 0xA1, 0xA2, 0xA3]);
        assert!(core.next_half(1, Direction::In));
        let InResponse::Data(second) = core.host_in(1) else {
            panic!("expected second packet");
        };
        assert_eq!(second.bytes(), &[0xB0, 0xB1, 0xB2, 0xB3]);
        assert!(!core.next_half(1, Direction::In));
        assert_eq!(
            core.read_dpram_u32(regs::ep_buf_ctrl_offset(1, Direction::In)) & 0xC000_C000,
            0
        );
    }

    #[test]
    fn stall_is_reported_before_data() {
        let mut core = ready_core();
        configure_in_ep(&mut core, 2, 0x200);
        core.write_dpram_u32(
            regs::ep_buf_ctrl_offset(2, Direction::In),
            BUF_CTRL_AVAIL | BUF_CTRL_FULL | BUF_CTRL_STALL,
        );
        assert_eq!(core.host_in(2), InResponse::Stall);
    }

    #[test]
    fn ep0_shares_buffer_zero() {
        let mut core = ready_core();
        core.write_dpram_u32(
            regs::ep_buf_ctrl_offset(0, Direction::In),
            BUF_CTRL_AVAIL | 3,
        );
        for i in 0..3 {
            core.write_dpram_u8(UsbCore::ep0_buffer() + i, 0x55 + i as u8);
        }
        let InResponse::Data(packet) = core.host_in(0) else {
            panic!("expected EP0 data");
        };
        assert_eq!(packet.bytes(), &[0x55, 0x56, 0x57]);
        // The buffer address constant must equal the documented layout.
        assert_eq!(UsbCore::ep0_buffer(), 0x100);
        assert_eq!(UsbCore::ep_buf_ctrl_base(), 0x080);
        assert_eq!(UsbCore::ep_ctrl_base(), 0x008);
        assert_eq!(UsbCore::setup_packet_offset(), 0x000);
    }
}
