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
    // RV32M Extension
    Mul { rd: u8, rs1: u8, rs2: u8 },
    Mulh { rd: u8, rs1: u8, rs2: u8 },
    Mulhsu { rd: u8, rs1: u8, rs2: u8 },
    Mulhu { rd: u8, rs1: u8, rs2: u8 },
    Div { rd: u8, rs1: u8, rs2: u8 },
    Divu { rd: u8, rs1: u8, rs2: u8 },
    Rem { rd: u8, rs1: u8, rs2: u8 },
    Remu { rd: u8, rs1: u8, rs2: u8 },

    // RV32C Extension (Selection of common ones)
    CAddi { rd: u8, imm: i32 },
    CLw { rd: u8, rs1: u8, imm: u32 },
    CSw { rs2: u8, rs1: u8, imm: u32 },
    CJr { rs1: u8 },
    CJalr { rs1: u8 },
    CLi { rd: u8, imm: i32 },
    CMv { rd: u8, rs2: u8 },
    CAddi16sp { imm: i32 },
    CAddi4spn { rd: u8, imm: u32 },
    CSli { rd: u8, shamt: u8 },
    CLwsp { rd: u8, imm: u32 },
    CSwsp { rs2: u8, imm: u32 },
    CJ { imm: i32 },
    CBeqz { rs1: u8, imm: i32 },
    CBnez { rs1: u8, imm: i32 },

    Unknown(u32),
}

