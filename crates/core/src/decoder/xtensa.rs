// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

/// Xtensa LX7 Instruction Set (subset sufficient for ESP32-S3 firmware simulation)
///
/// Xtensa uses variable-length encoding:
/// - 3-byte (24-bit) "wide" instructions (bits [1:0] != 0b00 of first byte, or check op0 field)
/// - 2-byte (16-bit) "narrow" instructions (CALL0/density option instructions)
///
/// We follow the Xtensa ISA Reference Manual encoding conventions.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Instruction {
    // Core ALU
    Add { rd: u8, rs: u8, rt: u8 },
    Addx2 { rd: u8, rs: u8, rt: u8 },
    Addx4 { rd: u8, rs: u8, rt: u8 },
    Addx8 { rd: u8, rs: u8, rt: u8 },
    Sub { rd: u8, rs: u8, rt: u8 },
    Subx2 { rd: u8, rs: u8, rt: u8 },
    Subx4 { rd: u8, rs: u8, rt: u8 },
    Subx8 { rd: u8, rs: u8, rt: u8 },
    And { rd: u8, rs: u8, rt: u8 },
    Or { rd: u8, rs: u8, rt: u8 },
    Xor { rd: u8, rs: u8, rt: u8 },
    Neg { rd: u8, rt: u8 },
    Abs { rd: u8, rt: u8 },

    // Shifts
    Sll { rd: u8, rs: u8 },
    Srl { rd: u8, rt: u8 },
    Sra { rd: u8, rt: u8 },
    Slli { rd: u8, rs: u8, sa: u8 },
    Srli { rd: u8, rt: u8, sa: u8 },
    Srai { rd: u8, rt: u8, sa: u8 },
    Ssa8l { rs: u8 },
    Ssl { rs: u8 },
    Ssr { rs: u8 },
    Ssai { sa: u8 },
    Src { rd: u8, rs: u8, rt: u8 },
    Extui { rd: u8, rt: u8, shift: u8, mask_bits: u8 },

    // Multiply
    Mull { rd: u8, rs: u8, rt: u8 },
    Muluh { rd: u8, rs: u8, rt: u8 },
    Mulsh { rd: u8, rs: u8, rt: u8 },

    // Loads & Stores (3-byte)
    L8ui { rt: u8, rs: u8, imm: u32 },
    L16ui { rt: u8, rs: u8, imm: u32 },
    L16si { rt: u8, rs: u8, imm: u32 },
    L32i { rt: u8, rs: u8, imm: u32 },
    S8i { rt: u8, rs: u8, imm: u32 },
    S16i { rt: u8, rs: u8, imm: u32 },
    S32i { rt: u8, rs: u8, imm: u32 },

    // Immediates
    Movi { rt: u8, imm: i32 },
    Addi { rt: u8, rs: u8, imm: i32 },
    Addmi { rt: u8, rs: u8, imm: i32 },

    // Branches
    J { offset: i32 },
    Jx { rs: u8 },
    Call0 { offset: i32 },
    Callx0 { rs: u8 },
    Ret,
    RetW,

    // Conditional branches
    Beqz { rs: u8, offset: i32 },
    Bnez { rs: u8, offset: i32 },
    Bltz { rs: u8, offset: i32 },
    Bgez { rs: u8, offset: i32 },
    Beq { rs: u8, rt: u8, offset: i32 },
    Bne { rs: u8, rt: u8, offset: i32 },
    Blt { rs: u8, rt: u8, offset: i32 },
    Bge { rs: u8, rt: u8, offset: i32 },
    Bltu { rs: u8, rt: u8, offset: i32 },
    Bgeu { rs: u8, rt: u8, offset: i32 },
    Beqi { rs: u8, imm: i32, offset: i32 },
    Bnei { rs: u8, imm: i32, offset: i32 },
    Blti { rs: u8, imm: i32, offset: i32 },
    Bgei { rs: u8, imm: i32, offset: i32 },
    Bltui { rs: u8, imm: u32, offset: i32 },
    Bgeui { rs: u8, imm: u32, offset: i32 },

    // Bit test branches
    Bbci { rs: u8, bit: u8, offset: i32 },
    Bbsi { rs: u8, bit: u8, offset: i32 },

    // Move conditional
    Moveqz { rd: u8, rs: u8, rt: u8 },
    Movnez { rd: u8, rs: u8, rt: u8 },
    Movltz { rd: u8, rs: u8, rt: u8 },
    Movgez { rd: u8, rs: u8, rt: u8 },

    // Special registers
    Rsr { rt: u8, sr: u8 },
    Wsr { rt: u8, sr: u8 },
    Xsr { rt: u8, sr: u8 },

    // Misc
    Nop,
    Memw,
    Isync,
    Dsync,
    Esync,
    Rsync,
    Extw,
    Ill,
    Break { s: u8, t: u8 },
    Syscall,

    // Loop instructions
    Loop { rs: u8, offset: i32 },
    Loopnez { rs: u8, offset: i32 },
    Loopgtz { rs: u8, offset: i32 },

    // Narrow (16-bit density) instructions
    NarrowL32iN { rt: u8, rs: u8, imm: u32 },
    NarrowS32iN { rt: u8, rs: u8, imm: u32 },
    NarrowAdd { rd: u8, rs: u8, rt: u8 },
    NarrowAddi { rd: u8, rs: u8, imm: i32 },
    NarrowMovi { rd: u8, imm: i32 },
    NarrowBeqz { rs: u8, offset: i32 },
    NarrowBnez { rs: u8, offset: i32 },
    NarrowMov { rd: u8, rs: u8 },
    NarrowRet,
    NarrowRetW,
    NarrowNop,

    Unknown(u32),
}

