//! Thumb-32 load/store multiple, exclusive and table branch instructions
//! Thumb-32 多寄存器加载/存储、独占访问与表分支指令
//!
//! Covers LDM/STM (including the `.W` write-back forms used by push/pop),
//! LDREX/STREX and TBB/TBH within the `op1 == 0b01` encoding group.
//! 覆盖 `op1 == 0b01` 编码组内的 LDM/STM（含 push/pop 使用的 `.W` 写回形式）、
//! LDREX/STREX 与 TBB/TBH。

use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;

use crate::execute_thumb16::StepResult;
use crate::registers::CortexM33Registers;

/// Execute LDM/STM, `op1 == 0b01`, `op2 == 0b00`, bit 22 clear
/// 执行 LDM/STM，`op1 == 0b01`，`op2 == 0b00`，位 22 清零
///
/// Addressing mode comes from P (bit 24) and U (bit 23): `P == 0, U == 1`
/// increments after the base; `P == 1, U == 0` decrements before it. `W`
/// (bit 21) enables base write-back and `L` (bit 20) selects load or store.
/// 寻址模式由 P（位 24）与 U（位 23）给出：`P == 0, U == 1` 为基址后递增；
/// `P == 1, U == 0` 为基址前递减。`W`（位 21）启用基址写回，`L`（位 20）
/// 选择加载或存储。
pub(super) fn execute_register_list(
    instr: u32,
    regs: &mut CortexM33Registers,
    bus: &mut dyn AddressBus,
    cycles: &mut u64,
) -> Result<StepResult, CoreError> {
    let list = instr & 0xFFFF;
    if list == 0 {
        return Ok(StepResult::Undefined);
    }
    let decrement_before = (instr >> 24) & 1 != 0;
    let add_offset = (instr >> 23) & 1 != 0;
    // Only the two architecturally defined mode combinations are valid.
    // 仅两种架构定义的模式组合合法。
    if add_offset == decrement_before {
        return Ok(StepResult::Undefined);
    }
    let write_back = (instr >> 21) & 1 != 0;
    let load = (instr >> 20) & 1 != 0;
    // Storing PC is UNPREDICTABLE.
    // 存储 PC 属 UNPREDICTABLE。
    if !load && (list & 0x8000) != 0 {
        return Ok(StepResult::Undefined);
    }

    let rn = ((instr >> 16) & 0xF) as u8;
    let base = regs.read(rn);
    let count = list.count_ones();
    let start = if decrement_before {
        base.wrapping_sub(count * 4)
    } else {
        base
    };

    let mut addr = start;
    let mut loaded_pc = None;
    for index in 0..16u8 {
        if list & (1 << index) == 0 {
            continue;
        }
        if load {
            let value = bus.read_u32(addr, cycles)?;
            if index == 15 {
                loaded_pc = Some(value);
            } else {
                regs.write(index, value);
            }
        } else {
            let value = regs.read(index);
            bus.write_u32(addr, value, cycles)?;
        }
        addr = addr.wrapping_add(4);
    }

    if write_back {
        let updated = if decrement_before {
            start
        } else {
            base.wrapping_add(count * 4)
        };
        regs.write(rn, updated);
    }

    *cycles += u64::from(count);

    if let Some(value) = loaded_pc {
        regs.pc = value & !1;
        return Ok(StepResult::BranchTaken);
    }
    Ok(StepResult::Ok)
}

