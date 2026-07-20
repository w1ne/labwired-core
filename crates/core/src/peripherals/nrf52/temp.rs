// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 TEMP peripheral — register-surface model.
//!
//! Source: nRF52840 PS rev 1.7 §6.32 (TEMP). Built-in temperature sensor,
//! 0.25 °C resolution. TASKS_START latches an in-range measurement (25 °C = raw 100)
//! and fires EVENTS_DATARDY synchronously, matching silicon fidelity for firmware
//! that polls EVENTS_DATARDY in a tight loop after TASKS_START (typical pattern).

use crate::{Peripheral, SimResult};

const OFF_TASKS_START: u64 = 0x000;
const OFF_TASKS_STOP: u64 = 0x004;
const OFF_EVENTS_DATARDY: u64 = 0x100;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_TEMP: u64 = 0x508;
// Calibration coefficients (A0..A5, B0..B5, T0..T4) at 0x520..0x570.
const OFF_CAL_FIRST: u64 = 0x520;
const OFF_CAL_LAST: u64 = 0x570;

#[derive(Debug, Default)]
pub struct Nrf52Temp {
    events_datardy: u32,
    inten: u32,
    temp: u32,      // signed 32-bit; firmware reads as i32
    cal: [u32; 21], // 0x520..=0x570 step 4: A0-A5, gap, B0-B5, gap, T0-T4
}

impl Nrf52Temp {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Nrf52Temp {
    /// Not in the per-cycle walk: this model overrides neither `tick()` nor
    /// `tick_elapsed()`, so every visit ran the default no-op and returned a
    /// default `PeripheralTickResult`. Skipping it removes dispatch, never an
    /// effect — byte-identical by construction.
    ///
    /// Safe against the "sleeps and never wakes" trap: the bus calls
    /// `refresh_legacy_tick_index()` on every MMIO write, so if this model ever
    /// gains a tick and a state-dependent condition, a firmware write re-arms it.
    fn legacy_tick_active(&self) -> bool {
        false
    }
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_START | OFF_TASKS_STOP => 0,
            OFF_EVENTS_DATARDY => self.events_datardy,
            OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_TEMP => self.temp,
            OFF_CAL_FIRST..=OFF_CAL_LAST if offset.is_multiple_of(4) => {
                self.cal[((offset - OFF_CAL_FIRST) / 4) as usize]
            }
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_START if value & 1 != 0 => {
                // TASKS_START fires immediately with an in-range measurement.
                // 25 °C = raw 100 (0.25 °C steps). Firmware polls EVENTS_DATARDY
                // in a tight loop after TASKS_START, so event must be synchronous.
                self.temp = 100;
                self.events_datardy = 1;
            }
            OFF_TASKS_STOP if value & 1 != 0 => {
                // TASKS_STOP clears the pending measurement.
                self.events_datardy = 0;
            }
            OFF_EVENTS_DATARDY => self.events_datardy = value & 1,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,
            OFF_TEMP => self.temp = value,
            OFF_CAL_FIRST..=OFF_CAL_LAST if offset.is_multiple_of(4) => {
                self.cal[((offset - OFF_CAL_FIRST) / 4) as usize] = value;
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_register_round_trips() {
        let mut t = Nrf52Temp::new();
        t.write_u32(OFF_TEMP, 100).unwrap(); // 25.0 °C in 0.25 steps
        assert_eq!(t.read_u32(OFF_TEMP).unwrap(), 100);
    }

    #[test]
    fn tasks_start_produces_measurement() {
        let mut t = Nrf52Temp::new();
        // Firmware clears EVENTS_DATARDY first.
        t.write_u32(OFF_EVENTS_DATARDY, 0).unwrap();
        assert_eq!(t.read_u32(OFF_EVENTS_DATARDY).unwrap(), 0);
        // Firmware writes TASKS_START.
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        // Immediately after, EVENTS_DATARDY must be 1 (synchronous).
        assert_eq!(t.read_u32(OFF_EVENTS_DATARDY).unwrap(), 1);
        // TEMP must contain an in-range value (25 °C = 100).
        let raw = t.read_u32(OFF_TEMP).unwrap() as i32;
        assert_eq!(raw, 100);
        assert!((-200..=400).contains(&raw), "expected in-range TEMP value");
    }

    #[test]
    fn tasks_stop_clears_datardy() {
        let mut t = Nrf52Temp::new();
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        assert_eq!(t.read_u32(OFF_EVENTS_DATARDY).unwrap(), 1);
        // TASKS_STOP clears the event.
        t.write_u32(OFF_TASKS_STOP, 1).unwrap();
        assert_eq!(t.read_u32(OFF_EVENTS_DATARDY).unwrap(), 0);
    }
}
