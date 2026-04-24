// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

/// RISC-V RV32I Base Integer Instruction Set
#[derive(Debug, PartialEq, Eq)]
pub enum Instruction {
    Lui { rd: u8, imm: u32 },             // LUI rd, imm
    Auipc { rd: u8, imm: u32 },           // AUIPC rd, imm
    Jal { rd: u8, imm: i32 },             // JAL rd, offset
    Jalr { rd: u8, rs1: u8, imm: i32 },   // JALR rd, rs1, offset
    Beq { rs1: u8, rs2: u8, imm: i32 },   // BEQ rs1, rs2, offset
    Bne { rs1: u8, rs2: u8, imm: i32 },   // BNE rs1, rs2, offset
    Blt { rs1: u8, rs2: u8, imm: i32 },   // BLT rs1, rs2, offset
    Bge { rs1: u8, rs2: u8, imm: i32 },   // BGE rs1, rs2, offset
    Bltu { rs1: u8, rs2: u8, imm: i32 },  // BLTU rs1, rs2, offset
    Bgeu { rs1: u8, rs2: u8, imm: i32 },  // BGEU rs1, rs2, offset
    Lb { rd: u8, rs1: u8, imm: i32 },     // LB rd, offset(rs1)
    Lh { rd: u8, rs1: u8, imm: i32 },     // LH rd, offset(rs1)
    Lw { rd: u8, rs1: u8, imm: i32 },     // LW rd, offset(rs1)
    Lbu { rd: u8, rs1: u8, imm: i32 },    // LBU rd, offset(rs1)
    Lhu { rd: u8, rs1: u8, imm: i32 },    // LHU rd, offset(rs1)
    Sb { rs1: u8, rs2: u8, imm: i32 },    // SB rs2, offset(rs1)
    Sh { rs1: u8, rs2: u8, imm: i32 },    // SH rs2, offset(rs1)
    Sw { rs1: u8, rs2: u8, imm: i32 },    // SW rs2, offset(rs1)
    Addi { rd: u8, rs1: u8, imm: i32 },   // ADDI rd, rs1, imm
    Slti { rd: u8, rs1: u8, imm: i32 },   // SLTI rd, rs1, imm
    Sltiu { rd: u8, rs1: u8, imm: i32 },  // SLTIU rd, rs1, imm
    Xori { rd: u8, rs1: u8, imm: i32 },   // XORI rd, rs1, imm
    Ori { rd: u8, rs1: u8, imm: i32 },    // ORI rd, rs1, imm
    Andi { rd: u8, rs1: u8, imm: i32 },   // ANDI rd, rs1, imm
    Slli { rd: u8, rs1: u8, shamt: u8 },  // SLLI rd, rs1, shamt
    Srli { rd: u8, rs1: u8, shamt: u8 },  // SRLI rd, rs1, shamt
    Srai { rd: u8, rs1: u8, shamt: u8 },  // SRAI rd, rs1, shamt
    Add { rd: u8, rs1: u8, rs2: u8 },     // ADD rd, rs1, rs2
    Sub { rd: u8, rs1: u8, rs2: u8 },     // SUB rd, rs1, rs2
    Sll { rd: u8, rs1: u8, rs2: u8 },     // SLL rd, rs1, rs2
    Slt { rd: u8, rs1: u8, rs2: u8 },     // SLT rd, rs1, rs2
    Sltu { rd: u8, rs1: u8, rs2: u8 },    // SLTU rd, rs1, rs2
    Xor { rd: u8, rs1: u8, rs2: u8 },     // XOR rd, rs1, rs2
    Srl { rd: u8, rs1: u8, rs2: u8 },     // SRL rd, rs1, rs2
    Sra { rd: u8, rs1: u8, rs2: u8 },     // SRA rd, rs1, rs2
    Or { rd: u8, rs1: u8, rs2: u8 },      // OR rd, rs1, rs2
    And { rd: u8, rs1: u8, rs2: u8 },     // AND rd, rs1, rs2
    Fence,                                // FENCE
    Ecall,                                // ECALL
    Ebreak,                               // EBREAK
    Mret,                                 // MRET
    Csrrw { rd: u8, rs1: u8, csr: u16 },  // CSRRW
    Csrrs { rd: u8, rs1: u8, csr: u16 },  // CSRRS
    Csrrc { rd: u8, rs1: u8, csr: u16 },  // CSRRC
    Csrrwi { rd: u8, imm: u8, csr: u16 }, // CSRRWI
    Csrrsi { rd: u8, imm: u8, csr: u16 }, // CSRRSI
    Csrrci { rd: u8, imm: u8, csr: u16 }, // CSRRCI
    // RV32M — Integer Multiplication and Division
    Mul { rd: u8, rs1: u8, rs2: u8 },
    Mulh { rd: u8, rs1: u8, rs2: u8 },
    Mulhsu { rd: u8, rs1: u8, rs2: u8 },
    Mulhu { rd: u8, rs1: u8, rs2: u8 },
    Div { rd: u8, rs1: u8, rs2: u8 },
    Divu { rd: u8, rs1: u8, rs2: u8 },
    Rem { rd: u8, rs1: u8, rs2: u8 },
    Remu { rd: u8, rs1: u8, rs2: u8 },
    // RV32A — Atomics. `aq` / `rl` encoded ordering bits are ignored by
    // this single-threaded interpreter.
    LrW { rd: u8, rs1: u8 },
    ScW { rd: u8, rs1: u8, rs2: u8 },
    AmoSwapW { rd: u8, rs1: u8, rs2: u8 },
    AmoAddW { rd: u8, rs1: u8, rs2: u8 },
    AmoXorW { rd: u8, rs1: u8, rs2: u8 },
    AmoOrW { rd: u8, rs1: u8, rs2: u8 },
    AmoAndW { rd: u8, rs1: u8, rs2: u8 },
    AmoMinW { rd: u8, rs1: u8, rs2: u8 },
    AmoMaxW { rd: u8, rs1: u8, rs2: u8 },
    AmoMinuW { rd: u8, rs1: u8, rs2: u8 },
    AmoMaxuW { rd: u8, rs1: u8, rs2: u8 },
    Unknown(u32),
}

