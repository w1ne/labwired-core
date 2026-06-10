// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 ECB peripheral — AES-128 ECB block coprocessor.
//!
//! Source: nRF52840 PS rev 1.7 §6.5 (ECB).
//!
//! Register surface:
//!   TASKS_STARTECB  0x000   write 1 → arms the encryption engine
//!   TASKS_STOPECB   0x004   write 1 → abort (not typically used)
//!   EVENTS_ENDECB   0x100   set to 1 when encryption completes
//!   EVENTS_ERRORECB 0x104   set to 1 on error
//!   INTENSET        0x304   interrupt enable set
//!   INTENCLR        0x308   interrupt enable clear
//!   ECBDATAPTR      0x504   pointer to { key[16], cleartext[16], ciphertext[16] }
//!
//! EasyDMA operation: on TASKS_STARTECB the peripheral reads the 16-byte key
//! (bytes 0..16) and 16-byte cleartext (bytes 16..32) from the RAM struct
//! pointed to by ECBDATAPTR, computes AES-128-ECB encrypt, then writes the
//! 16-byte ciphertext back to ECBDATAPTR+32 and sets EVENTS_ENDECB=1.

use crate::{Bus, Peripheral, PeripheralTickResult, SimResult};

const OFF_TASKS_STARTECB: u64 = 0x000;
const OFF_TASKS_STOPECB: u64 = 0x004;
const OFF_EVENTS_ENDECB: u64 = 0x100;
const OFF_EVENTS_ERRORECB: u64 = 0x104;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_ECBDATAPTR: u64 = 0x504;

const INTEN_ENDECB: u32 = 1 << 0;

// ── AES-128 engine ────────────────────────────────────────────────────────────

/// AES S-box (forward substitution table).
#[rustfmt::skip]
const SBOX: [u8; 256] = [
    0x63,0x7c,0x77,0x7b,0xf2,0x6b,0x6f,0xc5,0x30,0x01,0x67,0x2b,0xfe,0xd7,0xab,0x76,
    0xca,0x82,0xc9,0x7d,0xfa,0x59,0x47,0xf0,0xad,0xd4,0xa2,0xaf,0x9c,0xa4,0x72,0xc0,
    0xb7,0xfd,0x93,0x26,0x36,0x3f,0xf7,0xcc,0x34,0xa5,0xe5,0xf1,0x71,0xd8,0x31,0x15,
    0x04,0xc7,0x23,0xc3,0x18,0x96,0x05,0x9a,0x07,0x12,0x80,0xe2,0xeb,0x27,0xb2,0x75,
    0x09,0x83,0x2c,0x1a,0x1b,0x6e,0x5a,0xa0,0x52,0x3b,0xd6,0xb3,0x29,0xe3,0x2f,0x84,
    0x53,0xd1,0x00,0xed,0x20,0xfc,0xb1,0x5b,0x6a,0xcb,0xbe,0x39,0x4a,0x4c,0x58,0xcf,
    0xd0,0xef,0xaa,0xfb,0x43,0x4d,0x33,0x85,0x45,0xf9,0x02,0x7f,0x50,0x3c,0x9f,0xa8,
    0x51,0xa3,0x40,0x8f,0x92,0x9d,0x38,0xf5,0xbc,0xb6,0xda,0x21,0x10,0xff,0xf3,0xd2,
    0xcd,0x0c,0x13,0xec,0x5f,0x97,0x44,0x17,0xc4,0xa7,0x7e,0x3d,0x64,0x5d,0x19,0x73,
    0x60,0x81,0x4f,0xdc,0x22,0x2a,0x90,0x88,0x46,0xee,0xb8,0x14,0xde,0x5e,0x0b,0xdb,
    0xe0,0x32,0x3a,0x0a,0x49,0x06,0x24,0x5c,0xc2,0xd3,0xac,0x62,0x91,0x95,0xe4,0x79,
    0xe7,0xc8,0x37,0x6d,0x8d,0xd5,0x4e,0xa9,0x6c,0x56,0xf4,0xea,0x65,0x7a,0xae,0x08,
    0xba,0x78,0x25,0x2e,0x1c,0xa6,0xb4,0xc6,0xe8,0xdd,0x74,0x1f,0x4b,0xbd,0x8b,0x8a,
    0x70,0x3e,0xb5,0x66,0x48,0x03,0xf6,0x0e,0x61,0x35,0x57,0xb9,0x86,0xc1,0x1d,0x9e,
    0xe1,0xf8,0x98,0x11,0x69,0xd9,0x8e,0x94,0x9b,0x1e,0x87,0xe9,0xce,0x55,0x28,0xdf,
    0x8c,0xa1,0x89,0x0d,0xbf,0xe6,0x42,0x68,0x41,0x99,0x2d,0x0f,0xb0,0x54,0xbb,0x16,
];

