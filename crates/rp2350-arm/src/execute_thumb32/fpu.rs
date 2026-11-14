//! Floating-point data movement and dispatch (Stage 2 + Stage 3)
//! 浮点数据搬移与分派（Stage 2 + Stage 3）
//!
//! Implements `VLDR`/`VSTR`, `VLDM`/`VSTM`, `VMOV`, `VMRS` and `VMSR` for
//! the FPv5-SP unit and dispatches the single-precision data-processing
//! encodings to [`super::fpu_arith`]. Double-precision arithmetic is not
//! implemented; the `D0-D15` view exists only for block transfers and 64-bit
//! core transfers.
//! 实现 FPv5-SP 单元的 `VLDR`/`VSTR`、`VLDM`/`VSTM`、`VMOV`、`VMRS` 与
//! `VMSR`，并把单精度数据处理编码分派给 [`super::fpu_arith`]。不实现双精度
//! 算术；`D0-D15` 视图仅用于块传输和 64 位内核传输。
//!
//! ## Not implemented / 未实现清单
//!
//! - `VCVT` between single and double precision (reported as undefined)
//!   单精度与双精度之间的 `VCVT`（按未定义指令上报）
//! - Double-precision arithmetic (`VADD.F64` …) — the D view is storage only
//!   双精度算术（`VADD.F64` 等）—— D 视图仅作存储
//! - Half-precision `VCVTB`/`VCVTT`, 16-bit integer `VCVT` and `VRINT*`
//!   半精度 `VCVTB`/`VCVTT`、16 位整数 `VCVT` 与 `VRINT*`
//! - Lazy FP context stacking (`LSPACT`/`FPCAR`/`FPCCR` are not modeled; an
//!   exception taken with live FP state logs an explicit unsupported warning)
//!   惰性浮点上下文压栈（不建模 `LSPACT`/`FPCAR`/`FPCCR`；浮点活跃时进入
//!   异常会显式告警未支持）

// 位域解码与寄存器索引转换是 ARM 仿真固有特征。
// Bitfield decoding and register index conversions are inherent to ARM emulation.
#![allow(clippy::cast_possible_truncation)]

use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;

use crate::execute_thumb16::StepResult;
use crate::fpu::{fpexc, fpscr};
use crate::registers::CortexM33Registers;

/// Try to decode and execute a Stage 2 FPU data-movement instruction
/// 尝试解码并执行一条 Stage 2 浮点数据搬移指令
///
/// Returns `Ok(None)` when the encoding is not handled here, letting the
/// normal Thumb-32 decode tree continue.
/// 编码不属于本模块时返回 `Ok(None)`，由常规 Thumb-32 解码树继续处理。
pub(super) fn try_execute(
    instr: u32,
    regs: &mut CortexM33Registers,
    bus: &mut dyn AddressBus,
    cycles: &mut u64,
) -> Result<Option<StepResult>, CoreError> {
    // Transfers are classified first: `VMOV Dm, Rt, Rt2` shares the `0xEC`
    // extension-space prefix with `VLDM`/`VSTM` but uses `bits[23:21] = 010`,
    // an addressing mode that is illegal for block transfers.
    // 先归类传输指令：`VMOV Dm, Rt, Rt2` 与 `VLDM`/`VSTM` 共用 `0xEC`
    // 扩展空间前缀，但使用 `bits[23:21] = 010`，该寻址模式对块传输非法。
    if let Some(kind) = transfer_kind(instr) {
        if !begin_fp_instruction(regs) {
            return Ok(Some(StepResult::Ok));
        }
        return Ok(Some(execute_transfer(instr, kind, regs)));
    }

    // Floating-point arithmetic, comparison, conversion and conditional
    // select (Stage 3) occupy the `0xEE` data-processing and `0xFE` `VSEL`
    // encoding spaces, which are disjoint from the extension-register
    // load/store encodings below.
    // 浮点算术、比较、转换与条件选择（Stage 3）位于 `0xEE` 数据处理空间与
    // `0xFE` 的 `VSEL` 空间，与下方的扩展寄存器加载/存储编码互不重叠。
    if let Some(result) = super::fpu_arith::try_execute(instr, regs) {
        return Ok(Some(result));
    }

    if is_extension_load_store(instr) {
        if !begin_fp_instruction(regs) {
            return Ok(Some(StepResult::Ok));
        }
        return Ok(Some(execute_memory(instr, regs, bus, cycles)?));
    }

    Ok(None)
}