pub fn decode_rv32(inst: u32) -> Instruction {
    let opcode = inst & 0x7F;
    let rd = ((inst >> 7) & 0x1F) as u8;
    let funct3 = ((inst >> 12) & 0x7) as u8;
    let rs1 = ((inst >> 15) & 0x1F) as u8;
    let rs2 = ((inst >> 20) & 0x1F) as u8;
    let funct7 = ((inst >> 25) & 0x7F) as u8;

    match opcode {
        0x37 => {
            // LUI
            let imm = inst & 0xFFFFF000;
            Instruction::Lui { rd, imm }
        }
        0x17 => {
            // AUIPC
            let imm = inst & 0xFFFFF000;
            Instruction::Auipc { rd, imm }
        }
        0x6F => {
            // JAL
            // imm[20|10:1|11|19:12]
            let imm20 = (inst >> 31) & 1;
            let imm10_1 = (inst >> 21) & 0x3FF;
            let imm11 = (inst >> 20) & 1;
            let imm19_12 = (inst >> 12) & 0xFF;
            let offset = (imm20 << 20) | (imm19_12 << 12) | (imm11 << 11) | (imm10_1 << 1);
            // Sign extend 21-bit
            let signed_offset = if imm20 == 1 {
                (offset as i32) | !0xFFFFF
            } else {
                offset as i32
            };
            Instruction::Jal {
                rd,
                imm: signed_offset,
            }
        }
        0x67 => {
            // JALR
            let imm = (inst as i32) >> 20; // Sign-extended 12-bit
            Instruction::Jalr { rd, rs1, imm }
        }
        0x63 => {
            // BRANCH
            // imm[12|10:5|4:1|11]
            let imm12 = (inst >> 31) & 1;
            let imm10_5 = (inst >> 25) & 0x3F;
            let imm4_1 = (inst >> 8) & 0xF;
            let imm11 = (inst >> 7) & 1;
            let offset = (imm12 << 12) | (imm11 << 11) | (imm10_5 << 5) | (imm4_1 << 1);
            let signed_offset = if imm12 == 1 {
                (offset as i32) | !0x1FFF
            } else {
                offset as i32
            };

            match funct3 {
                0 => Instruction::Beq {
                    rs1,
                    rs2,
                    imm: signed_offset,
                },
                1 => Instruction::Bne {
                    rs1,
                    rs2,
                    imm: signed_offset,
                },
                4 => Instruction::Blt {
                    rs1,
                    rs2,
                    imm: signed_offset,
                },
                5 => Instruction::Bge {
                    rs1,
                    rs2,
                    imm: signed_offset,
                },
                6 => Instruction::Bltu {
                    rs1,
                    rs2,
                    imm: signed_offset,
                },
                7 => Instruction::Bgeu {
                    rs1,
                    rs2,
                    imm: signed_offset,
                },
                _ => Instruction::Unknown(inst),
            }
        }
        0x03 => {
            // LOAD
            let imm = (inst as i32) >> 20;
            match funct3 {
                0 => Instruction::Lb { rd, rs1, imm },
                1 => Instruction::Lh { rd, rs1, imm },
                2 => Instruction::Lw { rd, rs1, imm },
                4 => Instruction::Lbu { rd, rs1, imm },
                5 => Instruction::Lhu { rd, rs1, imm },
                _ => Instruction::Unknown(inst),
            }
        }
        0x23 => {
            // STORE
            // imm[11:5|4:0]
            let imm11_5 = (inst >> 25) & 0x7F;
            let imm4_0 = (inst >> 7) & 0x1F;
            let offset = (imm11_5 << 5) | imm4_0;
            let signed_offset = if (imm11_5 >> 6) == 1 {
                (offset as i32) | !0xFFF
            } else {
                offset as i32
            };
            match funct3 {
                0 => Instruction::Sb {
                    rs1,
                    rs2,
                    imm: signed_offset,
                },
                1 => Instruction::Sh {
                    rs1,
                    rs2,
                    imm: signed_offset,
                },
                2 => Instruction::Sw {
                    rs1,
                    rs2,
                    imm: signed_offset,
                },
                _ => Instruction::Unknown(inst),
            }
        }
        0x13 => {
            // OP-IMM
            let imm = (inst as i32) >> 20;
            match funct3 {
                0 => Instruction::Addi { rd, rs1, imm },
                2 => Instruction::Slti { rd, rs1, imm },
                3 => Instruction::Sltiu { rd, rs1, imm }, // Immediate is sign-extended even for SLTIU comparison
                4 => Instruction::Xori { rd, rs1, imm },
                6 => Instruction::Ori { rd, rs1, imm },
                7 => Instruction::Andi { rd, rs1, imm },
                1 => {
                    // SLLI
                    let shamt = (imm & 0x1F) as u8;
                    Instruction::Slli { rd, rs1, shamt }
                }
                5 => {
                    // SRLI/SRAI
                    let shamt = (imm & 0x1F) as u8;
                    if (imm & 0x400) != 0 {
                        Instruction::Srai { rd, rs1, shamt }
                    } else {
                        Instruction::Srli { rd, rs1, shamt }
                    }
                }
                _ => Instruction::Unknown(inst),
            }
        }
        0x33 => {
            // OP — RV32I base + RV32M multiply/divide (funct7 == 0x01).
            match (funct3, funct7) {
                (0, 0x00) => Instruction::Add { rd, rs1, rs2 },
                (0, 0x20) => Instruction::Sub { rd, rs1, rs2 },
                (1, 0x00) => Instruction::Sll { rd, rs1, rs2 },
                (2, 0x00) => Instruction::Slt { rd, rs1, rs2 },
                (3, 0x00) => Instruction::Sltu { rd, rs1, rs2 },
                (4, 0x00) => Instruction::Xor { rd, rs1, rs2 },
                (5, 0x00) => Instruction::Srl { rd, rs1, rs2 },
                (5, 0x20) => Instruction::Sra { rd, rs1, rs2 },
                (6, 0x00) => Instruction::Or { rd, rs1, rs2 },
                (7, 0x00) => Instruction::And { rd, rs1, rs2 },
                (0, 0x01) => Instruction::Mul { rd, rs1, rs2 },
                (1, 0x01) => Instruction::Mulh { rd, rs1, rs2 },
                (2, 0x01) => Instruction::Mulhsu { rd, rs1, rs2 },
                (3, 0x01) => Instruction::Mulhu { rd, rs1, rs2 },
                (4, 0x01) => Instruction::Div { rd, rs1, rs2 },
                (5, 0x01) => Instruction::Divu { rd, rs1, rs2 },
                (6, 0x01) => Instruction::Rem { rd, rs1, rs2 },
                (7, 0x01) => Instruction::Remu { rd, rs1, rs2 },
                _ => Instruction::Unknown(inst),
            }
        }
        0x2F => {
            // RV32A — AMO. funct3 == 0x2 selects 32-bit word ops; we don't
            // model the `aq`/`rl` ordering bits (single-threaded).
            if funct3 != 0x2 {
                return Instruction::Unknown(inst);
            }
            let funct5 = (funct7 >> 2) & 0x1F;
            match funct5 {
                0x02 => Instruction::LrW { rd, rs1 },
                0x03 => Instruction::ScW { rd, rs1, rs2 },
                0x01 => Instruction::AmoSwapW { rd, rs1, rs2 },
                0x00 => Instruction::AmoAddW { rd, rs1, rs2 },
                0x04 => Instruction::AmoXorW { rd, rs1, rs2 },
                0x08 => Instruction::AmoOrW { rd, rs1, rs2 },
                0x0C => Instruction::AmoAndW { rd, rs1, rs2 },
                0x10 => Instruction::AmoMinW { rd, rs1, rs2 },
                0x14 => Instruction::AmoMaxW { rd, rs1, rs2 },
                0x18 => Instruction::AmoMinuW { rd, rs1, rs2 },
                0x1C => Instruction::AmoMaxuW { rd, rs1, rs2 },
                _ => Instruction::Unknown(inst),
            }
        }
        0x0F => {
            // FENCE
            Instruction::Fence
        }
        0x73 => {
            // SYSTEM
            let csr = (inst >> 20) as u16;
            match funct3 {
                0x0 => match inst >> 20 {
                    0x000 => Instruction::Ecall,
                    0x001 => Instruction::Ebreak,
                    0x302 => Instruction::Mret,
                    _ => Instruction::Unknown(inst),
                },
                1 => Instruction::Csrrw { rd, rs1, csr },
                2 => Instruction::Csrrs { rd, rs1, csr },
                3 => Instruction::Csrrc { rd, rs1, csr },
                5 => Instruction::Csrrwi { rd, imm: rs1, csr },
                6 => Instruction::Csrrsi { rd, imm: rs1, csr },
                7 => Instruction::Csrrci { rd, imm: rs1, csr },
                _ => Instruction::Unknown(inst),
            }
        }
        _ => Instruction::Unknown(inst),
    }
}

