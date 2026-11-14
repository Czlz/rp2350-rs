//! Static ARMv8-M target description served through `qXfer:features:read`
//! 通过 `qXfer:features:read` 提供的静态 ARMv8-M 目标描述
//!
//! GDB asks for `qXfer:features:read:target.xml:<offset>,<length>` right after
//! connecting. The reply is either `m<data>` (more chunks follow) or
//! `l<data>` (this is the last chunk). This module owns the XML document and
//! the byte-oriented slicing logic; the RSP dispatcher in [`crate::rsp`] only
//! parses the packet and calls [`read_chunk`].
//! GDB 在连接后会立即请求 `qXfer:features:read:target.xml:<偏移>,<长度>`。
//! 回复为 `m<数据>`（还有后续分块）或 `l<数据>`（最后一块）。本模块持有 XML
//! 文档与按字节切片逻辑；[`crate::rsp`] 中的 RSP 分发器只负责解析包并调用
//! [`read_chunk`]。

/// Annex name of the ARM target description / ARM 目标描述的 annex 名称
pub const ANNEX: &str = "target.xml";

/// Error reply for an unknown annex or a malformed transfer range
/// 未知 annex 或非法传输范围时的错误回复
///
/// GDB treats `E00` as "this annex is unavailable"; it is used instead of the
/// generic `E01` so a missing description never looks like a broken session.
/// GDB 将 `E00` 视为「该 annex 不可用」；相比通用 `E01`，它不会让缺少目标
/// 描述看起来像会话损坏。
pub const ANNEX_ERROR: &str = "E00";

/// Static `target.xml` describing the exposed ARMv8-M register file
/// 描述所暴露 ARMv8-M 寄存器文件的静态 `target.xml`
///
/// Register numbers match [`crate::target::regs`]: r0-r12 are `0..=12`, sp is
/// `13`, lr is `14`, pc is `15`, and xpsr is `16`. The document is pure ASCII,
/// so XML text and byte offsets coincide and chunking cannot split a UTF-8
/// sequence. MSP, PSP, CONTROL and the FPU registers are deliberately absent
/// because the target cannot read them yet; GDB must not be told about
/// registers that `p`/`P` would reject.
/// 寄存器编号与 [`crate::target::regs`] 一致：r0-r12 为 `0..=12`，sp 为 `13`，
/// lr 为 `14`，pc 为 `15`，xpsr 为 `16`。文档为纯 ASCII，因此 XML 文本偏移与
/// 字节偏移一致，分块不会切断 UTF-8 序列。MSP、PSP、CONTROL 与 FPU 寄存器
/// 故意不声明，因为目标尚不能读取它们；不能向 GDB 声明 `p`/`P` 会拒绝的寄存器。
pub const TARGET_XML: &str = "\
<?xml version=\"1.0\"?>
<!DOCTYPE target SYSTEM \"gdb-target.dtd\">
<target>
  <architecture>armv8-m.main</architecture>
  <feature name=\"org.gnu.gdb.arm.m-profile\">
    <reg name=\"r0\" bitsize=\"32\" regnum=\"0\" type=\"data_ptr\"/>
    <reg name=\"r1\" bitsize=\"32\" regnum=\"1\" type=\"data_ptr\"/>
    <reg name=\"r2\" bitsize=\"32\" regnum=\"2\" type=\"data_ptr\"/>
    <reg name=\"r3\" bitsize=\"32\" regnum=\"3\" type=\"data_ptr\"/>
    <reg name=\"r4\" bitsize=\"32\" regnum=\"4\" type=\"data_ptr\"/>
    <reg name=\"r5\" bitsize=\"32\" regnum=\"5\" type=\"data_ptr\"/>
    <reg name=\"r6\" bitsize=\"32\" regnum=\"6\" type=\"data_ptr\"/>
    <reg name=\"r7\" bitsize=\"32\" regnum=\"7\" type=\"data_ptr\"/>
    <reg name=\"r8\" bitsize=\"32\" regnum=\"8\" type=\"data_ptr\"/>
    <reg name=\"r9\" bitsize=\"32\" regnum=\"9\" type=\"data_ptr\"/>
    <reg name=\"r10\" bitsize=\"32\" regnum=\"10\" type=\"data_ptr\"/>
    <reg name=\"r11\" bitsize=\"32\" regnum=\"11\" type=\"data_ptr\"/>
    <reg name=\"r12\" bitsize=\"32\" regnum=\"12\" type=\"data_ptr\"/>
    <reg name=\"sp\" bitsize=\"32\" regnum=\"13\" type=\"data_ptr\"/>
    <reg name=\"lr\" bitsize=\"32\" regnum=\"14\" type=\"int\"/>
    <reg name=\"pc\" bitsize=\"32\" regnum=\"15\" type=\"code_ptr\"/>
    <reg name=\"xpsr\" bitsize=\"32\" regnum=\"16\" type=\"int\"/>
  </feature>
