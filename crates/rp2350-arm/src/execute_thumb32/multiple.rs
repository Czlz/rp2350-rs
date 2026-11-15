//! Thumb-32 load/store multiple, exclusive and table branch instructions
//! Thumb-32 多寄存器加载/存储、独占访问与表分支指令
//!
//! Covers LDM/STM (including the `.W` write-back forms used by push/pop),
//! the exclusive and load-acquire/store-release instructions, and TBB/TBH
//! within the `op1 == 0b01` encoding group.
//! 覆盖 `op1 == 0b01` 编码组内的 LDM/STM（含 push/pop 使用的 `.W` 写回形式）、
//! 独占访问与获取/释放指令，以及 TBB/TBH。
//!
//! The exclusive monitor is kept per core, inside each
//! `CortexM33Registers`, so an exclusive load on one core can never be
//! consumed by an exclusive store on the other core. Bus snooping (a write by
//! another core invalidating the monitor) and the ordering guarantees of
//! acquire/release are still not modelled; those instructions execute as plain
//! loads/stores with the architecturally visible data result.
//! 独占监视器按核心保存在各自的 `CortexM33Registers` 中，因此一个核心的独占
//! 加载绝不可能被另一核心的独占存储消费。仍未建模跨核总线监听（其它核对监视
//! 地址的写入不会自动清除监视器）以及获取/释放的顺序保证；这些指令按普通
//! 加载/存储执行，只保留架构上可见的数据结果。

use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;

use crate::execute_thumb16::StepResult;
use crate::registers::CortexM33Registers;

/// Size of an exclusive or acquire/release transfer
/// 独占或获取/释放传输的尺寸
#[derive(Clone, Copy, PartialEq, Eq)]
enum TransferSize {
    /// Byte transfer / 字节传输
    Byte,
    /// Halfword transfer / 半字传输
    Half,
    /// Word transfer / 字传输
    Word,
    /// Doubleword transfer / 双字传输
    Double,
}

impl TransferSize {
    /// Transfer width in bytes, used as the exclusive-monitor size code
    /// 以字节计的传输宽度，用作独占监视器的尺寸编码
    ///
    /// Address and width must both match for a `STREX*` to succeed, so the code
    /// is what distinguishes e.g. a word `LDREX` from a `STREXB` on the same
    /// address.
    /// `STREX*` 必须地址与宽度同时匹配才成功，因此该编码用于区分同一地址上的
    /// 字 `LDREX` 与 `STREXB`。
    const fn bytes(self) -> u8 {
        match self {
            Self::Byte => 1,
            Self::Half => 2,
            Self::Word => 4,
            Self::Double => 8,
        }
    }
}

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

    if write_back && (!load || (list & (1 << rn)) == 0) {
        let updated = if decrement_before {
            start
        } else {
            base.wrapping_add(count * 4)
        };
        regs.write(rn, updated);
    }

    *cycles += u64::from(count);

    if let Some(value) = loaded_pc {
        // LDM/POP.W 写 PC 同样使用 LoadWritePC 语义。
        // LDM/POP.W writes to PC also use LoadWritePC semantics.
        if crate::exceptions::write_pc(regs, value) {
            return Ok(StepResult::ExceptionReturn);
        }
        return Ok(StepResult::BranchTaken);
    }
    Ok(StepResult::Ok)
}

/// Execute LDREX/STREX, `op1 == 0b01`, `op2 == 0b00`, bit 22 set, bit 23 clear
/// 执行 LDREX/STREX，`op1 == 0b01`，`op2 == 0b00`，位 22 置位、位 23 清零
///
/// `imm8` is scaled by four. LDREX loads into `Rt` (bits 15:12) and arms this
/// core's monitor; STREX stores `Rt`, reports success in `Rd` (bits 11:8) and
/// clears the monitor, with `Rd == 15` discarding the status.
/// `imm8` 按四倍缩放。LDREX 加载到 `Rt`（位段 15:12）并武装本核的监视器；
/// STREX 存储 `Rt`，把成功状态写入 `Rd`（位段 11:8）并清除监视器，`Rd == 15`
/// 表示丢弃状态。
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
    let size = TransferSize::Word.bytes();

    if load {
        let value = bus.read_u32(addr, cycles)?;
        regs.write(rt, value);
        regs.arm_exclusive(addr, size);
    } else {
        let rd = ((instr >> 8) & 0xF) as u8;
        let success = regs.consume_exclusive(addr, size);
        if success {
            bus.write_u32(addr, regs.read(rt), cycles)?;
        }
        if rd != 15 {
            regs.write(rd, u32::from(!success));
        }
    }

    *cycles += 1;
    Ok(StepResult::Ok)
}

