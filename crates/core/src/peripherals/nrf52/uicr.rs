// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 UICR (User Information Configuration Registers).
//!
//! Source: nRF52840 PS rev 1.7 §6.33 (UICR). Customer-programmable
//! one-time configuration: APPROTECT, customer regs, PSELRESET, NFCPINS,
//! REGOUT0. Erased value is 0xFFFFFFFF for every field; the NVMC programs
//! it (single-write per bit, like flash). We model it as a stateful
//! peripheral where writes are masked (0→1 transitions are dropped, only
//! 1→0 takes effect), matching real flash semantics.

use crate::{Peripheral, SimResult};

const OFF_NRFFW_FIRST: u64 = 0x014;
const OFF_NRFFW_LAST: u64 = 0x050;
const OFF_NRFHW_FIRST: u64 = 0x050;
const OFF_NRFHW_LAST: u64 = 0x07C;
const OFF_CUSTOMER_FIRST: u64 = 0x080;
const OFF_CUSTOMER_LAST: u64 = 0x0FC;
const OFF_PSELRESET0: u64 = 0x200;
const OFF_PSELRESET1: u64 = 0x204;
const OFF_APPROTECT: u64 = 0x208;
const OFF_NFCPINS: u64 = 0x20C;
const OFF_DEBUGCTRL: u64 = 0x210;
const OFF_REGOUT0: u64 = 0x304;

const ERASED: u32 = 0xFFFF_FFFF;

#[derive(Debug)]
pub struct Nrf52Uicr {
    customer: [u32; 32], // 0x080..0x100, 32 words
    nrffw: [u32; 16],
    nrfhw: [u32; 12],
    pselreset: [u32; 2],
    approtect: u32,
    nfcpins: u32,
    debugctrl: u32,
    regout0: u32,
}

impl Default for Nrf52Uicr {
    fn default() -> Self {
        Self {
            customer: [ERASED; 32],
            nrffw: [ERASED; 16],
            nrfhw: [ERASED; 12],
            pselreset: [ERASED; 2],
            approtect: ERASED,
            // NFCPINS: 0xFFFFFFFE on this bench board — NFC pins are configured
            // (bit 0 cleared = use P0.09/P0.10 as NFC antenna, not GPIO).
            // Confirmed by live silicon read on the DK board used for hw-oracle tests.
            nfcpins: 0xFFFF_FFFE,
            debugctrl: ERASED,
            regout0: ERASED,
        }
    }
}

impl Nrf52Uicr {
    pub fn new() -> Self {
        Self::default()
    }

    /// Flash-write semantics: bits can only transition 1 → 0. A write
    /// effectively ANDs the new value with the current value.
    fn flash_write(slot: &mut u32, value: u32) {
        *slot &= value;
    }
}

impl Peripheral for Nrf52Uicr {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0xFF)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_NRFFW_FIRST..OFF_NRFFW_LAST if offset.is_multiple_of(4) => {
                self.nrffw[((offset - OFF_NRFFW_FIRST) / 4) as usize]
            }
            OFF_NRFHW_FIRST..OFF_NRFHW_LAST if offset.is_multiple_of(4) => {
                self.nrfhw[((offset - OFF_NRFHW_FIRST) / 4) as usize]
            }
            OFF_CUSTOMER_FIRST..=OFF_CUSTOMER_LAST if offset.is_multiple_of(4) => {
                self.customer[((offset - OFF_CUSTOMER_FIRST) / 4) as usize]
            }
            OFF_PSELRESET0 => self.pselreset[0],
            OFF_PSELRESET1 => self.pselreset[1],
            OFF_APPROTECT => self.approtect,
            OFF_NFCPINS => self.nfcpins,
            OFF_DEBUGCTRL => self.debugctrl,
            OFF_REGOUT0 => self.regout0,
            _ => ERASED,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_NRFFW_FIRST..OFF_NRFFW_LAST if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_NRFFW_FIRST) / 4) as usize;
                Self::flash_write(&mut self.nrffw[i], value);
            }
            OFF_NRFHW_FIRST..OFF_NRFHW_LAST if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_NRFHW_FIRST) / 4) as usize;
                Self::flash_write(&mut self.nrfhw[i], value);
            }
            OFF_CUSTOMER_FIRST..=OFF_CUSTOMER_LAST if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_CUSTOMER_FIRST) / 4) as usize;
                Self::flash_write(&mut self.customer[i], value);
            }
            OFF_PSELRESET0 => Self::flash_write(&mut self.pselreset[0], value),
            OFF_PSELRESET1 => Self::flash_write(&mut self.pselreset[1], value),
            OFF_APPROTECT => Self::flash_write(&mut self.approtect, value),
            OFF_NFCPINS => Self::flash_write(&mut self.nfcpins, value),
            OFF_DEBUGCTRL => Self::flash_write(&mut self.debugctrl, value),
            OFF_REGOUT0 => Self::flash_write(&mut self.regout0, value),
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approtect_starts_erased() {
        let u = Nrf52Uicr::new();
        assert_eq!(u.read_u32(OFF_APPROTECT).unwrap(), 0xFFFF_FFFF);
    }

    #[test]
    fn write_zero_clears_bits() {
        let mut u = Nrf52Uicr::new();
        u.write_u32(OFF_APPROTECT, 0x0000_00FF).unwrap();
        assert_eq!(u.read_u32(OFF_APPROTECT).unwrap(), 0x0000_00FF);
    }

    #[test]
    fn write_cannot_set_bits() {
        let mut u = Nrf52Uicr::new();
        u.write_u32(OFF_APPROTECT, 0).unwrap();
        u.write_u32(OFF_APPROTECT, 0xFFFF_FFFF).unwrap();
        assert_eq!(u.read_u32(OFF_APPROTECT).unwrap(), 0);
    }
}
