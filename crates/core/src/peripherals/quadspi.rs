// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! QUADSPI — STM32L4 quad-SPI flash interface (RM0351 §14).
//!
//! Register layout: CR/DCR/SR/FCR/DLR/CCR/AR/ABR/DR/PSMKR/PSMAR/PIR/LPTR.
//! Reset values per RM0351 §14.5: SR=0, all other registers = 0.
//! BUSY (SR bit 5) is gated by CR.EN; we keep it deasserted so HAL polls succeed.

use crate::SimResult;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Quadspi {
    cr: u32,
    dcr: u32,
    sr: u32,
    fcr: u32,
    dlr: u32,
    ccr: u32,
    ar: u32,
    abr: u32,
    dr: u32,
    psmkr: u32,
    psmar: u32,
    pir: u32,
    lptr: u32,
}

impl Quadspi {
    pub fn new() -> Self {
        Self {
            cr: 0,
            dcr: 0,
            // TCF + TOF + SMF + FTF cleared, BUSY=0, FLEVEL=0
            sr: 0,
            fcr: 0,
            dlr: 0,
            ccr: 0,
            ar: 0,
            abr: 0,
            dr: 0,
            psmkr: 0,
            psmar: 0,
            pir: 0,
            lptr: 0,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.dcr,
            0x08 => self.sr,
            0x0C => self.fcr,
            0x10 => self.dlr,
            0x14 => self.ccr,
            0x18 => self.ar,
            0x1C => self.abr,
            0x20 => self.dr,
            0x24 => self.psmkr,
            0x28 => self.psmar,
            0x2C => self.pir,
            0x30 => self.lptr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => {
                self.cr = value;
                // ABORT bit (3) self-clears once any in-flight op completes —
                // we have no in-flight ops, so clear immediately.
                self.cr &= !(1 << 3);
            }
            0x04 => self.dcr = value & 0x07FF_FF27,
            0x08 => {} // SR is read-only; flags cleared via FCR
            0x0C => {
                // FCR is rc_w1: clear matching bits in SR
                self.sr &= !(value & 0x1B);
                self.fcr = 0;
            }
            0x10 => self.dlr = value,
            0x14 => {
                self.ccr = value;
                // INDIRECT-WRITE / INDIRECT-READ functional modes assert TCF
                // (bit 1) immediately for survival-mode firmware that polls
                // for completion. Real hardware drives TCF after data phase.
                let fmode = (value >> 26) & 0x3;
                if fmode != 0 && (self.cr & 1) != 0 {
                    self.sr |= 1 << 1; // TCF
                }
            }
            0x18 => self.ar = value,
            0x1C => self.abr = value,
            0x20 => self.dr = value,
            0x24 => self.psmkr = value,
            0x28 => self.psmar = value,
            0x2C => self.pir = value & 0xFFFF,
            0x30 => self.lptr = value & 0xFFFF,
            _ => {}
        }
    }
}

impl Default for Quadspi {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for Quadspi {
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
