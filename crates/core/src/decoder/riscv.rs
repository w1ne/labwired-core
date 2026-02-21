// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

/// RISC-V RV32I Base Integer Instruction Set
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
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
            // OP
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
