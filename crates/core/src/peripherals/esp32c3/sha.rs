// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 SHA accelerator (`0x6003_B000`) — SHA-256 block mode.
//!
//! The 2nd-stage bootloader verifies the app image with this hardware, so an
//! unmodelled accelerator returns a zero digest and the image is rejected
//! ("Image hash failed - image is corrupt"). We implement the real SHA-256
//! compression so the computed digest matches the image's appended hash.
//!
//! Register map (offsets from base; soc/esp32c3 hwcrypto_reg.h + hal/sha_ll.h):
//!   0x00 MODE      — algorithm select (SHA-256 = 2; we compute SHA-256 only)
//!   0x10 START     — write 1: H = SHA-256 IV, then compress the TEXT block
//!   0x14 CONTINUE  — write 1: compress the TEXT block into the current H
//!   0x18 BUSY      — read: 0 (the op completes atomically in sim)
//!   0x40 H[0..7]   — digest/state, 8 words (readable; writable to restore state)
//!   0x80 TEXT[0..15] — the 512-bit input block, 16 words
//!
//! Endianness: the CPU writes the message little-endian into TEXT words, but
//! SHA-256 consumes big-endian message words, so each schedule word is
//! `bswap(TEXT[i])`. Likewise the digest the firmware reads back from H is the
//! big-endian hash, so reads return `bswap(H[i])` (and writes store `bswap`).

use crate::{Peripheral, SimResult};

const MODE: u64 = 0x00;
const START: u64 = 0x10;
const CONTINUE: u64 = 0x14;
const BUSY: u64 = 0x18;
const H_BASE: u64 = 0x40;
const TEXT_BASE: u64 = 0x80;

const SHA256_IV: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[derive(Debug)]
pub struct Esp32c3Sha {
    /// Internal hash state H[0..7] in standard (host) form.
    h: [u32; 8],
    /// The 512-bit input block (TEXT_BASE), as written by the CPU.
    text: [u32; 16],
    mode: u32,
}

impl Default for Esp32c3Sha {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32c3Sha {
    pub fn new() -> Self {
        Self {
            h: SHA256_IV,
            text: [0u32; 16],
            mode: 0,
        }
    }

    /// One SHA-256 compression of the current TEXT block into `self.h`.
    fn compress(&mut self) {
        let mut w = [0u32; 64];
        // Schedule words are the big-endian message words = bswap(TEXT[i]).
        for (wi, ti) in w.iter_mut().zip(self.text.iter()) {
            *wi = ti.swap_bytes();
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = self.h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ ((!v[4]) & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v[7] = v[6];
            v[6] = v[5];
            v[5] = v[4];
            v[4] = v[3].wrapping_add(t1);
            v[3] = v[2];
            v[2] = v[1];
            v[1] = v[0];
            v[0] = t1.wrapping_add(t2);
        }
        for (hi, vi) in self.h.iter_mut().zip(v.iter()) {
            *hi = hi.wrapping_add(*vi);
        }
    }
}

impl Peripheral for Esp32c3Sha {
    // Inert walk: SHA-256 compression runs atomically at the START/CONTINUE write (BUSY reads 0); tick() is the trait-default no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let w = self.read_u32(offset & !3)?;
        Ok((w >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let sh = (offset & 3) * 8;
        let cur = self.read_u32(aligned)?;
        self.write_u32(aligned, (cur & !(0xFFu32 << sh)) | ((value as u32) << sh))
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            MODE => self.mode,
            BUSY => 0, // op completes atomically
            o if (H_BASE..H_BASE + 32).contains(&o) => {
                // Firmware reads the big-endian digest from H.
                self.h[((o - H_BASE) / 4) as usize].swap_bytes()
            }
            o if (TEXT_BASE..TEXT_BASE + 64).contains(&o) => {
                self.text[((o - TEXT_BASE) / 4) as usize]
            }
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            MODE => self.mode = value,
            START => {
                self.h = SHA256_IV;
                self.compress();
            }
            CONTINUE => self.compress(),
            o if (H_BASE..H_BASE + 32).contains(&o) => {
                // Restore saved state (matches the bswap on read).
                self.h[((o - H_BASE) / 4) as usize] = value.swap_bytes();
            }
            o if (TEXT_BASE..TEXT_BASE + 64).contains(&o) => {
                self.text[((o - TEXT_BASE) / 4) as usize] = value;
            }
            _ => {}
        }
        Ok(())
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // SHA-256("abc") = ba7816bf 8f01cfea 414140de 5dae2223 b00361a3 96177a9c b410ff61 f20015ad
    #[test]
    fn sha256_abc_matches_known_vector() {
        let mut sha = Esp32c3Sha::new();
        // Padded single block for "abc": 'a''b''c' 0x80, then zeros, len=24 bits at end.
        let mut block = [0u8; 64];
        block[0..3].copy_from_slice(b"abc");
        block[3] = 0x80;
        block[63] = 24; // message length in bits, big-endian (fits one byte)
                        // Write the block as the CPU would (little-endian words) to TEXT.
        for i in 0..16 {
            let w = u32::from_le_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
            sha.write_u32(TEXT_BASE + (i as u64) * 4, w).unwrap();
        }
        sha.write_u32(MODE, 2).unwrap();
        sha.write_u32(START, 1).unwrap();
        // Read the digest back the way the firmware does.
        let mut digest = [0u8; 32];
        for i in 0..8 {
            let w = sha.read_u32(H_BASE + (i as u64) * 4).unwrap();
            digest[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        let expected = [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad,
        ];
        assert_eq!(digest, expected);
    }
}
