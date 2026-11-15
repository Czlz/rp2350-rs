//! `vCont` action-list parsing and execution planning
//! `vCont` 动作列表解析与执行规划
//!
//! A `vCont` payload is a semicolon-separated action list. Each entry is
//! `<action>[:<thread-id>]`, where the action is `c`/`C<sig>` (continue),
//! `s`/`S<sig>` (single-step) or `t` (stop), and `-1`/`0` mean "all threads".
//! This module turns the wire text into [`Action`] entries (validating
//! duplicates, unknown actions and unknown thread ids) and then plans the
//! [`ThreadAction`] list handed to [`crate::target::GdbTarget::cont`].
//! `vCont` 载荷是以分号分隔的动作列表，每条为 `<动作>[:<线程号>]`：动作为
//! `c`/`C<sig>`（继续）、`s`/`S<sig>`（单步）或 `t`（停止），`-1`/`0` 表示全部
//! 线程。本模块把线上文本转换为 [`Action`] 条目（校验重复、未知动作与未知线程号），
//! 再规划交给 [`crate::target::GdbTarget::cont`] 的 [`ThreadAction`] 列表。

use crate::rsp::Error;
use crate::target::{ResumeMode, ThreadAction, ThreadId};

/// One parsed `vCont` action / 一条已解析的 `vCont` 动作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    /// `c` / `C<sig>`: continue the thread (`None` = default action)
    /// `c` / `C<sig>`：继续该线程（`None` 表示缺省动作）
    Continue(Option<ThreadId>),
    /// `s` / `S<sig>`: single-step the thread (`None` = default action)
    /// `s` / `S<sig>`：单步该线程（`None` 表示缺省动作）
    Step(Option<ThreadId>),
    /// `t`: stop every thread / `t`：停止全部线程
    Stop(Option<ThreadId>),
}

/// Parse a `vCont` action list into validated entries
/// 将 `vCont` 动作列表解析为已校验的条目
///
/// `known` is the target's thread table; an explicit id outside it is
/// rejected, so a single-core target answers `E01` for `:2`. Duplicate ids,
/// conflicting default actions, actions mixed with `t`, and unknown action
/// characters are all rejected with [`Error::InvalidArgument`], which the
/// session maps to `E01`. Repeating the *same* threadless action (`vCont;c;c`)
/// is idempotent and collapses to one default action.
/// `known` 是目标的线程表；表外的显式编号会被拒绝，因此单核目标对 `:2` 回复
/// `E01`。重复编号、相互冲突的缺省动作、与 `t` 混用以及未知动作字符都返回
/// [`Error::InvalidArgument`]，由会话映射为 `E01`。重复的**同一**无线程动作
/// （`vCont;c;c`）是幂等的，折叠为一条缺省动作。
pub(super) fn parse(text: &str, known: &[u32]) -> Result<Vec<Action>, Error> {
    let mut actions = Vec::new();
    let mut explicit = Vec::new();
    let mut default_resume: Option<ResumeMode> = None;
    let mut default_stop = false;

    for part in text.split(';').filter(|part| !part.is_empty()) {
        let (mode, thread) = parse_entry(part, known)?;
        match (mode, thread) {
            // `t` never mixes with resume actions: the intent of a list that
            // both stops and resumes is ambiguous, so reject it instead of
            // guessing.
            // `t` 不与恢复动作混用：既停止又继续的列表意图含糊，直接拒绝。
            (EntryMode::Stop, thread) => {
                if actions.iter().any(is_resume) {
                    return Err(Error::InvalidArgument);
                }
                if !default_stop {
                    default_stop = true;
                    actions.push(Action::Stop(thread));
                }
            }
            (EntryMode::Resume(mode), thread) => {
                if default_stop {
                    return Err(Error::InvalidArgument);
                }
                if let Some(number) = thread {
                    if explicit.contains(&number) {
                        return Err(Error::InvalidArgument);
                    }
                    explicit.push(number);
                } else if let Some(previous) = default_resume {
                    // 两条缺省动作同义（`vCont;c;c`）时折叠，不同义（`vCont;c;s`）时报错。
                    // Identical default actions collapse; a conflicting pair is an error.
                    if previous != mode {
                        return Err(Error::InvalidArgument);
                    }
                    continue;
                } else {
                    default_resume = Some(mode);
                }
                actions.push(match mode {
                    ResumeMode::Continue => Action::Continue(thread),
                    ResumeMode::Step => Action::Step(thread),
                });
            }
        }
    }

    if actions.is_empty() {
        return Err(Error::InvalidArgument);
    }
    Ok(actions)
}

/// Target of a `t` action / `t` 动作的目标线程
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stop {
    /// `t` without an id: every thread, so the caller picks the current thread
    /// `t` 未带编号：全部线程，由调用方选择当前线程
    AllThreads,
    /// `t:<tid>`: the named thread / `t:<tid>`：指定线程
    Thread(ThreadId),
}

/// Thread that a `t` action stops, if the list contains one
/// 若列表含 `t` 动作，返回其目标线程
///
/// `None` means the list has no `t` action at all; [`Stop::AllThreads`] is a
/// `t` without an id, which the caller maps to its selected thread.
/// `None` 表示列表没有 `t` 动作；[`Stop::AllThreads`] 是不带编号的 `t`，由调用方
/// 映射到当前选中线程。
pub(super) fn stop_thread(actions: &[Action]) -> Option<Stop> {
    actions.iter().find_map(|action| match action {
        Action::Stop(thread) => Some(match thread {
            Some(thread) => Stop::Thread(*thread),
            None => Stop::AllThreads,
        }),
        _ => None,
    })
}

