//! Watchpoint coverage for the RSP dispatcher / RSP 分发器的监视点测试
//!
//! `rsp.rs` stays within the per-file code-line budget by keeping protocol
//! tests in `lib.rs`; the watchpoint scenarios live here as a separate unit-test
//! module of the same crate.
//! `rsp.rs` 通过把协议测试放在 `lib.rs` 控制单文件代码行数；监视点场景单独放在
//! 本模块，仍是同一 crate 的单元测试。

use rp2350_cpu::CpuState;

use crate::rsp::Session;
use crate::target::{GdbError, GdbTarget, ResumeMode, StopReason, StopReply, TargetStatus};

/// Mock target that records watchpoint requests and can simulate a hit
/// 记录监视点请求并可模拟命中的 mock 目标
#[derive(Default)]
struct WatchTarget {
    /// `(insert, address, length)` for every forwarded request
    /// 每次转发的请求 `(插入, 地址, 长度)`
    calls: Vec<(bool, u32, usize)>,
    /// Whether `qSupported` should advertise `watchpoint+`
    /// `qSupported` 是否应声明 `watchpoint+`
    supported: bool,
    /// Address reported as the trigger of the latest stop
    /// 最近一次停止所上报的触发地址
    hit: Option<u32>,
}

impl WatchTarget {
    /// Build a target that supports write watchpoints
    /// 构造支持写监视点的目标
    fn supporting() -> Self {
        Self {
            supported: true,
            ..Self::default()
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
        self.calls.push((true, address, length));
        Ok(())
    }

    fn clear_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        self.calls.push((false, address, length));
        Ok(())
    }

    fn watchpoint_hit(&self) -> Option<u32> {
        self.hit
    }

    fn resume(&mut self, _mode: ResumeMode) -> Result<StopReply, GdbError> {
        let reason = if self.hit.is_some() {
            StopReason::Watchpoint
        } else {
            StopReason::SingleStep
        };
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
    assert_eq!(session.target().calls, vec![(true, 0x2000_0000, 4)]);

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
        vec![(true, 0x2000_0000, 4), (false, 0x2000_0000, 4)]
    );
}

#[test]
fn read_and_access_watchpoints_reply_empty_packets() {
    let mut session = Session::new(WatchTarget::supporting());
    for packet in [
        "Z3,20000000,4",
        "z3,20000000,4",
        "Z4,20000000,4",
        "z4,20000000,4",
        "Z2,20000000,3",
    ] {
        assert_eq!(
            session.handle_packet(packet).as_deref(),
            Some(""),
            "{packet}"
        );
    }
    // Unsupported request types must not reach the target at all.
    // 不支持的请求类型不得传递给目标。
    assert_eq!(session.target().calls, Vec::new());
}

#[test]
fn watchpoint_stop_reply_uses_t05_and_the_hit_address() {
    let mut target = WatchTarget::supporting();
    target.hit = Some(0x2000_0010);
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
    // A target without support still reports the failure instead of claiming OK.
    // 不支持的目标仍会上报失败，而不是回复 OK。
    assert_eq!(
        unsupported.handle_packet("Z2,20000000,4").as_deref(),
        Some("E01")
    );
}