/// Extension-register load/store encoding space (`0xEC`/`0xED` group)
/// 扩展寄存器加载/存储编码空间（`0xEC`/`0xED` 组）
///
/// `VLDR`/`VSTR`/`VLDM`/`VSTM` share `bits[31:25] = 1110110` and select
/// coprocessor 10 or 11. The coprocessor check matters because `MCRR`/`MRRC`
/// for CP0/CP4 use the same `0xEC` prefix and must fall through to the
/// coprocessor decoder.
/// `VLDR`/`VSTR`/`VLDM`/`VSTM` 共用 `bits[31:25] = 1110110` 并选择协处理器
/// 10 或 11。必须检查协处理器号，因为 CP0/CP4 的 `MCRR`/`MRRC` 使用相同的
/// `0xEC` 前缀，需要继续交给协处理器解码器。
fn is_extension_load_store(instr: u32) -> bool {
    (instr >> 25) & 0x7F == 0b111_0110 && matches!((instr >> 8) & 0xF, 0xA | 0xB)
}

/// Check CPACR access, or raise the synchronous NOCP UsageFault
/// 检查 CPACR 权限，或触发同步 NOCP 用法故障
///
/// Returns `true` when the instruction may execute. On denial the PC is
/// rewound to the faulting instruction so the stacked frame is correct, and a
/// pending UsageFault is recorded for the core to enter immediately.
/// 允许执行时返回 `true`。被拒绝时把 PC 回退到故障指令以保证压栈帧正确，
/// 并记录待处理的用法故障，由核心立即进入异常。
pub(super) fn begin_fp_instruction(regs: &mut CortexM33Registers) -> bool {
    if regs.fp_access_enabled() {
        regs.fpu.fpexc |= fpexc::EN;
        return true;
    }
    regs.pc = regs.pc.wrapping_sub(4);
    regs.raise_usage_fault();
    false
}

/// Transfer instruction selector / 传输指令分类
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransferKind {
    /// `VMOV Sn, Rt` / 内核寄存器到单精度寄存器
    CoreToSingle,
    /// `VMOV Rt, Sn` / 单精度寄存器到内核寄存器
    SingleToCore,
    /// `VMOV Dm, Rt, Rt2` / 两个内核寄存器到双字寄存器
    CoreToDouble,
    /// `VMOV Rt, Rt2, Dm` / 双字寄存器到两个内核寄存器
    DoubleToCore,
    /// `VMOV.F32 Sd, #imm` / 单精度立即数
    Immediate,
    /// `VMRS Rt, FPSCR` / 读浮点状态寄存器
    Vmrs,
    /// `VMSR FPSCR, Rt` / 写浮点状态寄存器
    Vmsr,
}

/// Classify a `0xEE`/`0xEC` transfer encoding / 归类 `0xEE`/`0xEC` 传输编码
fn transfer_kind(instr: u32) -> Option<TransferKind> {
    match instr >> 24 {
        0xEE => {
            // VMOV Sn, Rt / VMOV Rt, Sn: bits[23:21] = 000, coproc key = 1010,
            // bits[6:4] = 001, low nibble = 0.
            // VMOV Sn, Rt / VMOV Rt, Sn：bits[23:21] = 000，协处理器键 = 1010，
            // bits[6:4] = 001，低半字节 = 0。
            if (instr & 0x00E0_0000) == 0 && (instr & 0x0000_0F7F) == 0x0000_0A10 {
                return Some(if instr & 0x0010_0000 != 0 {
                    TransferKind::SingleToCore
                } else {
                    TransferKind::CoreToSingle
                });
            }
            // VMRS/VMSR use sysm = 0b0001 (FPSCR); low nibble is fixed zero.
            // VMRS/VMSR 使用 sysm = 0b0001（FPSCR）；低半字节固定为零。
            if instr & 0xFFFF_0FFF == 0xEEF1_0A10 {
                return Some(TransferKind::Vmrs);
            }
            if instr & 0xFFFF_0FFF == 0xEEE1_0A10 {
                return Some(TransferKind::Vmsr);
            }
            // VMOV.F32 immediate: bits[23:20] = 1D11, coproc = 1010, bits[7:4] = 0.
            // VMOV.F32 立即数：bits[23:20] = 1D11，协处理器 = 1010，bits[7:4] = 0。
            if instr & 0x00B0_0000 == 0x00B0_0000 && instr & 0x0000_0FF0 == 0x0000_0A00 {
                return Some(TransferKind::Immediate);
            }
        }
        0xEC if instr & 0x00E0_0000 == 0x0040_0000 && instr & 0x0000_0FF0 == 0x0000_0B10 => {
            // VMOV Dm, Rt, Rt2 / VMOV Rt, Rt2, Dm: bits[23:21] = 010,
            // coproc key = 1011, bits[7:4] = 0001.
            // VMOV Dm, Rt, Rt2 / VMOV Rt, Rt2, Dm：bits[23:21] = 010，
            // 协处理器键 = 1011，bits[7:4] = 0001。
            return Some(if instr & 0x0010_0000 != 0 {
                TransferKind::DoubleToCore
            } else {
                TransferKind::CoreToDouble
            });
        }
        _ => {}
    }
    None
}

