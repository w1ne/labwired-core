// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Instruction {
    Nop,
    MovImm {
        rd: u8,
        imm: u8,
    }, // MOV Rd, #imm8
    Branch {
        offset: i32,
    }, // B <label>
    BranchCond {
        cond: u8,
        offset: i32,
    }, // Bcc <label>

    // Arithmetic & Logic
    AddReg {
        rd: u8,
        rn: u8,
        rm: u8,
    }, // ADD Rd, Rn, Rm
    AddImm3 {
        rd: u8,
        rn: u8,
        imm: u8,
    }, // ADD Rd, Rn, #imm3
    AddImm8 {
        rd: u8,
        imm: u8,
    }, // ADD Rd, #imm8

    SubReg {
        rd: u8,
        rn: u8,
        rm: u8,
    }, // SUB Rd, Rn, Rm
    SubImm3 {
        rd: u8,
        rn: u8,
        imm: u8,
    }, // SUB Rd, Rn, #imm3
    SubImm8 {
        rd: u8,
        imm: u8,
    }, // SUB Rd, #imm8

    CmpImm {
        rn: u8,
        imm: u8,
    }, // CMP Rn, #imm8
    CmpReg {
        rn: u8,
        rm: u8,
    }, // CMP Rn, Rm
    Cmn {
        rn: u8,
        rm: u8,
    }, // CMN Rn, Rm
    Tst {
        rn: u8,
        rm: u8,
    }, // TST Rn, Rm
    MovReg {
        rd: u8,
        rm: u8,
    }, // MOV Rd, Rm (High registers)
    Movw {
        rd: u8,
        imm: u16,
    }, // MOVW Rd, #imm16
    Movt {
        rd: u8,
        imm: u16,
    }, // MOVT Rd, #imm16

    AddSp {
        imm: u16,
    }, // ADD SP, SP, #imm
    SubSp {
        imm: u16,
    }, // SUB SP, SP, #imm
    AddRegHigh {
        rd: u8,
        rm: u8,
    }, // ADD Rd, Rm (at least one high register)
    Cpsie, // CPSIE i
    Cpsid, // CPSID i

    And {
        rd: u8,
        rm: u8,
    }, // AND Rd, Rm
    Bic {
        rd: u8,
        rm: u8,
    }, // BIC Rd, Rm
    Orr {
        rd: u8,
        rm: u8,
    }, // ORR Rd, Rm
    Eor {
        rd: u8,
        rm: u8,
    }, // EOR Rd, Rm
    Mvn {
        rd: u8,
        rm: u8,
    }, // MVN Rd, Rm

    // Shifts
    Lsl {
        rd: u8,
        rm: u8,
        imm: u8,
    }, // LSL Rd, Rm, #imm5
    Lsr {
        rd: u8,
        rm: u8,
        imm: u8,
    }, // LSR Rd, Rm, #imm5
    Asr {
        rd: u8,
        rm: u8,
        imm: u8,
    }, // ASR Rd, Rm, #imm5
    Adc {
        rd: u8,
        rm: u8,
    }, // ADC Rd, Rm
    Sbc {
        rd: u8,
        rm: u8,
    }, // SBC Rd, Rm
    Ror {
        rd: u8,
        rm: u8,
    }, // ROR Rd, Rm

    // Memory
    LdrImm {
        rt: u8,
        rn: u8,
        imm: u8,
    }, // LDR Rt, [Rn, #imm] (imm is *4)
    StrImm {
        rt: u8,
        rn: u8,
        imm: u8,
    }, // STR Rt, [Rn, #imm] (imm is *4)
    LdrLit {
        rt: u8,
        imm: u16,
    }, // LDR Rt, [PC, #imm]
    LdrImm32 {
        rt: u8,
        rn: u8,
        imm12: u16,
    },
    StrImm32 {
        rt: u8,
        rn: u8,
        imm12: u16,
    },
    LdrbImm {
        rt: u8,
        rn: u8,
        imm: u8,
    }, // LDRB Rt, [Rn, #imm]
    LdrbReg {
        rt: u8,
        rn: u8,
        rm: u8,
    }, // LDRB Rt, [Rn, Rm]
    StrbImm {
        rt: u8,
        rn: u8,
        imm: u8,
    }, // STRB Rt, [Rn, #imm]
    LdrhImm {
        rt: u8,
        rn: u8,
        imm: u8,
    }, // LDRH Rt, [Rn, #imm] (imm is *2)
    StrhImm {
        rt: u8,
        rn: u8,
        imm: u8,
    }, // STRH Rt, [Rn, #imm] (imm is *2)

    // Stack
    Push {
        registers: u8,
        m: bool,
    }, // PUSH {Rlist, LR?}
    Pop {
        registers: u8,
        p: bool,
    }, // POP {Rlist, PC?}
    Ldm {
        rn: u8,
        registers: u8,
    }, // LDM Rn, {Rlist}
    Stm {
        rn: u8,
        registers: u8,
    }, // STM Rn, {Rlist}

    // Control Flow
    Cbz {
        rn: u8,
        imm: u8,
    }, // CBZ Rn, <label>
    Cbnz {
        rn: u8,
        imm: u8,
    }, // CBNZ Rn, <label>
    Bl {
        offset: i32,
    }, // BL <label> (32-bit T1+T2)
    Bx {
        rm: u8,
    }, // BX Rm
    Mul {
        rd: u8,
        rn: u8,
    }, // MUL Rd, Rn (Rd = Rn * Rd)

    // SP-Relative
    LdrSp {
        rt: u8,
        imm: u16,
    }, // LDR Rt, [SP, #imm]
    StrSp {
        rt: u8,
        imm: u16,
    }, // STR Rt, [SP, #imm]
    AddSpReg {
        rd: u8,
        imm: u16,
    }, // ADD Rd, SP, #imm (ADR-like for SP)

    // Other ALU
    Uxtb {
        rd: u8,
        rm: u8,
    }, // UXTB Rd, Rm
    Adr {
        rd: u8,
        imm: u16,
    }, // ADR Rd, <label>
    AsrReg {
        rd: u8,
        rm: u8,
    }, // ASR Rd, Rm
    LdrReg {
        rt: u8,
        rn: u8,
        rm: u8,
    }, // LDR Rt, [Rn, Rm]
    Rsbs {
        rd: u8,
        rn: u8,
    }, // RSBS Rd, Rn, #0

    // Bit Field Instructions (Thumb-2)
    Bfi {
        rd: u8,
        rn: u8,
        lsb: u8,
        width: u8,
    }, // BFI Rd, Rn, #lsb, #width
    Bfc {
        rd: u8,
        lsb: u8,
        width: u8,
    }, // BFC Rd, #lsb, #width
    Sbfx {
        rd: u8,
        rn: u8,
        lsb: u8,
        width: u8,
    }, // SBFX Rd, Rn, #lsb, #width
    Ubfx {
        rd: u8,
        rn: u8,
        lsb: u8,
        width: u8,
    }, // UBFX Rd, Rn, #lsb, #width

    // Misc Thumb-2 Instructions
    Clz {
        rd: u8,
        rm: u8,
    }, // CLZ Rd, Rm
    Rbit {
        rd: u8,
        rm: u8,
    }, // RBIT Rd, Rm
    Rev {
        rd: u8,
        rm: u8,
    }, // REV Rd, Rm
    Rev16 {
        rd: u8,
        rm: u8,
    }, // REV16 Rd, Rm
    RevSh {
        rd: u8,
        rm: u8,
    }, // REVSH Rd, Rm
    Udiv {
        rd: u8,
        rn: u8,
        rm: u8,
    }, // UDIV Rd, Rn, Rm
    Sdiv {
        rd: u8,
        rn: u8,
        rm: u8,
    }, // SDIV Rd, Rn, Rm

    DataProc32 {
        op: u8,
        rn: u8,
        rd: u8,
        rm: u8,
        imm5: u8,
        shift_type: u8,
        set_flags: bool,
    },
    DataProcImm32 {
        op: u8,
        rn: u8,
        rd: u8,
        imm12: u32,
        set_flags: bool,
    },
    ShiftReg32 {
        rd: u8,
        rn: u8,
        rm: u8,
        shift_type: u8,
    }, // LSL/LSR/ASR/ROR (register)

    It {
        cond: u8,
        mask: u8,
    }, // IT <cond> <mask...ish>

    Ldrd {
        rt: u8,
        rt2: u8,
        rn: u8,
        imm8: u32,
    },
    Strd {
        rt: u8,
        rt2: u8,
        rn: u8,
        imm8: u32,
    },
    Tbb {
        rn: u8,
        rm: u8,
    },
    Tbh {
        rn: u8,
        rm: u8,
    },

    Unknown(u16),
    Unknown32(u16, u16),
}

