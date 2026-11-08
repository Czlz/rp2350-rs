//! Thumb-32 load/store instructions
//! Thumb-32 加载/存储指令
//!
//! Covers register-offset, 12-bit-immediate and 8-bit-immediate pre/post-index
//! single data transfers (byte/halfword/word and signed variants), PC-relative
//! literal loads, plus dualword LDRD/STRD with pre-indexed, post-indexed and
//! write-back addressing.
//! 覆盖寄存器偏移、12 位立即数与 8 位立即数前/后索引的单字数据传输
//! （字节/半字/字及有符号变体）、PC 相对字面量加载，以及支持前索引、
//! 后索引和写回寻址的双字 LDRD/STRD。

use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;

use crate::execute_thumb16::StepResult;
use crate::registers::CortexM33Registers;

use super::multiple;
use super::sign_extend;

/// Access width of a single data transfer / 单次数据传输的访问宽度
#[derive(Clone, Copy)]
enum Width {
    /// Byte access / 字节访问
    Byte,
    /// Halfword access / 半字访问
    Half,
    /// Word access / 字访问
    Word,
}

/// Decoded single-transfer operation / 解码后的单次传输操作
#[derive(Clone, Copy)]
struct Transfer {
    /// Access width / 访问宽度
    width: Width,
    /// True for loads, false for stores / 加载为真，存储为假
    load: bool,
    /// True for sign-extending loads / 符号扩展加载为真
    signed: bool,
}

/// Map `op` (bits 26:20) to its transfer kind, distinguishing 8-bit and
/// 12-bit immediate encodings that share the same operation.
/// 将 `op`（位段 26:20）映射为传输类型，并合并共享同一操作的 8 位与
/// 12 位立即数编码。
fn transfer_for(op: u8) -> Option<Transfer> {
    let (width, load, signed) = match op {
        0x00 | 0x08 => (Width::Byte, false, false),
        0x01 | 0x09 => (Width::Byte, true, false),
        0x02 | 0x0A => (Width::Half, false, false),
        0x03 | 0x0B => (Width::Half, true, false),
        0x04 | 0x0C => (Width::Word, false, false),
        0x05 | 0x0D => (Width::Word, true, false),
        0x11 | 0x19 => (Width::Byte, true, true),
        0x13 | 0x1B => (Width::Half, true, true),
        _ => return None,
    };
    Some(Transfer {
        width,
        load,
        signed,
    })
}

/// Perform one transfer at `addr` and route the result to `Rt`
/// 在 `addr` 执行一次传输并把结果送达 `Rt`
///
/// Writes to `Rt == 15` on a load branch to the loaded address; stores to `PC`
/// are UNPREDICTABLE and decode as undefined.
/// 加载写入 `Rt == 15` 时跳转到加载地址；存储到 `PC` 属 UNPREDICTABLE，
/// 按未定义编码处理。
fn access(
    instr: u32,
    regs: &mut CortexM33Registers,
    bus: &mut dyn AddressBus,
    cycles: &mut u64,
    transfer: Transfer,
    addr: u32,
) -> Result<StepResult, CoreError> {
    let rt = ((instr >> 12) & 0xF) as u8;
    if transfer.load {
        let value = match transfer.width {
            Width::Byte => {
                let raw = bus.read_u8(addr, cycles)?;
                if transfer.signed {
                    sign_extend(u32::from(raw), 8)
                } else {
                    u32::from(raw)
                }
            }
            Width::Half => {
                let raw = bus.read_u16(addr, cycles)?;
                if transfer.signed {
                    sign_extend(u32::from(raw), 16)
                } else {
                    u32::from(raw)
                }
            }
            Width::Word => bus.read_u32(addr, cycles)?,
        };
        if rt == 15 {
            regs.pc = value & !1;
            Ok(StepResult::BranchTaken)
        } else {
            regs.write(rt, value);
            Ok(StepResult::Ok)
        }
    } else {
        if rt == 15 {
            return Ok(StepResult::Undefined);
        }
        match transfer.width {
            Width::Byte => bus.write_u8(addr, regs.read(rt) as u8, cycles)?,
            Width::Half => bus.write_u16(addr, regs.read(rt) as u16, cycles)?,
            Width::Word => bus.write_u32(addr, regs.read(rt), cycles)?,
        }
        Ok(StepResult::Ok)
    }
}

