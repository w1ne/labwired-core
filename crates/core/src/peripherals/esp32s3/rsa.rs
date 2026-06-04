// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RSA accelerator (big-number Multiple-Precision-Integer engine) for the
//! ESP32-S3.
//!
//! The S3 RSA peripheral performs three big-integer primitives on operands of
//! up to 4096 bits:
//!
//! * **MODEXP**  — `Z = X^Y mod M`            (RSA exponentiation)
//! * **MODMULT** — `Z = (X * Y) mod M`        (Montgomery modular multiply)
//! * **MULT**    — `Z = X * Y`                (plain double-length multiply)
//!
//! This is a *functionally-exact* digital twin: the firmware sees the same
//! register interface and reads back the same `Z` result that real silicon
//! would produce. We compute the math directly on arbitrary-precision
//! unsigned integers, so the result is identical to the hardware's.
//!
//! ## What we deliberately ignore
//!
//! Real hardware uses the Montgomery method and therefore *requires* the
//! firmware to pre-load two helper values:
//!
//! * `M'` (a.k.a. `M_DASH`, "M-prime") — `-M^-1 mod 2^32`, and
//! * `r^2 mod M` — loaded into the Z block before a MODEXP/MODMULT.
//!
//! Those exist purely to make the *Montgomery* reduction work on silicon.
//! Because we reduce with a direct schoolbook `mod`, they have no effect on
//! the answer. We still round-trip `M_DASH` (firmware writes and may read it
//! back), but we never consult it for the computation. The result is
//! bit-for-bit identical to what the Montgomery hardware emits.
//!
//! ## Register layout (ESP32-S3 TRM §22; `soc/esp32s3/include/soc/hwcrypto_reg.h`)
//!
//! `DR_REG_RSA_BASE = 0x6003_C000`. Offsets are relative to the base.
//!
//! | Offset | Name                  | Dir | Behaviour |
//! |-------:|-----------------------|-----|-----------|
//! | 0x000  | `RSA_MEM_M_BLOCK`     | R/W | M operand words (128 words, 512 B) |
//! | 0x200  | `RSA_MEM_Z_BLOCK`     | R/W | Z result words (also "RB" during a phase) |
//! | 0x400  | `RSA_MEM_Y_BLOCK`     | R/W | Y operand words |
//! | 0x600  | `RSA_MEM_X_BLOCK`     | R/W | X operand words |
//! | 0x800  | `RSA_M_DASH`          | R/W | M' (round-tripped, ignored by math) |
//! | 0x804  | `RSA_LENGTH` (MODE)   | R/W | operand length in words minus 1 |
//! | 0x808  | `RSA_QUERY_CLEAN`     | RO  | 1 when the engine has finished its reset/clean |
//! | 0x80C  | `RSA_MODEXP_START`    | WT  | write 1 → run `Z = X^Y mod M` |
//! | 0x810  | `RSA_MOD_MULT_START`  | WT  | write 1 → run `Z = X*Y mod M` |
//! | 0x814  | `RSA_MULT_START`      | WT  | write 1 → run `Z = X*Y` (double length) |
//! | 0x818  | `RSA_QUERY_INTERRUPT` | RO  | 1 when an operation has completed (idle/done) |
//! | 0x81C  | `RSA_CLEAR_INTERRUPT` | WT  | write 1 → clear the done/idle flag (W1C) |
//! | 0x820  | `RSA_CONSTANT_TIME`   | R/W | acceleration option (round-tripped, default 1) |
//! | 0x824  | `RSA_SEARCH_OPEN`     | R/W | acceleration option (round-tripped) |
//! | 0x828  | `RSA_SEARCH_POS`      | R/W | acceleration option (round-tripped) |
//! | 0x82C  | `RSA_INT_ENA`         | R/W | done-interrupt enable |
//!
//! The four operand blocks are each `0x200` bytes apart (512 B = 128 words),
//! enough for a 4096-bit operand. The TRM names the `0x818` register
//! "QUERY_INTERRUPT" but esp-idf polls it as the *idle/done* flag — when it
//! reads 1, the operation has finished. We model both meanings with one
//! sticky `done` bit: a completed op sets it, and reading 0x818 returns it.
//!
//! ## Little-endian word assembly convention
//!
//! Operand block word `i` (the 32-bit value at block-offset `i*4`) is the
//! `i`-th least-significant limb of the big integer. So the integer is
//! `sum(word[i] << (32*i))` for `i` in `0..words`, where `words = MODE + 1`.
//! Each 32-bit word is itself stored little-endian in MMIO bytes (matching
//! the bus' byte-granular `read`/`write`). `Z` is written back the same way,
//! zero-padded to `words` limbs (or `2*words` for plain MULT).
//!
//! ## Interrupt
//!
//! When an operation completes it latches the done flag. While the done flag
//! is set *and* `RSA_INT_ENA` bit 0 is set, `tick()` emits the RSA
//! interrupt-matrix source (`ETS_RSA_INTR_SOURCE = 76`) via
//! `PeripheralTickResult.explicit_irqs` on every tick — the level-sensitive
//! "emit source while int asserts" convention shared with SYSTIMER. The
//! firmware clears it by writing `RSA_CLEAR_INTERRUPT` (W1C).

