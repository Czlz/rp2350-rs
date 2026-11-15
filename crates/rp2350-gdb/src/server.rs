//! GDB RSP server over TCP / 基于 TCP 的 GDB RSP 服务器
//!
//! [`GdbServer`] binds a listening socket, accepts clients sequentially, and
//! runs the byte-level RSP frame loop on top of the transport-independent
//! [`Session`] dispatcher from [`crate::rsp`]. Acknowledgements,
//! `QStartNoAckMode` negotiation, checksum rejection, and clean client
//! disconnects are handled here; command semantics stay in `rsp.rs`.
//! [`GdbServer`] 绑定监听 socket、顺序接受客户端，并在与传输无关的
//! [`Session`] 分发器之上运行字节级 RSP 帧循环。应答、`QStartNoAckMode`
//! 协商、校验和拒绝与客户端断开处理位于此处；命令语义仍由 `rsp.rs` 负责。

use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};

use rp2350_cpu::CpuState;

use crate::rsp::{ACK, NACK, Session, decode_packet, encode_packet};
use crate::target::{
    GdbError, GdbTarget, ResumeMode, StopReply, TargetStatus, ThreadAction, ThreadId,
};

/// Blocking GDB Remote Serial Protocol server / 阻塞式 GDB 远程串行协议服务器
pub struct GdbServer {
    /// Listen address / 监听地址
    addr: SocketAddr,
    /// Whether a serve call is currently active / 当前是否有 serve 调用在运行
    running: bool,
}

impl GdbServer {
    /// Create a new GDB server / 创建新的 GDB 服务器
    #[must_use]
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            running: false,
        }
    }

    /// Create with default address (0.0.0.0:3333) / 使用默认地址创建
    #[must_use]
    pub fn default_addr() -> Self {
        Self::new(SocketAddr::from(([0, 0, 0, 0], 3333)))
    }

    /// Bind the configured address and serve clients until the listener fails
    /// 绑定配置地址并提供服务，直到监听器出错
    ///
    /// Each accepted client runs on the shared target with a fresh transport
    /// session: the no-ack mode negotiated by a previous client is cleared
    /// before the next connection starts. The call blocks and keeps accepting
    /// after a client disconnects.
    /// 每个接受的客户端在共享目标上使用全新的传输会话：上一客户端协商的无应答
    /// 模式会在下一连接开始前清除。本调用阻塞，并在客户端断开后继续接受连接。
    ///
    /// # Errors
    /// Returns an I/O error if binding the address or accepting a client fails.
    /// 绑定地址或接受客户端失败时返回 I/O 错误。
    pub fn serve(&mut self, target: Box<dyn GdbTarget + Send>) -> io::Result<()> {
        let listener = TcpListener::bind(self.addr)?;
        self.addr = listener.local_addr()?;
        self.running = true;
        log::info!("GDB server listening on {}", self.addr);

        let mut session = Session::new(target);
        let result: io::Result<()> = loop {
            let (stream, peer) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) => break Err(error),
            };
            log::info!("GDB client connected from {peer}");
            match serve_connection(stream, &mut session) {
                Ok(()) => log::info!("GDB client from {peer} disconnected"),
                Err(error) if is_disconnect(&error) => {
                    log::info!("GDB client from {peer} disconnected: {error}");
                }
                Err(error) => log::warn!("GDB connection from {peer} failed: {error}"),
            }
        };
        self.running = false;
        result
    }

    /// Serve exactly one already-bound listener (test/integration entry point)
    /// 服务一个已绑定的监听器（测试/集成入口）
    ///
    /// The caller supplies the listener so tests can bind `127.0.0.1:0` and
    /// learn the assigned port. The call returns after the client disconnects;
    /// use [`Self::serve`] for the long-running accept loop.
    /// 监听器由调用方提供，测试可绑定 `127.0.0.1:0` 并获取分配端口。客户端断开
    /// 后本调用返回；长期接受循环请使用 [`Self::serve`]。
    ///
    /// # Errors
    /// Returns an I/O error if accepting or serving the connection fails.
    /// 接受连接或服务该连接失败时返回 I/O 错误。
    pub fn serve_with(
        &mut self,
        listener: &TcpListener,
        target: Box<dyn GdbTarget + Send>,
    ) -> io::Result<()> {
        self.addr = listener.local_addr()?;
        self.running = true;

        let mut session = Session::new(target);
        let result = match listener.accept() {
            Ok((stream, peer)) => {
                log::info!("GDB client connected from {peer}");
                serve_connection(stream, &mut session)
            }
            Err(error) => Err(error),
        };
        self.running = false;

        match result {
            Err(error) if is_disconnect(&error) => Ok(()),
            other => other,
        }
    }

    /// Stop the GDB server / 停止 GDB 服务器
    ///
    /// Only the running flag is cleared; a blocking `serve` call observes it
    /// when it returns. / 仅清除运行标志；阻塞中的 `serve` 在返回时观察该状态。
    pub fn stop(&mut self) {
        self.running = false;
    }

    /// Check if the server is running / 检查服务器是否在运行
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Get the listen address / 获取监听地址
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