/// Execute load/store single data item, `op1 == 0b11`, `op < 0x20`
/// 执行单字加载/存储，`op1 == 0b11`，`op < 0x20`
///
/// Register offset uses `address = Rn + (Rm << imm2)`. Bit 11 of the second
/// halfword selects the 8-bit pre/post-index form (P/U/W/imm8) over the
/// register form; the 12-bit immediate operations use `imm12` and become
/// PC-relative literal loads when `Rn == 15`.
/// 寄存器偏移使用 `地址 = Rn + (Rm << imm2)`。第二半字位 11 在 8 位前/后
/// 索引形式（P/U/W/imm8）与寄存器形式之间选择；12 位立即数操作使用
/// `imm12`，当 `Rn == 15` 时转为 PC 相对字面量加载。
pub(super) fn execute_single(
    instr: u32,
    regs: &mut CortexM33Registers,
    bus: &mut dyn AddressBus,
    cycles: &mut u64,
) -> Result<StepResult, CoreError> {
    let op = ((instr >> 20) & 0x7F) as u8;
    let Some(transfer) = transfer_for(op) else {
        return Ok(StepResult::Undefined);
    };
    let rn = ((instr >> 16) & 0xF) as u8;
    let second = instr & 0xFFFF;

    // 12-bit immediate forms: STRB/LDRB/STRH/LDRH/STR/LDR and the signed
    // variants. `Rn == 15` encodes a PC-relative literal load.
    // 12 位立即数形式：STRB/LDRB/STRH/LDRH/STR/LDR 及有符号变体。
    // `Rn == 15` 表示 PC 相对字面量加载。
    if matches!(op, 0x08..=0x0D | 0x19 | 0x1B) {
        if rn == 15 && !transfer.load {
            return Ok(StepResult::Undefined);
        }
        let imm12 = second & 0xFFF;
        let addr = if rn == 15 {
            (regs.pc & !3).wrapping_add(imm12)
        } else {
            regs.read(rn).wrapping_add(imm12)
        };
        let result = access(instr, regs, bus, cycles, transfer, addr)?;
        *cycles += 1;
        return Ok(result);
    }

    // 8-bit immediate pre/post-indexed forms, selected by second-halfword
    // bit 11: `P` bit 10, `U` bit 9, `W` bit 8 and `imm8` in bits 7:0.
    // 由第二半字位 11 选择的 8 位立即数前/后索引形式：
    // `P` 位 10、`U` 位 9、`W` 位 8，`imm8` 在位段 7:0。
    if second & 0x800 != 0 {
        let pre_index = (second >> 10) & 1 != 0;
        let add_offset = (second >> 9) & 1 != 0;
        let write_back = (second >> 8) & 1 != 0;
        // Post-indexed forms require W == 1, and Rn == PC is not a valid base.
        // 后索引形式要求 W == 1，且 Rn == PC 不是合法基址。
        if (!pre_index && !write_back) || rn == 15 {
            return Ok(StepResult::Undefined);
        }
        let offset = second & 0xFF;
        let delta = if add_offset {
            offset
        } else {
            offset.wrapping_neg()
        };
        let base = regs.read(rn);
        let addr = if pre_index {
            base.wrapping_add(delta)
        } else {
            base
        };
        let result = access(instr, regs, bus, cycles, transfer, addr)?;
        if write_back {
            regs.write(rn, base.wrapping_add(delta));
        }
        *cycles += 1;
        return Ok(result);
    }

    // PC-relative literal loads with a negative offset (`U == 0`).
    // PC 相对字面量加载的负偏移形式（`U == 0`）。
    if rn == 15 {
        if !transfer.load {
            return Ok(StepResult::Undefined);
        }
        let imm12 = second & 0xFFF;
        let addr = (regs.pc & !3).wrapping_sub(imm12);
        let result = access(instr, regs, bus, cycles, transfer, addr)?;
        *cycles += 1;
        return Ok(result);
    }

    // Register offset: bits 11:6 must be zero, bits 5:4 are `imm2`, bits 3:0
    // are `Rm`.
    // 寄存器偏移：位段 11:6 必须为零，位段 5:4 为 `imm2`，位段 3:0 为 `Rm`。
    if second & 0x0FC0 != 0 {
        return Ok(StepResult::Undefined);
    }
    let imm2 = (instr >> 4) & 0x3;
    let rm = (instr & 0xF) as u8;
    let addr = regs.read(rn).wrapping_add(regs.read(rm) << imm2);
    let result = access(instr, regs, bus, cycles, transfer, addr)?;
    *cycles += 1;
    Ok(result)
}

