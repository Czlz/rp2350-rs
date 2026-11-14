//! FPv5-SP arithmetic, comparison and conversion instructions (Stage 3)
//! FPv5-SP 算术、比较与转换指令（Stage 3）
//!
//! Covers the single-precision data-processing encodings rooted at `0xEE`
//! with `bits[11:8] = 0b1010` and `bit[4] = 0`:
//! 覆盖以 `0xEE` 开头、`bits[11:8] = 0b1010` 且 `bit[4] = 0` 的单精度
//! 数据处理编码：
//!
//! - Three-register arithmetic / 三寄存器算术: `VMLA`/`VMLS`,
//!   `VNMLA`/`VNMLS`, `VMUL`/`VNMUL`, `VADD`/`VSUB`, `VDIV`,
//!   `VFMA`/`VFMS`, `VFNMA`/`VFNMS`.
//! - Two-register unary ops / 两寄存器一元运算: `VMOV.F32 Sd, Sm`, `VABS`,
//!   `VNEG`, `VSQRT`.
//! - Comparisons / 比较: `VCMP`/`VCMPE` against a register or `#0.0`.
//! - Conversions / 转换: `VCVT.F32.S32`, `VCVT.F32.U32`,
//!   `VCVT.S32.F32`, `VCVT.U32.F32` plus the `VCVTR` rounding-mode forms.
//! - Conditional select / 条件选择: `VSELEQ`, `VSELVS`, `VSELGE`, `VSELGT`
//!   (the `0xFE` encoding space, selected by the FPSCR condition flags).
//! - Round to integral / 取整: `VRINTR`, `VRINTZ`, `VRINTX` plus the directed
//!   `VRINTA`, `VRINTN`, `VRINTP` and `VRINTM` forms.
//!
//! ## Numeric semantics / 数值语义
//!
//! All arithmetic is performed with Rust `f32`, i.e. IEEE-754 binary32 with
//! round-to-nearest, ties-to-even, which matches the FPv5-SP default rounding
//! mode. The `VFMA` family uses [`f32::mul_add`], so the multiply-add is fused
//! with a single rounding exactly like the hardware; `VMLA`/`VMLS` perform the
//! two roundings the architecture specifies.
//!
//! 全部算术使用 Rust `f32`，即 IEEE-754 binary32、就近舍入、偶数优先，与
//! FPv5-SP 的默认舍入模式一致。`VFMA` 族使用 [`f32::mul_add`]，与硬件一样
//! 融合乘加只做一次舍入；`VMLA`/`VMLS` 按架构规定做两次舍入。
//!
//! ## FPSCR modeling / FPSCR 建模范围
//!
//! - `N`/`Z`/`C`/`V` are written by `VCMP`/`VCMPE` following ARM semantics;
//!   an unordered comparison sets `C = 1` and `V = 1`.
//! - `IOC` is set for invalid operations (e.g. `0/0`, `inf - inf`,
//!   `0 × inf`, `sqrt(-1)`, unordered compares, out-of-range conversions),
//!   `DZC` for division of a finite non-zero value by zero, and `OFC | IXC`
//!   when a finite result overflows to infinity.
//! - `IXC` is also set by inexact integer conversions.
//! - `UFC` (underflow), the `FZ` flush-to-zero mode, `DN` default-NaN mode,
//!   `AHP`, and the FPSCR trap-enable bits are **not** modeled. NaN
//!   propagation follows the IEEE-754 quiet-NaN rules implemented by Rust.
//!
//! - `N`/`Z`/`C`/`V` 由 `VCMP`/`VCMPE` 按 ARM 语义写入；无序比较置
//!   `C = 1`、`V = 1`。
//! - 无效操作置 `IOC`（如 `0/0`、`inf - inf`、`0 × inf`、`sqrt(-1)`、
//!   无序比较、越界转换），有限非零值除以零置 `DZC`，有限结果上溢为无穷
//!   置 `OFC | IXC`。
//! - 不精确的整数转换同样置 `IXC`。
//! - **不建模** `UFC`（下溢）、`FZ` 清零模式、`DN` 默认 NaN 模式、`AHP`
//!   以及 FPSCR 的陷阱使能位。NaN 传播遵循 Rust 实现的 IEEE-754 静默 NaN
//!   规则。
//!
//! ## Not implemented / 未实现清单
//!
//! Double-precision encodings (`bits[11:8] = 0b1011`: `VADD.F64`,
//! `VCVT.F64.F32`, `VCVT.F32.F64`, …) are reported as undefined instructions
//! by this module. Half-precision (`VCVTB`/`VCVTT`), the 16-bit integer
//! conversion forms (`VCVT.F32.S16`, `VCVT.S16.F32`, …) and the
//! `VCVTA`/`VCVTN`/`VCVTP`/`VCVTM` directed float-to-integer conversions are
//! likewise undefined.
//! 双精度编码（`bits[11:8] = 0b1011`：`VADD.F64`、`VCVT.F64.F32`、
//! `VCVT.F32.F64` 等）由本模块按未定义指令上报。半精度
//! （`VCVTB`/`VCVTT`）、16 位整数转换形式（`VCVT.F32.S16`、
//! `VCVT.S16.F32` 等）与定向浮点转整数 `VCVTA`/`VCVTN`/`VCVTP`/`VCVTM`
//! 同样未定义。

