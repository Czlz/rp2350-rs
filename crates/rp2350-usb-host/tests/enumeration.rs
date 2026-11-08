//! End-to-end enumeration and raw REPL tests against a scripted firmware
//! 面向脚本化固件的端到端枚举与 raw REPL 测试
//!
//! The scripted firmware reacts to exactly the registers and DPSRAM buffers
//! the real `MicroPython` CDC stack would touch, so these tests exercise the
//! host bridge and the device model together without a CPU.
//! 脚本化固件只响应真实 `MicroPython` CDC 栈会触碰的寄存器与 DPSRAM 缓冲，因此
//! 这些测试在没有 CPU 的情况下同时验证主机桥与设备模型。

use std::collections::VecDeque;

use rp2350_usb::controller::Controller;
use rp2350_usb::regs::{
    self, Alias, BUF_CTRL_AVAIL, BUF_CTRL_FULL, BUF_CTRL_LAST, BUF_CTRL_LEN_MASK, Direction,
    EP_CTRL_ENABLE, MAIN_CTRL_CONTROLLER_EN, SIE_CTRL_PULLUP_EN, SIE_STATUS_SETUP_REC,
};
use rp2350_usb::setup::{SetupPacket, request};
use rp2350_usb_host::bridge::{CdcEndpoints, HostProgress, UsbHostBridge};
use rp2350_usb_host::cdc::{CdcChannel, CdcLink};
use rp2350_usb_host::repl::{
    BANNER, BANNER_LINE, CTRL_A, CTRL_B, CTRL_D, RawReplSession, SOFT_REBOOT,
};

/// Device descriptor advertised by the scripted firmware
/// 脚本化固件声明的设备描述符
const DEVICE: [u8; 18] = [
    18, 0x01, 0x00, 0x02, 0x02, 0x02, 0x00, 0x40, 0x8A, 0x2E, 0x05, 0x00, 0x00, 0x02, 0x01, 0x02,
    0x03, 0x01,
];

/// Configuration descriptor: comm interface, data interface, bulk IN/OUT
/// 配置描述符：通信接口、数据接口、批量 IN/OUT
const CONFIG: [u8; 51] = [
    9, 0x02, 51, 0x00, 2, 1, 0, 0xC0, 0x32, // configuration
    9, 0x04, 0, 0, 1, 0x02, 0x02, 0x01, 0, // CDC comm interface
    5, 0x24, 0x00, 0x10, 0x01, // header functional
    5, 0x24, 0x02, 0x02, 0x02, // call management functional
    9, 0x04, 1, 0, 2, 0x0A, 0x00, 0x00, 0, // CDC data interface
    7, 0x05, 0x81, 0x02, 0x40, 0x00, 0x01, // bulk IN
    7, 0x05, 0x02, 0x02, 0x40, 0x00, 0x01, // bulk OUT
];

const EP1_IN_OFFSET: u32 = 0x180;
const EP2_OUT_OFFSET: u32 = 0x200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Friendly,
    Raw,
}

/// Minimal firmware stub for the USBCTRL registers and CDC buffers
/// 针对 USBCTRL 寄存器与 CDC 缓冲的最小固件桩
struct FakeFirmware {
    controller: Controller,
    configured: bool,
    mode: Mode,
    source: Vec<u8>,
    outbox: VecDeque<u8>,
    soft_reset_armed: bool,
}

impl FakeFirmware {
    fn new(controller: Controller) -> Self {
        Self {
            controller,
            configured: false,
            mode: Mode::Friendly,
            source: Vec::new(),
            outbox: VecDeque::new(),
            soft_reset_armed: false,
        }
    }

