//! Watchpoint and monitor coverage for the RSP dispatcher
//! RSP 分发器的监视点与 monitor 测试
//!
//! `rsp.rs` stays within the per-file code-line budget by keeping protocol
//! tests in `lib.rs`; the watchpoint and monitor scenarios live here as a
//! separate unit-test module of the same crate.
//! `rsp.rs` 通过把协议测试放在 `lib.rs` 控制单文件代码行数；监视点与 monitor
//! 场景单独放在本模块，仍是同一 crate 的单元测试。

use rp2350_cpu::CpuState;

use crate::rsp::Session;
use crate::target::{
    GdbError, GdbTarget, ResumeMode, StopReason, StopReply, TargetStatus, WatchKind,
};

/// Mock target that records watchpoint requests and can simulate a stop
/// 记录监视点请求并可模拟停止的 mock 目标
#[derive(Default)]
struct WatchTarget {
    /// `(insert, address, length, kind)` for every forwarded request
    /// 每次转发的请求 `(插入, 地址, 长度, 类型)`
    calls: Vec<(bool, u32, usize, WatchKind)>,
    /// Whether `qSupported` should advertise `watchpoint+`
    /// `qSupported` 是否应声明 `watchpoint+`
    supported: bool,
    /// Stop reason reported by the next resume / 下次恢复上报的停止原因
    reason: Option<StopReason>,
    /// Address reported as the trigger of the latest stop
    /// 最近一次停止所上报的触发地址
    hit: Option<u32>,
    /// Monitor commands received through `qRcmd` / 通过 `qRcmd` 收到的 monitor 命令
    monitor: Vec<String>,
    /// Text returned for the next monitor command / 下一条 monitor 命令的返回文本
    monitor_reply: String,
}

impl WatchTarget {
    /// Build a target that supports all three watchpoint kinds
    /// 构造支持三种监视点的目标
    fn supporting() -> Self {
        Self {
            supported: true,
            ..Self::default()
        }
    }

    /// Record one watchpoint request of `kind` / 记录一次指定类型的监视点请求
    fn record(&mut self, insert: bool, address: u32, length: usize, kind: WatchKind) {
        if self.supported {
            self.calls.push((insert, address, length, kind));
        }
    }
}

impl GdbTarget for WatchTarget {
    fn read_register(&mut self, _register: u32) -> Result<u32, GdbError> {
        Ok(0)
    }

    fn write_register(&mut self, _register: u32, _value: u32) -> Result<(), GdbError> {
        Ok(())
    }

    fn read_memory(&mut self, _address: u32, length: usize) -> Result<Vec<u8>, GdbError> {
        Ok(vec![0; length])
    }

    fn write_memory(&mut self, _address: u32, _bytes: &[u8]) -> Result<(), GdbError> {
        Ok(())
    }

    fn set_breakpoint(&mut self, _address: u32) -> Result<(), GdbError> {
        Ok(())
    }

    fn clear_breakpoint(&mut self, _address: u32) -> Result<(), GdbError> {
        Ok(())
    }

    fn has_breakpoint(&self, _address: u32) -> bool {
        false
    }

    fn supports_watchpoints(&self) -> bool {
        self.supported
    }

    fn set_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        if !self.supported {
            return Err(GdbError::Unsupported("write watchpoints"));
        }
        self.record(true, address, length, WatchKind::Write);
        Ok(())
    }

    fn clear_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        self.record(false, address, length, WatchKind::Write);
        Ok(())
    }

    fn set_read_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        if !self.supported {
            return Err(GdbError::Unsupported("read watchpoints"));
        }
        self.record(true, address, length, WatchKind::Read);
        Ok(())
    }

    fn clear_read_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        self.record(false, address, length, WatchKind::Read);
        Ok(())
    }

    fn set_access_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        if !self.supported {
            return Err(GdbError::Unsupported("access watchpoints"));
        }
        self.record(true, address, length, WatchKind::Access);
        Ok(())
    }

    fn clear_access_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        self.record(false, address, length, WatchKind::Access);
        Ok(())
    }

    fn watchpoint_hit(&self) -> Option<u32> {
        self.hit
    }

    fn monitor_command(&mut self, command: &str) -> String {
        self.monitor.push(command.to_string());
        self.monitor_reply.clone()
    }

    fn resume(&mut self, mode: ResumeMode) -> Result<StopReply, GdbError> {
        let reason = self.reason.unwrap_or(match mode {
            ResumeMode::Continue => StopReason::Breakpoint,
            ResumeMode::Step => StopReason::SingleStep,
        });
        Ok(StopReply {
            reason,
            pc: 0x2000_0100,
        })
    }

    fn status(&self) -> TargetStatus {
        TargetStatus {
            state: CpuState::Halted,
            stop_reason: None,
            pc: 0x2000_0100,
            total_cycles: 0,
        }
    }
}

