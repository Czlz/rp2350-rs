//! RV32C compressed-instruction expansion / RV32C 压缩指令展开
//!
//! Compressed instructions are expanded into their 32-bit equivalents, then the
//! normal decoder executes them. This keeps one source of truth for semantics.
//! The Zcmp push/pop macros are decoded separately because one 16-bit macro
//! performs several memory accesses and cannot be expressed as a single 32-bit
//! instruction. / 压缩指令先展开为等价的 32 位指令，再由常规译码器执行，从而
//! 保证语义只有一份实现。Zcmp push/pop 宏单独译码，因为一条 16 位宏指令会执行
//! 多次内存访问，无法表示为单条 32 位指令。

/// Zcmp stack-adjustment base for each register-list index / Zcmp 每个寄存器列表的栈调整基准
const ZCMP_STACK_ADJ: [u32; 16] = [0, 0, 0, 0, 16, 16, 16, 16, 32, 32, 32, 32, 48, 48, 48, 64];

/// Zcmp register lists, highest register first (higher address first)
/// Zcmp 寄存器列表，高编号寄存器在前（对应更高地址）
const ZCMP_REG_LISTS: [&[u8]; 16] = [
    &[],
    &[],
    &[],
    &[],
    &[1],
    &[8, 1],
    &[9, 8, 1],
    &[18, 9, 8, 1],
    &[19, 18, 9, 8, 1],
    &[20, 19, 18, 9, 8, 1],
    &[21, 20, 19, 18, 9, 8, 1],
    &[22, 21, 20, 19, 18, 9, 8, 1],
    &[23, 22, 21, 20, 19, 18, 9, 8, 1],
    &[24, 23, 22, 21, 20, 19, 18, 9, 8, 1],
    &[25, 24, 23, 22, 21, 20, 19, 18, 9, 8, 1],
    &[27, 26, 25, 24, 23, 22, 21, 20, 19, 18, 9, 8, 1],
];

/// Decoded Zcmp push/pop macro / 已译码的 Zcmp push/pop 宏
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ZcmpOp {
    /// `cm.push {reg_list}, -stack_adj` / 建立栈帧
    Push { rlist: u8, spimm: u8 },
    /// `cm.pop {reg_list}, stack_adj` / 销毁栈帧
    Pop { rlist: u8, spimm: u8 },
    /// `cm.popret {reg_list}, stack_adj` / 销毁栈帧并返回
    PopRet { rlist: u8, spimm: u8 },
    /// `cm.popretz {reg_list}, stack_adj` / 销毁栈帧、清零 a0 并返回
    PopRetZ { rlist: u8, spimm: u8 },
}

impl ZcmpOp {
    /// Registers saved or restored, highest address first
    /// 保存或恢复的寄存器，最高地址在前
    pub(crate) fn registers(self) -> &'static [u8] {
        let rlist = match self {
            Self::Push { rlist, .. }
            | Self::Pop { rlist, .. }
            | Self::PopRet { rlist, .. }
            | Self::PopRetZ { rlist, .. } => rlist,
        };
        ZCMP_REG_LISTS[usize::from(rlist) & 0xF]
    }

    /// Total stack frame size in bytes / 栈帧总字节数
    ///
    /// The base adjustment depends on the register list; the two `spimm` bits
    /// add a multiple of 16 bytes for larger frames. / 基准调整取决于寄存器
    /// 列表；两位 `spimm` 以 16 字节为单位追加更大的栈帧。
    pub(crate) fn stack_adj(self) -> u32 {
        let (rlist, spimm) = match self {
            Self::Push { rlist, spimm }
            | Self::Pop { rlist, spimm }
            | Self::PopRet { rlist, spimm }
            | Self::PopRetZ { rlist, spimm } => (rlist, spimm),
        };
        ZCMP_STACK_ADJ[usize::from(rlist) & 0xF] + u32::from(spimm) * 16
    }

    /// Whether the macro returns to `ra` / 宏是否返回到 `ra`
    pub(crate) fn returns(self) -> bool {
        matches!(self, Self::PopRet { .. } | Self::PopRetZ { .. })
    }

    /// Whether the macro moves zero into `a0` / 宏是否把 0 写入 `a0`
    pub(crate) fn zeroes_a0(self) -> bool {
        matches!(self, Self::PopRetZ { .. })
    }
}

