//! Coprocessor transfers: GPIO coprocessor (CP0) and DCP stub (CP4)
//! 协处理器传输：GPIO 协处理器（CP0）与 DCP 桩（CP4）
//!
//! Implements the Thumb-32 `MCR`/`MRC` (`0xEE`) and `MCRR`/`MRRC` (`0xEC`)
//! encodings when the coprocessor number selects the RP2350 GPIO
//! coprocessor (CP0) or the double-precision coprocessor (CP4). Other
//! coprocessor numbers are left to the normal decode tree.
//! 当协处理器号选择 RP2350 GPIO 协处理器（CP0）或双精度协处理器（CP4）时，
//! 实现 Thumb-32 `MCR`/`MRC`（`0xEE`）与 `MCRR`/`MRRC`（`0xEC`）编码。
//! 其他协处理器号留给常规解码树处理。
//!
//! ## CP0 / GPIO coprocessor / GPIO 协处理器
//!
//! CP0 maps directly onto the SIO GPIO registers. The implementation follows
//! `pico-sdk` `hardware/gpio_coproc.h`: 32-bit mask writes (`MCR`), paired
//! low/high writes and reads (`MCRR`/`MRRC`), single-pin operations, and
//! indexed writes. No `rp2350-periph` dependency is needed; the private
//! [`SIO_BASE`] constant keeps this crate self-contained.
//! CP0 直接映射到 SIO GPIO 寄存器，语义遵循 `pico-sdk` 的
//! `hardware/gpio_coproc.h`：32 位掩码写（`MCR`）、低/高寄存器成对读写
//! （`MCRR`/`MRRC`）、单引脚操作与索引写。为避免依赖 `rp2350-periph`，
//! 本模块用私有常量 [`SIO_BASE`] 保持 crate 自包含。
//!
//! ## CP4 / DCP stub / DCP 桩
//!
//! The double-precision coprocessor is modeled as permanently idle. `MRC`
//! returns zero, `MCR`/`MCRR` accept and ignore their operands, and `MRRC`
//! returns zero for both registers. This is enough for pico-sdk's
//! `runtime_init_per_core_enable_coprocessors`, which executes
//! `mrc p4,#0,r0,c0,c0,#1` to clear the DCP engaged flag.
//! 双精度协处理器被建模为永远空闲：`MRC` 返回零，`MCR`/`MCRR` 接受并忽略
//! 操作数，`MRRC` 对两个寄存器都返回零。这足以运行 pico-sdk 的
//! `runtime_init_per_core_enable_coprocessors`——它执行
//! `mrc p4,#0,r0,c0,c0,#1` 清除 DCP engaged 标志。
//!
//! ## Not implemented / 未实现清单
//!
//! - `CDP`, `LDC`/`STC` and the `...2` coprocessor encodings remain undefined.
//!   `CDP`、`LDC`/`STC` 以及 `...2` 协处理器编码仍视为未定义。
//! - DCP arithmetic is not modeled: the unit never reports engaged/busy and
//!   never produces a computed result.
//!   DCP 算术未建模：该单元不会报告 engaged/busy，也不会产生计算结果。

use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;

use crate::execute_thumb16::StepResult;
use crate::registers::CortexM33Registers;

/// SIO base address on RP2350 / RP2350 上的 SIO 基地址
///
/// Kept private so `rp2350-arm` does not depend on `rp2350-periph`.
/// 保持私有，使 `rp2350-arm` 无需依赖 `rp2350-periph`。
const SIO_BASE: u32 = 0xD000_0000;

