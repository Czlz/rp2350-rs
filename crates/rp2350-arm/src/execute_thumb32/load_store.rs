//! Thumb-32 load/store instructions
//! Thumb-32 加载/存储指令
//!
//! Covers register-offset and immediate-offset single data transfers
//! (byte/halfword/word and signed variants) plus dualword LDRD/STRD with
//! pre-indexed, post-indexed, and write-back addressing.
//! 覆盖寄存器偏移与立即数偏移的单字数据传输（字节/半字/字及有符号变体），
//! 以及支持前索引、后索引和写回寻址的双字 LDRD/STRD。

use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;

use crate::execute_thumb16::StepResult;
use crate::registers::CortexM33Registers;

use super::sign_extend;

/// Execute load/store single data item, `op1 == 0b11`, `op < 0x20`
/// 执行单字加载/存储，`op1 == 0b11`，`op < 0x20`
///
/// Register offset uses `address = Rn + (Rm << imm2)`; immediate forms use
/// bit 11 of the second halfword to choose between 8-bit and 12-bit offsets.
/// 寄存器偏移使用 `地址 = Rn + (Rm << imm2)`；立即数形式由第二半字位 11
/// 在 8 位与 12 位偏移之间选择。
pub(super) fn execute_single(
    instr: u32,
    regs: &mut CortexM33Registers,
    bus: &mut dyn AddressBus,
    cycles: &mut u64,
) -> Result<StepResult, CoreError> {
    let op = ((instr >> 20) & 0x7F) as u8;
    let rn = ((instr >> 16) & 0xF) as u8;
    let rt = ((instr >> 12) & 0xF) as u8;

    match op {
        // Register offset forms, distinguished by bits[23:20].
        // 寄存器偏移形式，由 bits[23:20] 区分。
        0x00 | 0x01 | 0x02 | 0x03 | 0x04 | 0x05 | 0x11 | 0x13 => {
            let imm2 = (instr >> 4) & 0x3;
            let rm = (instr & 0xF) as u8;
            let addr = regs.read(rn).wrapping_add(regs.read(rm) << imm2);
            match op {
                0x00 => bus.write_u8(addr, regs.read(rt) as u8, cycles)?,
                0x01 => {
                    let value = bus.read_u8(addr, cycles)?;
                    regs.write(rt, u32::from(value));
                }
                0x02 => bus.write_u16(addr, regs.read(rt) as u16, cycles)?,
                0x03 => {
                    let value = bus.read_u16(addr, cycles)?;
                    regs.write(rt, u32::from(value));
                }
                0x04 => bus.write_u32(addr, regs.read(rt), cycles)?,
                0x05 => {
                    let value = bus.read_u32(addr, cycles)?;
                    regs.write(rt, value);
                }
                0x11 => {
                    let value = bus.read_u8(addr, cycles)?;
                    regs.write(rt, sign_extend(u32::from(value), 8));
                }
                _ => {
                    let value = bus.read_u16(addr, cycles)?;
                    regs.write(rt, sign_extend(u32::from(value), 16));
                }
            }
            *cycles += 1;
            Ok(StepResult::Ok)
        }
        // Positive 8-bit immediate offset forms.
        // 正 8 位立即数偏移形式。
        0x08 | 0x09 | 0x0A | 0x0B | 0x19 | 0x1B => {
            let addr = regs.read(rn).wrapping_add(instr & 0xFF);
            match op {
                0x08 => bus.write_u8(addr, regs.read(rt) as u8, cycles)?,
                0x09 => {
                    let value = bus.read_u8(addr, cycles)?;
                    regs.write(rt, u32::from(value));
                }
                0x0A => bus.write_u16(addr, regs.read(rt) as u16, cycles)?,
                0x0B => {
                    let value = bus.read_u16(addr, cycles)?;
                    regs.write(rt, u32::from(value));
                }
                0x19 => {
                    let value = bus.read_u8(addr, cycles)?;
                    regs.write(rt, sign_extend(u32::from(value), 8));
                }
                _ => {
                    let value = bus.read_u16(addr, cycles)?;
                    regs.write(rt, sign_extend(u32::from(value), 16));
                }
            }
            *cycles += 1;
            Ok(StepResult::Ok)
        }
        0x0C => {
            let addr = regs.read(rn).wrapping_add(instr & 0xFFF);
            bus.write_u32(addr, regs.read(rt), cycles)?;
            *cycles += 1;
            Ok(StepResult::Ok)
        }
        0x0D => {
            let addr = regs.read(rn).wrapping_add(instr & 0xFFF);
            let value = bus.read_u32(addr, cycles)?;
            regs.write(rt, value);
            *cycles += 1;
            Ok(StepResult::Ok)
        }
        _ => Ok(StepResult::Undefined),
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

    // Post-indexed forms require W == 1.
    // 后索引形式要求 W == 1。
    if !pre_index && !write_back {
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
