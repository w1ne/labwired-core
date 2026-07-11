// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SHA hardware accelerator for ESP32-classic.
//!
//! Per ESP32 TRM v4.6 §24 ("SHA Accelerator"). The block sits at base
//! `0x3FF0_3000` (`DR_REG_SHA_BASE`), inside the DPORT crypto region, and
//! implements the FIPS-180-4 SHA family in fixed-512/1024-bit block mode.
//! Register layout (matches ESP-IDF `soc/esp32/include/soc/hwcrypto_reg.h`):
//!
//!   * `SHA_TEXT_BASE`  (offset 0x00, 64 bytes / 16 words) — message block
//!     **and** digest scratch. Firmware writes one big-endian message block
//!     here, then reads the digest back from the same memory after a `LOAD`.
//!   * `SHA_{N}_START_REG`    (offset 0x80 + type*0x10) — write 1: hash the
//!     block in TEXT memory starting from the algorithm's standard initial
//!     hash value (`H0`), then update the internal digest state.
//!   * `SHA_{N}_CONTINUE_REG` (offset 0x84 + type*0x10) — write 1: hash the
//!     block in TEXT memory continuing from the current internal digest state.
//!   * `SHA_{N}_LOAD_REG`     (offset 0x88 + type*0x10) — write 1: copy the
//!     current internal digest state into TEXT memory (big-endian) so firmware
//!     can read it out.
//!   * `SHA_{N}_BUSY_REG`     (offset 0x8C + type*0x10) — read: 1 while the
//!     engine is hashing a block, 0 once idle.
//!
//! `{N}` is one of 1 / 256 / 384 / 512; `type` is the `esp_sha_type` enum
//! index (SHA1=0, SHA2_256=1, SHA2_384=2, SHA2_512=3), so the four 0x10-byte
//! command banks tile from 0x80..0xC0 (`SHA_LL_TYPE_OFFSET = 0x10`).
//!
//! ## Fidelity
//!
//! SHA-1 and SHA-256 share the same 512-bit (64-byte) block memory and are
//! the algorithms ESP-IDF's mbedTLS HW backend and the ROM boot-image hash
//! actually exercise on ESP32-classic. Those two are modeled with the real
//! FIPS-180-4 compression functions: START/CONTINUE run a genuine block
//! compression and LOAD writes back the true big-endian digest. SHA-384 and
//! SHA-512 use a 1024-bit block split across two text banks and a different
//! word-order convention; their register interface (START/CONTINUE/LOAD/BUSY)
//! is modeled faithfully but the 64-bit compression is left as a state-
//! preserving stub (the internal state advances but is not the true digest).
//! The `as u8` bus model means BUSY clears synchronously inside the START /
//! CONTINUE write — there is no DMA, so firmware's `while (sha_busy());`
//! poll loop observes idle on its first read, exactly as a fast core would.

use crate::{Peripheral, PeripheralTickResult, SimResult};

// ── Register offsets (per ESP32 TRM v4.6 §24 / hwcrypto_reg.h) ───────────

/// `SHA_TEXT_BASE` — message-block / digest scratch memory (16 words).
pub const SHA_TEXT_OFFSET: u64 = 0x00;
/// Size of the text memory window in bytes (64 = one 512-bit block).
pub const SHA_TEXT_LEN: usize = 64;
/// `SHA_1_START_REG` — base of the per-algorithm command banks.
pub const SHA_CMD_BASE_OFFSET: u64 = 0x80;
/// Stride between successive algorithm command banks (`SHA_LL_TYPE_OFFSET`).
pub const SHA_TYPE_STRIDE: u64 = 0x10;
/// Within-bank offset of the START register.
pub const SHA_START_OFFSET: u64 = 0x00;
/// Within-bank offset of the CONTINUE register.
pub const SHA_CONTINUE_OFFSET: u64 = 0x04;
/// Within-bank offset of the LOAD register.
pub const SHA_LOAD_OFFSET: u64 = 0x08;
/// Within-bank offset of the BUSY register.
pub const SHA_BUSY_OFFSET: u64 = 0x0C;

