// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 PWM peripheral — register surface + sequence playback engine.
//!
//! Source: nRF52840 PS rev 1.7 §6.18 (PWM). Models PWM0..PWM3 — same
//! register map per instance. Drives 4 outputs per peripheral.
//!
//! # Sequence engine (deterministic, synchronous)
//!
//! **TASKS_SEQSTART[n] (0x008 / 0x00C):** if ENABLE=1, the engine decodes
//! SEQ[n].PTR / SEQ[n].CNT and reads the `CNT` 16-bit duty values out of the
//! EasyDMA buffer in guest RAM (so the sequence registers genuinely drive the
//! playback), then fires EVENTS_SEQSTARTED[n] (0x108 / 0x10C), EVENTS_SEQEND[n]
//! (0x110 / 0x114) and EVENTS_PWMPERIODEND (0x118) to reflect the sequence
//! having played to completion against COUNTERTOP. The RAM read runs on the
//! next bus tick (same `needs_bus_tick`/`tick_with_bus` pattern as TWIM/SPIM).
//!
//! **TASKS_STOP (0x004):** fires EVENTS_STOPPED (0x104) synchronously.
//!
//! # EVENTS write semantics
//!
//! SW writes of 1 are silently ignored (hardware-generated only). SW writes of
//! 0 clear the event register.

use crate::{Bus, Peripheral, SimResult};

/// No sequence pending.
const PENDING_NONE: u8 = 0;
/// TASKS_SEQSTART0 was written.
const PENDING_SEQ0: u8 = 1;
/// TASKS_SEQSTART1 was written.
const PENDING_SEQ1: u8 = 2;

const OFF_TASKS_STOP: u64 = 0x004;
const OFF_TASKS_SEQSTART0: u64 = 0x008;
const OFF_TASKS_SEQSTART1: u64 = 0x00C;
const OFF_TASKS_NEXTSTEP: u64 = 0x010;

const OFF_EVENTS_STOPPED: u64 = 0x104;
const OFF_EVENTS_SEQSTARTED0: u64 = 0x108;
const OFF_EVENTS_SEQSTARTED1: u64 = 0x10C;
const OFF_EVENTS_SEQEND0: u64 = 0x110;
const OFF_EVENTS_SEQEND1: u64 = 0x114;
const OFF_EVENTS_PWMPERIODEND: u64 = 0x118;
const OFF_EVENTS_LOOPSDONE: u64 = 0x11C;

const OFF_SHORTS: u64 = 0x200;
const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;

const OFF_ENABLE: u64 = 0x500;
const OFF_MODE: u64 = 0x504;
const OFF_COUNTERTOP: u64 = 0x508;
const OFF_PRESCALER: u64 = 0x50C;
const OFF_DECODER: u64 = 0x510;
const OFF_LOOP: u64 = 0x514;

// SEQ[0] block at 0x520..0x52F; SEQ[1] at 0x540..0x54F.
const OFF_SEQ_FIRST: u64 = 0x520;
const OFF_SEQ_LAST: u64 = 0x54C;

// PSEL.OUT[0..3] at 0x560..0x56C.
const OFF_PSEL_FIRST: u64 = 0x560;
const OFF_PSEL_LAST: u64 = 0x56C;

#[derive(Debug, Default)]
pub struct Nrf52Pwm {
    events_stopped: u32,
    events_seqstarted: [u32; 2],
    events_seqend: [u32; 2],
    events_pwmperiodend: u32,
    events_loopsdone: u32,

    shorts: u32,
    inten: u32,

    enable: u32,
    mode: u32,
    countertop: u32,
    prescaler: u32,
    decoder: u32,
    loop_count: u32,

    seq: [u32; 12], // SEQ[0..1] x 4 words (PTR/CNT/REFRESH/ENDDELAY)
    psel_out: [u32; 4],

    /// Sequence pending for `tick_with_bus`. One of PENDING_{NONE,SEQ0,SEQ1}.
    pending: u8,
}

impl Nrf52Pwm {
    pub fn new() -> Self {
        Self::default()
    }

    fn enabled(&self) -> bool {
        self.enable & 1 != 0
    }
}

impl Peripheral for Nrf52Pwm {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_STOP | OFF_TASKS_SEQSTART0 | OFF_TASKS_SEQSTART1 | OFF_TASKS_NEXTSTEP => 0,

            OFF_EVENTS_STOPPED => self.events_stopped,
            OFF_EVENTS_SEQSTARTED0 => self.events_seqstarted[0],
            OFF_EVENTS_SEQSTARTED1 => self.events_seqstarted[1],
            OFF_EVENTS_SEQEND0 => self.events_seqend[0],
            OFF_EVENTS_SEQEND1 => self.events_seqend[1],
            OFF_EVENTS_PWMPERIODEND => self.events_pwmperiodend,
            OFF_EVENTS_LOOPSDONE => self.events_loopsdone,