/// Plan the resume request handed to the target / 规划交给目标的恢复请求
///
/// Steps come first; if any step exists the continue entries are dropped, which
/// is how `vCont;s:2;c` steps thread 2 and leaves thread 1 stopped. A default
/// (threadless) action collapses the matching entries into one whole-target
/// request, so `vCont;c:1;c:2` equals `vCont;c` and `vCont;s:2;s` steps every
/// core exactly once. Unlisted threads are not resumed at all: this
/// implementation chooses "stay stopped" over "apply the default action" for a
/// thread the client did not mention while other threads step.
/// 单步优先；存在单步动作时丢弃继续条目，这正是 `vCont;s:2;c` 只单步线程 2、线程 1
/// 保持停止的实现方式。缺省（无线程）动作会把同义条目折叠为一次整目标请求，因此
/// `vCont;c:1;c:2` 等于 `vCont;c`，`vCont;s:2;s` 每核只单步一次。未列出的线程完全
/// 不参与恢复：在其他线程单步时，本实现选择“保持停止”而不是“套用缺省动作”。
pub(super) fn plan(actions: &[Action], known: &[u32]) -> Vec<ThreadAction> {
    let steps: Vec<ThreadAction> = actions
        .iter()
        .filter_map(|action| match action {
            Action::Step(thread) => Some(ThreadAction {
                thread: *thread,
                mode: ResumeMode::Step,
            }),
            _ => None,
        })
        .collect();
    if steps.iter().any(|action| action.thread.is_none()) {
        // 缺省单步作用于全部线程，此时点名同一动作的线程号是冗余的，折叠成一次
        // 整目标单步，避免同一核被步进两次。
        // A default step covers every thread, so explicit ids naming the same
        // action are redundant: collapse to one whole-target step so no core is
        // stepped twice.
        return vec![ThreadAction {
            thread: None,
            mode: ResumeMode::Step,
        }];
    }
    if !steps.is_empty() {
        return steps;
    }

    let continues: Vec<ThreadAction> = actions
        .iter()
        .filter_map(|action| match action {
            Action::Continue(thread) => Some(ThreadAction {
                thread: *thread,
                mode: ResumeMode::Continue,
            }),
            _ => None,
        })
        .collect();
    let whole_target = continues.iter().any(|action| action.thread.is_none())
        || (!known.is_empty()
            && known.iter().all(|number| {
                continues
                    .iter()
                    .any(|action| action.thread == Some(ThreadId(*number)))
            }));
    if whole_target {
        return vec![ThreadAction {
            thread: None,
            mode: ResumeMode::Continue,
        }];
    }
    continues
}

/// Action kind plus its optional thread, before validation
/// 校验前的动作类型与其可选线程
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryMode {
    /// Continue or step / 继续或单步
    Resume(ResumeMode),
    /// Stop / 停止
    Stop,
}

/// Parse one `<action>[:<tid>]` entry / 解析一条 `<动作>[:<线程号>]` 条目
fn parse_entry(text: &str, known: &[u32]) -> Result<(EntryMode, Option<ThreadId>), Error> {
    let (action, thread) = match text.split_once(':') {
        Some((action, thread)) => (action, Some(parse_thread(thread, known)?)),
        None => (text, None),
    };
    let thread = thread.flatten();
    let mode = match action.as_bytes().first().copied() {
        Some(b'c') if action.len() == 1 => EntryMode::Resume(ResumeMode::Continue),
        Some(b'C') if is_signal(&action[1..]) => EntryMode::Resume(ResumeMode::Continue),
        Some(b's') if action.len() == 1 => EntryMode::Resume(ResumeMode::Step),
        Some(b'S') if is_signal(&action[1..]) => EntryMode::Resume(ResumeMode::Step),
        Some(b't') if action.len() == 1 => EntryMode::Stop,
        // Everything else (including `vCont;r` range steps) is unsupported.
        // 其余动作（含 `vCont;r` 区间单步）均不支持。
        _ => return Err(Error::InvalidArgument),
    };
    Ok((mode, thread))
}

/// Parse a `vCont` thread id / 解析 `vCont` 线程号
///
/// `-1` and `0` mean every thread and become `None`; any other id must exist in
/// the target's thread table.
/// `-1` 与 `0` 表示全部线程并解析为 `None`；其他编号必须存在于目标线程表中。
fn parse_thread(text: &str, known: &[u32]) -> Result<Option<ThreadId>, Error> {
    if text == "-1" {
        return Ok(None);
    }
    let number = u32::from_str_radix(text, 16).map_err(|_| Error::InvalidArgument)?;
    if number == 0 {
        return Ok(None);
    }
    if !known.contains(&number) {
        return Err(Error::InvalidArgument);
    }
    Ok(Some(ThreadId(number)))
}

/// Whether `text` is a one- or two-digit signal number (`C05`, `S0b`)
/// `text` 是否为一位或两位信号号（`C05`、`S0b`）
fn is_signal(text: &str) -> bool {
    !text.is_empty() && text.len() <= 2 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Whether an entry resumes a thread (used by the stop/resume conflict check)
/// 条目是否为恢复动作（用于停止/恢复冲突校验）
fn is_resume(action: &Action) -> bool {
    matches!(action, Action::Continue(_) | Action::Step(_))
}
