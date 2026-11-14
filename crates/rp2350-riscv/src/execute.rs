//! RV32IMAC instruction fetch, decode and execute / RV32IMAC 指令取指、译码与执行

use super::Hazard3Core;
use super::execute_c::{ZcmpOp, decompress, zcmp_op};
use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;

/// Opcode: load / 加载指令
const OP_LOAD: u32 = 0x03;
/// Opcode: op-imm / 立即数运算
const OP_IMM: u32 = 0x13;
/// Opcode: op-imm-32 / 32 位立即数运算
const OP_IMM_32: u32 = 0x1B;
/// Opcode: store / 存储指令
const OP_STORE: u32 = 0x23;
/// Opcode: register-register op / 寄存器-寄存器运算
const OP: u32 = 0x33;
/// Opcode: register-register op-32 / 32 位寄存器运算
const OP_32: u32 = 0x3B;
/// Opcode: lui / 加载高位立即数
const OP_LUI: u32 = 0x37;
/// Opcode: auipc / PC 相对高位立即数
const OP_AUIPC: u32 = 0x17;
/// Opcode: jal / 跳转并链接
const OP_JAL: u32 = 0x6F;
/// Opcode: jalr / 间接跳转并链接
const OP_JALR: u32 = 0x67;
/// Opcode: branch / 条件分支
const OP_BRANCH: u32 = 0x63;
/// Opcode: system / 系统指令
const OP_SYSTEM: u32 = 0x73;
/// Opcode: fence / 内存屏障
const OP_FENCE: u32 = 0x0F;
/// Opcode: atomic memory operation / 原子内存操作
const OP_AMO: u32 = 0x2F;

/// Internal trap raised while executing one instruction / 单条指令执行期间产生的内部陷阱
struct Trap {
    /// `mcause` value / 异常原因
    cause: u32,
    /// `mtval` value / 陷阱值
    tval: u32,
}

impl Trap {
    /// Build an illegal-instruction trap / 构造非法指令陷阱
    fn illegal(instr: u32) -> Self {
        Self {
            cause: 2,
            tval: instr,
        }
    }
}

/// Result of executing one instruction: next PC or a trap
/// 单条指令执行结果：下一条 PC 或陷阱
type ExecResult = Result<u32, Trap>;

impl Hazard3Core {
    /// Fetch, decode and execute one instruction, updating PC
    /// 取指、译码并执行一条指令，同时更新 PC
    pub(crate) fn execute_one(&mut self, bus: &mut dyn AddressBus) -> Result<(), CoreError> {
        let pc = self.registers.pc;
        let low = Self::fetch_u16(bus, pc)?;
        // Zcmp push/pop 宏是多条内存访问的组合，无法展开成单条 32 位指令，
        // 必须在压缩指令路径之前单独执行。
        // Zcmp push/pop macros combine several memory accesses and cannot be
        // expanded into one 32-bit instruction, so they run before the
        // compressed-instruction path.
        if low & 0b11 != 0b11 {
            if let Some(op) = zcmp_op(low) {
                return match self.execute_zcmp(op, bus, pc.wrapping_add(2)) {
                    Ok(target) => {
                        self.registers.pc = target;
                        Ok(())
                    }
                    Err(trap) => {
                        self.registers.pc = self.enter_trap(trap.cause, trap.tval, pc);
                        Ok(())
                    }
                };
            }
        }
        let (instr, length) = if low & 0b11 == 0b11 {
            let high = Self::fetch_u16(bus, pc.wrapping_add(2))?;
            (u32::from(low) | (u32::from(high) << 16), 4)
        } else if let Some(expanded) = decompress(low) {
            (expanded, 2)
        } else {
            self.registers.pc = self.enter_trap(2, u32::from(low), pc);
            return Ok(());
        };
        let next_pc = pc.wrapping_add(length);

        match self.execute_instr(instr, bus, pc, next_pc) {
            Ok(target) => {
                self.registers.pc = target;
                Ok(())
            }
            Err(trap) => {
                self.registers.pc = self.enter_trap(trap.cause, trap.tval, pc);
                Ok(())
            }
        }
    }

