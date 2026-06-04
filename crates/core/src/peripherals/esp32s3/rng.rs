// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 hardware RNG data register (`WDEV_RND_REG`, `0x6003_507C`).
//!
//! On silicon this register yields a fresh 32-bit random value on every read,
//! sourced from on-chip entropy (RC fast clock jitter + the SAR ADC / RF
//! subsystems). The 2nd-stage bootloader's `bootloader_fill_random` reads it
//! repeatedly, XOR-mixing successive reads, and — for keys/nonces — *retries
//! until it gets a non-zero result*. A constant return value (what a generic
//! read-as-ones MMIO stub gives) makes that retry loop spin forever.
//!
//! We model it with a **deterministic** PRNG (xorshift32) so the firmware sees
//! varying, almost-always-non-zero entropy while the simulation stays fully
//! reproducible — the cornerstone of LabWired. The seed is fixed; the same
//! firmware run produces the same "random" stream every time.

use crate::{Peripheral, SimResult};
use std::sync::atomic::{AtomicU32, Ordering};

/// Offset of `WDEV_RND_REG` within the modeled window (base `0x6003_5000`).
const RND_OFFSET: u64 = 0x7C;

/// Deterministic RNG-data register model. Interior mutability (atomic) because
/// the `Peripheral::read` contract takes `&self` yet each read must advance the
/// generator, mirroring the hardware's read-side-effect.
#[derive(Debug)]
pub struct Esp32s3Rng {
    state: AtomicU32,
}

impl Default for Esp32s3Rng {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32s3Rng {
    pub fn new() -> Self {
        // Fixed non-zero seed → deterministic, reproducible entropy stream.
        Self {
            state: AtomicU32::new(0x2545_F491),
        }
    }

    /// Advance the xorshift32 generator and return the next value.
    fn next(&self) -> u32 {
        // Standard xorshift32 (Marsaglia). State stays non-zero for a non-zero
        // seed, so reads are effectively always non-zero.
        let mut x = self.state.load(Ordering::Relaxed);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state.store(x, Ordering::Relaxed);
        x
    }
}

impl Peripheral for Esp32s3Rng {
    fn read(&self, offset: u64) -> SimResult<u8> {
        if offset & !3 == RND_OFFSET {
            let word = self.next();
            return Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8);
        }
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // RNG_DATA is read-only; writes are ignored on hardware.
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if offset & !3 == RND_OFFSET {
            return Ok(self.next());
        }
        Ok(0)
    }

    fn write_u32(&mut self, _offset: u64, _value: u32) -> SimResult<()> {
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successive_reads_differ_and_are_nonzero() {
        let rng = Esp32s3Rng::new();
        let a = rng.read_u32(RND_OFFSET).unwrap();
        let b = rng.read_u32(RND_OFFSET).unwrap();
        let c = rng.read_u32(RND_OFFSET).unwrap();
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, 0);
        assert_ne!(b, 0);
    }

    #[test]
    fn deterministic_across_instances() {
        // Same seed → identical stream (reproducibility invariant).
        let r1 = Esp32s3Rng::new();
        let r2 = Esp32s3Rng::new();
        for _ in 0..16 {
            assert_eq!(
                r1.read_u32(RND_OFFSET).unwrap(),
                r2.read_u32(RND_OFFSET).unwrap()
            );
        }
    }
}