// 浮点比较与精度改变转换是 IEEE-754 仿真的固有需求。
// Float comparisons and precision-changing casts are inherent to IEEE-754 emulation.
#![allow(
    clippy::float_cmp,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]

use crate::execute_thumb16::StepResult;
use crate::fpu::fpscr;
use crate::registers::CortexM33Registers;

use super::fpu::begin_fp_instruction;

/// Single-precision data-processing operation / 单精度数据处理操作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    /// `VADD.F32` / 浮点加
    Add,
    /// `VSUB.F32` / 浮点减
    Sub,
    /// `VMUL.F32` / 浮点乘
    Mul,
    /// `VNMUL.F32` / 取反的浮点乘
    Nmul,
    /// `VMLA.F32` / 乘加（两次舍入）
    Mla,
    /// `VMLS.F32` / 乘减（两次舍入）
    Mls,
    /// `VNMLA.F32` / 取反的乘加
    Nmla,
    /// `VNMLS.F32` / 取反的乘减
    Nmls,
    /// `VDIV.F32` / 浮点除
    Div,
    /// `VFMA.F32` / 融合乘加（单次舍入）
    Fma,
    /// `VFMS.F32` / 融合乘减（单次舍入）
    Fms,
    /// `VFNMA.F32` / 取反的融合乘加
    Fnma,
    /// `VFNMS.F32` / 取反的融合乘减
    Fnms,
    /// `VMOV.F32 Sd, Sm` / 浮点寄存器复制
    Mov,
    /// `VABS.F32` / 绝对值
    Abs,
    /// `VNEG.F32` / 取负
    Neg,
    /// `VSQRT.F32` / 平方根
    Sqrt,
    /// `VCMP.F32`（寄存器）/ 静默比较
    Cmp,
    /// `VCMPE.F32`（寄存器）/ 信号比较
    Cmpe,
    /// `VCMP.F32 Sd, #0.0` / 与零比较
    CmpZero,
    /// `VCMPE.F32 Sd, #0.0` / 与零的信号比较
    CmpeZero,
    /// `VCVT.F32.S32` / 有符号整数转浮点
    CvtF32S32,
    /// `VCVT.F32.U32` / 无符号整数转浮点
    CvtF32U32,
    /// `VCVT.S32.F32` / 浮点截断转有符号整数
    CvtS32F32,
    /// `VCVT.U32.F32` / 浮点截断转无符号整数
    CvtU32F32,
    /// `VCVTR.S32.F32` / 按 FPSCR 舍入模式转有符号整数
    CvtrS32F32,
    /// `VCVTR.U32.F32` / 按 FPSCR 舍入模式转无符号整数
    CvtrU32F32,
    /// `VRINTA.F32` / 取整，就近、远离零
    RoundAway,
    /// `VRINTN.F32` / 取整，就近、偶数优先
    RoundEven,
    /// `VRINTP.F32` / 取整，向正无穷
    RoundUp,
    /// `VRINTM.F32` / 取整，向负无穷
    RoundDown,
    /// `VRINTR.F32` / 按 FPSCR 舍入模式取整
    RoundFpscr,
    /// `VRINTX.F32` / 精确取整（就近偶数）
    RoundExact,
    /// `VRINTZ.F32` / 取整，向零
    RoundZero,
}

