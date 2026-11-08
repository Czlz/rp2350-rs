//! MicroPython raw REPL protocol over an abstract byte link
//! 基于抽象字节链路的 MicroPython raw REPL 协议
//!
//! The protocol is taken from `lib/utils/pyexec.c` and `tools/pyboard.py`:
//! 协议语义取自 `lib/utils/pyexec.c` 与 `tools/pyboard.py`：
//!
//! - `\r\x03` interrupts a running program.
//!   `\r\x03` 中断正在运行的程序。
//! - `\r\x01` enters raw REPL and the device answers
//!   `raw REPL; CTRL-B to exit\r\n>`.
//!   `\r\x01` 进入 raw REPL，设备回应 `raw REPL; CTRL-B to exit\r\n>`。
//! - After the banner, `\x04` performs a soft reset and the device prints
//!   `soft reboot\r\n` followed by the banner again.
//!   收到 banner 后，`\x04` 执行软复位，设备打印 `soft reboot\r\n` 并再次打印
//!   banner。
//! - Executing source means writing the source, then `\x04`, then reading
//!   `OK`, stdout up to `\x04` and stderr up to `\x04`.
//!   执行源码即写入源码与 `\x04`，随后读取 `OK`、以 `\x04` 结尾的标准输出，
//!   以及以 `\x04` 结尾的标准错误。
//! - `\r\x02` leaves raw REPL. / `\r\x02` 退出 raw REPL。

use std::collections::VecDeque;

/// Fully matched raw REPL banner, including the prompt
/// 完整匹配的 raw REPL banner（含提示符）
pub const BANNER: &[u8] = b"raw REPL; CTRL-B to exit\r\n>";

/// Banner line emitted after a soft reset (without the prompt)
/// 软复位后打印的 banner 行（不含提示符）
pub const BANNER_LINE: &[u8] = b"raw REPL; CTRL-B to exit\r\n";

/// Soft-reset acknowledgement / 软复位确认
pub const SOFT_REBOOT: &[u8] = b"soft reboot\r\n";

/// Ctrl-A: enter raw REPL / Ctrl-A：进入 raw REPL
pub const CTRL_A: u8 = 0x01;
/// Ctrl-B: leave raw REPL / Ctrl-B：退出 raw REPL
pub const CTRL_B: u8 = 0x02;
/// Ctrl-C: interrupt / Ctrl-C：中断
pub const CTRL_C: u8 = 0x03;
/// Ctrl-D: execute or soft reset / Ctrl-D：执行或软复位
pub const CTRL_D: u8 = 0x04;

/// Default per-operation timeout in simulated microseconds
/// 单次操作的默认超时（模拟微秒）
pub const DEFAULT_TIMEOUT_MICROS: u64 = 10_000_000;

/// Default simulation step between polls / 轮询之间的默认模拟步长
pub const DEFAULT_STEP_MICROS: u64 = 100;

/// Byte link between the raw REPL protocol and a device transport
/// raw REPL 协议与设备传输之间的字节链路
///
/// Implementations may wrap a [`crate::cdc::CdcLink`], a pty, a TCP socket or
/// a scripted test device. The protocol never blocks on real I/O itself; it
/// calls [`Self::advance`] when it needs the device to make progress.
/// 实现可以包装 [`crate::cdc::CdcLink`]、pty、TCP socket 或脚本化测试设备。
/// 协议本身不阻塞真实 I/O，需要设备推进时调用 [`Self::advance`]。
pub trait ByteLink {
    /// Queue host-to-device bytes / 将主机到设备的字节入队
    ///
    /// Returns the number of bytes accepted. A transport with real
    /// backpressure returns fewer bytes and the session retries the rest.
    /// 返回已接受的字节数；具有真实背压的传输返回较少字节，会话会重试剩余部分。
    fn send(&mut self, bytes: &[u8]) -> usize;

    /// Move available device-to-host bytes into `out` / 将可用的设备到主机字节移入 `out`
    ///
    /// Returns the number of bytes written into `out`.
    /// 返回写入 `out` 的字节数。
    fn receive(&mut self, out: &mut [u8]) -> usize;

    /// Let the device or simulation run for at most `micros` microseconds
    /// 让设备或模拟器最多推进 `micros` 微秒
    fn advance(&mut self, micros: u64);

