//! rp2350-gdb — GDB RSP (Remote Serial Protocol) server
//!
//! Implements the transport-independent RSP protocol layer (packet codec and
//! command dispatch) that exposes the simulated RP2350 as a debug target. The
//! TCP transport and CLI wiring arrive in a later task.
//!
//! 实现与传输无关的 RSP 协议层（包编解码与命令分发），将模拟的 RP2350 暴露
//! 为调试目标。TCP 传输与 CLI 接线将在后续任务中完成。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod cortex_m33;
pub mod rsp;
pub mod server;
pub mod target;

pub use cortex_m33::{CortexM33Target, DEFAULT_INSTRUCTION_BUDGET};
pub use rsp::{
    ACK, Ack, ERROR_REPLY, Error as RspError, MAX_PACKET_SIZE, NACK, Session as RspSession,
    checksum, decode_hex, decode_packet, encode_hex, encode_packet, escape_binary, parse_ack,
    unescape_binary,
};
pub use server::GdbServer;
pub use target::{GdbError, GdbTarget, ResumeMode, StopReason, StopReply, TargetStatus};

// 协议测试放在 lib.rs，使 rsp.rs 的实现保持在单文件 500 行代码上限内；
// 它们仍是同一 crate 的单元测试。
// Protocol tests live in lib.rs so the rsp.rs implementation stays within the
// 500-code-line per-file limit; they remain unit tests of the same crate.
#[cfg(test)]
mod rsp_tests {
    use std::collections::BTreeSet;

    use rp2350_core::error::{BusErrorKind, CoreError};
    use rp2350_cpu::CpuState;

    use crate::rsp::{
        ACK, Ack, Error, NACK, Session, checksum, decode_hex, decode_packet, encode_hex,
        encode_packet, escape_binary, parse_ack, unescape_binary,
    };
    use crate::target::{GdbError, GdbTarget, ResumeMode, StopReason, StopReply, TargetStatus};

    const MEMORY_BASE: u32 = 0x2000_0000;

    /// Two-register, eight-byte mock target used by the protocol tests
    /// 协议测试使用的双寄存器、八字节 mock 目标
    struct MockTarget {
        registers: [u32; 2],
        memory: Vec<u8>,
        breakpoints: BTreeSet<u32>,
        last_stop: Option<StopReason>,
    }

    impl MockTarget {
        /// Build a deterministic mock target / 构造确定性的 mock 目标
        fn new() -> Self {
            Self {
                registers: [0x1111_1111, 0x2222_2222],
                memory: vec![0; 8],
                breakpoints: BTreeSet::new(),
                last_stop: None,
            }
        }
    }

    /// Build the generic no-device bus error for unmapped mock addresses
    /// 为未映射的 mock 地址构造通用无设备总线错误
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

    /// Build a session around a fresh mock target / 围绕全新 mock 目标创建会话
    fn session() -> Session<MockTarget> {
        Session::new(MockTarget::new())
    }

    #[test]
    fn checksum_wraps_payload_bytes() {
        assert_eq!(checksum(b""), 0);
        assert_eq!(checksum(b"OK"), 0x9a);
        assert_eq!(checksum(&[0xff, 0x02]), 0x01);
    }

    #[test]
    fn packet_codec_round_trips_with_lowercase_checksum() {
        assert_eq!(encode_packet("OK"), "$OK#9a");
        assert_eq!(decode_packet(b"$OK#9a").unwrap(), "OK");
        assert_eq!(decode_packet(b"$OK#9A").unwrap(), "OK");
        // `?` has checksum 0x3f and is the attach query.
        // `?` 的校验和为 0x3f，是附加查询。
        assert_eq!(decode_packet(b"$?#3f").unwrap(), "?");
    }

    #[test]
    fn decode_rejects_malformed_and_mismatched_packets() {
        assert!(matches!(decode_packet(b"OK#9a"), Err(Error::MissingStart)));
        assert!(matches!(
            decode_packet(b"$OK9a"),
            Err(Error::MissingChecksum)
        ));
        assert!(matches!(
            decode_packet(b"$OK#9"),
            Err(Error::InvalidChecksumLength)
        ));
        assert!(matches!(
            decode_packet(b"$OK#zz"),
            Err(Error::InvalidChecksumDigit)
        ));
        assert!(matches!(
            decode_packet(b"$OK#9b"),
            Err(Error::ChecksumMismatch {
                computed: 0x9a,
                received: 0x9b
            })
        ));
        assert!(matches!(
            decode_packet(b"$\xff#ff"),
            Err(Error::InvalidUtf8)
        ));
    }

