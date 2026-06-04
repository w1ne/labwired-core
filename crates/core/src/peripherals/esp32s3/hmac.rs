// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 HMAC accelerator digital twin.
//!
//! Mapped at base `DR_REG_HMAC_BASE = 0x6003_E000`, window 4 KiB.
//! See ESP32-S3 TRM §22 ("HMAC Accelerator") and ESP-IDF
//! `soc/esp32s3/.../hmac_ll.h` for the register-level driver contract.
//!
//! The real HMAC accelerator computes `HMAC-SHA256(key, message)` where the
//! key is one of the 256-bit eFuse key blocks (selected by purpose + key-id)
//! and the message is fed one 512-bit block at a time. This twin reproduces
//! the *driver-visible* behaviour: register round-trip, message-block
//! accumulation, busy/error status, and a real HMAC-SHA256 digest readable
//! from the result registers.
//!
//! ## HONEST LIMITATION — the eFuse key is not reproducible
//!
//! On real silicon the HMAC key lives in an eFuse key block configured as a
//! *downstream* ("write-only") key: it is consumed by the crypto hardware but
//! is **not CPU-readable** (that is the whole security point — a key the CPU
//! could read would not be a hardware-protected key). Therefore this twin
//! **cannot** reproduce the byte-for-byte HMAC that real hardware would
//! produce, because it does not know the secret key bytes.
//!
//! Instead the twin uses a **deterministic modeled key** derived solely from
//! the selected key-block id (see [`modeled_key`]): block `n` maps to a fixed,
//! reproducible 32-byte key. The default / id-0 key is all-zero. This keeps
//! the accelerator *functional and reproducible* — firmware that feeds a
//! message and reads back a 256-bit HMAC gets a correct, stable HMAC-SHA256
//! over the modeled key, and a test can recompute the same value with `sha2`
//! directly. Matching a *specific* silicon device requires that device's real
//! eFuse key, which is out of scope.
//!
//! This is the same spirit as a SHA-accelerator twin (real crypto via the
//! `sha2` crate), but key-limited.
//!
//! ## Register map (offsets from base, per `hmac_ll.h` / `hwcrypto_reg.h`)
//!
//! | Offset | Name                       | Dir | Notes                                    |
//! |--------|----------------------------|-----|------------------------------------------|
//! | 0x40   | HMAC_SET_START             | WS  | Begin an HMAC operation                  |
//! | 0x44   | HMAC_SET_PARA_PURPOSE      | WO  | Key purpose (UP=8, DS=7, JTAG=6, ALL=5)  |
//! | 0x48   | HMAC_SET_PARA_KEY          | WO  | eFuse key-block id [2:0]                  |
//! | 0x4C   | HMAC_SET_PARA_FINISH       | WS  | Apply + check config                     |
//! | 0x50   | HMAC_SET_MESSAGE_ONE       | WS  | Process the 512-bit block in WDATA       |
//! | 0x54   | HMAC_SET_MESSAGE_ING       | WS  | "more blocks follow"                     |
//! | 0x58   | HMAC_SET_MESSAGE_END       | WS  | HW padding for the final block           |
//! | 0x5C   | HMAC_SET_RESULT_FINISH     | WO  | Return to idle after reading result      |
//! | 0x60   | HMAC_SET_INVALIDATE_JTAG   | WS  | Clear downstream JTAG result             |
//! | 0x64   | HMAC_SET_INVALIDATE_DS     | WS  | Clear downstream DS result               |
//! | 0x68   | HMAC_QUERY_ERROR           | RO  | bit0: 1 = key/purpose mismatch           |
//! | 0x6C   | HMAC_QUERY_BUSY            | RO  | bit0: 1 = busy, 0 = idle/done            |
//! | 0x80   | HMAC_WDATA_BASE (16 words) | WO  | 512-bit message block                    |
//! | 0xC0   | HMAC_RDATA_BASE (8 words)  | RO  | 256-bit HMAC result, big-endian words    |
//! | 0xF0   | HMAC_SET_MESSAGE_PAD       | WO  | Software-padding mode                    |
//! | 0xF4   | HMAC_ONE_BLOCK             | WS  | Whole message fit one block, no padding  |
//! | 0xF8   | HMAC_SOFT_JTAG_CTRL        | WS  | JTAG verification (unused here)          |
//! | 0xFC   | HMAC_WR_JTAG               | WO  | JTAG key word (unused here)              |
//! | 0x1FC  | HMAC_DATE                  | R/W | Version word                             |
//!
//! ## Result byte order
//!
//! `hmac_ll_read_result_256()` reads eight `u32` words from `HMAC_RDATA_BASE`
//! into `result[0..8]`. The HMAC-SHA256 digest is a 32-byte big-endian value;
//! the hardware presents `digest[0..4]` as the first word, etc. So each result
//! word is the big-endian assembly of four consecutive digest bytes. We store
//! the digest so that `read_u32(0xC0 + 4*i) == u32::from_be_bytes(digest[4i..])`
//! — matching how firmware reassembles the standard digest.
//!
//! ## Interrupt
//!
//! The ESP32-S3 interrupt-source table (`soc/esp32s3/interrupts.h`) lists
//! `ETS_RSA_INTR_SOURCE`, `ETS_AES_INTR_SOURCE`, `ETS_SHA_INTR_SOURCE` — but
//! **no HMAC source**: the HMAC accelerator has no dedicated interrupt-matrix
//! line on the S3 (the driver polls `HMAC_QUERY_BUSY`). The constructor still
//! takes a `source_id` for API symmetry with the other S3 peripherals, but
//! `tick()` never emits it (documented no-op).

