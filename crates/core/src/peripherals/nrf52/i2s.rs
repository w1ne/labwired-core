// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 I2S peripheral — register-surface model.
//!
//! Source: nRF52840 PS rev 1.7 §6.10 (I2S). Inter-IC Sound interface.
//! Models the configuration and pointer surface; no sample streaming.

use crate::{Peripheral, SimResult};

const OFF_TASKS_START: u64 = 0x000;
const OFF_TASKS_STOP: u64 = 0x004;
const OFF_EVENTS_RXPTRUPD: u64 = 0x104;
const OFF_EVENTS_STOPPED: u64 = 0x108;
const OFF_EVENTS_TXPTRUPD: u64 = 0x114;
const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_ENABLE: u64 = 0x500;
const OFF_CONFIG_FIRST: u64 = 0x504;
const OFF_CONFIG_LAST: u64 = 0x524;
const OFF_RXD_PTR: u64 = 0x538;
const OFF_TXD_PTR: u64 = 0x540;
const OFF_RXTXD_MAXCNT: u64 = 0x550;
const OFF_PSEL_FIRST: u64 = 0x560;
const OFF_PSEL_LAST: u64 = 0x574;

#[derive(Debug, Default)]
pub struct Nrf52I2s {
    events_rxptrupd: u32,
    events_stopped: u32,
    events_txptrupd: u32,
    inten: u32,
    enable: u32,
    config: [u32; 9], // CONFIG.MODE, RXEN, TXEN, MCKEN, MCKFREQ, RATIO, SWIDTH, ALIGN, FORMAT, CHANNELS
    rxd_ptr: u32,
    txd_ptr: u32,
    rxtxd_maxcnt: u32,
    psel: [u32; 6],
}

impl Nrf52I2s {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Nrf52I2s {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_START | OFF_TASKS_STOP => 0,
            OFF_EVENTS_RXPTRUPD => self.events_rxptrupd,
            OFF_EVENTS_STOPPED => self.events_stopped,
            OFF_EVENTS_TXPTRUPD => self.events_txptrupd,
            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_ENABLE => self.enable & 1,
            OFF_CONFIG_FIRST..=OFF_CONFIG_LAST if offset.is_multiple_of(4) => {
                self.config[((offset - OFF_CONFIG_FIRST) / 4) as usize]
            }
            OFF_RXD_PTR => self.rxd_ptr,
            OFF_TXD_PTR => self.txd_ptr,
            OFF_RXTXD_MAXCNT => self.rxtxd_maxcnt & 0x3FFF,
            OFF_PSEL_FIRST..=OFF_PSEL_LAST if offset.is_multiple_of(4) => {
                self.psel[((offset - OFF_PSEL_FIRST) / 4) as usize]
            }
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_START | OFF_TASKS_STOP => {}
            OFF_EVENTS_RXPTRUPD => self.events_rxptrupd = value & 1,
            OFF_EVENTS_STOPPED => self.events_stopped = value & 1,
            OFF_EVENTS_TXPTRUPD => self.events_txptrupd = value & 1,
            OFF_INTEN => self.inten = value,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,
            OFF_ENABLE => self.enable = value & 1,
            OFF_CONFIG_FIRST..=OFF_CONFIG_LAST if offset.is_multiple_of(4) => {
                self.config[((offset - OFF_CONFIG_FIRST) / 4) as usize] = value;
            }
            OFF_RXD_PTR => self.rxd_ptr = value,
            OFF_TXD_PTR => self.txd_ptr = value,
            OFF_RXTXD_MAXCNT => self.rxtxd_maxcnt = value & 0x3FFF,
            OFF_PSEL_FIRST..=OFF_PSEL_LAST if offset.is_multiple_of(4) => {
                self.psel[((offset - OFF_PSEL_FIRST) / 4) as usize] = value;
            }
            _ => {}
        }
        Ok(())
    }
}