#[test]
fn write_watchpoint_packets_are_idempotent() {
    let mut session = Session::new(WatchTarget::supporting());
    assert_eq!(
        session.handle_packet("Z2,20000000,4").as_deref(),
        Some("OK")
    );
    assert_eq!(
        session.handle_packet("Z2,20000000,4").as_deref(),
        Some("OK")
    );
    assert_eq!(
        session.target().calls,
        vec![(true, 0x2000_0000, 4, WatchKind::Write)]
    );

    assert_eq!(
        session.handle_packet("z2,20000000,4").as_deref(),
        Some("OK")
    );
    assert_eq!(
        session.handle_packet("z2,20000000,4").as_deref(),
        Some("OK")
    );
    assert_eq!(
        session.target().calls,
        vec![
            (true, 0x2000_0000, 4, WatchKind::Write),
            (false, 0x2000_0000, 4, WatchKind::Write)
        ]
    );
}

#[test]
fn read_watchpoint_packets_are_idempotent() {
    let mut session = Session::new(WatchTarget::supporting());
    assert_eq!(
        session.handle_packet("Z3,20000004,2").as_deref(),
        Some("OK")
    );
    assert_eq!(
        session.handle_packet("Z3,20000004,2").as_deref(),
        Some("OK")
    );
    assert_eq!(
        session.target().calls,
        vec![(true, 0x2000_0004, 2, WatchKind::Read)]
    );
    assert_eq!(
        session.handle_packet("z3,20000004,2").as_deref(),
        Some("OK")
    );
    assert_eq!(
        session.handle_packet("z3,20000004,2").as_deref(),
        Some("OK")
    );
    assert_eq!(
        session.target().calls,
        vec![
            (true, 0x2000_0004, 2, WatchKind::Read),
            (false, 0x2000_0004, 2, WatchKind::Read)
        ]
    );
}

#[test]
fn access_watchpoint_packets_are_idempotent() {
    let mut session = Session::new(WatchTarget::supporting());
    assert_eq!(
        session.handle_packet("Z4,20000008,8").as_deref(),
        Some("OK")
    );
    assert_eq!(
        session.handle_packet("Z4,20000008,8").as_deref(),
        Some("OK")
    );
    assert_eq!(
        session.handle_packet("z4,20000008,8").as_deref(),
        Some("OK")
    );
    assert_eq!(
        session.target().calls,
        vec![
            (true, 0x2000_0008, 8, WatchKind::Access),
            (false, 0x2000_0008, 8, WatchKind::Access)
        ]
    );
}

#[test]
fn watchpoint_kinds_are_tracked_independently() {
    let mut session = Session::new(WatchTarget::supporting());
    for packet in ["Z2,20000000,4", "Z3,20000000,4", "Z4,20000000,4"] {
        assert_eq!(
            session.handle_packet(packet).as_deref(),
            Some("OK"),
            "{packet}"
        );
    }
    // 同一地址的三种类型互不干扰：删除写监视点不会影响读/访问监视点。
    // The three kinds at one address are independent: removing the write watch
    // leaves the read and access watches armed.
    assert_eq!(
        session.handle_packet("z2,20000000,4").as_deref(),
        Some("OK")
    );
    assert_eq!(
        session.target().calls,
        vec![
            (true, 0x2000_0000, 4, WatchKind::Write),
            (true, 0x2000_0000, 4, WatchKind::Read),
            (true, 0x2000_0000, 4, WatchKind::Access),
            (false, 0x2000_0000, 4, WatchKind::Write)
        ]
    );
}

#[test]
fn invalid_watchpoint_widths_are_not_forwarded() {
    let mut session = Session::new(WatchTarget::supporting());
    for packet in [
        "Z2,20000000,3",
        "Z3,20000000,0",
        "Z3,20000000,3",
        "Z4,20000000,16",
        "z4,20000000,3",
    ] {
        assert_eq!(
            session.handle_packet(packet).as_deref(),
            Some(""),
            "{packet}"
        );
    }
    // 非法宽度不得传递给目标，也不得误报成功。
    // Invalid widths must not reach the target nor claim success.
    assert_eq!(session.target().calls, Vec::new());
}

#[test]
fn watchpoint_stop_reply_uses_t05_and_the_hit_address() {
    let mut target = WatchTarget::supporting();
    target.hit = Some(0x2000_0010);
    target.reason = Some(StopReason::Watchpoint);
    let mut session = Session::new(target);

    assert_eq!(
        session.handle_packet("c").as_deref(),
        Some("T05watch:20000010;")
    );
    assert_eq!(
        session.handle_packet("vCont;c").as_deref(),
        Some("T05watch:20000010;")
    );
}

#[test]
fn read_watchpoint_stop_reply_uses_rwatch() {
    let mut target = WatchTarget::supporting();
    target.hit = Some(0x2000_0020);
    target.reason = Some(StopReason::ReadWatch);
    let mut session = Session::new(target);

    assert_eq!(
        session.handle_packet("c").as_deref(),
        Some("T05rwatch:20000020;")
    );
    assert_eq!(
        session.handle_packet("s").as_deref(),
        Some("T05rwatch:20000020;")
    );
}

