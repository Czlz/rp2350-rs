//! `vCont` mixed action-list coverage / `vCont` 混合动作列表测试
//!
//! These tests pin the RSP semantics implemented by `rsp/vcont.rs` plus
//! [`crate::target::GdbTarget::cont`]: one action per thread, step-before-continue
//! execution, and `E01` for every malformed list.
//! 本模块锁定 `rsp/vcont.rs` 与 [`crate::target::GdbTarget::cont`] 实现的 RSP
//! 语义：每个线程一个动作、先单步后继续，以及所有非法列表回复 `E01`。

use super::MultiTarget;
use crate::rsp::Session;
use crate::target::{GdbTarget, ResumeMode, StopReason, ThreadAction, ThreadId};

/// A mixed list must move only the stepped thread / 混合列表只能推进被单步的线程
#[test]
fn mixed_step_and_continue_steps_only_the_named_thread() {
    let mut session = Session::new(MultiTarget::dual());
    assert_eq!(
        session.handle_packet("vCont;s:2;c").as_deref(),
        Some("T05thread:2;")
    );
    // 缺省 `c` 只在列表没有单步时生效：本例中 core0 保持停止。
    // The default `c` only runs when the list has no step: core 0 stays stopped.
    assert_eq!(
        session.target().resumed,
        vec![(Some(ThreadId::CORE1), ResumeMode::Step)]
    );
}

#[test]
fn continue_before_step_still_only_steps_the_named_thread() {
    let mut session = Session::new(MultiTarget::dual());
    assert_eq!(
        session.handle_packet("vCont;c:1;s:2").as_deref(),
        Some("T05thread:2;")
    );
    assert_eq!(
        session.target().resumed,
        vec![(Some(ThreadId::CORE1), ResumeMode::Step)]
    );
}

#[test]
fn two_step_actions_step_both_threads_once() {
    let mut session = Session::new(MultiTarget::dual());
    assert_eq!(
        session.handle_packet("vCont;s:1;s:2").as_deref(),
        Some("T05thread:2;")
    );
    assert_eq!(
        session.target().resumed,
        vec![
            (Some(ThreadId::CORE0), ResumeMode::Step),
            (Some(ThreadId::CORE1), ResumeMode::Step)
        ]
    );
}

#[test]
fn repeated_identical_default_action_collapses_to_a_bare_continue() {
    let mut session = Session::new(MultiTarget::dual());
    assert_eq!(
        session.handle_packet("vCont;c;c").as_deref(),
        Some("T05thread:1;")
    );
    assert_eq!(
        session.target().resumed,
        vec![(None, ResumeMode::Continue)],
        "vCont;c;c must equal a bare c"
    );
}

#[test]
fn explicit_all_thread_continue_normalizes_to_a_whole_target_request() {
    let mut session = Session::new(MultiTarget::dual());
    assert_eq!(
        session.handle_packet("vCont;c:2;c:1").as_deref(),
        Some("T05thread:1;")
    );
    assert_eq!(
        session.target().resumed,
        vec![(None, ResumeMode::Continue)],
        "naming every thread collapses to the whole-target resume"
    );
}

#[test]
fn default_step_collapses_explicit_step_entries() {
    let mut session = Session::new(MultiTarget::dual());
    // 整目标单步上报目标自己的停止线程（此处为线程 1）。
    // A whole-target step reports the target's stopped thread (thread 1 here).
    assert_eq!(
        session.handle_packet("vCont;s:2;s").as_deref(),
        Some("T05thread:1;")
    );
    // 缺省单步已覆盖全部线程，点名线程 2 不应让它被步进两次。
    // The default step already covers every thread; naming thread 2 must not
    // step it twice.
    assert_eq!(
        session.target().resumed,
        vec![(None, ResumeMode::Step)],
        "a default step collapses to one whole-target step"
    );
}