// The TCP server owns its target behind a trait object; forwarding keeps the
// generic `Session` usable without changing its API.
// TCP 服务器以 trait object 持有目标；转发实现让泛型 `Session` 无需修改 API
// 即可使用。
impl GdbTarget for Box<dyn GdbTarget + Send> {
    fn thread_count(&self) -> usize {
        (**self).thread_count()
    }

    fn thread_id(&self, index: usize) -> Option<ThreadId> {
        (**self).thread_id(index)
    }

    fn thread_name(&self, thread: ThreadId) -> String {
        (**self).thread_name(thread)
    }

    fn thread_state(&self, thread: ThreadId) -> Option<CpuState> {
        (**self).thread_state(thread)
    }

    fn read_thread_register(&mut self, thread: ThreadId, register: u32) -> Result<u32, GdbError> {
        (**self).read_thread_register(thread, register)
    }

    fn write_thread_register(
        &mut self,
        thread: ThreadId,
        register: u32,
        value: u32,
    ) -> Result<(), GdbError> {
        (**self).write_thread_register(thread, register, value)
    }

    fn continue_thread(&mut self, thread: ThreadId) -> Result<StopReply, GdbError> {
        (**self).continue_thread(thread)
    }

    fn step_thread(&mut self, thread: ThreadId) -> Result<StopReply, GdbError> {
        (**self).step_thread(thread)
    }

    fn cont(&mut self, actions: &[ThreadAction]) -> Result<StopReply, GdbError> {
        (**self).cont(actions)
    }

    fn stopped_thread(&self) -> ThreadId {
        (**self).stopped_thread()
    }

    fn register_count(&self) -> u32 {
        (**self).register_count()
    }

    fn target_description(&self) -> Option<&'static str> {
        (**self).target_description()
    }

    fn read_register(&mut self, register: u32) -> Result<u32, GdbError> {
        (**self).read_register(register)
    }

    fn write_register(&mut self, register: u32, value: u32) -> Result<(), GdbError> {
        (**self).write_register(register, value)
    }

    fn read_memory(&mut self, address: u32, length: usize) -> Result<Vec<u8>, GdbError> {
        (**self).read_memory(address, length)
    }

    fn write_memory(&mut self, address: u32, bytes: &[u8]) -> Result<(), GdbError> {
        (**self).write_memory(address, bytes)
    }

    fn set_breakpoint(&mut self, address: u32) -> Result<(), GdbError> {
        (**self).set_breakpoint(address)
    }

    fn clear_breakpoint(&mut self, address: u32) -> Result<(), GdbError> {
        (**self).clear_breakpoint(address)
    }

    fn has_breakpoint(&self, address: u32) -> bool {
        (**self).has_breakpoint(address)
    }

    fn supports_watchpoints(&self) -> bool {
        (**self).supports_watchpoints()
    }

    fn set_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        (**self).set_watchpoint(address, length)
    }

    fn clear_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        (**self).clear_watchpoint(address, length)
    }

    fn set_read_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        (**self).set_read_watchpoint(address, length)
    }

    fn clear_read_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        (**self).clear_read_watchpoint(address, length)
    }

    fn set_access_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        (**self).set_access_watchpoint(address, length)
    }

    fn clear_access_watchpoint(&mut self, address: u32, length: usize) -> Result<(), GdbError> {
        (**self).clear_access_watchpoint(address, length)
    }

    fn monitor_command(&mut self, command: &str) -> String {
        (**self).monitor_command(command)
    }

    fn watchpoint_hit(&self) -> Option<u32> {
        (**self).watchpoint_hit()
    }

    fn resume(&mut self, mode: ResumeMode) -> Result<StopReply, GdbError> {
        (**self).resume(mode)
    }

    fn status(&self) -> TargetStatus {
        (**self).status()
    }
}