/// Fully decoded data-processing instruction / 完全解码的数据处理指令
#[derive(Debug, Clone, Copy)]
struct Decoded {
    /// Operation selected by the encoding / 编码选中的操作
    op: Op,
    /// First operand and destination / 第一操作数与目的寄存器
    vd: u8,
    /// Second operand (three-register forms) / 第二操作数（三寄存器形式）
    vn: u8,
    /// Third operand (three-register forms) / 第三操作数（三寄存器形式）
    vm: u8,
}

/// Conditional-select or round-to-integral encoding in the `0xFE` space
/// `0xFE` 空间中的条件选择或取整编码
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeOp {
    /// `VSELEQ`/`VSELVS`/`VSELGE`/`VSELGT` / 条件选择
    Select,
    /// `VRINTA`/`VRINTN`/`VRINTP`/`VRINTM` / 取整
    Round,
}

/// Try to decode and execute a single-precision data-processing instruction
/// 尝试解码并执行一条单精度数据处理指令
///
/// Returns `None` when the encoding lies outside the FPv5-SP single-precision
/// data-processing space, so the caller can continue decoding. Encodings
/// inside that space that this module does not implement (double precision,
/// half precision, 16-bit integer conversions, `VCVTA`/`VCVTN`/`VCVTP`/
/// `VCVTM`, `VCVT.F64.F32`) return `StepResult::Undefined` after the CPACR
/// check.
/// 编码不在 FPv5-SP 单精度数据处理空间时返回 `None`，由调用方继续解码。
/// 位于该空间但本模块未实现的编码（双精度、半精度、16 位整数转换、
/// `VCVTA`/`VCVTN`/`VCVTP`/`VCVTM`、`VCVT.F64.F32`）在完成 CPACR 检查后
/// 返回 `StepResult::Undefined`。
pub(super) fn try_execute(instr: u32, regs: &mut CortexM33Registers) -> Option<StepResult> {
    let top = instr >> 24;
    if top != 0xEE && top != 0xFE {
        return None;
    }
    let precision = (instr >> 8) & 0xF;
    if precision != 0xA && precision != 0xB {
        return None;
    }
    // `bit[4]` distinguishes data processing from the register-transfer and
    // system-register encodings handled by the data-movement module.
    // `bit[4]` 区分数据处理与数据搬移模块处理的寄存器传输/系统寄存器编码。
    if (instr >> 4) & 1 != 0 {
        return None;
    }
    // `VSEL` and the directed `VRINT` forms live in the `0xFE` space.
    // `VSEL` 与定向取整 `VRINT` 形式位于 `0xFE` 空间。
    let fe_op = if top == 0xFE {
        Some(classify_fe(instr)?)
    } else {
        None
    };

    // CPACR is checked before decoding so that a denied access always takes
    // the NOCP fault even for encodings this module treats as undefined.
    // 先检查 CPACR：即使编码被本模块视为未定义，拒绝访问也必须走 NOCP 故障。
    if !begin_fp_instruction(regs) {
        return Some(StepResult::Ok);
    }

    if precision == 0xB {
        // Double-precision arithmetic is storage-only in this emulator.
        // 本模拟器中双精度算术仅作存储，不做运算。
        return Some(StepResult::Undefined);
    }

    match fe_op {
        Some(FeOp::Select) => return Some(execute_select(instr, regs)),
        Some(FeOp::Round) => {
            let (vd, _, vm) = register_fields(instr);
            let op = match (instr >> 16) & 0xF {
                0x8 => Op::RoundAway,
                0x9 => Op::RoundEven,
                0xA => Op::RoundUp,
                _ => Op::RoundDown,
            };
            return Some(execute(Decoded { op, vd, vn: vd, vm }, regs));
        }
        None => {}
    }

    Some(match decode(instr) {
        Some(decoded) => execute(decoded, regs),
        None => StepResult::Undefined,
    })
}

