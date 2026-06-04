// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 TIMER peripheral.
//!
//! Source: nRF52840 Product Specification v1.7 §6.30 (TIMER), tables
//! 158–161.
//!
//! Models TIMER0..TIMER4 in Timer mode (`MODE=0`): runs on a 16 MHz base
//! clock divided by `2^PRESCALER` (1..=9), counter width set by BITMODE
//! (8/16/24/32 bits), CC[0..5] comparators each raising EVENTS_COMPARE[i]
//! when matched and pending the peripheral's NVIC IRQ when the
//! corresponding INTEN bit is set. SHORTS provides auto-CLEAR and
//! auto-STOP on compare match (PS table 161).
//!
//! Counter mode (`MODE=1` / `MODE=2`) increments only on TASKS_COUNT and
//! is supported by the register surface but not auto-driven by `tick()`.
//!
//! Timing fidelity: the sim ticks the prescaler accumulator once per
//! `tick()` call (i.e. once per CPU step at the default
//! peripheral_tick_interval=1).  That means TIMER advances at ~CPU rate,
//! not 16 MHz, but the ordering of events is preserved — which is what
//! firmware control flow depends on. Absolute wall-clock fidelity is
//! left to a future cycle-budget calibration pass.

use crate::{Peripheral, PeripheralTickResult, SimResult};

// ── Register offsets (PS §6.30.13, table 159) ────────────────────────────────

const OFF_TASKS_START: u64 = 0x000;
const OFF_TASKS_STOP: u64 = 0x004;
const OFF_TASKS_COUNT: u64 = 0x008;
const OFF_TASKS_CLEAR: u64 = 0x00C;
const OFF_TASKS_SHUTDOWN: u64 = 0x010;
const OFF_TASKS_CAPTURE0: u64 = 0x040;
const OFF_TASKS_CAPTURE5: u64 = 0x054;
const OFF_EVENTS_COMPARE0: u64 = 0x140;
const OFF_EVENTS_COMPARE5: u64 = 0x154;
const OFF_SHORTS: u64 = 0x200;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_MODE: u64 = 0x504;
const OFF_BITMODE: u64 = 0x508;
const OFF_PRESCALER: u64 = 0x510;
const OFF_CC0: u64 = 0x540;
const OFF_CC5: u64 = 0x554;

// SHORTS bits (PS table 161): COMPARE[i]_CLEAR at bit i, COMPARE[i]_STOP at bit i+8.
const SHORT_COMPARE_CLEAR_SHIFT: u32 = 0;
const SHORT_COMPARE_STOP_SHIFT: u32 = 8;

// INTEN bits: COMPARE[i] at bit i+16 (PS table 160).
const INTEN_COMPARE_SHIFT: u32 = 16;

// MODE values (PS table 161): 0=Timer, 1=Counter, 2=LowPowerCounter.
const MODE_TIMER: u32 = 0;

#[derive(Debug, Default)]
pub struct Nrf52Timer {
    events_compare: [u32; 6],
    shorts: u32,
    inten: u32,
    mode: u32,
    bitmode: u32,
    prescaler: u32,
    cc: [u32; 6],

    // Dynamic state — driven by tick().
    running: bool,
    counter: u32,
    prescaler_accum: u32,
}

impl Nrf52Timer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mask the counter to the active BITMODE width.
    fn counter_mask(&self) -> u32 {
        match self.bitmode & 0x3 {
            0 => 0x0000_FFFF, // 16-bit (reset)
            1 => 0x0000_00FF, // 8-bit
            2 => 0x00FF_FFFF, // 24-bit
            3 => 0xFFFF_FFFF, // 32-bit
            _ => unreachable!(),
        }
    }
}

impl Peripheral for Nrf52Timer {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let val = match offset {
            OFF_TASKS_START | OFF_TASKS_STOP | OFF_TASKS_COUNT | OFF_TASKS_CLEAR
            | OFF_TASKS_SHUTDOWN => 0,
            OFF_TASKS_CAPTURE0..=OFF_TASKS_CAPTURE5 if offset.is_multiple_of(4) => 0,

            OFF_EVENTS_COMPARE0..=OFF_EVENTS_COMPARE5 if offset.is_multiple_of(4) => {
                self.events_compare[((offset - OFF_EVENTS_COMPARE0) / 4) as usize]
            }

            OFF_SHORTS => self.shorts,

            OFF_INTENSET | OFF_INTENCLR => self.inten,