    /// Handle one control SETUP packet, if present / 若存在则处理一个控制 SETUP 包
    fn service_setup(&mut self) {
        let pending = self
            .controller
            .with_core(|core| core.read_reg(regs::SIE_STATUS) & SIE_STATUS_SETUP_REC != 0);
        if !pending {
            return;
        }
        let setup = self.controller.with_core(|core| core.setup_packet());
        let payload = self.response_payload(setup);
        if setup.is_standard() && setup.b_request == request::SET_CONFIGURATION {
            self.configure_endpoints();
        }
        if setup.is_standard() && setup.b_request == request::SET_ADDRESS {
            self.controller.with_core(|core| {
                let address = u8::try_from(setup.w_value).unwrap_or(0);
                core.write_reg(regs::ADDR_ENDP, Alias::Rw, u32::from(address));
            });
        }
        self.arm_ep0_in(&payload);
        // The status stage of an IN transfer is an OUT zero-length packet;
        // arm EP0 OUT so the host's status packet is accepted.
        // IN 传输的状态阶段是 OUT 零长包；提前准备 EP0 OUT 以便主机状态包被接收。
        self.controller.with_core(|core| {
            core.write_dpram_u32(regs::ep_buf_ctrl_offset(0, Direction::Out), BUF_CTRL_AVAIL);
        });
        self.controller
            .with_core(rp2350_usb::UsbCore::acknowledge_setup);
    }

