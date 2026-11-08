//! USB descriptor parsing for the host bridge
//! 主机桥使用的 USB 描述符解析

/// Endpoint description extracted from a configuration descriptor
/// 从配置描述符中提取的端点描述
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EndpointInfo {
    /// `bEndpointAddress` including the direction bit / 含方向位的端点地址
    pub address: u8,
    /// `wMaxPacketSize` / 最大包长
    pub max_packet_size: u16,
    /// Bulk transfer (`bmAttributes[1:0] == 2`) / 是否为批量传输
    pub bulk: bool,
}

impl EndpointInfo {
    /// Endpoint number without the direction bit / 去掉方向位的端点号
    #[must_use]
    pub const fn number(&self) -> u8 {
        self.address & 0x0F
    }

    /// Whether the endpoint is IN (device to host) / 是否为 IN（设备到主机）
    #[must_use]
    pub const fn is_in(&self) -> bool {
        self.address & 0x80 != 0
    }
}

/// Parsed device descriptor fields used by the host bridge
/// 主机桥使用的设备描述符字段
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeviceInfo {
    /// `idVendor` / 厂商 ID
    pub vendor_id: u16,
    /// `idProduct` / 产品 ID
    pub product_id: u16,
    /// `bcdDevice` / 设备版本
    pub device_version: u16,
    /// `bMaxPacketSize0` / EP0 最大包长
    pub max_packet_size0: u8,
    /// `bNumConfigurations` / 配置数量
    pub configurations: u8,
    /// `iManufacturer` / 厂商字符串索引
    pub manufacturer: u8,
    /// `iProduct` / 产品字符串索引
    pub product: u8,
    /// `iSerialNumber` / 序列号字符串索引
    pub serial: u8,
}

/// Parsed configuration with the CDC data endpoints
/// 解析后的配置及 CDC 数据端点
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfigInfo {
    /// `wTotalLength` / 配置总长度
    pub total_length: u16,
    /// `bConfigurationValue` / 配置值
    pub value: u8,
    /// Endpoints found on the CDC data interface
    /// 在 CDC 数据接口上找到的端点
    pub cdc_in: Option<EndpointInfo>,
    /// CDC bulk OUT endpoint / CDC 批量 OUT 端点
    pub cdc_out: Option<EndpointInfo>,
    /// CDC control interface number / CDC 控制接口号
    pub comm_interface: Option<u8>,
    /// CDC data interface number / CDC 数据接口号
    pub data_interface: Option<u8>,
}

impl ConfigInfo {
    /// Bulk IN endpoint or an error when it is missing
    /// 批量 IN 端点；缺失时报错
    ///
    /// # Errors
    /// Returns [`DescriptorError::MissingCdcEndpoint`] when the configuration
    /// has no bulk IN endpoint on a CDC data interface.
    /// 当配置在 CDC 数据接口上没有批量 IN 端点时返回
    /// [`DescriptorError::MissingCdcEndpoint`]。
    pub fn require_cdc_in(&self) -> Result<EndpointInfo, DescriptorError> {
        self.cdc_in.ok_or(DescriptorError::MissingCdcEndpoint)
    }

    /// Bulk OUT endpoint or an error when it is missing
    /// 批量 OUT 端点；缺失时报错
    ///
    /// # Errors
    /// Returns [`DescriptorError::MissingCdcEndpoint`] when the configuration
    /// has no bulk OUT endpoint on a CDC data interface.
    /// 当配置在 CDC 数据接口上没有批量 OUT 端点时返回
    /// [`DescriptorError::MissingCdcEndpoint`]。
    pub fn require_cdc_out(&self) -> Result<EndpointInfo, DescriptorError> {
        self.cdc_out.ok_or(DescriptorError::MissingCdcEndpoint)
    }
}

/// Descriptor parsing failures / 描述符解析失败
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorError {
    /// Buffer shorter than the descriptor header / 缓冲短于描述符头
    Truncated,
    /// Unexpected descriptor type / 意外的描述符类型
    UnexpectedType,
    /// Configuration has no CDC data interface / 配置没有 CDC 数据接口
    MissingCdcDataInterface,
    /// Configuration has no bulk IN/OUT endpoint / 配置缺少批量 IN/OUT 端点
    MissingCdcEndpoint,
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    let lo = u16::from(bytes[offset]);
    let hi = u16::from(bytes[offset + 1]);
    lo | (hi << 8)
}

/// Parse an 18-byte device descriptor / 解析 18 字节设备描述符
///
/// # Errors
/// Returns [`DescriptorError::Truncated`] or
/// [`DescriptorError::UnexpectedType`] for malformed input.
/// 输入格式错误时返回 [`DescriptorError::Truncated`] 或
/// [`DescriptorError::UnexpectedType`]。
pub fn parse_device(bytes: &[u8]) -> Result<DeviceInfo, DescriptorError> {
    if bytes.len() < 18 {
        return Err(DescriptorError::Truncated);
    }
    if bytes[1] != 0x01 {
        return Err(DescriptorError::UnexpectedType);
    }
    Ok(DeviceInfo {
        vendor_id: read_u16(bytes, 8),
        product_id: read_u16(bytes, 10),
        device_version: read_u16(bytes, 12),
        max_packet_size0: bytes[7],
        configurations: bytes[17],
        manufacturer: bytes[14],
        product: bytes[15],
        serial: bytes[16],
    })
}

