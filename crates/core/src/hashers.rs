// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Tiny, dependency-free hashers for hot integer-keyed maps.
//!
//! `std`'s default [`HashMap`](std::collections::HashMap) hasher is
//! SipHash-1-3: DoS-resistant, but ~expensive per lookup. On simulator hot
//! paths that key a map by a small unique integer (guest PCs, block ids, …)
//! the DoS resistance buys nothing and the SipHash cost dominates. This module
//! provides an [`FxBuildHasher`] — an FxHash-style multiply-mix over the raw
//! integer bytes — plus an [`FxHashMap`] alias, so any such map can opt out of
//! SipHash without pulling in a third-party crate.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// Multiplicative mixing constant (Firefox's FxHash 64-bit constant).
const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

/// FxHash-style hasher: fold each written chunk into the accumulator with a
/// rotate + multiply. Fast, non-cryptographic; intended for maps keyed by
/// small unique integers (e.g. guest program counters).
#[derive(Default)]
pub struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        // rotate-left then xor-in, then multiply — the classic FxHash step.
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.add(u64::from_le_bytes(chunk.try_into().unwrap()));
        }
        let rem = chunks.remainder();
        if !rem.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rem.len()].copy_from_slice(rem);
            self.add(u64::from_le_bytes(buf));
        }
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
}

/// [`BuildHasher`](std::hash::BuildHasher) producing [`FxHasher`]s.
pub type FxBuildHasher = BuildHasherDefault<FxHasher>;

/// A [`HashMap`] that uses [`FxHasher`] instead of SipHash. Drop-in for maps
/// keyed by small unique integers on hot paths.
pub type FxHashMap<K, V> = HashMap<K, V, FxBuildHasher>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn behaves_as_a_map() {
        let mut m: FxHashMap<u64, u32> = FxHashMap::default();
        m.insert(0x4000_1000, 7);
        m.insert(0x4000_1004, 9);
        assert_eq!(m.get(&0x4000_1000), Some(&7));
        assert_eq!(m.get(&0x4000_1004), Some(&9));
        assert_eq!(m.get(&0x4000_2000), None);
        *m.get_mut(&0x4000_1000).unwrap() += 1;
        assert_eq!(m.get(&0x4000_1000), Some(&8));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn distinct_small_ints_do_not_collide_into_one_bucket_value() {
        // Sanity: many sequential PC-like keys all round-trip.
        let mut m: FxHashMap<u64, u64> = FxHashMap::default();
        for pc in (0x4000_0000u64..0x4000_0400).step_by(2) {
            m.insert(pc, pc ^ 0xAB);
        }
        for pc in (0x4000_0000u64..0x4000_0400).step_by(2) {
            assert_eq!(m.get(&pc), Some(&(pc ^ 0xAB)));
        }
    }
}
