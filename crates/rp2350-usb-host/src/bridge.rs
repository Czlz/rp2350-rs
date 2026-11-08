//! Host-side enumeration state machine driving the simulated controller
//! 驱动模拟控制器的主机侧枚举状态机

use std::collections::VecDeque;

use rp2350_usb::controller::Controller;
use rp2350_usb::endpoint::{InResponse, OutResponse};
use rp2350_usb::regs::Direction;
use rp2350_usb::setup::{SetupPacket, request};

use crate::descriptor::{ConfigInfo, DescriptorError, DeviceInfo, parse_config, parse_device};

/// CDC bulk endpoints discovered during enumeration
/// 枚举过程中发现的 CDC 批量端点
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CdcEndpoints {
    /// Bulk IN endpoint address (with bit 7 set) / 批量 IN 端点地址
    pub in_address: u8,
    /// Bulk IN maximum packet size / 批量 IN 最大包长
    pub in_max_packet: u16,
    /// Bulk OUT endpoint address / 批量 OUT 端点地址
    pub out_address: u8,
    /// Bulk OUT maximum packet size / 批量 OUT 最大包长
    pub out_max_packet: u16,
    /// CDC data interface number / CDC 数据接口号
    pub interface: u8,
}

/// Host bridge failures / 主机桥失败
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostError {
    /// A control phase did not complete in time / 控制阶段超时
    Timeout {
        /// Failing request code / 失败的请求码
        request: u8,
    },
    /// The device stalled a required request / 设备对必需请求返回 STALL
    Stall {
        /// Stalled request code / 被 STALL 的请求码
        request: u8,
    },
    /// Descriptor parsing failed / 描述符解析失败
    Descriptor(DescriptorError),
    /// The device violated the control-transfer protocol / 设备违反控制传输协议
    Protocol(&'static str),
}

/// Result of one [`UsbHostBridge::poll`] call / 单次 [`UsbHostBridge::poll`] 的结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostProgress {
    /// Run the guest and call again no earlier than this cycle
    /// 运行客户机并在该周期之后再次调用
    Wait(u64),
    /// Enumeration finished and CDC endpoints are ready / 枚举完成且 CDC 端点就绪
    Configured(CdcEndpoints),
    /// Enumeration failed / 枚举失败
    Failed(HostError),
    /// The device disappeared / 设备已消失
    Detached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Detached,
    Enumerating,
    Configured,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    DeviceHeader,
    DeviceDescriptor,
    ConfigHeader,
    ConfigFull,
    StringDescriptor,
    SetAddress,
    SetConfiguration,
    Other,
}

#[derive(Debug, Clone)]
struct Request {
    setup: SetupPacket,
    kind: Kind,
    optional: bool,
    payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Setup,
    Data,
    Status,
}

struct Transfer {
    request: Request,
    phase: Phase,
    received: Vec<u8>,
    sent: usize,
    payload: Vec<u8>,
}

impl Transfer {
    fn new(request: Request) -> Self {
        Self {
            payload: request.payload.clone(),
            request,
            phase: Phase::Setup,
            received: Vec::new(),
            sent: 0,
        }
    }
}

/// Host bridge that enumerates a [`Controller`] and exposes CDC endpoints
/// 对 [`Controller`] 执行枚举并暴露 CDC 端点的主机桥
pub struct UsbHostBridge {
    controller: Controller,
    sysclk_hz: u32,
    stage: Stage,
    queue: VecDeque<Request>,
    transfer: Option<Transfer>,
    device: Option<DeviceInfo>,
    config: Option<ConfigInfo>,
    address: u8,
    retry_at: u64,
    deadline: u64,
    last_error: Option<HostError>,
}

impl UsbHostBridge {
    /// Default system clock used for cycle conversion (150 MHz)
    /// 用于周期换算的默认系统时钟（150 MHz）
    pub const DEFAULT_SYSCLK_HZ: u32 = 150_000_000;