#[test]
fn access_watchpoint_stop_reply_uses_awatch() {
    let mut target = WatchTarget::supporting();
    target.hit = Some(0x2000_0030);
    target.reason = Some(StopReason::AccessWatch);
    let mut session = Session::new(target);

    assert_eq!(
        session.handle_packet("vCont;c:1").as_deref(),
        Some("T05awatch:20000030;")
    );
}

#[test]
fn watchpoint_stop_without_an_address_degrades_to_t05() {
    let mut target = WatchTarget::supporting();
    target.reason = Some(StopReason::ReadWatch);
    let mut session = Session::new(target);
    assert_eq!(session.handle_packet("c").as_deref(), Some("T05"));
}

#[test]
fn q_supported_advertises_watchpoints_only_when_supported() {
    let mut supported = Session::new(WatchTarget::supporting());
    assert!(
        supported
            .handle_packet("qSupported")
            .expect("reply")
            .contains("watchpoint+")
    );

    let mut unsupported = Session::new(WatchTarget::default());
    assert!(
        !unsupported
            .handle_packet("qSupported")
            .expect("reply")
            .contains("watchpoint+")
    );
    // 不支持的目标仍会上报失败，而不是回复 OK。
    // A target without support still reports the failure instead of claiming OK.
    for packet in ["Z2,20000000,4", "Z3,20000000,4", "Z4,20000000,4"] {
        assert_eq!(
            unsupported.handle_packet(packet).as_deref(),
            Some("E01"),
            "{packet}"
        );
    }
}

#[test]
fn boxed_targets_forward_read_watch_and_monitor_hooks() {
    let mut target = WatchTarget::supporting();
    target.monitor_reply = "core0: ok\n".to_string();
    let mut boxed: Box<dyn GdbTarget + Send> = Box::new(target);

    // `GdbTarget for Box<dyn GdbTarget + Send>` must forward the newer hooks;
    // otherwise the defaults would silently disable read/access watchpoints and
    // monitor commands on the real TCP server.
    // `GdbTarget for Box<dyn GdbTarget + Send>` 必须转发新增钩子，否则默认实现
    // 会在真实 TCP 服务上静默禁用读/访问监视点与 monitor 命令。
    boxed.set_read_watchpoint(0x2000_0004, 4).expect("rwatch");
    boxed.set_access_watchpoint(0x2000_0008, 4).expect("awatch");
    boxed.clear_read_watchpoint(0x2000_0004, 4).expect("clear");
    boxed
        .clear_access_watchpoint(0x2000_0008, 4)
        .expect("clear");
    assert_eq!(boxed.monitor_command("info"), "core0: ok\n");

    let mut session: Session<Box<dyn GdbTarget + Send>> =
        Session::new(Box::new(WatchTarget::supporting()));
    assert_eq!(
        session.handle_packet("Z3,20000004,4").as_deref(),
        Some("OK")
    );
    assert_eq!(
        session.handle_packet("Z4,20000008,4").as_deref(),
        Some("OK")
    );
}

#[test]
fn qrcmd_decodes_hex_and_returns_hex_encoded_output() {
    let mut target = WatchTarget::supporting();
    target.monitor_reply = "core0 pc=0x10000000\n".to_string();
    let mut session = Session::new(target);

    // `info` = 696e666f / hex for "info"
    let reply = session.handle_packet("qRcmd,696e666f").expect("reply");
    assert_eq!(session.target().monitor, vec!["info".to_string()]);
    assert_eq!(reply, crate::encode_hex(b"core0 pc=0x10000000\n"));
}

#[test]
fn qrcmd_empty_output_replies_ok() {
    let mut session = Session::new(WatchTarget::supporting());
    // Unknown commands are accepted silently: the target returns no text.
    // 未知命令被静默接受：目标返回空文本。
    assert_eq!(
        session.handle_packet("qRcmd,756e6b6e6f776e").as_deref(),
        Some("OK")
    );
    assert_eq!(session.target().monitor, vec!["unknown".to_string()]);
    assert_eq!(session.handle_packet("qRcmd,").as_deref(), Some("OK"));
}

#[test]
fn qrcmd_rejects_malformed_hex_and_non_utf8_payloads() {
    let mut session = Session::new(WatchTarget::supporting());
    for packet in ["qRcmd,zz", "qRcmd,696e66f", "qRcmd,ff"] {
        assert_eq!(
            session.handle_packet(packet).as_deref(),
            Some("E01"),
            "{packet}"
        );
    }
    assert_eq!(session.target().monitor.len(), 0);
    // 非法 monitor 包后会话仍可用。
    // The session survives malformed monitor packets.
    assert_eq!(session.handle_packet("?").as_deref(), Some("S05"));
}

#[test]
fn qrcmd_without_a_payload_is_not_a_monitor_command() {
    let mut session = Session::new(WatchTarget::supporting());
    assert_eq!(session.handle_packet("qRcmd").as_deref(), Some(""));
    assert_eq!(session.target().monitor.len(), 0);
}
