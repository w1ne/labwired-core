// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 RNG peripheral.
//!
//! Source: nRF52840 PS rev 1.7 §6.22 (RNG). True-random byte source on
//! silicon; we substitute a xorshift32 PRNG deterministically seeded so
//! firmware tests are reproducible. Firmware sees the same byte-ready
//! cadence and event semantics as real silicon.
//!
//! Timing: real RNG produces a byte roughly every 8 µs (CONFIG.DERCEN
//! cleared) or every 35 µs (DERCEN set). We approximate at the sim's
//! `tick()` cadence: one byte every BYTE_PERIOD calls. Absolute timing
//! is not matched; firmware that polls EVENTS_VALRDY or takes the IRQ
//! sees the same control flow.

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};

const OFF_TASKS_START: u64 = 0x000;
const OFF_TASKS_STOP: u64 = 0x004;
const OFF_EVENTS_VALRDY: u64 = 0x100;
const OFF_SHORTS: u64 = 0x200;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_CONFIG: u64 = 0x504;
const OFF_VALUE: u64 = 0x508;

const INTEN_VALRDY: u32 = 1 << 0;
const SHORTS_VALRDY_STOP: u32 = 1 << 0;

/// Ticks between byte productions when running.  Picked to match Zephyr
/// driver expectations (it polls EVENTS_VALRDY in a busy loop, then
/// disables and re-enables) without being so fast that test loops
/// burn CPU.
const BYTE_PERIOD: u32 = 64;

/// Xorshift32 seed.  Pulled from a fixed constant so RNG output is
/// reproducible across test runs.
const PRNG_SEED: u32 = 0xC0DE_F00D;

#[derive(Debug)]
pub struct Nrf52Rng {
    events_valrdy: u32,
    shorts: u32,
    inten: u32,
    config: u32,
    value: u32,

    running: bool,
    prng_state: u32,
    accum: u32,
    clock: Option<CycleClock>,
    anchor: u64,
    arm_seq: u32,
}

impl Default for Nrf52Rng {
    fn default() -> Self {
        Self {
            events_valrdy: 0,
            shorts: 0,
            inten: 0,
            config: 0,
            value: 0,
            running: false,
            prng_state: PRNG_SEED,
            accum: 0,
            clock: None,
            anchor: 0,
            arm_seq: 0,
        }
    }
}

impl Nrf52Rng {
    pub fn new() -> Self {
        Self::default()
    }

    /// xorshift32 — fast, deterministic, plenty good enough as a
    /// stand-in for the real TRNG byte stream.
    fn next_byte(&mut self) -> u8 {
        let mut x = self.prng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.prng_state = x;
        (x & 0xFF) as u8
    }
}

impl Peripheral for Nrf52Rng {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_START | OFF_TASKS_STOP => 0,
            OFF_EVENTS_VALRDY => self.events_valrdy,
            OFF_SHORTS => self.shorts,
            OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_CONFIG => self.config,
            OFF_VALUE => self.value,
            _ => {
                crate::census_reg!("nrf52.rng:Nrf52Rng", offset, "read");
                0
            }
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_START if value & 1 != 0 => {
                self.running = true;
            }
            OFF_TASKS_STOP if value & 1 != 0 => {
                self.running = false;
            }
            OFF_EVENTS_VALRDY => self.events_valrdy = value & 1,
            OFF_SHORTS => self.shorts = value & 1,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,
            OFF_CONFIG => self.config = value & 0x1,
            OFF_VALUE => {} // RO
            _ => {
                crate::census_reg!("nrf52.rng:Nrf52Rng", offset, "write");
            }
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.advance_cycles(1)
    }

    fn uses_scheduler(&self) -> bool {
        self.clock.is_some()
    }

    fn needs_legacy_walk(&self) -> bool {
        self.clock.is_none()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn sync_to(&mut self, now_cycle: u64) {
        if self.clock.is_none() || now_cycle <= self.anchor {
            return;
        }
        let delta = now_cycle - self.anchor;
        self.anchor = now_cycle;
        let _ = self.advance_cycles(delta);
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if self.clock.is_none() || !self.running {
            return Vec::new();
        }
        let remain = (BYTE_PERIOD - self.accum).max(1) as u64;
        self.arm_seq = self.arm_seq.wrapping_add(1);
        vec![(remain.saturating_sub(1), self.arm_seq)]
    }

    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if self.clock.is_none() || event_token != self.arm_seq {
            return crate::sched::EventResult::default();
        }
        let now = sched.now();
        let res = if now > self.anchor {
            let d = now - self.anchor;
            self.anchor = now;
            self.advance_cycles(d)
        } else {
            // Deadline hit with no lag: produce one byte now.
            self.accum = BYTE_PERIOD - 1;
            self.advance_cycles(1)
        };
        let next = if self.running {
            Some((BYTE_PERIOD - self.accum).max(1) as u64)
        } else {
            None
        };
        // Keep arm_seq so reschedule reuses this token.
        crate::sched::EventResult {
            raise_own_irq: res.irq,
            reschedule_delay: next.map(|d| d.saturating_sub(1)),
            ..Default::default()
        }
    }
}