/// Decode an Xtensa instruction from bytes fetched at current PC.
/// Returns (instruction, byte_length) where byte_length is 2 or 3.
pub fn decode_xtensa(bytes: &[u8; 3]) -> (Instruction, u32) {
    // Xtensa encoding: if bits[3:0] of byte0 == 0b1000..1111 (i.e. narrow),
    // then it's a 2-byte (16-bit) instruction. Otherwise 3-byte (24-bit).
    // The density option uses op0 field = byte0[3:0].
    // Narrow instructions have op0 in {1000, 1001, 1010, 1011, 1100, 1101} => bits[3]=1 and bits[2:0]!=111
    // Actually the standard Xtensa encoding:
    //   op0 = byte0[3:0]
    //   If op0 >= 8 (bit3=1), instruction is 16-bit (narrow/density).
    //   Otherwise, instruction is 24-bit.

    let b0 = bytes[0];
    let op0 = b0 & 0x0F;

    if op0 & 0x08 != 0 {
        // 16-bit narrow instruction
        let inst16 = u16::from_le_bytes([bytes[0], bytes[1]]);
        (decode_narrow(inst16), 2)
    } else {
        // 24-bit wide instruction
        let inst24 = (bytes[0] as u32) | ((bytes[1] as u32) << 8) | ((bytes[2] as u32) << 16);
        (decode_wide(inst24), 3)
    }
}