/// Execute load/store exclusive byte/halfword/doubleword and the
/// load-acquire/store-release family, `op1 == 0b01`, bit 22 set, bit 23 set
/// 执行字节/半字/双字独占访问与获取/释放指令族，`op1 == 0b01`，位 22 置位、
/// 位 23 置位
///
/// The second halfword bits 7:4 select the operation and bits 3:0 hold either
/// the status register of an exclusive store or the fixed `1111` marker. This
/// covers `STREXB/H/D`, `LDREXB/H/D`, `STLEXB/H/D`, `LDAEXB/H/D`, and
/// `STLB/H`, `LDA/B/H` and the word forms. Immediate offsets do not exist in
/// this encoding, so the address is simply `Rn`.
/// 第二半字位段 7:4 选择操作，位段 3:0 或为独占存储的状态寄存器，或为固定
/// 标记 `1111`。覆盖 `STREXB/H/D`、`LDREXB/H/D`、`STLEXB/H/D`、
/// `LDAEXB/H/D`，以及 `STLB/H`、`LDA/B/H` 与字形式。该编码没有立即数偏移，
/// 地址就是 `Rn`。
pub(super) fn execute_acquire_release(
    instr: u32,
    regs: &mut CortexM33Registers,
    bus: &mut dyn AddressBus,
    cycles: &mut u64,
) -> Result<StepResult, CoreError> {
    let load = (instr >> 20) & 1 != 0;
    let rn = ((instr >> 16) & 0xF) as u8;
    let rt = ((instr >> 12) & 0xF) as u8;
    let low = (instr & 0xF) as u8;
    let Some((size, exclusive)) = decode_acquire_release((instr >> 4) & 0xF) else {
        return Ok(StepResult::Undefined);
    };
    if rn == 15 || rt == 15 {
        return Ok(StepResult::Undefined);
    }

    let double = size == TransferSize::Double;
    let rt2 = ((instr >> 8) & 0xF) as u8;
    if double {
        if rt2 == 15 {
            return Ok(StepResult::Undefined);
        }
    } else if (instr >> 8) & 0xF != 0xF {
        return Ok(StepResult::Undefined);
    }
    // Loads and non-exclusive stores use the fixed 1111 low nibble.
    // 加载与非独占存储使用固定的低位半字节 1111。
    if (load || !exclusive) && low != 15 {
        return Ok(StepResult::Undefined);
    }

    let addr = regs.read(rn);

    if load {
        if double {
            let first = bus.read_u32(addr, cycles)?;
            let second = bus.read_u32(addr.wrapping_add(4), cycles)?;
            regs.write(rt, first);
            regs.write(rt2, second);
        } else {
            let value = read_sized(size, bus, addr, cycles)?;
            regs.write(rt, value);
        }
        if exclusive {
            regs.arm_exclusive(addr, size.bytes());
        }
        *cycles += 1;
        return Ok(StepResult::Ok);
    }

    let success = if exclusive {
        regs.consume_exclusive(addr, size.bytes())
    } else {
        true
    };
    if success {
        if double {
            bus.write_u32(addr, regs.read(rt), cycles)?;
            bus.write_u32(addr.wrapping_add(4), regs.read(rt2), cycles)?;
        } else {
            write_sized(size, bus, addr, regs.read(rt), cycles)?;
        }
    }
    if exclusive && low != 15 {
        regs.write(low, u32::from(!success));
    }
    *cycles += 1;
    Ok(StepResult::Ok)
}

/// Map second-halfword bits 7:4 to a transfer size and exclusivity flag
/// 将第二半字位段 7:4 映射为传输尺寸与是否独占
fn decode_acquire_release(sub_op: u32) -> Option<(TransferSize, bool)> {
    Some(match sub_op {
        // `0x4/0xC`: STREXB/LDREXB and STLEXB/LDAEXB
        // `0x5/0xD`: STREXH/LDREXH and STLEXH/LDAEXH
        // `0x7/0xF`: STREXD/LDREXD and STLEXD/LDAEXD
        // `0x4/0xC`：STREXB/LDREXB 与 STLEXB/LDAEXB
        // `0x5/0xD`：STREXH/LDREXH 与 STLEXH/LDAEXH
        // `0x7/0xF`：STREXD/LDREXD 与 STLEXD/LDAEXD
        0x4 | 0xC => (TransferSize::Byte, true),
        0x5 | 0xD => (TransferSize::Half, true),
        0x7 | 0xF => (TransferSize::Double, true),
        // `0x8-0xA`: STLB/LDAB, STLH/LDAH, STL/LDA (no monitor)
        // `0x8-0xA`：STLB/LDAB、STLH/LDAH、STL/LDA（无监视器）
        0x8 => (TransferSize::Byte, false),
        0x9 => (TransferSize::Half, false),
        0xA => (TransferSize::Word, false),
        // `0xE`: STLEX/LDAEX (word)
        // `0xE`：STLEX/LDAEX（字）
        0xE => (TransferSize::Word, true),
        _ => return None,
    })
}

/// Read a byte or halfword and widen it to a word
/// 读取字节或半字并零扩展到字
fn read_sized(
    size: TransferSize,
    bus: &mut dyn AddressBus,
    addr: u32,
    cycles: &mut u64,
) -> Result<u32, CoreError> {
    Ok(match size {
        TransferSize::Byte => u32::from(bus.read_u8(addr, cycles)?),
        TransferSize::Half => u32::from(bus.read_u16(addr, cycles)?),
        TransferSize::Word | TransferSize::Double => bus.read_u32(addr, cycles)?,
    })
}

/// Write the low byte, halfword or word of a register
/// 写入寄存器的低字节、半字或字
fn write_sized(
    size: TransferSize,
    bus: &mut dyn AddressBus,
    addr: u32,
    value: u32,
    cycles: &mut u64,
) -> Result<(), CoreError> {
    match size {
        TransferSize::Byte => bus.write_u8(addr, value as u8, cycles),
        TransferSize::Half => bus.write_u16(addr, value as u16, cycles),
        TransferSize::Word | TransferSize::Double => bus.write_u32(addr, value, cycles),
    }
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