/// Classify a `0xFE` data-processing encoding
/// 归类 `0xFE` 数据处理编码
///
/// `bit[23] = 0` selects `VSEL` (with `bit[7] = 1`, `bit[6] = 0`); `bit[23] = 1`
/// with `bits[23:21] = 0b101` and `bits[19:16]` in `0b1000..=0b1011` selects
/// `VRINTA`/`VRINTN`/`VRINTP`/`VRINTM` (with `bit[7] = 0`, `bit[6] = 1`).
/// `bit[23] = 0` 选择 `VSEL`（要求 `bit[7] = 1`、`bit[6] = 0`）；
/// `bit[23] = 1` 且 `bits[23:21] = 0b101`、`bits[19:16]` 位于
/// `0b1000..=0b1011` 时选择 `VRINTA`/`VRINTN`/`VRINTP`/`VRINTM`
/// （要求 `bit[7] = 0`、`bit[6] = 1`）。
fn classify_fe(instr: u32) -> Option<FeOp> {
    if (instr >> 23) & 1 == 0 {
        if (instr >> 6) & 1 != 0 || (instr >> 7) & 1 == 0 {
            return None;
        }
        return Some(FeOp::Select);
    }
    if (instr >> 20) & 0xB != 0xB {
        return None;
    }
    if !matches!((instr >> 16) & 0xF, 0x8..=0xB) {
        return None;
    }
    if (instr >> 6) & 1 == 0 || (instr >> 7) & 1 != 0 {
        return None;
    }
    Some(FeOp::Round)
}

/// Execute a `VSEL` conditional select using the FPSCR condition flags
/// 依据 FPSCR 条件标志执行一条 `VSEL` 条件选择
///
/// `bit[21:20]` selects the condition: `00` = `VSELEQ` (`Z == 1`),
/// `01` = `VSELVS` (`V == 1`), `10` = `VSELGE` (`N == V`) and
/// `11` = `VSELGT` (`Z == 0 && N == V`). The selected source is copied
/// verbatim, so NaN payloads survive, and no flags are modified.
/// `bit[21:20]` 选择条件：`00` = `VSELEQ`（`Z == 1`）、`01` = `VSELVS`
/// （`V == 1`）、`10` = `VSELGE`（`N == V`）、`11` = `VSELGT`
/// （`Z == 0 && N == V`）。被选中的源寄存器按位复制，因此 NaN 载荷保留，
/// 且不修改任何标志。
fn execute_select(instr: u32, regs: &mut CortexM33Registers) -> StepResult {
    let (vd, vn, vm) = register_fields(instr);

    let flags = regs.fpu.fpscr;
    let n = flags & fpscr::N != 0;
    let z = flags & fpscr::Z != 0;
    let v = flags & fpscr::V != 0;
    let use_vn = match (instr >> 20) & 0x3 {
        0 => z,
        1 => v,
        2 => n == v,
        _ => !z && n == v,
    };

    let source = if use_vn { vn } else { vm };
    let value = regs.fpu.read_s(source);
    regs.fpu.write_s(vd, value);
    StepResult::Ok
}

