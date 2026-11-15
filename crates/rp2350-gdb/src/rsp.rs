//! GDB Remote Serial Protocol codec and command dispatcher
//! GDB 远程串行协议编解码与命令分发
//!
//! The transport-independent half of the GDB server lives here: packet framing
//! (`$<payload>#<checksum>`), binary escaping, hexadecimal helpers, and a
//! [`Session`] that maps RSP commands onto a [`GdbTarget`]. The session never
//! touches a socket, so every packet path is unit-testable.
//! GDB 服务器与传输无关的一半逻辑位于此处：包封装（`$<载荷>#<校验和>`）、
//! 二进制转义、十六进制工具，以及把 RSP 命令映射到 [`GdbTarget`] 的
//! [`Session`]。会话不接触 socket，因此所有包处理路径均可单元测试。

use std::collections::BTreeSet;

use crate::target::{GdbError, GdbTarget, ResumeMode, StopReason, WatchKind};
use crate::target_xml::{ANNEX, ANNEX_ERROR};

/// Positive acknowledgement byte sent by GDB / GDB 发送的肯定应答字节
pub const ACK: u8 = b'+';
/// Negative acknowledgement byte sent by GDB / GDB 发送的否定应答字节
pub const NACK: u8 = b'-';
/// Reply payload for malformed packets and target errors
/// 非法包与目标错误的回复载荷
pub const ERROR_REPLY: &str = "E01";
/// Maximum payload size advertised through `qSupported`
/// 通过 `qSupported` 声明的最大载荷尺寸
pub const MAX_PACKET_SIZE: usize = 0x0fff;

/// Default stop reply (`S05`, SIGTRAP) / 默认停止回复（`S05`，SIGTRAP）
const DEFAULT_STOP: &str = "S05";

/// Acknowledgement parsed from the transport byte stream
/// 从传输字节流解析出的应答
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ack {
    /// `+`: the previous packet arrived intact / 上一包完整到达
    Positive,
    /// `-`: the previous packet was corrupted, resend it
    /// 上一包已损坏，需要重发
    Negative,
}

impl Ack {
    /// Wire byte for this acknowledgement / 此应答对应的线上字节
    #[must_use]
    pub const fn as_byte(self) -> u8 {
        match self {
            Self::Positive => ACK,
            Self::Negative => NACK,
        }
    }
}

/// Parse an acknowledgement byte / 解析应答字节
///
/// Returns `None` for any byte other than `+` or `-`.
/// 对 `+`、`-` 之外的字节返回 `None`。
#[must_use]
pub fn parse_ack(byte: u8) -> Option<Ack> {
    match byte {
        ACK => Some(Ack::Positive),
        NACK => Some(Ack::Negative),
        _ => None,
    }
}

/// Packet or command error / 包或命令错误
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Frame does not start with `$` / 帧未以 `$` 开头
    #[error("packet does not start with '$' / 包未以 '$' 开头")]
    MissingStart,
    /// Frame has no `#<checksum>` terminator / 帧缺少 `#<校验和>` 终止符
    #[error("packet has no '#<checksum>' terminator / 包缺少 '#<校验和>' 终止符")]
    MissingChecksum,
    /// Checksum text is not exactly two digits / 校验和文本不是恰好两位
    #[error("checksum must be exactly two digits / 校验和必须恰好是两位")]
    InvalidChecksumLength,
    /// Checksum digit is not hexadecimal / 校验和数字不是十六进制
    #[error("invalid checksum digit / 无效的校验和数字")]
    InvalidChecksumDigit,
    /// Computed checksum differs from the transmitted checksum
    /// 计算出的校验和与传输的校验和不一致
    #[error(
        "checksum mismatch: computed {computed:#04x}, received {received:#04x} / 校验和不匹配：计算 {computed:#04x}，收到 {received:#04x}"
    )]
    ChecksumMismatch {
        /// Checksum computed over the payload / 对载荷计算出的校验和
        computed: u8,
        /// Checksum read from the frame / 从帧中读取的校验和
        received: u8,
    },
    /// Packet payload is not valid UTF-8 / 包载荷不是有效 UTF-8
    #[error("packet payload is not valid UTF-8 / 包载荷不是有效 UTF-8")]
    InvalidUtf8,
    /// Hexadecimal text is malformed / 十六进制文本格式错误
    #[error("invalid hexadecimal text / 无效的十六进制文本")]
    InvalidHex,
    /// A binary escape marker has no following byte / 二进制转义标记后没有字节
    #[error("binary escape has no payload byte / 二进制转义缺少载荷字节")]
    TrailingEscape,
    /// Command arguments are missing or malformed / 命令参数缺失或格式错误
    #[error("invalid command arguments / 无效的命令参数")]
    InvalidArgument,
    /// Packet is larger than the advertised payload limit / 包超过声明的载荷上限
    #[error("packet exceeds the maximum payload size / 包超过最大载荷尺寸")]
    PacketTooLarge,
    /// The debug target rejected the operation / 调试目标拒绝了该操作
    #[error(transparent)]
    Target(#[from] GdbError),
}

