//! Single- and double-buffered endpoint buffer semantics
//! 单缓冲与双缓冲端点缓冲语义
//!
//! A 32-bit endpoint buffer-control word holds two independent 16-bit buffer
//! descriptors when the endpoint (or EP0 via `SIE_CTRL.EP0_DOUBLE_BUF`) is
//! double buffered: the low half is buffer 0 (`AVAILABLE_0`) and the high half
//! is buffer 1 (`AVAILABLE_1`). Each half carries its own `AVAIL` (owned by the
//! controller), `FULL` (holds a payload) and `LEN` fields, so a single-buffer
//! endpoint must ignore the high half entirely.
//! 端点（或 EP0，经 `SIE_CTRL.EP0_DOUBLE_BUF`）双缓冲时，32 位端点缓冲控制字
//! 承载两个独立的 16 位缓冲描述符：低半为缓冲 0（`AVAILABLE_0`），高半为缓冲 1
//! （`AVAILABLE_1`）。每个半字都有自己的 `AVAIL`（归控制器所有）、`FULL`
//! （持有负载）与 `LEN` 字段，因此单缓冲端点必须完全忽略高半。
//!
//! Transitions modelled here:
//! 此处建模的状态转移：
//!
//! - OUT: the host fills the half selected by the rotation, clears its `AVAIL`
//!   and sets `FULL|LEN`, then raises `BUFF_STATUS`. The half stays owned by the
//!   CPU until the guest re-arms it (`AVAIL`, `FULL=0`).
//!   OUT：主机填充轮转选中的半，清除其 `AVAIL`、置 `FULL|LEN`，并置位
//!   `BUFF_STATUS`。该半在客户机重新布防（写 `AVAIL`、`FULL=0`）之前归 CPU 所有。
//! - IN: the controller sends the armed half, clears `AVAIL|FULL`, raises
//!   `BUFF_STATUS` and returns the buffer to the CPU.
//!   IN：控制器发送已布防的半，清除 `AVAIL|FULL`，置位 `BUFF_STATUS` 并把缓冲
//!   交还 CPU。
//! - `EP_ABORT` discards both halves and their rotation and raises
//!   `EP_ABORT_DONE`.
//!   `EP_ABORT` 丢弃两个半与其轮转状态，并置位 `EP_ABORT_DONE`。

use crate::DPRAM_SIZE;
use crate::device::UsbCore;
use crate::regs::{
    self, BUF_CTRL_AVAIL, BUF_CTRL_FULL, BUF_CTRL_STALL, Direction, EP_ABORT_DONE,
    EP_CTRL_DOUBLE_BUFFERED, EP0_BUF_A, EP0_BUF_B, SIE_CTRL, SIE_CTRL_EP0_DOUBLE_BUF,
    buf_ctrl_half,
};

impl UsbCore {
    /// Abort the endpoints selected by an `EP_ABORT` write.
    /// 中止由 `EP_ABORT` 写选中的端点。
    ///
    /// `EP_ABORT` is not a plain endpoint bitmap: datasheet Table 1317 encodes
    /// one direction per bit as `2*ep + dir` (even = IN, odd = OUT), so bit 15
    /// is `EP7_OUT`, not a direction flag. `EP_ABORT_DONE` (Table 1318) mirrors
    /// the same layout, so the request mask is echoed back unchanged and
    /// clearing only the IN half of EP0 leaves the OUT half untouched.
    /// Both buffer halves and the double-buffer rotation are discarded; the
    /// buffers become free for the guest to re-arm.
    /// `EP_ABORT` 并非纯端点位图：数据手册表 1317 用 `2*ep + dir` 每位编码一个
    /// 方向（偶数为 IN、奇数为 OUT），因此位 15 是 `EP7_OUT` 而不是方向标志。
    /// `EP_ABORT_DONE`（表 1318）沿用同一布局，因此请求掩码原样回写；只中止
    /// EP0 IN 时不会触碰 EP0 OUT。两个缓冲半与双缓冲轮转一并丢弃，缓冲随即
    /// 可被客户机重新布防。
    pub(crate) fn abort_endpoints(&mut self, mask: u32) {
        for ep in 0..16 {
            for dir in [Direction::In, Direction::Out] {
                if mask & regs::ep_abort_bit(ep, dir) == 0 {
                    continue;
                }
                // 32 位写一并清除低/高两个半，双缓冲因此被完整清理。
                // A 32-bit store clears both the low and the high half, so a
                // double-buffered endpoint is fully cleaned up.
                self.write_dpram_u32(regs::ep_buf_ctrl_offset(ep, dir), 0);
                self.set_next_half(ep, dir, false);
            }
        }
        self.set_reg_bits(EP_ABORT_DONE, mask);
    }

