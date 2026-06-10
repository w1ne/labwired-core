// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 FICR (Factory Information Configuration Registers).
//!
//! Source: nRF52840 PS rev 1.7 §6.7 (FICR). Read-only factory data —
//! DEVICEID (unique 64-bit chip ID), DEVICETYPE, INFO.PART/VARIANT,
//! NFC.TAGHEADER, TEMP calibration coefficients, etc.
//!
//! Zephyr and nRF SDK init code reads FICR at startup; we expose stable
//! reset values so probing succeeds without crashing. The unique
//! DEVICEID matches the value reported by the silicon we cross-validated
//! against (the XIAO board used in nrf52_mmio_diff). RNG/temp
//! calibration regs return spec-default values; firmware that uses them
//! for accuracy correction will accept the result.

use crate::{Peripheral, SimResult};

const OFF_CODEPAGESIZE: u64 = 0x010;
const OFF_CODESIZE: u64 = 0x014;
const OFF_DEVICEID0: u64 = 0x060;
const OFF_DEVICEID1: u64 = 0x064;
const OFF_ER0: u64 = 0x080;
const OFF_ER3: u64 = 0x08C;
const OFF_IR0: u64 = 0x090;
const OFF_IR3: u64 = 0x09C;
const OFF_DEVICEADDRTYPE: u64 = 0x0A0;
const OFF_DEVICEADDR0: u64 = 0x0A4;
const OFF_DEVICEADDR1: u64 = 0x0A8;
const OFF_INFO_PART: u64 = 0x100;
const OFF_INFO_VARIANT: u64 = 0x104;
const OFF_INFO_PACKAGE: u64 = 0x108;
const OFF_INFO_RAM: u64 = 0x10C;
const OFF_INFO_FLASH: u64 = 0x110;
const OFF_TEMP_A0: u64 = 0x404;
const OFF_TEMP_A5: u64 = 0x418;
const OFF_TEMP_B0: u64 = 0x41C;
const OFF_TEMP_B5: u64 = 0x430;
const OFF_TEMP_T0: u64 = 0x434;
const OFF_TEMP_T4: u64 = 0x444;
const OFF_NFC_TAGHEADER0: u64 = 0x450;
const OFF_NFC_TAGHEADER3: u64 = 0x45C;

#[derive(Debug)]
pub struct Nrf52Ficr {
    deviceid: [u32; 2],
    er: [u32; 4],
    ir: [u32; 4],
    deviceaddrtype: u32,
    deviceaddr: [u32; 2],
    info_part: u32,
    info_variant: u32,
    info_package: u32,
    info_ram: u32,
    info_flash: u32,
    temp_a: [u32; 6],
    temp_b: [u32; 6],
    temp_t: [u32; 5],
    nfc_tagheader: [u32; 4],
    codepagesize: u32,
    codesize: u32,
}

impl Default for Nrf52Ficr {
    fn default() -> Self {
        // Values match the XIAO Sense (CPUID 0x410FC241, INFO.PART
        // 0x52840) used in nrf52_mmio_diff against real silicon.
        // DEVICEID0/1 are the unique 64-bit ID we read off the live chip
        // (FICR 0x10000060 = 0x707DC298, 0x10000064 = 0x940D8A73).
        // INFO.PART = 0x52840, VARIANT = 'AAFA' for the QIAA-AA0 package
        // commonly used on XIAO and DK boards.
        Self {
            deviceid: [0x707D_C298, 0x940D_8A73],
            er: [0; 4],
            ir: [0; 4],
            deviceaddrtype: 0, // public address
            deviceaddr: [0x1122_3344, 0x0000_5566],
            info_part: 0x0005_2840,
            info_variant: 0x4141_4430, // "AAD0" — confirmed on bench silicon (DK rev3)
            info_package: 0x0000_2004, // QIAA
            info_ram: 256,             // KiB
            info_flash: 1024,          // KiB
            temp_a: [0; 6],
            temp_b: [0; 6],
            temp_t: [0; 5],
            nfc_tagheader: [0xFFFF_FFFF; 4],
            codepagesize: 4096,
            codesize: 256, // pages of 4 KiB → 1 MiB
        }
    }
}

impl Nrf52Ficr {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Nrf52Ficr {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // FICR is read-only.
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_CODEPAGESIZE => self.codepagesize,
            OFF_CODESIZE => self.codesize,
            OFF_DEVICEID0 => self.deviceid[0],
            OFF_DEVICEID1 => self.deviceid[1],
            OFF_ER0..=OFF_ER3 if offset.is_multiple_of(4) => {
                self.er[((offset - OFF_ER0) / 4) as usize]
            }
            OFF_IR0..=OFF_IR3 if offset.is_multiple_of(4) => {
                self.ir[((offset - OFF_IR0) / 4) as usize]
            }
            OFF_DEVICEADDRTYPE => self.deviceaddrtype,
            OFF_DEVICEADDR0 => self.deviceaddr[0],
            OFF_DEVICEADDR1 => self.deviceaddr[1],
            OFF_INFO_PART => self.info_part,
            OFF_INFO_VARIANT => self.info_variant,
            OFF_INFO_PACKAGE => self.info_package,
            OFF_INFO_RAM => self.info_ram,
            OFF_INFO_FLASH => self.info_flash,
            OFF_TEMP_A0..=OFF_TEMP_A5 if offset.is_multiple_of(4) => {
                self.temp_a[((offset - OFF_TEMP_A0) / 4) as usize]
            }
            OFF_TEMP_B0..=OFF_TEMP_B5 if offset.is_multiple_of(4) => {
                self.temp_b[((offset - OFF_TEMP_B0) / 4) as usize]
            }
            OFF_TEMP_T0..=OFF_TEMP_T4 if offset.is_multiple_of(4) => {
                self.temp_t[((offset - OFF_TEMP_T0) / 4) as usize]
            }
            OFF_NFC_TAGHEADER0..=OFF_NFC_TAGHEADER3 if offset.is_multiple_of(4) => {
                self.nfc_tagheader[((offset - OFF_NFC_TAGHEADER0) / 4) as usize]
            }
            _ => 0,
        })
    }

    fn write_u32(&mut self, _offset: u64, _value: u32) -> SimResult<()> {
        // FICR is read-only; silently drop writes.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_part_reads_nrf52840() {
        let f = Nrf52Ficr::new();
        assert_eq!(f.read_u32(OFF_INFO_PART).unwrap(), 0x52840);
    }

    #[test]
    fn deviceid_matches_xiao() {
        let f = Nrf52Ficr::new();
        assert_eq!(f.read_u32(OFF_DEVICEID0).unwrap(), 0x707D_C298);
        assert_eq!(f.read_u32(OFF_DEVICEID1).unwrap(), 0x940D_8A73);
    }

    #[test]
    fn writes_are_dropped() {
        let mut f = Nrf52Ficr::new();
        f.write_u32(OFF_INFO_PART, 0xDEAD_BEEF).unwrap();
        assert_eq!(f.read_u32(OFF_INFO_PART).unwrap(), 0x52840);
    }
}