    /// Execute a Zcmp push/pop macro / 执行 Zcmp push/pop 宏
    ///
    /// `cm.push` stores the register list just below the old stack pointer and
    /// then allocates the frame; the pop forms load from the top of the frame
    /// and release it. `cm.popret`/`cm.popretz` additionally return to `ra`.
    /// `cm.push` 把寄存器列表保存在旧栈指针下方并分配栈帧；pop 形式从栈帧
    /// 顶部恢复并释放栈帧。`cm.popret`/`cm.popretz` 还会返回到 `ra`。
    fn execute_zcmp(&mut self, op: ZcmpOp, bus: &mut dyn AddressBus, next_pc: u32) -> ExecResult {
        let sp = self.registers.read(2);
        let stack_adj = op.stack_adj();
        match op {
            ZcmpOp::Push { .. } => {
                let mut addr = sp.wrapping_sub(4);
                for &reg in op.registers() {
                    let mut cycles = 0u64;
                    bus.write_u32(addr, self.registers.read(reg), &mut cycles)
                        .map_err(|_| Trap {
                            cause: 7,
                            tval: addr,
                        })?;
                    addr = addr.wrapping_sub(4);
                }
                self.registers.write(2, sp.wrapping_sub(stack_adj));
                Ok(next_pc)
            }
            ZcmpOp::Pop { .. } | ZcmpOp::PopRet { .. } | ZcmpOp::PopRetZ { .. } => {
                let mut addr = sp.wrapping_add(stack_adj).wrapping_sub(4);
                for &reg in op.registers() {
                    let mut cycles = 0u64;
                    let value = bus.read_u32(addr, &mut cycles).map_err(|_| Trap {
                        cause: 5,
                        tval: addr,
                    })?;
                    self.registers.write(reg, value);
                    addr = addr.wrapping_sub(4);
                }
                self.registers.write(2, sp.wrapping_add(stack_adj));
                if op.zeroes_a0() {
                    self.registers.write(10, 0);
                }
                if op.returns() {
                    return Ok(self.registers.read(1));
                }
                Ok(next_pc)
            }
        }
    }

    /// Fetch a 16-bit halfword / 取 16 位半字
    fn fetch_u16(bus: &mut dyn AddressBus, addr: u32) -> Result<u16, CoreError> {
        let mut cycles = 0u64;
        bus.read_u16(addr, &mut cycles)
    }

