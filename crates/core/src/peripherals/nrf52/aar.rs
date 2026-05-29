// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 AAR (Accelerated Address Resolver).
//!
//! Source: nRF52840 PS rev 1.7 §6.1 (AAR). BLE crypto helper for
//! resolving private addresses. Register-surface only — no crypto.

use crate::{Peripheral, SimResult};

const OFF_TASKS_START: u64 = 0x000;
const OFF_TASKS_STOP: u64 = 0x008;
const OFF_EVENTS_END: u64 = 0x100;
const OFF_EVENTS_RESOLVED: u64 = 0x104;
const OFF_EVENTS_NOTRESOLVED: u64 = 0x108;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_STATUS: u64 = 0x400;
const OFF_ENABLE: u64 = 0x500;
const OFF_NIRK: u64 = 0x504;
const OFF_IRKPTR: u64 = 0x508;
const OFF_ADDRPTR: u64 = 0x510;
const OFF_SCRATCHPTR: u64 = 0x514;

#[derive(Debug, Default)]
pub struct Nrf52Aar {
    events_end: u32,
    events_resolved: u32,
    events_notresolved: u32,
    inten: u32,
    enable: u32,
    nirk: u32,
    irkptr: u32,
    addrptr: u32,
    scratchptr: u32,
}

impl Nrf52Aar {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Nrf52Aar {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_START | OFF_TASKS_STOP => 0,
            OFF_EVENTS_END => self.events_end,
            OFF_EVENTS_RESOLVED => self.events_resolved,
            OFF_EVENTS_NOTRESOLVED => self.events_notresolved,
            OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_STATUS => 0,
            OFF_ENABLE => self.enable & 0x3,
            OFF_NIRK => self.nirk & 0x1F,
            OFF_IRKPTR => self.irkptr,
            OFF_ADDRPTR => self.addrptr,
            OFF_SCRATCHPTR => self.scratchptr,
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_START | OFF_TASKS_STOP => {}
            OFF_EVENTS_END => self.events_end = value & 1,
            OFF_EVENTS_RESOLVED => self.events_resolved = value & 1,
            OFF_EVENTS_NOTRESOLVED => self.events_notresolved = value & 1,
            OFF_INTENSET => self.inten |= value & 0x7,
            OFF_INTENCLR => self.inten &= !value,
            OFF_ENABLE => self.enable = value & 0x3,
            OFF_NIRK => self.nirk = value & 0x1F,
            OFF_IRKPTR => self.irkptr = value,
            OFF_ADDRPTR => self.addrptr = value,
            OFF_SCRATCHPTR => self.scratchptr = value,
            _ => {}
        }
        Ok(())
    }
}
