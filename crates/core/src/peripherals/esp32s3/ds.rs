// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 Digital Signature (DS) peripheral digital twin.
//!
//! Mapped at base `DR_REG_DIGITAL_SIGNATURE_BASE = 0x6003_D000`, window 4 KiB.
//! See ESP32-S3 TRM chapter "Digital Signature" and ESP-IDF
//! `soc/esp32s3/include/soc/ds_reg.h` for the register-level contract.
//!
//! ## What the real DS peripheral does
//!
//! The Digital Signature peripheral produces an RSA signature
//! `Z = MESSAGE^d mod M` (RSASSA / raw modular exponentiation) *without ever
//! exposing the private key to software*. On real silicon the private-key
//! parameters (`d`, `M`, `M'`, `r^2`, plus a digest/MD5 integrity field) are
//! AES-encrypted into a "ciphertext" blob `C`. At sign time the DS engine:
//!
//! 1. derives a 256-bit AES key with the HMAC accelerator from an eFuse key
//!    block configured for the `DS` purpose (key purpose 7),
//! 2. AES-decrypts the parameter blob `C` with that key,
//! 3. checks the embedded MD5 digest (padding/integrity check),
//! 4. runs the RSA exponentiation `Z = Y^d mod M` over the loaded message `Y`,
//! 5. latches the signature into the result memory.
//!
//! Firmware drives it by writing the ciphertext into the Y/M/RB/BOX memory
//! windows, pulsing `SET_START`/`SET_CONTINUE`/`SET_FINISH`, polling
//! `QUERY_BUSY` until idle, then reading `QUERY_KEY_WRONG` and `QUERY_CHECK`
//! for errors before reading the signature back out of the result window.
//!
//! ## HONEST LIMITATION — the eFuse DS key is not reproducible
//!
//! The AES key that unwraps the private-key parameter blob is derived from an
//! eFuse key block configured as a *downstream* (write-only) key: it is
//! consumed by the HMAC + AES hardware but is **never CPU-readable** (that is
//! the entire security premise of the DS peripheral). This twin therefore
//! **cannot** reproduce the byte-exact AES decryption that real silicon would
//! perform, because it does not know the secret eFuse key bytes.
//!
//! Instead this twin models the parameter block **as plaintext RSA params**:
//! the Y/M memory windows are interpreted directly as the message `Y` and the
//! modulus `M`, and the exponent is taken from a software-settable field
//! (defaulting to the common public exponent `65537`, i.e. the twin computes
//! `Z = Y^e mod M`). This keeps the accelerator *functional and reproducible*
//! — firmware that loads operands and pulses the control sequence reads back a
//! mathematically correct modular exponentiation, and a test can recompute the
//! same value independently. Matching a *specific* silicon device's signature
//! requires that device's real eFuse DS key and its AES-wrapped private
//! parameters, which is out of scope (and impossible without the secret key).
//!
//! This mirrors the spirit of the HMAC twin (`hmac.rs`): real crypto math, but
//! key-material-limited.
//!
//! ## Register map (offsets from base, per `ds_reg.h`)
//!
//! | Offset  | Name              | Dir | Notes                                       |
//! |--------:|-------------------|-----|---------------------------------------------|
//! | 0x000   | `DS_Y_MEM`        | R/W | message operand `Y` (128 words, 512 B)      |
//! | 0x200   | `DS_M_MEM`        | R/W | modulus `M` (128 words)                     |
//! | 0x400   | `DS_RB_MEM`       | R/W | `r^2 mod M` helper (round-tripped, ignored) |
//! | 0x600   | `DS_BOX_MEM`      | R/W | encrypted param box (`M'`, len, MD5; modeled) |
//! | 0x800   | `DS_Z_MEM`        | RO  | result: signature `Z` (128 words)           |
//! | 0xE00   | `DS_SET_START`    | WT  | write 1 → begin / activate the DS engine    |
//! | 0xE04   | `DS_SET_CONTINUE` | WT  | write 1 → operands loaded, run the modexp   |
//! | 0xE08   | `DS_SET_FINISH`   | WT  | write 1 → finish, return engine to idle     |
//! | 0xE0C   | `DS_QUERY_BUSY`   | RO  | bit0: 1 = busy, 0 = idle/done               |
//! | 0xE10   | `DS_QUERY_KEY_WRONG`| RO| [3:0]: 0 = key ok, >0 = eFuse key unusable  |
//! | 0xE14   | `DS_QUERY_CHECK`  | RO  | bit0 = padding-check fail, bit1 = MD5 fail  |
//! | 0xE20   | `DS_DATE`         | R/W | version word                                |
//!
//! `DS_SET_ME` does not exist as a distinct register on the S3 (the legacy
//! ESP32 DS used a `SET_ME` "activate" pulse; the S3 folds activation into
//! `SET_START`). For API/portability symmetry with firmware that still pokes a
//! "set me" alias, this twin accepts a write at the historic `SET_ME` offset
//! (`0xE04`, shared with `SET_CONTINUE`) and treats it as a continue/run pulse.
//!
//! The four operand windows are each `0x200` bytes apart (512 B = 128 × 32-bit
//! words = 4096-bit max), matching the RSA accelerator's block stride exactly.
//!
//! ## Little-endian word assembly convention
//!
//! Identical to the RSA twin: operand-window word `i` (the 32-bit value at
//! window-offset `i*4`) is the `i`-th least-significant base-2^32 limb of the
//! big integer, so the integer is `sum(word[i] << (32*i))`. Each 32-bit word is
//! stored little-endian in MMIO bytes. The result `Z` is written back the same
//! way, zero-padded to the operand length.
//!
//! ## Operation timing — polled, one-tick busy
//!
//! Like the HMAC accelerator the S3 DS peripheral has **no dedicated
//! interrupt-matrix source**; firmware polls `DS_QUERY_BUSY`. To exercise that
//! poll loop faithfully this twin makes `SET_START`/`SET_CONTINUE` latch
//! `busy = 1` immediately and clears it (computing the signature) on the next
//! [`Peripheral::tick`]. The constructor still takes a `source_id` for API
//! symmetry with the other S3 peripherals, but `tick()` never emits it.