/// `GPIO_IN` offset / `GPIO_IN` 偏移
const GPIO_IN: u32 = 0x004;
/// `GPIO_HI_IN` offset / `GPIO_HI_IN` 偏移
const GPIO_HI_IN: u32 = 0x008;
/// `GPIO_OUT` offset / `GPIO_OUT` 偏移
const GPIO_OUT: u32 = 0x010;
/// `GPIO_HI_OUT` offset / `GPIO_HI_OUT` 偏移
const GPIO_HI_OUT: u32 = 0x014;
/// `GPIO_OUT_SET` alias offset / `GPIO_OUT_SET` 别名偏移
const GPIO_OUT_SET: u32 = 0x018;
/// `GPIO_HI_OUT_SET` alias offset / `GPIO_HI_OUT_SET` 别名偏移
const GPIO_HI_OUT_SET: u32 = 0x01C;
/// `GPIO_OUT_CLR` alias offset / `GPIO_OUT_CLR` 别名偏移
const GPIO_OUT_CLR: u32 = 0x020;
/// `GPIO_HI_OUT_CLR` alias offset / `GPIO_HI_OUT_CLR` 别名偏移
const GPIO_HI_OUT_CLR: u32 = 0x024;
/// `GPIO_OUT_XOR` alias offset / `GPIO_OUT_XOR` 别名偏移
const GPIO_OUT_XOR: u32 = 0x028;
/// `GPIO_HI_OUT_XOR` alias offset / `GPIO_HI_OUT_XOR` 别名偏移
const GPIO_HI_OUT_XOR: u32 = 0x02C;
/// `GPIO_OE` offset / `GPIO_OE` 偏移
const GPIO_OE: u32 = 0x030;
/// `GPIO_HI_OE` offset / `GPIO_HI_OE` 偏移
const GPIO_HI_OE: u32 = 0x034;
/// `GPIO_OE_SET` alias offset / `GPIO_OE_SET` 别名偏移
const GPIO_OE_SET: u32 = 0x038;
/// `GPIO_HI_OE_SET` alias offset / `GPIO_HI_OE_SET` 别名偏移
const GPIO_HI_OE_SET: u32 = 0x03C;
/// `GPIO_OE_CLR` alias offset / `GPIO_OE_CLR` 别名偏移
const GPIO_OE_CLR: u32 = 0x040;
/// `GPIO_HI_OE_CLR` alias offset / `GPIO_HI_OE_CLR` 别名偏移
const GPIO_HI_OE_CLR: u32 = 0x044;
/// `GPIO_OE_XOR` alias offset / `GPIO_OE_XOR` 别名偏移
const GPIO_OE_XOR: u32 = 0x048;
/// `GPIO_HI_OE_XOR` alias offset / `GPIO_HI_OE_XOR` 别名偏移
const GPIO_HI_OE_XOR: u32 = 0x04C;

/// The four write forms of one 32-bit SIO GPIO register
/// 一个 32 位 SIO GPIO 寄存器的四种写形式
#[derive(Clone, Copy)]
struct BankRegisters {
    /// Plain write / 普通写
    put: u32,
    /// Write-1-to-toggle alias / 写 1 翻转别名
    xor: u32,
    /// Write-1-to-set alias / 写 1 置位别名
    set: u32,
    /// Write-1-to-clear alias / 写 1 清零别名
    clr: u32,
}

impl BankRegisters {
    /// Select the register address for an `opc1` write form (0-3)
    /// 按 `opc1` 写形式（0-3）选择寄存器地址
    fn alias(self, opc1: u32) -> u32 {
        match opc1 {
            0 => self.put,
            1 => self.xor,
            2 => self.set,
            _ => self.clr,
        }
    }
}

/// The low/high register pair selected by `CRm = 0` or `CRm = 4`
/// 由 `CRm = 0` 或 `CRm = 4` 选择的低/高寄存器对
#[derive(Clone, Copy)]
struct RegisterPair {
    /// GPIO 0-31 registers / GPIO 0-31 寄存器
    lo: BankRegisters,
    /// GPIO 32-63 registers / GPIO 32-63 寄存器
    hi: BankRegisters,
}

/// Output register pair / 输出寄存器对
const OUT_REGISTERS: RegisterPair = RegisterPair {
    lo: BankRegisters {
        put: GPIO_OUT,
        xor: GPIO_OUT_XOR,
        set: GPIO_OUT_SET,
        clr: GPIO_OUT_CLR,
    },
    hi: BankRegisters {
        put: GPIO_HI_OUT,
        xor: GPIO_HI_OUT_XOR,
        set: GPIO_HI_OUT_SET,
        clr: GPIO_HI_OUT_CLR,
    },
};

/// Output-enable register pair / 输出使能寄存器对
const OE_REGISTERS: RegisterPair = RegisterPair {
    lo: BankRegisters {
        put: GPIO_OE,
        xor: GPIO_OE_XOR,
        set: GPIO_OE_SET,
        clr: GPIO_OE_CLR,
    },
    hi: BankRegisters {
        put: GPIO_HI_OE,
        xor: GPIO_HI_OE_XOR,
        set: GPIO_HI_OE_SET,
        clr: GPIO_HI_OE_CLR,
    },
};