/// ASCII digits used by hexadecimal output / 十六进制输出使用的 ASCII 数字
const HEX_DIGITS: [u8; 16] = *b"0123456789abcdef";

/// Compute the RSP checksum of a payload / 计算载荷的 RSP 校验和
///
/// The checksum is the wrapping 8-bit sum of every payload byte.
/// 校验和是载荷所有字节的 8 位回绕和。
#[must_use]
pub fn checksum(payload: &[u8]) -> u8 {
    payload
        .iter()
        .fold(0u8, |sum, byte| sum.wrapping_add(*byte))
}

/// Encode bytes as lowercase hexadecimal / 将字节编码为小写十六进制
#[must_use]
pub fn encode_hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(HEX_DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

/// Decode hexadecimal text into bytes / 将十六进制文本解码为字节
///
/// Uppercase digits are accepted on input; output is always lowercase.
/// 输入接受大写数字；输出始终为小写。
///
/// # Errors
/// Returns [`Error::InvalidHex`] for odd-length text or non-hexadecimal bytes.
/// 文本长度为奇数或包含非十六进制字节时返回 [`Error::InvalidHex`]。
pub fn decode_hex(text: &str) -> Result<Vec<u8>, Error> {
    let bytes = text.as_bytes();
    if bytes.len() % 2 != 0 {
        return Err(Error::InvalidHex);
    }
    let mut decoded = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let high = hex_value(pair[0]).ok_or(Error::InvalidHex)?;
        let low = hex_value(pair[1]).ok_or(Error::InvalidHex)?;
        decoded.push((high << 4) | low);
    }
    Ok(decoded)
}

/// Encode a complete RSP packet / 编码完整的 RSP 包
#[must_use]
pub fn encode_packet(payload: &str) -> String {
    format!("${payload}#{:02x}", checksum(payload.as_bytes()))
}

/// Decode a complete RSP packet frame / 解码完整的 RSP 包帧
///
/// The frame must have the exact shape `$<payload>#<2 hex checksum>`.
/// 帧必须严格符合 `$<载荷>#<2 位十六进制校验和>` 形式。
///
/// # Errors
/// Returns [`Error`] when markers are missing, the checksum is malformed or
/// mismatched, or the payload is not UTF-8.
/// 缺少标记、校验和格式错误或不匹配、载荷非 UTF-8 时返回 [`Error`]。
pub fn decode_packet(frame: &[u8]) -> Result<String, Error> {
    let body = frame.strip_prefix(b"$").ok_or(Error::MissingStart)?;
    let hash = body
        .iter()
        .rposition(|byte| *byte == b'#')
        .ok_or(Error::MissingChecksum)?;
    let payload = &body[..hash];
    let checksum_text = &body[hash + 1..];
    if checksum_text.len() != 2 {
        return Err(Error::InvalidChecksumLength);
    }
    let computed = checksum(payload);
    let received = (hex_value(checksum_text[0]).ok_or(Error::InvalidChecksumDigit)? << 4)
        | hex_value(checksum_text[1]).ok_or(Error::InvalidChecksumDigit)?;
    if computed != received {
        return Err(Error::ChecksumMismatch { computed, received });
    }
    let payload = std::str::from_utf8(payload).map_err(|_| Error::InvalidUtf8)?;
    Ok(payload.to_string())
}

