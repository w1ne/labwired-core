// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 PDM peripheral — register-surface model.
//!
//! Source: nRF52840 PS rev 1.7 §6.20 (PDM). Pulse-density-modulation
//! microphone input. Models the static configuration surface; no sample
//! generation.

use crate::{Peripheral, SimResult};

const OFF_TASKS_START: u64 = 0x000;
const OFF_TASKS_STOP: u64 = 0x004;
const OFF_EVENTS_STARTED: u64 = 0x100;
const OFF_EVENTS_STOPPED: u64 = 0x104;
const OFF_EVENTS_END: u64 = 0x108;
const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_ENABLE: u64 = 0x500;
const OFF_PDMCLKCTRL: u64 = 0x504;
const OFF_MODE: u64 = 0x508;
const OFF_GAINL: u64 = 0x518;
const OFF_GAINR: u64 = 0x51C;
const OFF_RATIO: u64 = 0x520;
const OFF_PSEL_CLK: u64 = 0x540;
const OFF_PSEL_DIN: u64 = 0x544;
const OFF_SAMPLE_PTR: u64 = 0x560;
const OFF_SAMPLE_MAXCNT: u64 = 0x564;

#[derive(Debug, Default)]
pub struct Nrf52Pdm {
    events_started: u32,
    events_stopped: u32,
    events_end: u32,
    inten: u32,
    enable: u32,
    pdmclkctrl: u32,
    mode: u32,
    gainl: u32,
    gainr: u32,
    ratio: u32,
    psel_clk: u32,
    psel_din: u32,
    sample_ptr: u32,
    sample_maxcnt: u32,
}

impl Nrf52Pdm {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Nrf52Pdm {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_START | OFF_TASKS_STOP => 0,
            OFF_EVENTS_STARTED => self.events_started,
            OFF_EVENTS_STOPPED => self.events_stopped,
            OFF_EVENTS_END => self.events_end,
            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_ENABLE => self.enable & 1,
            OFF_PDMCLKCTRL => self.pdmclkctrl,
            OFF_MODE => self.mode & 0x3,
            OFF_GAINL => self.gainl & 0x7F,
            OFF_GAINR => self.gainr & 0x7F,
            OFF_RATIO => self.ratio & 0x1,
            OFF_PSEL_CLK => self.psel_clk,
            OFF_PSEL_DIN => self.psel_din,
            OFF_SAMPLE_PTR => self.sample_ptr,
            OFF_SAMPLE_MAXCNT => self.sample_maxcnt & 0x7FFF,
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_START | OFF_TASKS_STOP => {}
            OFF_EVENTS_STARTED => self.events_started = value & 1,
            OFF_EVENTS_STOPPED => self.events_stopped = value & 1,
            OFF_EVENTS_END => self.events_end = value & 1,
            OFF_INTEN => self.inten = value,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,
            OFF_ENABLE => self.enable = value & 1,
            OFF_PDMCLKCTRL => self.pdmclkctrl = value,
            OFF_MODE => self.mode = value & 0x3,
            OFF_GAINL => self.gainl = value & 0x7F,
            OFF_GAINR => self.gainr = value & 0x7F,
            OFF_RATIO => self.ratio = value & 0x1,
            OFF_PSEL_CLK => self.psel_clk = value,
            OFF_PSEL_DIN => self.psel_din = value,
            OFF_SAMPLE_PTR => self.sample_ptr = value,
            OFF_SAMPLE_MAXCNT => self.sample_maxcnt = value & 0x7FFF,
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdmclkctrl_round_trips() {
        let mut p = Nrf52Pdm::new();
        p.write_u32(OFF_PDMCLKCTRL, 0x0800_0000).unwrap();
        assert_eq!(p.read_u32(OFF_PDMCLKCTRL).unwrap(), 0x0800_0000);
    }
}
