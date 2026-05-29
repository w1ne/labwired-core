// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 NFCT peripheral — register-surface model.
//!
//! Source: nRF52840 PS rev 1.7 §6.13 (NFCT). Near-Field Communication
//! tag emulator. Models the configuration surface — FRAMEDELAY,
//! NFCID, PACKETPTR, MAXLEN, AUTOCOLRESCONFIG — and lets all task/event
//! registers round-trip. No NFC carrier or peer interaction is modeled.

use crate::{Peripheral, SimResult};

const OFF_TASKS_ACTIVATE: u64 = 0x000;
const OFF_TASKS_DISABLE: u64 = 0x004;
const OFF_TASKS_SENSE: u64 = 0x008;
const OFF_TASKS_STARTTX: u64 = 0x00C;
const OFF_TASKS_ENABLERXDATA: u64 = 0x01C;
const OFF_TASKS_GOIDLE: u64 = 0x024;
const OFF_TASKS_GOSLEEP: u64 = 0x028;

const OFF_EVENTS_FIRST: u64 = 0x100;
const OFF_EVENTS_LAST: u64 = 0x150;

const OFF_SHORTS: u64 = 0x200;
const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;

const OFF_ERRORSTATUS: u64 = 0x404;
const OFF_NFCTAGSTATE: u64 = 0x410;
const OFF_SLEEPSTATE: u64 = 0x420;
const OFF_FIELDPRESENT: u64 = 0x43C;

const OFF_FRAMEDELAYMIN: u64 = 0x504;
const OFF_FRAMEDELAYMAX: u64 = 0x508;
const OFF_FRAMEDELAYMODE: u64 = 0x50C;
const OFF_PACKETPTR: u64 = 0x510;
const OFF_MAXLEN: u64 = 0x514;
const OFF_TXD_FRAMECONFIG: u64 = 0x518;
const OFF_TXD_AMOUNT: u64 = 0x51C;
const OFF_RXD_FRAMECONFIG: u64 = 0x520;
const OFF_RXD_AMOUNT: u64 = 0x524;
const OFF_SENSRES: u64 = 0x540;
const OFF_SELRES: u64 = 0x544;
const OFF_NFCID1_LAST: u64 = 0x590;
const OFF_NFCID1_2ND_LAST: u64 = 0x594;
const OFF_NFCID1_3RD_LAST: u64 = 0x598;
const OFF_AUTOCOLRESCONFIG: u64 = 0x59C;

#[derive(Debug, Default)]
pub struct Nrf52Nfct {
    events: [u32; 21], // 0x100..0x150 step 4 → 21 slots
    shorts: u32,
    inten: u32,

    errorstatus: u32,
    nfctagstate: u32,
    sleepstate: u32,
    fieldpresent: u32,

    framedelaymin: u32,
    framedelaymax: u32,
    framedelaymode: u32,
    packetptr: u32,
    maxlen: u32,
    txd_frameconfig: u32,
    txd_amount: u32,
    rxd_frameconfig: u32,
    rxd_amount: u32,
    sensres: u32,
    selres: u32,
    nfcid1_last: u32,
    nfcid1_2nd_last: u32,
    nfcid1_3rd_last: u32,
    autocolresconfig: u32,
}

impl Nrf52Nfct {
    pub fn new() -> Self {
        Self {
            framedelaymax: 0x1000, // PS table 80 reset
            framedelaymode: 1,
            maxlen: 0xFF,
            sensres: 0,
            selres: 0,
            autocolresconfig: 0,
            ..Self::default()
        }
    }
}

impl Peripheral for Nrf52Nfct {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_ACTIVATE
            | OFF_TASKS_DISABLE
            | OFF_TASKS_SENSE
            | OFF_TASKS_STARTTX
            | OFF_TASKS_ENABLERXDATA
            | OFF_TASKS_GOIDLE
            | OFF_TASKS_GOSLEEP => 0,

            OFF_EVENTS_FIRST..=OFF_EVENTS_LAST if offset.is_multiple_of(4) => {
                let idx = ((offset - OFF_EVENTS_FIRST) / 4) as usize;
                if idx < 21 {
                    self.events[idx]
                } else {
                    0
                }
            }

            OFF_SHORTS => self.shorts,
            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten,

