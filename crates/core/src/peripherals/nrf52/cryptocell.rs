// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 CRYPTOCELL (CC310 bridge).
//!
//! Source: nRF52840 PS rev 1.7 §6.6 (CRYPTOCELL). The wrapper
//! peripheral that enables the CC310 (ARM CryptoCell-310) IP. We
//! model just ENABLE so firmware that calls nrf_cc310_enable() can
//! see the register round-trip; the underlying CC310 engine itself
//! is not implemented.

use crate::{Peripheral, SimResult};

const OFF_ENABLE: u64 = 0x500;

#[derive(Debug, Default)]
pub struct Nrf52Cryptocell {
    enable: u32,
}

impl Nrf52Cryptocell {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Nrf52Cryptocell {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_ENABLE => self.enable & 1,
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset == OFF_ENABLE {
            self.enable = value & 1;
        }
        Ok(())
    }
}
