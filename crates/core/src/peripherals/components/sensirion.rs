// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Shared framing helpers for Sensirion I²C sensors (SCD4x, SGP4x, SPS30).
//!
//! Every Sensirion air-quality part speaks the same wire protocol:
//! - The master writes a **16-bit big-endian command** (e.g. `0xEC05`
//!   `read_measurement`). Some commands carry parameters: each parameter is a
//!   16-bit big-endian word followed by its CRC byte.
//! - Commands that return data are followed (after a wait, or a repeated
//!   START) by a read of N **16-bit words, each followed by a CRC-8 byte**.
//! - CRC-8 uses polynomial `0x31`, init `0xFF`, no final XOR — identical to the
//!   AHT20 part already modelled here, and to the checksum the real Sensirion
//!   embedded drivers compute. Getting this byte-exact is what lets an
//!   unmodified vendor driver decode our model.
//!
//! This module deliberately exposes only the primitives (`crc8`,
//! `encode_words`, `decode_word`). Each device owns its own little command
//! state machine — the command sets differ enough that a shared engine would
//! leak. Keeping the shared surface tiny keeps the device models independent.

/// CRC-8 with polynomial `0x31`, init `0xFF`, no final XOR (Sensirion spec).
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0xFF;
    for &byte in data {
        crc ^= byte;
        for _ in 0..8 {
            if (crc & 0x80) != 0 {
                crc = (crc << 1) ^ 0x31;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

/// Encode measurement words into a Sensirion read buffer: each `u16` becomes
/// `[hi, lo, crc(hi,lo)]`, big-endian, exactly as a real part clocks them out.
pub fn encode_words(words: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(words.len() * 3);
    for &w in words {
        let hi = (w >> 8) as u8;
        let lo = (w & 0xFF) as u8;
        out.push(hi);
        out.push(lo);
        out.push(crc8(&[hi, lo]));
    }
    out
}

/// Decode a parameter word the master wrote as `[hi, lo, crc]`. Returns the
/// 16-bit value ignoring the CRC byte (the model trusts the master's framing).
pub fn decode_word(hi: u8, lo: u8) -> u16 {
    ((hi as u16) << 8) | (lo as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc8_matches_sensirion_reference_vector() {
        // Datasheet reference: CRC of {0xBE, 0xEF} is 0x92 for poly 0x31/init 0xFF.
        assert_eq!(crc8(&[0xBE, 0xEF]), 0x92);
    }

    #[test]
    fn encode_words_appends_valid_crc_per_word() {
        let buf = encode_words(&[0xBEEF]);
        assert_eq!(buf, vec![0xBE, 0xEF, 0x92]);
        // Every 3rd byte must validate as the CRC of the preceding two.
        for chunk in buf.chunks(3) {
            assert_eq!(chunk[2], crc8(&chunk[..2]));
        }
    }

    #[test]
    fn decode_word_is_big_endian() {
        assert_eq!(decode_word(0x01, 0x02), 0x0102);
    }
}
