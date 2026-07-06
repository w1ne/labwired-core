// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 RTC_CNTL main timer (`0x6000_8000`) — the free-running RTC
//! slow-clock counter the IDF reads via `rtc_time_get`.
//!
//! `rtc_time_get` does: set `RTC_CNTL_TIME_UPDATE` (offset `0x0C`, bit31) to
//! latch the 48-bit counter into the readout registers, then read the low word
//! (`RTC_CNTL_TIME0`, `0x10`) and high word (`RTC_CNTL_TIME1`, `0x14`).
//!
//! A plain register-backed stub returns a *constant* timer, so every IDF loop
//! that waits on an RTC deadline spins forever. The most load-bearing example
//! is `calibrate_ocode` (RTC bandgap offset-code calibration): it polls a
//! regi2c comparator and exits either when the comparator settles *or* when a
//! ~10 ms RTC deadline expires. With no real RF/analog the comparator never
//! settles, so the loop relies entirely on the timeout — which never fires if
//! the timer is frozen. Modelling the counter as a real advancing timer lets
//! that loop (and every other RTC-deadline wait: clock cal, PHY cal, delays)
//! reach its designed timeout and continue, exactly as silicon does when the
//! calibration can't converge.
//!
//! The counter advances one tick per simulated CPU step (`tick()` is called
//! every step at the default `peripheral_tick_interval = 1`). The absolute
//! slow-clock rate is not modelled — only that time *advances* monotonically,
//! which is all the deadline comparisons observe. All other registers in the
//! window are register-backed (writes stored, reads return the last value) so
//! the rest of RTC_CNTL bring-up (reset-cause seed at `0x38`, ANA config, …)
//! behaves like the previous declarative stub.

use crate::{Peripheral, SimResult};
use std::cell::Cell;

const TIME_UPDATE: u64 = 0x0C; // bit31 = latch request
const TIME_LOW: u64 = 0x10;
const TIME_HIGH: u64 = 0x14;
const TIME_UPDATE_BIT: u32 = 1 << 31;

#[derive(Debug)]
pub struct Esp32c3RtcTimer {
    /// Register-backed storage for the whole window (non-timer registers).
    regs: Vec<u32>,
    /// Free-running 48-bit slow-clock counter, advanced once per step.
    counter: Cell<u64>,
    /// Counter value latched by the most recent TIME_UPDATE write; what the
    /// TIME0/TIME1 readout registers return.
    latched: Cell<u64>,
    /// Scheduler/elapsed-mode anchor in peripheral-tick units.
    anchor_tick: Cell<u64>,
}

impl Default for Esp32c3RtcTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32c3RtcTimer {
    /// `size_bytes` is the window size (rounded up to whole 32-bit words).
    pub fn new_sized(size_bytes: usize) -> Self {
        Self {
            regs: vec![0u32; size_bytes.div_ceil(4)],
            counter: Cell::new(0),
            latched: Cell::new(0),
            anchor_tick: Cell::new(0),
        }
    }

    pub fn new() -> Self {
        Self::new_sized(0x100)
    }
}