use crate::{Peripheral, SimResult};

pub const HMAC_BASE: u32 = 0x6003_E000;
pub const HMAC_SIZE: u64 = 0x1000;

// --- control / config registers ---
const REG_SET_START: u64 = 0x40;
const REG_SET_PARA_PURPOSE: u64 = 0x44;
const REG_SET_PARA_KEY: u64 = 0x48;
const REG_SET_PARA_FINISH: u64 = 0x4C;
const REG_SET_MESSAGE_ONE: u64 = 0x50;
const REG_SET_MESSAGE_ING: u64 = 0x54;
const REG_SET_MESSAGE_END: u64 = 0x58;
const REG_SET_RESULT_FINISH: u64 = 0x5C;
const REG_SET_INVALIDATE_JTAG: u64 = 0x60;
const REG_SET_INVALIDATE_DS: u64 = 0x64;
const REG_QUERY_ERROR: u64 = 0x68;
const REG_QUERY_BUSY: u64 = 0x6C;

// --- message block (16 words) ---
const REG_WDATA_BASE: u64 = 0x80;
const WDATA_WORDS: usize = 16; // 512-bit / 32

// --- result (8 words) ---
const REG_RDATA_BASE: u64 = 0xC0;
const RDATA_WORDS: usize = 8; // 256-bit / 32

const REG_SET_MESSAGE_PAD: u64 = 0xF0;
const REG_ONE_BLOCK: u64 = 0xF4;
const REG_SOFT_JTAG_CTRL: u64 = 0xF8;
const REG_WR_JTAG: u64 = 0xFC;
const REG_DATE: u64 = 0x1FC;

/// Key purposes (`HMAC_LL_EFUSE_KEY_PURPOSE_*`).
const PURPOSE_DOWN_ALL: u32 = 5;
const PURPOSE_DOWN_JTAG: u32 = 6;
const PURPOSE_DOWN_DS: u32 = 7;
const PURPOSE_UP: u32 = 8;

const SHA256_BLOCK: usize = 64;
const SHA256_DIGEST: usize = 32;

/// Default version word (`HMAC_DATE` reset value, 538969624 = 0x20190618).
const HMAC_DATE_RESET: u32 = 0x2019_0618;

/// Deterministic modeled key for eFuse key-block `id`.
///
/// The real eFuse key is not CPU-readable (see module docs), so we synthesize
/// a fixed reproducible 32-byte key from the block id. Block 0 is all-zero;
/// other blocks repeat their id byte. This is *not* the silicon key — it only
/// makes the accelerator functional and deterministic.
pub fn modeled_key(key_id: u8) -> [u8; SHA256_DIGEST] {
    [key_id; SHA256_DIGEST]
}