/// Escape binary payload bytes for RSP framing / 为 RSP 帧转义二进制载荷
///
/// `$`, `#`, `}` and `*` are encoded as `}` followed by the byte XOR `0x20`.
/// `$`、`#`、`}`、`*` 编码为 `}` 后接原字节与 `0x20` 异或的结果。
#[must_use]
pub fn escape_binary(payload: &[u8]) -> Vec<u8> {
    let mut escaped = Vec::with_capacity(payload.len() + 8);
    for byte in payload {
        if matches!(*byte, b'$' | b'#' | b'}' | b'*') {
            escaped.push(b'}');
            escaped.push(*byte ^ 0x20);
        } else {
            escaped.push(*byte);
        }
    }
    escaped
}

/// Decode RSP binary escaping / 解码 RSP 二进制转义
///
/// # Errors
/// Returns [`Error::TrailingEscape`] when a `}` has no following byte.
/// `}` 后没有字节时返回 [`Error::TrailingEscape`]。
pub fn unescape_binary(payload: &[u8]) -> Result<Vec<u8>, Error> {
    let mut decoded = Vec::with_capacity(payload.len());
    let mut bytes = payload.iter().copied();
    while let Some(byte) = bytes.next() {
        if byte == b'}' {
            let escaped = bytes.next().ok_or(Error::TrailingEscape)?;
            decoded.push(escaped ^ 0x20);
        } else {
            decoded.push(byte);
        }
    }
    Ok(decoded)
}

/// Stateful RSP command dispatcher / 有状态的 RSP 命令分发器
///
/// The session owns the debug target plus transport flags negotiated by
/// `QStartNoAckMode`, but performs no I/O. `handle_frame` is the codec entry
/// point; `handle_packet` is the payload-only entry point used by tests and
/// future transports.
/// 会话持有调试目标以及由 `QStartNoAckMode` 协商的传输标志，但不执行 I/O。
/// `handle_frame` 是编解码入口；`handle_packet` 是仅处理载荷的入口，供测试
/// 与未来的传输层使用。
pub struct Session<T: GdbTarget> {
    /// Debug target driven by RSP commands / RSP 命令驱动的调试目标
    target: T,
    /// Whether GDB requested no-acknowledgement mode / GDB 是否请求了无应答模式
    no_ack_mode: bool,
    /// Watchpoints requested by GDB, keyed by `(address, length, kind)`
    /// GDB 请求的监视点，以 `(地址, 长度, 类型)` 为键
    watchpoints: BTreeSet<(u32, usize, WatchKind)>,
}

impl<T: GdbTarget> Session<T> {
    /// Wrap a debug target in a new dispatcher / 用调试目标创建分发器
    #[must_use]
    pub fn new(target: T) -> Self {
        Self {
            target,
            no_ack_mode: false,
            watchpoints: BTreeSet::new(),
        }
    }

    /// Whether `QStartNoAckMode` has been negotiated
    /// 是否已协商 `QStartNoAckMode`
    #[must_use]
    pub fn no_ack_mode(&self) -> bool {
        self.no_ack_mode
    }

    /// Read-only access to the wrapped target / 只读访问被包装的目标
    #[must_use]
    pub fn target(&self) -> &T {
        &self.target
    }

    /// Reset per-connection transport state / 重置每连接传输状态
    ///
    /// Clears the `QStartNoAckMode` negotiation so a new client starts in
    /// acknowledged mode. The debug target itself is not touched: memory,
    /// registers, breakpoints, and watchpoints survive a reconnect.
    /// 清除 `QStartNoAckMode` 协商结果，使新客户端从应答模式开始。调试目标本身
    /// 不受影响：内存、寄存器、断点与监视点在重连后保持。
    pub fn reset_transport(&mut self) {
        self.no_ack_mode = false;
    }

    /// Decode and dispatch a complete frame / 解码并分发完整帧
    ///
    /// Malformed frames (including checksum errors) are answered with
    /// [`ERROR_REPLY`] instead of closing the connection. Returns `None` only
    /// for `k`, which has no reply.
    /// 非法帧（包括校验和错误）以 [`ERROR_REPLY`] 应答而不是关闭连接。仅
    /// `k` 命令返回 `None`，因为它不需要回复。
    #[must_use]
    pub fn handle_frame(&mut self, frame: &[u8]) -> Option<String> {
        match decode_packet(frame) {
            Ok(payload) => self.handle_packet(&payload),
            Err(error) => {
                log::debug!("malformed RSP frame: {error}");
                Some(ERROR_REPLY.to_string())
            }
        }
    }