/// Decode a Zcmp push/pop macro / 译码一条 Zcmp push/pop 宏
///
/// Zcmp reuses the `c.j` encoding space (quadrant 2, funct3 = 5). Register
/// lists 0..3 are reserved, so they are left to the illegal-instruction path.
/// Zcmp 复用 `c.j` 的编码空间（quadrant 2，funct3 = 5）。寄存器列表 0..3 为
/// 保留值，交由非法指令路径处理。
pub(crate) fn zcmp_op(half: u16) -> Option<ZcmpOp> {
    let masked = half & 0xFF03;
    let rlist = u8::try_from((half >> 4) & 0xF).unwrap();
    if rlist < 4 {
        return None;
    }
    let spimm = u8::try_from((half >> 2) & 0x3).unwrap();
    match masked {
        0xB802 => Some(ZcmpOp::Push { rlist, spimm }),
        0xBA02 => Some(ZcmpOp::Pop { rlist, spimm }),
        0xBC02 => Some(ZcmpOp::PopRetZ { rlist, spimm }),
        0xBE02 => Some(ZcmpOp::PopRet { rlist, spimm }),
        _ => None,
    }
}

/// Expand a 16-bit compressed instruction to its 32-bit equivalent
/// 将 16 位压缩指令展开为等价的 32 位指令
///
/// Returns `None` for reserved, floating-point, and unsupported instructions.
/// 对保留、浮点及尚未支持的指令返回 `None`。
pub(crate) fn decompress(half: u16) -> Option<u32> {
    match half & 0b11 {
        0 => quadrant0(half),
        1 => quadrant1(half),
        2 => quadrant2(half),
        _ => None,
    }
}

/// Compressed quadrant 0 / 压缩指令象限 0
fn quadrant0(half: u16) -> Option<u32> {
    let funct3 = (half >> 13) & 0b111;
    let rd_short = reg(((half >> 2) & 0b111) + 8);
    let rs1 = reg(((half >> 7) & 0b111) + 8);
    match funct3 {
        0 => {
            let immediate = addi4spn_imm(half);
            if immediate == 0 {
                return None;
            }
            Some(encode_i(0x13, rd_short, 0, 2, immediate))
        }
        2 => Some(encode_i(0x03, rd_short, 2, rs1, lw_sw_imm(half))),
        4 => zcb(half),
        6 => Some(encode_s(0x23, 2, rs1, rd_short, lw_sw_imm(half))),
        _ => None,
    }
}

/// Zcb byte/halfword load and store instructions / Zcb 字节、半字加载与存储指令
fn zcb(half: u16) -> Option<u32> {
    let sub = (half >> 10) & 0b111;
    let rd = reg(((half >> 2) & 0b111) + 8);
    let rs1 = reg(((half >> 7) & 0b111) + 8);
    let rs2 = reg(((half >> 2) & 0b111) + 8);
    let byte_imm = i32::from(((half >> 5) & 1) << 1 | ((half >> 6) & 1));
    let half_imm = i32::from(((half >> 5) & 1) << 1);
    match sub {
        0 => Some(encode_i(0x03, rd, 4, rs1, byte_imm)),
        1 => {
            let funct3 = if (half >> 6) & 1 == 0 { 5 } else { 1 };
            Some(encode_i(0x03, rd, funct3, rs1, half_imm))
        }
        2 => Some(encode_s(0x23, 0, rs1, rs2, byte_imm)),
        3 => Some(encode_s(0x23, 1, rs1, rs2, half_imm)),
        _ => None,
    }
}