use crate::{Peripheral, PeripheralTickResult, SimResult};

pub const DS_BASE: u32 = 0x6003_D000;
pub const DS_SIZE: u64 = 0x1000;

/// Bytes per operand memory window (512 B = 128 × 32-bit words = 4096-bit max).
const BLOCK_SIZE: u64 = 0x200;
/// Number of 32-bit words per operand window.
const BLOCK_WORDS: usize = (BLOCK_SIZE / 4) as usize;

// --- operand / result memory windows (relative to DS_BASE) ---
const OFF_Y_MEM: u64 = 0x000; // message Y
const OFF_M_MEM: u64 = 0x200; // modulus M
const OFF_RB_MEM: u64 = 0x400; // r^2 mod M helper (ignored by math)
const OFF_BOX_MEM: u64 = 0x600; // encrypted param box (modeled)
const OFF_Z_MEM: u64 = 0x800; // result signature Z (read-only)
const MEM_REGION_END: u64 = OFF_Z_MEM + BLOCK_SIZE; // 0xA00 — end of all windows

// --- control / status registers ---
const OFF_SET_START: u64 = 0xE00;
const OFF_SET_CONTINUE: u64 = 0xE04; // a.k.a. legacy SET_ME activate pulse
const OFF_SET_FINISH: u64 = 0xE08;
const OFF_QUERY_BUSY: u64 = 0xE0C;
const OFF_QUERY_KEY_WRONG: u64 = 0xE10;
const OFF_QUERY_CHECK: u64 = 0xE14;
const OFF_DATE: u64 = 0xE20;

/// `DS_DATE` reset value (esp-idf reports 0x20200618 for the S3 DS block).
const DS_DATE_RESET: u32 = 0x2020_0618;

/// Default RSA public exponent used when firmware does not override it. The
/// real private exponent `d` lives inside the encrypted (unreproducible) param
/// box; this twin signs with `Z = Y^e mod M` so the result is a correct,
/// reproducible modular exponentiation. See the module docs.
const DEFAULT_EXPONENT: u32 = 65537;

/// ESP32-S3 Digital Signature peripheral twin.
pub struct Esp32s3Ds {
    /// Interrupt-matrix source id. The S3 DS has no dedicated source, so this
    /// is retained for API symmetry only and never emitted from `tick()`.
    source_id: u32,