    /// Current simulated time in microseconds / 当前模拟时间（微秒）
    fn now_micros(&self) -> u64;
}

/// Result of one raw REPL execution / 单次 raw REPL 执行的结果
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecOutput {
    /// Bytes written to stdout / 标准输出字节
    pub stdout: Vec<u8>,
    /// Bytes written to stderr / 标准错误字节
    pub stderr: Vec<u8>,
}

impl ExecOutput {
    /// Whether the execution produced no stderr output
    /// 执行是否没有产生标准错误
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.stderr.is_empty()
    }
}

/// Raw REPL failures / raw REPL 失败
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplError {
    /// A marker was not observed before the deadline / 在期限内未观察到标记
    Timeout {
        /// Marker the session was waiting for / 会话等待的标记
        marker: &'static str,
    },
    /// The protocol contract was violated / 违反协议约定
    Protocol {
        /// Description of the violation / 违规说明
        reason: &'static str,
    },
}

/// Blocking-style raw REPL session / 阻塞风格的 raw REPL 会话
///
/// "Blocking" means bounded waiting in simulated time: every loop advances the
/// link and checks a simulated deadline, so the session never sleeps on the
/// host and stays deterministic.
/// “阻塞”指在模拟时间上有界等待：每个循环推进链路并检查模拟期限，因此会话
/// 不会让主机真正睡眠，并保持确定性。
pub struct RawReplSession<L: ByteLink> {
    link: L,
    pending: VecDeque<u8>,
    timeout_micros: u64,
    step_micros: u64,
}

impl<L: ByteLink> RawReplSession<L> {
    /// Create a session with the default timeout / 使用默认超时创建会话
    #[must_use]
    pub fn new(link: L) -> Self {
        Self::with_timeout(link, DEFAULT_TIMEOUT_MICROS)
    }

    /// Create a session with an explicit per-operation timeout
    /// 使用显式的单次操作超时创建会话
    #[must_use]
    pub fn with_timeout(link: L, timeout_micros: u64) -> Self {
        Self {
            link,
            pending: VecDeque::new(),
            timeout_micros,
            step_micros: DEFAULT_STEP_MICROS,
        }
    }

    /// Underlying link / 底层链路
    #[must_use]
    pub fn link(&self) -> &L {
        &self.link
    }

    /// Underlying link, mutably / 可变访问底层链路
    pub fn link_mut(&mut self) -> &mut L {
        &mut self.link
    }

    /// Override the simulation step used between polls
    /// 覆盖轮询之间的模拟步长
    pub fn set_step_micros(&mut self, micros: u64) {
        self.step_micros = micros.max(1);
    }

    /// Send Ctrl-C to interrupt a running program
    /// 发送 Ctrl-C 中断正在运行的程序
    ///
    /// # Errors
    /// Returns [`ReplError::Timeout`] when the link cannot accept the bytes.
    /// 链路无法接受字节时返回 [`ReplError::Timeout`]。
    pub fn interrupt(&mut self) -> Result<(), ReplError> {
        self.send_all(b"\r\x03")
    }

    /// Enter the raw REPL, optionally performing a soft reset first
    /// 进入 raw REPL，可选先执行软复位
    ///
    /// A soft reset matches `pyboard.py`'s default `enter_raw_repl()` flow:
    /// banner, Ctrl-D, `soft reboot\r\n`, banner again.
    /// 软复位与 `pyboard.py` 默认的 `enter_raw_repl()` 流程一致：banner、
    /// Ctrl-D、`soft reboot\r\n`、再次 banner。
    ///
    /// # Errors
    /// Returns [`ReplError::Timeout`] when the banner or soft-reset marker is
    /// missing.
    /// banner 或软复位标记缺失时返回 [`ReplError::Timeout`]。
    pub fn enter_raw_repl(&mut self, soft_reset: bool) -> Result<(), ReplError> {
        self.interrupt()?;
        self.pending.clear();
        self.drain_input();
        self.send_all(b"\r\x01")?;
        self.wait_for(BANNER, "raw REPL banner")?;
        if soft_reset {
            self.send_all(&[CTRL_D])?;
            self.wait_for(SOFT_REBOOT, "soft reboot")?;
            self.wait_for(BANNER_LINE, "raw REPL banner after soft reset")?;
        }
        Ok(())
    }