/// Execute a non-memory transfer instruction / 执行非内存传输指令
fn execute_transfer(instr: u32, kind: TransferKind, regs: &mut CortexM33Registers) -> StepResult {
    let rt = ((instr >> 12) & 0xF) as u8;
    let vd_or_vn = (instr >> 16) & 0xF;

    match kind {
        TransferKind::Vmrs => {
            let sysm = vd_or_vn;
            if rt == 15 {
                if sysm != 1 {
                    return StepResult::Undefined;
                }
                let flags = regs.fpu.fpscr;
                regs.set_apsr_nzcv(
                    flags & fpscr::N != 0,
                    flags & fpscr::Z != 0,
                    flags & fpscr::C != 0,
                    flags & fpscr::V != 0,
                );
                return StepResult::Ok;
            }
            match sysm {
                1 => regs.write(rt, regs.fpu.fpscr),
                8 => regs.write(rt, regs.fpu.fpexc),
                _ => return StepResult::Undefined,
            }
        }
        TransferKind::Vmsr => {
            if rt == 15 || vd_or_vn != 1 {
                return StepResult::Undefined;
            }
            regs.fpu.fpscr = regs.read(rt);
        }
        TransferKind::Immediate => {
            // The register field is Vd (bits[15:12]) plus the D bit (bit 22);
            // imm4H lives in bits[19:16] and is not part of the register index.
            // 寄存器字段是 Vd（bits[15:12]）加 D 位（bit 22）；imm4H 位于
            // bits[19:16]，不属于寄存器索引。
            let d = (instr >> 22) & 1;
            let sd = (((instr >> 12) & 0xF) << 1) | d;
            let imm8 = vd_or_vn << 4 | instr & 0xF;
            regs.fpu.write_s(sd as u8, expand_vfp_immediate(imm8));
        }
        TransferKind::CoreToSingle => {
            if rt == 15 {
                return StepResult::Undefined;
            }
            let sn = (vd_or_vn << 1) | (instr >> 7) & 1;
            regs.fpu.write_s(sn as u8, regs.read(rt));
        }
        TransferKind::SingleToCore => {
            if rt == 15 {
                return StepResult::Undefined;
            }
            let sn = (vd_or_vn << 1) | (instr >> 7) & 1;
            regs.write(rt, regs.fpu.read_s(sn as u8));
        }
        TransferKind::CoreToDouble => {
            let rt2 = vd_or_vn as u8;
            if rt == 15 || rt2 == 15 {
                return StepResult::Undefined;
            }
            let low = u64::from(regs.read(rt));
            let high = u64::from(regs.read(rt2));
            let dm = (instr & 0xF) as u8;
            regs.fpu.write_d(dm, low | high << 32);
        }
        TransferKind::DoubleToCore => {
            let rt2 = vd_or_vn as u8;
            if rt == 15 || rt2 == 15 {
                return StepResult::Undefined;
            }
            let dm = (instr & 0xF) as u8;
            let value = regs.fpu.read_d(dm);
            regs.write(rt, value as u32);
            regs.write(rt2, (value >> 32) as u32);
        }
    }
    StepResult::Ok
}

/// Expand an 8-bit VFP immediate to a single-precision bit pattern
/// 将 8 位 VFP 立即数扩展为单精度位模式
///
/// `imm8 = abcdefgh`: sign = a, exponent = NOT(b):b:b:b:b:b:c:d,
/// fraction = efgh followed by 19 zeros. This matches the GNU assembler's
/// encodings for `VMOV.F32 Sd, #imm`.
/// `imm8 = abcdefgh`：符号位 = a，指数 = NOT(b):b:b:b:b:b:c:d，
/// 尾数 = efgh 后接 19 个零。与 GNU 汇编器对 `VMOV.F32 Sd, #imm` 的编码一致。
fn expand_vfp_immediate(imm8: u32) -> u32 {
    let sign = (imm8 >> 7) & 1;
    let b = (imm8 >> 6) & 1;
    let c = (imm8 >> 5) & 1;
    let d = (imm8 >> 4) & 1;
    let low = imm8 & 0xF;
    let exponent =
        ((b ^ 1) << 7) | (b << 6) | (b << 5) | (b << 4) | (b << 3) | (b << 2) | (c << 1) | d;
    (sign << 31) | (exponent << 23) | (low << 19)
}