/// Decodes a 16-bit Thumb instruction
pub fn decode_thumb_16(opcode: u16) -> Instruction {
    // 0. Shift (immediate), add, subtract, move, and compare
    // 0.1 Shift (immediate) (T1): 000xx ...
    if (opcode & 0xE000) == 0x0000 {
        let op = (opcode >> 11) & 0x3;
        let imm5 = ((opcode >> 6) & 0x1F) as u8;
        let rm = ((opcode >> 3) & 0x7) as u8;
        let rd = (opcode & 0x7) as u8;

        match op {
            0 => return Instruction::Lsl { rd, rm, imm: imm5 },
            1 => return Instruction::Lsr { rd, rm, imm: imm5 },
            2 => return Instruction::Asr { rd, rm, imm: imm5 },
            _ => {} // Possibly Add/Sub (Register/Imm3) handled later
        }
    }

    // 1. Move Immediate (T1): 0010 0ddd iiii iiii
    if (opcode & 0xE000) == 0x2000 {
        let op = (opcode >> 11) & 0x3;
        let rd = ((opcode >> 8) & 0x7) as u8;
        let imm = (opcode & 0xFF) as u8;

        return match op {
            0 => Instruction::MovImm { rd, imm },     // 00100 = MOV
            1 => Instruction::CmpImm { rn: rd, imm }, // 00101 = CMP
            2 => Instruction::AddImm8 { rd, imm },    // 00110 = ADD
            3 => Instruction::SubImm8 { rd, imm },    // 00111 = SUB
            _ => Instruction::Unknown(opcode),
        };
    }

    // 2. Add/Sub (Register/Imm3) (T1): 0001 1xx ...
    if (opcode & 0xF800) == 0x1800 {
        let op_sub = (opcode >> 9) & 0x3;
        let rm_imm = ((opcode >> 6) & 0x7) as u8;
        let rn = ((opcode >> 3) & 0x7) as u8;
        let rd = (opcode & 0x7) as u8;

        return match op_sub {
            0 => Instruction::AddReg { rd, rn, rm: rm_imm },
            1 => Instruction::SubReg { rd, rn, rm: rm_imm },
            2 => Instruction::AddImm3 {
                rd,
                rn,
                imm: rm_imm,
            },
            3 => Instruction::SubImm3 {
                rd,
                rn,
                imm: rm_imm,
            },
            _ => unreachable!(),
        };
    }

    // 3. ALU Operations (T1): 0100 00xx ...
    if (opcode & 0xFC00) == 0x4000 {
        let op_alu = (opcode >> 6) & 0xF;
        let rm = ((opcode >> 3) & 0x7) as u8;
        let rd = (opcode & 0x7) as u8;

        return match op_alu {
            0x0 => Instruction::And { rd, rm },        // AND
            0x1 => Instruction::Eor { rd, rm },        // EOR
            0x5 => Instruction::Adc { rd, rm },        // ADC
            0x6 => Instruction::Sbc { rd, rm },        // SBC
            0x7 => Instruction::Ror { rd, rm },        // ROR
            0x8 => Instruction::Tst { rn: rd, rm },    // TST
            0x9 => Instruction::Rsbs { rd, rn: rm },   // RSBS Rd, Rn, #0
            0xA => Instruction::CmpReg { rn: rd, rm }, // CMP (register) T1
            0xB => Instruction::Cmn { rn: rd, rm },    // CMN
            0xC => Instruction::Orr { rd, rm },        // ORR
            0xD => Instruction::Mul { rd, rn: rm },    // MUL
            0xE => Instruction::Bic { rd, rm },        // BIC
            0xF => Instruction::Mvn { rd, rm },        // MVN
            _ => Instruction::Unknown(opcode),
        };
    }

    // 3.1 Special Data / Branch Exchange (T1): 0100 01xx ...
    if (opcode & 0xFC00) == 0x4400 {
        let op = (opcode >> 8) & 0x3;
        match op {
            0 => {
                // ADD (register) T2 (High registers)
                let rd = (((opcode >> 4) & 0x8) | (opcode & 0x7)) as u8;
                let rm = ((opcode >> 3) & 0xF) as u8;
                return Instruction::AddRegHigh { rd, rm };
            }
            1 => {
                // CMP (register) T2 (High registers)
                let n = ((opcode >> 7) & 0x1) << 3;
                let rn = (n | (opcode & 0x7)) as u8;
                let rm = ((opcode >> 3) & 0xF) as u8;
                return Instruction::CmpReg { rn, rm };
            }
            2 => {
                // MOV (register) T1
                let d = ((opcode >> 7) & 0x1) << 3;
                let rd = (d | (opcode & 0x7)) as u8;
                let rm = ((opcode >> 3) & 0xF) as u8;
                return Instruction::MovReg { rd, rm };
            }
            3 => {
                // BX (T1)
                let rm = ((opcode >> 3) & 0xF) as u8;
                return Instruction::Bx { rm };
            }
            _ => return Instruction::Unknown(opcode),
        }
    }

    // 4. Load/Store (Imm5) (T1): 0110 0... -> STR, 0110 1... -> LDR
    // Format: 0110 Liii iinn nttt
    if (opcode & 0xF000) == 0x6000 {
        let is_load = (opcode & 0x0800) != 0;
        let imm5 = ((opcode >> 6) & 0x1F) as u8;
        // The immediate is scaled by 4 for word access
        let imm = imm5 << 2;
        let rn = ((opcode >> 3) & 0x7) as u8;
        let rt = (opcode & 0x7) as u8;

        if is_load {
            return Instruction::LdrImm { rt, rn, imm }; // 0x68xx
        } else {
            return Instruction::StrImm { rt, rn, imm }; // 0x60xx
        }
    }

    // 4.3 Load/Store Byte (Imm5) (T1): 0111 Liii iinn nttt
    if (opcode & 0xF000) == 0x7000 {
        let is_load = (opcode & 0x0800) != 0;
        let imm = ((opcode >> 6) & 0x1F) as u8;
        let rn = ((opcode >> 3) & 0x7) as u8;
        let rt = (opcode & 0x7) as u8;

        if is_load {
            return Instruction::LdrbImm { rt, rn, imm }; // 0x78xx
        } else {
            return Instruction::StrbImm { rt, rn, imm }; // 0x70xx
        }
    }

    // 4.5 Load/Store Halfword (Imm5) (T1): 1000 Liii iinn nttt
    if (opcode & 0xF000) == 0x8000 {
        let is_load = (opcode & 0x0800) != 0;
        let imm5 = ((opcode >> 6) & 0x1F) as u8;
        // The immediate is scaled by 2 for halfword access
        let imm = imm5 << 1;
        let rn = ((opcode >> 3) & 0x7) as u8;
        let rt = (opcode & 0x7) as u8;

        if is_load {
            return Instruction::LdrhImm { rt, rn, imm }; // 0x88xx
        } else {
            return Instruction::StrhImm { rt, rn, imm }; // 0x80xx
        }
    }

    // 4.1 LDR Literal (T1): 0100 1ttt iiii iiii
    if (opcode & 0xF800) == 0x4800 {
        let rt = ((opcode >> 8) & 0x7) as u8;
        let imm8 = opcode & 0xFF;
        return Instruction::LdrLit { rt, imm: imm8 << 2 };
    }

    // 4.2 LDR (register) (T1): 0101 100 mmm nnn ttt
    if (opcode & 0xFE00) == 0x5800 {
        let rm = ((opcode >> 6) & 0x7) as u8;
        let rn = ((opcode >> 3) & 0x7) as u8;
        let rt = (opcode & 0x7) as u8;
        return Instruction::LdrReg { rt, rn, rm };
    }

    // 4.2 LDRB (register) (T1): 0101 110 mmm nnn ttt
    if (opcode & 0xFE00) == 0x5C00 {
        let rm = ((opcode >> 6) & 0x7) as u8;
        let rn = ((opcode >> 3) & 0x7) as u8;
        let rt = (opcode & 0x7) as u8;
        return Instruction::LdrbReg { rt, rn, rm };
    }

    // 4.2 PUSH/POP
    // PUSH: 1011 010M rrrr rrrr (0xB400)
    if (opcode & 0xFE00) == 0xB400 {
        let m = (opcode & 0x0100) != 0; // LR saved?
        let registers = (opcode & 0xFF) as u8;
        return Instruction::Push { registers, m };
    }
    // POP: 1011 110P rrrr rrrr (0xBC00)
    if (opcode & 0xFE00) == 0xBC00 {
        let p = (opcode & 0x0100) != 0; // PC restored?
        let registers = (opcode & 0xFF) as u8;
        return Instruction::Pop { registers, p };
    }

    // 6. SP-relative Load/Store (T1): 1001 Lttt iiii iiii (0x9000 mask 0xF000)
    // STR: 1001 0... (0x90xx)
    // LDR: 1001 1... (0x98xx)
    if (opcode & 0xF000) == 0x9000 {
        let rt = ((opcode >> 8) & 0x7) as u8;
        let imm8 = opcode & 0xFF;
        // Immediate is scaled by 4
        let imm = imm8 << 2;

        if (opcode & 0x0800) != 0 {
            return Instruction::LdrSp { rt, imm };
        } else {
            return Instruction::StrSp { rt, imm };
        }
    }

    // 6.5 Load/Store Multiple (T1): 1100 Lnnn rrrr rrrr
    if (opcode & 0xF000) == 0xC000 {
        let is_load = (opcode & 0x0800) != 0;
        let rn = ((opcode >> 8) & 0x7) as u8;
        let registers = (opcode & 0xFF) as u8;

        if is_load {
            return Instruction::Ldm { rn, registers }; // 0xC8xx
        } else {
            return Instruction::Stm { rn, registers }; // 0xC0xx
        }
    }

    // 7. Conditional Branch (Bcc): 1101 xxxx iiii iiii
    if (opcode & 0xF000) == 0xD000 {
        let cond = ((opcode >> 8) & 0xF) as u8;
        // Don't match SWI (1101 1111 ...) -> cond 0xF is SWI
        if cond != 0xF {
            let mut offset = (opcode & 0xFF) as i32;
            // Sign extend 8-bit to 32-bit
            if (offset & 0x80) != 0 {
                offset |= !0xFF;
            }
            return Instruction::BranchCond {
                cond,
                offset: offset << 1,
            };
        }
    }

    // 7.1 ADR (T1) / ADD (SP) (T1)
    if (opcode & 0xF000) == 0xA000 {
        let is_add_sp = (opcode & 0x0800) != 0;
        let rd = ((opcode >> 8) & 0x7) as u8;
        let imm8 = opcode & 0xFF;
        let imm = imm8 << 2;
        if is_add_sp {
            return Instruction::AddSpReg { rd, imm };
        } else {
            return Instruction::Adr { rd, imm };
        }
    }

    // 8. Branch (T1/T2)
    // Unconditional Branch T2: 1110 0...
    if (opcode & 0xF800) == 0xE000 {
        let mut offset = (opcode & 0x7FF) as i32;
        if (offset & 0x400) != 0 {
            offset |= !0x7FF;
        }
        return Instruction::Branch {
            offset: offset << 1,
        };
    }

    // 8.1 Misc (T1) (0xBxxx)
    if (opcode & 0xF000) == 0xB000 {
        // UXTB (T1): 1011 0010 11 mmm ddd -> 0xB2C0 base
        if (opcode & 0xFFC0) == 0xB2C0 {
            let rm = ((opcode >> 3) & 0x7) as u8;
            let rd = (opcode & 0x7) as u8;
            return Instruction::Uxtb { rd, rm };
        }

        // CBZ/CBNZ (T1): 1011 op i 1 imm5 rn
        if (opcode & 0xF500) == 0xB100 {
            let op = (opcode >> 11) & 1;
            let i = (opcode >> 9) & 1;
            let imm5 = ((opcode >> 3) & 0x1F) as u8;
            let rn = (opcode & 0x7) as u8;
            let imm = ((i << 6) as u8) | (imm5 << 1);
            if op == 0 {
                return Instruction::Cbz { rn, imm };
            } else {
                return Instruction::Cbnz { rn, imm };
            }
        }

        // HINT/IT (T1): 1011 1111 ...
        if (opcode & 0xFF00) == 0xBF00 {
            let cond = ((opcode >> 4) & 0xF) as u8;
            let mask = (opcode & 0xF) as u8;
            if mask != 0 {
                return Instruction::It { cond, mask };
            }
            return Instruction::Nop;
        }
    }

    // 6. 32-bit Instruction Prefix (0xE800-0xFFFF range, excluding B/BL 16-bit range)
    // 32-bit Thumb instructions start with 111, with bits [12:11] != 00
    if (opcode & 0xE000) == 0xE000 && (opcode & 0x1800) != 0 {
        return Instruction::Unknown(opcode);
    }

    // ADD/SUB SP (T1): 1011 0000 x iii iiii
    if (opcode & 0xFF00) == 0xB000 {
        let is_sub = (opcode & 0x0080) != 0;
        let imm7 = opcode & 0x7F;
        let imm = imm7 << 2;
        if is_sub {
            return Instruction::SubSp { imm };
        } else {
            return Instruction::AddSp { imm };
        }
    }

    // CPS (T1): 1011 0110 011 effect 0 interrupt_flags (0xB660 mask 0xFFE0)
    if (opcode & 0xFFEF) == 0xB662 {
        // Matches B662 or B672 (ignoring bit 4)
        let disable = (opcode & 0x0010) != 0;
        if disable {
            return Instruction::Cpsid;
        } else {
            return Instruction::Cpsie;
        }
    }

    // NOP: 1011 1111 0000 0000 -> 0xBF00
    if opcode == 0xBF00 {
        return Instruction::Nop;
    }

    Instruction::Unknown(opcode)
}