/// Execute LDREX/STREX, `op1 == 0b01`, `op2 == 0b00`, bit 22 set, bit 23 clear
/// 执行 LDREX/STREX，`op1 == 0b01`，`op2 == 0b00`，位 22 置位、位 23 清零
///
/// `imm8` is scaled by four. LDREX loads into `Rt` (bits 15:12); STREX stores
/// `Rt` and writes the success status 0 into `Rd` (bits 11:8), with `Rd == 15`
/// discarding the status.
/// `imm8` 按四倍缩放。LDREX 加载到 `Rt`（位段 15:12）；STREX 存储 `Rt`
/// 并把成功状态 0 写入 `Rd`（位段 11:8），`Rd == 15` 表示丢弃状态。
pub(super) fn execute_exclusive(
    instr: u32,
    regs: &mut CortexM33Registers,
    bus: &mut dyn AddressBus,
    cycles: &mut u64,
) -> Result<StepResult, CoreError> {
    let load = (instr >> 20) & 1 != 0;
    let rn = ((instr >> 16) & 0xF) as u8;
    let rt = ((instr >> 12) & 0xF) as u8;
    let offset = (instr & 0xFF) << 2;
    let addr = regs.read(rn).wrapping_add(offset);

    if load {
        let value = bus.read_u32(addr, cycles)?;
        regs.write(rt, value);
    } else {
        let rd = ((instr >> 8) & 0xF) as u8;
        let value = regs.read(rt);
        bus.write_u32(addr, value, cycles)?;
        if rd != 15 {
            regs.write(rd, 0);
        }
    }

    *cycles += 1;
    Ok(StepResult::Ok)
}

/// Execute TT (Test Target), ARMv8-M, second halfword `0xF0xx`
/// 执行 TT（Test Target），ARMv8-M，第二半字为 `0xF0xx`
///
/// Encoding: `1110 1000 0100 Rn 1111 Rd A T 000000`. It shares the bit-22-set
/// exclusive encoding space, distinguished by `Rt2 == 15` in bits 15:12.
/// The response is written to `Rd`; `Rd` must not be SP or PC and `Rn` must
/// not be PC.
/// 编码：`1110 1000 0100 Rn 1111 Rd A T 000000`。它与位 22 置位的独占编码
/// 共享空间，靠位段 15:12 中的 `Rt2 == 15` 区分。响应写入 `Rd`；`Rd` 不得为
/// SP 或 PC，`Rn` 不得为 PC。
///
/// The modelled response reports "secure, readable and writable, no MPU" so
/// the `!ns` bit (22) is set, which is what secure-firmware lookups expect.
/// 本模型返回“安全态、可读可写、无 MPU”，因此 `!ns` 位（22）置位，这正是
/// 安全固件查找路径所期望的结果。
pub(super) fn execute_test_target(
    instr: u32,
    regs: &mut CortexM33Registers,
    cycles: &mut u64,
) -> StepResult {
    let rn = ((instr >> 16) & 0xF) as u8;
    let rd = ((instr >> 8) & 0xF) as u8;
    if rn == 15 || rd == 13 || rd == 15 {
        return StepResult::Undefined;
    }
    // bit 22 = !ns, bits 19:18 = R/RW permission; no MPU/SAU region info.
    // 位 22 = !ns，位段 19:18 为 R/RW 权限；无 MPU/SAU 区域信息。
    regs.write(rd, 0x004C_0000);
    *cycles += 1;
    StepResult::Ok
}

/// Execute TBB/TBH, `op1 == 0b01`, `op == 0x0D`, second halfword `0xF0xx`
/// 执行 TBB/TBH，`op1 == 0b01`，`op == 0x0D`，第二半字为 `0xF0xx`
///
/// The branch offset is an unsigned byte (TBB) or halfword (TBH) table entry,
/// scaled by two and added to the address of the following instruction.
/// 分支偏移为无符号字节（TBB）或半字（TBH）表项，按二倍缩放后加到
/// 下一条指令的地址。
pub(super) fn execute_table_branch(
    instr: u32,
    regs: &mut CortexM33Registers,
    bus: &mut dyn AddressBus,
    cycles: &mut u64,
) -> Result<StepResult, CoreError> {
    let rn = ((instr >> 16) & 0xF) as u8;
    let rm = (instr & 0xF) as u8;
    let halfword = (instr >> 4) & 1 != 0;
    let base = regs.read(rn);
    let index = regs.read(rm);

    let offset = if halfword {
        let addr = base.wrapping_add(index << 1);
        u32::from(bus.read_u16(addr, cycles)?)
    } else {
        let addr = base.wrapping_add(index);
        u32::from(bus.read_u8(addr, cycles)?)
    };
    regs.pc = regs.pc.wrapping_add(offset << 1);
    *cycles += 2;
    Ok(StepResult::BranchTaken)
}