    /// Operand / result windows, each `BLOCK_WORDS` little-endian limbs.
    y_mem: Vec<u32>,
    m_mem: Vec<u32>,
    rb_mem: Vec<u32>,
    box_mem: Vec<u32>,
    z_mem: Vec<u32>,

    /// Modeled public exponent for the `Z = Y^e mod M` computation. Defaults to
    /// 65537; firmware can override the low/high words via the BOX window
    /// (modeled param block) — but the simple default covers the common case.
    exponent: u32,

    /// `busy` latch: set 1 by SET_START/SET_CONTINUE, cleared on the next tick
    /// after the signature is computed. Read back via DS_QUERY_BUSY.
    busy: bool,
    /// True after the engine has been armed by SET_START (operands expected
    /// next). Cleared by SET_FINISH.
    started: bool,
    /// `DS_QUERY_KEY_WRONG`: 0 = key ok. This twin always reports 0 (it models
    /// plaintext params; there is no real key to be "wrong"). Round-tripped for
    /// completeness but never set to a fault value.
    key_wrong: u32,
    /// `DS_QUERY_CHECK`: 0 = padding + digest OK. Set 0 after a successful
    /// modexp (see module docs — we model the integrity check as always
    /// passing for plaintext params).
    check: u32,

    date: u32,
}

impl std::fmt::Debug for Esp32s3Ds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32s3Ds(source_id={}, exponent={}, busy={}, started={}, key_wrong={}, check={})",
            self.source_id, self.exponent, self.busy, self.started, self.key_wrong, self.check,
        )
    }
}

impl Esp32s3Ds {
    /// Construct a DS peripheral. `source_id` is accepted for API symmetry with
    /// the other S3 peripherals but is never emitted (the DS block is polled,
    /// not interrupt-driven — register it on the bus with source `None`).
    pub fn new(source_id: u32) -> Self {
        Self {
            source_id,
            y_mem: vec![0; BLOCK_WORDS],
            m_mem: vec![0; BLOCK_WORDS],
            rb_mem: vec![0; BLOCK_WORDS],
            box_mem: vec![0; BLOCK_WORDS],
            z_mem: vec![0; BLOCK_WORDS],
            exponent: DEFAULT_EXPONENT,
            busy: false,
            started: false,
            key_wrong: 0,
            check: 0,
            date: DS_DATE_RESET,
        }
    }

    /// Mutable handle to the word-vector for a memory window, given an offset.
    /// Z is read-only so it is not returned here. Returns the relative word
    /// index alongside the vector.
    fn block_word_mut(&mut self, offset: u64) -> Option<(&mut Vec<u32>, usize)> {
        let (base, vec): (u64, &mut Vec<u32>) = match offset {
            o if o < OFF_M_MEM => (OFF_Y_MEM, &mut self.y_mem),
            o if o < OFF_RB_MEM => (OFF_M_MEM, &mut self.m_mem),
            o if o < OFF_BOX_MEM => (OFF_RB_MEM, &mut self.rb_mem),
            o if o < OFF_Z_MEM => (OFF_BOX_MEM, &mut self.box_mem),
            // Z window is read-only; writes are ignored.
            _ => return None,
        };
        let word = ((offset - base) / 4) as usize;
        if word < BLOCK_WORDS {
            Some((vec, word))
        } else {
            None
        }
    }

    /// Read-only handle for the same window lookup (covers Z too).
    fn block_word(&self, offset: u64) -> Option<u32> {
        let (base, vec): (u64, &Vec<u32>) = match offset {
            o if o < OFF_M_MEM => (OFF_Y_MEM, &self.y_mem),
            o if o < OFF_RB_MEM => (OFF_M_MEM, &self.m_mem),
            o if o < OFF_BOX_MEM => (OFF_RB_MEM, &self.rb_mem),
            o if o < OFF_Z_MEM => (OFF_BOX_MEM, &self.box_mem),
            o if o < MEM_REGION_END => (OFF_Z_MEM, &self.z_mem),
            _ => return None,
        };
        let word = ((offset - base) / 4) as usize;
        vec.get(word).copied()
    }

