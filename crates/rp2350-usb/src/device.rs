//! Controller state machine: DPSRAM, register shadow and bus events
//! 控制器状态机：DPSRAM、寄存器影子与总线事件

use crate::DPRAM_SIZE;
use crate::regs::{
    self, Alias, BUFF_CPU_SHOULD_HANDLE, BUFF_STATUS, Direction, EP_ABORT, EP_ABORT_DONE,
    EP_BUF_CTRL_BASE, EP_CTRL_BASE, EP_CTRL_DOUBLE_BUFFERED, EP_STATUS_STALL_NAK, EP0_BUF_A,
    EP0_BUF_B, INTE, INTF, INTR, INTR_ABORT_DONE, INTR_BUFF_STATUS, INTR_BUS_RESET,
    INTR_DEV_CONN_DIS, INTR_DEV_RESUME_FROM_HOST, INTR_DEV_SOF, INTR_DEV_SUSPEND,
    INTR_EP_STALL_NAK, INTR_IMPLEMENTED_MASK, INTR_SETUP_REQ, INTR_TRANS_COMPLETE,
    INTR_VBUS_DETECT, INTS, MAIN_CTRL, MAIN_CTRL_CONTROLLER_EN, REG_WORD_COUNT, SETUP_PACKET,
    SIE_CTRL, SIE_CTRL_EP0_DOUBLE_BUF, SIE_CTRL_PULLUP_EN, SIE_STATUS, SIE_STATUS_BUS_RESET,
    SIE_STATUS_CONNECTED, SIE_STATUS_RESUME, SIE_STATUS_SETUP_REC, SIE_STATUS_SUSPENDED,
    SIE_STATUS_TRANS_COMPLETE, SIE_STATUS_VBUS_DETECTED, SIE_STATUS_W1C_MASK, SOF_RD, SOF_WR,
};

/// Pure device-side USB controller state.
/// 纯设备侧 USB 控制器状态。
///
/// The core owns the 4 KB DPSRAM and the control register shadow. Guest MMIO
/// accesses go through [`Self::read_reg`] / [`Self::write_reg`]; host-side
/// transactions go through the `host_*` methods implemented in
/// [`crate::endpoint`]. No locking is involved, so it can be unit tested
/// without a CPU.
/// 该核心持有 4 KB DPSRAM 与控制寄存器影子。客户机 MMIO 访问经由
/// [`Self::read_reg`] / [`Self::write_reg`]；主机侧事务经由
/// [`crate::endpoint`] 中的 `host_*` 方法。核心不含锁，因此无需 CPU 即可单测。
pub struct UsbCore {
    regs: [u32; REG_WORD_COUNT],
    dpram: [u8; DPRAM_SIZE],
    /// Which double-buffer half each (endpoint, direction) should serve next.
    /// 每个（端点，方向）下一次应优先服务的双缓冲半。
    next_high: [[bool; 2]; 16],
    attached_once: bool,
    conn_dis_pending: bool,
    dev_sof_pending: bool,
}

impl Default for UsbCore {
    fn default() -> Self {
        Self::new()
    }
}