/// Whether an I/O error only means the client went away
/// 判断 I/O 错误是否仅表示客户端已离开
fn is_disconnect(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::BrokenPipe
    )
}

/// Run the RSP frame loop for one connected client
/// 为一个已连接客户端运行 RSP 帧循环
///
/// A valid frame is acknowledged with `+`, decoded, dispatched, and answered
/// with `$reply#checksum`. A bad checksum is answered with `-` and the
/// connection stays open. `k` acknowledges, suppresses the reply, and closes
/// the connection by returning.
/// 合法帧以 `+` 应答，随后解码、分发并回复 `$reply#校验和`。校验和错误以 `-`
/// 应答且连接保持打开。`k` 会先应答，抑制回复并通过返回来关闭连接。
fn serve_connection(
    stream: TcpStream,
    session: &mut Session<Box<dyn GdbTarget + Send>>,
) -> io::Result<()> {
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    // Every TCP connection is a fresh RSP session; forget no-ack negotiation.
    // 每个 TCP 连接都是新的 RSP 会话；清除无应答模式协商结果。
    session.reset_transport();

    loop {
        let Some(frame) = read_frame(&mut reader)? else {
            return Ok(());
        };
        match decode_packet(&frame) {
            Ok(payload) => {
                if !session.no_ack_mode() {
                    writer.write_all(&[ACK])?;
                }
                let Some(reply) = session.handle_packet(&payload) else {
                    // `k` has no reply; close the connection as RSP requires.
                    // `k` 无回复；按 RSP 约定关闭此连接。
                    return Ok(());
                };
                writer.write_all(encode_packet(&reply).as_bytes())?;
            }
            Err(error) => {
                log::debug!("rejecting malformed RSP frame: {error}");
                if !session.no_ack_mode() {
                    writer.write_all(&[NACK])?;
                }
            }
        }
    }
}

/// Read one complete `$<payload>#<checksum>` frame from the client
/// 从客户端读取一个完整的 `$<载荷>#<校验和>` 帧
///
/// Bytes outside a frame are ignored, including `+`/`-` acknowledgements sent
/// by GDB for earlier replies. `}`-escaped payload bytes cannot terminate a
/// frame. `Ok(None)` means the client closed the connection.
/// 帧外字节会被忽略，包括 GDB 针对先前回复发送的 `+`/`-` 应答。`}` 转义的
/// 载荷字节不会终止帧。`Ok(None)` 表示客户端已关闭连接。
fn read_frame(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    loop {
        let Some(byte) = read_byte(reader)? else {
            return Ok(None);
        };
        if byte != b'$' {
            if matches!(byte, ACK | NACK) {
                log::debug!("ignoring client acknowledgement byte 0x{byte:02x}");
            }
            continue;
        }

        let mut frame = vec![b'$'];
        let mut escaped = false;
        loop {
            let Some(byte) = read_byte(reader)? else {
                return Ok(None);
            };
            frame.push(byte);
            if escaped {
                escaped = false;
                continue;
            }
            match byte {
                b'}' => escaped = true,
                b'#' => {
                    for _ in 0..2 {
                        let Some(digit) = read_byte(reader)? else {
                            return Ok(None);
                        };
                        frame.push(digit);
                    }
                    return Ok(Some(frame));
                }
                _ => {}
            }
        }
    }
}