/// AES round constants (rcon[1..11] for key expansion).
#[rustfmt::skip]
const RCON: [u8; 11] = [0x00,0x01,0x02,0x04,0x08,0x10,0x20,0x40,0x80,0x1b,0x36];

/// GF(2^8) multiply by 2 (xtime).
#[inline(always)]
fn xtime(a: u8) -> u8 {
    (a << 1) ^ if a & 0x80 != 0 { 0x1b } else { 0 }
}

/// GF(2^8) multiply: a * b using Russian peasant.
#[inline(always)]
fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut p: u8 = 0;
    while b != 0 {
        if b & 1 != 0 {
            p ^= a;
        }
        a = xtime(a);
        b >>= 1;
    }
    p
}

/// AES-128 key expansion: key[16] → round_keys[11][16].
fn key_expand(key: &[u8; 16]) -> [[u8; 16]; 11] {
    let mut w = [[0u8; 4]; 44];

    // Copy the original key into w[0..4].
    for i in 0..4 {
        w[i] = [key[4 * i], key[4 * i + 1], key[4 * i + 2], key[4 * i + 3]];
    }

    for i in 4..44 {
        let mut temp = w[i - 1];
        if i % 4 == 0 {
            // RotWord
            temp = [temp[1], temp[2], temp[3], temp[0]];
            // SubWord
            temp = [
                SBOX[temp[0] as usize],
                SBOX[temp[1] as usize],
                SBOX[temp[2] as usize],
                SBOX[temp[3] as usize],
            ];
            // XOR with Rcon
            temp[0] ^= RCON[i / 4];
        }
        w[i] = [
            w[i - 4][0] ^ temp[0],
            w[i - 4][1] ^ temp[1],
            w[i - 4][2] ^ temp[2],
            w[i - 4][3] ^ temp[3],
        ];
    }

    // Pack into 11 × 16-byte round keys.
    let mut rk = [[0u8; 16]; 11];
    for r in 0..11 {
        for c in 0..4 {
            rk[r][4 * c..4 * c + 4].copy_from_slice(&w[r * 4 + c]);
        }
    }
    rk
}

/// AES-128 block encrypt: plaintext[16] + key[16] → ciphertext[16].
///
/// Follows FIPS 197 §5.1 column-major state ordering.
pub fn aes128_encrypt(plaintext: &[u8; 16], key: &[u8; 16]) -> [u8; 16] {
    let rk = key_expand(key);

    // State: column-major 4×4.  state[col][row].
    let mut state = [[0u8; 4]; 4];
    for col in 0..4 {
        for row in 0..4 {
            state[col][row] = plaintext[col * 4 + row];
        }
    }

    // Initial AddRoundKey (round 0).
    add_round_key(&mut state, &rk[0]);

    // Rounds 1..9 with MixColumns.
    for rk_round in &rk[1..10] {
        sub_bytes(&mut state);
        shift_rows(&mut state);
        mix_columns(&mut state);
        add_round_key(&mut state, rk_round);
    }

    // Round 10: no MixColumns.
    sub_bytes(&mut state);
    shift_rows(&mut state);
    add_round_key(&mut state, &rk[10]);

    // Unpack state back to byte array.
    let mut out = [0u8; 16];
    for col in 0..4 {
        for row in 0..4 {
            out[col * 4 + row] = state[col][row];
        }
    }
    out
}

fn add_round_key(state: &mut [[u8; 4]; 4], rk: &[u8; 16]) {
    for col in 0..4 {
        for row in 0..4 {
            state[col][row] ^= rk[col * 4 + row];
        }
    }
}

fn sub_bytes(state: &mut [[u8; 4]; 4]) {
    for col in 0..4 {
        for row in 0..4 {
            state[col][row] = SBOX[state[col][row] as usize];
        }
    }
}

fn shift_rows(state: &mut [[u8; 4]; 4]) {
    // Row 0: no shift.
    // Row 1: left shift by 1.
    let r1 = [state[0][1], state[1][1], state[2][1], state[3][1]];
    state[0][1] = r1[1];
    state[1][1] = r1[2];
    state[2][1] = r1[3];
    state[3][1] = r1[0];
    // Row 2: left shift by 2.
    let r2 = [state[0][2], state[1][2], state[2][2], state[3][2]];
    state[0][2] = r2[2];
    state[1][2] = r2[3];
    state[2][2] = r2[0];
    state[3][2] = r2[1];
    // Row 3: left shift by 3.
    let r3 = [state[0][3], state[1][3], state[2][3], state[3][3]];
    state[0][3] = r3[3];
    state[1][3] = r3[0];
    state[2][3] = r3[1];
    state[3][3] = r3[2];
}