/// Decodes a 32-bit Thumb instruction (requires two 16-bit halfwords)
pub fn decode_thumb_32(h1: u16, h2: u16) -> Instruction {
    // 32-bit Thumb instruction encoding:
    // First Halfword: 1110 1... or 1111 ...

    // Data processing (modified immediate) / Plain binary immediate
    // 1111 0 <i1> 0 <op> <S> <Rn> 0 <imm3> <Rd> <imm8>
    if (h1 & 0xFB00) == 0xF000 && (h2 & 0x8000) == 0 && (h2 & 0x0700) != 0x0700 {
        let op = ((h1 >> 5) & 0xF) as u8;
        let s = (h1 & 0x0010) != 0;
        let rn = (h1 & 0xF) as u8;
        let rd = ((h2 >> 8) & 0xF) as u8;

        let i = (h1 >> 10) & 1;
        let imm3 = (h2 >> 12) & 7;
        let imm8 = h2 & 0xFF;
        let imm12 = (i << 11) | (imm3 << 8) | imm8;

        return Instruction::DataProcImm32 {
            op,
            rn,
            rd,
            imm12: imm12 as u32,
            set_flags: s,
        };
    }

    // Data Processing (Reg) - For LSL, LSR, ASR, ROR, etc
    // 1110 1010 ... (EA..)
    if (h1 & 0xFE00) == 0xEA00 && (h2 & 0x8000) == 0 {
        let op = ((h1 >> 5) & 0xF) as u8;
        let s = ((h1 >> 4) & 0x1) != 0;
        let rn = (h1 & 0xF) as u8;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let rm = (h2 & 0xF) as u8;

        let imm3 = ((h2 >> 12) & 0x7) as u8;
        let imm2 = ((h2 >> 6) & 0x3) as u8;
        let imm5 = (imm3 << 2) | imm2;
        let shift_type = ((h2 >> 4) & 0x3) as u8;

        return Instruction::DataProc32 {
            op,
            rn,
            rd,
            rm,
            imm5,
            shift_type,
            set_flags: s,
        };
    }

    // MOVW: 1111 0 i 10 0100 imm4 0 imm3 rd imm8 -> F24..
    if (h1 & 0xFBF0) == 0xF240 {
        let i = (h1 >> 10) & 1;
        let imm4 = h1 & 0xF;
        let imm3 = (h2 >> 12) & 7;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let imm8 = h2 & 0xFF;
        let imm16 = (imm4 << 12) | (i << 11) | (imm3 << 8) | imm8;
        return Instruction::Movw { rd, imm: imm16 };
    }

    // MOVT: 1111 0 i 10 1100 imm4 0 imm3 rd imm8 -> F2C..
    if (h1 & 0xFBF0) == 0xF2C0 {
        let i = (h1 >> 10) & 1;
        let imm4 = h1 & 0xF;
        let imm3 = (h2 >> 12) & 7;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let imm8 = h2 & 0xFF;
        let imm16 = (imm4 << 12) | (i << 11) | (imm3 << 8) | imm8;
        return Instruction::Movt { rd, imm: imm16 };
    }

    // Shift by register (Thumb-2), e.g. `LSL.W Rd, Rn, Rm`.
    // Example seen in H563 firmware path: FA01 F202
    if (h1 & 0xFFE0) == 0xFA00 && (h2 & 0xF0F0) == 0xF000 {
        let rn = (h1 & 0xF) as u8;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let rm = (h2 & 0xF) as u8;
        let shift_type = ((h2 >> 4) & 0x3) as u8;
        return Instruction::ShiftReg32 {
            rd,
            rn,
            rm,
            shift_type,
        };
    }

    // 1. Bitfield and Miscellaneous Instructions
    // Encoding: 1111 0011 0110 ... => F36x ...
    if (h1 & 0xFFF0) == 0xF360 {
        let _op = (h1 & 0xF) as u8;
        let rn = (h1 & 0xF) as u8;
        let rd = ((h2 >> 8) & 0xF) as u8;

        // BFI / BFC
        if (h2 & 0x8000) == 0 {
            let lsbbb = ((h2 >> 12) & 0x7) << 2 | ((h2 >> 6) & 0x3);
            // Encoding of msb in h2 is mmmmm
            let msb = (h2 & 0x1F) as u8;
            let lsb = lsbbb as u8; // 5 bits

            // Width = msb - lsb + 1
            // If msb < lsb, it's UNPREDICTABLE (or handled as 0 length?)
            if msb >= lsb {
                let width = msb - lsb + 1;
                if rn == 0xF {
                    return Instruction::Bfc { rd, lsb, width };
                } else {
                    return Instruction::Bfi { rd, rn, lsb, width };
                }
            }
        }
    }

    // ADR.W (T3): 1111 0 i 10 1010 .... F2A..
    if (h1 & 0xFBF0) == 0xF2A0 {
        let i = (h1 >> 10) & 1;
        let imm4 = h1 & 0xF;
        let imm3 = (h2 >> 12) & 7;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let imm8 = h2 & 0xFF;
        let imm12 = (imm4 << 12) | (i << 11) | (imm3 << 8) | imm8;
        return Instruction::Adr { rd, imm: imm12 };
    }

    // LDR.W (immediate) (T3): 1111 1000 1101 ... -> F8D..
    if (h1 & 0xFFF0) == 0xF8D0 {
        let rn = (h1 & 0xF) as u8;
        let rt = ((h2 >> 12) & 0xF) as u8;
        let imm12 = h2 & 0xFFF;
        return Instruction::LdrImm32 { rt, rn, imm12 };
    }

    // STR.W (immediate) (T3): 1111 1000 1100 ... -> F8C..
    if (h1 & 0xFFF0) == 0xF8C0 {
        let rn = (h1 & 0xF) as u8;
        let rt = ((h2 >> 12) & 0xF) as u8;
        let imm12 = h2 & 0xFFF;
        return Instruction::StrImm32 { rt, rn, imm12 };
    }

    // SBFX / UBFX
    if (h1 & 0xFFF0) == 0xF340 || (h1 & 0xFFF0) == 0xF3C0 {
        let is_unsigned = (h1 & 0x0080) != 0; // F3C0 vs F340 (0x0080 bit)
        let rn = (h1 & 0xF) as u8;
        let rd = ((h2 >> 8) & 0xF) as u8;

        let lsb = (((h2 >> 12) & 0x7) << 2 | ((h2 >> 6) & 0x3)) as u8; // 5 bits
        let width_m1 = (h2 & 0x1F) as u8;
        let width = width_m1 + 1;

        if is_unsigned {
            return Instruction::Ubfx { rd, rn, lsb, width };
        } else {
            return Instruction::Sbfx { rd, rn, lsb, width };
        }
    }

    // Misc Instructions: REV, REV16, REVSH, CLZ, RBIT
    // All start with 1111 1010 ... (FA..)
    if (h1 & 0xFF80) == 0xFA80 {
        let rn = (h1 & 0xF) as u8; // Rm in decoding usually
        let rm = rn; // Encoding uses Rm in H1

        let rd = ((h2 >> 8) & 0xF) as u8;

        if (h1 & 0xFFF0) == 0xFA90 {
            // FA9m
            let op = (h2 >> 4) & 0xF;
            match op {
                0x8 => return Instruction::Rev { rd, rm },
                0x9 => return Instruction::Rev16 { rd, rm },
                0xA => return Instruction::Rbit { rd, rm },
                0xB => return Instruction::RevSh { rd, rm },
                _ => {}
            }
        } else if (h1 & 0xFFF0) == 0xFAB0 {
            // FABm
            let op = (h2 >> 4) & 0xF;
            if op == 0x8 {
                return Instruction::Clz { rd, rm };
            }
        }
    }

    // UXTB.W (Thumb-2): 1111 1010 0100 1111 1111 <rd> 1000 <rm> -> FA4F ...
    // My log said FA23 F404 (LSR) and FA2E F303 (LSR).
    // Shift by register (Thumb-2), e.g. `LSL.W Rd, Rn, Rm`.
    // Encodings: 1111 1010 0... (FA0x to FA7x)
    if (h1 & 0xFF80) == 0xFA00 && (h2 & 0xF0F0) == 0xF000 {
        let rn = (h1 & 0xF) as u8;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let rm = (h2 & 0xF) as u8;
        let shift_type = ((h2 >> 4) & 0x3) as u8;
        return Instruction::ShiftReg32 {
            rd,
            rn,
            rm,
            shift_type,
        };
    }

    // UXTB.W etc (Miscellaneous)
    if (h1 & 0xFFC0) == 0xFA40 {
        let rd = ((h2 >> 8) & 0xF) as u8;
        let rm = (h2 & 0xF) as u8;
        return Instruction::Uxtb { rd, rm };
    }

    // LDRD / STRD / TBB / TBH (Encoding A1)
    if (h1 & 0xFE00) == 0xE800 {
        let op = ((h1 >> 7) & 3) as u8;
        let rn = (h1 & 0xF) as u8;
        let rt = ((h2 >> 12) & 0xF) as u8;
        let rt2 = ((h2 >> 8) & 0xF) as u8;
        let imm8 = (h2 & 0xFF) as u32;

        if (h1 & 0x01F0) == 0x00D0 && (h2 & 0xFFF0) == 0xF000 {
            let rm = (h2 & 0xF) as u8;
            let is_tbh = (h2 & 0x0010) != 0;
            if is_tbh {
                return Instruction::Tbh { rn, rm };
            } else {
                return Instruction::Tbb { rn, rm };
            }
        } else if op == 2 {
            return Instruction::Strd {
                rt,
                rt2,
                rn,
                imm8,
            };
        } else if op == 3 {
            return Instruction::Ldrd {
                rt,
                rt2,
                rn,
                imm8,
            };
        }
    }

    // B.W / BL
    if (h1 & 0xF800) == 0xF000 && (h2 & 0x8000) == 0x8000 {
        let s = ((h1 >> 10) & 0x1) as i32;
        let j1 = ((h2 >> 13) & 0x1) as i32;
        let j2 = ((h2 >> 11) & 0x1) as i32;
        let i1 = (!(j1 ^ s)) & 0x1;
        let i2 = (!(j2 ^ s)) & 0x1;
        let imm11 = (h2 & 0x7FF) as i32;

        let is_bl = (h2 & 0x1000) != 0;
        let imm_h1 = if is_bl {
            (h1 & 0x3FF) as i32
        } else {
            (h1 & 0x7FF) as i32
        };

        let mut offset = (s << 24)
            | (i1 << 23)
            | (i2 << 22)
            | (imm_h1 << 12)
            | (imm11 << 1);

        if (offset & (1 << 24)) != 0 {
            offset |= !0x01FF_FFFF;
        }

        if is_bl {
            return Instruction::Bl { offset };
        } else {
            return Instruction::Branch { offset };
        }
    }

    // UDIV / SDIV: 1111 1011 10x1 ... -> FB9.. / FBB..
    if (h1 & 0xFFD0) == 0xFB90 && (h2 & 0xF0F0) == 0xF0F0 {
        let is_unsigned = (h1 & 0x0020) != 0;
        let rn = (h1 & 0xF) as u8;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let rm = (h2 & 0xF) as u8;
        if is_unsigned {
            return Instruction::Udiv { rd, rn, rm };
        } else {
            return Instruction::Sdiv { rd, rn, rm };
        }
    }

    Instruction::Unknown32(h1, h2)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ... (existing tests)

    #[test]
    fn test_decode_bfi() {
        // BFI R0, R1, #4, #12
        // Encoding: T1
        // h1 = F361 (Rn=1)
        // h2: 0ii0 dddd iiim mmmm
        // i=0, d=0 (Rd=0)
        // imm3=1 (4>>2), imm2=0 (4&3) -> lsb=4
        // msb = 4+12-1 = 15 = 01111
        // h2 = 0000 0000 0001 01111 -> 0x010F ??
        // No.
        // imm3 is bits 14:12. imm2 bits 7:6.
        // imm3=1 -> 001. d=0 -> 0000.
        // h2 top: 0 001 0 0000 -> 0x10.
        // imm2=0 -> 00. msb=15 -> 01111.
        // h2 bot: 00 01111 -> 0x0F.
        // h2 = 0x100F.

        // Wait, decode logic:
        // lsb = ((h2 >> 12) & 0x7) << 2 | ((h2 >> 6) & 0x3);
        // (0x1<<2)|0 = 4. Correct.
        // msb = h2 & 0x1F = 0xF = 15. Correct.

        assert_eq!(
            decode_thumb_32(0xF361, 0x100F),
            Instruction::Bfi {
                rd: 0,
                rn: 1,
                lsb: 4,
                width: 12
            }
        );
    }

    #[test]
    fn test_decode_bfc() {
        // BFC R2, #8, #16
        // Rn=15 (0xF) -> h1 = F36F
        // lsb=8 -> imm3=2 (8>>2), imm2=0.
        // width=16 -> msb = 8+16-1 = 23 (0x17).
        // imm3=2 -> 010. d=2 -> 0010.
        // h2 top: 0 010 0 0010 -> 0x22..
        // imm2=0 -> 00. msb=23 -> 10111.
        // h2 bot: 00 10111 -> 0x17.
        // h2 = 0x2217.

        assert_eq!(
            decode_thumb_32(0xF36F, 0x2217),
            Instruction::Bfc {
                rd: 2,
                lsb: 8,
                width: 16
            }
        );
    }

    #[test]
    fn test_decode_ubfx() {
        // UBFX R3, R4, #2, #5
        // lsb=2 -> imm3=0, imm2=2.
        // width=5 -> widthm1=4.
        // h2 = 0x0384 (bits 7:6 = 10 -> imm2=2)
        assert_eq!(
            decode_thumb_32(0xF3C4, 0x0384),
            Instruction::Ubfx {
                rd: 3,
                rn: 4,
                lsb: 2,
                width: 5
            }
        );
    }

    #[test]
    fn test_decode_basic_arithmetic() {
        // MOVS R0, #10 (T1) -> 200A
        assert_eq!(
            decode_thumb_16(0x200A),
            Instruction::MovImm { rd: 0, imm: 10 }
        );

        // ADDS R1, #5 (T1) -> 3105
        assert_eq!(
            decode_thumb_16(0x3105),
            Instruction::AddImm8 { rd: 1, imm: 5 }
        );

        // SUBS R2, #3 (T1) -> 3A03
        assert_eq!(
            decode_thumb_16(0x3A03),
            Instruction::SubImm8 { rd: 2, imm: 3 }
        );
    }

    #[test]
    fn test_decode_branch() {
        // B <offset> (T1) -> E000 (offset=0) -> B PC+4
        // E7FE -> B -2 (infinite loop)
        // Offset is imm11 << 1.
        // 0x7FE = 2046. Signed 11-bit: -2.
        assert_eq!(
            decode_thumb_16(0xE7FE),
            Instruction::Branch { offset: -4 } // -2 * 2 = -4
        );

        // B (T2) not implemented in 16-bit decoder, handled in T2?
        // Wait, T2 encoding is 32-bit.
        // T1 is 16-bit.
    }

    #[test]
    fn test_decode_memory_ops() {
        // LDR R0, [R1, #4] (T1) -> 6848
        // imm5 = 1 (4>>2). rn=1. rt=0.
        // 0110 1000 0100 1000 -> 6848
        assert_eq!(
            decode_thumb_16(0x6848),
            Instruction::LdrImm {
                rt: 0,
                rn: 1,
                imm: 4
            }
        );

        // STR R2, [R3, #0] (T1) -> 601A
        // imm5=0. rn=3. rt=2.
        // 0110 0000 0001 1010 -> 601A
        assert_eq!(
            decode_thumb_16(0x601A),
            Instruction::StrImm {
                rt: 2,
                rn: 3,
                imm: 0
            }
        );
    }

    #[test]
    fn test_decode_push_pop() {
        // PUSH {R0, LR} (T1) -> B501
        // 1011 0101 0000 0001
        // m=1 (LR), regs=1 (R0)
        assert_eq!(
            decode_thumb_16(0xB501),
            Instruction::Push {
                registers: 1,
                m: true
            }
        );

        // POP {R1, PC} (T1) -> BD02
        // 1011 1101 0000 0010
        // p=1 (PC), regs=2 (R1)
        assert_eq!(
            decode_thumb_16(0xBD02),
            Instruction::Pop {
                registers: 2,
                p: true
            }
        );
    }

    #[test]
    fn test_decode_misc_rev() {
        // REV R0, R2 (using F081 -> Rd=0)
        assert_eq!(
            decode_thumb_32(0xFA92, 0xF081),
            Instruction::Rev { rd: 0, rm: 2 }
        );
    }

    #[test]
    fn test_decode_shift_reg32_lsl() {
        // LSL.W R2, R1, R2
        assert_eq!(
            decode_thumb_32(0xFA01, 0xF202),
            Instruction::ShiftReg32 {
                rd: 2,
                rn: 1,
                rm: 2,
                shift_type: 0
            }
        );
    }

    #[test]
    fn test_decode_dataproc32_eb_prefix() {
        // Pattern seen in H563 path.
        assert_eq!(
            decode_thumb_32(0xEB00, 0x1010),
            Instruction::DataProc32 {
                op: 8,
                rn: 0,
                rd: 0,
                rm: 0,
                imm5: 4,
                shift_type: 1,
                set_flags: false
            }
        );
    }

    #[test]
    fn test_decode_mov_cmp_add_sub_imm8() {
        // MOV R0, #42 -> 0x202A
        assert_eq!(
            decode_thumb_16(0x202A),
            Instruction::MovImm { rd: 0, imm: 42 }
        );
        // CMP R1, #10 -> 0x290A (0010 1001 0000 1010)
        assert_eq!(
            decode_thumb_16(0x290A),
            Instruction::CmpImm { rn: 1, imm: 10 }
        );
        // ADD R2, #5 -> 0x3205
        assert_eq!(
            decode_thumb_16(0x3205),
            Instruction::AddImm8 { rd: 2, imm: 5 }
        );
        // SUB R3, #1 -> 0x3B01
        assert_eq!(
            decode_thumb_16(0x3B01),
            Instruction::SubImm8 { rd: 3, imm: 1 }
        );
    }

    #[test]
    fn test_decode_add_sub_reg_imm3() {
        // ADD R0, R1, R2 -> 0x1888 (0001 100 0 10 001 000)
        assert_eq!(
            decode_thumb_16(0x1888),
            Instruction::AddReg {
                rd: 0,
                rn: 1,
                rm: 2
            }
        );
        // SUB R3, R4, R5 -> 0x1B63 (0001 101 1 01 100 011) ?
        // 0001 101 101 100 011 -> 0x1B63
        // Op=1 (SubReg), Rm=5, Rn=4, Rd=3
        assert_eq!(
            decode_thumb_16(0x1B63),
            Instruction::SubReg {
                rd: 3,
                rn: 4,
                rm: 5
            }
        );

        // ADD R1, R2, #7 -> 0x1DD1 (0001 110 111 010 001)
        assert_eq!(
            decode_thumb_16(0x1DD1),
            Instruction::AddImm3 {
                rd: 1,
                rn: 2,
                imm: 7
            }
        );
        // SUB R0, R0, #1 -> 0x1E40 (0001 111 001 000 000)
        assert_eq!(
            decode_thumb_16(0x1E40),
            Instruction::SubImm3 {
                rd: 0,
                rn: 0,
                imm: 1
            }
        );
    }

    #[test]
    fn test_decode_ldr_str() {
        // STR R0, [R1, #4] -> 0x6048
        // 0110 0 00001 001 000
        // L=0, imm5=1 (so imm=4), Rn=1, Rt=0
        assert_eq!(
            decode_thumb_16(0x6048),
            Instruction::StrImm {
                rt: 0,
                rn: 1,
                imm: 4
            }
        );

        // LDR R2, [R3, #0] -> 0x681A
        // 0110 1 00000 011 010
        // L=1, imm5=0, Rn=3, Rt=2
        assert_eq!(
            decode_thumb_16(0x681A),
            Instruction::LdrImm {
                rt: 2,
                rn: 3,
                imm: 0
            }
        );
    }

    #[test]
    fn test_decode_alu() {
        // AND R0, R1 -> 0x4008 (0100 00 0000 001 000)
        assert_eq!(decode_thumb_16(0x4008), Instruction::And { rd: 0, rm: 1 });
        // ORR R2, R3 -> 0x431A (0100 00 1100 011 010)
        assert_eq!(decode_thumb_16(0x431A), Instruction::Orr { rd: 2, rm: 3 });
        // EOR R4, R5 -> 0x406C (0100 00 0001 101 100)
        assert_eq!(decode_thumb_16(0x406C), Instruction::Eor { rd: 4, rm: 5 });
        // BIC R1, R2 -> 0x4391 (0100 00 1110 010 001)
        assert_eq!(decode_thumb_16(0x4391), Instruction::Bic { rd: 1, rm: 2 });
        // MVN R6, R7 -> 0x43FE (0100 00 1111 111 110)
        assert_eq!(decode_thumb_16(0x43FE), Instruction::Mvn { rd: 6, rm: 7 });
    }

    #[test]
    fn test_decode_stack_control() {
        // PUSH {R0, LR} -> 0xB501 (1011 0101 0000 0001)
        // M=1, Regs=0x01
        assert_eq!(
            decode_thumb_16(0xB501),
            Instruction::Push {
                registers: 1,
                m: true
            }
        );

        // POP {R1, PC} -> 0xBD02 (1011 1101 0000 0010)
        // P=1, Regs=0x02
        assert_eq!(
            decode_thumb_16(0xBD02),
            Instruction::Pop {
                registers: 2,
                p: true
            }
        );

        // BX R14 -> 0x4770 (0100 0111 0111 0000)
        // Rm=14 (LR)
        assert_eq!(decode_thumb_16(0x4770), Instruction::Bx { rm: 14 });
    }

    #[test]
    fn test_decode_sp_rel() {
        // STR R0, [SP, #0] -> 0x9000 (1001 0 000 00000000)
        assert_eq!(
            decode_thumb_16(0x9000),
            Instruction::StrSp { rt: 0, imm: 0 }
        );

        // LDR R1, [SP, #4] -> 0x9901 (1001 1 001 00000001)
        // imm8=1, scaled*4 = 4.
        assert_eq!(
            decode_thumb_16(0x9901),
            Instruction::LdrSp { rt: 1, imm: 4 }
        );
    }

    #[test]
    fn test_decode_cond_branch() {
        // BNE +4 (Target PC+4+4)
        // Encoding: 1101 0001 0000 0001 -> 0xD101
        // Cond=1 (NE), imm8=1. Offset = 1<<1 = 2.
        assert_eq!(
            decode_thumb_16(0xD101),
            Instruction::BranchCond { cond: 1, offset: 2 }
        );

        // BEQ -4 (0xFD) -> 0xD0FD
        // Cond=0 (EQ), imm8=FD (-3). Offset = -3<<1 = -6.
        assert_eq!(
            decode_thumb_16(0xD0FD),
            Instruction::BranchCond {
                cond: 0,
                offset: -6
            }
        );
    }

    #[test]
    fn test_decode_nop() {
        assert_eq!(decode_thumb_16(0xBF00), Instruction::Nop);
    }

    #[test]
    fn test_decode_shifts() {
        // LSLS R0, R1, #2 -> 0x0088 (000 00 00010 001 000)
        assert_eq!(
            decode_thumb_16(0x0088),
            Instruction::Lsl {
                rd: 0,
                rm: 1,
                imm: 2
            }
        );
        // LSRS R2, R3, #4 -> 0x091A (000 01 00100 011 010)
        assert_eq!(
            decode_thumb_16(0x091A),
            Instruction::Lsr {
                rd: 2,
                rm: 3,
                imm: 4
            }
        );
        // ASRS R4, R5, #6 -> 0x11AC (000 10 00110 101 100)
        assert_eq!(
            decode_thumb_16(0x11AC),
            Instruction::Asr {
                rd: 4,
                rm: 5,
                imm: 6
            }
        );

        // LSLS R0, R0, #0 (Opcode 0x0000)
        assert_eq!(
            decode_thumb_16(0x0000),
            Instruction::Lsl {
                rd: 0,
                rm: 0,
                imm: 0
            }
        );
    }

    #[test]
    fn test_decode_cmp_reg() {
        // CMP R1, R0 -> 0x4281 (0100 0010 10 000 001)
        assert_eq!(
            decode_thumb_16(0x4281),
            Instruction::CmpReg { rn: 1, rm: 0 }
        );
    }

    #[test]
    fn test_decode_mov_reg() {
        // MOV R7, SP -> 0x466F (0100 0110 0110 1111)
        // Rd=7, Rm=13 (SP)
        assert_eq!(
            decode_thumb_16(0x466F),
            Instruction::MovReg { rd: 7, rm: 13 }
        );
    }

    #[test]
    fn test_decode_ldrb_strb_imm() {
        // STRB R1, [R0, #0] -> 0x7001 (0111 0 00000 000 001)
        assert_eq!(
            decode_thumb_16(0x7001),
            Instruction::StrbImm {
                rt: 1,
                rn: 0,
                imm: 0
            }
        );
        // LDRB R1, [R0, #0] -> 0x7801 (0111 1 00000 000 001)
        assert_eq!(
            decode_thumb_16(0x7801),
            Instruction::LdrbImm {
                rt: 1,
                rn: 0,
                imm: 0
            }
        );
    }
}
