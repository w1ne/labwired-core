// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 64-bit microsecond timer (datasheet §4.6, base `0x40054000`).
//!
//! A single free-running 64-bit counter that, on real silicon, increments once
//! per microsecond off the watchdog tick. This model advances the counter by
//! one on every peripheral tick — the absolute rate is arbitrary (the sim has
//! no wall clock), but the counter is genuinely monotonic, so firmware that
//! samples it twice always sees forward progress.
//!
//! `TIMERAWL` / `TIMERAWH` read the live low / high words. `TIMELR` / `TIMEHR`
//! read the same live counter: the datasheet's read-latch protocol (TIMELR
//! latches the high word for the next TIMEHR read) is a read-time side effect
//! that this trait's `&self` read path cannot carry, so the latch is not
//! modelled — both pairs return the live counter. `PAUSE.bit0` freezes the
//! counter (the debug / firmware pause control). Writing `TIMELW` then `TIMEHW`
//! seeds the counter. Every other register is plain storage so configuration
//! writes and their read-backs never fault.

use crate::{Peripheral, SimResult};

// Register offsets (relative to the TIMER base).
const TIMEHW: u64 = 0x00; // write high word (commits staged low word)
const TIMELW: u64 = 0x04; // write (stage) low word
const TIMEHR: u64 = 0x08; // read high word
const TIMELR: u64 = 0x0c; // read low word
const TIMERAWH: u64 = 0x24; // read live high word
const TIMERAWL: u64 = 0x28; // read live low word
const PAUSE: u64 = 0x30; // bit0: freeze counter

#[derive(Debug)]
pub struct Rp2040Timer {
    /// Live 64-bit microsecond counter.
    counter: u64,
    /// `PAUSE.bit0` — when set, `tick` does not advance the counter.
    paused: bool,
    /// Low word staged by a `TIMELW` write (committed by `TIMEHW`).
    pending_low: u32,
}

impl Default for Rp2040Timer {
    fn default() -> Self {
        Self::new()
    }
}

impl Rp2040Timer {
    pub fn new() -> Self {
        Self {
            counter: 0,
            paused: false,
            pending_low: 0,
        }
    }
}

impl Peripheral for Rp2040Timer {
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let val = match offset {
            TIMERAWL | TIMELR => self.counter as u32,
            TIMERAWH | TIMEHR => (self.counter >> 32) as u32,
            PAUSE => self.paused as u32,
            _ => 0,
        };
        Ok(val)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            PAUSE => self.paused = value & 0x1 != 0,
            TIMELW => self.pending_low = value,
            // Writing the high word commits the staged low word as the new
            // counter base (datasheet: write TIMELW then TIMEHW).
            TIMEHW => self.counter = ((value as u64) << 32) | self.pending_low as u64,
            _ => {}
        }
        Ok(())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !0x3)?;
        Ok((word >> ((offset & 0x3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !0x3;
        let shift = (offset & 0x3) * 8;
        let cur = self.read_u32(aligned)?;
        let new = (cur & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_u32(aligned, new)
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        if !self.paused {
            self.counter = self.counter.wrapping_add(1);
        }
        crate::PeripheralTickResult::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_advances_on_tick() {
        let mut t = Rp2040Timer::new();
        let a = t.read_u32(TIMERAWL).unwrap();
        for _ in 0..10 {
            t.tick();
        }
        let b = t.read_u32(TIMERAWL).unwrap();
        assert_eq!(b.wrapping_sub(a), 10, "counter must advance by tick count");
    }

    #[test]
    fn pause_freezes_counter() {
        let mut t = Rp2040Timer::new();
        t.write_u32(PAUSE, 1).unwrap();
        let a = t.read_u32(TIMERAWL).unwrap();
        for _ in 0..10 {
            t.tick();
        }
        assert_eq!(t.read_u32(TIMERAWL).unwrap(), a, "paused counter must hold");
        // Un-pausing resumes advancing.
        t.write_u32(PAUSE, 0).unwrap();
        t.tick();
        assert_eq!(t.read_u32(TIMERAWL).unwrap(), a.wrapping_add(1));
    }

    #[test]
    fn raw_high_low_track_64bit_counter() {
        let mut t = Rp2040Timer::new();
        // Force the counter just below the 32-bit boundary via the write path.
        t.write_u32(TIMELW, 0xFFFF_FFFE).unwrap();
        t.write_u32(TIMEHW, 0).unwrap();
        t.tick();
        t.tick();
        // 0xFFFF_FFFE + 2 = 0x1_0000_0000.
        assert_eq!(t.read_u32(TIMERAWL).unwrap(), 0);
        assert_eq!(t.read_u32(TIMERAWH).unwrap(), 1);
    }
}