    /// Execute `source` in raw REPL mode / 在 raw REPL 模式下执行 `source`
    ///
    /// # Errors
    /// Returns [`ReplError::Protocol`] when the source contains Ctrl-D, and
    /// [`ReplError::Timeout`] when the device does not answer.
    /// 源码包含 Ctrl-D 时返回 [`ReplError::Protocol`]，设备无应答时返回
    /// [`ReplError::Timeout`]。
    pub fn exec_raw(&mut self, source: &[u8]) -> Result<ExecOutput, ReplError> {
        if source.contains(&CTRL_D) {
            return Err(ReplError::Protocol {
                reason: "source contains Ctrl-D",
            });
        }
        self.send_all(source)?;
        self.send_all(&[CTRL_D])?;
        self.wait_for(b"OK", "execution acknowledgement")?;
        let stdout = self.take_until(CTRL_D, "stdout terminator")?;
        let stderr = self.take_until(CTRL_D, "stderr terminator")?;
        Ok(ExecOutput { stdout, stderr })
    }

    /// Leave the raw REPL and return to the friendly REPL
    /// 退出 raw REPL 并返回友好 REPL
    ///
    /// # Errors
    /// Returns [`ReplError::Timeout`] when the link cannot accept the bytes.
    /// 链路无法接受字节时返回 [`ReplError::Timeout`]。
    pub fn exit_raw_repl(&mut self) -> Result<(), ReplError> {
        self.send_all(b"\r\x02")
    }

    /// Perform a soft reset through the raw REPL / 通过 raw REPL 执行软复位
    ///
    /// # Errors
    /// Returns [`ReplError::Timeout`] when the banner or soft-reset marker is
    /// missing.
    /// banner 或软复位标记缺失时返回 [`ReplError::Timeout`]。
    pub fn soft_reset(&mut self) -> Result<(), ReplError> {
        self.send_all(&[CTRL_D])?;
        self.wait_for(SOFT_REBOOT, "soft reboot")?;
        self.wait_for(BANNER_LINE, "raw REPL banner after soft reset")
    }

    /// Write every byte, retrying while the link applies backpressure
    /// 写入每个字节，链路产生背压时重试
    fn send_all(&mut self, data: &[u8]) -> Result<(), ReplError> {
        let deadline = self.link.now_micros() + self.timeout_micros;
        let mut sent = 0;
        while sent < data.len() {
            let accepted = self.link.send(&data[sent..]);
            sent += accepted;
            if accepted == 0 {
                if self.link.now_micros() >= deadline {
                    return Err(ReplError::Timeout { marker: "send" });
                }
                self.link.advance(self.step_micros);
            }
        }
        Ok(())
    }

    /// Read available bytes into the pending buffer, returning the count
    /// 将可用字节读入待处理缓冲，返回字节数
    fn fill(&mut self) -> usize {
        let mut scratch = [0u8; 64];
        let read = self.link.receive(&mut scratch);
        self.pending.extend(scratch[..read].iter().copied());
        read
    }

    /// Discard all currently available device output / 丢弃当前可用的设备输出
    fn drain_input(&mut self) {
        while self.fill() > 0 {}
    }

    /// Wait until `marker` appears, discarding everything through it
    /// 等待 `marker` 出现，并丢弃直至其末尾的全部字节
    fn wait_for(&mut self, marker: &[u8], name: &'static str) -> Result<(), ReplError> {
        let deadline = self.link.now_micros() + self.timeout_micros;
        loop {
            if let Some(position) = find_subslice(&self.pending, marker) {
                self.pending.drain(..position + marker.len());
                return Ok(());
            }
            if self.link.now_micros() >= deadline {
                return Err(ReplError::Timeout { marker: name });
            }
            if self.fill() == 0 {
                self.link.advance(self.step_micros);
            }
        }
    }