/// Try to decode and execute a CP0/CP4 coprocessor transfer
/// 尝试解码并执行一条 CP0/CP4 协处理器传输
///
/// Returns `Ok(None)` when the encoding is not a CP0/CP4 transfer, letting
/// the normal Thumb-32 decode tree continue.
/// 编码不是 CP0/CP4 传输时返回 `Ok(None)`，由常规 Thumb-32 解码树继续处理。
pub(super) fn try_execute(
    instr: u32,
    regs: &mut CortexM33Registers,
    bus: &mut dyn AddressBus,
    cycles: &mut u64,
) -> Result<Option<StepResult>, CoreError> {
    let top = instr >> 24;
    let coproc = (instr >> 8) & 0xF;
    if !matches!(coproc, 0 | 4) {
        return Ok(None);
    }

    // MCR/MRC use bit 4 as a fixed 1 to distinguish themselves from CDP.
    // MCR/MRC 用固定的 bit 4 = 1 与 CDP 区分。
    if top == 0xEE && instr & 0x10 != 0 {
        if !begin_coprocessor(regs, coproc) {
            return Ok(Some(StepResult::Ok));
        }
        return Ok(Some(execute_mcr_mrc(instr, regs, bus, cycles)?));
    }

    // MCRR/MRRC use `1110 1100 010x` and encode `opc1` in bits[7:4].
    // MCRR/MRRC 使用 `1110 1100 010x`，`opc1` 位于 bits[7:4]。
    if top == 0xEC && (instr >> 21) & 0x7 == 0b010 {
        if !begin_coprocessor(regs, coproc) {
            return Ok(Some(StepResult::Ok));
        }
        return Ok(Some(execute_mcrr_mrrc(instr, regs, bus, cycles)?));
    }

    Ok(None)
}

/// Check CPACR access, or raise the synchronous NOCP UsageFault
/// 检查 CPACR 权限，或触发同步 NOCP 用法故障
///
/// Mirrors the FPU path: on denial the PC is rewound to the faulting
/// instruction and a pending UsageFault is recorded for the core to enter.
/// 与 FPU 通路一致：拒绝访问时回退 PC 到故障指令，并记录待处理的用法故障，
/// 由核心立即进入异常。
fn begin_coprocessor(regs: &mut CortexM33Registers, coproc: u32) -> bool {
    if regs.cp_access_enabled(coproc as u8) {
        return true;
    }
    regs.pc = regs.pc.wrapping_sub(4);
    regs.raise_usage_fault();
    false
}

/// Execute an `MCR`/`MRC` encoding / 执行 `MCR`/`MRC` 编码
fn execute_mcr_mrc(
    instr: u32,
    regs: &mut CortexM33Registers,
    bus: &mut dyn AddressBus,
    cycles: &mut u64,
) -> Result<StepResult, CoreError> {
    let opc1 = (instr >> 21) & 0x7;
    let load = instr & (1 << 20) != 0;
    let crn = (instr >> 16) & 0xF;
    let rt = ((instr >> 12) & 0xF) as u8;
    let coproc = (instr >> 8) & 0xF;
    let opc2 = (instr >> 5) & 0x7;
    let crm = instr & 0xF;

    if rt == 15 {
        return Ok(StepResult::Undefined);
    }

    if coproc == 4 {
        // DCP is modeled idle: reads return zero, writes have no effect.
        // DCP 建模为空闲：读取返回零，写入无效果。
        if load {
            regs.write(rt, 0);
        }
        return Ok(StepResult::Ok);
    }

    // GPIOC transfers use CRn = c0 and opc2 = 0.
    // GPIOC 传输使用 CRn = c0 且 opc2 = 0。
    if crn != 0 || opc2 != 0 {
        return Ok(StepResult::Undefined);
    }

    if load {
        // All GPIOC read encodings require opc1 = 0.
        // GPIOC 的所有读取编码都要求 opc1 = 0。
        if opc1 != 0 {
            return Ok(StepResult::Undefined);
        }
        let Some(offset) = gpioc_read_offset(crm) else {
            return Ok(StepResult::Undefined);
        };
        let value = bus.read_u32(SIO_BASE.wrapping_add(offset), cycles)?;
        regs.write(rt, value);
        Ok(StepResult::Ok)
    } else {
        execute_gpioc_mcr(opc1, rt, crm, regs, bus, cycles)
    }
}