    /// Create a bridge over the given controller / 在给定控制器上创建主机桥
    #[must_use]
    pub fn new(controller: Controller) -> Self {
        Self {
            controller,
            sysclk_hz: Self::DEFAULT_SYSCLK_HZ,
            stage: Stage::Detached,
            queue: VecDeque::new(),
            transfer: None,
            device: None,
            config: None,
            address: 0,
            retry_at: 0,
            deadline: 0,
            last_error: None,
        }
    }

    /// Override the system clock used for timing conversion
    /// 覆盖用于时序换算的系统时钟
    pub fn set_sysclk_hz(&mut self, hz: u32) {
        self.sysclk_hz = hz;
    }

    /// Parsed device descriptor, once available / 已解析的设备描述符
    #[must_use]
    pub fn device_info(&self) -> Option<DeviceInfo> {
        self.device
    }

    /// Parsed configuration descriptor, once available / 已解析的配置描述符
    #[must_use]
    pub fn config_info(&self) -> Option<&ConfigInfo> {
        self.config.as_ref()
    }

    /// Device address assigned by SET_ADDRESS / SET_ADDRESS 分配的设备地址
    #[must_use]
    pub fn address(&self) -> u8 {
        self.address
    }

    /// Shared controller handle for data-endpoint transfers
    /// 供数据端点传输使用的共享控制器句柄
    #[must_use]
    pub fn controller(&self) -> &Controller {
        &self.controller
    }

    fn us_to_cycles(&self, microseconds: u64) -> u64 {
        (u64::from(self.sysclk_hz) * microseconds).div_ceil(1_000_000)
    }

    fn begin_enumeration(&mut self, now: u64) {
        self.stage = Stage::Enumerating;
        self.address = 0;
        self.retry_at = now + self.us_to_cycles(10_000);
        self.deadline = now + self.us_to_cycles(200_000);
        self.queue.clear();
        self.queue.push_back(Self::request(
            SetupPacket::standard_in(request::GET_DESCRIPTOR, 0x0100, 0, 8),
            Kind::DeviceHeader,
            false,
        ));
        self.queue.push_back(Self::request(
            SetupPacket::standard_out(request::SET_ADDRESS, 1, 0, 0),
            Kind::SetAddress,
            false,
        ));
        self.queue.push_back(Self::request(
            SetupPacket::standard_in(request::GET_DESCRIPTOR, 0x0100, 0, 18),
            Kind::DeviceDescriptor,
            false,
        ));
        self.queue.push_back(Self::request(
            SetupPacket::standard_in(request::GET_DESCRIPTOR, 0x0200, 0, 9),
            Kind::ConfigHeader,
            false,
        ));
    }

    fn request(setup: SetupPacket, kind: Kind, optional: bool) -> Request {
        Request {
            setup,
            kind,
            optional,
            payload: Vec::new(),
        }
    }

    /// Advance enumeration by at most one USB transaction.
    /// 至多推进一次 USB 事务的枚举步骤。
    pub fn poll(&mut self, now: u64) -> HostProgress {
        match self.stage {
            Stage::Detached => {
                if !self.controller.device_visible() {
                    return HostProgress::Wait(now + self.us_to_cycles(1_000));
                }
                self.controller.host_attach();
                self.controller.host_bus_reset();
                self.begin_enumeration(now);
                return HostProgress::Wait(self.retry_at);
            }
            Stage::Configured => return HostProgress::Configured(self.configured_endpoints()),
            Stage::Failed => {
                return HostProgress::Failed(
                    self.last_error
                        .unwrap_or(HostError::Protocol("enumeration failed")),
                );
            }
            Stage::Enumerating => {}
        }

        if !self.controller.device_visible() {
            self.stage = Stage::Detached;
            self.transfer = None;
            self.queue.clear();
            self.controller.host_detach();
            return HostProgress::Detached;
        }

        if now >= self.deadline {
            let request = self
                .transfer
                .as_ref()
                .map_or(0, |t| t.request.setup.b_request);
            return self.fail(HostError::Timeout { request });
        }

        if now < self.retry_at {
            return HostProgress::Wait(self.retry_at);
        }

        if self.transfer.is_none() {
            let Some(request) = self.queue.pop_front() else {
                self.stage = Stage::Configured;
                return HostProgress::Configured(self.configured_endpoints());
            };
            self.deadline = now + self.us_to_cycles(25_000);
            self.transfer = Some(Transfer::new(request));
        }

        self.step_transfer(now)
    }