    #[test]
    fn binary_escaping_round_trips_reserved_bytes() {
        let payload = b"a$b#c}d*e\x00\xff";
        let escaped = escape_binary(payload);
        assert_eq!(escaped, b"a}\x04b}\x03c}\x5dd}\x0ae\x00\xff".to_vec());
        assert_eq!(unescape_binary(&escaped).unwrap(), payload.to_vec());
    }

    #[test]
    fn unescape_rejects_trailing_escape() {
        assert!(matches!(
            unescape_binary(b"abc}"),
            Err(Error::TrailingEscape)
        ));
    }

    #[test]
    fn hex_codec_is_lowercase_and_validates_length() {
        assert_eq!(encode_hex(&[0x00, 0xab, 0xff]), "00abff");
        assert_eq!(decode_hex("00ABff").unwrap(), vec![0x00, 0xab, 0xff]);
        assert!(matches!(decode_hex("abc"), Err(Error::InvalidHex)));
        assert!(matches!(decode_hex("zz"), Err(Error::InvalidHex)));
    }

    #[test]
    fn ack_bytes_parse_to_transport_signals() {
        assert_eq!(parse_ack(ACK), Some(Ack::Positive));
        assert_eq!(parse_ack(NACK), Some(Ack::Negative));
        assert_eq!(parse_ack(b'x'), None);
        assert_eq!(Ack::Positive.as_byte(), b'+');
        assert_eq!(Ack::Negative.as_byte(), b'-');
    }

    #[test]
    fn attach_query_reports_sigtrap() {
        assert_eq!(session().handle_packet("?").as_deref(), Some("S05"));
    }

    #[test]
    fn q_supported_advertises_core_features() {
        let reply = session().handle_packet("qSupported:multiprocess+").unwrap();
        assert!(reply.contains("PacketSize="));
        assert!(reply.contains("QStartNoAckMode+"));
        assert!(reply.contains("vContSupported+"));
        assert!(reply.contains("hwbreak+"));
    }

    #[test]
    fn thread_and_detach_commands_reply_ok() {
        let mut session = session();
        assert_eq!(session.handle_packet("qC").as_deref(), Some("QC1"));
        assert_eq!(session.handle_packet("qAttached").as_deref(), Some("1"));
        assert_eq!(session.handle_packet("Hg0").as_deref(), Some("OK"));
        assert_eq!(session.handle_packet("Hc1").as_deref(), Some("OK"));
        assert_eq!(session.handle_packet("D").as_deref(), Some("OK"));
        assert_eq!(session.handle_packet("k"), None);
    }

    #[test]
    fn bulk_register_reads_and_writes_round_trip() {
        let mut session = session();
        assert_eq!(
            session.handle_packet("g").as_deref(),
            Some("1111111122222222")
        );
        let data = "78563412efcdab90";
        assert_eq!(
            session.handle_packet(&format!("G{data}")).as_deref(),
            Some("OK")
        );
        assert_eq!(session.handle_packet("g").as_deref(), Some(data));
        assert_eq!(session.target().registers, [0x1234_5678, 0x90ab_cdef]);
    }

    #[test]
    fn single_register_packets_round_trip() {
        let mut session = session();
        assert_eq!(session.handle_packet("p0").as_deref(), Some("11111111"));
        assert_eq!(session.handle_packet("P0=78563412").as_deref(), Some("OK"));
        assert_eq!(session.handle_packet("p0").as_deref(), Some("78563412"));
        assert_eq!(session.handle_packet("p2").as_deref(), Some("E01"));
        assert_eq!(session.handle_packet("P2=78563412").as_deref(), Some("E01"));
    }