    /// Dispatch a packet payload and return its reply payload
    /// 分发包载荷并返回回复载荷
    ///
    /// `None` means the packet must not be answered (`k`); unknown or
    /// unsupported commands return an empty reply payload. Target failures are
    /// mapped to [`ERROR_REPLY`] and leave the session usable.
    /// `None` 表示不回复（`k`）；未知或不支持的命令返回空回复载荷。目标失败
    /// 映射为 [`ERROR_REPLY`]，会话保持可用。
    #[must_use]
    pub fn handle_packet(&mut self, payload: &str) -> Option<String> {
        if payload == "k" {
            return None;
        }
        Some(match self.dispatch(payload) {
            Ok(reply) => reply,
            Err(error) => {
                log::debug!("RSP command failed: {error}");
                ERROR_REPLY.to_string()
            }
        })
    }

    /// Route a payload to its command handler / 将载荷路由到对应命令处理器
    fn dispatch(&mut self, payload: &str) -> Result<String, Error> {
        let Some(first) = payload.as_bytes().first().copied() else {
            return Ok(String::new());
        };
        match first {
            b'?' => Ok(DEFAULT_STOP.to_string()),
            b'g' => self.read_all_registers(),
            b'G' => self.write_all_registers(&payload[1..]),
            b'p' => self.read_register(&payload[1..]),
            b'P' => self.write_register(&payload[1..]),
            b'm' => self.read_memory(&payload[1..]),
            b'M' => self.write_memory(&payload[1..]),
            b'c' | b'C' => self.resume(ResumeMode::Continue),
            b's' | b'S' => self.resume(ResumeMode::Step),
            b'Z' => self.change_breakpoint(&payload[1..], true),
            b'z' => self.change_breakpoint(&payload[1..], false),
            b'D' => Ok("OK".to_string()),
            b'H' => Ok(match payload.as_bytes().get(1) {
                Some(b'g' | b'c') => "OK".to_string(),
                _ => String::new(),
            }),
            b'Q' => Ok(self.handle_extended(payload)),
            b'v' => self.handle_v(payload),
            b'q' => self.handle_query(payload),
            _ => Ok(String::new()),
        }
    }

    /// Route `q...` packets, including `qRcmd` monitor commands
    /// 路由 `q...` 包，包括 `qRcmd` monitor 命令
    fn handle_query(&mut self, payload: &str) -> Result<String, Error> {
        // `qRcmd,<hex>` carries a monitor command: the payload is the hex
        // encoding of a UTF-8 string and the reply is the hex encoding of the
        // text to print. An empty reply payload is not allowed here, so a
        // command without output becomes `OK`.
        // `qRcmd,<hex>` 承载 monitor 命令：载荷是 UTF-8 字符串的十六进制编码，
        // 回复是要打印文本的十六进制编码。此处不允许空回复载荷，因此无输出的命令
        // 回复 `OK`。
        if let Some(args) = payload.strip_prefix("qRcmd,") {
            return self.monitor(args);
        }
        Ok(self.query_reply(payload))
    }

    /// Decode and run one `qRcmd` monitor command / 解码并执行一条 `qRcmd` monitor 命令
    fn monitor(&mut self, args: &str) -> Result<String, Error> {
        let bytes = decode_hex(args)?;
        let command = String::from_utf8(bytes).map_err(|_| Error::InvalidUtf8)?;
        let output = self.target.monitor_command(command.trim());
        if output.is_empty() {
            return Ok("OK".to_string());
        }
        Ok(encode_hex(output.as_bytes()))
    }

    /// Read every register as little-endian 32-bit hex (`g`)
    /// 以 32 位小端十六进制读取所有寄存器（`g`）
    fn read_all_registers(&mut self) -> Result<String, Error> {
        let count = self.target.register_count();
        let mut reply = String::new();
        for register in 0..count {
            let value = self.target.read_register(register)?;
            reply.push_str(&encode_u32_le(value));
        }
        Ok(reply)
    }

