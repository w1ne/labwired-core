// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 SHA accelerator (`DR_REG_SHA_BASE`, `0x6003_B000`).
//!
//! The boot ROM (`ets_sha_*`) and 2nd-stage bootloader hash the app image with
//! this block to verify the SHA-256 appended to every ESP-IDF image. Without a
//! real implementation the digest reads back as 0xFF…FF and every image is
//! rejected ("Image hash failed - image is corrupt").
//!
//! Protocol (per `hal/esp32s3/sha_ll.h`):
//!   * `SHA_MODE_REG` (+0x00) selects the algorithm (SHA-256 = 2).
//!   * The 512-bit message block is written to the M / TEXT registers
//!     (`SHA_TEXT_BASE` = +0x80, 16 words).
//!   * `SHA_START_REG` (+0x10) ← 1 processes the *first* block (hash state
//!     initialised to the SHA-256 IV).
//!   * `SHA_CONTINUE_REG` (+0x14) ← 1 processes a *subsequent* block, chaining
//!     from the current hash state.
//!   * `SHA_BUSY_REG` (+0x18) reads 0 once done (instant here — no latency).
//!   * The digest is read from the H registers (`SHA_H_BASE` = +0x40, 8 words).
//!
//! The firmware does the message padding itself and feeds whole 64-byte
//! blocks; the accelerator only runs the per-block compression — which is
//! exactly `sha2::compress256`.
//!
//! ## Endianness
//!
//! The firmware writes message words and reads digest words with plain 32-bit
//! loads/stores, then treats the H-register bytes as the digest byte stream.
//! So `H_reg[i]` must hold `h_i` byte-swapped: a little-endian store of
//! `h_i.swap_bytes()` reproduces the big-endian SHA-256 digest bytes. Verified
//! against the FIPS-180 "abc" test vector below.

use crate::{Peripheral, SimResult};
use sha2::compress256;
use sha2::digest::generic_array::GenericArray;
use std::collections::HashMap;

const MODE: u64 = 0x00;
const START: u64 = 0x10;
const CONTINUE: u64 = 0x14;
const BUSY: u64 = 0x18;
const H_BASE: u64 = 0x40; // digest, 8 words (0x40..0x5C)
const TEXT_BASE: u64 = 0x80; // message block, 16 words (0x80..0xBC)

/// SHA-256 initial hash values (FIPS 180-4 §5.3.3).
const SHA256_IV: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

#[derive(Debug, Default)]
pub struct Esp32s3Sha {
    regs: HashMap<u64, u32>,
}

impl Esp32s3Sha {
    pub fn new() -> Self {
        Self::default()
    }

    fn reg(&self, off: u64) -> u32 {
        self.regs.get(&off).copied().unwrap_or(0)
    }

    /// Assemble the 64-byte message block from the 16 M/TEXT registers. Each
    /// register holds a message word as the firmware stored it (native
    /// little-endian); `compress256` reads the block as big-endian words, so
    /// laying the words down little-endian yields the correct schedule.
    fn message_block(&self) -> [u8; 64] {
        let mut block = [0u8; 64];
        for i in 0..16 {
            let w = self.reg(TEXT_BASE + (i as u64) * 4);
            block[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        block
    }

    /// Read the current hash state back out of the H registers, undoing the
    /// store-side byte swap.
    fn load_state(&self) -> [u32; 8] {
        let mut s = [0u32; 8];
        for (i, h) in s.iter_mut().enumerate() {
            *h = self.reg(H_BASE + (i as u64) * 4).swap_bytes();
        }
        s
    }

    /// Store the hash state into the H registers (byte-swapped so a plain
    /// little-endian read reconstructs the big-endian digest bytes).
    fn store_state(&mut self, state: [u32; 8]) {
        for (i, h) in state.iter().enumerate() {
            self.regs.insert(H_BASE + (i as u64) * 4, h.swap_bytes());
        }
    }

    /// Run one block of SHA-256 compression. `first` selects the IV vs. the
    /// chained state already in the H registers.
    fn run_block(&mut self, first: bool) {
        let mut state = if first { SHA256_IV } else { self.load_state() };
        let block = self.message_block();
        let ga = GenericArray::clone_from_slice(&block);
        compress256(&mut state, &[ga]);
        self.store_state(state);
    }
}

impl Peripheral for Esp32s3Sha {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !3)?;
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let mut w = self.reg(word_off);
        w = (w & !(0xFFu32 << byte_off)) | ((value as u32) << byte_off);
        self.write_u32(word_off, w)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        // The accelerator completes instantly; never report busy.
        if offset & !3 == BUSY {
            return Ok(0);
        }
        Ok(self.reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let off = offset & !3;
        self.regs.insert(off, value);
        match off {
            START if value != 0 => self.run_block(true),
            CONTINUE if value != 0 => self.run_block(false),
            _ => {}
        }
        // START/CONTINUE auto-clear (the firmware polls BUSY, not these).
        if off == START || off == CONTINUE {
            self.regs.insert(off, 0);
        }
        let _ = MODE; // SHA-256 only; mode is accepted/round-tripped.
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive the accelerator exactly as firmware would for the single-block
    /// FIPS-180 "abc" vector and check the digest bytes the firmware reads.
    #[test]
    fn abc_single_block_matches_fips_vector() {
        let mut sha = Esp32s3Sha::new();
        // Padded "abc": 0x61626380, then zeros, length 24 bits (0x18) at the end.
        let mut block = [0u8; 64];
        block[0..3].copy_from_slice(b"abc");
        block[3] = 0x80;
        block[63] = 0x18; // bit length = 24
                          // Firmware writes the block as 16 little-endian words to TEXT_BASE.
        for i in 0..16 {
            let w = u32::from_le_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
            sha.write_u32(TEXT_BASE + (i as u64) * 4, w).unwrap();
        }
        sha.write_u32(START, 1).unwrap();
        assert_eq!(sha.read_u32(BUSY).unwrap(), 0, "must not be busy");
        // Read the digest the way firmware does: H words → byte stream (LE).
        let mut digest = [0u8; 32];
        for i in 0..8 {
            let w = sha.read_u32(H_BASE + (i as u64) * 4).unwrap();
            digest[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        let expected = hex_literal_ba7816bf();
        assert_eq!(digest, expected, "SHA256(\"abc\") mismatch");
    }

    /// SHA256("abc") = ba7816bf 8f01cfea 414140de 5dae2223 b00361a3 96177a9c
    ///                 b410ff61 f20015ad
    fn hex_literal_ba7816bf() -> [u8; 32] {
        [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad,
        ]
    }

    #[test]
    fn busy_reads_zero() {
        let sha = Esp32s3Sha::new();
        assert_eq!(sha.read_u32(BUSY).unwrap(), 0);
    }
}