    /// Decode and execute one 32-bit instruction / 译码并执行一条 32 位指令
    fn execute_instr(
        &mut self,
        instr: u32,
        bus: &mut dyn AddressBus,
        pc: u32,
        next_pc: u32,
    ) -> ExecResult {
        let opcode = instr & 0x7F;
        let rd = ((instr >> 7) & 0x1F) as u8;
        let funct3 = (instr >> 12) & 0x7;
        let rs1 = ((instr >> 15) & 0x1F) as u8;
        let rs2 = ((instr >> 20) & 0x1F) as u8;
        let funct7 = (instr >> 25) & 0x7F;

        match opcode {
            OP_LUI => {
                self.registers.write(rd, instr & 0xFFFF_F000);
                Ok(next_pc)
            }
            OP_AUIPC => {
                self.registers
                    .write(rd, pc.wrapping_add(instr & 0xFFFF_F000));
                Ok(next_pc)
            }
            OP_JAL => {
                self.registers.write(rd, next_pc);
                Ok(pc.wrapping_add(imm_j(instr) as u32))
            }
            OP_JALR => {
                let target = self.registers.read(rs1).wrapping_add(imm_i(instr) as u32) & !1;
                self.registers.write(rd, next_pc);
                Ok(target)
            }
            OP_BRANCH => {
                let a = self.registers.read(rs1);
                let b = self.registers.read(rs2);
                let taken = match funct3 {
                    0 => a == b,
                    1 => a != b,
                    4 => (a as i32) < (b as i32),
                    5 => (a as i32) >= (b as i32),
                    6 => a < b,
                    7 => a >= b,
                    _ => return Err(Trap::illegal(instr)),
                };
                if taken {
                    Ok(pc.wrapping_add(imm_b(instr) as u32))
                } else {
                    Ok(next_pc)
                }
            }
            OP_LOAD => {
                let addr = self.registers.read(rs1).wrapping_add(imm_i(instr) as u32);
                let value = load_value(bus, addr, funct3)?;
                self.registers.write(rd, value);
                Ok(next_pc)
            }
            OP_STORE => {
                let addr = self.registers.read(rs1).wrapping_add(imm_s(instr) as u32);
                store_value(bus, addr, funct3, self.registers.read(rs2))?;
                Ok(next_pc)
            }
            OP_IMM => {
                let value = imm_i(instr) as u32;
                let source = self.registers.read(rs1);
                // Zbb/Zbs place extra funct7 encodings in the shift-immediate
                // group; only funct7 == 0 is a plain shift.
                // Zbb/Zbs 在移位立即数组中使用额外 funct7 编码；只有 funct7 == 0
                // 才是普通移位。
                let result = match funct3 {
                    0 => source.wrapping_add(value),
                    2 => u32::from((source as i32) < (value as i32)),
                    3 => u32::from(source < value),
                    4 => source ^ value,
                    6 => source | value,
                    7 => source & value,
                    1 => match funct7 {
                        0x00 => source << (rs2 & 0x1F),
                        0x14 => source | (1 << (rs2 & 0x1F)),
                        0x24 => source & !(1 << (rs2 & 0x1F)),
                        0x34 => source ^ (1 << (rs2 & 0x1F)),
                        0x30 => match rs2 {
                            0 => source.leading_zeros(),
                            1 => source.trailing_zeros(),
                            2 => source.count_ones(),
                            4 => sign_extend_byte(source),
                            5 => sign_extend_half(source),
                            _ => return Err(Trap::illegal(instr)),
                        },
                        _ => return Err(Trap::illegal(instr)),
                    },
                    5 => match funct7 {
                        0x00 => shift_right(source, u32::from(rs2 & 0x1F), false),
                        0x20 => shift_right(source, u32::from(rs2 & 0x1F), true),
                        0x24 => (source >> (rs2 & 0x1F)) & 1,
                        0x30 => source.rotate_right(u32::from(rs2 & 0x1F)),
                        0x14 if rs2 == 7 => orc_b(source),
                        0x34 if rs2 == 0x18 => source.swap_bytes(),
                        0x34 if rs2 == 7 => brev8(source),
                        _ => return Err(Trap::illegal(instr)),
                    },
                    _ => return Err(Trap::illegal(instr)),
                };
                self.registers.write(rd, result);
                Ok(next_pc)
            }
            OP => {
                let a = self.registers.read(rs1);
                let b = self.registers.read(rs2);
                let result = if funct7 == 0x01 {
                    multiply_divide(a, b, funct3)?
                } else {
                    match funct3 {
                        0 if funct7 == 0x20 => a.wrapping_sub(b),
                        0 => a.wrapping_add(b),
                        // Zbs single-bit operations. Zbs 单位操作。
                        1 if funct7 == 0x14 => a | (1 << (b & 0x1F)),
                        1 if funct7 == 0x24 => a & !(1 << (b & 0x1F)),
                        1 if funct7 == 0x34 => a ^ (1 << (b & 0x1F)),
                        1 if funct7 == 0x30 => a.rotate_left(b & 0x1F),
                        1 => a << (b & 0x1F),
                        // Zba address generation. Zba 地址生成。
                        2 if funct7 == 0x10 => (a << 1).wrapping_add(b),
                        2 => u32::from((a as i32) < (b as i32)),
                        3 => u32::from(a < b),
                        // Zbb and/or/xor-with-inverted-second-operand, Zba sh2add,
                        // min/max and PACK share funct3 == 4. PACK with rs2 == 0 is
                        // the canonical ZEXT.H encoding emitted by GCC.
                        // Zbb 与第二操作数取反后的 and/or/xor、Zba sh2add、min/max
                        // 与 PACK 共用 funct3 == 4。rs2 == 0 的 PACK 即 GCC 使用的
                        // ZEXT.H 规范编码。
                        4 if funct7 == 0x04 => (a & 0xFFFF) | ((b & 0xFFFF) << 16),
                        4 if funct7 == 0x10 => (a << 2).wrapping_add(b),
                        4 if funct7 == 0x05 => {
                            if (a as i32) < (b as i32) {
                                a
                            } else {
                                b
                            }
                        }
                        4 if funct7 == 0x20 => a ^ !b,
                        4 => a ^ b,
                        5 if funct7 == 0x05 => {
                            if a < b {
                                a
                            } else {
                                b
                            }
                        }
                        5 if funct7 == 0x24 => (a >> (b & 0x1F)) & 1,
                        5 if funct7 == 0x30 => a.rotate_right(b & 0x1F),
                        5 => shift_right(a, b & 0x1F, funct7 == 0x20),
                        // Zba sh3add and min/max share funct3 == 6.
                        // Zba sh3add 与 min/max 共用 funct3 == 6。
                        6 if funct7 == 0x10 => (a << 3).wrapping_add(b),
                        6 if funct7 == 0x05 => {
                            if (a as i32) > (b as i32) {
                                a
                            } else {
                                b
                            }
                        }
                        6 if funct7 == 0x20 => a | !b,
                        6 => a | b,
                        7 if funct7 == 0x04 => (a & 0xFF) | ((b & 0xFF) << 8),
                        7 if funct7 == 0x05 => {
                            if a > b {
                                a
                            } else {
                                b
                            }
                        }
                        7 if funct7 == 0x20 => a & !b,
                        7 => a & b,
                        _ => return Err(Trap::illegal(instr)),
                    }
                };
                self.registers.write(rd, result);
                Ok(next_pc)
            }
            OP_IMM_32 => {
                let source = self.registers.read(rs1);
                let shift = u32::from(rs2 & 0x1F);
                let result = match funct3 {
                    0 => sign_extend_word(source.wrapping_add(imm_i(instr) as u32)),
                    1 if funct7 == 0 => sign_extend_word(source << shift),
                    5 => sign_extend_word(shift_right(source, shift, funct7 == 0x20)),
                    _ => return Err(Trap::illegal(instr)),
                };
                self.registers.write(rd, result);
                Ok(next_pc)
            }
            OP_32 => {
                let a = self.registers.read(rs1);
                let b = self.registers.read(rs2);
                let result = if funct7 == 0x01 {
                    let word = multiply_divide(a, b, funct3)?;
                    sign_extend_word(word)
                } else {
                    match funct3 {
                        0 if funct7 == 0x20 => sign_extend_word(a.wrapping_sub(b)),
                        0 => sign_extend_word(a.wrapping_add(b)),
                        1 => sign_extend_word(a << (b & 0x1F)),
                        2 => u32::from((a as i32) < (b as i32)),
                        3 => u32::from(a < b),
                        4 => a ^ b,
                        5 => sign_extend_word(shift_right(a, b & 0x1F, funct7 == 0x20)),
                        6 => a | b,
                        7 => a & b,
                        _ => return Err(Trap::illegal(instr)),
                    }
                };
                self.registers.write(rd, result);
                Ok(next_pc)
            }
            OP_FENCE => Ok(next_pc),
            OP_AMO => self.execute_atomic(instr, funct7, funct3, rd, rs1, rs2, bus, next_pc),
            OP_SYSTEM => self.execute_system(instr, funct3, rd, rs1, next_pc),
            _ => Err(Trap::illegal(instr)),
        }
    }