    #[allow(clippy::unused_self)]
    fn response_payload(&self, setup: SetupPacket) -> Vec<u8> {
        if !setup.is_standard() {
            return Vec::new();
        }
        let mut data = match setup.b_request {
            request::GET_DESCRIPTOR => match (setup.w_value >> 8) as u8 {
                0x01 => DEVICE.to_vec(),
                0x02 => CONFIG.to_vec(),
                0x03 => vec![4, 0x03, 0x09, 0x04],
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        data.truncate(usize::from(setup.w_length).min(data.len()));
        data
    }

    fn arm_ep0_in(&self, data: &[u8]) {
        self.controller.with_core(|core| {
            for (index, byte) in data.iter().enumerate() {
                core.write_dpram_u8(regs::EP0_BUF_A + index, *byte);
            }
            let control = BUF_CTRL_AVAIL
                | BUF_CTRL_FULL
                | BUF_CTRL_LAST
                | (u32::try_from(data.len()).unwrap_or(0) & BUF_CTRL_LEN_MASK);
            core.write_dpram_u32(regs::ep_buf_ctrl_offset(0, Direction::In), control);
        });
    }

    fn configure_endpoints(&mut self) {
        self.controller.with_core(|core| {
            core.write_dpram_u32(
                regs::ep_ctrl_offset(1, Direction::In),
                EP_CTRL_ENABLE | (EP1_IN_OFFSET & regs::EP_CTRL_BUFFER_OFFSET_MASK),
            );
            core.write_dpram_u32(
                regs::ep_ctrl_offset(2, Direction::Out),
                EP_CTRL_ENABLE | (EP2_OUT_OFFSET & regs::EP_CTRL_BUFFER_OFFSET_MASK),
            );
            core.write_dpram_u32(regs::ep_buf_ctrl_offset(2, Direction::Out), BUF_CTRL_AVAIL);
        });
        self.configured = true;
    }

    /// Drain the bulk OUT endpoint and consume its bytes
    /// 取空批量 OUT 端点并消费其字节
    fn service_bulk_out(&mut self) {
        let control = self
            .controller
            .with_core(|core| core.read_dpram_u32(regs::ep_buf_ctrl_offset(2, Direction::Out)));
        if control & BUF_CTRL_FULL == 0 {
            return;
        }
        let len = (control & BUF_CTRL_LEN_MASK) as usize;
        let data: Vec<u8> = self.controller.with_core(|core| {
            (0..len)
                .map(|index| core.read_dpram_u8(EP2_OUT_OFFSET as usize + index))
                .collect()
        });
        self.controller.with_core(|core| {
            core.write_dpram_u32(regs::ep_buf_ctrl_offset(2, Direction::Out), BUF_CTRL_AVAIL);
        });
        self.consume(&data);
    }

    fn consume(&mut self, data: &[u8]) {
        for byte in data {
            match *byte {
                CTRL_A => {
                    self.mode = Mode::Raw;
                    self.source.clear();
                    self.soft_reset_armed = true;
                    self.emit(BANNER);
                }
                CTRL_B => self.mode = Mode::Friendly,
                CTRL_D if self.mode == Mode::Raw => {
                    if self.soft_reset_armed {
                        self.soft_reset_armed = false;
                        self.emit(SOFT_REBOOT);
                        self.emit(BANNER_LINE);
                    } else {
                        self.emit(b"OK");
                        self.emit(b"hi\r\n");
                        self.emit(&[CTRL_D, CTRL_D]);
                    }
                }
                _ if self.mode == Mode::Raw => {
                    self.source.push(*byte);
                    self.soft_reset_armed = false;
                }
                _ => {}
            }
        }
    }

    fn emit(&mut self, bytes: &[u8]) {
        self.outbox.extend(bytes.iter().copied());
    }

    /// Arm the bulk IN endpoint when the previous packet was consumed
    /// 上一包被取走后准备批量 IN 端点
    fn service_bulk_in(&mut self) {
        if self.outbox.is_empty() {
            return;
        }
        let in_control = self
            .controller
            .with_core(|core| core.read_dpram_u32(regs::ep_buf_ctrl_offset(1, Direction::In)));
        if in_control & BUF_CTRL_AVAIL != 0 {
            return;
        }
        let count = self.outbox.len().min(64);
        let chunk: Vec<u8> = self.outbox.drain(..count).collect();
        self.controller.with_core(|core| {
            for (index, byte) in chunk.iter().enumerate() {
                core.write_dpram_u8(EP1_IN_OFFSET as usize + index, *byte);
            }
            core.write_dpram_u32(
                regs::ep_buf_ctrl_offset(1, Direction::In),
                BUF_CTRL_AVAIL
                    | BUF_CTRL_FULL
                    | BUF_CTRL_LAST
                    | u32::try_from(chunk.len()).unwrap_or(0),
            );
        });
    }

    fn service(&mut self) {
        self.service_setup();
        if self.configured {
            self.service_bulk_out();
            self.service_bulk_in();
        }
    }
}

fn ready_controller() -> Controller {
    let controller = Controller::new();
    controller.with_core(|core| {
        core.write_reg(regs::MAIN_CTRL, Alias::Rw, MAIN_CTRL_CONTROLLER_EN);
        core.write_reg(regs::SIE_CTRL, Alias::Rw, SIE_CTRL_PULLUP_EN);
    });
    controller
}

fn drive_enumeration(controller: &Controller, firmware: &mut FakeFirmware) -> CdcEndpoints {
    let mut bridge = UsbHostBridge::new(controller.clone());
    let mut now = 0u64;
    for _ in 0..20_000 {
        firmware.service();
        match bridge.poll(now) {
            HostProgress::Wait(next) => now = now.max(next) + 1,
            HostProgress::Configured(endpoints) => return endpoints,
            HostProgress::Failed(error) => panic!("enumeration failed: {error:?}"),
            HostProgress::Detached => panic!("device detached during enumeration"),
        }
    }
    panic!("enumeration did not finish");
}

#[test]
fn enumerates_a_cdc_device_without_a_cpu() {
    let controller = ready_controller();
    let mut firmware = FakeFirmware::new(controller.clone());
    let endpoints = drive_enumeration(&controller, &mut firmware);
    assert_eq!(endpoints.in_address, 0x81);
    assert_eq!(endpoints.in_max_packet, 64);
    assert_eq!(endpoints.out_address, 0x02);
    assert_eq!(endpoints.out_max_packet, 64);
    assert_eq!(endpoints.interface, 1);
}

#[test]
fn runs_a_raw_repl_transaction_over_cdc() {
    let controller = ready_controller();
    let mut firmware = FakeFirmware::new(controller.clone());
    let endpoints = drive_enumeration(&controller, &mut firmware);

    let channel = CdcChannel::new(controller, endpoints);
    let mut session = RawReplSession::new(CdcLink::new(channel, move |_cycles| firmware.service()));
    session.enter_raw_repl(true).unwrap();
    let output = session.exec_raw(b"print('hi')").unwrap();
    assert_eq!(output.stdout, b"hi\r\n");
    assert_eq!(output.stderr, Vec::<u8>::new());
    session.exit_raw_repl().unwrap();
}