    fn configured_endpoints(&self) -> CdcEndpoints {
        let Some(config) = self.config.as_ref() else {
            return CdcEndpoints::default();
        };
        CdcEndpoints {
            in_address: config.cdc_in.unwrap_or_default().address,
            in_max_packet: config.cdc_in.unwrap_or_default().max_packet_size,
            out_address: config.cdc_out.unwrap_or_default().address,
            out_max_packet: config.cdc_out.unwrap_or_default().max_packet_size,
            interface: config.data_interface.unwrap_or(0),
        }
    }

    fn step_transfer(&mut self, now: u64) -> HostProgress {
        let Some(request) = self.transfer.as_ref().map(|t| t.request.clone()) else {
            return HostProgress::Wait(now);
        };
        match self.transfer.as_ref().map(|t| t.phase) {
            Some(Phase::Setup) => {
                if !self.controller.host_setup(&request.setup) {
                    self.retry_at = now + self.us_to_cycles(50);
                    return HostProgress::Wait(self.retry_at);
                }
                let next = if request.setup.w_length == 0 {
                    Phase::Status
                } else {
                    Phase::Data
                };
                if let Some(transfer) = self.transfer.as_mut() {
                    transfer.phase = next;
                }
                self.retry_at = now + self.us_to_cycles(20);
                HostProgress::Wait(self.retry_at)
            }
            Some(Phase::Data) => self.step_data(now),
            Some(Phase::Status) => self.step_status(now),
            None => HostProgress::Wait(now),
        }
    }

    fn step_data(&mut self, now: u64) -> HostProgress {
        let Some(direction) = self.transfer.as_ref().map(|t| t.request.setup.direction()) else {
            return HostProgress::Wait(now);
        };
        let request_code = self
            .transfer
            .as_ref()
            .map_or(0, |t| t.request.setup.b_request);
        if direction == Direction::In {
            match self.controller.host_in(0) {
                InResponse::NoDevice => self.device_lost(),
                InResponse::Nak => self.nak(now),
                InResponse::Stall => self.fail(HostError::Stall {
                    request: request_code,
                }),
                InResponse::Data(packet) => {
                    let Some(transfer) = self.transfer.as_mut() else {
                        return HostProgress::Wait(now);
                    };
                    transfer.received.extend_from_slice(packet.bytes());
                    let wants_more =
                        usize::from(transfer.request.setup.w_length) > transfer.received.len();
                    if packet.last || packet.len() < 64 || !wants_more {
                        transfer.phase = Phase::Status;
                    }
                    self.retry_at = now + self.us_to_cycles(10);
                    HostProgress::Wait(self.retry_at)
                }
            }
        } else {
            let payload = {
                let Some(transfer) = self.transfer.as_ref() else {
                    return HostProgress::Wait(now);
                };
                let remaining =
                    usize::from(transfer.request.setup.w_length).saturating_sub(transfer.sent);
                let chunk = remaining.min(64);
                if chunk == 0 {
                    None
                } else {
                    Some(transfer.payload[transfer.sent..transfer.sent + chunk].to_vec())
                }
            };
            let Some(chunk) = payload else {
                if let Some(transfer) = self.transfer.as_mut() {
                    transfer.phase = Phase::Status;
                }
                return HostProgress::Wait(now);
            };
            let chunk_len = chunk.len();
            match self.controller.host_out(0, &chunk) {
                OutResponse::NoDevice => self.device_lost(),
                OutResponse::Nak => self.nak(now),
                OutResponse::Stall => self.fail(HostError::Stall {
                    request: request_code,
                }),
                OutResponse::Ack { last } => {
                    if let Some(transfer) = self.transfer.as_mut() {
                        transfer.sent += chunk_len;
                        if last || transfer.sent >= usize::from(transfer.request.setup.w_length) {
                            transfer.phase = Phase::Status;
                        }
                    }
                    self.retry_at = now + self.us_to_cycles(10);
                    HostProgress::Wait(self.retry_at)
                }
            }
        }
    }

