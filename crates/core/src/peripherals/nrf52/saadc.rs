// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 SAADC peripheral — register surface + EasyDMA conversion
//! engine.
//!
//! Source: nRF52840 PS rev 1.7 §6.23 (SAADC). 8-channel 12-bit successive
//! approximation ADC. Models ENABLE / RESOLUTION / CH[i].PSELP / CH[i].PSELN /
//! CH[i].CONFIG round-tripping plus a deterministic conversion engine:
//!
//! # Conversion engine (deterministic, no analog source)
//!
//! **TASKS_START (0x000):** if ENABLE=1, fires EVENTS_STARTED (0x100) — the
//! peripheral is now armed and waiting for a sample trigger.
//!
//! **TASKS_SAMPLE (0x004):** if ENABLE=1, performs `RESULT.MAXCNT` 12-bit
//! conversions, writing a deterministic 16-bit sample (`SAMPLE_VALUE`) to the
//! EasyDMA buffer at RESULT.PTR in guest RAM, sets RESULT.AMOUNT, then fires
//! EVENTS_END (0x104) + EVENTS_RESULTDONE (0x10C). There is no analog source,
//! so every sample is the same constant — enough for firmware to observe a
//! genuine EasyDMA result round-trip. The memory write runs on the next bus
//! tick (same `needs_bus_tick`/`tick_with_bus` pattern as TWIM/SPIM), keeping
//! the register write itself synchronous.
//!
//! **TASKS_STOP (0x008):** fires EVENTS_STOPPED (0x114).
//!
//! # EVENTS write semantics
//!
//! SW writes of 1 are silently ignored (hardware-generated only). SW writes of
//! 0 clear the event register, matching the other Nordic peripherals.

use crate::{Bus, Peripheral, SimResult};

/// Deterministic 12-bit sample value written for every conversion. There is no
/// analog input source in simulation, so the engine returns a fixed code; this
/// is sufficient for firmware to observe a real EasyDMA RESULT round-trip.
const SAMPLE_VALUE: u16 = 0x0AAA;

/// No conversion pending.
const PENDING_NONE: u8 = 0;
/// TASKS_SAMPLE was written — run conversions on the next bus tick.
const PENDING_SAMPLE: u8 = 1;

// Tasks
const OFF_TASKS_START: u64 = 0x000;
const OFF_TASKS_SAMPLE: u64 = 0x004;
const OFF_TASKS_STOP: u64 = 0x008;
const OFF_TASKS_CALIBRATEOFFSET: u64 = 0x00C;

// Events
const OFF_EVENTS_STARTED: u64 = 0x100;
const OFF_EVENTS_END: u64 = 0x104;
const OFF_EVENTS_DONE: u64 = 0x108;
const OFF_EVENTS_RESULTDONE: u64 = 0x10C;
const OFF_EVENTS_CALIBRATEDONE: u64 = 0x110;
const OFF_EVENTS_STOPPED: u64 = 0x114;
// EVENTS_CH[i].LIMITH at 0x118 + 0x10*i, .LIMITL at 0x11C + 0x10*i.
const OFF_EVENTS_CH_FIRST: u64 = 0x118;
const OFF_EVENTS_CH_LAST: u64 = 0x184; // CH[7].LIMITL

const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_STATUS: u64 = 0x400;
const OFF_ENABLE: u64 = 0x500;

// Per-channel block at 0x510 + 0x10*i, 4 words: PSELP/PSELN/CONFIG/LIMIT.
const OFF_CH_FIRST: u64 = 0x510;
const OFF_CH_LAST: u64 = 0x58C; // CH[7].LIMIT

const OFF_RESOLUTION: u64 = 0x5F0;
const OFF_OVERSAMPLE: u64 = 0x5F4;
const OFF_SAMPLERATE: u64 = 0x5F8;

const OFF_RESULT_PTR: u64 = 0x62C;
const OFF_RESULT_MAXCNT: u64 = 0x630;
const OFF_RESULT_AMOUNT: u64 = 0x634;

#[derive(Debug, Default)]
pub struct Nrf52Saadc {
    events_started: u32,
    events_end: u32,
    events_done: u32,
    events_resultdone: u32,
    events_calibratedone: u32,
    events_stopped: u32,
    events_ch: [u32; 16], // CH[0..7] LIMITH/LIMITL alternating

    inten: u32,
    status: u32,
    enable: u32,
    ch: [u32; 32], // CH[0..7] x 4 registers (PSELP/PSELN/CONFIG/LIMIT)
    resolution: u32,
    oversample: u32,
    samplerate: u32,
    result_ptr: u32,
    result_maxcnt: u32,
    result_amount: u32,