use crate::{Peripheral, PeripheralTickResult, SimResult};

/// Bytes per operand memory block (512 B = 128 × 32-bit words = 4096-bit max).
const BLOCK_SIZE: u64 = 0x200;
/// Number of 32-bit words per operand block.
const BLOCK_WORDS: usize = (BLOCK_SIZE / 4) as usize;

// --- operand block base offsets (relative to DR_REG_RSA_BASE) ---
const OFF_M_MEM: u64 = 0x000;
const OFF_Z_MEM: u64 = 0x200;
const OFF_Y_MEM: u64 = 0x400;
const OFF_X_MEM: u64 = 0x600;

// --- control / status registers ---
const OFF_M_DASH: u64 = 0x800;
const OFF_LENGTH: u64 = 0x804; // a.k.a. RSA_MODE: words - 1
const OFF_QUERY_CLEAN: u64 = 0x808;
const OFF_MODEXP_START: u64 = 0x80C;
const OFF_MODMULT_START: u64 = 0x810;
const OFF_MULT_START: u64 = 0x814;
const OFF_QUERY_INTERRUPT: u64 = 0x818; // idle/done query (RO)
const OFF_CLEAR_INTERRUPT: u64 = 0x81C; // W1C done/int clear
const OFF_CONSTANT_TIME: u64 = 0x820;
const OFF_SEARCH_OPEN: u64 = 0x824;
const OFF_SEARCH_POS: u64 = 0x828;
const OFF_INT_ENA: u64 = 0x82C;

/// `ETS_RSA_INTR_SOURCE` from `soc/esp32s3/include/soc/interrupts.h`
/// (enum value 76; AES = 77, SHA = 78 follow). Verified by walking the enum
/// honouring its explicit `= N` assignments.
pub const ETS_RSA_INTR_SOURCE: u32 = 76;

/// ESP32-S3 RSA accelerator twin.
pub struct Esp32s3Rsa {
    /// Interrupt-matrix source id this peripheral emits (default 76).
    source_id: u32,

    /// Operand blocks, each `BLOCK_WORDS` little-endian limbs.
    x_mem: Vec<u32>,
    y_mem: Vec<u32>,
    m_mem: Vec<u32>,
    z_mem: Vec<u32>,

    /// Control registers (round-tripped).
    m_dash: u32,
    /// RSA_LENGTH / RSA_MODE: operand length in words minus one.
    mode: u32,
    constant_time: u32,
    search_open: u32,
    search_pos: u32,

    /// Done-interrupt enable (RSA_INT_ENA bit 0).
    int_ena: bool,
    /// Sticky done / idle flag. Set when an operation completes; read back via
    /// RSA_QUERY_INTERRUPT (0x818); cleared by RSA_CLEAR_INTERRUPT (0x81C).
    done: bool,
}

