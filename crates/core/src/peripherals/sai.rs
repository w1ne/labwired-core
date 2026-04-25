// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! SAI — STM32L4 serial audio interface (RM0351 §41).
//!
//! Two sub-blocks A and B sharing the same register file:
//!   GCR     @ 0x00
//!   ACR1    @ 0x04   ACR2 @ 0x08   AFRCR @ 0x0C   ASLOTR @ 0x10
//!   AIM     @ 0x14   ASR  @ 0x18   ACLRFR@ 0x1C   ADR    @ 0x20
//!   BCR1    @ 0x24   BCR2 @ 0x28   BFRCR @ 0x2C   BSLOTR @ 0x30
//!   BIM     @ 0x34   BSR  @ 0x38   BCLRFR@ 0x3C   BDR    @ 0x40
//!
//! Reset values per RM0351 §41.6 are all zero except SR.FLVL = 0b000 (FIFO empty).

use crate::SimResult;

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct SaiBlock {
    cr1: u32,
    cr2: u32,
    frcr: u32,
    slotr: u32,
    im: u32,
    sr: u32,
    clrfr: u32,
    dr: u32,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Sai {
    gcr: u32,
    a: SaiBlock,
    b: SaiBlock,
}

impl Sai {
    pub fn new() -> Self {
        Self::default()
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.gcr,
            0x04 => self.a.cr1,
            0x08 => self.a.cr2,
            0x0C => self.a.frcr,
            0x10 => self.a.slotr,
            0x14 => self.a.im,
            0x18 => self.a.sr,
            0x1C => self.a.clrfr,
            0x20 => self.a.dr,
            0x24 => self.b.cr1,
            0x28 => self.b.cr2,
            0x2C => self.b.frcr,
            0x30 => self.b.slotr,
            0x34 => self.b.im,
            0x38 => self.b.sr,
            0x3C => self.b.clrfr,
            0x40 => self.b.dr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.gcr = value & 0x33,
            0x04 => self.a.cr1 = value,
            0x08 => self.a.cr2 = value & 0xFFFF,
            0x0C => self.a.frcr = value,
            0x10 => self.a.slotr = value,
            0x14 => self.a.im = value & 0x7F,
            0x18 => {} // SR is read-only
            0x1C => {
                self.a.sr &= !(value & 0x77);
                self.a.clrfr = 0;
            }
            0x20 => self.a.dr = value,
            0x24 => self.b.cr1 = value,
            0x28 => self.b.cr2 = value & 0xFFFF,
            0x2C => self.b.frcr = value,
            0x30 => self.b.slotr = value,
            0x34 => self.b.im = value & 0x7F,
            0x38 => {}
            0x3C => {
                self.b.sr &= !(value & 0x77);
                self.b.clrfr = 0;
            }
            0x40 => self.b.dr = value,
            _ => {}
        }
    }
}

impl crate::Peripheral for Sai {
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