/// Dispatch the `op1 == 0b01`, `op2 == 0b00` group by the ARM ARM bit fields
/// 按 ARM ARM 位域分派 `op1 == 0b01`、`op2 == 0b00` 组
///
/// Table branch requires second halfword `0xF0xx`; exclusive transfers have
/// `U == 0`, `P == 0` and `W == 0` with bit 22 set; the remaining
/// bit-22-set encodings are LDRD/STRD; `P != U` selects LDM/STM.
/// 表分支要求第二半字为 `0xF0xx`；独占传输满足 `U == 0`、`P == 0`、`W == 0`
/// 且位 22 置位；其余位 22 置位的编码为 LDRD/STRD；`P != U` 选择 LDM/STM。
pub(super) fn execute_group(
    instr: u32,
    regs: &mut CortexM33Registers,
    bus: &mut dyn AddressBus,
    cycles: &mut u64,
) -> Result<StepResult, CoreError> {
    let second = instr & 0xFFFF;
    // Table branch is `op == 0x0D` (bit 23 set) with second halfword `0xF0xx`.
    // The bit-23 test keeps `TT r0, r0` (`0xE840 0xF000`) out of this path,
    // because the ARMv8-M test-target encoding also ends in `0xF000`.
    // 表分支为 `op == 0x0D`（位 23 置位）且第二半字为 `0xF0xx`。位 23 判断
    // 可把 `TT r0, r0`（`0xE840 0xF000`）排除在外，因为 ARMv8-M 的
    // Test Target 编码也以 `0xF000` 结尾。
    if second & 0xFFE0 == 0xF000 && (instr >> 23) & 1 != 0 {
        return multiple::execute_table_branch(instr, regs, bus, cycles);
    }

    let pre_index = (instr >> 24) & 1 != 0;
    let add_offset = (instr >> 23) & 1 != 0;
    let write_back = (instr >> 21) & 1 != 0;
    if (instr >> 22) & 1 != 0 {
        if !pre_index && !add_offset && !write_back {
            // ARMv8-M `TT` shares this encoding space with STREX; `Rt2 == 15`
            // (bits 15:12 of the second halfword) identifies it, since a
            // STREX with `Rt2 == 15` is UNPREDICTABLE.
            // ARMv8-M 的 `TT` 与 STREX 共享此编码空间；第二半字位段 15:12
            // 的 `Rt2 == 15` 可识别它，因为 `Rt2 == 15` 的 STREX 属
            // UNPREDICTABLE。
            if second & 0xF000 == 0xF000 && second.trailing_zeros() >= 6 && (instr >> 20) & 1 == 0 {
                Ok(multiple::execute_test_target(instr, regs, cycles))
            } else {
                multiple::execute_exclusive(instr, regs, bus, cycles)
            }
        } else {
            execute_dual(instr, regs, bus, cycles)
        }
    } else if add_offset != pre_index {
        multiple::execute_register_list(instr, regs, bus, cycles)
    } else {
        Ok(StepResult::Undefined)
    }
}

/// Execute LDRD/STRD, `op1 == 0b01`, `op == 0x1C` (STRD) or `op == 0x1D` (LDRD)
/// 执行 LDRD/STRD，`op1 == 0b01`，`op == 0x1C`（STRD）或 `op == 0x1D`（LDRD）
///
/// Addressing fields: P (bit 24), U (bit 23), W (bit 21), L (bit 20).
/// 寻址字段：P（位 24）、U（位 23）、W（位 21）、L（位 20）。
pub(super) fn execute_dual(
    instr: u32,
    regs: &mut CortexM33Registers,
    bus: &mut dyn AddressBus,
    cycles: &mut u64,
) -> Result<StepResult, CoreError> {
    let pre_index = (instr >> 24) & 1 != 0;
    let add_offset = (instr >> 23) & 1 != 0;
    let write_back = (instr >> 21) & 1 != 0;
    let load = (instr >> 20) & 1 != 0;
    let rn = ((instr >> 16) & 0xF) as u8;
    let rt = ((instr >> 12) & 0xF) as u8;
    let rt2 = ((instr >> 8) & 0xF) as u8;
    let offset = (instr & 0xFF) << 2;

    // Post-indexed forms require W == 1, and PC cannot be a transfer register.
    // 后索引形式要求 W == 1，且 PC 不能作为传输寄存器。
    if (!pre_index && !write_back) || rt == 15 || rt2 == 15 {
        return Ok(StepResult::Undefined);
    }

    let delta = if add_offset {
        offset
    } else {
        offset.wrapping_neg()
    };
    let base = regs.read(rn);
    let addr = if pre_index {
        base.wrapping_add(delta)
    } else {
        base
    };

    if load {
        let first = bus.read_u32(addr, cycles)?;
        let second = bus.read_u32(addr.wrapping_add(4), cycles)?;
        regs.write(rt, first);
        regs.write(rt2, second);
    } else {
        bus.write_u32(addr, regs.read(rt), cycles)?;
        bus.write_u32(addr.wrapping_add(4), regs.read(rt2), cycles)?;
    }

    if !pre_index || write_back {
        regs.write(rn, base.wrapping_add(delta));
    }

    *cycles += 2;
    Ok(StepResult::Ok)
}