    /// Execute an A-extension atomic operation / 执行 A 扩展原子操作
    fn execute_atomic(
        &mut self,
        instr: u32,
        funct7: u32,
        funct3: u32,
        rd: u8,
        rs1: u8,
        rs2: u8,
        bus: &mut dyn AddressBus,
        next_pc: u32,
    ) -> ExecResult {
        if funct3 != 0b010 {
            return Err(Trap::illegal(instr));
        }
        let addr = self.registers.read(rs1);
        let funct5 = funct7 >> 2;
        match funct5 {
            0b00010 => {
                self.reservation = Some(addr);
                self.registers.write(rd, load_word(bus, addr)?);
            }
            0b00011 => {
                let success = self.reservation == Some(addr);
                if success {
                    store_word(bus, addr, self.registers.read(rs2))?;
                }
                self.registers.write(rd, u32::from(!success));
                self.reservation = None;
            }
            _ => {
                let old = load_word(bus, addr)?;
                let operand = self.registers.read(rs2);
                let new = match funct5 {
                    0b00000 => old.wrapping_add(operand),
                    0b00001 => operand,
                    0b00100 => old ^ operand,
                    0b01100 => old & operand,
                    0b01000 => old | operand,
                    0b10000 => (old as i32).min(operand as i32) as u32,
                    0b10100 => (old as i32).max(operand as i32) as u32,
                    0b11000 => old.min(operand),
                    0b11100 => old.max(operand),
                    _ => return Err(Trap::illegal(instr)),
                };
                store_word(bus, addr, new)?;
                self.registers.write(rd, old);
            }
        }
        Ok(next_pc)
    }