            OFF_MODE => self.mode,
            OFF_BITMODE => self.bitmode,
            OFF_PRESCALER => self.prescaler,
            OFF_CC0..=OFF_CC5 if offset.is_multiple_of(4) => {
                self.cc[((offset - OFF_CC0) / 4) as usize]
            }

            _ => 0,
        };
        Ok(val)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_START
                if value & 1 != 0 => {
                    self.running = true;
                }
            OFF_TASKS_STOP | OFF_TASKS_SHUTDOWN
                if value & 1 != 0 => {
                    self.running = false;
                }
            OFF_TASKS_COUNT
                // Counter mode advance — gated by MODE != Timer.
                if value & 1 != 0 && self.mode != MODE_TIMER => {
                    self.counter = (self.counter.wrapping_add(1)) & self.counter_mask();
                }
            OFF_TASKS_CLEAR
                if value & 1 != 0 => {
                    self.counter = 0;
                    self.prescaler_accum = 0;
                }
            OFF_TASKS_CAPTURE0..=OFF_TASKS_CAPTURE5 if offset.is_multiple_of(4)
                && value & 1 != 0 => {
                    let i = ((offset - OFF_TASKS_CAPTURE0) / 4) as usize;
                    self.cc[i] = self.counter;
                }

            OFF_EVENTS_COMPARE0..=OFF_EVENTS_COMPARE5 if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_EVENTS_COMPARE0) / 4) as usize;
                self.events_compare[i] = value & 1;
            }

            OFF_SHORTS => self.shorts = value,

            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,

            OFF_MODE => self.mode = value & 0x3,
            OFF_BITMODE => self.bitmode = value & 0x3,
            OFF_PRESCALER => self.prescaler = value & 0xF,

            OFF_CC0..=OFF_CC5 if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_CC0) / 4) as usize;
                self.cc[i] = value & self.counter_mask();
            }

            _ => {}
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if !self.running || self.mode != MODE_TIMER {
            return PeripheralTickResult::default();
        }

        // Prescaler divides the base clock by 2^PRESCALER. We accumulate
        // one base tick per call; when the accumulator reaches the divider
        // we advance the main counter by one.
        let divider = 1u32 << (self.prescaler & 0xF);
        self.prescaler_accum = self.prescaler_accum.wrapping_add(1);
        if self.prescaler_accum < divider {
            return PeripheralTickResult {
                cycles: 1,
                ..Default::default()
            };
        }
        self.prescaler_accum = 0;

        let mask = self.counter_mask();
        self.counter = self.counter.wrapping_add(1) & mask;

        let mut irq = false;
        let mut fired_events = Vec::new();
        for i in 0..6 {
            if self.counter == (self.cc[i] & mask) {
                // Per PS §6.30.5: the compare-match pulse re-arms on every
                // hardware tick — PPI and NVIC see it whether or not the
                // EVENTS_COMPARE register is still latched from a prior
                // match.  We always emit the fired_event; the register
                // bit becomes a sticky latch that firmware clears.
                self.events_compare[i] = 1;
                fired_events.push(OFF_EVENTS_COMPARE0 as u32 + 4 * i as u32);

                if (self.inten >> (INTEN_COMPARE_SHIFT + i as u32)) & 1 != 0 {
                    irq = true;
                }

                if (self.shorts >> (SHORT_COMPARE_CLEAR_SHIFT + i as u32)) & 1 != 0 {
                    self.counter = 0;
                }
                if (self.shorts >> (SHORT_COMPARE_STOP_SHIFT + i as u32)) & 1 != 0 {
                    self.running = false;
                }
            }
        }

        PeripheralTickResult {
            irq,
            cycles: 1,
            fired_events,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmode_round_trips() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_BITMODE, 3).unwrap();
        assert_eq!(t.read_u32(OFF_BITMODE).unwrap(), 3);
        t.write_u32(OFF_BITMODE, 0xFFFF_FFFF).unwrap();
        assert_eq!(t.read_u32(OFF_BITMODE).unwrap(), 0x3);
    }

    #[test]
    fn prescaler_masks_to_4_bits() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_PRESCALER, 0xFFFF_FFFF).unwrap();
        assert_eq!(t.read_u32(OFF_PRESCALER).unwrap(), 0xF);
    }

    #[test]
    fn cc_array_full_width() {
        let mut t = Nrf52Timer::new();
        // BITMODE=3 (32-bit) so the full value survives CC masking.
        t.write_u32(OFF_BITMODE, 3).unwrap();
        for i in 0..6u64 {
            t.write_u32(OFF_CC0 + i * 4, 0xDEAD_0000 | i as u32)
                .unwrap();
        }
        for i in 0..6u64 {
            assert_eq!(t.read_u32(OFF_CC0 + i * 4).unwrap(), 0xDEAD_0000 | i as u32);
        }
    }

    #[test]
    fn intenset_intenclr_alias_inten() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_INTENSET, 0b0111).unwrap();
        assert_eq!(t.read_u32(OFF_INTENSET).unwrap(), 0b0111);
        assert_eq!(t.read_u32(OFF_INTENCLR).unwrap(), 0b0111);
        t.write_u32(OFF_INTENCLR, 0b0010).unwrap();
        assert_eq!(t.read_u32(OFF_INTENSET).unwrap(), 0b0101);
    }

    #[test]
    fn tasks_read_as_zero_even_after_write() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        assert_eq!(t.read_u32(OFF_TASKS_START).unwrap(), 0);
    }

    #[test]
    fn tick_advances_counter_through_prescaler() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_BITMODE, 3).unwrap(); // 32-bit
        t.write_u32(OFF_PRESCALER, 0).unwrap(); // divider = 1
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        for _ in 0..10 {
            t.tick();
        }
        // PRESCALER=0 → 1:1, expect counter==10.
        assert_eq!(t.counter, 10);
    }

    #[test]
    fn cc_match_sets_event_and_pends_irq() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_BITMODE, 3).unwrap();
        t.write_u32(OFF_PRESCALER, 0).unwrap();
        t.write_u32(OFF_CC0, 5).unwrap();
        t.write_u32(OFF_INTENSET, 1 << 16).unwrap(); // COMPARE[0]
        t.write_u32(OFF_TASKS_START, 1).unwrap();

        let mut irq_count = 0;
        for _ in 0..5 {
            if t.tick().irq {
                irq_count += 1;
            }
        }
        // After 5 ticks at PRESCALER=0: counter advanced 0→5 → CC[0] match.
        assert_eq!(t.counter, 5);
        assert_eq!(t.read_u32(OFF_EVENTS_COMPARE0).unwrap(), 1);
        assert_eq!(irq_count, 1);
    }

    #[test]
    fn shorts_compare0_clear_resets_counter() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_BITMODE, 3).unwrap();
        t.write_u32(OFF_PRESCALER, 0).unwrap();
        t.write_u32(OFF_CC0, 3).unwrap();
        t.write_u32(OFF_SHORTS, 1).unwrap(); // COMPARE[0]_CLEAR
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        for _ in 0..10 {
            t.tick();
        }
        // Sequence: 1,2,3(→0),1,2,3(→0),1,2,3(→0),1 → counter==1 after 10 ticks.
        assert_eq!(t.counter, 1);
    }

    #[test]
    fn shorts_compare0_stop_halts_timer() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_BITMODE, 3).unwrap();
        t.write_u32(OFF_PRESCALER, 0).unwrap();
        t.write_u32(OFF_CC0, 3).unwrap();
        t.write_u32(OFF_SHORTS, 1 << 8).unwrap(); // COMPARE[0]_STOP
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        for _ in 0..10 {
            t.tick();
        }
        // Stopped at counter==3 after 3 ticks; remaining 7 ticks no-op.
        assert_eq!(t.counter, 3);
        assert!(!t.running);
    }

    #[test]
    fn prescaler_divider_slows_counter() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_BITMODE, 3).unwrap();
        t.write_u32(OFF_PRESCALER, 3).unwrap(); // divider = 8
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        for _ in 0..32 {
            t.tick();
        }
        // 32 ticks / 8 = 4 increments.
        assert_eq!(t.counter, 4);
    }

    #[test]
    fn capture_records_current_counter() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_BITMODE, 3).unwrap();
        t.write_u32(OFF_PRESCALER, 0).unwrap();
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        for _ in 0..7 {
            t.tick();
        }
        t.write_u32(OFF_TASKS_CAPTURE0, 1).unwrap();
        assert_eq!(t.read_u32(OFF_CC0).unwrap(), 7);
    }

    #[test]
    fn tasks_clear_resets_counter_and_prescaler() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_BITMODE, 3).unwrap();
        t.write_u32(OFF_PRESCALER, 0).unwrap();
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        for _ in 0..10 {
            t.tick();
        }
        t.write_u32(OFF_TASKS_CLEAR, 1).unwrap();
        assert_eq!(t.counter, 0);
        assert_eq!(t.prescaler_accum, 0);
    }
}