    /// Write a register block (`G`) / 写入寄存器块（`G`）
    fn write_all_registers(&mut self, data: &str) -> Result<String, Error> {
        let bytes = decode_hex(data)?;
        if bytes.len() % 4 != 0 {
            return Err(Error::InvalidArgument);
        }
        let count = self.target.register_count();
        for (index, chunk) in bytes.chunks_exact(4).enumerate() {
            let register = u32::try_from(index).map_err(|_| Error::InvalidArgument)?;
            if register >= count {
                return Err(Error::InvalidArgument);
            }
            let value = u32::from_le_bytes(chunk.try_into().map_err(|_| Error::InvalidArgument)?);
            self.target.write_register(register, value)?;
        }
        Ok("OK".to_string())
    }

    /// Read one register (`p N`) / 读取单个寄存器（`p N`）
    fn read_register(&mut self, args: &str) -> Result<String, Error> {
        let register = parse_u32_hex(args)?;
        let value = self.target.read_register(register)?;
        Ok(encode_u32_le(value))
    }

    /// Write one register (`P N=value`) / 写入单个寄存器（`P N=值`）
    fn write_register(&mut self, args: &str) -> Result<String, Error> {
        let (register, value) = args.split_once('=').ok_or(Error::InvalidArgument)?;
        let register = parse_u32_hex(register)?;
        let value = decode_u32_le(value)?;
        self.target.write_register(register, value)?;
        Ok("OK".to_string())
    }

    /// Read memory (`m address,length`) / 读取内存（`m 地址,长度`）
    fn read_memory(&mut self, args: &str) -> Result<String, Error> {
        let (address, length) = args.split_once(',').ok_or(Error::InvalidArgument)?;
        let address = parse_u32_hex(address)?;
        let length = parse_usize_hex(length)?;
        if length > MAX_PACKET_SIZE {
            return Err(Error::PacketTooLarge);
        }
        let bytes = self.target.read_memory(address, length)?;
        Ok(encode_hex(&bytes))
    }

    /// Write memory (`M address,length:data`) / 写入内存（`M 地址,长度:数据`）
    fn write_memory(&mut self, args: &str) -> Result<String, Error> {
        let (header, data) = args.split_once(':').ok_or(Error::InvalidArgument)?;
        let (address, length) = header.split_once(',').ok_or(Error::InvalidArgument)?;
        let address = parse_u32_hex(address)?;
        let length = parse_usize_hex(length)?;
        if length > MAX_PACKET_SIZE {
            return Err(Error::PacketTooLarge);
        }
        let bytes = decode_hex(data)?;
        if bytes.len() != length {
            return Err(Error::InvalidArgument);
        }
        self.target.write_memory(address, &bytes)?;
        Ok("OK".to_string())
    }

    /// Insert or remove an instruction breakpoint (`Z0`/`Z1`) or a watchpoint
    /// (`Z2`/`Z3`/`Z4`, write/read/access)
    /// 插入或移除指令断点（`Z0`/`Z1`）或监视点（`Z2`/`Z3`/`Z4`，写/读/访问）
    fn change_breakpoint(&mut self, args: &str, insert: bool) -> Result<String, Error> {
        let (kind, rest) = args.split_once(',').ok_or(Error::InvalidArgument)?;
        let (address, size) = rest.split_once(',').ok_or(Error::InvalidArgument)?;
        let address = parse_u32_hex(address)?;
        let size = parse_usize_hex(size)?;
        match kind {
            "0" | "1" => {
                if insert {
                    self.target.set_breakpoint(address)?;
                } else {
                    self.target.clear_breakpoint(address)?;
                }
                Ok("OK".to_string())
            }
            // `Z2` = write (`watch`), `Z3` = read (`rwatch`), `Z4` = access
            // (`awatch`): identical syntax and width validation.
            // `Z2` = 写（`watch`），`Z3` = 读（`rwatch`），`Z4` = 访问（`awatch`）：
            // 语法与宽度校验完全一致。
            "2" => self.change_watchpoint(address, size, WatchKind::Write, insert),
            "3" => self.change_watchpoint(address, size, WatchKind::Read, insert),
            "4" => self.change_watchpoint(address, size, WatchKind::Access, insert),
            _ => Err(Error::InvalidArgument),
        }
    }