    /// Significant operand length in words, derived from the modulus `M` (the
    /// number of non-zero limbs, at least 1). This keeps the modexp tight even
    /// though firmware loads full 512-byte windows.
    fn words(&self) -> usize {
        let n = self
            .m_mem
            .iter()
            .rposition(|&w| w != 0)
            .map(|i| i + 1)
            .unwrap_or(1);
        n.min(BLOCK_WORDS)
    }

    /// Compute the signature `Z = Y^e mod M` and latch it into the Z window.
    ///
    /// Models the DS RSA exponentiation with plaintext params (see module
    /// docs). On completion `key_wrong` and `check` both read 0 (ok).
    fn compute_signature(&mut self) {
        let n = self.words();
        let y = BigUint::from_le_words(&self.y_mem[..n]);
        let m = BigUint::from_le_words(&self.m_mem[..n]);
        let z = if m.is_zero() {
            // No modulus loaded → degenerate; emit zero rather than panic.
            BigUint::zero()
        } else {
            let e = BigUint::from_le_words(&[self.exponent]);
            y.modpow(&e, &m)
        };
        self.store_z(&z, n);
        // Plaintext-param model: integrity + key checks always pass.
        self.key_wrong = 0;
        self.check = 0;
    }

    /// Write `z` little-endian into the Z window, zero-padded to `n` limbs.
    fn store_z(&mut self, z: &BigUint, n: usize) {
        for word in self.z_mem.iter_mut() {
            *word = 0;
        }
        let limbs = z.to_le_words();
        for (i, limb) in limbs.iter().enumerate() {
            if i < n && i < BLOCK_WORDS {
                self.z_mem[i] = *limb;
            }
        }
    }
}

