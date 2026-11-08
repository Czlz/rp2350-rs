//! USBCTRL register offsets, bit definitions and atomic aliases
//! USBCTRL 寄存器偏移、位定义与原子别名

/// Endpoint direction / 端点方向
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Device-to-host (IN) / 设备到主机（IN）
    In,
    /// Host-to-device (OUT) / 主机到设备（OUT）
    Out,
}

impl Direction {
    /// Byte offset between the IN and OUT halves of a register pair
    /// 寄存器对中 IN 与 OUT 两半之间的字节偏移
    #[must_use]
    pub const fn pair_offset(self) -> usize {
        match self {
            Self::In => 0,
            Self::Out => 4,
        }
    }

    /// Whether this direction moves data from device to host
    /// 该方向是否由设备向主机传送数据
    #[must_use]
    pub const fn is_in(self) -> bool {
        matches!(self, Self::In)
    }

    /// Index used for arrays keyed by direction / 按方向索引数组的下标
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::In => 0,
            Self::Out => 1,
        }
    }
}

/// Register write alias selected by the address window / 由地址窗口选择的写别名
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alias {
    /// Plain read/write / 普通读写
    Rw,
    /// `+0x1000`: bitwise XOR / 按位异或
    Xor,
    /// `+0x2000`: bitwise OR / 按位或
    Set,
    /// `+0x3000`: bitwise AND-NOT / 按位清零
    Clear,
}

impl Alias {
    /// Decode the alias from the 4 KB sub-window index / 由 4 KB 子窗口下标解码别名
    #[must_use]
    pub const fn from_window(index: u32) -> Self {
        match index {
            1 => Self::Xor,
            2 => Self::Set,
            3 => Self::Clear,
            _ => Self::Rw,
        }
    }
}

// ---------------------------------------------------------------------------
// USBCTRL_REGS offsets (RP2350; differs from RP2040 for the interrupt block)
// USBCTRL_REGS 偏移（RP2350；中断块偏移与 RP2040 不同）
// ---------------------------------------------------------------------------

/// Device address / 设备地址
pub const ADDR_ENDP: usize = 0x000;
/// Main control / 主控制
pub const MAIN_CTRL: usize = 0x040;
/// SOF write (host mode) / SOF 写入（主机模式）
pub const SOF_WR: usize = 0x044;
/// SOF read / SOF 读取
pub const SOF_RD: usize = 0x048;
/// SIE control / SIE 控制
pub const SIE_CTRL: usize = 0x04C;
/// SIE status / SIE 状态
pub const SIE_STATUS: usize = 0x050;
/// Interrupt endpoint control (host mode) / 中断端点控制（主机模式）
pub const INT_EP_CTRL: usize = 0x054;
/// Buffer status / 缓冲状态
pub const BUFF_STATUS: usize = 0x058;
/// Buffer CPU should handle / CPU 应处理的缓冲
pub const BUFF_CPU_SHOULD_HANDLE: usize = 0x05C;
/// Endpoint abort request / 端点中止请求
pub const EP_ABORT: usize = 0x060;
/// Endpoint abort done / 端点中止完成
pub const EP_ABORT_DONE: usize = 0x064;
/// Endpoint stall arm / 端点 STALL 预置
pub const EP_STALL_ARM: usize = 0x068;
/// NAK poll (host mode) / NAK 轮询（主机模式）
pub const NAK_POLL: usize = 0x06C;
/// Endpoint STALL/NAK status / 端点 STALL/NAK 状态
pub const EP_STATUS_STALL_NAK: usize = 0x070;
/// USB muxing / USB 复用
pub const USB_MUXING: usize = 0x074;
/// USB power / USB 供电
pub const USB_PWR: usize = 0x078;
/// PHY direct control / PHY 直接控制
pub const USBPHY_DIRECT: usize = 0x07C;
/// PHY direct override / PHY 直接覆盖
pub const USBPHY_DIRECT_OVERRIDE: usize = 0x080;
/// PHY trim / PHY 微调
pub const USBPHY_TRIM: usize = 0x084;
/// Raw interrupt status / 原始中断状态
pub const INTR: usize = 0x08C;
/// Interrupt enable / 中断使能
pub const INTE: usize = 0x090;
/// Interrupt force / 中断强制
pub const INTF: usize = 0x094;
/// Interrupt status (`(INTR & INTE) | INTF`) / 中断状态
pub const INTS: usize = 0x098;

/// Number of 32-bit words in the register block / 寄存器块 32 位字数
pub const REG_WORD_COUNT: usize = 0x1000 / 4;

// ---------------------------------------------------------------------------
// MAIN_CTRL / MAIN_CTRL
// ---------------------------------------------------------------------------