    /// Conversion pending for `tick_with_bus`. One of PENDING_{NONE,SAMPLE}.
    pending: u8,
}

impl Nrf52Saadc {
    pub fn new() -> Self {
        Self::default()
    }

    fn enabled(&self) -> bool {
        self.enable & 1 != 0
    }
}

impl Peripheral for Nrf52Saadc {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_START | OFF_TASKS_SAMPLE | OFF_TASKS_STOP | OFF_TASKS_CALIBRATEOFFSET => 0,
            OFF_EVENTS_STARTED => self.events_started,
            OFF_EVENTS_END => self.events_end,
            OFF_EVENTS_DONE => self.events_done,
            OFF_EVENTS_RESULTDONE => self.events_resultdone,
            OFF_EVENTS_CALIBRATEDONE => self.events_calibratedone,
            OFF_EVENTS_STOPPED => self.events_stopped,
            OFF_EVENTS_CH_FIRST..=OFF_EVENTS_CH_LAST if offset.is_multiple_of(4) => {
                self.events_ch[((offset - OFF_EVENTS_CH_FIRST) / 4) as usize]
            }
            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_STATUS => self.status,
            OFF_ENABLE => self.enable & 1,
            OFF_CH_FIRST..=OFF_CH_LAST if offset.is_multiple_of(4) => {
                self.ch[((offset - OFF_CH_FIRST) / 4) as usize]
            }
            OFF_RESOLUTION => self.resolution & 0x7,
            OFF_OVERSAMPLE => self.oversample & 0xF,
            OFF_SAMPLERATE => self.samplerate,
            OFF_RESULT_PTR => self.result_ptr,
            OFF_RESULT_MAXCNT => self.result_maxcnt & 0x7FFF,
            OFF_RESULT_AMOUNT => self.result_amount & 0x7FFF,
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            // ── TASKS (conversion engine; gated on ENABLE) ──────────────────
            // START arms the ADC and fires STARTED synchronously (no RAM).
            OFF_TASKS_START if value != 0 && self.enabled() => {
                self.events_started = 1;
            }
            // SAMPLE performs the EasyDMA conversion on the next bus tick.
            OFF_TASKS_SAMPLE if value != 0 && self.enabled() => {
                self.pending = PENDING_SAMPLE;
            }
            // STOP fires STOPPED synchronously.
            OFF_TASKS_STOP if value != 0 && self.enabled() => {
                self.events_stopped = 1;
            }
            OFF_TASKS_START | OFF_TASKS_SAMPLE | OFF_TASKS_STOP | OFF_TASKS_CALIBRATEOFFSET => {}
            // EVENTS_*: hardware-generated. SW write-1 is ignored; SW write-0 clears.
            OFF_EVENTS_STARTED if value == 0 => self.events_started = 0,
            OFF_EVENTS_END if value == 0 => self.events_end = 0,
            OFF_EVENTS_DONE if value == 0 => self.events_done = 0,
            OFF_EVENTS_RESULTDONE if value == 0 => self.events_resultdone = 0,
            OFF_EVENTS_CALIBRATEDONE if value == 0 => self.events_calibratedone = 0,
            OFF_EVENTS_STOPPED if value == 0 => self.events_stopped = 0,
            // write-1 falls through to the no-op default (ignored).
            OFF_EVENTS_CH_FIRST..=OFF_EVENTS_CH_LAST if offset.is_multiple_of(4) && value == 0 => {
                self.events_ch[((offset - OFF_EVENTS_CH_FIRST) / 4) as usize] = 0;
            }
            OFF_INTEN => self.inten = value,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,
            OFF_STATUS => {} // RO
            OFF_ENABLE => self.enable = value & 1,
            OFF_CH_FIRST..=OFF_CH_LAST if offset.is_multiple_of(4) => {
                self.ch[((offset - OFF_CH_FIRST) / 4) as usize] = value;
            }
            OFF_RESOLUTION => self.resolution = value & 0x7,
            OFF_OVERSAMPLE => self.oversample = value & 0xF,
            OFF_SAMPLERATE => self.samplerate = value,
            OFF_RESULT_PTR => self.result_ptr = value,
            OFF_RESULT_MAXCNT => self.result_maxcnt = value & 0x7FFF,
            OFF_RESULT_AMOUNT => {} // RO
            _ => {}
        }
        Ok(())
    }

    fn needs_bus_tick(&self) -> bool {
        self.pending != PENDING_NONE
    }

    /// Conversion engine. Runs on the bus tick after TASKS_SAMPLE: writes
    /// `RESULT.MAXCNT` deterministic 16-bit samples into the EasyDMA buffer at
    /// `RESULT.PTR`, sets RESULT.AMOUNT, and fires EVENTS_END + RESULTDONE.
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if self.pending != PENDING_SAMPLE {
            return;
        }
        self.pending = PENDING_NONE;

        let ptr = self.result_ptr as u64;
        let maxcnt = (self.result_maxcnt & 0x7FFF) as usize;
        let sample = SAMPLE_VALUE.to_le_bytes();

        for i in 0..maxcnt {
            let base = ptr + (i as u64) * 2;
            let _ = bus.write_u8(base, sample[0]);
            let _ = bus.write_u8(base + 1, sample[1]);
        }

        self.result_amount = maxcnt as u32;
        self.events_end = 1;
        self.events_resultdone = 1;
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
        fn read_slice(&self, base: u64, len: usize) -> Vec<u8> {
            (0..len)
                .map(|i| *self.mem.get(&(base + i as u64)).unwrap_or(&0))
                .collect()
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
    fn resolution_masks_to_3_bits() {
        let mut s = Nrf52Saadc::new();
        s.write_u32(OFF_RESOLUTION, 0xFF).unwrap();
        assert_eq!(s.read_u32(OFF_RESOLUTION).unwrap(), 7);
    }

    #[test]
    fn channel0_config_round_trips() {
        let mut s = Nrf52Saadc::new();
        // CH[0].CONFIG = +0x518 → offset 0x008 within channel block.
        s.write_u32(0x518, 0x0002_0210).unwrap();
        assert_eq!(s.read_u32(0x518).unwrap(), 0x0002_0210);
    }

    #[test]
    fn start_sets_started_when_enabled() {
        let mut s = Nrf52Saadc::new();
        s.write_u32(OFF_ENABLE, 1).unwrap();
        s.write_u32(OFF_TASKS_START, 1).unwrap();
        assert_eq!(s.read_u32(OFF_EVENTS_STARTED).unwrap(), 1);
    }

    #[test]
    fn start_ignored_when_disabled() {
        let mut s = Nrf52Saadc::new();
        // ENABLE left at 0.
        s.write_u32(OFF_TASKS_START, 1).unwrap();
        assert_eq!(s.read_u32(OFF_EVENTS_STARTED).unwrap(), 0);
    }

    #[test]
    fn sample_writes_easydma_result_and_fires_events() {
        let mut s = Nrf52Saadc::new();
        let mut bus = FlatRam::new();
        let base: u64 = 0x2000_0000;

        s.write_u32(OFF_ENABLE, 1).unwrap();
        s.write_u32(0x510, 0).unwrap(); // CH[0].PSELP = AnalogInput0 (config read)
        s.write_u32(OFF_RESULT_PTR, base as u32).unwrap();
        s.write_u32(OFF_RESULT_MAXCNT, 4).unwrap();

        s.write_u32(OFF_TASKS_START, 1).unwrap();
        s.write_u32(OFF_TASKS_SAMPLE, 1).unwrap();
        // Events not set before the bus tick performs the conversion.
        assert_eq!(s.read_u32(OFF_EVENTS_END).unwrap(), 0);
        assert!(s.needs_bus_tick());

        s.tick_with_bus(&mut bus);

        assert_eq!(s.read_u32(OFF_EVENTS_END).unwrap(), 1, "END fired");
        assert_eq!(
            s.read_u32(OFF_EVENTS_RESULTDONE).unwrap(),
            1,
            "RESULTDONE fired"
        );
        assert_eq!(s.read_u32(OFF_RESULT_AMOUNT).unwrap(), 4, "AMOUNT = MAXCNT");
        assert!(!s.needs_bus_tick(), "pending cleared after tick");

        // Four little-endian 16-bit samples of SAMPLE_VALUE.
        let expect: Vec<u8> = (0..4).flat_map(|_| SAMPLE_VALUE.to_le_bytes()).collect();
        assert_eq!(bus.read_slice(base, 8), expect, "RESULT buffer filled");
    }

    #[test]
    fn sample_ignored_when_disabled() {
        let mut s = Nrf52Saadc::new();
        s.write_u32(OFF_RESULT_MAXCNT, 4).unwrap();
        s.write_u32(OFF_TASKS_SAMPLE, 1).unwrap();
        assert!(
            !s.needs_bus_tick(),
            "disabled SAADC does not arm conversion"
        );
    }

    #[test]
    fn stop_sets_stopped_when_enabled() {
        let mut s = Nrf52Saadc::new();
        s.write_u32(OFF_ENABLE, 1).unwrap();
        s.write_u32(OFF_TASKS_STOP, 1).unwrap();
        assert_eq!(s.read_u32(OFF_EVENTS_STOPPED).unwrap(), 1);
    }
}