impl Peripheral for Esp32s3Ds {
    fn read(&self, offset: u64) -> SimResult<u8> {
        // Read the containing 32-bit register, then slice the requested byte.
        let reg_base = offset & !0x3;
        let byte = (offset & 0x3) as u32;
        let word = self.read_reg(reg_base);
        Ok(((word >> (byte * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // Byte writes accumulate into the underlying word; we read-modify-write
        // the aligned register, then dispatch on the *aligned* address so a
        // triggering register only fires once the whole word is assembled.
        let reg_base = offset & !0x3;
        let byte = (offset & 0x3) as u32;
        let mut word = self.read_reg(reg_base);
        word &= !(0xFFu32 << (byte * 8));
        word |= (value as u32) << (byte * 8);
        self.write_reg(reg_base, word);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !0x3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !0x3, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Polled engine: a START/CONTINUE pulse leaves `busy = 1`; on the next
        // tick we run the modexp and clear busy so the firmware's
        // `while (QUERY_BUSY) {}` loop terminates. No interrupt source is
        // emitted (the S3 DS has none — see module docs).
        if self.busy {
            self.compute_signature();
            self.busy = false;
        }
        PeripheralTickResult::default()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl Esp32s3Ds {
    /// Read the 32-bit register at an aligned offset.
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            // Start triggers read back 0 (write-trigger registers).
            OFF_SET_START | OFF_SET_CONTINUE | OFF_SET_FINISH => 0,
            OFF_QUERY_BUSY => self.busy as u32,
            OFF_QUERY_KEY_WRONG => self.key_wrong,
            OFF_QUERY_CHECK => self.check,
            OFF_DATE => self.date,
            // Operand / result memory windows.
            o if o < MEM_REGION_END => self.block_word(o).unwrap_or(0),
            _ => 0,
        }
    }

    /// Write the 32-bit register at an aligned offset, dispatching triggers.
    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            OFF_SET_START
                if value & 1 != 0 => {
                    // Arm the engine and assert busy; the modexp runs on the
                    // next tick. Reset stale result/status.
                    self.started = true;
                    self.busy = true;
                    self.check = 0;
                    self.key_wrong = 0;
                    for w in self.z_mem.iter_mut() {
                        *w = 0;
                    }
                }
            OFF_SET_CONTINUE
                // Operands now loaded → run. (Also the legacy SET_ME activate
                // alias; treated identically.) Only meaningful after START.
                if value & 1 != 0 && self.started => {
                    self.busy = true;
                }
            OFF_SET_FINISH
                if value & 1 != 0 => {
                    // Return the engine to idle; the signature stays readable.
                    self.started = false;
                    self.busy = false;
                }
            OFF_QUERY_BUSY | OFF_QUERY_KEY_WRONG | OFF_QUERY_CHECK => {} // RO
            OFF_DATE => self.date = value,
            // Operand windows (Y/M/RB/BOX); Z is read-only (ignored by
            // `block_word_mut`).
            o if o < MEM_REGION_END => {
                if let Some((vec, word)) = self.block_word_mut(o) {
                    vec[word] = value;
                }
            }
            _ => {}
        }
    }

    /// Override the modeled public exponent used by `Z = Y^e mod M`. Exposed
    /// for tests / integrators that want to model a specific key pair; firmware
    /// itself never writes this (the real exponent is inside the encrypted
    /// param box, which this twin does not decrypt — see module docs).
    pub fn set_exponent(&mut self, e: u32) {
        self.exponent = e;
    }
}

// ---------------------------------------------------------------------------
// Self-contained arbitrary-precision unsigned integer.
//
// Kept private to this module (copied from the RSA twin) so the DS twin has no
// cross-module coupling and adds no external crate dependency. Limbs are
// little-endian base-2^32 `u32` words, matching the DS operand-window layout
// exactly, so assembly/disassembly is trivial.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
struct BigUint {
    /// Little-endian base-2^32 limbs, no trailing zero limbs (canonical).
    limbs: Vec<u32>,
}

impl BigUint {
    fn zero() -> Self {
        BigUint { limbs: Vec::new() }
    }

    fn one() -> Self {
        BigUint { limbs: vec![1] }
    }

    fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    /// Build from little-endian `u32` words (word[0] = least significant).
    fn from_le_words(words: &[u32]) -> Self {
        let mut limbs = words.to_vec();
        Self::trim(&mut limbs);
        BigUint { limbs }
    }

    /// Emit canonical little-endian `u32` limbs.
    fn to_le_words(&self) -> Vec<u32> {
        self.limbs.clone()
    }

    fn trim(limbs: &mut Vec<u32>) {
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
    }

    /// Number of significant bits.
    fn bits(&self) -> usize {
        match self.limbs.last() {
            None => 0,
            Some(&top) => (self.limbs.len() - 1) * 32 + (32 - top.leading_zeros() as usize),
        }
    }

    fn bit(&self, i: usize) -> bool {
        let limb = i / 32;
        let off = i % 32;
        self.limbs.get(limb).is_some_and(|w| (w >> off) & 1 == 1)
    }

    /// Schoolbook multiply.
    fn mul(&self, other: &BigUint) -> BigUint {
        if self.is_zero() || other.is_zero() {
            return BigUint::zero();
        }
        let mut out = vec![0u32; self.limbs.len() + other.limbs.len()];
        for (i, &a) in self.limbs.iter().enumerate() {
            let mut carry: u64 = 0;
            for (j, &b) in other.limbs.iter().enumerate() {
                let cur = out[i + j] as u64 + (a as u64) * (b as u64) + carry;
                out[i + j] = cur as u32;
                carry = cur >> 32;
            }
            let mut k = i + other.limbs.len();
            while carry != 0 {
                let cur = out[k] as u64 + carry;
                out[k] = cur as u32;
                carry = cur >> 32;
                k += 1;
            }
        }
        Self::trim(&mut out);
        BigUint { limbs: out }
    }

    /// Compare magnitudes.
    fn cmp(&self, other: &BigUint) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        if self.limbs.len() != other.limbs.len() {
            return self.limbs.len().cmp(&other.limbs.len());
        }
        for i in (0..self.limbs.len()).rev() {
            match self.limbs[i].cmp(&other.limbs[i]) {
                Ordering::Equal => continue,
                ord => return ord,
            }
        }
        Ordering::Equal
    }

    /// `self << bits` (bit shift).
    fn shl(&self, bits: usize) -> BigUint {
        if self.is_zero() {
            return BigUint::zero();
        }
        let word_shift = bits / 32;
        let bit_shift = bits % 32;
        let mut out = vec![0u32; self.limbs.len() + word_shift + 1];
        for (i, &w) in self.limbs.iter().enumerate() {
            let v = (w as u64) << bit_shift;
            out[i + word_shift] |= v as u32;
            out[i + word_shift + 1] |= (v >> 32) as u32;
        }
        Self::trim(&mut out);
        BigUint { limbs: out }
    }

    /// In-place subtract `other` from `self` (requires `self >= other`).
    fn sub_assign(&mut self, other: &BigUint) {
        let mut borrow: i64 = 0;
        for i in 0..self.limbs.len() {
            let b = *other.limbs.get(i).unwrap_or(&0) as i64;
            let cur = self.limbs[i] as i64 - b - borrow;
            if cur < 0 {
                self.limbs[i] = (cur + (1i64 << 32)) as u32;
                borrow = 1;
            } else {
                self.limbs[i] = cur as u32;
                borrow = 0;
            }
        }
        Self::trim(&mut self.limbs);
    }

    /// `self mod modulus` via shift-and-subtract long division. Modulus must be
    /// non-zero.
    fn rem(&self, modulus: &BigUint) -> BigUint {
        use std::cmp::Ordering;
        assert!(!modulus.is_zero(), "DS modulus is zero");
        if self.cmp(modulus) == Ordering::Less {
            return self.clone();
        }
        let mut rem = self.clone();
        let mod_bits = modulus.bits();
        loop {
            if rem.cmp(modulus) == Ordering::Less {
                break;
            }
            let shift = rem.bits() - mod_bits;
            // Try the largest shift; if the shifted modulus exceeds rem, back
            // off by one bit.
            let mut shifted = modulus.shl(shift);
            if shifted.cmp(&rem) == Ordering::Greater {
                shifted = modulus.shl(shift - 1);
            }
            rem.sub_assign(&shifted);
        }
        rem
    }

    /// `(self * other) mod modulus`.
    fn mulmod(&self, other: &BigUint, modulus: &BigUint) -> BigUint {
        self.mul(other).rem(modulus)
    }

    /// Modular exponentiation `self^exp mod modulus` (square-and-multiply).
    fn modpow(&self, exp: &BigUint, modulus: &BigUint) -> BigUint {
        assert!(!modulus.is_zero(), "DS modulus is zero");
        // mod 1 == 0 for every base/exponent.
        if modulus.cmp(&BigUint::one()) == std::cmp::Ordering::Equal {
            return BigUint::zero();
        }
        if exp.is_zero() {
            return BigUint::one();
        }
        let mut result = BigUint::one();
        let mut base = self.rem(modulus);
        let nbits = exp.bits();
        for i in 0..nbits {
            if exp.bit(i) {
                result = result.mulmod(&base, modulus);
            }
            if i + 1 < nbits {
                base = base.mulmod(&base, modulus);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Word-granular write of a 32-bit value at an aligned offset, mirroring
    /// the bus' four little-endian byte writes.
    fn wr(p: &mut Esp32s3Ds, off: u64, val: u32) {
        for b in 0..4 {
            p.write(off + b, ((val >> (b * 8)) & 0xFF) as u8).unwrap();
        }
    }

    /// Word-granular read reassembled from four little-endian byte reads.
    fn rd(p: &Esp32s3Ds, off: u64) -> u32 {
        let mut v = 0u32;
        for b in 0..4 {
            v |= (p.read(off + b).unwrap() as u32) << (b * 8);
        }
        v
    }

    fn load_operand(p: &mut Esp32s3Ds, block_base: u64, words: &[u32]) {
        for (i, &w) in words.iter().enumerate() {
            wr(p, block_base + (i as u64) * 4, w);
        }
    }

    /// Full sign sequence over a small known vector: load Y/M, set the modeled
    /// exponent, START → busy=1 → tick → busy=0, then Z holds Y^e mod M and the
    /// padding/digest check reads ok.
    #[test]
    fn sign_known_vector_busy_clears_on_tick() {
        let mut p = Esp32s3Ds::new(0);
        // Z = 4^3 mod 497 = 64. Use exponent 3 for a tiny, checkable vector.
        p.set_exponent(3);
        load_operand(&mut p, OFF_Y_MEM, &[4]);
        load_operand(&mut p, OFF_M_MEM, &[497]);

        // Idle before start.
        assert_eq!(rd(&p, OFF_QUERY_BUSY), 0, "idle before start");

        wr(&mut p, OFF_SET_START, 1);
        assert_eq!(rd(&p, OFF_QUERY_BUSY), 1, "busy latched right after START");
        // Result not yet computed while busy.
        assert_eq!(rd(&p, OFF_Z_MEM), 0, "Z not ready while busy");

        // One tick completes the modexp and clears busy.
        let res = p.tick();
        assert!(res.explicit_irqs.is_none(), "DS emits no interrupt source");
        assert_eq!(rd(&p, OFF_QUERY_BUSY), 0, "busy clears after a tick");

        // 4^3 mod 497 = 64.
        assert_eq!(rd(&p, OFF_Z_MEM), 64, "Z = Y^e mod M");
        // Padding/digest + key checks report ok.
        assert_eq!(rd(&p, OFF_QUERY_CHECK), 0, "padding/digest check ok");
        assert_eq!(rd(&p, OFF_QUERY_KEY_WRONG), 0, "key ok");
    }

    /// Classic textbook RSA modexp 4^13 mod 497 = 445, driven via the default
    /// public exponent override.
    #[test]
    fn sign_textbook_rsa_vector() {
        let mut p = Esp32s3Ds::new(0);
        p.set_exponent(13);
        load_operand(&mut p, OFF_Y_MEM, &[4]);
        load_operand(&mut p, OFF_M_MEM, &[497]);
        wr(&mut p, OFF_SET_START, 1);
        p.tick();
        assert_eq!(rd(&p, OFF_Z_MEM), 445);
    }

    /// Multi-limb (64-bit) signature spanning more than one result word,
    /// cross-checked against the in-house BigUint reference.
    #[test]
    fn sign_multiword_result() {
        let y = BigUint::from_le_words(&[0x0000_0007, 0x0000_0001]);
        let e = BigUint::from_le_words(&[65537]);
        let m = BigUint::from_le_words(&[0xFFFF_FFC5, 0xFFFF_FFFB]);
        let expect = y.modpow(&e, &m).to_le_words();

        let mut p = Esp32s3Ds::new(0);
        // Default exponent is 65537; no override needed.
        load_operand(&mut p, OFF_Y_MEM, &[0x0000_0007, 0x0000_0001]);
        load_operand(&mut p, OFF_M_MEM, &[0xFFFF_FFC5, 0xFFFF_FFFB]);
        wr(&mut p, OFF_SET_START, 1);
        p.tick();

        let z0 = rd(&p, OFF_Z_MEM);
        let z1 = rd(&p, OFF_Z_MEM + 4);
        assert_eq!(z0, *expect.first().unwrap_or(&0));
        assert_eq!(z1, *expect.get(1).unwrap_or(&0));
    }

    /// SET_CONTINUE after START re-arms busy (operands-loaded run pulse), and
    /// SET_FINISH returns the engine to idle while the signature stays readable.
    #[test]
    fn continue_and_finish_sequence() {
        let mut p = Esp32s3Ds::new(0);
        p.set_exponent(3);
        load_operand(&mut p, OFF_Y_MEM, &[5]);
        load_operand(&mut p, OFF_M_MEM, &[1000]);

        wr(&mut p, OFF_SET_START, 1);
        p.tick(); // 5^3 mod 1000 = 125
        assert_eq!(rd(&p, OFF_Z_MEM), 125);

        // CONTINUE pulse re-runs; busy asserts then clears on tick.
        wr(&mut p, OFF_SET_CONTINUE, 1);
        assert_eq!(rd(&p, OFF_QUERY_BUSY), 1, "CONTINUE re-arms busy");
        p.tick();
        assert_eq!(rd(&p, OFF_QUERY_BUSY), 0);

        // FINISH returns to idle; signature is still readable.
        wr(&mut p, OFF_SET_FINISH, 1);
        assert_eq!(rd(&p, OFF_QUERY_BUSY), 0, "idle after finish");
        assert_eq!(rd(&p, OFF_Z_MEM), 125, "result persists after finish");
    }

    /// CONTINUE before START is ignored (engine not armed).
    #[test]
    fn continue_without_start_is_inert() {
        let mut p = Esp32s3Ds::new(0);
        wr(&mut p, OFF_SET_CONTINUE, 1);
        assert_eq!(rd(&p, OFF_QUERY_BUSY), 0, "no busy without prior START");
    }

    /// Tick with no pending operation is a no-op (busy stays 0, no IRQ).
    #[test]
    fn idle_tick_is_noop() {
        let mut p = Esp32s3Ds::new(99);
        let res = p.tick();
        assert_eq!(rd(&p, OFF_QUERY_BUSY), 0);
        assert!(res.explicit_irqs.is_none());
        assert!(!res.irq);
    }

    /// All four operand windows + the DATE register round-trip; the Z window is
    /// read-only (firmware writes are dropped).
    #[test]
    fn memory_and_control_round_trip() {
        let mut p = Esp32s3Ds::new(0);

        wr(&mut p, OFF_Y_MEM, 0x1111_2222);
        wr(&mut p, OFF_Y_MEM + 4, 0x3333_4444);
        wr(&mut p, OFF_M_MEM, 0x5555_6666);
        wr(&mut p, OFF_RB_MEM, 0x7777_8888);
        wr(&mut p, OFF_BOX_MEM, 0x9999_AAAA);
        assert_eq!(rd(&p, OFF_Y_MEM), 0x1111_2222);
        assert_eq!(rd(&p, OFF_Y_MEM + 4), 0x3333_4444);
        assert_eq!(rd(&p, OFF_M_MEM), 0x5555_6666);
        assert_eq!(rd(&p, OFF_RB_MEM), 0x7777_8888);
        assert_eq!(rd(&p, OFF_BOX_MEM), 0x9999_AAAA);

        // DATE round-trips and resets to the documented value.
        assert_eq!(rd(&p, OFF_DATE), DS_DATE_RESET);
        wr(&mut p, OFF_DATE, 0x1234_5678);
        assert_eq!(rd(&p, OFF_DATE), 0x1234_5678);

        // Z window is read-only: writes are ignored.
        wr(&mut p, OFF_Z_MEM, 0xDEAD_BEEF);
        assert_eq!(rd(&p, OFF_Z_MEM), 0, "Z is read-only");
    }

    /// The u32 read/write fast path agrees with the byte path.
    #[test]
    fn u32_path_matches_byte_path() {
        let mut p = Esp32s3Ds::new(0);
        p.write_u32(OFF_M_MEM, 0xCAFE_F00D).unwrap();
        assert_eq!(p.read_u32(OFF_M_MEM).unwrap(), 0xCAFE_F00D);
        assert_eq!(rd(&p, OFF_M_MEM), 0xCAFE_F00D, "byte read sees same value");
    }

    /// START clears any stale result before re-running.
    #[test]
    fn start_clears_stale_result() {
        let mut p = Esp32s3Ds::new(0);
        p.set_exponent(2);
        load_operand(&mut p, OFF_Y_MEM, &[7]);
        load_operand(&mut p, OFF_M_MEM, &[1000]);
        wr(&mut p, OFF_SET_START, 1);
        p.tick();
        assert_eq!(rd(&p, OFF_Z_MEM), 49); // 7^2 mod 1000

        // New operands; START must clear old Z while busy.
        load_operand(&mut p, OFF_Y_MEM, &[8]);
        wr(&mut p, OFF_SET_START, 1);
        assert_eq!(rd(&p, OFF_Z_MEM), 0, "stale result cleared at START");
        p.tick();
        assert_eq!(rd(&p, OFF_Z_MEM), 64); // 8^2 mod 1000
    }

    /// Cross-check the in-house BigUint modpow against u128 ground truth.
    #[test]
    fn bignum_modpow_matches_reference() {
        let base = BigUint::from_le_words(&[7]);
        let exp = BigUint::from_le_words(&[13]);
        let m = BigUint::from_le_words(&[101]);
        let got = base.modpow(&exp, &m).to_le_words();
        let expect = {
            let mut r: u128 = 1;
            for _ in 0..13 {
                r = (r * 7) % 101;
            }
            r as u32
        };
        assert_eq!(got.first().copied().unwrap_or(0), expect);
    }
}