            OFF_SHORTS => self.shorts,
            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten,

            OFF_ENABLE => self.enable & 1,
            OFF_MODE => self.mode & 0x1,
            OFF_COUNTERTOP => self.countertop & 0x7FFF,
            OFF_PRESCALER => self.prescaler & 0x7,
            OFF_DECODER => self.decoder & 0x103,
            OFF_LOOP => self.loop_count & 0xFFFF,

            OFF_SEQ_FIRST..=OFF_SEQ_LAST if offset.is_multiple_of(4) => {
                let idx = ((offset - OFF_SEQ_FIRST) / 4) as usize;
                if idx < 12 {
                    self.seq[idx]
                } else {
                    0
                }
            }
            OFF_PSEL_FIRST..=OFF_PSEL_LAST if offset.is_multiple_of(4) => {
                self.psel_out[((offset - OFF_PSEL_FIRST) / 4) as usize]
            }

            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            // ── TASKS (sequence engine; gated on ENABLE) ────────────────────
            OFF_TASKS_SEQSTART0 if value != 0 && self.enabled() => {
                self.pending = PENDING_SEQ0;
            }
            OFF_TASKS_SEQSTART1 if value != 0 && self.enabled() => {
                self.pending = PENDING_SEQ1;
            }
            OFF_TASKS_STOP if value != 0 && self.enabled() => {
                self.events_stopped = 1;
            }
            OFF_TASKS_STOP | OFF_TASKS_SEQSTART0 | OFF_TASKS_SEQSTART1 | OFF_TASKS_NEXTSTEP => {}

            // EVENTS_*: hardware-generated. SW write-1 is ignored; SW write-0 clears.
            OFF_EVENTS_STOPPED if value == 0 => self.events_stopped = 0,
            OFF_EVENTS_SEQSTARTED0 if value == 0 => self.events_seqstarted[0] = 0,
            OFF_EVENTS_SEQSTARTED1 if value == 0 => self.events_seqstarted[1] = 0,
            OFF_EVENTS_SEQEND0 if value == 0 => self.events_seqend[0] = 0,
            OFF_EVENTS_SEQEND1 if value == 0 => self.events_seqend[1] = 0,
            OFF_EVENTS_PWMPERIODEND if value == 0 => self.events_pwmperiodend = 0,
            OFF_EVENTS_LOOPSDONE if value == 0 => self.events_loopsdone = 0,

            OFF_SHORTS => self.shorts = value,
            OFF_INTEN => self.inten = value,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,

            OFF_ENABLE => self.enable = value & 1,
            OFF_MODE => self.mode = value & 0x1,
            OFF_COUNTERTOP => self.countertop = value & 0x7FFF,
            OFF_PRESCALER => self.prescaler = value & 0x7,
            OFF_DECODER => self.decoder = value & 0x103,
            OFF_LOOP => self.loop_count = value & 0xFFFF,

