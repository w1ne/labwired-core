// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa instruction length predecoder.
//!
//! Narrow (Code Density) instructions are 2 bytes; all others are 3 bytes.
//! Classification is by `op0` (bits [3:0] of byte 0) per Xtensa ISA RM §3.3.
//!
//! Narrow iff op0 ∈ {0x8, 0x9, 0xA, 0xD}.

#[inline]
pub fn instruction_length(byte0: u8) -> u32 {
    match byte0 & 0x0F {
        0x8 | 0x9 | 0xA | 0xD => 2,
        _ => 3,
    }
}