    /// Execute a SYSTEM instruction / 执行 SYSTEM 指令
    fn execute_system(
        &mut self,
        instr: u32,
        funct3: u32,
        rd: u8,
        rs1: u8,
        next_pc: u32,
    ) -> ExecResult {
        match instr {
            0x0000_0073 => return Err(Trap { cause: 11, tval: 0 }),
            0x0010_0073 => return Err(Trap { cause: 3, tval: 0 }),
            0x3020_0073 => return Ok(self.csr.mret()),
            // WFI is treated as a NOP so single-core bring-up does not deadlock.
            // WFI 视为 NOP，避免单核引导时死锁。
            0x1050_0073 => return Ok(next_pc),
            _ => {}
        }
        if !matches!(funct3, 1..=3 | 5..=7) {
            return Err(Trap::illegal(instr));
        }

        let csr_addr = instr >> 20;
        let source = if funct3 >= 5 {
            u32::from(rs1)
        } else {
            self.registers.read(rs1)
        };
        // Xh3irq array CSRs use the operand's low five bits as a window index
        // and the upper 16 bits as data, so the read needs the raw operand.
        // Xh3irq 数组 CSR 使用操作数低 5 位作为窗口索引、高 16 位作为数据，
        // 因此读取时需要原始操作数。
        let old = self.csr.read(csr_addr, source);
        let operation = funct3 & 3;
        let new_value = match operation {
            1 => source,
            2 => old | source,
            3 => old & !source,
            _ => return Err(Trap::illegal(instr)),
        };
        let should_write = operation == 1 || rs1 != 0;
        if should_write {
            if (csr_addr >> 10) & 3 == 3 {
                return Err(Trap::illegal(instr));
            }
            self.csr.write_raw(csr_addr, new_value, source);
        }
        self.registers.write(rd, old);
        Ok(next_pc)
    }
}

/// Read a load value by `funct3` / 按 `funct3` 读取加载值
fn load_value(bus: &mut dyn AddressBus, addr: u32, funct3: u32) -> Result<u32, Trap> {
    let mut cycles = 0u64;
    let fault = Trap {
        cause: 5,
        tval: addr,
    };
    match funct3 {
        // LB/LH sign-extend to XLEN; LBU/LHU below zero-extend.
        // LB/LH 符号扩展到 XLEN；下面的 LBU/LHU 做零扩展。
        0 => bus
            .read_u8(addr, &mut cycles)
            .map(|v| v as i8 as u32)
            .map_err(|_| fault),
        1 => bus
            .read_u16(addr, &mut cycles)
            .map(|v| v as i16 as u32)
            .map_err(|_| fault),
        2 => bus.read_u32(addr, &mut cycles).map_err(|_| fault),
        4 => bus
            .read_u8(addr, &mut cycles)
            .map(u32::from)
            .map_err(|_| fault),
        5 => bus
            .read_u16(addr, &mut cycles)
            .map(u32::from)
            .map_err(|_| fault),
        _ => Err(Trap {
            cause: 2,
            tval: funct3,
        }),
    }
}

/// Write a store value by `funct3` / 按 `funct3` 写入存储值
fn store_value(bus: &mut dyn AddressBus, addr: u32, funct3: u32, value: u32) -> Result<(), Trap> {
    let mut cycles = 0u64;
    let fault = Trap {
        cause: 7,
        tval: addr,
    };
    match funct3 {
        0 => bus
            .write_u8(addr, value as u8, &mut cycles)
            .map_err(|_| fault),
        1 => bus
            .write_u16(addr, value as u16, &mut cycles)
            .map_err(|_| fault),
        2 => bus.write_u32(addr, value, &mut cycles).map_err(|_| fault),
        _ => Err(Trap {
            cause: 2,
            tval: funct3,
        }),
    }
}