impl UsbCore {
    /// Create a controller in its power-on reset state
    /// 创建处于上电复位状态的控制器
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: [0; REG_WORD_COUNT],
            dpram: [0; DPRAM_SIZE],
            next_high: [[false; 2]; 16],
            attached_once: false,
            conn_dis_pending: false,
            dev_sof_pending: false,
        }
    }

    /// Reset all registers, endpoint state and DPSRAM contents
    /// 复位所有寄存器、端点状态与 DPSRAM 内容
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    // -----------------------------------------------------------------
    // DPSRAM access / DPSRAM 访问
    // -----------------------------------------------------------------

    /// Whole DPSRAM contents (host bridge inspection) / 整个 DPSRAM 内容
    #[must_use]
    pub fn dpram(&self) -> &[u8] {
        &self.dpram
    }

    /// Read one DPSRAM byte / 读取一个 DPSRAM 字节
    #[must_use]
    pub fn read_dpram_u8(&self, offset: usize) -> u8 {
        self.dpram.get(offset).copied().unwrap_or(0)
    }

    /// Write one DPSRAM byte / 写入一个 DPSRAM 字节
    pub fn write_dpram_u8(&mut self, offset: usize, value: u8) {
        if let Some(slot) = self.dpram.get_mut(offset) {
            *slot = value;
        }
    }

    /// Read a 16-bit DPSRAM halfword / 读取 16 位 DPSRAM 半字
    #[must_use]
    pub fn read_dpram_u16(&self, offset: usize) -> u16 {
        let lo = u16::from(self.read_dpram_u8(offset));
        let hi = u16::from(self.read_dpram_u8(offset + 1));
        lo | (hi << 8)
    }

    /// Write a 16-bit DPSRAM halfword / 写入 16 位 DPSRAM 半字
    pub fn write_dpram_u16(&mut self, offset: usize, value: u16) {
        self.write_dpram_u8(offset, value as u8);
        self.write_dpram_u8(offset + 1, (value >> 8) as u8);
    }

    /// Read a 32-bit DPSRAM word / 读取 32 位 DPSRAM 字
    #[must_use]
    pub fn read_dpram_u32(&self, offset: usize) -> u32 {
        let lo = u32::from(self.read_dpram_u16(offset));
        let hi = u32::from(self.read_dpram_u16(offset + 2));
        lo | (hi << 16)
    }

    /// Write a 32-bit DPSRAM word / 写入 32 位 DPSRAM 字
    pub fn write_dpram_u32(&mut self, offset: usize, value: u32) {
        self.write_dpram_u16(offset, value as u16);
        self.write_dpram_u16(offset + 2, (value >> 16) as u16);
    }

    // -----------------------------------------------------------------
    // Register access / 寄存器访问
    // -----------------------------------------------------------------

    /// Read a register word, applying read side effects
    /// 读取寄存器字，并应用读副作用
    pub fn read_reg(&mut self, offset: usize) -> u32 {
        match offset {
            INTR => self.intr(),
            INTS => self.ints(),
            SOF_RD => {
                self.dev_sof_pending = false;
                self.reg_word(SOF_RD) & 0x7FF
            }
            BUFF_CPU_SHOULD_HANDLE => self.buff_cpu_should_handle(),
            _ => self.reg_word(offset),
        }
    }

    /// Write a register word through the selected atomic alias
    /// 通过选定的原子别名写入寄存器字
    pub fn write_reg(&mut self, offset: usize, alias: Alias, value: u32) {
        self.write_reg_masked(offset, alias, value, u32::MAX);
    }

    /// Write a masked register word; `mask` selects the affected bit lanes
    /// 写入带掩码的寄存器字；`mask` 选择受影响的位通道
    pub fn write_reg_masked(&mut self, offset: usize, alias: Alias, value: u32, mask: u32) {
        let index = offset / 4;
        if index >= REG_WORD_COUNT {
            return;
        }
        let masked = value & mask;
        match offset {
            EP_ABORT => {
                if masked != 0 {
                    self.abort_endpoints(masked);
                }
            }
            _ if alias != Alias::Rw => match alias {
                Alias::Xor => self.regs[index] ^= masked,
                Alias::Set => self.regs[index] |= masked,
                Alias::Clear => self.regs[index] &= !masked,
                Alias::Rw => unreachable!(),
            },
            BUFF_STATUS | EP_ABORT_DONE | EP_STATUS_STALL_NAK => {
                self.regs[index] &= !masked;
            }
            SIE_STATUS => {
                self.regs[index] &= !(masked & SIE_STATUS_W1C_MASK);
            }
            INTR | INTS => {}
            SOF_WR => self.regs[index] = masked & 0x7FF,
            _ => self.regs[index] = (self.regs[index] & !mask) | masked,
        }
    }

    /// Raw interrupt status word (`INTR`) / 原始中断状态字
    #[must_use]
    pub fn intr(&self) -> u32 {
        let sie = self.reg_word(SIE_STATUS);
        let mut value = 0u32;
        if self.reg_word(BUFF_STATUS) != 0 {
            value |= INTR_BUFF_STATUS;
        }
        if self.reg_word(EP_ABORT_DONE) != 0 {
            value |= INTR_ABORT_DONE;
        }
        if self.reg_word(EP_STATUS_STALL_NAK) != 0 {
            value |= INTR_EP_STALL_NAK;
        }
        if sie & SIE_STATUS_BUS_RESET != 0 {
            value |= INTR_BUS_RESET;
        }
        if sie & SIE_STATUS_SETUP_REC != 0 {
            value |= INTR_SETUP_REQ;
        }
        if sie & SIE_STATUS_TRANS_COMPLETE != 0 {
            value |= INTR_TRANS_COMPLETE;
        }
        if sie & SIE_STATUS_VBUS_DETECTED != 0 {
            value |= INTR_VBUS_DETECT;
        }
        if sie & SIE_STATUS_SUSPENDED != 0 {
            value |= INTR_DEV_SUSPEND;
        }
        if sie & SIE_STATUS_RESUME != 0 {
            value |= INTR_DEV_RESUME_FROM_HOST;
        }
        if self.dev_sof_pending {
            value |= INTR_DEV_SOF;
        }
        if self.conn_dis_pending {
            value |= INTR_DEV_CONN_DIS;
        }
        value & INTR_IMPLEMENTED_MASK
    }

    /// Aggregated interrupt status (`(INTR & INTE) | INTF`) / 聚合中断状态
    #[must_use]
    pub fn ints(&self) -> u32 {
        (self.intr() & self.reg_word(INTE)) | self.reg_word(INTF)
    }

    /// Whether the controller currently asserts `USBCTRL_IRQ`
    /// 控制器当前是否拉高 `USBCTRL_IRQ`
    #[must_use]
    pub fn irq_asserted(&self) -> bool {
        self.ints() != 0
    }

    /// ADDR_ENDP address field / ADDR_ENDP 地址字段
    #[must_use]
    pub fn address(&self) -> u8 {
        (self.reg_word(regs::ADDR_ENDP) & 0x7F) as u8
    }

    /// Whether firmware enabled the USB pull-up / 固件是否使能 USB 上拉
    #[must_use]
    pub fn pullup_enabled(&self) -> bool {
        self.reg_word(SIE_CTRL) & SIE_CTRL_PULLUP_EN != 0
    }

    /// Whether firmware enabled the controller / 固件是否启用了控制器
    #[must_use]
    pub fn controller_enabled(&self) -> bool {
        self.reg_word(MAIN_CTRL) & MAIN_CTRL_CONTROLLER_EN != 0
    }

    /// Whether the host may issue transactions to the device
    /// 主机是否可向设备发起事务
    #[must_use]
    pub fn transaction_ready(&self) -> bool {
        self.reg_word(SIE_STATUS) & SIE_STATUS_CONNECTED != 0 && self.controller_enabled()
    }

    /// Whether the device is visible to the host (pull-up and controller on)
    /// 设备对主机是否可见（上拉与控制器均已开启）
    #[must_use]
    pub fn device_visible(&self) -> bool {
        self.pullup_enabled() && self.controller_enabled()
    }

    // -----------------------------------------------------------------
    // Bus events injected by the host / 主机注入的总线事件
    // -----------------------------------------------------------------

    /// Host detected attach (VBUS + pull-up) / 主机检测到设备附着
    pub fn host_attach(&mut self) {
        self.regs[SIE_STATUS / 4] |= SIE_STATUS_VBUS_DETECTED | SIE_STATUS_CONNECTED;
        self.attached_once = true;
        self.conn_dis_pending = false;
    }

    /// Host detected detach (pull-up lost) / 主机检测到设备拔出
    pub fn host_detach(&mut self) {
        if self.attached_once {
            self.conn_dis_pending = true;
        }
        self.regs[SIE_STATUS / 4] = 0;
    }

    /// Host issued a bus reset / 主机发起总线复位
    pub fn host_bus_reset(&mut self) {
        self.regs[regs::ADDR_ENDP / 4] = 0;
        for ep in 0..16 {
            for dir in [Direction::In, Direction::Out] {
                let offset = regs::ep_buf_ctrl_offset(ep, dir);
                self.write_dpram_u32(offset, 0);
            }
            self.next_high[ep] = [false; 2];
        }
        for ep in 1..16 {
            for dir in [Direction::In, Direction::Out] {
                let offset = regs::ep_ctrl_offset(ep, dir);
                self.write_dpram_u32(offset, 0);
            }
        }
        self.regs[SIE_STATUS / 4] =
            SIE_STATUS_VBUS_DETECTED | SIE_STATUS_CONNECTED | SIE_STATUS_BUS_RESET;
    }

    /// Host delivered a start-of-frame token / 主机送达帧起始令牌
    pub fn host_sof(&mut self, frame: u16) {
        self.regs[SOF_RD / 4] = u32::from(frame) & 0x7FF;
        self.dev_sof_pending = true;
    }

    // -----------------------------------------------------------------
    // Internal helpers / 内部辅助
    // -----------------------------------------------------------------

    fn reg_word(&self, offset: usize) -> u32 {
        self.regs.get(offset / 4).copied().unwrap_or(0)
    }

    /// Raise `SIE_STATUS.SETUP_REC` / 置位 `SIE_STATUS.SETUP_REC`
    pub(crate) fn raise_setup(&mut self) {
        self.regs[SIE_STATUS / 4] |= SIE_STATUS_SETUP_REC;
    }

    /// Clear write-one-to-clear bits of `SIE_STATUS` / 清除 `SIE_STATUS` 的写 1 清零位
    pub(crate) fn clear_sie_bits(&mut self, bits: u32) {
        self.regs[SIE_STATUS / 4] &= !(bits & SIE_STATUS_W1C_MASK);
    }

    fn buff_cpu_should_handle(&self) -> u32 {
        let mut mask = 0u32;
        for ep in 0..16 {
            for dir in [Direction::In, Direction::Out] {
                if self.next_high[ep][dir.index()] {
                    mask |= regs::buff_status_bit(ep, dir);
                }
            }
        }
        mask
    }

    fn abort_endpoints(&mut self, mask: u32) {
        for ep in 0..16 {
            if mask & (1 << ep) == 0 {
                continue;
            }
            for dir in [Direction::In, Direction::Out] {
                let offset = regs::ep_buf_ctrl_offset(ep, dir);
                self.write_dpram_u32(offset, 0);
            }
            self.next_high[ep] = [false; 2];
        }
        self.regs[EP_ABORT_DONE / 4] |= mask & 0xFFFF;
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

    /// Record a completed buffer and raise `BUFF_STATUS`
    /// 记录完成的缓冲并置位 `BUFF_STATUS`
    pub(crate) fn complete_buffer(&mut self, ep: usize, dir: Direction, last: bool) {
        let bit = regs::buff_status_bit(ep, dir);
        self.regs[BUFF_STATUS / 4] |= bit;
        if last {
            self.regs[SIE_STATUS / 4] |= SIE_STATUS_TRANS_COMPLETE;
        }
    }

    /// Clear endpoint buffer-control halves without touching the endpoint control
    /// 清除端点缓冲控制半字，不影响端点控制寄存器
    pub(crate) fn clear_buffers(&mut self, ep: usize) {
        for dir in [Direction::In, Direction::Out] {
            self.write_dpram_u32(regs::ep_buf_ctrl_offset(ep, dir), 0);
        }
        self.next_high[ep] = [false; 2];
    }

    /// EP0 base buffer address constant used by the host bridge tests
    /// 主机桥测试使用的 EP0 基础缓冲地址常量
    #[must_use]
    pub const fn ep0_buffer() -> usize {
        EP0_BUF_A
    }

    /// EP0 second buffer address constant / EP0 第二缓冲地址常量
    #[must_use]
    pub const fn ep0_buffer_b() -> usize {
        EP0_BUF_B
    }

    /// Endpoint buffer control base constant / 端点缓冲控制基址常量
    #[must_use]
    pub const fn ep_buf_ctrl_base() -> usize {
        EP_BUF_CTRL_BASE
    }

    /// Endpoint control base constant / 端点控制基址常量
    #[must_use]
    pub const fn ep_ctrl_base() -> usize {
        EP_CTRL_BASE
    }

    /// SETUP packet DPSRAM offset / SETUP 包 DPSRAM 偏移
    #[must_use]
    pub const fn setup_packet_offset() -> usize {
        SETUP_PACKET
    }

    /// Expose the next-half selector (host bridge diagnostics)
    /// 暴露下一半选择器（主机桥诊断用）
    #[must_use]
    pub fn next_half(&self, ep: usize, dir: Direction) -> bool {
        self.next_high[ep][dir.index()]
    }

    /// Set the next-half selector / 设置下一半选择器
    pub(crate) fn set_next_half(&mut self, ep: usize, dir: Direction, high: bool) {
        self.next_high[ep][dir.index()] = high;
    }

    /// Clear a DPSRAM buffer control register / 清除 DPSRAM 缓冲控制寄存器
    pub(crate) fn set_buf_ctrl(&mut self, offset: usize, value: u32) {
        self.write_dpram_u32(offset, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regs::{
        BUF_CTRL_AVAIL, INTR_BUS_RESET, MAIN_CTRL_CONTROLLER_EN, SIE_CTRL_PULLUP_EN,
    };

    fn enabled_core() -> UsbCore {
        let mut core = UsbCore::new();
        core.write_reg(MAIN_CTRL, Alias::Rw, MAIN_CTRL_CONTROLLER_EN);
        core.write_reg(SIE_CTRL, Alias::Rw, SIE_CTRL_PULLUP_EN);
        core.host_attach();
        core
    }

    #[test]
    fn alias_writes_apply_bit_operations() {
        let mut core = UsbCore::new();
        core.write_reg(INTE, Alias::Rw, 0x0000_0005);
        core.write_reg(INTE, Alias::Set, 0x0000_0002);
        assert_eq!(core.read_reg(INTE), 0x0000_0007);
        core.write_reg(INTE, Alias::Xor, 0x0000_0004);
        assert_eq!(core.read_reg(INTE), 0x0000_0003);
        core.write_reg(INTE, Alias::Clear, 0x0000_0001);
        assert_eq!(core.read_reg(INTE), 0x0000_0002);
    }

    #[test]
    fn ints_combines_masked_raw_and_forced_bits() {
        let mut core = enabled_core();
        core.host_bus_reset();
        core.write_reg(INTE, Alias::Rw, INTR_BUS_RESET);
        assert_eq!(core.read_reg(INTS), INTR_BUS_RESET);
        // Raw bit not enabled is filtered out, forced bits bypass INTE.
        core.write_reg(INTE, Alias::Rw, 0);
        assert_eq!(core.read_reg(INTS), 0);
        core.write_reg(INTF, Alias::Rw, INTR_BUS_RESET);
        assert_eq!(core.read_reg(INTS), INTR_BUS_RESET);
        assert!(core.irq_asserted());
    }

    #[test]
    fn sie_status_write_one_to_clear() {
        let mut core = enabled_core();
        core.host_bus_reset();
        assert_ne!(core.read_reg(SIE_STATUS) & SIE_STATUS_BUS_RESET, 0);
        core.write_reg(SIE_STATUS, Alias::Rw, SIE_STATUS_BUS_RESET);
        assert_eq!(core.read_reg(SIE_STATUS) & SIE_STATUS_BUS_RESET, 0);
        // CONNECTED must survive a W1C of BUS_RESET.
        assert_ne!(core.read_reg(SIE_STATUS) & SIE_STATUS_CONNECTED, 0);
    }

    #[test]
    fn buff_status_is_write_one_to_clear() {
        let mut core = enabled_core();
        core.complete_buffer(3, Direction::In, false);
        assert_ne!(core.read_reg(BUFF_STATUS), 0);
        assert!(core.intr() & INTR_BUFF_STATUS != 0);
        core.write_reg(
            BUFF_STATUS,
            Alias::Rw,
            regs::buff_status_bit(3, Direction::In),
        );
        assert_eq!(core.read_reg(BUFF_STATUS), 0);
        assert_eq!(core.intr() & INTR_BUFF_STATUS, 0);
    }

    #[test]
    fn bus_reset_clears_address_endpoints_and_raises_irq() {
        let mut core = enabled_core();
        core.write_reg(regs::ADDR_ENDP, Alias::Rw, 7);
        core.write_dpram_u32(regs::ep_ctrl_offset(2, Direction::In), 0x8000_00C0);
        core.host_bus_reset();
        assert_eq!(core.address(), 0);
        assert_eq!(
            core.read_dpram_u32(regs::ep_ctrl_offset(2, Direction::In)),
            0
        );
        assert_eq!(core.intr() & INTR_BUS_RESET, INTR_BUS_RESET);
    }

    #[test]
    fn detach_latches_device_connection_interrupt() {
        let mut core = enabled_core();
        core.host_detach();
        assert_eq!(core.intr() & INTR_DEV_CONN_DIS, INTR_DEV_CONN_DIS);
        core.host_attach();
        assert_eq!(core.intr() & INTR_DEV_CONN_DIS, 0);
    }

    #[test]
    fn sof_read_clears_device_sof_interrupt() {
        let mut core = enabled_core();
        core.host_sof(12);
        assert_eq!(core.read_reg(SOF_RD), 12);
        assert_eq!(core.intr() & INTR_DEV_SOF, 0);
    }

    #[test]
    fn abort_requests_complete_immediately() {
        let mut core = enabled_core();
        core.write_dpram_u32(regs::ep_buf_ctrl_offset(1, Direction::Out), BUF_CTRL_AVAIL);
        core.write_reg(EP_ABORT, Alias::Rw, 1 << 1);
        assert_eq!(
            core.read_dpram_u32(regs::ep_buf_ctrl_offset(1, Direction::Out)),
            0
        );
        assert_ne!(core.read_reg(EP_ABORT_DONE) & (1 << 1), 0);
        assert_ne!(core.intr() & INTR_ABORT_DONE, 0);
        core.write_reg(EP_ABORT_DONE, Alias::Rw, 1 << 1);
        assert_eq!(core.read_reg(EP_ABORT_DONE), 0);
    }

    #[test]
    fn byte_lane_writes_touch_only_their_lane() {
        let mut core = UsbCore::new();
        core.write_reg(MAIN_CTRL, Alias::Rw, 0xFFFF_FFFF);
        core.write_reg_masked(MAIN_CTRL + 3, Alias::Rw, 0x00, 0xFF00_0000);
        assert_eq!(core.read_reg(MAIN_CTRL), 0x00FF_FFFF);
    }
}
