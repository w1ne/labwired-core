// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 ECB peripheral — register-surface model.
//!
//! Source: nRF52840 PS rev 1.7 §6.5 (ECB). AES-128 ECB block coprocessor.
//! Models the pointer + event surface; no AES engine.

use crate::{Peripheral, SimResult};

const OFF_TASKS_STARTECB: u64 = 0x000;
const OFF_TASKS_STOPECB: u64 = 0x004;
const OFF_EVENTS_ENDECB: u64 = 0x100;
const OFF_EVENTS_ERRORECB: u64 = 0x104;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_ECBDATAPTR: u64 = 0x504;

#[derive(Debug, Default)]
pub struct Nrf52Ecb {
    events_endecb: u32,
    events_errorecb: u32,
    inten: u32,
    ecbdataptr: u32,
}

impl Nrf52Ecb {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Nrf52Ecb {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_STARTECB | OFF_TASKS_STOPECB => 0,
            OFF_EVENTS_ENDECB => self.events_endecb,
            OFF_EVENTS_ERRORECB => self.events_errorecb,
            OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_ECBDATAPTR => self.ecbdataptr,
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_STARTECB | OFF_TASKS_STOPECB => {}
            OFF_EVENTS_ENDECB => self.events_endecb = value & 1,
            OFF_EVENTS_ERRORECB => self.events_errorecb = value & 1,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,
            OFF_ECBDATAPTR => self.ecbdataptr = value,
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecbdataptr_round_trips() {
        let mut e = Nrf52Ecb::new();
        e.write_u32(OFF_ECBDATAPTR, 0x2000_2000).unwrap();
        assert_eq!(e.read_u32(OFF_ECBDATAPTR).unwrap(), 0x2000_2000);
    }
}
