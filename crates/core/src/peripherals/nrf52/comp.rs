// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 COMP (Comparator).
//!
//! Source: nRF52840 PS rev 1.7 §6.2 (COMP). Register-surface model;
//! comparison output is not driven (firmware reading RESULT sees 0).

use crate::{Peripheral, SimResult};

const OFF_TASKS_START: u64 = 0x000;
const OFF_TASKS_STOP: u64 = 0x004;
const OFF_TASKS_SAMPLE: u64 = 0x008;
const OFF_EVENTS_READY: u64 = 0x100;
const OFF_EVENTS_DOWN: u64 = 0x104;
const OFF_EVENTS_UP: u64 = 0x108;
const OFF_EVENTS_CROSS: u64 = 0x10C;
const OFF_SHORTS: u64 = 0x200;
const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_RESULT: u64 = 0x400;
const OFF_ENABLE: u64 = 0x500;
const OFF_PSEL: u64 = 0x504;
const OFF_REFSEL: u64 = 0x508;
const OFF_EXTREFSEL: u64 = 0x50C;
const OFF_TH: u64 = 0x530;
const OFF_MODE: u64 = 0x534;
const OFF_HYST: u64 = 0x538;
const OFF_ISOURCE: u64 = 0x53C;

#[derive(Debug, Default)]
pub struct Nrf52Comp {
    events_ready: u32,
    events_down: u32,
    events_up: u32,
    events_cross: u32,
    shorts: u32,
    inten: u32,
    enable: u32,
    psel: u32,
    refsel: u32,
    extrefsel: u32,
    th: u32,
    mode: u32,
    hyst: u32,
    isource: u32,
}

impl Nrf52Comp {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Nrf52Comp {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_START | OFF_TASKS_STOP | OFF_TASKS_SAMPLE => 0,
            OFF_EVENTS_READY => self.events_ready,
            OFF_EVENTS_DOWN => self.events_down,
            OFF_EVENTS_UP => self.events_up,
            OFF_EVENTS_CROSS => self.events_cross,
            OFF_SHORTS => self.shorts,
            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_RESULT => 0,
            OFF_ENABLE => self.enable & 0x3,
            OFF_PSEL => self.psel & 0x7,
            OFF_REFSEL => self.refsel & 0x7,
            OFF_EXTREFSEL => self.extrefsel & 0x7,
            OFF_TH => self.th & 0x3F3F,
            OFF_MODE => self.mode & 0x303,
            OFF_HYST => self.hyst & 0x1,
            OFF_ISOURCE => self.isource & 0x3,
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_START | OFF_TASKS_STOP | OFF_TASKS_SAMPLE => {}
            OFF_EVENTS_READY => self.events_ready = value & 1,
            OFF_EVENTS_DOWN => self.events_down = value & 1,
            OFF_EVENTS_UP => self.events_up = value & 1,
            OFF_EVENTS_CROSS => self.events_cross = value & 1,
            OFF_SHORTS => self.shorts = value & 0xF,
            OFF_INTEN => self.inten = value & 0xF,
            OFF_INTENSET => self.inten |= value & 0xF,
            OFF_INTENCLR => self.inten &= !value,
            OFF_ENABLE => self.enable = value & 0x3,
            OFF_PSEL => self.psel = value & 0x7,
            OFF_REFSEL => self.refsel = value & 0x7,
            OFF_EXTREFSEL => self.extrefsel = value & 0x7,
            OFF_TH => self.th = value & 0x3F3F,
            OFF_MODE => self.mode = value & 0x303,
            OFF_HYST => self.hyst = value & 0x1,
            OFF_ISOURCE => self.isource = value & 0x3,
            _ => {}
        }
        Ok(())
    }
}
