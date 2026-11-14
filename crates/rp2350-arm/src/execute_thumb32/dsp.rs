//! Thumb-32 DSP packed SIMD arithmetic
//! Thumb-32 DSP 打包 SIMD 算术
//!
//! Covers the ARMv8-M DSP parallel byte/halfword add/subtract family
//! (`SADD8` … `USAX`), their saturating `Q`/`UQ` forms, the 32-bit saturating
//! `QADD`/`QSUB`/`QDADD`/`QDSUB`, `SEL`, and the `SSAT`/`USAT`/`SSAT16`/
//! `USAT16` saturation instructions.
//!
//! Flag rules (ARM ARM):
//!
//! * parallel add/subtract (`S*`/`U*`) update `APSR.GE` — one flag per byte
//!   lane, or a duplicated pair per halfword lane — and never touch `Q`/`NZCV`;
//! * saturating forms (parallel `Q*`/`UQ*`, `QADD`/`QSUB`/`QDADD`/`QDSUB`,
//!   `SSAT`/`USAT`) update only `APSR.Q`, which is sticky, and never touch
//!   `GE`/`NZCV`;
//! * `SEL` reads `GE` and updates no flag at all.
//!
//! 覆盖 ARMv8-M DSP 并行字节/半字加/减指令族（`SADD8` … `USAX`）、其饱和
//! `Q`/`UQ` 形式、32 位饱和 `QADD`/`QSUB`/`QDADD`/`QDSUB`、`SEL`，以及
//! `SSAT`/`USAT`/`SSAT16`/`USAT16` 饱和指令。
//!
//! 标志规则（ARM ARM）：
//!
//! * 并行加/减（`S*`/`U*`）更新 `APSR.GE` —— 每个字节通道一个标志，半字通道
//!   为一对重复标志 —— 不影响 `Q`/`NZCV`；
//! * 饱和形式（并行 `Q*`/`UQ*`、`QADD`/`QSUB`/`QDADD`/`QDSUB`、
//!   `SSAT`/`USAT`）只更新保持型的 `APSR.Q`，不影响 `GE`/`NZCV`；
//! * `SEL` 读取 `GE`，不更新任何标志。

use crate::execute_thumb16::StepResult;
use crate::registers::CortexM33Registers;

use super::sign_extend;

/// Packed lane operation selected by instruction bits [23:20]
/// 由指令 bits[23:20] 选择的打包通道操作
#[derive(Clone, Copy, PartialEq, Eq)]
enum LaneOp {
    /// Byte-lane add (`ADD8`) / 字节通道加法
    Add8,
    /// Halfword-lane add (`ADD16`) / 半字通道加法
    Add16,
    /// Exchanged add/subtract (`ASX`) / 交换加/减
    Asx,
    /// Byte-lane subtract (`SUB8`) / 字节通道减法
    Sub8,
    /// Halfword-lane subtract (`SUB16`) / 半字通道减法
    Sub16,
    /// Reversed exchanged add/subtract (`SAX`) / 反向交换加/减
    Sax,
}

impl LaneOp {
    /// Decode the operation from instruction bits [23:20]
    /// 从指令 bits[23:20] 解码操作
    fn from_bits(bits: u32) -> Option<Self> {
        match bits {
            0x8 => Some(Self::Add8),
            0x9 => Some(Self::Add16),
            0xA => Some(Self::Asx),
            0xC => Some(Self::Sub8),
            0xD => Some(Self::Sub16),
            0xE => Some(Self::Sax),
            _ => None,
        }
    }

    /// Lane width in bits / 通道位宽
    fn lane_bits(self) -> u32 {
        match self {
            Self::Add8 | Self::Sub8 => 8,
            _ => 16,
        }
    }
}

/// Saturation behaviour of a packed lane operation
/// 打包通道操作的饱和行为
#[derive(Clone, Copy, PartialEq, Eq)]
enum Variant {
    /// Signed, wrapping (`S`) / 有符号回绕（`S`）
    WrapSigned,
    /// Signed, saturating (`Q`) / 有符号饱和（`Q`）
    SatSigned,
    /// Unsigned, wrapping (`U`) / 无符号回绕（`U`）
    WrapUnsigned,
    /// Unsigned, saturating (`UQ`) / 无符号饱和（`UQ`）
    SatUnsigned,
}