/// Decode the register fields and operation of a single-precision encoding
/// 解码单精度编码的寄存器字段与操作
///
/// The register fields follow the VFP convention: `bits[15:12]`/`bit[22]` hold
/// `Vd[4:1]`/`Vd[0]`, `bits[19:16]`/`bit[7]` hold `Vn[4:1]`/`Vn[0]` and
/// `bits[3:0]`/`bit[5]` hold `Vm[4:1]`/`Vm[0]`. `bits[23:21]` and `bit[20]`
/// are the major opcode (bit 22 is the `D` register bit), and `bit[6]` is the
/// sub-opcode that selects the negating/difference variant.
/// 寄存器字段遵循 VFP 约定：`bits[15:12]`/`bit[22]` 为 `Vd[4:1]`/`Vd[0]`，
/// `bits[19:16]`/`bit[7]` 为 `Vn[4:1]`/`Vn[0]`，`bits[3:0]`/`bit[5]` 为
/// `Vm[4:1]`/`Vm[0]`。`bits[23:21]` 与 `bit[20]` 为主操作码（bit 22 是
/// 寄存器 `D` 位），`bit[6]` 为选择取反/减法变体的子操作码。
fn decode(instr: u32) -> Option<Decoded> {
    let (vd, vn, vm) = register_fields(instr);

    let opcode = (instr >> 20) & 0xF;
    // The major opcode is formed from bits 23, 21 and 20; bit 22 is `D`.
    // 主操作码由位 23、21、20 组成；位 22 是 `D`。
    let major = ((opcode & 0x8) >> 1) | (opcode & 0x3);
    let sub = (instr >> 6) & 1;
    let high = (instr >> 7) & 1;

    let op = match major {
        0 => pick(sub, Op::Mla, Op::Mls),
        1 => pick(sub, Op::Nmls, Op::Nmla),
        2 => pick(sub, Op::Mul, Op::Nmul),
        3 => pick(sub, Op::Add, Op::Sub),
        4 if sub == 0 => Op::Div,
        5 => pick(sub, Op::Fnms, Op::Fnma),
        6 => pick(sub, Op::Fma, Op::Fms),
        7 => unary_op(instr, high)?,
        _ => return None,
    };

    Some(Decoded { op, vd, vn, vm })
}

/// Extract the `Vd`, `Vn` and `Vm` register indices from a data-processing encoding
/// 从数据处理编码中提取 `Vd`、`Vn`、`Vm` 寄存器索引
///
/// `bit[22]` supplies `Vd[0]`, `bit[7]` supplies `Vn[0]` and `bit[5]` supplies
/// `Vm[0]`; the remaining bits live in `bits[15:12]`, `bits[19:16]` and
/// `bits[3:0]` respectively.
/// `bit[22]` 提供 `Vd[0]`，`bit[7]` 提供 `Vn[0]`，`bit[5]` 提供 `Vm[0]`；
/// 其余位分别位于 `bits[15:12]`、`bits[19:16]` 与 `bits[3:0]`。
fn register_fields(instr: u32) -> (u8, u8, u8) {
    let d = (instr >> 22) & 1;
    let vd = ((((instr >> 12) & 0xF) << 1) | d) as u8;
    let vn = ((((instr >> 16) & 0xF) << 1) | ((instr >> 7) & 1)) as u8;
    let vm = (((instr & 0xF) << 1) | ((instr >> 5) & 1)) as u8;
    (vd, vn, vm)
}

/// Select between the plain and sub-opcode variant of a three-register op
/// 在三寄存器操作的普通变体与子操作码变体之间选择
fn pick(sub: u32, plain: Op, variant: Op) -> Op {
    if sub == 0 { plain } else { variant }
}