            OFF_ERRORSTATUS => self.errorstatus,
            OFF_NFCTAGSTATE => self.nfctagstate & 0x7,
            OFF_SLEEPSTATE => self.sleepstate & 0x1,
            OFF_FIELDPRESENT => self.fieldpresent & 0x3,

            OFF_FRAMEDELAYMIN => self.framedelaymin & 0xFFFF,
            OFF_FRAMEDELAYMAX => self.framedelaymax & 0xFFFFF,
            OFF_FRAMEDELAYMODE => self.framedelaymode & 0x3,
            OFF_PACKETPTR => self.packetptr,
            OFF_MAXLEN => self.maxlen & 0x1FF,
            OFF_TXD_FRAMECONFIG => self.txd_frameconfig & 0x1F,
            OFF_TXD_AMOUNT => self.txd_amount & 0x1FFF,
            OFF_RXD_FRAMECONFIG => self.rxd_frameconfig & 0x1F,
            OFF_RXD_AMOUNT => self.rxd_amount & 0x1FFF,
            OFF_SENSRES => self.sensres & 0xFFFF,
            OFF_SELRES => self.selres & 0x7F,
            OFF_NFCID1_LAST => self.nfcid1_last,
            OFF_NFCID1_2ND_LAST => self.nfcid1_2nd_last & 0xFFFFFF,
            OFF_NFCID1_3RD_LAST => self.nfcid1_3rd_last & 0xFFFFFF,
            OFF_AUTOCOLRESCONFIG => self.autocolresconfig & 0x3,

            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_ACTIVATE
            | OFF_TASKS_DISABLE
            | OFF_TASKS_SENSE
            | OFF_TASKS_STARTTX
            | OFF_TASKS_ENABLERXDATA
            | OFF_TASKS_GOIDLE
            | OFF_TASKS_GOSLEEP => {}

            OFF_EVENTS_FIRST..=OFF_EVENTS_LAST if offset.is_multiple_of(4) => {
                let idx = ((offset - OFF_EVENTS_FIRST) / 4) as usize;
                if idx < 21 {
                    self.events[idx] = value & 1;
                }
            }

            OFF_SHORTS => self.shorts = value,
            OFF_INTEN => self.inten = value,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,

            OFF_ERRORSTATUS => self.errorstatus &= !value, // W1C
            OFF_NFCTAGSTATE | OFF_SLEEPSTATE | OFF_FIELDPRESENT => {} // RO

            OFF_FRAMEDELAYMIN => self.framedelaymin = value & 0xFFFF,
            OFF_FRAMEDELAYMAX => self.framedelaymax = value & 0xFFFFF,
            OFF_FRAMEDELAYMODE => self.framedelaymode = value & 0x3,
            OFF_PACKETPTR => self.packetptr = value,
            OFF_MAXLEN => self.maxlen = value & 0x1FF,
            OFF_TXD_FRAMECONFIG => self.txd_frameconfig = value & 0x1F,
            OFF_TXD_AMOUNT => self.txd_amount = value & 0x1FFF,
            OFF_RXD_FRAMECONFIG => self.rxd_frameconfig = value & 0x1F,
            OFF_RXD_AMOUNT => {} // RO per PS
            OFF_SENSRES => self.sensres = value & 0xFFFF,
            OFF_SELRES => self.selres = value & 0x7F,
            OFF_NFCID1_LAST => self.nfcid1_last = value,
            OFF_NFCID1_2ND_LAST => self.nfcid1_2nd_last = value & 0xFFFFFF,
            OFF_NFCID1_3RD_LAST => self.nfcid1_3rd_last = value & 0xFFFFFF,
            OFF_AUTOCOLRESCONFIG => self.autocolresconfig = value & 0x3,

            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framedelaymax_masks_to_20_bits() {
        let mut n = Nrf52Nfct::new();
        n.write_u32(OFF_FRAMEDELAYMAX, 0xFFFF_FFFF).unwrap();
        assert_eq!(n.read_u32(OFF_FRAMEDELAYMAX).unwrap(), 0xFFFFF);
    }

    #[test]
    fn nfcid1_last_round_trips() {
        let mut n = Nrf52Nfct::new();
        n.write_u32(OFF_NFCID1_LAST, 0xDEAD_BEEF).unwrap();
        assert_eq!(n.read_u32(OFF_NFCID1_LAST).unwrap(), 0xDEAD_BEEF);
    }
}