impl Variant {
    /// Decode the variant from instruction bits [7:4]
    /// 从指令 bits[7:4] 解码变体
    ///
    /// `0b0010`/`0b0110` (halving) stay unclaimed and therefore undefined.
    /// `0b0010`/`0b0110`（折半）不被认领，因此仍视为未定义。
    fn from_bits(bits: u32) -> Option<Self> {
        match bits {
            0x0 => Some(Self::WrapSigned),
            0x1 => Some(Self::SatSigned),
            0x4 => Some(Self::WrapUnsigned),
            0x5 => Some(Self::SatUnsigned),
            _ => None,
        }
    }

    /// Whether the lanes are interpreted as signed values
    /// 通道是否按有符号值解释
    fn signed(self) -> bool {
        matches!(self, Self::WrapSigned | Self::SatSigned)
    }

    /// Whether the variant updates `APSR.Q` instead of `APSR.GE`
    /// 该变体是否更新 `APSR.Q` 而非 `APSR.GE`
    fn saturating(self) -> bool {
        matches!(self, Self::SatSigned | Self::SatUnsigned)
    }
}

/// Try to execute a packed parallel add/subtract instruction
/// 尝试执行打包并行加/减指令
///
/// The Thumb-32 encodings are `1111 1010 10xx Rn 1111 Rd vvvv Rm`: bits[23:20]
/// select `ADD8` (0x8), `ADD16` (0x9), `ASX` (0xA), `SUB8` (0xC), `SUB16` (0xD)
/// or `SAX` (0xE), and bits[7:4] select `S` (0), `Q` (1), `U` (4) or `UQ` (5).
/// Two encodings share those bits and are claimed first: `QADD/QDADD/QSUB/QDSUB`
/// (`op == 0x8`, bits[7:4] = 0x8…0xB) and `SEL` (`op == 0xA`, bits[7:4] = 0x8).
/// Thumb-32 编码为 `1111 1010 10xx Rn 1111 Rd vvvv Rm`：bits[23:20] 选择
/// `ADD8`（0x8）、`ADD16`（0x9）、`ASX`（0xA）、`SUB8`（0xC）、`SUB16`（0xD）
/// 或 `SAX`（0xE），bits[7:4] 选择 `S`（0）、`Q`（1）、`U`（4）或 `UQ`（5）。
/// 有两条编码与上述位段重叠，需先认领：`QADD/QDADD/QSUB/QDSUB`
/// （`op == 0x8`，bits[7:4] = 0x8…0xB）与 `SEL`（`op == 0xA`，bits[7:4] = 0x8）。
pub(super) fn try_execute(instr: u32, regs: &mut CortexM33Registers) -> Option<StepResult> {
    let op = (instr >> 20) & 0xF;
    let op2 = (instr >> 4) & 0xF;
    // Rd occupies bits[11:8]; bits[15:12] are fixed 1111 in this whole space.
    // Rd 位于 bits[11:8]；该编码空间的 bits[15:12] 固定为 1111。
    if (instr >> 12) & 0xF != 0xF {
        return None;
    }

    if op == 0x8 && (0x8..=0xB).contains(&op2) {
        return Some(execute_qadd_family(instr, regs));
    }
    if op == 0xA && op2 == 0x8 {
        return Some(execute_select(instr, regs));
    }

    let lane_op = LaneOp::from_bits(op)?;
    let variant = Variant::from_bits(op2)?;
    Some(execute_lanes(instr, regs, lane_op, variant))
}

