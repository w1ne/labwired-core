// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 hardware RNG data register (`WDEV_RND_REG`, `0x6002_60B0`).
//!
//! On silicon this is a single read-only register that yields a fresh random
//! word on every read. Firmware (and the 2nd-stage bootloader) depend on that:
//! `bootloader_fill_random` builds each entropy byte from `RNG ^ RNG` of two
//! successive reads, and `process_segments` refills `ram_obfs_value` until it is
//! non-zero — so a constant RNG (the declarative stub returns 0) yields 0 and
//! spins forever. Model it as a fast xorshift32 that advances on every read,
//! giving varying, deterministic (reproducible) words. Not cryptographic, which
//! is fine for sim — it only has to *vary*.

use crate::{Peripheral, SimResult};
use std::cell::Cell;

#[derive(Debug)]
pub struct Esp32c3Rng {
    state: Cell<u32>,
}

impl Default for Esp32c3Rng {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32c3Rng {
    pub fn new() -> Self {
        Self {
            state: Cell::new(0xACE1_5EED), // non-zero xorshift seed
        }
    }

    fn next_word(&self) -> u32 {
        let mut x = self.state.get();
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state.set(x);
        x
    }
}

impl Peripheral for Esp32c3Rng {
    // Inert walk: xorshift state advances on read, never in the walk; tick() is the trait-default no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let w = self.next_word();
        Ok((w >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(()) // read-only
    }

    fn read_u32(&self, _offset: u64) -> SimResult<u32> {
        Ok(self.next_word())
    }

    fn write_u32(&mut self, _offset: u64, _value: u32) -> SimResult<()> {
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