/// Reduce timings for simulation / 简化仿真时序
pub const MAIN_CTRL_SIM_TIMING: u32 = 1 << 31;
/// Host mode (must stay 0 for this device model) / 主机模式（本模型保持 0）
pub const MAIN_CTRL_HOST_NDEVICE: u32 = 1 << 1;
/// Enable the controller / 启用控制器
pub const MAIN_CTRL_CONTROLLER_EN: u32 = 1 << 0;

// ---------------------------------------------------------------------------
// SIE_CTRL / SIE_CTRL
// ---------------------------------------------------------------------------
/// Device pull-up enable (host sees attach) / 设备上拉使能（主机据此判定附着）
pub const SIE_CTRL_PULLUP_EN: u32 = 1 << 16;
/// Device pull-down enable / 设备下拉使能
pub const SIE_CTRL_PULLDOWN_EN: u32 = 1 << 15;
/// EP0 single-buffer interrupt / EP0 单缓冲中断
pub const SIE_CTRL_EP0_INT_1BUF: u32 = 1 << 29;
/// EP0 double buffering / EP0 双缓冲
pub const SIE_CTRL_EP0_DOUBLE_BUF: u32 = 1 << 30;
/// EP0 STALL interrupt / EP0 STALL 中断
pub const SIE_CTRL_EP0_INT_STALL: u32 = 1 << 31;
/// EP0 NAK interrupt / EP0 NAK 中断
pub const SIE_CTRL_EP0_INT_NAK: u32 = 1 << 27;
/// Host resume signaling / 主机恢复信号
pub const SIE_CTRL_RESUME: u32 = 1 << 12;

// ---------------------------------------------------------------------------
// SIE_STATUS / SIE_STATUS
// ---------------------------------------------------------------------------
/// VBUS detected / 检测到 VBUS
pub const SIE_STATUS_VBUS_DETECTED: u32 = 1 << 0;
/// Bus suspended / 总线挂起
pub const SIE_STATUS_SUSPENDED: u32 = 1 << 4;
/// Resume detected / 检测到恢复
pub const SIE_STATUS_RESUME: u32 = 1 << 11;
/// Device connected / 设备已连接
pub const SIE_STATUS_CONNECTED: u32 = 1 << 16;
/// SETUP packet received / 收到 SETUP 包
pub const SIE_STATUS_SETUP_REC: u32 = 1 << 17;
/// Transfer complete / 传输完成
pub const SIE_STATUS_TRANS_COMPLETE: u32 = 1 << 18;
/// Bus reset occurred / 发生总线复位
pub const SIE_STATUS_BUS_RESET: u32 = 1 << 19;

/// Write-one-to-clear mask of [`SIE_STATUS`] / [`SIE_STATUS`] 的写 1 清零掩码
pub const SIE_STATUS_W1C_MASK: u32 = (1 << 31)
    | (1 << 30)
    | (1 << 29)
    | (1 << 28)
    | (1 << 27)
    | (1 << 26)
    | (1 << 25)
    | (1 << 24)
    | SIE_STATUS_BUS_RESET
    | SIE_STATUS_TRANS_COMPLETE
    | SIE_STATUS_SETUP_REC
    | SIE_STATUS_CONNECTED
    | SIE_STATUS_RESUME;

// ---------------------------------------------------------------------------
// INTR bits (RP2350 layout) / INTR 位（RP2350 布局）
// ---------------------------------------------------------------------------
/// Host connection/disconnection / 主机连接或断开
pub const INTR_HOST_CONN_DIS: u32 = 1 << 0;
/// Host resume / 主机恢复
pub const INTR_HOST_RESUME: u32 = 1 << 1;
/// Host SOF / 主机帧起始
pub const INTR_HOST_SOF: u32 = 1 << 2;
/// Transfer complete / 传输完成
pub const INTR_TRANS_COMPLETE: u32 = 1 << 3;
/// Buffer status changed / 缓冲状态变化
pub const INTR_BUFF_STATUS: u32 = 1 << 4;
/// Data sequence error / 数据序列错误
pub const INTR_ERROR_DATA_SEQ: u32 = 1 << 5;
/// RX timeout / 接收超时
pub const INTR_ERROR_RX_TIMEOUT: u32 = 1 << 6;
/// RX overflow / 接收溢出
pub const INTR_ERROR_RX_OVERFLOW: u32 = 1 << 7;
/// Bit-stuff error / 位填充错误
pub const INTR_ERROR_BIT_STUFF: u32 = 1 << 8;
/// CRC error / CRC 错误
pub const INTR_ERROR_CRC: u32 = 1 << 9;
/// STALL received or sent / 收到或发出 STALL
pub const INTR_STALL: u32 = 1 << 10;
/// VBUS detect / VBUS 检测
pub const INTR_VBUS_DETECT: u32 = 1 << 11;
/// Bus reset / 总线复位
pub const INTR_BUS_RESET: u32 = 1 << 12;
/// Device connection/disconnection / 设备连接或断开
pub const INTR_DEV_CONN_DIS: u32 = 1 << 13;
/// Device suspend / 设备挂起
pub const INTR_DEV_SUSPEND: u32 = 1 << 14;
/// Device resume from host / 设备从主机恢复
pub const INTR_DEV_RESUME_FROM_HOST: u32 = 1 << 15;
/// SETUP request / SETUP 请求
pub const INTR_SETUP_REQ: u32 = 1 << 16;
/// Device SOF / 设备帧起始
pub const INTR_DEV_SOF: u32 = 1 << 17;
/// Abort done / 中止完成
pub const INTR_ABORT_DONE: u32 = 1 << 18;
/// Endpoint STALL/NAK / 端点 STALL/NAK
pub const INTR_EP_STALL_NAK: u32 = 1 << 19;
/// RX short packet / 接收短包
pub const INTR_RX_SHORT_PACKET: u32 = 1 << 20;
/// Endpoint error / 端点错误
pub const INTR_ENDPOINT_ERROR: u32 = 1 << 21;
/// Device state machine watchdog / 设备状态机看门狗
pub const INTR_DEV_SM_WATCHDOG_FIRED: u32 = 1 << 22;
/// Endpoint stopped on NAK / 端点因 NAK 停止
pub const INTR_EPX_STOPPED_ON_NAK: u32 = 1 << 23;

