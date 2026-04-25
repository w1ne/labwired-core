// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! PWR (power-control) peripheral — STM32L4 layout.
//!
//! Reset values verified against real NUCLEO-L476RG silicon via SWD
//! register dump. Used by every HAL-generated firmware: HAL_Init() calls
//! HAL_PWREx_ControlVoltageScaling() before any RCC PLL reconfiguration,
//! and a missing PWR peripheral bus-faults at the very first store.

use crate::SimResult;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Pwr {
    cr1: u32,
    cr2: u32,
    cr3: u32,
    cr4: u32,
    sr1: u32,
    sr2: u32,
    scr: u32,
    pucra: u32, pdcra: u32,
    pucrb: u32, pdcrb: u32,
    pucrc: u32, pdcrc: u32,
    pucrd: u32, pdcrd: u32,
    pucre: u32, pdcre: u32,
    pucrf: u32, pdcrf: u32,
    pucrg: u32, pdcrg: u32,
    pucrh: u32, pdcrh: u32,
    pucri: u32, pdcri: u32,
}

impl Pwr {
    pub fn new() -> Self {
        // Hardware-verified reset state from NUCLEO-L476RG SWD dump:
        //   CR1 = 0x0000_0200  VOS = 01 (range 1, default).
        //   CR3 = 0x0000_8000  EIWUL = 1 (internal wake-up line enabled).
        //   SR2 = 0x0000_0100  REGLPF = 1 (low-power regulator stabilised).
        // Other registers reset to 0.
        Self {
            cr1: 0x0000_0200,
            cr2: 0,
            cr3: 0x0000_8000,
            cr4: 0,
            sr1: 0,
            sr2: 0x0000_0100,
            scr: 0,
            pucra: 0, pdcra: 0,
            pucrb: 0, pdcrb: 0,
            pucrc: 0, pdcrc: 0,
            pucrd: 0, pdcrd: 0,
            pucre: 0, pdcre: 0,
            pucrf: 0, pdcrf: 0,
            pucrg: 0, pdcrg: 0,
            pucrh: 0, pdcrh: 0,
            pucri: 0, pdcri: 0,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.cr3,
            0x0C => self.cr4,
            0x10 => self.sr1,
            0x14 => self.sr2,
            0x18 => self.scr,
            0x20 => self.pucra, 0x24 => self.pdcra,
            0x28 => self.pucrb, 0x2C => self.pdcrb,
            0x30 => self.pucrc, 0x34 => self.pdcrc,
            0x38 => self.pucrd, 0x3C => self.pdcrd,
            0x40 => self.pucre, 0x44 => self.pdcre,
            0x48 => self.pucrf, 0x4C => self.pdcrf,
            0x50 => self.pucrg, 0x54 => self.pdcrg,
            0x58 => self.pucrh, 0x5C => self.pdcrh,
            0x60 => self.pucri, 0x64 => self.pdcri,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            // CR1 writable bits: LPMS[2:0], DBP, LPR, VOS[1:0], R1MODE, RRSTP — keep 17:0.
            0x00 => self.cr1 = value & 0x0003_FFFF,
            0x04 => self.cr2 = value,
            0x08 => self.cr3 = value,
            0x0C => self.cr4 = value,
            // SR1 / SR2 are read-mostly. Some bits are W1C via SCR; for
            // simplicity allow direct writes to leave register state.
            0x10 => self.sr1 = value,
            0x14 => self.sr2 = value,
            // SCR is write-1-to-clear into SR1 wake-up flags.
            0x18 => {
                // bits [4:0] clear corresponding SR1 wake-up flags; bit 8
                // clears SBF (standby flag); bit 9 clears WUFI.
                self.sr1 &= !(value & 0x0000_031F);
                self.scr = 0;
            }
            0x20 => self.pucra = value, 0x24 => self.pdcra = value,
            0x28 => self.pucrb = value, 0x2C => self.pdcrb = value,
            0x30 => self.pucrc = value, 0x34 => self.pdcrc = value,
            0x38 => self.pucrd = value, 0x3C => self.pdcrd = value,
            0x40 => self.pucre = value, 0x44 => self.pdcre = value,
            0x48 => self.pucrf = value, 0x4C => self.pdcrf = value,
            0x50 => self.pucrg = value, 0x54 => self.pdcrg = value,
            0x58 => self.pucrh = value, 0x5C => self.pdcrh = value,
            0x60 => self.pucri = value, 0x64 => self.pdcri = value,
            _ => {}
        }
    }
}

impl Default for Pwr { fn default() -> Self { Self::new() } }

impl crate::Peripheral for Pwr {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