impl Nrf52Rng {
    fn advance_cycles(&mut self, cycles: u64) -> PeripheralTickResult {
        if !self.running || cycles == 0 {
            return PeripheralTickResult::default();
        }
        let mut left = cycles;
        let mut irq = false;
        while left > 0 && self.running {
            let need = (BYTE_PERIOD - self.accum) as u64;
            if left < need {
                self.accum += left as u32;
                break;
            }
            left -= need;
            self.accum = 0;
            self.value = self.next_byte() as u32;
            self.events_valrdy = 1;
            if self.shorts & SHORTS_VALRDY_STOP != 0 {
                self.running = false;
            }
            if self.inten & INTEN_VALRDY != 0 {
                irq = true;
            }
            // One byte per period; if more cycles remain keep producing.
        }
        PeripheralTickResult {
            irq,
            cycles: 1,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_masks_to_1_bit() {
        let mut r = Nrf52Rng::new();
        r.write_u32(OFF_CONFIG, 0xFF).unwrap();
        assert_eq!(r.read_u32(OFF_CONFIG).unwrap(), 1);
    }

    #[test]
    fn idle_until_start() {
        let mut r = Nrf52Rng::new();
        for _ in 0..1000 {
            assert!(!r.tick().irq);
        }
        assert_eq!(r.read_u32(OFF_EVENTS_VALRDY).unwrap(), 0);
    }

    #[test]
    fn start_produces_byte_after_period() {
        let mut r = Nrf52Rng::new();
        r.write_u32(OFF_INTENSET, INTEN_VALRDY).unwrap();
        r.write_u32(OFF_TASKS_START, 1).unwrap();

        let mut irq_seen = false;
        for _ in 0..BYTE_PERIOD {
            if r.tick().irq {
                irq_seen = true;
            }
        }
        assert!(irq_seen, "expected VALRDY IRQ within one byte period");
        assert_eq!(r.read_u32(OFF_EVENTS_VALRDY).unwrap(), 1);
        // VALUE should be populated with the first PRNG byte.
        assert_ne!(r.read_u32(OFF_VALUE).unwrap(), 0);
    }

    #[test]
    fn shorts_valrdy_stop_halts_after_first_byte() {
        let mut r = Nrf52Rng::new();
        r.write_u32(OFF_SHORTS, SHORTS_VALRDY_STOP).unwrap();
        r.write_u32(OFF_TASKS_START, 1).unwrap();

        for _ in 0..(BYTE_PERIOD * 4) {
            r.tick();
        }
        assert!(!r.running);
    }

    #[test]
    fn prng_is_deterministic_across_runs() {
        let mut r1 = Nrf52Rng::new();
        let mut r2 = Nrf52Rng::new();
        r1.write_u32(OFF_TASKS_START, 1).unwrap();
        r2.write_u32(OFF_TASKS_START, 1).unwrap();
        for _ in 0..(BYTE_PERIOD * 3) {
            r1.tick();
            r2.tick();
        }
        assert_eq!(
            r1.read_u32(OFF_VALUE).unwrap(),
            r2.read_u32(OFF_VALUE).unwrap()
        );
    }
}