/// Execute `SSAT`/`SSAT16`/`USAT`/`USAT16`
/// 执行 `SSAT`/`SSAT16`/`USAT`/`USAT16`
///
/// These live in data-processing (plain binary immediate) with
/// `bits[26:20] = 0b0110000` (signed, `sh` clear), `0b0110010` (signed `asr`,
/// or the 16-bit form), `0b0111000` (unsigned) and `0b0111010` (unsigned `asr`,
/// or the 16-bit form). The saturation width is `sat_imm + 1` (signed,
/// 1-32 bits) or `sat_imm` (unsigned, 0-31 bits); `imm3:imm2` is the shift
/// amount and bit 21 selects `lsl` (0) or `asr` (1). The 16-bit forms are the
/// `sh = 1`, zero-shift encodings with a saturable width of at most 16 bits.
/// Any saturation sets the sticky `Q` flag; `GE` and `NZCV` are preserved.
/// 这些指令属于数据处理（纯二进制立即数）组，`bits[26:20]` 为
/// `0b0110000`（有符号、`sh` 清零）、`0b0110010`（有符号 `asr` 或 16 位形式）、
/// `0b0111000`（无符号）与 `0b0111010`（无符号 `asr` 或 16 位形式）。饱和位宽
/// 为 `sat_imm + 1`（有符号，1-32 位）或 `sat_imm`（无符号，0-31 位）；
/// `imm3:imm2` 为移位量，位 21 选择 `lsl`（0）或 `asr`（1）。16 位形式即
/// `sh = 1` 且移位量为零、饱和位宽不超过 16 位的编码。任一次饱和都会置保持型
/// `Q` 标志；`GE` 与 `NZCV` 保持不变。
pub(super) fn execute_saturate(instr: u32, regs: &mut CortexM33Registers) -> StepResult {
    let op = (instr >> 20) & 0x1F;
    let rd = ((instr >> 8) & 0xF) as u8;
    let rn = ((instr >> 16) & 0xF) as u8;
    let sat_imm = instr & 0x1F;
    // bits[24] = 1, bits[22] and bits[20] = 0, bit 5 reserved-zero, bit 15 = 0.
    // bits[24] = 1，bits[22] 与 bits[20] = 0，位 5 保留为零，位 15 = 0。
    if op & 0b1_0101 != 0b1_0000 || instr & (1 << 5) != 0 || instr & (1 << 15) != 0 || rd == 15 {
        return StepResult::Undefined;
    }

    let unsigned = op & 0b0_1000 != 0;
    let shift = (((instr >> 12) & 0x7) << 2) | ((instr >> 6) & 0x3);
    let value = regs.read(rn);
    let shifted = if shift == 0 {
        value
    } else if op & 0b0_0010 != 0 {
        ((value as i32) >> shift) as u32
    } else {
        value << shift
    };

    // The 16-bit forms have no shift field and a width of at most 16 bits.
    // 16 位形式没有移位字段，且位宽不超过 16 位。
    if op & 0b0_0010 != 0 && shift == 0 && sat_imm < 16 {
        let (result, q) = saturate_halfwords(shifted, sat_imm, unsigned);
        regs.write(rd, result);
        if q {
            regs.set_q_flag();
        }
        return StepResult::Ok;
    }

    let exact = i64::from(shifted as i32);
    let (result, saturated) = if unsigned {
        clamp_unsigned_lane(exact, sat_imm)
    } else {
        // Signed results are written sign-extended to 32 bits.
        // 有符号结果写入时按符号扩展到 32 位。
        let (lane, saturated) = clamp_signed_lane(exact, sat_imm + 1);
        (sign_extend(lane, sat_imm + 1), saturated)
    };
    regs.write(rd, result);
    if saturated {
        regs.set_q_flag();
    }
    StepResult::Ok
}

/// Saturate both halfwords of a register independently
/// 独立饱和寄存器的两个半字
///
/// `sat_imm` is the signed width minus one, or the unsigned width.
/// `sat_imm` 为有符号位宽减一，或无符号位宽。
fn saturate_halfwords(value: u32, sat_imm: u32, unsigned: bool) -> (u32, bool) {
    let mut result = 0u32;
    let mut q = false;
    for i in 0..2u32 {
        let exact = i64::from(((value >> (i * 16)) & 0xFFFF) as u16 as i16);
        let (lane, saturated) = if unsigned {
            clamp_unsigned_lane(exact, sat_imm)
        } else {
            // Each saturated halfword lane is sign-extended to 16 bits.
            // 每个饱和半字通道按符号扩展到 16 位。
            let (lane, saturated) = clamp_signed_lane(exact, sat_imm + 1);
            (sign_extend(lane, sat_imm + 1) & 0xFFFF, saturated)
        };
        result |= lane << (i * 16);
        q |= saturated;
    }
    (result, q)
}