#[test]
fn signal_suffixes_are_accepted_in_mixed_lists() {
    let mut session = Session::new(MultiTarget::dual());
    assert_eq!(
        session.handle_packet("vCont;S05:2;C05").as_deref(),
        Some("T05thread:2;")
    );
    assert_eq!(
        session.target().resumed,
        vec![(Some(ThreadId::CORE1), ResumeMode::Step)]
    );
}

#[test]
fn duplicate_thread_ids_reply_e01() {
    for packet in ["vCont;c:1;c:1", "vCont;s:2;c:2", "vCont;s:1;S05:1"] {
        let mut session = Session::new(MultiTarget::dual());
        assert_eq!(
            session.handle_packet(packet).as_deref(),
            Some("E01"),
            "{packet}"
        );
        assert_eq!(session.target().resumed, Vec::new(), "{packet}");
    }
}

#[test]
fn unknown_actions_and_empty_lists_reply_e01() {
    for packet in [
        "vCont;x",
        "vCont;r",
        "vCont;",
        "vCont;c:zz",
        "vCont;s:2:3",
        "vCont;C",
        "vCont;c;s",
        "vCont;t;c",
    ] {
        let mut session = Session::new(MultiTarget::dual());
        assert_eq!(
            session.handle_packet(packet).as_deref(),
            Some("E01"),
            "{packet}"
        );
        assert_eq!(session.target().resumed, Vec::new(), "{packet}");
    }
}

#[test]
fn out_of_range_thread_ids_reply_e01() {
    for packet in ["vCont;s:3", "vCont;c:9", "vCont;t:3", "vCont;s:2;c:3"] {
        let mut session = Session::new(MultiTarget::dual());
        assert_eq!(
            session.handle_packet(packet).as_deref(),
            Some("E01"),
            "{packet}"
        );
        assert_eq!(session.target().resumed, Vec::new(), "{packet}");
    }
}

#[test]
fn single_thread_targets_reject_thread_two() {
    let mut target = MultiTarget::dual();
    target.threads = 1;
    let mut session = Session::new(target);
    for packet in ["vCont;s:2", "vCont;c:2", "vCont;t:2"] {
        assert_eq!(
            session.handle_packet(packet).as_deref(),
            Some("E01"),
            "{packet}"
        );
    }
    assert_eq!(session.target().resumed, Vec::new());
    assert_eq!(
        session.handle_packet("vCont;s:1").as_deref(),
        Some("T05thread:1;")
    );
}

#[test]
fn stop_actions_report_sigint_with_the_named_thread() {
    let mut session = Session::new(MultiTarget::dual());
    assert_eq!(
        session.handle_packet("vCont;t").as_deref(),
        Some("T02thread:1;")
    );
    assert_eq!(
        session.handle_packet("vCont;t:2").as_deref(),
        Some("T02thread:2;")
    );
    // `t` 不恢复执行，因此没有任何 resume 记录。
    // `t` never resumes, so no resume call is recorded.
    assert_eq!(session.target().resumed, Vec::new());
}

#[test]
fn default_cont_impl_runs_steps_before_continues() {
    let mut target = MultiTarget::dual();
    let reply = target
        .cont(&[
            ThreadAction {
                thread: Some(ThreadId::CORE1),
                mode: ResumeMode::Continue,
            },
            ThreadAction {
                thread: None,
                mode: ResumeMode::Step,
            },
        ])
        .expect("cont");
    assert_eq!(reply.reason, StopReason::Breakpoint);
    // 有单步动作时只有单步条目执行，继续条目被丢弃。
    // With a step present only the step entry runs; the continue entry is dropped.
    assert_eq!(target.resumed, vec![(None, ResumeMode::Step)]);

    let mut only_continue = MultiTarget::dual();
    only_continue
        .cont(&[ThreadAction {
            thread: Some(ThreadId::CORE1),
            mode: ResumeMode::Continue,
        }])
        .expect("continue");
    assert_eq!(
        only_continue.resumed,
        vec![(Some(ThreadId::CORE1), ResumeMode::Continue)]
    );

    let mut empty = MultiTarget::dual();
    assert!(empty.cont(&[]).is_err(), "an empty action list must fail");
}
