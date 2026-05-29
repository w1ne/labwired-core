// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 PWM peripheral — register-surface model.
//!
//! Source: nRF52840 PS rev 1.7 §6.18 (PWM). Models PWM0..PWM3 — same
//! register map per instance. Drives 4 outputs per peripheral.

use crate::{Peripheral, SimResult};

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
}

impl Nrf52Pwm {
    pub fn new() -> Self {
        Self::default()
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
            OFF_TASKS_STOP
            | OFF_TASKS_SEQSTART0
            | OFF_TASKS_SEQSTART1
            | OFF_TASKS_NEXTSTEP => 0,

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
            OFF_TASKS_STOP
            | OFF_TASKS_SEQSTART0
            | OFF_TASKS_SEQSTART1
            | OFF_TASKS_NEXTSTEP => {}

            OFF_EVENTS_STOPPED => self.events_stopped = value & 1,
            OFF_EVENTS_SEQSTARTED0 => self.events_seqstarted[0] = value & 1,
            OFF_EVENTS_SEQSTARTED1 => self.events_seqstarted[1] = value & 1,
            OFF_EVENTS_SEQEND0 => self.events_seqend[0] = value & 1,
            OFF_EVENTS_SEQEND1 => self.events_seqend[1] = value & 1,
            OFF_EVENTS_PWMPERIODEND => self.events_pwmperiodend = value & 1,
            OFF_EVENTS_LOOPSDONE => self.events_loopsdone = value & 1,

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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
