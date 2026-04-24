// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Integration tests for the Xtensa instruction length predecoder (Task B1).
//!
//! Verifies that `instruction_length` classifies every possible byte0 value
//! correctly against the authoritative narrow op0 set {0x8, 0x9, 0xA, 0xD}
//! from the Xtensa ISA RM §3.3 (RRRN narrow encoding format).

use labwired_core::decoder::xtensa_length::instruction_length;

fn is_narrow(b0: u8) -> bool {
    matches!(b0 & 0x0F, 0x08 | 0x09 | 0x0A | 0x0D)
}

#[test]
fn every_possible_byte0_classifies_coherently() {
    for b0 in 0u8..=0xFF {
        let expected = if is_narrow(b0) { 2 } else { 3 };
        assert_eq!(
            instruction_length(b0),
            expected,
            "classification mismatch for byte0 = 0x{:02X}", b0
        );
    }
}

#[test]
fn known_wide_opcodes_are_three_bytes() {
    // Wide op0 values (selection from ISA RM):
    assert_eq!(instruction_length(0x00), 3); // ADD (QRST op0=0x0)
    assert_eq!(instruction_length(0x01), 3); // L32R (op0=0x1)
    assert_eq!(instruction_length(0x02), 3); // LSAI loads/stores (op0=0x2)
    assert_eq!(instruction_length(0x05), 3); // CALLN (op0=0x5)
    assert_eq!(instruction_length(0x06), 3); // SI / J (op0=0x6)
    assert_eq!(instruction_length(0x07), 3); // BR branches (op0=0x7)
}

#[test]
fn known_narrow_opcodes_are_two_bytes() {
    // Narrow op0 values per the RRRN format:
    assert_eq!(instruction_length(0x08), 2); // L32I.N
    assert_eq!(instruction_length(0x09), 2); // S32I.N
    assert_eq!(instruction_length(0x0A), 2); // ADD.N / ADDI.N
    assert_eq!(instruction_length(0x0D), 2); // MOV.N / MOVI.N / zero-op narrows
}

#[test]
fn high_nibble_of_byte0_is_ignored() {
    // Length classification uses ONLY the low 4 bits (op0).
    // High nibble carries other fields (e.g., in the length predecode phase,
    // no other bits should affect the classification).
    for high in 0u8..=0xF {
        let b0 = (high << 4) | 0x8; // always narrow op0
        assert_eq!(instruction_length(b0), 2, "narrow should ignore high nibble, b0=0x{:02X}", b0);
        let b0 = (high << 4) | 0x2; // always wide op0
        assert_eq!(instruction_length(b0), 3, "wide should ignore high nibble, b0=0x{:02X}", b0);
    }
}