    /// Whether an endpoint is configured for double buffering
    /// 端点是否配置为双缓冲
    #[must_use]
    pub(crate) fn double_buffered(&self, ep: usize, dir: Direction) -> bool {
        if ep == 0 {
            self.reg_word(SIE_CTRL) & SIE_CTRL_EP0_DOUBLE_BUF != 0
        } else {
            let offset = regs::ep_ctrl_offset(ep, dir);
            self.read_dpram_u32(offset) & EP_CTRL_DOUBLE_BUFFERED != 0
        }
    }

    /// DPSRAM offset of a buffer half, or `None` when unconfigured
    /// 缓冲半的 DPSRAM 偏移；未配置时返回 `None`
    ///
    /// EP0 uses the fixed `0x100`/`0x140` buffers; EP1..EP15 use the
    /// 64-byte-aligned offset from their endpoint-control register, with the
    /// second half at `offset + 64`.
    /// EP0 使用固定的 `0x100`/`0x140` 缓冲；EP1..EP15 使用端点控制寄存器中的
    /// 64 字节对齐偏移，第二半位于 `offset + 64`。
    #[must_use]
    pub(crate) fn buffer_offset(&self, ep: usize, dir: Direction, half: u8) -> Option<usize> {
        let base = if ep == 0 {
            match half {
                0 => EP0_BUF_A,
                _ if self.double_buffered(0, dir) => EP0_BUF_B,
                _ => return None,
            }
        } else {
            let ctrl = self.read_dpram_u32(regs::ep_ctrl_offset(ep, dir));
            let offset = (ctrl & regs::EP_CTRL_BUFFER_OFFSET_MASK) as usize;
            if offset == 0 {
                return None;
            }
            offset
        };
        let offset = base + usize::from(half) * regs::DOUBLE_BUF_STRIDE;
        (offset + regs::MAX_PACKET_SIZE <= DPRAM_SIZE).then_some(offset)
    }

