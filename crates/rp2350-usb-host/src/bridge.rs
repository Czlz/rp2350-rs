//! Host-side enumeration state machine driving the simulated controller
//! 驱动模拟控制器的主机侧枚举状态机

use std::collections::VecDeque;

use rp2350_usb::controller::Controller;
use rp2350_usb::endpoint::{InResponse, OutResponse};
use rp2350_usb::regs::{self, Direction};
use rp2350_usb::setup::{SetupPacket, request};

use crate::descriptor::{ConfigInfo, DescriptorError, DeviceInfo, parse_config, parse_device};
use crate::sof::SofScheduler;

/// Full-speed control endpoint maximum packet size
/// 全速控制端点最大包长
const MAX_CONTROL_PACKET: usize = 64;

/// Number of control-transfer trace entries kept for diagnostics
/// 为诊断保留的控制传输轨迹条目数
const TRACE_DEPTH: usize = 12;

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
    /// Transfer snapshot captured before a failure clears it
    /// 失败清空传输前保存的现场快照
    failed_transfer: Option<String>,
    /// Newest control-transfer events, oldest first / 最新的控制传输事件，最旧在前
    trace: VecDeque<String>,
    /// 1 ms SOF schedule driven by simulated cycles
    /// 由模拟周期驱动的 1 ms SOF 调度
    sof: SofScheduler,
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
            failed_transfer: None,
            trace: VecDeque::new(),
            sof: SofScheduler::new(Self::DEFAULT_SYSCLK_HZ),
        }
    }

    /// Override the system clock used for timing conversion
    /// 覆盖用于时序换算的系统时钟
    pub fn set_sysclk_hz(&mut self, hz: u32) {
        self.sysclk_hz = hz;
        self.sof.set_sysclk_hz(hz);
    }

    /// Frame number of the last SOF sent to the device
    /// 最近发送给设备的 SOF 帧号
    #[must_use]
    pub fn sof_frame(&self) -> u16 {
        self.sof.frame()
    }

    /// Cycle at which the next SOF frame is due
    /// 下一个 SOF 帧到期的周期
    #[must_use]
    pub fn sof_next_cycle(&self) -> u64 {
        self.sof.next_cycle()
    }

    /// Total SOF tokens issued so far / 已下发的 SOF 令牌总数
    #[must_use]
    pub fn sof_emitted(&self) -> u64 {
        self.sof.emitted()
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

    /// Human-readable enumeration snapshot for diagnostics
    /// 供诊断使用的可读枚举进度快照
    ///
    /// A stalled enumeration otherwise reports only the failing request code;
    /// the stage, phase, bytes transferred and queue depth tell whether the
    /// guest armed an endpoint at all.
    /// 卡住的枚举原本只报告失败请求码；阶段、传输阶段、已传字节数与队列
    /// 深度可以判断客户机是否确实布防了端点。
    #[must_use]
    pub fn progress_line(&self) -> String {
        let stage = match self.stage {
            Stage::Detached => "detached",
            Stage::Enumerating => "enumerating",
            Stage::Configured => "configured",
            Stage::Failed => "failed",
        };
        let transfer = self.transfer.as_ref().map_or_else(
            || {
                self.failed_transfer
                    .clone()
                    .unwrap_or_else(|| String::from("none"))
            },
            |t| {
                format!(
                    "kind={:?} phase={:?} req=0x{:02X} w_value=0x{:04X} \
                     w_length={} received={} sent={}",
                    t.request.kind,
                    t.phase,
                    t.request.setup.b_request,
                    t.request.setup.w_value,
                    t.request.setup.w_length,
                    t.received.len(),
                    t.sent
                )
            },
        );
        // EP0 寄存器快照与最近的阶段轨迹能把「设备显式 STALL」与「主机过早
        // 发送状态令牌 / 设备尚未布防」区分开。
        // The EP0 register snapshot plus the recent phase trace distinguish a
        // device-side STALL from a status token issued before the guest armed
        // the endpoint.
        let ep0 = self.ep0_snapshot();
        let trace = self.trace.iter().cloned().collect::<Vec<_>>().join(" | ");
        // SOF 节拍也进入快照：帧号停在某个值上说明 1 ms 调度没有推进。
        // The SOF cadence joins the snapshot: a frozen frame number means the
        // 1 ms schedule stopped advancing.
        let sof = format!(
            "frame={} next={} emitted={}",
            self.sof.frame(),
            self.sof.next_cycle(),
            self.sof.emitted()
        );
        format!(
            "stage={stage}, address={}, queued={}, transfer=[{transfer}], \
             device={}, config={}, sof=[{sof}], ep0[{ep0}], trace=[{trace}]",
            self.address,
            self.queue.len(),
            self.device.is_some(),
            self.config.is_some()
        )
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

    /// Snapshot of the endpoint-0 state a control transfer depends on
    /// 控制传输依赖的端点 0 状态快照
    ///
    /// `ep0_in`/`ep0_out` are the 16-bit buffer-control halves, `sie` is
    /// `SIE_STATUS`, `buff` is `BUFF_STATUS`, `cpu` is `BUFF_CPU_SHOULD_HANDLE`
    /// and `stall_arm` is `EP_STALL_ARM`. A `STALL` bit in `ep0_in` proves the
    /// device stalled the IN status stage instead of NAK-ing it.
    /// `ep0_in`/`ep0_out` 为 16 位缓冲控制半字，`sie` 为 `SIE_STATUS`，
    /// `buff` 为 `BUFF_STATUS`，`cpu` 为 `BUFF_CPU_SHOULD_HANDLE`，
    /// `stall_arm` 为 `EP_STALL_ARM`。`ep0_in` 中的 `STALL` 位可证明是设备
    /// 显式 STALL 了 IN 状态阶段，而不是返回 NAK。
    fn ep0_snapshot(&self) -> String {
        self.controller.with_core(|core| {
            let in_ctrl = core.read_dpram_u32(regs::ep_buf_ctrl_offset(0, Direction::In));
            let out_ctrl = core.read_dpram_u32(regs::ep_buf_ctrl_offset(0, Direction::Out));
            format!(
                "ep0_in=0x{:04X} ep0_out=0x{:04X} sie=0x{:08X} buff=0x{:08X} \
                 cpu=0x{:08X} stall_arm=0x{:08X}",
                in_ctrl & 0xFFFF,
                out_ctrl & 0xFFFF,
                core.read_reg(regs::SIE_STATUS),
                core.read_reg(regs::BUFF_STATUS),
                core.read_reg(regs::BUFF_CPU_SHOULD_HANDLE),
                core.read_reg(regs::EP_STALL_ARM),
            )
        })
    }

    /// Append one diagnostic trace entry, dropping the oldest when full
    /// 追加一条诊断轨迹条目，满时丢弃最旧条目
    fn trace_event(&mut self, event: String) {
        if self.trace.len() == TRACE_DEPTH {
            self.trace.pop_front();
        }
        self.trace.push_back(event);
    }

    fn begin_enumeration(&mut self, now: u64) {
        self.stage = Stage::Enumerating;
        self.address = 0;
        // 附着后从新的帧边界开始：总线复位期间不产生旧帧号。
        // Restart the frame grid at attach: no stale frame numbers survive the
        // bus reset.
        self.sof.reset(now);
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
    ///
    /// Ordering is fixed and mirrors the bus: every SOF frame due at `now` is
    /// delivered **first**, then at most one endpoint transaction is attempted.
    /// `now` is the simulated cycle count, so the cadence is derived from the
    /// simulation clock rather than wall time.
    /// 顺序固定且与总线一致：先下发所有在 `now` 到期的 SOF 帧，然后至多尝试一次
    /// 端点事务。`now` 是模拟周期数，因此节拍来自模拟时钟而非真实时间。
    pub fn poll(&mut self, now: u64) -> HostProgress {
        // SOF 必须先于本帧事务：TinyUSB 的 errata-15 路径用 SOF_RD 判断能否启动
        // 批量 IN 缓冲，若先发传输会看到陈旧帧号。
        // SOF must precede this frame's transactions: TinyUSB's errata-15 path
        // reads SOF_RD to decide whether it may start a bulk-IN buffer, and a
        // transfer issued first would observe a stale frame number.
        if self.stage != Stage::Detached {
            self.sof.advance(&self.controller, now);
        }

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
                self.trace_event(format!(
                    "setup kind={:?} req=0x{:02X} w_value=0x{:04X} w_index=0x{:04X} \
                     w_length={}",
                    request.kind,
                    request.setup.b_request,
                    request.setup.w_value,
                    request.setup.w_index,
                    request.setup.w_length
                ));
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
                InResponse::Stall => self.stall(now, request_code),
                InResponse::Data(packet) => {
                    let Some(transfer) = self.transfer.as_mut() else {
                        return HostProgress::Wait(now);
                    };
                    transfer.received.extend_from_slice(packet.bytes());
                    // USB 控制传输的 IN 数据阶段只能由「短包」或「收满 wLength」
                    // 结束；设备侧 BUF_CTRL_LAST 是硬件内部的缓冲标记，真实
                    // 主机根本看不到它。此前把它当作终止条件，导致 75 字节
                    // 配置描述符的第一个 64 字节满包就被判为完成，主机随即
                    // 进入 Status 阶段，而固件仍在等第二个 IN 令牌，双方死锁。
                    // An IN data stage ends only on a short packet or once
                    // wLength bytes arrived; BUF_CTRL_LAST is a device-internal
                    // buffer flag invisible to a real host. Treating it as the
                    // end condition made the first full 64-byte packet of a
                    // 75-byte configuration descriptor look complete, so the
                    // host moved to Status while the device awaited another IN.
                    let short_packet = packet.len() < MAX_CONTROL_PACKET;
                    let length_reached =
                        transfer.received.len() >= usize::from(transfer.request.setup.w_length);
                    if short_packet || length_reached {
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
                let chunk = remaining.min(MAX_CONTROL_PACKET);
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
                OutResponse::Stall => self.stall(now, request_code),
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
        // 状态阶段是控制传输最容易误判的阶段：OUT 请求的状态阶段由设备发
        // IN ZLP，IN 请求的状态阶段由主机发 OUT ZLP。先记录 EP0 现场再发令牌。
        // The status stage is where control transfers are easiest to misjudge:
        // an OUT request ends with a device IN ZLP, an IN request with a host
        // OUT ZLP. Snapshot EP0 before issuing the token.
        let snapshot = self.ep0_snapshot();
        let direction = request.setup.direction();
        let (outcome, progress) = if direction == Direction::In {
            let response = self.controller.host_out(0, &[]);
            match response {
                OutResponse::NoDevice => ("no-device", self.device_lost()),
                OutResponse::Nak => ("nak", self.nak(now)),
                OutResponse::Stall => ("stall", self.stall(now, request.setup.b_request)),
                OutResponse::Ack { last } => (
                    if last { "ack-last" } else { "ack" },
                    self.complete_transfer(now),
                ),
            }
        } else {
            let response = self.controller.host_in(0);
            match response {
                InResponse::NoDevice => ("no-device", self.device_lost()),
                InResponse::Nak => ("nak", self.nak(now)),
                InResponse::Stall => ("stall", self.stall(now, request.setup.b_request)),
                InResponse::Data(packet) => (
                    if packet.is_empty() {
                        "data-zlp"
                    } else {
                        "data"
                    },
                    self.complete_transfer(now),
                ),
            }
        };
        self.trace_event(format!(
            "status req=0x{:02X} dir={direction:?} {snapshot} -> {outcome}",
            request.setup.b_request
        ));
        progress
    }

    fn nak(&mut self, now: u64) -> HostProgress {
        self.retry_at = now + self.us_to_cycles(100);
        HostProgress::Wait(self.retry_at)
    }

    /// Skip an optional request the device stalled, or fail enumeration
    /// 跳过设备返回 STALL 的可选请求，否则判定枚举失败
    ///
    /// String descriptors are best effort: a device may legitimately stall them
    /// and enumeration must continue with the descriptors already parsed.
    /// 字符串描述符是尽力而为的：设备可以合法地对它们返回 STALL，枚举应携带已解析
    /// 的描述符继续。
    fn stall(&mut self, now: u64, request: u8) -> HostProgress {
        let optional = self.transfer.as_ref().is_some_and(|t| t.request.optional);
        if optional {
            self.transfer = None;
            self.retry_at = now + self.us_to_cycles(20);
            HostProgress::Wait(self.retry_at)
        } else {
            self.fail(HostError::Stall { request })
        }
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
        // 保留清除前的传输现场：失败时 progress_line 仍能显示卡在哪个阶段、
        // 已收/已发多少字节，否则只剩 "transfer=[none]" 无从定位。
        // Keep the transfer snapshot before clearing: progress_line then still
        // shows the stuck phase and byte counts instead of "transfer=[none]".
        if let Some(t) = self.transfer.as_ref() {
            self.failed_transfer = Some(format!(
                "kind={:?} phase={:?} req=0x{:02X} w_value=0x{:04X} \
                 w_length={} received={} sent={}",
                t.request.kind,
                t.phase,
                t.request.setup.b_request,
                t.request.setup.w_value,
                t.request.setup.w_length,
                t.received.len(),
                t.sent
            ));
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use rp2350_usb::UsbCore;
    use rp2350_usb::regs::{
        self, Alias, BUF_CTRL_AVAIL, BUF_CTRL_FULL, BUF_CTRL_LAST, BUF_CTRL_STALL,
        MAIN_CTRL_CONTROLLER_EN, SIE_CTRL_PULLUP_EN, SIE_STATUS_SETUP_REC,
    };

    /// An 18-byte device descriptor with string indices 1/2/3
    /// 带字符串索引 1/2/3 的 18 字节设备描述符
    const DEVICE_DESC: [u8; 18] = [
        18, 1, 0x00, 0x02, 0x02, 0x02, 0x00, 0x40, 0x09, 0x12, 0x02, 0x00, 0x00, 0x02, 0x01, 0x02,
        0x03, 0x01,
    ];

    /// A 46-byte CDC configuration: control interface 0, data interface 1,
    /// bulk IN 0x81 and bulk OUT 0x02.
    /// 46 字节 CDC 配置：控制接口 0、数据接口 1、批量 IN 0x81、批量 OUT 0x02。
    const CONFIG_DESC: [u8; 46] = [
        9, 2, 46, 0, 2, 1, 0, 0xC0, 0x01, // configuration
        9, 4, 0, 0, 1, 0x02, 0x02, 0x01, 0x00, // CDC control interface
        5, 0x24, 0x00, 0x10, 0x01, // CDC functional header
        9, 4, 1, 0, 2, 0x0A, 0x00, 0x00, 0x00, // CDC data interface
        7, 5, 0x81, 0x02, 0x40, 0, 1, // bulk IN
        7, 5, 0x02, 0x02, 0x40, 0, 1, // bulk OUT
    ];

    /// EP0 control-transfer phase the scripted firmware is serving
    /// 脚本化固件当前服务的 EP0 控制传输阶段
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Ep0 {
        Idle,
        InData,
        OutData(usize),
        InStatus,
        OutStatus,
    }

    /// Scripted guest that answers control transfers like TinyUSB's DCD
    /// 像 TinyUSB DCD 一样应答控制传输的脚本化客户机
    // 测试脚本用三个独立开关分别控制字符串 STALL、设备描述符 STALL 与静默，
    // 组合起来覆盖枚举的可选/必需/超时分支。
    // The test script needs three independent switches for string stalls,
    // device-descriptor stalls and silence to cover optional/required/timeout.
    #[allow(clippy::struct_excessive_bools)]
    struct FakeFirmware {
        controller: Controller,
        state: Ep0,
        stall_strings: bool,
        stall_device: bool,
        respond: bool,
        configured: bool,
        /// SOF frame observed when the first SETUP arrived / 首个 SETUP 到达时的 SOF 帧号
        first_setup_frame: Option<u16>,
    }

    impl FakeFirmware {
        fn new() -> Self {
            let controller = Controller::new();
            controller.with_core(|core| {
                core.write_reg(regs::MAIN_CTRL, Alias::Rw, MAIN_CTRL_CONTROLLER_EN);
                core.write_reg(regs::SIE_CTRL, Alias::Rw, SIE_CTRL_PULLUP_EN);
                core.host_attach();
            });
            Self {
                controller,
                state: Ep0::Idle,
                stall_strings: false,
                stall_device: false,
                respond: true,
                configured: false,
                first_setup_frame: None,
            }
        }

        fn payload(setup: SetupPacket) -> Vec<u8> {
            match (setup.b_request, setup.w_value >> 8) {
                (request::GET_DESCRIPTOR, 0x01) => DEVICE_DESC.to_vec(),
                (request::GET_DESCRIPTOR, 0x02) => CONFIG_DESC.to_vec(),
                (request::GET_DESCRIPTOR, 0x03) => vec![4, 3, 0x09, 0x04],
                _ => Vec::new(),
            }
        }

        fn arm_in(core: &mut UsbCore, data: &[u8]) {
            let offset = regs::ep_buf_ctrl_offset(0, Direction::In);
            core.write_dpram_u32(offset, 0);
            for (index, byte) in data.iter().enumerate() {
                core.write_dpram_u8(UsbCore::ep0_buffer() + index, *byte);
            }
            let mut ctrl = BUF_CTRL_AVAIL | BUF_CTRL_FULL | data.len() as u32;
            if data.len() < MAX_CONTROL_PACKET {
                ctrl |= BUF_CTRL_LAST;
            }
            core.write_dpram_u32(offset, ctrl);
        }

        fn arm_out(core: &mut UsbCore) {
            core.write_dpram_u32(regs::ep_buf_ctrl_offset(0, Direction::Out), BUF_CTRL_AVAIL);
        }

        fn begin(&mut self, setup: SetupPacket) {
            let length = usize::from(setup.w_length);
            let descriptor_type = setup.w_value >> 8;
            let payload = Self::payload(setup);
            let stall = (self.stall_strings
                && setup.b_request == request::GET_DESCRIPTOR
                && descriptor_type == 0x03)
                || (self.stall_device
                    && setup.b_request == request::GET_DESCRIPTOR
                    && descriptor_type == 0x01);
            if setup.b_request == request::SET_CONFIGURATION {
                self.configured = true;
            }
            self.state = if setup.w_length == 0 {
                Ep0::InStatus
            } else if setup.direction() == Direction::Out {
                Ep0::OutData(length)
            } else {
                Ep0::InData
            };
            if stall {
                self.state = Ep0::Idle;
            }
            let data = payload[..length.min(payload.len()).min(MAX_CONTROL_PACKET)].to_vec();
            self.controller.with_core(|core| {
                if stall {
                    core.write_dpram_u32(
                        regs::ep_buf_ctrl_offset(0, Direction::In),
                        BUF_CTRL_STALL,
                    );
                } else if setup.w_length == 0 || setup.direction() == Direction::In {
                    Self::arm_in(core, &data);
                } else {
                    Self::arm_out(core);
                }
            });
        }

        fn after_in(&mut self) {
            match self.state {
                Ep0::InData => {
                    self.controller.with_core(Self::arm_out);
                    self.state = Ep0::OutStatus;
                }
                Ep0::InStatus => self.state = Ep0::Idle,
                _ => {}
            }
        }

        fn after_out(&mut self) {
            match self.state {
                Ep0::OutData(remaining) => {
                    let left = remaining.saturating_sub(MAX_CONTROL_PACKET);
                    if left > 0 {
                        self.controller.with_core(Self::arm_out);
                        self.state = Ep0::OutData(left);
                    } else {
                        self.controller.with_core(|core| Self::arm_in(core, &[]));
                        self.state = Ep0::InStatus;
                    }
                }
                Ep0::OutStatus => self.state = Ep0::Idle,
                _ => {}
            }
        }

        /// Handle one guest-side interrupt: SETUP first, then buffer status
        /// 处理一次客户机侧中断：先 SETUP，再处理缓冲状态
        fn service(&mut self) {
            if !self.respond {
                return;
            }
            let (setup, buff) = self.controller.with_core(|core| {
                let setup = (core.read_reg(regs::SIE_STATUS) & SIE_STATUS_SETUP_REC != 0)
                    .then(|| core.setup_packet());
                (setup, core.read_reg(regs::BUFF_STATUS))
            });
            if let Some(setup) = setup {
                if self.first_setup_frame.is_none() {
                    // 记录首个 SETUP 时的帧号：它证明同一轮 poll 里 SOF 先于事务。
                    // Record the frame seen at the first SETUP: it proves the SOF
                    // preceded the transaction inside the same poll call.
                    self.first_setup_frame = Some(
                        self.controller
                            .with_core(|core| core.read_reg(regs::SOF_RD))
                            as u16,
                    );
                }
                self.controller.with_core(UsbCore::acknowledge_setup);
                self.begin(setup);
                return;
            }
            if buff & regs::buff_status_bit(0, Direction::In) != 0 {
                self.controller.with_core(|core| {
                    core.write_reg(
                        regs::BUFF_STATUS,
                        Alias::Rw,
                        regs::buff_status_bit(0, Direction::In),
                    );
                });
                self.after_in();
            }
            if buff & regs::buff_status_bit(0, Direction::Out) != 0 {
                self.controller.with_core(|core| {
                    core.write_reg(
                        regs::BUFF_STATUS,
                        Alias::Rw,
                        regs::buff_status_bit(0, Direction::Out),
                    );
                });
                self.after_out();
            }
        }

        fn detach(&self) {
            self.controller.with_core(|core| {
                core.write_reg(regs::SIE_CTRL, Alias::Clear, SIE_CTRL_PULLUP_EN);
            });
        }
    }

    /// Drive the bridge and the scripted guest against one shared timeline
    /// 在同一时间线上驱动主机桥与脚本化客户机
    fn enumerate(firmware: &mut FakeFirmware) -> (HostProgress, UsbHostBridge) {
        let mut bridge = UsbHostBridge::new(firmware.controller.clone());
        let mut now = 0u64;
        for _ in 0..20_000 {
            firmware.service();
            match bridge.poll(now) {
                HostProgress::Wait(next) => {
                    now = next.max(now + 1);
                    firmware.service();
                }
                outcome => return (outcome, bridge),
            }
        }
        panic!("enumeration did not terminate");
    }

    #[test]
    fn enumeration_discovers_cdc_endpoints_and_skips_stalled_strings() {
        let mut firmware = FakeFirmware::new();
        // Devices may STALL string descriptors; enumeration must still finish.
        // 设备可以对字符串描述符返回 STALL；枚举仍必须完成。
        firmware.stall_strings = true;
        let (outcome, bridge) = enumerate(&mut firmware);
        let HostProgress::Configured(endpoints) = outcome else {
            panic!("expected configured, got {outcome:?}");
        };
        assert_eq!(
            endpoints,
            CdcEndpoints {
                in_address: 0x81,
                in_max_packet: 64,
                out_address: 0x02,
                out_max_packet: 64,
                interface: 1,
            }
        );
        assert_eq!(bridge.address(), 1);
        assert!(
            firmware.configured,
            "SET_CONFIGURATION must reach the guest"
        );
        assert_eq!(
            bridge.device_info().map(|info| info.vendor_id),
            Some(0x1209)
        );
        assert_eq!(
            bridge.config_info().and_then(|info| info.data_interface),
            Some(1)
        );
        assert!(bridge.progress_line().contains("stage=configured"));
    }

    #[test]
    fn stalled_required_request_fails_enumeration() {
        let mut firmware = FakeFirmware::new();
        firmware.stall_device = true;
        let (outcome, bridge) = enumerate(&mut firmware);
        assert_eq!(
            outcome,
            HostProgress::Failed(HostError::Stall {
                request: request::GET_DESCRIPTOR
            })
        );
        assert!(bridge.progress_line().contains("stage=failed"));
    }

    #[test]
    fn silent_firmware_times_out() {
        let mut firmware = FakeFirmware::new();
        firmware.respond = false;
        let (outcome, _) = enumerate(&mut firmware);
        assert!(matches!(
            outcome,
            HostProgress::Failed(HostError::Timeout { .. })
        ));
    }

    #[test]
    fn losing_the_pull_up_detaches_the_device() {
        let mut firmware = FakeFirmware::new();
        firmware.respond = false;
        let mut bridge = UsbHostBridge::new(firmware.controller.clone());
        let mut now = 0u64;
        // First poll attaches, resets the bus and starts the request queue.
        // 首次轮询完成附着、总线复位并启动请求队列。
        let _ = bridge.poll(now);
        now += 1_000_000;
        let _ = bridge.poll(now);
        firmware.detach();
        assert_eq!(bridge.poll(now + 1), HostProgress::Detached);
    }

    /// One 1 ms frame at the bridge's default 150 MHz system clock
    /// 主机桥默认 150 MHz 系统时钟下 1 毫秒帧的周期数
    const MS: u64 = 150_000;

    fn sof_rd(firmware: &FakeFirmware) -> u16 {
        firmware
            .controller
            .with_core(|core| core.read_reg(regs::SOF_RD) as u16)
    }

    fn dev_sof_pending(firmware: &FakeFirmware) -> bool {
        firmware
            .controller
            .with_core(|core| core.intr() & regs::INTR_DEV_SOF != 0)
    }

    #[test]
    fn poll_issues_one_sof_per_simulated_millisecond() {
        // 1 ms of simulated time, not wall time: 0.5 ms never produces a frame.
        // 以 1 毫秒模拟时间为节拍，而非真实时间：0.5 毫秒不产生新帧。
        let mut firmware = FakeFirmware::new();
        firmware.respond = false;
        let mut bridge = UsbHostBridge::new(firmware.controller.clone());
        let _ = bridge.poll(0);
        assert_eq!(bridge.sof_frame(), 0);
        assert_eq!(bridge.sof_next_cycle(), MS);
        assert_eq!(sof_rd(&firmware), 0);

        let _ = bridge.poll(MS / 2);
        assert_eq!(bridge.sof_frame(), 0);
        assert_eq!(sof_rd(&firmware), 0);

        let _ = bridge.poll(MS);
        assert_eq!(bridge.sof_frame(), 1);
        assert_eq!(sof_rd(&firmware), 1);
        assert_eq!(bridge.sof_emitted(), 1);

        let _ = bridge.poll(MS + MS / 2);
        assert_eq!(bridge.sof_frame(), 1);
        assert_eq!(sof_rd(&firmware), 1);

        let _ = bridge.poll(2 * MS);
        assert_eq!(bridge.sof_frame(), 2);
        assert_eq!(sof_rd(&firmware), 2);
        assert_eq!(bridge.sof_next_cycle(), 3 * MS);
    }

    #[test]
    fn sof_frame_is_fresh_for_the_errata_15_path() {
        // TinyUSB caches SOF_RD in e15_last_sof; every millisecond must present
        // a different frame number together with a DEV_SOF latch.
        // TinyUSB 把 SOF_RD 缓存到 e15_last_sof；每毫秒都必须呈现不同帧号并
        // 锁存 DEV_SOF。
        let mut firmware = FakeFirmware::new();
        firmware.respond = false;
        let mut bridge = UsbHostBridge::new(firmware.controller.clone());
        let _ = bridge.poll(0);
        let first = sof_rd(&firmware);

        let _ = bridge.poll(MS);
        assert!(dev_sof_pending(&firmware), "a SOF must latch DEV_SOF");
        let second = sof_rd(&firmware);
        assert_ne!(second, first, "errata-15 must see a new frame");

        let _ = bridge.poll(2 * MS);
        assert!(dev_sof_pending(&firmware));
        let third = sof_rd(&firmware);
        assert_ne!(third, second);
        assert_eq!(third, 2);
    }

    #[test]
    fn sof_is_delivered_before_the_transaction_it_precedes() {
        // The first transfer becomes possible 10 ms after attach; all 10 frame
        // tokens must already be on the bus when the device sees the SETUP.
        // 首次传输在附着后 10 毫秒才可能发生；设备看到 SETUP 时 10 个帧令牌
        // 必须已经下发。
        let mut firmware = FakeFirmware::new();
        let (outcome, bridge) = enumerate(&mut firmware);
        assert!(matches!(outcome, HostProgress::Configured(_)));
        assert_eq!(
            firmware.first_setup_frame,
            Some(10),
            "SOF for the frame must precede the SETUP token"
        );
        assert!(bridge.sof_frame() >= 10);
    }

    #[test]
    fn sof_stops_while_the_device_is_detached() {
        let mut firmware = FakeFirmware::new();
        firmware.respond = false;
        let mut bridge = UsbHostBridge::new(firmware.controller.clone());
        let _ = bridge.poll(0);
        let _ = bridge.poll(3 * MS);
        assert_eq!(bridge.sof_frame(), 3);

        firmware.detach();
        assert_eq!(bridge.poll(4 * MS), HostProgress::Detached);
        let frame_at_detach = bridge.sof_frame();
        assert_eq!(frame_at_detach, 4);
        // No frames exist on an unattached bus. / 未附着总线上没有帧。
        let _ = bridge.poll(10 * MS);
        assert_eq!(bridge.sof_frame(), frame_at_detach);
        assert_eq!(sof_rd(&firmware), 4);
    }
}