/// Decode the two-register/unary group (`bits[23:21] = 0b111`)
/// 解码两寄存器/一元指令组（`bits[23:21] = 0b111`）
///
/// `bits[19:16]` is the sub-opcode, `bit[7]` selects the second variant
/// (`VABS` vs `VMOV`, `VSQRT` vs `VNEG`, `VCMPE` vs `VCMP`, signed vs
/// unsigned conversion) and `bit[6]` must be set.
/// `bits[19:16]` 为子操作码，`bit[7]` 选择第二个变体（`VABS`/`VMOV`、
/// `VSQRT`/`VNEG`、`VCMPE`/`VCMP`、有符号/无符号转换），`bit[6]` 必须置位。
fn unary_op(instr: u32, high: u32) -> Option<Op> {
    if (instr >> 6) & 1 == 0 {
        return None;
    }
    let opc2 = (instr >> 16) & 0xF;
    let op = match opc2 {
        0 => pick(high, Op::Mov, Op::Abs),
        1 => pick(high, Op::Neg, Op::Sqrt),
        4 => pick(high, Op::Cmp, Op::Cmpe),
        5 => {
            // `VCMP{S}.F32 Sd, #0.0` fixes bits[5:0] to zero.
            // `VCMP{S}.F32 Sd, #0.0` 要求 bits[5:0] 为零。
            if instr & 0x3F != 0 {
                return None;
            }
            pick(high, Op::CmpZero, Op::CmpeZero)
        }
        6 => pick(high, Op::RoundFpscr, Op::RoundZero),
        7 => {
            // `bit[7] = 1` is `VCVT.F64.F32`, which is not implemented.
            // `bit[7] = 1` 为 `VCVT.F64.F32`，本模拟器未实现。
            if high != 0 {
                return None;
            }
            Op::RoundExact
        }
        8 => pick(high, Op::CvtF32U32, Op::CvtF32S32),
        0xC => pick(high, Op::CvtrU32F32, Op::CvtU32F32),
        0xD => pick(high, Op::CvtrS32F32, Op::CvtS32F32),
        _ => return None,
    };
    Some(op)
}

/// Execute a decoded instruction, always consuming the instruction
/// 执行已解码指令，始终消费该指令
fn execute(decoded: Decoded, regs: &mut CortexM33Registers) -> StepResult {
    let vd = decoded.vd;
    let x = single(regs, decoded.vn);
    let y = single(regs, decoded.vm);
    let z = single(regs, vd);

    match decoded.op {
        Op::Add => finish(regs, vd, x + y, &[x, y]),
        Op::Sub => finish(regs, vd, x - y, &[x, y]),
        Op::Mul => finish(regs, vd, x * y, &[x, y]),
        Op::Nmul => finish(regs, vd, -(x * y), &[x, y]),
        Op::Mla => finish(regs, vd, z + x * y, &[x, y, z]),
        Op::Mls => finish(regs, vd, z - x * y, &[x, y, z]),
        Op::Nmla => finish(regs, vd, -(z + x * y), &[x, y, z]),
        Op::Nmls => finish(regs, vd, -(z - x * y), &[x, y, z]),
        Op::Fma => finish(regs, vd, x.mul_add(y, z), &[x, y, z]),
        Op::Fms => finish(regs, vd, (-x).mul_add(y, z), &[x, y, z]),
        Op::Fnma => finish(regs, vd, (-x).mul_add(y, -z), &[x, y, z]),
        Op::Fnms => finish(regs, vd, x.mul_add(y, -z), &[x, y, z]),
        Op::Div => execute_divide(regs, vd, x, y),
        Op::Mov => regs.fpu.write_s(vd, regs.fpu.read_s(decoded.vm)),
        // Absolute value and negation are pure sign-bit operations; copying the
        // remaining bits keeps NaN payloads intact.
        // 绝对值与取负都是纯符号位操作；复制其余位可保留 NaN 载荷。
        Op::Abs => regs
            .fpu
            .write_s(vd, regs.fpu.read_s(decoded.vm) & 0x7FFF_FFFF),
        Op::Neg => regs
            .fpu
            .write_s(vd, regs.fpu.read_s(decoded.vm) ^ 0x8000_0000),
        Op::Sqrt => {
            if y < 0.0 {
                raise(regs, fpscr::IOC);
            }
            write_single(regs, vd, y.sqrt());
        }
        Op::Cmp | Op::Cmpe => compare(regs, z, y),
        Op::CmpZero | Op::CmpeZero => compare(regs, z, 0.0),
        Op::CvtF32S32 => convert_from_signed(regs, vd, y.to_bits()),
        Op::CvtF32U32 => convert_from_unsigned(regs, vd, y.to_bits()),
        Op::CvtS32F32 => {
            let value = to_signed(regs, y, y.trunc());
            write_single(regs, vd, f32::from_bits(value as u32));
        }
        Op::CvtU32F32 => {
            let value = to_unsigned(regs, y, y.trunc());
            write_single(regs, vd, f32::from_bits(value));
        }
        Op::CvtrS32F32 => {
            let value = to_signed(regs, y, y.round_ties_even());
            write_single(regs, vd, f32::from_bits(value as u32));
        }
        Op::CvtrU32F32 => {
            let value = to_unsigned(regs, y, y.round_ties_even());
            write_single(regs, vd, f32::from_bits(value));
        }
        // Round-to-integral keeps the result in a single-precision register.
        // `VRINTA` rounds ties away from zero, `VRINTN`/`VRINTR`/`VRINTX`
        // round ties to even (the only FPSCR rounding mode modeled), and
        // `VRINTP`/`VRINTM`/`VRINTZ` round toward the named direction.
        // 取整结果仍写入单精度寄存器。`VRINTA` 就近且远离零，`VRINTN`/
        // `VRINTR`/`VRINTX` 就近偶数（唯一建模的 FPSCR 舍入模式），
        // `VRINTP`/`VRINTM`/`VRINTZ` 朝指定方向取整。
        Op::RoundAway => write_single(regs, vd, y.round()),
        Op::RoundEven | Op::RoundFpscr | Op::RoundExact => {
            write_single(regs, vd, y.round_ties_even());
        }
        Op::RoundUp => write_single(regs, vd, y.ceil()),
        Op::RoundDown => write_single(regs, vd, y.floor()),
        Op::RoundZero => write_single(regs, vd, y.trunc()),
    }
    StepResult::Ok
}