/// Load a 32-bit word for atomics / 为原子操作加载 32 位字
fn load_word(bus: &mut dyn AddressBus, addr: u32) -> Result<u32, Trap> {
    let mut cycles = 0u64;
    bus.read_u32(addr, &mut cycles).map_err(|_| Trap {
        cause: 5,
        tval: addr,
    })
}

/// Store a 32-bit word for atomics / 为原子操作存储 32 位字
fn store_word(bus: &mut dyn AddressBus, addr: u32, value: u32) -> Result<(), Trap> {
    let mut cycles = 0u64;
    bus.write_u32(addr, value, &mut cycles).map_err(|_| Trap {
        cause: 7,
        tval: addr,
    })
}

/// Arithmetic/logical right shift / 算术或逻辑右移
fn shift_right(value: u32, shift: u32, arithmetic: bool) -> u32 {
    if arithmetic {
        (value as i32 >> shift) as u32
    } else {
        value >> shift
    }
}

/// Sign-extend the low 32 bits / 将低 32 位符号扩展
fn sign_extend_word(value: u32) -> u32 {
    value as i32 as u32
}

/// Sign-extend the low byte / 将低字节符号扩展
fn sign_extend_byte(value: u32) -> u32 {
    value as i8 as u32
}

/// Sign-extend the low halfword / 将低半字符号扩展
fn sign_extend_half(value: u32) -> u32 {
    value as i16 as u32
}

/// OR-combine the sign bit of every byte / 将每字节符号位扩展为整字节掩码
fn orc_b(value: u32) -> u32 {
    let mut result = 0u32;
    for shift in (0..32).step_by(8) {
        if value & (0x80 << shift) != 0 {
            result |= 0xFF << shift;
        }
    }
    result
}

/// Reverse the bit order within every byte / 反转每字节内的位序
fn brev8(value: u32) -> u32 {
    let mut result = 0u32;
    for shift in (0..32).step_by(8) {
        let byte = ((value >> shift) & 0xFF) as u8;
        result |= u32::from(byte.reverse_bits()) << shift;
    }
    result
}

/// Execute an M-extension operation / 执行 M 扩展运算
fn multiply_divide(a: u32, b: u32, funct3: u32) -> Result<u32, Trap> {
    let result = match funct3 {
        0 => a.wrapping_mul(b),
        1 => ((a as i32 as i64 * b as i32 as i64) >> 32) as u32,
        2 => (((a as i32 as i128) * (b as i128)) >> 32) as u32,
        3 => ((u64::from(a) * u64::from(b)) >> 32) as u32,
        4 => {
            if b == 0 {
                u32::MAX
            } else if a == i32::MIN as u32 && b == u32::MAX {
                a
            } else {
                ((a as i32) / (b as i32)) as u32
            }
        }
        5 => a.checked_div(b).unwrap_or(u32::MAX),
        6 => {
            if b == 0 {
                a
            } else if a == i32::MIN as u32 && b == u32::MAX {
                0
            } else {
                ((a as i32) % (b as i32)) as u32
            }
        }
        7 => a.checked_rem(b).unwrap_or(a),
        _ => {
            return Err(Trap {
                cause: 2,
                tval: funct3,
            });
        }
    };
    Ok(result)
}

/// I-type signed immediate / I 型有符号立即数
fn imm_i(instr: u32) -> i32 {
    (instr as i32) >> 20
}

/// S-type signed immediate / S 型有符号立即数
fn imm_s(instr: u32) -> i32 {
    let value = ((instr >> 25) << 5) | ((instr >> 7) & 0x1F);
    ((value << 20) as i32) >> 20
}

/// B-type signed immediate / B 型有符号立即数
fn imm_b(instr: u32) -> i32 {
    let value = ((instr >> 31) & 1) << 12
        | ((instr >> 7) & 1) << 11
        | ((instr >> 25) & 0x3F) << 5
        | ((instr >> 8) & 0xF) << 1;
    ((value << 19) as i32) >> 19
}

/// J-type signed immediate / J 型有符号立即数
fn imm_j(instr: u32) -> i32 {
    let value = ((instr >> 31) & 1) << 20
        | ((instr >> 12) & 0xFF) << 12
        | ((instr >> 20) & 1) << 11
        | ((instr >> 21) & 0x3FF) << 1;
    ((value << 11) as i32) >> 11
}
