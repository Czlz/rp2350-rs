//! USB SETUP packet decoding and standard request identifiers
//! USB SETUP 包解码与标准请求标识

use crate::regs::Direction;

/// Standard and CDC class request codes used during enumeration
/// 枚举期间使用的标准请求与 CDC 类请求码
pub mod request {
    /// GET_STATUS / 获取状态
    pub const GET_STATUS: u8 = 0x00;
    /// CLEAR_FEATURE / 清除特性
    pub const CLEAR_FEATURE: u8 = 0x01;
    /// SET_FEATURE / 设置特性
    pub const SET_FEATURE: u8 = 0x03;
    /// SET_ADDRESS / 设置地址
    pub const SET_ADDRESS: u8 = 0x05;
    /// GET_DESCRIPTOR / 获取描述符
    pub const GET_DESCRIPTOR: u8 = 0x06;
    /// SET_DESCRIPTOR / 设置描述符
    pub const SET_DESCRIPTOR: u8 = 0x07;
    /// GET_CONFIGURATION / 获取配置
    pub const GET_CONFIGURATION: u8 = 0x08;
    /// SET_CONFIGURATION / 设置配置
    pub const SET_CONFIGURATION: u8 = 0x09;
    /// GET_INTERFACE / 获取接口
    pub const GET_INTERFACE: u8 = 0x0A;
    /// SET_INTERFACE / 设置接口
    pub const SET_INTERFACE: u8 = 0x0B;
    /// CDC SET_LINE_CODING / CDC 设置线路编码
    pub const CDC_SET_LINE_CODING: u8 = 0x20;
    /// CDC GET_LINE_CODING / CDC 获取线路编码
    pub const CDC_GET_LINE_CODING: u8 = 0x21;
    /// CDC SET_CONTROL_LINE_STATE / CDC 设置控制线状态
    pub const CDC_SET_CONTROL_LINE_STATE: u8 = 0x22;
    /// CDC SEND_BREAK / CDC 发送断开
    pub const CDC_SEND_BREAK: u8 = 0x23;
}

/// Descriptor type identifiers / 描述符类型标识
pub mod descriptor {
    /// Device descriptor / 设备描述符
    pub const DEVICE: u16 = 0x01;
    /// Configuration descriptor / 配置描述符
    pub const CONFIGURATION: u16 = 0x02;
    /// String descriptor / 字符串描述符
    pub const STRING: u16 = 0x03;
    /// Interface descriptor / 接口描述符
    pub const INTERFACE: u16 = 0x04;
    /// Endpoint descriptor / 端点描述符
    pub const ENDPOINT: u16 = 0x05;
}

/// A decoded 8-byte USB SETUP packet / 解码后的 8 字节 USB SETUP 包
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SetupPacket {
    /// `bmRequestType` / 请求类型
    pub bm_request_type: u8,
    /// `bRequest` / 请求码
    pub b_request: u8,
    /// `wValue` / 值字段
    pub w_value: u16,
    /// `wIndex` / 索引字段
    pub w_index: u16,
    /// `wLength` / 长度字段
    pub w_length: u16,
}

impl SetupPacket {
    /// Build a packet from its raw little-endian bytes
    /// 由原始小端字节构建 SETUP 包
    #[must_use]
    pub const fn from_bytes(bytes: &[u8; 8]) -> Self {
        Self {
            bm_request_type: bytes[0],
            b_request: bytes[1],
            w_value: u16::from_le_bytes([bytes[2], bytes[3]]),
            w_index: u16::from_le_bytes([bytes[4], bytes[5]]),
            w_length: u16::from_le_bytes([bytes[6], bytes[7]]),
        }
    }

    /// Serialize the packet to its raw little-endian bytes
    /// 将 SETUP 包序列化为原始小端字节
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 8] {
        let mut out = [0u8; 8];
        out[0] = self.bm_request_type;
        out[1] = self.b_request;
        out[2] = self.w_value as u8;
        out[3] = (self.w_value >> 8) as u8;
        out[4] = self.w_index as u8;
        out[5] = (self.w_index >> 8) as u8;
        out[6] = self.w_length as u8;
        out[7] = (self.w_length >> 8) as u8;
        out
    }

    /// Data stage direction encoded by `bmRequestType.bit7`
    /// `bmRequestType.bit7` 编码的数据阶段方向
    #[must_use]
    pub const fn direction(self) -> Direction {
        if self.bm_request_type & 0x80 != 0 {
            Direction::In
        } else {
            Direction::Out
        }
    }

    /// Request type field (`bmRequestType[6:5]`) / 请求类型字段
    #[must_use]
    pub const fn request_type(self) -> u8 {
        (self.bm_request_type >> 5) & 0x03
    }

    /// Whether this is a standard (device/interface/endpoint) request
    /// 是否为标准（设备/接口/端点）请求
    #[must_use]
    pub const fn is_standard(self) -> bool {
        self.request_type() == 0
    }

    /// Whether this is a class-specific request / 是否为类特定请求
    #[must_use]
    pub const fn is_class(self) -> bool {
        self.request_type() == 1
    }

    /// Recipient field (`bmRequestType[4:0]`) / 接收者字段
    #[must_use]
    pub const fn recipient(self) -> u8 {
        self.bm_request_type & 0x1F
    }

    /// Build a standard IN request (host side convenience)
    /// 构建标准 IN 请求（主机侧便捷方法）
    #[must_use]
    pub const fn standard_in(request: u8, value: u16, index: u16, length: u16) -> Self {
        Self {
            bm_request_type: 0x80,
            b_request: request,
            w_value: value,
            w_index: index,
            w_length: length,
        }
    }

    /// Build a standard OUT request (host side convenience)
    /// 构建标准 OUT 请求（主机侧便捷方法）
    #[must_use]
    pub const fn standard_out(request: u8, value: u16, index: u16, length: u16) -> Self {
        Self {
            bm_request_type: 0x00,
            b_request: request,
            w_value: value,
            w_index: index,
            w_length: length,
        }
    }

    /// Build a class-specific OUT request to an interface
    /// 构建发往接口的类特定 OUT 请求
    #[must_use]
    pub const fn class_out(request: u8, value: u16, index: u16, length: u16) -> Self {
        Self {
            bm_request_type: 0x21,
            b_request: request,
            w_value: value,
            w_index: index,
            w_length: length,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_round_trip() {
        let packet = SetupPacket {
            bm_request_type: 0xC0,
            b_request: 0x06,
            w_value: 0x0100,
            w_index: 0x0000,
            w_length: 0x0012,
        };
        let bytes = packet.to_bytes();
        assert_eq!(SetupPacket::from_bytes(&bytes), packet);
    }

    #[test]
    fn direction_and_type_classification() {
        let get_descriptor = SetupPacket::standard_in(request::GET_DESCRIPTOR, 0x0100, 0, 18);
        assert_eq!(get_descriptor.direction(), Direction::In);
        assert!(get_descriptor.is_standard());
        assert_eq!(get_descriptor.recipient(), 0);

        let set_address = SetupPacket::standard_out(request::SET_ADDRESS, 1, 0, 0);
        assert_eq!(set_address.direction(), Direction::Out);
        assert!(set_address.is_standard());

        let line_state = SetupPacket::class_out(request::CDC_SET_CONTROL_LINE_STATE, 3, 1, 0);
        assert_eq!(line_state.direction(), Direction::Out);
        assert!(line_state.is_class());
        assert_eq!(line_state.recipient(), 1);
    }
}
