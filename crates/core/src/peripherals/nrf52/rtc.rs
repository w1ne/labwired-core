// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 RTC peripheral.
//!
//! Source: nRF52840 PS rev 1.7 §6.21 (RTC). Models RTC0..RTC2 on a
//! 32.768 kHz LFCLK; 24-bit counter with a 12-bit prescaler:
//! f_RTC = 32_768 / (PRESCALER + 1) Hz.
//!
//! `tick()` advances the prescaler accumulator once per call. When it
//! reaches PRESCALER+1, the counter ticks up. EVENTS_TICK fires on every
//! counter increment (gated by EVTEN.TICK); EVENTS_OVRFLW fires when the
//! counter wraps 0x00FF_FFFF → 0; EVENTS_COMPARE[i] fires when the
//! counter reaches CC[i]. Each event raises the configured NVIC IRQ if
//! the corresponding INTEN bit is set.

use crate::{Peripheral, PeripheralTickResult, SimResult};

// ── Register offsets (PS §6.21.13) ───────────────────────────────────────────

const OFF_TASKS_START: u64 = 0x000;
const OFF_TASKS_STOP: u64 = 0x004;
const OFF_TASKS_CLEAR: u64 = 0x008;
const OFF_TASKS_TRIGOVRFLW: u64 = 0x00C;
const OFF_EVENTS_TICK: u64 = 0x100;
const OFF_EVENTS_OVRFLW: u64 = 0x104;
const OFF_EVENTS_COMPARE0: u64 = 0x140;
const OFF_EVENTS_COMPARE3: u64 = 0x14C;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_EVTEN: u64 = 0x340;
const OFF_EVTENSET: u64 = 0x344;
const OFF_EVTENCLR: u64 = 0x348;
const OFF_COUNTER: u64 = 0x504;
const OFF_PRESCALER: u64 = 0x508;
const OFF_CC0: u64 = 0x540;
const OFF_CC3: u64 = 0x54C;

// INTEN / EVTEN bits (PS table 109):
//   TICK     bit 0
//   OVRFLW   bit 1
//   COMPARE0 bit 16, COMPARE1 bit 17, COMPARE2 bit 18, COMPARE3 bit 19
const EN_TICK: u32 = 1 << 0;
const EN_OVRFLW: u32 = 1 << 1;
const EN_COMPARE_SHIFT: u32 = 16;

const COUNTER_MASK: u32 = 0x00FF_FFFF;
const PRESCALER_MASK: u32 = 0xFFF;

#[derive(Debug, Default)]
pub struct Nrf52Rtc {
    events_tick: u32,
    events_ovrflw: u32,
    events_compare: [u32; 4],
    inten: u32,
    evten: u32,
    counter: u32,
    prescaler: u32,

    cc: [u32; 4],

    running: bool,
    prescaler_accum: u32,
}