    /// Whether either half of the given endpoint reports STALL
    /// 给定端点的任一半是否报告 STALL
    #[must_use]
    pub(crate) fn any_stalled(&self, value: u32, ep: usize, dir: Direction) -> bool {
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
    ///
    /// `AVAIL` is the single arming bit for both directions; `STALL` is handled
    /// by [`Self::any_stalled`] before this runs.
    /// 两个方向都以 `AVAIL` 作为唯一布防位；`STALL` 由 [`Self::any_stalled`]
    /// 在此之前处理。
    #[must_use]
    pub(crate) fn armed_half(&self, value: u32, ep: usize, dir: Direction) -> Option<(u8, u32)> {
        let first = u8::from(self.next_half(ep, dir));
        let order: &[u8] = if self.double_buffered(ep, dir) {
            &[first, 1 - first]
        } else {
            &[0]
        };
        for &half in order {
            let ctrl = buf_ctrl_half(value, half);
            if ctrl & BUF_CTRL_AVAIL != 0 {
                return Some((half, ctrl));
            }
        }
        None
    }

    /// Select the OUT half the host may fill next
    /// 选择主机接下来可填充的 OUT 半
    ///
    /// A half is usable only while the guest has handed it to the controller:
    /// `AVAIL=1` **and** `FULL=0`. A half that still holds a payload (`FULL=1`)
    /// belongs to the CPU, so a stale `AVAIL` bit must never let the host
    /// overwrite received data; the endpoint then answers NAK until the guest
    /// re-arms the half.
    /// 只有客户机把半字交给控制器时（`AVAIL=1` **且** `FULL=0`）才可用。仍持有
    /// 负载的半字（`FULL=1`）归 CPU 所有，因此残留的 `AVAIL` 位绝不能允许主机
    /// 覆盖已收数据；在客户机重新布防前该端点应答 NAK。
    #[must_use]
    pub(crate) fn armed_out_half(&self, value: u32, ep: usize) -> Option<(u8, u32)> {
        let first = u8::from(self.next_half(ep, Direction::Out));
        let order: &[u8] = if self.double_buffered(ep, Direction::Out) {
            &[first, 1 - first]
        } else {
            &[0]
        };
        for &half in order {
            let ctrl = buf_ctrl_half(value, half);
            if ctrl & BUF_CTRL_AVAIL != 0 && ctrl & BUF_CTRL_FULL == 0 {
                return Some((half, ctrl));
            }
        }
        None
    }

    /// Advance the double-buffer rotation after consuming `half`
    /// 消费 `half` 后推进双缓冲轮转
    pub(crate) fn schedule_next_half(&mut self, ep: usize, dir: Direction, half: u8) {
        let double = self.double_buffered(ep, dir);
        self.set_next_half(ep, dir, double && half == 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint::OutResponse;
    use crate::regs::{
        self, Alias, BUF_CTRL_LAST, BUF_CTRL_LEN_MASK, EP_ABORT, EP_CTRL_BUFFER_OFFSET_MASK,
        EP_CTRL_ENABLE, INTR_ABORT_DONE, MAIN_CTRL, MAIN_CTRL_CONTROLLER_EN, SIE_CTRL_PULLUP_EN,
        buff_status_bit,
    };

    /// Core with the controller and pull-up enabled and the bus attached
    /// 控制器与上拉已使能、总线已附着的核心
    fn ready_core() -> UsbCore {
        let mut core = UsbCore::new();
        core.write_reg(MAIN_CTRL, Alias::Rw, MAIN_CTRL_CONTROLLER_EN);
        core.write_reg(SIE_CTRL, Alias::Rw, SIE_CTRL_PULLUP_EN);
        core.host_attach();
        core
    }

    /// Configure `ep` as an OUT endpoint at `offset`, optionally double buffered
    /// 把 `ep` 配置为位于 `offset` 的 OUT 端点，可选双缓冲
    fn configure_out(core: &mut UsbCore, ep: usize, offset: u32, double: bool) {
        let mut ctrl = EP_CTRL_ENABLE | (offset & EP_CTRL_BUFFER_OFFSET_MASK);
        if double {
            ctrl |= EP_CTRL_DOUBLE_BUFFERED;
        }
        core.write_dpram_u32(regs::ep_ctrl_offset(ep, Direction::Out), ctrl);
    }

    fn out_ctrl(core: &UsbCore, ep: usize) -> u32 {
        core.read_dpram_u32(regs::ep_buf_ctrl_offset(ep, Direction::Out))
    }

    #[test]
    fn double_buffered_out_fills_both_halves_in_rotation() {
        // Both halves are armed; the host must fill 0 then 1 and raise
        // BUFF_STATUS once per received buffer.
        // 两半都已布防；主机必须先填 0 再填 1，且每个收到的缓冲置位一次
        // BUFF_STATUS。
        let mut core = ready_core();
        configure_out(&mut core, 3, 0x200, true);
        core.write_dpram_u32(
            regs::ep_buf_ctrl_offset(3, Direction::Out),
            BUF_CTRL_AVAIL | (BUF_CTRL_AVAIL << 16),
        );
        let status_bit = buff_status_bit(3, Direction::Out);

        assert_eq!(
            core.host_out(3, &[0x11, 0x22, 0x33]),
            OutResponse::Ack { last: true }
        );
        let value = out_ctrl(&core, 3);
        assert_eq!(buf_ctrl_half(value, 0) & BUF_CTRL_AVAIL, 0);
        assert_eq!(buf_ctrl_half(value, 0) & BUF_CTRL_FULL, BUF_CTRL_FULL);
        assert_eq!(buf_ctrl_half(value, 0) & BUF_CTRL_LEN_MASK, 3);
        assert_eq!(buf_ctrl_half(value, 1) & BUF_CTRL_AVAIL, BUF_CTRL_AVAIL);
        assert_eq!(core.read_dpram_u8(0x200), 0x11);
        assert_ne!(core.read_reg(regs::BUFF_STATUS) & status_bit, 0);
        core.write_reg(regs::BUFF_STATUS, Alias::Rw, status_bit);

        assert_eq!(
            core.host_out(3, &[0x44, 0x55]),
            OutResponse::Ack { last: true }
        );
        let value = out_ctrl(&core, 3);
        // The first half keeps its payload, the second one is now full.
        // 第一半保留负载，第二半此刻变满。
        assert_eq!(buf_ctrl_half(value, 0) & BUF_CTRL_FULL, BUF_CTRL_FULL);
        assert_eq!(buf_ctrl_half(value, 1) & BUF_CTRL_AVAIL, 0);
        assert_eq!(buf_ctrl_half(value, 1) & BUF_CTRL_FULL, BUF_CTRL_FULL);
        assert_eq!(buf_ctrl_half(value, 1) & BUF_CTRL_LEN_MASK, 2);
        assert_eq!(core.read_dpram_u8(0x240), 0x44);
        assert_ne!(core.read_reg(regs::BUFF_STATUS) & status_bit, 0);
    }

    #[test]
    fn double_buffered_out_naks_when_both_halves_hold_data() {
        // A full double buffer back-pressures the host instead of dropping data.
        // 双缓冲全满时对主机背压，而不是丢弃数据。
        let mut core = ready_core();
        configure_out(&mut core, 3, 0x200, true);
        core.write_dpram_u32(
            regs::ep_buf_ctrl_offset(3, Direction::Out),
            BUF_CTRL_AVAIL | (BUF_CTRL_AVAIL << 16),
        );
        assert_eq!(core.host_out(3, &[1]), OutResponse::Ack { last: true });
        assert_eq!(core.host_out(3, &[2]), OutResponse::Ack { last: true });
        assert_eq!(core.host_out(3, &[3]), OutResponse::Nak);
        // The rejected packet did not disturb either buffer.
        // 被拒绝的包没有破坏任一缓冲。
        let value = out_ctrl(&core, 3);
        assert_eq!(buf_ctrl_half(value, 0) & BUF_CTRL_LEN_MASK, 1);
        assert_eq!(buf_ctrl_half(value, 1) & BUF_CTRL_LEN_MASK, 1);
    }

    #[test]
    fn double_buffered_out_rearm_makes_a_half_reusable() {
        // Draining half 0 (write AVAIL, FULL=0) hands it back to the controller.
        // 取走半 0（写 AVAIL、FULL=0）即把它交还控制器。
        let mut core = ready_core();
        configure_out(&mut core, 3, 0x200, true);
        core.write_dpram_u32(
            regs::ep_buf_ctrl_offset(3, Direction::Out),
            BUF_CTRL_AVAIL | (BUF_CTRL_AVAIL << 16),
        );
        assert!(matches!(core.host_out(3, &[0xA1]), OutResponse::Ack { .. }));
        assert!(matches!(core.host_out(3, &[0xB2]), OutResponse::Ack { .. }));
        assert_eq!(core.host_out(3, &[0xC3]), OutResponse::Nak);
        // Guest consumes buffer 0 with a 16-bit write to the low half only.
        // 客户机只对低半做 16 位写，消费缓冲 0。
        core.write_dpram_u16(
            regs::ep_buf_ctrl_offset(3, Direction::Out),
            BUF_CTRL_AVAIL as u16,
        );
        let value = out_ctrl(&core, 3);
        assert_eq!(buf_ctrl_half(value, 0) & BUF_CTRL_FULL, 0);
        assert_eq!(buf_ctrl_half(value, 1) & BUF_CTRL_FULL, BUF_CTRL_FULL);
        // The rotation points at half 0 again, so the next packet lands there.
        // 轮转再次指向半 0，因此下一个包落在半 0。
        assert_eq!(core.host_out(3, &[0xC3]), OutResponse::Ack { last: true });
        assert_eq!(core.read_dpram_u8(0x200), 0xC3);
        assert_eq!(core.read_dpram_u8(0x240), 0xB2);
    }

    #[test]
    fn full_out_half_is_never_overwritten_by_a_stale_avail() {
        // FULL marks CPU ownership: a leftover AVAIL bit must not let the host
        // clobber a payload the guest has not read yet.
        // FULL 表示归 CPU 所有：残留的 AVAIL 位不得让主机覆盖客户机尚未读取的负载。
        let mut core = ready_core();
        configure_out(&mut core, 3, 0x200, true);
        core.write_dpram_u32(
            regs::ep_buf_ctrl_offset(3, Direction::Out),
            BUF_CTRL_AVAIL | BUF_CTRL_FULL | 4,
        );
        assert_eq!(core.host_out(3, &[0xEE]), OutResponse::Nak);
        assert_eq!(out_ctrl(&core, 3), BUF_CTRL_AVAIL | BUF_CTRL_FULL | 4);
    }

    #[test]
    fn single_buffered_out_ignores_the_high_half() {
        // A single-buffered endpoint must behave exactly as before: only the
        // low half is read, written or rotated.
        // 单缓冲端点的行为保持不变：只读写、轮转低半。
        let mut core = ready_core();
        configure_out(&mut core, 3, 0x200, false);
        core.write_dpram_u32(
            regs::ep_buf_ctrl_offset(3, Direction::Out),
            BUF_CTRL_AVAIL | (BUF_CTRL_AVAIL << 16) | (BUF_CTRL_LAST << 16),
        );
        assert_eq!(
            core.host_out(3, &[0x77, 0x88]),
            OutResponse::Ack { last: true }
        );
        let value = out_ctrl(&core, 3);
        assert_eq!(buf_ctrl_half(value, 0) & BUF_CTRL_LEN_MASK, 2);
        assert_eq!(buf_ctrl_half(value, 0) & BUF_CTRL_FULL, BUF_CTRL_FULL);
        // The untouched high half keeps its software-written contents.
        // 未被触碰的高半保留软件写入的内容。
        assert_eq!(buf_ctrl_half(value, 1), BUF_CTRL_AVAIL | BUF_CTRL_LAST);
        assert!(!core.next_half(3, Direction::Out));
    }

    #[test]
    fn abort_clears_both_halves_and_sets_ep_abort_done() {
        // EP_ABORT must discard a double buffer entirely, not just one half.
        // EP_ABORT 必须彻底丢弃双缓冲，而不是只清一个半。
        let mut core = ready_core();
        configure_out(&mut core, 3, 0x200, true);
        core.write_dpram_u32(
            regs::ep_buf_ctrl_offset(3, Direction::Out),
            (BUF_CTRL_FULL | 4) | ((BUF_CTRL_AVAIL | 8) << 16),
        );
        let bit = regs::ep_abort_bit(3, Direction::Out);
        core.write_reg(EP_ABORT, Alias::Rw, bit);
        assert_eq!(out_ctrl(&core, 3), 0);
        assert_eq!(core.read_reg(EP_ABORT_DONE) & bit, bit);
        assert_ne!(core.intr() & INTR_ABORT_DONE, 0);
    }

    #[test]
    fn abort_resets_the_double_buffer_rotation() {
        // After an abort the next packet must start from buffer 0 again.
        // 中止之后下一个包必须重新从缓冲 0 开始。
        let mut core = ready_core();
        configure_out(&mut core, 3, 0x200, true);
        core.write_dpram_u32(
            regs::ep_buf_ctrl_offset(3, Direction::Out),
            BUF_CTRL_AVAIL | (BUF_CTRL_AVAIL << 16),
        );
        assert!(matches!(core.host_out(3, &[1]), OutResponse::Ack { .. }));
        assert!(core.next_half(3, Direction::Out));
        core.write_reg(EP_ABORT, Alias::Rw, regs::ep_abort_bit(3, Direction::Out));
        assert!(!core.next_half(3, Direction::Out));
        core.write_dpram_u32(
            regs::ep_buf_ctrl_offset(3, Direction::Out),
            BUF_CTRL_AVAIL | (BUF_CTRL_AVAIL << 16),
        );
        assert!(matches!(
            core.host_out(3, &[0x99, 0x9A]),
            OutResponse::Ack { .. }
        ));
        let value = out_ctrl(&core, 3);
        assert_eq!(buf_ctrl_half(value, 0) & BUF_CTRL_LEN_MASK, 2);
        assert_eq!(buf_ctrl_half(value, 1) & BUF_CTRL_AVAIL, BUF_CTRL_AVAIL);
    }

    #[test]
    fn ep0_double_buffered_abort_clears_both_fixed_buffers() {
        // EP0 double buffering is gated by SIE_CTRL; abort still clears both
        // descriptors at 0x080/0x084.
        // EP0 双缓冲由 SIE_CTRL 控制；中止仍会清除 0x080/0x084 的两个描述符。
        let mut core = ready_core();
        core.write_reg(SIE_CTRL, Alias::Set, SIE_CTRL_EP0_DOUBLE_BUF);
        core.write_dpram_u32(regs::ep_buf_ctrl_offset(0, Direction::Out), 0x0400_0C00);
        core.write_reg(EP_ABORT, Alias::Rw, regs::ep_abort_bit(0, Direction::Out));
        assert_eq!(
            core.read_dpram_u32(regs::ep_buf_ctrl_offset(0, Direction::Out)),
            0
        );
        assert_eq!(
            core.read_reg(EP_ABORT_DONE),
            regs::ep_abort_bit(0, Direction::Out)
        );
    }

    #[test]
    fn abort_requests_complete_immediately() {
        let mut core = ready_core();
        core.write_dpram_u32(regs::ep_buf_ctrl_offset(1, Direction::Out), BUF_CTRL_AVAIL);
        let bit = regs::ep_abort_bit(1, Direction::Out);
        core.write_reg(EP_ABORT, Alias::Rw, bit);
        assert_eq!(
            core.read_dpram_u32(regs::ep_buf_ctrl_offset(1, Direction::Out)),
            0
        );
        assert_ne!(core.read_reg(EP_ABORT_DONE) & bit, 0);
        assert_ne!(core.intr() & INTR_ABORT_DONE, 0);
        core.write_reg(EP_ABORT_DONE, Alias::Rw, bit);
        assert_eq!(core.read_reg(EP_ABORT_DONE), 0);
    }

    #[test]
    fn ep0_in_abort_leaves_ep0_out_intact() {
        // Bit 0 of EP_ABORT is EP0_IN only (datasheet Table 1317). Treating it
        // as a plain endpoint number used to clear EP0 OUT as well, which lost
        // the OUT status-stage buffer.
        // EP_ABORT 的位 0 仅代表 EP0_IN（数据手册表 1317）。若按纯端点号解码
        // 会连带清除 EP0 OUT，从而丢失 OUT 状态阶段的缓冲。
        let mut core = ready_core();
        core.write_dpram_u32(regs::ep_buf_ctrl_offset(0, Direction::In), BUF_CTRL_AVAIL);
        let out_ctrl_value = BUF_CTRL_AVAIL | BUF_CTRL_FULL | 3;
        core.write_dpram_u32(regs::ep_buf_ctrl_offset(0, Direction::Out), out_ctrl_value);
        core.write_reg(EP_ABORT, Alias::Rw, regs::ep_abort_bit(0, Direction::In));
        assert_eq!(
            core.read_dpram_u32(regs::ep_buf_ctrl_offset(0, Direction::In)),
            0
        );
        assert_eq!(
            core.read_dpram_u32(regs::ep_buf_ctrl_offset(0, Direction::Out)),
            out_ctrl_value
        );
        assert_eq!(
            core.read_reg(EP_ABORT_DONE),
            regs::ep_abort_bit(0, Direction::In)
        );
    }

    #[test]
    fn directional_abort_hits_only_the_encoded_endpoint() {
        let mut core = ready_core();
        let ep1_in = BUF_CTRL_AVAIL;
        let ep2_in = BUF_CTRL_AVAIL | BUF_CTRL_FULL;
        let ep2_out = BUF_CTRL_AVAIL | BUF_CTRL_FULL | 64;
        core.write_dpram_u32(regs::ep_buf_ctrl_offset(1, Direction::In), ep1_in);
        core.write_dpram_u32(regs::ep_buf_ctrl_offset(2, Direction::In), ep2_in);
        core.write_dpram_u32(regs::ep_buf_ctrl_offset(2, Direction::Out), ep2_out);
        // EP2 OUT is numeric bit 5, not bit 2.
        let bit = regs::ep_abort_bit(2, Direction::Out);
        assert_eq!(bit, 1 << 5);
        core.write_reg(EP_ABORT, Alias::Rw, bit);
        assert_eq!(
            core.read_dpram_u32(regs::ep_buf_ctrl_offset(2, Direction::Out)),
            0
        );
        assert_eq!(
            core.read_dpram_u32(regs::ep_buf_ctrl_offset(2, Direction::In)),
            ep2_in
        );
        assert_eq!(
            core.read_dpram_u32(regs::ep_buf_ctrl_offset(1, Direction::In)),
            ep1_in
        );
        assert_eq!(core.read_reg(EP_ABORT_DONE), bit);
        assert!(!core.next_half(2, Direction::Out));
    }

    #[test]
    fn abort_bit_fifteen_is_ep7_out_not_a_direction_flag() {
        // Bit 15 is EP7_OUT in the datasheet layout; it must not be decoded as
        // "EP0 with direction = IN" or as a 15-bit endpoint number.
        // 数据手册布局中位 15 是 EP7_OUT，不能解码为“EP0 且方向为 IN”或
        // 15 号端点。
        let mut core = ready_core();
        core.write_dpram_u32(regs::ep_buf_ctrl_offset(7, Direction::In), BUF_CTRL_AVAIL);
        core.write_dpram_u32(regs::ep_buf_ctrl_offset(7, Direction::Out), BUF_CTRL_AVAIL);
        core.write_reg(EP_ABORT, Alias::Rw, 1 << 15);
        assert_eq!(
            core.read_dpram_u32(regs::ep_buf_ctrl_offset(7, Direction::Out)),
            0
        );
        assert_eq!(
            core.read_dpram_u32(regs::ep_buf_ctrl_offset(7, Direction::In)),
            BUF_CTRL_AVAIL
        );
        assert_eq!(core.read_reg(EP_ABORT_DONE), 1 << 15);
    }
}
