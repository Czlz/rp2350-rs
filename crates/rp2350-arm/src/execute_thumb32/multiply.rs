//! Thumb-32 multiply and divide instructions
//! Thumb-32 乘法与除法指令
//!
//! Covers MUL/MLA/MLS, the long multiplies SMULL/UMULL/SMLAL/UMLAL, the
//! halfword-accumulating long multiply `SMLALxy`, the signed/unsigned divide
//! instructions, and the DSP multiply family:
//! `SMULxy`/`SMLAxy`, `SMULWB`/`SMULWT`, `SMLAWB`/`SMLAWT`, `SMMUL`/`SMMLA`,
//! `SMMLS`/`SMMLSR`, the dual forms `SMLAD`/`SMLSD`/`SMUAD`/`SMUSD` (with their
//! `X` exchanges) and the long dual forms `SMLALD`/`SMLSLD`.
//! 覆盖 MUL/MLA/MLS、长乘法 SMULL/UMULL/SMLAL/UMLAL、半字长累加乘
//! `SMLALxy`、有符号/无符号除法，以及 DSP 乘法族：`SMULxy`/`SMLAxy`、
//! `SMULWB`/`SMULWT`、`SMLAWB`/`SMLAWT`、`SMMUL`/`SMMLA`、`SMMLS`/`SMMLSR`、
//! 双乘形式 `SMLAD`/`SMLSD`/`SMUAD`/`SMUSD`（含 `X` 交换）与长双乘形式
//! `SMLALD`/`SMLSLD`。

use crate::execute_thumb16::StepResult;
use crate::registers::CortexM33Registers;