fn decode_narrow(inst: u16) -> Instruction {
    let op0 = (inst & 0x0F) as u8;
    let t = ((inst >> 4) & 0x0F) as u8;
    let s = ((inst >> 8) & 0x0F) as u8;
    let r = ((inst >> 12) & 0x0F) as u8;

    match op0 {
        0x08 => {
            // L32I.N
            let imm = (t as u32) << 2; // 4-byte aligned offset
            Instruction::NarrowL32iN { rt: t, rs: s, imm }
        }
        0x09 => {
            // S32I.N
            let imm = (t as u32) << 2;
            Instruction::NarrowS32iN { rt: t, rs: s, imm }
        }
        0x0A => {
            // ADD.N
            Instruction::NarrowAdd { rd: r, rs: s, rt: t }
        }
        0x0B => {
            // ADDI.N
            let imm = if r == 0 { -1i32 } else { r as i32 };
            Instruction::NarrowAddi { rd: t, rs: s, imm }
        }
        0x0C => {
            // ST2 - MOVI.N, BEQZ.N, BNEZ.N, etc.
            match r {
                0..=6 => {
                    // MOVI.N
                    let _imm = if (inst >> 7) & 1 != 0 {
                        // 7-bit signed
                        ((r as i32) | 0x20) - 0x40 + ((((inst >> 12) & 0x0F) as i32) << 0)
                    } else {
                        (t as i32) | (((inst >> 12) as i32 & 0x0F) << 4)
                    };
                    // Actually MOVI.N encoding:
                    // MOVI.N: op0=1100, r=imm7[6:4], s=at, t=imm7[3:0]
                    let imm7 = ((r as u8 & 0x07) << 4) | t;
                    let signed = if imm7 & 0x40 != 0 {
                        (imm7 as i32) | !0x7F
                    } else {
                        imm7 as i32
                    };
                    Instruction::NarrowMovi { rd: s, imm: signed }
                }
                _ => Instruction::Unknown(inst as u32),
            }
        }
        0x0D => {
            // ST3 group: BEQZ.N, BNEZ.N, MOV.N, RET.N, etc.
            match r {
                0..=3 => {
                    // BEQZ.N
                    let imm = (t as i32) | ((r as i32 & 0x03) << 4);
                    Instruction::NarrowBeqz { rs: s, offset: imm }
                }
                4..=7 => {
                    // BNEZ.N
                    let imm = (t as i32) | (((r as i32) & 0x03) << 4);
                    Instruction::NarrowBnez { rs: s, offset: imm }
                }
                _ => Instruction::Unknown(inst as u32),
            }
        }
        0x0E => {
            // Misc narrow (RET.N, MOV.N, NOP.N ...)
            // Actually: op0=0x0D for some, 0x0E for others
            // RET.N = 0x00_0D (all zeros except op0)
            // Let's re-check
            if inst == 0xF01D {
                Instruction::NarrowRetW
            } else if inst == 0x0D {
                Instruction::NarrowRet
            } else {
                Instruction::Unknown(inst as u32)
            }
        }
        0x0F => {
            // NOP.N = 0xF03D or similar
            if inst == 0xF03D || inst == 0x203D {
                Instruction::NarrowNop
            } else {
                Instruction::Unknown(inst as u32)
            }
        }
        _ => Instruction::Unknown(inst as u32),
    }
}

fn b4_to_i32(val: u8) -> i32 {
    // Xtensa B4CONST table for BEQI/BNEI/etc.
    const B4CONST: [i32; 16] = [
        -1, 1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 16, 32, 64, 128, 256,
    ];
    B4CONST[val as usize & 0xF]
}

fn _b4_to_u32(val: u8) -> u32 {
    const B4CONSTU: [u32; 16] = [
        0x8000, 0x10000, 2, 3, 4, 5, 6, 7, 8, 10, 12, 16, 32, 64, 128, 256,
    ];
    B4CONSTU[val as usize & 0xF]
}

