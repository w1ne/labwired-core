// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 SAADC peripheral — register-surface model.
//!
//! Source: nRF52840 PS rev 1.7 §6.23 (SAADC). 8-channel 12-bit successive
//! approximation ADC. Models enough register state for firmware probing
//! ENABLE / RESOLUTION / CH[i].PSELP / CH[i].PSELN / CH[i].CONFIG to round
//! trip; no actual conversions are performed (RESULT will be whatever the
//! pointer at RESULT.PTR contains).

use crate::{Peripheral, SimResult};

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
}

impl Nrf52Saadc {
    pub fn new() -> Self {
        Self::default()
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