/// Execute a multiply/divide instruction, `op1 == 0b11`, `op` in `0x30..=0x3F`
/// 执行乘除指令，`op1 == 0b11`，`op` 在 `0x30..=0x3F`
///
/// Instruction bits [23:20] select the operation; the long multiplies and
/// divides use bits [15:12] and [11:8] as the two destination registers.
/// 指令位 [23:20] 选择操作；长乘法和除法使用位 [15:12] 与 [11:8]
/// 作为两个目标寄存器。
pub(super) fn execute(instr: u32, regs: &mut CortexM33Registers) -> StepResult {
    let sub = ((instr >> 20) & 0xF) as u8;
    let rn = ((instr >> 16) & 0xF) as u8;
    let ra_or_lo = ((instr >> 12) & 0xF) as u8;
    let rd = ((instr >> 8) & 0xF) as u8;
    let bits7_4 = ((instr >> 4) & 0xF) as u8;
    let rm = (instr & 0xF) as u8;

    match sub {
        // MUL/MLA when bits[7:4] = 0, MLS when bits[7:4] = 1.
        // bits[7:4] = 0 时为 MUL/MLA，bits[7:4] = 1 时为 MLS。
        0x0 => {
            let product = regs.read(rn).wrapping_mul(regs.read(rm));
            let result = match bits7_4 {
                0x0 => {
                    if ra_or_lo == 0xF {
                        product
                    } else {
                        regs.read(ra_or_lo).wrapping_add(product)
                    }
                }
                0x1 => regs.read(ra_or_lo).wrapping_sub(product),
                _ => return StepResult::Undefined,
            };
            regs.write(rd, result);
            StepResult::Ok
        }
        // SMULxy/SMLAxy (DSP extension): signed 16x16 multiply of selected
        // halfwords. `bits7_4` bit 1 selects the high half of Rn, bit 0 the
        // high half of Rm; Ra = 15 selects the non-accumulating SMULxy form.
        // SMULxy/SMLAxy（DSP 扩展）：选取半字做有符号 16×16 乘法。
        // `bits7_4` 位 1 选择 Rn 的高半字，位 0 选择 Rm 的高半字；
        // Ra = 15 表示非累加的 SMULxy 形式。
        0x1 => {
            // Only 0b00xy is allocated; the upper half of the nibble is reserved.
            // 仅分配 0b00xy，该半字节的高两位保留。
            if bits7_4 > 0b11 {
                return StepResult::Undefined;
            }
            let n_high = bits7_4 & 0b10 != 0;
            let m_high = bits7_4 & 0b01 != 0;
            let n_half = if n_high {
                i32::from((regs.read(rn) >> 16) as i16)
            } else {
                i32::from(regs.read(rn) as i16)
            };
            let m_half = if m_high {
                i32::from((regs.read(rm) >> 16) as i16)
            } else {
                i32::from(regs.read(rm) as i16)
            };
            let product = n_half.wrapping_mul(m_half);
            if ra_or_lo == 0xF {
                regs.write(rd, product as u32);
            } else {
                let exact = i64::from(regs.read(ra_or_lo) as i32) + i64::from(product);
                let accumulated = exact as i32;
                if exact != i64::from(accumulated) {
                    regs.set_q_flag();
                }
                regs.write(rd, accumulated as u32);
            }
            StepResult::Ok
        }
        // SMMLA/SMMUL (DSP extension): high 32 bits of a signed 32x32 product,
        // optionally rounded and accumulated. Ra = 15 selects SMMUL.
        // SMMLA/SMMUL（DSP 扩展）：有符号 32×32 乘积的高 32 位，可选舍入与
        // 累加。Ra = 15 表示 SMMUL。
        0x5 => {
            // `bits7_4` = 0b000R; the rest of the nibble is reserved.
            // `bits7_4` = 0b000R，该半字节其余位保留。
            if bits7_4 > 1 {
                return StepResult::Undefined;
            }
            let product = i64::from(regs.read(rn) as i32) * i64::from(regs.read(rm) as i32);
            let low = product as u32;
            let mut high = (product >> 32) as i32;
            if bits7_4 & 1 != 0 && low >= 0x8000_0000 {
                high = high.wrapping_add(1);
            }
            let accumulated = if ra_or_lo == 0xF {
                high
            } else {
                high.wrapping_add(regs.read(ra_or_lo) as i32)
            };
            regs.write(rd, accumulated as u32);
            StepResult::Ok
        }
        // SMLAD/SMLADX/SMUAD/SMUADX (add) and SMLSD/SMLSDX/SMUSD/SMUSDX
        // (subtract): two signed 16x16 products combined into one 32-bit
        // result. `bits7_4` bit 4 selects the exchanged halfwords and bit 5
        // of that nibble is reserved. Ra = 15 (SMUAD/SMUSD) still sets Q on
        // overflow, it only drops the accumulator.
        // SMLAD/SMLADX/SMUAD/SMUADX（加）与 SMLSD/SMLSDX/SMUSD/SMUSDX（减）：
        // 两个有符号 16×16 乘积合成一个 32 位结果。`bits7_4` 的位 4 选择交换
        // 半字，该半字节的位 5 保留。Ra = 15（SMUAD/SMUSD）仍在上溢时置 Q，
        // 只是不再累加。
        0x2 => execute_dual(instr, regs, false),
        0x4 => execute_dual(instr, regs, true),
        // SMULWB/SMULWT/SMLAWB/SMLAWT (DSP extension): signed 32x16 multiply
        // taking the top word of the 48-bit product. Ra = 15 selects the
        // non-accumulating SMULW form; `bits7_4` bit 4 selects the top half of
        // Rm and the rest of the nibble is reserved.
        // SMULWB/SMULWT/SMLAWB/SMLAWT（DSP 扩展）：有符号 32×16 乘法，取 48 位
        // 乘积的高字。Ra = 15 表示非累加的 SMULW 形式；`bits7_4` 位 4 选择 Rm
        // 的高半字，该半字节其余位保留。
        0x3 => execute_word_halfword(instr, regs),
        // SMMLS/SMMLSR (DSP extension): `Ra<<32 - Rn*Rm` high word, with the
        // optional rounding constant. `bits7_4` = 0b000R.
        // SMMLS/SMMLSR（DSP 扩展）：`Ra<<32 - Rn*Rm` 的高字，可选舍入常量。
        // `bits7_4` = 0b000R。
        0x6 => execute_smmls(instr, regs),
        // USAD8/USADA8 (DSP extension): sum of the absolute differences of the
        // four byte lanes. Ra = 15 selects the non-accumulating USAD8 form.
        // Flags are not modified.
        // USAD8/USADA8（DSP 扩展）：四个字节通道绝对差之和。Ra = 15 表示
        // 非累加的 USAD8 形式。不更新标志。
        0x7 => {
            if bits7_4 != 0x0 {
                return StepResult::Undefined;
            }
            if rd == 15 {
                return StepResult::Undefined;
            }
            let a = regs.read(rn);
            let b = regs.read(rm);
            let mut sum = 0u32;
            for i in 0..4u32 {
                let a_lane = (a >> (i * 8)) & 0xFF;
                let b_lane = (b >> (i * 8)) & 0xFF;
                sum += a_lane.abs_diff(b_lane);
            }
            let result = if ra_or_lo == 0xF {
                sum
            } else {
                regs.read(ra_or_lo).wrapping_add(sum)
            };
            regs.write(rd, result);
            StepResult::Ok
        }
        // SMULL: signed 64-bit product, low word in bits[15:12], high in [11:8].
        // SMULL：有符号 64 位乘积，低位字写入 bits[15:12]，高位字写入 [11:8]。
        0x8 => {
            let product = i64::from(regs.read(rn) as i32) * i64::from(regs.read(rm) as i32);
            regs.write(ra_or_lo, product as u32);
            regs.write(rd, (product >> 32) as u32);
            StepResult::Ok
        }
        // UMULL: unsigned 64-bit product.
        // UMULL：无符号 64 位乘积。
        0xA => {
            let product = u64::from(regs.read(rn)) * u64::from(regs.read(rm));
            regs.write(ra_or_lo, product as u32);
            regs.write(rd, (product >> 32) as u32);
            StepResult::Ok
        }
        // SMLAL: signed 64-bit accumulate. `bits7_4` = 0b1100/0b1101 selects
        // the dual form SMLALD/SMLALDX instead, and 0b10yz the halfword form
        // SMLALxy.
        // SMLAL：有符号 64 位累加。`bits7_4` = 0b1100/0b1101 时改为双乘形式
        // SMLALD/SMLALDX，0b10yz 时改为半字形式 SMLALxy。
        0xC => {
            if bits7_4 & 0b1110 == 0b1100 {
                return execute_dual_long(instr, regs, false);
            }
            // SMLALxy: the `10` prefix of bits[7:6] separates it from SMLAL
            // (0b0000), the reserved 0b00xx/0b01xx patterns and SMLALD
            // (0b110x). Bit 5 picks the top halfword of Rn, bit 4 the top
            // halfword of Rm.
            // SMLALxy：bits[7:6] 的 `10` 前缀把它与 SMLAL（0b0000）、保留的
            // 0b00xx/0b01xx 以及 SMLALD（0b110x）区分开。位 5 选择 Rn 的高
            // 半字，位 4 选择 Rm 的高半字。
            if bits7_4 & 0b1100 == 0b1000 {
                return execute_smlal_halfwords(instr, regs);
            }
            if bits7_4 != 0 {
                return StepResult::Undefined;
            }
            let accumulator = (u64::from(regs.read(rd)) << 32) | u64::from(regs.read(ra_or_lo));
            let product = i64::from(regs.read(rn) as i32) * i64::from(regs.read(rm) as i32);
            let result = (accumulator as i64).wrapping_add(product) as u64;
            regs.write(ra_or_lo, result as u32);
            regs.write(rd, (result >> 32) as u32);
            StepResult::Ok
        }
        // SMLSLD/SMLSLDX: dual signed multiply-subtract accumulating into a
        // 64-bit register pair, `bits7_4` = 0b1100/0b1101.
        // SMLSLD/SMLSLDX：双有符号乘减并累加到 64 位寄存器对，
        // `bits7_4` = 0b1100/0b1101。
        0xD => {
            if bits7_4 & 0b1110 != 0b1100 {
                return StepResult::Undefined;
            }
            execute_dual_long(instr, regs, true)
        }
        // UMLAL: unsigned 64-bit accumulate.
        // UMLAL：无符号 64 位累加。
        0xE => {
            let accumulator = (u64::from(regs.read(rd)) << 32) | u64::from(regs.read(ra_or_lo));
            let product = u64::from(regs.read(rn)) * u64::from(regs.read(rm));
            let result = accumulator.wrapping_add(product);
            regs.write(ra_or_lo, result as u32);
            regs.write(rd, (result >> 32) as u32);
            StepResult::Ok
        }
        0x9 => {
            if bits7_4 != 0xF {
                return StepResult::Undefined;
            }
            let divisor = regs.read(rm) as i32;
            // CCR.DIV_0_TRP：除数为零时进入 UFSR.DIVBYZERO，且不写目标寄存器。
            // 未置位时保持 ARM 规定的结果：除零为 0，INT_MIN / -1 为 INT_MIN。
            // CCR.DIV_0_TRP: a zero divisor raises UFSR.DIVBYZERO and leaves the
            // destination unwritten. With the bit clear the ARM results hold:
            // zero for divide-by-zero and INT_MIN for INT_MIN / -1.
            if divisor == 0 && regs.trap_divide_by_zero() {
                return StepResult::Ok;
            }
            let result = (regs.read(rn) as i32)
                .checked_div(divisor)
                .unwrap_or(if divisor == 0 { 0 } else { i32::MIN });
            regs.write(rd, result as u32);
            StepResult::Ok
        }
        0xB => {
            if bits7_4 != 0xF {
                return StepResult::Undefined;
            }
            let divisor = regs.read(rm);
            if divisor == 0 && regs.trap_divide_by_zero() {
                return StepResult::Ok;
            }
            let result = regs.read(rn).checked_div(divisor).unwrap_or(0);
            regs.write(rd, result);
            StepResult::Ok
        }
        _ => StepResult::Undefined,
    }
}

