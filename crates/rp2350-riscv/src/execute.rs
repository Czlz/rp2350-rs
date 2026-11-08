//! RV32IMAC instruction fetch, decode and execute / RV32IMAC 指令取指、译码与执行

use super::Hazard3Core;
use super::execute_c::decompress;
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
        let (instr, length) = if low & 0b11 == 0b11 {
            let high = Self::fetch_u16(bus, pc.wrapping_add(2))?;
            (u32::from(low) | (u32::from(high) << 16), 4)
        } else if let Some(expanded) = decompress(low) {
            (expanded, 2)
        } else {
            self.registers.pc = self.csr.take_trap(2, u32::from(low), pc);
            return Ok(());
        };
        let next_pc = pc.wrapping_add(length);

        match self.execute_instr(instr, bus, pc, next_pc) {
            Ok(target) => {
                self.registers.pc = target;
                Ok(())
            }
            Err(trap) => {
                self.registers.pc = self.csr.take_trap(trap.cause, trap.tval, pc);
                Ok(())
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
                let result = match funct3 {
                    0 => source.wrapping_add(value),
                    2 => u32::from((source as i32) < (value as i32)),
                    3 => u32::from(source < value),
                    4 => source ^ value,
                    6 => source | value,
                    7 => source & value,
                    1 => source << (rs2 & 0x1F),
                    5 => shift_right(source, u32::from(rs2 & 0x1F), funct7 == 0x20),
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
                        1 => a << (b & 0x1F),
                        2 => u32::from((a as i32) < (b as i32)),
                        3 => u32::from(a < b),
                        4 => a ^ b,
                        5 => shift_right(a, b & 0x1F, funct7 == 0x20),
                        6 => a | b,
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
        let old = self.csr.read(csr_addr);
        let source = if funct3 >= 5 {
            u32::from(rs1)
        } else {
            self.registers.read(rs1)
        };
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
            self.csr.write(csr_addr, new_value);
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
        0 => bus
            .read_u8(addr, &mut cycles)
            .map(|v| u32::from(v as i8 as u8))
            .map_err(|_| fault),
        1 => bus
            .read_u16(addr, &mut cycles)
            .map(|v| u32::from(v as i16 as u16))
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