/// Compressed quadrant 1 / 压缩指令象限 1
fn quadrant1(half: u16) -> Option<u32> {
    let funct3 = (half >> 13) & 0b111;
    let rd = reg((half >> 7) & 0x1F);
    match funct3 {
        0 => {
            let immediate = ci_imm(half);
            Some(encode_i(0x13, rd, 0, rd, immediate))
        }
        1 => Some(encode_j(0x6F, 1, cj_imm(half))),
        2 => Some(encode_i(0x13, rd, 0, 0, ci_imm(half))),
        3 => {
            if rd == 2 {
                let immediate = addi16sp_imm(half);
                if immediate == 0 {
                    return None;
                }
                Some(encode_i(0x13, 2, 0, 2, immediate))
            } else {
                let immediate = lui_imm(half);
                if immediate == 0 {
                    return None;
                }
                Some(encode_u(0x37, rd, immediate))
            }
        }
        4 => arithmetic(half),
        5 => Some(encode_j(0x6F, 0, cj_imm(half))),
        6 => Some(encode_b(
            0x63,
            0,
            reg(((half >> 7) & 0b111) + 8),
            0,
            cb_imm(half),
        )),
        7 => Some(encode_b(
            0x63,
            1,
            reg(((half >> 7) & 0b111) + 8),
            0,
            cb_imm(half),
        )),
        _ => None,
    }
}

/// Compressed register-register and shift group / 压缩寄存器运算与移位组
fn arithmetic(half: u16) -> Option<u32> {
    let funct2 = (half >> 10) & 0b11;
    let rd = reg(((half >> 7) & 0b111) + 8);
    let rs2 = reg(((half >> 2) & 0b111) + 8);
    let shamt = u32::from(((half >> 12) & 1) << 5 | ((half >> 2) & 0x1F));
    let funct6 = (half >> 10) & 0b111_111;
    match funct2 {
        0 => {
            if (half >> 12) & 1 != 0 {
                return None;
            }
            Some(encode_i(0x13, rd, 5, rd, shamt as i32))
        }
        1 => {
            if (half >> 12) & 1 != 0 {
                return None;
            }
            Some(encode_shift_i(0x13, rd, 5, rd, shamt, 0x20))
        }
        2 => Some(encode_i(0x13, rd, 7, rd, ci_imm(half))),
        3 => match (funct6, (half >> 5) & 0b11) {
            (0b10_0011, 0) => Some(encode_r(0x33, rd, 0, rd, rs2, 0x20)),
            (0b10_0011, 1) => Some(encode_r(0x33, rd, 4, rd, rs2, 0)),
            (0b10_0011, 2) => Some(encode_r(0x33, rd, 6, rd, rs2, 0)),
            (0b10_0011, 3) => Some(encode_r(0x33, rd, 7, rd, rs2, 0)),
            // Zcb: C.MUL and the C.ZEXT.B/H / C.SEXT.B/H / C.NOT group.
            // Zcb：C.MUL 以及 C.ZEXT.B/H、C.SEXT.B/H、C.NOT 组。
            (0b10_0111, 2) => Some(encode_r(0x33, rd, 0, rd, rs2, 0x01)),
            (0b10_0111, 3) => zcb_zext(half),
            _ => None,
        },
        _ => None,
    }
}

/// Expand the Zcb C.ZEXT.B/H, C.SEXT.B/H and C.NOT subgroup
/// 展开 Zcb 的 C.ZEXT.B/H、C.SEXT.B/H 与 C.NOT 子组
fn zcb_zext(half: u16) -> Option<u32> {
    let rd = reg(((half >> 7) & 0b111) + 8);
    match (half >> 2) & 0b111 {
        0 => Some(encode_i(0x13, rd, 7, rd, 0xFF)),
        1 => Some(encode_shift_i(0x13, rd, 1, rd, 4, 0x30)),
        2 => Some(encode_r(0x33, rd, 4, rd, 0, 0x04)),
        3 => Some(encode_shift_i(0x13, rd, 1, rd, 5, 0x30)),
        5 => Some(encode_i(0x13, rd, 4, rd, -1)),
        _ => None,
    }
}