/// Split a register into its two signed halfwords, low halfword first
/// 把寄存器拆成两个有符号半字，低半字在前
fn signed_halfwords(value: u32) -> (i64, i64) {
    (i64::from(value as i16), i64::from((value >> 16) as i16))
}

/// The two 16×16 products used by the dual multiply forms
/// 双乘形式使用的两个 16×16 乘积
///
/// With `exchange` the second operand is rotated right by 16 bits before the
/// multiplications, which pairs `top × bottom` with `bottom × top`.
/// `exchange` 为真时先把第二操作数循环右移 16 位，使乘积变为
/// `高 × 低` 与 `低 × 高`。
fn dual_products(lhs: u32, rhs: u32, exchange: bool) -> (i64, i64) {
    let (lhs_low, lhs_high) = signed_halfwords(lhs);
    let (rhs_low, rhs_high) = signed_halfwords(rhs);
    if exchange {
        (lhs_high * rhs_low, lhs_low * rhs_high)
    } else {
        (lhs_low * rhs_low, lhs_high * rhs_high)
    }
}

/// Execute `SMLAD`/`SMLADX`/`SMLSD`/`SMLSDX`/`SMUAD`/`SMUADX`/`SMUSD`/`SMUSDX`
/// 执行 `SMLAD`/`SMLADX`/`SMLSD`/`SMLSDX`/`SMUAD`/`SMUADX`/`SMUSD`/`SMUSDX`
///
/// The instruction adds (or subtracts) two signed 16×16 products and optionally
/// the accumulator `Ra`. `Ra = 1111` selects the non-accumulating `SMUAD`/
/// `SMUSD` forms. `APSR.Q` is set exactly once, when the final sum does not fit
/// in 32 bits; the products alone cannot overflow. `NZCV` and `GE` are
/// preserved.
/// 指令把两个有符号 16×16 乘积相加（或相减），可选再累加 `Ra`。
/// `Ra = 1111` 表示非累加的 `SMUAD`/`SMUSD` 形式。`APSR.Q` 只在最终和不
/// 能放入 32 位时置位一次；乘积本身不会上溢。`NZCV` 与 `GE` 保持不变。
fn execute_dual(instr: u32, regs: &mut CortexM33Registers, subtract: bool) -> StepResult {
    let rn = ((instr >> 16) & 0xF) as u8;
    let ra = ((instr >> 12) & 0xF) as u8;
    let rd = ((instr >> 8) & 0xF) as u8;
    let control = ((instr >> 4) & 0xF) as u8;
    let rm = (instr & 0xF) as u8;
    // bits[7:5] are reserved-zero and bit 4 is the X exchange selector.
    // bits[7:5] 保留为零，位 4 为 X 交换选择位。
    if control & 0b1110 != 0 || rd == 15 || rn == 15 || rm == 15 {
        return StepResult::Undefined;
    }
    let (first, second) = dual_products(regs.read(rn), regs.read(rm), control & 1 != 0);
    let products = if subtract {
        first - second
    } else {
        first + second
    };
    let exact = if ra == 0xF {
        products
    } else {
        products + i64::from(regs.read(ra) as i32)
    };
    let result = exact as i32;
    if exact != i64::from(result) {
        regs.set_q_flag();
    }
    regs.write(rd, result as u32);
    StepResult::Ok
}