impl Peripheral for Esp32c3RtcTimer {
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
            TIME_LOW => self.latched.get() as u32,
            TIME_HIGH => (self.latched.get() >> 32) as u32,
            _ => *self.regs.get((offset / 4) as usize).unwrap_or(&0),
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset == TIME_UPDATE && (value & TIME_UPDATE_BIT) != 0 {
            // Latch the current counter into the readout registers.
            self.latched.set(self.counter.get());
        }
        if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
            *slot = value;
        }
        Ok(())
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        self.tick_elapsed(1)
    }

    fn tick_elapsed(&mut self, cycles: u64) -> crate::PeripheralTickResult {
        // One slow-clock tick per simulated step — time advances monotonically.
        self.counter.set(self.counter.get().wrapping_add(cycles));
        self.anchor_tick
            .set(self.anchor_tick.get().wrapping_add(cycles));
        crate::PeripheralTickResult::default()
    }

    fn uses_scheduler(&self) -> bool {
        true
    }

    fn sync_to(&mut self, tick_now: u64) {
        if tick_now <= self.anchor_tick.get() {
            return;
        }
        let delta = tick_now - self.anchor_tick.get();
        self.counter.set(self.counter.get().wrapping_add(delta));
        self.anchor_tick.set(tick_now);
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    /// Capture the free-running RTC counter + latched readout + register
    /// window. Boot-critical for a rom-boot resume: IDF RTC-deadline loops
    /// (e.g. `calibrate_ocode`) exit on a ~10 ms RTC timeout, so the number of
    /// calibration retries — and hence the exact boot-log lines — depends on
    /// the counter value carrying across the snapshot.
    fn runtime_snapshot(&self) -> Vec<u8> {
        let snap = RtcTimerSnapshot {
            regs: self.regs.clone(),
            counter: self.counter.get(),
            latched: self.latched.get(),
            anchor_tick: self.anchor_tick.get(),
        };
        bincode::serialize(&snap).expect("bincode serialize Esp32c3RtcTimer")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        let snap: RtcTimerSnapshot = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("Esp32c3RtcTimer snapshot decode: {e}"))
        })?;
        self.regs = snap.regs;
        self.counter.set(snap.counter);
        self.latched.set(snap.latched);
        self.anchor_tick.set(snap.anchor_tick);
        Ok(())
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct RtcTimerSnapshot {
    regs: Vec<u32>,
    counter: u64,
    latched: u64,
    #[serde(default)]
    anchor_tick: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rtc_time_get(t: &mut Esp32c3RtcTimer) -> u64 {
        // Mirror the IDF sequence: set TIME_UPDATE, read low then high.
        t.write_u32(TIME_UPDATE, TIME_UPDATE_BIT).unwrap();
        let lo = t.read_u32(TIME_LOW).unwrap() as u64;
        let hi = t.read_u32(TIME_HIGH).unwrap() as u64;
        lo | (hi << 32)
    }

    #[test]
    fn timer_advances_and_latches() {
        let mut t = Esp32c3RtcTimer::new();
        let t0 = rtc_time_get(&mut t);
        for _ in 0..1000 {
            t.tick();
        }
        let t1 = rtc_time_get(&mut t);
        assert!(t1 > t0, "RTC timer must advance ({t0} -> {t1})");
        assert_eq!(t1 - t0, 1000, "one tick per step");
    }

    #[test]
    fn readout_frozen_until_next_update() {
        let mut t = Esp32c3RtcTimer::new();
        t.write_u32(TIME_UPDATE, TIME_UPDATE_BIT).unwrap();
        let snap = t.read_u32(TIME_LOW).unwrap();
        for _ in 0..50 {
            t.tick();
        }
        // No new latch: readout is unchanged even though the counter advanced.
        assert_eq!(t.read_u32(TIME_LOW).unwrap(), snap);
        t.write_u32(TIME_UPDATE, TIME_UPDATE_BIT).unwrap();
        assert_eq!(t.read_u32(TIME_LOW).unwrap(), snap + 50);
    }

    #[test]
    fn tick_elapsed_matches_repeated_tick() {
        let mut repeated = Esp32c3RtcTimer::new();
        let mut elapsed = Esp32c3RtcTimer::new();

        for _ in 0..1000 {
            repeated.tick();
        }
        elapsed.tick_elapsed(1000);

        assert_eq!(rtc_time_get(&mut elapsed), rtc_time_get(&mut repeated));
    }

    #[test]
    fn other_registers_are_register_backed() {
        let mut t = Esp32c3RtcTimer::new();
        // e.g. the reset-cause seed at offset 0x38.
        t.write_u32(0x38, 0x1).unwrap();
        assert_eq!(t.read_u32(0x38).unwrap(), 0x1);
    }
}