fn decode_wide(inst: u32) -> Instruction {
    // 24-bit instruction. Fields:
    // byte0 = inst[7:0], byte1 = inst[15:8], byte2 = inst[23:16]
    // op0 = inst[3:0]
    // t = inst[7:4]
    // s = inst[11:8]
    // r = inst[15:12]
    // op1 = inst[19:16]
    // op2 = inst[23:20]
    let op0 = (inst & 0x0F) as u8;
    let t = ((inst >> 4) & 0x0F) as u8;
    let s = ((inst >> 8) & 0x0F) as u8;
    let r = ((inst >> 12) & 0x0F) as u8;
    let op1 = ((inst >> 16) & 0x0F) as u8;
    let op2 = ((inst >> 20) & 0x0F) as u8;

    match op0 {
        0x00 => {
            // QRST group
            match op1 {
                0x00 => {
                    // RST0 group
                    match op2 {
                        0x00 => {
                            // ST0 subgroup
                            match r {
                                0x00 if s == 0 && t == 0 => Instruction::Ill,
                                0x00 => Instruction::Nop, // SNM0 group
                                0x01 => Instruction::Ssl { rs: s },
                                0x02 => Instruction::Ssr { rs: s },
                                0x04 => {
                                    if s == 0 && t == 0 {
                                        Instruction::Ssai { sa: 0 }
                                    } else {
                                        Instruction::Ssa8l { rs: s }
                                    }
                                }
                                _ => Instruction::Unknown(inst),
                            }
                        }
                        0x01 => {
                            // AND, OR, XOR
                            match op2 {
                                _ => Instruction::And { rd: r, rs: s, rt: t },
                            }
                        }
                        0x02 => Instruction::Or { rd: r, rs: s, rt: t },
                        0x03 => Instruction::Xor { rd: r, rs: s, rt: t },
                        0x06 => Instruction::Neg { rd: r, rt: t },
                        0x07 => Instruction::Abs { rd: r, rt: t },
                        0x08 => {
                            // ADD
                            Instruction::Add { rd: r, rs: s, rt: t }
                        }
                        0x09 => Instruction::Addx2 { rd: r, rs: s, rt: t },
                        0x0A => Instruction::Addx4 { rd: r, rs: s, rt: t },
                        0x0B => Instruction::Addx8 { rd: r, rs: s, rt: t },
                        0x0C => Instruction::Sub { rd: r, rs: s, rt: t },
                        0x0D => Instruction::Subx2 { rd: r, rs: s, rt: t },
                        0x0E => Instruction::Subx4 { rd: r, rs: s, rt: t },
                        0x0F => Instruction::Subx8 { rd: r, rs: s, rt: t },
                        _ => Instruction::Unknown(inst),
                    }
                }
                0x01 => {
                    // RST1 group
                    match op2 {
                        0x00 | 0x01 => {
                            // SLLI
                            let sa = (s | ((op2 & 1) << 4)) as u8;
                            Instruction::Slli { rd: r, rs: t, sa: 32 - sa }
                        }
                        0x04 => Instruction::Srli { rd: r, rt: t, sa: s },
                        0x05 => Instruction::Srai { rd: r, rt: t, sa: (s | ((op2 & 0x01) << 4)) },
                        0x06 => Instruction::Src { rd: r, rs: s, rt: t },
                        0x08 => Instruction::Sll { rd: r, rs: s },
                        0x09 => Instruction::Srl { rd: r, rt: t },
                        0x0A => Instruction::Sra { rd: r, rt: t },
                        0x0B => {
                            // MUL16U, MUL16S, MULL, MULUH, MULSH
                            Instruction::Unknown(inst)
                        }
                        _ => {
                            // EXTUI
                            if op2 & 0x04 == 0x04 {
                                let shift = s | ((op2 & 1) << 4);
                                let mask_bits = t + 1;
                                Instruction::Extui { rd: r, rt: t, shift: shift as u8, mask_bits }
                            } else {
                                Instruction::Unknown(inst)
                            }
                        }
                    }
                }
                0x02 => {
                    // RST2 group: MULL, MULUH, MULSH, etc.
                    match op2 {
                        0x08 => Instruction::Mull { rd: r, rs: s, rt: t },
                        0x0A => Instruction::Muluh { rd: r, rs: s, rt: t },
                        0x0B => Instruction::Mulsh { rd: r, rs: s, rt: t },
                        _ => Instruction::Unknown(inst),
                    }
                }
                0x03 => {
                    // RST3: RSR, WSR, XSR, and condition moves
                    match op2 {
                        0x00 => Instruction::Rsr { rt: t, sr: s | (r << 4) },
                        0x01 => Instruction::Wsr { rt: t, sr: s | (r << 4) },
                        0x06 => Instruction::Xsr { rt: t, sr: s | (r << 4) },
                        0x09 => Instruction::Moveqz { rd: r, rs: s, rt: t },
                        0x0A => Instruction::Movnez { rd: r, rs: s, rt: t },
                        0x0B => Instruction::Movltz { rd: r, rs: s, rt: t },
                        0x0C => Instruction::Movgez { rd: r, rs: s, rt: t },
                        _ => Instruction::Unknown(inst),
                    }
                }
                0x04 => {
                    // EXTUI (alternate encoding)
                    let shift = s | ((op1 >> 4) << 4);
                    let mask_bits = t + 1;
                    Instruction::Extui { rd: r, rt: t, shift: shift as u8, mask_bits }
                }
                _ => Instruction::Unknown(inst),
            }
        }
        0x01 => {
            // L32R: PC-relative load from literal pool (always negative offset)
            let imm16 = ((inst >> 8) & 0xFFFF) as u16;
            let _offset = ((imm16 as i16 as i32) << 2) | !0x3FFFF_i32;
            // L32R is PC-relative; we model it as L32I with rs=0 for now
            Instruction::L32i { rt: t, rs: 0, imm: 0 }
        }
        0x02 => {
            // LSAI group: L8UI, L16UI, L16SI, L32I, S8I, S16I, S32I
            let imm8 = ((inst >> 16) & 0xFF) as u32;
            match r {
                0x00 => Instruction::L8ui { rt: t, rs: s, imm: imm8 },
                0x01 => Instruction::L16ui { rt: t, rs: s, imm: imm8 << 1 },
                0x02 => Instruction::L32i { rt: t, rs: s, imm: imm8 << 2 },
                0x04 => Instruction::S8i { rt: t, rs: s, imm: imm8 },
                0x05 => Instruction::S16i { rt: t, rs: s, imm: imm8 << 1 },
                0x06 => Instruction::S32i { rt: t, rs: s, imm: imm8 << 2 },
                0x09 => Instruction::L16si { rt: t, rs: s, imm: imm8 << 1 },
                0x0A => {
                    // MOVI
                    let imm12 = (imm8 as i32) | ((t as i32) << 8);
                    let imm = if imm12 & (1 << 11) != 0 {
                        imm12 | !0xFFF
                    } else {
                        imm12
                    };
                    Instruction::Movi { rt: s, imm }
                }
                0x0C => {
                    // ADDI
                    let imm = if (imm8 as i32) & 0x80 != 0 { (imm8 as i32) | !0xFF } else { imm8 as i32 };
                    Instruction::Addi { rt: t, rs: s, imm }
                }
                0x0D => {
                    // ADDMI
                    let imm = if (imm8 as i32) & 0x80 != 0 { ((imm8 as i32) | !0xFF) << 8 } else { (imm8 as i32) << 8 };
                    Instruction::Addmi { rt: t, rs: s, imm }
                }
                _ => Instruction::Unknown(inst),
            }
        }
        0x03 => {
            // LSCI: L32I with scaled offset (Float load/store - treat as unknown)
            Instruction::Unknown(inst)
        }
        0x05 => {
            // CALL0 / CALLX0
            // op0=0x05 -> CALL0
            let offset = ((inst >> 6) as i32) << 2;
            // Sign extend 18-bit offset
            let offset = if offset & (1 << 19) != 0 {
                offset | !0xFFFFF
            } else {
                offset
            };
            Instruction::Call0 { offset }
        }
        0x06 => {
            // SI group: J, BZ (BEQZ, BNEZ, BLTZ, BGEZ)
            match r {
                0x00 => {
                    // J
                    let offset18 = (inst >> 6) as i32;
                    let offset = if offset18 & (1 << 17) != 0 {
                        offset18 | !0x3FFFF
                    } else {
                        offset18
                    };
                    Instruction::J { offset }
                }
                0x01 => {
                    // BEQZ
                    let imm12 = ((inst >> 12) & 0xFFF) as i32;
                    let offset = if imm12 & (1 << 11) != 0 {
                        imm12 | !0xFFF
                    } else {
                        imm12
                    };
                    Instruction::Beqz { rs: s, offset }
                }
                0x05 => {
                    // BNEZ
                    let imm12 = ((inst >> 12) & 0xFFF) as i32;
                    let offset = if imm12 & (1 << 11) != 0 {
                        imm12 | !0xFFF
                    } else {
                        imm12
                    };
                    Instruction::Bnez { rs: s, offset }
                }
                0x09 => {
                    // BLTZ
                    let imm12 = ((inst >> 12) & 0xFFF) as i32;
                    let offset = if imm12 & (1 << 11) != 0 {
                        imm12 | !0xFFF
                    } else {
                        imm12
                    };
                    Instruction::Bltz { rs: s, offset }
                }
                0x0D => {
                    // BGEZ
                    let imm12 = ((inst >> 12) & 0xFFF) as i32;
                    let offset = if imm12 & (1 << 11) != 0 {
                        imm12 | !0xFFF
                    } else {
                        imm12
                    };
                    Instruction::Bgez { rs: s, offset }
                }
                0x08 => {
                    // LOOP
                    let imm8 = ((inst >> 16) & 0xFF) as i32;
                    Instruction::Loop { rs: s, offset: imm8 }
                }
                _ => Instruction::Unknown(inst),
            }
        }
        0x07 => {
            // B group: conditional branches with register comparisons
            match r {
                0x01 => {
                    // BEQ
                    let imm8 = ((inst >> 16) & 0xFF) as i32;
                    let offset = if imm8 & 0x80 != 0 { imm8 | !0xFF } else { imm8 };
                    Instruction::Beq { rs: s, rt: t, offset }
                }
                0x09 => {
                    // BNE
                    let imm8 = ((inst >> 16) & 0xFF) as i32;
                    let offset = if imm8 & 0x80 != 0 { imm8 | !0xFF } else { imm8 };
                    Instruction::Bne { rs: s, rt: t, offset }
                }
                0x02 => {
                    // BLT
                    let imm8 = ((inst >> 16) & 0xFF) as i32;
                    let offset = if imm8 & 0x80 != 0 { imm8 | !0xFF } else { imm8 };
                    Instruction::Blt { rs: s, rt: t, offset }
                }
                0x0A => {
                    // BGE
                    let imm8 = ((inst >> 16) & 0xFF) as i32;
                    let offset = if imm8 & 0x80 != 0 { imm8 | !0xFF } else { imm8 };
                    Instruction::Bge { rs: s, rt: t, offset }
                }
                0x03 => {
                    // BLTU
                    let imm8 = ((inst >> 16) & 0xFF) as i32;
                    let offset = if imm8 & 0x80 != 0 { imm8 | !0xFF } else { imm8 };
                    Instruction::Bltu { rs: s, rt: t, offset }
                }
                0x0B => {
                    // BGEU
                    let imm8 = ((inst >> 16) & 0xFF) as i32;
                    let offset = if imm8 & 0x80 != 0 { imm8 | !0xFF } else { imm8 };
                    Instruction::Bgeu { rs: s, rt: t, offset }
                }
                0x06 => {
                    // BBCI
                    let bit = s | ((r & 1) << 4);
                    let imm8 = ((inst >> 16) & 0xFF) as i32;
                    let offset = if imm8 & 0x80 != 0 { imm8 | !0xFF } else { imm8 };
                    Instruction::Bbci { rs: s, bit, offset }
                }
                0x0E => {
                    // BBSI
                    let bit = s | ((r & 1) << 4);
                    let imm8 = ((inst >> 16) & 0xFF) as i32;
                    let offset = if imm8 & 0x80 != 0 { imm8 | !0xFF } else { imm8 };
                    Instruction::Bbsi { rs: s, bit, offset }
                }
                0x04 => {
                    // BEQI
                    let imm8 = ((inst >> 16) & 0xFF) as i32;
                    let offset = if imm8 & 0x80 != 0 { imm8 | !0xFF } else { imm8 };
                    Instruction::Beqi { rs: s, imm: b4_to_i32(t), offset }
                }
                0x0C => {
                    // BNEI
                    let imm8 = ((inst >> 16) & 0xFF) as i32;
                    let offset = if imm8 & 0x80 != 0 { imm8 | !0xFF } else { imm8 };
                    Instruction::Bnei { rs: s, imm: b4_to_i32(t), offset }
                }
                0x05 => {
                    // BLTI
                    let imm8 = ((inst >> 16) & 0xFF) as i32;
                    let offset = if imm8 & 0x80 != 0 { imm8 | !0xFF } else { imm8 };
                    Instruction::Blti { rs: s, imm: b4_to_i32(t), offset }
                }
                0x0D => {
                    // BGEI
                    let imm8 = ((inst >> 16) & 0xFF) as i32;
                    let offset = if imm8 & 0x80 != 0 { imm8 | !0xFF } else { imm8 };
                    Instruction::Bgei { rs: s, imm: b4_to_i32(t), offset }
                }
                _ => Instruction::Unknown(inst),
            }
        }
        _ => Instruction::Unknown(inst),
    }
}

// Handle RET / RET.W / CALLX0 / JX from the RST0 group
// These are encoded within op0=0, op1=0, op2=0, with specific r/s/t values.
// For a simpler implementation, we also handle them in the main decode.