/// Execute `SMLALD`/`SMLALDX`/`SMLSLD`/`SMLSLDX`
/// 执行 `SMLALD`/`SMLALDX`/`SMLSLD`/`SMLSLDX`
///
/// The two signed 16×16 products are added (or subtracted) together with the
/// 64-bit accumulator `RdHi:RdLo`. `RdLo` is the low word (`bits[15:12]`),
/// `RdHi` the high word (`bits[11:8]`). The accumulation wraps modulo 2^64 and
/// no flag is updated.
/// 两个有符号 16×16 乘积与 64 位累加器 `RdHi:RdLo` 相加（或相减）。
/// `RdLo` 为低字（`bits[15:12]`），`RdHi` 为高字（`bits[11:8]`）。累加按
/// 模 2^64 回绕，不更新任何标志。
fn execute_dual_long(instr: u32, regs: &mut CortexM33Registers, subtract: bool) -> StepResult {
    let rn = ((instr >> 16) & 0xF) as u8;
    let rd_lo = ((instr >> 12) & 0xF) as u8;
    let rd_hi = ((instr >> 8) & 0xF) as u8;
    let control = ((instr >> 4) & 0xF) as u8;
    let rm = (instr & 0xF) as u8;
    // bits[7:5] are fixed 110 (it is what distinguishes the long dual forms
    // from SMLAL with bits[7:4] = 0000) and bit 4 is the X selector.
    // bits[7:5] 固定为 110（用于与 bits[7:4] = 0000 的 SMLAL 区分），
    // 位 4 为 X 交换选择位。
    if control & 0b1110 != 0b1100
        || rd_lo == 15
        || rd_hi == 15
        || rd_lo == rd_hi
        || rn == 15
        || rm == 15
    {
        return StepResult::Undefined;
    }
    let (first, second) = dual_products(regs.read(rn), regs.read(rm), control & 1 != 0);
    let products = if subtract {
        first - second
    } else {
        first + second
    };
    let accumulator = ((u64::from(regs.read(rd_hi)) << 32) | u64::from(regs.read(rd_lo))) as i64;
    let result = accumulator.wrapping_add(products) as u64;
    regs.write(rd_lo, result as u32);
    regs.write(rd_hi, (result >> 32) as u32);
    StepResult::Ok
}