/// Parse a configuration descriptor, scanning interface and endpoint records
/// 解析配置描述符，扫描接口与端点记录
///
/// # Errors
/// Returns an error for malformed input or when the CDC data interface has no
/// bulk endpoints.
/// 输入格式错误或 CDC 数据接口缺少批量端点时返回错误。
pub fn parse_config(bytes: &[u8]) -> Result<ConfigInfo, DescriptorError> {
    if bytes.len() < 9 {
        return Err(DescriptorError::Truncated);
    }
    if bytes[1] != 0x02 {
        return Err(DescriptorError::UnexpectedType);
    }
    let total = read_u16(bytes, 2) as usize;
    let end = total.min(bytes.len());
    let mut info = ConfigInfo {
        total_length: read_u16(bytes, 2),
        value: bytes[5],
        ..ConfigInfo::default()
    };
    let mut offset = bytes[0] as usize;
    let mut current_class: Option<u8> = None;
    let mut current_interface: Option<u8> = None;
    while offset + 2 <= end {
        let length = bytes[offset] as usize;
        if length == 0 || offset + length > end {
            return Err(DescriptorError::Truncated);
        }
        let kind = bytes[offset + 1];
        match kind {
            0x04 if length >= 9 => {
                let interface = bytes[offset + 2];
                let class = bytes[offset + 5];
                current_interface = Some(interface);
                current_class = Some(class);
                match class {
                    0x02 if info.comm_interface.is_none() => {
                        info.comm_interface = Some(interface);
                    }
                    0x0A if info.data_interface.is_none() => {
                        info.data_interface = Some(interface);
                    }
                    _ => {}
                }
            }
            0x05 if length >= 7 => {
                let endpoint = EndpointInfo {
                    address: bytes[offset + 2],
                    max_packet_size: read_u16(bytes, offset + 4) & 0x07FF,
                    bulk: bytes[offset + 3] & 0x03 == 0x02,
                };
                if current_class == Some(0x0A) && endpoint.bulk {
                    if endpoint.is_in() {
                        info.cdc_in.get_or_insert(endpoint);
                    } else {
                        info.cdc_out.get_or_insert(endpoint);
                    }
                }
                let _ = current_interface;
            }
            _ => {}
        }
        offset += length;
    }
    if info.data_interface.is_none() {
        return Err(DescriptorError::MissingCdcDataInterface);
    }
    if info.cdc_in.is_none() || info.cdc_out.is_none() {
        return Err(DescriptorError::MissingCdcEndpoint);
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE: [u8; 18] = [
        18, 1, 0x00, 0x02, 0x02, 0x02, 0x00, 0x40, 0x09, 0x12, 0x02, 0x00, 0x00, 0x02, 0x01, 0x02,
        0x03, 0x01,
    ];

    const CONFIG: [u8; 46] = [
        9, 2, 46, 0, 2, 1, 0, 0xC0, 0x01, // configuration
        9, 4, 0, 0, 1, 0x02, 0x02, 0x01, 0x00, // CDC control interface
        5, 0x24, 0x00, 0x10, 0x01, // CDC functional header
        9, 4, 1, 0, 2, 0x0A, 0x00, 0x00, 0x00, // CDC data interface
        7, 5, 0x81, 0x02, 0x40, 0, 1, // bulk IN
        7, 5, 0x02, 0x02, 0x40, 0, 1, // bulk OUT
    ];

    #[test]
    fn device_descriptor_fields() {
        let info = parse_device(&DEVICE).unwrap();
        assert_eq!(info.vendor_id, 0x1209);
        assert_eq!(info.product_id, 0x0002);
        assert_eq!(info.max_packet_size0, 0x40);
        assert_eq!(info.manufacturer, 1);
        assert_eq!(info.product, 2);
        assert_eq!(info.serial, 3);
        assert_eq!(info.configurations, 1);
    }

    #[test]
    fn config_descriptor_finds_cdc_endpoints() {
        let info = parse_config(&CONFIG).unwrap();
        assert_eq!(info.total_length, 46);
        assert_eq!(info.comm_interface, Some(0));
        assert_eq!(info.data_interface, Some(1));
        let ep_in = info.require_cdc_in().unwrap();
        assert_eq!(ep_in.address, 0x81);
        assert_eq!(ep_in.number(), 1);
        assert!(ep_in.is_in());
        assert_eq!(ep_in.max_packet_size, 64);
        let ep_out = info.require_cdc_out().unwrap();
        assert_eq!(ep_out.address, 0x02);
        assert!(!ep_out.is_in());
    }

    #[test]
    fn truncated_device_descriptor_is_rejected() {
        assert_eq!(parse_device(&DEVICE[..10]), Err(DescriptorError::Truncated));
        assert_eq!(parse_device(&[0; 18]), Err(DescriptorError::UnexpectedType));
    }

    #[test]
    fn config_without_data_interface_is_rejected() {
        let mut broken = CONFIG;
        broken[28] = 0x03; // interface class of the data interface
        assert_eq!(
            parse_config(&broken),
            Err(DescriptorError::MissingCdcDataInterface)
        );
    }
}