/// Compute HMAC-SHA256(key, message) using the `sha2` crate directly.
///
/// Standard RFC 2104 construction with SHA-256 (block size 64, digest 32).
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; SHA256_DIGEST] {
    use sha2::{Digest, Sha256};

    // Keys longer than the block size are hashed first.
    let mut k0 = [0u8; SHA256_BLOCK];
    if key.len() > SHA256_BLOCK {
        let hashed = Sha256::digest(key);
        k0[..SHA256_DIGEST].copy_from_slice(&hashed);
    } else {
        k0[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; SHA256_BLOCK];
    let mut opad = [0x5cu8; SHA256_BLOCK];
    for i in 0..SHA256_BLOCK {
        ipad[i] ^= k0[i];
        opad[i] ^= k0[i];
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    let out = outer.finalize();

    let mut result = [0u8; SHA256_DIGEST];
    result.copy_from_slice(&out);
    result
}

/// ESP32-S3 HMAC accelerator twin.
pub struct Esp32s3Hmac {
    /// Interrupt-matrix source id. The S3 HMAC has no dedicated source, so
    /// this is retained for API symmetry only and never emitted.
    source_id: u32,

    // --- config (round-tripped) ---
    purpose: u32,
    key_id: u32,
    /// Set by HMAC_SET_PARA_FINISH; cleared on a new operation. Mirrors the
    /// "configuration applied" state.
    para_finished: bool,
    /// 1 = key/purpose mismatch detected at config-finish (HMAC_QUERY_ERROR).
    error: bool,

    // --- message feed ---
    /// 16-word message block staged in WDATA before a MESSAGE_* trigger.
    wdata: [u32; WDATA_WORDS],
    /// All message bytes fed so far across MESSAGE_ONE/ING/END blocks.
    message: Vec<u8>,
    /// True once HMAC_SET_START has armed an operation but it hasn't finished.
    started: bool,
    /// True between the final block and HMAC_SET_RESULT_FINISH: result valid.
    result_ready: bool,

    // --- result, stored big-endian-per-word ---
    rdata: [u32; RDATA_WORDS],

    date: u32,
    soft_jtag: u32,
    wr_jtag: u32,
}

impl Esp32s3Hmac {
    pub fn new(source_id: u32) -> Self {
        Self {
            source_id,
            purpose: 0,
            key_id: 0,
            para_finished: false,
            error: false,
            wdata: [0; WDATA_WORDS],
            message: Vec::new(),
            started: false,
            result_ready: false,
            rdata: [0; RDATA_WORDS],
            date: HMAC_DATE_RESET,
            soft_jtag: 0,
            wr_jtag: 0,
        }
    }

    /// Append the staged 512-bit WDATA block (16 words, big-endian per the
    /// SHA datapath) to the accumulated message.
    fn append_block(&mut self) {
        for w in self.wdata.iter() {
            self.message.extend_from_slice(&w.to_be_bytes());
        }
    }

    /// Finalize: compute HMAC-SHA256 over the accumulated message with the
    /// modeled key, and stage the digest into the result registers.
    ///
    /// The accumulated bytes are treated as the HMAC message. On the
    /// hardware-padding path (MESSAGE_END) those are the raw message bytes; on
    /// the ONE_BLOCK / software-padding path they include the firmware's own
    /// padding, so the digest is taken over exactly the bytes the firmware
    /// supplied (deterministic-twin behaviour — byte-exact silicon parity
    /// still needs the real eFuse key; see module docs).
    fn finalize(&mut self) {
        let key = modeled_key(self.key_id as u8);
        let digest = hmac_sha256(&key, &self.message);
        for i in 0..RDATA_WORDS {
            let mut word = [0u8; 4];
            word.copy_from_slice(&digest[i * 4..i * 4 + 4]);
            // Result word = big-endian assembly of four digest bytes, so a
            // firmware read_u32 reproduces digest[4i..4i+4] in order.
            self.rdata[i] = u32::from_be_bytes(word);
        }
        self.result_ready = true;
        self.started = false;
    }

    /// Recompute the config error flag: purpose must be one of the known
    /// values. (We can't actually validate against eFuse, so a known purpose
    /// is treated as "agrees".)
    fn check_config(&mut self) {
        self.error = !matches!(
            self.purpose,
            PURPOSE_DOWN_ALL | PURPOSE_DOWN_JTAG | PURPOSE_DOWN_DS | PURPOSE_UP
        );
        self.para_finished = true;
    }
}

impl std::fmt::Debug for Esp32s3Hmac {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Esp32s3Hmac")
            .field("source_id", &self.source_id)
            .field("purpose", &self.purpose)
            .field("key_id", &self.key_id)
            .field("para_finished", &self.para_finished)
            .field("error", &self.error)
            .field("started", &self.started)
            .field("result_ready", &self.result_ready)
            .field("message_len", &self.message.len())
            .finish()
    }
}

impl Peripheral for Esp32s3Hmac {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        // The driver only uses word accesses; stray byte reads return 0.
        Ok(0)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let v = match offset {
            REG_SET_PARA_PURPOSE => self.purpose,
            REG_SET_PARA_KEY => self.key_id,
            // QUERY_ERROR: 0 = key/purpose agree, >=1 = error.
            REG_QUERY_ERROR => self.error as u32,
            // QUERY_BUSY: this twin completes synchronously, so it is never
            // busy — every poll reads 0 (idle/done) and firmware loops exit.
            REG_QUERY_BUSY => 0,
            REG_RDATA_BASE..=REG_RDATA_END => {
                let idx = ((offset - REG_RDATA_BASE) / 4) as usize;
                self.rdata.get(idx).copied().unwrap_or(0)
            }
            REG_WDATA_BASE..=REG_WDATA_END => {
                let idx = ((offset - REG_WDATA_BASE) / 4) as usize;
                self.wdata.get(idx).copied().unwrap_or(0)
            }
            REG_SOFT_JTAG_CTRL => self.soft_jtag,
            REG_WR_JTAG => self.wr_jtag,
            REG_DATE => self.date,
            _ => 0,
        };
        Ok(v)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // Byte writes ignored — the driver writes whole 32-bit words.
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            REG_SET_START => {
                // Arm a fresh operation: reset message accumulation + result.
                self.started = true;
                self.result_ready = false;
                self.message.clear();
                self.rdata = [0; RDATA_WORDS];
            }
            REG_SET_PARA_PURPOSE => self.purpose = value & 0x0F,
            REG_SET_PARA_KEY => self.key_id = value & 0x07,
            REG_SET_PARA_FINISH => self.check_config(),
            REG_WDATA_BASE..=REG_WDATA_END => {
                let idx = ((offset - REG_WDATA_BASE) / 4) as usize;
                if let Some(slot) = self.wdata.get_mut(idx) {
                    *slot = value;
                }
            }
            // MESSAGE_ONE / MESSAGE_ING both push the staged block. (ONE is
            // the first block, ING continues; semantics for accumulation are
            // identical here.)
            REG_SET_MESSAGE_ONE | REG_SET_MESSAGE_ING => {
                self.append_block();
            }
            // MESSAGE_END: push the final block, then hardware-pad + compute.
            REG_SET_MESSAGE_END => {
                self.append_block();
                self.finalize();
            }
            // ONE_BLOCK: the staged block already holds the firmware's own
            // software padding (HMAC_SET_MESSAGE_PAD path). Push it and
            // finalize; the digest is computed over the padded bytes, which is
            // the deterministic-twin behaviour. (Byte-exact match with silicon
            // still requires the real eFuse key — see module docs.)
            REG_ONE_BLOCK => {
                self.append_block();
                self.finalize();
            }
            REG_SET_MESSAGE_PAD => { /* software-padding mode flag; no state */ }
            REG_SET_RESULT_FINISH => {
                // Return to idle; clear result-ready latch.
                self.result_ready = false;
                self.started = false;
            }
            REG_SET_INVALIDATE_JTAG | REG_SET_INVALIDATE_DS => {
                // Downstream-result clear; no CPU-visible state in this twin.
            }
            REG_SOFT_JTAG_CTRL => self.soft_jtag = value,
            REG_WR_JTAG => self.wr_jtag = value,
            REG_DATE => self.date = value & 0x3FFF_FFFF,
            _ => {} // accept-and-ignore
        }
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    // No `tick()` override: the S3 HMAC has no interrupt-matrix source, so the
    // twin emits no IRQ. `source_id` is retained only for API symmetry.
}

