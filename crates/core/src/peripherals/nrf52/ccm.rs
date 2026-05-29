// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 CCM (Cipher block Chaining MAC).
//!
//! Source: nRF52840 PS rev 1.7 §6.5 (CCM). BLE crypto. Register-
//! surface only — no actual AES-CCM operations.

use crate::{Peripheral, SimResult};

const OFF_TASKS_KSGEN: u64 = 0x000;
const OFF_TASKS_CRYPT: u64 = 0x004;
const OFF_TASKS_STOP: u64 = 0x008;
const OFF_TASKS_RATEOVERRIDE: u64 = 0x00C;
const OFF_EVENTS_ENDKSGEN: u64 = 0x100;
const OFF_EVENTS_ENDCRYPT: u64 = 0x104;
const OFF_EVENTS_ERROR: u64 = 0x108;
const OFF_SHORTS: u64 = 0x200;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_MICSTATUS: u64 = 0x400;
const OFF_ENABLE: u64 = 0x500;
const OFF_MODE: u64 = 0x504;
const OFF_CNFPTR: u64 = 0x508;
const OFF_INPTR: u64 = 0x50C;
const OFF_OUTPTR: u64 = 0x510;
const OFF_SCRATCHPTR: u64 = 0x514;
const OFF_MAXPACKETSIZE: u64 = 0x518;
const OFF_RATEOVERRIDE: u64 = 0x51C;

#[derive(Debug, Default)]
pub struct Nrf52Ccm {
    events_endksgen: u32,
    events_endcrypt: u32,
    events_error: u32,
    shorts: u32,
    inten: u32,
    enable: u32,
    mode: u32,
    cnfptr: u32,
    inptr: u32,
    outptr: u32,
    scratchptr: u32,
    maxpacketsize: u32,
    rateoverride: u32,
}

impl Nrf52Ccm {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Nrf52Ccm {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_KSGEN | OFF_TASKS_CRYPT | OFF_TASKS_STOP | OFF_TASKS_RATEOVERRIDE => 0,
            OFF_EVENTS_ENDKSGEN => self.events_endksgen,
            OFF_EVENTS_ENDCRYPT => self.events_endcrypt,
            OFF_EVENTS_ERROR => self.events_error,
            OFF_SHORTS => self.shorts,
            OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_MICSTATUS => 1, // MIC valid (we don't check)
            OFF_ENABLE => self.enable & 0x3,
            OFF_MODE => self.mode,
            OFF_CNFPTR => self.cnfptr,
            OFF_INPTR => self.inptr,
            OFF_OUTPTR => self.outptr,
            OFF_SCRATCHPTR => self.scratchptr,
            OFF_MAXPACKETSIZE => self.maxpacketsize & 0xFF,
            OFF_RATEOVERRIDE => self.rateoverride & 0xF,
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_KSGEN | OFF_TASKS_CRYPT | OFF_TASKS_STOP | OFF_TASKS_RATEOVERRIDE => {}
            OFF_EVENTS_ENDKSGEN => self.events_endksgen = value & 1,
            OFF_EVENTS_ENDCRYPT => self.events_endcrypt = value & 1,
            OFF_EVENTS_ERROR => self.events_error = value & 1,
            OFF_SHORTS => self.shorts = value,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,
            OFF_ENABLE => self.enable = value & 0x3,
            OFF_MODE => self.mode = value,
            OFF_CNFPTR => self.cnfptr = value,
            OFF_INPTR => self.inptr = value,
            OFF_OUTPTR => self.outptr = value,
            OFF_SCRATCHPTR => self.scratchptr = value,
            OFF_MAXPACKETSIZE => self.maxpacketsize = value & 0xFF,
            OFF_RATEOVERRIDE => self.rateoverride = value & 0xF,
            _ => {}
        }
        Ok(())
    }
}