/// Execute a `VLDR`/`VSTR`/`VLDM`/`VSTM` encoding
/// 执行 `VLDR`/`VSTR`/`VLDM`/`VSTM` 编码
fn execute_memory(
    instr: u32,
    regs: &mut CortexM33Registers,
    bus: &mut dyn AddressBus,
    cycles: &mut u64,
) -> Result<StepResult, CoreError> {
    let cp = (instr >> 8) & 0xF;
    if cp != 0xA && cp != 0xB {
        return Ok(StepResult::Undefined);
    }
    let double_precision = cp == 0xB;
    let p = (instr >> 24) & 1;
    let u = (instr >> 23) & 1;
    let d = (instr >> 22) & 1;
    let w = (instr >> 21) & 1;
    let load = (instr >> 20) & 1 == 1;
    let rn = ((instr >> 16) & 0xF) as u8;
    let vd = (instr >> 12) & 0xF;
    let imm8 = instr & 0xFF;

    if p == 1 && w == 0 {
        // Single-register VLDR/VSTR / 单寄存器 VLDR/VSTR
        // imm8 is the offset in words for both precisions.
        // 两种精度下 imm8 都是以字为单位的偏移。
        if rn == 15 && !load {
            return Ok(StepResult::Undefined);
        }
        let base = if rn == 15 {
            regs.pc & !3
        } else {
            regs.read(rn)
        };
        let offset = imm8.wrapping_mul(4);
        let addr = if u == 1 {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };
        if double_precision {
            if d == 1 {
                return Ok(StepResult::Undefined); // D16-D31 do not exist
            }
            transfer_double(bus, regs, vd as u8, addr, load, cycles)?;
        } else {
            let sd = ((vd << 1) | d) as u8;
            transfer_single(bus, regs, sd, addr, load, cycles)?;
        }
        return Ok(StepResult::Ok);
    }

    // VLDM/VSTM: only IA and DB addressing modes exist for the VFP unit.
    // VLDM/VSTM：VFP 单元只存在 IA 与 DB 寻址模式。
    if p == 0 && u == 0 {
        return Ok(StepResult::Undefined);
    }
    if p == 1 && u == 1 {
        return Ok(StepResult::Undefined);
    }
    if rn == 15 {
        return Ok(StepResult::Undefined);
    }
    if imm8 == 0 {
        return Ok(StepResult::Undefined);
    }

    let (first, count) = if double_precision {
        if d == 1 || imm8 % 2 != 0 {
            return Ok(StepResult::Undefined);
        }
        let count = imm8 / 2;
        if vd + count > 16 {
            return Ok(StepResult::Undefined);
        }
        (vd, count)
    } else {
        let first = (vd << 1) | d;
        if first + imm8 > 32 {
            return Ok(StepResult::Undefined);
        }
        (first, imm8)
    };

    let size = if double_precision { 8 } else { 4 };
    let total = count * size;
    let base = regs.read(rn);
    let addr = if p == 1 {
        if u == 1 {
            base.wrapping_add(total)
        } else {
            base.wrapping_sub(total)
        }
    } else {
        base
    };
    let writeback = if p == 1 {
        addr
    } else if u == 1 {
        base.wrapping_add(total)
    } else {
        base.wrapping_sub(total)
    };

    for index in 0..count {
        let register = first + index;
        let address = addr.wrapping_add(index * size);
        if double_precision {
            transfer_double(bus, regs, register as u8, address, load, cycles)?;
        } else {
            transfer_single(bus, regs, register as u8, address, load, cycles)?;
        }
    }

    if w == 1 {
        regs.write(rn, writeback);
    }
    Ok(StepResult::Ok)
}

/// Load or store one 32-bit single-precision register / 加载或存储一个单精度寄存器
fn transfer_single(
    bus: &mut dyn AddressBus,
    regs: &mut CortexM33Registers,
    index: u8,
    addr: u32,
    load: bool,
    cycles: &mut u64,
) -> Result<(), CoreError> {
    if load {
        let value = bus.read_u32(addr, cycles)?;
        regs.fpu.write_s(index, value);
    } else {
        let value = regs.fpu.read_s(index);
        bus.write_u32(addr, value, cycles)?;
    }
    Ok(())
}

/// Load or store one 64-bit doubleword view as two little-endian words
/// 以一个双字视图加载或存储两个小端字
fn transfer_double(
    bus: &mut dyn AddressBus,
    regs: &mut CortexM33Registers,
    index: u8,
    addr: u32,
    load: bool,
    cycles: &mut u64,
) -> Result<(), CoreError> {
    if load {
        let low = bus.read_u32(addr, cycles)?;
        let high = bus.read_u32(addr.wrapping_add(4), cycles)?;
        regs.fpu
            .write_d(index, u64::from(low) | (u64::from(high) << 32));
    } else {
        let value = regs.fpu.read_d(index);
        bus.write_u32(addr, value as u32, cycles)?;
        bus.write_u32(addr.wrapping_add(4), (value >> 32) as u32, cycles)?;
    }
    Ok(())
}