/// Decode a 16-bit RV32C compressed instruction. Returns an Instruction
/// from the RV32I set that matches the compressed form semantically —
/// the executor does not need to know the encoding is compressed, only
/// that it consumed 2 bytes instead of 4.
///
/// Covers the subset GCC/LLVM produce most often (~80% of compiled
/// code): C.ADDI / C.LI / C.LUI / C.MV / C.ADD / C.J / C.JAL / C.JR /
/// C.JALR / C.BEQZ / C.BNEZ / C.LW / C.SW / C.LWSP / C.SWSP /
/// C.ADDI4SPN / C.ADDI16SP / C.NOP / C.SLLI / C.SRLI / C.SRAI /
/// C.ANDI / C.SUB / C.XOR / C.OR / C.AND / C.EBREAK. Everything else
/// returns `Unknown(half as u32)`.
pub fn decode_rv32c(half: u16) -> Instruction {
    let op = half & 0x3;
    let funct3 = (half >> 13) & 0x7;

    // Compressed register fields use the 3-bit encoding for a restricted
    // set of regs: x8..x15. Offset back into the full 5-bit space with +8.
    let crs1p = ((half >> 7) & 0x7) as u8 + 8; // rs1' / rd'
    let crs2p = ((half >> 2) & 0x7) as u8 + 8; // rs2'

    let rd = ((half >> 7) & 0x1F) as u8;  // full 5-bit rd / rs1
    let rs2 = ((half >> 2) & 0x1F) as u8; // full 5-bit rs2

    match (op, funct3) {
        // ── Quadrant 0 ────────────────────────────────────────────
        (0, 0) => {
            // C.ADDI4SPN rd', uimm  → addi rd', x2, uimm (uimm != 0)
            let imm = (((half >> 5) & 0x1) as u32) << 3   // bit 3
                | (((half >> 6) & 0x1) as u32) << 2       // bit 2
                | (((half >> 11) & 0x3) as u32) << 4      // bits 5:4
                | (((half >> 7) & 0xF) as u32) << 6;      // bits 9:6
            if imm == 0 {
                return Instruction::Unknown(half as u32);
            }
            Instruction::Addi { rd: crs2p, rs1: 2, imm: imm as i32 }
        }
        (0, 2) => {
            // C.LW rd', uimm(rs1')
            let imm = (((half >> 5) & 0x1) as u32) << 6
                | (((half >> 10) & 0x7) as u32) << 3
                | (((half >> 6) & 0x1) as u32) << 2;
            Instruction::Lw { rd: crs2p, rs1: crs1p, imm: imm as i32 }
        }
        (0, 6) => {
            // C.SW rs2', uimm(rs1')
            let imm = (((half >> 5) & 0x1) as u32) << 6
                | (((half >> 10) & 0x7) as u32) << 3
                | (((half >> 6) & 0x1) as u32) << 2;
            Instruction::Sw { rs1: crs1p, rs2: crs2p, imm: imm as i32 }
        }

        // ── Quadrant 1 ────────────────────────────────────────────
        (1, 0) => {
            // C.NOP / C.ADDI rd, imm6 (nzimm)
            let imm6 = ((half >> 2) & 0x1F) as u32 | (((half >> 12) & 0x1) as u32) << 5;
            let imm = sign_extend(imm6, 6);
            if rd == 0 {
                // C.NOP (imm must be 0 per spec, but we don't enforce)
                Instruction::Addi { rd: 0, rs1: 0, imm: 0 }
            } else {
                Instruction::Addi { rd, rs1: rd, imm }
            }
        }
        (1, 1) => {
            // RV32: C.JAL imm11 → JAL x1, offset
            let imm = c_jal_imm(half);
            Instruction::Jal { rd: 1, imm }
        }
        (1, 2) => {
            // C.LI rd, imm6 → ADDI rd, x0, imm6
            if rd == 0 {
                return Instruction::Unknown(half as u32);
            }
            let imm6 = ((half >> 2) & 0x1F) as u32 | (((half >> 12) & 0x1) as u32) << 5;
            Instruction::Addi { rd, rs1: 0, imm: sign_extend(imm6, 6) }
        }
        (1, 3) => {
            if rd == 2 {
                // C.ADDI16SP nzimm10
                let imm10 = (((half >> 12) & 0x1) as u32) << 9
                    | (((half >> 3) & 0x3) as u32) << 7
                    | (((half >> 5) & 0x1) as u32) << 6
                    | (((half >> 2) & 0x1) as u32) << 5
                    | (((half >> 6) & 0x1) as u32) << 4;
                if imm10 == 0 {
                    return Instruction::Unknown(half as u32);
                }
                Instruction::Addi { rd: 2, rs1: 2, imm: sign_extend(imm10 as u32, 10) }
            } else if rd != 0 {
                // C.LUI rd, imm
                let imm17 = (((half >> 12) & 0x1) as u32) << 17
                    | (((half >> 2) & 0x1F) as u32) << 12;
                if imm17 == 0 {
                    return Instruction::Unknown(half as u32);
                }
                let imm = sign_extend(imm17 as u32, 18) as u32;
                Instruction::Lui { rd, imm: imm & 0xFFFF_F000 }
            } else {
                Instruction::Unknown(half as u32)
            }
        }
        (1, 4) => {
            // C.SRLI / C.SRAI / C.ANDI / C.SUB / C.XOR / C.OR / C.AND
            let funct2 = (half >> 10) & 0x3;
            match funct2 {
                0 => {
                    // C.SRLI rd', shamt
                    let shamt = ((half >> 2) & 0x1F) as u8 | ((((half >> 12) & 0x1) as u8) << 5);
                    Instruction::Srli { rd: crs1p, rs1: crs1p, shamt }
                }
                1 => {
                    // C.SRAI rd', shamt
                    let shamt = ((half >> 2) & 0x1F) as u8 | ((((half >> 12) & 0x1) as u8) << 5);
                    Instruction::Srai { rd: crs1p, rs1: crs1p, shamt }
                }
                2 => {
                    // C.ANDI rd', imm6
                    let imm6 = ((half >> 2) & 0x1F) as u32 | (((half >> 12) & 0x1) as u32) << 5;
                    Instruction::Andi { rd: crs1p, rs1: crs1p, imm: sign_extend(imm6, 6) }
                }
                3 => {
                    // C.SUB / C.XOR / C.OR / C.AND
                    let sub_op = ((half >> 12) & 0x1) << 2 | ((half >> 5) & 0x3);
                    match sub_op {
                        0 => Instruction::Sub { rd: crs1p, rs1: crs1p, rs2: crs2p },
                        1 => Instruction::Xor { rd: crs1p, rs1: crs1p, rs2: crs2p },
                        2 => Instruction::Or { rd: crs1p, rs1: crs1p, rs2: crs2p },
                        3 => Instruction::And { rd: crs1p, rs1: crs1p, rs2: crs2p },
                        _ => Instruction::Unknown(half as u32),
                    }
                }
                _ => Instruction::Unknown(half as u32),
            }
        }
        (1, 5) => {
            // C.J offset → JAL x0, offset
            let imm = c_jal_imm(half);
            Instruction::Jal { rd: 0, imm }
        }
        (1, 6) => {
            // C.BEQZ rs1', offset → BEQ rs1', x0, offset
            let imm = c_branch_imm(half);
            Instruction::Beq { rs1: crs1p, rs2: 0, imm }
        }
        (1, 7) => {
            // C.BNEZ rs1', offset → BNE rs1', x0, offset
            let imm = c_branch_imm(half);
            Instruction::Bne { rs1: crs1p, rs2: 0, imm }
        }

        // ── Quadrant 2 ────────────────────────────────────────────
        (2, 0) => {
            // C.SLLI rd, shamt
            if rd == 0 {
                return Instruction::Unknown(half as u32);
            }
            let shamt = ((half >> 2) & 0x1F) as u8 | ((((half >> 12) & 0x1) as u8) << 5);
            Instruction::Slli { rd, rs1: rd, shamt }
        }
        (2, 2) => {
            // C.LWSP rd, uimm → LW rd, offset(x2)
            if rd == 0 {
                return Instruction::Unknown(half as u32);
            }
            let imm = (((half >> 2) & 0x3) as u32) << 6
                | (((half >> 4) & 0x7) as u32) << 2
                | (((half >> 12) & 0x1) as u32) << 5;
            Instruction::Lw { rd, rs1: 2, imm: imm as i32 }
        }
        (2, 4) => {
            // C.JR / C.MV / C.JALR / C.ADD / C.EBREAK
            let bit12 = (half >> 12) & 0x1;
            if bit12 == 0 {
                if rs2 == 0 && rd != 0 {
                    // C.JR rs1 → JALR x0, rs1, 0
                    Instruction::Jalr { rd: 0, rs1: rd, imm: 0 }
                } else if rs2 != 0 && rd != 0 {
                    // C.MV rd, rs2 → ADD rd, x0, rs2
                    Instruction::Add { rd, rs1: 0, rs2 }
                } else {
                    Instruction::Unknown(half as u32)
                }
            } else {
                if rd == 0 && rs2 == 0 {
                    Instruction::Ebreak
                } else if rs2 == 0 && rd != 0 {
                    // C.JALR rs1 → JALR x1, rs1, 0
                    Instruction::Jalr { rd: 1, rs1: rd, imm: 0 }
                } else if rd != 0 && rs2 != 0 {
                    // C.ADD rd, rs2 → ADD rd, rd, rs2
                    Instruction::Add { rd, rs1: rd, rs2 }
                } else {
                    Instruction::Unknown(half as u32)
                }
            }
        }
        (2, 6) => {
            // C.SWSP rs2, uimm → SW rs2, offset(x2)
            let imm = (((half >> 7) & 0x3) as u32) << 6
                | (((half >> 9) & 0xF) as u32) << 2;
            Instruction::Sw { rs1: 2, rs2, imm: imm as i32 }
        }

        _ => Instruction::Unknown(half as u32),
    }
}