/// Execute `QADD`/`QDADD`/`QSUB`/`QDSUB`
/// 执行 `QADD`/`QDADD`/`QSUB`/`QDSUB`
///
/// `QDADD`/`QDSUB` saturate `2 * Rm` before the 32-bit accumulation, and
/// either saturation sets the sticky `Q` flag. `NZCV` and `GE` are preserved.
/// `QDADD`/`QDSUB` 先对 `2 * Rm` 饱和，再做 32 位累加；任一次饱和都置保持型
/// `Q` 标志。`NZCV` 与 `GE` 保持不变。
fn execute_qadd_family(instr: u32, regs: &mut CortexM33Registers) -> StepResult {
    let rd = ((instr >> 8) & 0xF) as u8;
    let rn = ((instr >> 16) & 0xF) as u8;
    let rm = (instr & 0xF) as u8;
    if rd == 15 {
        return StepResult::Undefined;
    }
    let n = i64::from(regs.read(rn) as i32);
    let m = i64::from(regs.read(rm) as i32);
    let op2 = (instr >> 4) & 0xF;

    let (exact, doubled_saturated) = match op2 {
        0x8 => (n + m, false),
        0xA => (n - m, false),
        0x9 | 0xB => {
            let (doubled, saturated) = clamp_signed_lane(m * 2, 32);
            let doubled = i64::from(doubled as i32);
            let exact = if op2 == 0x9 { n + doubled } else { n - doubled };
            (exact, saturated)
        }
        _ => return StepResult::Undefined,
    };

    let (result, saturated) = clamp_signed_lane(exact, 32);
    regs.write(rd, result);
    if saturated || doubled_saturated {
        regs.set_q_flag();
    }
    StepResult::Ok
}

/// Execute `SEL`: pick each byte from `Rn` or `Rm` using `APSR.GE`
/// 执行 `SEL`：按 `APSR.GE` 逐字节从 `Rn` 或 `Rm` 选择
fn execute_select(instr: u32, regs: &mut CortexM33Registers) -> StepResult {
    let rd = ((instr >> 8) & 0xF) as u8;
    let rn = ((instr >> 16) & 0xF) as u8;
    let rm = (instr & 0xF) as u8;
    if rd == 15 {
        return StepResult::Undefined;
    }
    let ge = regs.apsr_ge();
    let n = regs.read(rn);
    let m = regs.read(rm);
    let mut result = 0u32;
    for i in 0..4u32 {
        let mask = 0xFFu32 << (i * 8);
        result |= if ge & (1 << i) != 0 {
            n & mask
        } else {
            m & mask
        };
    }
    regs.write(rd, result);
    StepResult::Ok
}

/// Execute one parallel lane instruction
/// 执行一条并行通道指令
fn execute_lanes(
    instr: u32,
    regs: &mut CortexM33Registers,
    op: LaneOp,
    variant: Variant,
) -> StepResult {
    let rd = ((instr >> 8) & 0xF) as u8;
    let rn = ((instr >> 16) & 0xF) as u8;
    let rm = (instr & 0xF) as u8;
    if rd == 15 {
        // Writing PC is UNPREDICTABLE for these encodings.
        // 这些编码写 PC 属于不可预测行为。
        return StepResult::Undefined;
    }
    let a = regs.read(rn);
    let b = regs.read(rm);
    let (result, ge, q) = match op {
        LaneOp::Add8 | LaneOp::Sub8 => {
            pack_lanes(a, b, op.lane_bits(), op == LaneOp::Sub8, variant)
        }
        LaneOp::Add16 | LaneOp::Sub16 => {
            pack_lanes(a, b, op.lane_bits(), op == LaneOp::Sub16, variant)
        }
        LaneOp::Asx => pack_exchange(a, b, false, variant),
        LaneOp::Sax => pack_exchange(a, b, true, variant),
    };
    regs.write(rd, result);
    if variant.saturating() {
        if q {
            regs.set_q_flag();
        }
    } else {
        regs.set_apsr_ge(ge);
    }
    StepResult::Ok
}

/// Add or subtract independent byte or halfword lanes
/// 对独立的字节或半字通道做加/减
///
/// A halfword lane drives a whole GE pair: `GE[1:0]` for `bits[15:0]` and
/// `GE[3:2]` for `bits[31:16]`, with both bits of the pair set or cleared
/// together. Signed lanes flag GE when the full-width result is `>= 0`;
/// unsigned lanes use the carry-out / no-borrow rule.
/// 一个半字通道驱动整对 GE：`bits[15:0]` 对应 `GE[1:0]`，`bits[31:16]`
/// 对应 `GE[3:2]`，每对中的两位一起置位或清零。有符号通道在完整宽度结果
/// 大于等于零时置 GE；无符号通道采用进位/无借位规则。
fn pack_lanes(a: u32, b: u32, bits: u32, subtract: bool, variant: Variant) -> (u32, u32, bool) {
    let lanes = 32 / bits;
    let mut result = 0u32;
    let mut ge = 0u32;
    let mut q = false;
    for i in 0..lanes {
        let shift = i * bits;
        let (lane, lane_ge, lane_q) = compute_lane(
            lane_value(a, shift, bits, variant.signed()),
            lane_value(b, shift, bits, variant.signed()),
            subtract,
            bits,
            variant,
        );
        result |= lane << shift;
        if lane_ge {
            // One flag per byte lane, a duplicated pair per halfword lane.
            // 字节通道每道一个标志，半字通道为重复的一对。
            ge |= if bits == 8 { 1 << i } else { 0b11 << (i * 2) };
        }
        q |= lane_q;
    }
    (result, ge, q)
}

