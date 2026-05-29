// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 MWU (Memory Watch Unit).
//!
//! Source: nRF52840 PS rev 1.7 §6.12 (MWU). Debug peripheral that
//! generates events on memory accesses. Register-surface only —
//! no actual access monitoring.

use crate::{Peripheral, SimResult};

#[derive(Debug, Default)]
pub struct Nrf52Mwu {
    regs: std::collections::BTreeMap<u64, u32>,
}

impl Nrf52Mwu {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Nrf52Mwu {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.regs.get(&offset).copied().unwrap_or(0))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        // Tasks at low offsets are write-1-trigger; ignore.
        if offset >= 0x100 {
            self.regs.insert(offset, value);
        }
        Ok(())
    }
}