// C.JAL / C.J immediate: 11-bit signed offset, multiply of 2.
fn c_jal_imm(half: u16) -> i32 {
    let raw = (((half >> 12) & 0x1) as u32) << 11   // bit 11 (sign)
        | (((half >> 8) & 0x1) as u32) << 10         // bit 10
        | (((half >> 9) & 0x3) as u32) << 8          // bits 9:8
        | (((half >> 6) & 0x1) as u32) << 7          // bit 7
        | (((half >> 7) & 0x1) as u32) << 6          // bit 6
        | (((half >> 2) & 0x1) as u32) << 5          // bit 5
        | (((half >> 11) & 0x1) as u32) << 4         // bit 4
        | (((half >> 3) & 0x7) as u32) << 1;         // bits 3:1
    sign_extend(raw as u32, 12)
}

// C.BEQZ / C.BNEZ immediate: 8-bit signed offset, multiply of 2.
fn c_branch_imm(half: u16) -> i32 {
    let raw = (((half >> 12) & 0x1) as u32) << 8
        | (((half >> 5) & 0x3) as u32) << 6
        | (((half >> 2) & 0x1) as u32) << 5
        | (((half >> 10) & 0x3) as u32) << 3
        | (((half >> 3) & 0x3) as u32) << 1;
    sign_extend(raw as u32, 9)
}

fn sign_extend(val: u32, bits: u32) -> i32 {
    let sign_bit = 1u32 << (bits - 1);
    if val & sign_bit != 0 {
        (val | (!0u32 << bits)) as i32
    } else {
        val as i32
    }
}