</target>
";

/// Slice `target.xml` and format one `qXfer:features:read` reply
/// 对 `target.xml` 切片并生成一个 `qXfer:features:read` 回复
///
/// `offset` and `length` are byte counts. The reply carries `m` when more data
/// follows and `l` when the returned chunk reaches the end of the document.
/// Offsets at or beyond the end produce an empty `l` reply, which GDB accepts
/// as "nothing more to read". A `length` of zero also yields an empty `l`
/// reply: an `m` with no data would make the client loop forever.
/// `offset` 与 `length` 均以字节计。后续仍有数据时回复前缀为 `m`，返回块到达
/// 文档末尾时为 `l`。偏移等于或超出末尾返回空 `l` 块，GDB 将其视为「没有更多
/// 数据」；`length` 为 0 同样返回空 `l` 块，因为不带数据的 `m` 会让客户端
/// 无限循环。
///
/// # Examples
/// ```
/// use rp2350_gdb::target_xml::{TARGET_XML, read_chunk};
///
/// let whole = read_chunk(TARGET_XML.as_bytes(), 0, 0x0fff);
/// assert!(whole.starts_with('l'));
/// assert!(whole.contains("<architecture>armv8-m.main</architecture>"));
/// ```
#[must_use]
pub fn read_chunk(xml: &[u8], offset: usize, length: usize) -> String {
    let start = offset.min(xml.len());
    let end = offset.saturating_add(length).min(xml.len());
    let last = length == 0 || end >= xml.len();
    let marker = if last { 'l' } else { 'm' };
    format!("{marker}{}", String::from_utf8_lossy(&xml[start..end]))
}

#[cfg(test)]
mod tests {
    use super::{ANNEX, ANNEX_ERROR, TARGET_XML, read_chunk};

    #[test]
    fn target_xml_is_ascii_armv8_m_with_all_registers() {
        assert!(TARGET_XML.is_ascii(), "chunk offsets are byte offsets");
        assert!(TARGET_XML.contains("<architecture>armv8-m.main</architecture>"));
        assert!(TARGET_XML.contains("org.gnu.gdb.arm.m-profile"));
        assert!(TARGET_XML.contains("<reg name=\"xpsr\" bitsize=\"32\" regnum=\"16\""));
        assert_eq!(ANNEX, "target.xml");
        assert_eq!(ANNEX_ERROR, "E00");
    }

    #[test]
    fn full_read_is_a_single_last_chunk() {
        let reply = read_chunk(TARGET_XML.as_bytes(), 0, 0x0fff);
        assert_eq!(reply, format!("l{TARGET_XML}"));
    }

    #[test]
    fn chunk_boundaries_use_m_until_the_last_chunk() {
        let bytes = TARGET_XML.as_bytes();
        let first = read_chunk(bytes, 0, 8);
        assert!(first.starts_with('m'));
        assert_eq!(first.len(), 9);
        let tail = read_chunk(bytes, bytes.len() - 1, 8);
        assert!(tail.starts_with('l'));
        assert_eq!(&tail[1..], &TARGET_XML[TARGET_XML.len() - 1..]);
        assert_eq!(read_chunk(bytes, bytes.len(), 8), "l");
        assert_eq!(read_chunk(bytes, bytes.len() + 4, 8), "l");
        assert_eq!(read_chunk(bytes, 0, 0), "l");
    }
}
