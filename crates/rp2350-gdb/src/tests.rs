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
    GdbError, GdbTarget, ResumeMode, StopReason, StopReply, TargetStatus, ThreadId, WatchKind,
};

// `vCont` 动作列表测试拆到子模块，控制单文件行数。
// `vCont` action-list tests live in a submodule to keep this file compact.
mod vcont;

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
        Some("T05watch:20000010;thread:1;")
    );
    assert_eq!(
        session.handle_packet("vCont;c").as_deref(),
        Some("T05watch:20000010;thread:1;")
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
        Some("T05rwatch:20000020;thread:1;")
    );
    assert_eq!(
        session.handle_packet("s").as_deref(),
        Some("T05rwatch:20000020;thread:1;")
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
        Some("T05awatch:20000030;thread:1;")
    );
}

#[test]
fn watchpoint_stop_without_an_address_degrades_to_t05() {
    let mut target = WatchTarget::supporting();
    target.reason = Some(StopReason::ReadWatch);
    let mut session = Session::new(target);
    assert_eq!(session.handle_packet("c").as_deref(), Some("T05thread:1;"));
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
    assert_eq!(session.handle_packet("?").as_deref(), Some("T05thread:1;"));
}

#[test]
fn qrcmd_without_a_payload_is_not_a_monitor_command() {
    let mut session = Session::new(WatchTarget::supporting());
    assert_eq!(session.handle_packet("qRcmd").as_deref(), Some(""));
    assert_eq!(session.target().monitor.len(), 0);
}

/// Dual-thread mock target used by the thread-model tests
/// 线程模型测试使用的双线程 mock 目标
struct MultiTarget {
    /// Number of threads in the table / 线程表中的线程数
    threads: usize,
    /// Thread reported by `stopped_thread` / `stopped_thread` 上报的线程
    stopped: ThreadId,
    /// Stop reason returned by every resume / 每次恢复返回的停止原因
    reason: StopReason,
    /// Watchpoint hit address reported to the RSP layer
    /// 上报给 RSP 层的监视点命中地址
    hit: Option<u32>,
    /// `(thread, mode)` for every resume; `None` means all threads
    /// 每次恢复的 `(线程, 模式)`；`None` 表示全部线程
    resumed: Vec<(Option<ThreadId>, ResumeMode)>,
    /// `(thread, register)` for every thread register read
    /// 每次线程寄存器读的 `(线程, 寄存器)`
    reads: Vec<(ThreadId, u32)>,
    /// `(thread, register, value)` for every thread register write
    /// 每次线程寄存器写的 `(线程, 寄存器, 值)`
    writes: Vec<(ThreadId, u32, u32)>,
}

impl MultiTarget {
    /// Two-thread target with both cores alive / 两个核心均在线的双线程目标
    fn dual() -> Self {
        Self {
            threads: 2,
            stopped: ThreadId::CORE0,
            reason: StopReason::Breakpoint,
            hit: None,
            resumed: Vec::new(),
            reads: Vec::new(),
            writes: Vec::new(),
        }
    }
}

impl GdbTarget for MultiTarget {
    fn register_count(&self) -> u32 {
        1
    }

    fn thread_count(&self) -> usize {
        self.threads
    }

    fn thread_id(&self, index: usize) -> Option<ThreadId> {
        (index < self.threads).then(|| ThreadId(u32::try_from(index + 1).unwrap_or(u32::MAX)))
    }

    fn thread_name(&self, thread: ThreadId) -> String {
        match thread {
            ThreadId::CORE0 => "core0".to_string(),
            ThreadId::CORE1 => "core1".to_string(),
            other => format!("thread {}", other.number()),
        }
    }

    fn thread_state(&self, thread: ThreadId) -> Option<CpuState> {
        (thread.number() as usize <= self.threads).then_some(CpuState::Running)
    }

    fn read_register(&mut self, register: u32) -> Result<u32, GdbError> {
        Ok(0x1000 + register)
    }

    fn write_register(&mut self, _register: u32, _value: u32) -> Result<(), GdbError> {
        Ok(())
    }

    fn read_thread_register(&mut self, thread: ThreadId, register: u32) -> Result<u32, GdbError> {
        self.reads.push((thread, register));
        Ok((thread.number() << 8) | register)
    }