fn mix_columns(state: &mut [[u8; 4]; 4]) {
    for col in state.iter_mut() {
        let s0 = col[0];
        let s1 = col[1];
        let s2 = col[2];
        let s3 = col[3];
        col[0] = gmul(0x02, s0) ^ gmul(0x03, s1) ^ s2 ^ s3;
        col[1] = s0 ^ gmul(0x02, s1) ^ gmul(0x03, s2) ^ s3;
        col[2] = s0 ^ s1 ^ gmul(0x02, s2) ^ gmul(0x03, s3);
        col[3] = gmul(0x03, s0) ^ s1 ^ s2 ^ gmul(0x02, s3);
    }
}

// ── Peripheral struct ─────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct Nrf52Ecb {
    events_endecb: u32,
    events_errorecb: u32,
    inten: u32,
    ecbdataptr: u32,
    /// Set when TASKS_STARTECB is written; cleared after tick_with_bus runs.
    pending_start: bool,
}

impl Nrf52Ecb {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Nrf52Ecb {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_STARTECB | OFF_TASKS_STOPECB => 0,
            OFF_EVENTS_ENDECB => self.events_endecb,
            OFF_EVENTS_ERRORECB => self.events_errorecb,
            OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_ECBDATAPTR => self.ecbdataptr,
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_STARTECB if value & 1 != 0 => {
                self.pending_start = true;
            }
            OFF_TASKS_STARTECB => {}
            OFF_TASKS_STOPECB => {
                self.pending_start = false;
            }
            OFF_EVENTS_ENDECB => self.events_endecb = value & 1,
            OFF_EVENTS_ERRORECB => self.events_errorecb = value & 1,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,
            OFF_ECBDATAPTR => self.ecbdataptr = value,
            _ => {}
        }
        Ok(())
    }

    fn needs_bus_tick(&self) -> bool {
        self.pending_start
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if !self.pending_start {
            return;
        }
        self.pending_start = false;

        let base = self.ecbdataptr as u64;

        // Read 16-byte key (bytes 0..16) from ECBDATAPTR.
        let mut key = [0u8; 16];
        for (i, byte) in key.iter_mut().enumerate() {
            *byte = bus.read_u8(base + i as u64).unwrap_or(0);
        }

        // Read 16-byte cleartext (bytes 16..32) from ECBDATAPTR+16.
        let mut cleartext = [0u8; 16];
        for (i, byte) in cleartext.iter_mut().enumerate() {
            *byte = bus.read_u8(base + 16 + i as u64).unwrap_or(0);
        }

        // Compute AES-128-ECB encrypt.
        let ciphertext = aes128_encrypt(&cleartext, &key);

        // Write 16-byte ciphertext to ECBDATAPTR+32.
        for (i, byte) in ciphertext.iter().enumerate() {
            let _ = bus.write_u8(base + 32 + i as u64, *byte);
        }

        // Signal completion.
        self.events_endecb = 1;
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if self.events_endecb != 0 && self.inten & INTEN_ENDECB != 0 {
            return PeripheralTickResult {
                irq: true,
                cycles: 1,
                fired_events: vec![OFF_EVENTS_ENDECB as u32],
                ..Default::default()
            };
        }
        PeripheralTickResult::default()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// FIPS-197 Appendix B / C.1 test vector.
    /// key       = 000102030405060708090a0b0c0d0e0f
    /// plaintext = 00112233445566778899aabbccddeeff
    /// expected  = 69c4e0d86a7b0430d8cdb78070b4c55a
    #[test]
    fn fips197_appendix_b() {
        let key: [u8; 16] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ];
        let plaintext: [u8; 16] = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ];
        let expected: [u8; 16] = [
            0x69, 0xc4, 0xe0, 0xd8, 0x6a, 0x7b, 0x04, 0x30, 0xd8, 0xcd, 0xb7, 0x80, 0x70, 0xb4,
            0xc5, 0x5a,
        ];
        let ct = aes128_encrypt(&plaintext, &key);
        assert_eq!(ct, expected, "AES-128 FIPS-197 vector mismatch");

        // Also verify the LE u32 word that the conformance firmware reads.
        let word0 = u32::from_le_bytes([ct[0], ct[1], ct[2], ct[3]]);
        assert_eq!(word0, 0xD8E0_C469, "ecb_ct0 word mismatch");
    }

    #[test]
    fn ecbdataptr_round_trips() {
        let mut e = Nrf52Ecb::new();
        e.write_u32(OFF_ECBDATAPTR, 0x2000_2000).unwrap();
        assert_eq!(e.read_u32(OFF_ECBDATAPTR).unwrap(), 0x2000_2000);
    }

    #[test]
    fn needs_bus_tick_after_start() {
        let mut e = Nrf52Ecb::new();
        assert!(!e.needs_bus_tick());
        e.write_u32(OFF_TASKS_STARTECB, 1).unwrap();
        assert!(e.needs_bus_tick());
    }
}