    /// Insert or remove one watchpoint of `kind` / 插入或移除一个指定类型的监视点
    ///
    /// Re-inserting or re-removing the same `(address, length, kind)` is
    /// idempotent: the session remembers armed watchpoints and only forwards the
    /// first insert and the last remove to the target. The same address may be
    /// armed independently as a write, read and access watchpoint.
    /// 对同一 `(地址, 长度, 类型)` 重复插入或删除是幂等的：会话记住已生效的监视点，
    /// 只把首次插入与最后一次删除转发给目标。同一地址可分别安装写、读与访问监视点。
    fn change_watchpoint(
        &mut self,
        address: u32,
        size: usize,
        kind: WatchKind,
        insert: bool,
    ) -> Result<String, Error> {
        // GDB encodes the watchpoint width in bytes; only the hardware-supported
        // widths are meaningful for this target.
        // GDB 用字节数表示监视点宽度；只有硬件支持的宽度对该目标有意义。
        if !matches!(size, 1 | 2 | 4 | 8) {
            return Ok(String::new());
        }
        let entry = (address, size, kind);
        if insert {
            if !self.watchpoints.contains(&entry) {
                match kind {
                    WatchKind::Write => self.target.set_watchpoint(address, size)?,
                    WatchKind::Read => self.target.set_read_watchpoint(address, size)?,
                    WatchKind::Access => self.target.set_access_watchpoint(address, size)?,
                }
                self.watchpoints.insert(entry);
            }
        } else if self.watchpoints.contains(&entry) {
            match kind {
                WatchKind::Write => self.target.clear_watchpoint(address, size)?,
                WatchKind::Read => self.target.clear_read_watchpoint(address, size)?,
                WatchKind::Access => self.target.clear_access_watchpoint(address, size)?,
            }
            self.watchpoints.remove(&entry);
        }
        Ok("OK".to_string())
    }

    /// Resume or step and format the resulting stop reply (`c`/`s`/`vCont`)
    /// 继续或单步执行并格式化停止回复（`c`/`s`/`vCont`）
    fn resume(&mut self, mode: ResumeMode) -> Result<String, Error> {
        let reply = self.target.resume(mode)?;
        let keyword = match reply.reason {
            StopReason::Watchpoint => "watch",
            StopReason::ReadWatch => "rwatch",
            StopReason::AccessWatch => "awatch",
            _ => return Ok(format!("S{:02x}", reply.signal())),
        };
        let Some(address) = self.target.watchpoint_hit() else {
            return Ok(format!("T{:02x}", reply.signal()));
        };
        Ok(format!("T{:02x}{keyword}:{address:x};", reply.signal()))
    }

    /// Handle `Q...` extension packets / 处理 `Q...` 扩展包
    fn handle_extended(&mut self, payload: &str) -> String {
        if payload == "QStartNoAckMode" {
            self.no_ack_mode = true;
            "OK".to_string()
        } else {
            String::new()
        }
    }

    /// Handle `v...` packets, including minimal single-thread `vCont`
    /// 处理 `v...` 包，包括最小单线程 `vCont`
    fn handle_v(&mut self, payload: &str) -> Result<String, Error> {
        if payload == "vCont?" {
            return Ok("vCont;c;s".to_string());
        }
        let Some(actions) = payload.strip_prefix("vCont;") else {
            return Ok(String::new());
        };
        let mut parts = actions.split(';');
        let Some(action) = parts.next() else {
            return Ok(String::new());
        };
        // Multiple actions describe multi-core control, which is out of scope.
        // 多个动作描述多核控制，不在本阶段范围内。
        if parts.next().is_some() {
            return Ok(String::new());
        }
        let action = action.split(':').next().unwrap_or_default();
        match action {
            "c" => self.resume(ResumeMode::Continue),
            "s" => self.resume(ResumeMode::Step),
            _ => Ok(String::new()),
        }
    }