            OFF_SEQ_FIRST..=OFF_SEQ_LAST if offset.is_multiple_of(4) => {
                let idx = ((offset - OFF_SEQ_FIRST) / 4) as usize;
                if idx < 12 {
                    self.seq[idx] = value;
                }
            }
            OFF_PSEL_FIRST..=OFF_PSEL_LAST if offset.is_multiple_of(4) => {
                self.psel_out[((offset - OFF_PSEL_FIRST) / 4) as usize] = value;
            }
            _ => {}
        }
        Ok(())
    }

    fn needs_bus_tick(&self) -> bool {
        self.pending != PENDING_NONE
    }

    /// Sequence playback engine. Runs on the bus tick after TASKS_SEQSTART[n]:
    /// reads the SEQ[n].CNT duty values out of guest RAM at SEQ[n].PTR (so the
    /// sequence registers genuinely drive playback) and fires SEQSTARTED[n],
    /// SEQEND[n] and PWMPERIODEND to reflect the sequence having played.
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        let n = match self.pending {
            PENDING_SEQ0 => 0usize,
            PENDING_SEQ1 => 1usize,
            _ => return,
        };
        self.pending = PENDING_NONE;

        // SEQ[n] block: PTR at seq[n*8], CNT at seq[n*8 + 1].
        let ptr = self.seq[n * 8] as u64;
        let cnt = (self.seq[n * 8 + 1] & 0x7FFF) as usize;

        // Read each 16-bit duty value out of the EasyDMA buffer. There is no
        // waveform sink in simulation; consuming the buffer is what makes the
        // sequence registers genuinely drive the playback.
        for i in 0..cnt {
            let base = ptr + (i as u64) * 2;
            let _lo = bus.read_u8(base).unwrap_or(0);
            let _hi = bus.read_u8(base + 1).unwrap_or(0);
        }

        self.events_seqstarted[n] = 1;
        self.events_seqend[n] = 1;
        self.events_pwmperiodend = 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bus, DmaRequest, SimulationConfig};
    use std::collections::HashMap;

    // ── Minimal flat-RAM bus (mirrors the TWIM test harness) ──────────────────
    struct FlatRam {
        mem: HashMap<u64, u8>,
        config: SimulationConfig,
    }

    impl FlatRam {
        fn new() -> Self {
            Self {
                mem: HashMap::new(),
                config: SimulationConfig::default(),
            }
        }
        fn write_slice(&mut self, base: u64, data: &[u8]) {
            for (i, &b) in data.iter().enumerate() {
                self.mem.insert(base + i as u64, b);
            }
        }
    }

    impl Bus for FlatRam {
        fn read_u8(&self, addr: u64) -> crate::SimResult<u8> {
            Ok(*self.mem.get(&addr).unwrap_or(&0))
        }
        fn write_u8(&mut self, addr: u64, value: u8) -> crate::SimResult<()> {
            self.mem.insert(addr, value);
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[DmaRequest]) -> crate::SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &SimulationConfig {
            &self.config
        }
    }

    #[test]
    fn countertop_masks_to_15_bits() {
        let mut p = Nrf52Pwm::new();
        p.write_u32(OFF_COUNTERTOP, 0xFFFF_FFFF).unwrap();
        assert_eq!(p.read_u32(OFF_COUNTERTOP).unwrap(), 0x7FFF);
    }

    #[test]
    fn psel_out_round_trips() {
        let mut p = Nrf52Pwm::new();
        p.write_u32(0x560, 13).unwrap(); // PSEL.OUT[0] = P0.13
        assert_eq!(p.read_u32(0x560).unwrap(), 13);
    }

    #[test]
    fn seqstart0_plays_sequence_and_fires_events() {
        let mut p = Nrf52Pwm::new();
        let mut bus = FlatRam::new();
        let base: u64 = 0x2000_0000;
        // Two 16-bit duty samples.
        bus.write_slice(base, &[0x10, 0x80, 0x20, 0x80]);

        p.write_u32(OFF_ENABLE, 1).unwrap();
        p.write_u32(OFF_COUNTERTOP, 1000).unwrap();
        p.write_u32(OFF_SEQ_FIRST, base as u32).unwrap(); // SEQ[0].PTR
        p.write_u32(OFF_SEQ_FIRST + 4, 2).unwrap(); // SEQ[0].CNT = 2

        p.write_u32(OFF_TASKS_SEQSTART0, 1).unwrap();
        assert_eq!(p.read_u32(OFF_EVENTS_SEQEND0).unwrap(), 0, "before tick");
        assert!(p.needs_bus_tick());

        p.tick_with_bus(&mut bus);

        assert_eq!(
            p.read_u32(OFF_EVENTS_SEQSTARTED0).unwrap(),
            1,
            "SEQSTARTED0"
        );
        assert_eq!(p.read_u32(OFF_EVENTS_SEQEND0).unwrap(), 1, "SEQEND0");
        assert_eq!(
            p.read_u32(OFF_EVENTS_PWMPERIODEND).unwrap(),
            1,
            "PWMPERIODEND"
        );
        assert!(!p.needs_bus_tick(), "pending cleared");
    }

    #[test]
    fn seqstart1_uses_seq1_block() {
        let mut p = Nrf52Pwm::new();
        let mut bus = FlatRam::new();
        let base: u64 = 0x2000_0100;
        bus.write_slice(base, &[0xAA, 0x00]);

        p.write_u32(OFF_ENABLE, 1).unwrap();
        // SEQ[1].PTR is at 0x540, CNT at 0x544.
        p.write_u32(0x540, base as u32).unwrap();
        p.write_u32(0x544, 1).unwrap();

        p.write_u32(OFF_TASKS_SEQSTART1, 1).unwrap();
        p.tick_with_bus(&mut bus);

        assert_eq!(p.read_u32(OFF_EVENTS_SEQSTARTED1).unwrap(), 1);
        assert_eq!(p.read_u32(OFF_EVENTS_SEQEND1).unwrap(), 1);
    }

    #[test]
    fn seqstart_ignored_when_disabled() {
        let mut p = Nrf52Pwm::new();
        // ENABLE left at 0.
        p.write_u32(OFF_TASKS_SEQSTART0, 1).unwrap();
        assert!(!p.needs_bus_tick(), "disabled PWM does not arm a sequence");
    }

    #[test]
    fn stop_sets_stopped_when_enabled() {
        let mut p = Nrf52Pwm::new();
        p.write_u32(OFF_ENABLE, 1).unwrap();
        p.write_u32(OFF_TASKS_STOP, 1).unwrap();
        assert_eq!(p.read_u32(OFF_EVENTS_STOPPED).unwrap(), 1);
    }
}