impl Nrf52Rtc {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Nrf52Rtc {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_START | OFF_TASKS_STOP | OFF_TASKS_CLEAR | OFF_TASKS_TRIGOVRFLW => 0,
            OFF_EVENTS_TICK => self.events_tick,
            OFF_EVENTS_OVRFLW => self.events_ovrflw,
            OFF_EVENTS_COMPARE0..=OFF_EVENTS_COMPARE3 if offset.is_multiple_of(4) => {
                self.events_compare[((offset - OFF_EVENTS_COMPARE0) / 4) as usize]
            }
            OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_EVTEN | OFF_EVTENSET | OFF_EVTENCLR => self.evten,
            OFF_COUNTER => self.counter & COUNTER_MASK,
            OFF_PRESCALER => self.prescaler,
            OFF_CC0..=OFF_CC3 if offset.is_multiple_of(4) => {
                self.cc[((offset - OFF_CC0) / 4) as usize]
            }
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_START => {
                if value & 1 != 0 {
                    self.running = true;
                }
            }
            OFF_TASKS_STOP => {
                if value & 1 != 0 {
                    self.running = false;
                }
            }
            OFF_TASKS_CLEAR => {
                if value & 1 != 0 {
                    self.counter = 0;
                    self.prescaler_accum = 0;
                }
            }
            OFF_TASKS_TRIGOVRFLW => {
                // Per PS §6.21.5: sets COUNTER to 0x00FFFFF0 to trigger overflow
                // 16 ticks later. Useful for test programs.
                if value & 1 != 0 {
                    self.counter = 0x00FF_FFF0;
                }
            }
            OFF_EVENTS_TICK => self.events_tick = value & 1,
            OFF_EVENTS_OVRFLW => self.events_ovrflw = value & 1,
            OFF_EVENTS_COMPARE0..=OFF_EVENTS_COMPARE3 if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_EVENTS_COMPARE0) / 4) as usize;
                self.events_compare[i] = value & 1;
            }
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,
            OFF_EVTEN => self.evten = value,
            OFF_EVTENSET => self.evten |= value,
            OFF_EVTENCLR => self.evten &= !value,
            // COUNTER is RO.
            OFF_COUNTER => {}
            OFF_PRESCALER => {
                // PS §6.21.5: PRESCALER can only be written while STOPPED.
                if !self.running {
                    self.prescaler = value & PRESCALER_MASK;
                }
            }
            OFF_CC0..=OFF_CC3 if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_CC0) / 4) as usize;
                self.cc[i] = value & COUNTER_MASK;
            }
            _ => {}
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if !self.running {
            return PeripheralTickResult::default();
        }

        // PRESCALER+1 base-clock cycles per counter increment.
        let divider = (self.prescaler & PRESCALER_MASK) + 1;
        self.prescaler_accum = self.prescaler_accum.wrapping_add(1);
        if self.prescaler_accum < divider {
            return PeripheralTickResult {
                cycles: 1,
                ..Default::default()
            };
        }
        self.prescaler_accum = 0;

        let prev = self.counter;
        self.counter = (self.counter.wrapping_add(1)) & COUNTER_MASK;

        let mut irq = false;
        let mut fired_events = Vec::new();

        // EVENTS_TICK fires on every increment when EVTEN.TICK is set.
        // Per PS §6.21.4, the pulse re-fires on every counter advance
        // independent of whether the register is still latched.
        if self.evten & EN_TICK != 0 {
            self.events_tick = 1;
            fired_events.push(OFF_EVENTS_TICK as u32);
        }
        if self.inten & EN_TICK != 0 {
            irq = true;
        }

        // EVENTS_OVRFLW on wrap.
        if prev == COUNTER_MASK && self.counter == 0 {
            if self.evten & EN_OVRFLW != 0 {
                self.events_ovrflw = 1;
                fired_events.push(OFF_EVENTS_OVRFLW as u32);
            }
            if self.inten & EN_OVRFLW != 0 {
                irq = true;
            }
        }

        for i in 0..4 {
            if self.counter == (self.cc[i] & COUNTER_MASK) {
                let bit = EN_COMPARE_SHIFT + i as u32;
                if self.evten & (1 << bit) != 0 {
                    self.events_compare[i] = 1;
                    fired_events.push(OFF_EVENTS_COMPARE0 as u32 + 4 * i as u32);
                }
                if self.inten & (1 << bit) != 0 {
                    irq = true;
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
    fn prescaler_masks_to_12_bits() {
        let mut r = Nrf52Rtc::new();
        r.write_u32(OFF_PRESCALER, 0xFFFF_FFFF).unwrap();
        assert_eq!(r.read_u32(OFF_PRESCALER).unwrap(), 0xFFF);
    }

    #[test]
    fn prescaler_locked_while_running() {
        let mut r = Nrf52Rtc::new();
        r.write_u32(OFF_PRESCALER, 0x10).unwrap();
        r.write_u32(OFF_TASKS_START, 1).unwrap();
        r.write_u32(OFF_PRESCALER, 0x100).unwrap(); // dropped
        assert_eq!(r.read_u32(OFF_PRESCALER).unwrap(), 0x10);
    }

    #[test]
    fn cc_masks_to_24_bits() {
        let mut r = Nrf52Rtc::new();
        r.write_u32(OFF_CC0, 0xFFFF_FFFF).unwrap();
        assert_eq!(r.read_u32(OFF_CC0).unwrap(), 0x00FF_FFFF);
    }

    #[test]
    fn tick_compare_fires_event_and_irq() {
        let mut r = Nrf52Rtc::new();
        r.write_u32(OFF_PRESCALER, 0).unwrap();
        r.write_u32(OFF_CC0, 7).unwrap();
        r.write_u32(OFF_EVTENSET, 1 << 16).unwrap();
        r.write_u32(OFF_INTENSET, 1 << 16).unwrap();
        r.write_u32(OFF_TASKS_START, 1).unwrap();

        let mut fires = 0;
        for _ in 0..14 {
            if r.tick().irq {
                fires += 1;
            }
        }
        assert_eq!(r.read_u32(OFF_EVENTS_COMPARE0).unwrap(), 1);
        assert_eq!(fires, 1);
    }

    #[test]
    fn tick_tick_event_fires_when_enabled() {
        let mut r = Nrf52Rtc::new();
        r.write_u32(OFF_PRESCALER, 0).unwrap();
        r.write_u32(OFF_EVTENSET, 1).unwrap();
        r.write_u32(OFF_INTENSET, 1).unwrap();
        r.write_u32(OFF_TASKS_START, 1).unwrap();

        let result = r.tick();
        assert_eq!(r.read_u32(OFF_EVENTS_TICK).unwrap(), 1);
        assert!(result.irq);
    }

    #[test]
    fn trigovrflw_jumps_counter_to_pretrigger() {
        let mut r = Nrf52Rtc::new();
        r.write_u32(OFF_TASKS_TRIGOVRFLW, 1).unwrap();
        assert_eq!(r.read_u32(OFF_COUNTER).unwrap(), 0x00FF_FFF0);
    }

    #[test]
    fn counter_wraps_at_24_bits_and_fires_ovrflw() {
        let mut r = Nrf52Rtc::new();
        r.write_u32(OFF_PRESCALER, 0).unwrap();
        r.write_u32(OFF_EVTENSET, 1 << 1).unwrap(); // OVRFLW
        r.write_u32(OFF_INTENSET, 1 << 1).unwrap();
        r.write_u32(OFF_TASKS_TRIGOVRFLW, 1).unwrap(); // counter = 0x00FFFFF0
        r.write_u32(OFF_TASKS_START, 1).unwrap();

        let mut overflow_irq = false;
        for _ in 0..32 {
            if r.tick().irq {
                overflow_irq = true;
            }
        }
        assert!(overflow_irq);
        assert_eq!(r.read_u32(OFF_EVENTS_OVRFLW).unwrap(), 1);
    }
}