/// `esp_sha_type` index → algorithm. Matches ESP-IDF `hal/sha_types.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaType {
    /// SHA-1 (160-bit digest, 512-bit block).
    Sha1,
    /// SHA-256 (256-bit digest, 512-bit block).
    Sha256,
    /// SHA-384 (384-bit digest, 1024-bit block).
    Sha384,
    /// SHA-512 (512-bit digest, 1024-bit block).
    Sha512,
}

impl ShaType {
    /// Decode the algorithm from a command-bank type index (0..=3).
    fn from_index(idx: u64) -> Option<Self> {
        match idx {
            0 => Some(ShaType::Sha1),
            1 => Some(ShaType::Sha256),
            2 => Some(ShaType::Sha384),
            3 => Some(ShaType::Sha512),
            _ => None,
        }
    }
}

/// SHA-1 initial hash value (FIPS-180-4 §5.3.1).
const SHA1_H0: [u32; 5] = [
    0x6745_2301,
    0xEFCD_AB89,
    0x98BA_DCFE,
    0x1032_5476,
    0xC3D2_E1F0,
];
/// SHA-256 initial hash value (FIPS-180-4 §5.3.3).
const SHA256_H0: [u32; 8] = [
    0x6A09_E667,
    0xBB67_AE85,
    0x3C6E_F372,
    0xA54F_F53A,
    0x510E_527F,
    0x9B05_688C,
    0x1F83_D9AB,
    0x5BE0_CD19,
];
/// SHA-256 round constants (FIPS-180-4 §4.2.2).
const SHA256_K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

/// SHA hardware accelerator peripheral.
///
/// Holds the 64-byte text memory plus a per-algorithm internal digest state
/// register file. Command writes drive a synchronous block compression (no
/// DMA modeled), so `BUSY` is never observably set from firmware's point of
/// view — it reads back 0 on the first poll, matching a fast core.
#[derive(Debug)]
pub struct Sha {
    /// Base MMIO address (informational; bus dispatches by offset).
    base: u32,
    /// 64-byte text memory (`SHA_TEXT_BASE`): message block in, digest out.
    /// Stored little-endian by byte index; firmware writes/reads big-endian
    /// words via its own `HAL_SWAP32`, so the bytes here are exactly the
    /// register contents.
    text: [u8; SHA_TEXT_LEN],
    /// Internal SHA-256 digest state (also reused to back SHA-1's 5 words).
    state256: [u32; 8],
    /// Internal SHA-512 digest state (64-bit lanes, stub-advanced).
    state512: [u64; 8],
    /// Per-type BUSY flag. Always cleared synchronously after a command, so
    /// reads return 0; kept as state so a snapshot mid-command round-trips.
    busy: [bool; 4],
    /// Whether each algorithm's internal state has been initialized via a
    /// prior CONTINUE (true) — informational, START always re-seeds.
    started: [bool; 4],
}

impl Default for Sha {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha {
    /// Canonical MMIO base on ESP32-classic (`DR_REG_SHA_BASE`).
    pub const BASE: u32 = 0x3FF0_3000;
    /// Documented register window size (text memory + command banks).
    pub const SIZE: u32 = 0x100;

    /// Construct a freshly-reset SHA accelerator: text memory and all
    /// internal state cleared, every algorithm idle.
    pub fn new() -> Self {
        Self {
            base: Self::BASE,
            text: [0; SHA_TEXT_LEN],
            state256: [0; 8],
            state512: [0; 8],
            busy: [false; 4],
            started: [false; 4],
        }
    }

    /// Base MMIO address (informational).
    pub fn base(&self) -> u32 {
        self.base
    }

    /// Read one 512-bit (16-word) message block out of the text memory as
    /// big-endian `u32` words — the exact bytes firmware wrote.
    fn text_block_words(&self) -> [u32; 16] {
        let mut w = [0u32; 16];
        for (i, word) in w.iter_mut().enumerate() {
            let b = i * 4;
            *word = u32::from_be_bytes([
                self.text[b],
                self.text[b + 1],
                self.text[b + 2],
                self.text[b + 3],
            ]);
        }
        w
    }