/// Read a single byte, mapping end-of-stream to `None`
/// 读取单个字节，将流结束映射为 `None`
fn read_byte(reader: &mut impl BufRead) -> io::Result<Option<u8>> {
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => return Ok(None),
            Ok(_) => return Ok(Some(byte[0])),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::thread;
    use std::time::Duration;

    use rp2350_core::error::{BusErrorKind, CoreError};
    use rp2350_cpu::CpuState;

    use super::{GdbServer, TcpListener};
    use crate::rsp::{decode_packet, encode_packet};
    use crate::target::{GdbError, GdbTarget, ResumeMode, StopReason, StopReply, TargetStatus};

    const MEMORY_BASE: u32 = 0x2000_0000;
    const REGISTERS: [u32; 2] = [0x1111_1111, 0x2222_2222];

    /// Two-register, eight-byte mock target used by the TCP tests
    /// TCP 测试使用的双寄存器、八字节 mock 目标
    struct MockTarget {
        registers: [u32; 2],
        memory: Vec<u8>,
        breakpoints: BTreeSet<u32>,
        last_stop: Option<StopReason>,
    }

    impl MockTarget {
        fn new() -> Self {
            Self {
                registers: REGISTERS,
                memory: vec![0; 8],
                breakpoints: BTreeSet::new(),
                last_stop: None,
            }
        }
    }

    fn bus_error(address: u32) -> GdbError {
        GdbError::Bus(CoreError::BusError {
            addr: address,
            kind: BusErrorKind::NoDevice,
        })
    }

    impl GdbTarget for MockTarget {
        fn register_count(&self) -> u32 {
            2
        }

        fn read_register(&mut self, register: u32) -> Result<u32, GdbError> {
            self.registers
                .get(usize::try_from(register).unwrap_or(usize::MAX))
                .copied()
                .ok_or(GdbError::InvalidRegister(register))
        }

        fn write_register(&mut self, register: u32, value: u32) -> Result<(), GdbError> {
            let slot = self
                .registers
                .get_mut(usize::try_from(register).unwrap_or(usize::MAX))
                .ok_or(GdbError::InvalidRegister(register))?;
            *slot = value;
            Ok(())
        }

        fn read_memory(&mut self, address: u32, length: usize) -> Result<Vec<u8>, GdbError> {
            let offset = usize::try_from(address.wrapping_sub(MEMORY_BASE)).unwrap_or(usize::MAX);
            let end = offset
                .checked_add(length)
                .ok_or(GdbError::AddressOutOfRange(address))?;
            self.memory
                .get(offset..end)
                .map(<[u8]>::to_vec)
                .ok_or_else(|| bus_error(address))
        }

        fn write_memory(&mut self, address: u32, bytes: &[u8]) -> Result<(), GdbError> {
            let offset = usize::try_from(address.wrapping_sub(MEMORY_BASE)).unwrap_or(usize::MAX);
            let end = offset
                .checked_add(bytes.len())
                .ok_or(GdbError::AddressOutOfRange(address))?;
            let slot = self
                .memory
                .get_mut(offset..end)
                .ok_or_else(|| bus_error(address))?;
            slot.copy_from_slice(bytes);
            Ok(())
        }

        fn set_breakpoint(&mut self, address: u32) -> Result<(), GdbError> {
            self.breakpoints.insert(address);
            Ok(())
        }

        fn clear_breakpoint(&mut self, address: u32) -> Result<(), GdbError> {
            self.breakpoints.remove(&address);
            Ok(())
        }

        fn has_breakpoint(&self, address: u32) -> bool {
            self.breakpoints.contains(&address)
        }

        fn resume(&mut self, mode: ResumeMode) -> Result<StopReply, GdbError> {
            let reason = match mode {
                ResumeMode::Continue => StopReason::Breakpoint,
                ResumeMode::Step => StopReason::SingleStep,
            };
            self.last_stop = Some(reason);
            Ok(StopReply {
                reason,
                pc: MEMORY_BASE + 4,
            })
        }

        fn status(&self) -> TargetStatus {
            TargetStatus {
                state: CpuState::Halted,
                stop_reason: self.last_stop,
                pc: MEMORY_BASE,
                total_cycles: 0,
            }
        }
    }

    /// Bind an ephemeral port and run one server session on a background thread
    /// 绑定临时端口并在后台线程运行一个服务器会话
    fn spawn_server() -> (
        std::net::SocketAddr,
        thread::JoinHandle<std::io::Result<()>>,
    ) {
        let listener =
            TcpListener::bind(std::net::SocketAddr::from(([127, 0, 0, 1], 0))).expect("bind");
        let addr = listener.local_addr().expect("listener address");
        let handle = thread::spawn(move || {
            let mut server = GdbServer::new(addr);
            server.serve_with(&listener, Box::new(MockTarget::new()))
        });
        (addr, handle)
    }

    /// Run one client against a fresh server and join the server afterwards
    /// 针对全新服务器运行一个客户端并在结束后汇合服务器线程
    fn run_session(check: impl FnOnce(&mut TestClient)) {
        let (addr, handle) = spawn_server();
        let mut client = TestClient::connect(addr);
        check(&mut client);
        drop(client);
        handle
            .join()
            .expect("server thread panicked")
            .expect("server failed");
    }

    /// Minimal RSP client used by the TCP session tests / TCP 会话测试用最小 RSP 客户端
    struct TestClient {
        stream: TcpStream,
    }

    impl TestClient {
        fn connect(addr: std::net::SocketAddr) -> Self {
            let stream = TcpStream::connect(addr).expect("connect");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("read timeout");
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .expect("write timeout");
            Self { stream }
        }

        fn send_payload(&mut self, payload: &str) {
            self.send_frame(&encode_packet(payload));
        }

        fn send_frame(&mut self, frame: &str) {
            self.stream
                .write_all(frame.as_bytes())
                .expect("write frame");
        }

        fn read_byte(&mut self) -> u8 {
            let mut byte = [0u8; 1];
            self.stream.read_exact(&mut byte).expect("read byte");
            byte[0]
        }

        fn read_packet(&mut self) -> String {
            while self.read_byte() != b'$' {}
            self.read_packet_body()
        }

        fn read_packet_body(&mut self) -> String {
            let mut frame = vec![b'$'];
            loop {
                let byte = self.read_byte();
                frame.push(byte);
                if byte == b'#' {
                    frame.push(self.read_byte());
                    frame.push(self.read_byte());
                    break;
                }
            }
            decode_packet(&frame).expect("valid reply packet")
        }

        fn transact(&mut self, payload: &str) -> String {
            self.send_payload(payload);
            assert_eq!(self.read_byte(), b'+', "expected positive acknowledgement");
            self.read_packet()
        }
    }

    #[test]
    fn server_exposes_address_and_running_state() {
        let mut server = GdbServer::default_addr();
        assert_eq!(
            server.addr(),
            std::net::SocketAddr::from(([0, 0, 0, 0], 3333))
        );
        assert!(!server.is_running());
        server.stop();
        assert!(!server.is_running());
        assert_eq!(GdbServer::new(server.addr()).addr(), server.addr());
    }

    #[test]
    fn tcp_session_answers_queries_and_unknown_packets() {
        run_session(|client| {
            let supported = client.transact("qSupported:multiprocess+");
            assert!(supported.contains("PacketSize="));
            assert!(supported.contains("QStartNoAckMode+"));
            assert_eq!(client.transact("?"), "T05thread:1;");
            assert_eq!(client.transact("vMustReplyEmpty"), "");
        });
    }

    #[test]
    fn tcp_registers_memory_and_continue_round_trip() {
        run_session(|client| {
            assert_eq!(client.transact("g"), "1111111122222222");
            assert_eq!(client.transact("M20000000,4:a1b2c3d4"), "OK");
            assert_eq!(client.transact("m20000000,4"), "a1b2c3d4");
            assert_eq!(client.transact("c"), "T05thread:1;");
        });
    }

    #[test]
    fn tcp_bad_checksum_is_nacked_and_connection_survives() {
        run_session(|client| {
            // `?` really has checksum 0x3f; 0x00 forces a mismatch.
            // `?` 的真实校验和为 0x3f；0x00 强制不匹配。
            client.send_frame("$?#00");
            assert_eq!(client.read_byte(), b'-', "bad checksum must be nacked");
            assert_eq!(client.transact("?"), "T05thread:1;");
        });
    }

    #[test]
    fn tcp_no_ack_mode_stops_acknowledgements() {
        run_session(|client| {
            assert_eq!(client.transact("QStartNoAckMode"), "OK");

            client.send_payload("qSupported");
            assert_eq!(client.read_byte(), b'$', "no '+' in no-ack mode");
            assert!(client.read_packet_body().contains("PacketSize="));

            client.send_payload("?");
            assert_eq!(client.read_byte(), b'$', "no '+' in no-ack mode");
            assert_eq!(client.read_packet_body(), "T05thread:1;");
        });
    }

    #[test]
    fn tcp_kill_acknowledges_then_disconnects() {
        run_session(|client| {
            client.send_payload("k");
            assert_eq!(client.read_byte(), b'+', "expected acknowledgement for k");
            let mut byte = [0u8; 1];
            match client.stream.read(&mut byte) {
                Ok(0) => {}
                Ok(count) => panic!("expected EOF after k, read {count} extra bytes"),
                Err(error) => panic!("expected EOF after k, got {error}"),
            }
        });
    }
}