pub fn decode_rv32(inst: u32) -> Instruction {
    if (inst & 0x3) != 0x3 {
        return decode_rv32c((inst & 0xFFFF) as u16);
    }
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
                // RV32M Extension
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

pub fn decode_rv32c(inst: u16) -> Instruction {
    let op = inst & 0x3;
    let funct3 = (inst >> 13) & 0x7;

    match op {
        0 => {
            // Quadrant 0
            match funct3 {
                0 => {
                    // C.ADDI4SPN
                    let imm = ((inst >> 7) & 0x30) << 4 | // imm[5:4]
                              ((inst >> 1) & 0x3C0) |      // imm[9:6]
                              ((inst >> 4) & 0x4) |       // imm[2]
                              ((inst >> 2) & 0x8); // imm[3]
                    let rd = (((inst >> 2) & 0x7) + 8) as u8;
                    Instruction::CAddi4spn {
                        rd,
                        imm: imm as u32,
                    }
                }
                2 => {
                    // C.LW
                    let imm = ((inst >> 4) & 0x4) |    // imm[2]
                              ((inst >> 7) & 0x38) |   // imm[5:3]
                              ((inst << 1) & 0x40); // imm[6]
                    let rs1 = (((inst >> 7) & 0x7) + 8) as u8;
                    let rd = (((inst >> 2) & 0x7) + 8) as u8;
                    Instruction::CLw {
                        rd,
                        rs1,
                        imm: imm as u32,
                    }
                }
                6 => {
                    // C.SW
                    let imm = ((inst >> 4) & 0x4) |    // imm[2]
                              ((inst >> 7) & 0x38) |   // imm[5:3]
                              ((inst << 1) & 0x40); // imm[6]
                    let rs1 = (((inst >> 7) & 0x7) + 8) as u8;
                    let rs2 = (((inst >> 2) & 0x7) + 8) as u8;
                    Instruction::CSw {
                        rs2,
                        rs1,
                        imm: imm as u32,
                    }
                }
                _ => Instruction::Unknown(inst as u32),
            }
        }
        1 => {
            // Quadrant 1
            match funct3 {
                0 => {
                    // C.ADDI / C.NOP
                    let rd = ((inst >> 7) & 0x1F) as u8;
                    let imm = (((inst >> 12) & 1) << 5) | ((inst >> 2) & 0x1F);
                    let signed_imm = if (imm & 0x20) != 0 {
                        (imm as i32) | !0x3F
                    } else {
                        imm as i32
                    };
                    Instruction::CAddi {
                        rd,
                        imm: signed_imm,
                    }
                }
                1 => {
                    // C.JAL (RV32C only)
                    let imm = ((inst >> 2) & 0xE) |     // imm[3:1]
                              ((inst >> 7) & 0x10) |    // imm[4]
                              ((inst >> 2) & 0x20) |    // imm[5]
                              ((inst >> 1) & 0x40) |    // imm[6]
                              ((inst >> 11) & 0x80) |   // imm[7]
                              ((inst >> 1) & 0x300) |   // imm[9:8]
                              ((inst >> 1) & 0x400) |   // imm[10]
                              ((inst >> 1) & 0x800); // imm[11]
                    let signed_imm = if (imm & 0x800) != 0 {
                        (imm as i32) | !0xFFF
                    } else {
                        imm as i32
                    };
                    Instruction::Jal {
                        rd: 1,
                        imm: signed_imm,
                    }
                }
                2 => {
                    // C.LI
                    let rd = ((inst >> 7) & 0x1F) as u8;
                    let imm = ((inst >> 2) & 0x1F) | (((inst >> 12) & 1) << 5);
                    let signed_imm = if (imm & 0x20) != 0 {
                        (imm as i32) | !0x3F
                    } else {
                        imm as i32
                    };
                    Instruction::CLi {
                        rd,
                        imm: signed_imm,
                    }
                }
                3 => {
                    let rd = ((inst >> 7) & 0x1F) as u8;
                    if rd == 2 {
                        // C.ADDI16SP
                        let imm = ((inst >> 3) & 0x180) | // imm[8:7]
                                  ((inst >> 2) & 0x40) |  // imm[6]
                                  ((inst >> 1) & 0x20) |  // imm[5]
                                  ((inst >> 4) & 0x10) |  // imm[4]
                                  (((inst >> 12) & 1) << 9); // imm[9]
                        let signed_imm = if (imm & 0x200) != 0 {
                            (imm as i32) | !0x3FF
                        } else {
                            imm as i32
                        };
                        Instruction::CAddi16sp { imm: signed_imm }
                    } else {
                        // C.LUI
                        let imm = ((inst >> 2) & 0x1F) | (((inst >> 12) & 1) << 5);
                        let signed_imm = if (imm & 0x20) != 0 {
                            (imm as i32) | !0x3F
                        } else {
                            imm as i32
                        };
                        Instruction::Lui {
                            rd,
                            imm: (signed_imm << 12) as u32,
                        }
                    }
                }
                5 => {
                    // C.J
                    let imm = ((inst >> 2) & 0xE) |     // imm[3:1]
                              ((inst >> 7) & 0x10) |    // imm[4]
                              ((inst >> 2) & 0x20) |    // imm[5]
                              ((inst >> 1) & 0x40) |    // imm[6]
                              ((inst >> 11) & 0x80) |   // imm[7]
                              ((inst >> 1) & 0x300) |   // imm[9:8]
                              ((inst >> 1) & 0x400) |   // imm[10]
                              ((inst >> 1) & 0x800); // imm[11]
                    let signed_imm = if (imm & 0x800) != 0 {
                        (imm as i32) | !0xFFF
                    } else {
                        imm as i32
                    };
                    Instruction::CJ { imm: signed_imm }
                }
                6 => {
                    // C.BEQZ
                    let rs1 = (((inst >> 7) & 0x7) + 8) as u8;
                    let imm = ((inst >> 2) & 0x6) |     // imm[2:1]
                              ((inst >> 7) & 0x60) |    // imm[6:5]
                              ((inst >> 1) & 0x18) |    // imm[4:3]
                              (((inst >> 12) & 1) << 8) | // imm[8]
                              ((inst >> 2) & 0x80); // imm[7]
                    let signed_imm = if (imm & 0x100) != 0 {
                        (imm as i32) | !0x1FF
                    } else {
                        imm as i32
                    };
                    Instruction::CBeqz {
                        rs1,
                        imm: signed_imm,
                    }
                }
                7 => {
                    // C.BNEZ
                    let rs1 = (((inst >> 7) & 0x7) + 8) as u8;
                    let imm = ((inst >> 2) & 0x6) |     // imm[2:1]
                              ((inst >> 7) & 0x60) |    // imm[6:5]
                              ((inst >> 1) & 0x18) |    // imm[4:3]
                              (((inst >> 12) & 1) << 8) | // imm[8]
                              ((inst >> 2) & 0x80); // imm[7]
                    let signed_imm = if (imm & 0x100) != 0 {
                        (imm as i32) | !0x1FF
                    } else {
                        imm as i32
                    };
                    Instruction::CBnez {
                        rs1,
                        imm: signed_imm,
                    }
                }
                _ => Instruction::Unknown(inst as u32),
            }
        }
        2 => {
            // Quadrant 2
            match funct3 {
                0 => {
                    // C.SLLI
                    let rd = ((inst >> 7) & 0x1F) as u8;
                    let shamt = ((inst >> 12) & 0x1) as u8 | ((inst >> 2) & 0x1F) as u8;
                    Instruction::CSli { rd, shamt }
                }
                2 => {
                    // C.LWSP
                    let rd = ((inst >> 7) & 0x1F) as u8;
                    // inst[12] -> imm[5]
                    // inst[6:4] -> imm[4:2]
                    // inst[3:2] -> imm[7:6]
                    let imm = (((inst >> 2) & 0x3) << 6)
                        | (((inst >> 12) & 0x1) << 5)
                        | (((inst >> 4) & 0x7) << 2);
                    Instruction::CLwsp {
                        rd,
                        imm: imm as u32,
                    }
                }
                4 => {
                    let bit12 = (inst >> 12) & 1;
                    let rs1 = ((inst >> 7) & 0x1F) as u8;
                    let rs2 = ((inst >> 2) & 0x1F) as u8;
                    if bit12 == 0 {
                        if rs2 == 0 {
                            // C.JR
                            Instruction::CJr { rs1 }
                        } else {
                            // C.MV
                            Instruction::CMv { rd: rs1, rs2 }
                        }
                    } else if rs1 != 0 && rs2 == 0 {
                        // C.JALR
                        Instruction::CJalr { rs1 }
                    } else {
                        // C.ADD
                        Instruction::Add { rd: rs1, rs1, rs2 }
                    }
                }
                6 => {
                    // C.SWSP
                    let rs2 = ((inst >> 2) & 0x1F) as u8;
                    // inst[12:9] -> imm[5:2]
                    // inst[8:7] -> imm[7:6]
                    let imm = (((inst >> 9) & 0xF) << 2) | (((inst >> 7) & 0x3) << 6);
                    Instruction::CSwsp {
                        rs2,
                        imm: imm as u32,
                    }
                }
                _ => Instruction::Unknown(inst as u32),
            }
        }
        _ => Instruction::Unknown(inst as u32),
    }
}
