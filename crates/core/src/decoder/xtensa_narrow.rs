// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa Code Density (16-bit) decoder.
//!
//! Expands narrow encodings into the same `Instruction` enum from
//! `super::xtensa::Instruction` where semantics are identical, or uses a
//! narrow-only variant where they diverge.

use super::xtensa::Instruction;

/// Decode a 16-bit narrow instruction. Caller must have confirmed narrowness
/// via `super::xtensa_length::instruction_length(byte0) == 2`.
pub fn decode_narrow(halfword: u16) -> Instruction {
    let op0 = (halfword & 0x0F) as u8;
    match op0 {
        0x8 => Instruction::Unknown(halfword as u32), // L32I.N — filled in Task D8
        0x9 => Instruction::Unknown(halfword as u32), // S32I.N — filled in Task D8
        0xA => Instruction::Unknown(halfword as u32), // ADD.N / ADDI.N — Task D8
        0xD => Instruction::Unknown(halfword as u32), // MOV.N / MOVI.N / etc — Task D8
        _ => Instruction::Unknown(halfword as u32),
    }
}