    fn step_status(&mut self, now: u64) -> HostProgress {
        let Some(request) = self.transfer.as_ref().map(|t| t.request.clone()) else {
            return HostProgress::Wait(now);
        };
        if request.setup.direction() == Direction::In {
            match self.controller.host_out(0, &[]) {
                OutResponse::NoDevice => self.device_lost(),
                OutResponse::Nak => self.nak(now),
                OutResponse::Stall => self.fail(HostError::Stall {
                    request: request.setup.b_request,
                }),
                OutResponse::Ack { .. } => self.complete_transfer(now),
            }
        } else {
            match self.controller.host_in(0) {
                InResponse::NoDevice => self.device_lost(),
                InResponse::Nak => self.nak(now),
                InResponse::Stall => self.fail(HostError::Stall {
                    request: request.setup.b_request,
                }),
                InResponse::Data(_) => self.complete_transfer(now),
            }
        }
    }

    fn nak(&mut self, now: u64) -> HostProgress {
        self.retry_at = now + self.us_to_cycles(100);
        HostProgress::Wait(self.retry_at)
    }

    fn complete_transfer(&mut self, now: u64) -> HostProgress {
        let Some(transfer) = self.transfer.take() else {
            return HostProgress::Wait(now);
        };
        if let Err(error) = self.finish(&transfer) {
            if transfer.request.optional {
                return HostProgress::Wait(now);
            }
            return self.fail(error);
        }
        self.retry_at = now + self.us_to_cycles(20);
        HostProgress::Wait(self.retry_at)
    }

    fn finish(&mut self, transfer: &Transfer) -> Result<(), HostError> {
        match transfer.request.kind {
            Kind::DeviceDescriptor => {
                self.device =
                    Some(parse_device(&transfer.received).map_err(HostError::Descriptor)?);
            }
            Kind::ConfigHeader => {
                if transfer.received.len() < 9 {
                    return Err(HostError::Descriptor(DescriptorError::Truncated));
                }
                let total =
                    u16::from(transfer.received[2]) | (u16::from(transfer.received[3]) << 8);
                self.queue.push_front(Self::request(
                    SetupPacket::standard_in(request::GET_DESCRIPTOR, 0x0200, 0, total),
                    Kind::ConfigFull,
                    false,
                ));
            }
            Kind::ConfigFull => {
                let config = parse_config(&transfer.received).map_err(HostError::Descriptor)?;
                self.config = Some(config);
                self.queue_string_requests();
                self.queue.push_back(Self::request(
                    SetupPacket::standard_out(request::SET_CONFIGURATION, 1, 0, 0),
                    Kind::SetConfiguration,
                    false,
                ));
            }
            Kind::SetAddress => {
                self.address = transfer.request.setup.w_value as u8;
            }
            Kind::SetConfiguration => {
                self.queue.push_back(Self::request(
                    SetupPacket::class_out(request::CDC_SET_CONTROL_LINE_STATE, 0x0003, 0, 0),
                    Kind::Other,
                    true,
                ));
            }
            Kind::DeviceHeader | Kind::StringDescriptor | Kind::Other => {}
        }
        Ok(())
    }

    fn queue_string_requests(&mut self) {
        let Some(info) = self.device else {
            return;
        };
        let mut indices = vec![0u8];
        for index in [info.manufacturer, info.product, info.serial] {
            if index != 0 && !indices.contains(&index) {
                indices.push(index);
            }
        }
        for index in indices {
            self.queue.push_back(Self::request(
                SetupPacket::standard_in(
                    request::GET_DESCRIPTOR,
                    0x0300 | u16::from(index),
                    0x0409,
                    255,
                ),
                Kind::StringDescriptor,
                true,
            ));
        }
    }

    fn fail(&mut self, error: HostError) -> HostProgress {
        self.stage = Stage::Failed;
        self.last_error = Some(error);
        self.transfer = None;
        HostProgress::Failed(error)
    }

    fn device_lost(&mut self) -> HostProgress {
        self.stage = Stage::Detached;
        self.transfer = None;
        self.queue.clear();
        HostProgress::Detached
    }
}