/// Compressed quadrant 2 / 压缩指令象限 2
fn quadrant2(half: u16) -> Option<u32> {
    let funct3 = (half >> 13) & 0b111;
    let rd = reg((half >> 7) & 0x1F);
    let rs2 = reg((half >> 2) & 0x1F);
    match funct3 {
        0 => {
            let shamt = u32::from(((half >> 12) & 1) << 5 | ((half >> 2) & 0x1F));
            if (half >> 12) & 1 != 0 {
                return None;
            }
            Some(encode_i(0x13, rd, 1, rd, shamt as i32))
        }
        2 => {
            if rd == 0 {
                return None;
            }
            Some(encode_i(0x03, rd, 2, 2, lwsp_imm(half)))
        }
        4 => {
            let funct4 = (half >> 12) & 0xF;
            match funct4 {
                0b1000 => {
                    if rs2 == 0 {
                        if rd == 0 {
                            None
                        } else {
                            Some(encode_i(0x67, 0, 0, rd, 0))
                        }
                    } else if rd == 0 {
                        Some(encode_i(0x13, 0, 0, 0, 0))
                    } else {
                        Some(encode_r(0x33, rd, 0, 0, rs2, 0))
                    }
                }
                0b1001 => {
                    if rd == 0 && rs2 == 0 {
                        Some(0x0010_0073)
                    } else if rs2 == 0 {
                        Some(encode_i(0x67, 1, 0, rd, 0))
                    } else if rd == 0 {
                        Some(encode_i(0x13, 0, 0, 0, 0))
                    } else {
                        Some(encode_r(0x33, rd, 0, rd, rs2, 0))
                    }
                }
                _ => None,
            }
        }
        6 => Some(encode_s(0x23, 2, 2, rs2, swsp_imm(half))),
        _ => None,
    }
}

/// Decode the 6-bit CI immediate / 解码 6 位 CI 立即数
fn ci_imm(half: u16) -> i32 {
    sign_extend(u32::from(((half >> 12) & 1) << 5 | ((half >> 2) & 0x1F)), 6)
}

/// Decode the C.ADDI4SPN immediate / 解码 C.ADDI4SPN 立即数
fn addi4spn_imm(half: u16) -> i32 {
    let value = ((half >> 7) & 0xF) << 6
        | ((half >> 11) & 0b11) << 4
        | ((half >> 5) & 1) << 3
        | ((half >> 6) & 1) << 2;
    i32::from(value)
}

/// Decode the C.LW / C.SW offset / 解码 C.LW / C.SW 偏移
fn lw_sw_imm(half: u16) -> i32 {
    i32::from(((half >> 10) & 0b111) << 3 | ((half >> 6) & 1) << 2 | ((half >> 5) & 1) << 6)
}

/// Decode the C.LWSP offset / 解码 C.LWSP 偏移
fn lwsp_imm(half: u16) -> i32 {
    i32::from(((half >> 2) & 0b11) << 6 | ((half >> 4) & 0b111) << 2 | ((half >> 12) & 1) << 5)
}

/// Decode the C.SWSP offset / 解码 C.SWSP 偏移
fn swsp_imm(half: u16) -> i32 {
    i32::from(((half >> 7) & 0b11) << 6 | ((half >> 9) & 0xF) << 2)
}

/// Decode the C.ADDI16SP immediate / 解码 C.ADDI16SP 立即数
fn addi16sp_imm(half: u16) -> i32 {
    let value = ((half >> 12) & 1) << 9
        | ((half >> 3) & 0b11) << 7
        | ((half >> 5) & 1) << 6
        | ((half >> 2) & 1) << 5
        | ((half >> 6) & 1) << 4;
    sign_extend(u32::from(value), 10)
}

/// Decode the sign-extended C.LUI immediate positioned at bits 31:12
/// 解码 C.LUI 立即数，符号扩展到 32 位并放置在位 31:12
///
/// C.LUI carries a signed 6-bit immediate: bit 17 is the sign bit, so a
/// negative immediate must fill bits 31:18 with ones before the 12-bit
/// shift. Otherwise `c.lui rd, -16` would load `0x0003_0000` instead of
/// `0xFFFF_0000`.
/// C.LUI 携带 6 位有符号立即数：位 17 是符号位，负立即数在左移 12 位前
/// 必须把位 31:18 填为 1。否则 `c.lui rd, -16` 会装入 `0x0003_0000`
/// 而不是 `0xFFFF_0000`。
fn lui_imm(half: u16) -> u32 {
    let raw = u32::from(((half >> 12) & 1) << 5 | ((half >> 2) & 0x1F));
    (sign_extend(raw, 6) as u32) << 12
}

