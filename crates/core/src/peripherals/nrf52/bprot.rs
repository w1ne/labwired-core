// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 BPROT (Block Protect).

use crate::{Peripheral, SimResult};

const OFF_CONFIG0: u64 = 0x600;
const OFF_CONFIG3: u64 = 0x60C;
const OFF_DISABLEINDEBUG: u64 = 0x610;

#[derive(Debug, Default)]
pub struct Nrf52Bprot {
    config: [u32; 4],
    disable_in_debug: u32,
}

impl Nrf52Bprot {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Nrf52Bprot {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_CONFIG0..=OFF_CONFIG3 if offset.is_multiple_of(4) => {
                self.config[((offset - OFF_CONFIG0) / 4) as usize]
            }
            OFF_DISABLEINDEBUG => self.disable_in_debug & 1,
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_CONFIG0..=OFF_CONFIG3 if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_CONFIG0) / 4) as usize;
                self.config[i] |= value;
            }
            OFF_DISABLEINDEBUG => self.disable_in_debug = value & 1,
            _ => {}
        }
        Ok(())
    }
}