/// Execute a GPIOC `MCR` / 执行 GPIOC `MCR`
fn execute_gpioc_mcr(
    opc1: u32,
    rt: u8,
    crm: u32,
    regs: &CortexM33Registers,
    bus: &mut dyn AddressBus,
    cycles: &mut u64,
) -> Result<StepResult, CoreError> {
    match opc1 {
        // Plain write / XOR / set / clear of a 32-bit register.
        // 32 位寄存器的普通写 / 翻转 / 置位 / 清零。
        0..=3 => {
            let Some(bank) = mcr_bank(crm) else {
                return Ok(StepResult::Undefined);
            };
            bus.write_u32(
                SIO_BASE.wrapping_add(bank.alias(opc1)),
                regs.read(rt),
                cycles,
            )?;
            Ok(StepResult::Ok)
        }
        // Single-pin XOR / set / clear. The pin selects low or high bank.
        // 单引脚翻转 / 置位 / 清零；引脚号选择低或高寄存器组。
        5..=7 => {
            let pin = regs.read(rt);
            let Some(bank) = mcr_bit_bank(crm, pin) else {
                return Ok(StepResult::Undefined);
            };
            let mask = 1u32 << (pin & 31);
            bus.write_u32(SIO_BASE.wrapping_add(bank.alias(opc1 - 4)), mask, cycles)?;
            Ok(StepResult::Ok)
        }
        _ => Ok(StepResult::Undefined),
    }
}

/// Execute an `MCRR`/`MRRC` encoding / 执行 `MCRR`/`MRRC` 编码
fn execute_mcrr_mrrc(
    instr: u32,
    regs: &mut CortexM33Registers,
    bus: &mut dyn AddressBus,
    cycles: &mut u64,
) -> Result<StepResult, CoreError> {
    let load = instr & (1 << 20) != 0;
    let rt2 = ((instr >> 16) & 0xF) as u8;
    let rt = ((instr >> 12) & 0xF) as u8;
    let coproc = (instr >> 8) & 0xF;
    let opc1 = (instr >> 4) & 0xF;
    let crm = instr & 0xF;

    if rt == 15 || rt2 == 15 {
        return Ok(StepResult::Undefined);
    }

    if coproc == 4 {
        // DCP is modeled idle: MRRC reads zero, MCRR writes are ignored.
        // DCP 建模为空闲：MRRC 读零，MCRR 写被忽略。
        if load {
            regs.write(rt, 0);
            regs.write(rt2, 0);
        }
        return Ok(StepResult::Ok);
    }

    if load {
        // GPIOC MRRC requires opc1 = 0. / GPIOC MRRC 要求 opc1 = 0。
        if opc1 != 0 {
            return Ok(StepResult::Undefined);
        }
        execute_gpioc_mrrc(rt, rt2, crm, regs, bus, cycles)
    } else {
        execute_gpioc_mcrr(opc1, rt, rt2, crm, regs, bus, cycles)
    }
}

/// Execute a GPIOC `MRRC` / 执行 GPIOC `MRRC`
fn execute_gpioc_mrrc(
    rt: u8,
    rt2: u8,
    crm: u32,
    regs: &mut CortexM33Registers,
    bus: &mut dyn AddressBus,
    cycles: &mut u64,
) -> Result<StepResult, CoreError> {
    let Some((lo_offset, hi_offset)) = mrrc_offsets(crm) else {
        return Ok(StepResult::Undefined);
    };
    let lo = bus.read_u32(SIO_BASE.wrapping_add(lo_offset), cycles)?;
    let hi = bus.read_u32(SIO_BASE.wrapping_add(hi_offset), cycles)?;
    regs.write(rt, lo);
    regs.write(rt2, hi);
    Ok(StepResult::Ok)
}