// Inclusive upper bounds for the WDATA / RDATA word windows.
const REG_WDATA_END: u64 = REG_WDATA_BASE + (WDATA_WORDS as u64 - 1) * 4;
const REG_RDATA_END: u64 = REG_RDATA_BASE + (RDATA_WORDS as u64 - 1) * 4;

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: feed `message` (must be a single 512-bit block, hardware-padded
    /// case is exercised separately) and read back the 32-byte result.
    fn read_result(hmac: &Esp32s3Hmac) -> [u8; SHA256_DIGEST] {
        let mut out = [0u8; SHA256_DIGEST];
        for i in 0..RDATA_WORDS {
            let w = hmac.read_u32(REG_RDATA_BASE + (i as u64) * 4).unwrap();
            out[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
        }
        out
    }

    /// Drive the accelerator over a single 64-byte block and compare against an
    /// independently computed HMAC-SHA256 with the modeled (zero) key.
    #[test]
    fn hmac_matches_reference_zero_key() {
        let mut hmac = Esp32s3Hmac::new(0);

        // Configure: purpose UP, key block 0 (modeled key = all zero).
        hmac.write_u32(REG_SET_PARA_PURPOSE, PURPOSE_UP).unwrap();
        hmac.write_u32(REG_SET_PARA_KEY, 0).unwrap();
        hmac.write_u32(REG_SET_PARA_FINISH, 1).unwrap();
        assert_eq!(hmac.read_u32(REG_QUERY_ERROR).unwrap(), 0, "purpose UP ok");

        hmac.write_u32(REG_SET_START, 1).unwrap();

        // One full 512-bit message block: bytes 0..64.
        let mut block_bytes = [0u8; SHA256_BLOCK];
        for (i, b) in block_bytes.iter_mut().enumerate() {
            *b = i as u8;
        }
        for i in 0..WDATA_WORDS {
            let mut w = [0u8; 4];
            w.copy_from_slice(&block_bytes[i * 4..i * 4 + 4]);
            hmac.write_u32(REG_WDATA_BASE + (i as u64) * 4, u32::from_be_bytes(w))
                .unwrap();
        }
        // Single block, hardware padding path.
        hmac.write_u32(REG_SET_MESSAGE_END, 1).unwrap();

        let got = read_result(&hmac);

        // Independent reference: HMAC-SHA256(zero-key-32, block_bytes).
        let expect = reference_hmac(&[0u8; 32], &block_bytes);
        assert_eq!(got, expect, "twin HMAC must equal sha2 reference");

        hmac.write_u32(REG_SET_RESULT_FINISH, 2).unwrap();
    }

    /// Same, but with a non-zero modeled key (key block 3 → key = [3;32]).
    #[test]
    fn hmac_matches_reference_nonzero_modeled_key() {
        let mut hmac = Esp32s3Hmac::new(0);
        hmac.write_u32(REG_SET_PARA_PURPOSE, PURPOSE_UP).unwrap();
        hmac.write_u32(REG_SET_PARA_KEY, 3).unwrap();
        hmac.write_u32(REG_SET_PARA_FINISH, 1).unwrap();
        hmac.write_u32(REG_SET_START, 1).unwrap();

        let msg = b"LabWired HMAC twin determinism check 0123456789!"; // 48 bytes
        let mut block = [0u8; SHA256_BLOCK];
        block[..msg.len()].copy_from_slice(msg);
        for i in 0..WDATA_WORDS {
            let mut w = [0u8; 4];
            w.copy_from_slice(&block[i * 4..i * 4 + 4]);
            hmac.write_u32(REG_WDATA_BASE + (i as u64) * 4, u32::from_be_bytes(w))
                .unwrap();
        }
        hmac.write_u32(REG_SET_MESSAGE_END, 1).unwrap();

        let got = read_result(&hmac);
        let expect = reference_hmac(&[3u8; 32], &block);
        assert_eq!(got, expect);
    }

    /// QUERY_BUSY reads 0 (idle/done) so firmware poll loops terminate; result
    /// becomes ready after MESSAGE_END.
    #[test]
    fn query_busy_done_and_result_ready() {
        let mut hmac = Esp32s3Hmac::new(0);
        assert_eq!(hmac.read_u32(REG_QUERY_BUSY).unwrap(), 0, "idle = not busy");

        hmac.write_u32(REG_SET_PARA_PURPOSE, PURPOSE_UP).unwrap();
        hmac.write_u32(REG_SET_PARA_FINISH, 1).unwrap();
        hmac.write_u32(REG_SET_START, 1).unwrap();
        assert!(!hmac.result_ready);

        // Empty block then end.
        hmac.write_u32(REG_SET_MESSAGE_END, 1).unwrap();
        assert_eq!(
            hmac.read_u32(REG_QUERY_BUSY).unwrap(),
            0,
            "completes synchronously"
        );
        assert!(hmac.result_ready, "result ready after MESSAGE_END");
    }

    /// Config registers round-trip; bad purpose raises QUERY_ERROR.
    #[test]
    fn control_round_trip_and_error_flag() {
        let mut hmac = Esp32s3Hmac::new(7);

        hmac.write_u32(REG_SET_PARA_PURPOSE, PURPOSE_DOWN_DS).unwrap();
        hmac.write_u32(REG_SET_PARA_KEY, 5).unwrap();
        assert_eq!(hmac.read_u32(REG_SET_PARA_PURPOSE).unwrap(), PURPOSE_DOWN_DS);
        // key field is [2:0], so 5 round-trips intact.
        assert_eq!(hmac.read_u32(REG_SET_PARA_KEY).unwrap(), 5);

        // Known purpose => no error.
        hmac.write_u32(REG_SET_PARA_FINISH, 1).unwrap();
        assert_eq!(hmac.read_u32(REG_QUERY_ERROR).unwrap(), 0);

        // Unknown purpose => error asserted.
        hmac.write_u32(REG_SET_PARA_PURPOSE, 0xF).unwrap();
        hmac.write_u32(REG_SET_PARA_FINISH, 1).unwrap();
        assert_eq!(hmac.read_u32(REG_QUERY_ERROR).unwrap(), 1, "bad purpose errors");

        // DATE register round-trips (masked to 30 bits).
        hmac.write_u32(REG_DATE, 0xFFFF_FFFF).unwrap();
        assert_eq!(hmac.read_u32(REG_DATE).unwrap(), 0x3FFF_FFFF);

        // WDATA word staging round-trips.
        hmac.write_u32(REG_WDATA_BASE, 0xDEAD_BEEF).unwrap();
        assert_eq!(hmac.read_u32(REG_WDATA_BASE).unwrap(), 0xDEAD_BEEF);
    }

    /// Independent RFC 2104 HMAC-SHA256 reference computed with sha2 directly,
    /// kept separate from the production path so the test is a true oracle.
    fn reference_hmac(key: &[u8], msg: &[u8]) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut k0 = [0u8; 64];
        if key.len() > 64 {
            k0[..32].copy_from_slice(&Sha256::digest(key));
        } else {
            k0[..key.len()].copy_from_slice(key);
        }
        let ipad: Vec<u8> = k0.iter().map(|b| b ^ 0x36).collect();
        let opad: Vec<u8> = k0.iter().map(|b| b ^ 0x5c).collect();
        let mut inner = Sha256::new();
        inner.update(&ipad);
        inner.update(msg);
        let id = inner.finalize();
        let mut outer = Sha256::new();
        outer.update(&opad);
        outer.update(id);
        let mut out = [0u8; 32];
        out.copy_from_slice(&outer.finalize());
        out
    }
}