    /// Run one FIPS-180-4 SHA-256 compression round over the text block,
    /// updating `self.state256` (the first 8 words). Used for SHA-256.
    fn compress_sha256(&mut self) {
        let m = self.text_block_words();
        let mut w = [0u32; 64];
        w[..16].copy_from_slice(&m);
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut h = self.state256;
        for i in 0..64 {
            let s1 = h[4].rotate_right(6) ^ h[4].rotate_right(11) ^ h[4].rotate_right(25);
            let ch = (h[4] & h[5]) ^ ((!h[4]) & h[6]);
            let t1 = h[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = h[0].rotate_right(2) ^ h[0].rotate_right(13) ^ h[0].rotate_right(22);
            let maj = (h[0] & h[1]) ^ (h[0] & h[2]) ^ (h[1] & h[2]);
            let t2 = s0.wrapping_add(maj);
            h[7] = h[6];
            h[6] = h[5];
            h[5] = h[4];
            h[4] = h[3].wrapping_add(t1);
            h[3] = h[2];
            h[2] = h[1];
            h[1] = h[0];
            h[0] = t1.wrapping_add(t2);
        }
        for (s, &hv) in self.state256.iter_mut().zip(h.iter()) {
            *s = s.wrapping_add(hv);
        }
    }

    /// Run one FIPS-180-4 SHA-1 compression round over the text block,
    /// updating the first 5 words of `self.state256`.
    fn compress_sha1(&mut self) {
        let m = self.text_block_words();
        let mut w = [0u32; 80];
        w[..16].copy_from_slice(&m);
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let mut a = self.state256[0];
        let mut b = self.state256[1];
        let mut c = self.state256[2];
        let mut d = self.state256[3];
        let mut e = self.state256[4];
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        self.state256[0] = self.state256[0].wrapping_add(a);
        self.state256[1] = self.state256[1].wrapping_add(b);
        self.state256[2] = self.state256[2].wrapping_add(c);
        self.state256[3] = self.state256[3].wrapping_add(d);
        self.state256[4] = self.state256[4].wrapping_add(e);
    }

    /// Handle a START or CONTINUE command for one algorithm.
    ///
    /// START re-seeds the internal digest state from the algorithm's `H0`,
    /// then compresses the text block. CONTINUE compresses the text block on
    /// top of the existing internal state. BUSY is set then immediately
    /// cleared (synchronous compression — no DMA modeled).
    fn run_block(&mut self, ty: ShaType, is_continue: bool) {
        let idx = ty as usize;
        self.busy[idx] = true;
        match ty {
            ShaType::Sha1 => {
                if !is_continue {
                    self.state256[..5].copy_from_slice(&SHA1_H0);
                    self.state256[5..].fill(0);
                }
                self.compress_sha1();
            }
            ShaType::Sha256 => {
                if !is_continue {
                    self.state256 = SHA256_H0;
                }
                self.compress_sha256();
            }
            ShaType::Sha384 | ShaType::Sha512 => {
                // 1024-bit block / 64-bit lanes not modeled bit-exactly; keep
                // the engine state machine faithful by advancing a cheap mix so
                // START vs CONTINUE produce different state and snapshots round-
                // trip. Not the true digest (documented limitation).
                if !is_continue {
                    self.state512 = [0; 8];
                }
                for (i, lane) in self.state512.iter_mut().enumerate() {
                    *lane = lane
                        .rotate_left(7)
                        .wrapping_add(0x9E37_79B9_7F4A_7C15u64.wrapping_mul(i as u64 + 1));
                }
            }
        }
        self.started[idx] = true;
        // Synchronous engine: clears before firmware can observe it set.
        self.busy[idx] = false;
    }

    /// Handle a LOAD command: write the internal digest state into the text
    /// memory as big-endian words, mirroring real silicon. SHA-1 writes 5
    /// words, SHA-256 writes 8; the 64-bit algorithms write their lanes.
    fn run_load(&mut self, ty: ShaType) {
        match ty {
            ShaType::Sha1 => {
                for i in 0..5 {
                    let b = i * 4;
                    self.text[b..b + 4].copy_from_slice(&self.state256[i].to_be_bytes());
                }
            }
            ShaType::Sha256 => {
                for i in 0..8 {
                    let b = i * 4;
                    self.text[b..b + 4].copy_from_slice(&self.state256[i].to_be_bytes());
                }
            }
            ShaType::Sha384 | ShaType::Sha512 => {
                let words = if ty == ShaType::Sha384 { 6 } else { 8 };
                for i in 0..words {
                    let b = i * 8;
                    if b + 8 <= SHA_TEXT_LEN {
                        self.text[b..b + 8].copy_from_slice(&self.state512[i].to_be_bytes());
                    }
                }
            }
        }
    }

    /// Decode a command-bank offset into (algorithm, within-bank offset).
    fn decode_cmd(offset: u64) -> Option<(ShaType, u64)> {
        if offset < SHA_CMD_BASE_OFFSET {
            return None;
        }
        let rel = offset - SHA_CMD_BASE_OFFSET;
        let idx = rel / SHA_TYPE_STRIDE;
        let within = rel % SHA_TYPE_STRIDE;
        ShaType::from_index(idx).map(|ty| (ty, within))
    }

    /// Read a full 32-bit register word at a 4-byte-aligned offset.
    fn read_word(&self, word_off: u64) -> u32 {
        if word_off < SHA_CMD_BASE_OFFSET {
            // Text memory region.
            let b = word_off as usize;
            if b + 4 <= SHA_TEXT_LEN {
                return u32::from_le_bytes([
                    self.text[b],
                    self.text[b + 1],
                    self.text[b + 2],
                    self.text[b + 3],
                ]);
            }
            return 0;
        }
        match Self::decode_cmd(word_off) {
            Some((ty, SHA_BUSY_OFFSET)) => self.busy[ty as usize] as u32,
            // START / CONTINUE / LOAD read back 0 (write-only command regs).
            _ => 0,
        }
    }

    /// Write a full 32-bit register word at a 4-byte-aligned offset, applying
    /// command side-effects.
    fn write_word(&mut self, word_off: u64, value: u32) {
        if word_off < SHA_CMD_BASE_OFFSET {
            let b = word_off as usize;
            if b + 4 <= SHA_TEXT_LEN {
                self.text[b..b + 4].copy_from_slice(&value.to_le_bytes());
            }
            return;
        }
        if let Some((ty, within)) = Self::decode_cmd(word_off) {
            // Per the TRM a command fires on a "1" write to the trigger reg.
            if value & 1 == 0 {
                return;
            }
            match within {
                SHA_START_OFFSET => self.run_block(ty, false),
                SHA_CONTINUE_OFFSET => self.run_block(ty, true),
                SHA_LOAD_OFFSET => self.run_load(ty),
                // BUSY is read-only; writes are ignored.
                _ => {}
            }
        }
    }
}

impl Peripheral for Sha {
    // Inert walk: SHA ops run atomically at the command-register write (BUSY reads 0); tick() is an explicit no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        // Read-modify-write so byte-granular bus writes assemble a full word
        // before any command side-effect fires. For text memory this just
        // patches one byte; for command regs the trigger bit lives in byte 0.
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult::default()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Snap {
            text: Vec<u8>,
            state256: [u32; 8],
            state512: [u64; 8],
            busy: [bool; 4],
            started: [bool; 4],
        }
        let snap = Snap {
            text: self.text.to_vec(),
            state256: self.state256,
            state512: self.state512,
            busy: self.busy,
            started: self.started,
        };
        bincode::serialize(&snap).expect("bincode serialize Sha")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Snap {
            text: Vec<u8>,
            state256: [u32; 8],
            state512: [u64; 8],
            busy: [bool; 4],
            started: [bool; 4],
        }
        let snap: Snap = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("Sha snapshot decode: {e}"))
        })?;
        if snap.text.len() == SHA_TEXT_LEN {
            self.text.copy_from_slice(&snap.text);
        }
        self.state256 = snap.state256;
        self.state512 = snap.state512;
        self.busy = snap.busy;
        self.started = snap.started;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_u32_at(p: &mut Sha, offset: u64, value: u32) {
        for i in 0..4u64 {
            p.write(offset + i, ((value >> (i * 8)) & 0xFF) as u8)
                .unwrap();
        }
    }

    fn read_u32_at(p: &Sha, offset: u64) -> u32 {
        let mut v = 0u32;
        for i in 0..4u64 {
            v |= (p.read(offset + i).unwrap() as u32) << (i * 8);
        }
        v
    }

    /// Bank offset helpers.
    fn start_reg(ty: ShaType) -> u64 {
        SHA_CMD_BASE_OFFSET + (ty as u64) * SHA_TYPE_STRIDE + SHA_START_OFFSET
    }
    fn continue_reg(ty: ShaType) -> u64 {
        SHA_CMD_BASE_OFFSET + (ty as u64) * SHA_TYPE_STRIDE + SHA_CONTINUE_OFFSET
    }
    fn load_reg(ty: ShaType) -> u64 {
        SHA_CMD_BASE_OFFSET + (ty as u64) * SHA_TYPE_STRIDE + SHA_LOAD_OFFSET
    }
    fn busy_reg(ty: ShaType) -> u64 {
        SHA_CMD_BASE_OFFSET + (ty as u64) * SHA_TYPE_STRIDE + SHA_BUSY_OFFSET
    }

    /// Fill the text memory with the 64-byte padded block for the one-block
    /// message "abc", big-endian words exactly as firmware writes them.
    fn load_abc_block(p: &mut Sha) {
        // "abc" + 0x80 padding + length 24 bits in the final 64-bit big-endian.
        let mut block = [0u8; 64];
        block[0] = b'a';
        block[1] = b'b';
        block[2] = b'c';
        block[3] = 0x80;
        block[63] = 0x18; // bit length = 24
        for w in 0..16 {
            let word = u32::from_be_bytes([
                block[w * 4],
                block[w * 4 + 1],
                block[w * 4 + 2],
                block[w * 4 + 3],
            ]);
            // Firmware writes the word into TEXT (the bus stores LE bytes; the
            // model reads them back big-endian for compression).
            write_u32_at(p, SHA_TEXT_OFFSET + (w as u64) * 4, word.swap_bytes());
        }
    }

    #[test]
    fn base_is_esp32_classic_canonical_address() {
        let p = Sha::new();
        assert_eq!(p.base(), 0x3FF0_3000);
    }

    #[test]
    fn busy_reads_zero_after_command() {
        // Synchronous engine: BUSY is never observably set from firmware.
        let mut p = Sha::new();
        load_abc_block(&mut p);
        write_u32_at(&mut p, start_reg(ShaType::Sha256), 1);
        assert_eq!(
            read_u32_at(&p, busy_reg(ShaType::Sha256)),
            0,
            "BUSY must read 0 once the synchronous block compression completes"
        );
    }

    #[test]
    fn sha256_abc_matches_fips_vector() {
        // FIPS-180-4 example: SHA-256("abc") =
        //   ba7816bf 8f01cfea 414140de 5dae2223 b00361a3 96177a9c b410ff61 f20015ad
        let mut p = Sha::new();
        load_abc_block(&mut p);
        write_u32_at(&mut p, start_reg(ShaType::Sha256), 1);
        write_u32_at(&mut p, load_reg(ShaType::Sha256), 1);
        let expected: [u32; 8] = [
            0xba78_16bf,
            0x8f01_cfea,
            0x4141_40de,
            0x5dae_2223,
            0xb003_61a3,
            0x9617_7a9c,
            0xb410_ff61,
            0xf200_15ad,
        ];
        for (i, &e) in expected.iter().enumerate() {
            // Digest words land in TEXT memory big-endian; firmware reads the
            // word back and applies its own swap. Compare the big-endian word.
            let raw = read_u32_at(&p, SHA_TEXT_OFFSET + (i as u64) * 4);
            assert_eq!(
                raw.swap_bytes(),
                e,
                "SHA-256(\"abc\") word {i} mismatch: got {:08x}",
                raw.swap_bytes()
            );
        }
    }

    #[test]
    fn sha1_abc_matches_fips_vector() {
        // FIPS-180-4: SHA-1("abc") = a9993e36 4706816a ba3e2571 7850c26c 9cd0d89d
        let mut p = Sha::new();
        load_abc_block(&mut p);
        write_u32_at(&mut p, start_reg(ShaType::Sha1), 1);
        write_u32_at(&mut p, load_reg(ShaType::Sha1), 1);
        let expected: [u32; 5] = [
            0xa999_3e36,
            0x4706_816a,
            0xba3e_2571,
            0x7850_c26c,
            0x9cd0_d89d,
        ];
        for (i, &e) in expected.iter().enumerate() {
            let raw = read_u32_at(&p, SHA_TEXT_OFFSET + (i as u64) * 4);
            assert_eq!(raw.swap_bytes(), e, "SHA-1(\"abc\") word {i} mismatch");
        }
    }

    #[test]
    fn text_memory_round_trips() {
        let mut p = Sha::new();
        write_u32_at(&mut p, SHA_TEXT_OFFSET, 0xDEAD_BEEF);
        write_u32_at(&mut p, SHA_TEXT_OFFSET + 60, 0xCAFE_F00D);
        assert_eq!(read_u32_at(&p, SHA_TEXT_OFFSET), 0xDEAD_BEEF);
        assert_eq!(read_u32_at(&p, SHA_TEXT_OFFSET + 60), 0xCAFE_F00D);
    }

    #[test]
    fn command_banks_decode_correctly() {
        assert_eq!(start_reg(ShaType::Sha1), 0x80);
        assert_eq!(busy_reg(ShaType::Sha1), 0x8C);
        assert_eq!(start_reg(ShaType::Sha256), 0x90);
        assert_eq!(start_reg(ShaType::Sha384), 0xA0);
        assert_eq!(start_reg(ShaType::Sha512), 0xB0);
        assert_eq!(busy_reg(ShaType::Sha512), 0xBC);
    }

    #[test]
    fn zero_write_does_not_trigger() {
        // A 0 write to START must not run a block (only a 1 triggers).
        let mut p = Sha::new();
        load_abc_block(&mut p);
        let before: Vec<u32> = (0..8)
            .map(|i| read_u32_at(&p, SHA_TEXT_OFFSET + i * 4))
            .collect();
        write_u32_at(&mut p, start_reg(ShaType::Sha256), 0);
        write_u32_at(&mut p, load_reg(ShaType::Sha256), 0);
        let after: Vec<u32> = (0..8)
            .map(|i| read_u32_at(&p, SHA_TEXT_OFFSET + i * 4))
            .collect();
        assert_eq!(before, after, "no command should fire on a 0 write");
    }

    #[test]
    fn continue_differs_from_start() {
        // Hashing the same block twice via START then CONTINUE must change
        // the internal state (CONTINUE folds in the prior digest).
        let mut p = Sha::new();
        load_abc_block(&mut p);
        write_u32_at(&mut p, start_reg(ShaType::Sha256), 1);
        write_u32_at(&mut p, load_reg(ShaType::Sha256), 1);
        let after_start = read_u32_at(&p, SHA_TEXT_OFFSET);
        write_u32_at(&mut p, continue_reg(ShaType::Sha256), 1);
        write_u32_at(&mut p, load_reg(ShaType::Sha256), 1);
        let after_continue = read_u32_at(&p, SHA_TEXT_OFFSET);
        assert_ne!(
            after_start, after_continue,
            "CONTINUE must fold in prior state, changing the digest"
        );
    }

    #[test]
    fn runtime_snapshot_round_trip_preserves_state() {
        let mut p = Sha::new();
        load_abc_block(&mut p);
        write_u32_at(&mut p, start_reg(ShaType::Sha256), 1);
        write_u32_at(&mut p, load_reg(ShaType::Sha256), 1);
        let snap = p.runtime_snapshot();

        let mut restored = Sha::new();
        restored.restore_runtime_snapshot(&snap).unwrap();
        for i in 0..8u64 {
            assert_eq!(
                read_u32_at(&restored, SHA_TEXT_OFFSET + i * 4),
                read_u32_at(&p, SHA_TEXT_OFFSET + i * 4),
                "text word {i} must survive snapshot round-trip"
            );
        }
        // Internal state must also survive: a CONTINUE on the restored engine
        // must match a CONTINUE on the original.
        write_u32_at(&mut p, continue_reg(ShaType::Sha256), 1);
        write_u32_at(&mut p, load_reg(ShaType::Sha256), 1);
        write_u32_at(&mut restored, continue_reg(ShaType::Sha256), 1);
        write_u32_at(&mut restored, load_reg(ShaType::Sha256), 1);
        assert_eq!(
            read_u32_at(&p, SHA_TEXT_OFFSET),
            read_u32_at(&restored, SHA_TEXT_OFFSET),
            "internal digest state must survive snapshot round-trip"
        );
    }
}