    /// Reply to `q...` queries / 回复 `q...` 查询
    fn query_reply(&self, payload: &str) -> String {
        // `qXfer:features:read:<annex>:<offset>,<length>` is the only `qXfer`
        // object served today; everything else falls through to the empty
        // reply that tells GDB the object is unsupported.
        // `qXfer:features:read:<annex>:<偏移>,<长度>` 是目前唯一提供的 `qXfer`
        // 对象；其余请求落到空回复，告知 GDB 该对象不受支持。
        if let Some(args) = payload.strip_prefix("qXfer:features:read:") {
            return self.read_features(args);
        }
        if payload.starts_with("qSupported") {
            let mut reply = format!(
                "PacketSize={MAX_PACKET_SIZE:03x};qXfer:features:read+;QStartNoAckMode+;vContSupported+;hwbreak+"
            );
            // `watchpoint+` is an extension hint: it is only advertised when the
            // target really implements `Z2`/`z2`, so GDB can fall back cleanly.
            // `watchpoint+` 是扩展提示：仅在目标真正实现 `Z2`/`z2` 时声明，
            // 便于 GDB 干净地回退。
            if self.target.supports_watchpoints() {
                reply.push_str(";watchpoint+");
            }
            return reply;
        }
        if payload.starts_with("qAttached") {
            return "1".to_string();
        }
        if payload == "qfThreadInfo" {
            // One stopped thread; without this GDB declares "Thread 1 exited"
            // and refuses to continue. / 只有一个线程；缺少该回复时 GDB 会判定
            // “Thread 1 exited” 并拒绝继续。
            return "m1".to_string();
        }
        if payload == "qsThreadInfo" {
            return "l".to_string();
        }
        if payload == "qC" {
            return "QC1".to_string();
        }
        String::new()
    }

    /// Serve one `qXfer:features:read` transfer for `target.xml`
    /// 为 `target.xml` 提供一个 `qXfer:features:read` 传输
    ///
    /// The argument layout is `<annex>:<offset>,<length>` with hexadecimal
    /// numbers. Unknown annexes, malformed numbers, and targets without a
    /// description answer [`ANNEX_ERROR`] (`E00`); a valid annex is sliced as
    /// `m<chunk>` (more follows) or `l<chunk>` (last chunk, possibly empty).
    /// 参数格式为 `<annex>:<偏移>,<长度>`，数字为十六进制。未知 annex、非法
    /// 数字以及无目标描述的目标均回复 [`ANNEX_ERROR`]（`E00`）；合法 annex 按
    /// `m<块>`（还有后续）或 `l<块>`（最后一块，可为空）切片。
    fn read_features(&self, args: &str) -> String {
        // Split from the right: annex names of other `qXfer` objects may
        // contain colons, while the range is always the final field.
        // 从右侧分割：其他 `qXfer` 对象的 annex 名可能含冒号，而范围字段总在末尾。
        let Some((annex, range)) = args.rsplit_once(':') else {
            return ANNEX_ERROR.to_string();
        };
        let Some((offset, length)) = range.split_once(',') else {
            return ANNEX_ERROR.to_string();
        };
        let (Ok(offset), Ok(length)) = (parse_usize_hex(offset), parse_usize_hex(length)) else {
            return ANNEX_ERROR.to_string();
        };
        if annex != ANNEX {
            return ANNEX_ERROR.to_string();
        }
        let Some(xml) = self.target.target_description() else {
            return ANNEX_ERROR.to_string();
        };
        crate::target_xml::read_chunk(xml.as_bytes(), offset, length)
    }
}

/// Parse a hexadecimal `u32` / 解析十六进制 `u32`
fn parse_u32_hex(text: &str) -> Result<u32, Error> {
    u32::from_str_radix(text, 16).map_err(|_| Error::InvalidHex)
}

/// Parse a hexadecimal `usize` / 解析十六进制 `usize`
fn parse_usize_hex(text: &str) -> Result<usize, Error> {
    usize::from_str_radix(text, 16).map_err(|_| Error::InvalidHex)
}

/// Decode one ASCII hexadecimal digit / 解码一位 ASCII 十六进制数字
fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Encode a 32-bit register value as little-endian hex (`g`/`p` format)
/// 将 32 位寄存器值编码为小端十六进制（`g`/`p` 格式）
fn encode_u32_le(value: u32) -> String {
    encode_hex(&value.to_le_bytes())
}

/// Decode an eight-digit little-endian register value / 解码八位小端寄存器值
fn decode_u32_le(text: &str) -> Result<u32, Error> {
    if text.len() != 8 {
        return Err(Error::InvalidHex);
    }
    let bytes: [u8; 4] = decode_hex(text)?
        .try_into()
        .map_err(|_| Error::InvalidHex)?;
    Ok(u32::from_le_bytes(bytes))
}