impl std::fmt::Debug for Esp32s3Rsa {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32s3Rsa(source_id={}, mode={}, int_ena={}, done={})",
            self.source_id, self.mode, self.int_ena, self.done,
        )
    }
}

impl Esp32s3Rsa {
    /// Construct an RSA accelerator that raises interrupt-matrix source
    /// `source_id` (pass `ETS_RSA_INTR_SOURCE` for the real S3 wiring).
    pub fn new(source_id: u32) -> Self {
        Self {
            source_id,
            x_mem: vec![0; BLOCK_WORDS],
            y_mem: vec![0; BLOCK_WORDS],
            m_mem: vec![0; BLOCK_WORDS],
            z_mem: vec![0; BLOCK_WORDS],
            m_dash: 0,
            mode: 0,
            // CONSTANT_TIME resets to 1 on real silicon.
            constant_time: 1,
            search_open: 0,
            search_pos: 0,
            int_ena: false,
            done: false,
        }
    }

    /// Operand length in 32-bit words (`MODE + 1`).
    fn words(&self) -> usize {
        (self.mode as usize + 1).min(BLOCK_WORDS)
    }

    /// Mutable handle to the word-vector for an operand block, given the
    /// block's base offset. Returns the relative word index too.
    fn block_word_mut(&mut self, offset: u64) -> Option<(&mut Vec<u32>, usize)> {
        let (base, vec): (u64, &mut Vec<u32>) = match offset {
            o if o < OFF_Z_MEM => (OFF_M_MEM, &mut self.m_mem),
            o if o < OFF_Y_MEM => (OFF_Z_MEM, &mut self.z_mem),
            o if o < OFF_X_MEM => (OFF_Y_MEM, &mut self.y_mem),
            o if o < OFF_M_DASH => (OFF_X_MEM, &mut self.x_mem),
            _ => return None,
        };
        let word = ((offset - base) / 4) as usize;
        if word < BLOCK_WORDS {
            Some((vec, word))
        } else {
            None
        }
    }

    /// Read-only handle for the same block lookup.
    fn block_word(&self, offset: u64) -> Option<u32> {
        let (base, vec): (u64, &Vec<u32>) = match offset {
            o if o < OFF_Z_MEM => (OFF_M_MEM, &self.m_mem),
            o if o < OFF_Y_MEM => (OFF_Z_MEM, &self.z_mem),
            o if o < OFF_X_MEM => (OFF_Y_MEM, &self.y_mem),
            o if o < OFF_M_DASH => (OFF_X_MEM, &self.x_mem),
            _ => return None,
        };
        let word = ((offset - base) / 4) as usize;
        vec.get(word).copied()
    }

    /// Run `Z = X^Y mod M` and latch completion.
    fn run_modexp(&mut self) {
        let n = self.words();
        let x = BigUint::from_le_words(&self.x_mem[..n]);
        let y = BigUint::from_le_words(&self.y_mem[..n]);
        let m = BigUint::from_le_words(&self.m_mem[..n]);
        let z = x.modpow(&y, &m);
        self.store_z(&z, n);
        self.complete();
    }

    /// Run `Z = (X * Y) mod M` and latch completion.
    fn run_modmult(&mut self) {
        let n = self.words();
        let x = BigUint::from_le_words(&self.x_mem[..n]);
        let y = BigUint::from_le_words(&self.y_mem[..n]);
        let m = BigUint::from_le_words(&self.m_mem[..n]);
        let prod = x.mul(&y);
        let z = prod.rem(&m);
        self.store_z(&z, n);
        self.complete();
    }

    /// Run plain `Z = X * Y` (double-length result) and latch completion.
    fn run_mult(&mut self) {
        let n = self.words();
        let x = BigUint::from_le_words(&self.x_mem[..n]);
        let y = BigUint::from_le_words(&self.y_mem[..n]);
        let z = x.mul(&y);
        self.store_z(&z, (2 * n).min(BLOCK_WORDS));
        self.complete();
    }

