// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa instruction length predecoder.
//!
//! Narrow (Code Density) instructions are 2 bytes; all others are 3 bytes.
//! Classification is by `op0` (bits [3:0] of byte 0) per Xtensa ISA RM §3.3.
//!
//! Narrow iff op0 ∈ {0x8, 0x9, 0xA, 0xB, 0xC, 0xD}.
//!
//! HW-oracle verified (xtensa-esp32s3-elf-as + xtensa-esp32s3-elf-objdump):
//!   op0=0x8 → L32I.N (2 bytes)
//!   op0=0x9 → S32I.N (2 bytes)
//!   op0=0xA → ADD.N  (2 bytes)
//!   op0=0xB → ADDI.N (2 bytes)
//!   op0=0xC → MOVI.N / BEQZ.N / BNEZ.N (2 bytes)
//!   op0=0xD → MOV.N / NOP.N / RET.N / RETW.N / BREAK.N / ILL.N (2 bytes)

#[inline]
pub fn instruction_length(byte0: u8) -> u32 {
    match byte0 & 0x0F {
        0x8..=0xD => 2,
        _ => 3,
    }
}