/// Execute `SMLALBB`/`SMLALBT`/`SMLALTB`/`SMLALTT`
/// 执行 `SMLALBB`/`SMLALBT`/`SMLALTB`/`SMLALTT`
///
/// `RdHi:RdLo = RdHi:RdLo + (Rn.<x> * Rm.<y>)`, where `<x>`/`<y>` are `B`
/// (low halfword) or `T` (top halfword). `RdLo` is the low word
/// (`bits[15:12]`), `RdHi` the high word (`bits[11:8]`). The product is
/// infinite-precision signed and the accumulation wraps modulo 2^64, exactly
/// like the ARM pseudocode (`result<63:0> = product + SInt(R[dHi]:R[dLo])`),
/// so the 32-bit halves wrap independently of any flags. No flag is modified.
/// `RdHi:RdLo = RdHi:RdLo + (Rn.<x> * Rm.<y>)`，其中 `<x>`/`<y>` 为 `B`
/// （低半字）或 `T`（高半字）。`RdLo` 为低字（`bits[15:12]`），`RdHi` 为高字
/// （`bits[11:8]`）。乘积为无限精度有符号值，累加按模 2^64 回绕，与 ARM 伪
/// 代码（`result<63:0> = product + SInt(R[dHi]:R[dLo])`）完全一致，因此两个
/// 32 位半部独立回绕。不修改任何标志。
fn execute_smlal_halfwords(instr: u32, regs: &mut CortexM33Registers) -> StepResult {
    let rn = ((instr >> 16) & 0xF) as u8;
    let rd_lo = ((instr >> 12) & 0xF) as u8;
    let rd_hi = ((instr >> 8) & 0xF) as u8;
    let control = ((instr >> 4) & 0xF) as u8;
    let rm = (instr & 0xF) as u8;
    // bits[7:6] = 0b10 is the only allocated prefix; bit 5 selects Rn's top
    // halfword and bit 4 Rm's. PC operands and RdLo == RdHi are unpredictable.
    // bits[7:6] = 0b10 是唯一分配的编码前缀；位 5 选择 Rn 的高半字，位 4
    // 选择 Rm 的高半字。PC 操作数与 RdLo == RdHi 属不可预测编码。
    if control & 0b1100 != 0b1000
        || rd_lo == 15
        || rd_hi == 15
        || rd_lo == rd_hi
        || rn == 15
        || rm == 15
    {
        return StepResult::Undefined;
    }
    let n_half = if control & 0b10 != 0 {
        i64::from((regs.read(rn) >> 16) as i16)
    } else {
        i64::from(regs.read(rn) as i16)
    };
    let m_half = if control & 0b01 != 0 {
        i64::from((regs.read(rm) >> 16) as i16)
    } else {
        i64::from(regs.read(rm) as i16)
    };
    let product = n_half.wrapping_mul(m_half);
    let accumulator = (u64::from(regs.read(rd_hi)) << 32) | u64::from(regs.read(rd_lo));
    let result = (accumulator as i64).wrapping_add(product) as u64;
    regs.write(rd_lo, result as u32);
    regs.write(rd_hi, (result >> 32) as u32);
    StepResult::Ok
}