/// Decode the C.J / C.JAL offset / 解码 C.J / C.JAL 偏移
fn cj_imm(half: u16) -> i32 {
    let value = ((half >> 12) & 1) << 11
        | ((half >> 11) & 1) << 4
        | ((half >> 9) & 0b11) << 8
        | ((half >> 8) & 1) << 10
        | ((half >> 7) & 1) << 6
        | ((half >> 6) & 1) << 7
        | ((half >> 3) & 0b111) << 1
        | ((half >> 2) & 1) << 5;
    sign_extend(u32::from(value), 12)
}

/// Decode the C.BEQZ / C.BNEZ offset / 解码 C.BEQZ / C.BNEZ 偏移
fn cb_imm(half: u16) -> i32 {
    let value = ((half >> 12) & 1) << 8
        | ((half >> 10) & 0b11) << 3
        | ((half >> 5) & 0b11) << 6
        | ((half >> 3) & 0b11) << 1
        | ((half >> 2) & 1) << 5;
    sign_extend(u32::from(value), 9)
}

/// Sign-extend a value of `bits` width / 对 `bits` 位宽的值做符号扩展
fn sign_extend(value: u32, bits: u32) -> i32 {
    let shift = 32 - bits;
    ((value << shift) as i32) >> shift
}

/// Compressed register number / 压缩格式寄存器号
fn reg(index: u16) -> u32 {
    u32::from(index & 0x1F)
}

/// Encode an I-type instruction / 编码 I 型指令
fn encode_i(opcode: u32, rd: u32, funct3: u32, rs1: u32, immediate: i32) -> u32 {
    opcode | (rd << 7) | (funct3 << 12) | (rs1 << 15) | (((immediate as u32) & 0xFFF) << 20)
}

/// Encode a shift-immediate instruction with a `funct7` field
/// 编码带 `funct7` 字段的移位立即数指令
fn encode_shift_i(opcode: u32, rd: u32, funct3: u32, rs1: u32, shamt: u32, funct7: u32) -> u32 {
    opcode | (rd << 7) | (funct3 << 12) | (rs1 << 15) | ((shamt & 0x1F) << 20) | (funct7 << 25)
}

/// Encode an R-type instruction / 编码 R 型指令
fn encode_r(opcode: u32, rd: u32, funct3: u32, rs1: u32, rs2: u32, funct7: u32) -> u32 {
    opcode | (rd << 7) | (funct3 << 12) | (rs1 << 15) | (rs2 << 20) | (funct7 << 25)
}

/// Encode an S-type instruction / 编码 S 型指令
fn encode_s(opcode: u32, funct3: u32, rs1: u32, rs2: u32, immediate: i32) -> u32 {
    let value = immediate as u32;
    opcode
        | ((value & 0x1F) << 7)
        | (funct3 << 12)
        | (rs1 << 15)
        | (rs2 << 20)
        | (((value >> 5) & 0x7F) << 25)
}

/// Encode a B-type instruction / 编码 B 型指令
fn encode_b(opcode: u32, funct3: u32, rs1: u32, rs2: u32, immediate: i32) -> u32 {
    let value = immediate as u32;
    opcode
        | (((value >> 11) & 1) << 7)
        | (((value >> 1) & 0xF) << 8)
        | (funct3 << 12)
        | (rs1 << 15)
        | (rs2 << 20)
        | (((value >> 5) & 0x3F) << 25)
        | (((value >> 12) & 1) << 31)
}

/// Encode a U-type instruction / 编码 U 型指令
fn encode_u(opcode: u32, rd: u32, immediate: u32) -> u32 {
    opcode | (rd << 7) | (immediate & 0xFFFF_F000)
}

/// Encode a J-type instruction / 编码 J 型指令
fn encode_j(opcode: u32, rd: u32, immediate: i32) -> u32 {
    let value = immediate as u32;
    opcode
        | (rd << 7)
        | (value & 0x000F_F000)
        | (((value >> 11) & 1) << 20)
        | (((value >> 1) & 0x3FF) << 21)
        | (((value >> 20) & 1) << 31)
}