/// Execute `VDIV`, including the divide-by-zero and invalid-operation flags
/// 执行 `VDIV`，包含除零与无效操作标志
fn execute_divide(regs: &mut CortexM33Registers, vd: u8, x: f32, y: f32) {
    let result = x / y;
    if y == 0.0 && x.is_finite() && x != 0.0 {
        raise(regs, fpscr::DZC);
    } else if result.is_nan() && !x.is_nan() && !y.is_nan() {
        raise(regs, fpscr::IOC);
    } else if result.is_infinite() {
        raise(regs, fpscr::OFC | fpscr::IXC);
    }
    write_single(regs, vd, result);
}

/// Store a result and update the cumulative exception flags
/// 写入结果并更新累积异常标志
fn finish(regs: &mut CortexM33Registers, vd: u8, result: f32, operands: &[f32]) {
    if result.is_nan() && !operands.iter().any(|value| value.is_nan()) {
        raise(regs, fpscr::IOC);
    }
    if result.is_infinite() && operands.iter().all(|value| value.is_finite()) {
        raise(regs, fpscr::OFC | fpscr::IXC);
    }
    write_single(regs, vd, result);
}

/// Compare two values and install the ARM `VCMP` condition flags
/// 比较两个值并写入 ARM `VCMP` 条件标志
///
/// ARM semantics: equal sets `Z`/`C`; less-than sets `N`; greater-than sets
/// `C`; unordered sets `C` and `V`. A NaN operand additionally sets `IOC`.
/// ARM 语义：相等置 `Z`/`C`；小于置 `N`；大于置 `C`；无序置 `C` 与 `V`。
/// 存在 NaN 操作数时还会置 `IOC`。
fn compare(regs: &mut CortexM33Registers, x: f32, y: f32) {
    let mut flags = regs.fpu.fpscr & !fpscr::CONDITION_FLAGS;
    let unordered = x.is_nan() || y.is_nan();
    if unordered {
        flags |= fpscr::C | fpscr::V;
    } else if x == y {
        flags |= fpscr::Z | fpscr::C;
    } else if x < y {
        flags |= fpscr::N;
    } else {
        flags |= fpscr::C;
    }
    regs.fpu.fpscr = flags;
    if unordered {
        raise(regs, fpscr::IOC);
    }
}

