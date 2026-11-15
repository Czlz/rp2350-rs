//! Real play source: bridge from `rp2350_cli::play::PlaySession` to [`PlaySource`]
//! 真实游玩来源：把 `rp2350_cli::play::PlaySession` 桥接到 [`PlaySource`]
//!
//! This module is the only place where the GUI crate reaches into the CLI crate.
//! The dependency points `rp2350-egui → rp2350-cli` (the CLI no longer depends on
//! the GUI), so a concrete worker can be attached by the native entry point while
//! [`PlayView`](super::PlayView) keeps talking to the [`PlaySource`] and
//! [`PlayStarter`] traits.
//! 本模块是 GUI crate 引用 CLI crate 的唯一位置。依赖方向为 `rp2350-egui →
//! rp2350-cli`（CLI 不再依赖 GUI），因此原生入口可以挂载具体 worker，而
//! [`PlayView`](super::PlayView) 仍只与 [`PlaySource`] 和 [`PlayStarter`] trait
//! 交互。
//!
//! [`CliPlayStarter`] parses the toolbar's frame text with [`FrameSpec::parse`],
//! boots a [`PlaySession`] with default [`MachineOptions`] (`arch: None` so the
//! image format picks the core, `max_cycles = u64::MAX` so the game runs until the
//! user stops it) and wraps it in [`CliPlaySource`].
//! [`CliPlayStarter`] 用 [`FrameSpec::parse`] 解析工具栏的帧文本，以默认
//! [`MachineOptions`]（`arch: None` 由映像格式选核，`max_cycles = u64::MAX`
//! 使游戏运行到用户停止）启动 [`PlaySession`]，再用 [`CliPlaySource`] 包装。

use std::sync::{Arc, Mutex, MutexGuard};

use rp2350_cli::play::{FrameSpec, PlayConfig, PlaySession};
use rp2350_cli::run::MachineOptions;

use super::{PlayFrame, PlayRequest, PlaySource, PlayStarter, PlayStats};

/// Instruction budget handed to the GUI worker: no limit
/// 交给 GUI worker 的指令预算：不限制
///
/// The user stops a game with the Stop button, so the worker must not halt on a
/// budget the way a batch `run` does.
/// 用户用「停止」按钮结束游戏，因此 worker 不能像批处理 `run` 那样因预算耗尽而停机。
const UNLIMITED_CYCLES: u64 = u64::MAX;

/// Play source backed by a live CLI play session / 由运行中的 CLI 游玩会话支撑的来源
///
/// Frames are converted lazily: the wrapped session publishes `Arc<Frame>` and the
/// view wants [`PlayFrame`], so the last converted frame is cached by index. A UI
/// repaint faster than the guest therefore reuses one `Arc` instead of copying the
/// pixel buffer again.
/// 帧按需转换：被包装的会话发布 `Arc<Frame>`，而视图需要 [`PlayFrame`]，因此按
/// 帧序号缓存最近一次转换结果。UI 重绘快于客户机时复用同一个 `Arc`，不会重复
/// 拷贝像素缓冲。
pub struct CliPlaySource {
    /// Wrapped play worker / 被包装的游玩 worker
    session: PlaySession,
    /// Last converted frame, keyed by its index / 最近转换的帧（按序号缓存）
    cached: Mutex<Option<Arc<PlayFrame>>>,
}

impl CliPlaySource {
    /// Wrap an already-running session / 包装一个已在运行的会话
    #[must_use]
    pub fn new(session: PlaySession) -> Self {
        Self {
            session,
            cached: Mutex::new(None),
        }
    }

    /// Borrow the wrapped session for host diagnostics
    /// 借用被包装的会话，供宿主诊断
    #[must_use]
    pub fn session(&self) -> &PlaySession {
        &self.session
    }
}

impl PlaySource for CliPlaySource {
    fn latest_frame(&self) -> Option<Arc<PlayFrame>> {
        let frame = self.session.latest_frame()?;
        let mut cached = lock(&self.cached);
        if let Some(previous) = cached.as_ref() {
            if previous.index == frame.index {
                return Some(Arc::clone(previous));
            }
        }
        let converted = Arc::new(PlayFrame {
            index: frame.index,
            steps: frame.steps,
            width: frame.width,
            height: frame.height,
            rgb565: frame.rgb565.clone(),
            readable: frame.readable,
        });
        *cached = Some(Arc::clone(&converted));
        Some(converted)
    }

    fn set_buttons(&self, mask: u16) {
        self.session.set_buttons(mask);
    }

    fn stats(&self) -> PlayStats {
        gui_stats(&self.session.stats(), self.session.replaced_frames())
    }

    fn is_running(&self) -> bool {
        self.session.is_running()
    }

    fn button_pins(&self) -> u16 {
        self.session.button_pins()
    }

    fn shutdown(self: Box<Self>) -> PlayStats {
        // 帧替换计数必须在 shutdown 消费会话前取走。
        // Read the replace count before shutdown consumes the session.
        let replaced = self.session.replaced_frames();
        let stats = self.session.shutdown();
        gui_stats(&stats, replaced)
    }
}

/// Factory that boots a real CLI play session / 启动真实 CLI 游玩会话的工厂
///
/// Attach it with [`PlayView::with_starter`](super::PlayView::with_starter) or
/// [`EmuApp::with_play_starter`](crate::EmuApp::with_play_starter); the native
/// entry point does exactly that, so `cargo run -p rp2350-egui` can start a game
/// from the Play tab.
/// 通过 [`PlayView::with_starter`](super::PlayView::with_starter) 或
/// [`EmuApp::with_play_starter`](crate::EmuApp::with_play_starter) 挂载；原生入口
/// 正是这样做的，因此 `cargo run -p rp2350-egui` 可以直接在「游戏」标签页启动。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CliPlayStarter;

impl PlayStarter for CliPlayStarter {
    fn start(&self, request: PlayRequest) -> Result<Box<dyn PlaySource>, String> {
        let frame = FrameSpec::parse(request.frame.trim())?;
        if request.steps_per_frame == 0 {
            return Err("steps per frame must be positive / 每帧步数必须大于 0".to_owned());
        }
        let config = PlayConfig {
            frame,
            steps_per_frame: request.steps_per_frame,
            // 无帧数上限、无轨迹输出：交互式游玩运行到用户停止。
            // No frame limit and no trace output: an interactive game runs until stopped.
            max_frames: None,
            button_pins: request.button_pins,
            audio: request.audio,
            trace_steps: 0,
        };
        let options = MachineOptions {
            // `None` 让 UF2 家族 / ELF 决定架构；CLI 的 `play` 子命令行为一致。
            // `None` lets the UF2 family / ELF pick the core, matching `play`.
            arch: None,
            max_cycles: UNLIMITED_CYCLES,
            strict_decode: false,
        };
        let session = PlaySession::start(request.image, options, config)?;
        Ok(Box::new(CliPlaySource::new(session)))
    }
}

/// Map CLI statistics onto the GUI snapshot, adding the replace counter
/// 把 CLI 统计映射为 GUI 快照，并补上帧替换计数
fn gui_stats(stats: &rp2350_cli::play::PlayStats, replaced: u64) -> PlayStats {
    PlayStats {
        frames: stats.frames,
        steps: stats.steps,
        fps: stats.fps,
        steps_per_second: stats.steps_per_second,
        replaced,
        halted: stats.halted.clone(),
    }
}

/// Lock a mutex, recovering the data when a worker panicked while holding it
/// 获取互斥锁；worker panic 导致锁中毒时仍取回数据
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