    /// Collect bytes up to (excluding) `marker`, consuming the marker
    /// 收集直至 `marker`（不含）的字节，并消费该标记
    fn take_until(&mut self, marker: u8, name: &'static str) -> Result<Vec<u8>, ReplError> {
        let deadline = self.link.now_micros() + self.timeout_micros;
        loop {
            if let Some(position) = self.pending.iter().position(|byte| *byte == marker) {
                let collected = self.pending.drain(..position).collect();
                self.pending.pop_front();
                return Ok(collected);
            }
            if self.link.now_micros() >= deadline {
                return Err(ReplError::Timeout { marker: name });
            }
            if self.fill() == 0 {
                self.link.advance(self.step_micros);
            }
        }
    }
}

/// First index of `needle` inside `haystack` / `needle` 在 `haystack` 中的首个下标
fn find_subslice(haystack: &VecDeque<u8>, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|start| {
        haystack
            .iter()
            .skip(*start)
            .zip(needle.iter())
            .all(|(a, b)| a == b)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Mode {
        Friendly,
        Raw,
    }

    /// Device stub that answers the documented raw REPL byte sequences
    /// 按文档中的 raw REPL 字节序列应答的设备桩
    struct ScriptedLink {
        host: VecDeque<u8>,
        device: VecDeque<u8>,
        now: u64,
        mode: Mode,
        soft_reset_armed: bool,
    }

    impl ScriptedLink {
        fn new() -> Self {
            Self {
                host: VecDeque::new(),
                device: VecDeque::new(),
                now: 0,
                mode: Mode::Friendly,
                soft_reset_armed: false,
            }
        }

        fn emit(&mut self, bytes: &[u8]) {
            self.device.extend(bytes.iter().copied());
        }
    }

    impl ByteLink for ScriptedLink {
        fn send(&mut self, bytes: &[u8]) -> usize {
            self.host.extend(bytes.iter().copied());
            bytes.len()
        }

        fn receive(&mut self, out: &mut [u8]) -> usize {
            let mut count = 0;
            for slot in out.iter_mut() {
                let Some(byte) = self.device.pop_front() else {
                    break;
                };
                *slot = byte;
                count += 1;
            }
            count
        }

        fn advance(&mut self, micros: u64) {
            self.now += micros;
            while let Some(byte) = self.host.pop_front() {
                match byte {
                    CTRL_A => {
                        self.mode = Mode::Raw;
                        self.soft_reset_armed = true;
                        self.emit(BANNER);
                    }
                    CTRL_D if self.mode == Mode::Raw => {
                        if self.soft_reset_armed {
                            self.soft_reset_armed = false;
                            self.emit(SOFT_REBOOT);
                            self.emit(BANNER_LINE);
                        } else {
                            self.emit(b"OK");
                            self.emit(b"hello\r\n");
                            self.emit(&[CTRL_D]);
                            self.emit(&[CTRL_D]);
                        }
                    }
                    _ => self.soft_reset_armed = false,
                }
            }
        }

        fn now_micros(&self) -> u64 {
            self.now
        }
    }

    #[test]
    fn enter_exec_and_exit_round_trip() {
        let mut session = RawReplSession::new(ScriptedLink::new());
        session.enter_raw_repl(true).unwrap();
        let output = session.exec_raw(b"print('hi')").unwrap();
        assert_eq!(output.stdout, b"hello\r\n");
        assert_eq!(output.stderr, Vec::<u8>::new());
        assert!(output.is_success());
        session.exit_raw_repl().unwrap();
    }

    #[test]
    fn enter_without_soft_reset_stops_at_first_banner() {
        let mut session = RawReplSession::new(ScriptedLink::new());
        session.enter_raw_repl(false).unwrap();
        let output = session.exec_raw(b"1").unwrap();
        assert_eq!(output.stdout, b"hello\r\n");
    }

    struct Silent {
        now: u64,
    }

    impl Silent {
        fn new() -> Self {
            Self { now: 0 }
        }
    }

    impl ByteLink for Silent {
        fn send(&mut self, bytes: &[u8]) -> usize {
            bytes.len()
        }

        fn receive(&mut self, _out: &mut [u8]) -> usize {
            0
        }

        fn advance(&mut self, micros: u64) {
            self.now += micros;
        }

        fn now_micros(&self) -> u64 {
            self.now
        }
    }

    #[test]
    fn missing_banner_times_out() {
        let mut session = RawReplSession::with_timeout(Silent::new(), 1_000);
        let error = session.enter_raw_repl(false).unwrap_err();
        assert!(matches!(error, ReplError::Timeout { .. }));
    }
}