/// Convert a 32-bit signed integer to single precision
/// 将 32 位有符号整数转换为单精度浮点
fn convert_from_signed(regs: &mut CortexM33Registers, vd: u8, bits: u32) {
    let value = bits as i32;
    let result = value as f32;
    // `f64` represents every `i32` exactly, so this round-trip detects any
    // lost precision without being fooled by `f32 -> i32` saturation.
    // `f64` 可精确表示全部 `i32`，因此该往返比较能检测精度损失，且不受
    // `f32 -> i32` 饱和转换影响。
    if f64::from(result) != f64::from(value) {
        raise(regs, fpscr::IXC);
    }
    write_single(regs, vd, result);
}

/// Convert a 32-bit unsigned integer to single precision
/// 将 32 位无符号整数转换为单精度浮点
fn convert_from_unsigned(regs: &mut CortexM33Registers, vd: u8, bits: u32) {
    let result = bits as f32;
    if f64::from(result) != f64::from(bits) {
        raise(regs, fpscr::IXC);
    }
    write_single(regs, vd, result);
}

/// Convert to a signed 32-bit integer with ARM invalid/inexact behavior
/// 按 ARM 的无效/不精确行为转换为 32 位有符号整数
///
/// A NaN yields zero, and an out-of-range value yields the signed integer
/// indefinite value `0x8000_0000`; both set `IOC`. A lost fraction sets `IXC`.
/// NaN 返回零，越界返回有符号整数不确定值 `0x8000_0000`；两者都置 `IOC`。
/// 丢失小数部分时置 `IXC`。
fn to_signed(regs: &mut CortexM33Registers, source: f32, rounded: f32) -> i32 {
    if source.is_nan() {
        raise(regs, fpscr::IOC);
        return 0;
    }
    if !(-2_147_483_648.0..2_147_483_648.0).contains(&rounded) {
        raise(regs, fpscr::IOC);
        return i32::MIN;
    }
    if rounded != source {
        raise(regs, fpscr::IXC);
    }
    rounded as i32
}

/// Convert to an unsigned 32-bit integer with ARM invalid/inexact behavior
/// 按 ARM 的无效/不精确行为转换为 32 位无符号整数
///
/// A NaN yields zero, and an out-of-range value yields the unsigned integer
/// indefinite value zero; both set `IOC`. A lost fraction sets `IXC`.
/// NaN 返回零，越界返回无符号整数不确定值零；两者都置 `IOC`。
/// 丢失小数部分时置 `IXC`。
fn to_unsigned(regs: &mut CortexM33Registers, source: f32, rounded: f32) -> u32 {
    if source.is_nan() {
        raise(regs, fpscr::IOC);
        return 0;
    }
    if !(0.0..4_294_967_296.0).contains(&rounded) {
        raise(regs, fpscr::IOC);
        return 0;
    }
    if rounded != source {
        raise(regs, fpscr::IXC);
    }
    rounded as u32
}

/// Read one single-precision register as `f32`
/// 以 `f32` 读取一个单精度寄存器
fn single(regs: &CortexM33Registers, index: u8) -> f32 {
    f32::from_bits(regs.fpu.read_s(index))
}

/// Write one single-precision register from `f32`
/// 以 `f32` 写入一个单精度寄存器
fn write_single(regs: &mut CortexM33Registers, index: u8, value: f32) {
    regs.fpu.write_s(index, value.to_bits());
}

/// Set cumulative FPSCR exception flags / 置位 FPSCR 累积异常标志
fn raise(regs: &mut CortexM33Registers, flags: u32) {
    regs.fpu.fpscr |= flags;
}