    /// Write `z` little-endian into the Z block, zero-padded to `n` limbs.
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

    /// Latch the done/idle flag after an operation finishes.
    fn complete(&mut self) {
        self.done = true;
    }
}

impl Peripheral for Esp32s3Rsa {
    fn read(&self, offset: u64) -> SimResult<u8> {
        // Read the containing 32-bit register, then slice the requested byte.
        let reg_base = offset & !0x3;
        let byte = (offset & 0x3) as u32;
        let word = self.read_reg(reg_base);
        Ok(((word >> (byte * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // Byte writes accumulate into the underlying word. We read-modify-write
        // the register at the aligned base, then dispatch on the *aligned*
        // address so a triggering register only fires once the word is built.
        let reg_base = offset & !0x3;
        let byte = (offset & 0x3) as u32;
        let mut word = self.read_reg(reg_base);
        word &= !(0xFFu32 << (byte * 8));
        word |= (value as u32) << (byte * 8);
        self.write_reg(reg_base, word);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Level-sensitive: while the done flag is latched and the done
        // interrupt is enabled, re-emit the RSA source every tick so the
        // firmware sees a stable pending bit while its ISR iterates handlers.
        if self.done && self.int_ena {
            PeripheralTickResult {
                explicit_irqs: Some(vec![self.source_id]),
                ..PeripheralTickResult::default()
            }
        } else {
            PeripheralTickResult::default()
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl Esp32s3Rsa {
    /// Read the 32-bit register at an aligned offset.
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            OFF_M_DASH => self.m_dash,
            OFF_LENGTH => self.mode,
            // QUERY_CLEAN reads 1: the engine is always finished cleaning in
            // the twin (firmware polls this after the clean step).
            OFF_QUERY_CLEAN => 1,
            // Start triggers read back 0 (WT registers).
            OFF_MODEXP_START | OFF_MODMULT_START | OFF_MULT_START => 0,
            // QUERY_INTERRUPT doubles as the idle/done flag: 1 == finished.
            OFF_QUERY_INTERRUPT => self.done as u32,
            OFF_CLEAR_INTERRUPT => 0,
            OFF_CONSTANT_TIME => self.constant_time,
            OFF_SEARCH_OPEN => self.search_open,
            OFF_SEARCH_POS => self.search_pos,
            OFF_INT_ENA => self.int_ena as u32,
            // Operand memory blocks.
            o if o < OFF_M_DASH => self.block_word(o).unwrap_or(0),
            _ => 0,
        }
    }

    /// Write the 32-bit register at an aligned offset, dispatching triggers.
    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            OFF_M_DASH => self.m_dash = value,
            OFF_LENGTH => self.mode = value & 0x7F, // 7-bit field (0..127)
            OFF_QUERY_CLEAN => {}                   // RO
            OFF_MODEXP_START
                if value & 1 != 0 => {
                    self.run_modexp();
                }
            OFF_MODMULT_START
                if value & 1 != 0 => {
                    self.run_modmult();
                }
            OFF_MULT_START
                if value & 1 != 0 => {
                    self.run_mult();
                }
            OFF_QUERY_INTERRUPT => {} // RO
            OFF_CLEAR_INTERRUPT
                // W1C: writing 1 clears the done/idle + interrupt latch.
                if value & 1 != 0 => {
                    self.done = false;
                }
            OFF_CONSTANT_TIME => self.constant_time = value & 1,
            OFF_SEARCH_OPEN => self.search_open = value & 1,
            OFF_SEARCH_POS => self.search_pos = value & 0xFFF,
            OFF_INT_ENA => self.int_ena = value & 1 != 0,
            o if o < OFF_M_DASH => {
                if let Some((vec, word)) = self.block_word_mut(o) {
                    vec[word] = value;
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Self-contained arbitrary-precision unsigned integer.
//
// `num-bigint` is not a dependency of this crate (and Cargo.toml is off-limits
// for this task), so we provide just enough big-number math here — multiply,
// remainder, and modular exponentiation — to compute results that are
// bit-identical to `BigUint`. Limbs are little-endian `u32` words, matching
// the RSA operand-block layout exactly, so assembly/disassembly is trivial.
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

    /// `self mod modulus` via shift-and-subtract long division. Modulus must
    /// be non-zero.
    fn rem(&self, modulus: &BigUint) -> BigUint {
        use std::cmp::Ordering;
        assert!(!modulus.is_zero(), "RSA modulus is zero");
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
    /// Mirrors `num_bigint::BigUint::modpow`.
    fn modpow(&self, exp: &BigUint, modulus: &BigUint) -> BigUint {
        assert!(!modulus.is_zero(), "RSA modulus is zero");
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

    // Word-granular write of a 32-bit value at an aligned register offset,
    // mirroring the bus' four byte writes (little-endian).
    fn wr(p: &mut Esp32s3Rsa, off: u64, val: u32) {
        for b in 0..4 {
            p.write(off + b, ((val >> (b * 8)) & 0xFF) as u8).unwrap();
        }
    }

    fn rd(p: &Esp32s3Rsa, off: u64) -> u32 {
        let mut v = 0u32;
        for b in 0..4 {
            v |= (p.read(off + b).unwrap() as u32) << (b * 8);
        }
        v
    }

    /// Load a single-limb operand block (operand fits in one 32-bit word).
    fn load_operand(p: &mut Esp32s3Rsa, block_base: u64, words: &[u32]) {
        for (i, &w) in words.iter().enumerate() {
            wr(p, block_base + (i as u64) * 4, w);
        }
    }

    #[test]
    fn modexp_known_vector_4_pow_13_mod_497() {
        // Classic textbook RSA modexp: 4^13 mod 497 = 445.
        let mut p = Esp32s3Rsa::new(ETS_RSA_INTR_SOURCE);
        wr(&mut p, OFF_LENGTH, 0); // MODE = 0 → 1 word operands
        load_operand(&mut p, OFF_X_MEM, &[4]);
        load_operand(&mut p, OFF_Y_MEM, &[13]);
        load_operand(&mut p, OFF_M_MEM, &[497]);
        // Engine idle before start.
        assert_eq!(rd(&p, OFF_QUERY_INTERRUPT), 0);
        wr(&mut p, OFF_MODEXP_START, 1);
        // Done/idle latched.
        assert_eq!(rd(&p, OFF_QUERY_INTERRUPT), 1);
        assert_eq!(rd(&p, OFF_Z_MEM), 445);
    }

    #[test]
    fn modexp_multiword_rsa_style() {
        // Two-limb (64-bit) modexp. Modulus M = p*q with small primes,
        // exponent and base chosen so the result spans more than one limb.
        // X = 0x1_0000_0007, Y = 5, M = 0xFFFF_FFFB_FFFF_FFC5.
        // Verified result computed independently below via the same BigUint.
        let x = BigUint::from_le_words(&[0x0000_0007, 0x0000_0001]);
        let y = BigUint::from_le_words(&[5]);
        let m = BigUint::from_le_words(&[0xFFFF_FFC5, 0xFFFF_FFFB]);
        let expect = x.modpow(&y, &m).to_le_words();

        let mut p = Esp32s3Rsa::new(ETS_RSA_INTR_SOURCE);
        wr(&mut p, OFF_LENGTH, 1); // MODE = 1 → 2-word operands
        load_operand(&mut p, OFF_X_MEM, &[0x0000_0007, 0x0000_0001]);
        load_operand(&mut p, OFF_Y_MEM, &[5, 0]);
        load_operand(&mut p, OFF_M_MEM, &[0xFFFF_FFC5, 0xFFFF_FFFB]);
        wr(&mut p, OFF_MODEXP_START, 1);
        assert_eq!(rd(&p, OFF_QUERY_INTERRUPT), 1);
        // Z low/high limbs match the reference modpow.
        let z0 = rd(&p, OFF_Z_MEM);
        let z1 = rd(&p, OFF_Z_MEM + 4);
        assert_eq!(z0, *expect.first().unwrap_or(&0));
        assert_eq!(z1, *expect.get(1).unwrap_or(&0));
    }

    #[test]
    fn modmult_known_vector() {
        // (123456789 * 987654321) mod 1000000007.
        let mut p = Esp32s3Rsa::new(ETS_RSA_INTR_SOURCE);
        wr(&mut p, OFF_LENGTH, 0);
        load_operand(&mut p, OFF_X_MEM, &[123_456_789]);
        load_operand(&mut p, OFF_Y_MEM, &[987_654_321]);
        load_operand(&mut p, OFF_M_MEM, &[1_000_000_007]);
        wr(&mut p, OFF_MODMULT_START, 1);
        let expect = (123_456_789u128 * 987_654_321u128 % 1_000_000_007u128) as u32;
        assert_eq!(rd(&p, OFF_Z_MEM), expect);
    }

    #[test]
    fn plain_mult_double_length() {
        // 0xFFFF_FFFF * 0xFFFF_FFFF = 0xFFFF_FFFE_0000_0001 (two limbs).
        let mut p = Esp32s3Rsa::new(ETS_RSA_INTR_SOURCE);
        wr(&mut p, OFF_LENGTH, 0); // 1-word operands → 2-word result
        load_operand(&mut p, OFF_X_MEM, &[0xFFFF_FFFF]);
        load_operand(&mut p, OFF_Y_MEM, &[0xFFFF_FFFF]);
        wr(&mut p, OFF_MULT_START, 1);
        assert_eq!(rd(&p, OFF_Z_MEM), 0x0000_0001);
        assert_eq!(rd(&p, OFF_Z_MEM + 4), 0xFFFF_FFFE);
    }

    #[test]
    fn idle_done_flag_sets_on_completion() {
        let mut p = Esp32s3Rsa::new(ETS_RSA_INTR_SOURCE);
        wr(&mut p, OFF_LENGTH, 0);
        load_operand(&mut p, OFF_X_MEM, &[2]);
        load_operand(&mut p, OFF_Y_MEM, &[10]);
        load_operand(&mut p, OFF_M_MEM, &[1000]);
        assert_eq!(rd(&p, OFF_QUERY_INTERRUPT), 0, "idle before op");
        wr(&mut p, OFF_MODEXP_START, 1);
        assert_eq!(rd(&p, OFF_QUERY_INTERRUPT), 1, "done after op");
        assert_eq!(rd(&p, OFF_Z_MEM), 24); // 2^10 mod 1000 = 1024 mod 1000
    }

    #[test]
    fn clear_interrupt_is_w1c() {
        let mut p = Esp32s3Rsa::new(ETS_RSA_INTR_SOURCE);
        wr(&mut p, OFF_LENGTH, 0);
        load_operand(&mut p, OFF_X_MEM, &[4]);
        load_operand(&mut p, OFF_Y_MEM, &[13]);
        load_operand(&mut p, OFF_M_MEM, &[497]);
        wr(&mut p, OFF_MODEXP_START, 1);
        assert_eq!(rd(&p, OFF_QUERY_INTERRUPT), 1);
        // Writing 0 does nothing (W1C); writing 1 clears.
        wr(&mut p, OFF_CLEAR_INTERRUPT, 0);
        assert_eq!(rd(&p, OFF_QUERY_INTERRUPT), 1, "0 write must not clear");
        wr(&mut p, OFF_CLEAR_INTERRUPT, 1);
        assert_eq!(rd(&p, OFF_QUERY_INTERRUPT), 0, "1 write clears (W1C)");
    }

    #[test]
    fn interrupt_emits_source_while_done_and_enabled() {
        let mut p = Esp32s3Rsa::new(ETS_RSA_INTR_SOURCE);
        wr(&mut p, OFF_LENGTH, 0);
        load_operand(&mut p, OFF_X_MEM, &[4]);
        load_operand(&mut p, OFF_Y_MEM, &[13]);
        load_operand(&mut p, OFF_M_MEM, &[497]);

        // No interrupt while disabled.
        wr(&mut p, OFF_MODEXP_START, 1);
        assert!(p.tick().explicit_irqs.is_none(), "no IRQ while INT_ENA=0");

        // Enable: source emitted every tick while done is latched.
        wr(&mut p, OFF_INT_ENA, 1);
        assert_eq!(
            p.tick().explicit_irqs.as_deref(),
            Some(&[ETS_RSA_INTR_SOURCE][..]),
        );
        // Still asserting on the next tick (level-sensitive).
        assert_eq!(
            p.tick().explicit_irqs.as_deref(),
            Some(&[ETS_RSA_INTR_SOURCE][..]),
        );
        // Clear done → interrupt deasserts.
        wr(&mut p, OFF_CLEAR_INTERRUPT, 1);
        assert!(p.tick().explicit_irqs.is_none(), "deassert after clear");
    }

    #[test]
    fn control_registers_round_trip() {
        let mut p = Esp32s3Rsa::new(ETS_RSA_INTR_SOURCE);
        wr(&mut p, OFF_M_DASH, 0xDEAD_BEEF);
        assert_eq!(rd(&p, OFF_M_DASH), 0xDEAD_BEEF);
        wr(&mut p, OFF_LENGTH, 63);
        assert_eq!(rd(&p, OFF_LENGTH), 63);
        // CONSTANT_TIME resets to 1.
        assert_eq!(rd(&p, OFF_CONSTANT_TIME), 1);
        wr(&mut p, OFF_CONSTANT_TIME, 0);
        assert_eq!(rd(&p, OFF_CONSTANT_TIME), 0);
        wr(&mut p, OFF_SEARCH_OPEN, 1);
        assert_eq!(rd(&p, OFF_SEARCH_OPEN), 1);
        wr(&mut p, OFF_SEARCH_POS, 0xABC);
        assert_eq!(rd(&p, OFF_SEARCH_POS), 0xABC);
        // QUERY_CLEAN always reads ready.
        assert_eq!(rd(&p, OFF_QUERY_CLEAN), 1);
    }

    #[test]
    fn operand_memory_round_trips() {
        let mut p = Esp32s3Rsa::new(ETS_RSA_INTR_SOURCE);
        wr(&mut p, OFF_X_MEM, 0x1111_2222);
        wr(&mut p, OFF_X_MEM + 4, 0x3333_4444);
        wr(&mut p, OFF_Y_MEM, 0x5555_6666);
        wr(&mut p, OFF_M_MEM, 0x7777_8888);
        assert_eq!(rd(&p, OFF_X_MEM), 0x1111_2222);
        assert_eq!(rd(&p, OFF_X_MEM + 4), 0x3333_4444);
        assert_eq!(rd(&p, OFF_Y_MEM), 0x5555_6666);
        assert_eq!(rd(&p, OFF_M_MEM), 0x7777_8888);
    }

    #[test]
    fn bignum_matches_reference_arithmetic() {
        // Cross-check the in-house BigUint against u128 ground truth.
        let a = BigUint::from_le_words(&[0xFFFF_FFFF, 0x0000_00FF]);
        let b = BigUint::from_le_words(&[0x1234_5678]);
        let prod = a.mul(&b).to_le_words();
        let a128 = 0x0000_00FF_FFFF_FFFFu128;
        let b128 = 0x1234_5678u128;
        let expect = a128 * b128;
        let got = (prod.first().copied().unwrap_or(0) as u128)
            | ((prod.get(1).copied().unwrap_or(0) as u128) << 32)
            | ((prod.get(2).copied().unwrap_or(0) as u128) << 64);
        assert_eq!(got, expect);

        // rem
        let m = BigUint::from_le_words(&[1_000_003]);
        assert_eq!(
            a.rem(&m).to_le_words().first().copied().unwrap_or(0) as u128,
            a128 % 1_000_003u128,
        );
    }
}