/// Mask of implemented INTR bits / 已实现 INTR 位的掩码
pub const INTR_IMPLEMENTED_MASK: u32 = INTR_TRANS_COMPLETE
    | INTR_BUFF_STATUS
    | INTR_VBUS_DETECT
    | INTR_BUS_RESET
    | INTR_DEV_CONN_DIS
    | INTR_DEV_SUSPEND
    | INTR_DEV_RESUME_FROM_HOST
    | INTR_SETUP_REQ
    | INTR_DEV_SOF
    | INTR_ABORT_DONE
    | INTR_EP_STALL_NAK;

// ---------------------------------------------------------------------------
// DPSRAM layout / DPSRAM 布局
// ---------------------------------------------------------------------------
/// SETUP packet buffer offset / SETUP 包缓冲偏移
pub const SETUP_PACKET: usize = 0x000;
/// First EP1 control pair / EP1 控制寄存器对的起始偏移
pub const EP_CTRL_BASE: usize = 0x008;
/// First EP0 buffer control pair / EP0 缓冲控制寄存器对的起始偏移
pub const EP_BUF_CTRL_BASE: usize = 0x080;
/// EP0 single-buffer data buffer / EP0 单缓冲数据缓冲
pub const EP0_BUF_A: usize = 0x100;
/// EP0 second buffer (double buffered) / EP0 第二缓冲（双缓冲）
pub const EP0_BUF_B: usize = 0x140;
/// Start of the freely allocated endpoint data area / 自由分配的端点数据区起点
pub const EPX_DATA: usize = 0x180;
/// Maximum packet size of a full-speed endpoint buffer / 全速端点缓冲的最大包长
pub const MAX_PACKET_SIZE: usize = 64;
/// Distance between the two halves of a double-buffered endpoint / 双缓冲端点两半之间的距离
pub const DOUBLE_BUF_STRIDE: usize = 64;

/// Endpoint control register offset (EP 1..=15 only)
/// 端点控制寄存器偏移（仅 EP 1..=15）
///
/// # Panics
/// Panics when `ep` is 0 or greater than 15.
/// 当 `ep` 为 0 或大于 15 时 panic。
#[must_use]
pub const fn ep_ctrl_offset(ep: usize, dir: Direction) -> usize {
    assert!(ep >= 1 && ep <= 15, "EP control exists for EP1..EP15 only");
    EP_CTRL_BASE + (ep - 1) * 8 + dir.pair_offset()
}

/// Endpoint buffer control register offset (EP 0..=15)
/// 端点缓冲控制寄存器偏移（EP 0..=15）
///
/// # Panics
/// Panics when `ep` is greater than 15.
/// 当 `ep` 大于 15 时 panic。
#[must_use]
pub const fn ep_buf_ctrl_offset(ep: usize, dir: Direction) -> usize {
    assert!(ep <= 15, "EP buffer control exists for EP0..EP15 only");
    EP_BUF_CTRL_BASE + ep * 8 + dir.pair_offset()
}