/// Add and subtract across the halfword lanes (`ASX`/`SAX`)
/// 跨半字通道做加/减（`ASX`/`SAX`）
///
/// The low lane takes `b[31:16]` and the high lane `b[15:0]`; `SAX` swaps the
/// add and subtract roles relative to `ASX`.
/// 低通道取 `b[31:16]`，高通道取 `b[15:0]`；`SAX` 与 `ASX` 相比交换加/减角色。
fn pack_exchange(a: u32, b: u32, reverse: bool, variant: Variant) -> (u32, u32, bool) {
    let signed = variant.signed();
    let low_subtract = reverse;
    let (low, low_ge, low_q) = compute_lane(
        lane_value(a, 0, 16, signed),
        lane_value(b, 16, 16, signed),
        low_subtract,
        16,
        variant,
    );
    let (high, high_ge, high_q) = compute_lane(
        lane_value(a, 16, 16, signed),
        lane_value(b, 0, 16, signed),
        !low_subtract,
        16,
        variant,
    );
    let mut ge = 0u32;
    if low_ge {
        ge |= 0b11;
    }
    if high_ge {
        ge |= 0b1100;
    }
    (low | (high << 16), ge, low_q || high_q)
}

/// Compute one lane, returning the packed lane, its GE flag and its saturation
/// 计算单个通道，返回打包后的通道值、GE 标志与是否饱和
fn compute_lane(a: i64, b: i64, subtract: bool, bits: u32, variant: Variant) -> (u32, bool, bool) {
    let exact = if subtract { a - b } else { a + b };
    let mask = (1i64 << bits) - 1;
    match variant {
        Variant::WrapSigned => (((exact as u64) & mask as u64) as u32, exact >= 0, false),
        Variant::WrapUnsigned => {
            let ge = if subtract { exact >= 0 } else { exact > mask };
            (((exact as u64) & mask as u64) as u32, ge, false)
        }
        Variant::SatSigned => {
            let (lane, saturated) = clamp_signed_lane(exact, bits);
            (lane, false, saturated)
        }
        Variant::SatUnsigned => {
            let (lane, saturated) = clamp_unsigned_lane(exact, bits);
            (lane, false, saturated)
        }
    }
}

/// Extract one packed lane and interpret it as signed or unsigned
/// 取出一个打包通道并按有符号或无符号解释
fn lane_value(value: u32, shift: u32, bits: u32, signed: bool) -> i64 {
    let raw = (value >> shift) & ((1u32 << bits) - 1);
    if signed {
        i64::from(sign_extend(raw, bits) as i32)
    } else {
        i64::from(raw)
    }
}

/// Clamp a value into a signed lane of `bits` bits, reporting saturation
/// 把值钳制到 `bits` 位有符号通道，并报告是否饱和
fn clamp_signed_lane(exact: i64, bits: u32) -> (u32, bool) {
    let max = (1i64 << (bits - 1)) - 1;
    let min = -(1i64 << (bits - 1));
    let mask = (1i64 << bits) - 1;
    if exact > max {
        ((max & mask) as u32, true)
    } else if exact < min {
        ((min & mask) as u32, true)
    } else {
        ((exact & mask) as u32, false)
    }
}

/// Clamp a value into an unsigned lane of `bits` bits, reporting saturation
/// 把值钳制到 `bits` 位无符号通道，并报告是否饱和
fn clamp_unsigned_lane(exact: i64, bits: u32) -> (u32, bool) {
    let max = (1i64 << bits) - 1;
    if exact < 0 {
        (0, true)
    } else if exact > max {
        (max as u32, true)
    } else {
        (exact as u32, false)
    }
}