    #[test]
    fn memory_packets_round_trip_and_errors_keep_session_alive() {
        let mut session = session();
        assert_eq!(
            session.handle_packet("m20000000,4").as_deref(),
            Some("00000000")
        );
        assert_eq!(
            session.handle_packet("M20000000,4:a1b2c3d4").as_deref(),
            Some("OK")
        );
        assert_eq!(
            session.handle_packet("m20000000,4").as_deref(),
            Some("a1b2c3d4")
        );
        assert_eq!(session.handle_packet("m50000000,1").as_deref(), Some("E01"));
        assert_eq!(
            session.handle_packet("M20000000,4:a1").as_deref(),
            Some("E01")
        );
        assert_eq!(
            session.handle_packet("m20000000,2").as_deref(),
            Some("a1b2")
        );
    }

    #[test]
    fn breakpoint_packets_toggle_target_state() {
        let mut session = session();
        assert_eq!(
            session.handle_packet("Z0,20000004,2").as_deref(),
            Some("OK")
        );
        assert!(session.target().has_breakpoint(0x2000_0004));
        assert_eq!(
            session.handle_packet("Z1,20000008,4").as_deref(),
            Some("OK")
        );
        assert!(session.target().has_breakpoint(0x2000_0008));
        assert_eq!(
            session.handle_packet("z0,20000004,2").as_deref(),
            Some("OK")
        );
        assert!(!session.target().has_breakpoint(0x2000_0004));
        assert_eq!(
            session.handle_packet("Z2,20000004,4").as_deref(),
            Some("E01")
        );
    }

    #[test]
    fn continue_and_step_return_stop_replies() {
        let mut session = session();
        assert_eq!(session.handle_packet("c").as_deref(), Some("S05"));
        assert_eq!(session.handle_packet("C05").as_deref(), Some("S05"));
        assert_eq!(session.handle_packet("s").as_deref(), Some("S05"));
        assert_eq!(session.handle_packet("S05").as_deref(), Some("S05"));
    }

    #[test]
    fn vcont_negotiation_and_single_thread_actions() {
        let mut session = session();
        assert_eq!(
            session.handle_packet("vCont?").as_deref(),
            Some("vCont;c;s")
        );
        assert_eq!(session.handle_packet("vCont;c:1").as_deref(), Some("S05"));
        assert_eq!(session.handle_packet("vCont;s").as_deref(), Some("S05"));
        // Multi-action vCont is deliberately unsupported at this layer.
        // 多动作 vCont 在本层故意不支持。
        assert_eq!(session.handle_packet("vCont;c;s").as_deref(), Some(""));
    }

    #[test]
    fn no_ack_mode_is_stateful() {
        let mut session = session();
        assert!(!session.no_ack_mode());
        assert_eq!(
            session.handle_packet("QStartNoAckMode").as_deref(),
            Some("OK")
        );
        assert!(session.no_ack_mode());
        assert_eq!(
            session.handle_packet("QStartNoAckMode").as_deref(),
            Some("OK")
        );
    }

    #[test]
    fn unknown_commands_return_empty_packets() {
        let mut session = session();
        assert_eq!(
            session.handle_packet("vMustReplyEmpty").as_deref(),
            Some("")
        );
        assert_eq!(
            session
                .handle_packet("qXfer:features:read:target.xml:0,1")
                .as_deref(),
            Some("")
        );
        assert_eq!(session.handle_packet("!").as_deref(), Some(""));
        assert_eq!(session.handle_packet("").as_deref(), Some(""));
    }

    #[test]
    fn malformed_arguments_map_to_e01() {
        let mut session = session();
        assert_eq!(session.handle_packet("m20000000").as_deref(), Some("E01"));
        assert_eq!(session.handle_packet("M20000000,4").as_deref(), Some("E01"));
        assert_eq!(session.handle_packet("G123").as_deref(), Some("E01"));
        assert_eq!(session.handle_packet("P0=1234").as_deref(), Some("E01"));
        assert_eq!(session.handle_packet("p").as_deref(), Some("E01"));
        assert_eq!(
            session.handle_packet("m20000000,10000").as_deref(),
            Some("E01")
        );
    }

    #[test]
    fn frame_codec_errors_reply_e01_and_keep_session_usable() {
        let mut session = session();
        assert_eq!(session.handle_frame(b"$OK#9b").as_deref(), Some("E01"));
        assert_eq!(session.handle_frame(b"malformed").as_deref(), Some("E01"));
        let frame = encode_packet("qC");
        assert_eq!(
            session.handle_frame(frame.as_bytes()).as_deref(),
            Some("QC1")
        );
    }
}