// ---------------------------------------------------------------------------
// Endpoint control bits / 端点控制位
// ---------------------------------------------------------------------------
/// Endpoint enable / 端点使能
pub const EP_CTRL_ENABLE: u32 = 1 << 31;
/// Endpoint double buffering / 端点双缓冲
pub const EP_CTRL_DOUBLE_BUFFERED: u32 = 1 << 30;
/// Interrupt on every buffer / 每缓冲中断
pub const EP_CTRL_INT_PER_BUF: u32 = 1 << 29;
/// Interrupt on every double buffer / 每双缓冲中断
pub const EP_CTRL_INT_PER_DOUBLE: u32 = 1 << 28;
/// Interrupt on NAK / NAK 时中断
pub const EP_CTRL_ON_NAK: u32 = 1 << 16;
/// Interrupt on STALL / STALL 时中断
pub const EP_CTRL_ON_STALL: u32 = 1 << 17;
/// Transfer type field LSB / 传输类型字段最低位
pub const EP_CTRL_TYPE_LSB: u32 = 26;
/// Buffer offset field inside a 4 KB DPSRAM / 4 KB DPSRAM 内的缓冲偏移字段
pub const EP_CTRL_BUFFER_OFFSET_MASK: u32 = 0xFFC0;

// ---------------------------------------------------------------------------
// Endpoint buffer control bits / 端点缓冲控制位
// ---------------------------------------------------------------------------
/// Buffer full / 缓冲已满
pub const BUF_CTRL_FULL: u32 = 1 << 15;
/// Last buffer of a transfer / 传输的最后一个缓冲
pub const BUF_CTRL_LAST: u32 = 1 << 14;
/// DATA1 packet ID / DATA1 包标识
pub const BUF_CTRL_DATA1: u32 = 1 << 13;
/// Buffer selector for double buffering / 双缓冲的缓冲选择
pub const BUF_CTRL_SEL: u32 = 1 << 12;
/// Buffer stalled / 缓冲已 STALL
pub const BUF_CTRL_STALL: u32 = 1 << 11;
/// Buffer available to the controller / 缓冲对控制器可用
pub const BUF_CTRL_AVAIL: u32 = 1 << 10;
/// Payload length field / 负载长度字段
pub const BUF_CTRL_LEN_MASK: u32 = 0x03FF;

/// Extract the 16-bit buffer control half for `half` (0 = low, 1 = high)
/// 提取第 `half` 个 16 位缓冲控制半字（0 = 低半，1 = 高半）
#[must_use]
pub const fn buf_ctrl_half(value: u32, half: u8) -> u32 {
    if half == 0 {
        value & 0xFFFF
    } else {
        value >> 16
    }
}

/// Replace the 16-bit buffer control half for `half`
/// 替换第 `half` 个 16 位缓冲控制半字
#[must_use]
pub const fn buf_ctrl_set_half(value: u32, half: u8, half_value: u32) -> u32 {
    if half == 0 {
        (value & 0xFFFF_0000) | (half_value & 0xFFFF)
    } else {
        (value & 0x0000_FFFF) | ((half_value & 0xFFFF) << 16)
    }
}

/// Buffer status bit for endpoint `ep` and direction / 端点 `ep` 与方向对应的缓冲状态位
#[must_use]
pub const fn buff_status_bit(ep: usize, dir: Direction) -> u32 {
    1 << (ep * 2 + dir.index())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_window_decoding() {
        assert_eq!(Alias::from_window(0), Alias::Rw);
        assert_eq!(Alias::from_window(1), Alias::Xor);
        assert_eq!(Alias::from_window(2), Alias::Set);
        assert_eq!(Alias::from_window(3), Alias::Clear);
    }

    #[test]
    fn endpoint_offsets_match_dpram_layout() {
        assert_eq!(ep_ctrl_offset(1, Direction::In), 0x008);
        assert_eq!(ep_ctrl_offset(1, Direction::Out), 0x00C);
        assert_eq!(ep_ctrl_offset(15, Direction::Out), 0x07C);
        assert_eq!(ep_buf_ctrl_offset(0, Direction::In), 0x080);
        assert_eq!(ep_buf_ctrl_offset(0, Direction::Out), 0x084);
        assert_eq!(ep_buf_ctrl_offset(15, Direction::Out), 0x0FC);
    }

    #[test]
    fn buffer_control_halves_round_trip() {
        let value = buf_ctrl_set_half(0x1122_3344, 0, 0x8000 | 0x0010);
        assert_eq!(buf_ctrl_half(value, 0), 0x8010);
        assert_eq!(buf_ctrl_half(value, 1), 0x1122);
        let value = buf_ctrl_set_half(value, 1, 0x4000 | 0x0008);
        assert_eq!(buf_ctrl_half(value, 0), 0x8010);
        assert_eq!(buf_ctrl_half(value, 1), 0x4008);
    }

    #[test]
    fn buffer_status_bits_are_two_per_endpoint() {
        assert_eq!(buff_status_bit(0, Direction::In), 1);
        assert_eq!(buff_status_bit(0, Direction::Out), 2);
        assert_eq!(buff_status_bit(3, Direction::In), 1 << 6);
        assert_eq!(buff_status_bit(3, Direction::Out), 1 << 7);
    }
}