/// Execute a GPIOC `MCRR` / 执行 GPIOC `MCRR`
fn execute_gpioc_mcrr(
    opc1: u32,
    rt: u8,
    rt2: u8,
    crm: u32,
    regs: &CortexM33Registers,
    bus: &mut dyn AddressBus,
    cycles: &mut u64,
) -> Result<StepResult, CoreError> {
    let Some(pair) = mcrr_register_pair(crm) else {
        return Ok(StepResult::Undefined);
    };

    match opc1 {
        // Paired low/high plain write / XOR / set / clear.
        // 低/高寄存器成对的普通写 / 翻转 / 置位 / 清零。
        0..=3 => {
            let lo = regs.read(rt);
            let hi = regs.read(rt2);
            bus.write_u32(SIO_BASE.wrapping_add(pair.lo.alias(opc1)), lo, cycles)?;
            bus.write_u32(SIO_BASE.wrapping_add(pair.hi.alias(opc1)), hi, cycles)?;
        }
        // Conditional and unconditional single-pin operations.
        // 条件与无条件的单引脚操作。
        4..=7 => {
            let pin = regs.read(rt);
            let condition = regs.read(rt2);
            if pin >= 64 {
                return Ok(StepResult::Undefined);
            }
            let bank = if pin < 32 { pair.lo } else { pair.hi };
            let mask = 1u32 << (pin & 31);
            let alias = match opc1 {
                // opc1 = 4 always writes: set when true, clear otherwise.
                // opc1 = 4 总会写入：条件为真时置位，否则清零。
                4 => {
                    if condition != 0 {
                        2
                    } else {
                        3
                    }
                }
                // opc1 = 5/6/7 do nothing when the condition is false.
                // opc1 = 5/6/7 在条件为假时不执行任何操作。
                5..=7 if condition != 0 => opc1 - 4,
                _ => return Ok(StepResult::Ok),
            };
            bus.write_u32(SIO_BASE.wrapping_add(bank.alias(alias)), mask, cycles)?;
        }
        // Indexed write: Rt = value, Rt2 = 0 (low) or 1 (high).
        // 索引写：Rt = 数值，Rt2 = 0（低）或 1（高）。
        8..=11 => {
            let value = regs.read(rt);
            let index = regs.read(rt2);
            let bank = match index {
                0 => pair.lo,
                1 => pair.hi,
                _ => return Ok(StepResult::Undefined),
            };
            bus.write_u32(SIO_BASE.wrapping_add(bank.alias(opc1 - 8)), value, cycles)?;
        }
        _ => return Ok(StepResult::Undefined),
    }

    Ok(StepResult::Ok)
}

/// Map `MCR` `CRm` to a single 32-bit GPIO register bank
/// 将 `MCR` 的 `CRm` 映射到单个 32 位 GPIO 寄存器组
fn mcr_bank(crm: u32) -> Option<BankRegisters> {
    match crm {
        0 => Some(OUT_REGISTERS.lo),
        1 => Some(OUT_REGISTERS.hi),
        4 => Some(OE_REGISTERS.lo),
        5 => Some(OE_REGISTERS.hi),
        _ => None,
    }
}

/// Map a single-pin `MCR` `CRm` and pin to a GPIO register bank
/// 将单引脚 `MCR` 的 `CRm` 与引脚映射到 GPIO 寄存器组
fn mcr_bit_bank(crm: u32, pin: u32) -> Option<BankRegisters> {
    let pair = match crm {
        0 => OUT_REGISTERS,
        4 => OE_REGISTERS,
        _ => return None,
    };
    if pin >= 64 {
        return None;
    }
    Some(if pin < 32 { pair.lo } else { pair.hi })
}

/// Map `MCRR` `CRm` to a low/high GPIO register pair
/// 将 `MCRR` 的 `CRm` 映射到低/高 GPIO 寄存器对
fn mcrr_register_pair(crm: u32) -> Option<RegisterPair> {
    match crm {
        0 => Some(OUT_REGISTERS),
        4 => Some(OE_REGISTERS),
        _ => None,
    }
}

/// Map `MRC` `CRm` to a GPIO read register offset
/// 将 `MRC` 的 `CRm` 映射到 GPIO 读取寄存器偏移
fn gpioc_read_offset(crm: u32) -> Option<u32> {
    match crm {
        0 => Some(GPIO_OUT),
        1 => Some(GPIO_HI_OUT),
        4 => Some(GPIO_OE),
        5 => Some(GPIO_HI_OE),
        8 => Some(GPIO_IN),
        9 => Some(GPIO_HI_IN),
        _ => None,
    }
}

/// Map `MRRC` `CRm` to low/high GPIO read register offsets
/// 将 `MRRC` 的 `CRm` 映射到低/高 GPIO 读取寄存器偏移
fn mrrc_offsets(crm: u32) -> Option<(u32, u32)> {
    match crm {
        0 => Some((GPIO_OUT, GPIO_HI_OUT)),
        4 => Some((GPIO_OE, GPIO_HI_OE)),
        8 => Some((GPIO_IN, GPIO_HI_IN)),
        _ => None,
    }
}