/// Execute `SMMLS`/`SMMLSR`
/// 执行 `SMMLS`/`SMMLSR`
///
/// `Rd = (Ra<<32 - Rn*Rm) >> 32`, with `0x8000_0000` added before the high word
/// is extracted when the rounding bit is set. `bits7_4` = 0b000R; no flag is
/// modified.
/// `Rd = (Ra<<32 - Rn*Rm) >> 32`，舍入位置位时在取高字前加 `0x8000_0000`。
/// `bits7_4` = 0b000R；不修改任何标志。
fn execute_smmls(instr: u32, regs: &mut CortexM33Registers) -> StepResult {
    let rn = ((instr >> 16) & 0xF) as u8;
    let ra = ((instr >> 12) & 0xF) as u8;
    let rd = ((instr >> 8) & 0xF) as u8;
    let control = ((instr >> 4) & 0xF) as u8;
    let rm = (instr & 0xF) as u8;
    if control & 0b1110 != 0 || rd == 15 || ra == 15 || rn == 15 || rm == 15 {
        return StepResult::Undefined;
    }
    let product = i64::from(regs.read(rn) as i32) * i64::from(regs.read(rm) as i32);
    let mut result = (i64::from(regs.read(ra) as i32) << 32).wrapping_sub(product);
    if control & 1 != 0 {
        result = result.wrapping_add(0x8000_0000);
    }
    regs.write(rd, (result >> 32) as u32);
    StepResult::Ok
}

/// Execute `SMULWB`/`SMULWT`/`SMLAWB`/`SMLAWT`
/// 执行 `SMULWB`/`SMULWT`/`SMLAWB`/`SMLAWT`
///
/// Multiplies the signed 32-bit `Rn` by the selected signed halfword of `Rm`
/// (`bits7_4` bit 4 selects the top half) and keeps the top word of the 48-bit
/// result. `Ra = 1111` selects the non-accumulating `SMULW` forms;
/// `SMLAW` sets `APSR.Q` when the accumulation does not fit in 48 bits.
/// 用有符号 32 位 `Rn` 乘以 `Rm` 选定的有符号半字（`bits7_4` 位 4 选择高
/// 半字），保留 48 位乘积的高字。`Ra = 1111` 表示非累加的 `SMULW` 形式；
/// `SMLAW` 在累加结果超出 48 位时置 `APSR.Q`。
fn execute_word_halfword(instr: u32, regs: &mut CortexM33Registers) -> StepResult {
    let rn = ((instr >> 16) & 0xF) as u8;
    let ra = ((instr >> 12) & 0xF) as u8;
    let rd = ((instr >> 8) & 0xF) as u8;
    let control = ((instr >> 4) & 0xF) as u8;
    let rm = (instr & 0xF) as u8;
    if control & 0b1110 != 0 || rd == 15 || rn == 15 || rm == 15 {
        return StepResult::Undefined;
    }
    let half = if control & 1 != 0 {
        i64::from((regs.read(rm) >> 16) as i16)
    } else {
        i64::from(regs.read(rm) as i16)
    };
    let product = i64::from(regs.read(rn) as i32) * half;
    if ra == 0xF {
        regs.write(rd, (product >> 16) as u32);
        return StepResult::Ok;
    }
    let exact = product + (i64::from(regs.read(ra) as i32) << 16);
    let top = exact >> 16;
    // Signed overflow: the high word must sign-extend back to the 48-bit sum.
    // 有符号上溢：高字必须能符号扩展回 48 位和。
    if top != i64::from(top as i32) {
        regs.set_q_flag();
    }
    regs.write(rd, top as u32);
    StepResult::Ok
}