    fn write_thread_register(
        &mut self,
        thread: ThreadId,
        register: u32,
        value: u32,
    ) -> Result<(), GdbError> {
        self.writes.push((thread, register, value));
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

    fn watchpoint_hit(&self) -> Option<u32> {
        self.hit
    }

    fn resume(&mut self, mode: ResumeMode) -> Result<StopReply, GdbError> {
        self.resumed.push((None, mode));
        Ok(StopReply {
            reason: self.reason,
            pc: 0x2000_0100,
        })
    }

    fn continue_thread(&mut self, thread: ThreadId) -> Result<StopReply, GdbError> {
        self.resumed.push((Some(thread), ResumeMode::Continue));
        Ok(StopReply {
            reason: self.reason,
            pc: 0x2000_0100,
        })
    }

    fn step_thread(&mut self, thread: ThreadId) -> Result<StopReply, GdbError> {
        self.resumed.push((Some(thread), ResumeMode::Step));
        Ok(StopReply {
            reason: self.reason,
            pc: 0x2000_0100,
        })
    }

    fn stopped_thread(&self) -> ThreadId {
        self.stopped
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
fn qf_thread_info_enumerates_both_threads_and_ends_with_l() {
    let mut session = Session::new(MultiTarget::dual());
    assert_eq!(
        session.handle_packet("qfThreadInfo").as_deref(),
        Some("m1,2")
    );
    assert_eq!(session.handle_packet("qsThreadInfo").as_deref(), Some("l"));
    // A new `qfThreadInfo` restarts the enumeration.
    // 新的 `qfThreadInfo` 重新开始枚举。
    assert_eq!(
        session.handle_packet("qfThreadInfo").as_deref(),
        Some("m1,2")
    );
    assert_eq!(session.handle_packet("qsThreadInfo").as_deref(), Some("l"));
}

#[test]
fn q_thread_extra_info_returns_hex_encoded_names() {
    let mut session = Session::new(MultiTarget::dual());
    assert_eq!(
        session.handle_packet("qThreadExtraInfo,1").as_deref(),
        Some("636f726530")
    );
    assert_eq!(
        session.handle_packet("qThreadExtraInfo,2").as_deref(),
        Some("636f726531")
    );
    for packet in ["qThreadExtraInfo,3", "qThreadExtraInfo,zz"] {
        assert_eq!(
            session.handle_packet(packet).as_deref(),
            Some("E01"),
            "{packet}"
        );
    }
}

#[test]
fn thread_selection_routes_register_packets() {
    let mut session = Session::new(MultiTarget::dual());
    assert_eq!(session.handle_packet("Hg2").as_deref(), Some("OK"));
    assert_eq!(session.handle_packet("qC").as_deref(), Some("QC2"));
    assert_eq!(session.handle_packet("p0").as_deref(), Some("00020000"));
    assert_eq!(session.handle_packet("P0=78563412").as_deref(), Some("OK"));
    assert_eq!(session.handle_packet("g").as_deref(), Some("00020000"));
    assert_eq!(
        session.target().reads,
        vec![(ThreadId::CORE1, 0), (ThreadId::CORE1, 0)]
    );
    assert_eq!(
        session.target().writes,
        vec![(ThreadId::CORE1, 0, 0x1234_5678)]
    );

    // `Hg0`/`Hg-1` mean “any thread” and fall back to the stopped thread.
    // `Hg0`/`Hg-1` 表示“任意线程”，回退到停止线程。
    for packet in ["Hg0", "Hg-1"] {
        assert_eq!(
            session.handle_packet(packet).as_deref(),
            Some("OK"),
            "{packet}"
        );
        assert_eq!(session.handle_packet("qC").as_deref(), Some("QC1"));
    }
    assert_eq!(session.handle_packet("Hc2").as_deref(), Some("OK"));
    for packet in ["Hg3", "Hc3"] {
        assert_eq!(
            session.handle_packet(packet).as_deref(),
            Some("E01"),
            "{packet}"
        );
    }
    assert_eq!(session.handle_packet("Hx1").as_deref(), Some(""));
}

#[test]
fn thread_alive_query_checks_the_thread_table() {
    let mut session = Session::new(MultiTarget::dual());
    for packet in ["T1", "T2", "T0", "T-1"] {
        assert_eq!(
            session.handle_packet(packet).as_deref(),
            Some("OK"),
            "{packet}"
        );
    }
    for packet in ["T3", "Tzz", "T"] {
        assert_eq!(
            session.handle_packet(packet).as_deref(),
            Some("E01"),
            "{packet}"
        );
    }
}

#[test]
fn vcont_step_core1_targets_only_that_thread() {
    let mut session = Session::new(MultiTarget::dual());
    assert_eq!(
        session.handle_packet("vCont;s:2").as_deref(),
        Some("T05thread:2;")
    );
    assert_eq!(
        session.target().resumed,
        vec![(Some(ThreadId::CORE1), ResumeMode::Step)]
    );
}

#[test]
fn vcont_continue_core1_targets_only_that_thread() {
    let mut session = Session::new(MultiTarget::dual());
    assert_eq!(
        session.handle_packet("vCont;c:2").as_deref(),
        Some("T05thread:2;")
    );
    assert_eq!(
        session.target().resumed,
        vec![(Some(ThreadId::CORE1), ResumeMode::Continue)]
    );
}

#[test]
fn vcont_continue_all_and_explicit_lists_are_equivalent() {
    let mut session = Session::new(MultiTarget::dual());
    assert_eq!(
        session.handle_packet("vCont;c").as_deref(),
        Some("T05thread:1;")
    );
    assert_eq!(
        session.handle_packet("vCont;c:1;c:2").as_deref(),
        Some("T05thread:1;")
    );
    assert_eq!(
        session.target().resumed,
        vec![(None, ResumeMode::Continue), (None, ResumeMode::Continue)]
    );
}

#[test]
fn vcont_signal_forms_are_accepted() {
    let mut session = Session::new(MultiTarget::dual());
    assert_eq!(
        session.handle_packet("vCont;C05").as_deref(),
        Some("T05thread:1;")
    );
    assert_eq!(
        session.handle_packet("vCont;S05:2").as_deref(),
        Some("T05thread:2;")
    );
    assert_eq!(
        session.target().resumed,
        vec![
            (None, ResumeMode::Continue),
            (Some(ThreadId::CORE1), ResumeMode::Step)
        ]
    );
}

#[test]
fn vcont_stop_and_invalid_actions_reply_legally() {
    let mut session = Session::new(MultiTarget::dual());
    assert_eq!(
        session.handle_packet("vCont;t").as_deref(),
        Some("T02thread:1;")
    );
    assert_eq!(
        session.handle_packet("vCont;t:2").as_deref(),
        Some("T02thread:2;")
    );
    assert_eq!(session.handle_packet("vCont;x").as_deref(), Some("E01"));
    assert_eq!(session.handle_packet("vCont;c:9").as_deref(), Some("E01"));
    assert_eq!(session.handle_packet("vCont;c;s").as_deref(), Some("E01"));
    assert_eq!(session.target().resumed, Vec::new());
}

#[test]
fn stop_replies_carry_the_stopped_thread() {
    let mut target = MultiTarget::dual();
    target.stopped = ThreadId::CORE1;
    target.reason = StopReason::Breakpoint;
    let mut session = Session::new(target);
    // Plain `c` reports the thread that actually stopped.
    // 普通 `c` 上报实际停止的线程。
    assert_eq!(session.handle_packet("c").as_deref(), Some("T05thread:2;"));
    // `?` reuses the same stopped thread without resuming.
    // `?` 复用同一停止线程且不恢复执行。
    assert_eq!(session.handle_packet("?").as_deref(), Some("T05thread:2;"));
    assert_eq!(session.target().resumed.len(), 1);
}

#[test]
fn boxed_targets_forward_thread_hooks() {
    let mut boxed: Box<dyn GdbTarget + Send> = Box::new(MultiTarget::dual());
    assert_eq!(boxed.thread_count(), 2);
    assert_eq!(boxed.thread_id(0), Some(ThreadId::CORE0));
    assert_eq!(boxed.thread_id(1), Some(ThreadId::CORE1));
    assert_eq!(boxed.thread_name(ThreadId::CORE1), "core1");
    assert_eq!(boxed.thread_state(ThreadId::CORE0), Some(CpuState::Running));
    assert_eq!(
        boxed
            .read_thread_register(ThreadId::CORE1, 3)
            .expect("core1 register"),
        0x203
    );
    boxed
        .write_thread_register(ThreadId::CORE1, 0, 1)
        .expect("core1 write");
    assert_eq!(boxed.stopped_thread(), ThreadId::CORE0);
    assert_eq!(
        boxed.step_thread(ThreadId::CORE1).expect("step").reason,
        StopReason::Breakpoint
    );
    assert_eq!(
        boxed
            .continue_thread(ThreadId::CORE1)
            .expect("continue")
            .reason,
        StopReason::Breakpoint
    );
}

#[test]
fn watch_stop_reply_appends_the_thread_field() {
    let mut target = MultiTarget::dual();
    target.stopped = ThreadId::CORE1;
    target.reason = StopReason::Watchpoint;
    target.hit = Some(0x2000_0010);
    let mut session = Session::new(target);
    assert_eq!(
        session.handle_packet("vCont;c:1").as_deref(),
        Some("T05watch:20000010;thread:1;")
    );
    assert_eq!(
        session.handle_packet("vCont;c:2").as_deref(),
        Some("T05watch:20000010;thread:2;")
    );
}
